# Changelog

All notable changes to zextract are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps for breaking changes).

---

## [Unreleased]

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

[Unreleased]: https://github.com/codingfragments/zellij-zextract/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/codingfragments/zellij-zextract/releases/tag/v0.1.0
