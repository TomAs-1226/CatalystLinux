//! StandBy: on the charger and left alone, the handheld shows a face where
//! the screen would have gone off: a clock, the charge, the next match, the
//! weather, photos, or a little of everything. Designed for this panel
//! (320 × 170, a hand's length away on a desk), in the spirit of Bezel's
//! standby scapes: calm, dark, glanceable, nothing that blinks.
//!
//! - **Faces** page vertically, as Bezel's scapes do: ↑↓ turn to the next
//!   on the `page` spring, rubber-banding at either end, with the page dots
//!   at the right edge, which tuck away 2.5 s after the last key. ←→ change
//!   a face's style (the clock's digits, dial or dots) or, on Photos, the
//!   photo. Any other key leaves StandBy (and is spent doing so).
//! - **Motion**: the time's changed digits roll into place, the dial's
//!   second hand steps into each second on Bezel's `tick` spring, the charge
//!   ring glides to the charge. Nothing flashes or blinks.
//! - **Night**: from 10 pm to 7 am (there is no light sensor on the board:
//!   the infrared receiver only sees infrared), every face turns a deep red
//!   monochrome, and the backlight comes down to [`NIGHT_LEVEL`] of its
//!   brightness, both gliding over three seconds. At night the second hand
//!   rests (nothing moves each second).
//! - **Burn-in**: everything drifts a few pixels on a slow Lissajous path
//!   (21 and 29 minutes round), a pixel at a time.
//! - **Low power**: frames at 30 a second at most while something moves,
//!   none at rest; a face is drawn again only when what it shows changes
//!   (the minute, the second hand, the charge), the processor is at its
//!   saving pace (standby.rs), and only the panel's changed rows are sent.
//!
//! Which face and style were last shown are kept in the settings, so StandBy
//! comes back as it was left.

use bezel::{Align, Canvas, Color, GlassShape, Rect, TextStyle, Tokens, Tone};
use chrono::{Datelike, NaiveDateTime, Timelike};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::lock::{Choices, ClockStyle, Face};
use crate::platform::Key;
use crate::power::{Flow, Reading};
use crate::robot::Robot;

/// The backlight at night, as a share of its brightness.
pub const NIGHT_LEVEL: f32 = 0.4;
/// Night, by the clock: from this hour until the morning's.
pub const NIGHT_FROM: u32 = 22;
pub const NIGHT_TO: u32 = 7;
/// The page dots tuck away this long after the last key (Bezel's standby chrome).
const PAGER_FOR: f32 = 2.5;
/// Photos change by themselves this often.
const PHOTO_EVERY: f32 = 600.0;
/// The burn-in drift: how far, and how long round, in seconds.
const DRIFT: (f32, f32) = (3.0, 2.0);
const DRIFT_PERIOD: (f32, f32) = (1260.0, 1740.0);

// ------------------------------------------------------------- the time ---

#[cfg(any(test, not(target_arch = "aarch64")))]
thread_local! {
    /// The galleries' own time, so a picture always reads 9:41.
    static PRETEND: std::cell::Cell<Option<NaiveDateTime>> = const { std::cell::Cell::new(None) };
}

/// The time on the wall, here (or as the gallery pretends).
pub fn local_now() -> NaiveDateTime {
    #[cfg(any(test, not(target_arch = "aarch64")))]
    if let Some(at) = PRETEND.with(std::cell::Cell::get) {
        return at;
    }
    chrono::Local::now().naive_local()
}

/// For the galleries and the tests: the clock reads `at` from now on.
#[cfg(any(test, not(target_arch = "aarch64")))]
pub fn pretend_time(at: Option<NaiveDateTime>) {
    PRETEND.with(|p| p.set(at));
}

/// "9:41" or "21:41".
pub fn time_text(at: NaiveDateTime, h24: bool) -> String {
    at.format(if h24 { "%H:%M" } else { "%-I:%M" }).to_string()
}

/// "saturday 21 november".
pub fn date_text(at: NaiveDateTime) -> String {
    at.format("%A %-d %B").to_string().to_lowercase()
}

/// "sat 21 nov".
pub fn short_date(at: NaiveDateTime) -> String {
    at.format("%a %-d %b").to_string().to_lowercase()
}

/// Whether it is night at `at`, by the clock.
pub fn is_night(at: NaiveDateTime) -> bool {
    let h = at.hour();
    h >= NIGHT_FROM || h < NIGHT_TO
}

/// "10 pm to 7 am", or "22:00 to 7:00".
pub fn night_words(h24: bool) -> String {
    if h24 {
        format!("{NIGHT_FROM}:00 to {NIGHT_TO}:00")
    } else {
        format!("{} pm to {} am", NIGHT_FROM - 12, NIGHT_TO)
    }
}

/// The burn-in drift at Unix second `t`: whole pixels, one at a time.
pub fn drift_at(t: f64) -> (f32, f32) {
    let tau = std::f64::consts::TAU;
    let x = DRIFT.0 as f64 * (tau * t / DRIFT_PERIOD.0 as f64).sin();
    let y = DRIFT.1 as f64 * (tau * t / DRIFT_PERIOD.1 as f64 + 1.0).sin();
    (x.round() as f32, y.round() as f32)
}

// ------------------------------------------------------ shared drawing ---

/// A time in tabular figures: each digit in a cell as wide as the widest,
/// so nothing shifts as the minutes change (Bezel: tabular figures).
pub struct Tab {
    pub cell: f32,
    pub colon: f32,
    pub style: TextStyle,
}

impl Tab {
    pub fn new(g: &mut Gfx, style: TextStyle) -> Tab {
        let cell = (0..10).map(|d| g.measure(&d.to_string(), style)).fold(0.0, f32::max);
        Tab { cell, colon: g.measure(":", style), style }
    }

    pub fn width(&self, s: &str) -> f32 {
        s.chars().map(|c| if c.is_ascii_digit() { self.cell } else { self.colon }).sum()
    }

    /// The style at which `s` is `width` wide (for a time to fill a space).
    pub fn sized_to(g: &mut Gfx, base: TextStyle, s: &str, width: f32, most: f32) -> TextStyle {
        let probe = TextStyle { size: 100.0, ..base };
        let w = Tab::new(g, probe).width(s).max(1.0);
        TextStyle { size: (100.0 * width / w).min(most).floor(), ..base }
    }

    /// Draw `to` (rolling in from `from` as `roll` goes 0 → 1: a changed
    /// digit rises out as the new one rises into its place) with its left
    /// at `x`. Digits that did not change hold still.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, g: &mut Gfx, from: &str, to: &str, roll: f32, x: f32, baseline: f32, color: Color, alpha: f32) {
        let cap = g.text.cap_height(&self.style);
        let travel = cap * 0.55;
        let roll = roll.clamp(0.0, 1.0);
        // A change of length (9:59 to 10:00) crossfades as a whole.
        let same_shape = from.chars().count() == to.chars().count();
        let mut cx = x;
        let from_chars: Vec<char> = from.chars().collect();
        for (i, c) in to.chars().enumerate() {
            let w = if c.is_ascii_digit() { self.cell } else { self.colon };
            let old = from_chars.get(i).copied().filter(|_| same_shape);
            let centre = cx + w * 0.5;
            let glyph = |g: &mut Gfx, ch: char, dy: f32, a: f32| {
                if a <= 0.01 {
                    return;
                }
                let s = ch.to_string();
                g.text.draw_aligned(&mut g.canvas, centre, baseline + dy, Align::Center, &s, &self.style, color, alpha * a);
            };
            match old {
                Some(o) if o != c && roll < 1.0 => {
                    let band = Rect::new(cx - 2.0, baseline - cap - 4.0, w + 4.0, cap + 8.0);
                    g.canvas.set_clip(band);
                    glyph(g, o, -travel * roll, 1.0 - roll);
                    glyph(g, c, travel * (1.0 - roll), roll);
                    g.canvas.clear_clip();
                }
                Some(_) => glyph(g, c, 0.0, 1.0),
                None if !same_shape && roll < 1.0 => glyph(g, c, 0.0, roll),
                None => glyph(g, c, 0.0, 1.0),
            }
            cx += w;
        }
        if !same_shape && roll < 1.0 {
            let mut ox = x + (self.width(to) - self.width(from)) * 0.5;
            for c in from.chars() {
                let w = if c.is_ascii_digit() { self.cell } else { self.colon };
                g.text.draw_aligned(&mut g.canvas, ox + w * 0.5, baseline, Align::Center, &c.to_string(), &self.style, color, alpha * (1.0 - roll));
                ox += w;
            }
        }
    }
}

