# CLAUDE.md — zextract

Zellij WASM plugin that extracts typed matches from the focused pane's
scrollback and presents them in a floating fuzzy-picker with type-aware
actions. Replaces the tmux-ecosystem gap of fingers/extrakto/fzf-links.

This file is the orientation for Claude. The full design plan is in
`planning.md`; the canonical specs live in `~/.config/zellij/` (see below).

## Where to start

1. **Read `planning.md`** — the phased, vertically-sliced build order with
   acceptance criteria per phase.
2. **Read the canonical specs** in `~/.config/zellij/`:
   - `fuzzy_finder_spec.md` — v1 functional spec
   - `extractor_plugin_requirements.md` — broader requirements
   - `gap_analysis_2026-05-14.md` — tmux↔zellij gap inventory
3. **Phase 0 — spikes — must pass before locking architecture.** See
   `planning.md` Phase 0. Do these before writing any pattern or UI code.

## What this plugin does (one paragraph)

User presses a keybind → plugin opens as a floating pane → reads the
previously-focused pane's scrollback (by default capped at the last 150
lines for speed; `Ctrl-g` in the picker cycles to viewport-only or full
buffer) → extracts typed matches (URLs, file paths, commands, secrets, etc.)
using regex patterns → presents them in a fuzzy-filterable picker with
modal Input/List UX → fires type-aware actions (copy, open in browser,
insert back into source pane prompt, JSON export, custom shell commands
via templates) → closes and refocuses source pane.

## Locked architectural decisions

| Area | Decision |
|---|---|
| Scope | v1 = fuzzy picker only; hint-mode deferred |
| Language | Rust, target `wasm32-wasip1` |
| Plugin name | `zextract` |
| Repo layout | Single crate, `justfile`, symlink install into `~/.config/zellij/plugins/` |
| Regex engine | `regex` crate (no lookaround) |
| Scan strategy | Per-pattern scan; extract-once-on-launch, fuzzy-filter on every keystroke |
| Fuzzy library | `nucleo-matcher` (just the scoring crate) |
| TUI framework | `ratatui` (uncharted in Zellij plugins — Phase 0 spike) |
| Config format | KDL, separate file at `~/.config/zellij/zextract.kdl` |
| UX model | Modal: Input ↔ List, Tab switches; Esc always closes |
| Type filter | Inline `#type` syntax in query (`#url`, `#!secret`, `##main` escape) |
| Action model | Hybrid: built-in verbs + `command "..."` escape hatch with `{name}` templates |
| Clipboard | Zellij plugin API (`copy_to_clipboard`), not direct OSC52 |
| Movement keys | Arrow keys only (Ctrl-p/n, Ctrl-j/k NOT bound) |
| Insert keys | `i` (raw) / `I` (display) in List mode |
| Scrollback grab | Three modes — `recent` (default, last 150 lines, configurable) → `viewport` → `full`; `Ctrl-g` cycles |
| Snapshot tests | `insta` |

## Two Tier-0 spikes (GATING — before architecture lock)

Both are throwaway plugins. Both can fail.

**Spike A — `write_chars_to_pane_id` from plugin to sibling pane.**
The insert-back killer feature depends on it. Test matrix: local fish,
local fish over SSH, special chars, multi-line. See `planning.md` Phase 0.
**If A fails:** drop insert action, ship as clipboard-only picker.

**Spike B — `ratatui` rendering inside a Zellij WASI plugin.**
No existing Zellij plugin uses ratatui. **If B fails:** fall back to
hand-rolled ANSI with a small `Frame` abstraction.

Outcomes recorded in `spike-report.md` (not yet created).

## Default pattern set (v1)

On by default: `url`, `file` (+`:line[:col]`), `diagnostic`, `sha`, `ipv4`,
`uuid`, `quoted-string`, `command`, `secret`.

Off by default (opt-in): `ipv6`, `hex` (4+), `number` (4+).

