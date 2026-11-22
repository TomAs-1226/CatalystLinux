//! The player: the queue, and the song playing through `catalyst-play`.
//!
//! A song plays from the card whenever it is there: from the library (yours,
//! or saved), else from the played cache; only otherwise is it fetched
//! (fetch.rs), into the cache, and played from that file as it grows,
//! starting once about five seconds of it are in. So a song played once
//! plays again offline, a seek lands anywhere already downloaded, and a
//! song from the library starts nothing to do with YouTube at all.
//!
//! The worker sleeps until something happens (a key, a line from the
//! player, a song arriving): no polling. `catalyst-play` says where it is
//! every five seconds; in between the position runs on the clock.
//! Everything that changes is told to `nowplaying` for the lock screen,
//! StandBy and the island, and whether what plays next needs the network
//! (so music standby can switch Wi-Fi off once it is all here).
//!
//! As sound moves (sound.rs), the song follows: started again where it was
//! through headphones that take over, and paused when the USB headphones it
//! played through come out. The player may find them gone first (its sound
//! stops with an error): that too is a pause, not a failure.
//!
//! The sleep timer (sleep.rs) is looked at as the worker wakes, which it
//! does for the fade's start and the timer's end too: catalyst-play is
//! asked on its standard input to fade over the last half minute, then the
//! song pauses. catalyst-play also writes its level to `LEVEL_FILE` for
//! the island's equalizer bars.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::fetch::{self, Event};
use super::polite::{self, Manner};
use super::{covers, library, store, Track};
use crate::platform::Media;

/// Back to the start of a song rather than the one before, after this long.
pub const RESTART_AFTER: f32 = 3.0;
/// Songs fetched ahead of the one playing; fewer than `AHEAD_LOW` ready
/// and the network is wanted again.
const AHEAD: usize = 6;
const AHEAD_LOW: usize = 2;
/// A song starts once this much of it has downloaded.
const START_SECONDS: u64 = 5;
const START_BYTES_MIN: u64 = 96 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Nothing in the queue.
    Empty,
    /// Finding the song, or its first sound on the way.
    Loading,
    Playing,
    Paused,
    /// Stopped by a problem, said here; Enter tries again.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    Off,
    All,
    One,
}

/// What the player is doing, for every screen to read.
#[derive(Clone, Debug)]
pub struct Now {
    pub queue: Vec<Track>,
    pub at: usize,
    pub phase: Phase,
    /// Where in the song, in seconds.
    pub position: f32,
    /// Its length, when known.
    pub length: Option<f32>,
    /// While loading: what is happening ("finding it…", "starting…").
    pub loading: Option<String>,
    /// The download is behind the song: waiting for it.
    pub buffering: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Now {
    pub fn track(&self) -> Option<&Track> {
        self.queue.get(self.at)
    }
}

enum Cmd {
    /// A list, played from one of its songs; `radio`: the song's radio after it.
    Play(Vec<Track>, usize, bool),
    Toggle,
    Next,
    Previous,
    Seek(f32),
    Jump(usize),
    /// After the song playing now.
    PlayNext(Track),
    Remove(usize),
    /// Everything stopped and the queue emptied.
    Clear,
    Shuffle,
    Repeat,
    /// A song's radio arrived: added after it, if it is still the one.
    Radio(String, Vec<Track>),
    Fetch(Event),
    /// A line from the player, and its end.
    Said(u64, String),
    Closed(u64),
    /// Sound moved: what the song does.
    Moved(crate::sound::Follow),
    /// The sleep timer was set or cancelled (sleep.rs).
    Sleep,
}

/// Where catalyst-play says how loud the song is, for the island's bars
/// (its `CATALYST_PLAY_LEVEL`): memory, never the card.
pub const LEVEL_FILE: &str = "/run/catalyst-music/level";

struct Shared {
    queue: Vec<Track>,
    at: usize,
    phase: Phase,
    /// Where the song was at `anchor` (or is, when not playing).
    offset: f32,
    anchor: Option<Instant>,
    length: Option<f32>,
    loading: Option<String>,
    buffering: bool,
    shuffle: bool,
    repeat: Repeat,
    unshuffled: Option<Vec<Track>>,
}

impl Shared {
    fn position(&self) -> f32 {
        let run = match (&self.phase, self.anchor, self.buffering) {
            (Phase::Playing, Some(a), false) => a.elapsed().as_secs_f32(),
            _ => 0.0,
        };
        let p = self.offset + run;
        match self.length {
            Some(l) if l > 0.0 => p.min(l),
            _ => p,
        }
    }

    fn track(&self) -> Option<&Track> {
        self.queue.get(self.at)
    }
}

struct Player {
    tx: Sender<Cmd>,
    state: Arc<Mutex<Shared>>,
}

static PLAYER: OnceLock<Player> = OnceLock::new();
/// What plays next needs the network (music standby keeps Wi-Fi for it).
static NEEDS_NETWORK: AtomicBool = AtomicBool::new(false);
/// Times ALSA ran dry since the shell started (also in /run/catalyst-music/xruns).
static XRUNS: AtomicU32 = AtomicU32::new(0);
/// When the song was last paused by the USB headphones it played through
/// coming out (for the island's words, which may come a moment after).
static UNPLUGGED_AT: Mutex<Option<Instant>> = Mutex::new(None);

fn player() -> &'static Player {
    PLAYER.get_or_init(|| {
        let (tx, rx) = channel::<Cmd>();
        let state = Arc::new(Mutex::new(Shared { queue: Vec::new(), at: 0, phase: Phase::Empty, offset: 0.0, anchor: None, length: None, loading: None, buffering: false, shuffle: false, repeat: Repeat::Off, unshuffled: None }));
        let worker = Worker { tx: tx.clone(), state: state.clone(), sound: None, gen: 0, errors: Arc::new(Mutex::new(String::new())), retry_at: None, busy_tries: 0, ffmpeg_for: None, fetch_failed: None, sim_due: None, played_on: None, feed: None, fed: None, fading: false, paused_since: None, retired: false };
        std::thread::Builder::new().name("music".into()).stack_size(256 * 1024).spawn(move || worker.run(rx)).ok();
        fetch::on_event(|e| {
            if let Some(p) = PLAYER.get() {
                p.tx.send(Cmd::Fetch(e)).ok();
            }
        });
        crate::nowplaying::on_control(|c| match c {
            crate::nowplaying::Control::PlayPause => send(Cmd::Toggle),
            crate::nowplaying::Control::Next => send(Cmd::Next),
            crate::nowplaying::Control::Previous => send(Cmd::Previous),
        });
        crate::nowplaying::on_needs_network(|| NEEDS_NETWORK.load(Ordering::Relaxed));
        Player { tx, state }
    })
}

