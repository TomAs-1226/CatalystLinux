//! Saving to the library, many songs at a time: an album, a playlist, an
//! artist's top songs, all your likes, everything played.
//!
//! The queue is kept in `/data/music/downloads.json`, so it goes on after a
//! restart; a song half-downloaded goes on from where it was (its part in
//! `library/.incoming/`). One song at a time, through Music's one network
//! worker and behind whatever is playing; at low priority, and only while
//! the choices allow (Wi-Fi only, while charging), the battery is not low,
//! memory is free, and the handheld is not in deep standby. A song already
//! played (in the cache) is saved from there, without the network. Each
//! album's cover is fetched once, for the album.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::library::{self, Meta};
use super::{covers, fetch, store, Track};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Waiting,
    Active,
    /// Saved, here.
    Done(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub n: u64,
    pub track: Track,
    pub meta: Meta,
    /// Saved into this playlist too (a playlist downloaded).
    pub playlist: Option<String>,
    /// What it came with: an album's name, "your likes".
    pub from: String,
    pub state: State,
    pub have: u64,
    pub total: u64,
}

impl Item {
    fn to_json(&self) -> Value {
        let t = &self.track;
        let (state, detail) = match &self.state {
            State::Waiting | State::Active => ("waiting", String::new()),
            State::Done(p) => ("done", p.clone()),
            State::Failed(w) => ("failed", w.clone()),
        };
        json!({"n": self.n, "id": t.id, "title": t.title, "artist": t.artist, "album": t.album, "seconds": t.seconds.unwrap_or(0), "art": t.art,
            "album_artist": self.meta.album_artist, "track_no": self.meta.track_no, "year": self.meta.year, "playlist": self.playlist, "from": self.from,
            "state": state, "detail": detail, "total": self.total})
    }

    fn from_json(v: &Value) -> Option<Item> {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        let id = s("id");
        if id.is_empty() {
            return None;
        }
        let state = match v["state"].as_str() {
            Some("done") => State::Done(s("detail")),
            Some("failed") => State::Failed(s("detail")),
            _ => State::Waiting,
        };
        Some(Item {
            n: n("n"),
            track: Track { id, title: s("title"), artist: s("artist"), album: s("album"), seconds: Some(n("seconds") as u32).filter(|&x| x > 0), art: s("art"), local: None },
            meta: Meta { album_artist: s("album_artist"), track_no: n("track_no") as u32, year: n("year") as u32 },
            playlist: v["playlist"].as_str().map(str::to_owned),
            from: s("from"),
            state,
            have: 0,
            total: n("total"),
        })
    }
}

#[derive(Default)]
struct Queue {
    items: Vec<Item>,
    next: u64,
}

static QUEUE: Mutex<Option<Queue>> = Mutex::new(None);
static STAMP: AtomicU64 = AtomicU64::new(0);

fn file() -> PathBuf {
    store::root().join("downloads.json")
}

fn incoming() -> PathBuf {
    store::library_dir().join(".incoming")
}

fn with<R>(f: impl FnOnce(&mut Queue) -> R) -> R {
    let mut q = QUEUE.lock().unwrap();
    let q = q.get_or_insert_with(|| {
        let items: Vec<Item> = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().map(|a| a.iter().filter_map(Item::from_json).collect())).unwrap_or_default();
        let next = items.iter().map(|i| i.n).max().unwrap_or(0) + 1;
        Queue { items, next }
    });
    f(q)
}

fn save(q: &Queue) {
    let v = Value::Array(q.items.iter().map(Item::to_json).collect());
    store::write_atomic(&file(), v.to_string().as_bytes()).ok();
    STAMP.fetch_add(1, Ordering::Relaxed);
}

/// Counts changes, for the Downloads page.
pub fn stamp() -> u64 {
    STAMP.load(Ordering::Relaxed)
}

/// What the songs will take, about (their lengths at YouTube's Opus rate).
pub fn estimate(tracks: &[Track]) -> u64 {
    tracks.iter().map(|t| store::estimate_bytes(t.seconds)).sum()
}

/// Songs to save, after any waiting already. Those in the library, or in
/// the queue already, are left out. Returns how many were added.
pub fn add(songs: Vec<(Track, Meta)>, from: &str, playlist: Option<&str>) -> usize {
    let added = with(|q| {
        let mut added = 0;
        for (t, meta) in songs {
            if t.id.is_empty() || t.id.starts_with("lib:") || library::by_youtube(&t.id).is_some() {
                continue;
            }
            if q.items.iter().any(|i| i.track.id == t.id && !matches!(i.state, State::Failed(_))) {
                continue;
            }
            q.items.retain(|i| i.track.id != t.id);
            let n = q.next;
            q.next += 1;
            let total = store::estimate_bytes(t.seconds);
            q.items.push(Item { n, track: t, meta, playlist: playlist.map(str::to_owned), from: from.to_owned(), state: State::Waiting, have: 0, total });
            added += 1;
        }
        if added > 0 {
            save(q);
        }
        added
    });
    if added > 0 {
        fetch::wake();
    }
    added
}

