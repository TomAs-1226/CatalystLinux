//! Crossfades between songs, and the fade a sleep timer asks for.
//!
//! A crossfade overlaps the end of one song with the start of the next
//! at equal power (so the sum is as loud as either), for as long as the
//! setting says (up to 8 seconds), never more than a third of either song.
//! Within an album it is gapless instead: its songs are meant to follow
//! one another exactly.

use std::f32::consts::FRAC_PI_2;

/// The longest crossfade, in seconds.
pub const MOST: u8 = 8;

/// The same album: both named, and named alike.
pub fn same_album(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    !a.is_empty() && a.eq_ignore_ascii_case(b)
}

/// How long the two songs overlap, in seconds: 0 for gapless.
pub fn overlap(setting: u8, same_album: bool, this_length: Option<f32>, next_length: Option<f32>) -> f32 {
    if setting == 0 || same_album {
        return 0.0;
    }
    let mut s = setting.min(MOST) as f32;
    for len in [this_length, next_length] {
        match len {
            Some(l) if l > 0.0 => s = s.min(l / 3.0),
            // Not knowing a song's length, it cannot be timed: gapless.
            _ => return 0.0,
        }
    }
    if s < 0.5 {
        0.0
    } else {
        s
    }
}

/// The gains at `t` (0 to 1) through a crossfade: the song ending, and the
/// one beginning. Their squares sum to one.
pub fn gains(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0) * FRAC_PI_2;
    (t.cos(), t.sin())
}

/// A fade of the whole sound, over some seconds, to a level (0 silent,
/// 1 as it was): a sleep timer's, or its undoing. Even in decibels
/// (down to -60 dB, then silence), as the ear hears a fade.
#[derive(Clone, Debug, PartialEq)]
pub struct Fade {
    from: f32,
    to: f32,
    frames: u64,
    done: u64,
}

const FLOOR_DB: f32 = -60.0;

fn to_db(level: f32) -> f32 {
    if level <= 0.0 {
        FLOOR_DB
    } else {
        crate::gain_to_db(level).max(FLOOR_DB)
    }
}

impl Fade {
    pub fn new(from: f32, to: f32, seconds: f32, rate: u32) -> Fade {
        Fade { from: from.clamp(0.0, 1.0), to: to.clamp(0.0, 1.0), frames: (seconds.max(0.0) * rate as f32) as u64, done: 0 }
    }

    /// The level now, 0 to 1.
    pub fn level(&self) -> f32 {
        if self.done >= self.frames {
            return self.to;
        }
        let t = self.done as f32 / self.frames as f32;
        let db = to_db(self.from) + (to_db(self.to) - to_db(self.from)) * t;
        if db <= FLOOR_DB {
            0.0
        } else {
            crate::db_to_gain(db)
        }
    }

    pub fn finished(&self) -> bool {
        self.done >= self.frames
    }

    /// Stereo frames, faded; the level steps every 64 frames.
    pub fn process(&mut self, frames: &mut [f32]) {
        for block in frames.chunks_mut(128) {
            let g = self.level();
            block.iter_mut().for_each(|x| *x *= g);
            self.done = (self.done + block.len() as u64 / 2).min(self.frames);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn songs_of_one_album_run_on_and_others_overlap_by_the_setting() {
        assert_eq!(overlap(6, false, Some(200.0), Some(180.0)), 6.0);
        assert_eq!(overlap(6, true, Some(200.0), Some(180.0)), 0.0);
        assert_eq!(overlap(0, false, Some(200.0), Some(180.0)), 0.0);
        // A short song: a third of it at most.
        assert_eq!(overlap(8, false, Some(200.0), Some(12.0)), 4.0);
        assert_eq!(overlap(8, false, None, Some(200.0)), 0.0);
        assert!(same_album("Rumours", "rumours "));
        assert!(!same_album("", ""));
        assert!(!same_album("Rumours", "Tusk"));
    }

    #[test]
    fn a_crossfade_keeps_the_power_even() {
        for i in 0..=10 {
            let (a, b) = gains(i as f32 / 10.0);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
        assert_eq!(gains(0.0), (1.0, 0.0));
        let (end, start) = gains(1.0);
        assert!(end.abs() < 1e-6 && (start - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_fade_goes_down_evenly_in_decibels_and_stays_there() {
        let rate = 1000;
        let mut f = Fade::new(1.0, 0.0, 2.0, rate);
        let mut levels = Vec::new();
        for _ in 0..30 {
            let mut chunk = vec![1.0f32; 200];
            f.process(&mut chunk);
            levels.push(chunk[198]);
        }
        assert!(levels.windows(2).all(|w| w[1] <= w[0]));
        // Half way through (one second): -30 dB.
        assert!((crate::gain_to_db(levels[9]) + 30.0).abs() < 2.5, "{levels:?}");
        assert!(f.finished() && levels[29] == 0.0);
        let mut back = Fade::new(0.0, 1.0, 0.0, rate);
        let mut c = vec![0.5f32; 4];
        back.process(&mut c);
        assert_eq!(c, [0.5; 4]);
    }
}
