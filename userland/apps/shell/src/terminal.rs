//! The Terminal app: a shell on the handheld's own screen. The same screen
//! runs an external app that asked for a terminal.
//!
//! The login shell outlives the screen: going Home and coming back finds it
//! where it was left, with whatever it printed in the meantime. An app's
//! program ends when its screen is left.

use std::cell::RefCell;
use std::path::PathBuf;

use bezel::{Align, Color, Rect, TextStyle, Tone};

use crate::gfx::{Gfx, H, W};
use crate::platform::Key;
use crate::pty::Session;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::term::{Term, DEFAULT};

thread_local! {
    /// The login shell's session, kept while the Terminal app is closed.
    static KEPT: RefCell<Option<(Session, Term)>> = const { RefCell::new(None) };
}

pub struct Terminal {
    title: String,
    /// The program to run instead of the login shell: a command line, and the
    /// folder to run it from.
    program: Option<(String, PathBuf)>,
    /// Or a serial port to talk to, and its speed.
    serial: Option<(String, u32)>,
    /// A dim line put at the top before the session's own words: what it
    /// is (the command, the host, the port), the title being short.
    banner: Option<String>,
    /// What is running, and the screen it has drawn. The emulator stays after
    /// the program ends, so its last words can still be read.
    session: Option<Session>,
    term: Option<Term>,
    /// Lines scrolled back into the history.
    back: usize,
    /// The program has exited; the next key leaves.
    ended: bool,
    greeted: bool,
    /// When the program last printed: while it does, the screen stays awake.
    printed: Option<std::time::Instant>,
}

/// How long after a program last printed the screen may go off.
const AWAKE_AFTER_OUTPUT: std::time::Duration = std::time::Duration::from_secs(60);

impl Terminal {
    /// The login shell, picked up where it was left.
    pub fn new() -> Self {
        let (session, term) = KEPT.with_borrow_mut(Option::take).map_or((None, None), |(s, t)| (Some(s), Some(t)));
        Terminal { title: "Terminal".into(), program: None, serial: None, banner: None, session, term, back: 0, ended: false, greeted: false, printed: None }
    }

    /// The same, with a dim first line saying what it is.
    pub fn with_banner(mut self, banner: &str) -> Self {
        self.banner = Some(banner.to_owned());
        self
    }

    /// One program in a terminal of its own: an external app, an SSH
    /// session, a saved command.
    pub fn running(title: &str, command: &str, dir: PathBuf) -> Self {
        Terminal {
            title: title.to_owned(),
            program: Some((command.to_owned(), dir)),
            serial: None,
            banner: None,
            session: None,
            term: None,
            back: 0,
            ended: false,
            greeted: false,
            printed: None,
        }
    }

