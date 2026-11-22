//! Taps: two small things the shell asks of the sound on its way to ALSA.
//! Both are off unless asked for in the environment, so the player plays as
//! ever without them, and a shell asking an older player is simply ignored.
//!
//! - **The level** (`CATALYST_PLAY_LEVEL=FILE`), for the island's equalizer
//!   bars: the sound's loudness in three bands (low, middle, high), 30 times
//!   a second, as one fixed-width line written over itself at the file's
//!   start: `SEQ LOW MID HIGH`, say `004211 072 055 031`. Each band is 0 to
//!   100 on a decibel scale; the sequence number moves on with each line,
//!   so a reader can tell a player that has stopped. The line is the sound
//!   as heard: ALSA's buffer (a third of a second) after it was written.
//!   The file belongs in /run (memory: nothing touches the card).
//! - **A fade** (`CATALYST_PLAY_FADE=stdin`), for the sleep timer: lines of
//!   `gain G SECONDS` on standard input take the loudness from where it is
//!   to G (0 to 1) over that long, sample by sample. Loudness is cubed into
//!   amplitude, so the fade sounds even all the way down. At gain 1 nothing
//!   is touched.
//!
//! The cost: three one-pole filters and three sums a frame for the level,
//! one multiply a sample while fading; a small write 30 times a second.

use std::collections::VecDeque;
use std::io::{BufRead, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use crate::RATE;

/// Frames in each level: a thirtieth of a second.
const WINDOW: u64 = RATE as u64 / 30;
/// How long after it is written a frame is heard: ALSA's buffer (alsa.rs
/// asks for 340 ms; dmix keeps 341).
const HEARD_AFTER: u64 = RATE as u64 * 340 / 1000;
/// Levels waiting to be heard: more than a buffer's worth.
const WAITING: usize = 32;

pub struct Taps {
    level: Option<Level>,
    /// A fade asked for on standard input, not yet taken up.
    asked: Option<Arc<Mutex<Option<(f32, f32)>>>>,
    /// Loudness now, where it is going, and by how much each frame.
    gain: f32,
    target: f32,
    step: f32,
    scaled: Vec<i16>,
}

impl Taps {
    /// As the environment asks.
    pub fn from_env() -> Taps {
        let level = std::env::var_os("CATALYST_PLAY_LEVEL").and_then(|path| Level::open(std::path::Path::new(&path)));
        let asked = (std::env::var("CATALYST_PLAY_FADE").as_deref() == Ok("stdin")).then(|| {
            let asked = Arc::new(Mutex::new(None));
            let to = asked.clone();
            std::thread::Builder::new()
                .name("fade".into())
                .stack_size(64 * 1024)
                .spawn(move || {
                    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                        if let Some(fade) = parse_gain(&line) {
                            *to.lock().unwrap() = Some(fade);
                        }
                    }
                })
                .ok();
            asked
        });
        Taps { level, asked, gain: 1.0, target: 1.0, step: 0.0, scaled: Vec::new() }
    }

    /// Neither tap: what the tests start from.
    #[cfg(test)]
    fn none() -> Taps {
        Taps { level: None, asked: None, gain: 1.0, target: 1.0, step: 0.0, scaled: Vec::new() }
    }

    /// A fade from where the loudness is to `target` over `seconds`.
    fn fade(&mut self, target: f32, seconds: f32) {
        self.target = target.clamp(0.0, 1.0);
        if seconds <= 0.0 {
            self.gain = self.target;
            self.step = 0.0;
        } else {
            self.step = (self.target - self.gain) / (seconds * RATE as f32);
        }
    }

    /// Interleaved stereo at 48 kHz on its way to ALSA: faded, if a fade is
    /// under way or done, and measured for the level. Returns what to write.
    pub fn pass<'a>(&'a mut self, pcm: &'a [i16]) -> &'a [i16] {
        // Never waits for the reader: a fade asked for while it holds the
        // lock is taken up with the next block.
        if let Some(fade) = self.asked.as_ref().and_then(|a| a.try_lock().ok().and_then(|mut f| f.take())) {
            self.fade(fade.0, fade.1);
        }
        let out: &[i16] = if self.gain >= 1.0 && self.target >= 1.0 {
            pcm
        } else {
            self.scaled.clear();
            for frame in pcm.chunks_exact(2) {
                if self.step != 0.0 {
                    self.gain += self.step;
                    if (self.step < 0.0 && self.gain <= self.target) || (self.step > 0.0 && self.gain >= self.target) {
                        self.gain = self.target;
                        self.step = 0.0;
                    }
                }
                let amp = self.gain * self.gain * self.gain;
                self.scaled.extend(frame.iter().map(|&s| (s as f32 * amp) as i16));
            }
            &self.scaled
        };
        if let Some(level) = self.level.as_mut() {
            level.feed(out);
        }
        out
    }
}

