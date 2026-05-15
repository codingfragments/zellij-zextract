//! zextract — Zellij plugin for typed scrollback extraction.
//!
//! Phase 1 scope: URL-only picker. No fuzzy filter, no modal flow, no
//! config file. Arrow to select, Enter copies to clipboard, Esc closes.
//! See planning.md Phase 1 for acceptance criteria.

mod extract;
mod render;
mod source_pane;

use std::collections::BTreeMap;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget};
use zellij_tile::prelude::*;

use crate::extract::{Match, MatchType};

/// Hardcoded grab cap for Phase 1. Becomes configurable in Phase 7.
const RECENT_LINES: usize = 150;

#[derive(Default)]
struct State {
    matches: Vec<Match>,
    list_state: ListState,
    source_pane: Option<u32>,
    extraction_done: bool,
    last_message: Option<String>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::Key,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
        ]);
        self.list_state.select(Some(0));
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(_) => {
                // Trigger extraction once we have permissions AND a source pane.
                self.try_extract();
                true
            }
            Event::PaneUpdate(manifest) => {
                let new_source = source_pane::pick(&manifest);
                let changed = new_source.is_some() && self.source_pane != new_source;
                if changed {
                    self.source_pane = new_source;
                }
                if !self.extraction_done {
                    self.try_extract();
                    return true;
                }
                changed
            }
            Event::Key(key) => self.handle_key(key),
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let area = Rect {
            x: 0,
            y: 0,
            width: cols as u16,
            height: rows as u16,
        };
        let mut buf = Buffer::empty(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(area);

        self.render_header(chunks[0], &mut buf);
        self.render_list(chunks[1], &mut buf);
        self.render_footer(chunks[2], &mut buf);

        render::flush(&buf);
    }
}

impl State {
    fn try_extract(&mut self) {
        if self.extraction_done {
            return;
        }
        let Some(source) = self.source_pane else { return };
        // Phase 1 default: grab full scrollback, then cap to RECENT_LINES.
        // Phase 8 wires this to a user-toggleable grab mode (Ctrl-g).
        let Ok(contents) = get_pane_scrollback(PaneId::Terminal(source), true) else {
            // Pane not ready yet; retry on next PaneUpdate.
            return;
        };
        let mut all = String::new();
        for line in contents.lines_above_viewport.iter().chain(contents.viewport.iter()) {
            all.push_str(line);
            all.push('\n');
        }
        let trimmed = extract::take_recent(&all, RECENT_LINES);
        self.matches = extract::extract(&trimmed);
        self.extraction_done = true;
        if self.list_state.selected().is_none() && !self.matches.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        match key.bare_key {
            BareKey::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                if i > 0 {
                    self.list_state.select(Some(i - 1));
                }
                true
            }
            BareKey::Down => {
                let i = self.list_state.selected().unwrap_or(0);
                if !self.matches.is_empty() && i + 1 < self.matches.len() {
                    self.list_state.select(Some(i + 1));
                }
                true
            }
            BareKey::Enter => {
                if let Some(i) = self.list_state.selected() {
                    if let Some(m) = self.matches.get(i) {
                        copy_to_clipboard(&m.raw);
                        self.last_message = Some(format!("copied: {}", m.raw));
                        close_self();
                    }
                }
                false
            }
            BareKey::Esc => {
                close_self();
                false
            }
            _ => false,
        }
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let count = self.matches.len();
        let line = Line::from(vec![
            Span::styled(
                "zextract",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(format!("{count} matches")),
        ]);
        Paragraph::new(line)
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        if self.matches.is_empty() {
            let msg = if self.extraction_done {
                "No URLs in pane scrollback."
            } else {
                "Extracting..."
            };
            Paragraph::new(msg)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL))
                .render(area, buf);
            return;
        }

        let items: Vec<ListItem> = self
            .matches
            .iter()
            .map(|m| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("[{}]  ", m.ty.tag()),
                        Style::default().fg(type_color(m.ty)),
                    ),
                    Span::raw(m.display.clone()),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        StatefulWidget::render(list, area, buf, &mut self.list_state);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = vec![
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" navigate  ·  "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" copy  ·  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" close"),
        ];
        if let Some(msg) = &self.last_message {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Green),
            ));
        }
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
    }
}

fn type_color(ty: MatchType) -> Color {
    match ty {
        MatchType::Url => Color::Blue,
    }
}
