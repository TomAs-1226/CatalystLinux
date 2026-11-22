//! Hex: any file as bytes, eight to a row with their offset and their
//! characters, from Files (h). Only the rows on screen are read, so a file
//! of any size opens at once. `g` goes to an offset, `/` finds bytes (hex
//! like `de ad be ef` or `0x7f454c46`, or text), `n` the next match.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

const PER_ROW: u64 = 8;

/// What is being searched for, as bytes: hex pairs (spaces allowed, an
/// optional 0x), else the text itself.
pub fn pattern(text: &str) -> Option<Vec<u8>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let hexish = t.strip_prefix("0x").unwrap_or(t).replace(' ', "");
    if hexish.len() % 2 == 0 && !hexish.is_empty() && hexish.chars().all(|c| c.is_ascii_hexdigit()) && (t.contains(' ') || t.starts_with("0x") || hexish.chars().any(|c| c.is_ascii_digit())) {
        return (0..hexish.len()).step_by(2).map(|i| u8::from_str_radix(&hexish[i..i + 2], 16).ok()).collect();
    }
    Some(t.as_bytes().to_vec())
}

/// An offset typed in hex (0x…, or with any of a–f) or decimal.
pub fn offset(text: &str) -> Option<u64> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix("0x") {
        return u64::from_str_radix(h, 16).ok();
    }
    if t.chars().any(|c| c.is_ascii_alphabetic()) {
        return u64::from_str_radix(t, 16).ok();
    }
    t.parse().ok()
}

/// Where `needle` next appears in the file at or after `from`, read a
/// megabyte at a time (a match across two reads is found too).
pub fn find(path: &Path, needle: &[u8], from: u64) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut at = from;
    let mut carry: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        let start = at - carry.len() as u64;
        carry.extend_from_slice(&buf[..n]);
        if let Some(i) = carry.windows(needle.len()).position(|w| w == needle) {
            return Some(start + i as u64);
        }
        at += n as u64;
        let keep = needle.len().saturating_sub(1).min(carry.len());
        carry.drain(..carry.len() - keep);
    }
}

/// One row as text: its offset, its bytes in hex, and them as characters.
pub fn row_text(offset: u64, bytes: &[u8], offset_digits: usize) -> (String, String, String) {
    let hex: Vec<String> = (0..PER_ROW as usize).map(|i| bytes.get(i).map_or("  ".to_owned(), |b| format!("{b:02x}"))).collect();
    let chars: String = bytes.iter().map(|b| if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '·' }).collect();
    (format!("{offset:0offset_digits$x}"), hex.join(" "), chars)
}

pub struct HexView {
    path: PathBuf,
    name: String,
    size: u64,
    /// The first row on screen, and where the scroll glides.
    first: u64,
    scroll: Animated,
    /// The bytes last read, from where.
    window: (u64, Vec<u8>),
    asking: Option<(Field, bool)>,
    /// The last match: where, and how long.
    found: Option<(u64, usize)>,
    needle: Option<Vec<u8>>,
}

impl HexView {
    pub fn open(path: &Path) -> Result<Self, String> {
        let size = std::fs::metadata(path).map_err(|e| format!("cannot open: {e}"))?.len();
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Ok(HexView { path: path.to_owned(), name, size, first: 0, scroll: Animated::new(0.0, role::hold()).with_rest(0.01), window: (u64::MAX, Vec::new()), asking: None, found: None, needle: None })
    }

    fn rows(&self) -> u64 {
        self.size.div_ceil(PER_ROW).max(1)
    }

    /// The bytes from `from` (row-aligned), `len` of them, read now if the
    /// last read does not have them.
    fn bytes(&mut self, from: u64, len: usize) -> &[u8] {
        let (at, have) = &self.window;
        if *at > from || at + (have.len() as u64) < (from + len as u64).min(self.size) {
            let start = from.saturating_sub(PER_ROW * 32);
            let mut buf = vec![0u8; len + (PER_ROW * 64) as usize];
            let n = std::fs::File::open(&self.path).and_then(|mut f| {
                f.seek(SeekFrom::Start(start))?;
                let mut n = 0;
                while n < buf.len() {
                    let got = f.read(&mut buf[n..])?;
                    if got == 0 {
                        break;
                    }
                    n += got;
                }
                Ok(n)
            });
            buf.truncate(n.unwrap_or(0));
            self.window = (start, buf);
        }
        let (at, have) = &self.window;
        let skip = (from - at) as usize;
        &have[skip.min(have.len())..(skip + len).min(have.len())]
    }

