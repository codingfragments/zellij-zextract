# Changelog

All notable changes to zextract are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps for breaking changes).

---

## [Unreleased]

---

## [0.4.0] — 2026-06-10

### Changed
- **Preview fills available height** — the context window around a match now derives from the live panel height instead of a hardcoded 3-line constant. Context lines are split evenly above and below the full match span, so the preview always uses all available vertical space. Multi-line matches are handled correctly; matches larger than the viewport show no padding rather than clipping context. Resize events are handled for free since `area` is passed fresh on every render.

### Fixed
- **Git pattern never fired** — `extract_timed` called every built-in pattern except `git`; git log lines were only matched by the `sha` pattern as a fallback. `MatchType::Git` entries now appear correctly.
- **Git pattern misses pager output** — bat and `less -N` prepend a line-number column (`  1  │ `) to each line in the scrollback. Both the oneline and full-format `commit` regexes now accept an optional `(?:\s*\d+\s*[│|]\s*)?` prefix, and the subject lookahead in full-format mode strips the prefix before checking the 4-space indent. Raw `git log`, `git log --color`, and bat-paged output all produce `MatchType::Git` matches.

---

## [0.3.1] — 2026-06-01

### Added
- **Auto-grow floating pane** — when the rendered area drops below the minimum, the plugin asks Zellij to resize the float to 95% on whichever dimension is too small. One-shot per launch, re-armed on each preview toggle. Fixes the "terminal too small" banner appearing on terminals where the configured float width or Zellij's default float height landed below the floor.

### Changed
- **Minimum render area** — lowered from 60×15 to 60×12. Banner text updated to match.

### Docs
- README now embeds the demo GIF.

---

## [0.3.0] — 2026-05-28

### Added
- **Pattern-chunked progress bar** — slow grab profiles (deep, full) now show a single-row progress bar while patterns run one at a time, so partial results appear while the scan continues.
- **Per-keybind pattern allowlist** — `patterns "url" "ipv4"` in the `LaunchOrFocusPlugin` configuration block restricts the picker to the listed patterns and ignores all `disable` config. Use to wire dedicated single-type keybinds.
- **Per-profile and global pattern disable** — `disabled "ipv6" "hex"` at the top of `patterns { ... }` or inside a grab `profile { ... }` block turns off the listed patterns globally or per profile.
- **Incremental tab extraction** — `source "tab"` now scans panes one-per-timer-tick instead of blocking, surfacing matches progressively. Per-pane timing metrics logged at info.

### Fixed
- Multi-line format in `DEFAULT_CONFIG` grab profiles (caused a parse error when bootstrapped fresh).

### Docs
- `disable` and `patterns` allowlist features documented.
- Inline profile format examples corrected.
- `progress true` uncommented in the deep/full DEFAULT_CONFIG profiles.

### Tests
- Installer and help-output fixtures added for pattern coverage.

---

## [0.2.1] — 2026-05-20

### Added
- **PageUp/PageDown navigation** — works in both Input and List mode, jumps by a viewport-minus-one each tap.

---

## [0.2.0] — 2026-05-19

### Added
- **Multi-pane tab-wide scrollback grab** — `source "tab"` in a grab profile pulls scrollback from every non-floating, non-plugin pane on the active tab, ranked by recency. Per-pane scrollback caps still apply.
- **Flag-anchored command detection** — opt-in pattern that catches commands beginning with a long flag (`--help`, `-v`, …) even when no prompt or executable anchor is visible.
- **Prose-prefix path detection** — file/path matches now survive when prefixed by a phrase like `see file `.
- **Extension-anchored command pass** — commands ending in a known file extension argument get picked up even when the executable trigger list misses them.
- **Inline comment handling** — strips `#` and `//` trailing comments from command matches; adds a `comment_anchored` opt-in for commands that appear inside comments.
- **Trigger list growth** — `zellij` and `tmux` join the exec-anchored trigger list.
- **Secret entropy check** — Shannon-entropy floor of 3.5 bits/char added to the fallback path so high-frequency tokens (UUIDs already covered by their own pattern, hex blobs, long words) stop matching as secrets.
- **Configurable rprompt trim threshold** — default 5 spaces; tunable per profile.
- **Pre-config debug logging** — early events (PaneUpdate, PermissionRequestResult) now log at debug before the config file is read, easing first-load diagnosis.

### Changed
- **Rust toolchain pinned** to 1.94.1 via `rust-toolchain.toml`, with rustfmt + clippy components included.

### Fixed
- **Command pattern hardening** — skip comment lines; don't scan past inline `#`/`//` for triggers; strip rprompt and multi-whitespace trailer; enforce 5-char minimum length; reject matches with zero alphabetic characters.
- **Source pane resolution** — restricted to the active tab; track last-focused non-plugin pane so SSH'd sessions don't grab the wrong scrollback.
- Clippy lints (`collapsible_match`, `collapse_if_else_branches`, missing `Default` derive on `CommandPatternConfig`).

### Docs
- Pane & content extraction architecture write-up.
- Consolidated `docs/` into `doc/`; added built-in patterns reference.
- `flag_anchored` documented in DEFAULT_CONFIG and config reference.
- README gains a pattern-detection / false-positives section.

---

## [0.1.1] — 2026-05-17

