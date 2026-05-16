//! Spike B — verify that ratatui's widgets + Buffer system render correctly
//! inside a Zellij WASI plugin. Crossterm doesn't compile for wasm32-wasip1,
//! so we skip ratatui's Backend layer entirely: render widgets into a Buffer,
//! then walk the Buffer and emit ANSI escapes ourselves.
//!
//! What this verifies:
//!   - ratatui's Layout solver works in WASI.
//!   - Widget rendering into a Buffer works.
//!   - Unicode glyphs land at correct column positions.
//!   - Our Buffer→ANSI flush is correct enough to look right.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
};
use zellij_tile::prelude::*;

struct State {
    items: Vec<String>,
    list_state: ListState,
    last_size: (u16, u16),
    frames_rendered: u64,
}

impl Default for State {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            items: (1..=30)
                .map(|i| format!("Item {i:02} — unicode test ❯ ● ▸ ✓ ⚠ — width-sensitive glyphs"))
                .collect(),
            list_state,
            last_size: (0, 0),
            frames_rendered: 0,
        }
    }
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        subscribe(&[EventType::Key]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => match key.bare_key {
                BareKey::Up => {
                    let i = self.list_state.selected().unwrap_or(0);
                    if i > 0 {
                        self.list_state.select(Some(i - 1));
                    }
                    true
                }
                BareKey::Down => {
                    let i = self.list_state.selected().unwrap_or(0);
                    if i + 1 < self.items.len() {
                        self.list_state.select(Some(i + 1));
                    }
                    true
                }
                BareKey::PageUp => {
                    let i = self.list_state.selected().unwrap_or(0);
                    self.list_state.select(Some(i.saturating_sub(10)));
                    true
                }
                BareKey::PageDown => {
                    let i = self.list_state.selected().unwrap_or(0);
                    self.list_state
                        .select(Some((i + 10).min(self.items.len() - 1)));
                    true
                }
                BareKey::Esc => {
                    close_self();
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.last_size = (rows as u16, cols as u16);
        self.frames_rendered += 1;

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

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                "Spike B — ratatui (no crossterm) in WASI plugin",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  ·  frame #{}  ·  size {}x{}",
                self.frames_rendered, self.last_size.0, self.last_size.1
            )),
        ]))
        .block(Block::default().borders(Borders::ALL).title("header"));
        Widget::render(header, chunks[0], &mut buf);

        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|s| ListItem::new(s.as_str()))
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("list (↑/↓ ; PgUp/PgDn for ±10)"),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        StatefulWidget::render(list, chunks[1], &mut buf, &mut self.list_state);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" navigate · "),
            Span::styled("PgUp/PgDn", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" page · "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" close"),
        ]))
        .block(Block::default().borders(Borders::ALL).title("footer"));
        Widget::render(footer, chunks[2], &mut buf);

        flush_buffer_to_stdout(&buf);
    }
}

/// Walk the Buffer and emit ANSI to stdout. We re-emit style on every cell
/// where the style differs from the previous cell (simple, no diffing).
/// Cursor positioning per row keeps us synced even if the previous frame
/// printed less.
fn flush_buffer_to_stdout(buf: &Buffer) {
    let area = buf.area();
    let mut out = String::with_capacity(area.width as usize * area.height as usize * 4);
    // home cursor; do NOT clear screen — Zellij handles full re-render per frame
    out.push_str("\x1b[H");
    let mut last_style: Option<Style> = None;
    for y in 0..area.height {
        // jump to start of row y
        let _ = write!(out, "\x1b[{};1H", y + 1);
        for x in 0..area.width {
            let cell = match buf.cell((x, y)) {
                Some(c) => c,
                None => continue,
            };
            let style = cell.style();
            if last_style != Some(style) {
                out.push_str("\x1b[0m");
                emit_style(&mut out, style);
                last_style = Some(style);
            }
            out.push_str(cell.symbol());
        }
    }
    out.push_str("\x1b[0m");
    print!("{out}");
}

fn emit_style(out: &mut String, s: Style) {
    if let Some(fg) = s.fg {
        emit_color(out, fg, false);
    }
    if let Some(bg) = s.bg {
        emit_color(out, bg, true);
    }
    let m = s.add_modifier;
    if m.contains(Modifier::BOLD) {
        out.push_str("\x1b[1m");
    }
    if m.contains(Modifier::DIM) {
        out.push_str("\x1b[2m");
    }
    if m.contains(Modifier::ITALIC) {
        out.push_str("\x1b[3m");
    }
    if m.contains(Modifier::UNDERLINED) {
        out.push_str("\x1b[4m");
    }
    if m.contains(Modifier::REVERSED) {
        out.push_str("\x1b[7m");
    }
}

fn emit_color(out: &mut String, c: Color, bg: bool) {
    let (base_3bit, base_bright, base_256, base_rgb) = if bg {
        (40, 100, "\x1b[48;5;", "\x1b[48;2;")
    } else {
        (30, 90, "\x1b[38;5;", "\x1b[38;2;")
    };
    match c {
        Color::Reset => out.push_str(if bg { "\x1b[49m" } else { "\x1b[39m" }),
        Color::Black => {
            let _ = write!(out, "\x1b[{}m", base_3bit);
        }
        Color::Red => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 1);
        }
        Color::Green => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 2);
        }
        Color::Yellow => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 3);
        }
        Color::Blue => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 4);
        }
        Color::Magenta => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 5);
        }
        Color::Cyan => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 6);
        }
        Color::Gray => {
            let _ = write!(out, "\x1b[{}m", base_3bit + 7);
        }
        Color::DarkGray => {
            let _ = write!(out, "\x1b[{}m", base_bright);
        }
        Color::LightRed => {
            let _ = write!(out, "\x1b[{}m", base_bright + 1);
        }
        Color::LightGreen => {
            let _ = write!(out, "\x1b[{}m", base_bright + 2);
        }
        Color::LightYellow => {
            let _ = write!(out, "\x1b[{}m", base_bright + 3);
        }
        Color::LightBlue => {
            let _ = write!(out, "\x1b[{}m", base_bright + 4);
        }
        Color::LightMagenta => {
            let _ = write!(out, "\x1b[{}m", base_bright + 5);
        }
        Color::LightCyan => {
            let _ = write!(out, "\x1b[{}m", base_bright + 6);
        }
        Color::White => {
            let _ = write!(out, "\x1b[{}m", base_bright + 7);
        }
        Color::Indexed(i) => {
            let _ = write!(out, "{}{}m", base_256, i);
        }
        Color::Rgb(r, g, b) => {
            let _ = write!(out, "{}{};{};{}m", base_rgb, r, g, b);
        }
    }
}
