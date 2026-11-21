//! The Level and the Compass, read from the motion sensor ([`crate::imu`]).
//!
//! The Level reads how far the handheld is tipped, to a tenth of a degree.
//! Lying (near enough) flat it is a bubble level: pitch and roll. Stood on an
//! edge it reads that edge's angle to level, as a spirit level laid along a
//! frame or an arm does. Space zeroes it against whatever it rests on, so it
//! then reads how far something else is from that; Enter holds the reading.
//!
//! The Compass reads the top edge's magnetic heading, held level against the
//! way up so it stays true as the handheld tips. A magnetometer is pulled off
//! true by the metal around it (the battery, the speaker's magnet); turning
//! the handheld round once, flat, measures that pull so it can be taken away
//! (a hard-iron calibration, kept beside the settings). Offered only where a
//! magnetometer answers.

use std::path::PathBuf;

use bezel::{Align, Color, Rect, TextStyle};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::imu::{self, Vec3};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The smoothing's time constant: steady to a tenth of a degree, yet quick
/// to follow a hand. (My own tuning.)
const SMOOTH: f32 = 0.15;
/// How far from level the bubble reaches the dial's rim, in degrees.
const DIAL_RANGE: f32 = 10.0;
/// Within this of level, it is level.
const LEVEL_WITHIN: f32 = 0.2;
/// Turning faster than this (degrees a second), the reading is on the move.
const MOVING: f32 = 15.0;
/// The bubble's radius.
const BUBBLE: f32 = 9.0;

/// How much of the way to a new reading the smoothed one goes in `dt`.
fn follow(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt.max(0.0) / tau).exp()
}

/// The dial both draw: a circle as tall as the page allows above the hint,
/// at the left; and the column beside it.
fn dial(g: &Gfx) -> (f32, f32, f32, Rect) {
    let top = g.top();
    let bottom = H - g.hint_h() - 4.0;
    let r = ((bottom - top) * 0.5).floor();
    let (cx, cy) = (PAD + 6.0 + r, top + r);
    let x = cx + r + 18.0;
    (cx, cy, r, Rect::new(x, top, W - PAD - x, bottom - top))
}

/// A capsule with a word in it, centred on `cx` (or from `x`, left).
fn chip(g: &mut Gfx, x: f32, cy: f32, words: &str, fill: Color, ink: Color, alpha: f32) {
    let style = g.ty.caption;
    let w = g.measure(words, style) + 14.0;
    let h = style.size + 7.0;
    let r = Rect::new(x, cy - h * 0.5, w, h);
    g.canvas.rrect(r.offset(g.ox, 0.0), h * 0.5, fill, alpha);
    g.text_alpha(r.x + 7.0, r.cy(), words, style, ink, alpha);
}

/// Centred text at an opacity.
fn centred(g: &mut Gfx, cx: f32, cy: f32, s: &str, style: TextStyle, color: Color, alpha: f32) {
    let w = g.measure(s, style);
    g.text_alpha(cx - w * 0.5, cy, s, style, color, alpha);
}

/// An angle for the page: a tenth of a degree, and never "-0.0".
fn degrees(v: f32) -> String {
    let v = (v * 10.0).round() / 10.0;
    format!("{:.1}°", if v == 0.0 { 0.0 } else { v })
}

/// Keep what is shown unless the reading moved more than a little past the
/// next tenth: the last digit does not flicker between two.
fn steady(shown: f32, now: f32) -> f32 {
    if (now - shown).abs() >= 0.07 {
        now
    } else {
        shown
    }
}

// ------------------------------------------------------------------ level ---

/// Lying flat, or standing on an edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Flat,
    Edge,
}

