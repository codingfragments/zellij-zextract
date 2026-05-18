# Pane & Content Extraction — Architecture

How zextract finds the right source pane and gets its scrollback, including
the two race conditions that exist, which one is fixed, and which one
deliberately remains.

---

## The core problem

A Zellij plugin opened with `LaunchOrFocusPlugin` steals focus from the
terminal pane. By the time any event handler runs inside the plugin, the
terminal pane is no longer `is_focused` in the `PaneUpdate` manifest. A naive
"find the focused non-plugin pane right now" always comes up empty.

---

## Why the persistent-plugin pattern fixes it

`LaunchOrFocusPlugin` keeps the plugin process alive between invocations
(reusing the existing instance rather than spawning a fresh one). While the
plugin sits in the background it keeps receiving `PaneUpdate` on every focus
change. The fix is to record `last_focused_non_plugin` during those background
events — before the user ever presses the keybind — so the ID is already known
when the picker opens.

**Critical ordering constraint** (was a bug): the hint must be updated *before*
`pick()` is called in the same `PaneUpdate` handler, not after. There is a
brief transitional `PaneUpdate` where the terminal pane still appears as
focused even as the plugin pane is being raised. If you update the hint after
calling `pick()` you lose that window. The current code:

```rust
// Update hint FIRST.
for pane in active_panes {
    if !pane.is_plugin && pane.is_focused {
        self.last_focused_non_plugin = Some(pane.id);
    }
}
// Then pick, which can use the just-recorded hint.
let new_source = source_pane::pick(&manifest, self.last_focused_non_plugin, …);
```

### `source_pane::pick` — four-tier preference

| Priority | Condition | Why |
|---|---|---|
| 1 | `is_focused && !is_plugin` in current manifest | Brief window before focus fully transfers |
| 2 | `last_focused_non_plugin` hint (if pane still exists) | Workhorse for the persistent-plugin pattern |
| 3 | First tiled, non-suppressed, non-plugin pane | Cold start: prefers the main workspace pane |
| 4 | Any non-plugin pane | Last resort |

The hint is validated against the current manifest before use — if the terminal
pane was closed between invocations, the hint is discarded and tier 3/4 takes
over.

**Tab scoping**: `active_tab_index` (updated from `TabUpdate`) restricts both
the hint-recording loop and `pick()` to the user's current tab. Without this
guard, focused panes in background tabs (each tab has one) would pollute the
hint and cause cross-tab source-pane selection in multi-tab sessions.

---

## Content extraction — `try_extract`

Once `source_pane` is set, extraction runs via `try_extract()`:

```
source_pane resolved
        │
        ▼
get_pane_scrollback(PaneId::Terminal(source), want_full)
        │
        ▼
Assemble text:
  Scrollback profiles → lines_above_viewport + viewport
  Viewport profiles   → viewport only
        │
        ▼
Cap to profile.lines (recent N lines from the tail)
        │
        ▼
extract::extract(text, &config.patterns)
        │
        ▼
self.matches  ←  stored
self.captured_text  ←  retained for preview pane
extraction_done = true
```

`want_full = true` only for scrollback-source profiles — fetching
`lines_above_viewport` has a cost and is unnecessary for viewport-only grabs.

The `extraction_done` flag is an idempotency guard: `try_extract` is called
from multiple event paths (`PaneUpdate`, `PermissionRequestResult`,
`HostFolderChanged`) and must only run once per picker invocation. It is reset
to `false` in exactly two places: plugin load/focus and the `Ctrl-g` profile
cycle.

---

## Race condition 1 — config arrives after first extraction (partially fixed)

### What happens

The event order at plugin open is:

```
LaunchOrFocusPlugin keybind fires
    │
    ├── load() → subscribe([PaneUpdate, PermissionRequestResult, …])
    │
    ├── PaneUpdate #1  ← source pane resolved, try_extract() fires
    │                     uses Config::default() — custom patterns,
    │                     grab profile are NOT loaded yet
    │
    ├── PermissionRequestResult
    │       → request_host_change_for_config_load()
    │         (calls change_host_folder($HOME))
    │
    └── HostFolderChanged
            → load_config_from_host()  ← reads zextract.kdl
            → apply_config_after_load()
            → extraction_done = false
            → try_extract()  ← SECOND extraction, now with real config
```

`PaneUpdate` reliably arrives before `HostFolderChanged` (~130 ms gap in
practice). The first `try_extract` therefore always uses defaults.

### The fix

`HostFolderChanged` resets `extraction_done = false` and calls `try_extract()`
again. The second extraction overwrites the first with the user's actual
config. From the user's perspective the picker shows content once the loading
placeholder disappears (gated on `config_loaded = true`), which only happens
after `HostFolderChanged` completes — so the user never sees the defaults-based
results at all.

```rust
Event::HostFolderChanged(_) => {
    self.load_config_from_host();
    self.config_loaded = true;       // unblocks render
    self.extraction_done = false;    // allows re-extraction
    self.try_extract();              // runs with real config
}
```

The `config_loaded` gate in `render()` is the key: it holds the "loading…"
placeholder until this path completes, hiding the transient defaults-based
state entirely.

### What remains: the cold-start double extraction

The first extraction still runs against defaults and is then discarded. This is
intentional: it means `source_pane` is resolved and scrollback fetched early.
If `HostFolderChanged` is slow (disk I/O, NFS home), the plugin is ready to
show results the instant the config lands rather than having to resolve the
source pane at that point.

The cost is one extra `get_pane_scrollback` call per open. Acceptable.

---

## Race condition 2 — Ctrl-W file write (deliberately accepted)

When the user presses `Ctrl-W` on the missing-config banner, the plugin writes
`DEFAULT_CONFIG` to disk. There is a TOCTOU window between the "does the file
exist?" check and the write:

```rust
if std::path::Path::new(path).exists() {
    // abort — already exists
}
// … time passes …
std::fs::write(path, DEFAULT_CONFIG)?;   // could overwrite
```

If the user creates the file externally between those two calls, their file is
overwritten. This is left unfixed because:

1. The window is milliseconds wide and requires concurrent external action.
2. The safer fix (O_CREAT | O_EXCL via `OpenOptions`) is not available through
   the WASI sandbox's `std::fs` surface.
3. The impact (losing a just-created config) is recoverable — the plugin shows
   a parse error or the user can restore from their editor's undo history.

---

## Reuse pattern for similar plugins

If you build another Zellij plugin that needs to act on the previously-focused
pane:

1. **Use `LaunchOrFocusPlugin`, not `LaunchPlugin`** — persistent process is
   required for the hint to accumulate.
2. **Subscribe to `PaneUpdate` and `TabUpdate` from `load()`** — background
   events must flow before the first invocation.
3. **Update hint before pick in the same handler** — do not split the update
   and the decision across two events.
4. **Scope the hint to the active tab** — use `TabUpdate` to track
   `active_tab_index` and filter `PaneUpdate` accordingly.
5. **Gate render on `config_loaded`** — if you have async config loading (any
   filesystem read), block the real render behind a flag. Show a placeholder
   until the flag is set. This hides the double-extraction from the user.
6. **Reset extraction state in the config-loaded handler** — re-run extraction
   with the real settings; discard the defaults-based first pass.