pub fn items() -> Vec<Item> {
    with(|q| q.items.clone())
}

pub fn has_waiting() -> bool {
    with(|q| q.items.iter().any(|i| i.state == State::Waiting))
}

/// The queue at a glance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

pub fn summary() -> Summary {
    with(|q| {
        let mut s = Summary::default();
        for i in &q.items {
            s.total += 1;
            s.bytes_total += i.total.max(i.have);
            match i.state {
                State::Done(_) => {
                    s.done += 1;
                    s.bytes_done += i.total;
                }
                State::Failed(_) => s.failed += 1,
                _ => s.bytes_done += i.have,
            }
        }
        s
    })
}

/// A failed song tried again.
pub fn retry(n: u64) {
    with(|q| {
        if let Some(i) = q.items.iter_mut().find(|i| i.n == n && matches!(i.state, State::Failed(_))) {
            i.state = State::Waiting;
            save(q);
        }
    });
    fetch::wake();
}

pub fn retry_failed() {
    with(|q| {
        for i in q.items.iter_mut().filter(|i| matches!(i.state, State::Failed(_))) {
            i.state = State::Waiting;
        }
        save(q);
    });
    fetch::wake();
}

/// A song taken out of the queue (what it had downloaded too).
pub fn cancel(n: u64) {
    with(|q| {
        if let Some(i) = q.items.iter().find(|i| i.n == n) {
            if !matches!(i.state, State::Done(_)) {
                for ext in ["webm", "m4a"] {
                    std::fs::remove_file(incoming().join(format!("{}.{ext}.part", store::safe_id(&i.track.id)))).ok();
                }
            }
        }
        q.items.retain(|i| i.n != n);
        save(q);
    });
}

/// The saved and the taken-out gone from the list (the songs stay saved).
pub fn clear_finished() {
    with(|q| {
        q.items.retain(|i| !matches!(i.state, State::Done(_)));
        save(q);
    });
}

fn set(n: u64, state: State) {
    with(|q| {
        if let Some(i) = q.items.iter_mut().find(|i| i.n == n) {
            i.state = state;
        }
        save(q);
    });
    report();
}

fn progress(n: u64, have: u64, total: Option<u64>) {
    with(|q| {
        if let Some(i) = q.items.iter_mut().find(|i| i.n == n) {
            i.have = have;
            if let Some(t) = total {
                i.total = t;
            }
        }
    });
    STAMP.fetch_add(1, Ordering::Relaxed);
    report();
}

/// The downloads as a live activity (the island, the lock screen, StandBy):
/// under way with how far the queue has come, then done.
static TASK: Mutex<Option<crate::liveactivity::TaskHandle>> = Mutex::new(None);

fn report() {
    let s = summary();
    let under_way = with(|q| q.items.iter().any(|i| matches!(i.state, State::Waiting | State::Active)));
    let mut task = TASK.lock().unwrap();
    if under_way {
        let left = s.total - s.done - s.failed;
        let h = task.get_or_insert_with(|| crate::liveactivity::task(crate::liveactivity::Kind::Download, "Saving music", format!("{left} to the library")));
        if s.bytes_total > 0 {
            h.progress(s.bytes_done as f32 / s.bytes_total as f32);
        }
    } else if let Some(h) = task.take() {
        h.done(if s.failed > 0 { format!("{} saved · {} failed", s.done, s.failed) } else { format!("{} saved", s.done) });
    }
}

/// Whether the one at work now should stop for something before it: the
/// player wants the network, a choice now forbids it, or it was taken out.
fn should_stop(n: u64) -> bool {
    fetch::player_waiting() || fetch::waits(true).is_some() || with(|q| !q.items.iter().any(|i| i.n == n && i.state == State::Active))
}

/// The first song waiting, saved (by the network worker).
pub fn run_next() {
    let Some(item) = with(|q| {
        let i = q.items.iter_mut().find(|i| i.state == State::Waiting)?;
        i.state = State::Active;
        Some(i.clone())
    }) else {
        return;
    };
    STAMP.fetch_add(1, Ordering::Relaxed);
    match save_one(&item) {
        Ok(Some(path)) => {
            if let Some(name) = &item.playlist {
                library::add_to_playlist(name, std::path::Path::new(&path));
            }
            set(item.n, State::Done(path));
        }
        // Stopped for something before it: waits its turn again, keeping what it has.
        Ok(None) => {
            with(|q| {
                if let Some(i) = q.items.iter_mut().find(|i| i.n == item.n && i.state == State::Active) {
                    i.state = State::Waiting;
                }
            });
            STAMP.fetch_add(1, Ordering::Relaxed);
        }
        Err(why) => set(item.n, State::Failed(why)),
    }
}