    /// A serial port at `baud`, shown as a terminal: what the device sends,
    /// and the keys typed to it. Ctrl-Q (or Esc held) leaves.
    pub fn serial(title: &str, path: &str, baud: u32) -> Self {
        Terminal {
            title: title.to_owned(),
            program: None,
            serial: Some((path.to_owned(), baud)),
            banner: None,
            session: None,
            term: None,
            back: 0,
            ended: false,
            greeted: false,
            printed: None,
        }
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

impl Drop for Terminal {
    fn drop(&mut self) {
        // The login shell waits for the next visit; an app's program goes
        // with its screen (dropping the session hangs up on it).
        if self.program.is_none() && self.serial.is_none() && !self.ended {
            if let (Some(session), Some(term)) = (self.session.take(), self.term.take()) {
                KEPT.with_borrow_mut(|kept| *kept = Some((session, term)));
            }
        }
    }
}

/// The 16 terminal colours, in keeping with the palette in use.
fn ansi(g: &Gfx, index: u8) -> Color {
    let t = &g.tokens;
    let dark = t.tone == Tone::Dark;
    let pick = |on_dark: u32, on_light: u32| Color::hex(if dark { on_dark } else { on_light });
    match index % 16 {
        0 => t.surface3,
        // Red is red, whatever the accent.
        1 | 9 => g.fault(),
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
        &self.title
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("ctrl-q returns home")
    }

    fn busy(&self) -> bool {
        // The program may print at any time; look often while the screen is open.
        !self.ended
    }

    fn keeps_awake(&self) -> bool {
        // An app's program, while it runs; the login shell, while what runs in
        // it is printing (a build, a log being followed), not at a quiet prompt.
        !self.ended && (self.program.is_some() || self.printed.is_some_and(|at| at.elapsed() < AWAKE_AFTER_OUTPUT))
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        // Esc belongs to the programs in the terminal, so leaving is Ctrl-Q,
        // or Esc held down.
        if self.ended || key.leaves() {
            return Nav::Pop;
        }
        let bytes: Vec<u8> = match key {
            Key::PageUp | Key::PageDown => {
                if let Some(term) = &self.term {
                    let page = term.rows.saturating_sub(1).max(1);
                    self.back = if key == Key::PageUp { (self.back + page).min(term.history()) } else { self.back.saturating_sub(page) };
                }
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
            Key::Leave => return Nav::Pop,
        };
        // Typing returns to the live screen.
        self.back = 0;
        if let Some(session) = &mut self.session {
            session.write(&bytes);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let (Some(session), Some(term)) = (&mut self.session, &mut self.term) else {
            // Not started yet (the first draw does that): ask for that draw once.
            return !std::mem::replace(&mut self.greeted, true);
        };
        let bytes = session.read();
        let mut changed = !bytes.is_empty();
        if changed {
            self.printed = Some(std::time::Instant::now());
        }
        term.feed(&bytes);
        let replies = term.take_replies();
        if !replies.is_empty() {
            session.write(&replies);
        }
        if !session.alive() {
            // Whatever it printed last stays on screen; the next key leaves.
            self.ended = true;
            self.session = None;
            changed = true;
        }
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (cols, rows, cell_w, cell_h, panel) = Self::layout(g);
        g.surface(panel, 10.0, t.surface1);
        let style = g.ty.caption;
        if self.term.is_none() && !self.ended {
            let mut term = Term::new(cols, rows);
            if let Some(banner) = &self.banner {
                term.feed(format!("\x1b[2m{banner}\x1b[0m\r\n").as_bytes());
            }
            self.session = match (&self.program, &self.serial) {
                (Some((command, dir)), _) => Some(Session::run(cols, rows, command, dir)),
                (None, Some((path, baud))) => match Session::serial(path, *baud) {
                    Ok(port) => Some(port),
                    // Said where the device's words would have been; any key leaves.
                    Err(why) => {
                        term.feed(format!("{why}\r\n").as_bytes());
                        self.ended = true;
                        None
                    }
                },
                (None, None) => Some(Session::start(cols, rows)),
            };
            self.term = Some(term);
        }
        let Some(term) = &mut self.term else { return };
        if (term.cols, term.rows) != (cols, rows) {
            term.resize(cols, rows);
            if let Some(session) = &mut self.session {
                session.resize(cols, rows);
            }
        }
        let back = self.back.min(term.history());
        let (x0, y0) = (panel.x + 4.0, panel.y + 3.0);
        let bold = TextStyle { wght: 700.0, ..style };
        let mut utf8 = [0u8; 4];
        for row in 0..rows {
            let cy = y0 + (row as f32 + 0.5) * cell_h;
            for (col, cell) in term.line(row, back).iter().enumerate().take(cols) {
                let x = x0 + col as f32 * cell_w;
                let is_cursor = back == 0 && !self.ended && term.cursor_visible && (col, row) == term.cursor;
                // Reverse video swaps ink and ground; the cursor is a block in the signal colour.
                let (mut ink, mut ground) =
                    (if cell.fg == DEFAULT { t.ink } else { ansi(g, cell.fg) }, if cell.bg == DEFAULT { None } else { Some(ansi(g, cell.bg)) });
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
        // A note in the corner: how far back the view is, or that the program is done.
        let note = if self.ended {
            Some("ended · any key returns".to_owned())
        } else if back > 0 {
            Some(format!("{back} lines back · page down"))
        } else {
            None
        };
        if let Some(note) = note {
            let w = g.measure(&note, style) + 14.0;
            let chip = Rect::new(panel.right() - w - 4.0, panel.bottom() - cell_h - 6.0, w, cell_h + 3.0);
            g.surface(chip, chip.h * 0.5, t.surface3);
            g.text_at(chip.cx(), chip.cy(), Align::Center, &note, style, t.ink);
        }
    }
}
