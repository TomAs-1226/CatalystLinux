//! A line of text being typed: the cursor, and the keys that move it, as the
//! shell's text fields have them (Fn + Z and C move the cursor, Fn +
//! Backspace deletes forward, Fn + K and N go to the start and end).

use bezel::{Align, Rect};

use crate::keys::Key;
use crate::ui::Ui;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    pub text: String,
    /// The cursor, in characters from the start.
    pub cursor: usize,
    /// The longest the text may grow, in characters (0: no limit).
    pub limit: usize,
}

impl Field {
    pub fn new(text: &str) -> Field {
        Field { text: text.to_owned(), cursor: text.chars().count(), limit: 0 }
    }

    pub fn with_limit(mut self, limit: usize) -> Field {
        self.limit = limit;
        self
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }

    /// Take a key. True if it was the field's (typed, deleted or moved).
    pub fn key(&mut self, key: Key) -> bool {
        let len = self.text.chars().count();
        match key {
            Key::Char(c) if !c.is_control() => {
                if self.limit == 0 || len < self.limit {
                    let at = self.byte_at(self.cursor);
                    self.text.insert(at, c);
                    self.cursor += 1;
                }
            }
            Key::Backspace if self.cursor > 0 => {
                let at = self.byte_at(self.cursor - 1);
                self.text.remove(at);
                self.cursor -= 1;
            }
            Key::Delete if self.cursor < len => {
                let at = self.byte_at(self.cursor);
                self.text.remove(at);
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(len),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = len,
            Key::Ctrl('u') => {
                self.text.clear();
                self.cursor = 0;
            }
            _ => return false,
        }
        true
    }

    /// Draw it in `r`: the text (its end kept in view), a cursor, and
    /// `placeholder` quietly while it is empty.
    pub fn draw(&self, ui: &mut Ui, r: Rect, placeholder: &str, focused: bool) {
        let (t, ty) = (ui.tokens, ui.ty);
        let radius = ui.radius(8.0).min(r.h * 0.5);
        ui.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
        let style = ty.body;
        let room = r.w - 20.0;
        if self.text.is_empty() {
            ui.text_fit(r.x + 10.0, r.cy(), placeholder, style, t.faint, room);
        }
        // What fits, ending at the cursor when the line is longer than the field.
        let before: String = self.text.chars().take(self.cursor).collect();
        let mut start = 0;
        while ui.measure(&before.chars().skip(start).collect::<String>(), style) > room && start < before.chars().count() {
            start += 1;
        }
        let shown: String = self.text.chars().skip(start).collect();
        let shown = ui.text.fit(&shown, &style, room);
        ui.text_at(r.x + 10.0, r.cy(), Align::Left, &shown, style, t.ink);
        if focused {
            let at: String = before.chars().skip(start).collect();
            let x = r.x + 10.0 + ui.measure(&at, style);
            let ring = ui.ring_color();
            ui.surface(Rect::new(x, r.cy() - style.size * 0.55, 1.5, style.size * 1.1), 0.5, ring);
            ui.focus_ring(r, radius);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_moves_and_deletes_as_a_text_field_does() {
        let mut f = Field::new("ct");
        f.key(Key::Left);
        assert!(f.key(Key::Char('a')));
        assert_eq!((f.text.as_str(), f.cursor), ("cat", 2));
        f.key(Key::End);
        f.key(Key::Backspace);
        f.key(Key::Home);
        f.key(Key::Delete);
        assert_eq!(f.text, "a");
        assert!(!f.key(Key::Enter), "Enter is the page's");
        let mut short = Field::new("").with_limit(2);
        for c in "abc".chars() {
            short.key(Key::Char(c));
        }
        assert_eq!(short.text, "ab");
        // Letters beyond ASCII are whole characters.
        let mut word = Field::new("né");
        word.key(Key::Backspace);
        assert_eq!(word.text, "n");
    }
}
