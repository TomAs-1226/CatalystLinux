//! Music on the island: what keeps its live activity alive.
//!
//! - **Equalizer bars** at the compact island's end: four, rising and
//!   falling with the song while it plays and settling flat as it pauses.
//!   They follow catalyst-play's level (`music::LEVEL_FILE`: the left and
//!   right loudness as heard, each period) while it writes one; else a
//!   motion seeded by the song, a beat and its off-beats over a slow swell.
//!   Each bar rides a Detent spring (Bezel's hold role: a value following a
//!   hand) to a new height twenty times a second. They run only while the
//!   island shows them and the song plays; otherwise they settle and stop
//!   asking for frames. In the calm version they stand still, risen while
//!   it plays.
//! - **The cover's colour**: Bezel's `bezelSeedFromPixels`, simply. The
//!   cover's pixels as Oklab, their hues counted by chroma, and the hue both
//!   common and vivid taken (Material's Score: 70 for its share, chroma
//!   above 48 counting three times what below it does), then matched to the
//!   nearest of the 72 palettes Bezel grows round the wheel
//!   (`bezel::hues`), whose signal colours the bars and the card's
//!   progress. Those palettes pass Bezel's contrast checks, so any cover's
//!   colour reads. A grey cover keeps the accent's own signal.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bezel::{Color, Tone};
use detent::{role, Animated};

use crate::gfx::Gfx;
use crate::photos::Picture;

pub const BARS: usize = 4;
/// How often the bars are given new heights, seconds.
const SAMPLE: f32 = 1.0 / 20.0;
/// The level not moved on for this long: the player is not saying one.
const STALE: Duration = Duration::from_millis(400);
/// The bars standing still (calm): their heights, risen.
pub const STILL: [f32; BARS] = [0.55, 1.0, 0.7, 0.85];

// ------------------------------------------------------------------ bars ---

pub struct Bars {
    springs: [Animated; BARS],
    due: f32,
    t: f32,
    rng: u64,
    /// Each band's loudest lately, for heights that use the bars' range.
    peaks: [f32; 3],
    /// The level's last sequence number, and when it last moved on.
    heard: Option<(u32, Instant)>,
    live: bool,
}

impl Default for Bars {
    fn default() -> Self {
        Self::new()
    }
}

impl Bars {
    pub fn new() -> Bars {
        Bars { springs: [Animated::new(0.0, role::hold()).with_rest(0.003); BARS], due: 0.0, t: 0.0, rng: 0x9E37_79B9_7F4A_7C15, peaks: [0.5; 3], heard: None, live: false }
    }

    /// Time passing (`md`, the motion's own step). `live`: the song plays
    /// and the island shows its bars; `seed`, the song's. Returns whether a
    /// frame is wanted.
    pub fn tick(&mut self, dt: f32, md: f32, live: bool, seed: u64) -> bool {
        if live {
            self.t += dt;
            self.due -= dt;
            if self.due <= 0.0 || !self.live {
                self.due = SAMPLE;
                let to = match read_level().and_then(|l| self.fresh(l)) {
                    Some(level) => self.from_level(level),
                    None => self.seeded(seed),
                };
                for (s, v) in self.springs.iter_mut().zip(to) {
                    s.set(v);
                }
            }
        } else if self.live {
            for s in &mut self.springs {
                s.set(0.0);
            }
        }
        self.live = live;
        let mut moving = false;
        for s in &mut self.springs {
            moving |= s.tick(md);
        }
        moving || live
    }

    /// Each bar's height, 0 (flat) to 1.
    pub fn heights(&self) -> [f32; BARS] {
        self.springs.map(|s| s.value().clamp(0.0, 1.1))
    }