fn send(cmd: Cmd) {
    player().tx.send(cmd).ok();
}

/// The player now: its queue, the song, where it is.
pub fn now() -> Now {
    let Some(p) = PLAYER.get() else {
        return Now { queue: Vec::new(), at: 0, phase: Phase::Empty, position: 0.0, length: None, loading: None, buffering: false, shuffle: false, repeat: Repeat::Off };
    };
    let s = p.state.lock().unwrap();
    Now { queue: s.queue.clone(), at: s.at, phase: s.phase.clone(), position: s.position(), length: s.length.or(s.track().and_then(|t| t.seconds).map(|x| x as f32)), loading: s.loading.clone(), buffering: s.buffering, shuffle: s.shuffle, repeat: s.repeat }
}

/// Music is playing, or about to (a song being found, or its first sound
/// on its way): deep standby waits for it to stop.
pub fn playing() -> bool {
    // The unit tests' player (the simulator's) plays beside tests of deep
    // standby, which must not see it.
    if cfg!(test) {
        return false;
    }
    PLAYER.get().is_some_and(|p| matches!(p.state.lock().unwrap().phase, Phase::Playing | Phase::Loading))
}


/// Sound moved (sound.rs): the song follows, if there is a player at all.
pub fn moved(follow: crate::sound::Follow) {
    if let Some(p) = PLAYER.get() {
        p.tx.send(Cmd::Moved(follow)).ok();
    }
}

/// The sleep timer changed: the player looks at it again (if there is one).
pub fn sleep_changed() {
    if let Some(p) = PLAYER.get() {
        p.tx.send(Cmd::Sleep).ok();
    }
}

/// The song in the player's place (playing, paused or loading), by its id:
/// cheaper than `now()` for what is asked every frame or second.
pub fn current_id() -> Option<String> {
    let p = PLAYER.get()?;
    let s = p.state.lock().unwrap();
    s.track().filter(|_| s.phase != Phase::Empty).map(|t| t.id.clone())
}

/// Seconds left in the song, while it is known.
pub fn song_left() -> Option<f32> {
    let p = PLAYER.get()?;
    let s = p.state.lock().unwrap();
    let length = s.length.or(s.track().and_then(|t| t.seconds).map(|x| x as f32))?;
    Some((length - s.position()).max(0.0))
}

/// The song was paused a moment ago because the USB headphones it played
/// through came out (once: the flag is taken).
pub fn paused_unplugged() -> bool {
    UNPLUGGED_AT.lock().unwrap().take().is_some_and(|at| at.elapsed() < Duration::from_secs(5))
}

pub fn play(list: Vec<Track>, at: usize, radio: bool) {
    send(Cmd::Play(list, at, radio));
}
pub fn toggle() {
    send(Cmd::Toggle);
}
pub fn next() {
    send(Cmd::Next);
}
pub fn previous() {
    send(Cmd::Previous);
}
pub fn seek(by: f32) {
    send(Cmd::Seek(by));
}
pub fn jump(i: usize) {
    send(Cmd::Jump(i));
}
pub fn play_next(t: Track) {
    send(Cmd::PlayNext(t));
}
/// The queue emptied, nothing playing.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn clear() {
    send(Cmd::Clear);
}
pub fn remove(i: usize) {
    send(Cmd::Remove(i));
}
pub fn shuffle() {
    send(Cmd::Shuffle);
}
pub fn repeat() {
    send(Cmd::Repeat);
}

/// Home's tile: the song playing, else what the app is.
pub fn tile() -> (String, Option<&'static str>) {
    let n = now();
    match (n.track(), &n.phase) {
        (Some(t), Phase::Playing | Phase::Loading) => (t.title.clone(), None),
        (Some(_), Phase::Paused) => ("paused".into(), None),
        _ => ("youtube music".into(), None),
    }
}

/// Play or pause, next, previous, over any screen: what the island says.
pub fn media_key(key: Media) -> Option<String> {
    let n = now();
    let Some(track) = n.track().cloned() else {
        return Some("nothing is playing".into());
    };
    match key {
        Media::PlayPause => {
            send(Cmd::Toggle);
            Some(if matches!(n.phase, Phase::Playing | Phase::Loading) { "paused".into() } else { format!("playing {}", track.title) })
        }
        Media::Next => {
            send(Cmd::Next);
            Some(n.queue.get(n.at + 1).map_or_else(|| "the end of the queue".into(), |t| format!("next: {}", t.title)))
        }
        Media::Previous => {
            send(Cmd::Previous);
            Some(if n.position > RESTART_AFTER || n.at == 0 { format!("from the start: {}", track.title) } else { format!("back to {}", n.queue[n.at - 1].title) })
        }
        _ => None,
    }
}

/// Where a song plays from on the card, if it is there: the library (as
/// itself, or saved from YouTube), else the played cache.
pub fn on_card(t: &Track) -> Option<PathBuf> {
    if let Some(p) = t.local.as_ref().filter(|p| p.exists() || !super::on_device()) {
        return Some(p.clone());
    }
    if let Some(s) = library::by_youtube(&t.id) {
        return Some(s.path);
    }
    store::cached(&t.id).map(|p| p.file())
}

