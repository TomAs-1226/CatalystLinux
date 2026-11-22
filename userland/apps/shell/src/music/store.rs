//! Where Music keeps songs on the card, and its choices.
//!
//! ```text
//! /data/music/
//!   settings                  the choices below, key=value
//!   cache/<id>.webm           every song played, kept to play again offline
//!   cache/<id>.webm.part      one still downloading (the player reads it as it grows)
//!   cache/<id>.jpg, -small.jpg   its album's cover, 512 and 96 px
//!   cache/played.json         what each is, and when it last played
//!   library/<Artist>/<Album>/<NN> <Title>.opus   songs saved, and your own
//!   library/<Artist>/<Album>/cover.jpg, cover-small.jpg
//!   library/Playlists/<name>.m3u8
//!   library/.index            the library's index (library.rs)
//!   downloads.json            the download queue (downloads.rs)
//!   covers/                   covers made for songs that came without one
//! ```
//!
//! The played cache has a limit (4 GB unless changed in Settings › Music):
//! past it the songs played longest ago make way. Saved songs are never
//! let go. Both are personal copies for this handheld: nothing here shares
//! or exports them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use super::Track;

/// Music's folder: /data/music on the handheld; the simulator's own beside
/// its settings.
pub fn root() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join(format!("catalyst-music-test-{}", std::process::id()))
    } else if super::on_device() {
        PathBuf::from(super::DIR)
    } else {
        crate::config_dir().join("music")
    }
}

pub fn cache_dir() -> PathBuf {
    root().join("cache")
}

pub fn library_dir() -> PathBuf {
    root().join("library")
}

pub fn covers_dir() -> PathBuf {
    root().join("covers")
}

/// A file written whole or not at all: beside it, then renamed over it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

// --------------------------------------------------------------- choices ---

/// Played cache limits offered, in GB.
pub const CACHE_LIMITS: [u32; 7] = [1, 2, 4, 8, 16, 32, 64];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choices {
    /// The played cache's limit, GB.
    pub cache_gb: u32,
    /// Downloads only over Wi-Fi (never over a computer's USB link or the
    /// robot's cable).
    pub wifi_only: bool,
    /// Downloads only while charging.
    pub charging_only: bool,
    /// Your likes (signed in) saved by themselves, over Wi-Fi.
    pub auto: bool,
    /// Every download paused.
    pub paused: bool,
    /// The line being sung, on the island's card (its lyrics asked for as
    /// each song starts).
    pub lyrics_island: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices { cache_gb: 4, wifi_only: true, charging_only: false, auto: true, paused: false, lyrics_island: true }
    }
}

impl Choices {
    pub fn to_text(self) -> String {
        format!("cache_gb={}\nwifi_only={}\ncharging_only={}\nauto={}\npaused={}\nlyrics_island={}\n", self.cache_gb, self.wifi_only, self.charging_only, self.auto, self.paused, self.lyrics_island)
    }

    pub fn from_text(text: &str) -> Choices {
        let mut c = Choices::default();
        for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
            let on = v.trim() == "true";
            match k.trim() {
                "cache_gb" => c.cache_gb = v.trim().parse().ok().filter(|g| CACHE_LIMITS.contains(g)).unwrap_or(c.cache_gb),
                "wifi_only" => c.wifi_only = on,
                "charging_only" => c.charging_only = on,
                "auto" => c.auto = on,
                "paused" => c.paused = on,
                "lyrics_island" => c.lyrics_island = on,
                _ => {}
            }
        }
        c
    }

    pub fn cache_limit(self) -> u64 {
        self.cache_gb as u64 * 1_000_000_000
    }
}

static CHOICES: Mutex<Option<Choices>> = Mutex::new(None);

pub fn choices() -> Choices {
    *CHOICES.lock().unwrap().get_or_insert_with(|| Choices::from_text(&std::fs::read_to_string(root().join("settings")).unwrap_or_default()))
}

pub fn set_choices(c: Choices) {
    *CHOICES.lock().unwrap() = Some(c);
    write_atomic(&root().join("settings"), c.to_text().as_bytes()).ok();
    super::fetch::wake();
}

// ---------------------------------------------------------- played cache ---

/// A song in the played cache.
#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub track: Track,
    /// "webm" (Opus) or "m4a".
    pub ext: String,
    pub bytes: u64,
    /// All of it is here (else it is still `.part`).
    pub complete: bool,
    /// When it last played (unix seconds), and how often.
    pub last: i64,
    pub plays: u32,
}

