//! The Camera's kit (0.9.4): what a photo goes through, timed, and the
//! tools around the dials. camera.rs holds the screen; this holds the
//! decisions and the overlays' shapes, each checked on its own.
//!
//! - **The still path, timed.** Each photo's marks as the shell sees them
//!   (the key, the order sent, the sensor exposing, done, the picture in,
//!   written: [`Timeline`]), with catalyst-camera's own marks beside them,
//!   in the log for every photo and, in pro mode on `t`, as a readout
//!   ("last shot: 0.42 s capture · 0.7 s save"). "Hold still" shows only
//!   while the sensor is exposing the photo, and only when that is long
//!   enough to matter (a slow shutter, detail mode's burst, a bracketing
//!   set), with the seconds left.
//! - **Detail mode's "auto"** ([`detail_burst`]): a burst only in real low
//!   light, never with RAW, a set or a long exposure.
//! - **The self-timer** (`c`): off, 2 s or 10 s.
//! - **Grids** (`g`): thirds, or squares; inside the aspect's frame.
//! - **The level** (`h`, [`Horizon`]): the horizon from the motion sensor,
//!   a line that turns ice when it is level; lying flat, a bubble.
//! - **Aspect** (`o`, [`Aspect`]): 4:3 (the whole sensor), 3:2, 16:9 and
//!   1:1, a framing guide over the picture and a crop as the photo is saved.
//! - **Bracketing** (`b`, [`Bracket`]): 3 or 5 frames, ±1 or ±2 EV apart,
//!   saved as a set beside each other (`IMG_…_ev-1.jpg`).
//! - **Programs** (`m`, [`Program`]): P, shutter priority, ISO priority and
//!   manual; the dial left on auto is filled by the camera (pro.rs's still).
//! - **Long exposures** up to the IMX219's 11.77 s (pro.rs's shutter dial).
//! - **Grey card** (`y`, [`grey_gains`]): white balance from the middle of
//!   the picture.
//! - **Histogram** (`u`): brightness, RGB, or none; **false colour** (`x`,
//!   after the zebras, [`false_colour`]).
//! - **Presets** C1 to C3 ([`Setup`]): `1` `2` `3` recall, `!` `@` `#` keep.
//! - **The intervalometer** (`v`, [`Interval`], [`Run`]): a photo every so
//!   often, so many times; the screen dims while it runs and the handheld
//!   stays awake.
//!
//! The sizes, colours and timings here that the Bezel documents do not give
//! are my own tuning, said where they are.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::burst::DetailMode;
use crate::filters::Look;
use crate::imu::Vec3;
use crate::pro::{self, Metered, Pro, StillControls, Wb, MAX_GAIN};

// ------------------------------------------------------------ still path ---

/// The marks along one photo's way, in the order they come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// The shutter key (or the timer, or the intervalometer).
    Key,
    /// The order handed to the camera program.
    Request,
    /// The sensor exposing the photo (catalyst-camera's kind 6).
    Exposing,
    /// The sensor done with it (kind 7): the hand may move.
    Exposed,
    /// Its picture copied out, the preview back (kind 4).
    Captured,
    /// Written to the card (kind 2).
    Written,
}

/// What the sensor is exposing: one frame, detail mode's burst, a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposing {
    Still,
    Detail,
    Bracket,
    /// Night's and HDR's stacks (1.4.0): merged as they come.
    Night,
    Hdr,
    /// Light trails and a burst (1.5.0): the brightest of each frame laid
    /// over the last, and the sharpest of about ten kept.
    Trails,
    Burst,
}

impl Exposing {
    pub fn from_word(word: &str) -> Exposing {
        match word {
            "detail" => Exposing::Detail,
            "bracket" => Exposing::Bracket,
            "night" => Exposing::Night,
            "hdr" => Exposing::Hdr,
            "trails" => Exposing::Trails,
            "burst" => Exposing::Burst,
            _ => Exposing::Still,
        }
    }

    /// A run of frames the camera program tells as they come (kind 8),
    /// whose "hold still" is the Camera's own (the ring, the trails' chip).
    pub fn stack(self) -> bool {
        matches!(self, Exposing::Night | Exposing::Hdr | Exposing::Trails | Exposing::Burst)
    }
}

/// A stack's frames so far, as catalyst-camera says after each (kind 8:
/// "<taken> <planned> <merged> <ms left>").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StackNews {
    pub taken: u32,
    pub planned: u32,
    pub merged: u32,
    pub left_ms: u64,
}

impl StackNews {
    pub fn read(line: &str) -> Option<StackNews> {
        let mut w = line.split_whitespace().map(|v| v.parse::<u64>().ok());
        let (taken, planned, merged, left_ms) = (w.next()??, w.next()??, w.next()??, w.next()??);
        (planned > 0).then_some(StackNews { taken: taken as u32, planned: planned as u32, merged: merged as u32, left_ms })
    }
}

/// A saved photo's stack, from catalyst-camera's line ("… night 5 8 …"):
/// what it was for, the frames merged and asked; light trails' frames and
/// milliseconds ("trails 186 12400"); a burst's frame kept and of how many
/// ("best 4 10").
pub fn read_stacked(took: &str) -> Option<(Exposing, u32, u32)> {
    let words: Vec<&str> = took.split_whitespace().collect();
    words.windows(3).find_map(|w| {
        let what = match w[0] {
            "night" => Exposing::Night,
            "hdr" => Exposing::Hdr,
            "trails" => Exposing::Trails,
            "best" => Exposing::Burst,
            _ => return None,
        };
        Some((what, w[1].parse().ok()?, w[2].parse().ok()?))
    })
}

/// "Hold still" is shown only for an exposure at least this long: shorter,
/// the shutter's flash says it (my own tuning: a pill that shows for less
/// than a quarter of a second only flickers).
pub const HOLD_AT_LEAST: Duration = Duration::from_millis(250);

/// One photo's way through the still path, as the shell sees it.
#[derive(Clone, Debug)]
pub struct Timeline {
    pub path: PathBuf,
    marks: Vec<(Mark, Instant)>,
    /// What the sensor is exposing, and how long it takes at most.
    pub exposing: Option<(Exposing, Duration, u32)>,
    /// catalyst-camera's own marks ("room 0 stop 28 config 81 …").
    pub camera: String,
}

impl Timeline {
    pub fn new(path: PathBuf, key: Instant) -> Self {
        Timeline { path, marks: vec![(Mark::Key, key)], exposing: None, camera: String::new() }
    }

    /// A mark, kept only in its place: none goes back before one already
    /// made (a mark the program repeats, as a retried "exposing", moves
    /// to its new time; a late one from an earlier step is ignored).
    pub fn mark(&mut self, mark: Mark, at: Instant) -> bool {
        if self.marks.iter().any(|(m, _)| *m > mark) {
            return false;
        }
        self.marks.retain(|(m, _)| *m != mark);
        self.marks.push((mark, at));
        true
    }

    pub fn at(&self, mark: Mark) -> Option<Instant> {
        self.marks.iter().find(|(m, _)| *m == mark).map(|(_, t)| *t)
    }

    /// The furthest mark reached.
    pub fn reached(&self) -> Mark {
        self.marks.iter().map(|(m, _)| *m).max().unwrap_or(Mark::Key)
    }

    /// The marks in order, each since the key: `[(Request, 0.002), …]`.
    pub fn since_key(&self) -> Vec<(Mark, f32)> {
        let key = self.at(Mark::Key).unwrap_or_else(Instant::now);
        let mut out: Vec<(Mark, f32)> = self.marks.iter().map(|(m, t)| (*m, t.saturating_duration_since(key).as_secs_f32())).collect();
        out.sort_by_key(|(m, _)| *m);
        out
    }

    /// From the key to the picture in (the photo taken), and from then to
    /// its file on the card.
    pub fn capture_s(&self) -> Option<f32> {
        Some(self.at(Mark::Captured)?.saturating_duration_since(self.at(Mark::Key)?).as_secs_f32())
    }