/// The dot-matrix figures, five dots by seven, row by row (a bit a dot,
/// the left one highest).
const DOT_FONT: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// A time as lit dots: (columns wide, the lit dots as (column, row)).
pub fn dot_time(s: &str) -> (usize, Vec<(usize, usize)>) {
    let mut lit = Vec::new();
    let mut col = 0;
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            col += 1;
        }
        match c.to_digit(10) {
            Some(d) => {
                for (row, bits) in DOT_FONT[d as usize].iter().enumerate() {
                    for x in 0..5 {
                        if bits & (0b10000 >> x) != 0 {
                            lit.push((col + x, row));
                        }
                    }
                }
                col += 5;
            }
            None => {
                lit.push((col, 2));
                lit.push((col, 4));
                col += 1;
            }
        }
    }
    (col, lit)
}

/// Draw a dot-matrix time: `pitch` apart, centred on `cx`, its top row at
/// `top`; the dots of `from` give way to those of `to` as `roll` goes to 1.
#[allow(clippy::too_many_arguments)]
pub fn draw_dots(g: &mut Gfx, from: &str, to: &str, roll: f32, cx: f32, top: f32, pitch: f32, color: Color, alpha: f32) {
    let roll = roll.clamp(0.0, 1.0);
    let (cols_to, lit_to) = dot_time(to);
    let (cols_from, lit_from) = dot_time(from);
    let r = pitch * 0.37;
    let place = |cols: usize, (c, row): (usize, usize)| (cx + (c as f32 - (cols as f32 - 1.0) * 0.5) * pitch, top + row as f32 * pitch);
    for &d in &lit_to {
        let was = cols_from == cols_to && lit_from.contains(&d);
        let k = if was { 1.0 } else { roll };
        let (x, y) = place(cols_to, d);
        g.canvas.circle(x, y, r * (0.55 + 0.45 * k), color, alpha * k);
    }
    if roll < 1.0 {
        for &d in &lit_from {
            if cols_from == cols_to && lit_to.contains(&d) {
                continue;
            }
            let (x, y) = place(cols_from, d);
            g.canvas.circle(x, y, r * (0.55 + 0.45 * (1.0 - roll)), color, alpha * (1.0 - roll));
        }
    }
}

/// A small capsule: an icon (or a coloured dot) and a few words.
#[allow(clippy::too_many_arguments)]
pub fn chip(g: &mut Gfx, x: f32, cy: f32, icon: Option<&str>, dot: Option<Color>, words: &str, fill: Color, ink: Color, alpha: f32) -> f32 {
    let style = g.ty.caption;
    let h = (style.size + 9.0).round();
    let lead = if icon.is_some() || dot.is_some() { 17.0 } else { 9.0 };
    let w = (lead + g.measure(words, style) + 9.0).round();
    let r = Rect::new(x.round(), (cy - h * 0.5).round(), w, h);
    g.canvas.rrect(r, h * 0.5, fill, alpha);
    if let Some(icon) = icon {
        g.icon_alpha(icon, true, r.x + 10.0, r.cy(), 12.0, ink, alpha);
    } else if let Some(dot) = dot {
        g.canvas.circle(r.x + 10.0, r.cy(), 3.0, dot, alpha);
    }
    g.text.draw(&mut g.canvas, r.x + lead, (r.cy() + style.size * 0.36).round(), words, &style, ink, alpha);
    w
}

/// How wide a chip of these words is.
pub fn chip_width(g: &mut Gfx, icon: bool, words: &str) -> f32 {
    ((if icon { 17.0 } else { 9.0 }) + g.measure(words, g.ty.caption) + 9.0).round()
}

/// Night's red, over a whole picture: each pixel's brightness as a deep red,
/// `amount` of the way.
pub fn redden(px: &mut [u32], amount: f32) {
    let k = (amount.clamp(0.0, 1.0) * 256.0) as u32;
    if k == 0 {
        return;
    }
    for p in px.iter_mut() {
        let (r, g, b) = ((*p >> 16) & 0xff, (*p >> 8) & 0xff, *p & 0xff);
        let lum = (r * 54 + g * 183 + b * 19) >> 8;
        let (nr, ng, nb) = (lum, lum * 34 >> 8, lum * 16 >> 8);
        let mix = |a: u32, b: u32| (a * (256 - k) + b * k) >> 8;
        *p = (mix(r, nr) << 16) | (mix(g, ng) << 8) | mix(b, nb);
    }
}

/// The ground behind a face: the tone's own, with one soft glow of `tint`
/// above and to the left, where Bezel's light comes from.
pub fn ground(canvas: &mut Canvas, t: &Tokens, tint: Color, ox: f32, oy: f32) {
    // Past the panel's edges by more than the drift reaches.
    canvas.fill_rect(Rect::new(ox - 8.0, oy - 8.0, W + 16.0, H + 16.0), t.ground, 1.0);
    let strength = if t.tone == Tone::Dark { 0.10 } else { 0.16 };
    canvas.glow(ox + 40.0, oy + 10.0, 190.0, tint, strength);
}

// ------------------------------------------------------- what they show ---

/// What a face shows, gathered by the shell.
pub struct Feed<'a> {
    pub reading: Option<&'a Reading>,
    pub h24: bool,
    /// Notices said while it was not looked at (only how many show).
    pub notices: usize,
    /// The schedule's generation: a new one, and the match is read again.
    pub generation: u64,
    /// Live activities, most important first; the one that has just
    /// changed, and how open its card is (it comes down from the top for a
    /// moment); music's level.
    pub activities: &'a [crate::liveactivity::Activity],
    pub opened: Option<&'a crate::liveactivity::Activity>,
    pub open: f32,
    pub level: f32,
}

/// A photo for the Photos face, decoded on a thread of its own.
struct PhotoSlot {
    list: Vec<crate::photos::Photo>,
    at: usize,
    listed_age: f32,
    shown: Option<crate::photos::Picture>,
    /// The one before, fading out as the new one fades in.
    before: Option<crate::photos::Picture>,
    fade: Animated,
    loading: Option<std::sync::mpsc::Receiver<Option<crate::photos::Picture>>>,
    age: f32,
}

impl PhotoSlot {
    fn new() -> PhotoSlot {
        PhotoSlot { list: Vec::new(), at: 0, listed_age: f32::MAX, shown: None, before: None, fade: Animated::new(1.0, role::effect()).with_rest(0.004), loading: None, age: 0.0 }
    }

    fn load(&mut self) {
        let Some(photo) = self.list.get(self.at).cloned() else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("standby photo".into()).stack_size(256 * 1024).spawn(move || {
            let pic = crate::photos::decode(&photo.path, W as usize, H as usize).ok().map(|p| p.cover(W as usize, H as usize));
            tx.send(pic).ok();
        });
        if spawned.is_ok() {
            self.loading = Some(rx);
        }
    }

    fn step(&mut self, by: i32) {
        if self.list.is_empty() {
            return;
        }
        self.at = (self.at as i32 + by).rem_euclid(self.list.len() as i32) as usize;
        self.age = 0.0;
        self.load();
    }

    /// True when the picture changed.
    fn tick(&mut self, dt: f32, md: f32, wanted: bool) -> bool {
        let mut changed = self.fade.tick(md);
        if !wanted {
            return changed;
        }
        self.listed_age += dt;
        if self.listed_age > PHOTO_EVERY {
            self.listed_age = 0.0;
            let list = crate::photos::list();
            if list.len() != self.list.len() || self.shown.is_none() {
                self.list = list;
                self.at = 0;
                self.load();
                changed = true;
            }
        }
        self.age += dt;
        if self.age >= PHOTO_EVERY && self.list.len() > 1 {
            self.step(1);
        }
        if let Some(rx) = &self.loading {
            if let Ok(pic) = rx.try_recv() {
                self.loading = None;
                if let Some(pic) = pic {
                    self.before = self.shown.replace(pic);
                    self.fade.jump(0.0);
                    self.fade.set(1.0);
                    changed = true;
                }
            }
        }
        if self.fade.value() >= 0.999 && self.before.is_some() {
            self.before = None;
        }
        changed
    }
}