pub struct Level {
    _hold: imu::Hold,
    /// The way up, smoothed; none until the first reading.
    up: Option<Vec3>,
    /// How fast it is turning, smoothed.
    turning: f32,
    view: View,
    /// What Space zeroed against in each view: pitch and roll, the edge's angle.
    zero_flat: Option<(f32, f32)>,
    zero_edge: Option<f32>,
    /// The reading (pitch and roll; or the edge's angle, and 0), smoothed:
    /// what the bubble and the line follow. And the numbers shown, which
    /// move a tenth at a time.
    now: (f32, f32),
    shown: (f32, f32),
    held: bool,
    /// The view fading in after it changed.
    fade: Animated,
    /// 1 while level: the bubble's colour glides to ice.
    level: Animated,
}

impl Level {
    pub fn new() -> Self {
        Level {
            _hold: imu::hold(imu::Use::Motion),
            up: None,
            turning: 0.0,
            view: View::Flat,
            zero_flat: None,
            zero_edge: None,
            now: (0.0, 0.0),
            shown: (0.0, 0.0),
            held: false,
            fade: Animated::new(1.0, role::smooth()).with_rest(0.01),
            level: Animated::new(0.0, role::smooth()).with_rest(0.01),
        }
    }

    /// The reading now from the smoothed way up, less the zero.
    fn reading(&self, up: Vec3) -> (f32, f32) {
        match self.view {
            View::Flat => {
                let (pitch, roll) = imu::tilt(up);
                let (p0, r0) = self.zero_flat.unwrap_or((0.0, 0.0));
                (pitch - p0, roll - r0)
            }
            View::Edge => {
                let e = imu::edge(up) - self.zero_edge.unwrap_or(0.0);
                ((e + 180.0).rem_euclid(360.0) - 180.0, 0.0)
            }
        }
    }

    fn is_level(&self) -> bool {
        self.up.is_some() && self.shown.0.abs() < LEVEL_WITHIN && self.shown.1.abs() < LEVEL_WITHIN
    }

    fn zeroed(&self) -> bool {
        match self.view {
            View::Flat => self.zero_flat.is_some(),
            View::Edge => self.zero_edge.is_some(),
        }
    }

    /// Where the bubble sits, as a share of the way to the rim: towards
    /// the high side, as a bubble does.
    fn bubble(&self) -> (f32, f32) {
        let (pitch, roll) = self.now;
        let (x, y) = (roll / DIAL_RANGE, -pitch / DIAL_RANGE);
        let d = (x * x + y * y).sqrt();
        if d > 1.0 {
            (x / d, y / d)
        } else {
            (x, y)
        }
    }

    fn draw_flat(&mut self, g: &mut Gfx, alpha: f32) {
        let (t, ty) = (g.tokens, g.ty);
        let (cx, cy, r, side) = dial(g);
        let ox = g.ox;
        let tau = std::f32::consts::TAU;
        g.canvas.circle(cx + ox, cy, r, t.surface1, alpha);
        // Crosshairs, and a ring at 5° (the rim is 10°).
        g.canvas.fill_rect(Rect::new(cx + ox - r + 8.0, cy - 0.5, 2.0 * r - 16.0, 1.0), t.track, alpha);
        g.canvas.fill_rect(Rect::new(cx + ox - 0.5, cy - r + 8.0, 1.0, 2.0 * r - 16.0), t.track, alpha);
        let reach = r - BUBBLE - 4.0;
        g.canvas.arc(cx + ox, cy, reach * 0.5 + BUBBLE * 0.5, 1.0, 0.0, tau, t.track, alpha);
        let level = self.level.value().clamp(0.0, 1.0);
        // The ring the bubble sits in when level, and the bubble.
        g.canvas.arc(cx + ox, cy, BUBBLE + 3.0, 1.5, 0.0, tau, t.dim.mix(t.ice, level), alpha);
        let (bx, by) = self.bubble();
        g.canvas.circle(cx + bx * reach + ox, cy + by * reach, BUBBLE, t.signal.mix(t.ice, level), alpha);

        // Beside it: the two readings, large, each under its name; then
        // how the reading stands.
        let big = ty.display(30.0);
        let (label_h, value_h) = (ty.caption.size + 4.0, big.size * 0.85);
        for (i, (name, v)) in [("pitch", self.shown.0), ("roll", self.shown.1)].into_iter().enumerate() {
            let y = side.y + i as f32 * (label_h + value_h + 4.0);
            g.text_alpha(side.x, y + label_h * 0.5, name, ty.caption, t.dim, alpha);
            let text = degrees(v);
            let style = if g.measure(&text, big) > side.w { ty.display(24.0) } else { big };
            g.text_alpha(side.x - 1.0, y + label_h + value_h * 0.5, &text, style, t.ink, alpha);
        }
        self.draw_state(g, side.x, side.bottom() - ty.caption.size * 0.5 - 4.0, alpha);
    }

