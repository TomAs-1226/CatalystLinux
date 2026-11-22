//! Loudness matching: every song at the same loudness, -14 LUFS (as
//! YouTube Music and most streaming play), so one mastered loud is not a
//! jump after one mastered quietly.
//!
//! The gain comes from the song's own tags where it has them: ReplayGain
//! (`REPLAYGAIN_TRACK_GAIN`, made for -18 LUFS) or Opus's R128 gain
//! (`R128_TRACK_GAIN`, for -23 LUFS). Otherwise it is measured as the song
//! plays, cheaply ([`Meter`]: BS.1770's K-weighting, two biquads a
//! channel, gated as the standard says), and kept in a small file
//! ([`Cache`]) so every later play starts at the right level. A gain that
//! would lift the song is held to what its peak allows.

use std::path::{Path, PathBuf};

use crate::biquad::{Biquad, Coeffs};

pub const TARGET_LUFS: f32 = -14.0;
/// ReplayGain 2's reference, and EBU R128's.
const REPLAYGAIN_LUFS: f32 = -18.0;
const R128_LUFS: f32 = -23.0;
/// The most it turns a song down or up, in decibels.
const MOST_CUT: f32 = -20.0;
const MOST_LIFT: f32 = 8.0;
/// A lift with no peak known stops here.
const BLIND_LIFT: f32 = 3.0;
/// Where a lifted peak may reach: -1 dBFS.
const CEILING: f32 = 0.891;

/// A song's loudness, from its tags or measured: the gain to the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub gain_db: f32,
    pub peak: Option<f32>,
}

impl Level {
    /// The gain to play it at: cuts as they come, lifts held under the peak.
    pub fn safe_gain_db(&self) -> f32 {
        let g = self.gain_db.clamp(MOST_CUT, MOST_LIFT);
        if g <= 0.0 {
            return g;
        }
        match self.peak.filter(|p| *p > 0.0) {
            Some(peak) => g.min(crate::gain_to_db(CEILING / peak)).max(0.0),
            None => g.min(BLIND_LIFT),
        }
    }

    pub fn from_lufs(lufs: f32, peak: Option<f32>) -> Level {
        Level { gain_db: TARGET_LUFS - lufs, peak }
    }
}

/// "-7.32 dB", "+1.2", "0.98".
fn number(text: &str) -> Option<f32> {
    let t = text.trim().trim_end_matches(|c: char| c.is_alphabetic()).trim();
    t.trim_start_matches('+').parse().ok().filter(|v: &f32| v.is_finite())
}

/// The level a song's tags give (keys in any case, as each format spells
/// them: Vorbis comments, ID3's TXXX, MP4's freeform atoms).
pub fn from_tags<'a>(tags: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Level> {
    let (mut rg, mut rg_album, mut peak, mut r128, mut r128_album) = (None, None, None, None, None);
    for (key, value) in tags {
        let key = key.rsplit(':').next().unwrap_or(key).to_ascii_uppercase();
        match key.as_str() {
            "REPLAYGAIN_TRACK_GAIN" => rg = number(value),
            "REPLAYGAIN_ALBUM_GAIN" => rg_album = number(value),
            "REPLAYGAIN_TRACK_PEAK" => peak = number(value),
            // Q7.8 fixed point, in addition to the stream's output gain.
            "R128_TRACK_GAIN" => r128 = value.trim().parse::<i32>().ok().map(|q| q as f32 / 256.0),
            "R128_ALBUM_GAIN" => r128_album = value.trim().parse::<i32>().ok().map(|q| q as f32 / 256.0),
            _ => {}
        }
    }
    if let Some(g) = rg.or(rg_album) {
        return Some(Level { gain_db: g + (TARGET_LUFS - REPLAYGAIN_LUFS), peak });
    }
    r128.or(r128_album).map(|g| Level { gain_db: g + (TARGET_LUFS - R128_LUFS), peak: None })
}

