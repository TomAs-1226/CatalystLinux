//! What the developer tools share: a line of text typed on the keyboard, a
//! small graph of recent values, list headings the focus steps over,
//! commands run without a window, and the team this handheld is with.

use bezel::{Align, Color, Rect};

use crate::gfx::{Gfx, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;

// ----------------------------------------------------------------- field ---

/// A line typed on the keyboard: what it is for, and what has been typed.
pub struct Field {
    pub label: &'static str,
    pub text: String,
    limit: usize,
}

/// What a key did to a field.
pub enum Typed {
    /// Still typing.
    More,
    /// Enter: the text as it stands.
    Done(String),
    /// Esc: left as it was.
    Cancelled,
}

impl Field {
    pub fn new(label: &'static str, text: &str, limit: usize) -> Self {
        Field { label, text: text.to_owned(), limit }
    }

    /// A key while typing; `allowed` says which characters belong.
    pub fn key(&mut self, key: Key, allowed: impl Fn(char) -> bool) -> Typed {
        match key {
            Key::Char(c) if self.text.chars().count() < self.limit && allowed(c) => self.text.push(c),
            Key::Backspace => {
                self.text.pop();
            }
            Key::Enter => return Typed::Done(self.text.trim().to_owned()),
            Key::Back | Key::Leave => return Typed::Cancelled,
            _ => {}
        }
        Typed::More
    }

    /// The field at the top of the page, ringed, with the keys along the bottom.
    pub fn draw(&self, g: &mut Gfx, hint: &str) {
        let t = g.tokens;
        let ty = g.ty;
        let field = Rect::new(PAD, g.top(), W - 2.0 * PAD, ty.caption.size + ty.label.size + 20.0);
        g.surface(field, 14.0, t.surface2);
        g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, self.label, ty.caption, t.dim);
        let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
        // A long line shows its end, where the typing is.
        let room = field.w - 26.0;
        let mut shown: &str = &self.text;
        while g.measure(shown, ty.label) > room {
            let mut chars = shown.chars();
            chars.next();
            shown = chars.as_str();
        }
        let w = g.text_at(field.x + 10.0, cy, Align::Left, shown, ty.label, t.ink);
        g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
        g.focus_ring(field, 14.0);
        g.hint(hint);
    }
}

/// Characters a host name, an address or a list of ports is made of.
pub fn host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || ".-_:@[]".contains(c)
}

// ----------------------------------------------------------------- graph ---

/// Recent values as a small graph inside `r`: a line over a faint fill,
/// newest at the right, scaled to `max` (or to the largest value shown,
/// whichever is greater). Fewer values than fit start part-way across.
pub fn sparkline(g: &mut Gfx, r: Rect, values: &[f32], max: f32, color: Color, slots: usize) {
    if values.len() < 2 || slots < 2 {
        return;
    }
    let top = values.iter().cloned().fold(max, f32::max).max(f32::EPSILON);
    let step = r.w / (slots - 1) as f32;
    let shown = &values[values.len().saturating_sub(slots)..];
    let start = slots - shown.len();
    let pts: Vec<(f32, f32)> =
        shown.iter().enumerate().map(|(i, v)| (r.x + (start + i) as f32 * step + g.ox, r.bottom() - (v / top).clamp(0.0, 1.0) * r.h)).collect();
    let mut area = pts.clone();
    area.push((pts[pts.len() - 1].0, r.bottom()));
    area.push((pts[0].0, r.bottom()));
    g.canvas.polygon(&area, color, 0.16);
    g.canvas.polyline(&pts, 1.5, color, 1.0);
}

// -------------------------------------------------------------- headings ---

/// Move a list's focus by `key`, stepping over the rows `is_heading` says
/// are headings. Up past the first row that is not one stays on it.
pub fn step_over_headings(list: &mut crate::gfx::ListState, key: Key, len: usize, is_heading: impl Fn(usize) -> bool) {
    if len == 0 {
        return;
    }
    let before = list.focus;
    crate::screens::list_key(list, key, len);
    // On past a heading the way the key went; at an end, back the other way.
    let mut way: isize = if list.focus >= before { 1 } else { -1 };
    for _ in 0..2 * len {
        if !is_heading(list.focus) {
            break;
        }
        let next = list.focus as isize + way;
        if next < 0 || next >= len as isize {
            way = -way;
        } else {
            list.focus = next as usize;
        }
    }
}

