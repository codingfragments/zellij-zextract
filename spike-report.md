# Phase 0 Spike Report

Two throwaway plugins answering load-bearing API questions before we lock the
v1 architecture. See `planning.md` Phase 0 for the test matrix and the
pre-decided fail-pivots for each spike.

Run order: Spike A first (insert-back is the more impactful unknown). If it
fails, Spike B still matters because the renderer choice is independent.

**Status:** ⏳ pending execution. Fill in each section as tests are run.

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

(Fill in after running. Note Zellij version, OS, terminal emulator,
shell, and any deviations from expected behavior.)

- Zellij version:
- OS:
- Terminal emulator:
- Shell:
- API surface verified:
- Deviations:
- Decision:

## Spike B — `ratatui` in WASI plugin

**Goal:** confirm ratatui renders inside Zellij's WASI sandbox without
panicking, with correct widths for Unicode glyphs, and reflows on resize.

**Setup:** launch the plugin (`Ctrl-s 2`). Use arrow keys / PgUp / PgDn to
move the selection. Resize the floating pane (via Zellij's resize mode) and
verify reflow. Close with `Esc`.

| Check | Expected | Actual |
|---|---|---|
| Renders without panic | Three bordered boxes (header, list, footer) visible | ☐ |
| Selected row highlight | Blue background, bold white text on cursor row | ☐ |
| Unicode glyphs (`❯ ● ▸ ✓ ⚠`) | Render at correct column position; list rows align | ☐ |
| `↑`/`↓` navigation | Cursor moves one row | ☐ |
| `PgUp`/`PgDn` | Cursor jumps ±10 rows; clamped at ends | ☐ |
| Resize | Layout reflows cleanly, no garbage | ☐ |
| `Esc` closes | Plugin pane gone, source pane refocused | ☐ |
| Binary size | `.wasm` ≤ 1.5 MB stripped+LTO (release profile) | ☐ MB |

**Pass criteria:** all checks pass, no panics in `zellij-log/zellij.log`.

**Pivot if FAILED:** fall back to hand-rolled ANSI with a small `Frame`
abstraction (~300 lines). Layout design is unchanged; only the rendering
implementation swaps. Update `planning.md` Phase 1 TUI framework row.

### Findings

(Fill in after running.)

- Ratatui version used:
- Binary size (release):
- Reflow behavior on resize:
- Panic log (if any):
- Decision:

## Decision summary

Once both spikes are executed, update `planning.md`'s "Locked architectural
decisions" table if anything changed, and amend Phase 1's scope. Merge this
report on the `phase-0/spikes` PR so the decisions are committed to the
project history.

| Spike | Status | Pivot triggered? | Notes |
|---|---|---|---|
| A — write_chars_to_pane_id | ⏳ | — | |
| B — ratatui in WASI | ⏳ | — | |