/// The sound being made: the player process, or (in the simulator) a clock.
enum Sound {
    Process(Child),
    Silent { until: Instant },
}

struct Worker {
    tx: Sender<Cmd>,
    state: Arc<Mutex<Shared>>,
    sound: Option<Sound>,
    /// Each sound started has its number; lines from an old one are ignored.
    gen: u64,
    errors: Arc<Mutex<String>>,
    /// The sound device was busy (Bluetooth takes one program at a time,
    /// and a chime may have it): tried again a moment later, a few times.
    retry_at: Option<Instant>,
    busy_tries: u8,
    /// A song catalyst-play could not read: played by ffmpeg instead.
    ffmpeg_for: Option<String>,
    /// The download of the song playing failed: said if the song runs dry.
    fetch_failed: Option<String>,
    /// The simulator's songs are "found" a moment after they are asked for.
    sim_due: Option<Instant>,
    /// The USB headphones' card the player was started on, if they were
    /// what played.
    played_on: Option<u32>,
    /// catalyst-play's standard input: the song after this one told to it
    /// (its id, and the line said), so it runs on into it without a gap (or
    /// crossfades, as Settings › Sound says), and the sleep timer's fade
    /// (`fade SECONDS LEVEL`). None for ffmpeg, which takes neither.
    feed: Option<std::process::ChildStdin>,
    fed: Option<(String, String)>,
    /// The sound playing has been asked to fade out.
    fading: bool,
    /// Paused since then.
    paused_since: Option<Instant>,
    /// Paused so long it has let go: off the island, nothing fetched ahead,
    /// no network wanted. The queue and the place in the song are kept, and
    /// play carries on from there.
    retired: bool,
}

/// Paused this long, the music lets go (see `Worker::retired`).
const RETIRE_AFTER: Duration = Duration::from_secs(10 * 60);