impl Played {
    pub fn file(&self) -> PathBuf {
        cache_dir().join(format!("{}.{}", safe_id(&self.track.id), self.ext))
    }

    fn to_json(&self) -> Value {
        let t = &self.track;
        json!([t.id, t.title, t.artist, t.album, t.seconds.unwrap_or(0), t.art, self.ext, self.bytes, self.complete, self.last, self.plays])
    }

    fn from_json(v: &Value) -> Option<Played> {
        let a = v.as_array()?;
        let s = |i: usize| a.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
        let n = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0);
        let id = s(0);
        (!id.is_empty()).then(|| Played {
            track: Track { id, title: s(1), artist: s(2), album: s(3), seconds: Some(n(4) as u32).filter(|&x| x > 0), art: s(5), local: None },
            ext: s(6),
            bytes: n(7),
            complete: a.get(8).and_then(Value::as_bool).unwrap_or(false),
            last: a.get(9).and_then(Value::as_i64).unwrap_or(0),
            plays: n(10) as u32,
        })
    }
}

/// A YouTube id as a file name: its own letters only.
pub fn safe_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

static PLAYED: Mutex<Option<Vec<Played>>> = Mutex::new(None);
static PLAYED_CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn with_played<R>(f: impl FnOnce(&mut Vec<Played>) -> R) -> R {
    let mut p = PLAYED.lock().unwrap();
    let list = p.get_or_insert_with(|| {
        let text = std::fs::read_to_string(cache_dir().join("played.json")).unwrap_or_default();
        serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_array().map(|a| a.iter().filter_map(Played::from_json).collect())).unwrap_or_default()
    });
    f(list)
}

fn save_played(list: &[Played]) {
    let v = Value::Array(list.iter().map(Played::to_json).collect());
    write_atomic(&cache_dir().join("played.json"), v.to_string().as_bytes()).ok();
    PLAYED_CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Counts changes, for pages showing the played songs.
pub fn played_stamp() -> u64 {
    PLAYED_CHANGED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Songs played and kept, the latest first (only those all here).
pub fn played() -> Vec<Played> {
    let mut list = with_played(|l| l.iter().filter(|p| p.complete).cloned().collect::<Vec<_>>());
    list.sort_by(|a, b| b.last.cmp(&a.last));
    list
}

/// A song's file in the cache, if all of it is here.
pub fn cached(id: &str) -> Option<Played> {
    with_played(|l| l.iter().find(|p| p.track.id == id && p.complete).cloned()).filter(|p| p.file().exists())
}

/// A song part downloaded: its file and how much is here.
pub fn partial(id: &str) -> Option<(Played, u64)> {
    let p = with_played(|l| l.iter().find(|p| p.track.id == id && !p.complete).cloned())?;
    let part = PathBuf::from(format!("{}.part", p.file().display()));
    let have = std::fs::metadata(&part).map(|m| m.len()).ok()?;
    Some((p, have))
}

/// A song's entry: begun (or finished) downloading.
pub fn note(track: &Track, ext: &str, bytes: u64, complete: bool) {
    with_played(|l| {
        let last = now_unix();
        match l.iter_mut().find(|p| p.track.id == track.id) {
            Some(p) => {
                p.ext = ext.to_owned();
                p.bytes = bytes;
                p.complete = complete;
                if !track.title.is_empty() {
                    p.track = Track { local: None, ..track.clone() };
                }
            }
            None => l.push(Played { track: Track { local: None, ..track.clone() }, ext: ext.to_owned(), bytes, complete, last, plays: 0 }),
        }
        save_played(l);
    });
}

/// A song began playing: the latest played now.
pub fn played_now(id: &str) {
    with_played(|l| {
        if let Some(p) = l.iter_mut().find(|p| p.track.id == id) {
            p.last = now_unix();
            p.plays += 1;
            save_played(l);
        }
    });
}

/// The cache's size: what is downloaded, part-downloaded songs too.
pub fn cache_bytes() -> u64 {
    with_played(|l| l.iter().map(|p| p.bytes).sum())
}

pub fn cache_count() -> usize {
    with_played(|l| l.iter().filter(|p| p.complete).count())
}

/// The songs to let go so the cache fits `limit`: those played longest
/// ago first, never one in `keep` (playing, or coming up).
pub fn to_evict(list: &[Played], limit: u64, keep: &[String]) -> Vec<String> {
    let mut total: u64 = list.iter().map(|p| p.bytes).sum();
    let mut order: Vec<&Played> = list.iter().filter(|p| !keep.contains(&p.track.id)).collect();
    order.sort_by_key(|p| p.last);
    let mut out = Vec::new();
    for p in order {
        if total <= limit {
            break;
        }
        total = total.saturating_sub(p.bytes);
        out.push(p.track.id.clone());
    }
    out
}

fn remove_files(p: &Played) {
    let file = p.file();
    std::fs::remove_file(&file).ok();
    std::fs::remove_file(format!("{}.part", file.display())).ok();
    let id = safe_id(&p.track.id);
    std::fs::remove_file(cache_dir().join(format!("{id}.jpg"))).ok();
    std::fs::remove_file(cache_dir().join(format!("{id}-small.jpg"))).ok();
    super::lyrics::forget_cached(&p.track.id);
}

/// Past the limit: the songs played longest ago let go.
pub fn evict(keep: &[String]) -> usize {
    let limit = choices().cache_limit();
    with_played(|l| {
        let gone = to_evict(l, limit, keep);
        if gone.is_empty() {
            return 0;
        }
        for p in l.iter().filter(|p| gone.contains(&p.track.id)) {
            remove_files(p);
        }
        l.retain(|p| !gone.contains(&p.track.id));
        save_played(l);
        gone.len()
    })
}

/// Settings › Music › Clear played cache: all of it but `keep`.
pub fn clear_cache(keep: &[String]) -> u64 {
    with_played(|l| {
        let freed = l.iter().filter(|p| !keep.contains(&p.track.id)).map(|p| p.bytes).sum();
        for p in l.iter().filter(|p| !keep.contains(&p.track.id)) {
            remove_files(p);
        }
        l.retain(|p| keep.contains(&p.track.id));
        save_played(l);
        freed
    })
}

/// "1.2 GB", "340 MB", "12 KB".
pub fn size_words(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 10_000_000 => format!("{} MB", b / 1_000_000),
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1e6),
        b => format!("{} KB", (b / 1000).max(if b > 0 { 1 } else { 0 })),
    }
}

