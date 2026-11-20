//! The control centre: the switches reached for most, a pull away from any
//! screen. Ctrl-Space brings it down over whatever is showing (Up from the
//! top of Home does too) and puts it away again.
//!
//! It is a sheet of glass that drops from the top on a spring and can be
//! turned back at any point. The switches' knobs slide, and the brightness
//! bar glides to its level. Along its foot, two actions: the recent apps,
//! and a screenshot of the screen beneath (R and S while it is down).

use std::sync::atomic::{AtomicBool, Ordering};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, W};
use crate::platform::Key;
use crate::shell::Settings;

/// Where the sheet rests when it is down.
pub const SHEET: Rect = Rect { x: 8.0, y: 5.0, w: W - 16.0, h: 159.0 };
pub const SHEET_RADIUS: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Wifi,
    Ssh,
    Light,
    Calm,
    Brightness,
    Recents,
    Screenshot,
}

/// In rows: two switches, two switches, the brightness bar, two actions.
const ITEMS: [Item; 7] = [Item::Wifi, Item::Ssh, Item::Light, Item::Calm, Item::Brightness, Item::Recents, Item::Screenshot];

/// Each item's row and column.
fn place(at: usize) -> (usize, usize) {
    match at {
        0..=3 => (at / 2, at % 2),
        4 => (2, 0),
        _ => (3, at - 5),
    }
}

/// The item at a row and column (the brightness bar spans its row).
fn at(row: usize, col: usize) -> usize {
    match row {
        0 | 1 => row * 2 + col,
        2 => 4,
        _ => 5 + col,
    }
}

/// The actions along the sheet's foot: icon, name, and the key for each.
const ACTIONS: [(&str, &str, &str); 2] = [("history", "Recents", "r"), ("photo_camera", "Screenshot", "s")];

/// Off the handheld there is no radio or SSH server to switch: these stand in.
static DEMO_SSH: AtomicBool = AtomicBool::new(false);
static DEMO_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(70);

fn on_device() -> bool {
    std::path::Path::new("/usr/sbin/catalyst-ssh-toggle").exists()
}

fn brightness() -> u32 {
    crate::sys::brightness().unwrap_or_else(|| DEMO_LEVEL.load(Ordering::Relaxed))
}

pub struct Control {
    /// 0 put away, 1 fully down.
    pub shown: Animated,
    open: bool,
    focus: usize,
    wifi: bool,
    ssh: bool,
    level: u32,
    /// Each switch's knob, 0 (off) to 1 (on), and the brightness bar's fill.
    knobs: [Animated; 4],
    bar: Animated,
    /// Its own focus ring (x, y, width, height), gliding between switches.
    /// The screen underneath keeps its ring where it was.
    ring: [Animated; 4],
    ring_placed: bool,
}

/// What a key did.
pub enum Outcome {
    Nothing,
    /// A setting changed: apply and save it.
    Settings,
    /// The recent apps, in place of the sheet.
    Recents,
    /// A screenshot of the screen beneath, as the sheet goes.
    Screenshot,
}

