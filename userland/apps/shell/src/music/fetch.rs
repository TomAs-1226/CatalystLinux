//! Music's one network worker: every song fetched goes through it, one at
//! a time, so two lookups or two downloads never run at once.
//!
//! In order: the song playing (into the played cache, the player reading it
//! as it grows); the next few in the queue (looked ahead, so the next song
//! starts at once and, screen off, Wi-Fi can rest between); then saving to
//! the library (downloads.rs). Between chunks it looks again, so a song
//! asked for now overtakes one fetched ahead (which keeps what it has and
//! goes on later from there).
//!
//! A song comes in 1 MiB ranges over one curl and one connection:
//! googlevideo serves ranges at full speed (3.4 MB/s measured) but slows a
//! single whole-file request to about the song's own bit rate (35 KB/s).
//! The ranges and the address go to curl in a file of its own, not on its
//! command line.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::lookup::{self, Step, Stream};
use super::polite::{self, Manner};
use super::{covers, store, Track};

/// Each range asked for.
const CHUNK: u64 = 1 << 20;
/// Progress is said this often while a song comes (and once at the first
/// few seconds' worth, so it can start).
const SAY_EVERY: u64 = 256 * 1024;
const FIRST_SAY: u64 = 96 * 1024;
/// A song looked ahead that failed is tried again after this long.
const AHEAD_BACKOFF: Duration = Duration::from_secs(600);

/// What the player is told.
#[derive(Clone, Debug)]
pub enum Event {
    /// Finding the song playing: how far it has got.
    Step(String, Step),
    /// Part of a song is on the card (`done`: all of it, at `path`).
    Progress { id: String, have: u64, total: Option<u64>, path: PathBuf, done: bool },
    /// The song playing could not be fetched.
    Failed(String, String),
    /// Its covers are on the card.
    Covers(String),
}

#[derive(Default)]
struct State {
    current: Option<Track>,
    ahead: Vec<Track>,
    started: bool,
    woken: bool,


    urls: HashMap<String, Stream>,
    failed: HashMap<String, Instant>,
    events: Option<fn(Event)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static CV: Condvar = Condvar::new();

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock().unwrap();
    f(s.get_or_insert_with(State::default))
}

/// How the player hears of songs coming.
pub fn on_event(f: fn(Event)) {
    with(|s| s.events = Some(f));
}

fn tell(e: Event) {
    if let Some(f) = with(|s| s.events) {
        f(e);
    }
}

/// The song playing (if it is not on the card yet) and those after it.
pub fn want(current: Option<Track>, ahead: Vec<Track>) {
    with(|s| {
        s.current = current;
        s.ahead = ahead;
    });
    wake();
}

/// Something changed (a choice, a download added, the network back): look again.
pub fn wake() {
    let start = with(|s| {
        s.woken = true;
        !std::mem::replace(&mut s.started, true)
    });
    if start {
        std::thread::Builder::new().name("music-fetch".into()).stack_size(256 * 1024).spawn(run).ok();
    }
    CV.notify_all();
}


/// A song needs fetching: not in the library, not all in the cache.
pub fn needs(t: &Track) -> bool {
    t.local.as_ref().is_none_or(|p| !p.exists()) && super::library::by_youtube(&t.id).is_none() && store::cached(&t.id).is_none()
}

/// Why background work waits now, if it does (`saving`: to the library,
/// which also minds the Wi-Fi and charger choices).
pub fn waits(saving: bool) -> Option<&'static str> {
    if !polite::online() {
        return Some("waiting for the internet");
    }
    if polite::deep() {
        return Some("waiting while in deep standby");
    }
    if polite::memory_short() {
        return Some("waiting for memory to free up");
    }
    if !saving {
        return None;
    }
    let c = store::choices();
    let battery = polite::battery();
    if c.paused {
        Some("paused")
    } else if c.wifi_only && !polite::on_wifi() {
        Some("waiting for Wi-Fi")
    } else if c.charging_only && battery.is_some_and(|(charging, _)| !charging) {
        Some("waiting for the charger")
    } else if polite::battery_low() {
        Some("battery low: waiting for the charger")
    } else {
        None
    }
}

enum Job {
    Song(Track, bool),
    Save,
}

fn choose(s: &mut State) -> Option<Job> {
    if let Some(t) = s.current.clone().filter(needs) {
        if polite::online() {
            return Some(Job::Song(t, true));
        }
    }
    if waits(false).is_none() {
        let now = Instant::now();
        s.failed.retain(|_, at| now.duration_since(*at) < AHEAD_BACKOFF);
        if let Some(t) = s.ahead.iter().find(|t| !s.failed.contains_key(&t.id) && needs(t)).cloned() {
            return Some(Job::Song(t, false));
        }
    }
    if super::downloads::has_waiting() && waits(true).is_none() {
        return Some(Job::Save);
    }
    None
}

/// The player wants the network for something now (the song playing, or
/// one coming up): downloads to the library give way.
pub fn player_waiting() -> bool {
    let (current, ahead, failed) = with(|s| (s.current.clone(), s.ahead.clone(), s.failed.keys().cloned().collect::<Vec<_>>()));
    (current.as_ref().is_some_and(needs) && polite::online()) || (waits(false).is_none() && ahead.iter().any(|t| !failed.contains(&t.id) && needs(t)))
}

