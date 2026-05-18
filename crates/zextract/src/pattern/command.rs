//! Command pattern: hybrid prompt-anchored + executable-anchored detection.
//!
//! Strategy (per planning.md Q11-Q13):
//!   1. PROMPT-ANCHORED: line starts with a recognized prompt marker
//!      (`❯ `, `$ `, `> `, `% `, `# `). The command is the rest of the line
//!      plus any trailing-backslash continuation lines spliced in.
//!   2. EXEC-ANCHORED (fallback): line contains a known trigger executable
//!      (`sudo`, `curl`, `wget`, `cat`, `git`, ...). The command runs from
//!      the trigger to end-of-line. No continuation splicing for the exec
//!      flavor — too risky when embedded in prose.
//!
//! Captures `{match}` only in v1; `{argv0}`/`{args}` deferred to v2.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::config::schema::CommandPatternConfig;
use crate::extract::{Match, MatchType};

const MAX_CONTINUATION_LINES: usize = 10;

/// Minimum character length for a command match. Filters out spurious
/// single-word or near-empty matches (e.g. bare `❯` lines, lone `$`).
const MIN_COMMAND_LEN: usize = 5;

/// Returns true if `s` looks like a plausible command — must contain at
/// least one ASCII letter. Rejects pure-numeric/punctuation strings such
/// as fish's right-aligned timestamp (`18:48:12`) that bleed onto an
/// otherwise empty prompt line in the terminal scrollback.
fn looks_like_command(s: &str) -> bool {
    s.trim().len() >= MIN_COMMAND_LEN && s.trim().chars().any(|c| c.is_ascii_alphabetic())
}

/// Default prompt markers. Configurable via KDL in Phase 7.
const PROMPT_MARKERS: &[&str] = &["❯ ", "$ ", "> ", "% ", "# "];

/// Default trigger list. Configurable via KDL in Phase 7. Order doesn't
/// matter — we collapse to leftmost-longest in code.
const TRIGGERS: &[&str] = &[
    // Install / package managers
    "sudo",
    "apt",
    "apt-get",
    "yum",
    "dnf",
    "pacman",
    "brew",
    "snap",
    "pip",
    "pip3",
    "pipx",
    "gem",
    "cargo",
    "go",
    "npm",
    "yarn",
    "pnpm",
    "bun",
    "uv",
    "poetry",
    "conda",
    "mamba",
    // Fetch
    "curl",
    "wget",
    "fetch",
    // Shell exec
    "sh",
    "bash",
    "zsh",
    "fish",
    "/bin/sh",
    "/bin/bash",
    // Build
    "make",
    "cmake",
    "ninja",
    "just",
    "nix",
    "nix-shell",
    "nix-build",
    // Editor / pager / IO
    "nvim",
    "vim",
    "nano",
    "emacs",
    "less",
    "more",
    "cat",
    "tee",
    "xargs",
    "awk",
    "sed",
    "grep",
    "find",
    // VCS
    "git",
    "hg",
    "svn",
    // Containers / orchestration / multiplexers
    "docker",
    "podman",
    "kubectl",
    "helm",
    "zellij",
    "tmux",
    // Language runners
    "python",
    "python3",
    "node",
    "deno",
    "ruby",
    "rustc",
    "java",
    "mvn",
    "gradle",
    // File ops
    "tar",
    "gunzip",
    "unzip",
    "chmod",
    "chown",
    "ln",
    "mkdir",
    "rm",
    "cp",
    "mv",
    "ssh",
    "scp",
    "rsync",
];

/// Patterns we strip from the start of a continuation line during
/// splicing. Order: most specific first.
const CONTINUATION_STRIP: &[&str] = &[
    r"^\s*\d+[:\.]?\s+", // line numbers ("  42  ", "2: ", "2. ")
    r"^[+\-]\s+",        // diff add/remove markers
    r"^[#>|]\s+",        // comment / quote / table-cell markers
    r"^\s+",             // leading whitespace (catchall)
];

fn trigger_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pattern = format!(
            r"\b({})\b",
            TRIGGERS
                .iter()
                .map(|t| regex_escape(t))
                .collect::<Vec<_>>()
                .join("|")
        );
        Regex::new(&pattern).expect("trigger regex compiles")
    })
}

