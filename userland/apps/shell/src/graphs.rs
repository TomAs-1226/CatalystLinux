//! Live graphs of the robot's numbers. In Tables, Enter on a value puts it on
//! the graph (up to four at once) and Enter again takes it off; the Graphs
//! app plots the last minute of each, every one on its own scale since they
//! are rarely in the same units, with its value now and the range of what
//! is in view.
//!
//! The values are sampled ten times a second by the robot's own recorder,
//! whichever screen is showing, so a graph opened late already has its
//! minute. Which values are graphed is remembered (`graphs` beside the
//! settings).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect, Tokens};
use nt4::Value;

use crate::gfx::{Gfx, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The most values graphed at once.
pub const MOST: usize = 4;
/// The time in view, seconds.
pub const WINDOW: f64 = 60.0;
/// Kept a little longer than shown, so a graph unpaused is whole at once.
const KEEP: f64 = WINDOW + 15.0;
/// How often values are sampled.
const EVERY: f64 = 0.1;

/// A value as a graph reads it: a number, or a flag as 1 and 0.
pub fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

/// A key's last part in words: `Systemcore/BatteryVolts` is "battery volts".
pub fn words(key: &str) -> String {
    let leaf = key.rsplit('/').next().unwrap_or(key);
    let chars: Vec<char> = leaf.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            out.push(' ');
            continue;
        }
        // A word starts at a capital after a small letter or a digit, or at
        // the last capital of a run that a small letter follows ("CANBus").
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1);
        let starts = c.is_uppercase()
            && (prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) || (prev.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase())));
        if starts && !out.ends_with(' ') {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out.trim().to_owned()
}

/// The table a key is in, in words: `Elevator/StatorCurrentAmps` is "elevator".
pub fn place(key: &str) -> String {
    key.rsplit('/').nth(1).map(words).unwrap_or_default()
}

/// Each graph's colour: the signal, ice, amber and ink, all distinct and all
/// readable as a line against the plot in either tone. (Ice is a fill: in
/// light mode it is too pale for a thin line, so it is taken deeper, toward
/// what is written on it.)
pub fn colour(t: &Tokens, i: usize) -> Color {
    let ice = if t.tone == bezel::Tone::Light { t.ice.mix(t.on_ice, 0.6) } else { t.ice };
    [t.signal, ice, t.amber_ink, t.ink][i % MOST]
}

/// What happened when a value was put on the graph or taken off.
#[derive(Debug, PartialEq)]
pub enum Toggled {
    Added,
    Removed,
    Full,
    NotANumber,
}

/// The values on the graph and their last minute or so: (seconds, value).
#[derive(Default)]
pub struct Recorder {
    keys: Vec<String>,
    points: Vec<VecDeque<(f64, f64)>>,
    /// Seconds of the recorder's own clock, and when it last sampled.
    clock: f64,
    last: Option<f64>,
    loaded: bool,
}

impl Recorder {
    fn file() -> PathBuf {
        crate::config_dir().join("graphs")
    }