    fn draw_edge(&mut self, g: &mut Gfx, alpha: f32) {
        let (t, ty) = (g.tokens, g.ty);
        let top = g.top();
        let bottom = H - g.hint_h() - 4.0;
        let (cx, cy) = (W * 0.5, (top + bottom) * 0.5);
        let ox = g.ox;
        // The world's level, a line across the page that stays level as the
        // handheld turns; and short marks where the handheld's own level is.
        let a = self.now.0.to_radians();
        let (s, c) = a.sin_cos();
        let reach = W * 0.62;
        let level = self.level.value().clamp(0.0, 1.0);
        g.canvas.polyline(&[(cx + ox - reach * c, cy + reach * s), (cx + ox + reach * c, cy - reach * s)], 2.0, t.signal.mix(t.ice, level), alpha);
        for x in [PAD + 2.0, W - PAD - 18.0] {
            g.canvas.fill_rect(Rect::new(x + ox, cy - 0.75, 16.0, 1.5), t.dim, alpha);
        }
        // The angle, large, on a plate so the line passes behind it.
        let big = ty.display(40.0);
        let text = degrees(self.shown.0.abs());
        let w = g.measure(&text, big);
        let plate = Rect::new(cx - w * 0.5 - 12.0, cy - big.size * 0.62, w + 24.0, big.size * 1.1);
        g.canvas.rrect(plate.offset(ox, 0.0), g.radius(10.0), t.surface1, alpha);
        centred(g, cx, cy, &text, big, t.ink, alpha);
        let side = if self.is_level() {
            "level"
        } else if self.shown.0 > 0.0 {
            "right end low"
        } else {
            "left end low"
        };
        centred(g, cx, plate.bottom() + ty.caption.size * 0.5 + 5.0, side, ty.caption, t.dim, alpha);
        let x = PAD;
        self.draw_state(g, x, top + ty.caption.size * 0.5 + 1.0, alpha);
    }

    /// What state the reading is in, as a chip: held, level, moving, zeroed.
    fn draw_state(&self, g: &mut Gfx, x: f32, cy: f32, alpha: f32) {
        let t = g.tokens;
        if self.held {
            chip(g, x, cy, "held", t.surface3, t.ink, alpha);
        } else if self.is_level() {
            chip(g, x, cy, "level", t.ice, t.on_ice, alpha);
        } else if self.turning > MOVING {
            chip(g, x, cy, "moving", t.surface2, t.dim, alpha);
        } else if self.zeroed() {
            chip(g, x, cy, "zeroed", t.surface3, t.ink, alpha);
        }
    }
}