fn continuation_strip_regexes() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        CONTINUATION_STRIP
            .iter()
            .map(|p| Regex::new(p).expect("continuation-strip regex compiles"))
            .collect()
    })
}

fn regex_escape(s: &str) -> String {
    // Minimal regex escape for our trigger list.
    s.chars()
        .map(|c| match c {
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\'
            | '/' => format!(r"\{}", c),
            other => other.to_string(),
        })
        .collect()
}

pub fn extract(text: &str, cfg: &CommandPatternConfig) -> Vec<Match> {
    let lines: Vec<&str> = text.lines().collect();
    let line_offsets: Vec<usize> = compute_line_offsets(&lines);

    let mut out = Vec::new();
    let mut skip_until: usize = 0;

    for (i, line) in lines.iter().enumerate() {
        if i < skip_until {
            continue;
        }

        // 1. PROMPT-ANCHORED.
        if let Some((prompt_len, cmd_after_prompt)) = match_prompt(line) {
            let cmd_after_prompt = trim_rprompt(cmd_after_prompt, cfg.rprompt_min_spaces);
            if !cmd_after_prompt.trim().is_empty() {
                let (full_cmd, context, lines_consumed) =
                    splice_continuation(&lines, i, cmd_after_prompt);
                if looks_like_command(&full_cmd) {
                    let span_start = line_offsets[i] + prompt_len;
                    let span_end = if lines_consumed == 1 {
                        span_start + cmd_after_prompt.len()
                    } else {
                        line_offsets[i + lines_consumed - 1] + lines[i + lines_consumed - 1].len()
                    };
                    out.push(make_match(full_cmd, context, span_start, span_end));
                    skip_until = i + lines_consumed;
                    continue;
                }
            }
        }

        // 2. EXEC-ANCHORED (fallback). No continuation splice — too risky in prose.
        if let Some(start_col) = match_exec(line) {
            let cmd = &line[start_col..];
            let trimmed = trim_rprompt(cmd, cfg.rprompt_min_spaces).trim_end();
            if looks_like_command(trimmed) {
                let span_start = line_offsets[i] + start_col;
                let span_end = span_start + trimmed.len();
                out.push(make_match(
                    trimmed.to_string(),
                    line.to_string(),
                    span_start,
                    span_end,
                ));
            }
        }
    }
    out
}

fn compute_line_offsets(lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len());
    let mut off = 0;
    for line in lines {
        offsets.push(off);
        off += line.len() + 1; // +1 for '\n'
    }
    offsets
}

/// If `line` begins with a known prompt marker, return (marker_len, rest).
fn match_prompt(line: &str) -> Option<(usize, &str)> {
    for marker in PROMPT_MARKERS {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((marker.len(), rest));
        }
    }
    None
}

/// Return the byte column where a leftmost trigger occurs in `line`, or
/// None if no trigger fires. Filters out triggers that aren't in a
/// command-start context — \b alone matches `sh` inside `install.sh`
/// (the `.` is a non-word char so a word boundary exists), so we
/// additionally require the byte preceding the trigger to be a real
/// command-start (whitespace, line start, shell operator, ...).
fn match_exec(line: &str) -> Option<usize> {
    let re = trigger_regex();
    for m in re.find_iter(line) {
        let start = m.start();
        let prev = if start == 0 {
            None
        } else {
            line.as_bytes().get(start - 1).copied()
        };
        if ok_command_preceding_byte(prev) {
            return Some(start);
        }
    }
    None
}

fn ok_command_preceding_byte(b: Option<u8>) -> bool {
    match b {
        None => true,
        Some(c) if c.is_ascii_whitespace() => true,
        // Shell separators / operators + prose punctuation that can
        // precede a command word.
        Some(
            b'|' | b';' | b'&' | b'(' | b'[' | b'{' | b'`' | b'$' | b'=' | b'>' | b'<' | b'"'
            | b'\'' | b':' | b',',
        ) => true,
        // `.` and `/` are explicitly rejected — they signal file-extension
        // (`install.sh`) or path-component (`/usr/bin/sh`) context, not a
        // standalone command word.
        _ => false,
    }
}