/// A song's likely size from its length: YouTube's Opus averages about
/// 130 kbit/s (16.5 KB a second).
pub fn estimate_bytes(seconds: Option<u32>) -> u64 {
    seconds.unwrap_or(210) as u64 * 16_500
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, bytes: u64, last: i64) -> Played {
        Played { track: Track { id: id.into(), ..Track::default() }, ext: "webm".into(), bytes, complete: true, last, plays: 1 }
    }

    #[test]
    fn past_the_limit_the_songs_played_longest_ago_go_first_never_those_playing() {
        let list = [p("a", 400, 10), p("b", 400, 30), p("c", 400, 20), p("d", 400, 40)];
        assert!(to_evict(&list, 2000, &[]).is_empty(), "under the limit: nothing");
        assert_eq!(to_evict(&list, 1000, &[]), ["a", "c"]);
        // The oldest is playing: the next oldest instead.
        assert_eq!(to_evict(&list, 1000, &["a".into()]), ["c", "b"]);
    }

    #[test]
    fn choices_and_entries_survive_a_restart() {
        let c = Choices { cache_gb: 16, wifi_only: false, charging_only: true, auto: false, paused: true, lyrics_island: false };
        assert_eq!(Choices::from_text(&c.to_text()), c);
        assert_eq!(Choices::from_text("cache_gb=3\n"), Choices::default(), "a limit not offered: the default");
        let mut e = p("xukbqwRuN5w", 4_115_008, 1_700_000_000);
        e.track.title = "Choosin' Texas".into();
        e.track.seconds = Some(232);
        assert_eq!(Played::from_json(&e.to_json()), Some(e));
        assert_eq!(safe_id("ab/../c d"), "ab____c_d");
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!((size_words(1_234_000_000), size_words(340_000_000), size_words(4_100_000), size_words(1500), size_words(0)), ("1.2 GB".into(), "340 MB".into(), "4.1 MB".into(), "1 KB".into(), "0 KB".into()));
        assert_eq!(estimate_bytes(Some(200)), 3_300_000);
    }
}