impl Screen for Level {
    fn title(&self) -> &str {
        "Level"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                self.held = !self.held;
                *cx.say = Some(if self.held { "held".into() } else { "live".into() });
            }
            Key::Char(' ') if !self.held => {
                let Some(up) = self.up else { return Nav::Stay };
                let was = self.zeroed();
                match self.view {
                    View::Flat => self.zero_flat = if was { None } else { Some(imu::tilt(up)) },
                    View::Edge => self.zero_edge = if was { None } else { Some(imu::edge(up)) },
                }
                self.now = self.reading(up);
                self.shown = self.now;
                *cx.say = Some(if was { "zero cleared".into() } else { "zeroed here".into() });
            }
            _ => {}
        }
        Nav::Stay
    }

    fn busy(&self) -> bool {
        !self.held
    }

    /// While the reading is live, someone is measuring with it.
    fn keeps_awake(&self) -> bool {
        !self.held
    }

    /// Not kept among recent apps: leaving it lets the sensor sleep.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut redraw = self.fade.tick(dt) | self.level.tick(dt);
        if self.held {
            return redraw;
        }
        let Some(r) = imu::reading() else { return redraw };
        let k = follow(dt, SMOOTH);
        let up = match self.up {
            Some(u) => u.add(r.up.sub(u).scale(k)),
            None => {
                redraw = true;
                r.up
            }
        };
        self.up = Some(up);
        self.turning += (r.turn.len() - self.turning) * k;
        // Flat within 35° of face up (or down); on an edge past 55°; in
        // between, as it was, so it does not flick from one to the other.
        let flat = up.unit().map_or(1.0, |u| u.z.abs());
        let view = if flat > 0.82 {
            View::Flat
        } else if flat < 0.57 {
            View::Edge
        } else {
            self.view
        };
        if view != self.view {
            self.view = view;
            self.fade.jump(0.0);
            self.fade.set(1.0);
            redraw = true;
        }
        // The bubble and the line follow the smoothed reading (drawn again
        // once it has moved by a fiftieth of a degree); the numbers move a
        // tenth at a time.
        let now = self.reading(up);
        redraw |= (now.0 - self.now.0).abs() > 0.02 || (now.1 - self.now.1).abs() > 0.02;
        self.now = now;
        let shown = (steady(self.shown.0, now.0), steady(self.shown.1, now.1));
        if shown != self.shown {
            self.shown = shown;
            redraw = true;
        }
        self.level.set(if self.is_level() { 1.0 } else { 0.0 });
        redraw
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        if self.up.is_none() {
            let words = imu::trouble().unwrap_or_else(|| "starting the motion sensor".into());
            g.empty("straighten", &words);
            return;
        }
        let alpha = self.fade.value().clamp(0.0, 1.0);
        match self.view {
            View::Flat => self.draw_flat(g, alpha),
            View::Edge => self.draw_edge(g, alpha),
        }
        let hint = if self.held {
            "enter goes on"
        } else if self.zeroed() {
            "enter holds · space clears the zero"
        } else {
            "enter holds · space zeroes on this"
        };
        g.hint(hint);
    }
}

// ---------------------------------------------------------------- compass ---

/// What the metal around the magnetometer adds to the field, measured by
/// turning the handheld round: taken away from every reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// The field the handheld itself makes, µT, in the screen's frame.
    pub offset: Vec3,
    /// Each axis's stretch to make the turn a circle.
    pub scale: Vec3,
}

impl Calibration {
    fn path() -> PathBuf {
        crate::config_dir().join("compass.json")
    }

    pub fn load() -> Option<Calibration> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let three = |key: &str| -> Option<Vec3> {
            let a = v.get(key)?.as_array()?;
            let f = |i: usize| a.get(i).and_then(serde_json::Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite());
            Some(Vec3::new(f(0)?, f(1)?, f(2)?))
        };
        let scale = three("scale").unwrap_or(Vec3::new(1.0, 1.0, 1.0));
        let sane = |s: f32| (0.5..=2.0).contains(&s);
        Some(Calibration { offset: three("offset")?, scale }).filter(|_| sane(scale.x) && sane(scale.y) && sane(scale.z))
    }

    pub fn save(&self) -> std::io::Result<()> {
        let (o, s) = (self.offset, self.scale);
        let text = serde_json::json!({ "offset": [o.x, o.y, o.z], "scale": [s.x, s.y, s.z] }).to_string();
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::sys::write_atomic(&path, text.as_bytes())
    }

    pub fn apply(&self, raw: Vec3) -> Vec3 {
        let d = raw.sub(self.offset);
        Vec3::new(d.x * self.scale.x, d.y * self.scale.y, d.z * self.scale.z)
    }
}