    /// The values graphed last time, read the first time they are needed.
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let text = std::fs::read_to_string(Self::file()).unwrap_or_default();
        self.keys = text.lines().map(str::trim).filter(|k| !k.is_empty()).take(MOST).map(str::to_owned).collect();
        self.points = vec![VecDeque::new(); self.keys.len()];
    }

    fn save(&self) {
        let file = Self::file();
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        crate::sys::write_atomic(&file, self.keys.join("\n").as_bytes()).ok();
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    pub fn points(&self, i: usize) -> Option<&VecDeque<(f64, f64)>> {
        self.points.get(i)
    }

    /// The recorder's clock: the right-hand edge of a graph.
    pub fn now(&self) -> f64 {
        self.clock
    }

    /// Where a key is on the graph, if it is.
    pub fn place_of(&mut self, key: &str) -> Option<usize> {
        self.load();
        self.keys.iter().position(|k| k == key)
    }

    /// Put a value on the graph, or take it off if it is there.
    pub fn toggle(&mut self, key: &str, numeric: bool) -> Toggled {
        self.load();
        if let Some(at) = self.place_of(key) {
            self.remove(at);
            return Toggled::Removed;
        }
        if !numeric {
            return Toggled::NotANumber;
        }
        if self.keys.len() >= MOST {
            return Toggled::Full;
        }
        self.keys.push(key.to_owned());
        self.points.push(VecDeque::new());
        self.save();
        Toggled::Added
    }

    /// Nothing on the graph (the screenshot tour starts from it).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn clear(&mut self) {
        self.load();
        self.keys.clear();
        self.points.clear();
        self.save();
    }

    pub fn remove(&mut self, i: usize) {
        self.load();
        if i < self.keys.len() {
            self.keys.remove(i);
            self.points.remove(i);
            self.save();
        }
    }

    /// Time passes: sample each value, ten times a second, and let go of
    /// what has passed out of view.
    pub fn sample(&mut self, dt: f32, robot: &Robot) {
        self.load();
        self.clock += dt as f64;
        if self.keys.is_empty() || self.last.is_some_and(|at| self.clock - at < EVERY) {
            return;
        }
        self.last = Some(self.clock);
        for (key, points) in self.keys.iter().zip(&mut self.points) {
            if let Some(v) = robot.get(key).and_then(|v| number(&v)) {
                points.push_back((self.clock, v));
            }
            while points.front().is_some_and(|(t, _)| *t < self.clock - KEEP) {
                points.pop_front();
            }
        }
    }
}

/// What the keys do, live and paused.
/// (d, taking one off, is in the shortcut list: with a wide range there is
/// not room for it here at the largest text.)
const HINTS: [&str; 2] = ["space: pause", "paused · space"];

/// The graphs as they were when paused: the time at the right edge, and each line.
struct Frozen {
    at: f64,
    points: Vec<VecDeque<(f64, f64)>>,
}

/// The Graphs app: a lane for each value graphed, its name and reading on
/// the left and its last minute on its own scale on the right; the one
/// chosen says its range at the bottom. Up and Down (or Left and Right)
/// choose one; Space pauses; d takes the one chosen off; Enter opens it in
/// Tables.
pub struct Graphs {
    focus: usize,
    paused: Option<Frozen>,
    drawn: Instant,
}

impl Graphs {
    pub fn new() -> Self {
        Graphs { focus: 0, paused: None, drawn: Instant::now() }
    }
}

/// The range a line is drawn over: what is in view, a little beyond, and
/// never nothing (a flat line sits in the middle).
fn range(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    let (lo, hi) = values.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if lo > hi {
        return None;
    }
    let span = hi - lo;
    if span < 1e-9 {
        let pad = (lo.abs() * 0.05).max(0.5);
        return Some((lo - pad, hi + pad));
    }
    Some((lo - span * 0.08, hi + span * 0.08))
}

/// A value as short as it can be read: whole above a hundred, two decimals under ten.
pub fn show(v: f64) -> String {
    match v.abs() {
        a if a >= 100.0 => format!("{v:.0}"),
        a if a >= 10.0 => format!("{v:.1}"),
        _ => format!("{v:.2}"),
    }
}

impl Screen for Graphs {
    fn title(&self) -> &str {
        "Graphs"
    }