/// One song into the library: from the played cache if it is there, else
/// downloaded. `Ok(None)`: stopped part way, to go on later.
fn save_one(item: &Item) -> Result<Option<String>, String> {
    let t = &item.track;
    if let Some(song) = library::by_youtube(&t.id) {
        return Ok(Some(song.path.display().to_string()));
    }
    // The album's cover, once for the album: beside it if the album has one already.
    let folder = library::place_for(t, &item.meta, "opus").parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let (fbig, fsmall) = covers::folder_pair(&folder);
    let (cbig, csmall) = covers::cache_pair(&t.id);
    let cover = if fsmall.exists() {
        Some((fbig.clone(), fsmall.clone()))
    } else if csmall.exists() || covers::fetch_pair(t, &cbig, &csmall) {
        Some((cbig.clone(), csmall.clone()))
    } else {
        None
    };
    let cover_refs = cover.as_ref().map(|(b, s)| (b.as_path(), s.as_path()));
    if let Some(played) = store::cached(&t.id) {
        return library::import(t, &item.meta, &played.file(), &played.ext, cover_refs).map(|p| Some(p.display().to_string()));
    }
    if !super::on_device() {
        // The simulator: a pretend download, for the pages.
        let total = item.total.max(1);
        for step in 1..=10 {
            std::thread::sleep(Duration::from_millis(120));
            if should_stop(item.n) {
                return Ok(None);
            }
            progress(item.n, total * step / 10, Some(total));
        }
        let src = incoming().join(format!("{}.webm", store::safe_id(&t.id)));
        store::write_atomic(&src, b"demo").map_err(|e| e.to_string())?;
        let saved = library::import(t, &item.meta, &src, "webm", cover_refs);
        std::fs::remove_file(&src).ok();
        return saved.map(|p| Some(p.display().to_string()));
    }
    let slow_too = super::polite::memory_available_mb().is_some_and(|mb| mb >= 160);
    let stream = fetch::stream_for(&t.id, slow_too, &|_| {})?;
    let dir = incoming();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not save it: {e}"))?;
    let part = dir.join(format!("{}.{}.part", store::safe_id(&t.id), stream.ext));
    let mut tries = 0;
    let mut stream = stream;
    loop {
        let n = item.n;
        let total = stream.bytes;
        let mut told = |have: u64| {
            progress(n, have, total);
            !should_stop(n)
        };
        match fetch::download(&stream.url, &part, total, &mut told) {
            fetch::Dl::Done(_) => {
                let saved = library::import(t, &item.meta, &part, &stream.ext, cover_refs);
                std::fs::remove_file(&part).ok();
                return saved.map(|p| Some(p.display().to_string()));
            }
            fetch::Dl::Stopped => return Ok(None),
            fetch::Dl::Expired | fetch::Dl::Failed(_) if tries < 2 => {
                tries += 1;
                fetch::forget_stream(&t.id);
                std::thread::sleep(Duration::from_secs(2 * tries));
                if should_stop(item.n) {
                    return Ok(None);
                }
                stream = fetch::stream_for(&t.id, slow_too, &|_| {})?;
            }
            fetch::Dl::Expired => return Err("youtube stopped sending it".into()),
            fetch::Dl::Failed(why) => return Err(why),
        }
    }
}

// ------------------------------------------------- your likes, by itself ---

const LIKES_EVERY: i64 = 12 * 3600;

/// Signed in, with automatic downloads on: your likes looked at twice a
/// day over Wi-Fi, and any not saved yet added to the queue.
pub fn maybe_save_likes() {
    let c = store::choices();
    if !c.auto || !super::signed_in() || !super::polite::on_wifi() {
        return;
    }
    let mark = store::root().join("likes-saved");
    let last: i64 = std::fs::read_to_string(&mark).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    if store::now_unix() - last < LIKES_EVERY {
        return;
    }
    store::write_atomic(&mark, store::now_unix().to_string().as_bytes()).ok();
    if let Ok(likes) = super::liked_all() {
        let songs = likes.into_iter().map(|t| (t, Meta::default())).collect();
        add(songs, "your likes", Some("Your likes"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queued_song_survives_a_restart_and_half_done_waits_again() {
        let item = Item {
            n: 7,
            track: Track { id: "abc123def45".into(), title: "Glasshouse".into(), artist: "Ocean Avenue Club".into(), album: "Glasshouse".into(), seconds: Some(226), ..Track::default() },
            meta: Meta { album_artist: "Ocean Avenue Club".into(), track_no: 1, year: 2019 },
            playlist: Some("Summer".into()),
            from: "Glasshouse".into(),
            state: State::Active,
            have: 1234,
            total: 3_729_000,
        };
        let back = Item::from_json(&item.to_json()).unwrap();
        assert_eq!(back.state, State::Waiting, "at work when it stopped: waits again");
        assert_eq!((back.n, back.meta.clone(), back.playlist.clone(), back.total), (7, item.meta.clone(), item.playlist.clone(), item.total));
        let failed = Item { state: State::Failed("no internet".into()), ..item.clone() };
        assert_eq!(Item::from_json(&failed.to_json()).unwrap().state, State::Failed("no internet".into()));
        assert_eq!(estimate(&[item.track.clone(), item.track]), 2 * 226 * 16_500);
    }
}