/// A heading in a list: lowercase mono, quiet, sitting on the row's baseline.
pub fn heading(g: &mut Gfx, y: f32, row_h: f32, name: &str) {
    let (style, ink) = (g.ty.caption, g.tokens.dim);
    g.text_at(PAD + 4.0, y + row_h * 0.62, Align::Left, name, style, ink);
}

// -------------------------------------------------------------- commands ---

/// A program run for its output, with no window of its own: on Windows a
/// console program would otherwise flash one up beside the simulator.
pub fn quiet(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.stdin(std::process::Stdio::null());
    command
}

/// A string as one word for `sh`, in single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ------------------------------------------------------------------ team ---

/// The team this handheld is with: from the robot's address, else the
/// number setup saved.
pub fn team(robot: &Robot) -> Option<u32> {
    crate::schedule::team_of(&robot.address).or_else(|| {
        let saved = std::fs::read_to_string(crate::config_dir().join("robot")).ok()?;
        crate::schedule::team_of(saved.trim())
    })
}

/// The robot's address: the one in use, else the team's usual one.
pub fn robot_address(robot: &Robot) -> Option<String> {
    let address = robot.address.trim();
    if !address.is_empty() && !robot.is_demo() {
        return Some(address.to_owned());
    }
    team(robot).map(|t| crate::screens::resolve_address(&t.to_string()))
}

/// Bytes in a few characters: `512 b`, `2.8 kb`, `640 kb`, `26 mb`; a
/// decimal only while it says something.
pub fn bytes(n: f64) -> String {
    let (v, unit) = match n {
        n if n < 1024.0 => return format!("{n:.0} b"),
        n if n < 1024.0 * 1024.0 => (n / 1024.0, "kb"),
        n if n < 1024.0 * 1024.0 * 1024.0 => (n / (1024.0 * 1024.0), "mb"),
        n => (n / (1024.0 * 1024.0 * 1024.0), "gb"),
    };
    if v < 10.0 { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_takes_what_belongs_and_ends_with_enter_or_esc() {
        let mut field = Field::new("host", "", 8);
        for c in "10.58 5!x9".chars() {
            field.key(Key::Char(c), host_char);
        }
        assert_eq!(field.text, "10.585x9", "the space and the ! are not a host's");
        field.key(Key::Backspace, host_char);
        assert!(matches!(field.key(Key::Enter, host_char), Typed::Done(t) if t == "10.585x"));
        assert!(matches!(field.key(Key::Back, host_char), Typed::Cancelled));
        let mut full = Field::new("x", "abcdefgh", 8);
        full.key(Key::Char('i'), |_| true);
        assert_eq!(full.text, "abcdefgh", "no longer than its limit");
    }

    #[test]
    fn quoting_keeps_a_word_whole() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(bytes(512.0), "512 b");
        assert_eq!(bytes(2.84 * 1024.0), "2.8 kb");
        assert_eq!(bytes(640.0 * 1024.0), "640 kb");
        assert_eq!(bytes(3.1 * 1024.0 * 1024.0), "3.1 mb");
        assert_eq!(bytes(26.4 * 1024.0 * 1024.0), "26 mb");
    }

    #[test]
    fn the_focus_steps_over_headings() {
        let heads = [true, false, false, true, false];
        let mut list = crate::gfx::ListState::default();
        list.focus = 1;
        step_over_headings(&mut list, Key::Down, 5, |i| heads[i]);
        assert_eq!(list.focus, 2);
        step_over_headings(&mut list, Key::Down, 5, |i| heads[i]);
        assert_eq!(list.focus, 4, "over the heading");
        step_over_headings(&mut list, Key::Up, 5, |i| heads[i]);
        assert_eq!(list.focus, 2, "and back over it");
        step_over_headings(&mut list, Key::Up, 5, |i| heads[i]);
        step_over_headings(&mut list, Key::Up, 5, |i| heads[i]);
        assert_eq!(list.focus, 1, "never onto the first heading");
    }
}
