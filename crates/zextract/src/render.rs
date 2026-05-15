//! Buffer → ANSI emitter for ratatui-rendered frames. Bypasses ratatui's
//! Backend layer entirely; ratatui widgets fill a Buffer, we walk it and
//! emit ANSI to stdout. Crossterm cannot compile for wasm32-wasip1
//! (verified by Phase 0 Spike B); this is the production renderer pattern.

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

/// Walk the given Buffer and emit ANSI to stdout. Re-emits style on every
/// cell where the style differs from the previous cell; uses cursor
/// positioning per row so prior-frame remnants don't leak through.
pub fn flush(buf: &Buffer) {
    let area = buf.area();
    let cap = area.width as usize * area.height as usize * 4;
    let mut out = String::with_capacity(cap);

    out.push_str("\x1b[H"); // home cursor; Zellij re-renders the pane per frame
    let mut last_style: Option<Style> = None;

    for y in 0..area.height {
        let _ = write!(out, "\x1b[{};1H", y + 1);
        for x in 0..area.width {
            let Some(cell) = buf.cell((x, y)) else { continue };
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
