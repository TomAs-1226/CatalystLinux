//! A terminal emulator: turns the bytes a program prints into a grid of
//! characters. It understands the VT100/xterm subset that shells, editors
//! and the usual tools emit: cursor movement, erasing, colours, scrolling
//! regions, the alternate screen, and the status queries programs ask.
//!
//! It knows nothing about drawing or about processes; `pty` runs the shell
//! and the terminal screen draws the grid.

use std::collections::VecDeque;

/// Lines kept above the screen for scrolling back.
const SCROLLBACK: usize = 400;
/// "No colour set": the terminal's own ink or ground.
pub const DEFAULT: u8 = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cell {
    pub ch: char,
    /// 0–15 are the ANSI colours; `DEFAULT` is none.
    pub fg: u8,
    pub bg: u8,
    pub bold: bool,
    pub reverse: bool,
}

impl Cell {
    const BLANK: Cell = Cell { ch: ' ', fg: DEFAULT, bg: DEFAULT, bold: false, reverse: false };
}

pub struct Term {
    pub cols: usize,
    pub rows: usize,
    grid: Vec<Cell>,
    /// The main screen, while a full-screen program has the alternate one.
    main: Option<(Vec<Cell>, (usize, usize))>,
    scrollback: VecDeque<Vec<Cell>>,
    pub cursor: (usize, usize),
    saved: (usize, usize),
    pen: Cell,
    /// The rows that scroll, first and last.
    region: (usize, usize),
    /// The cursor sits past the last column until the next character wraps it.
    wrap_pending: bool,
    pub cursor_visible: bool,
    parser: vte::Parser,
    /// Answers to queries, to be written back to the program.
    replies: Vec<u8>,
    /// The program asked for pastes marked as such (bracketed paste, `?2004`).
    pub bracketed: bool,
    /// What the program put on the clipboard (OSC 52), not yet taken.
    clip: Option<String>,
}

/// The most an OSC 52 copy may be, as base64: a little over the clipboard's 64 kb.
const OSC52_MOST: usize = 96 * 1024;

