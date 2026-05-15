//! Source-pane resolution: which sibling pane did the user launch from?
//!
//! Strategy for Phase 1 — sufficient for single-shell layouts:
//!   - On every PaneUpdate, pick the most-recently-`is_focused`
//!     non-plugin pane we see; if no focused-non-plugin exists in the
//!     current event (because our own plugin pane is now focused after
//!     launch), fall back to the first non-plugin pane in the manifest.
//!
//! Limitations to address in later phases:
//!   - With multiple non-plugin panes, "first in the manifest" is not
//!     necessarily the one the user launched from. Phase 4 will need
//!     persistent "last-focused-before-us" tracking.

use zellij_tile::prelude::PaneManifest;

/// Returns the terminal pane id that should receive insert/copy actions.
/// `None` if no non-plugin pane is visible (which means we have nothing
/// to operate on — copy still works since clipboard is independent).
pub fn pick(manifest: &PaneManifest) -> Option<u32> {
    let mut focused_non_plugin = None;
    let mut first_non_plugin = None;
    for panes in manifest.panes.values() {
        for pane in panes {
            if pane.is_plugin {
                continue;
            }
            if first_non_plugin.is_none() {
                first_non_plugin = Some(pane.id);
            }
            if pane.is_focused {
                focused_non_plugin = Some(pane.id);
            }
        }
    }
    focused_non_plugin.or(first_non_plugin)
}