/// BS.1770's integrated loudness, measured as the song plays: K-weighting,
/// 400 ms blocks overlapping by three quarters (100 ms steps), gated at
/// -70 LUFS and then 10 LU under the mean.
pub struct Meter {
    filters: [[Biquad; 2]; 2],
    step: usize,
    in_step: usize,
    acc: f64,
    /// Each 100 ms step's mean square (both channels summed).
    steps: Vec<f32>,
    peak: f32,
}

impl Meter {
    pub fn new(rate: u32) -> Meter {
        let shelf = Coeffs::high_shelf(rate, 1681.97, 0.7072, 4.0);
        let high = Coeffs::high_pass(rate, 38.13, 0.5003);
        Meter { filters: [[Biquad::new(shelf), Biquad::new(high)]; 2], step: (rate / 10) as usize, in_step: 0, acc: 0.0, steps: Vec::with_capacity(3000), peak: 0.0 }
    }

    pub fn push(&mut self, frames: &[f32]) {
        for f in frames.chunks_exact(2) {
            let mut e = 0.0f32;
            for ch in 0..2 {
                self.peak = self.peak.max(f[ch].abs());
                let [a, b] = &mut self.filters[ch];
                let y = b.run(a.run(f[ch]));
                e += y * y;
            }
            self.acc += e as f64;
            self.in_step += 1;
            if self.in_step == self.step {
                self.steps.push((self.acc / self.step as f64) as f32);
                self.acc = 0.0;
                self.in_step = 0;
            }
        }
    }

    /// Seconds heard so far.
    pub fn seconds(&self) -> f32 {
        self.steps.len() as f32 / 10.0
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// The loudness so far, once there are three seconds of it.
    pub fn lufs(&self) -> Option<f32> {
        if self.steps.len() < 30 {
            return None;
        }
        let blocks: Vec<f32> = self.steps.windows(4).map(|w| w.iter().sum::<f32>() / 4.0).collect();
        let loud = |z: f32| -0.691 + 10.0 * z.max(1e-12).log10();
        let above = |gate: f32| -> Vec<f32> { blocks.iter().cloned().filter(|&z| loud(z) > gate).collect() };
        let first = above(-70.0);
        if first.is_empty() {
            return None;
        }
        let mean = first.iter().sum::<f32>() / first.len() as f32;
        let second = above(loud(mean) - 10.0);
        let kept = if second.is_empty() { first } else { second };
        Some(loud(kept.iter().sum::<f32>() / kept.len() as f32))
    }
}

/// Songs measured, kept between plays: a line each, `key<TAB>lufs<TAB>peak`.
pub struct Cache {
    path: PathBuf,
}

/// The most songs kept (about 100 kB); the oldest go first.
const KEPT: usize = 3000;

impl Cache {
    pub const PATH: &'static str = "/data/catalyst/loudness";

    pub fn at(path: impl Into<PathBuf>) -> Cache {
        Cache { path: path.into() }
    }

    /// A song's key: its file, a download's `.part` left off.
    pub fn key(file: &Path) -> String {
        let s = file.to_string_lossy();
        s.strip_suffix(".part").unwrap_or(&s).replace(['\t', '\n'], " ")
    }

    pub fn get(&self, key: &str) -> Option<Level> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        text.lines().rev().find_map(|line| {
            let mut f = line.split('\t');
            (f.next()? == key).then_some(())?;
            let lufs: f32 = f.next()?.parse().ok()?;
            let peak: Option<f32> = f.next().and_then(|p| p.parse().ok());
            Some(Level::from_lufs(lufs, peak))
        })
    }