impl Worker {
    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            let silent = match &self.sound {
                Some(Sound::Silent { until }) => Some(*until),
                _ => None,
            };
            let retire = self.paused_since.filter(|_| !self.retired).map(|at| at + RETIRE_AFTER);
            let deadline = [self.retry_at, self.sim_due, silent, self.sleep_look(), retire].into_iter().flatten().min();
            let cmd = match deadline {
                Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match cmd {
                Ok(cmd) => {
                    if !matches!(cmd, Cmd::Fetch(_) | Cmd::Said(..) | Cmd::Closed(_) | Cmd::Radio(..) | Cmd::Moved(_) | Cmd::Sleep) {
                        self.retry_at = None;
                        self.busy_tries = 0;
                    }
                    // Where the song is, every five seconds: nothing else to look at.
                    let quiet = matches!(&cmd, Cmd::Said(_, l) if l.starts_with("at ") || l.starts_with("xrun"));
                    self.handle(cmd);
                    if !quiet {
                        self.after();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.deadline();
                    self.after();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.stop_sound();
    }

    /// The song after this one, told to catalyst-play once it is on the
    /// card (and told again if the queue changes under it).
    fn feed_next(&mut self) {
        if self.feed.is_none() {
            return;
        }
        let next = {
            let s = self.state.lock().unwrap();
            let i = match s.repeat {
                Repeat::One => Some(s.at),
                Repeat::All if !s.queue.is_empty() => Some((s.at + 1) % s.queue.len()),
                _ => Some(s.at + 1).filter(|&i| i < s.queue.len()),
            };
            i.and_then(|i| s.queue.get(i).cloned())
        };
        let line = next.as_ref().and_then(|t| on_card(t).map(|p| (t.id.clone(), format!("next\t{}\talbum={}\n", p.display(), album_key(t)))));
        if line.as_ref().map(|l| &l.1) == self.fed.as_ref().map(|f| &f.1) {
            return;
        }
        let words = line.as_ref().map_or("next\n", |l| l.1.as_str()).to_owned();
        if let Some(feed) = self.feed.as_mut() {
            use std::io::Write;
            if feed.write_all(words.as_bytes()).and_then(|_| feed.flush()).is_err() {
                self.feed = None;
            }
        }
        self.fed = line;
    }

    /// After anything: what to fetch, whether the network is needed.
    fn after(&mut self) {
        self.feed_next();
        let (current, ahead) = {
            let s = self.state.lock().unwrap();
            let current = s.track().cloned();
            let mut ahead: Vec<Track> = s.queue.iter().skip(s.at + 1).take(AHEAD).cloned().collect();
            if s.repeat == Repeat::All && ahead.len() < AHEAD {
                ahead.extend(s.queue.iter().take(AHEAD - ahead.len()).cloned());
            }
            if s.phase == Phase::Paused {
                self.paused_since.get_or_insert_with(Instant::now);
            } else {
                self.paused_since = None;
                self.retired = false;
            }
            let active = matches!(s.phase, Phase::Playing | Phase::Loading) || (s.phase == Phase::Paused && !self.retired);
            (current.filter(|_| active), ahead)
        };
        if super::on_device() {
            let wanted = current.clone().filter(fetch::needs);
            fetch::want(wanted, if current.is_some() { ahead.clone() } else { Vec::new() });
        }
        // Wanted: the song playing is still coming, or fewer than two after
        // it are here; not wanted once all those looked ahead are.
        let was = NEEDS_NETWORK.load(Ordering::Relaxed);
        let needs = match &current {
            None => false,
            Some(t) if super::on_device() && fetch::needs(t) => true,
            Some(_) if !super::on_device() => false,
            Some(_) => {
                let ready = ahead.iter().take_while(|t| !fetch::needs(t)).count();
                if ready >= ahead.len() {
                    false
                } else if ready < AHEAD_LOW {
                    true
                } else {
                    was
                }
            }
        };
        NEEDS_NETWORK.store(needs, Ordering::Relaxed);
        self.sleep_check();
    }

    /// The song's seconds left, and whether it plays (or is about to).
    fn song_now(&self) -> (Option<f32>, bool) {
        let s = self.state.lock().unwrap();
        let length = s.length.or(s.track().and_then(|t| t.seconds).map(|x| x as f32));
        let playing = matches!(s.phase, Phase::Playing | Phase::Loading) && self.sound.is_some();
        (length.map(|l| (l - s.position()).max(0.0)), playing)
    }

    /// When the sleep timer next needs looking at.
    fn sleep_look(&self) -> Option<Instant> {
        let timer = super::sleep::get()?;
        let (left, playing) = self.song_now();
        timer.next_look(Instant::now(), left, playing, self.fading)
    }

    /// The sleep timer, looked at: the fade asked for as it begins (and of
    /// each song started during it, from where it has got to), and music
    /// paused at its end. Cancelled mid-fade, the sound comes back up.
    fn sleep_check(&mut self) {
        let Some(timer) = super::sleep::get() else {
            if self.fading {
                self.fading = false;
                self.gain(1.0, 0.8);
            }
            return;
        };
        let (left, playing) = self.song_now();
        match timer.act(Instant::now(), left, playing, self.fading) {
            super::sleep::Act::Stop => {
                super::sleep::clear();
                if playing || self.state.lock().unwrap().phase == Phase::Loading {
                    self.pause();
                }
            }
            super::sleep::Act::Fade { gain, over } => {
                self.fading = true;
                self.gain(gain, 0.0);
                self.gain(0.0, over);
            }
            super::sleep::Act::Nothing => {}
        }
    }

    /// catalyst-play asked to take the loudness to `to` over `over` seconds.
    fn gain(&mut self, to: f32, over: f32) {
        use std::io::Write;
        if let Some(c) = self.feed.as_mut() {
            if writeln!(c, "fade\t{over:.2}\t{to:.3}").is_err() {
                self.feed = None;
            }
        }
    }

    /// The sleep timer waited for this song to end: on to the next, paused
    /// at its start (Enter plays on from there in the morning).
    fn stop_after_song(&mut self) {
        self.stop_sound();
        {
            let mut s = self.state.lock().unwrap();
            if s.at + 1 < s.queue.len() {
                s.at += 1;
            }
            let length = s.track().and_then(|t| t.seconds).map(|x| x as f32);
            s.length = length;
            s.offset = 0.0;
            s.anchor = None;
            s.phase = Phase::Paused;
            s.loading = None;
            s.buffering = false;
        }
        self.publish();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play(list, at, radio) => {
                let id = list.get(at).map(|t| t.id.clone());
                let alone = list.len() == 1;
                {
                    let mut s = self.state.lock().unwrap();
                    s.queue = list;
                    s.at = at;
                    s.unshuffled = None;
                    s.shuffle = false;
                }
                self.ffmpeg_for = None;
                self.start(0.0);
                if let (true, true, Some(id)) = (radio || alone, !id.as_deref().unwrap_or("lib:").starts_with("lib:"), id) {
                    if polite::online() || !super::on_device() {
                        let tx = self.tx.clone();
                        std::thread::Builder::new()
                            .name("music-radio".into())
                            .stack_size(256 * 1024)
                            .spawn(move || {
                                if let Ok(next) = super::up_next(&id) {
                                    tx.send(Cmd::Radio(id, next)).ok();
                                }
                            })
                            .ok();
                    }
                }
            }
            Cmd::Radio(id, next) => {
                let mut s = self.state.lock().unwrap();
                if s.queue.len() == 1 && s.queue[0].id == id {
                    super::know(&next);
                    s.queue.extend(next.into_iter().filter(|t| t.id != id).take(40));
                }
            }
            Cmd::Toggle => {
                let phase = self.state.lock().unwrap().phase.clone();
                match phase {
                    Phase::Playing | Phase::Loading => self.pause(),
                    Phase::Paused | Phase::Failed(_) => {
                        let at = self.state.lock().unwrap().offset;
                        self.start(at);
                    }
                    Phase::Empty => {}
                }
            }
            Cmd::Next => self.step(1, false),
            Cmd::Previous => {
                let (position, at) = {
                    let s = self.state.lock().unwrap();
                    (s.position(), s.at)
                };
                if position > RESTART_AFTER || at == 0 {
                    self.start(0.0);
                } else {
                    self.step(-1, false);
                }
            }
            Cmd::Seek(delta) => {
                let (position, length, paused) = {
                    let s = self.state.lock().unwrap();
                    (s.position(), s.length.or(s.track().and_then(|t| t.seconds).map(|x| x as f32)).unwrap_or(f32::MAX), s.phase == Phase::Paused)
                };
                let to = (position + delta).clamp(0.0, (length - 1.0).max(0.0));
                if paused {
                    self.state.lock().unwrap().offset = to;
                    self.publish();
                } else {
                    self.start(to);
                }
            }
            Cmd::Jump(i) => {
                let ok = {
                    let mut s = self.state.lock().unwrap();
                    let ok = i < s.queue.len();
                    if ok {
                        s.at = i;
                    }
                    ok
                };
                if ok {
                    self.start(0.0);
                }
            }
            Cmd::PlayNext(track) => {
                let empty = {
                    let mut s = self.state.lock().unwrap();
                    let empty = s.queue.is_empty();
                    let at = if empty { 0 } else { (s.at + 1).min(s.queue.len()) };
                    s.queue.insert(at, track);
                    empty
                };
                if empty {
                    self.start(0.0);
                }
            }
            Cmd::Remove(i) => self.remove(i),
            Cmd::Clear => {
                self.stop_sound();
                let mut s = self.state.lock().unwrap();
                s.queue.clear();
                s.at = 0;
                s.phase = Phase::Empty;
                s.offset = 0.0;
                drop(s);
                crate::nowplaying::set(None);
            }
            Cmd::Shuffle => {
                let mut s = self.state.lock().unwrap();
                if s.shuffle {
                    // Back to the order it had, at the song playing.
                    if let Some(order) = s.unshuffled.take() {
                        let id = s.track().map(|t| t.id.clone());
                        let extra: Vec<Track> = s.queue.iter().filter(|t| !order.iter().any(|o| o.id == t.id)).cloned().collect();
                        s.queue = order;
                        s.queue.extend(extra);
                        s.at = id.and_then(|id| s.queue.iter().position(|t| t.id == id)).unwrap_or(0);
                    }
                    s.shuffle = false;
                } else {
                    s.unshuffled = Some(s.queue.clone());
                    let from = (s.at + 1).min(s.queue.len());
                    shuffle_slice(&mut s.queue[from..]);
                    s.shuffle = true;
                }
            }
            Cmd::Repeat => {
                let mut s = self.state.lock().unwrap();
                s.repeat = match s.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
            }
            Cmd::Fetch(e) => self.fetched(e),
            Cmd::Said(gen, line) if gen == self.gen => self.said(&line),
            Cmd::Closed(gen) if gen == self.gen => self.closed(),
            Cmd::Said(..) | Cmd::Closed(_) => {}
            Cmd::Moved(follow) => self.moved(follow),
            // Looked at once this is handled (`after`).
            Cmd::Sleep => {}
        }
    }

    /// Sound moved while a song plays (or is on its way): on from where it
    /// is through the headphones that took over, or paused as the USB ones
    /// it played through came out. A song still being found opens what
    /// plays then by itself.
    fn moved(&mut self, follow: crate::sound::Follow) {
        use crate::sound::Follow;
        let (active, position) = {
            let s = self.state.lock().unwrap();
            (matches!(s.phase, Phase::Playing | Phase::Loading), s.position())
        };
        match follow {
            Follow::Pause if active => {
                *UNPLUGGED_AT.lock().unwrap() = Some(Instant::now());
                self.pause();
            }
            Follow::Carry if active && self.sound.is_some() => self.start(position),
            _ => {}
        }
    }

    fn remove(&mut self, i: usize) {
        // What happens to the song now: nothing (another was taken out),
        // the next one in its place, or the end of the queue.
        enum After {
            Nothing,
            Next { was_playing: bool },
            End,
        }
        let after = {
            let mut s = self.state.lock().unwrap();
            if i >= s.queue.len() {
                return;
            }
            s.queue.remove(i);
            if i < s.at {
                s.at -= 1;
                After::Nothing
            } else if i > s.at {
                After::Nothing
            } else if s.at < s.queue.len() {
                After::Next { was_playing: matches!(s.phase, Phase::Playing | Phase::Loading) }
            } else {
                After::End
            }
        };
        match after {
            After::Nothing => {}
            After::Next { was_playing: true } => self.start(0.0),
            After::Next { was_playing: false } | After::End => {
                self.stop_sound();
                let mut s = self.state.lock().unwrap();
                s.at = s.at.min(s.queue.len().saturating_sub(1));
                s.offset = 0.0;
                s.length = None;
                s.phase = if s.queue.is_empty() { Phase::Empty } else { Phase::Paused };
                drop(s);
                self.publish();
            }
        }
    }

    /// The song `step` along the queue, from its start; past the end,
    /// stopped (or round again, repeating all). `by_itself`: the song
    /// ended (offline, songs not on the card are passed over).
    fn step(&mut self, step: i32, by_itself: bool) {
        let to = {
            let s = self.state.lock().unwrap();
            let len = s.queue.len() as i32;
            let mut to = s.at as i32 + step;
            if s.repeat == Repeat::All && len > 0 {
                to = to.rem_euclid(len);
            }
            // Offline, on to the next one here.
            if by_itself && super::on_device() && !polite::online() {
                while to >= 0 && to < len && on_card(&s.queue[to as usize]).is_none() {
                    to += step.signum();
                }
            }
            (to >= 0 && to < len).then_some(to as usize)
        };
        match to {
            Some(i) => {
                self.state.lock().unwrap().at = i;
                self.start(0.0);
            }
            None => {
                self.stop_sound();
                let offline_end = by_itself && super::on_device() && !polite::online();
                let mut s = self.state.lock().unwrap();
                if s.phase != Phase::Empty {
                    s.phase = if offline_end && s.at + 1 < s.queue.len() { Phase::Failed("offline · the rest of the queue is not on the card".into()) } else { Phase::Paused };
                    s.offset = 0.0;
                }
                drop(s);
                // The queue has ended: nothing playing for the live activities.
                crate::nowplaying::set(None);
            }
        }
    }

    /// The song at the queue's place, from `at` seconds: from the card if it
    /// is there, else fetched and started once it begins to arrive.
    fn start(&mut self, at: f32) {
        self.stop_sound();
        self.fetch_failed = None;
        let track = {
            let mut s = self.state.lock().unwrap();
            let Some(track) = s.track().cloned() else {
                s.phase = Phase::Empty;
                drop(s);
                crate::nowplaying::set(None);
                return;
            };
            s.offset = at;
            s.anchor = None;
            s.phase = Phase::Loading;
            s.loading = None;
            s.buffering = false;
            s.length = track.seconds.map(|x| x as f32);
            track
        };
        self.sim_due = None;
        // Its words, for the island's card (from the card, else asked for).
        if store::choices().lyrics_island {
            super::lyrics::want(&track);
        }
        if !super::on_device() {
            // The simulator: a song from the "library" at once, else "found"
            // a moment later, as the handheld would.
            if track.local.is_some() {
                self.silent(at);
            } else {
                self.state.lock().unwrap().loading = Some("finding it…".into());
                self.sim_due = Some(Instant::now() + Duration::from_millis(600));
                self.publish();
            }
            return;
        }
        if let Some(path) = on_card(&track) {
            self.spawn(&track, &path, at, None);
            return;
        }
        if !polite::online() {
            self.fail("offline · this song is not on the card".into());
            return;
        }
        // Part here already (played before, or fetched ahead): from that.
        if let Some((p, have)) = store::partial(&track.id) {
            if p.bytes > 0 && have >= start_bytes(p.bytes, track.seconds) {
                let part = PathBuf::from(format!("{}.part", p.file().display()));
                self.spawn(&track, &part, at, Some(p.bytes));
                return;
            }
        }
        self.state.lock().unwrap().loading = Some("finding it…".into());
        self.publish();
    }

    fn silent(&mut self, at: f32) {
        // The simulator keeps what it plays too, as the handheld does.
        let playing = self.state.lock().unwrap().track().filter(|t| t.local.is_none() && !cfg!(test)).cloned();
        if let Some(t) = playing {
            store::note(&t, "webm", store::estimate_bytes(t.seconds), true);
            store::played_now(&t.id);
        }
        let length = self.state.lock().unwrap().length.unwrap_or(200.0);
        self.sound = Some(Sound::Silent { until: Instant::now() + Duration::from_secs_f32((length - at).max(0.5)) });
        {
            let mut s = self.state.lock().unwrap();
            s.phase = Phase::Playing;
            s.anchor = Some(Instant::now());
            s.loading = None;
        }
        self.publish();
    }

    fn fetched(&mut self, e: Event) {
        let (current, waiting) = {
            let s = self.state.lock().unwrap();
            (s.track().map(|t| t.id.clone()), s.phase == Phase::Loading && self.sound.is_none() && self.retry_at.is_none())
        };
        match e {
            Event::Step(id, step) if current.as_deref() == Some(id.as_str()) && waiting => {
                self.state.lock().unwrap().loading = Some(step.words().into());
            }
            Event::Progress { id, have, total, path, done } if current.as_deref() == Some(id.as_str()) => {
                if waiting {
                    let track = self.state.lock().unwrap().track().cloned().unwrap_or_default();
                    let enough = done || total.is_some_and(|t| have >= start_bytes(t, track.seconds)) || (total.is_none() && have >= START_BYTES_MIN * 2);
                    if enough {
                        let at = self.state.lock().unwrap().offset;
                        self.spawn(&track, &path, at, if done { None } else { total });
                    } else {
                        self.state.lock().unwrap().loading = Some("starting…".into());
                    }
                }
            }
            Event::Failed(id, why) if current.as_deref() == Some(id.as_str()) => {
                if waiting {
                    self.fail(why);
                } else {
                    // Playing what came: said if it runs out.
                    self.fetch_failed = Some(why);
                }
            }
            Event::Covers(id) if current.as_deref() == Some(id.as_str()) => self.publish(),
            _ => {}
        }
    }

    fn fail(&mut self, why: String) {
        self.stop_sound();
        let mut s = self.state.lock().unwrap();
        s.offset = s.position();
        s.phase = Phase::Failed(why);
        s.loading = None;
        s.anchor = None;
        drop(s);
        crate::nowplaying::set(None);
    }

    /// The player started on a file (`size`: still downloading, to this size).
    fn spawn(&mut self, track: &Track, path: &Path, at: f32, size: Option<u64>) {
        self.stop_sound();
        self.gen += 1;
        let gen = self.gen;
        let ffmpeg = self.ffmpeg_for.as_deref() == Some(track.id.as_str()) || !Path::new(library::PLAYER).exists();
        let mut cmd = if ffmpeg { ffmpeg_command(path, at) } else { player_command(path, at, size) };
        let album = album_key(track);
        if !ffmpeg && !album.is_empty() {
            cmd.arg("--album").arg(album);
        }
        self.played_on = crate::sound::route().usb_playing();
        // catalyst-play is told the song after this one, and the sleep
        // timer's fade, on its standard input; it writes its level for the
        // island's bars.
        if !ffmpeg {
            if let Some(dir) = Path::new(LEVEL_FILE).parent() {
                std::fs::create_dir_all(dir).ok();
            }
            cmd.arg("--levels").arg(LEVEL_FILE);
        }
        let child = polite::polite(&mut cmd, Manner::Playing).stdin(if ffmpeg { Stdio::null() } else { Stdio::piped() }).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => return self.fail("the player could not start".into()),
        };
        self.feed = child.stdin.take();
        self.fed = None;
        self.fading = false;
        let errors = Arc::new(Mutex::new(String::new()));
        self.errors = errors.clone();
        if let Some(mut err) = child.stderr.take() {
            std::thread::Builder::new()
                .name("music-errors".into())
                .stack_size(64 * 1024)
                .spawn(move || {
                    let mut text = String::new();
                    err.read_to_string(&mut text).ok();
                    *errors.lock().unwrap() = text.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
                })
                .ok();
        }
        if let Some(out) = child.stdout.take() {
            let tx = self.tx.clone();
            std::thread::Builder::new()
                .name("music-said".into())
                .stack_size(64 * 1024)
                .spawn(move || {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        // ffmpeg's progress, as the player's own words.
                        let line = match line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                            Some(us) => format!("at {:.1}", us.max(0) as f64 / 1e6),
                            None if ffmpeg && line.starts_with("progress=") => continue,
                            None => line,
                        };
                        tx.send(Cmd::Said(gen, line)).ok();
                    }
                    tx.send(Cmd::Closed(gen)).ok();
                })
                .ok();
        }
        self.sound = Some(Sound::Process(child));
        self.state.lock().unwrap().loading = Some("starting…".into());
        if ffmpeg {
            // ffmpeg says nothing until it plays: taken as playing now.
            self.said(&format!("at {at:.1}"));
        }
        if !track.id.starts_with("lib:") && track.local.is_none() {
            store::played_now(&track.id);
        }
    }

    /// A line from the player.
    fn said(&mut self, line: &str) {
        // It ran on into the song told to it: that one is playing now.
        if line.starts_with("track ") {
            if let Some((id, _)) = self.fed.take() {
                let mut s = self.state.lock().unwrap();
                let after = (s.at + 1).min(s.queue.len());
                let found = s.queue[after..].iter().position(|t| t.id == id).map(|i| i + after).or_else(|| s.queue.iter().position(|t| t.id == id));
                if let Some(i) = found {
                    s.at = i;
                    s.offset = 0.0;
                    s.anchor = Some(Instant::now());
                    s.length = s.queue[i].seconds.map(|x| x as f32);
                    let t = s.queue[i].clone();
                    drop(s);
                    if !t.id.starts_with("lib:") && t.local.is_none() {
                        store::played_now(&t.id);
                    }
                }
            }
            self.publish();
            return;
        }
        let mut publish = false;
        {
            let mut s = self.state.lock().unwrap();
            if let Some(rest) = line.strip_prefix("ready ") {
                if let Some(secs) = rest.split_whitespace().find_map(|w| w.strip_prefix("seconds=")).and_then(|v| v.parse::<f32>().ok()).filter(|&v| v > 0.0) {
                    s.length = Some(secs);
                }
                publish = true;
            } else if let Some(at) = line.strip_prefix("at ").and_then(|v| v.trim().parse::<f32>().ok()) {
                let first = s.phase != Phase::Playing;
                // A correction more than a second off is told on too.
                publish = first || (s.position() - at).abs() > 1.0;
                s.offset = at;
                s.anchor = Some(Instant::now());
                if first {
                    s.phase = Phase::Playing;
                    s.loading = None;
                    self.busy_tries = 0;
                }
            } else if line == "buffering" {
                s.offset = s.position();
                s.buffering = true;
                publish = true;
            } else if line == "playing" {
                s.buffering = false;
                s.anchor = Some(Instant::now());
                publish = true;
            } else if let Some(n) = line.strip_prefix("xrun ").and_then(|v| v.trim().parse::<u32>().ok()) {
                let _ = n;
                let total = XRUNS.fetch_add(1, Ordering::Relaxed) + 1;
                if super::on_device() {
                    std::fs::create_dir_all("/run/catalyst-music").ok();
                    std::fs::write("/run/catalyst-music/xruns", format!("{total}\n")).ok();
                }
            }
        }
        // Run dry with the download stopped: said, not waited on forever.
        if line == "buffering" {
            if let Some(why) = self.fetch_failed.take() {
                return self.fail(why);
            }
        }
        if publish {
            self.publish();
        }
    }

    /// The player ended: the song's end, or a problem.
    fn closed(&mut self) {
        let Some(Sound::Process(mut child)) = self.sound.take() else { return };
        let status = child.wait().ok();
        // Its standard error is closed by now: what it said is in.
        std::thread::sleep(Duration::from_millis(20));
        let why = self.errors.lock().unwrap().clone();
        let code = status.and_then(|s| s.code());
        let (id, position, played) = {
            let s = self.state.lock().unwrap();
            (s.track().map(|t| t.id.clone()).unwrap_or_default(), s.position(), s.phase == Phase::Playing)
        };
        match code {
            Some(0) => self.song_ended(),
            // A song catalyst-play does not read (HE-AAC, say): ffmpeg plays it.
            Some(3) if self.ffmpeg_for.as_deref() != Some(id.as_str()) => {
                self.ffmpeg_for = Some(id);
                self.start(position);
            }
            Some(4) if self.busy_tries < 4 => {
                // Waiting a moment for whatever has the device.
                self.busy_tries += 1;
                self.retry_at = Some(Instant::now() + Duration::from_millis(900));
                let mut s = self.state.lock().unwrap();
                s.offset = position;
                s.phase = Phase::Loading;
            }
            // Its sound stopped because the USB headphones came out: paused
            // where it was, as their going would pause it anyway.
            _ if self.played_on.is_some_and(crate::sound::usb_gone) => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.offset = position;
                    s.anchor = None;
                }
                *UNPLUGGED_AT.lock().unwrap() = Some(Instant::now());
                self.pause();
            }
            _ => {
                let said = short_error(&why, played);
                let mut s = self.state.lock().unwrap();
                s.offset = position;
                drop(s);
                self.fail(said);
            }
        }
    }

    fn deadline(&mut self) {
        if !self.retired && self.paused_since.is_some_and(|at| at.elapsed() >= RETIRE_AFTER) {
            self.retired = true;
            self.publish();
            return;
        }
        if self.sim_due.is_some_and(|at| Instant::now() >= at) {
            self.sim_due = None;
            let at = self.state.lock().unwrap().offset;
            self.silent(at);
            return;
        }
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            let at = self.state.lock().unwrap().offset;
            self.start(at);
            return;
        }
        if let Some(Sound::Silent { until }) = &self.sound {
            if Instant::now() >= *until {
                self.sound = None;
                self.song_ended();
            }
        }
    }

    /// The song played to its end: again (repeating one), the next, or, if
    /// the sleep timer was waiting for it, paused there.
    fn song_ended(&mut self) {
        if super::sleep::take_song_end() {
            return self.stop_after_song();
        }
        let one = self.state.lock().unwrap().repeat == Repeat::One;
        if one {
            self.start(0.0);
        } else {
            self.step(1, true);
        }
    }

    fn pause(&mut self) {
        let position = self.state.lock().unwrap().position();
        self.stop_sound();
        {
            let mut s = self.state.lock().unwrap();
            s.offset = position;
            s.anchor = None;
            s.phase = Phase::Paused;
            s.loading = None;
            s.buffering = false;
        }
        self.publish();
    }

    /// The player stopped (the device closed, so the amplifier can go off).
    fn stop_sound(&mut self) {
        self.gen += 1;
        self.sim_due = None;
        self.feed = None;
        self.fed = None;
        self.fading = false;
        if let Some(Sound::Process(mut child)) = self.sound.take() {
            polite::end(&mut child);
        }
    }

    /// What plays now, for the live activities.
    fn publish(&self) {
        let s = self.state.lock().unwrap();
        let Some(t) = s.track().cloned() else {
            drop(s);
            crate::nowplaying::set(None);
            return;
        };
        if matches!(s.phase, Phase::Empty | Phase::Failed(_)) || (s.phase == Phase::Paused && self.retired) {
            drop(s);
            crate::nowplaying::set(None);
            return;
        }
        let np = crate::nowplaying::NowPlaying {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            cover: None,
            position: s.position(),
            duration: s.length.unwrap_or(0.0),
            playing: s.phase == Phase::Playing && !s.buffering,
            at: Instant::now(),
        };
        drop(s);
        let cover = covers::small_for(&t);
        crate::nowplaying::set(Some(crate::nowplaying::NowPlaying { cover, ..np }));
    }
}