    pub fn save_s(&self) -> Option<f32> {
        Some(self.at(Mark::Written)?.saturating_duration_since(self.at(Mark::Captured)?).as_secs_f32())
    }

    /// The readout: "last shot: 0.42 s capture · 0.7 s save", shorter
    /// forms for a narrow place (the longest that fits is used).
    pub fn readout(&self) -> [String; 4] {
        let s = |v: Option<f32>| v.map_or("…".to_owned(), |v| if v < 10.0 { format!("{v:.2} s").replace(".00 s", " s") } else { format!("{v:.0} s") });
        let save = |v: Option<f32>| v.map_or("…".to_owned(), |v| if v < 10.0 { format!("{v:.1} s") } else { format!("{v:.0} s") });
        let (c, w) = (s(self.capture_s()), save(self.save_s()));
        [format!("last shot: {c} capture · {w} save"), format!("{c} capture · {w} save"), format!("{c} · save {w}"), format!("{c} · {w}")]
    }

    /// Its line for the log: every mark since the key, in milliseconds,
    /// and the camera program's own.
    pub fn log_line(&self) -> String {
        let name = |m: Mark| match m {
            Mark::Key => "key",
            Mark::Request => "request",
            Mark::Exposing => "exposing",
            Mark::Exposed => "exposed",
            Mark::Captured => "captured",
            Mark::Written => "written",
        };
        let marks = self.since_key().iter().map(|(m, s)| format!("{} {:.0}", name(*m), s * 1000.0)).collect::<Vec<_>>().join(" ");
        let file = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if self.camera.is_empty() {
            format!("camera: shot {file}: {marks} ms")
        } else {
            format!("camera: shot {file}: {marks} ms (camera: {})", self.camera)
        }
    }

    /// "Hold still", when the sensor is exposing something long enough to
    /// say so: its words and the seconds left at most.
    pub fn hold_still(&self, now: Instant) -> Option<(String, f32)> {
        if self.reached() != Mark::Exposing {
            return None;
        }
        let (what, most, _) = self.exposing?;
        if most < HOLD_AT_LEAST {
            return None;
        }
        let since = now.saturating_duration_since(self.at(Mark::Exposing)?);
        let left = most.saturating_sub(since).as_secs_f32().max(0.1);
        let words = match what {
            Exposing::Still => "hold still",
            Exposing::Detail => "detail · hold still",
            Exposing::Bracket => "bracket · hold still",
            Exposing::Night => "night · hold still",
            Exposing::Hdr => "hdr · hold still",
            Exposing::Trails => "trails · hold still",
            Exposing::Burst => "burst · hold still",
        };
        Some((words.to_owned(), left))
    }

    /// Given up on, and why: no word from the camera program for too long
    /// at the step it is at (the order waiting for room, the camera
    /// changing mode and trying the ways it has, then the exposure). The
    /// exposure's wait is longer than the camera program's own last try
    /// (8.8 s and the exposure: still.h's deadline_ms, twice): at 8 s the
    /// shell gave up first, and that try's photo would have been lost.
    pub fn overdue(&self, now: Instant) -> Option<&'static str> {
        let since = |m: Mark| self.at(m).map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        match self.reached() {
            Mark::Key | Mark::Request if since(Mark::Key) > Duration::from_secs(15) => Some("the camera did not start the photo"),
            // A stack's own bound (still.h's stack_ms) is each frame's wait
            // over, past what it should take: three times that, and more.
            Mark::Exposing if since(Mark::Exposing) > self.exposing.map_or(Duration::ZERO, |(what, d, _)| if what.stack() { d * 3 } else { d }) + Duration::from_secs(12) => Some("the camera did not finish the photo"),
            Mark::Exposed if since(Mark::Exposed) > Duration::from_secs(8) => Some("the photo's picture did not come"),
            _ => None,
        }
    }
}

/// catalyst-camera's "exposing" line: "<ms> <frames> <what>".
pub fn read_exposing(line: &str) -> (Exposing, Duration, u32) {
    let mut words = line.split_whitespace();
    let ms = words.next().and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
    let frames = words.next().and_then(|w| w.parse::<u32>().ok()).unwrap_or(1);
    (Exposing::from_word(words.next().unwrap_or("still")), Duration::from_millis(ms), frames)
}

// ------------------------------------------------------------ detail auto ---

/// Detail mode's burst for a photo: `None` for one frame, `Some(false)` for
/// a burst asked outright, `Some(true)` to leave it to the camera program
/// (no metering yet). Never with RAW (it needs the memory), a bracketing
/// set (each frame is its own photo) or an exposure past a quarter of a
/// second (a burst of those would keep the hand still for seconds); on
/// "auto" only in real low light (pro.rs's [`pro::low_light`]).
pub fn detail_burst(mode: DetailMode, metered: Option<&Metered>, still: Option<&StillControls>, raw: bool, set: bool) -> Option<bool> {
    if mode == DetailMode::Off || raw || set || still.is_some_and(|s| s.exposure_us > 250_000) {
        return None;
    }
    match (mode, metered, still) {
        (DetailMode::On, _, _) => Some(false),
        (_, Some(m), Some(s)) => pro::low_light(m, s).then_some(false),
        _ => Some(true),
    }
}

// ----------------------------------------------------------------- timer ---

/// The self-timer's choices, in turn (seconds; 0 is off).
pub const TIMERS: [u32; 3] = [0, 2, 10];

pub fn next_timer(now: u32) -> u32 {
    let at = TIMERS.iter().position(|t| *t == now).unwrap_or(0);
    TIMERS[(at + 1) % TIMERS.len()]
}

// ------------------------------------------------------------------ grid ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grid {
    #[default]
    Off,
    /// Rule of thirds.
    Thirds,
    /// Squares (four across at 4:3, as many as fit the aspect's frame).
    Square,
}

impl Grid {
    pub fn next(self) -> Grid {
        match self {
            Grid::Off => Grid::Thirds,
            Grid::Thirds => Grid::Square,
            Grid::Square => Grid::Off,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Grid::Off => "off",
            Grid::Thirds => "thirds",
            Grid::Square => "square",
        }
    }

    pub fn from_id(id: &str) -> Grid {
        [Grid::Off, Grid::Thirds, Grid::Square].into_iter().find(|g| g.id() == id).unwrap_or_default()
    }

