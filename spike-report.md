# Phase 0 Spike Report

Two throwaway plugins answering load-bearing API questions before we lock the
v1 architecture. See `planning.md` Phase 0 for the test matrix and the
pre-decided fail-pivots for each spike.

Run order: Spike A first (insert-back is the more impactful unknown). If it
fails, Spike B still matters because the renderer choice is independent.

**Status:** Spike B passed all checks. Spike A plugin loads and runs;
behavioral test matrix (rows 1–6 in the table below) still to be marked
by the operator after walking through them.

## Build & install

```sh
just build         # cargo build --release --target wasm32-wasip1
just install       # symlinks both .wasm into ~/.config/zellij/plugins/
```

Add temporary bindings to `~/.config/zellij/config.kdl` inside a mode block
(e.g. `tmux { ... }` or `shared_except "normal" { ... }`). Zellij uses
single-key binds per mode; chord-style `bind "Ctrl s" "2"` is not valid
syntax. Multiple action statements within a bind are semicolon-separated.

```kdl
// Inside your tmux (or other) mode block:
bind "1" {
    LaunchOrFocusPlugin "file:~/.config/zellij/plugins/spike-a.wasm" {
        floating true
    };
    SwitchToMode "locked"
}
bind "2" {
    LaunchOrFocusPlugin "file:~/.config/zellij/plugins/spike-b.wasm" {
        floating true
    };
    SwitchToMode "locked"
}
```

Reload Zellij (`zellij action reload-config` or restart the session) and
grant permissions on first launch.

## Spike A — `write_chars_to_pane_id`

**Goal:** confirm that bytes written from the plugin land at the source
shell's line editor as user input, locally and over SSH, with no auto-execute
and no character corruption.

**Setup:** launch the plugin (`Ctrl-s 1`) from a pane running fish. Inside
the plugin, press a number to fire a test payload. Close the plugin (`Esc`),
return to the source pane, inspect the prompt.

| # | Payload | Scenario | Expected | Actual |
|---|---|---|---|---|
| 1 | `echo hello world` | local fish | text at prompt, no auto-execute | ☐ |
| 1 | `echo hello world` | fish over SSH (`ssh` then test from inside the SSH session) | text at remote prompt, no auto-execute | ☐ |
| 2 | `echo 'q' "d" \`b\` $v` | local fish | special chars preserved literally | ☐ |
| 3 | `echo a\necho b` | local fish | two lines at prompt; **second line NOT auto-executed** | ☐ |
| 4 | `ls -la` | local fish | command at prompt, ready to execute | ☐ |
| 5 | empty string | local fish | no-op, no crash | ☐ |
| 6 | 500-char ASCII | local fish | full string delivered intact | ☐ |
| — | (any) | TUI app in pane (htop) | bytes received without crash | ☐ |

**Pass criteria:** rows 1, 2, 3, 4 all behave as expected. Row 3 is the
critical safety check — if multi-line payloads auto-execute the intermediate
lines, the spec needs to amend the insert action to strip / escape newlines.

**Pivot if FAILED:** drop the `insert` action verb from Phase 4. Plugin ships
as clipboard-only picker. Update `planning.md` Phase 4 scope; update the
allow-lists in the default config (no `insert` entries).

### Findings

- **Zellij version:** 0.44.2
- **OS:** macOS (darwin 25.4.0)
- **Terminal emulator:** (operator to fill — likely WezTerm/Kitty/iTerm2 per setup)
- **Shell:** fish
- **Plugin loads cleanly:** ✓ after the scaffolding fixes (see "Lessons learned" section below)
- **API surface verified:** `request_permission`, `subscribe`,
  `Event::PaneUpdate`, `Event::Key`, `KeyWithModifier { bare_key }`,
  `write_chars_to_pane_id(text: &str, pane_id: PaneId)`,
  `PaneId::Terminal(u32)`, `close_self()`, `get_plugin_ids()`. All
  match the zellij-tile 0.44.3 surface.