/// `gain 0.25 30` → (0.25, 30.0).
fn parse_gain(line: &str) -> Option<(f32, f32)> {
    let mut words = line.split_whitespace();
    if words.next()? != "gain" {
        return None;
    }
    let g: f32 = words.next()?.parse().ok()?;
    let s: f32 = words.next().map_or(Some(0.0), |w| w.parse().ok())?;
    (g.is_finite() && s.is_finite()).then_some((g.clamp(0.0, 1.0), s.max(0.0)))
}

/// The level, measured and said when it is heard.
struct Level {
    file: std::fs::File,
    seq: u32,
    /// One-pole low passes at 150 Hz and 2.5 kHz: below the first is low,
    /// between them middle, above the second high.
    low: f32,
    upper: f32,
    sums: [f64; 3],
    in_window: u64,
    /// Frames passed so far.
    frames: u64,
    /// Each window's end (in frames passed) and its three levels.
    waiting: VecDeque<(u64, [u8; 3])>,
}

/// A one-pole filter's coefficient for a corner frequency.
fn pole(hz: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / RATE as f32).exp()
}

impl Level {
    fn open(path: &std::path::Path) -> Option<Level> {
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(path).ok()?;
        Some(Level { file, seq: 0, low: 0.0, upper: 0.0, sums: [0.0; 3], in_window: 0, frames: 0, waiting: VecDeque::new() })
    }

    fn feed(&mut self, pcm: &[i16]) {
        let (a_low, a_upper) = (pole(150.0), pole(2500.0));
        for frame in pcm.chunks_exact(2) {
            let m = (frame[0] as f32 + frame[1] as f32) * (0.5 / 32768.0);
            self.low += a_low * (m - self.low);
            self.upper += a_upper * (m - self.upper);
            let bands = [self.low, self.upper - self.low, m - self.upper];
            for (sum, b) in self.sums.iter_mut().zip(bands) {
                *sum += (b * b) as f64;
            }
            self.in_window += 1;
            self.frames += 1;
            if self.in_window == WINDOW {
                let levels = self.sums.map(|s| to_level((s / WINDOW as f64).sqrt() as f32));
                if self.waiting.len() == WAITING {
                    self.waiting.pop_front();
                }
                self.waiting.push_back((self.frames, levels));
                self.sums = [0.0; 3];
                self.in_window = 0;
            }
        }
        // What is being heard now: the last window ALSA has played out.
        let heard = self.frames.saturating_sub(HEARD_AFTER);
        let mut now = None;
        while self.waiting.front().is_some_and(|(end, _)| *end <= heard) {
            now = self.waiting.pop_front().map(|(_, l)| l);
        }
        if let Some(l) = now {
            self.say(l);
        }
    }

    fn say(&mut self, l: [u8; 3]) {
        self.seq = (self.seq + 1) % 1_000_000;
        let line = format!("{:06} {:03} {:03} {:03}\n", self.seq, l[0], l[1], l[2]);
        if self.file.seek(SeekFrom::Start(0)).is_ok() {
            self.file.write_all(line.as_bytes()).ok();
        }
    }
}

