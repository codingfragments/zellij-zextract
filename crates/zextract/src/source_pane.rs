//! Source-pane resolution: which sibling pane did the user launch from?
//!
//! ## Strategy
//!
//! The reliable pattern requires the plugin to run persistently
//! (LaunchOrFocus rather than a fresh Launch each time). While the plugin
//! lives in the background it receives PaneUpdate on every focus change,
//! letting it track the last non-plugin pane that was focused. When the
//! keybind fires and the plugin pane steals focus, the terminal pane
//! becomes unfocused — but we already have its ID.
//!
//! `pick()` implements a four-tier preference:
//!
//!   1. **Currently focused non-plugin pane** — set when the picker is
//!      opened from a terminal (brief window before focus fully transfers).
//!   2. **`hint`** — the caller's `State.last_focused_non_plugin`, which
//!      is the ID seen as focused in the most recent PaneUpdate where a
//!      non-plugin pane had focus. This is the workhorse for the persistent
//!      plugin pattern.
//!   3. **First tiled, non-suppressed non-plugin pane** — best-effort
//!      heuristic for cold-start (first-ever launch): prefers the main
//!      workspace pane over floating/suppressed background panes that may
//!      only have shell startup output.
//!   4. **Any non-plugin pane** — last resort.

use zellij_tile::prelude::PaneManifest;

/// Returns the terminal pane id that should receive insert/copy actions.
///
/// `hint` is `State.last_focused_non_plugin`: the pane ID last seen as
/// focused in a PaneUpdate where a non-plugin pane held focus. Pass `None`
/// on cold start (no prior PaneUpdate received).
pub fn pick(manifest: &PaneManifest, hint: Option<u32>, active_tab: Option<usize>) -> Option<u32> {
    let mut focused_non_plugin: Option<u32> = None;
    let mut hint_exists = false;
    let mut first_tiled: Option<u32> = None; // non-floating, non-suppressed
    let mut first_any: Option<u32> = None;

    // Restrict to the active tab when known; fall back to all tabs so cold-
    // start and single-tab sessions still work (no TabUpdate received yet).
    let tab_panes: Box<dyn Iterator<Item = &Vec<zellij_tile::prelude::PaneInfo>>> = match active_tab
    {
        Some(idx) => match manifest.panes.get(&idx) {
            Some(panes) => Box::new(std::iter::once(panes)),
            None => Box::new(manifest.panes.values()),
        },
        None => Box::new(manifest.panes.values()),
    };

    for panes in tab_panes {
        for pane in panes {
            if pane.is_plugin {
                continue;
            }
            if pane.is_focused {
                focused_non_plugin = Some(pane.id);
            }
            if Some(pane.id) == hint {
                hint_exists = true;
            }
            if first_any.is_none() {
                first_any = Some(pane.id);
            }
            if first_tiled.is_none() && !pane.is_floating && !pane.is_suppressed {
                first_tiled = Some(pane.id);
            }
        }
    }

    focused_non_plugin
        .or(if hint_exists { hint } else { None })
        .or(first_tiled)
        .or(first_any)
}