/// Splice a prompt-anchored command's continuations. Returns
/// `(full_command_text, full_context, lines_consumed)`. `lines_consumed`
/// is at least 1 (the starting line itself).
fn splice_continuation(
    lines: &[&str],
    start_idx: usize,
    first_cmd: &str,
) -> (String, String, usize) {
    let mut cmd = first_cmd.to_string();
    let mut context = lines[start_idx].to_string();
    let mut consumed = 1usize;
    let strip_res = continuation_strip_regexes();

    while ends_with_continuation(&cmd) && consumed < MAX_CONTINUATION_LINES {
        let next_idx = start_idx + consumed;
        if next_idx >= lines.len() {
            break;
        }
        let next_line = lines[next_idx];
        // Strip leading noise.
        let stripped = strip_leading(next_line, strip_res);
        // Drop trailing backslash AND any whitespace around it, then add
        // exactly one space before the spliced continuation.
        let trimmed_len = cmd
            .trim_end_matches(|c: char| c.is_whitespace() || c == '\\')
            .len();
        cmd.truncate(trimmed_len);
        cmd.push(' ');
        cmd.push_str(stripped);
        context.push('\n');
        context.push_str(next_line);
        consumed += 1;
    }
    (cmd, context, consumed)
}

fn ends_with_continuation(s: &str) -> bool {
    s.trim_end().ends_with('\\')
}

/// Truncate `s` at the first run of `min_spaces` or more consecutive ASCII
/// whitespace characters. Fish/zsh right-side prompts (timestamps, git status)
/// are pushed to the right edge with a wide column of spaces; `min_spaces`
/// controls how many spaces in a row constitute a cut point. Default (5) avoids
/// false positives on double-spaced output like `git diff --stat` while still
/// reliably catching rprompts.
fn trim_rprompt(s: &str, min_spaces: usize) -> &str {
    if min_spaces == 0 {
        return s;
    }
    let b = s.as_bytes();
    let mut run = 0usize;
    for (i, &byte) in b.iter().enumerate() {
        if byte.is_ascii_whitespace() {
            run += 1;
            if run >= min_spaces {
                return &s[..i + 1 - run];
            }
        } else {
            run = 0;
        }
    }
    s
}

fn strip_leading<'a>(line: &'a str, patterns: &[Regex]) -> &'a str {
    for re in patterns {
        if let Some(m) = re.find(line) {
            if m.start() == 0 {
                return &line[m.end()..];
            }
        }
    }
    line
}

// ---- Flag-anchored detection ----

/// Boundary bytes that end a context prefix and start a new command context.
const FLAG_BOUNDARY: &[u8] = b"]})[{><:;|&(,'\"";

fn flag_anchor_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Matches --long-flag or -x / -xyz (combined short flags).
        // First char after `-` must be alphabetic — rejects negative
        // numbers (-42) and ranges (-1..5).
        Regex::new(r"(?:--[a-zA-Z][\w-]*|-[a-zA-Z]\w*)").expect("flag anchor regex")
    })
}

/// A byte may precede a standalone flag token (`-x`, `--foo`) only if it
/// signals a word boundary in a command context. This rejects flags that
/// are embedded in compound words like `dry-run` or `some-file`, where
/// the preceding byte is an alphanumeric letter.
fn ok_flag_preceding_byte(b: Option<u8>) -> bool {
    match b {
        None => true,
        Some(c) if c.is_ascii_whitespace() => true,
        Some(b'(' | b'&' | b'|' | b';' | b'=') => true,
        _ => false,
    }
}

