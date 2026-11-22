//! The sleep timer: music stops by itself after 15, 30, 45 or 60 minutes,
//! or at the end of the song playing. Over the last half minute its volume
//! fades out (catalyst-play's fade, asked for by the player), then it
//! pauses. The island's music counts it down.
//!
//! It is kept while the shell runs, so leaving Music keeps it, and never
//! across a restart. A timer for the end of the song waits while the song
//! is paused; one for a time counts on, as a phone's does, and with the
//! song paused when it ends there is nothing left to stop.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The fade before the end, seconds.
pub const FADE: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Minutes(u32),
    /// When the song playing ends.
    SongEnd,
}

/// What Music offers.
pub const CHOICES: [Choice; 5] = [Choice::Minutes(15), Choice::Minutes(30), Choice::Minutes(45), Choice::Minutes(60), Choice::SongEnd];

impl Choice {
    pub fn words(self) -> String {
        match self {
            Choice::Minutes(m) => format!("{m} minutes"),
            Choice::SongEnd => "end of this song".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timer {
    pub choice: Choice,
    /// For a time: when it ends.
    ends: Option<Instant>,
}

/// What the player does about the timer, now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Nothing,
    /// Fade from `gain` (where the fade is by now) down to silence over
    /// `over` seconds: the end.
    Fade { gain: f32, over: f32 },
    /// Its time is up: paused, and the timer gone.
    Stop,
}

impl Timer {
    pub fn new(choice: Choice, now: Instant) -> Timer {
        let ends = match choice {
            Choice::Minutes(m) => Some(now + Duration::from_secs(u64::from(m) * 60)),
            Choice::SongEnd => None,
        };
        Timer { choice, ends }
    }

    /// One that ends after `d` (the tests and the galleries).
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn after(d: Duration, now: Instant) -> Timer {
        Timer { choice: Choice::Minutes((d.as_secs() / 60) as u32), ends: Some(now + d) }
    }

    /// Seconds left: to its time, or (for the song's end) the song's own
    /// time left, `None` while that is not known.
    pub fn left(&self, now: Instant, song_left: Option<f32>) -> Option<f32> {
        match self.ends {
            Some(at) => Some(at.saturating_duration_since(now).as_secs_f32()),
            None => song_left.map(|s| s.max(0.0)),
        }
    }

    /// How loud, by the timer: whole until the fade, then down to nothing.
    #[cfg(test)]
    pub fn gain(&self, now: Instant, song_left: Option<f32>) -> f32 {
        self.left(now, song_left).map_or(1.0, |l| (l / FADE).clamp(0.0, 1.0))
    }

    /// What to do now, for a song `playing` (or about to) with `song_left`
    /// seconds to go, its sound already `fading` or not.
    pub fn act(&self, now: Instant, song_left: Option<f32>, playing: bool, fading: bool) -> Act {
        let Some(left) = self.left(now, song_left) else { return Act::Nothing };
        // A time's end stops it; the song's end is the song ending itself.
        if self.ends.is_some() && left <= 0.0 {
            return Act::Stop;
        }
        if playing && !fading && left <= FADE {
            return Act::Fade { gain: (left / FADE).clamp(0.0, 1.0), over: left };
        }
        Act::Nothing
    }

    /// When to look again (the player sleeps until then): the fade's start,
    /// then the end. The song's end needs no look: it ends.
    pub fn next_look(&self, now: Instant, song_left: Option<f32>, playing: bool, fading: bool) -> Option<Instant> {
        match self.ends {
            Some(end) if fading || !playing => Some(end),
            Some(end) => Some(end.checked_sub(Duration::from_secs_f32(FADE)).unwrap_or(now).max(now)),
            None if playing && !fading => song_left.map(|l| now + Duration::from_secs_f32((l - FADE).max(0.0))),
            None => None,
        }
    }
}

static TIMER: Mutex<Option<Timer>> = Mutex::new(None);

/// Set (or with `None`, cancelled), and the player told.
pub fn set(choice: Option<Choice>) {
    *TIMER.lock().unwrap() = choice.map(|c| Timer::new(c, Instant::now()));
    super::player::sleep_changed();
}

/// For a gallery: a timer set some while ago, `left` from its end.
#[cfg(not(target_arch = "aarch64"))]
pub fn pretend(timer: Option<(Choice, Duration)>) {
    *TIMER.lock().unwrap() = timer.map(|(choice, left)| Timer { choice, ..Timer::after(left, Instant::now()) });
}

pub fn get() -> Option<Timer> {
    *TIMER.lock().unwrap()
}

/// Its time is up (or the song it waited for ended): gone.
pub fn clear() {
    *TIMER.lock().unwrap() = None;
}

/// The song it waited for has ended: true once, and the timer gone.
pub fn take_song_end() -> bool {
    let mut t = TIMER.lock().unwrap();
    let ends = t.is_some_and(|t| t.choice == Choice::SongEnd);
    if ends {
        *t = None;
    }
    ends
}

/// Seconds left now, for the island and Music.
pub fn left_now(song_left: Option<f32>) -> Option<f32> {
    get().and_then(|t| t.left(Instant::now(), song_left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_for_a_time_fades_over_its_last_half_minute_then_stops() {
        let now = Instant::now();
        let t = Timer::new(Choice::Minutes(15), now);
        assert_eq!(t.left(now, None), Some(900.0));
        assert_eq!(t.act(now, Some(100.0), true, false), Act::Nothing);
        // It wakes the player at the fade's start, not before.
        assert_eq!(t.next_look(now, Some(100.0), true, false), Some(now + Duration::from_secs(870)));
        let fade_at = now + Duration::from_secs(870);
        assert_eq!(t.act(fade_at, None, true, false), Act::Fade { gain: 1.0, over: 30.0 });
        assert_eq!(t.gain(fade_at + Duration::from_secs(15), None), 0.5);
        // Fading: the next look is the end, which stops it.
        assert_eq!(t.next_look(fade_at, None, true, true), Some(now + Duration::from_secs(900)));
        assert_eq!(t.act(fade_at + Duration::from_secs(10), None, true, true), Act::Nothing);
        assert_eq!(t.act(now + Duration::from_secs(900), None, true, true), Act::Stop);
        assert_eq!(t.act(now + Duration::from_secs(901), None, false, false), Act::Stop, "paused by then: the timer goes all the same");
    }

    #[test]
    fn a_song_started_part_way_through_the_fade_starts_as_quiet_as_the_fade_is() {
        let now = Instant::now();
        let t = Timer::after(Duration::from_secs(12), now);
        assert_eq!(t.act(now, None, true, false), Act::Fade { gain: 0.4, over: 12.0 });
        // Paused: no fade until it plays again; the end still comes.
        assert_eq!(t.act(now, None, false, false), Act::Nothing);
        assert_eq!(t.next_look(now, None, false, false), Some(now + Duration::from_secs(12)));
    }

    #[test]
    fn the_end_of_the_song_waits_for_the_song_and_fades_with_its_last_half_minute() {
        let now = Instant::now();
        let t = Timer::new(Choice::SongEnd, now);
        assert_eq!(t.left(now, None), None, "not known until the song's length is");
        assert_eq!(t.left(now, Some(95.0)), Some(95.0));
        assert_eq!(t.next_look(now, Some(95.0), true, false), Some(now + Duration::from_secs(65)));
        assert_eq!(t.next_look(now, Some(95.0), false, false), None, "paused: waiting with it");
        assert_eq!(t.act(now, Some(29.0), true, false), Act::Fade { gain: 29.0 / 30.0, over: 29.0 });
        assert_eq!(t.act(now, Some(0.0), true, true), Act::Nothing, "the song ending stops it, not the clock");
    }

    #[test]
    fn the_timer_is_kept_until_it_ends_and_the_song_end_is_taken_once() {
        *TIMER.lock().unwrap() = Some(Timer::new(Choice::SongEnd, Instant::now()));
        assert_eq!(left_now(Some(40.0)), Some(40.0));
        assert!(take_song_end());
        assert!(!take_song_end());
        assert_eq!(get(), None);
        assert_eq!(CHOICES.map(|c| c.words()).join(", "), "15 minutes, 30 minutes, 45 minutes, 60 minutes, end of this song");
    }
}
