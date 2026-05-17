# Source-pane race condition — analysis and fix

## The symptom

When opening zextract in a multi-pane Zellij window, the picker
sometimes showed results from the wrong pane — often a background
pane that contained only fish shell startup output, with none of the
actual terminal history the user wanted to search.  The wrong pane was
selected non-deterministically: the same layout and the same keybind
would sometimes pick the right pane and sometimes not.

---

## Why it is a race condition

### How source-pane selection worked before the fix

zextract needs to know which terminal pane to read scrollback from
before it can extract anything.  That information comes from
`PaneUpdate` events, which carry a `PaneManifest` — a snapshot of
every pane in the session.  Each `PaneInfo` entry has an `is_focused`
flag.

The old `source_pane::pick()` logic was:

```
focused_non_plugin  →  first_non_plugin
```

1. Walk every non-plugin pane in the manifest.
2. If one has `is_focused = true`, use it.
3. Otherwise, return the **first** non-plugin pane encountered while
   iterating the manifest's internal `HashMap`.

Step 3 is the problem.

### Why a HashMap iteration is non-deterministic

`PaneManifest.panes` is a `HashMap<usize, Vec<PaneInfo>>` (tab index
→ panes in that tab).  HashMap iteration order in Rust is not defined
and changes with every program run (hash seed is randomised).  With
two or more non-plugin panes, the "first" pane returned by step 3
could be any of them — and there is no way to predict which one.

### When step 3 actually fires

Step 3 fires whenever **no non-plugin pane is currently focused**.
That is exactly what happens the moment zextract's floating pane
opens: it steals focus from the terminal pane.  Zellij sends a
`PaneUpdate` reflecting the new state — the plugin pane is now
`is_focused = true`, all terminal panes are `is_focused = false` —
and step 3 kicks in.

Because the plugin did not exist before the keybind was pressed, it
has no history of prior `PaneUpdate` events.  On its very first event
it falls into step 3, and the result is random.

### The event-ordering constraint

There is no dedicated `PaneFocused` event in Zellij 0.44.x.  The only
pane-related events available are `PaneUpdate` and `PaneClosed`.
`PaneUpdate` is emitted *after* the focus change has been applied by
the host, so the first snapshot the plugin ever sees already shows the
plugin itself as focused and the terminal as unfocused.

---

## The fix

### Core mechanism: `last_focused_non_plugin`

A new field was added to `State`:

```rust
last_focused_non_plugin: Option<u32>,
```

On every `PaneUpdate`, before calling `pick()`, the handler scans the
manifest for a focused non-plugin pane and records its ID:

```rust
for panes in manifest.panes.values() {
    for pane in panes {
        if !pane.is_plugin && pane.is_focused {
            self.last_focused_non_plugin = Some(pane.id);
        }
    }
}
```

This is then passed to `source_pane::pick()` as a hint.

### Updated pick() priority order

`pick()` now has four tiers:

| Tier | Condition | What it means |
|------|-----------|---------------|
| 1 | `is_focused && !is_plugin` in current manifest | Plugin opened before focus fully transferred — rare but clean |
| 2 | `last_focused_non_plugin` hint, still exists in manifest | The pane the user was in immediately before the plugin opened |
| 3 | First non-floating, non-suppressed non-plugin pane | Best-effort cold-start heuristic; avoids background/hidden panes |
| 4 | Any non-plugin pane | Last resort |

Tier 2 is the fix for the common case.  Tier 3 replaces the old
random HashMap walk with a structural preference: tiled, visible panes
are far more likely to be the user's active terminal than floating or
suppressed ones.

### Why this works for a persistent plugin

The fix only gives a reliable result when the plugin has already been
running in the background — i.e., when it is re-used via
`LaunchOrFocusPlugin` rather than created fresh on each invocation.

While the plugin runs invisibly, it keeps receiving `PaneUpdate`
events on every focus change.  Each time the user switches to a
terminal pane, `last_focused_non_plugin` is updated.  By the time the
keybind is pressed, that field already holds the ID of the pane the
user was just in.  The moment the plugin steals focus and tier 1 fails,
tier 2 immediately provides the correct answer.