/// Enough of a song to start it: about five seconds' worth.
fn start_bytes(total: u64, seconds: Option<u32>) -> u64 {
    let per_second = seconds.filter(|&s| s > 0).map_or(20_000, |s| total / s as u64);
    (per_second * START_SECONDS).max(START_BYTES_MIN).min(total)
}

/// A song's album, as catalyst-play compares them (a crossfade is never
/// made within one): its name alone, so a compilation's songs by their
/// several artists (a DJ mix, a live album) still run on into each other.
fn album_key(t: &Track) -> String {
    t.album.trim().to_lowercase().replace(['\t', '\n'], " ")
}

fn player_command(path: &Path, at: f32, size: Option<u64>) -> Command {
    let mut cmd = Command::new(library::PLAYER);
    if at > 0.05 {
        cmd.arg("--start").arg(format!("{at:.2}"));
    }
    if let Some(s) = size {
        cmd.arg("--size").arg(s.to_string());
    }
    cmd.arg(path);
    cmd
}

/// ffmpeg, for what catalyst-play does not read: one thread, resampled to
/// 48 kHz only when it is not already, saying where it is every 5 s.
fn ffmpeg_command(path: &Path, at: f32) -> Command {
    let mut cmd = Command::new(super::FFMPEG);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-threads", "1"]).args(["-ss", &format!("{at:.2}"), "-i"]).arg(path);
    cmd.args(["-vn", "-ac", "2", "-af", "aresample=48000", "-progress", "pipe:1", "-stats_period", "5", "-f", "alsa", "default"]);
    cmd
}