impl Drop for Level {
    /// Stopped: the bars lie flat.
    fn drop(&mut self) {
        self.say([0, 0, 0]);
    }
}

/// An RMS (full scale 1) as 0 to 100: −60 dB and below is 0, −6 dB is 100.
fn to_level(rms: f32) -> u8 {
    if rms <= 1e-6 {
        return 0;
    }
    ((20.0 * rms.log10() + 60.0) / 54.0 * 100.0).clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_full_loudness_the_sound_passes_untouched() {
        let mut taps = Taps::none();
        let pcm = [1000i16, -1000, 32767, -32768];
        assert_eq!(taps.pass(&pcm), &pcm);
    }

    #[test]
    fn a_fade_goes_down_evenly_to_silence_and_stays_there() {
        let mut taps = Taps::none();
        taps.fade(0.0, 0.5);
        let block = vec![10_000i16; 2 * RATE as usize / 10];
        let mut peaks = Vec::new();
        for _ in 0..6 {
            peaks.push(taps.pass(&block).iter().map(|s| s.abs()).max().unwrap());
        }
        assert!(peaks.windows(2).all(|w| w[1] < w[0] || w[1] == 0), "down all the way: {peaks:?}");
        assert_eq!(*peaks.last().unwrap(), 0, "silent once the fade is over");
        // Cubed: half way through, the loudness is a half, the amplitude an eighth.
        let mut taps = Taps::none();
        taps.fade(0.0, 1.0);
        let half = vec![10_000i16; RATE as usize];
        let out = taps.pass(&half);
        assert!((out[out.len() - 2] as f32 - 10_000.0 / 8.0).abs() < 30.0, "{}", out[out.len() - 2]);
    }

    #[test]
    fn a_fade_back_up_ends_at_full_and_touches_nothing_after() {
        let mut taps = Taps::none();
        taps.fade(0.2, 0.0);
        taps.fade(1.0, 0.1);
        let block = vec![1000i16; 2 * RATE as usize / 5];
        taps.pass(&block);
        assert_eq!(taps.gain, 1.0);
        assert_eq!(taps.pass(&block), &block[..]);
    }

    #[test]
    fn gain_lines_are_read_and_anything_else_is_not() {
        assert_eq!(parse_gain("gain 0.25 30"), Some((0.25, 30.0)));
        assert_eq!(parse_gain("gain 1"), Some((1.0, 0.0)));
        assert_eq!(parse_gain("gain 3 -2"), Some((1.0, 0.0)));
        assert_eq!(parse_gain("volume 0.5 1"), None);
        assert_eq!(parse_gain("gain loud 1"), None);
    }

    #[test]
    fn the_level_is_said_as_heard_in_its_bands() {
        let path = std::env::temp_dir().join(format!("catalyst-play-level-{}", std::process::id()));
        let mut level = Level::open(&path).unwrap();
        // A low hum, loud: nothing said until a buffer's worth has passed.
        let hum: Vec<i16> = (0..RATE as usize / 4).flat_map(|i| {
            let s = ((i as f32 * std::f32::consts::TAU * 60.0 / RATE as f32).sin() * 16_000.0) as i16;
            [s, s]
        }).collect();
        level.feed(&hum);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "not heard yet");
        level.feed(&hum);
        let line = std::fs::read_to_string(&path).unwrap();
        let parts: Vec<u32> = line.split_whitespace().map(|w| w.parse().unwrap()).collect();
        assert_eq!(line.len(), 19, "fixed width: {line:?}");
        assert!(parts[1] > 70 && parts[1] > parts[3] + 30, "a hum is low: {line:?}");
        drop(level);
        assert!(std::fs::read_to_string(&path).unwrap().ends_with(" 000 000 000\n"), "flat once stopped");
        std::fs::remove_file(&path).ok();
    }
}