/// What the faces show, read again now and then rather than every frame.
#[derive(Default)]
struct Cache {
    weather: Option<crate::weather::Glance>,
    next: Option<Result<crate::upnext::NextMatch, String>>,
    reminder: Option<crate::upnext::Reminder>,
    age: f32,
    generation: u64,
    /// To be read again at the next frame drawn.
    stale: bool,
}

// ------------------------------------------------------------- the layer ---

/// StandBy, over everything while it shows.
pub struct Faces {
    showing: bool,
    /// How present the layer is: 0 gone, 1 covering everything.
    shown: Animated,
    /// Where along the faces it is (a face's place; between two while
    /// turning), and along the clock's styles.
    pos: Animated,
    style_pos: Animated,
    /// The page dots: how present, and how long before they tuck away.
    pager: Animated,
    pager_left: f32,
    night: Animated,
    /// The time shown, the one before, and how far the change has rolled.
    time: String,
    time_before: String,
    roll: Animated,
    /// The dial's second hand, in seconds round the hour (continuous).
    second: Animated,
    second_at: Option<u32>,
    charge: Animated,
    photo: PhotoSlot,
    cache: Cache,
    minute: Option<u32>,
    drift: (f32, f32),
    scratch: Vec<u32>,
    /// Music's face was turned to by itself (music started on the charger),
    /// and whether music was playing at the last tick.
    auto_music: bool,
    music_seen: bool,
}

impl Default for Faces {
    fn default() -> Self {
        Self::new()
    }
}

impl Faces {
    pub fn new() -> Faces {
        Faces {
            showing: false,
            shown: Animated::new(0.0, role::smooth()).with_rest(0.004),
            pos: Animated::new(0.0, role::page()).with_rest(0.002),
            style_pos: Animated::new(0.0, role::page()).with_rest(0.002),
            pager: Animated::new(0.0, role::effect()).with_rest(0.004),
            pager_left: 0.0,
            night: Animated::new(0.0, detent::Spring::duration_bounce(3.0, 0.0)).with_rest(0.004),
            time: String::new(),
            time_before: String::new(),
            roll: Animated::new(1.0, role::release()).with_rest(0.004),
            second: Animated::new(0.0, role::tick()).with_rest(0.01),
            second_at: None,
            charge: Animated::new(0.0, role::smooth()).with_rest(0.002),
            photo: PhotoSlot::new(),
            cache: Cache { age: f32::MAX, ..Cache::default() },
            minute: None,
            drift: (0.0, 0.0),
            scratch: Vec::new(),
            auto_music: false,
            music_seen: false,
        }
    }

    pub fn showing(&self) -> bool {
        self.showing
    }

    /// Anything of it is on screen.
    pub fn visible(&self) -> bool {
        self.showing || self.shown.value() > 0.004
    }

    /// It covers the whole panel: nothing beneath need be drawn.
    pub fn covers(&self) -> bool {
        self.showing && self.shown.value() >= 0.999
    }

    /// The light StandBy wants now: lower at night (when night is on).
    pub fn light_level(choices: &Choices) -> f32 {
        if choices.night && is_night(local_now()) {
            NIGHT_LEVEL
        } else {
            1.0
        }
    }

    /// The face turned to (or being turned to).
    #[cfg(test)]
    pub fn shown_face(&self) -> Face {
        Face::ALL[self.at()]
    }

    fn at(&self) -> usize {
        (self.pos.target().round().max(0.0) as usize).min(Face::ALL.len() - 1)
    }

    /// Come up, on the face last shown.
    pub fn show(&mut self, choices: &Choices, calm: bool) {
        if self.showing {
            return;
        }
        self.showing = true;
        self.auto_music = false;
        self.music_seen = false;
        let at = choices.face.index() as f32;
        self.pos.jump(at);
        self.style_pos.jump(choices.clock.index() as f32);
        self.pager_left = PAGER_FOR;
        self.pager.set(1.0);
        let night = if choices.night && is_night(local_now()) { 1.0 } else { 0.0 };
        self.night.jump(night);
        self.time.clear();
        self.charge.jump(0.0);
        self.second_at = None;
        self.minute = None;
        self.cache.age = f32::MAX;
        self.cache.stale = true;
        self.photo.listed_age = f32::MAX;
        if calm {
            self.shown.jump(1.0);
        } else {
            self.shown.set(1.0);
        }
    }

    /// Give way (a key, or it came off the charger).
    pub fn hide(&mut self, calm: bool) {
        if !self.showing {
            return;
        }
        self.showing = false;
        if calm {
            self.shown.jump(0.0);
        } else {
            self.shown.set(0.0);
        }
        // Let go of the photos (half a megabyte each) once gone.
        self.photo.shown = None;
        self.photo.before = None;
        self.photo.loading = None;
        self.photo.list.clear();
    }

    /// A key while StandBy shows: the arrows turn faces (↑↓) and styles
    /// (←→); true when it was taken. Any other key is the shell's (it leaves).
    pub fn key(&mut self, key: Key, choices: &mut Choices, calm: bool) -> bool {
        let key = match key {
            Key::Char('f') => Key::Up,
            Key::Char('x') => Key::Down,
            Key::Char('z') => Key::Left,
            Key::Char('c') => Key::Right,
            other => other,
        };
        if !matches!(key, Key::Up | Key::Down | Key::Left | Key::Right) {
            return false;
        }
        self.pager_left = PAGER_FOR;
        self.pager.set(1.0);
        let faces = Face::ALL.len() as i32;
        // Turned by hand: music's face is no longer the one picked for it.
        self.auto_music = false;
        choices.face = Face::ALL[self.at()];
        match key {
            Key::Up | Key::Down => {
                let step = if key == Key::Down { 1 } else { -1 };
                let at = choices.face.index() as i32 + step;
                if (0..faces).contains(&at) {
                    choices.face = Face::ALL[at as usize];
                    self.pos.set(at as f32);
                } else if !calm {
                    // Past either end: a give, and back (rubber-banding).
                    self.pos.nudge(step as f32 * 2.2);
                }
            }
            _ => {
                let step = if key == Key::Right { 1 } else { -1 };
                match choices.face {
                    Face::Clock => {
                        let styles = ClockStyle::ALL.len() as i32;
                        let at = choices.clock.index() as i32 + step;
                        if (0..styles).contains(&at) {
                            choices.clock = ClockStyle::ALL[at as usize];
                            self.style_pos.set(at as f32);
                            self.time_before.clone_from(&self.time);
                        } else if !calm {
                            self.style_pos.nudge(step as f32 * 2.2);
                        }
                    }
                    Face::Photos if self.photo.list.len() > 1 => self.photo.step(step),
                    // Faces of one style give a little, and come back.
                    _ if !calm => self.style_pos.nudge(step as f32 * 1.6),
                    _ => {}
                }
            }
        }
        if calm {
            self.pos.jump(self.pos.target());
            self.style_pos.jump(self.style_pos.target());
        }
        true
    }