`command` pattern is hybrid: prompt-anchored (`❯ $ > % #` markers) primary,
executable-anchored (curated trigger list: `sudo`, `curl`, `wget`, `cat`,
`tee`, `xargs`, `make`, `git`, `kubectl`, ... see `planning.md` Appendix A
for the full list) fallback. Captures multi-line commands via trailing-`\`
splicing with per-continuation-line prefix stripping (line numbers, diff
markers, comment prefixes); max 10 splice depth.

`secret` pattern is curated formats (JWT, AWS, GitHub, GitLab, Stripe,
OpenAI, Anthropic, Slack, Bearer) + entropy fallback (length 20-200,
3+ char classes, ≥3.5 bits/char Shannon entropy).

## Conventions

- **No emojis in code or docs** unless the user asks. Default off.
- **No multi-paragraph docstrings.** One short line max.
- **No comments explaining WHAT.** Only WHY when non-obvious.
- **No backwards-compatibility shims** — change the code.
- **No emoji in commit messages or PR titles** unless explicit.
- **Tests use `insta` snapshots** for extraction; per-pattern unit tests
  cover boundary cases.
- **Synthetic secrets only** in test fixtures — never check in real tokens.

## Build gotchas (Zellij plugin specifics)

Four WASI-plugin gotchas discovered during Phase 0 — see
`spike-report.md` for the full write-up; the short form here so future
phases don't re-discover them.

1. **Plugins are `[[bin]]`, not `cdylib`.** Only binary crates emit
   `_start` and `__main_void` (the WASI reactor exports the Zellij
   plugin host requires). `register_plugin!` provides `main()`, so
   don't add your own. Symptom of getting this wrong:
   "could not find exported function" at plugin instantiation.
2. **`zellij-tile` minor must match the running Zellij minor.** Plugin
   ABI is not stable across minor versions. If `zellij --version` says
   0.44.2, pin `zellij-tile = "0.44.x"`. Symptom of mismatch: same
   "could not find exported function" error.
3. **Zellij caches compiled wasm by *load path*, not content hash.**
   Rebuilding does not invalidate. After any ABI-affecting change
   (zellij-tile bump, crate-type change), run `just clear-cache` to
   nuke the on-disk wasmtime artifact cache under
   `~/Library/Caches/org.Zellij-Contributors.Zellij/`.
4. **`ratatui` works in WASI only with `default-features = false`.**
   The default `crossterm` backend's `sys` module is `cfg(unix)` /
   `cfg(windows)` only — no `wasm32-wasip1` path. Pattern: ratatui
   widgets render into a `Buffer`, then a custom ANSI emitter
   walks the Buffer and writes to stdout (see
   `crates/spike-b-ratatui/src/main.rs` for the reference
   implementation; ~100 lines for `flush_buffer_to_stdout` +
   `emit_style` + `emit_color`).

## Build / install / dev

(Once Phase 1 is in place.)

```
just build         # cargo build --release --target wasm32-wasip1
just install       # copy target/.../zextract.wasm into ~/.config/zellij/plugins/
just dev           # build + zellij action reload-plugin zextract
just test          # cargo test
just clear-cache   # rm ABI-stale wasmtime artifacts for this project (see gotcha #3)
just clean
```

## Plugin lifecycle (locked)

Plugin is loadable EITHER eagerly (in zellij `load_plugins { ... }`) OR
lazily (just referenced in a keybind — Zellij compiles on first invocation,
caches for the session). Plugin code is agnostic to which.

- `Load` event handler: cheap. Register subscriptions, parse compiled
  defaults, return.
- Heavy work (scrollback fetch, extraction, render) happens on
  `LaunchOrFocus` event when the picker actually opens.

## Permissions requested at plugin init

- `ReadApplicationState` — `get_pane_scrollback()`, `get_focused_pane_info()`
- `ChangeApplicationState` — open/close floating panes, refocus
- `WriteToStdin` — `write_chars_to_pane_id()` for insert
- `RunCommands` — open/edit/reveal actions and custom `command "..."` verbs
- FS read (for `~/.config/zellij/zextract.kdl`)

FS **write** permission requested on-demand (only when user presses
`Ctrl-W` on the bootstrap banner), not at init.

## Things explicitly deferred to v2

- Hint mode (in-place labels next to matches in source pane). Requires
  Zellij core changes for true overlays.
- Tab-completion of `#type` tokens in the query.
- File-content preview (with syntax highlighting). v1 has context-only
  preview from the captured scrollback.
- Configurable built-in keymap (only custom action verbs get user-bound
  keys via KDL in v1).
- `{name?}` optional-with-cleanup template substitution.
- Multi-pane scrollback grab.
- Pattern priority tiers within a single type.

## Memory pointers

Cross-session memory for this project lives at
`~/.claude/projects/-Users-stefan-marx-projekteHome-marxworld-workEnvironment-zellij-extractor/memory/`.
Contains feedback memory on preferred TUI patterns (modal over single-mode)
and config layout (separate files over inline blocks), plus project and
reference memories.