/// The parts of a turn a calibration wants seen: eighths of a full turn.
const SECTORS: usize = 8;

/// A calibration under way: the field's extremes so far, and how much of a
/// turn the gyroscope has seen.
#[derive(Debug)]
pub struct Turning {
    min: Vec3,
    max: Vec3,
    /// How far it has turned about the screen's axis, degrees.
    angle: f32,
    seen: [bool; SECTORS],
}

impl Turning {
    pub fn new() -> Turning {
        Turning { min: Vec3::new(f32::MAX, f32::MAX, f32::MAX), max: Vec3::new(f32::MIN, f32::MIN, f32::MIN), angle: 0.0, seen: [false; SECTORS] }
    }

    /// A field reading, and the turning about the screen's axis `dt` seconds on.
    pub fn feed(&mut self, field: Vec3, turn_z: f32, dt: f32) {
        self.min = Vec3::new(self.min.x.min(field.x), self.min.y.min(field.y), self.min.z.min(field.z));
        self.max = Vec3::new(self.max.x.max(field.x), self.max.y.max(field.y), self.max.z.max(field.z));
        self.angle += turn_z * dt;
        let sector = (self.angle.rem_euclid(360.0) / (360.0 / SECTORS as f32)) as usize % SECTORS;
        self.seen[sector] = true;
    }

    pub fn seen(&self) -> usize {
        self.seen.iter().filter(|s| **s).count()
    }

    /// Done: a whole turn seen, and the field swung across it.
    pub fn done(&self) -> bool {
        self.seen() == SECTORS
    }

    /// What the turn measured. None when the field hardly changed as it
    /// turned (a magnetometer not reading).
    pub fn result(&self) -> Option<Calibration> {
        let span = self.max.sub(self.min);
        if span.x < 10.0 || span.y < 10.0 {
            return None;
        }
        let mid = self.min.add(self.max).scale(0.5);
        // Turned flat, the up-and-down axis hardly swings: its offset is
        // kept only if it did.
        let offset = Vec3::new(mid.x, mid.y, if span.z >= 30.0 { mid.z } else { 0.0 });
        let (rx, ry) = (span.x * 0.5, span.y * 0.5);
        let avg = (rx + ry) * 0.5;
        let scale = Vec3::new((avg / rx).clamp(0.7, 1.4), (avg / ry).clamp(0.7, 1.4), 1.0);
        Some(Calibration { offset, scale })
    }
}

pub struct Compass {
    _hold: imu::Hold,
    /// The way up and the field, smoothed.
    up: Option<Vec3>,
    field: Option<Vec3>,
    /// The heading, smoothed round the circle as a direction; and the
    /// whole degree shown.
    dir: Option<(f32, f32)>,
    shown: Option<f32>,
    calibration: Option<Calibration>,
    turning: Option<Turning>,
    /// Something to say in the island, at the next chance.
    say: Option<String>,
}

impl Compass {
    pub fn new() -> Self {
        Compass { _hold: imu::hold(imu::Use::Compass), up: None, field: None, dir: None, shown: None, calibration: Calibration::load(), turning: None, say: None }
    }

    /// The heading being shown, as the dial turns to it.
    fn heading(&self) -> Option<f32> {
        self.dir.map(|(c, s)| s.atan2(c).to_degrees().rem_euclid(360.0))
    }

    fn strength(&self) -> Option<f32> {
        self.field.map(Vec3::len)
    }

    fn start_turning(&mut self) {
        self.turning = Some(Turning::new());
        imu::simulate_turning(true);
    }

    fn stop_turning(&mut self) {
        self.turning = None;
        imu::simulate_turning(false);
    }
}

impl Drop for Compass {
    fn drop(&mut self) {
        imu::simulate_turning(false);
    }
}