---

## Remaining low-probability race: first-ever cold start

On the very first invocation of a session — before any prior
`PaneUpdate` has been received — `last_focused_non_plugin` is `None`
and tier 2 cannot fire.  The fallback is tier 3 (first tiled pane),
which is better than the old random pick but still not guaranteed to
be correct with multiple tiled panes.

This only happens once per session, at first launch.  Every subsequent
use of the plugin works correctly via tier 2.

---

## Eliminating the cold-start race with `load_plugins`

The cold-start race exists because the plugin has no history.  The
solution is to give it history by loading it at session start.

Zellij layouts support a `load_plugins` block that initialises plugins
before any user interaction.  The plugin starts, calls `load()`,
subscribes to `PaneUpdate`, and begins accumulating
`last_focused_non_plugin` from the very first pane focus in the
session.  By the time the user presses the keybind for the first time,
tier 2 is already populated.

### Layout configuration

Add a `load_plugins` block to your layout file
(e.g. `~/.config/zellij/layouts/default.kdl`):

```kdl
layout {
    load_plugins {
        plugin location="file:~/.config/zellij/plugins/zextract.wasm" {
            // optional: set any default plugin config here
        }
    }

    // rest of your layout
    pane
    // ...
}
```

### Using a plugin alias (recommended)

If you reference the plugin in both `load_plugins` and a keybind, you
would normally have to repeat the full path in both places.  A plugin
alias avoids this and keeps the path as a single source of truth.

Define the alias in your Zellij config (`~/.config/zellij/config.kdl`):

```kdl
plugins {
    zextract location="file:~/.config/zellij/plugins/zextract.wasm"
}
```

Then reference it by alias everywhere:

```kdl
// In your layout file:
layout {
    load_plugins {
        plugin location="zextract"
    }
    pane
}

// In your keybind config:
keybinds {
    normal {
        bind "Alt u" {
            LaunchOrFocusPlugin "zextract" {
                floating true
                move_to_focused_tab true
            }
        }
    }
}
```

This ensures both `load_plugins` and the keybind refer to the same
plugin instance.  Zellij uses `LaunchOrFocus` semantics: the keybind
brings the existing background instance to the foreground rather than
creating a new one, which is what preserves the `last_focused_non_plugin`
history.

### Permission dialog on first session start

When the plugin is loaded in the background for the first time, it
calls `request_permission()` but has no visible pane to show the
dialog in.  The permission prompt will not appear until the plugin is
first surfaced via the keybind.  After the user accepts once, Zellij
caches the grant and every subsequent session starts silently with
full permissions already active.

---

## Debugging source-pane selection

The plugin logs a `Debug`-level line on every `PaneUpdate`:

```
[zextract] PaneUpdate: last_focused_hint=Some(3) picked=Some(3) current=None
```

| Field | Meaning |
|-------|---------|
| `last_focused_hint` | Stored hint before this update — `None` on cold start |
| `picked` | Pane ID chosen by `pick()` |
| `current` | `source_pane` before this update |

Until the user's config file is loaded, the plugin logs at `Debug`
level by default so these early events are visible.  Once the config
is applied, the log level switches to whatever `log_level` is set to
in `~/.config/zellij/zextract.kdl`.

Log output goes to Zellij's session log file.  Find the most recent
one and tail it:

```fish
set log (ls -t ~/.local/share/zellij/ | head -1)
tail -f ~/.local/share/zellij/$log/zellij.log | grep zextract
```

A correctly working session after the fix looks like:

```
# Plugin starts, no history yet (cold start):
PaneUpdate: last_focused_hint=None  picked=Some(2)  current=None

# User works in terminal pane 2, then switches to pane 3:
PaneUpdate: last_focused_hint=Some(2)  picked=Some(3)  current=Some(2)

# User triggers keybind — plugin steals focus, pane 3 is now unfocused:
PaneUpdate: last_focused_hint=Some(3)  picked=Some(3)  current=Some(3)
#                                       ^^^^^^^^^^^^^ tier-2 hint fires
```