impl Term {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(2));
        Term {
            cols,
            rows,
            grid: vec![Cell::BLANK; cols * rows],
            main: None,
            scrollback: VecDeque::new(),
            cursor: (0, 0),
            saved: (0, 0),
            pen: Cell::BLANK,
            region: (0, rows - 1),
            wrap_pending: false,
            cursor_visible: true,
            parser: vte::Parser::new(),
            replies: Vec::new(),
            bracketed: false,
            clip: None,
        }
    }

    /// What the program put on the clipboard (OSC 52), once.
    pub fn take_clip(&mut self) -> Option<String> {
        self.clip.take()
    }

    /// Lines there are, the history's and the screen's: a line's number
    /// counts from the oldest kept.
    pub fn lines_total(&self) -> usize {
        self.scrollback.len() + self.rows
    }

    /// The words of line `at` (counted as `lines_total`), without the
    /// spaces after them.
    pub fn line_text(&self, at: usize) -> String {
        let cells: &[Cell] = if at < self.scrollback.len() {
            &self.scrollback[at]
        } else {
            let y = (at - self.scrollback.len()).min(self.rows - 1);
            &self.grid[y * self.cols..(y + 1) * self.cols]
        };
        cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_owned()
    }

    /// Take in what the program printed.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        parser.advance(self, bytes);
        self.parser = parser;
    }

    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    /// How many lines can be scrolled back to.
    pub fn history(&self) -> usize {
        self.scrollback.len()
    }

    /// A row of what is in view, `back` lines up into the history.
    pub fn line(&self, row: usize, back: usize) -> &[Cell] {
        let back = back.min(self.scrollback.len());
        if row < back {
            &self.scrollback[self.scrollback.len() - back + row]
        } else {
            let y = row - back;
            &self.grid[y * self.cols..(y + 1) * self.cols]
        }
    }

    /// The text of a screen row.
    #[cfg(test)]
    pub fn text(&self, row: usize) -> String {
        self.line(row, 0).iter().map(|c| c.ch).collect::<String>().trim_end().to_owned()
    }

    /// A new size: what is on screen is kept where it fits, anchored at the bottom.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(2));
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        let mut grid = vec![Cell::BLANK; cols * rows];
        // Keep the lines nearest the cursor.
        let keep = rows.min(self.rows);
        let from = (self.cursor.1 + 1).saturating_sub(keep).min(self.rows - keep);
        for y in 0..keep {
            for x in 0..cols.min(self.cols) {
                grid[y * cols + x] = self.grid[(from + y) * self.cols + x];
            }
        }
        self.cursor = (self.cursor.0.min(cols - 1), (self.cursor.1 - from).min(rows - 1));
        self.grid = grid;
        self.cols = cols;
        self.rows = rows;
        self.region = (0, rows - 1);
        self.main = None;
        self.wrap_pending = false;
    }

    fn at(&mut self, x: usize, y: usize) -> &mut Cell {
        &mut self.grid[y * self.cols + x]
    }

    fn blank(&self) -> Cell {
        // Erased cells take the current background, as xterm does.
        Cell { bg: self.pen.bg, ..Cell::BLANK }
    }

    fn go(&mut self, x: usize, y: usize) {
        self.cursor = (x.min(self.cols - 1), y.min(self.rows - 1));
        self.wrap_pending = false;
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = self.region;
        for _ in 0..n.min(bottom - top + 1) {
            if top == 0 && self.main.is_none() {
                self.scrollback.push_back(self.grid[..self.cols].to_vec());
                if self.scrollback.len() > SCROLLBACK {
                    self.scrollback.pop_front();
                }
            }
            self.grid.copy_within((top + 1) * self.cols..(bottom + 1) * self.cols, top * self.cols);
            let blank = self.blank();
            self.grid[bottom * self.cols..(bottom + 1) * self.cols].fill(blank);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = self.region;
        for _ in 0..n.min(bottom - top + 1) {
            self.grid.copy_within(top * self.cols..bottom * self.cols, (top + 1) * self.cols);
            let blank = self.blank();
            self.grid[top * self.cols..(top + 1) * self.cols].fill(blank);
        }
    }

    fn line_feed(&mut self) {
        if self.cursor.1 == self.region.1 {
            self.scroll_up(1);
        } else if self.cursor.1 + 1 < self.rows {
            self.cursor.1 += 1;
        }
        self.wrap_pending = false;
    }

    fn reverse_index(&mut self) {
        if self.cursor.1 == self.region.0 {
            self.scroll_down(1);
        } else {
            self.cursor.1 = self.cursor.1.saturating_sub(1);
        }
    }

    /// Clear cells from `from` up to `to` on a row.
    fn erase(&mut self, y: usize, from: usize, to: usize) {
        let blank = self.blank();
        let to = to.min(self.cols);
        if from < to {
            self.grid[y * self.cols + from..y * self.cols + to].fill(blank);
        }
    }

    fn alternate(&mut self, on: bool) {
        match (on, self.main.take()) {
            (true, None) => {
                let blank = vec![Cell::BLANK; self.cols * self.rows];
                self.main = Some((std::mem::replace(&mut self.grid, blank), self.cursor));
                self.go(0, 0);
            }
            (false, Some((grid, cursor))) => {
                if grid.len() == self.grid.len() {
                    self.grid = grid;
                }
                self.go(cursor.0, cursor.1);
            }
            (_, main) => self.main = main,
        }
        self.region = (0, self.rows - 1);
    }

    fn select_graphic(&mut self, params: &[u16]) {
        let mut i = 0;
        if params.is_empty() {
            self.pen = Cell::BLANK;
        }
        while i < params.len() {
            match params[i] {
                0 => self.pen = Cell::BLANK,
                1 => self.pen.bold = true,
                22 => self.pen.bold = false,
                7 => self.pen.reverse = true,
                27 => self.pen.reverse = false,
                n @ 30..=37 => self.pen.fg = (n - 30) as u8,
                39 => self.pen.fg = DEFAULT,
                n @ 40..=47 => self.pen.bg = (n - 40) as u8,
                49 => self.pen.bg = DEFAULT,
                n @ 90..=97 => self.pen.fg = (n - 90 + 8) as u8,
                n @ 100..=107 => self.pen.bg = (n - 100 + 8) as u8,
                // 256-colour and true-colour requests, brought down to the 16.
                kind @ (38 | 48) => {
                    let color = match params.get(i + 1) {
                        Some(5) => {
                            let n = params.get(i + 2).copied().unwrap_or(0);
                            i += 2;
                            nearest_256(n)
                        }
                        Some(2) => {
                            let rgb = (params.get(i + 2).copied().unwrap_or(0), params.get(i + 3).copied().unwrap_or(0), params.get(i + 4).copied().unwrap_or(0));
                            i += 4;
                            nearest_rgb(rgb.0, rgb.1, rgb.2)
                        }
                        _ => DEFAULT,
                    };
                    if kind == 38 { self.pen.fg = color } else { self.pen.bg = color }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// The nearest of the 16 colours to an RGB one: by which channels are lit.
fn nearest_rgb(r: u16, g: u16, b: u16) -> u8 {
    let max = r.max(g).max(b);
    if max < 60 {
        return 0;
    }
    let lit = |v: u16| v * 2 > max;
    let base = lit(r) as u8 | (lit(g) as u8) << 1 | (lit(b) as u8) << 2;
    if max > 200 { base + 8 } else { base }
}

fn nearest_256(n: u16) -> u8 {
    match n {
        0..=15 => n as u8,
        16..=231 => {
            let n = n - 16;
            let level = |v: u16| if v == 0 { 0 } else { 55 + v * 40 };
            nearest_rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        // The grey ramp.
        232..=243 => 8,
        244..=250 => 7,
        _ => 15,
    }
}

impl vte::Perform for Term {
    fn print(&mut self, c: char) {
        if self.wrap_pending {
            self.cursor.0 = 0;
            self.line_feed();
        }
        let (x, y) = self.cursor;
        let pen = self.pen;
        *self.at(x, y) = Cell { ch: c, ..pen };
        if x + 1 == self.cols {
            self.wrap_pending = true;
        } else {
            self.cursor.0 += 1;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.cursor.0 = self.cursor.0.saturating_sub(1);
                self.wrap_pending = false;
            }
            0x09 => self.go((self.cursor.0 / 8 + 1) * 8, self.cursor.1),
            0x0a..=0x0c => self.line_feed(),
            0x0d => self.go(0, self.cursor.1),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _ignore: bool, action: char) {
        let p: Vec<u16> = params.iter().map(|sub| sub[0]).collect();
        // A missing or zero count means one.
        let n = |i: usize| p.get(i).copied().filter(|v| *v > 0).unwrap_or(1) as usize;
        let (x, y) = self.cursor;
        if intermediates == [b'?'] {
            if let 'h' | 'l' = action {
                for mode in &p {
                    match mode {
                        25 => self.cursor_visible = action == 'h',
                        47 | 1047 | 1049 => self.alternate(action == 'h'),
                        2004 => self.bracketed = action == 'h',
                        _ => {}
                    }
                }
            }
            return;
        }
        if !intermediates.is_empty() {
            return;
        }
        match action {
            'A' => self.go(x, y.saturating_sub(n(0))),
            'B' | 'e' => self.go(x, y + n(0)),
            'C' | 'a' => self.go(x + n(0), y),
            'D' => self.go(x.saturating_sub(n(0)), y),
            'E' => self.go(0, y + n(0)),
            'F' => self.go(0, y.saturating_sub(n(0))),
            'G' | '`' => self.go(n(0) - 1, y),
            'd' => self.go(x, n(0) - 1),
            'H' | 'f' => self.go(n(1) - 1, n(0) - 1),
            'J' => match p.first().copied().unwrap_or(0) {
                0 => {
                    self.erase(y, x, self.cols);
                    for row in y + 1..self.rows {
                        self.erase(row, 0, self.cols);
                    }
                }
                1 => {
                    for row in 0..y {
                        self.erase(row, 0, self.cols);
                    }
                    self.erase(y, 0, x + 1);
                }
                _ => {
                    for row in 0..self.rows {
                        self.erase(row, 0, self.cols);
                    }
                }
            },
            'K' => match p.first().copied().unwrap_or(0) {
                0 => self.erase(y, x, self.cols),
                1 => self.erase(y, 0, x + 1),
                _ => self.erase(y, 0, self.cols),
            },
            'L' | 'M' if (self.region.0..=self.region.1).contains(&y) => {
                // Insert or delete lines: scroll the part of the region from here down.
                let region = self.region;
                self.region = (y, region.1);
                if action == 'L' { self.scroll_down(n(0)) } else { self.scroll_up_quiet(n(0)) }
                self.region = region;
            }
            'P' => {
                let count = n(0).min(self.cols - x);
                let row = y * self.cols;
                self.grid.copy_within(row + x + count..row + self.cols, row + x);
                self.erase(y, self.cols - count, self.cols);
            }
            '@' => {
                let count = n(0).min(self.cols - x);
                let row = y * self.cols;
                self.grid.copy_within(row + x..row + self.cols - count, row + x + count);
                self.erase(y, x, x + count);
            }
            'X' => self.erase(y, x, x + n(0)),
            'S' => self.scroll_up_quiet(n(0)),
            'T' => self.scroll_down(n(0)),
            'm' => self.select_graphic(&p),
            'r' => {
                let top = n(0) - 1;
                let bottom = p.get(1).copied().filter(|v| *v > 0).map_or(self.rows, |v| v as usize).min(self.rows) - 1;
                if top < bottom {
                    self.region = (top, bottom);
                    self.go(0, 0);
                }
            }
            's' => self.saved = self.cursor,
            'u' => self.go(self.saved.0, self.saved.1),
            // "Where is the cursor?" and "what are you?"
            'n' if p.first() == Some(&6) => self.replies.extend(format!("\x1b[{};{}R", y + 1, x + 1).bytes()),
            'c' => self.replies.extend(b"\x1b[?6c"),
            _ => {}
        }
    }

    /// OSC 52: a program sets the clipboard (`ESC ] 52 ; c ; <base64> BEL`),
    /// as tmux, vim and neovim can. Asking for it back (`?`) is not
    /// answered, as xterm by default: a program printing to the screen does
    /// not learn what was copied elsewhere.
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let [b"52", _, data, ..] = params else { return };
        if data.is_empty() || *data == b"?" || data.len() > OSC52_MOST {
            return;
        }
        let Ok(text) = std::str::from_utf8(data) else { return };
        if let Ok(bytes) = speech::base64_decode(text) {
            if let Ok(text) = String::from_utf8(bytes) {
                self.clip = Some(text);
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor.0 = 0;
                self.line_feed();
            }
            b'M' => self.reverse_index(),
            b'7' => self.saved = self.cursor,
            b'8' => self.go(self.saved.0, self.saved.1),
            b'c' => {
                let (cols, rows) = (self.cols, self.rows);
                let history = std::mem::take(&mut self.scrollback);
                *self = Term::new(cols, rows);
                self.scrollback = history;
            }
            _ => {}
        }
    }
}

impl Term {
    /// Scroll the region up without adding to the history: for deleted lines
    /// and explicit scrolls, which are edits rather than output going by.
    fn scroll_up_quiet(&mut self, n: usize) {
        let main = self.main.take();
        let held = std::mem::take(&mut self.scrollback);
        self.scroll_up(n);
        self.scrollback = held;
        self.main = main;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(text: &str) -> Term {
        let mut t = Term::new(20, 5);
        t.feed(text.as_bytes());
        t
    }

    #[test]
    fn text_lands_where_it_is_printed() {
        let t = term("hello\r\nworld");
        assert_eq!((t.text(0).as_str(), t.text(1).as_str()), ("hello", "world"));
        assert_eq!(t.cursor, (5, 1));
    }

    #[test]
    fn a_long_line_wraps_and_a_full_screen_scrolls_into_history() {
        let mut t = term("0123456789abcdefghijXYZ");
        assert_eq!((t.text(0).as_str(), t.text(1).as_str()), ("0123456789abcdefghij", "XYZ"));
        t.feed(b"\r\n2\r\n3\r\n4\r\n5\r\n6");
        assert_eq!(t.text(4), "6");
        assert_eq!(t.history(), 2);
        // Scrolled back by two, the first line is on top again.
        let top: String = t.line(0, 2).iter().map(|c| c.ch).collect();
        assert_eq!(top.trim_end(), "0123456789abcdefghij");
    }

    #[test]
    fn the_cursor_goes_where_it_is_sent_and_erasing_clears() {
        let mut t = term("aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
        t.feed(b"\x1b[2;4H\x1b[K");
        assert_eq!(t.cursor, (3, 1));
        assert_eq!(t.text(1), "bbb");
        t.feed(b"\x1b[1;1H\x1b[J");
        assert_eq!((t.text(0).as_str(), t.text(2).as_str()), ("", ""));
        t.feed(b"abc\x1b[2D\x1b[1P");
        assert_eq!(t.text(0), "ac");
    }

    #[test]
    fn colours_bold_and_reverse_are_kept_per_cell() {
        let t = term("\x1b[1;31mE\x1b[0m \x1b[44;97mW\x1b[7mR\x1b[38;5;196mX\x1b[38;2;0;200;0mG");
        let c = |x: usize| t.line(0, 0)[x];
        assert_eq!((c(0).ch, c(0).fg, c(0).bold), ('E', 1, true));
        assert_eq!((c(1).fg, c(1).bold), (DEFAULT, false));
        assert_eq!((c(2).fg, c(2).bg), (15, 4));
        assert!(c(3).reverse);
        assert_eq!(c(4).fg, 9, "256-colour red comes down to bright red");
        assert_eq!(c(5).fg, 2, "true-colour green comes down to green");
    }

    #[test]
    fn a_scrolling_region_leaves_the_rest_alone() {
        // What an editor does: a status line at the bottom, text scrolling above.
        let mut t = term("one\r\ntwo\r\nthree\r\nfour\r\nSTATUS");
        t.feed(b"\x1b[1;4r\x1b[4;1H\r\nfive");
        assert_eq!(t.text(0), "two");
        assert_eq!(t.text(3), "five");
        assert_eq!(t.text(4), "STATUS");
        // Reverse index at the top of the region scrolls it down.
        t.feed(b"\x1b[1;1H\x1bMzero");
        assert_eq!((t.text(0).as_str(), t.text(1).as_str(), t.text(4).as_str()), ("zero", "two", "STATUS"));
    }

    #[test]
    fn a_full_screen_program_gives_the_screen_back() {
        let mut t = term("$ vi notes\r\n");
        t.feed(b"\x1b[?1049h\x1b[2J\x1b[Hediting");
        assert_eq!(t.text(0), "editing");
        t.feed(b"\x1b[?1049l");
        assert_eq!(t.text(0), "$ vi notes");
        assert_eq!(t.cursor, (0, 1));
    }

    #[test]
    fn queries_are_answered() {
        let mut t = term("ab\x1b[6n");
        assert_eq!(t.take_replies(), b"\x1b[1;3R");
        t.feed(b"\x1b[c");
        assert_eq!(t.take_replies(), b"\x1b[?6c");
    }

    #[test]
    fn a_program_sets_the_clipboard_by_osc_52_but_cannot_read_it() {
        // "10.58.5.2" in base64, ended by BEL, then by ST.
        let mut t = term("\x1b]52;c;MTAuNTguNS4y\x07");
        assert_eq!(t.take_clip().as_deref(), Some("10.58.5.2"));
        assert_eq!(t.take_clip(), None, "taken once");
        t.feed(b"\x1b]52;;aMOpbGxv\x1b\\");
        assert_eq!(t.take_clip().as_deref(), Some("h\u{e9}llo"));
        t.feed(b"\x1b]52;c;?\x07");
        assert!(t.take_clip().is_none() && t.take_replies().is_empty(), "asking for it is not answered");
        t.feed(b"\x1b]52;c;not base64!\x07");
        assert!(t.take_clip().is_none());
        assert_eq!(t.text(0), "", "nothing of it printed");
    }

    #[test]
    fn bracketed_paste_is_followed_and_lines_are_read_by_number() {
        let mut t = term("\x1b[?2004h");
        assert!(t.bracketed);
        t.feed(b"\x1b[?2004l");
        assert!(!t.bracketed);
        let t = term("one\r\ntwo\r\nthree\r\nfour\r\nfive");
        assert_eq!(t.lines_total(), t.history() + t.rows);
        let last = t.history() + t.cursor.1;
        assert_eq!(t.line_text(last), "five");
        assert_eq!(t.line_text(last - 4), "one");
    }

    #[test]
    fn resizing_keeps_what_was_on_screen() {
        let mut t = term("one\r\ntwo\r\nthree");
        t.resize(10, 2);
        assert_eq!((t.text(0).as_str(), t.text(1).as_str()), ("two", "three"));
        assert_eq!(t.cursor, (5, 1));
    }
}