    pub fn put(&self, key: &str, lufs: f32, peak: f32) -> std::io::Result<()> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut lines: Vec<&str> = text.lines().filter(|l| l.split('\t').next() != Some(key)).collect();
        let line = format!("{key}\t{lufs:.2}\t{peak:.4}");
        lines.push(&line);
        let from = lines.len().saturating_sub(KEPT);
        let mut out = lines[from..].join("\n");
        out.push('\n');
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = self.path.with_extension("new");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// The gain a song plays at, as its loudness becomes known. Known from the
/// start (tags, the cache), it is that gain throughout. Measured on a first
/// play, it holds at unity for the first 15 seconds, then glides to what
/// the meter says over three seconds, and after that follows it only
/// slowly, so the level never jumps.
pub struct Matcher {
    known: Option<f32>,
    meter: Option<Meter>,
    gain_db: f32,
    rate: f32,
    /// The gain the meter last asked for, and when (seconds heard).
    want: Option<f32>,
    looked: f32,
}

impl Matcher {
    const LISTEN: f32 = 15.0;
    const GLIDE_DB_PER_S: f32 = 2.0;
    const FOLLOW_DB_PER_S: f32 = 0.1;

    pub fn known(level: Level, rate: u32) -> Matcher {
        let g = level.safe_gain_db();
        Matcher { known: Some(g), meter: None, gain_db: g, rate: rate as f32, want: None, looked: 0.0 }
    }

    pub fn measuring(rate: u32) -> Matcher {
        Matcher { known: None, meter: Some(Meter::new(rate)), gain_db: 0.0, rate: rate as f32, want: None, looked: 0.0 }
    }

    /// Unity, measuring nothing (loudness matching off).
    pub fn off(rate: u32) -> Matcher {
        Matcher { known: Some(0.0), meter: None, gain_db: 0.0, rate: rate as f32, want: None, looked: 0.0 }
    }

