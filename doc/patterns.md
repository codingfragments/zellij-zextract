# Built-in pattern reference

zextract scans every scrollback line against all enabled patterns and
presents the combined results in the fuzzy picker. Each built-in pattern
is described below: what it matches, how it detects matches, and what
`raw` value (used for copy / insert / dedup) it produces.

For the precision / false-positive trade-off see
[README — Pattern detection and false positives](../README.md#pattern-detection-and-false-positives).

---

## `url` — URLs

**Tag:** `#url`  
**Default action:** open in browser

Matches any string that starts with a recognised scheme followed by `://`
and at least one non-whitespace character.

Recognised schemes: `http`, `https`, `ftp`, `ftps`, `ssh`, `git`, `svn`,
`file`, `mailto`, `irc`, `ircs`, `slack`, `vscode`.

Trailing punctuation that is unlikely to be part of the URL (`.`, `,`,
`)`, `]`, `>`, `'`, `"`, `;`) is stripped from the right end.

**Examples matched:**

```
https://github.com/owner/repo/pull/42
http://localhost:3000/api/v1/users
git://git.example.com/myrepo.git
file:///home/user/.config/zellij/config.kdl
```

---

## `file` — File paths

**Tag:** `#file`  
**Default action:** open in editor

Matches paths that contain at least one `/`. Both absolute paths
(`/home/user/foo`) and relative paths (`src/main.rs`, `./config`) are
matched. A bare filename without a separator (e.g. `Cargo.toml`) does
**not** match — the slash requirement keeps false positives low.

An optional `:line` or `:line:col` suffix is captured and stored in the
`{line}` template variable; the suffix is stripped from the display path.

Trailing punctuation (`,`, `)`, `>`, `"`, `'`) is stripped. Pure-numeric
segments and URLs (already caught by the `url` pattern) are excluded.

**Examples matched:**

```
/home/user/.config/zellij/config.kdl
src/main.rs:42:8
./scripts/install.sh
crates/zextract/src/pattern/command.rs
```

---

## `diag` — Compiler / linter diagnostics

**Tag:** `#diag`  
**Default action:** open in editor at the matched line

Matches file-path + line references that appear in compiler or linter
output. Two forms are recognised:

- **Colon form:** `path:line` or `path:line:col` — the classic
  `rustc`/`gcc`/`clang`/`eslint` format.
- **Prose form:** `at path line N` — used by Python tracebacks and some
  test runners.

The `{file}` and `{line}` template variables are populated separately so
an editor command like `hx {file}:{line}` works directly.

**Examples matched:**

```
src/main.rs:42:8
crates/zextract/src/extract.rs:140
  File "/home/user/app.py", line 23, in main
```

---

## `sha` — Git commit SHAs

**Tag:** `#sha`  
**Default action:** copy

Matches 7–40 character lowercase hexadecimal strings that look like Git
SHAs. Pure-numeric strings are excluded (e.g. `12345678` does not match).
The string must be at a word boundary on both sides.

**Examples matched:**

```
78bef8d
d7d8438f19a2bc3
a1b2c3d4e5f6789012345678901234567890abcd
```

---

## `ipv4` — IPv4 addresses

**Tag:** `#ipv4`  
**Default action:** copy

Matches standard dotted-quad notation. All four octets must be in the
range 0–255. An optional `:port` suffix (1–65535) is included in the
match.

**Examples matched:**

```
192.168.1.1
10.0.0.1:8080
127.0.0.1
```

---

## `ipv6` — IPv6 addresses

**Tag:** `#ipv6`  
**Default action:** copy  
**Default state:** off — opt in via `types { ipv6 { ... } }`

Matches full and compressed IPv6 notation. Bracketed form with port
(`[::1]:8080`) is also matched.

**Examples matched:**

```
2001:db8::1
::1
[fe80::1%eth0]:443
```

---

## `uuid` — UUIDs

**Tag:** `#uuid`  
**Default action:** copy

Matches both lowercase and uppercase UUIDs in the standard
`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` form (8-4-4-4-12 hex groups
separated by hyphens).

**Examples matched:**

```
123e4567-e89b-12d3-a456-426614174000
A987FBC9-4BED-3078-CF07-9141BA07C9F3
```

---

## `quote` — Quoted strings

**Tag:** `#quote`  
**Default action:** copy

Matches text enclosed in matching pairs of `"`, `'`, or `` ` ``. Empty
strings are excluded. The quotes themselves are not included in the raw
value — only the content between them.

**Examples matched:**

```
"hello world"      →  hello world
'~/.config'        →  ~/.config
`some command`     →  some command
```

---

## `secret` — Secrets and tokens

**Tag:** `#secret`  
**Default action:** copy (open / edit / reveal are always denied)

Two-tier detection:

**Tier 1 — Curated format matchers** (high precision):

| Format | Prefix / shape |
|--------|----------------|
| JWT | `eyJ` … three base64url segments separated by `.` |
| AWS access key | `AKIA` + 16 uppercase alphanumeric chars |
| GitHub token | `gh[pousr]_` + alphanumeric |
| GitLab PAT | `glpat-` + alphanumeric |
| Stripe key | `sk_live_` or `pk_live_` + alphanumeric |
| OpenAI key | `sk-` + 48 alphanumeric chars |
| Anthropic key | `sk-ant-` + alphanumeric |
| Slack token | `xox[bpaso]-` + alphanumeric |
| Bearer token | `Bearer ` + 20+ non-whitespace chars |

**Tier 2 — Entropy fallback** (broader, more false positives):

Strings of 20–200 characters with at least 3 distinct character classes
and ≥ 3.5 bits/character Shannon entropy. Catches API keys and tokens in
formats not covered above. A curated-format match suppresses the entropy
fallback for the same span.

---

## `cmd` — Commands

**Tag:** `#cmd`  
**Default action:** insert into source pane prompt

The most complex pattern. Uses three detection strategies applied in
order per line. A line can produce at most one prompt-anchored match or
one exec-anchored match; flag-anchored runs as a separate additive pass.

### Quality filters applied to all strategies

Before a match is emitted, the candidate text must pass two guards:

- **Minimum length:** 5 characters (after trimming)
- **At least one ASCII letter:** rejects pure-numeric / punctuation
  strings such as the fish right-prompt timestamp (`18:48:12`) that
  bleeds onto otherwise-empty `❯` lines in the terminal scrollback

### Strategy 1 — Prompt-anchored (always active)

A line that starts with one of the recognised prompt markers is treated
as a command line. The marker is stripped and the rest of the line
becomes the command text.

**Recognised markers:** `❯ ` · `$ ` · `> ` · `% ` · `# `

Multi-line commands joined with a trailing `\` are spliced together: the
continuation line's leading noise (line numbers, diff markers `+`/`-`,
comment prefixes `#`/`>`/`|`, leading whitespace) is stripped and the
lines are joined with a single space. Up to 10 continuation lines are
spliced.

**Examples:**

```
❯ git log --oneline -n 20         →  git log --oneline -n 20
$ cargo build --release           →  cargo build --release
❯ curl -fsSL https://example.com \
    | sudo bash                   →  curl -fsSL https://example.com | sudo bash
```

### Strategy 2 — Exec-anchored (always active)

Scans the line for the leftmost occurrence of a known trigger word at a
command-start position (preceded by whitespace, line start, or a shell
operator). Captures from the trigger to end-of-line.

**Command-start preceding bytes:** whitespace, `|`, `;`, `&`, `(`, `[`,
`{`, `` ` ``, `$`, `=`, `>`, `<`, `"`, `'`, `:`, `,`

**Full trigger list (80 entries):**

| Group | Triggers |
|-------|----------|
| Package managers | `sudo` `apt` `apt-get` `yum` `dnf` `pacman` `brew` `snap` `pip` `pip3` `pipx` `gem` `cargo` `go` `npm` `yarn` `pnpm` `bun` `uv` `poetry` `conda` `mamba` |
| Fetch | `curl` `wget` `fetch` |
| Shell exec | `sh` `bash` `zsh` `fish` `/bin/sh` `/bin/bash` |
| Build | `make` `cmake` `ninja` `just` `nix` `nix-shell` `nix-build` |
| Editor / pager / IO | `nvim` `vim` `nano` `emacs` `less` `more` `cat` `tee` `xargs` `awk` `sed` `grep` `find` |
| VCS | `git` `hg` `svn` |
| Containers / multiplexers | `docker` `podman` `kubectl` `helm` `zellij` `tmux` |
| Language runners | `python` `python3` `node` `deno` `ruby` `rustc` `java` `mvn` `gradle` |
| File ops | `tar` `gunzip` `unzip` `chmod` `chown` `ln` `mkdir` `rm` `cp` `mv` `ssh` `scp` `rsync` |

No continuation splicing — capturing to end-of-line is safer in prose
contexts where continuation backslashes may not be shell continuations.

**Examples:**

```
To install run: sudo apt install zellij  →  sudo apt install zellij
[dry-run] zellij --session foo           →  zellij --session foo
Running curl -fsSL https://example.com  →  curl -fsSL https://example.com
```

### Strategy 3 — Flag-anchored (opt-in)

**Disabled by default.** Enable with:

```kdl
patterns {
    command {
        flag_anchored true
    }
}
```

Scans for the leftmost standalone `-x`, `-xyz` (combined short flags), or
`--long-flag` argument on the line. "Standalone" means the byte before
the `-` is whitespace, `(`, `&`, `|`, `;`, or `=` — flags inside
compound words like `dry-run` or `some-file` are not counted.

Once a flag is found, the algorithm walks backward through the prefix to
the nearest boundary character (`][}{><:;|&(,'"`), then skips any
leading whitespace to locate the command word.

**Guards against false positives:**

| Guard | What it rejects |
|-------|-----------------|
| First char must be `[a-z]` | `The --verbose flag` (uppercase start), `❯` non-ASCII prompt chars, `--option value` (flag-first lines) |
| First word must be ≥ 2 chars | Single-letter noise |
| Prompt-anchored lines are skipped | Avoids producing a second match alongside the prompt-anchored result |

**Examples:**

```
[dry-run] rsync -avz src/ dest/          →  rsync -avz src/ dest/
output: cargo build --release --target   →  cargo build --release --target
[info] ssh -i ~/.ssh/id_ed25519 user@h   →  ssh -i ~/.ssh/id_ed25519 user@host
```

**Known false-positive categories with flag-anchored enabled:**

- Lowercase-starting prose before a flag: `"missing argument -v"` → matches
  `missing argument -v` (no uppercase guard triggers)
- Log lines with lowercase prefixes: `"note: --edition 2024"` → boundary
  `:` leaves `note` as the command word

These are acceptable for an opt-in feature. Use `#cmd` type filter to
view only command matches, or `#!cmd` to exclude them when the noise is
too high for a particular session.

### Deduplication

All three strategies feed into the shared dedup pipeline:

1. Same `(type, raw)` → keep only the latest occurrence (most recent in
   scrollback)
2. Same `raw` across types → keep the type with the highest priority
   (`cmd` ranks below `url`, `file`, `diag` — so a git URL captured as
   both a URL and a command keeps only the URL entry)