impl Control {
    pub fn new() -> Self {
        let knob = || Animated::new(0.0, role::tick()).with_rest(0.003);
        Control {
            shown: Animated::new(0.0, role::release()).with_rest(0.002),
            open: false,
            focus: 0,
            wifi: false,
            ssh: false,
            level: 70,
            knobs: [knob(), knob(), knob(), knob()],
            bar: Animated::new(0.7, role::hold()).with_rest(0.003),
            ring: std::array::from_fn(|_| Animated::new(0.0, role::hold()).with_rest(0.05)),
            ring_placed: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// On screen at all, even on its way out.
    pub fn visible(&self) -> bool {
        self.open || self.shown.value() > 0.004
    }

    /// Bring it down, or put it away, from wherever it is.
    pub fn set_open(&mut self, open: bool, settings: &Settings) {
        self.open = open;
        if open {
            // What the switches say is read when the sheet is summoned.
            self.wifi = crate::wifi::switched_on();
            self.ssh = if on_device() { crate::ssh_enabled() } else { DEMO_SSH.load(Ordering::Relaxed) };
            self.level = brightness();
            let states = self.states(settings);
            for (knob, on) in self.knobs.iter_mut().zip(states) {
                knob.jump(if on { 1.0 } else { 0.0 });
            }
            self.bar.jump(self.level as f32 / 100.0);
            self.ring_placed = false;
        }
        let target = if open { 1.0 } else { 0.0 };
        if settings.calm {
            self.shown.jump(target);
        } else {
            // Arriving on the release spring; leaving on smooth, with no bounce.
            self.shown.set_with(target, if open { role::release() } else { role::smooth() });
        }
    }

    fn states(&self, settings: &Settings) -> [bool; 4] {
        [self.wifi, self.ssh, settings.light, settings.calm]
    }

    /// A key while the sheet is down. `None` means "put it away".
    pub fn key(&mut self, key: Key, settings: &mut Settings) -> Option<Outcome> {
        let item = ITEMS[self.focus];
        let (row, col) = place(self.focus);
        match key {
            Key::Back | Key::Leave | Key::Ctrl(' ') => return None,
            // Up past the top row pushes the sheet back where it came from.
            Key::Up if row == 0 => return None,
            Key::Up => self.focus = at(row - 1, col),
            Key::Down if row < 3 => self.focus = at(row + 1, col),
            // The two actions have keys of their own while the sheet is down.
            Key::Char('r') => return Some(Outcome::Recents),
            Key::Char('s') => return Some(Outcome::Screenshot),
            Key::Enter | Key::Char(' ') if item == Item::Recents => return Some(Outcome::Recents),
            Key::Enter | Key::Char(' ') if item == Item::Screenshot => return Some(Outcome::Screenshot),
            Key::Left | Key::Right if item == Item::Brightness => {
                let step = if key == Key::Left { -10 } else { 10 };
                self.level = (self.level as i32 + step).clamp(10, 100) as u32;
                if !crate::sys::set_brightness(self.level) {
                    DEMO_LEVEL.store(self.level, Ordering::Relaxed);
                }
                self.bar.set(self.level as f32 / 100.0);
            }
            Key::Left if col == 1 => self.focus -= 1,
            Key::Right if col == 0 => self.focus += 1,
            Key::Tab => self.focus = (self.focus + 1) % ITEMS.len(),
            Key::Enter | Key::Char(' ') => return Some(self.toggle(item, settings)),
            _ => {}
        }
        Some(Outcome::Nothing)
    }

    fn toggle(&mut self, item: Item, settings: &mut Settings) -> Outcome {
        let outcome = match item {
            Item::Wifi => {
                self.wifi = !self.wifi;
                crate::wifi::switch(self.wifi);
                Outcome::Nothing
            }
            Item::Ssh => {
                self.ssh = !self.ssh;
                let on = self.ssh;
                if on_device() {
                    // Starting the server can take a moment: not on the frame's time.
                    std::thread::spawn(move || crate::set_ssh(on).ok());
                } else {
                    DEMO_SSH.store(on, Ordering::Relaxed);
                }
                Outcome::Nothing
            }
            Item::Light => {
                settings.light = !settings.light;
                Outcome::Settings
            }
            Item::Calm => {
                settings.calm = !settings.calm;
                Outcome::Settings
            }
            Item::Brightness | Item::Recents | Item::Screenshot => Outcome::Nothing,
        };
        let states = self.states(settings);
            for (knob, on) in self.knobs.iter_mut().zip(states) {
            let target = if on { 1.0 } else { 0.0 };
            if settings.calm {
                knob.jump(target);
            } else {
                knob.set(target);
            }
        }
        outcome
    }

    /// Advance its springs. True while anything is still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        // Each spring must be advanced, so no short-circuit.
        let mut moving = self.shown.tick(dt) | self.bar.tick(dt);
        for spring in self.knobs.iter_mut().chain(&mut self.ring) {
            moving |= spring.tick(dt);
        }
        moving
    }

    /// How far below its resting place the sheet is drawn (negative: above).
    pub fn offset(&self) -> f32 {
        (self.shown.value() - 1.0) * (SHEET.bottom() + 8.0)
    }

    /// What sits on the glass: four switches and the brightness bar.
    pub fn draw(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let dy = self.offset();
        let inner = SHEET.inset(8.0).offset(0.0, dy);
        let gap = 5.0;
        let (tw, th) = ((inner.w - gap) * 0.5, 32.0);
        let names = [("wifi", "Wi-Fi"), ("terminal", "SSH"), ("lightbulb", "Light"), ("tune", "Calm")];
        let mut focus_rect = None;
        for (i, ((icon, name), knob)) in names.iter().zip(&self.knobs).enumerate() {
            let r = Rect::new(inner.x + (i % 2) as f32 * (tw + gap), inner.y + (i / 2) as f32 * (th + gap), tw, th);
            let on = knob.value().clamp(0.0, 1.0);
            g.canvas.rrect(r, 12.0, t.surface1, 0.72);
            g.icon(icon, on > 0.5, r.x + 16.0, r.cy(), 16.0, if on > 0.5 { t.ink } else { t.dim });
            g.text_fit(r.x + 30.0, r.cy(), name, ty.body, t.ink, r.w - 66.0);
            // The switch: its track takes colour as the knob crosses.
            let track = Rect::new(r.right() - 34.0, r.cy() - 7.0, 26.0, 14.0);
            g.canvas.rrect(track, 7.0, t.meter, 1.0);
            g.canvas.rrect(track, 7.0, t.ice, on);
            let knob_x = track.x + 7.0 + (track.w - 14.0) * knob.value().clamp(-0.1, 1.1);
            // The knob stays readable on either track: dark on the lit one.
            let lit = if t.tone == bezel::Tone::Dark { t.ground } else { t.ink };
            g.canvas.circle(knob_x, track.cy(), 5.0, if on > 0.5 { lit } else { t.ink }, 1.0);
            if i == self.focus {
                focus_rect = Some((r, 12.0));
            }
        }
        let r = Rect::new(inner.x, inner.y + 2.0 * (th + gap), inner.w, th);
        g.canvas.rrect(r, 12.0, t.surface1, 0.72);
        g.icon("lightbulb", true, r.x + 16.0, r.cy(), 16.0, t.dim);
        let value = format!("{}%", self.level);
        let value_w = g.measure("100%", ty.label);
        g.text_at(r.right() - 10.0, r.cy(), Align::Right, &value, ty.label, t.ink);
        g.meter(Rect::new(r.x + 32.0, r.cy() - 3.0, r.w - 50.0 - value_w, 6.0), self.bar.value(), t.ice);
        if self.focus == 4 {
            focus_rect = Some((r, 12.0));
        }
        // The two actions, each with the key that does it while the sheet is down.
        for (i, (icon, name, key)) in ACTIONS.iter().enumerate() {
            let r = Rect::new(inner.x + i as f32 * (tw + gap), inner.y + 3.0 * (th + gap), tw, th);
            g.canvas.rrect(r, 12.0, t.surface1, 0.72);
            g.icon(icon, false, r.x + 16.0, r.cy(), 16.0, t.ink);
            g.text_fit(r.x + 30.0, r.cy(), name, ty.body, t.ink, r.w - 30.0 - 26.0);
            let cap = Rect::new(r.right() - 22.0, r.cy() - 7.5, 14.0, 15.0);
            g.canvas.rrect(cap, 4.0, t.surface2, 0.9);
            g.text_at(cap.cx(), r.cy(), Align::Center, key, ty.caption, t.dim);
            if self.focus == 5 + i {
                focus_rect = Some((r, 12.0));
            }
        }
        if let Some((r, radius)) = focus_rect {
            // The ring glides between switches, and rides down with the sheet:
            // its place is kept as if the sheet were at rest.
            let at = [r.x, r.y - dy, r.w, r.h];
            for (spring, to) in self.ring.iter_mut().zip(at) {
                if !self.ring_placed || g.calm {
                    spring.jump(to);
                } else if spring.target() != to {
                    spring.set(to);
                    g.started_moving();
                }
            }
            self.ring_placed = true;
            let [x, y, w, h] = [0, 1, 2, 3].map(|i| self.ring[i].value());
            g.canvas.stroke_rrect(Rect::new(x, y + dy, w, h).inset(-2.5), radius + 2.5, 1.5, g.ring_color(), 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn it_comes_down_turns_back_mid_way_and_goes() {
        let mut s = settings();
        let mut c = Control::new();
        c.set_open(true, &s);
        for _ in 0..6 {
            c.tick(1.0 / 60.0);
        }
        let part_way = c.shown.value();
        assert!(part_way > 0.05 && part_way < 0.95, "still on its way down: {part_way}");
        // Put away mid-flight: it turns round from where it is, without a jump.
        c.set_open(false, &s);
        c.tick(1.0 / 60.0);
        assert!((c.shown.value() - part_way).abs() < 0.12, "a jump from {part_way} to {}", c.shown.value());
        assert!(c.visible());
        for _ in 0..120 {
            c.tick(1.0 / 60.0);
        }
        assert!(!c.visible());
        // The calm version lands at once.
        s.calm = true;
        c.set_open(true, &s);
        assert_eq!(c.shown.value(), 1.0);
    }

    #[test]
    fn keys_move_between_switches_flip_them_and_put_it_away() {
        let mut s = settings();
        let mut c = Control::new();
        c.set_open(true, &s);
        // Right to SSH, down to Calm, left to Light: flip it.
        for key in [Key::Right, Key::Down, Key::Left] {
            assert!(c.key(key, &mut s).is_some());
        }
        assert!(matches!(c.key(Key::Enter, &mut s), Some(Outcome::Settings)));
        assert!(s.light);
        // The knob slides rather than jumping.
        c.tick(1.0 / 60.0);
        assert!(c.knobs[2].value() > 0.0 && c.knobs[2].value() < 1.0);
        // Down to brightness: left and right change it, within bounds.
        assert!(c.key(Key::Down, &mut s).is_some());
        let before = c.level;
        c.key(Key::Left, &mut s);
        assert_eq!(c.level, (before - 10).max(10));
        for _ in 0..20 {
            c.key(Key::Right, &mut s);
        }
        assert_eq!(c.level, 100);
        // Up from the top row, or Esc, puts the sheet away.
        c.key(Key::Up, &mut s);
        c.key(Key::Up, &mut s);
        assert!(c.key(Key::Up, &mut s).is_none());
        assert!(c.key(Key::Back, &mut s).is_none());
    }

    #[test]
    fn the_foot_of_the_sheet_has_the_recent_apps_and_a_screenshot() {
        let mut s = settings();
        let mut c = Control::new();
        c.set_open(true, &s);
        // Down past the brightness bar to Recents; right to Screenshot.
        for key in [Key::Down, Key::Down, Key::Down] {
            c.key(key, &mut s);
        }
        assert!(matches!(c.key(Key::Enter, &mut s), Some(Outcome::Recents)));
        c.key(Key::Right, &mut s);
        assert!(matches!(c.key(Key::Enter, &mut s), Some(Outcome::Screenshot)));
        // Up from there: the bar, then the switches.
        c.key(Key::Up, &mut s);
        assert_eq!(c.focus, 4);
        c.key(Key::Up, &mut s);
        assert_eq!(c.focus, 2);
        // Their keys work from anywhere on the sheet.
        assert!(matches!(c.key(Key::Char('r'), &mut s), Some(Outcome::Recents)));
        assert!(matches!(c.key(Key::Char('s'), &mut s), Some(Outcome::Screenshot)));
    }

    #[test]
    fn every_name_on_the_sheet_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let tw = (SHEET.inset(8.0).w - 5.0) * 0.5;
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let body = g.ty.body;
            for (_, name, _) in ACTIONS {
                assert!(g.measure(name, body) <= tw - 56.0, "{name}");
            }
            for name in ["Wi-Fi", "SSH", "Light", "Calm"] {
                assert!(g.measure(name, body) <= tw - 66.0, "{name}");
            }
        }
        // The sheet, its four rows, rests on the panel.
        assert!(SHEET.bottom() <= crate::gfx::H - 4.0);
    }
}