- **Source-pane resolution:** `Event::PaneUpdate` arrives reliably; the
  first non-plugin pane in the manifest is a serviceable source-pane
  identifier in single-shell layouts. For multi-pane layouts in v1 we'll
  need a more deliberate "most-recently-focused-non-plugin" tracking
  strategy.
- **Behavioral matrix:** (operator to fill — payloads 1–6 against local
  fish and SSH'd fish, with row 3 as the critical multi-line safety
  check).
- **Decision:** pending behavioral matrix. Plugin-side scaffolding is
  green.

## Spike B — `ratatui` in WASI plugin

**Goal:** confirm ratatui renders inside Zellij's WASI sandbox without
panicking, with correct widths for Unicode glyphs, and reflows on resize.

**Setup:** launch the plugin (`Ctrl-s 2`). Use arrow keys / PgUp / PgDn to
move the selection. Resize the floating pane (via Zellij's resize mode) and
verify reflow. Close with `Esc`.

| Check | Expected | Actual |
|---|---|---|
| Renders without panic | Three bordered boxes (header, list, footer) visible | X |
| Selected row highlight | Blue background, bold white text on cursor row | X |
| Unicode glyphs (`❯ ● ▸ ✓ ⚠`) | Render at correct column position; list rows align | X |
| `↑`/`↓` navigation | Cursor moves one row | X |
| `PgUp`/`PgDn` | Cursor jumps ±10 rows; clamped at ends | X |
| Resize | Layout reflows cleanly, no garbage | X |
| `Esc` closes | Plugin pane gone, source pane refocused | X |
| Binary size | `.wasm` ≤ 1.5 MB stripped+LTO (release profile) | ✓ 926 KB |

**Pass criteria:** all checks pass, no panics in `zellij-log/zellij.log`.

**Pivot if FAILED:** fall back to hand-rolled ANSI with a small `Frame`
abstraction (~300 lines). Layout design is unchanged; only the rendering
implementation swaps. Update `planning.md` Phase 1 TUI framework row.

### Findings

- **Ratatui version used:** 0.29 with `default-features = false` (skip
  `crossterm` — see Lesson #1 below).
- **Binary size (release, stripped+LTO):** 926 KB. Well under the
  1.5 MB budget.
- **Reflow behavior on resize:** ✓ clean reflow; layout solver and
  widgets recompute per frame against the (rows, cols) Zellij hands us
  in the render callback.
- **Panic log:** none. No entries in `~/Library/Caches/.../zellij.log`
  attributable to spike-b after the bin-crate fix.
- **Unicode width:** ✓ list rows align correctly with `❯ ● ▸ ✓ ⚠`
  glyphs; ratatui's Buffer fills cells correctly, our naive
  cell-by-cell ANSI emission preserves column positions.
- **Decision: PASS.** ratatui via `Buffer + Widget::render` with our
  own ANSI emitter is a viable production path. Update `planning.md`
  to reflect this (TUI framework row stays "ratatui", but with the
  caveat that we own the I/O — no Backend, no crossterm).

## Lessons learned during scaffolding

Five distinct issues hit during scaffolding before either spike loaded
successfully. Capturing them here so future phases don't re-discover
them. None was a typo — each is a real WASI-plugin gotcha.

### 1. `crossterm` doesn't compile for `wasm32-wasip1`

**Symptom:** compile-time errors deep inside crossterm — `cannot find
function window_size in module sys`, `not all trait items implemented:
eval`, etc.

**Cause:** crossterm 0.28's `sys` module is gated `#[cfg(unix)]` or
`#[cfg(windows)]`. For target `wasm32-wasip1`, neither matches — `sys`
is empty, but the parent module still tries to call `sys::window_size()`.

**Fix:** `ratatui = { version = "0.29", default-features = false }`.
Skip the `crossterm` feature, skip the Backend abstraction. Render
widgets into a `Buffer`, then walk and emit ANSI yourself
(~100 lines).

**Takeaway:** **the production renderer keeps this pattern** — ratatui
widgets + layout, no Backend.

### 2. zellij-tile version must match the running Zellij minor version

**Symptom:** `failed to load plugin from instance ... could not find
exported function`. Plugin loaded into the host (loader reported
success in ~7 ms), then host-plugin handshake failed.

**Cause:** initially pinned `zellij-tile = "0.42.0"`; the running Zellij
is 0.44.2. The plugin ABI changes across minor versions — the macros
in tile 0.42 emit a function set that the 0.44 host doesn't look for.

**Fix:** `zellij-tile = "0.44.3"` (closest patch to 0.44.2). Pin to the
running Zellij minor; treat zellij-tile as a **deployment dependency**.

**Takeaway:** document the supported Zellij range in the README.
Consider whether to publish multiple compatible `.wasm` artifacts per
release.

### 3. Zellij caches compiled wasm by **path**, not content hash

**Symptom:** after bumping zellij-tile and rebuilding, the same load
error persisted even though the wasm bytes on disk had changed.

**Cause:** Zellij precompiles wasm to wasmtime artifacts at
`~/Library/Caches/org.Zellij-Contributors.Zellij/<session-uuid>/file:<absolute-path>.wasm/<plugin-id>/`.
The cache key is the loaded path. New bytes at the same path → cache
miss is not triggered. Restarting Zellij also doesn't help — the
cache is on disk.

**Fix:** surgical `rm -rf` on cache entries matching the plugin's path.
Captured as a `just clear-cache` recipe so this is one command going
forward.

**Takeaway:** **clearing the on-disk cache is part of the rebuild
dance** until/unless Zellij switches to content-hashed caching.

### 4. Zellij plugins must be **binary** crates, not `cdylib` (the actual root cause)

**Symptom:** "could not find exported function" persisted *after*
version pin + cache clear. The substantive bug; prior fixes were both
necessary but not sufficient.

**Cause:** WASM exports from `[lib] crate-type = ["cdylib"]`:
```
memory, load, pipe, plugin_version, render, update
```
WASM exports from `[[bin]]` (zjstatus and every other working plugin):
```
memory, _start, __main_void, load, pipe, plugin_version, render, update
```
Zellij 0.44's plugin host calls `_start` (the WASI reactor entry
point) at instantiation. `cdylib` doesn't emit `_start` — the Rust
compiler doesn't generate it unless there's a `main()` function
linked into a binary target. `register_plugin!` emits the plugin's
`main()`, but only the bin crate-type wires it through to `_start`.

**Fix:** `[[bin]] name = "..." path = "src/main.rs"` in Cargo.toml,
rename `src/lib.rs` → `src/main.rs`. **Don't** add your own `fn main()` —
`register_plugin!` provides it.

**Takeaway:** **Zellij plugins are binary crates.** The fact that
`register_plugin!` works without a visible `fn main()` makes this
easy to miss. First thing the README plugin-dev section should say.

### 5. `cp -f` rejects same-file-via-symlink

**Symptom:** `cp: ... and ... are identical (not copied)`.

**Cause:** prior `ln -sf` install runs had created the destination as a
symlink to the build output. Switching to `cp -f` saw `src == dest`
through the symlink and refused.

**Fix:** `rm -f $dest` before `cp -f`. Now in the justfile install
recipe.

**Takeaway:** install recipes should be idempotent across symlink/copy
mode switches.

### Pattern across all five

Three of these (1, 4, and arguably 2) are **WASI-specific gotchas in
tooling that mostly assumes native targets**. Zellij's plugin ABI is
not yet a well-documented surface for plugin authors; the contract is
discovered by comparing exports to a working plugin.

## Decision summary

Once both spikes are executed, update `planning.md`'s "Locked architectural
decisions" table if anything changed, and amend Phase 1's scope. Merge this
report on the `phase-0/spikes` PR so the decisions are committed to the
project history.

| Spike | Status | Pivot triggered? | Notes |
|---|---|---|---|
| A — write_chars_to_pane_id | ✓ loads; behavioral matrix pending | — | API surface confirmed; row-by-row payload tests still to be marked |
| B — ratatui in WASI | ✓ PASS | No | ratatui 0.29 with `default-features = false`, custom ANSI flush; 926 KB binary; clean reflow |
