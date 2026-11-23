//! A match played back from its log: the field with the robot's path, a
//! scrubber with auto, teleop and the endgame marked, play and pause at 1×,
//! 2× and 4×, and under it the battery, CAN and current, each a compact
//! line. A brownout or a CAN spike is a marked moment to jump to; Logs'
//! summary of a match opens its replay at the moment a finding happened
//! (the times the digest keeps: `wpilog::Summary::moments`), so "Q14 · the
//! robot browned out at 1:18" opens at 1:18.
//!
//! The log is read once, on a thread of its own, into a [`Timeline`] of a
//! few hundred kilobytes however long the log (each series is kept to
//! [`CAP`] samples). The field and the charts are drawn once into layers
//! (`field::Layer`); a frame lays them down and draws only the robot, its
//! trail and the cursor over them.

use std::io::{self, Read};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use bezel::{Align, Canvas, Color, Rect};
use detent::{role, Animated};

use crate::field::{self, Alliance, Camera, Layer, Look, Pose};
use crate::gfx::{Gfx, GAP, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::wpilog::Level;

/// The most samples a series keeps: a match at 50 a second fits whole.
pub const CAP: usize = 8192;
/// Systemcore browns out below this (it logs it as `BrownoutVolts`).
const BROWNOUT_VOLTS: f32 = 6.8;
/// The CAN bus at or over this is a spike worth marking.
const CAN_SPIKE: f32 = 0.6;
/// The endgame: the last 30 seconds of the match (2026 manual, G420).
const ENDGAME: f32 = 30.0;
/// How long the trail behind the robot is.
const TRAIL: f32 = 10.0;

/// A time, seconds into the match, and a value.
pub type Sample = (f32, f32);

/// A value over the match. Past [`CAP`] samples it keeps every other one,
/// and from then each one of every two (then four…) that arrive: the lower
/// of each for the battery, the higher for the rest, so a dip or a spike
/// is never what is dropped.
#[derive(Clone, Debug, Default)]
pub struct Series {
    pub points: Vec<Sample>,
    keep_low: bool,
    stride: usize,
    pending: Option<(Sample, usize)>,
}

impl Series {
    fn new(keep_low: bool) -> Self {
        Series { points: Vec::new(), keep_low, stride: 1, pending: None }
    }

    fn better(&self, a: f32, b: f32) -> bool {
        if self.keep_low {
            a < b
        } else {
            a > b
        }
    }

    fn push(&mut self, t: f32, v: f32) {
        if !v.is_finite() || self.points.last().is_some_and(|(last, _)| *last == t) {
            return;
        }
        if self.stride == 1 {
            self.points.push((t, v));
        } else {
            let (best, n) = match self.pending {
                Some((s, n)) => (if self.better(v, s.1) { (t, v) } else { s }, n + 1),
                None => ((t, v), 1),
            };
            if n >= self.stride {
                self.points.push(best);
                self.pending = None;
            } else {
                self.pending = Some((best, n));
            }
        }
        if self.points.len() >= CAP {
            let low = self.keep_low;
            self.points = self.points.chunks(2).map(|pair| *pair.iter().min_by(|a, b| if low { a.1.total_cmp(&b.1) } else { b.1.total_cmp(&a.1) }).unwrap()).collect();
            self.stride *= 2;
        }
    }

    fn finish(&mut self) {
        if let Some((s, _)) = self.pending.take() {
            self.points.push(s);
        }
    }

    fn shift(&mut self, by: f32) {
        for p in &mut self.points {
            p.0 -= by;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The value at `t`: the last one logged by then, if it is recent.
    pub fn at(&self, t: f32) -> Option<f32> {
        let i = self.points.partition_point(|(at, _)| *at <= t + 1e-4);
        let (at, v) = *self.points.get(i.checked_sub(1)?)?;
        (t - at < 1.5).then_some(v)
    }

    /// The lowest and highest values between `t0` and `t1`.
    pub fn range(&self, t0: f32, t1: f32) -> Option<(f32, f32)> {
        self.points.iter().filter(|(t, _)| (t0..=t1).contains(t)).fold(None, |r, (_, v)| Some(r.map_or((*v, *v), |(lo, hi): (f32, f32)| (lo.min(*v), hi.max(*v)))))
    }
}

/// What a chart draws of a series `columns` pixels wide over `t0..t1`: in
/// each column the lowest and the highest sample, in the order they came,
/// one where they are the same. A brownout's dip a tenth of a second long
/// is as deep on the chart as in the log, and a column costs at most two
/// points however many samples fall in it.
pub fn decimate(points: &[Sample], t0: f32, t1: f32, columns: usize) -> Vec<Sample> {
    let mut out = Vec::with_capacity(columns * 2);
    if columns == 0 || t1 <= t0 {
        return out;
    }
    let width = (t1 - t0) / columns as f32;
    let first = points.partition_point(|(t, _)| *t < t0);
    let mut column = usize::MAX;
    let (mut lo, mut hi) = ((0.0f32, f32::MAX), (0.0f32, f32::MIN));
    let flush = |out: &mut Vec<Sample>, lo: Sample, hi: Sample| {
        if lo.1 == hi.1 {
            out.push(lo);
        } else if lo.0 <= hi.0 {
            out.extend([lo, hi]);
        } else {
            out.extend([hi, lo]);
        }
    };
    for &(t, v) in &points[first..] {
        if t > t1 {
            break;
        }
        let c = (((t - t0) / width) as usize).min(columns - 1);
        if c != column {
            if column != usize::MAX {
                flush(&mut out, lo, hi);
            }
            column = c;
            (lo, hi) = ((t, v), (t, v));
        } else {
            if v < lo.1 {
                lo = (t, v);
            }
            if v > hi.1 {
                hi = (t, v);
            }
        }
    }
    if column != usize::MAX {
        flush(&mut out, lo, hi);
    }
    out
}

/// A moment marked on the scrubber.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub at: f32,
    pub kind: MarkKind,
    /// The battery's low point for a brownout; the bus's use for CAN.
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Brownout,
    CanSpike,
}

impl Mark {
    pub fn level(&self) -> Level {
        match self.kind {
            MarkKind::Brownout => Level::Fault,
            MarkKind::CanSpike => Level::Warn,
        }
    }

    /// In a few words, as the analysis says it.
    pub fn words(&self) -> String {
        match self.kind {
            MarkKind::Brownout if self.value > 0.0 => format!("browned out · {:.1} v", self.value),
            MarkKind::Brownout => "browned out".into(),
            MarkKind::CanSpike => format!("CAN at {:.0}%", self.value * 100.0),
        }
    }
}

/// A part of the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Before,
    Auto,
    Pause,
    Teleop,
    Endgame,
    After,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Before => "before the match",
            Phase::Auto => "auto",
            Phase::Pause => "auto ends",
            Phase::Teleop => "teleop",
            Phase::Endgame => "endgame",
            Phase::After => "after the match",
        }
    }
}

