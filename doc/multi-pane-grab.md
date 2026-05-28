# Multi-pane grab (`source "tab"`)

zextract normally grabs scrollback from the single pane you launched from.
The `source "tab"` grab profile variant extends that to every non-floating,
non-plugin pane on the active tab, so one keypress can search across an
entire layout.

## How to enable it

Add a `tab-scan` profile to your `~/.config/zellij/zextract.kdl`:

```kdl
grab {
    default_profile "quick"
    profiles {
        quick {
            source "scrollback"
            lines 150
        }
        deep {
            source "scrollback"
            lines 1500
        }
        viewport {
            source "viewport"
        }
        full {
            source "scrollback"
        }
        tab-scan {
            source "tab"
            lines 150
        }
    }
}
```

Then bind a key that launches zextract with `grab "tab-scan"`.
Shift-F (`"F"` in zellij keybind syntax) is a natural choice:

```kdl
// In your zellij config, inside the relevant mode block:
bind "F" {
    LaunchOrFocusPlugin "file:~/.config/zellij/plugins/zextract.wasm" {
        grab "tab-scan"
        floating true; move_to_focused_tab true;
    }
}
```

The built-in default profile list already includes `tab-scan` (150 lines
per pane), so if you have not overridden `profiles {}` in your config you
can reference it from a keybind without adding anything to your config file.

## Behaviour

### Pane selection

All non-floating, non-plugin, non-suppressed panes on the active tab are
included. Floating panes are excluded — they are transient overlays and
their scrollback is rarely what you want to search.

### Ordering

The last-focused pane's matches appear first in the picker. Remaining panes
follow in left-to-right layout order (by `pane_x`, then `pane_y`). This
mirrors the spatial order visible on screen.

### Line cap

The `lines N` setting in the profile applies **per pane**. With four panes
and `lines 150` you get up to 600 lines total. This matches the intuition
that `lines 150` means "150 recent lines of history" regardless of how many
panes are open.

### Pane-title prefix

When more than one pane contributes matches, each row in the picker gains a
dim `[title]  ` prefix before the type tag:

```
[editor]    [file]  src/main.rs:42
[terminal]  [url]   https://example.com
[editor]    [cmd]   cargo build --release
```

Titles are end-truncated at 15 characters. Panes without a title show as
`[pane N]` where N is the pane's numeric id. In single-pane mode the prefix
is omitted entirely.

### Deduplication

The same value appearing in two different panes shows up twice — once for
each pane, with its own title prefix. Per-pane within-type and cross-type
deduplication still runs as usual (most recent occurrence wins within a
single pane's text).

### Actions

All actions behave the same as in single-pane mode. Insert (`i` / `I` /
Shift-Enter) always writes to the **last-focused pane** — the one you
launched from — regardless of which pane the selected match came from.
Copy, JSON export, open, edit, and reveal are also unaffected.

### Grab-profile cycling

Alt-g (or `g` in List mode) cycles through all configured profiles as
normal. If you cycle from `tab-scan` to `quick`, zextract re-extracts from
the single source pane only. Cycling back to `tab-scan` re-grabs all panes.

### Failure handling

If a pane's scrollback cannot be fetched (the pane closed between the
PaneUpdate snapshot and the grab call), it is skipped silently. A Debug-
level log entry is written. If every pane fails, the picker shows the
standard "No matches" empty state.

## Design decisions

| Decision | Choice | Reason |
|---|---|---|
| Insert target | Last-focused pane | Preserves the "pull into my prompt" contract |
| Pane ordering | Last-focused first, then left-to-right | Spatial and predictable |
| Lines cap | Per-pane | Consistent with single-pane mental model |
| Dedup across panes | None | Title prefix disambiguates; dedup needs a definition |
| Prefix width | 15 chars, end-truncated | Balances readability and list width |
| Title fallback | `pane <id>` | Unambiguous, stable within session |
| Floating panes | Excluded | Transient overlays; unlikely to have useful history |
| `tab-viewport` variant | Deferred | No known use case yet |