    /// Time passing: `dt` on the clock, `md` for motion (none after an idle
    /// spell, so motion a key begins starts from its beginning). Returns
    /// (something moves, a new frame is wanted).
    pub fn tick(&mut self, dt: f32, md: f32, feed: &Feed, choices: &Choices, calm: bool) -> (bool, bool) {
        let mut moving = self.shown.tick(md);
        if !self.visible() {
            // Gone: the picture kept for fading let go too.
            self.scratch = Vec::new();
            return (moving, false);
        }
        let mut changed = false;
        moving |= self.pos.tick(md) | self.style_pos.tick(md);
        // The page dots tuck away a moment after the last key.
        if self.pager_left > 0.0 {
            self.pager_left -= dt;
            if self.pager_left <= 0.0 {
                self.pager.set(0.0);
            }
        }
        moving |= self.pager.tick(md);
        let now = local_now();
        let night = choices.night && is_night(now);
        if (self.night.target() - if night { 1.0 } else { 0.0 }).abs() > 0.5 {
            self.night.set(if night { 1.0 } else { 0.0 });
        }
        moving |= self.night.tick(md);
        // The time: a change rolls in.
        let time = time_text(now, feed.h24);
        if time != self.time {
            if self.time.is_empty() || calm {
                self.time_before.clone_from(&time);
                self.roll.jump(1.0);
            } else {
                self.time_before = std::mem::replace(&mut self.time, time.clone());
                self.roll.jump(0.0);
                self.roll.set(1.0);
            }
            self.time = time;
            changed = true;
        }
        moving |= self.roll.tick(md);
        // The second hand, on the dial by day.
        let dial = choices.face == Face::Clock && choices.clock == ClockStyle::Dial && self.night.value() < 0.5;
        let second = now.minute() * 60 + now.second();
        if dial && self.second_at != Some(second) {
            let ahead = match self.second_at {
                // Round the hour: carried on, not wound back.
                Some(was) if second < was => {
                    self.second.shift(-3600.0);
                    second as f32
                }
                _ => second as f32,
            };
            if self.second_at.is_none() || calm {
                self.second.jump(ahead);
            } else {
                self.second.set(ahead);
            }
            self.second_at = Some(second);
            changed = true;
        }
        if !dial {
            self.second_at = None;
        }
        moving |= self.second.tick(md);
        // The charge.
        let fraction = feed.reading.map_or(0.0, |r| r.percent as f32 / 100.0);
        if (self.charge.target() - fraction).abs() > 0.001 {
            if calm {
                self.charge.jump(fraction);
            } else {
                self.charge.set(fraction);
            }
            changed = true;
        }
        moving |= self.charge.tick(md);
        // What the faces show, read again once a minute (at the next frame).
        self.cache.age += dt;
        if self.cache.age >= 60.0 || feed.generation != self.cache.generation || self.minute != Some(now.minute()) {
            self.cache.age = 0.0;
            self.cache.generation = feed.generation;
            self.minute = Some(now.minute());
            self.cache.stale = true;
            changed = true;
        }
        // Music playing on the charger: its face, picked by itself (not
        // kept as the face chosen); stopped, back to the one chosen.
        let playing = feed.activities.iter().any(|a| a.kind == crate::liveactivity::Kind::Music && a.playing == Some(true));
        let music = Face::Music.index();
        if playing && !self.auto_music && self.at() != music && self.showing && !self.music_seen {
            self.auto_music = true;
            if calm {
                self.pos.jump(music as f32);
            } else {
                self.pos.set(music as f32);
            }
            self.pager_left = PAGER_FOR;
            self.pager.set(1.0);
        }
        if !feed.activities.iter().any(|a| a.kind == crate::liveactivity::Kind::Music) && self.auto_music {
            self.auto_music = false;
            self.pos.set(choices.face.index() as f32);
        }
        self.music_seen = playing;
        // A card that has just changed comes down over the face.
        moving |= feed.open > 0.004 && feed.open < 0.996;
        if feed.opened.is_some() && feed.open > 0.004 {
            changed = true;
        }
        let photos = (self.pos.value() - Face::Photos.index() as f32).abs() < 1.0;
        changed |= self.photo.tick(dt, md, photos && self.showing);
        // The drift, a pixel at a time.
        let drift = if calm { (0.0, 0.0) } else { drift_at(chrono::Utc::now().timestamp() as f64) };
        if drift != self.drift {
            self.drift = drift;
            changed = true;
        }
        if calm {
            for a in [&mut self.pos, &mut self.style_pos, &mut self.roll, &mut self.night, &mut self.pager] {
                a.jump(a.target());
            }
        }
        (moving, changed || moving)
    }

    /// Draw over whatever is on the canvas (the screen beneath shows
    /// through while it comes and goes).
    pub fn draw(&mut self, g: &mut Gfx, robot: &Robot, feed: &Feed, choices: &Choices) {
        let shown = self.shown.value().clamp(0.0, 1.0);
        if shown <= 0.004 {
            return;
        }
        if self.cache.stale {
            self.cache.stale = false;
            let unix = chrono::Utc::now().timestamp();
            self.cache.weather = crate::weather::glance(feed.h24);
            self.cache.next = Some(crate::upnext::next_match(robot, unix));
            self.cache.reminder = crate::upnext::next_reminder(unix);
        }
        let fading = shown < 0.999;
        if fading {
            self.scratch.clone_from(&g.canvas.px);
        }
        let night = self.night.value().clamp(0.0, 1.0);
        // By night the faces are dark whatever the mode, and red.
        let day_tokens = g.tokens;
        if night > 0.0 && day_tokens.tone == Tone::Light {
            g.tokens = Tokens::of(g.accent, Tone::Dark);
        }
        let (dx, dy) = self.drift;
        // Coming, it rises a little into place; going, it drops away
        // (and fades faster than it moves, so two pictures never muddle).
        let rise = if self.showing { (1.0 - shown) * 10.0 } else { (1.0 - shown) * H * 0.35 };
        // Each face in view: the one turned to, and its neighbour while turning.
        let pos = self.pos.value();
        let first = pos.floor().max(0.0) as usize;
        for i in first..=(first + 1).min(Face::ALL.len() - 1) {
            let oy = (i as f32 - pos) * H;
            if oy.abs() >= H {
                continue;
            }
            let face = Face::ALL[i];
            g.canvas.set_clip(Rect::new(0.0, oy.max(0.0), W, H - oy.abs()));
            self.draw_face(g, face, feed, choices, (dx, dy + oy + rise));
            g.canvas.clear_clip();
        }
        // Rubber-banded past either end: the ground shows beyond.
        let last = (Face::ALL.len() - 1) as f32;
        if pos < 0.0 || pos > last {
            let gap = if pos < 0.0 { -pos * H } else { (pos - last) * H };
            let r = if pos < 0.0 { Rect::new(0.0, 0.0, W, gap) } else { Rect::new(0.0, H - gap, W, gap) };
            g.canvas.fill_rect(r, g.tokens.ground, 1.0);
        }
        // A live activity that has just changed: its card down from the top
        // for a moment (music has its own face).
        if let Some(a) = feed.opened.filter(|a| a.kind != crate::liveactivity::Kind::Music) {
            let open = feed.open.clamp(0.0, 1.05);
            if open > 0.004 {
                let r = Rect::new(PAD, (6.0 - (1.0 - open) * 76.0).round(), W - 2.0 * PAD, 58.0);
                let t = g.tokens;
                g.shadow(r, 16.0, open.min(1.0));
                g.canvas.rrect(r, 16.0, t.surface2, 1.0);
                crate::liveactivity::draw_card(g, r.inset(3.0), a, feed.level, open.min(1.0));
            }
        }
        self.draw_pager(g, choices);
        redden(&mut g.canvas.px, night);
        g.tokens = day_tokens;
        if fading {
            let faces = std::mem::replace(&mut g.canvas.px, std::mem::take(&mut self.scratch));
            let alpha = if self.showing { shown } else { shown * shown * shown };
            bezel::simd::mix_into(&mut g.canvas.px, Some(&faces), (alpha * 256.0) as u32);
            self.scratch = faces;
        }
    }

    fn draw_pager(&mut self, g: &mut Gfx, choices: &Choices) {
        let p = self.pager.value().clamp(0.0, 1.0);
        if p <= 0.01 {
            return;
        }
        let n = Face::ALL.len();
        let h = n as f32 * 9.0 + 7.0;
        let platter = Rect::new(W - 15.0, (H - h) * 0.5, 10.0, h);
        // Glass comes and goes by its refraction, never its opacity.
        g.glass(&[GlassShape { strength: p, ..GlassShape::new(platter, 5.0, 5) }]);
        let t = g.tokens;
        let _ = choices;
        let at = self.at();
        for i in 0..n {
            let color = if i == at { t.ink } else { t.faint };
            g.canvas.circle(platter.cx(), platter.y + 8.0 + i as f32 * 9.0, 2.0, color, p);
        }
    }