/// A match as the replay shows it, times in seconds into the match (from
/// its first enable, as the offline analysis counts them).
#[derive(Debug, Default)]
pub struct Timeline {
    /// Where the scrubber ends: the match's end.
    pub end: f32,
    /// When auto ended, and teleop's start and end, if the log says.
    pub auto_end: Option<f32>,
    pub teleop: Option<(f32, f32)>,
    pub poses: Vec<(f32, Pose)>,
    pub battery: Series,
    pub can: Series,
    pub current: Series,
    pub marks: Vec<Mark>,
    pub alliance: Option<Alliance>,
    /// Which pose it read: "advantagekit" or "field2d".
    #[cfg_attr(not(test), allow(dead_code))]
    pub pose_source: Option<&'static str>,
    pub truncated: bool,
}

/// What the replay wants from an entry.
enum Want {
    Enabled,
    Auto,
    Battery,
    Brownout,
    Can,
    Total,
    Current(usize),
    Red,
    /// A pose: AdvantageKit's (struct, or old double[] in radians), or Field2d's.
    Pose { advantagekit: bool, raw: bool },
}

fn want(name: &str, kind: &str, motors: &mut usize) -> Option<Want> {
    match (name, kind) {
        ("DS:enabled", "boolean") => return Some(Want::Enabled),
        ("DS:autonomous", "boolean") => return Some(Want::Auto),
        _ => {}
    }
    let key = name.strip_prefix("NT:").unwrap_or(name);
    if key.ends_with("SmartDashboard/Field/Robot") && kind == "double[]" {
        return Some(Want::Pose { advantagekit: false, raw: false });
    }
    if key.ends_with("RealOutputs/Odometry/Robot") && matches!(kind, "struct:Pose2d" | "double[]") {
        return Some(Want::Pose { advantagekit: true, raw: kind != "double[]" });
    }
    let key = key.strip_prefix("/Catalyst/").or_else(|| key.strip_prefix("Catalyst/")).unwrap_or(key).trim_start_matches('/');
    match (key, kind) {
        ("FMSInfo/IsRedAlliance", "boolean") => Some(Want::Red),
        ("Systemcore/BatteryVolts", "double") => Some(Want::Battery),
        ("Systemcore/BrownedOut", "boolean") => Some(Want::Brownout),
        ("Systemcore/CanUtilization", "double[]" | "double") => Some(Want::Can),
        ("Brownout/TotalCurrent", "double") => Some(Want::Total),
        (k, "double") if k.ends_with("/StatorCurrentAmps") && !k.starts_with("Systemcore/") => {
            *motors += 1;
            Some(Want::Current(*motors - 1))
        }
        _ => None,
    }
}

/// Everything gathered while the log streams past, in log seconds.
#[derive(Default)]
struct Reading {
    first: Option<f64>,
    last: f64,
    enabled: bool,
    auto: bool,
    saw_auto: bool,
    /// Each enabled stretch: start, end, and whether in auto.
    stretches: Vec<(f64, Option<f64>, bool)>,
    akit: Vec<(f32, Pose)>,
    field2d: Vec<(f32, Pose)>,
    battery: Series,
    can: Series,
    total: Series,
    summed: Series,
    motors: Vec<f32>,
    summed_at: f64,
    browned: Option<bool>,
    brownouts: Vec<f64>,
    low: bool,
    lows: Vec<f64>,
    can_peak: Option<(f64, f32)>,
    red: Option<bool>,
}

impl Reading {
    fn value(&mut self, w: &Want, time: u64, payload: &[u8]) {
        let t = time as f64 / 1e6;
        self.first.get_or_insert(t);
        self.last = self.last.max(t);
        let number = || payload.get(..8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32);
        let flag = payload.first().map(|b| *b != 0);
        let ts = t as f32;
        match w {
            Want::Enabled => {
                let on = flag == Some(true);
                if on && !self.enabled {
                    self.stretches.push((t, None, self.auto));
                } else if !on && self.enabled {
                    if let Some(s) = self.stretches.last_mut() {
                        s.1 = Some(t);
                    }
                }
                self.enabled = on;
            }
            Want::Auto => {
                self.saw_auto = true;
                self.auto = flag == Some(true);
            }
            Want::Red => self.red = flag,
            Want::Battery => {
                if let Some(v) = number().filter(|v| *v > 0.5) {
                    self.battery.push(ts, v);
                    let low = v < BROWNOUT_VOLTS;
                    if low && !self.low {
                        self.lows.push(t);
                    }
                    self.low = low;
                }
            }
            Want::Brownout => {
                let on = flag == Some(true);
                if on && self.browned != Some(true) {
                    self.brownouts.push(t);
                }
                self.browned = Some(on);
            }
            Want::Can => {
                let busiest = payload.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32).filter(|v| v.is_finite()).fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))));
                if let Some(v) = busiest {
                    self.can.push(ts, v);
                    if self.can_peak.is_none_or(|(_, p)| v > p) {
                        self.can_peak = Some((t, v));
                    }
                }
            }
            Want::Total => {
                if let Some(v) = number() {
                    self.total.push(ts, v.abs());
                }
            }
            Want::Current(i) => {
                if let Some(v) = number() {
                    if self.motors.len() <= *i {
                        self.motors.resize(*i + 1, 0.0);
                    }
                    self.motors[*i] = v.abs();
                    // The motors' sum, ten times a second.
                    if t - self.summed_at >= 0.1 {
                        self.summed_at = t;
                        self.summed.push(ts, self.motors.iter().sum());
                    }
                }
            }
            Want::Pose { advantagekit, raw } => {
                let pose = if *raw { field::pose_from_struct(payload) } else { field::pose_from_doubles(&payload.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>(), !advantagekit) };
                if let Some(p) = pose {
                    let poses = if *advantagekit { &mut self.akit } else { &mut self.field2d };
                    if poses.last().is_none_or(|(at, _)| ts > *at) {
                        poses.push((ts, p));
                        if poses.len() >= CAP {
                            *poses = poses.iter().step_by(2).copied().collect();
                        }
                    }
                }
            }
        }
    }
}