    /// A level read, if the player is still saying it.
    fn fresh(&mut self, (seq, bands): (u32, [f32; 3])) -> Option<[f32; 3]> {
        let now = Instant::now();
        match self.heard {
            Some((last, at)) if last == seq => (now.duration_since(at) < STALE).then_some(bands),
            _ => {
                self.heard = Some((seq, now));
                Some(bands)
            }
        }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Heights from the bands as heard, each against its loudest lately.
    fn from_level(&mut self, bands: [f32; 3]) -> [f32; BARS] {
        let mut n = [0.0; 3];
        for k in 0..3 {
            self.peaks[k] = bands[k].max(self.peaks[k] * 0.985).max(0.35);
            n[k] = (bands[k] / self.peaks[k]).clamp(0.0, 1.0);
        }
        let [low, mid, high] = n;
        let raw = [low, low * 0.45 + mid * 0.55, mid, high];
        let mut out = [0.0; BARS];
        for (o, v) in out.iter_mut().zip(raw) {
            // A little sharper than the bands themselves, with some life of its own.
            *o = (v.powf(1.3) * (0.85 + 0.3 * self.random())).clamp(0.1, 1.0);
        }
        out
    }

    /// Believable without a level: a beat and its off-beats over a swell,
    /// at a tempo from the song.
    fn seeded(&mut self, seed: u64) -> [f32; BARS] {
        let bpm = 92.0 + (seed % 37) as f32;
        let beat = self.t * bpm / 60.0 + (seed >> 8 & 255) as f32 / 255.0;
        let kick = (-beat.fract() * 5.0).exp();
        let off = (-(beat * 2.0 + 0.5).fract() * 7.0).exp();
        let swell = 0.62 + 0.22 * (self.t * 0.53 + (seed >> 16 & 255) as f32 * 0.1).sin() + 0.14 * (self.t * 1.7).sin();
        let r = [self.random(), self.random(), self.random(), self.random()];
        [
            0.38 + 0.6 * kick * (0.8 + 0.2 * r[0]),
            0.32 + 0.25 * kick + 0.4 * r[1] * swell,
            0.28 + 0.5 * r[2] * swell + 0.15 * off,
            0.22 + 0.4 * off * (0.6 + 0.4 * r[3]) + 0.25 * r[3] * swell,
        ]
        .map(|v| v.clamp(0.1, 1.0))
    }
}

/// The level, on the handheld (a few bytes from memory); none elsewhere.
fn read_level() -> Option<(u32, [f32; 3])> {
    if !cfg!(target_arch = "aarch64") {
        return None;
    }
    // catalyst-play's left and right loudness (audiofx::levels): the bars
    // take the left, the two together, and the right.
    let (seq, left, right) = audiofx::levels::read(std::path::Path::new(crate::music::LEVEL_FILE))?;
    Some((u32::from(seq), [left, (left + right) * 0.5, right]))
}

/// A song's seed for the bars' motion: from its name.
pub fn song_seed(title: &str) -> u64 {
    title.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Music's bars: four capsules on a baseline at `cy + h / 2`, `heights` 0
/// (flat dots) to 1 (the whole height).
pub fn draw_bars(g: &mut Gfx, cx: f32, cy: f32, h: f32, heights: [f32; BARS], color: Color, alpha: f32) {
    let (w, pitch) = (2.0, 3.5);
    let left = cx - pitch * (BARS as f32 - 1.0) * 0.5;
    for (i, v) in heights.iter().enumerate() {
        let bar_h = (h * v.clamp(0.0, 1.1)).max(w);
        let x = left + i as f32 * pitch;
        g.canvas.rrect(bezel::Rect::new(x - w * 0.5, cy + h * 0.5 - bar_h, w, bar_h), w * 0.5, color, alpha);
    }
}

// ----------------------------------------------------------------- colour ---

/// sRGB to linear, by table.
fn linear(c: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    })[c as usize]
}

/// A colour as Oklab (L, a, b).
fn oklab(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    (0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s)
}

/// Oklab chroma as Material's (HCT) chroma, near enough to score by.
const HCT_CHROMA: f32 = 420.0;
const BINS: usize = 36;

/// The palette a cover gives (an index into `bezel::hues::HUES`), or `None`
/// for a cover with no colour to speak of.
pub fn cover_hue(p: &Picture) -> Option<usize> {
    if p.w == 0 || p.h == 0 || p.rgb.len() < p.w * p.h * 3 {
        return None;
    }
    // About 1600 pixels looked at, evenly.
    let step = ((p.w * p.h) as f32 / 1600.0).sqrt().max(1.0) as usize;
    let mut count = [0u32; BINS];
    let mut chroma = [0.0f32; BINS];
    let mut sum = [(0.0f32, 0.0f32); BINS];
    let mut total = 0u32;
    for y in (0..p.h).step_by(step) {
        for x in (0..p.w).step_by(step) {
            let i = (y * p.w + x) * 3;
            let (l, a, b) = oklab(p.rgb[i], p.rgb[i + 1], p.rgb[i + 2]);
            total += 1;
            let c = (a * a + b * b).sqrt();
            if c * HCT_CHROMA < 8.0 || !(0.12..=0.97).contains(&l) {
                continue;
            }
            let bin = ((b.atan2(a).to_degrees() + 360.0) % 360.0 / (360.0 / BINS as f32)) as usize % BINS;
            count[bin] += 1;
            chroma[bin] += c;
            sum[bin].0 += a;
            sum[bin].1 += b;
        }
    }
    if total == 0 {
        return None;
    }
    // Score each hue with half its neighbours' share: common and vivid wins.
    let mut best: Option<(f32, usize)> = None;
    for bin in 0..BINS {
        if count[bin] == 0 {
            continue;
        }
        let near = count[bin] as f32 + 0.5 * (count[(bin + BINS - 1) % BINS] + count[(bin + 1) % BINS]) as f32;
        let share = near / total as f32;
        let c = chroma[bin] / count[bin] as f32 * HCT_CHROMA;
        if share < 0.01 || c < 15.0 {
            continue;
        }
        let score = share * 100.0 * 0.7 + (c - 48.0) * if c < 48.0 { 0.1 } else { 0.3 };
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, bin));
        }
    }
    let (_, bin) = best?;
    let hue = sum[bin].1.atan2(sum[bin].0);
    // The grown palette whose seed is nearest in hue.
    bezel::hues::HUES
        .iter()
        .enumerate()
        .map(|(i, (seed, _, _))| {
            let c = Color::hex(*seed);
            let (_, a, b) = oklab(c.0, c.1, c.2);
            let d = (b.atan2(a) - hue).rem_euclid(std::f32::consts::TAU);
            (i, d.min(std::f32::consts::TAU - d))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(i, _)| i)
}