    fn go_to_row(&mut self, row: u64) {
        self.first = row.min(self.rows().saturating_sub(1));
        self.scroll.set(self.first as f32);
    }

    fn search(&mut self, from: u64) -> Option<String> {
        let needle = self.needle.clone()?;
        match find(&self.path, &needle, from).or_else(|| (from > 0).then(|| find(&self.path, &needle, 0)).flatten()) {
            Some(at) => {
                self.found = Some((at, needle.len()));
                self.go_to_row((at / PER_ROW).saturating_sub(1));
                (at < from).then(|| "from the start again".to_owned())
            }
            None => {
                self.found = None;
                Some("not in this file".into())
            }
        }
    }
}

impl Screen for HexView {
    fn title(&self) -> &str {
        &self.name
    }

    fn wants_text(&self) -> bool {
        self.asking.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let page = visible_rows(cx);
        if let Some((field, is_search)) = &mut self.asking {
            let is_search = *is_search;
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) => {
                    self.asking = None;
                    if is_search {
                        self.needle = pattern(&text);
                        if let Some(said) = self.search(0) {
                            *cx.say = Some(said);
                        }
                    } else {
                        match offset(&text) {
                            Some(at) if at < self.size.max(1) => self.go_to_row(at / PER_ROW),
                            _ => *cx.say = Some(format!("the file is {} bytes", self.size)),
                        }
                    }
                }
                Typed::Cancelled => self.asking = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let rows = self.rows();
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Up => self.go_to_row(self.first.saturating_sub(1)),
            Key::Down => self.go_to_row((self.first + 1).min(rows.saturating_sub(page))),
            Key::PageUp | Key::Left => self.go_to_row(self.first.saturating_sub(page)),
            Key::PageDown | Key::Right => self.go_to_row((self.first + page).min(rows.saturating_sub(page))),
            Key::Home => self.go_to_row(0),
            Key::End => self.go_to_row(rows.saturating_sub(page)),
            Key::Char('g') => self.asking = Some((Field::new("go to offset · 0x1f0 or 496", "", 20), false)),
            Key::Char('/') => self.asking = Some((Field::new("find · hex (de ad be ef) or text", "", 60), true)),
            Key::Char('n') => {
                let from = self.found.map_or(self.first * PER_ROW, |(at, _)| at + 1);
                if let Some(said) = self.search(from) {
                    *cx.say = Some(said);
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some((field, _)) = &self.asking {
            return field.draw(g, "enter: go · esc: cancel");
        }
        if g.calm {
            self.scroll.jump(self.first as f32);
        }
        let digits = if self.size > 0xFF_FFFF { 8 } else { 6 };
        // Eight bytes and their letters across, always: at a larger text size
        // the figures shrink to fit rather than run off the edge.
        let mut style = ty.caption;
        let need = g.measure(&"0".repeat(digits + 1), style) + 4.0 + g.measure(&"00 ".repeat(PER_ROW as usize), style) + 6.0 + g.measure(&"0".repeat(PER_ROW as usize), style);
        if need > W - 2.0 * PAD {
            style.size *= (W - 2.0 * PAD) / need;
        }
        let line = style.size + 4.0;
        let top = g.top();
        let bottom = g.above_hint();
        // The last row's own spacing may run under the hint's margin.
        let shown = (((bottom - top + 4.0) / line).floor() as u64).max(1);
        VISIBLE.with(|v| v.set(shown));
        let scroll = self.scroll.value();
        let from_row = scroll.floor().max(0.0) as u64;
        let bytes = self.bytes(from_row * PER_ROW, ((shown + 2) * PER_ROW) as usize).to_vec();
        let x_hex = PAD + g.measure(&"0".repeat(digits + 1), style) + 4.0;
        let x_chars = x_hex + g.measure(&"00 ".repeat(PER_ROW as usize), style) + 6.0;
        // Whole rows only: one sliding in is cut where the last one ends.
        g.canvas.set_clip(Rect::new(0.0, top - 2.0, W, shown as f32 * line + 2.0));
        for k in 0..shown + 2 {
            let row = from_row + k;
            if row >= self.rows() {
                break;
            }
            let cy = top + (row as f32 - scroll) * line + line * 0.5;
            let at = row * PER_ROW;
            let start = (k * PER_ROW) as usize;
            let chunk = &bytes[start.min(bytes.len())..(start + PER_ROW as usize).min(bytes.len())];
            if chunk.is_empty() && self.size > 0 {
                continue;
            }
            // A match, under its bytes.
            if let Some((m, len)) = self.found {
                for i in 0..PER_ROW {
                    let b = at + i;
                    if b >= m && b < m + len as u64 {
                        let x = x_hex + g.measure(&"00 ".repeat(i as usize), style) - 1.5;
                        let w = g.measure("00", style) + 3.0;
                        g.surface(Rect::new(x, cy - line * 0.5 + 1.0, w, line - 2.0), 3.0, t.surface3);
                    }
                }
            }
            let (off, hex, chars) = row_text(at, chunk, digits);
            g.text_at(PAD, cy, Align::Left, &off, style, t.faint);
            g.text_at(x_hex, cy, Align::Left, &hex, style, t.ink);
            g.text_at(x_chars, cy, Align::Left, &chars, style, t.dim);
        }
        g.canvas.clear_clip();
        let end = ((self.first + shown) * PER_ROW).min(self.size);
        let at = format!("{end} of {}", self.size);
        g.hint_any(&[&format!("{at} · g: go · /: find · n: next"), &format!("{at} · g · / · n"), &at]);
    }
}

thread_local! {
    /// Rows on screen at the last draw, for paging.
    static VISIBLE: std::cell::Cell<u64> = const { std::cell::Cell::new(10) };
}

fn visible_rows(_cx: &Cx) -> u64 {
    VISIBLE.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_to_find_reads_as_hex_or_text() {
        assert_eq!(pattern("de ad be ef"), Some(vec![0xde, 0xad, 0xbe, 0xef]));
        assert_eq!(pattern("0x7f454c46"), Some(vec![0x7f, 0x45, 0x4c, 0x46]));
        assert_eq!(pattern("robot"), Some(b"robot".to_vec()));
        assert_eq!(pattern("cafe"), Some(b"cafe".to_vec()), "letters only: text");
        assert_eq!(pattern("  "), None);
        assert_eq!(offset("0x1f0"), Some(496));
        assert_eq!(offset("1f0"), Some(496));
        assert_eq!(offset("496"), Some(496));
        assert_eq!(offset("zz"), None);
    }

    #[test]
    fn rows_show_offset_bytes_and_characters() {
        let (off, hex, chars) = row_text(16, b"Hi\x00\x7f!", 6);
        assert_eq!(off, "000010");
        assert!(hex.starts_with("48 69 00 7f 21 "), "{hex:?}");
        assert_eq!(hex.chars().count(), 8 * 3 - 1, "a short last row keeps its columns");
        assert_eq!(chars, "Hi··!");
    }

    #[test]
    fn bytes_are_found_across_reads_and_from_where_asked() {
        let dir = std::env::temp_dir().join(format!("catalyst-hex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        let mut data = vec![0u8; (1 << 20) + 10];
        // Straddling the first megabyte's edge.
        data[(1 << 20) - 2..(1 << 20) + 2].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        data[5..9].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        std::fs::write(&path, &data).unwrap();
        assert_eq!(find(&path, &[0xde, 0xad, 0xbe, 0xef], 0), Some(5));
        assert_eq!(find(&path, &[0xde, 0xad, 0xbe, 0xef], 6), Some((1 << 20) - 2));
        assert_eq!(find(&path, &[0xca, 0xfe], 0), None);
        let mut view = HexView::open(&path).unwrap();
        assert_eq!(view.rows(), ((1 << 20) + 10_u64).div_ceil(8));
        assert_eq!(&view.bytes(8, 8)[..1], &[0xef]);
        assert_eq!(view.bytes((1 << 20) - 8, 8)[6..], [0xde, 0xad]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