fn run() {
    polite::background_thread();
    loop {
        let job = {
            let mut guard = STATE.lock().unwrap();
            let s = guard.get_or_insert_with(State::default);
            s.woken = false;
            choose(s)
        };
        match job {
            Some(Job::Song(t, current)) => {

                fetch_song(&t, current);

            }
            Some(Job::Save) => super::downloads::run_next(),
            None => {
                super::downloads::maybe_save_likes();
                // Nothing to do: asleep until asked (or, with downloads
                // waiting on Wi-Fi or the charger, a look a minute).
                let wait = if super::downloads::has_waiting() { Duration::from_secs(60) } else { Duration::from_secs(3600) };
                let guard = STATE.lock().unwrap();
                let _ = CV.wait_timeout_while(guard, wait, |s| !s.as_ref().is_some_and(|s| s.woken)).unwrap();
            }
        }
    }
}

/// Whether the song being fetched should give way: it is no longer the
/// song playing nor one coming up (or the one playing needs fetching).
fn give_way(id: &str, current: bool) -> bool {
    with(|s| {
        let playing = s.current.as_ref().map(|t| t.id.as_str());
        if current {
            playing != Some(id)
        } else {
            s.current.as_ref().is_some_and(needs) && playing != Some(id) || !s.ahead.iter().any(|t| t.id == id)
        }
    })
}

/// A song's stream: kept from before if it is good yet, else looked up.
pub fn stream_for(id: &str, slow_too: bool, step: &dyn Fn(Step)) -> Result<Stream, String> {
    if let Some(s) = with(|s| s.urls.get(id).cloned()).filter(|s| s.expires > Instant::now()) {
        return Ok(s);
    }
    let found = lookup::find(id, slow_too, super::signed_in(), step)?;
    with(|s| {
        s.urls.retain(|_, u| u.expires > Instant::now());
        s.urls.insert(id.to_owned(), found.clone());
    });
    Ok(found)
}

pub fn forget_stream(id: &str) {
    with(|s| s.urls.remove(id));
}

fn fetch_song(t: &Track, current: bool) {
    let id = t.id.clone();
    let say_step = |step: Step| {
        if current {
            tell(Event::Step(id.clone(), step));
        }
    };
    // Looked ahead, the slow ways only with memory to spare.
    let slow_too = current || polite::memory_available_mb().is_some_and(|mb| mb >= 160);
    let mut stream = match stream_for(&t.id, slow_too, &say_step) {
        Ok(s) => s,
        Err(why) => return failed(t, current, why),
    };
    let ext = stream.ext.clone();
    let entry = store::Played { track: t.clone(), ext: ext.clone(), bytes: 0, complete: false, last: 0, plays: 0 };
    let file = entry.file();
    let part = PathBuf::from(format!("{}.part", file.display()));
    std::fs::create_dir_all(store::cache_dir()).ok();
    // A part of another kind (Opus before, AAC now) cannot be carried on.
    if let Some((was, _)) = store::partial(&t.id) {
        if was.ext != ext {
            std::fs::remove_file(format!("{}.part", was.file().display())).ok();
        }
    }
    store::note(t, &ext, stream.bytes.unwrap_or(0), false);
    let mut covers_done = false;
    let mut tries = 0;
    loop {
        let total = stream.bytes;
        let result = {
            let mut progress = |have: u64| {
                tell(Event::Progress { id: t.id.clone(), have, total, path: part.clone(), done: false });
                if !covers_done && have >= FIRST_SAY {
                    covers_done = true;
                    let (big, small) = covers::cache_pair(&t.id);
                    if covers::fetch_pair(t, &big, &small) {
                        tell(Event::Covers(t.id.clone()));
                    }
                }
                !give_way(&t.id, current)
            };
            download(&stream.url, &part, total, &mut progress)
        };
        match result {
            Dl::Done(have) => {
                if std::fs::rename(&part, &file).is_err() {
                    return failed(t, current, "could not keep the song on the card".into());
                }
                store::note(t, &ext, have, true);
                if !covers_done {
                    let (big, small) = covers::cache_pair(&t.id);
                    if covers::fetch_pair(t, &big, &small) {
                        tell(Event::Covers(t.id.clone()));
                    }
                }
                tell(Event::Progress { id: t.id.clone(), have, total: Some(have), path: file.clone(), done: true });
                let keep: Vec<String> = with(|s| s.current.iter().chain(s.ahead.iter()).map(|t| t.id.clone()).collect());
                store::evict(&keep);
                return;
            }
            Dl::Stopped => return,
            Dl::Expired | Dl::Failed(_) if tries < 2 => {
                // The address may have gone stale: found again, and on from where it was.
                tries += 1;
                forget_stream(&t.id);
                std::thread::sleep(Duration::from_secs(if tries == 1 { 1 } else { 4 }));
                stream = match stream_for(&t.id, slow_too, &say_step) {
                    Ok(s) => s,
                    Err(why) => return failed(t, current, why),
                };
            }
            Dl::Expired => return failed(t, current, "youtube stopped sending the song".into()),
            Dl::Failed(why) => return failed(t, current, why),
        }
    }
}