impl Timeline {
    /// Read a log into a timeline.
    pub fn read(r: impl Read) -> io::Result<Timeline> {
        let mut motors = 0;
        let mut reading = Reading { battery: Series::new(true), can: Series::new(false), total: Series::new(false), summed: Series::new(false), summed_at: f64::MIN, ..Reading::default() };
        let truncated = crate::wpilog::read_entries(r, |name, kind| want(name, kind, &mut motors), |w, time, payload| reading.value(w, time, payload))?;
        let mut r = reading;
        for s in [&mut r.battery, &mut r.can, &mut r.total, &mut r.summed] {
            s.finish();
        }
        // The match starts at its first enable; without one, the log's start.
        let zero = r.stretches.first().map(|s| s.0).or(r.first).unwrap_or(0.0);
        let last = r.last;
        let into = |t: f64| (t - zero) as f32;
        // Auto: the first stretch, if the Driver Station said so (or, with no
        // word of it, if it ended within auto's 20 s and a little).
        let first_auto = r.stretches.first().filter(|s| if r.saw_auto { s.2 } else { s.1.is_some_and(|end| end - s.0 < 25.0) && r.stretches.len() > 1 });
        let auto_end = first_auto.map(|s| into(s.1.unwrap_or(last)));
        let teleop_stretches: Vec<_> = r.stretches.iter().skip(first_auto.is_some() as usize).collect();
        let teleop = match (teleop_stretches.first(), teleop_stretches.last()) {
            (Some(a), Some(b)) => Some((into(a.0), into(b.1.unwrap_or(last)))),
            _ => None,
        };
        let end = teleop.map(|t| t.1).or(auto_end).unwrap_or(into(last)).max(1.0);
        let (mut poses, pose_source) = if !r.akit.is_empty() {
            (r.akit, Some("advantagekit"))
        } else if !r.field2d.is_empty() {
            (r.field2d, Some("field2d"))
        } else {
            (Vec::new(), None)
        };
        for p in &mut poses {
            p.0 -= zero as f32;
        }
        let mut battery = r.battery;
        battery.shift(zero as f32);
        let mut can = r.can;
        can.shift(zero as f32);
        let mut current = if r.total.is_empty() { r.summed } else { r.total };
        current.shift(zero as f32);
        // The moments: each brownout (Systemcore's own flag, else the
        // battery under its brownout voltage), with the battery's low there;
        // the CAN bus at its busiest, if that was a spike.
        let browns = if r.brownouts.is_empty() { r.lows } else { r.brownouts };
        let mut marks: Vec<Mark> = browns
            .into_iter()
            .map(into)
            .filter(|at| (0.0..=end).contains(at))
            .map(|at| Mark { at, kind: MarkKind::Brownout, value: battery.range(at - 0.3, at + 1.0).map_or(0.0, |(lo, _)| lo) })
            .collect();
        if let Some((t, peak)) = r.can_peak.filter(|(_, p)| *p >= CAN_SPIKE) {
            let at = into(t);
            if (0.0..=end).contains(&at) {
                marks.push(Mark { at, kind: MarkKind::CanSpike, value: peak });
            }
        }
        marks.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(Timeline { end, auto_end, teleop, poses, battery, can, current, marks, alliance: Alliance::of_red(r.red), pose_source, truncated })
    }

    /// Where the robot was at `t`, between the poses either side.
    pub fn pose_at(&self, t: f32) -> Option<Pose> {
        let i = self.poses.partition_point(|(at, _)| *at <= t);
        match (i.checked_sub(1).and_then(|k| self.poses.get(k)), self.poses.get(i)) {
            (Some(a), Some(b)) if b.0 - a.0 < 1.0 => Some(a.1.lerp(b.1, ((t - a.0) / (b.0 - a.0).max(1e-6)).clamp(0.0, 1.0))),
            (Some(a), _) => Some(a.1),
            (None, Some(b)) if b.0 - t < 1.0 => Some(b.1),
            _ => None,
        }
    }

    /// The part of the match at `t`, and the seconds left in it.
    pub fn phase(&self, t: f32) -> (Phase, f32) {
        let auto_end = self.auto_end.unwrap_or(0.0);
        if t < 0.0 {
            return (Phase::Before, -t);
        }
        if self.auto_end.is_some() && t < auto_end {
            return (Phase::Auto, auto_end - t);
        }
        match self.teleop {
            Some((start, _)) if t < start => (Phase::Pause, start - t),
            Some((_, end)) if t < end - ENDGAME => (Phase::Teleop, end - t),
            Some((_, end)) if t <= end => (Phase::Endgame, end - t),
            _ => (Phase::After, 0.0),
        }
    }

    /// Where the endgame starts, if there is a teleop to have one.
    pub fn endgame(&self) -> Option<f32> {
        self.teleop.map(|(start, end)| (end - ENDGAME).max(start))
    }

    /// The next marked moment after `t`, and the last one before it.
    pub fn next_mark(&self, t: f32) -> Option<&Mark> {
        self.marks.iter().find(|m| m.at > t + 0.05)
    }
    pub fn prev_mark(&self, t: f32) -> Option<&Mark> {
        self.marks.iter().rev().find(|m| m.at < t - 0.05)
    }