/// A cover palette's signal, in the tone showing.
pub fn signal_of(g: &Gfx, hue: Option<usize>) -> Color {
    match hue.and_then(|i| bezel::hues::HUES.get(i)) {
        Some((_, dark, light)) => Color::hex(if g.tokens.tone == Tone::Dark { dark[9] } else { light[9] }),
        None => g.tokens.signal,
    }
}

/// Covers' palettes, worked out once each (the last few).
static SEEDS: Mutex<Vec<(PathBuf, Option<usize>)>> = Mutex::new(Vec::new());

/// A cover's palette: `None` while it is still being decoded.
pub fn cover_seed(path: &Path) -> Option<Option<usize>> {
    if let Some((_, hue)) = SEEDS.lock().unwrap().iter().find(|(p, _)| p == path) {
        return Some(*hue);
    }
    let pic = crate::liveactivity::cover(path)?;
    let hue = cover_hue(&pic);
    let mut seeds = SEEDS.lock().unwrap();
    if seeds.len() >= 8 {
        seeds.remove(0);
    }
    seeds.push((path.to_owned(), hue));
    Some(hue)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Picture {
        Picture { w, h, rgb: (0..w * h).flat_map(|i| f(i % w, i / w)).collect() }
    }

    fn seed_hex(i: usize) -> u32 {
        bezel::hues::HUES[i].0
    }

    #[test]
    fn a_covers_colour_is_its_common_vivid_hue_and_grey_has_none() {
        // Mostly a deep blue, a little vivid orange: the blue is the cover.
        let blue = solid(64, 64, |x, _| if x < 52 { [30, 70, 200] } else { [255, 120, 20] });
        let i = cover_hue(&blue).unwrap();
        let (_, a, b) = { let c = Color::hex(seed_hex(i)); oklab(c.0, c.1, c.2) };
        assert!(b < 0.0 && a.abs() < b.abs(), "a blue palette: #{:06x}", seed_hex(i));
        // A small vivid patch on grey: still that patch's colour.
        let patch = solid(64, 64, |x, y| if x < 16 && y < 16 { [220, 30, 40] } else { [128, 128, 128] });
        let red = cover_hue(&patch).unwrap();
        let (_, a, _) = { let c = Color::hex(seed_hex(red)); oklab(c.0, c.1, c.2) };
        assert!(a > 0.1, "a red palette: #{:06x}", seed_hex(red));
        assert_eq!(cover_hue(&solid(32, 32, |x, _| [x as u8 * 8; 3])), None, "grey: the accent's own");
        assert_eq!(cover_hue(&Picture { w: 0, h: 0, rgb: Vec::new() }), None);
    }

    #[test]
    fn the_bars_move_while_live_and_settle_flat_and_still_when_not() {
        let mut bars = Bars::new();
        let mut seen = Vec::new();
        for _ in 0..120 {
            assert!(bars.tick(1.0 / 60.0, 1.0 / 60.0, true, song_seed("Harbour Lights")), "live: frames wanted");
            seen.push(bars.heights());
        }
        let first: Vec<f32> = seen.iter().map(|h| h[0]).collect();
        let spread = first.iter().cloned().fold(0.0, f32::max) - first.iter().cloned().fold(1.0, f32::min);
        assert!(spread > 0.2, "the bass bar moves: {spread}");
        assert!(seen.iter().flatten().all(|v| (0.0..=1.1).contains(v)));
        // Paused (or the island gone): flat, then no more frames.
        let mut frames = 0;
        while bars.tick(1.0 / 60.0, 1.0 / 60.0, false, 0) {
            frames += 1;
            assert!(frames < 120, "settles");
        }
        assert!(bars.heights().iter().all(|v| *v < 0.01), "{:?}", bars.heights());
    }

    #[test]
    fn a_level_heard_drives_the_bars_and_one_gone_quiet_gives_way() {
        let mut bars = Bars::new();
        let loud_low = bars.from_level([0.9, 0.2, 0.1]);
        assert!(loud_low[0] > loud_low[3] * 2.0, "{loud_low:?}");
        assert!(bars.fresh((7, [0.5; 3])).is_some());
        assert!(bars.fresh((7, [0.5; 3])).is_some(), "the same line, a moment on: still heard");
        bars.heard = Some((7, Instant::now() - STALE));
        assert!(bars.fresh((7, [0.5; 3])).is_none(), "not moved on: the player says none");
        assert!(bars.fresh((8, [0.5; 3])).is_some());
    }
}