    fn draw_face(&mut self, g: &mut Gfx, face: Face, feed: &Feed, choices: &Choices, o: (f32, f32)) {
        let t = g.tokens;
        match face {
            Face::Clock => {
                ground(&mut g.canvas, &t, t.signal, o.0, o.1);
                let sp = self.style_pos.value();
                let first = sp.floor().clamp(0.0, (ClockStyle::ALL.len() - 1) as f32) as usize;
                for i in first..=(first + 1).min(ClockStyle::ALL.len() - 1) {
                    let ox = (i as f32 - sp) * W;
                    if ox.abs() >= W {
                        continue;
                    }
                    let style = ClockStyle::ALL[i];
                    self.draw_clock(g, style, feed, (o.0 + ox, o.1));
                }
            }
            Face::Charge => self.draw_charge(g, feed, (o.0 + self.give(), o.1)),
            Face::Match => self.draw_match(g, (o.0 + self.give(), o.1)),
            Face::Weather => self.draw_weather(g, (o.0 + self.give(), o.1)),
            Face::Photos => self.draw_photos(g, feed, o),
            Face::Music => self.draw_music(g, feed, (o.0 + self.give(), o.1)),
            Face::Everything => self.draw_everything(g, feed, choices, (o.0 + self.give(), o.1)),
        }
    }

    /// A face of one style, nudged sideways by ←→: how far it gives.
    fn give(&self) -> f32 {
        (self.style_pos.value() - self.style_pos.target()) * -W * 0.25
    }

    fn date_line(&self, g: &mut Gfx, feed: &Feed, now: NaiveDateTime, room: f32) -> String {
        let charge = feed.reading.filter(|r| !r.flow.on_battery()).map(|r| format!(" · {}%", r.percent)).unwrap_or_default();
        let long = format!("{}{charge}", date_text(now));
        if g.measure(&long, g.ty.caption) <= room {
            long
        } else {
            format!("{}{charge}", short_date(now))
        }
    }

    fn draw_clock(&mut self, g: &mut Gfx, style: ClockStyle, feed: &Feed, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        let now = local_now();
        let roll = self.roll.value();
        match style {
            ClockStyle::Digits => {
                // As large as the panel takes, in Bezel's display voice.
                let base = ty.display(100.0);
                let probe = if feed.h24 { "88:88" } else { "18:88" };
                let style = Tab::sized_to(g, base, probe, W - 2.0 * PAD - 16.0, 118.0);
                let tab = Tab::new(g, style);
                let cap = g.text.cap_height(&style);
                let w = tab.width(&self.time);
                let caption_h = ty.caption.size;
                let block = cap + 18.0 + caption_h;
                let baseline = ((H - block) * 0.5 + cap).round() - 2.0;
                tab.draw(g, &self.time_before, &self.time, roll, o.0 + (W - w) * 0.5, o.1 + baseline, t.ink, 1.0);
                let line = self.date_line(g, feed, now, W - 2.0 * PAD);
                g.text.draw_aligned(&mut g.canvas, o.0 + W * 0.5, o.1 + baseline + 18.0 + caption_h * 0.72, Align::Center, &line, &ty.caption, t.dim, 1.0);
            }
            ClockStyle::Dial => self.draw_dial(g, feed, now, o),
            ClockStyle::Dots => {
                let pitch = 10.0;
                let (cols, rows) = ((W / pitch) as usize, (H / pitch) as usize);
                let caption_y = H - 26.0;
                // The quiet grid, clear of the words beneath.
                for row in 0..rows {
                    for col in 0..cols {
                        let (x, y) = (5.0 + col as f32 * pitch, 5.0 + row as f32 * pitch);
                        if y > caption_y - 12.0 && y < caption_y + 12.0 {
                            continue;
                        }
                        g.canvas.circle(o.0 + x, o.1 + y, 1.1, t.faint, 0.55);
                    }
                }
                draw_dots(g, &self.time_before, &self.time, roll, o.0 + W * 0.5, o.1 + 45.0, pitch, t.ink, 1.0);
                let line = self.date_line(g, feed, now, W - 2.0 * PAD);
                g.text.draw_aligned(&mut g.canvas, o.0 + W * 0.5, o.1 + caption_y + ty.caption.size * 0.36, Align::Center, &line, &ty.caption, t.dim, 1.0);
            }
        }
    }

    /// A Braun-style dial: minute ticks, ink hands with a soft shadow, the
    /// signal second hand stepping into each second; the day beside it.
    fn draw_dial(&mut self, g: &mut Gfx, feed: &Feed, now: NaiveDateTime, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        let (cx, cy, r) = (o.0 + 92.0, o.1 + H * 0.5, 74.0);
        g.canvas.circle(cx, cy, r, t.surface1, 1.0);
        for i in 0..60 {
            let a = i as f32 / 60.0 * std::f32::consts::TAU;
            let five = i % 5 == 0;
            let (outer, inner) = (r - 5.0, r - if five { 12.0 } else { 8.0 });
            let p = |d: f32| (cx + d * a.sin(), cy - d * a.cos());
            g.canvas.polyline(&[p(inner), p(outer)], if five { 2.2 } else { 1.0 }, if five { t.ink } else { t.dim }, 1.0);
        }
        for h in [12, 3, 6, 9] {
            let a = h as f32 / 12.0 * std::f32::consts::TAU;
            let d = r - 25.0;
            g.text.draw_aligned(&mut g.canvas, cx + d * a.sin(), (cy - d * a.cos() + ty.caption.size * 0.36).round(), Align::Center, &h.to_string(), &ty.caption, t.dim, 1.0);
        }
        let minutes = now.minute() as f32 + now.second() as f32 / 60.0;
        let hours = (now.hour() % 12) as f32 + minutes / 60.0;
        let hand = |g: &mut Gfx, turn: f32, length: f32, tail: f32, width: f32, color: Color, shadow: bool| {
            let a = turn * std::f32::consts::TAU;
            let (sx, sy) = (a.sin(), -a.cos());
            let tip = (cx + length * sx, cy + length * sy);
            let back = (cx - tail * sx, cy - tail * sy);
            if shadow {
                g.canvas.polyline(&[(back.0 + 1.0, back.1 + 2.0), (tip.0 + 1.0, tip.1 + 2.0)], width + 1.5, Color::BLACK, 0.28);
            }
            g.canvas.polyline(&[back, tip], width, color, 1.0);
        };
        hand(g, hours / 12.0, 40.0, 8.0, 5.0, t.ink, true);
        hand(g, minutes / 60.0, 60.0, 10.0, 3.4, t.ink, true);
        if self.second_at.is_some() {
            hand(g, self.second.value() / 60.0, 64.0, 16.0, 1.5, t.signal, false);
            g.canvas.circle(cx, cy, 3.6, t.signal, 1.0);
        } else {
            g.canvas.circle(cx, cy, 3.6, t.ink, 1.0);
        }
        // The day, beside the dial.
        let x = o.0 + 192.0;
        let day = now.format("%A").to_string().to_lowercase();
        let big = ty.display(54.0);
        g.text.draw(&mut g.canvas, x, o.1 + 56.0, &day, &ty.caption, t.dim, 1.0);
        g.text.draw(&mut g.canvas, x - 2.0, o.1 + 56.0 + 10.0 + g.text.cap_height(&big), &now.day().to_string(), &big, t.ink, 1.0);
        let month = now.format("%B").to_string().to_lowercase();
        let below = o.1 + 56.0 + 10.0 + g.text.cap_height(&big) + 20.0;
        g.text.draw(&mut g.canvas, x, below, &month, &ty.caption, t.dim, 1.0);
        if let Some(r) = feed.reading.filter(|r| !r.flow.on_battery()) {
            g.text.draw(&mut g.canvas, x, below + ty.caption.size + 6.0, &format!("{}% · {}", r.percent, charge_words(r)), &ty.caption, t.faint.mix(t.dim, 0.5), 1.0);
        }
    }