    /// Whether it changes the samples now.
    pub fn is_unity(&self) -> bool {
        self.gain_db.abs() < 0.01 && self.meter.is_none()
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    pub fn meter(&self) -> Option<&Meter> {
        self.meter.as_ref()
    }

    /// Frames as the decoder made them (before the gain), measured, then
    /// the gain applied to them.
    pub fn process(&mut self, frames: &mut [f32]) {
        if let (None, Some(meter)) = (self.known, self.meter.as_mut()) {
            meter.push(frames);
            let heard = meter.seconds();
            // The loudness worked out again once a second of sound.
            if heard >= Self::LISTEN && heard >= self.looked + 1.0 {
                self.looked = heard;
                self.want = meter.lufs().map(|lufs| Level::from_lufs(lufs, Some(meter.peak())).safe_gain_db());
            }
            if let Some(want) = self.want {
                let seconds = frames.len() as f32 / 2.0 / self.rate;
                let speed = if heard < Self::LISTEN + 4.0 { Self::GLIDE_DB_PER_S } else { Self::FOLLOW_DB_PER_S };
                let most = speed * seconds;
                self.gain_db += (want - self.gain_db).clamp(-most, most);
            }
        }
        if self.gain_db.abs() < 0.01 {
            return;
        }
        let g = crate::db_to_gain(self.gain_db);
        frames.iter_mut().for_each(|x| *x *= g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_give_the_gain_to_minus_fourteen() {
        let level = from_tags([("REPLAYGAIN_TRACK_GAIN", "-7.50 dB"), ("replaygain_track_peak", "0.98")]).unwrap();
        assert!((level.gain_db + 3.5).abs() < 1e-4);
        assert_eq!(level.peak, Some(0.98));
        // MP4's freeform atom, by its full name.
        let mp4 = from_tags([("com.apple.iTunes:replaygain_track_gain", "+2.0 dB")]).unwrap();
        assert!((mp4.gain_db - 6.0).abs() < 1e-4);
        // R128 in Q7.8: -1536 is -6 dB from -23 LUFS, so +3 dB from -14.
        let opus = from_tags([("R128_TRACK_GAIN", "-1536")]).unwrap();
        assert!((opus.gain_db - 3.0).abs() < 1e-4);
        assert_eq!(from_tags([("TITLE", "x")]), None);
        // Album gain when there is no track gain.
        assert!((from_tags([("REPLAYGAIN_ALBUM_GAIN", "-4 dB")]).unwrap().gain_db).abs() < 1e-4);
    }

    #[test]
    fn a_lift_is_held_under_the_peak() {
        assert_eq!(Level { gain_db: -9.0, peak: Some(1.0) }.safe_gain_db(), -9.0);
        // Peak at -6 dBFS: lifted at most to -1 dBFS.
        let g = Level { gain_db: 7.0, peak: Some(0.5) }.safe_gain_db();
        assert!((g - 5.02).abs() < 0.05, "{g}");
        assert_eq!(Level { gain_db: 7.0, peak: None }.safe_gain_db(), 3.0);
        assert_eq!(Level { gain_db: 3.0, peak: Some(1.0) }.safe_gain_db(), 0.0);
        assert_eq!(Level { gain_db: -40.0, peak: None }.safe_gain_db(), -20.0);
    }

    fn sine(rate: u32, seconds: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize).flat_map(|i| {
            let x = amp * (2.0 * std::f32::consts::PI * 997.0 * i as f32 / rate as f32).sin();
            [x, x]
        }).collect()
    }

    #[test]
    fn the_meter_reads_a_known_tone_and_gates_silence() {
        // BS.1770: a 997 Hz sine at -20 dBFS in both channels reads about -20 LUFS
        // (the K filter adds a little at 1 kHz), -23 in one.
        let rate = 48_000;
        let mut m = Meter::new(rate);
        m.push(&sine(rate, 2.0, 0.0));
        let tone = sine(rate, 10.0, 0.1);
        m.push(&tone);
        let lufs = m.lufs().unwrap();
        assert!((lufs + 20.0).abs() < 0.5, "{lufs}");
        assert!((m.peak() - 0.1).abs() < 1e-3);
        let mut short = Meter::new(rate);
        short.push(&sine(rate, 1.0, 0.1));
        assert_eq!(short.lufs(), None);
    }

    #[test]
    fn a_first_play_holds_then_glides_and_a_known_gain_is_there_at_once() {
        let rate = 48_000;
        // A loud tone (-6 LUFS or so): it should come down about 8 dB.
        let mut m = Matcher::measuring(rate);
        let chunk = sine(rate, 0.5, 0.5);
        let mut gains = Vec::new();
        for _ in 0..60 {
            let mut c = chunk.clone();
            m.process(&mut c);
            gains.push(m.gain_db());
        }
        assert_eq!(gains[20], 0.0, "unity while listening");
        assert!(gains[59] < -6.0, "{:?}", &gains[28..]);
        // Never more than a decibel in half a second.
        assert!(gains.windows(2).all(|w| (w[1] - w[0]).abs() <= 1.01));
        let mut k = Matcher::known(Level { gain_db: -4.0, peak: None }, rate);
        let mut c = vec![0.5, 0.5];
        k.process(&mut c);
        assert!((c[0] - 0.5 * db_to(-4.0)).abs() < 1e-5);
    }

    fn db_to(db: f32) -> f32 {
        crate::db_to_gain(db)
    }

    #[test]
    fn the_cache_keeps_one_line_a_song() {
        let dir = std::env::temp_dir().join(format!("audiofx-loudness-{}", std::process::id()));
        let cache = Cache::at(dir.join("loudness"));
        let key = Cache::key(Path::new("/data/music/cache/abc.webm.part"));
        assert_eq!(key, "/data/music/cache/abc.webm");
        assert_eq!(cache.get(&key), None);
        cache.put(&key, -9.0, 0.97).unwrap();
        cache.put("other", -20.0, 0.5).unwrap();
        cache.put(&key, -10.0, 0.95).unwrap();
        let got = cache.get(&key).unwrap();
        assert!((got.gain_db + 4.0).abs() < 1e-4 && got.peak == Some(0.95));
        assert_eq!(std::fs::read_to_string(dir.join("loudness")).unwrap().lines().count(), 2);
        std::fs::remove_dir_all(dir).ok();
    }
}
