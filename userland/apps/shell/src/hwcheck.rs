//! The hardware check: each part of the handheld on a row of its own, looked
//! for as the page opens, saying plainly what did not answer and what to try.
//! The keys need a hand: Enter on their row opens a line to type that uses
//! every letter and digit. In first-start setup it follows Welcome; it is
//! also in Settings › About.

use std::cell::Cell;
use std::rc::Rc;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    /// Found, and what was found.
    Ok(String),
    /// Not found, and what to try.
    Missing(String),
    /// Waits for a hand (the keys).
    Needs(String),
}

impl State {
    pub fn ok(&self) -> bool {
        matches!(self, State::Ok(_))
    }

    pub fn words(&self) -> &str {
        match self {
            State::Ok(s) | State::Missing(s) | State::Needs(s) => s,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Keys,
    Screen,
    Battery,
    Storage,
    Clock,
    Wifi,
    Ethernet,
    Camera,
    Motion,
    Compass,
    Infrared,
}

pub const PARTS: [Part; 11] =
    [Part::Keys, Part::Screen, Part::Battery, Part::Storage, Part::Clock, Part::Wifi, Part::Ethernet, Part::Camera, Part::Motion, Part::Compass, Part::Infrared];

impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Part::Keys => "Keys",
            Part::Screen => "Screen",
            Part::Battery => "Battery",
            Part::Storage => "Storage",
            Part::Clock => "Clock",
            Part::Wifi => "Wi-Fi",
            Part::Ethernet => "Ethernet",
            Part::Camera => "Camera",
            Part::Motion => "Motion sensor",
            Part::Compass => "Compass",
            Part::Infrared => "Infrared",
        }
    }

}

/// The line typed to check the keys: every letter, then every digit.
pub const LINE: &str = "pack my box with five dozen liquor jugs 1234567890";

/// On the handheld: its image's own programs are there.
fn handheld() -> bool {
    std::path::Path::new("/usr/sbin/catalyst-power").exists()
}

fn dir_has_entries(path: &str, matches: impl Fn(&str) -> bool) -> bool {
    std::fs::read_dir(path).into_iter().flatten().flatten().any(|e| matches(&e.file_name().to_string_lossy()))
}

/// Look for one part. `sim`: on a computer, where each stands in.
pub fn look(part: Part, sim: bool) -> State {
    if sim {
        return match part {
            Part::Keys => State::Needs("type a line ›".into()),
            Part::Screen => State::Ok("320 × 170".into()),
            Part::Battery => State::Ok("76% · simulated".into()),
            Part::Compass => State::Ok("simulated".into()),
            _ => State::Ok("simulated".into()),
        };
    }
    match part {
        Part::Keys => State::Needs("type a line ›".into()),
        Part::Screen => State::Ok("320 × 170".into()),
        Part::Battery => match crate::power::Reading::read() {
            Some(r) => State::Ok(format!("{}% · {}", r.percent, if r.flow.on_battery() { "on battery" } else { "charging" })),
            None => State::Missing("gauge silent".into()),
        },
        Part::Storage => match crate::sys::storage() {
            Some(s) => State::Ok(s),
            None => State::Missing("not mounted".into()),
        },
        Part::Clock => {
            if std::path::Path::new("/sys/class/rtc/rtc0").exists() {
                State::Ok("keeps time".into())
            } else {
                State::Missing("chip silent".into())
            }
        }
        // The radio sits on the SDIO bus whether or not Wi-Fi is switched on.
        Part::Wifi => {
            if std::path::Path::new("/sys/class/net/wlan0").exists() || dir_has_entries("/sys/bus/sdio/devices", |_| true) {
                State::Ok("radio found".into())
            } else {
                State::Missing("radio silent".into())
            }
        }
        Part::Ethernet => {
            let found = std::fs::read_dir("/sys/bus/usb/devices").into_iter().flatten().flatten().any(|d| {
                let id = |n: &str| std::fs::read_to_string(d.path().join(n)).map(|s| s.trim().to_owned()).unwrap_or_default();
                id("idVendor") == "0fe6" && id("idProduct") == "9900"
            });
            if found || std::path::Path::new("/sys/class/net/eth0").exists() {
                State::Ok("adapter found".into())
            } else {
                // Its hub is behind the USB switch: to the right.
                State::Missing("USB switch right?".into())
            }
        }
        // The sensor's driver binds once the camera answers on its bus.
        Part::Camera => {
            if dir_has_entries("/sys/bus/i2c/drivers/imx219", |n| n.contains('-')) {
                State::Ok("IMX219 found".into())
            } else {
                // (The Lite has none.)
                State::Missing("not found".into())
            }
        }
        Part::Motion => motion(),
        Part::Compass => compass(),
        Part::Infrared => infrared(),
    }
}

