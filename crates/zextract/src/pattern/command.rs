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

use crate::extract::{Match, MatchType};

const MAX_CONTINUATION_LINES: usize = 10;

/// Default prompt markers. Configurable via KDL in Phase 7.
const PROMPT_MARKERS: &[&str] = &["❯ ", "$ ", "> ", "% ", "# "];

/// Default trigger list. Configurable via KDL in Phase 7. Order doesn't
/// matter — we collapse to leftmost-longest in code.
const TRIGGERS: &[&str] = &[
    // Install / package managers
    "sudo", "apt", "apt-get", "yum", "dnf", "pacman", "brew", "snap",
    "pip", "pip3", "pipx", "gem", "cargo", "go", "npm", "yarn", "pnpm",
    "bun", "uv", "poetry", "conda", "mamba",
    // Fetch
    "curl", "wget", "fetch",
    // Shell exec
    "sh", "bash", "zsh", "fish", "/bin/sh", "/bin/bash",
    // Build
    "make", "cmake", "ninja", "just", "nix", "nix-shell", "nix-build",
    // Editor / pager / IO
    "nvim", "vim", "nano", "emacs", "less", "more", "cat", "tee",
    "xargs", "awk", "sed", "grep", "find",
    // VCS
    "git", "hg", "svn",
    // Containers / orchestration
    "docker", "podman", "kubectl", "helm",
    // Language runners
    "python", "python3", "node", "deno", "ruby", "rustc", "java", "mvn", "gradle",
    // File ops
    "tar", "gunzip", "unzip", "chmod", "chown", "ln", "mkdir", "rm", "cp", "mv",
    "ssh", "scp", "rsync",
];

/// Patterns we strip from the start of a continuation line during
/// splicing. Order: most specific first.
const CONTINUATION_STRIP: &[&str] = &[
    r"^\s*\d+[:\.]?\s+",  // line numbers ("  42  ", "2: ", "2. ")
    r"^[+\-]\s+",          // diff add/remove markers
    r"^[#>|]\s+",          // comment / quote / table-cell markers
    r"^\s+",               // leading whitespace (catchall)
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

pub fn extract(text: &str) -> Vec<Match> {
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
            if !cmd_after_prompt.trim().is_empty() {
                let (full_cmd, context, lines_consumed) =
                    splice_continuation(&lines, i, cmd_after_prompt);
                if !full_cmd.trim().is_empty() {
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
            let trimmed = cmd.trim_end();
            if !trimmed.is_empty() {
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
            b'|' | b';' | b'&' | b'(' | b'[' | b'{' | b'`' | b'$' | b'='
            | b'>' | b'<' | b'"' | b'\'' | b':' | b','
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
fn splice_continuation(lines: &[&str], start_idx: usize, first_cmd: &str) -> (String, String, usize) {
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

fn make_match(raw: String, context: String, span_start: usize, span_end: usize) -> Match {
    let mut fields = HashMap::new();
    fields.insert("match".to_string(), raw.clone());
    Match {
        ty: MatchType::Command,
        raw: raw.clone(),
        display: raw,
        context,
        label: None, span: (span_start, span_end),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_anchored_simple() {
        let m = extract("$ git log --oneline -n 20");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "git log --oneline -n 20");
    }

    #[test]
    fn prompt_anchored_unicode() {
        let m = extract("❯ cargo build --release");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "cargo build --release");
    }

    #[test]
    fn exec_anchored_in_prose() {
        let m = extract("To install run sudo apt install zellij from the README.");
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("sudo apt install zellij"));
    }

    #[test]
    fn exec_anchored_pipeline_kept_together() {
        let m = extract("curl -fsSL https://example.com/install.sh | sudo bash");
        assert_eq!(m.len(), 1);
        // Full pipeline captured as one match.
        assert!(m[0].raw.contains("curl"));
        assert!(m[0].raw.contains("| sudo bash"));
    }

    #[test]
    fn continuation_splicing_basic() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n    | sudo bash";
        let m = extract(text);
        assert_eq!(m.len(), 1);
        // The trailing backslash and leading whitespace on line 2 are stripped.
        assert_eq!(m[0].raw, "curl -fsSL https://example.com/install.sh | sudo bash");
    }

    #[test]
    fn continuation_strips_line_number_prefix() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n2:  | sudo bash";
        let m = extract(text);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "curl -fsSL https://example.com/install.sh | sudo bash");
    }

    #[test]
    fn continuation_strips_diff_marker() {
        let text = "$ curl -fsSL https://example.com/install.sh \\\n+   | sudo bash";
        let m = extract(text);
        assert_eq!(m[0].raw, "curl -fsSL https://example.com/install.sh | sudo bash");
    }

    #[test]
    fn continuation_capped_at_max_lines() {
        // 12 continuation lines — should stop at MAX_CONTINUATION_LINES.
        let mut text = String::from("$ echo \\");
        for _ in 0..12 {
            text.push_str("\n  hello \\");
        }
        text.push_str("\n  final");
        let m = extract(&text);
        assert_eq!(m.len(), 1);
        // Cap means not all 12 lines are spliced.
        let backslash_count = m[0].raw.matches('\\').count();
        // There should still be backslashes in raw because we stopped early.
        assert!(backslash_count > 0);
    }

    #[test]
    fn prompt_wins_over_exec_on_same_line() {
        let m = extract("❯ sudo apt install foo");
        // Only one match, prompt-anchored.
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].raw, "sudo apt install foo");
    }

    #[test]
    fn no_match_in_random_prose() {
        let m = extract("the quick brown fox jumps over the lazy dog");
        assert!(m.is_empty());
    }

    #[test]
    fn rejects_trigger_inside_filename() {
        // `sh` inside `install.sh` must NOT trigger the command pattern —
        // the trigger is preceded by `.`, signaling a file extension.
        let m = extract("Downloaded install.sh from the mirror");
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn rejects_trigger_inside_path() {
        // `sh` inside `/usr/bin/sh foo` is preceded by `/` — path
        // component, not a command word. NOTE: shells DO invoke
        // /bin/sh via the full path, and our trigger list has it
        // explicitly, so this is about the bare `sh` at the END of an
        // arbitrary path, not the literal /bin/sh form.
        let m = extract("path/to/sh detected");
        assert!(m.is_empty(), "false positive: {m:?}");
    }

    #[test]
    fn still_triggers_after_space() {
        let m = extract("Run sh -c 'foo' please");
        assert_eq!(m.len(), 1);
        assert!(m[0].raw.starts_with("sh"));
    }
}