    fn draw_charge(&mut self, g: &mut Gfx, feed: &Feed, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        ground(&mut g.canvas, &t, t.signal, o.0, o.1);
        let now = local_now();
        // The ring, at the right: the charge, filling in the signal colour;
        // ice once full.
        let (cx, cy, r) = (o.0 + W - PAD - 66.0, o.1 + H * 0.5, 56.0);
        // The time, as large as the room left of the ring takes, and how
        // the charging goes beneath it.
        let x = o.0 + PAD + 8.0;
        let room = cx - r - 14.0 - x;
        let probe = if feed.h24 { "88:88" } else { "18:88" };
        let style = Tab::sized_to(g, ty.display(58.0), probe, room, 58.0);
        let tab = Tab::new(g, style);
        let cap = g.text.cap_height(&style);
        let line = ty.caption.size + 6.0;
        let baseline = (o.1 + (H - (cap + 22.0 + line)) * 0.5 + cap).round();
        tab.draw(g, &self.time_before, &self.time, self.roll.value(), x, baseline, t.ink, 1.0);
        let line_y = baseline + 22.0;
        g.text.draw(&mut g.canvas, x + 1.0, line_y, &short_date(now), &ty.caption, t.dim, 1.0);
        if let Some(words) = feed.reading.and_then(Reading::draw_line) {
            let words = g.text.fit(&words, &ty.caption, room);
            g.text.draw(&mut g.canvas, x + 1.0, line_y + line, &words, &ty.caption, t.faint.mix(t.dim, 0.5), 1.0);
        }
        // The track: the meter's tone on dark; on light ground, where the
        // meter's own is all but the ground's, a step darker.
        let track = if t.tone == Tone::Dark { t.meter } else { t.surface3.mix(t.dim, 0.18) };
        g.canvas.arc(cx, cy, r, 9.0, 0.0, std::f32::consts::TAU, track, 1.0);
        let full = feed.reading.is_some_and(|r| r.flow == Flow::Full || r.percent >= 100);
        let fraction = self.charge.value().clamp(0.0, 1.0);
        if fraction > 0.005 {
            g.canvas.arc(cx, cy, r, 9.0, 0.0, std::f32::consts::TAU * fraction, if full { t.ice } else { t.signal }, 1.0);
        }
        let (percent, under) = match feed.reading {
            Some(r) => (format!("{}%", (fraction * 100.0).round() as u32), charge_words(r)),
            None => ("–".into(), "no battery gauge".into()),
        };
        let big = ty.display(30.0);
        let pc = g.text.cap_height(&big);
        g.icon_alpha(if full { "battery_full" } else { "bolt" }, true, cx, cy - pc * 0.5 - 13.0, 14.0, if full { t.ice } else { t.signal }, 1.0);
        g.text.draw_aligned(&mut g.canvas, cx, cy + pc * 0.5, Align::Center, &percent, &big, t.ink, 1.0);
        let under = g.text.fit(&under, &ty.caption, r * 1.6);
        g.text.draw_aligned(&mut g.canvas, cx, cy + pc * 0.5 + 17.0, Align::Center, &under, &ty.caption, t.dim, 1.0);
    }

    fn draw_match(&mut self, g: &mut Gfx, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        ground(&mut g.canvas, &t, t.ice, o.0, o.1);
        let m = match self.cache.next.clone() {
            Some(Ok(m)) => m,
            Some(Err(why)) => {
                let hint = if why == "no team set" { "set it in Settings › Robot" } else { "the schedule shows here once it is out" };
                return empty(g, o, "event", &why, hint);
            }
            None => return,
        };
        let x = o.0 + PAD + 8.0;
        // Left: whose event, the match, the alliance and when, with whom.
        let room = 176.0;
        let head = m.event.clone().map_or_else(|| format!("team {}", m.team), |e| e.to_lowercase());
        let head = if g.measure(&head, ty.caption) <= room { head } else { format!("team {} · next match", m.team) };
        g.text.draw(&mut g.canvas, x, o.1 + 24.0, &head, &ty.caption, t.dim, 1.0);
        let big = ty.display(56.0);
        let cap = g.text.cap_height(&big);
        let baseline = o.1 + 34.0 + cap;
        g.text.draw(&mut g.canvas, x - 2.0, baseline, &m.label, &big, t.ink, 1.0);
        let mut y = baseline + 20.0;
        g.canvas.circle(x + 3.0, y - ty.caption.size * 0.36, 3.5, m.color, 1.0);
        let side = m.side.map_or_else(|| m.when.clone(), |s| format!("{s} · {}", m.when));
        g.text.draw(&mut g.canvas, x + 12.0, y, &side, &ty.caption, t.ink, 1.0);
        let line = ty.caption.size + 6.0;
        y += line;
        let rows: Vec<(&str, String, bool)> = [
            Some(("with", crate::upnext::teams(&m.with), false)),
            Some(("against", crate::upnext::teams(&m.against), false)),
            m.queue.clone().map(|(name, value)| (name, value, true)),
        ]
        .into_iter()
        .flatten()
        .collect();
        for (name, value, warn) in rows {
            if y > o.1 + H - 8.0 {
                break;
            }
            let w = g.text.draw(&mut g.canvas, x, y, name, &ty.caption, t.dim, 1.0);
            g.text.draw(&mut g.canvas, x + w + 8.0, y, &value, &ty.caption, if warn { t.amber_ink } else { t.ink }, 1.0);
            y += line;
        }
        // Right: the matches after it, on a panel of their own, each its
        // label and, beneath, when.
        let panel = Rect::new(o.0 + 204.0, o.1 + 14.0, W - 204.0 - PAD - 6.0, H - 28.0);
        g.canvas.rrect(panel, g.tile_radius(), t.surface1, 1.0);
        g.text.draw(&mut g.canvas, panel.x + 12.0, panel.y + 20.0, "after", &ty.caption, t.dim, 1.0);
        let row_h = (panel.h - 30.0) / 3.0;
        if m.after.is_empty() {
            g.text.draw(&mut g.canvas, panel.x + 12.0, panel.y + 30.0 + row_h * 0.6, "the last one", &ty.caption, t.dim, 1.0);
        }
        for (i, (label, when, side)) in m.after.iter().enumerate() {
            let top = panel.y + 28.0 + row_h * i as f32;
            let cy = top + ty.body.size * 0.6;
            g.canvas.circle(panel.x + 15.0, cy, 3.0, crate::upnext::side_color(*side), 1.0);
            g.text.draw(&mut g.canvas, panel.x + 24.0, (cy + ty.body.size * 0.36).round(), label, &ty.body, t.ink, 1.0);
            let when = g.text.fit(when, &ty.caption, panel.w - 34.0);
            g.text.draw(&mut g.canvas, panel.x + 24.0, (cy + ty.body.size * 0.5 + 4.0 + ty.caption.size * 0.72).round(), &when, &ty.caption, t.dim, 1.0);
        }
    }

    fn draw_weather(&mut self, g: &mut Gfx, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        ground(&mut g.canvas, &t, t.amber, o.0, o.1);
        let Some(w) = self.cache.weather.clone() else {
            return empty(g, o, "cloud_off", "no forecast yet", "open Weather once to fetch one");
        };
        let x = o.0 + PAD + 8.0;
        g.text.draw(&mut g.canvas, x, o.1 + 24.0, &w.place.to_lowercase(), &ty.caption, t.dim, 1.0);
        let big = ty.display(64.0);
        let cap = g.text.cap_height(&big);
        let baseline = o.1 + 36.0 + cap;
        g.icon_alpha(w.icon, true, x + 20.0, baseline - cap * 0.5, 40.0, t.amber, 1.0);
        g.text.draw(&mut g.canvas, x + 48.0, baseline, &w.temp, &big, t.ink, 1.0);
        let mut y = baseline + 22.0;
        g.text.draw(&mut g.canvas, x, (y - 2.0).round(), &w.condition.to_lowercase(), &ty.body, t.ink, 1.0);
        y += ty.body.size + 4.0;
        let detail = match (&w.range, w.rain) {
            (Some(range), Some(rain)) => format!("{range} · rain {rain}%"),
            (Some(range), None) => range.clone(),
            (None, Some(rain)) => format!("rain {rain}%"),
            (None, None) => String::new(),
        };
        let detail = g.text.fit(&detail, &ty.caption, 186.0);
        g.text.draw(&mut g.canvas, x, y, &detail, &ty.caption, t.dim, 1.0);
        // The next hours, down a panel.
        let panel = Rect::new(o.0 + 212.0, o.1 + 14.0, W - 212.0 - PAD - 6.0, H - 28.0);
        g.canvas.rrect(panel, g.tile_radius(), t.surface1, 1.0);
        let row_h = (panel.h - 8.0) / 4.0;
        for (i, (hour, icon, temp)) in w.hours.iter().enumerate().take(4) {
            let cy = panel.y + 4.0 + row_h * (i as f32 + 0.5);
            g.text.draw(&mut g.canvas, panel.x + 10.0, (cy + ty.caption.size * 0.36).round(), hour, &ty.caption, t.dim, 1.0);
            g.icon_alpha(icon, true, panel.x + 46.0, cy, 15.0, t.ink, 1.0);
            g.text.draw_aligned(&mut g.canvas, panel.right() - 10.0, (cy + ty.label.size * 0.36).round(), Align::Right, temp, &ty.label, t.ink, 1.0);
        }
    }