/// The BMI270 (filled in with the IMU's own module).
fn motion() -> State {
    State::Missing("not looked for yet".into())
}

/// The BMM150 behind it, on units that have one.
fn compass() -> State {
    State::Missing("not looked for yet".into())
}

/// The infrared LED and receiver: their device-tree nodes (their drivers
/// load only while Remote uses them).
fn infrared() -> State {
    let p = crate::ir::parts();
    match (p.transmitter, p.receiver) {
        (true, true) => State::Ok("LED and receiver".into()),
        (true, false) => State::Missing("no receiver".into()),
        (false, true) => State::Missing("no LED".into()),
        (false, false) => State::Missing("not set up".into()),
    }
}

/// Every part's state, looked for once and again every few seconds (a
/// part can come late: the Ethernet adapter as the USB switch moves).
pub struct Check {
    pub states: Vec<(Part, State)>,
    /// The keys' line was typed whole.
    pub keys_ok: Rc<Cell<bool>>,
    sim: bool,
    age: f32,
}

impl Check {
    pub fn run() -> Check {
        let sim = !handheld();
        let mut check = Check { states: Vec::new(), keys_ok: Rc::new(Cell::new(false)), sim, age: 0.0 };
        check.look_again();
        check
    }

    fn look_again(&mut self) {
        let keys = self.keys_ok.get();
        self.states = PARTS
            .iter()
            .map(|&p| (p, if p == Part::Keys && keys { State::Ok("every key works".into()) } else { look(p, self.sim) }))
            // A compass is not missing on a unit made without one: it is
            // simply not listed.
            .filter(|(p, s)| !(*p == Part::Compass && !s.ok() && self.sim))
            .collect();
    }

    /// Time passing. True if a row changed.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.age += dt;
        let keys_now = self.keys_ok.get() && !self.states.iter().any(|(p, s)| *p == Part::Keys && s.ok());
        if self.age < 3.0 && !keys_now {
            return false;
        }
        self.age = 0.0;
        let before = self.states.clone();
        self.look_again();
        before != self.states
    }

    /// Found, out of all.
    pub fn found(&self) -> (usize, usize) {
        (self.states.iter().filter(|(_, s)| s.ok()).count(), self.states.len())
    }

    /// What Enter on a row opens, if anything.
    pub fn open(&self, row: usize) -> Option<Box<dyn Screen>> {
        match self.states.get(row)?.0 {
            Part::Keys => Some(Box::new(KeysTest::new(self.keys_ok.clone()))),
            _ => None,
        }
    }

    /// The rows, as a list from `top` to `bottom`.
    pub fn draw(&self, g: &mut Gfx, list: &mut ListState, top: f32, bottom: f32) {
        let t = g.tokens;
        let row_h = g.row_h();
        for (i, y) in list.layout(g, self.states.len(), top, row_h, 3.0, bottom) {
            let (part, state) = &self.states[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            // Found reads quietly; missing in the fault colour; the keys,
            // waiting for a hand, in ink.
            let ink = match state {
                State::Ok(_) => t.dim,
                State::Missing(_) => g.fault(),
                State::Needs(_) => t.ink,
            };
            g.row(r, part.name(), state.words(), ink, i == list.focus);
        }
        list.scrollbar(g, self.states.len(), top, bottom - 2.0);
    }
}

/// Settings › About › Hardware check.
pub struct HardwareScreen {
    check: Check,
    list: ListState,
}

impl HardwareScreen {
    pub fn new() -> HardwareScreen {
        HardwareScreen { check: Check::run(), list: ListState::default() }
    }
}

impl Screen for HardwareScreen {
    fn title(&self) -> &str {
        "Hardware"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right => self.check.open(self.list.focus).map_or(Nav::Stay, Nav::Push),
            other => {
                crate::screens::list_key(&mut self.list, other, self.check.states.len());
                Nav::Stay
            }
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.check.tick(dt)
    }

    fn resume(&mut self) {
        self.check.tick(10.0);
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (found, all) = self.check.found();
        let top = g.top();
        self.check.draw(g, &mut self.list, top, H - g.hint_h() - 3.0);
        g.hint(&format!("{found} of {all} found · enter on keys to type"));
    }
}

/// Type the line: each letter and digit, as it is meant to be.
pub struct KeysTest {
    typed: String,
    done: Rc<Cell<bool>>,
    /// Seconds since the line was finished.
    finished: Option<f32>,
}

impl KeysTest {
    pub fn new(done: Rc<Cell<bool>>) -> KeysTest {
        KeysTest { typed: String::new(), done, finished: None }
    }

    /// The letters and digits of the line not yet typed right.
    fn left(&self) -> usize {
        let right = LINE.chars().zip(self.typed.chars()).take_while(|(a, b)| a == b).count();
        LINE.chars().skip(right).filter(|c| c.is_ascii_alphanumeric()).collect::<std::collections::HashSet<_>>().len()
    }
}