fn failed(t: &Track, current: bool, why: String) {
    if current {
        tell(Event::Failed(t.id.clone(), why));
    } else {
        with(|s| s.failed.insert(t.id.clone(), Instant::now()));
    }
}

pub enum Dl {
    Done(u64),
    Stopped,
    /// Refused (403/410): the address has gone stale.
    Expired,
    Failed(String),
}

/// For the xrun harness: downloads held to this many bytes a second.
fn rate_limit() -> Option<String> {
    std::env::var("CATALYST_MUSIC_RATE").ok().filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// A stream into `part`, from what is there already, in ranges.
/// `progress(have)` is told as it comes; false stops it there.
pub fn download(url: &str, part: &Path, total: Option<u64>, progress: &mut dyn FnMut(u64) -> bool) -> Dl {
    let mut have = std::fs::metadata(part).map_or(0, |m| m.len());
    if let Some(t) = total {
        if have > t {
            std::fs::remove_file(part).ok();
            have = 0;
        }
        if have == t && t > 0 {
            return Dl::Done(have);
        }
    }
    // The ranges still to come; with the size not known, one at a time
    // until one comes short.
    let end = total.unwrap_or(have + CHUNK);
    let mut config = String::from("fail-early\n");
    let mut from = have;
    while from < end {
        let to = (from + CHUNK).min(end) - 1;
        config.push_str(&format!(
            "silent\nshow-error\nfail\nproto = \"=https\"\nconnect-timeout = 15\nspeed-limit = 2048\nspeed-time = 20\nuser-agent = \"{}\"\nrange = \"{from}-{to}\"\nurl = \"{}\"\n",
            lookup::USER_AGENT,
            url.replace('"', "%22")
        ));
        if let Some(r) = rate_limit() {
            config.push_str(&format!("limit-rate = \"{r}\"\n"));
        }
        from = to + 1;
        if from < end {
            config.push_str("next\n");
        }
    }
    let file = super::private_file("ranges");
    if crate::sys::write_private(&file, config.as_bytes()).is_err() {
        return Dl::Failed("could not prepare the download".into());
    }
    let mut cmd = Command::new("curl");
    cmd.arg("-K").arg(&file).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = polite::polite(&mut cmd, Manner::Background).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => {
            std::fs::remove_file(&file).ok();
            return Dl::Failed("curl could not start".into());
        }
    };
    let mut out = match std::fs::OpenOptions::new().create(true).append(true).open(part) {
        Ok(f) => f,
        Err(e) => {
            polite::end(&mut child);
            std::fs::remove_file(&file).ok();
            return Dl::Failed(format!("could not write the song: {e}"));
        }
    };
    let mut err_text = String::new();
    let mut errors = child.stderr.take();
    let err_reader = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut s = String::new();
            if let Some(e) = errors.as_mut() {
                e.read_to_string(&mut s).ok();
            }
            s
        })
        .ok();
    let mut stdout = child.stdout.take();
    let mut buf = vec![0u8; 64 * 1024];
    let started = have;
    let mut next_say = if have < FIRST_SAY { FIRST_SAY } else { have + SAY_EVERY };
    let mut stopped = false;
    if let Some(o) = stdout.as_mut() {
        loop {
            let n = match o.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if out.write_all(&buf[..n]).is_err() {
                stopped = true;
                break;
            }
            have += n as u64;
            if have >= next_say {
                next_say = have + SAY_EVERY;
                out.flush().ok();
                if !progress(have) {
                    stopped = true;
                    break;
                }
            }
        }
    }
    out.flush().ok();
    drop(stdout);
    if stopped {
        polite::end(&mut child);
    }
    let status = child.wait().ok();
    if let Some(r) = err_reader {
        err_text = r.join().unwrap_or_default();
    }
    std::fs::remove_file(&file).ok();
    if stopped {
        return Dl::Stopped;
    }
    let ok = status.is_some_and(|s| s.success());
    match total {
        Some(t) if have >= t => Dl::Done(have),
        None if ok && have - started < CHUNK => Dl::Done(have),
        None if ok => download(url, part, None, progress),
        _ if err_text.contains("403") || err_text.contains("410") => Dl::Expired,
        _ if err_text.contains("Could not resolve") || err_text.contains("Failed to connect") || err_text.contains("Couldn't connect") => Dl::Failed(lookup::NO_INTERNET.into()),
        _ if have > started => Dl::Failed("the connection dropped".into()),
        _ => Dl::Failed(err_text.lines().last().map(|l| l.trim_start_matches("curl: ").chars().take(60).collect::<String>()).filter(|l| !l.is_empty()).unwrap_or_else(|| "the download failed".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_on_the_card_needs_no_fetching() {
        let t = Track { id: "notcached01".into(), ..Track::default() };
        assert!(needs(&t));
        let here = std::env::temp_dir();
        assert!(!needs(&Track { local: Some(here), ..t.clone() }), "a library song");
    }
}