    fn draw_photos(&mut self, g: &mut Gfx, feed: &Feed, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        let full = Rect::new(0.0, o.1 - self.drift.1, W, H);
        g.canvas.fill_rect(full, Color::BLACK, 1.0);
        let fade = self.photo.fade.value().clamp(0.0, 1.0);
        if let Some(before) = &self.photo.before {
            crate::photos::draw_picture(g, full, before, crate::photos::whole(before), 0.0, 1.0);
        }
        let Some(pic) = &self.photo.shown else {
            if self.photo.loading.is_none() && self.photo.listed_age != f32::MAX && self.photo.list.is_empty() {
                ground(&mut g.canvas, &t, t.ice, o.0, o.1);
                empty(g, o, "photo_library", "no photos yet", "the Camera keeps them here");
            }
            return;
        };
        crate::photos::draw_picture(g, full, pic, crate::photos::whole(pic), 0.0, fade);
        // A scrim along the foot, so the time reads over any photo.
        for i in 0..84 {
            let k = i as f32 / 84.0;
            g.canvas.fill_rect(Rect::new(0.0, full.y + H - 84.0 + i as f32, W, 1.0), Color::BLACK, 0.72 * k.powf(1.6));
        }
        let style = ty.display(44.0);
        let tab = Tab::new(g, style);
        let baseline = o.1 + H - 30.0;
        tab.draw(g, &self.time_before, &self.time, self.roll.value(), o.0 + PAD + 6.0, baseline, Color::WHITE, 1.0);
        let line = self.date_line(g, feed, local_now(), 170.0);
        g.text.draw(&mut g.canvas, o.0 + PAD + 8.0, baseline + 18.0, &line, &ty.caption, Color::hex(0xd8dbde), 1.0);
    }

    /// Now playing: the cover large, the song, its time going by, and the
    /// keys that work on it (Fn + W, Q, E), with the time small above.
    fn draw_music(&mut self, g: &mut Gfx, feed: &Feed, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        ground(&mut g.canvas, &t, t.signal, o.0, o.1);
        let Some(a) = feed.activities.iter().find(|a| a.kind == crate::liveactivity::Kind::Music) else {
            return empty(g, o, "music_note", "nothing playing", "a song from Music shows here");
        };
        let size = 118.0;
        let (cx, cy) = (o.0 + PAD + 8.0 + size * 0.5, o.1 + H * 0.5);
        crate::liveactivity::draw_mark(g, cx, cy, size, &a.mark, 1.0);
        let x = o.0 + PAD + 8.0 + size + 16.0;
        let room = o.0 + W - PAD - 8.0 - x;
        // The time, small, at the top right: StandBy is a clock too.
        g.text.draw_aligned(&mut g.canvas, o.0 + W - PAD - 8.0, o.1 + 20.0, Align::Right, &self.time, &ty.label, t.dim, 1.0);
        let lines = crate::help::wrap(g, &a.title, ty.name, room);
        let mut y = o.1 + 46.0;
        for line in lines.iter().take(2) {
            g.text.draw(&mut g.canvas, x, y, line, &ty.name, t.ink, 1.0);
            y += ty.name.size + 4.0;
        }
        let detail = g.text.fit(&a.detail, &ty.caption, room);
        g.text.draw(&mut g.canvas, x, y, &detail, &ty.caption, t.dim, 1.0);
        // The song's time going by.
        let bar = Rect::new(x, o.1 + 104.0, room, 3.0);
        g.canvas.rrect(bar, 1.5, t.meter, 1.0);
        let done = a.meter.map_or(0.0, |m| m.now());
        g.canvas.rrect_clipped(Rect::new(bar.x, bar.y, (bar.w * done).max(3.0), bar.h), 1.5, bar, 1.5, t.signal, 1.0);
        if let Some(np) = crate::nowplaying::get().filter(|n| n.duration > 0.0) {
            let at = np.position_now();
            let times_y = bar.bottom() + 6.0 + ty.caption.size * 0.72;
            g.text.draw(&mut g.canvas, x, times_y, &crate::liveactivity::clock_text(at as f64), &ty.caption, t.dim, 1.0);
            let left = format!("−{}", crate::liveactivity::clock_text((np.duration - at).max(0.0) as f64));
            g.text.draw_aligned(&mut g.canvas, x + room, times_y, Align::Right, &left, &ty.caption, t.dim, 1.0);
        }
        // The keys: previous, play or pause, next.
        let xs = [x + room * 0.2, x + room * 0.5, x + room * 0.8];
        crate::liveactivity::draw_music_keys(g, xs, o.1 + 138.0, a.playing == Some(true), 17.0, 1.0);
    }

    /// A little of everything: the time and the day along the top, and a
    /// tile each for the charge, the next match, the weather and the next
    /// reminder (those there are), two by two.
    fn draw_everything(&mut self, g: &mut Gfx, feed: &Feed, _choices: &Choices, o: (f32, f32)) {
        let t = g.tokens;
        let ty = g.ty;
        ground(&mut g.canvas, &t, t.signal, o.0, o.1);
        let now = local_now();
        let x = o.0 + PAD + 6.0;
        let right = o.0 + W - PAD - 6.0;
        let style = ty.display(40.0);
        let tab = Tab::new(g, style);
        let cap = g.text.cap_height(&style);
        let baseline = (o.1 + 12.0 + cap).round();
        tab.draw(g, &self.time_before, &self.time, self.roll.value(), x, baseline, t.ink, 1.0);
        // The day, and notices kept, to the right of the time.
        let room = right - (x + tab.width(&self.time) + 12.0);
        let day = date_text(now);
        let day = if g.measure(&day, ty.caption) <= room { day } else { short_date(now) };
        let first = if feed.notices > 0 { baseline - cap * 0.72 } else { baseline - cap * 0.4 };
        g.text.draw_aligned(&mut g.canvas, right, (first + ty.caption.size * 0.36).round(), Align::Right, &day, &ty.caption, t.dim, 1.0);
        if feed.notices > 0 {
            let words = if feed.notices == 1 { "1 notice".to_owned() } else { format!("{} notices", feed.notices) };
            let w = g.measure(&words, ty.caption);
            let cy = baseline - cap * 0.12;
            g.icon_alpha("notifications", true, right - w - 9.0, cy, 12.0, t.ink, 1.0);
            g.text.draw_aligned(&mut g.canvas, right, (cy + ty.caption.size * 0.36).round(), Align::Right, &words, &ty.caption, t.ink, 1.0);
        }
        // The tiles: (icon or dot, the value, what it is).
        let unix = chrono::Utc::now().timestamp();
        let mut tiles: Vec<(&str, Option<Color>, String, String)> = Vec::new();
        if let Some(r) = feed.reading {
            tiles.push((if r.flow.on_battery() { "battery_full" } else { "bolt" }, None, format!("{}%", r.percent), charge_words(r)));
        }
        if let Some(Ok(m)) = &self.cache.next {
            let side = m.side.map_or_else(|| m.when.clone(), |s| format!("{s} · {}", m.when));
            tiles.push(("", Some(m.color), m.label.clone(), side));
        }
        if let Some(w) = &self.cache.weather {
            tiles.push((w.icon, None, w.temp.clone(), w.condition.to_lowercase()));
        }
        if let Some(r) = &self.cache.reminder {
            let when = r.due.map_or_else(|| "to do".to_owned(), |at| crate::schedule::when(at, unix));
            tiles.push(("checklist", None, when, r.title.clone()));
        }
        let gap = 6.0;
        let top = baseline + 14.0;
        let tw = ((right - x) - gap) * 0.5;
        let th = ((o.1 + H - PAD - 2.0 - top) - gap) * 0.5;
        for (i, (icon, dot, value, what)) in tiles.into_iter().enumerate().take(4) {
            let r = Rect::new(x + (i % 2) as f32 * (tw + gap), top + (i / 2) as f32 * (th + gap), tw, th);
            g.canvas.rrect(r, g.radius(12.0).min(th * 0.5), t.surface1, 1.0);
            let cy = r.y + th * 0.36;
            match dot {
                Some(c) => g.canvas.circle(r.x + 15.0, cy, 4.0, c, 1.0),
                None => g.icon_alpha(icon, true, r.x + 15.0, cy, 15.0, t.dim, 1.0),
            }
            let value = g.text.fit(&value, &ty.name, r.w - 38.0);
            g.text.draw(&mut g.canvas, r.x + 28.0, (cy + ty.name.size * 0.36).round(), &value, &ty.name, t.ink, 1.0);
            let what = g.text.fit(&what, &ty.caption, r.w - 22.0);
            g.text.draw(&mut g.canvas, r.x + 11.0, (r.y + th * 0.74 + ty.caption.size * 0.36).round(), &what, &ty.caption, t.dim, 1.0);
        }
    }
}

