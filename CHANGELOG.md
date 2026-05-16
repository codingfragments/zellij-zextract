# Changelog

All notable changes to zextract are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps for breaking changes).

---

## [Unreleased]

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

[Unreleased]: https://github.com/codingfragments/zellij-zextract/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/codingfragments/zellij-zextract/releases/tag/v0.1.0