impl Screen for Compass {
    fn title(&self) -> &str {
        "Compass"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if self.turning.is_some() => {
                self.stop_turning();
                self.say = Some("calibration stopped".into());
            }
            Key::Enter => self.start_turning(),
            _ => {}
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(words) = self.say.take() {
            *cx.say = Some(words);
        }
        Nav::Stay
    }

    fn busy(&self) -> bool {
        true
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn tick(&mut self, dt: f32) -> bool {
        let Some(r) = imu::reading() else { return false };
        let Some(raw) = r.field else { return false };
        let mut redraw = self.up.is_none();
        if let Some(turning) = &mut self.turning {
            turning.feed(raw, r.turn.z, dt);
            if turning.done() {
                match turning.result() {
                    Some(cal) => {
                        self.say = Some(if cal.save().is_ok() { "compass calibrated".into() } else { "calibrated, but it could not be kept".into() });
                        self.calibration = Some(cal);
                    }
                    None => self.say = Some("the field did not change as it turned".into()),
                }
                self.stop_turning();
            }
            redraw = true;
        }
        let field = self.calibration.map_or(raw, |c| c.apply(raw));
        let k = follow(dt, SMOOTH);
        let up = self.up.map_or(r.up, |u| u.add(r.up.sub(u).scale(k)));
        let field = self.field.map_or(field, |f| f.add(field.sub(f).scale(k)));
        (self.up, self.field) = (Some(up), Some(field));
        match imu::heading(up, field) {
            Some(h) => {
                let (s, c) = h.to_radians().sin_cos();
                let (dc, ds) = self.dir.map_or((c, s), |(dc, ds)| (dc + (c - dc) * k, ds + (s - ds) * k));
                let n = (dc * dc + ds * ds).sqrt().max(1e-6);
                let before = self.heading();
                self.dir = Some((dc / n, ds / n));
                let now = self.heading().unwrap_or(h);
                // The dial turns smoothly; the number moves by whole degrees.
                let whole = now.round().rem_euclid(360.0);
                if self.shown.is_none_or(|s| ((whole - s + 540.0).rem_euclid(360.0) - 180.0).abs() >= 1.0) {
                    self.shown = Some(whole);
                    redraw = true;
                }
                redraw |= before.is_none_or(|b| ((now - b + 540.0).rem_euclid(360.0) - 180.0).abs() > 0.05);
            }
            None => {
                redraw |= self.shown.is_some();
                self.shown = None;
            }
        }
        redraw
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        if self.up.is_none() {
            let words = imu::trouble().unwrap_or_else(|| "starting the compass".into());
            g.empty("explore", &words);
            return;
        }
        let (cx, cy, r, side) = dial(g);
        let ox = g.ox;
        let tau = std::f32::consts::TAU;
        g.canvas.circle(cx + ox, cy, r, t.surface1, 1.0);
        // A calibration: the rim fills clockwise, an eighth for each eighth
        // of a turn seen (whichever way it is turned).
        if let Some(turning) = &self.turning {
            let seen = turning.seen();
            for i in 0..SECTORS {
                let start = i as f32 * tau / SECTORS as f32 + 0.06;
                g.canvas.arc(cx + ox, cy, r - 3.0, 3.0, start, tau / SECTORS as f32 - 0.12, if i < seen { t.ice } else { t.track }, 1.0);
            }
        }
        // The card, turned so north on it lies towards north.
        let heading = self.heading().unwrap_or(0.0);
        let at = |bearing: f32, radius: f32| {
            let a = (bearing - heading).to_radians();
            (cx + ox + radius * a.sin(), cy - radius * a.cos())
        };
        for i in 0..36 {
            let bearing = i as f32 * 10.0;
            let (long, color) = if i % 9 == 0 {
                (10.0, t.dim)
            } else if i % 3 == 0 {
                (7.0, t.faint)
            } else {
                (4.0, t.track)
            };
            let outer = r - 8.0;
            g.canvas.polyline(&[at(bearing, outer - long), at(bearing, outer)], 1.2, color, 1.0);
        }
        for (bearing, letter) in [(0.0, "N"), (90.0, "E"), (180.0, "S"), (270.0, "W")] {
            let (x, y) = at(bearing, r - 27.0);
            let color = if letter == "N" { t.signal } else { t.dim };
            g.text_at(x - ox, y, Align::Center, letter, ty.label, color);
        }
        // The top edge's mark, fixed: where the handheld points.
        g.canvas.polyline(&[(cx + ox, cy - r + 2.0), (cx + ox, cy - r + 15.0)], 2.5, t.signal, 1.0);
        g.canvas.circle(cx + ox, cy, 2.5, t.faint, 1.0);

        // Beside it: the heading, large, and how it stands.
        let big = ty.display(34.0);
        let line1 = side.y + side.h * 0.42;
        match self.shown {
            Some(h) => {
                let text = format!("{h:.0}°");
                let w = g.text(side.x - 1.0, line1, &text, big, t.ink);
                g.text(side.x + w + 6.0, line1, imu::point(h), ty.name, t.dim);
            }
            None => {
                g.text(side.x - 1.0, line1, "–", big, t.faint);
            }
        }
        g.text_at(side.x, line1 + 12.0, Align::Left, "magnetic", ty.caption, t.faint);
        let strength = self.strength().unwrap_or(0.0);
        let state_y = side.bottom() - ty.caption.size * 0.5 - 3.0;
        if self.turning.is_some() {
            chip(g, side.x, state_y, "turn it round", t.ice, t.on_ice, 1.0);
        } else if self.shown.is_none() {
            chip(g, side.x, state_y, "hold it flatter", t.surface3, t.ink, 1.0);
        } else if strength > 100.0 {
            chip(g, side.x, state_y, "a magnet is near", t.amber_container, t.amber_ink, 1.0);
        } else if self.calibration.is_none() {
            chip(g, side.x, state_y, "not calibrated", t.amber_container, t.amber_ink, 1.0);
        } else {
            let words = format!("field {strength:.0} µT");
            g.text_at(side.x, state_y, Align::Left, &words, ty.caption, t.dim);
        }
        let hint = match &self.turning {
            Some(turning) => format!("turn once, flat · {} of {SECTORS} · enter stops", turning.seen()),
            None if self.calibration.is_some() => "enter calibrates again".into(),
            None => "enter calibrates: turn it round once".into(),
        };
        g.hint(&hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::TextSize;

    #[test]
    fn every_reading_and_state_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let (cx, _, r, side) = dial(&g);
            assert!(cx - r >= PAD && side.w > 60.0, "{size:?}: the dial and its column fit");
            // The widest each reading can be beside the dial.
            let widest = g.measure(&degrees(-89.9), ty.display(30.0)).min(g.measure(&degrees(-89.9), ty.display(24.0)));
            assert!(widest <= side.w, "{size:?}: a reading, {widest} in {}", side.w);
            let heading = g.measure("359°", ty.display(34.0)) + 6.0 + g.measure("WNW", ty.name);
            assert!(heading <= side.w, "{size:?}: the heading, {heading} in {}", side.w);
            for words in ["held", "level", "moving", "zeroed", "turn it round", "hold it flatter", "a magnet is near", "not calibrated", "field 160 µT"] {
                let w = g.measure(words, ty.caption) + 14.0;
                assert!(w <= side.w + PAD, "{size:?}: {words}, {w} in {}", side.w);
            }
            for hint in ["enter holds · space clears the zero", "turn once, flat · 8 of 8 · enter stops", "enter calibrates: turn it round once"] {
                assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{size:?}: {hint}");
            }
            // Stood on an edge: the angle on its plate across the middle.
            assert!(g.measure(&degrees(179.9), ty.display(40.0)) + 24.0 < W - 2.0 * PAD);
        }
    }

    #[test]
    fn readings_are_shown_steadily_to_a_tenth() {
        assert_eq!(degrees(-0.04), "0.0°");
        assert_eq!(degrees(12.345), "12.3°");
        assert_eq!(degrees(-3.06), "-3.1°");
        // A reading wavering about a tenth's edge keeps what is shown.
        assert_eq!(steady(1.25, 1.29), 1.25);
        assert_eq!(steady(1.25, 1.35), 1.35);
    }

    #[test]
    fn a_whole_turn_calibrates_the_compass() {
        // Turned round once on the bench, the simulated handheld's own field
        // (the offset a real one has from the metal around it) comes out.
        let mut turning = Turning::new();
        let offset = imu::SIMULATED_OFFSET;
        let earth = Vec3::new(0.0, 20.0, -42.0);
        for step in 0..=80 {
            let heading = step as f32 * 4.5;
            let field = imu::in_screen(earth, heading, 0.0, 0.0).add(offset);
            // Clockwise from above is the screen's axis turning negatively.
            turning.feed(field, -45.0, 0.1);
        }
        assert!(turning.done(), "all eighths seen: {}", turning.seen());
        let cal = turning.result().expect("a calibration");
        assert!((cal.offset.x - offset.x).abs() < 0.2 && (cal.offset.y - offset.y).abs() < 0.2, "{cal:?}");
        assert_eq!(cal.offset.z, 0.0, "turned flat: its up-and-down offset is left alone");
        // Taken away, the heading reads true again.
        let field = cal.apply(imu::in_screen(earth, 200.0, 0.0, 0.0).add(offset));
        let h = imu::heading(imu::in_screen(Vec3::new(0.0, 0.0, 1.0), 200.0, 0.0, 0.0), field).unwrap();
        assert!((h - 200.0).abs() < 1.5, "{h}");
        // A magnetometer whose reading never changed calibrates nothing.
        let mut dead = Turning::new();
        for _ in 0..90 {
            dead.feed(Vec3::new(3.0, 4.0, 5.0), 45.0, 0.1);
        }
        assert!(dead.done() && dead.result().is_none());
    }

    #[test]
    fn a_calibration_is_kept_and_a_bad_one_ignored() {
        crate::shell::tests::sandbox();
        let cal = Calibration { offset: Vec3::new(14.0, -9.0, 0.0), scale: Vec3::new(1.02, 0.98, 1.0) };
        cal.save().unwrap();
        assert_eq!(Calibration::load(), Some(cal));
        std::fs::write(Calibration::path(), r#"{"offset": [1, 2], "scale": [1, 1, 1]}"#).unwrap();
        assert_eq!(Calibration::load(), None);
        std::fs::write(Calibration::path(), r#"{"offset": [1, 2, 3], "scale": [9, 1, 1]}"#).unwrap();
        assert_eq!(Calibration::load(), None, "a stretch no turn could give");
        std::fs::remove_file(Calibration::path()).ok();
    }

    // On a computer, where the simulated sensor reads.
    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn the_level_zeroes_and_holds() {
        let mut level = Level::new();
        // A few readings from the simulated bench.
        for _ in 0..20 {
            level.tick(1.0 / 30.0);
        }
        assert!(level.up.is_some(), "reading");
        assert_eq!(level.view, View::Flat, "lying on the bench");
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut say = None;
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        level.key(Key::Char(' '), &mut cx);
        assert!(level.zeroed());
        assert!(level.shown.0.abs() < 0.05 && level.shown.1.abs() < 0.05, "zeroed reads level: {:?}", level.shown);
        level.key(Key::Enter, &mut cx);
        assert!(level.held && !level.busy(), "held still");
        let held = level.shown;
        std::thread::sleep(std::time::Duration::from_millis(300));
        level.tick(0.3);
        assert_eq!(level.shown, held, "held");
        level.key(Key::Enter, &mut cx);
        level.key(Key::Char(' '), &mut cx);
        assert!(!level.zeroed(), "Space again clears the zero");
    }
}