    /// Its lines in `r`, each a 1 px rectangle (x, y, w, h), rounded to
    /// whole pixels: thirds each way, or squares from the middle.
    pub fn lines(self, r: (f32, f32, f32, f32)) -> Vec<(f32, f32, f32, f32)> {
        let (x, y, w, h) = r;
        let mut out = Vec::new();
        match self {
            Grid::Off => {}
            Grid::Thirds => {
                for i in 1..3 {
                    out.push(((x + w * i as f32 / 3.0).round(), y, 1.0, h));
                    out.push((x, (y + h * i as f32 / 3.0).round(), w, 1.0));
                }
            }
            Grid::Square => {
                // Three down the shorter side, and as many across the
                // longer as keep them square (4:3 gives 4 x 3).
                let side = w.min(h) / 3.0;
                let across = (w / side).round().max(1.0) as i32;
                let down = (h / side).round().max(1.0) as i32;
                let (sw, sh) = (w / across as f32, h / down as f32);
                for i in 1..across {
                    out.push(((x + sw * i as f32).round(), y, 1.0, h));
                }
                for j in 1..down {
                    out.push((x, (y + sh * j as f32).round(), w, 1.0));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- aspect ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Aspect {
    /// The sensor's own 4:3: the whole picture.
    #[default]
    Full,
    ThreeTwo,
    SixteenNine,
    Square,
}

pub const ASPECTS: [Aspect; 4] = [Aspect::Full, Aspect::ThreeTwo, Aspect::SixteenNine, Aspect::Square];

impl Aspect {
    pub fn ratio(self) -> (u32, u32) {
        match self {
            Aspect::Full => (4, 3),
            Aspect::ThreeTwo => (3, 2),
            Aspect::SixteenNine => (16, 9),
            Aspect::Square => (1, 1),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Aspect::Full => "4:3",
            Aspect::ThreeTwo => "3:2",
            Aspect::SixteenNine => "16:9",
            Aspect::Square => "1:1",
        }
    }

    pub fn from_name(name: &str) -> Aspect {
        ASPECTS.into_iter().find(|a| a.name() == name).unwrap_or_default()
    }

    pub fn next(self) -> Aspect {
        let at = ASPECTS.iter().position(|a| *a == self).unwrap_or(0);
        ASPECTS[(at + 1) % ASPECTS.len()]
    }

    /// Its word in a still order, none for the whole picture.
    pub fn word(self) -> Option<String> {
        (self != Aspect::Full).then(|| format!("aspect={}", self.name()))
    }

    /// The photo's crop of a `w` x `h` frame (x, y, w, h): its middle, at
    /// sides of four pixels and an even corner, as YUV 4:2:0 needs, the
    /// same sums as catalyst-camera's still.h.
    pub fn crop(self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        if self == Aspect::Full {
            return (0, 0, w, h);
        }
        let (a, b) = self.ratio();
        let four = |v: f64| ((v + 2.0) / 4.0) as u32 * 4;
        let (mut cw, mut ch) = (w, h);
        if u64::from(a) * u64::from(h) >= u64::from(b) * u64::from(w) {
            ch = h.min(four(f64::from(w) * f64::from(b) / f64::from(a)));
        } else {
            cw = w.min(four(f64::from(h) * f64::from(a) / f64::from(b)));
        }
        (((w - cw) / 2) & !1, ((h - ch) / 2) & !1, cw, ch)
    }

    /// Its frame within the viewfinder `r` (x, y, w, h): the same share of
    /// the picture the crop keeps.
    pub fn frame(self, r: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = r;
        let (a, b) = self.ratio();
        if self == Aspect::Full {
            return r;
        }
        let want = a as f32 / b as f32;
        if want >= w / h {
            let nh = (w / want).round();
            (x, (y + (h - nh) * 0.5).round(), w, nh)
        } else {
            let nw = (h * want).round();
            ((x + (w - nw) * 0.5).round(), y, nw, h)
        }
    }
}

/// A YUV 4:2:0 frame (packed planes) cropped to (x, y, w, h), as the
/// simulator saves its photos.
pub fn crop_yuv420(yuv: &[u8], w: usize, h: usize, crop: (u32, u32, u32, u32)) -> Vec<u8> {
    let (cx, cy, cw, ch) = (crop.0 as usize, crop.1 as usize, crop.2 as usize, crop.3 as usize);
    let mut out = Vec::with_capacity(cw * ch * 3 / 2);
    for row in cy..cy + ch {
        out.extend_from_slice(&yuv[row * w + cx..row * w + cx + cw]);
    }
    let (u, v) = (w * h, w * h + (w / 2) * (h / 2));
    for plane in [u, v] {
        for row in cy / 2..(cy + ch) / 2 {
            let at = plane + row * (w / 2) + cx / 2;
            out.extend_from_slice(&yuv[at..at + cw / 2]);
        }
    }
    out
}

// --------------------------------------------------------------- program ---

/// Who sets the exposure: the camera (P), the shutter by hand (S), the ISO
/// by hand, or both (M). The dial left on auto is the camera's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Auto,
    Shutter,
    Iso,
    Manual,
}

pub const PROGRAMS: [Program; 4] = [Program::Auto, Program::Shutter, Program::Iso, Program::Manual];

impl Program {
    pub fn of(pro: &Pro) -> Program {
        match (pro.shutter.is_some(), pro.iso.is_some()) {
            (false, false) => Program::Auto,
            (true, false) => Program::Shutter,
            (false, true) => Program::Iso,
            (true, true) => Program::Manual,
        }
    }

    /// As the rail says it, lowercase as Bezel's labels are.
    pub fn word(self) -> &'static str {
        match self {
            Program::Auto => "p",
            Program::Shutter => "s",
            Program::Iso => "iso",
            Program::Manual => "m",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Program::Auto => "auto exposure",
            Program::Shutter => "shutter priority",
            Program::Iso => "iso priority",
            Program::Manual => "manual",
        }
    }

    /// Exif's ExposureProgram (ISO priority has none of its own: normal).
    pub fn exif(self) -> u8 {
        match self {
            Program::Manual => 1,
            Program::Shutter => 4,
            Program::Auto | Program::Iso => 2,
        }
    }

    pub fn next(self) -> Program {
        let at = PROGRAMS.iter().position(|p| *p == self).unwrap_or(0);
        PROGRAMS[(at + 1) % PROGRAMS.len()]
    }

    /// The dials set for this program: a dial taken by hand starts where
    /// the camera had it (the exposure it was taking), one given back to
    /// the camera goes to auto.
    pub fn apply(self, pro: &mut Pro, still: Option<&StillControls>, m: Option<&Metered>) {
        let (us, gain) = match (still, m) {
            (Some(s), _) => (s.exposure_us, s.gain),
            (None, Some(m)) => (m.exposure_us, m.analogue * m.digital),
            _ => (10_000, 1.0),
        };
        let hand_shutter = matches!(self, Program::Shutter | Program::Manual);
        let hand_iso = matches!(self, Program::Iso | Program::Manual);
        pro.shutter = if hand_shutter { pro.shutter.or(Some(pro::nearest_shutter(us))) } else { None };
        pro.iso = if hand_iso { pro.iso.or(Some(pro::nearest_iso(gain))) } else { None };
    }
}

// ------------------------------------------------------------ bracketing ---

/// Exposure bracketing: so many frames, so many stops apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bracket {
    pub frames: u8,
    pub stops: u8,
}

/// The bracketing key's choices, in turn: off, then 3 at ±1, 3 at ±2, 5 at ±1, 5 at ±2.
pub const BRACKETS: [Option<Bracket>; 5] = [None, Some(Bracket { frames: 3, stops: 1 }), Some(Bracket { frames: 3, stops: 2 }), Some(Bracket { frames: 5, stops: 1 }), Some(Bracket { frames: 5, stops: 2 })];

pub fn next_bracket(now: Option<Bracket>) -> Option<Bracket> {
    let at = BRACKETS.iter().position(|b| *b == now).unwrap_or(0);
    BRACKETS[(at + 1) % BRACKETS.len()]
}

/// The shortest exposure asked of the sensor (µs).
pub const SHORTEST_US: u32 = 100;

impl Bracket {
    /// Its steps in stops, in the order taken: the metered one first, then
    /// under and over, then further under and over.
    pub fn evs(self) -> Vec<i32> {
        let s = i32::from(self.stops);
        let mut out = vec![0, -s, s];
        if self.frames >= 5 {
            out.extend([-2 * s, 2 * s]);
        }
        out
    }

    /// "aeb 3 ±1".
    pub fn words(self) -> String {
        format!("aeb {} ±{}", self.frames, self.stops)
    }

    pub fn id(self) -> String {
        format!("{}x{}", self.frames, self.stops)
    }

    pub fn from_id(id: &str) -> Option<Bracket> {
        let (f, s) = id.split_once('x')?;
        let b = Bracket { frames: f.parse().ok()?, stops: s.parse().ok()? };
        BRACKETS.contains(&Some(b)).then_some(b)
    }

    /// Each step's exposure and gain from the photo's own (`base`): the
    /// shutter moves (the lens has no aperture to); with the shutter set
    /// by hand (shutter priority) the gain moves instead; whatever one
    /// cannot give (the gain past 1 to 10, the shutter past the sensor's
    /// limits) the other makes up. Each is (stops, µs, gain).
    pub fn exposures(self, base: &StillControls, program: Program) -> Vec<(i32, u32, f32)> {
        let total = base.exposure_us as f64 * f64::from(base.gain);
        self.evs()
            .into_iter()
            .map(|ev| {
                let want = total * 2f64.powi(ev);
                let (us, gain) = if program == Program::Shutter {
                    let gain = (want / base.exposure_us as f64).clamp(1.0, f64::from(MAX_GAIN));
                    let us = (want / gain).clamp(f64::from(SHORTEST_US), crate::pro::LONGEST_US as f64);
                    (us, gain)
                } else {
                    let us = (want / f64::from(base.gain)).clamp(f64::from(SHORTEST_US), crate::pro::LONGEST_US as f64);
                    let gain = (want / us).clamp(1.0, f64::from(MAX_GAIN));
                    (us, gain)
                };
                (ev, us.round() as u32, gain as f32)
            })
            .collect()
    }
}

/// A set's file for a step: the photo's own name for the metered one,
/// "_ev-1", "_ev+2" beside it for the others.
pub fn set_path(base: &Path, ev: i32) -> PathBuf {
    if ev == 0 {
        return base.to_path_buf();
    }
    let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = base.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "jpg".into());
    base.with_file_name(format!("{stem}_ev{ev:+}.{ext}"))
}

// -------------------------------------------------------------- grey card ---

/// White balance from a grey card: the red and blue gains that make the
/// middle of the picture (its mean `rgb`, as the ISP gave it, gamma and
/// all) as red and blue as it is green, from the gains it was taken with.
/// None, and why, when what is in the middle is no grey to go by.
pub fn grey_gains(rgb: [f32; 3], current: [f32; 2]) -> Result<[f32; 2], &'static str> {
    if rgb.iter().any(|c| *c > 245.0) {
        return Err("too bright to read: aim at grey, out of the light");
    }
    if rgb[1] < 24.0 {
        return Err("too dark to read: more light on the grey");
    }
    // Back to the sensor's light (the ISP's gamma, about 2.2).
    let lin = rgb.map(|c| (c / 255.0).powf(2.2).max(1e-4));
    let (lo, hi) = (lin.iter().copied().fold(f32::MAX, f32::min), lin.iter().copied().fold(0.0, f32::max));
    if hi / lo > 3.0 {
        return Err("too colourful: aim at something grey or white");
    }
    let r = (current[0] * lin[1] / lin[0]).clamp(0.5, 8.0);
    let b = (current[1] * lin[1] / lin[2]).clamp(0.5, 8.0);
    Ok([r, b])
}

