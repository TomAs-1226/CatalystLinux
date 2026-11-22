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
    /// Copy mode (Ctrl-Shift-C): the lines picked, by their number in the
    /// terminal's lines (`Term::line_text`).
    copying: Option<Copying>,
}

/// Lines being picked to copy: the one the highlight is on, and the one
/// Space marked, the lines between them picked too.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Copying {
    at: usize,
    mark: Option<usize>,
}

impl Copying {
    fn range(&self) -> (usize, usize) {
        let mark = self.mark.unwrap_or(self.at);
        (mark.min(self.at), mark.max(self.at))
    }
}

/// How long after a program last printed the screen may go off.
const AWAKE_AFTER_OUTPUT: std::time::Duration = std::time::Duration::from_secs(60);

impl Terminal {
    /// The login shell, picked up where it was left.
    pub fn new() -> Self {
        let (session, term) = KEPT.with_borrow_mut(Option::take).map_or((None, None), |(s, t)| (Some(s), Some(t)));
        Terminal { title: "Terminal".into(), program: None, serial: None, banner: None, session, term, back: 0, ended: false, greeted: false, printed: None, copying: None }
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
            copying: None,
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
            copying: None,
        }
    }

    /// For the gallery: a terminal showing `printed` (no program behind it),
    /// in copy mode with the last `lines` lines it printed picked.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing(printed: &str, cols: usize, rows: usize, lines: usize) -> Self {
        let mut t = Terminal::running("Terminal", "true", std::env::temp_dir());
        let mut term = Term::new(cols, rows);
        term.feed(printed.as_bytes());
        t.term = Some(term);
        if lines > 0 {
            t.start_copying();
            if let Some(c) = &mut t.copying {
                c.mark = Some(c.at);
                c.at = c.at.saturating_sub(lines - 1);
            }
        }
        t
    }

    /// The grid the panel holds at the text size in use (for the gallery).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn grid_for(g: &mut Gfx) -> (usize, usize) {
        let (cols, rows, ..) = Self::layout(g);
        (cols, rows)
    }

    /// Copy mode: the last line with words above the cursor (what the last
    /// command printed) is picked first.
    fn start_copying(&mut self) {
        let Some(term) = &self.term else { return };
        let cursor = term.history() + term.cursor.1;
        let at = (0..cursor).rev().take(term.rows).find(|&i| !term.line_text(i).is_empty()).unwrap_or(cursor);
        self.copying = Some(Copying { at, mark: None });
        self.show_line(at);
    }

    /// Scroll so that line `at` is in view.
    fn show_line(&mut self, at: usize) {
        let Some(term) = &self.term else { return };
        let history = term.history();
        // In view are lines history - back … history - back + rows - 1.
        let first = history - self.back.min(history);
        if at < first {
            self.back = history - at;
        } else if at >= first + term.rows {
            self.back = (history + term.rows - 1).saturating_sub(at).min(history);
        }
    }

    /// A key in copy mode: ↑ ↓ (or F, X: nothing types here) move, Space
    /// marks where more lines start, Enter (or Ctrl-Shift-C again) copies,
    /// Esc leaves it.
    fn copy_key(&mut self, key: Key, cx: &mut Cx) {
        let (Some(term), Some(mut c)) = (&self.term, self.copying) else { return };
        let last = term.lines_total() - 1;
        let page = term.rows.saturating_sub(1).max(1);
        match key {
            Key::Up | Key::Char('f') => c.at = c.at.saturating_sub(1),
            Key::Down | Key::Char('x') => c.at = (c.at + 1).min(last),
            Key::PageUp => c.at = c.at.saturating_sub(page),
            Key::PageDown => c.at = (c.at + page).min(last),
            Key::Home => c.at = 0,
            Key::End => c.at = last,
            Key::Char(' ') => c.mark = if c.mark.is_some() { None } else { Some(c.at) },
            Key::Enter | Key::Ctrl('C') => {
                let (from, to) = c.range();
                let lines: Vec<String> = (from..=to).map(|i| term.line_text(i)).collect();
                let text = lines.join("\n");
                self.copying = None;
                self.back = 0;
                if text.trim().is_empty() {
                    *cx.say = Some("nothing on those lines".into());
                } else {
                    let n = to - from + 1;
                    crate::clipboard::copy_quietly(&text, crate::clipboard::Sensitivity::Normal);
                    *cx.say = Some(if n == 1 { "line copied".into() } else { format!("{n} lines copied") });
                }
                return;
            }
            Key::Back | Key::Backspace => {
                self.copying = None;
                self.back = 0;
                return;
            }
            _ => {}
        }
        self.copying = Some(c);
        self.show_line(c.at);
    }

    /// Text into the program, as a terminal pastes it: line breaks as Enter,
    /// other control characters left out (so a paste cannot carry escape
    /// sequences in), and marked as a paste when the program asked
    /// (bracketed paste).
    fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
        let mut clean = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' if chars.peek() == Some(&'\n') => {}
                '\r' | '\n' => clean.push('\r'),
                '\t' => clean.push('\t'),
                c if c.is_control() => {}
                c => clean.push(c),
            }
        }
        if bracketed {
            format!("\x1b[200~{clean}\x1b[201~").into_bytes()
        } else {
            clean.into_bytes()
        }
    }

    /// The grid that fits the panel at the current text size: columns, rows,
    /// and the size of a cell.
    fn layout(g: &mut Gfx) -> (usize, usize, f32, f32, Rect) {
        let style = g.ty.caption;
        let cell_w = g.measure("0", style);
        // Lines a pixel apart (a terminal's own spacing): a row more at
        // every text size than with the page's looser leading.
        let cell_h = style.size + 1.0;
        let panel = Rect::new(4.0, g.top() - 3.0, W - 8.0, H - g.top() - 1.0);
        let cols = ((panel.w - 8.0) / cell_w).floor().max(2.0) as usize;
        let rows = ((panel.h - 4.0) / cell_h).floor().max(2.0) as usize;
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

/// What a function key sends, as xterm does it: F1 to F4 as `ESC O P`…`S`,
/// the rest as numbered sequences (htop's F10 quits, its F9 kills).
fn function_key(n: u8) -> String {
    match n {
        1..=4 => format!("\x1bO{}", (b'P' + n - 1) as char),
        5 => "\x1b[15~".into(),
        6..=10 => format!("\x1b[{}~", 11 + n),
        11 | 12 => format!("\x1b[{}~", 12 + n),
        _ => String::new(),
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

    /// Ctrl-C is the interrupt and Ctrl-V the next key as it is: copying
    /// here is Ctrl-Shift-C (copy mode) and pasting Ctrl-Shift-V.
    fn keeps_ctrl(&self) -> bool {
        true
    }

    /// A paste (Ctrl-Shift-V, or Enter in the clipboard's history) goes to
    /// the program as typed, its lines as Enter.
    fn paste(&mut self, text: &str, _cx: &mut Cx) -> Nav {
        if self.ended {
            return Nav::Stay;
        }
        self.copying = None;
        self.back = 0;
        let bracketed = self.term.as_ref().is_some_and(|t| t.bracketed);
        if let Some(session) = &mut self.session {
            session.write(&Self::paste_bytes(text, bracketed));
        }
        Nav::Stay
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("ctrl-q returns home")
    }

    fn busy(&self) -> bool {
        // The program may print at any time; look often while the screen is open.
        !self.ended
    }

    /// Only the login shell is kept among recent apps (its session was always
    /// kept), and only while it runs: one that has ended is not brought back
    /// saying so, Terminal opens on a new one. A program started in a
    /// terminal of its own (an app, an SSH session, a saved command, a
    /// serial line) ends when it is left, as an app that draws for itself
    /// does: nothing goes on running unseen.
    fn keep_in_recents(&self) -> bool {
        self.program.is_none() && self.serial.is_none() && !self.ended
    }

    fn keeps_awake(&self) -> bool {
        // An app's program, while it runs; the login shell, while what runs in
        // it is printing (a build, a log being followed), not at a quiet prompt.
        !self.ended && (self.program.is_some() || self.printed.is_some_and(|at| at.elapsed() < AWAKE_AFTER_OUTPUT))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        // Esc belongs to the programs in the terminal, so leaving is Ctrl-Q,
        // or Esc held down.
        if self.ended || key.leaves() {
            return Nav::Pop;
        }
        // Copy mode takes the keys while it is on.
        if self.copying.is_some() {
            self.copy_key(key, cx);
            return Nav::Stay;
        }
        match key {
            Key::Ctrl('C') => {
                self.start_copying();
                return Nav::Stay;
            }
            Key::Ctrl('V') => {
                return match crate::clipboard::paste_text() {
                    Some(text) => self.paste(&text, cx),
                    None => {
                        *cx.say = Some("the clipboard is empty".into());
                        Nav::Stay
                    }
                };
            }
            _ => {}
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
            // As xterm sends them, which every program here understands.
            Key::Delete => b"\x1b[3~".to_vec(),
            Key::Home => b"\x1b[H".to_vec(),
            Key::End => b"\x1b[F".to_vec(),
            Key::F(n) => function_key(n).into_bytes(),
            // The shell takes these for the backlight and the sound before any screen sees them.
            Key::BrightnessDown | Key::BrightnessUp | Key::Media(_) | Key::Lock => return Nav::Stay,
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
        // A program put something on the clipboard (OSC 52).
        if let Some(text) = term.take_clip() {
            crate::clipboard::copy(&text, crate::clipboard::Sensitivity::Normal);
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
        // The rows sit in the middle of the panel, as much room above as below.
        let (x0, y0) = (panel.x + 4.0, panel.y + ((panel.h - rows as f32 * cell_h) * 0.5).round());
        let bold = TextStyle { wght: 700.0, ..style };
        let mut utf8 = [0u8; 4];
        // Copy mode: the lines picked under a state layer, the one the
        // highlight is on ringed.
        let first_line = term.history() - back;
        if let Some(c) = self.copying {
            let (from, to) = c.range();
            for row in 0..rows {
                let at = first_line + row;
                if (from..=to).contains(&at) {
                    let r = Rect::new(x0 - 2.0, y0 + row as f32 * cell_h, cols as f32 * cell_w + 4.0, cell_h);
                    g.canvas.rrect(r, 2.0, t.ink, 0.12);
                    if at == c.at {
                        g.canvas.stroke_rrect(r.inset(-1.0), 3.0, 1.5, g.ring_color(), 1.0);
                    }
                }
            }
        }
        for row in 0..rows {
            let cy = y0 + (row as f32 + 0.5) * cell_h;
            for (col, cell) in term.line(row, back).iter().enumerate().take(cols) {
                let x = x0 + col as f32 * cell_w;
                let is_cursor = back == 0 && !self.ended && self.copying.is_none() && term.cursor_visible && (col, row) == term.cursor;
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
        let note = if let Some(c) = self.copying {
            let (from, to) = c.range();
            let n = to - from + 1;
            let picked = if n == 1 { "1 line".to_owned() } else { format!("{n} lines") };
            let choices = [format!("{picked} · ↑↓ · space marks · enter copies"), format!("{picked} · enter copies · esc"), format!("{picked} · enter copies")];
            let room = panel.w - 22.0;
            let fits = choices.iter().find(|s| g.measure(s, style) <= room).unwrap_or(&choices[2]).clone();
            Some(fits)
        } else if self.ended {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_is_typed_with_lines_as_enter_and_no_escapes_and_bracketed_when_asked() {
        assert_eq!(Terminal::paste_bytes("ls -la\r\ncd /data\n", false), b"ls -la\rcd /data\r");
        assert_eq!(Terminal::paste_bytes("a\x1b[201~b\tc", false), b"a[201~b\tc", "nothing can end the paste early");
        assert_eq!(Terminal::paste_bytes("x", true), b"\x1b[200~x\x1b[201~");
    }

    #[test]
    fn copy_mode_picks_the_last_lines_printed_and_copies_them() {
        let mut t = Terminal::running("t", "true", std::env::temp_dir());
        let mut term = Term::new(20, 4);
        term.feed(b"$ ip\r\n10.58.5.2\r\nup 3 days\r\n$ ");
        t.term = Some(term);
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        assert!(t.keeps_ctrl(), "Ctrl-C is the program's");
        t.key(Key::Ctrl('C'), &mut cx);
        assert_eq!(t.copying.map(|c| c.at), Some(2), "on what the last command printed");
        t.key(Key::Char(' '), &mut cx);
        t.key(Key::Char('f'), &mut cx);
        t.key(Key::Enter, &mut cx);
        assert!(t.copying.is_none());
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("10.58.5.2\nup 3 days"));
        assert_eq!(say.as_deref(), Some("2 lines copied"));
        // Esc leaves it with nothing copied.
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        t.key(Key::Ctrl('C'), &mut cx);
        t.key(Key::Back, &mut cx);
        assert!(t.copying.is_none());
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("10.58.5.2\nup 3 days"));
    }

    #[test]
    fn function_keys_send_what_xterm_sends() {
        let sent: Vec<String> = (1..=12).map(function_key).collect();
        assert_eq!(
            sent,
            ["\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~", "\x1b[20~", "\x1b[21~", "\x1b[23~", "\x1b[24~"]
        );
    }

    #[test]
    fn only_the_login_shell_is_kept_among_recent_apps() {
        // A program in a terminal of its own ends when it is left.
        assert!(!Terminal::running("Top", "top", std::env::temp_dir()).keep_in_recents());
        assert!(!Terminal::serial("Serial", "/dev/null", 115200).keep_in_recents());
        // The login shell goes on, as its session always has.
        assert!(Terminal::new().keep_in_recents());
    }
}