    /// A mark within a couple of seconds of `t`, the nearest.
    pub fn mark_near(&self, t: f32) -> Option<&Mark> {
        self.marks.iter().filter(|m| (m.at - t).abs() <= 2.0).min_by(|a, b| (a.at - t).abs().total_cmp(&(b.at - t).abs()))
    }

    /// The point `t` comes to on the scrubber, kept to the match.
    pub fn seek(&self, t: f32) -> f32 {
        t.clamp(0.0, self.end)
    }
}

/// "1:18" for 78 seconds in.
pub fn clock(t: f32) -> String {
    let s = t.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

// ------------------------------------------------------------- the page ---

/// Where a match's log is read from.
pub enum Source {
    /// One of the demo robot's made-up matches, by its log's name.
    Demo(String),
    File(PathBuf),
}

impl Source {
    fn read(&self) -> Result<Timeline, String> {
        match self {
            Source::Demo(name) => {
                let spec = crate::offline::synth::demo_event().into_iter().find(|s| s.name == *name).ok_or("not one of the demo's matches")?;
                Timeline::read(&crate::offline::synth::write(&spec)[..]).map_err(|e| e.to_string())
            }
            Source::File(path) => {
                let file = std::fs::File::open(path).map_err(|_| "the log is not on this handheld".to_owned())?;
                Timeline::read(io::BufReader::with_capacity(64 * 1024, file)).map_err(|e| e.to_string())
            }
        }
    }
}

/// The speeds play steps through.
const SPEEDS: [f32; 3] = [1.0, 2.0, 4.0];

pub struct Replay {
    title: String,
    loading: Option<JoinHandle<Result<Timeline, String>>>,
    timeline: Option<Box<Timeline>>,
    failed: Option<String>,
    /// Where the match is shown at, on the settle spring when moved by a key.
    head: Animated,
    opens_at: Option<f32>,
    playing: bool,
    speed: usize,
    camera: Camera,
    field_layer: Layer,
    chart_layer: Layer,
    field_rect: Rect,
    calm: bool,
    /// Tells this timeline's layers from another's.
    id: u64,
}

impl Replay {
    /// A match's replay, opening at `at` seconds into it (else its start).
    pub fn open(source: Source, title: &str, at: Option<f64>) -> Replay {
        let loading = std::thread::Builder::new().name("replay".into()).spawn(move || source.read()).ok();
        static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Replay {
            title: title.to_owned(),
            failed: loading.is_none().then(|| "could not start reading".to_owned()),
            loading,
            timeline: None,
            head: Animated::new(0.0, role::settle()).with_rest(0.01),
            opens_at: at.map(|a| a as f32),
            playing: false,
            speed: 0,
            camera: Camera::default(),
            field_layer: Layer::default(),
            chart_layer: Layer::default(),
            field_rect: Rect::new(0.0, 0.0, 120.0, 60.0),
            calm: false,
            id: IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// The replay of one of the robot's logs, by name: the demo's own made-up
    /// match, or the file in the robot's logs folder.
    pub fn of_log(name: &str, title: &str, logs: &crate::logs::Logs, at: Option<f64>) -> Option<Replay> {
        let source = if logs.is_demo() { Source::Demo(name.to_owned()) } else { Source::File(logs.dir()?.join(name)) };
        Some(Replay::open(source, title, at))
    }

    /// Wait for the log to be read (the galleries and the tests).
    #[cfg(test)]
    pub fn wait(&mut self) {
        while self.loading.is_some() {
            self.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(test)]
    pub fn timeline(&self) -> Option<&Timeline> {
        self.timeline.as_deref()
    }

    #[cfg(test)]
    pub fn at(&self) -> f32 {
        self.head.value()
    }

    #[cfg(test)]
    pub fn set_playing(&mut self, playing: bool, speed: usize) {
        self.playing = playing;
        self.speed = speed;
    }

    /// The reading thread done: the timeline in place, at where it opens.
    fn poll(&mut self) -> bool {
        if !self.loading.as_ref().is_some_and(|h| h.is_finished()) {
            return false;
        }
        match self.loading.take().unwrap().join() {
            Ok(Ok(timeline)) => {
                let at = timeline.seek(self.opens_at.unwrap_or(0.0));
                self.head.jump(at);
                self.timeline = Some(Box::new(timeline));
            }
            Ok(Err(why)) => self.failed = Some(why),
            Err(_) => self.failed = Some("the log could not be read".into()),
        }
        true
    }

    fn go(&mut self, to: f32) {
        let Some(tl) = &self.timeline else { return };
        let to = tl.seek(to);
        if self.calm {
            self.head.jump(to);
        } else {
            self.head.set_with(to, role::settle());
        }
    }

    fn local_field(&self) -> Rect {
        Rect::new(0.0, 0.0, self.field_rect.w, self.field_rect.h)
    }

    fn follow(&mut self) {
        let Some(tl) = &self.timeline else { return };
        if let Some(p) = tl.pose_at(self.head.value()) {
            let red_left = tl.alliance == Some(Alliance::Red);
            let view = self.camera.view(self.local_field(), FIELD_MARGIN, red_left);
            self.camera.track(self.local_field(), FIELD_MARGIN, view.oriented(p.x, p.y), self.calm);
        }
    }
}

/// The field's margin in its tile on this page (it is small here).
const FIELD_MARGIN: f32 = 3.0;

/// Where everything goes, measured from the text size.
struct Places {
    field: Rect,
    readout: Rect,
    chart: Rect,
    lanes: Vec<Rect>,
    scrub: Rect,
    /// Where the plot runs, left to right, in every lane and the scrubber.
    x0: f32,
    x1: f32,
}

fn places(g: &mut Gfx, lanes: usize) -> Places {
    let ty = g.ty;
    let top = g.top().round();
    let bottom = H - 3.0;
    let cap = ty.caption.size;
    let lane_h = (cap + 3.0).round();
    let scrub_h = (cap + 7.0).round();
    let chart_h = 4.0 + lanes as f32 * lane_h + lanes.saturating_sub(1) as f32 * 2.0 + 4.0 + scrub_h + 4.0;
    let chart = Rect::new(PAD, bottom - chart_h, W - 2.0 * PAD, chart_h);
    let row_h = chart.y - GAP - top;
    let field_w = ((row_h - 2.0 * FIELD_MARGIN) * field::LENGTH / field::WIDTH + 2.0 * FIELD_MARGIN).round();
    let field = Rect::new(PAD, top, field_w, row_h);
    let readout = Rect::new(field.right() + GAP, top, W - PAD - field.right() - GAP, row_h);
    // The values at the cursor, and the play state, to the left of the plot.
    let gutter = g.measure("12.6 v", ty.caption).max(g.measure("4×", ty.caption) + ty.caption.size + 7.0) + 8.0;
    let x0 = chart.x + 4.0 + gutter;
    let x1 = chart.right() - 7.0;
    let lanes: Vec<Rect> = (0..lanes).map(|i| Rect::new(chart.x + 4.0, chart.y + 4.0 + i as f32 * (lane_h + 2.0), chart.w - 8.0, lane_h)).collect();
    let scrub = Rect::new(x0, chart.bottom() - 4.0 - scrub_h, x1 - x0, scrub_h);
    Places { field, readout, chart, lanes, scrub, x0, x1 }
}

/// The charts there are for a timeline: battery, CAN, current, as logged.
fn lanes_of(tl: &Timeline) -> Vec<(&Series, Lane)> {
    let mut out = Vec::new();
    if !tl.battery.is_empty() {
        out.push((&tl.battery, Lane::Battery));
    }
    if !tl.can.is_empty() {
        out.push((&tl.can, Lane::Can));
    }
    if !tl.current.is_empty() {
        out.push((&tl.current, Lane::Current));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Lane {
    Battery,
    Can,
    Current,
}

impl Lane {
    /// The range a lane's chart spans: the battery from under its brownout
    /// line to a full battery, CAN 0 to 100%, the current from 0.
    fn span(self, s: &Series, end: f32) -> (f32, f32) {
        let (lo, hi) = s.range(0.0, end).unwrap_or((0.0, 1.0));
        match self {
            Lane::Battery => (lo.min(BROWNOUT_VOLTS - 0.4), hi.max(12.8)),
            Lane::Can => (0.0, 1.0),
            Lane::Current => (0.0, (hi * 1.1).max(10.0)),
        }
    }

    fn value(self, v: f32) -> String {
        match self {
            Lane::Battery => format!("{v:.1} v"),
            Lane::Can => format!("{:.0}%", v * 100.0),
            Lane::Current => format!("{v:.0} a"),
        }
    }
}

impl Screen for Replay {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Logs"
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("space plays · ↑↓ moments")
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        let Some(tl) = &self.timeline else { return Nav::Stay };
        let (now, end) = (self.head.target(), tl.end);
        let (prev, next) = (tl.prev_mark(now).map(|m| m.at), tl.next_mark(now).map(|m| m.at));
        let (r, calm) = (self.local_field(), self.calm);
        match key {
            Key::Char(' ') | Key::Enter => {
                self.playing = !self.playing;
                if self.playing && now >= end - 0.05 {
                    self.head.jump(0.0);
                }
            }
            Key::Left => self.go(now - 5.0),
            Key::Right => self.go(now + 5.0),
            Key::Up => {
                if let Some(at) = prev {
                    self.go(at);
                }
            }
            Key::Down => {
                if let Some(at) = next {
                    self.go(at);
                }
            }
            Key::Home => self.go(0.0),
            Key::End => self.go(end),
            Key::Char('1') => self.speed = 0,
            Key::Char('2') => self.speed = 1,
            Key::Char('4') => self.speed = 2,
            Key::Char('s') => self.speed = (self.speed + 1) % SPEEDS.len(),
            Key::Char('+' | '=') => {
                self.camera.step_zoom(r, FIELD_MARGIN, 1, calm);
                self.follow();
            }
            Key::Char('-' | '_') => self.camera.step_zoom(r, FIELD_MARGIN, -1, calm),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moved = self.head.tick(dt);
        if moved {
            self.follow();
        }
        moved | self.camera.tick(dt)
    }

    /// Reading the log, then playing: the match moves on in real time,
    /// drawn 30 times a second, without holding the frame beat.
    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        if self.loading.is_some() {
            return self.poll();
        }
        let Some(end) = self.timeline.as_ref().map(|t| t.end) else { return false };
        if !self.playing {
            return false;
        }
        let step = dt * SPEEDS[self.speed];
        if self.head.value() + step >= end {
            self.head.jump(end);
            self.playing = false;
        } else {
            self.head.shift(step);
        }
        self.follow();
        true
    }

    fn ambient(&self) -> Option<Duration> {
        if self.loading.is_some() {
            Some(Duration::from_millis(50))
        } else if self.playing {
            Some(Duration::from_millis(33))
        } else {
            None
        }
    }

    fn keeps_awake(&self) -> bool {
        self.playing
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.calm = g.calm;
        if let Some(why) = &self.failed {
            g.empty("description", why);
            return;
        }
        let look = Look::of(&t);
        let lanes = self.timeline.as_deref().map_or(3, |tl| lanes_of(tl).len().max(1));
        let p = places(g, lanes);
        self.field_rect = p.field;
        let red_left = self.timeline.as_ref().is_some_and(|tl| tl.alliance == Some(Alliance::Red));
        let color = self.timeline.as_ref().and_then(|tl| tl.alliance).map_or(t.ink, Alliance::color);

        // The field, the whole match's path on it faintly (one layer).
        let view = self.camera.view(self.local_field(), FIELD_MARGIN, red_left);
        let radius = g.radius(18.0).min(12.0);
        let id = if self.timeline.is_some() { self.id } else { 0 };
        let path: Vec<(f32, f32)> = match &self.timeline {
            Some(tl) => tl.poses.iter().filter(|(at, _)| (0.0..=tl.end).contains(at)).map(|(_, p)| view.to_screen(p.x, p.y)).collect(),
            None => Vec::new(),
        };
        self.field_layer.show(g, p.field, radius, field::field_key(&view, &look, id), |c| {
            field::paint(c, &view, &look);
            c.polyline(&path, 1.0, color, 0.2);
        });
        let Some(tl) = self.timeline.as_deref() else {
            let r = p.readout;
            g.surface(r, radius, t.surface1);
            g.text_at(r.cx(), r.cy(), Align::Center, "reading the log", ty.caption, t.dim);
            g.surface(p.chart, radius, t.surface1);
            return;
        };
        let now = self.head.value().clamp(0.0, tl.end);
        let on = view.moved(p.field.x + g.ox, p.field.y);
        g.canvas.set_clip(p.field.offset(g.ox, 0.0));
        if let Some(pose) = tl.pose_at(now) {
            field::draw_trail(g, &on, tl.poses.iter(), now, TRAIL, color);
            field::draw_robot(g, &on, pose, color, look.ring);
        }
        g.canvas.clear_clip();
        if tl.poses.is_empty() {
            field::chip(g, p.field.cx(), p.field.cy(), Align::Center, "no pose logged", t.dim);
        }

        // The readout: where in the match, and what happened there.
        let r = p.readout;
        g.surface(r, radius, t.surface1);
        let (phase, left) = tl.phase(now);
        let moment = tl.mark_near(now).copied();
        let inner = r.inset(8.0);
        let three = r.h >= ty.caption.size * 2.0 + ty.label.size + 30.0;
        let display = ty.display((r.h - ty.label.size - if three { ty.caption.size + 22.0 } else { 16.0 }).clamp(18.0, 30.0) / ty.display_scale);
        let mut y = inner.y;
        if three {
            let words = match phase {
                Phase::Before | Phase::After => phase.name().to_owned(),
                _ => format!("{} · {} left", phase.name(), clock(left)),
            };
            g.text_fit(inner.x, y + ty.caption.size / 2.0, &words, ty.caption, t.dim, inner.w);
            y += ty.caption.size + 4.0;
        }
        let time_cy = y + display.size * 0.42;
        g.text_at(inner.x, time_cy, Align::Left, &clock(now), display, t.ink);
        if !three {
            let words = phase.name();
            g.text_at(inner.right(), time_cy, Align::Right, words, ty.caption, t.dim);
        }
        let line_cy = inner.bottom() - ty.label.size / 2.0;
        match moment {
            Some(m) => {
                let level = match m.level() {
                    Level::Fault => g.fault(),
                    Level::Warn => t.amber_ink,
                    Level::Fine => t.dim,
                };
                g.dot(inner.x + 3.0, line_cy, 3.0, level);
                g.text_fit(inner.x + 11.0, line_cy, &m.words(), ty.label, t.ink, inner.w - 11.0);
            }
            None => {
                let words = match (tl.next_mark(now), tl.prev_mark(now)) {
                    (Some(m), _) => format!("↓ {} at {}", if m.kind == MarkKind::Brownout { "brownout" } else { "CAN spike" }, clock(m.at)),
                    (None, Some(m)) => format!("↑ {} at {}", if m.kind == MarkKind::Brownout { "brownout" } else { "CAN spike" }, clock(m.at)),
                    (None, None) if tl.truncated => "the log ends early".to_owned(),
                    (None, None) => "nothing stood out".to_owned(),
                };
                g.text_fit(inner.x, line_cy, &words, ty.caption, t.dim, inner.w);
            }
        }

        // The charts and the scrubber: drawn once, laid down each frame.
        let lanes = lanes_of(tl);
        let end = tl.end;
        let x_of = |at: f32| p.x0 + (p.x1 - p.x0) * (at / end).clamp(0.0, 1.0);
        let chart = p.chart;
        let local = |r: Rect| r.offset(-chart.x, -chart.y);
        let good = g.good();
        let (fault, amber) = (g.fault(), t.amber_ink);
        let key = field::key_of(|h| {
            use std::hash::Hash;
            (self.id, chart.w as i32, chart.h as i32, p.x0 as i32, t.surface1.to_u32(), t.ink.to_u32(), good.to_u32(), fault.to_u32()).hash(h);
        });
        let scrub = p.scrub;
        let endgame = tl.endgame();
        let auto_end = tl.auto_end;
        let teleop = tl.teleop;
        // The worst drawn last, on top where two are close.
        let mut marks = tl.marks.clone();
        marks.sort_by_key(|m| std::cmp::Reverse(m.level()));
        let color_of = |m: &Mark| if m.kind == MarkKind::Brownout { fault } else { amber };
        // Each lane on its own scale (one axis a chart), in the chart's own
        // pixels: the line, the battery's brownout voltage dotted under it,
        // and each marked moment as a point on the lane it is about.
        let plotted: Vec<(Vec<(f32, f32)>, Option<f32>, Vec<(f32, f32, Color)>)> = lanes
            .iter()
            .zip(&p.lanes)
            .map(|((series, lane), r)| {
                let (lo, hi) = lane.span(series, end);
                let y_of = |v: f32| r.bottom() - 1.5 - chart.y - (r.h - 3.0) * ((v - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
                let pts = decimate(&series.points, 0.0, end, (p.x1 - p.x0).max(1.0) as usize).into_iter().map(|(at, v)| (x_of(at) - chart.x, y_of(v))).collect();
                let reference = (*lane == Lane::Battery).then(|| y_of(BROWNOUT_VOLTS));
                let points = marks
                    .iter()
                    .filter(|m| matches!((m.kind, lane), (MarkKind::Brownout, Lane::Battery) | (MarkKind::CanSpike, Lane::Can)))
                    .map(|m| (x_of(m.at) - chart.x, y_of(m.value), color_of(m)))
                    .collect();
                (pts, reference, points)
            })
            .collect();
        self.chart_layer.show(g, chart, radius, key, |c: &mut Canvas| {
            c.clear(t.surface1);
            for (line, reference, points) in &plotted {
                if let Some(y) = reference {
                    let mut x = p.x0 - chart.x;
                    while x < p.x1 - chart.x {
                        c.fill_rect(Rect::new(x, y - 0.5, 3.0, 1.0), t.faint, 0.8);
                        x += 6.0;
                    }
                }
                c.polyline(line, 1.5, good, 1.0);
                for &(x, y, color) in points {
                    c.circle(x, y, 3.6, t.surface1, 1.0);
                    c.circle(x, y, 2.4, color, 1.0);
                }
            }
            // The scrubber: auto, teleop and the endgame as parts of its track.
            let s = local(scrub);
            let track = |c: &mut Canvas, a: f32, b: f32, color: Color| {
                let (xa, xb) = (x_of(a) - chart.x, x_of(b) - chart.x);
                if xb - xa >= 1.0 {
                    c.rrect(Rect::new(xa, s.y, xb - xa, s.h), (s.h / 2.0).min(6.0), color, 1.0);
                }
            };
            track(c, 0.0, end, t.surface2);
            if let Some(a) = auto_end {
                track(c, 0.0, a, t.surface3);
            }
            if let (Some(e), Some((_, te))) = (endgame, teleop) {
                track(c, e, te, t.surface3);
            }
            for a in [auto_end, endgame].into_iter().flatten() {
                let x = x_of(a) - chart.x;
                c.fill_rect(Rect::new(x - 1.0, s.y, 2.0, s.h), t.surface1, 1.0);
            }
            for m in &marks {
                let x = x_of(m.at) - chart.x;
                c.circle(x, s.y + s.h - 1.5, 4.0, t.surface1, 1.0);
                c.circle(x, s.y + s.h - 1.5, 2.6, color_of(m), 1.0);
            }
        });
        // Over the layer: what each part of the track is, the play state,
        // the part played, the cursor, and each lane's value there.
        let cx = x_of(now);
        let label = |g: &mut Gfx, a: f32, b: f32, word: &str, icon: &str| {
            let (xa, xb) = (x_of(a), x_of(b));
            let icon_size = ty.caption.size + 1.0;
            // The word centred in its part, or beside the cursor where it
            // would sit under it; where the word cannot fit whole, its icon
            // (the robot for auto, the flag for the endgame).
            let clear = |l: f32, r: f32, w: f32| r - l >= w && (cx + 5.0 <= (l + r - w) / 2.0 || cx - 5.0 >= (l + r + w) / 2.0);
            let spans = [(xa, xb), ((cx + 5.0).max(xa), xb), (xa, (cx - 5.0).min(xb))];
            let w = g.measure(word, ty.caption) + 2.0;
            if let Some((l, r)) = spans.into_iter().find(|(l, r)| clear(*l, *r, w)) {
                g.text_at((l + r) / 2.0, scrub.cy() - 1.0, Align::Center, word, ty.caption, t.dim);
            } else if let Some((l, r)) = spans.into_iter().find(|(l, r)| !icon.is_empty() && clear(*l, *r, icon_size + 4.0)) {
                g.icon(icon, false, (l + r) / 2.0, scrub.cy() - 1.0, icon_size, t.dim);
            }
        };
        if let Some(a) = auto_end {
            label(g, 0.0, a, "auto", "smart_toy");
        }
        let teleop_end = endgame.or(teleop.map(|t| t.1)).unwrap_or(end);
        label(g, teleop.map_or(auto_end.unwrap_or(0.0), |t| t.0), teleop_end, "teleop", "");
        if let (Some(e), Some((_, te))) = (endgame, teleop) {
            label(g, e, te, "endgame", "flag");
        }
        g.canvas.fill_rect(Rect::new(p.x0 + 2.0 + g.ox, scrub.bottom() - 2.0, (cx - p.x0 - 2.0).max(0.0), 2.0), t.signal, 1.0);
        let top = p.lanes.first().map_or(scrub.y, |r| r.y);
        g.canvas.fill_rect(Rect::new(cx - 0.75 + g.ox, top, 1.5, scrub.bottom() - top), t.ink, 0.9);
        g.surface(Rect::new(cx - 2.5, scrub.y - 1.0, 5.0, scrub.h + 2.0), 2.5, t.ink);
        for ((series, lane), r) in lanes.iter().zip(&p.lanes) {
            let value = series.at(now).map(|v| lane.value(v)).unwrap_or_else(|| "–".into());
            g.text_at(r.x + 4.0, r.cy(), Align::Left, &value, ty.caption, t.dim);
        }
        let state = if self.playing { "pause" } else { "play_arrow" };
        let icon = ty.caption.size + 3.0;
        g.icon(state, true, chart.x + 4.0 + icon / 2.0 + 2.0, scrub.cy(), icon, t.ink);
        g.text_at(chart.x + 8.0 + icon, scrub.cy(), Align::Left, &format!("{}×", SPEEDS[self.speed]), ty.caption, t.dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q14() -> Timeline {
        let spec = crate::offline::synth::demo_event().into_iter().find(|s| s.name.contains("Q14")).unwrap();
        Timeline::read(&crate::offline::synth::write(&spec)[..]).unwrap()
    }

    #[test]
    fn a_match_reads_into_its_phases_and_moments() {
        let tl = q14();
        // Auto 20 s, a half-second pause, teleop to 2:40.5.
        assert!((tl.auto_end.unwrap() - 20.0).abs() < 0.2, "{:?}", tl.auto_end);
        let (start, end) = tl.teleop.unwrap();
        assert!((start - 20.5).abs() < 0.2 && (end - 160.5).abs() < 0.2, "{start} {end}");
        assert!((tl.end - end).abs() < 1e-3);
        assert!((tl.endgame().unwrap() - 130.5).abs() < 0.2);
        assert_eq!(tl.phase(5.0).0, Phase::Auto);
        assert_eq!(tl.phase(20.2).0, Phase::Pause);
        let (phase, left) = tl.phase(78.0);
        assert_eq!(phase, Phase::Teleop);
        assert!((left - 82.5).abs() < 0.3);
        assert_eq!(tl.phase(140.0).0, Phase::Endgame);
        assert_eq!(tl.phase(170.0).0, Phase::After);
        // The brownout at 1:18, as the offline analysis finds it, and the
        // CAN spike in the scramble round it.
        let brownout = tl.marks.iter().find(|m| m.kind == MarkKind::Brownout).unwrap();
        assert!((brownout.at - 78.0).abs() < 0.06, "{}", brownout.at);
        assert!((brownout.value - 6.3).abs() < 0.05);
        assert_eq!(brownout.words(), "browned out · 6.3 v");
        let facts = crate::offline::facts::analyse(&crate::offline::synth::write(&crate::offline::synth::demo_event()[0])[..], "FRC_20261116_101500_NYRO_Q14.wpilog", 0).unwrap();
        let fact = facts.facts.iter().find(|f| f.kind == crate::offline::facts::Kind::Brownout).unwrap();
        assert!((fact.at.unwrap() as f32 - brownout.at).abs() < 0.06, "the same moment as the analysis");
        let can = tl.marks.iter().find(|m| m.kind == MarkKind::CanSpike).unwrap();
        assert!((76.0..82.0).contains(&can.at) && (can.value - 0.72).abs() < 0.01);
        // Its pose: AdvantageKit's, the robot on the blue side.
        assert_eq!(tl.pose_source, Some("advantagekit"));
        assert_eq!(tl.alliance, Some(Alliance::Blue));
        assert!(tl.poses.len() > 1000);
        assert!(tl.battery.points.len() > 1000 && !tl.can.is_empty() && !tl.current.is_empty());
    }

    #[test]
    fn seeking_and_jumping_to_a_moment() {
        let tl = q14();
        assert_eq!(tl.seek(-4.0), 0.0);
        assert_eq!(tl.seek(999.0), tl.end);
        // From the start, the next moment; from past it, back to it.
        let first = tl.next_mark(0.0).unwrap();
        assert!(first.at > 70.0 && first.at < 80.0);
        let brownout = tl.marks.iter().find(|m| m.kind == MarkKind::Brownout).unwrap();
        assert_eq!(tl.prev_mark(100.0).map(|m| m.at), tl.marks.iter().rev().find(|m| m.at < 100.0).map(|m| m.at));
        assert_eq!(tl.next_mark(brownout.at), tl.marks.iter().find(|m| m.at > brownout.at + 0.05));
        // Near the brownout, it is the one said; nowhere near one, none.
        assert_eq!(tl.mark_near(brownout.at - 0.5).map(|m| m.kind), Some(MarkKind::Brownout));
        assert!(tl.mark_near(30.0).is_none());
        // The robot between two logged poses is part way between them.
        let (a, b) = (tl.poses[400], tl.poses[401]);
        let mid = tl.pose_at((a.0 + b.0) / 2.0).unwrap();
        assert!((mid.x - (a.1.x + b.1.x) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn the_replay_opens_at_the_moment_it_was_asked_for() {
        // Logs' summary of Q14: the brownout's highlight carries its moment.
        let logs = crate::logs::Logs::demo();
        let row = logs.list().into_iter().find(|r| r.name.contains("Q14")).unwrap();
        let digest = row.digest.unwrap();
        let at = digest.moment("1 brownout").expect("the brownout's moment");
        assert!((at - 78.0).abs() < 0.06);
        let mut replay = Replay::of_log(&row.name, "Q14", &logs, Some(at)).unwrap();
        replay.wait();
        assert!((replay.at() - 78.0).abs() < 0.06, "{}", replay.at());
        // Playing at 2×: a second of real time is two of the match, drawn
        // without counting as motion; at the end it stops.
        let robot = Robot::none();
        replay.set_playing(true, 1);
        assert!(replay.ambient_tick(1.0, &robot));
        assert!((replay.at() - 80.0).abs() < 0.1);
        assert_eq!(replay.ambient(), Some(Duration::from_millis(33)));
        replay.ambient_tick(500.0, &robot);
        assert!(!replay.playing && (replay.at() - replay.timeline().unwrap().end).abs() < 1e-3);
        assert_eq!(replay.ambient(), None, "at rest the beat rests");
    }

    #[test]
    fn a_series_keeps_its_extremes_when_it_is_long() {
        let mut s = Series::new(true);
        for k in 0..(CAP * 3) {
            let t = k as f32 * 0.02;
            s.push(t, if k == CAP * 2 + 7 { 6.0 } else { 12.0 });
        }
        s.finish();
        assert!(s.points.len() <= CAP);
        assert_eq!(s.range(0.0, 1e9).unwrap().0, 6.0, "the dip survives");
        assert!(s.points.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn charts_are_decimated_to_two_points_a_column_keeping_the_dip() {
        // 150 s at 50 a second, a 0.4 s dip at 78 s.
        let pts: Vec<Sample> = (0..7500).map(|k| k as f32 / 50.0).map(|t| (t, if (78.0..78.4).contains(&t) { 6.3 } else { 12.0 + (t * 0.7).sin() * 0.4 })).collect();
        let out = decimate(&pts, 0.0, 150.0, 200);
        assert!(out.len() <= 400, "{}", out.len());
        assert!(out.iter().any(|(_, v)| (*v - 6.3).abs() < 1e-4), "the dip is drawn");
        assert!(out.windows(2).all(|w| w[0].0 <= w[1].0), "in time order");
        // A flat line is a point a column; nothing outside the range.
        let flat: Vec<Sample> = (0..1000).map(|k| (k as f32 * 0.1, 5.0)).collect();
        assert_eq!(decimate(&flat, 0.0, 100.0, 50).len(), 50);
        assert!(decimate(&flat, 20.0, 30.0, 10).iter().all(|(t, _)| (20.0..=30.0).contains(t)));
        assert!(decimate(&flat, 0.0, 10.0, 0).is_empty());
        assert_eq!(clock(78.0), "1:18");
    }
}