/// The mean colour of the middle of an RGB picture (a sixth of it each
/// way), for the grey card.
pub fn middle_mean(rgb: &[u8], w: usize, h: usize) -> [f32; 3] {
    let (x0, x1, y0, y1) = (w * 5 / 12, w * 7 / 12, h * 5 / 12, h * 7 / 12);
    let mut sum = [0f64; 3];
    let mut n = 0f64;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = &rgb[(y * w + x) * 3..(y * w + x) * 3 + 3];
            for c in 0..3 {
                sum[c] += f64::from(p[c]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| (s / n.max(1.0)) as f32)
}

// ----------------------------------------------------- histogram, colour ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HistMode {
    #[default]
    Luma,
    Rgb,
    Off,
}

impl HistMode {
    pub fn next(self) -> HistMode {
        match self {
            HistMode::Luma => HistMode::Rgb,
            HistMode::Rgb => HistMode::Off,
            HistMode::Off => HistMode::Luma,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            HistMode::Luma => "luma",
            HistMode::Rgb => "rgb",
            HistMode::Off => "off",
        }
    }

    pub fn from_id(id: &str) -> HistMode {
        [HistMode::Luma, HistMode::Rgb, HistMode::Off].into_iter().find(|m| m.id() == id).unwrap_or_default()
    }
}

/// A frame's red, green and blue, each in [`pro::BINS`] bins.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbHistogram {
    pub bins: [[u32; pro::BINS]; 3],
}

impl Default for RgbHistogram {
    fn default() -> Self {
        RgbHistogram { bins: [[0; pro::BINS]; 3] }
    }
}

impl RgbHistogram {
    /// Of a YUV 4:2:0 frame, every `step`th pixel each way, as the camera
    /// caught it (before any look), turned to RGB by the preview's rule.
    pub fn of_yuv420(yuv: &[u8], w: usize, h: usize, step: usize) -> RgbHistogram {
        let mut out = RgbHistogram::default();
        let step = step.max(1);
        let (us, vs) = (&yuv[w * h..w * h * 5 / 4], &yuv[w * h * 5 / 4..w * h * 3 / 2]);
        for y in (0..h).step_by(step) {
            for x in (0..w).step_by(step) {
                let l = i32::from(yuv[y * w + x]);
                let c = (y / 2) * (w / 2) + x / 2;
                let (u, v) = (i32::from(us[c]) - 128, i32::from(vs[c]) - 128);
                let rgb = [l + ((359 * v) >> 8), l - ((88 * u + 183 * v) >> 8), l + ((454 * u) >> 8)];
                for (ch, value) in rgb.into_iter().enumerate() {
                    out.bins[ch][(value.clamp(0, 255) as usize * pro::BINS) >> 8] += 1;
                }
            }
        }
        out
    }

    /// Each channel's heights, 0 to 1 of the tallest bin of any channel
    /// (square-rooted, as the brightness histogram is drawn).
    pub fn heights(&self) -> [[f32; pro::BINS]; 3] {
        let top = self.bins.iter().flat_map(|c| c.iter().copied()).max().unwrap_or(0).max(1) as f32;
        let mut out = [[0.0; pro::BINS]; 3];
        for (o, c) in out.iter_mut().zip(&self.bins) {
            for (h, b) in o.iter_mut().zip(c) {
                *h = (*b as f32 / top).sqrt();
            }
        }
        out
    }
}

/// False colour: each brightness (the camera's Y, 0 to 255) as it shows,
/// the bands a cinematographer reads (after ARRI's: purple where black
/// crushes, blue near it, green at a mid grey, pink a stop over it, yellow
/// near white, red clipping), the rest a darkened grey. Built once.
pub fn false_colour() -> &'static [[u8; 3]; 256] {
    static TABLE: std::sync::OnceLock<[[u8; 3]; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [[0u8; 3]; 256];
        for (y, out) in t.iter_mut().enumerate() {
            *out = match y {
                0..=6 => [132, 40, 170],
                7..=25 => [40, 90, 235],
                97..=107 => [70, 200, 90],
                133..=142 => [240, 125, 175],
                247..=252 => [245, 215, 40],
                253..=255 => [235, 45, 40],
                _ => {
                    let g = (y as f32 * 0.8) as u8;
                    [g, g, g]
                }
            };
        }
        t
    })
}

/// A frame's brightness laid over `rgb` in false colour.
pub fn paint_false_colour(y: &[u8], rgb: &mut [u8]) {
    let table = false_colour();
    for (px, v) in rgb.chunks_exact_mut(3).zip(y) {
        px.copy_from_slice(&table[usize::from(*v)]);
    }
}

// ----------------------------------------------------------------- level ---

/// The level: how far the handheld is turned from the horizon, and how far
/// its lens points up or down, held up to shoot; or lying flat (shooting
/// down at a table), its tilt each way, as a bubble.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Horizon {
    /// Turned clockwise (its right end low), degrees.
    pub roll: f32,
    /// Its lens pointing up, degrees (down negative).
    pub pitch: f32,
    pub flat: bool,
}

/// Within this, level (degrees: my own tuning, about what a 227 px wide
/// line can show).
pub const LEVEL_WITHIN: f32 = 0.6;

impl Horizon {
    /// From the way up, in the screen's frame (imu.rs).
    pub fn of(up: Vec3) -> Option<Horizon> {
        let up = up.unit()?;
        // Lying within about 30° of flat, face up or down: the bubble.
        if up.z.abs() > 0.86 {
            let (pitch, roll) = crate::imu::tilt(up);
            return Some(Horizon { roll, pitch, flat: true });
        }
        let roll = crate::imu::edge(up);
        // The lens is on the back: the screen tipping up (towards the sky)
        // points it down.
        let pitch = (-up.z).atan2((up.x * up.x + up.y * up.y).sqrt()).to_degrees();
        Some(Horizon { roll: (roll + 180.0).rem_euclid(360.0) - 180.0, pitch, flat: false })
    }