### Added
- **Pane title** — floating pane shows `zextract` by default; override per keybind with `popupTitle "My Picker"` in the `LaunchOrFocusPlugin` configuration block (`name` and `title` are consumed by Zellij before reaching the plugin — use `popupTitle`).
- **Status message auto-dismiss** — transient messages (cap exceeded, insert failed, etc.) now clear automatically after 3 seconds via `Event::Timer`, in addition to clearing on the next keypress.
- **Documentation** — `docs/` directory with per-type reference, complete config key reference table, customization guide with worked examples, and use-case walkthroughs.
- **v2 design brief** in `planning.md` — 4 locked decisions + 6 scoped ideas ready for the next cycle.

### Changed
- **File pattern** — bare filenames without a path separator (`Cargo.toml`, `stefan.marx`, `call.json`) no longer match. Requires at least one `/`. Add `./` prefix to force-match.
- **Preview match highlight** — the matched text is now bold + underlined in the type colour within the preview pane context lines.
- **Empty state** — "No URLs in pane scrollback" → "No matches in pane scrollback" with a dim `Try Alt-g to widen the grab depth` hint.
- **Truncation** — URL/file/diag matches middle-truncated in the list; all others end-truncated.
- **Minimum-size guard** — renders "terminal too small (need ≥60×15)" when the pane is too small.
- **Source pane disappears** — yellow warning banner shown when the source pane closes mid-session; copy and JSON export remain available.
- **Footer** — `p:preview-on`/`p:preview-off` simplified to `p:preview`; verb hints dim when selection count exceeds the verb's cap.
- **Bootstrap config** — Ctrl-W now writes a comprehensive commented config covering all sections, defaults, and example customisations.
- **CI** — opts into Node.js 24 for GitHub Actions runners.

### Fixed
- Clippy warnings (`for_kv_map`, `unnecessary_sort_by`, `manual_pattern_char_comparison`, `needless_borrows_for_generic_args`).
- Rustfmt formatting across all crates.

---

## [0.1.0] — 2026-05-16

First public release. Full v1 feature set.

### Added

**Core extraction**
- 10 built-in match types: `url`, `file`, `diag`, `sha`, `ipv4`, `ipv6`, `uuid`, `quote`, `cmd`, `secret`
- File pattern requires at least one path separator — bare names like `Cargo.toml` intentionally excluded to reduce noise
- Cross-pattern dedup: same raw text keeps the highest-priority type
- Recency ordering: latest occurrence in scrollback ranks first

**Picker UI**
- Modal Input ↔ List with `Tab`
- Live fuzzy filter via `nucleo-matcher`
- `#type` inline filter syntax with unique-prefix resolution (`#ur` → `#url`), excludes (`#!secret`), escapes (`##main`)
- Custom pattern names filterable with `#name`
- Preview pane: context lines with match highlighted in type colour (bold + underlined), toggled with `p` / `Ctrl-P`
- Multi-select with `Space`; batch verbs up to per-verb caps
- Grab profile label `[quick]` shown outside the input box; cycles with `g` (List) / `Alt-g` (both modes)

**Actions**
- Copy raw / copy display, insert raw / insert display, open, edit, reveal, JSON export
- `actions { }` config block: full command templates per type with `{editor}`, `{file}`, `{line}`, `{url}`, `{match}`, `{0}`, `{1}`, `{2}`, …
- `{line}` stripping: `:` / `+` / space before an absent line number stripped automatically
- `default` type key as fallback for any unspecified type
- Multi-target edit chains commands with ` && `

**Configuration** (`~/.config/zellij/zextract.kdl`)
- Hand-rolled KDL-subset parser with line/col error messages
- `grab { profiles { ... } default_profile "..." }` — named scrollback depth profiles
- `limits { copy insert open edit reveal json }` — per-verb multi-target caps
- `log_level "off|error|warn|info|debug"` — gates all `[zextract]` stderr output
- `types { url { actions [...] default "..." } }` — per-type verb allow-lists and defaults
- `actions { file { edit command "hx {file}:{line}" } }` — command templates
- `patterns { jira { regex "..." type "url" template "..." } }` — user-defined patterns with capture groups
- `ui { preview "off|auto|always" preview_open_width "90%" ... }`
- Bootstrap banner on first launch; `Ctrl-W` writes default config; parse-error banner with line/col

**Per-keybind overrides** (via Zellij `configuration` map)
- `type "url"` — pre-fill query with `#url`
- `preview "on"|"off"` — force preview state
- `grab "deep"` — start on a named profile

**Custom patterns**
- `regex`, `type`, `template` per pattern
- Capture groups: `{0}` full match, `{1}` group 1 (alias `{match}`), `{2}`, `{3}`, …
- Pattern name used as display label; filterable with `#name`
- Template present → `raw` = expanded result (correct dedup key)

**Polish**
- Middle-truncation for url/file/diag; end-truncation for others
- Minimum-size guard: "terminal too small (need ≥60×15)"
- Source-pane-gone warning banner; copy/JSON remain available
- Empty state: "No matches in pane scrollback" with `Alt-g` hint
- Loading placeholder during async config load

[Unreleased]: https://github.com/codingfragments/zellij-zextract/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/codingfragments/zellij-zextract/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/codingfragments/zellij-zextract/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/codingfragments/zellij-zextract/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/codingfragments/zellij-zextract/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/codingfragments/zellij-zextract/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/codingfragments/zellij-zextract/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/codingfragments/zellij-zextract/releases/tag/v0.1.0