fn short_error(text: &str, played: bool) -> String {
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().trim_start_matches("catalyst-play: ");
    match line {
        l if l.contains("busy") => "the sound device is busy".into(),
        l if l.contains("download stopped") => "the download stopped".into(),
        "" if played => "the song stopped".into(),
        "" => "the song could not be played".into(),
        l => format!("the song stopped: {}", l.chars().take(70).collect::<String>()),
    }
}

/// Fisher–Yates, from the clock (a shuffle need not be secret).
fn shuffle_slice(items: &mut [Track]) {
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |d| d.as_nanos() as u64) | 1;
    for i in (1..items.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        items.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(want: &dyn Fn(&Now) -> bool) -> bool {
        for _ in 0..100 {
            if want(&now()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        false
    }

    #[test]
    fn the_simulators_player_plays_pauses_shuffles_and_moves_along() {
        let songs = super::super::demo_catalogue();
        play(songs.clone(), 2, false);
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 2), "{:?}", now().phase);
        toggle();
        assert!(wait(&|n| n.phase == Phase::Paused));
        next();
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 3));
        // Sound moving: on through headphones that take over; paused as the
        // USB ones it played through come out, and left paused after.
        moved(crate::sound::Follow::Carry);
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 3));
        moved(crate::sound::Follow::Pause);
        assert!(wait(&|n| n.phase == Phase::Paused && n.at == 3));
        assert!(paused_unplugged(), "the island says it was paused");
        assert!(!paused_unplugged(), "once");
        moved(crate::sound::Follow::Carry);
        moved(crate::sound::Follow::Stay);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(now().phase, Phase::Paused, "paused: nothing to carry on");
        toggle();
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 3));
        // Back from the start of a song is the song before.
        previous();
        assert!(wait(&|n| n.at == 2));
        play_next(songs[9].clone());
        assert!(wait(&|n| n.queue.get(3).is_some_and(|t| t.id == "demo10")));
        remove(3);
        assert!(wait(&|n| n.queue.len() == songs.len()));
        // Shuffled: the song playing stays, the rest is all there, in another order; and back.
        shuffle();
        assert!(wait(&|n| n.shuffle));
        let n = now();
        assert_eq!((n.at, n.track().map(|t| t.id.clone())), (2, Some("demo03".into())));
        let mut ids: Vec<String> = n.queue.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        assert_eq!(ids.len(), songs.len());
        shuffle();
        assert!(wait(&|n| !n.shuffle && n.queue.iter().map(|t| &t.id).eq(songs.iter().map(|t| &t.id))));
        // Repeat: all, one, off.
        repeat();
        assert!(wait(&|n| n.repeat == Repeat::All));
        repeat();
        repeat();
        assert!(wait(&|n| n.repeat == Repeat::Off));
        assert_eq!(media_key(Media::PlayPause).as_deref(), Some("paused"));
        assert!(wait(&|n| n.phase == Phase::Paused));
        // Taking out the song paused on: the next in its place, still paused.
        let before = now();
        let next_id = before.queue[before.at + 1].id.clone();
        remove(before.at);
        assert!(wait(&|n| n.queue.len() == before.queue.len() - 1 && n.phase == Phase::Paused && n.track().is_some_and(|t| t.id == next_id)));
    }

    #[test]
    fn a_song_starts_once_five_seconds_of_it_are_in() {
        // 4 MB over 232 s: about 17.7 KB a second, so 89 KB, but never under 96 KB.
        assert_eq!(start_bytes(4_115_008, Some(232)), START_BYTES_MIN);
        // A high-rate song: five seconds of it.
        assert_eq!(start_bytes(12_000_000, Some(200)), 300_000);
        // A short file: all of it.
        assert_eq!(start_bytes(50_000, Some(3)), 50_000);
        let args = |c: &Command| c.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(args(&player_command(Path::new("/x.webm.part"), 12.5, Some(99))), ["--start", "12.50", "--size", "99", "/x.webm.part"]);
        assert_eq!(args(&player_command(Path::new("/x.opus"), 0.0, None)), ["/x.opus"]);
    }
}