    pub fn level(&self) -> bool {
        if self.flat {
            self.roll.abs() < LEVEL_WITHIN * 1.5 && self.pitch.abs() < LEVEL_WITHIN * 1.5
        } else {
            self.roll.abs() < LEVEL_WITHIN
        }
    }

    /// Eased toward a new reading (the sensor's jitter smoothed over about
    /// a tenth of a second: my own tuning), angles the short way round.
    pub fn ease(&mut self, to: Horizon, dt: f32) {
        let k = 1.0 - (-dt / 0.1).exp();
        let near = |a: f32, b: f32| a + ((b - a + 180.0).rem_euclid(360.0) - 180.0) * k;
        if self.flat != to.flat {
            *self = to;
            return;
        }
        self.roll = near(self.roll, to.roll);
        self.pitch = near(self.pitch, to.pitch);
    }
}

// --------------------------------------------------------------- presets ---

/// Everything the Camera is set to, kept as a preset (C1 to C3) and
/// recalled whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub pro: Pro,
    pub look: Look,
    pub hue: f32,
    pub raw: bool,
    pub zebras: bool,
    pub false_colour: bool,
    pub hist: HistMode,
    pub grid: Grid,
    pub aspect: Aspect,
    pub bracket: Option<Bracket>,
    pub timer: u32,
    pub level: bool,
    pub night: pro::NightMode,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { pro: Pro::default(), look: Look::Original, hue: 32.0, raw: false, zebras: true, false_colour: false, hist: HistMode::Luma, grid: Grid::Off, aspect: Aspect::Full, bracket: None, timer: 0, level: false, night: pro::NightMode::Auto }
    }
}

impl Setup {
    /// A line of words with no spaces in any value (the preset's line in
    /// the Camera's file, commas between).
    pub fn to_line(&self) -> String {
        let on = |b: bool| if b { "1" } else { "0" };
        format!(
            "{} look={} hue={:.0} raw={} zebras={} false={} hist={} grid={} aspect={} aeb={} timer={} level={} night={}",
            self.pro.to_line(),
            self.look.id(),
            self.hue,
            on(self.raw),
            on(self.zebras),
            on(self.false_colour),
            self.hist.id(),
            self.grid.id(),
            self.aspect.name(),
            self.bracket.map_or("off".to_owned(), Bracket::id),
            self.timer,
            on(self.level),
            self.night.name()
        )
        .replace(' ', ",")
    }

    pub fn from_line(line: &str) -> Setup {
        let line = line.replace(',', " ");
        let mut s = Setup { pro: Pro::from_line(&line), ..Setup::default() };
        for word in line.split_whitespace() {
            let Some((k, v)) = word.split_once('=') else { continue };
            match k {
                "look" => s.look = Look::from_id(v).unwrap_or_default(),
                "hue" => s.hue = v.parse().unwrap_or(s.hue),
                "raw" => s.raw = v == "1",
                "zebras" => s.zebras = v == "1",
                "false" => s.false_colour = v == "1",
                "hist" => s.hist = HistMode::from_id(v),
                "grid" => s.grid = Grid::from_id(v),
                "aspect" => s.aspect = Aspect::from_name(v),
                "aeb" => s.bracket = Bracket::from_id(v),
                "timer" => s.timer = v.parse().ok().filter(|t| TIMERS.contains(t)).unwrap_or(0),
                "level" => s.level = v == "1",
                // Before 1.4.0 on or off (off the usual): both are auto now.
                "night" => s.night = pro::NightMode::from_name(v).unwrap_or_default(),
                _ => {}
            }
        }
        s
    }

    /// What a preset is, briefly, for the note as it is kept or recalled:
    /// "s 1/250 · iso auto · 3:2 · aeb 3 ±1".
    pub fn summary(&self) -> String {
        let mut parts = vec![Program::of(&self.pro).word().to_owned()];
        if let Some(i) = self.pro.shutter {
            parts.push(pro::SHUTTERS[i].label.to_owned());
        }
        if let Some(i) = self.pro.iso {
            parts.push(format!("iso {}", pro::ISOS[i]));
        }
        if self.pro.ev != 0 {
            parts.push(self.pro.ev_words());
        }
        if self.pro.wb != Wb::Auto {
            parts.push(self.pro.wb.words());
        }
        if self.aspect != Aspect::Full {
            parts.push(self.aspect.name().to_owned());
        }
        if let Some(b) = self.bracket {
            parts.push(b.words());
        }
        if self.look != Look::Original {
            parts.push(self.look.name().to_lowercase());
        }
        parts.join(" · ")
    }
}

/// The preset keys: `1` `2` `3` recall, `!` `@` `#` (shift and the same)
/// keep. Which preset, and whether it is kept.
pub fn preset_key(c: char) -> Option<(usize, bool)> {
    match c {
        '1' | '2' | '3' => Some((c as usize - '1' as usize, false)),
        '!' => Some((0, true)),
        '@' => Some((1, true)),
        '#' => Some((2, true)),
        _ => None,
    }
}

// --------------------------------------------------------- intervalometer ---

/// How often, and how many times (0: until stopped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub every_s: u32,
    pub count: u32,
}

impl Default for Interval {
    fn default() -> Self {
        Interval { every_s: 10, count: 50 }
    }
}

/// The intervals offered: two seconds at the least (a photo and its save
/// take about one), half an hour at the most.
pub const EVERY: [u32; 11] = [2, 3, 5, 10, 15, 30, 60, 120, 300, 600, 1800];
pub const COUNTS: [u32; 8] = [10, 25, 50, 100, 250, 500, 999, 0];

impl Interval {
    pub fn step_every(&mut self, by: i32) {
        let at = EVERY.iter().position(|e| *e == self.every_s).unwrap_or(3) as i32;
        self.every_s = EVERY[(at + by).clamp(0, EVERY.len() as i32 - 1) as usize];
    }

    pub fn step_count(&mut self, by: i32) {
        let at = COUNTS.iter().position(|c| *c == self.count).unwrap_or(2) as i32;
        self.count = COUNTS[(at + by).clamp(0, COUNTS.len() as i32 - 1) as usize];
    }

    pub fn every_words(&self) -> String {
        duration_words(self.every_s)
    }

    pub fn count_words(&self) -> String {
        if self.count == 0 { "×∞".into() } else { format!("×{}", self.count) }
    }

    pub fn id(&self) -> String {
        format!("{}x{}", self.every_s, self.count)
    }

    pub fn from_id(id: &str) -> Interval {
        id.split_once('x')
            .and_then(|(e, c)| Some(Interval { every_s: e.parse().ok().filter(|e| EVERY.contains(e))?, count: c.parse().ok().filter(|c| COUNTS.contains(c))? }))
            .unwrap_or_default()
    }
}