/// How the charging goes: "full in 40 m", "charged", "plugged in".
pub fn charge_words(r: &Reading) -> String {
    match r.flow {
        Flow::Charging => r.minutes_to_full.map_or_else(|| "charging".into(), |m| format!("full in {}", crate::sys::minutes_text(m))),
        Flow::Full => "charged".into(),
        Flow::Holding => "plugged in".into(),
        Flow::Discharging => r.minutes_left.map_or_else(|| "on battery".into(), |m| format!("{} left", crate::sys::minutes_text(m))),
        Flow::Unknown => "–".into(),
    }
}

/// A face with nothing to show yet: what, and where it comes from.
fn empty(g: &mut Gfx, o: (f32, f32), icon: &str, line: &str, hint: &str) {
    let t = g.tokens;
    let ty = g.ty;
    let cy = o.1 + H * 0.5;
    g.icon_alpha(icon, false, o.0 + W * 0.5, cy - 24.0, 28.0, t.dim, 1.0);
    g.text.draw_aligned(&mut g.canvas, o.0 + W * 0.5, (cy + 8.0 + ty.body.size * 0.36).round(), Align::Center, line, &ty.body, t.ink, 1.0);
    let hint = g.text.fit(hint, &ty.caption, W - 2.0 * PAD);
    g.text.draw_aligned(&mut g.canvas, o.0 + W * 0.5, (cy + 28.0 + ty.caption.size * 0.36).round(), Align::Center, &hint, &ty.caption, t.dim, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn night_is_ten_to_seven_by_the_clock() {
        assert!(!is_night(at(21, 59)) && is_night(at(22, 0)) && is_night(at(3, 0)) && is_night(at(6, 59)) && !is_night(at(7, 0)));
        assert_eq!(night_words(false), "10 pm to 7 am");
        assert_eq!(night_words(true), "22:00 to 7:00");
    }

    #[test]
    fn the_drift_is_small_whole_pixels_and_slow() {
        let mut steps = 0;
        let mut last = drift_at(0.0);
        for s in 0..3600 {
            let d = drift_at(s as f64);
            assert!(d.0.abs() <= 3.0 && d.1.abs() <= 2.0 && d.0.fract() == 0.0 && d.1.fract() == 0.0);
            assert!((d.0 - last.0).abs() <= 1.0 && (d.1 - last.1).abs() <= 1.0, "a pixel at a time");
            if d != last {
                steps += 1;
            }
            last = d;
        }
        assert!((10..=60).contains(&steps), "a few steps an hour, not a shimmer: {steps}");
    }

    #[test]
    fn dot_figures_set_the_time_in_dots() {
        let (cols, lit) = dot_time("12:47");
        assert_eq!(cols, 25);
        assert!(lit.contains(&(12, 2)) && lit.contains(&(12, 4)), "the colon");
        let (cols, _) = dot_time("9:41");
        assert_eq!(cols, 19);
        assert_eq!(dot_time("8").1.len(), 17);
    }

    #[test]
    fn night_turns_the_picture_red() {
        let mut px = vec![0xffffff, 0x2080ff, 0x000000];
        redden(&mut px, 1.0);
        let (r, g, b) = ((px[0] >> 16) & 0xff, (px[0] >> 8) & 0xff, px[0] & 0xff);
        assert!(r > 240 && g < 40 && b < 20, "{:06x}", px[0]);
        assert_eq!(px[2], 0);
        let mut day = vec![0x2080ffu32];
        redden(&mut day, 0.0);
        assert_eq!(day[0], 0x2080ff);
    }

    #[test]
    fn every_face_draws_at_every_text_size_by_day_and_night_with_its_icons() {
        crate::shell::tests::sandbox();
        pretend_time(Some(at(9, 41)));
        let robot = Robot::demo();
        let charging = Reading { flow: Flow::Charging, minutes_to_full: Some(42), ..Reading::demo() };
        for tone in [Tone::Dark, Tone::Light] {
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                let mut g = Gfx::new(Tokens::of(bezel::Accent::Orange, tone));
                g.set_text_size(size);
                for night in [false, true] {
                    pretend_time(Some(if night { at(23, 18) } else { at(9, 41) }));
                    for (i, face) in Face::ALL.iter().enumerate() {
                        for clock in ClockStyle::ALL {
                            let choices = Choices { face: *face, clock: *clock, ..Choices::default() };
                            let mut faces = Faces::new();
                            faces.show(&choices, true);
                            let feed = Feed { reading: Some(&charging), h24: false, notices: 2, generation: 0, activities: &[], opened: None, open: 0.0, level: 0.0 };
                            faces.tick(0.1, 0.1, &feed, &choices, true);
                            assert_eq!(faces.pos.value(), i as f32);
                            faces.draw(&mut g, &robot, &feed, &choices);
                            // The 12-hour clock's widest reading fits too.
                            let style = g.ty.display(58.0);
                            assert!(Tab::new(&mut g, style).width("12:59") < W);
                        }
                    }
                }
                assert!(g.missing_icons().is_empty(), "{:?}", g.missing_icons());
            }
        }
        pretend_time(None);
    }

    #[test]
    fn the_arrows_turn_faces_and_styles_and_other_keys_are_not_taken() {
        let mut faces = Faces::new();
        let mut c = Choices::default();
        faces.show(&c, true);
        assert!(faces.key(Key::Down, &mut c, false));
        assert_eq!(c.face, Face::Music, "the song playing comes after the clock");
        assert!(faces.key(Key::Char('f'), &mut c, false), "the bare arrow keys too");
        assert_eq!(c.face, Face::Clock);
        assert!(faces.key(Key::Up, &mut c, false), "past the first: a give, no change");
        assert_eq!(c.face, Face::Clock);
        assert!(faces.key(Key::Right, &mut c, false));
        assert_eq!(c.clock, ClockStyle::Dial);
        for key in [Key::Enter, Key::Back, Key::Char(' '), Key::Ctrl('\t'), Key::Char('a')] {
            assert!(!faces.key(key, &mut c, false), "{key:?} leaves StandBy");
        }
        faces.hide(true);
        assert!(!faces.visible());
    }

    #[test]
    fn the_time_and_date_read_plainly() {
        assert_eq!(time_text(at(9, 41), false), "9:41");
        assert_eq!(time_text(at(21, 41), false), "9:41");
        assert_eq!(time_text(at(21, 41), true), "21:41");
        assert_eq!(date_text(at(9, 41)), "saturday 21 november");
        assert_eq!(short_date(at(9, 41)), "sat 21 nov");
    }
}