    fn busy(&self) -> bool {
        self.paused.is_none()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let space = if self.paused.is_some() { keys("paused", &[("space", "Go live again")]) } else { keys("live", &[("space", "Pause")]) };
        vec![space, keys("lines", &[("arrows", "Choose a line"), ("enter", "Its table"), ("d", "Take it off")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let count = cx.robot.graphs.keys().len();
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Right | Key::Down if self.focus + 1 < count => self.focus += 1,
            Key::Char(' ') => {
                self.paused = match self.paused.take() {
                    Some(_) => None,
                    None => {
                        let rec = &cx.robot.graphs;
                        Some(Frozen { at: rec.now(), points: (0..count).filter_map(|i| rec.points(i).cloned()).collect() })
                    }
                };
                *cx.say = Some(if self.paused.is_some() { "paused".into() } else { "live".into() });
            }
            Key::Char('d') if count > 0 => {
                let key = cx.robot.graphs.keys()[self.focus.min(count - 1)].clone();
                cx.robot.graphs.remove(self.focus.min(count - 1));
                if let Some(frozen) = &mut self.paused {
                    if self.focus < frozen.points.len() {
                        frozen.points.remove(self.focus);
                    }
                }
                self.focus = self.focus.min(count.saturating_sub(2));
                *cx.say = Some(format!("{} off the graph", name_and_unit(&key).0));
            }
            Key::Enter if count > 0 => {
                let key = cx.robot.graphs.keys()[self.focus.min(count - 1)].clone();
                return Nav::Push(Box::new(crate::screens::Tables::at(&key, cx.robot)));
            }
            _ => {}
        }
        Nav::Stay
    }

    /// Ten frames a second while live: the lines move a fraction of a pixel
    /// each, which reads as smooth without drawing at the full beat.
    fn tick(&mut self, _dt: f32) -> bool {
        if self.paused.is_none() && self.drawn.elapsed() >= Duration::from_millis(100) {
            self.drawn = Instant::now();
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let rec = &robot.graphs;
        let keys = rec.keys().to_vec();
        if keys.is_empty() {
            g.empty("insights", "in tables, enter on a value graphs it");
            return;
        }
        self.focus = self.focus.min(keys.len() - 1);
        let (end, lines): (f64, Vec<VecDeque<(f64, f64)>>) = match &self.paused {
            Some(frozen) => (frozen.at, frozen.points.clone()),
            None => (rec.now(), (0..keys.len()).filter_map(|i| rec.points(i).cloned()).collect()),
        };
        let start = end - WINDOW;
        let top = g.top();
        let bottom = g.above_hint();
        // A lane each, sharing the height: the fewer graphed, the taller.
        let n = keys.len();
        let lane_h = ((bottom - top) - LANE_GAP * (n - 1) as f32) / n as f32;
        let two_lines = lane_h >= ty.caption.size + ty.label.size + 12.0;
        // Each lane's points in view, name, unit and reading; and the room the
        // widest name and reading need, the line taking the rest.
        let lanes: Vec<(Vec<(f64, f64)>, String, &str, String)> = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let in_view: Vec<(f64, f64)> = lines.get(i).map(|p| p.iter().copied().filter(|(time, _)| *time >= start && *time <= end).collect()).unwrap_or_default();
                let (name, unit) = name_and_unit(key);
                let now = in_view.last().map_or("–".to_owned(), |(_, v)| with_unit(*v, unit));
                (in_view, name, unit, now)
            })
            .collect();
        let left_w = lanes
            .iter()
            .map(|(_, name, _, now)| {
                let (name_w, now_w) = (g.measure(name, ty.caption), g.measure(now, ty.label));
                if two_lines { 22.0 + name_w.max(now_w) } else { 30.0 + name_w + now_w }
            })
            .fold(LEFT_W, f32::max)
            .min(LEFT_MOST);
        let mut range_of_chosen = None;
        for (i, (key, (in_view, name, unit, now))) in keys.iter().zip(&lanes).enumerate() {
            let unit = *unit;
            let r = Rect::new(PAD, top + i as f32 * (lane_h + LANE_GAP), W - 2.0 * PAD, lane_h);
            let chosen = i == self.focus;
            let corner = 10.0f32.min(r.h * 0.5);
            g.surface(r, corner, if chosen { t.surface2 } else { t.surface1 });
            // What it is and what it reads, in the lane's left part.
            let ink = colour(&t, i);
            let name_ink = if chosen { t.ink } else { t.dim };
            if two_lines {
                let line1 = r.y + 6.0 + ty.caption.size * 0.5;
                g.dot(r.x + 10.0, line1, 3.0, ink);
                let w = g.text_fit(r.x + 17.0, line1, name, ty.caption, name_ink, left_w - 20.0);
                // Where it lives, when there is room.
                let place = format!("· {}", place(key));
                if place.len() > 3 && g.measure(&place, ty.caption) <= left_w - 20.0 - w - 4.0 {
                    g.text_at(r.x + 21.0 + w, line1, Align::Left, &place, ty.caption, t.faint);
                }
                g.text_fit(r.x + 10.0, r.bottom() - 6.0 - ty.label.size * 0.5, now, ty.label, t.ink, left_w - 12.0);
            } else {
                g.dot(r.x + 10.0, r.cy(), 3.0, ink);
                let w = g.text_fit(r.x + 17.0, r.cy(), now, ty.label, t.ink, left_w - 20.0);
                g.text_fit(r.x + 22.0 + w, r.cy(), name, ty.caption, name_ink, left_w - 25.0 - w);
            }
            // Its last minute, on its own scale; a tick every quarter of it.
            let spark = Rect::new(r.x + left_w, r.y + 5.0, r.w - left_w - 9.0, r.h - 10.0);
            for k in 1..4 {
                let x = (spark.x + spark.w * k as f32 / 4.0).round() + g.ox;
                g.canvas.rrect(Rect::new(x, spark.bottom() - 2.0, 1.0, 2.0), 0.0, t.faint, 0.8);
            }
            match range(in_view.iter().map(|(_, v)| *v)) {
                Some((lo, hi)) => {
                    let x_of = |time: f64| spark.x + ((time - start) / WINDOW) as f32 * spark.w + g.ox;
                    let y_of = |v: f64| spark.bottom() - ((v - lo) / (hi - lo)) as f32 * spark.h;
                    // At most two points a pixel across.
                    let step = (in_view.len() as f32 / (spark.w * 2.0)).ceil().max(1.0) as usize;
                    let pts: Vec<(f32, f32)> = in_view.iter().step_by(step).map(|(time, v)| (x_of(*time), y_of(*v))).collect();
                    if pts.len() >= 2 {
                        g.canvas.polyline(&pts, if chosen { 2.0 } else { 1.5 }, ink, 1.0);
                    } else if let Some((x, y)) = pts.first() {
                        g.canvas.circle(*x, *y, 1.5, ink, 1.0);
                    }
                    if chosen {
                        let (seen_lo, seen_hi) = in_view.iter().fold((f64::MAX, f64::MIN), |(a, b), (_, v)| (a.min(*v), b.max(*v)));
                        range_of_chosen = Some((seen_lo, seen_hi, unit));
                    }
                }
                None => {
                    let why = if robot.linked() { "waiting" } else { "no robot" };
                    g.text_at(spark.cx(), spark.cy(), Align::Center, why, ty.caption, t.faint);
                }
            }
            if chosen {
                g.focus_ring(r, corner);
            }
        }
        // The chosen one's range over the minute, then the keys.
        let span = range_of_chosen.map(|(lo, hi, unit)| format!("{} to {} · ", show(lo), with_unit(hi, unit))).unwrap_or_default();
        g.hint(&format!("{span}{}", if self.paused.is_some() { HINTS[1] } else { HINTS[0] }));
    }
}

/// The room in a lane for its name and reading, at least and at most (as
/// much as the widest needs, between the two); the line has the rest.
const LEFT_W: f32 = 132.0;
const LEFT_MOST: f32 = 205.0;
const LANE_GAP: f32 = 3.0;

/// A value with its unit after it, as a reading is written.
fn with_unit(v: f64, unit: &str) -> String {
    match unit {
        "" => show(v),
        "%" | "°" | "°c" => format!("{}{unit}", show(v)),
        _ => format!("{} {unit}", show(v)),
    }
}

/// A key's words without the unit its name ends in, and that unit as it is
/// written after a number: `Systemcore/BatteryVolts` is ("battery", "v").
pub fn name_and_unit(key: &str) -> (String, &'static str) {
    let words = words(key);
    let mut parts: Vec<&str> = words.split(' ').collect();
    let last = parts.last().copied().unwrap_or_default();
    let before = parts.len().checked_sub(2).map(|i| parts[i]);
    let unit = match last {
        "volts" | "voltage" => "v",
        "amps" => "a",
        "ms" => "ms",
        "seconds" | "secs" => "s",
        "celsius" => "°c",
        "c" if matches!(before, Some("temperature" | "temp")) => "°c",
        "rps" => "rps",
        "rpm" => "rpm",
        "mps" => "m/s",
        "deg" | "degrees" => "°",
        "percent" => "%",
        "meters" => "m",
        "hz" => "hz",
        _ => return (words, ""),
    };
    if parts.len() > 1 {
        parts.pop();
    }
    (parts.join(" "), unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_as_words() {
        assert_eq!(words("Systemcore/BatteryVolts"), "battery volts");
        assert_eq!(words("Loop/Robot/LastMs"), "last ms");
        assert_eq!(words("Systemcore/CanUtilization"), "can utilization");
        assert_eq!(words("Drive/CANBusLoad"), "can bus load");
        assert_eq!(words("Shooter/PrimaryVelocityRPS"), "primary velocity rps");
        assert_eq!(words("Arm/Motor2Amps"), "motor2 amps");
        assert_eq!(place("Elevator/StatorCurrentAmps"), "elevator");
        assert_eq!(place("Loose"), "");
    }

    #[test]
    fn the_demo_robots_values_move_on_the_graph() {
        crate::shell::tests::sandbox();
        let mut robot = Robot::demo();
        // A fresh recorder, not whatever the sandbox's file says.
        robot.graphs = Recorder { loaded: true, ..Recorder::default() };
        assert_eq!(robot.graphs.toggle("Systemcore/BatteryVolts", true), Toggled::Added);
        assert_eq!(robot.graphs.toggle("Elevator/AtSetpoint", true), Toggled::Added);
        assert_eq!(robot.graphs.toggle("Robot/Identity/Name", false), Toggled::NotANumber);
        // Two seconds at 60 frames a second: twenty samples, and the battery moved.
        for _ in 0..120 {
            robot.refresh(1.0 / 60.0);
        }
        let battery = robot.graphs.points(0).unwrap();
        assert!((19..=21).contains(&battery.len()), "{}", battery.len());
        let (lo, hi) = range(battery.iter().map(|(_, v)| *v)).unwrap();
        assert!(hi - lo > 0.01, "the demo battery moves");
        // A flag graphs as 1 and 0.
        assert!(robot.graphs.points(1).unwrap().iter().all(|(_, v)| *v == 0.0 || *v == 1.0));
        // Only so many at once, and again takes it off.
        robot.graphs.toggle("Loop/Robot/LastMs", true);
        robot.graphs.toggle("Swerve/SpeedMPS", true);
        assert_eq!(robot.graphs.toggle("Shooter/SetpointRPS", true), Toggled::Full);
        assert_eq!(robot.graphs.toggle("Systemcore/BatteryVolts", true), Toggled::Removed);
        assert_eq!(robot.graphs.keys().len(), 3);
        // What passes out of view is let go.
        for _ in 0..(KEEP as usize + 10) * 10 {
            robot.refresh(0.1);
        }
        assert!(robot.graphs.points(0).unwrap().len() <= (KEEP / EVERY) as usize + 2);
        for _ in 0..3 {
            robot.graphs.remove(0);
        }
    }

    #[test]
    fn a_flat_line_is_drawn_in_the_middle() {
        assert_eq!(range([4.0, 4.0].into_iter()), Some((3.5, 4.5)));
        let (lo, hi) = range([0.0, 10.0].into_iter()).unwrap();
        assert!(lo < 0.0 && hi > 10.0);
        assert_eq!(range(std::iter::empty()), None);
    }

    /// WCAG's contrast for a graphical object (a line) is 3:1.
    fn contrast(a: Color, b: Color) -> f32 {
        let lum = |c: Color| {
            let p = c.to_u32();
            let ch = |v: u32| {
                let s = v as f32 / 255.0;
                if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch((p >> 16) & 255) + 0.7152 * ch((p >> 8) & 255) + 0.0722 * ch(p & 255)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_line_reads_against_the_plot_in_both_tones() {
        for accent in bezel::Accent::ALL {
            for tone in [bezel::Tone::Dark, bezel::Tone::Light] {
                let t = Tokens::of(accent, tone);
                for i in 0..MOST {
                    let c = contrast(colour(&t, i), t.surface1);
                    assert!(c >= 3.0, "{accent:?} {tone:?} line {i}: {c:.2}");
                }
            }
        }
    }

    #[test]
    fn names_lose_their_unit_to_the_reading() {
        assert_eq!(name_and_unit("Systemcore/BatteryVolts"), ("battery".to_owned(), "v"));
        assert_eq!(name_and_unit("Elevator/StatorCurrentAmps"), ("stator current".to_owned(), "a"));
        assert_eq!(name_and_unit("Elevator/TemperatureC"), ("temperature".to_owned(), "°c"));
        assert_eq!(name_and_unit("Systemcore/CpuPercent"), ("cpu".to_owned(), "%"));
        assert_eq!(name_and_unit("Loop/Robot/LastMs"), ("last".to_owned(), "ms"));
        assert_eq!(name_and_unit("Swerve/SpeedMPS"), ("speed".to_owned(), "m/s"));
        // A key that is only a unit keeps its name; one with no unit keeps all its words.
        assert_eq!(name_and_unit("Volts"), ("volts".to_owned(), "v"));
        assert_eq!(name_and_unit("Elevator/AtSetpoint"), ("at setpoint".to_owned(), ""));
        assert_eq!(with_unit(12.48, "v"), "12.5 v");
        assert_eq!(with_unit(4.216, "ms"), "4.22 ms");
        assert_eq!(with_unit(41.0, "%"), "41.0%");
    }

    #[test]
    fn every_line_of_the_graphs_fits() {
        let mut g = Gfx::new(Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            // The hint with a wide range before the keys, live and paused.
            for hint in HINTS {
                let line = format!("{} to {} · {hint}", show(-1234.5), with_unit(5678.0, "rpm"));
                assert!(g.measure(&line, ty.caption) <= W - 2.0 * PAD, "{line}");
            }
            assert!(g.measure("in tables, enter on a value graphs it", ty.caption) <= W - 2.0 * PAD);
            // Robot values' names and readings in their lane: one line when
            // four share the screen, the room grown to fit them. (At the
            // largest text, a name as long as "primary velocity" beside its
            // reading is shortened when four share the screen.)
            let long = size == crate::gfx::TextSize::Large;
            let keys = ["Systemcore/BatteryVolts", "Systemcore/CpuPercent", "Elevator/PositionMeters", "Elevator/TemperatureC", "Elevator/StatorCurrentAmps", "Loop/Robot/LastMs", "Shooter/PrimaryVelocityRPS"];
            for key in keys.iter().filter(|k| !(long && k.ends_with("PrimaryVelocityRPS"))) {
                let (name, unit) = name_and_unit(key);
                let reading = with_unit(-188.8, unit);
                let (name_w, now_w) = (g.measure(&name, ty.caption), g.measure(&reading, ty.label));
                assert!(30.0 + name_w + now_w <= LEFT_MOST, "{name} {reading}");
            }
        }
    }
}