/// Opt-in: find the byte column where a flag-anchored command starts on
/// `line`, or `None` if the line doesn't qualify.
fn flag_anchored_start(line: &str) -> Option<usize> {
    let re = flag_anchor_regex();

    // Find the leftmost flag that looks like a standalone argument — not
    // embedded in a compound word like `dry-run` or `some-file`.
    let flag_match = re.find_iter(line).find(|m| {
        let prev = (m.start() > 0).then(|| line.as_bytes()[m.start() - 1]);
        ok_flag_preceding_byte(prev)
    })?;

    // Walk backward through the prefix to find the last boundary char.
    let prefix = &line[..flag_match.start()];
    let cmd_start = prefix
        .as_bytes()
        .iter()
        .rposition(|&b| FLAG_BOUNDARY.contains(&b))
        .map(|i| i + 1)
        .unwrap_or(0);

    // Skip leading whitespace after the boundary.
    let cmd_start = cmd_start
        + line[cmd_start..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| c.len_utf8())
            .sum::<usize>();

    let first_char = line[cmd_start..].chars().next()?;

    // Guard: first char must be lowercase ASCII.
    //   - Rejects flag-first lines (`--option val`) where first char is `-`
    //   - Rejects prose starting uppercase (`The --flag`) → `T` fails
    //   - Rejects non-ASCII prompt chars (`❯`) → multi-byte, not ascii_lowercase
    if !first_char.is_ascii_lowercase() {
        return None;
    }

    // Guard: first word must be at least 2 chars (avoids lone-letter noise).
    let word_end = cmd_start
        + line[cmd_start..]
            .find(|c: char| c.is_whitespace())
            .unwrap_or(line.len() - cmd_start);
    if word_end - cmd_start < 2 {
        return None;
    }

    Some(cmd_start)
}

/// Opt-in pass: extract commands anchored by a flag argument rather than a
/// prompt marker or trigger word. Skips prompt-anchored lines to avoid
/// producing a redundant match alongside the prompt-anchored result.
pub fn extract_flag_anchored(text: &str, cfg: &CommandPatternConfig) -> Vec<Match> {
    let lines: Vec<&str> = text.lines().collect();
    let line_offsets = compute_line_offsets(&lines);
    let mut out = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if match_prompt(line).is_some() {
            continue; // already handled by prompt-anchored path
        }
        let Some(start) = flag_anchored_start(line) else {
            continue;
        };
        let trimmed = trim_rprompt(&line[start..], cfg.rprompt_min_spaces).trim_end();
        if !looks_like_command(trimmed) {
            continue;
        }
        let span_start = line_offsets[i] + start;
        let span_end = span_start + trimmed.len();
        out.push(make_match(
            trimmed.to_string(),
            line.to_string(),
            span_start,
            span_end,
        ));
    }
    out
}