/// The line broken into rows that fit `width`, at word boundaries, as
/// character ranges.
fn rows_of(g: &mut Gfx, style: bezel::TextStyle, width: f32) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let chars: Vec<char> = LINE.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let mut end = chars.len();
        while g.measure(&chars[start..end].iter().collect::<String>(), style) > width {
            end = chars[start..end - 1].iter().rposition(|c| *c == ' ').map_or(end - 1, |p| start + p + 1);
        }
        rows.push((start, end));
        start = end;
    }
    rows
}

impl Screen for KeysTest {
    fn title(&self) -> &str {
        "Keys"
    }

    fn wants_text(&self) -> bool {
        self.finished.is_none()
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if self.finished.is_some() {
            return Nav::Pop;
        }
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Backspace => {
                self.typed.pop();
            }
            Key::Char(c) if self.typed.chars().count() < LINE.len() => self.typed.push(c),
            _ => {}
        }
        if self.typed == LINE {
            self.done.set(true);
            self.finished = Some(0.0);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        match &mut self.finished {
            Some(t) => {
                *t += dt;
                true
            }
            None => false,
        }
    }

    fn finished(&self) -> bool {
        self.finished.is_some_and(|t| t > 1.2)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let style = ty.label;
        let width = W - 2.0 * PAD - 12.0;
        let card = Rect::new(PAD, top, W - 2.0 * PAD, H - top - g.hint_h() - 6.0);
        g.surface(card, 14.0, t.surface1);
        let rows = rows_of(g, style, width);
        let typed: Vec<char> = self.typed.chars().collect();
        let line_h = style.size + 7.0;
        let y0 = card.y + 14.0;
        for (row, (start, end)) in rows.iter().enumerate() {
            let mut x = card.x + 10.0;
            let cy = y0 + row as f32 * line_h;
            for (i, c) in LINE.chars().enumerate().take(*end).skip(*start) {
                let s = c.to_string();
                let w = g.measure(&s, style);
                let color = match typed.get(i) {
                    Some(t2) if *t2 == c => t.ink,
                    Some(_) => g.fault(),
                    None => t.faint,
                };
                // A mistyped space shows as a mark, not as nothing.
                let shown = match typed.get(i) {
                    Some(t2) if *t2 != c && c == ' ' => "·".to_owned(),
                    _ => s,
                };
                if i == typed.len() && self.finished.is_none() {
                    g.surface(Rect::new(x - 0.5, cy - style.size * 0.55, 1.5, style.size * 1.1), 0.5, t.signal);
                }
                g.text_at(x, cy, Align::Left, &shown, style, color);
                x += w;
            }
        }
        let (words, color) = if self.finished.is_some() {
            ("every key answered".to_owned(), t.ink)
        } else {
            let left = self.left();
            (format!("{left} {} to go", if left == 1 { "key" } else { "keys" }), t.dim)
        };
        g.text_at(card.x + 10.0, card.bottom() - 12.0, Align::Left, &words, ty.caption, color);
        g.hint("type the line · esc leaves");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_has_every_letter_and_digit() {
        for c in ('a'..='z').chain('0'..='9') {
            assert!(LINE.contains(c), "{c}");
        }
    }

    #[test]
    fn typing_the_line_whole_passes_the_keys() {
        let done = Rc::new(Cell::new(false));
        let mut test = KeysTest::new(done.clone());
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        assert_eq!(test.left(), 36);
        for c in "pack my bix".chars() {
            test.key(Key::Char(c), &mut cx);
        }
        assert!(test.left() < 36 && !done.get());
        test.key(Key::Backspace, &mut cx);
        test.key(Key::Backspace, &mut cx);
        for c in LINE.chars().skip(9) {
            test.key(Key::Char(c), &mut cx);
        }
        assert!(done.get(), "typed whole: {}", test.typed);
        test.tick(2.0);
        assert!(test.finished());
    }

    #[test]
    fn every_row_and_the_line_fit() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let width = W - 2.0 * PAD - 5.0;
            for part in PARTS {
                for state in [look(part, false), look(part, true), State::Missing("USB switch right?".into()), State::Ok("100% · on battery".into()), State::Ok("every key works".into())] {
                    let room = width - 30.0 - g.measure(state.words(), ty.label);
                    assert!(g.measure(part.name(), ty.body) <= room, "{} beside {} at {size:?}", part.name(), state.words());
                }
            }
            let rows = rows_of(&mut g, ty.label, W - 2.0 * PAD - 12.0);
            let card_h = H - g.top() - g.hint_h() - 6.0;
            assert!(14.0 + rows.len() as f32 * (ty.label.size + 7.0) + 12.0 <= card_h, "{} rows at {size:?}", rows.len());
        }
    }
}
