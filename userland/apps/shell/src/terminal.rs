//! The Terminal app: a shell on the handheld's own screen.
//!
//! The session outlives the screen: going Home and coming back finds the
//! shell where it was left, with whatever it printed in the meantime.

use std::cell::RefCell;

use bezel::{Align, Color, Rect, TextStyle, Tone};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::pty::Session;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::term::{Term, DEFAULT};

thread_local! {
    /// The one terminal session, kept while the app is closed.
    static SESSION: RefCell<Option<(Session, Term)>> = const { RefCell::new(None) };
}

pub struct Terminal {
    /// Lines scrolled back into the history.
    back: usize,
    /// The shell has exited; the next key leaves.
    ended: bool,
    greeted: bool,
}

impl Terminal {
    pub fn new() -> Self {
        Terminal { back: 0, ended: false, greeted: false }
    }

    /// The grid that fits the panel at the current text size: columns, rows,
    /// and the size of a cell.
    fn layout(g: &mut Gfx) -> (usize, usize, f32, f32, Rect) {
        let style = g.ty.caption;
        let cell_w = g.measure("0", style);
        let cell_h = style.size + 2.0;
        let panel = Rect::new(4.0, g.top() - 3.0, W - 8.0, H - g.top() - 1.0);
        let cols = ((panel.w - 8.0) / cell_w).floor().max(2.0) as usize;
        let rows = ((panel.h - 6.0) / cell_h).floor().max(2.0) as usize;
        (cols, rows, cell_w, cell_h, panel)
    }
}

/// The 16 terminal colours, in keeping with the palette in use.
fn ansi(g: &Gfx, index: u8) -> Color {
    let t = &g.tokens;
    let dark = t.tone == Tone::Dark;
    let pick = |on_dark: u32, on_light: u32| Color::hex(if dark { on_dark } else { on_light });
    match index % 16 {
        0 => t.surface3,
        1 | 9 => t.signal,
        2 | 10 => pick(0x7fd18b, 0x1f8a3d),
        3 | 11 => t.amber_ink,
        4 | 12 => pick(0x7aa7ff, 0x2b5fd9),
        5 | 13 => pick(0xc792ea, 0x8a3fb3),
        6 | 14 => pick(0x7fd6e0, 0x0f7f8c),
        7 => t.dim,
        8 => t.faint,
        _ => t.ink,
    }
}

impl Screen for Terminal {
    fn title(&self) -> &str {
        "Terminal"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("ctrl-q returns home")
    }

    fn busy(&self) -> bool {
        // The shell may print at any time; look often while the screen is open.
        !self.ended
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.ended {
            return Nav::Pop;
        }
        let bytes: Vec<u8> = match key {
            // Esc belongs to the programs in the terminal, so leaving is Ctrl-Q.
            Key::Ctrl('q') => return Nav::Pop,
            Key::PageUp | Key::PageDown => {
                SESSION.with_borrow(|s| {
                    if let Some((_, term)) = s {
                        let page = term.rows.saturating_sub(1).max(1);
                        self.back = if key == Key::PageUp { (self.back + page).min(term.history()) } else { self.back.saturating_sub(page) };
                    }
                });
                return Nav::Stay;
            }
            Key::Char(c) => c.to_string().into_bytes(),
            Key::Ctrl(c) => vec![(c.to_ascii_lowercase() as u8) & 0x1f],
            Key::Enter => vec![b'\r'],
            Key::Backspace => vec![0x7f],
            Key::Tab => vec![b'\t'],
            Key::Back => vec![0x1b],
            Key::Up => b"\x1b[A".to_vec(),
            Key::Down => b"\x1b[B".to_vec(),
            Key::Right => b"\x1b[C".to_vec(),
            Key::Left => b"\x1b[D".to_vec(),
        };
        // Typing returns to the live screen.
        self.back = 0;
        SESSION.with_borrow_mut(|s| {
            if let Some((session, _)) = s {
                session.write(&bytes);
            }
        });
        let _ = cx;
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        SESSION.with_borrow_mut(|slot| {
            let Some((session, term)) = slot else { return !std::mem::replace(&mut self.greeted, true) };
            let bytes = session.read();
            let mut changed = !bytes.is_empty();
            term.feed(&bytes);
            let replies = term.take_replies();
            if !replies.is_empty() {
                session.write(&replies);
            }
            if !self.ended && !session.alive() {
                self.ended = true;
                changed = true;
            }
            if self.ended {
                // The next time the app opens, a new shell starts.
                *slot = None;
            }
            changed
        })
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (cols, rows, cell_w, cell_h, panel) = Self::layout(g);
        g.surface(panel, 10.0, t.surface1);
        let style = g.ty.caption;
        if self.ended {
            g.text_at(panel.cx(), panel.cy(), Align::Center, "the shell has ended · any key returns", style, t.dim);
            return;
        }
        SESSION.with_borrow_mut(|slot| {
            let (session, term) = slot.get_or_insert_with(|| (Session::start(cols, rows), Term::new(cols, rows)));
            if (term.cols, term.rows) != (cols, rows) {
                term.resize(cols, rows);
                session.resize(cols, rows);
            }
            let back = self.back.min(term.history());
            let (x0, y0) = (panel.x + 4.0, panel.y + 3.0);
            let bold = TextStyle { wght: 700.0, ..style };
            let mut utf8 = [0u8; 4];
            for row in 0..rows {
                let cy = y0 + (row as f32 + 0.5) * cell_h;
                for (col, cell) in term.line(row, back).iter().enumerate().take(cols) {
                    let x = x0 + col as f32 * cell_w;
                    let is_cursor = back == 0 && term.cursor_visible && (col, row) == term.cursor;
                    // Reverse video swaps ink and ground; the cursor is a block in the signal colour.
                    let (mut ink, mut ground) = (
                        if cell.fg == DEFAULT { t.ink } else { ansi(g, cell.fg) },
                        if cell.bg == DEFAULT { None } else { Some(ansi(g, cell.bg)) },
                    );
                    if cell.reverse {
                        (ink, ground) = (ground.unwrap_or(t.surface1), Some(ink));
                    }
                    if is_cursor {
                        (ink, ground) = (t.on_signal, Some(t.signal));
                    }
                    if let Some(color) = ground {
                        g.surface(Rect::new(x, cy - cell_h * 0.5, cell_w + 0.5, cell_h), if is_cursor { 2.0 } else { 0.0 }, color);
                    }
                    if cell.ch != ' ' {
                        g.text_at(x, cy, Align::Left, cell.ch.encode_utf8(&mut utf8), if cell.bold { bold } else { style }, ink);
                    }
                }
            }
            // Scrolled back: say how far, where the newest line would be.
            if back > 0 {
                let note = format!("{back} lines back · page down");
                let w = g.measure(&note, style) + 14.0;
                let chip = Rect::new(panel.right() - w - 4.0, panel.bottom() - cell_h - 6.0, w, cell_h + 3.0);
                g.surface(chip, chip.h * 0.5, t.surface3);
                g.text_at(chip.cx(), chip.cy(), Align::Center, &note, style, t.ink);
            }
        });
        let _ = PAD;
    }
}