fn make_match(raw: String, context: String, span_start: usize, span_end: usize) -> Match {
    let mut fields = HashMap::new();
    fields.insert("match".to_string(), raw.clone());
    Match {
        ty: MatchType::Command,
        raw: raw.clone(),
        display: raw,
        context,
        label: None,
        span: (span_start, span_end),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> CommandPatternConfig {
        CommandPatternConfig::default()
    }

    #[test]
    fn prompt_anchored_simple() {
        let m = extract("$ git log --oneline -n 20", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "git log --oneline -n 20");
    }

    #[test]
    fn prompt_anchored_unicode() {
        let m = extract("❯ cargo build --release", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "cargo build --release");
    }

    #[test]
    fn exec_anchored_in_prose() {
        let m = extract("To install run sudo apt install zellij from the README.", &def());
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("sudo apt install zellij"));
    }

    #[test]
    fn exec_anchored_pipeline_kept_together() {
        let m = extract("curl -fsSL https://example.com/install.sh | sudo bash", &def());
        assert_eq!(m.len(), 1);
        // Full pipeline captured as one match.
        assert!(m[0].raw.contains("curl"));
        assert!(m[0].raw.contains("| sudo bash"));
    }

    #[test]
    fn continuation_splicing_basic() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n    | sudo bash";
        let m = extract(text, &def());
        assert_eq!(m.len(), 1);
        // The trailing backslash and leading whitespace on line 2 are stripped.
        assert_eq!(
            m[0].raw,
            "curl -fsSL https://example.com/install.sh | sudo bash"
        );
    }

    #[test]
    fn continuation_strips_line_number_prefix() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n2:  | sudo bash";
        let m = extract(text, &def());
        assert_eq!(m.len(), 1);
        assert_eq!(
            m[0].raw,
            "curl -fsSL https://example.com/install.sh | sudo bash"
        );
    }

    #[test]
    fn continuation_strips_diff_marker() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n+   | sudo bash";
        let m = extract(text, &def());
        assert_eq!(
            m[0].raw,
            "curl -fsSL https://example.com/install.sh | sudo bash"
        );
    }

    #[test]
    fn continuation_capped_at_max_lines() {
        // 12 continuation lines — should stop at MAX_CONTINUATION_LINES.
        let mut text = String::from("$ echo \\");
        for _ in 0..12 {
            text.push_str("\n  hello \\");
        }
        text.push_str("\n  final");
        let m = extract(&text, &def());
        assert_eq!(m.len(), 1);
        // Cap means not all 12 lines are spliced.
        let backslash_count = m[0].raw.matches('\\').count();
        // There should still be backslashes in raw because we stopped early.
        assert!(backslash_count > 0);
    }

    #[test]
    fn prompt_wins_over_exec_on_same_line() {
        let m = extract("❯ sudo apt install foo", &def());
        // Only one match, prompt-anchored.
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "sudo apt install foo");
    }

    #[test]
    fn no_match_in_random_prose() {
        let m = extract("the quick brown fox jumps over the lazy dog", &def());
        assert!(m.is_empty());
    }

    #[test]
    fn rejects_trigger_inside_filename() {
        // `sh` inside `install.sh` must NOT trigger the command pattern —
        // the trigger is preceded by `.`, signaling a file extension.
        let m = extract("Downloaded install.sh from the mirror", &def());
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn rejects_trigger_inside_path() {
        // `sh` inside `/usr/bin/sh foo` is preceded by `/` — path
        // component, not a command word. NOTE: shells DO invoke
        // /bin/sh via the full path, and our trigger list has it
        // explicitly, so this is about the bare `sh` at the END of an
        // arbitrary path, not the literal /bin/sh form.
        let m = extract("path/to/sh detected", &def());
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn still_triggers_after_space() {
        let m = extract("Run sh -c 'foo' please", &def());
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("sh"));
    }

    #[test]
    fn zellij_exec_anchored_in_output() {
        // `[dry-run]` is not a prompt — exec-anchored must catch zellij.
        let m = extract("[dry-run] zellij --session claude-chats --layout cfdefault.kdl", &def());
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("zellij --session"));
    }

    #[test]
    fn tmux_exec_anchored() {
        let m = extract("running: tmux new-session -s main", &def());
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("tmux new-session"));
    }

    // ---- flag-anchored tests ----

    #[test]
    fn flag_anchored_bracket_prefix() {
        let m = extract_flag_anchored("[dry-run] zellij --session foo --layout bar.kdl", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "zellij --session foo --layout bar.kdl");
    }

    #[test]
    fn flag_anchored_colon_prefix() {
        let m = extract_flag_anchored("output: cargo build --release --target wasm32", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "cargo build --release --target wasm32");
    }

    #[test]
    fn flag_anchored_no_prefix() {
        let m = extract_flag_anchored("rsync -avz src/ dest/", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "rsync -avz src/ dest/");
    }

    #[test]
    fn flag_anchored_rejects_uppercase_start() {
        let m = extract_flag_anchored("The --verbose flag was deprecated", &def());
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn flag_anchored_rejects_flag_first() {
        let m = extract_flag_anchored("--option value", &def());
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn flag_anchored_skips_prompt_lines() {
        // Prompt-anchored handles these; flag-anchored must not produce a
        // second match with a different (shorter) raw value.
        assert!(extract_flag_anchored("❯ cargo build --release", &def()).is_empty());
        assert!(extract_flag_anchored("$ git push --force-with-lease", &def()).is_empty());
    }

    #[test]
    fn flag_anchored_short_flag() {
        let m = extract_flag_anchored("[info] ssh -i ~/.ssh/id_ed25519 user@host", &def());
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("ssh -i"));
    }

    #[test]
    fn flag_anchored_dry_run_inner_dash_not_a_flag() {
        // `dry-run` contains `-run` but it is inside brackets and preceded
        // by `y` (not whitespace) — must not be treated as a flag start.
        // The real flag `--session` should anchor the match instead.
        let m = extract_flag_anchored("[dry-run] zellij --session foo", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "zellij --session foo");
    }

    #[test]
    fn flag_anchored_via_extract_with_config() {
        use crate::config::schema::{CommandPatternConfig, PatternsConfig};
        let patterns = PatternsConfig {
            command: CommandPatternConfig {
                flag_anchored: true,
                ..CommandPatternConfig::default()
            },
            ..PatternsConfig::default()
        };
        let text = "[dry-run] zellij --session foo --layout bar.kdl";
        let matches = crate::extract::extract(text, &patterns);
        let cmds: Vec<_> = matches
            .iter()
            .filter(|m| m.ty == crate::extract::MatchType::Command)
            .collect();
        assert!(!cmds.is_empty());
        assert!(cmds.iter().any(|m| m.raw.starts_with("zellij --session")));
    }

    #[test]
    fn min_length_and_alpha_filter() {
        // Too short.
        assert!(extract("❯ ls", &def()).is_empty());
        // No alphabetic chars — fish right-prompt timestamp on an empty prompt.
        assert!(
            extract("❯                                                   18:48:12", &def()).is_empty(),
            "empty prompt with timestamp should not match"
        );
        assert!(
            extract("❯                                                   18:48:49", &def()).is_empty(),
            "second empty prompt with timestamp should not match"
        );
        // Real commands still match.
        assert!(!extract("❯ git status", &def()).is_empty());
        assert!(!extract("❯ cat /tmp/test", &def()).is_empty());
    }

    // ---- rprompt / trailing-whitespace trim tests ----

    #[test]
    fn prompt_anchored_strips_rprompt() {
        // Fish/zsh right-prompt: timestamp pushed to the right edge.
        let m = extract("❯ git status                                        18:48:12", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "git status");
    }

    #[test]
    fn prompt_anchored_strips_rprompt_dollar() {
        let m = extract("$ cargo build --release                             10:23:45", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "cargo build --release");
    }

    #[test]
    fn exec_anchored_strips_rprompt() {
        let m = extract("running: tmux new-session -s main                   18:48:12", &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "tmux new-session -s main");
    }

    #[test]
    fn prompt_anchored_continuation_with_rprompt_on_first_line() {
        // The `\` sits before the rprompt gap — splice must still fire.
        let text = "$ curl https://example.com \\                        18:48:12\n    | jq .";
        let m = extract(text, &def());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "curl https://example.com | jq .");
    }

    #[test]
    fn flag_anchored_strips_rprompt() {
        use crate::config::schema::{CommandPatternConfig, PatternsConfig};
        let patterns = PatternsConfig {
            command: CommandPatternConfig {
                flag_anchored: true,
                ..CommandPatternConfig::default()
            },
            ..PatternsConfig::default()
        };
        let text = "output: cargo build --release                       10:23:45";
        let matches = crate::extract::extract(text, &patterns);
        let cmds: Vec<_> = matches
            .iter()
            .filter(|m| m.ty == crate::extract::MatchType::Command)
            .collect();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].raw, "cargo build --release");
    }

    #[test]
    fn flag_anchored_off_by_default() {
        use crate::config::schema::PatternsConfig;
        // With default config (flag_anchored false), zellij IS in triggers
        // so exec-anchored still catches it — but flag-anchored path is off.
        assert!(!PatternsConfig::default().command.flag_anchored);
    }

    #[test]
    fn rprompt_min_spaces_default_is_five() {
        assert_eq!(CommandPatternConfig::default().rprompt_min_spaces, 5);
    }

    #[test]
    fn rprompt_custom_threshold_lower() {
        // With min_spaces=2, two spaces already truncate.
        let cfg = CommandPatternConfig {
            rprompt_min_spaces: 2,
            ..CommandPatternConfig::default()
        };
        let m = extract("❯ git status  extra", &cfg);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "git status");
    }

    #[test]
    fn rprompt_custom_threshold_higher() {
        // With min_spaces=10, four spaces are NOT a cut point.
        let cfg = CommandPatternConfig {
            rprompt_min_spaces: 10,
            ..CommandPatternConfig::default()
        };
        // Four spaces between words — should NOT be trimmed with threshold 10.
        let m = extract("❯ git log    --oneline", &cfg);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "git log    --oneline");
    }
}
