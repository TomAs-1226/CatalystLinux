//! What is playing now, for the lock screen, StandBy and the island to show
//! as a live activity: set by Music while it plays (and cleared when it
//! stops), read by whatever shows it. Controls go the other way: the lock
//! screen or the island asks, Music does.

use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The cover, if there is one: an image file on /data (JPEG or PNG),
    /// decoded by whatever shows it.
    pub cover: Option<PathBuf>,
    /// Seconds into the song when this was set, and its length (0 when not
    /// known yet).
    pub position: f32,
    pub duration: f32,
    pub playing: bool,
    /// When `position` was read: while playing, readers move it on from here
    /// themselves, so Music need not set it every frame.
    pub at: Instant,
}

impl NowPlaying {
    /// Seconds into the song just now.
    pub fn position_now(&self) -> f32 {
        if !self.playing {
            return self.position;
        }
        let now = self.position + self.at.elapsed().as_secs_f32();
        if self.duration > 0.0 {
            now.min(self.duration)
        } else {
            now
        }
    }
}

/// What a live activity can ask of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
}

#[cfg(not(test))]
mod store {
    use super::{Control, NowPlaying};
    use std::sync::Mutex;

    static NOW: Mutex<Option<NowPlaying>> = Mutex::new(None);
    static CONTROL: Mutex<Option<fn(Control)>> = Mutex::new(None);

    pub fn now() -> std::sync::MutexGuard<'static, Option<NowPlaying>> {
        NOW.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn control() -> std::sync::MutexGuard<'static, Option<fn(Control)>> {
        CONTROL.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Each test thread has its own, as tests run side by side.
#[cfg(test)]
mod store {
    use super::{Control, NowPlaying};
    use std::cell::RefCell;

    thread_local! {
        static NOW: RefCell<Option<NowPlaying>> = const { RefCell::new(None) };
        static CONTROL: RefCell<Option<fn(Control)>> = const { RefCell::new(None) };
    }

    pub struct Now;
    impl Now {
        pub fn get(&self) -> Option<NowPlaying> {
            NOW.with(|n| n.borrow().clone())
        }
        pub fn set(&self, v: Option<NowPlaying>) {
            NOW.with(|n| *n.borrow_mut() = v);
        }
    }
    pub fn now() -> Now {
        Now
    }
    pub fn control_get() -> Option<fn(Control)> {
        CONTROL.with(|c| *c.borrow())
    }
    pub fn control_set(f: Option<fn(Control)>) {
        CONTROL.with(|c| *c.borrow_mut() = f);
    }
}

/// Music: what is playing now, or `None` once nothing is.
#[allow(dead_code)]
pub fn set(now: Option<NowPlaying>) {
    #[cfg(not(test))]
    {
        *store::now() = now;
    }
    #[cfg(test)]
    store::now().set(now);
}

/// What is playing now, if anything.
#[allow(dead_code)]
pub fn get() -> Option<NowPlaying> {
    #[cfg(not(test))]
    {
        store::now().clone()
    }
    #[cfg(test)]
    {
        store::now().get()
    }
}

/// Whether something is playing just now (not paused).
#[allow(dead_code)]
pub fn playing() -> bool {
    get().is_some_and(|n| n.playing)
}

/// Music: what to do when a live activity asks.
#[allow(dead_code)]
pub fn on_control(f: fn(Control)) {
    #[cfg(not(test))]
    {
        *store::control() = Some(f);
    }
    #[cfg(test)]
    store::control_set(Some(f));
}

/// A live activity asks the player; nothing happens with no player.
#[allow(dead_code)]
pub fn control(c: Control) {
    #[cfg(not(test))]
    let f = *store::control();
    #[cfg(test)]
    let f = store::control_get();
    if let Some(f) = f {
        f(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_moves_on_by_itself_while_playing_and_stops_at_its_end() {
        assert_eq!(get(), None);
        assert!(!playing());
        let mut song = NowPlaying {
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: "Homework".into(),
            cover: None,
            position: 10.0,
            duration: 10.5,
            playing: true,
            at: Instant::now() - std::time::Duration::from_secs(2),
        };
        assert_eq!(song.position_now(), 10.5, "not past its end");
        song.playing = false;
        assert_eq!(song.position_now(), 10.0, "paused: where it was");
        set(Some(song.clone()));
        assert_eq!(get(), Some(song));
        set(None);
        assert_eq!(get(), None);
    }

    #[test]
    fn controls_reach_the_player_once_it_has_said_how() {
        use std::sync::atomic::{AtomicU8, Ordering};
        static ASKED: AtomicU8 = AtomicU8::new(0);
        control(Control::Next);
        assert_eq!(ASKED.load(Ordering::Relaxed), 0, "no player: nothing happens");
        on_control(|c| ASKED.store(if c == Control::Next { 2 } else { 1 }, Ordering::Relaxed));
        control(Control::Next);
        assert_eq!(ASKED.load(Ordering::Relaxed), 2);
    }
}