/// "2 s", "1 min", "1 min 30 s", "30 min".
pub fn duration_words(s: u32) -> String {
    match (s / 60, s % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// An intervalometer running: the photos taken, and when the next is due.
#[derive(Clone, Debug)]
pub struct Run {
    pub interval: Interval,
    pub taken: u32,
    pub next: Instant,
}

impl Run {
    /// Started now: the first photo at once.
    pub fn start(interval: Interval, now: Instant) -> Run {
        Run { interval, taken: 0, next: now }
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.next && !self.finished()
    }

    pub fn finished(&self) -> bool {
        self.interval.count > 0 && self.taken >= self.interval.count
    }

    /// A photo taken (asked for) at `now`: the next on the run's own beat,
    /// a beat missed (the camera still busy) skipped, not made up.
    pub fn took(&mut self, now: Instant) {
        self.taken += 1;
        let every = Duration::from_secs(u64::from(self.interval.every_s));
        self.next += every;
        while self.next <= now {
            self.next += every;
        }
    }

    /// "12/50 · next 4 s".
    pub fn words(&self, now: Instant) -> String {
        let of = if self.interval.count == 0 { format!("{}", self.taken) } else { format!("{}/{}", self.taken, self.interval.count) };
        let left = self.next.saturating_duration_since(now).as_secs_f32().ceil() as u32;
        format!("{of} · next {}", duration_words(left))
    }
}

/// Seconds without a key before the screen dims while it runs (my own
/// tuning: long enough to see the first photos land).
pub const RUN_DIMS_AFTER: f32 = 6.0;

// ------------------------------------------------------------- the kit ---

/// The kit's state in the Camera: its settings (kept between visits), the
/// photo under way and the last one's timing, the level's reading, the
/// intervalometer, and the overlays worked out.
pub struct Kit {
    pub grid: Grid,
    pub aspect: Aspect,
    pub level: bool,
    pub bracket: Option<Bracket>,
    pub false_colour: bool,
    pub hist: HistMode,
    pub readout: bool,
    pub presets: [Option<Setup>; 3],
    pub interval: Interval,
    pub run: Option<Run>,
    /// The intervalometer's panel, open on its row (0 the interval, 1 the count).
    pub panel: Option<usize>,
    /// The photo under way, and the last one finished.
    pub shot: Option<Timeline>,
    pub last_shot: Option<Timeline>,
    /// Photos taken and still being written, timed until they are.
    pub writing: Vec<Timeline>,
    /// The level: as read (eased), and the motion sensor held for it.
    pub horizon: Option<Horizon>,
    pub imu: Option<crate::imu::Hold>,
    pub overlay: Overlay,
    pub rgb: RgbHistogram,
    /// The words that change with time (a countdown, "next in"), as last
    /// drawn: only a change is a new frame.
    pub drawn: String,
}

impl Default for Kit {
    fn default() -> Self {
        Kit {
            grid: Grid::Off,
            aspect: Aspect::Full,
            level: false,
            bracket: None,
            false_colour: false,
            hist: HistMode::Luma,
            readout: false,
            presets: [None, None, None],
            interval: Interval::default(),
            run: None,
            panel: None,
            shot: None,
            last_shot: None,
            writing: Vec::new(),
            horizon: None,
            imu: None,
            overlay: Overlay::default(),
            rgb: RgbHistogram::default(),
            drawn: String::new(),
        }
    }
}

impl Kit {
    /// Its settings as lines of the Camera's file.
    pub fn lines(&self) -> String {
        let mut out = format!(
            "grid={}\naspect={}\nlevel={}\naeb={}\nfalse={}\nhist={}\nreadout={}\ninterval={}\n",
            self.grid.id(),
            self.aspect.name(),
            u8::from(self.level),
            self.bracket.map_or("off".to_owned(), Bracket::id),
            u8::from(self.false_colour),
            self.hist.id(),
            u8::from(self.readout),
            self.interval.id()
        );
        for (i, p) in self.presets.iter().enumerate() {
            if let Some(p) = p {
                out.push_str(&format!("c{}={}\n", i + 1, p.to_line()));
            }
        }
        out
    }

    /// One line of the Camera's file taken in, if it is the kit's.
    pub fn read(&mut self, key: &str, value: &str) {
        match key {
            "grid" => self.grid = Grid::from_id(value),
            "aspect" => self.aspect = Aspect::from_name(value),
            "level" => self.level = value == "1",
            "aeb" => self.bracket = Bracket::from_id(value),
            "false" => self.false_colour = value == "1",
            "hist" => self.hist = HistMode::from_id(value),
            "readout" => self.readout = value == "1",
            "interval" => self.interval = Interval::from_id(value),
            "c1" | "c2" | "c3" => {
                let i = usize::from(key.as_bytes()[1] - b'1');
                self.presets[i] = Some(Setup::from_line(value));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- overlays ---

/// The overlays that change only when a setting does (the aspect's mask,
/// the grid), worked out once for the viewfinder and kept until then.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    key: Option<(Grid, Aspect, [i32; 4])>,
    /// The aspect's frame, the mask's bars and the grid's lines (x, y, w, h).
    pub frame: (f32, f32, f32, f32),
    pub bars: Vec<(f32, f32, f32, f32)>,
    pub lines: Vec<(f32, f32, f32, f32)>,
}

impl Overlay {
    /// For the viewfinder `vf` with this grid and aspect; recomputed only
    /// when one of them changed. True when it was.
    pub fn update(&mut self, grid: Grid, aspect: Aspect, vf: (f32, f32, f32, f32)) -> bool {
        let key = (grid, aspect, [vf.0 as i32, vf.1 as i32, vf.2 as i32, vf.3 as i32]);
        if self.key == Some(key) {
            return false;
        }
        self.key = Some(key);
        let frame = aspect.frame(vf);
        let (x, y, w, h) = vf;
        self.bars.clear();
        if frame.1 > y {
            self.bars.push((x, y, w, frame.1 - y));
            self.bars.push((x, frame.1 + frame.3, w, y + h - frame.1 - frame.3));
        }
        if frame.0 > x {
            self.bars.push((x, y, frame.0 - x, h));
            self.bars.push((frame.0 + frame.2, y, x + w - frame.0 - frame.2, h));
        }
        self.frame = frame;
        self.lines = grid.lines(frame);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still(us: u32, gain: f32) -> StillControls {
        StillControls { exposure_us: us, gain, gains: Some([1.8, 1.5]), ct: None, sharpness: 1.0, denoise: pro::Denoise::Fast }
    }

    #[test]
    fn the_still_paths_marks_keep_their_order_and_time_the_shot() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut t = Timeline::new(PathBuf::from("/data/photos/IMG_1.jpg"), t0);
        assert!(t.mark(Mark::Request, at(2)));
        assert_eq!(t.hold_still(at(5)), None, "nothing to hold still for before the sensor exposes");
        t.exposing = Some(read_exposing("120 1 still"));
        assert!(t.mark(Mark::Exposing, at(300)));
        assert_eq!(t.hold_still(at(320)), None, "a short exposure: the flash says it, no pill");
        assert!(t.mark(Mark::Exposed, at(420)));
        // A late "exposing" (from an earlier try) cannot go back.
        assert!(!t.mark(Mark::Exposing, at(430)));
        assert!(t.mark(Mark::Captured, at(455)));
        assert!(t.mark(Mark::Written, at(1155)));
        assert_eq!(t.reached(), Mark::Written);
        let order: Vec<Mark> = t.since_key().iter().map(|(m, _)| *m).collect();
        assert_eq!(order, vec![Mark::Key, Mark::Request, Mark::Exposing, Mark::Exposed, Mark::Captured, Mark::Written]);
        assert!((t.capture_s().unwrap() - 0.455).abs() < 1e-3);
        assert!((t.save_s().unwrap() - 0.7).abs() < 1e-3);
        assert_eq!(t.readout()[0], "last shot: 0.46 s capture · 0.7 s save");
        let line = t.log_line();
        assert!(line.contains("key 0 request 2 exposing 300 exposed 420 captured 455 written 1155"), "{line}");
    }

    #[test]
    fn hold_still_shows_only_while_a_long_exposure_or_a_burst_is_taken() {
        let t0 = Instant::now();
        let mut t = Timeline::new(PathBuf::from("a.jpg"), t0);
        t.mark(Mark::Request, t0);
        t.exposing = Some(read_exposing("1800 6 detail"));
        t.mark(Mark::Exposing, t0 + Duration::from_millis(250));
        let (words, left) = t.hold_still(t0 + Duration::from_millis(750)).unwrap();
        assert_eq!(words, "detail · hold still");
        assert!((left - 1.3).abs() < 0.01, "{left}");
        t.mark(Mark::Exposed, t0 + Duration::from_millis(1500));
        assert_eq!(t.hold_still(t0 + Duration::from_millis(1600)), None, "gone the moment the sensor is done");
        // A second try's exposing, retried by the camera program, resets it.
        let mut r = Timeline::new(PathBuf::from("b.jpg"), t0);
        r.exposing = Some(read_exposing("2000 1 still"));
        r.mark(Mark::Exposing, t0);
        r.mark(Mark::Exposing, t0 + Duration::from_millis(1000));
        assert!((r.hold_still(t0 + Duration::from_millis(1000)).unwrap().1 - 2.0).abs() < 0.01);
    }

    #[test]
    fn a_photo_that_never_comes_is_given_up_on_at_each_step() {
        let t0 = Instant::now();
        let mut t = Timeline::new(PathBuf::from("a.jpg"), t0);
        t.mark(Mark::Request, t0);
        assert_eq!(t.overdue(t0 + Duration::from_secs(10)), None);
        assert!(t.overdue(t0 + Duration::from_secs(16)).is_some());
        t.exposing = Some(read_exposing("8000 1 still"));
        t.mark(Mark::Exposing, t0 + Duration::from_secs(1));
        assert_eq!(t.overdue(t0 + Duration::from_secs(16)), None, "a long exposure is given its time");
        assert_eq!(t.overdue(t0 + Duration::from_secs(19)), None, "and the camera program's last try");
        assert!(t.overdue(t0 + Duration::from_secs(22)).is_some());
    }

    fn metered(lux: f32, total: f32) -> Metered {
        let exposure_us = (total / 4.0).min(66_667.0) as u32;
        Metered { exposure_us, analogue: total / exposure_us as f32, digital: 1.0, lux, settled: true, ..Metered::default() }
    }

    #[test]
    fn detail_autos_burst_runs_only_in_real_low_light() {
        let decide = |lux: f32| {
            let m = metered(lux, pro::LUX_REFERENCE / lux);
            let s = pro::still(None, &m, false);
            detail_burst(DetailMode::Auto, Some(&m), Some(&s), false, false)
        };
        // An evening room (0.9.3 burst under about 140 lux), a dim one, a dark one.
        assert_eq!(decide(320.0), None);
        assert_eq!(decide(100.0), None);
        assert_eq!(decide(50.0), None);
        assert_eq!(decide(15.0), Some(false));
        // A lux reading of a dark room in a bright scene (the AE far from its limit): no burst.
        let m = metered(15.0, 20_000.0);
        let s = pro::still(None, &m, false);
        assert_eq!(detail_burst(DetailMode::Auto, Some(&m), Some(&s), false, false), None);
        // Never with RAW, a set or a long exposure; always when on; the camera decides without metering.
        let dark = metered(10.0, 3_000_000.0);
        let s = pro::still(None, &dark, false);
        assert_eq!(detail_burst(DetailMode::Auto, Some(&dark), Some(&s), true, false), None);
        assert_eq!(detail_burst(DetailMode::On, Some(&dark), Some(&s), false, true), None);
        assert_eq!(detail_burst(DetailMode::On, Some(&dark), Some(&still(2_000_000, 1.0)), false, false), None);
        assert_eq!(detail_burst(DetailMode::On, Some(&dark), Some(&s), false, false), Some(false));
        assert_eq!(detail_burst(DetailMode::Off, Some(&dark), Some(&s), false, false), None);
        assert_eq!(detail_burst(DetailMode::Auto, None, None, false, false), Some(true));
    }

    #[test]
    fn brackets_step_by_whole_stops_and_shutter_priority_moves_the_gain() {
        let base = still(10_000, 2.0);
        let b = Bracket { frames: 5, stops: 1 };
        assert_eq!(b.evs(), vec![0, -1, 1, -2, 2]);
        let total = |(_, us, g): (i32, u32, f32)| us as f32 * g;
        for step in b.exposures(&base, Program::Auto) {
            let want = 20_000.0 * 2f32.powi(step.0);
            assert!((total(step) / want - 1.0).abs() < 0.01, "{step:?}");
            assert!((step.2 - 2.0).abs() < 1e-4, "the gain held, the shutter moved: {step:?}");
        }
        for step in b.exposures(&base, Program::Shutter) {
            let want = 20_000.0 * 2f32.powi(step.0);
            assert!((total(step) / want - 1.0).abs() < 0.01, "{step:?}");
            // At a gain of 2, a stop either way is the gain's to give; two
            // stops under is past a gain of 1, so the shutter gives the rest.
            if step.0.abs() <= 1 || step.0 == 2 {
                assert_eq!(step.1, 10_000, "the shutter held: {step:?}");
            } else {
                assert_eq!((step.1, step.2), (5_000, 1.0), "{step:?}");
            }
        }
        // ±2 in shutter priority at a low gain: under runs out of gain, the shutter makes it up.
        let wide = Bracket { frames: 3, stops: 2 };
        let low = wide.exposures(&still(4_000, 1.0), Program::Shutter);
        assert_eq!(low[1], (-2, 1_000, 1.0));
        assert!((total(low[2]) / 16_000.0 - 1.0).abs() < 0.01);
        assert_eq!(set_path(Path::new("/p/IMG_1.jpg"), -1), PathBuf::from("/p/IMG_1_ev-1.jpg"));
        assert_eq!(set_path(Path::new("/p/IMG_1.jpg"), 2), PathBuf::from("/p/IMG_1_ev+2.jpg"));
        assert_eq!(set_path(Path::new("/p/IMG_1.jpg"), 0), PathBuf::from("/p/IMG_1.jpg"));
        assert_eq!(next_bracket(None), Some(Bracket { frames: 3, stops: 1 }));
        assert_eq!(next_bracket(Some(Bracket { frames: 5, stops: 2 })), None);
        assert_eq!(Bracket::from_id("5x2"), Some(Bracket { frames: 5, stops: 2 }));
        assert_eq!(Bracket::from_id("4x1"), None);
    }

    #[test]
    fn aspect_crops_match_the_camera_programs_and_frame_the_viewfinder() {
        assert_eq!(Aspect::Full.crop(3280, 2464), (0, 0, 3280, 2464));
        assert_eq!(Aspect::ThreeTwo.crop(3280, 2464), (0, 138, 3280, 2188));
        assert_eq!(Aspect::SixteenNine.crop(3280, 2464), (0, 310, 3280, 1844));
        assert_eq!(Aspect::Square.crop(3280, 2464), (408, 0, 2464, 2464));
        assert_eq!(Aspect::ThreeTwo.crop(1640, 1232), (0, 70, 1640, 1092));
        assert_eq!(Aspect::Square.crop(320, 240), (40, 0, 240, 240));
        assert_eq!(Aspect::ThreeTwo.word().as_deref(), Some("aspect=3:2"));
        assert_eq!(Aspect::Full.word(), None);
        let vf = (0.0, 0.0, 227.0, 170.0);
        assert_eq!(Aspect::Square.frame(vf), (29.0, 0.0, 170.0, 170.0));
        let wide = Aspect::SixteenNine.frame(vf);
        assert_eq!((wide.2, wide.3), (227.0, 128.0));
        // A crop of a small frame: its planes in order and the right size.
        let (w, h) = (8usize, 4usize);
        let yuv: Vec<u8> = (0..w * h * 3 / 2).map(|i| i as u8).collect();
        let out = crop_yuv420(&yuv, w, h, (2, 0, 4, 4));
        assert_eq!(out.len(), 4 * 4 * 3 / 2);
        assert_eq!(&out[..4], &[2, 3, 4, 5]);
        assert_eq!(out[16], yuv[w * h + 1], "U from its own plane, offset by half");
        let mut o = Overlay::default();
        assert!(o.update(Grid::Thirds, Aspect::Square, vf));
        assert!(!o.update(Grid::Thirds, Aspect::Square, vf), "kept until something changes");
        assert_eq!(o.bars.len(), 2);
        assert_eq!(o.lines.len(), 4);
        assert!(o.update(Grid::Square, Aspect::Full, vf));
        assert_eq!(o.lines.len(), 3 + 2, "four squares across, three down");
    }

    #[test]
    fn programs_set_the_dials_and_the_still_fills_the_other() {
        let m = Metered { exposure_us: 20_000, analogue: 4.0, digital: 1.0, lux: 100.0, settled: true, ..Metered::default() };
        let mut p = Pro::default();
        assert_eq!(Program::of(&p), Program::Auto);
        Program::Shutter.apply(&mut p, None, Some(&m));
        assert_eq!(Program::of(&p), Program::Shutter);
        // Shutter priority: the shutter as set, the gain keeps the metered exposure.
        p.shutter = Some(pro::nearest_shutter(10_000));
        let s = pro::still(Some(&p), &m, false);
        assert_eq!(s.exposure_us, 10_000);
        assert!((s.gain - 8.0).abs() < 0.01, "{}", s.gain);
        Program::Iso.apply(&mut p, Some(&s), Some(&m));
        assert_eq!(Program::of(&p), Program::Iso);
        assert_eq!(p.shutter, None);
        // ISO priority: the gain as set, the shutter keeps the exposure.
        p.iso = Some(pro::ISOS.iter().position(|i| *i == 200).unwrap());
        let s = pro::still(Some(&p), &m, false);
        assert!((s.gain - 2.0).abs() < 1e-4);
        assert_eq!(s.exposure_us, 40_000);
        Program::Manual.apply(&mut p, Some(&s), Some(&m));
        assert_eq!(Program::of(&p), Program::Manual);
        Program::Auto.apply(&mut p, Some(&s), Some(&m));
        assert_eq!((p.shutter, p.iso), (None, None));
        assert_eq!((Program::Shutter.exif(), Program::Manual.exif(), Program::Auto.exif()), (4, 1, 2));
        // Shutter priority with too little light for the gain: the most gain, not more.
        let dark = Metered { exposure_us: 66_667, analogue: 10.0, digital: 1.0, lux: 5.0, ..m };
        let p = Pro { shutter: Some(pro::nearest_shutter(1_000)), ..Pro::default() };
        assert!((pro::still(Some(&p), &dark, false).gain - MAX_GAIN).abs() < 1e-4);
    }

    #[test]
    fn presets_keep_and_give_back_everything() {
        let s = Setup {
            pro: Pro { shutter: Some(9), iso: None, ev: -2, wb: Wb::Grey(1920, 1550) },
            look: Look::Tint,
            hue: 210.0,
            raw: true,
            zebras: false,
            false_colour: true,
            hist: HistMode::Rgb,
            grid: Grid::Square,
            aspect: Aspect::SixteenNine,
            bracket: Some(Bracket { frames: 5, stops: 1 }),
            timer: 2,
            level: true,
            night: pro::NightMode::Fixed(2),
        };
        let line = s.to_line();
        assert!(!line.contains(' '), "{line}");
        assert_eq!(Setup::from_line(&line), s);
        assert_eq!(Setup::from_line(""), Setup::default());
        assert!(s.summary().starts_with("s · 1/250"), "{}", s.summary());
        assert_eq!(preset_key('2'), Some((1, false)));
        assert_eq!(preset_key('#'), Some((2, true)));
        assert_eq!(preset_key('4'), None);
    }

    #[test]
    fn the_grey_card_evens_red_and_blue_with_green() {
        let even = grey_gains([128.0, 128.0, 128.0], [1.8, 1.5]).unwrap();
        assert!((even[0] - 1.8).abs() < 1e-3 && (even[1] - 1.5).abs() < 1e-3);
        // Too red, too little blue: the red gain falls, the blue rises, by the light's own ratio.
        let warm = grey_gains([150.0, 128.0, 110.0], [2.0, 1.4]).unwrap();
        assert!((warm[0] - 2.0 * (128.0f32 / 150.0).powf(2.2)).abs() < 1e-3);
        assert!(warm[1] > 1.4);
        assert!(grey_gains([10.0, 10.0, 10.0], [1.8, 1.5]).is_err());
        assert!(grey_gains([250.0, 250.0, 250.0], [1.8, 1.5]).is_err());
        assert!(grey_gains([220.0, 60.0, 40.0], [1.8, 1.5]).is_err());
        let mut rgb = vec![0u8; 12 * 12 * 3];
        for p in rgb.chunks_exact_mut(3) {
            p.copy_from_slice(&[90, 100, 110]);
        }
        assert_eq!(middle_mean(&rgb, 12, 12), [90.0, 100.0, 110.0]);
    }

    #[test]
    fn false_colour_marks_the_bands_and_the_rgb_histogram_counts_each_channel() {
        let t = false_colour();
        assert_eq!(t[0], [132, 40, 170]);
        assert_eq!(t[100], [70, 200, 90], "a mid grey in green");
        assert_eq!(t[255], [235, 45, 40], "clipping in red");
        assert_eq!(t[60][0], t[60][1], "the rest grey");
        // A frame all a warm grey: red highest, blue lowest.
        let (w, h) = (8, 8);
        let mut yuv = vec![128u8; w * h * 3 / 2];
        yuv[w * h..w * h * 5 / 4].fill(110);
        yuv[w * h * 5 / 4..].fill(150);
        let hist = RgbHistogram::of_yuv420(&yuv, w, h, 1);
        let peak = |c: usize| hist.bins[c].iter().position(|n| *n > 0).unwrap();
        assert!(peak(0) > peak(1) && peak(1) > peak(2));
        assert_eq!(hist.bins[0].iter().sum::<u32>(), 64);
        assert!((hist.heights()[0][peak(0)] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_level_reads_the_horizon_held_up_and_a_bubble_lying_flat() {
        // Held up, a few degrees clockwise.
        let a = 3f32.to_radians();
        let h = Horizon::of(Vec3::new(-a.sin(), a.cos(), 0.0)).unwrap();
        assert!(!h.flat && (h.roll - 3.0).abs() < 0.01, "{h:?}");
        assert!(!h.level());
        let level = Horizon::of(Vec3::new(0.0, 1.0, 0.0)).unwrap();
        assert!(level.level() && level.pitch.abs() < 0.01);
        // Screen tipped back (towards the sky): the lens points down.
        let tipped = Horizon::of(Vec3::new(0.0, 0.94, 0.34)).unwrap();
        assert!(tipped.pitch < -15.0, "{tipped:?}");
        let flat = Horizon::of(Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert!(flat.flat && flat.level());
        // Eased over a tenth of a second, the short way round.
        let mut e = Horizon { roll: 179.0, pitch: 0.0, flat: false };
        e.ease(Horizon { roll: -179.0, pitch: 0.0, flat: false }, 1.0);
        assert!(e.roll.rem_euclid(360.0) > 179.9 && e.roll.rem_euclid(360.0) < 181.1, "{e:?}");
    }

    #[test]
    fn the_intervalometer_keeps_its_beat_and_ends() {
        let t0 = Instant::now();
        let mut run = Run::start(Interval { every_s: 5, count: 3 }, t0);
        assert!(run.due(t0));
        run.took(t0);
        assert!(!run.due(t0 + Duration::from_secs(4)));
        assert!(run.due(t0 + Duration::from_secs(5)));
        // Late by more than a beat: the missed one skipped, the beat kept.
        run.took(t0 + Duration::from_secs(11));
        assert_eq!(run.next, t0 + Duration::from_secs(15));
        assert_eq!(run.words(t0 + Duration::from_secs(11)), "2/3 · next 4 s");
        run.took(t0 + Duration::from_secs(15));
        assert!(run.finished() && !run.due(t0 + Duration::from_secs(60)));
        let mut i = Interval::default();
        i.step_every(-10);
        assert_eq!(i.every_s, 2, "two seconds at the least");
        i.step_count(10);
        assert_eq!((i.count, i.count_words().as_str()), (0, "×∞"));
        assert_eq!(Interval::from_id(&i.id()), i);
        assert_eq!(duration_words(90), "1 min 30 s");
        assert_eq!(next_timer(0), 2);
        assert_eq!(next_timer(2), 10);
        assert_eq!(next_timer(10), 0);
    }
}
