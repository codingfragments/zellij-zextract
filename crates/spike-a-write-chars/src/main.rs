//! Spike A — verify that `write_chars_to_pane_id` delivers bytes from a
//! plugin pane to a sibling shell pane (the load-bearing "insert-back"
//! mechanic). See planning.md Phase 0 for the test matrix.

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

#[derive(Default)]
struct State {
    plugin_id: u32,
    source_pane: Option<u32>,
    permissions_granted: bool,
    log: Vec<String>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::WriteToStdin,
        ]);
        subscribe(&[
            EventType::Key,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
        ]);
        self.plugin_id = get_plugin_ids().plugin_id;
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(result) => {
                self.permissions_granted = matches!(result, PermissionStatus::Granted);
                self.log.push(format!("permissions: {:?}", result));
                true
            }
            Event::PaneUpdate(manifest) => {
                // Pick a sibling terminal pane: focused if available, else the
                // first non-plugin pane we see in any tab. Our own plugin pane
                // ends up focused after launch, so the "focused" check usually
                // misses — the fallback (first non-plugin) is what does the work
                // in single-shell sessions.
                let mut focused_non_plugin = None;
                let mut first_non_plugin = None;
                for (_tab, panes) in &manifest.panes {
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
                let chosen = focused_non_plugin.or(first_non_plugin);
                if chosen.is_some() && self.source_pane != chosen {
                    self.source_pane = chosen;
                    self.log.push(format!("source pane = {:?}", chosen));
                }
                true
            }
            Event::Key(key) => self.handle_key(key),
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        print!("\x1b[2J\x1b[H");
        println!("Spike A — write_chars_to_pane_id\r");
        println!("\r");
        println!("permissions: {}\r", self.permissions_granted);
        println!("source pane: {:?}\r", self.source_pane);
        println!("\r");
        println!("Press a number to fire a test payload at the source pane:\r");
        println!("  [1] basic ASCII:     echo hello world\r");
        println!("  [2] special chars:   echo 'q' \"d\" `b` $v\r");
        println!("  [3] multi-line via \\n (intentional embedded newline)\r");
        println!("  [4] common command:  ls -la\r");
        println!("  [5] empty string (no-op test)\r");
        println!("  [6] long string (500 ASCII chars)\r");
        println!("\r");
        println!("[Esc] close\r");
        println!("\r");
        println!("Log (newest first):\r");
        for line in self.log.iter().rev().take(12) {
            println!("  {}\r", line);
        }
    }
}

impl State {
    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Char('1') => {
                self.fire("echo hello world");
                true
            }
            BareKey::Char('2') => {
                self.fire("echo 'q' \"d\" `b` $v");
                true
            }
            BareKey::Char('3') => {
                self.fire("echo a\necho b");
                true
            }
            BareKey::Char('4') => {
                self.fire("ls -la");
                true
            }
            BareKey::Char('5') => {
                self.fire("");
                true
            }
            BareKey::Char('6') => {
                let long: String = "x".repeat(500);
                self.fire(&long);
                true
            }
            BareKey::Esc => {
                close_self();
                false
            }
            _ => false,
        }
    }

    fn fire(&mut self, payload: &str) {
        let Some(pane_id) = self.source_pane else {
            self.log.push("FAIL: no source pane known".into());
            return;
        };
        write_chars_to_pane_id(payload, PaneId::Terminal(pane_id));
        self.log
            .push(format!("wrote {} bytes -> pane {}", payload.len(), pane_id));
    }
}
