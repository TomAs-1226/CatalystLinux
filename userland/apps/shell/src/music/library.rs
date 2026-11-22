//! The library: songs saved from YouTube Music and your own, in
//! `/data/music/library/<Artist>/<Album>/<NN> <Title>.<ext>` with each
//! album's `cover.jpg` and `cover-small.jpg`, and playlists as `.m3u8`
//! files in `Playlists/`. Bring your own (MP3, FLAC, M4A/AAC, Ogg, Opus,
//! WAV) over SSH (`scp -r "Album" root@<handheld>:/data/music/library/Artist/`)
//! or with Files; the layout is only a habit, tags win where a file has them.
//!
//! **The index** (`library/.index`, a few hundred KB for thousands of
//! songs) holds every song's tags with its file's time and size, and every
//! folder's time with its subfolders. It is read the first time something
//! asks, and brought up to date once a run in the background: a folder
//! whose time has not moved is not listed again (adding, removing or
//! renaming a file moves it), and only a file new or changed since is
//! opened (by `catalyst-play --probe`, at low priority). A file retagged in
//! place without its folder changing is found by Settings › Music › Look
//! for new music, which looks at every file's time once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::polite::{self, Manner};
use super::{covers, store, Track};

/// What the player and the index read.
pub const AUDIO: [&str; 10] = ["mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "webm", "mka"];
pub const PLAYER: &str = "/usr/bin/catalyst-play";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub path: PathBuf,
    /// Its place under the library, with `/` between folders.
    pub rel: String,
    pub mtime: i64,
    pub size: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: u32,
    pub year: u32,
    pub seconds: u32,
    /// Saved from YouTube Music: the song's id there.
    pub youtube: String,
}

impl Song {
    /// As the player and the pages take a song.
    pub fn track(&self) -> Track {
        Track {
            id: if self.youtube.is_empty() { format!("lib:{}", self.rel) } else { self.youtube.clone() },
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            seconds: Some(self.seconds).filter(|&s| s > 0),
            art: String::new(),
            local: Some(self.path.clone()),
        }
    }

    pub fn album_artist_or_artist(&self) -> String {
        if self.album_artist.is_empty() {
            self.artist.clone()
        } else {
            self.album_artist.clone()
        }
    }

    fn folder(&self) -> &str {
        self.rel.rsplit_once('/').map_or("", |(d, _)| d)
    }

    fn to_json(&self) -> Value {
        json!([self.rel, self.mtime, self.size, self.title, self.artist, self.album, self.album_artist, self.track, self.year, self.seconds, self.youtube])
    }

    fn from_json(root: &Path, v: &Value) -> Option<Song> {
        let a = v.as_array()?;
        let s = |i: usize| a.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
        let n = |i: usize| a.get(i).and_then(Value::as_i64).unwrap_or(0);
        let rel = s(0);
        (!rel.is_empty()).then(|| Song {
            path: root.join(&rel),
            rel,
            mtime: n(1),
            size: n(2) as u64,
            title: s(3),
            artist: s(4),
            album: s(5),
            album_artist: s(6),
            track: n(7) as u32,
            year: n(8) as u32,
            seconds: n(9) as u32,
            youtube: s(10),
        })
    }
}

/// A song's tags as read from its file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probed {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: u32,
    pub year: u32,
    pub seconds: u32,
    pub youtube: String,
    /// Its embedded cover, written to this file.
    pub picture: Option<PathBuf>,
}

impl Probed {
    pub fn parse(text: &str) -> Probed {
        let mut p = Probed::default();
        for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
            let v = v.trim().to_owned();
            match k {
                "title" => p.title = v,
                "artist" => p.artist = v,
                "album" => p.album = v,
                "album_artist" => p.album_artist = v,
                "youtube" => p.youtube = v,
                "track" => p.track = v.parse().unwrap_or(0),
                "year" => p.year = v.parse().unwrap_or(0),
                "seconds" => p.seconds = v.parse::<f32>().map(|s| s.round() as u32).unwrap_or(0),
                _ => {}
            }
        }
        p
    }
}

/// A song's tags, from the player tool (on the handheld).
pub fn probe(path: &Path) -> Option<Probed> {
    if !Path::new(PLAYER).exists() {
        return None;
    }
    let picture = super::private_file("picture");
    let mut cmd = Command::new(PLAYER);
    cmd.arg("--probe").arg(path).arg("--picture-out").arg(&picture);
    let out = polite::output_within(polite::polite(&mut cmd, Manner::Background), Duration::from_secs(20)).ok()?;
    if !out.status.success() {
        std::fs::remove_file(&picture).ok();
        return None;
    }
    let mut p = Probed::parse(&String::from_utf8_lossy(&out.stdout));
    p.picture = picture.exists().then_some(picture);
    Some(p)
}

/// "03 Copper Season.flac" → (3, "Copper Season").
fn number_and_title(stem: &str) -> (u32, String) {
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    let rest = stem[digits.len()..].trim_start_matches([' ', '.', '-', '_']).trim();
    match digits.parse::<u32>() {
        Ok(n) if !rest.is_empty() && digits.len() <= 3 => (n, rest.to_owned()),
        _ => (0, stem.trim().to_owned()),
    }
}

/// A song as its place says it, where its tags are silent.
pub fn from_path(rel: &str, tags: Option<Probed>) -> Song {
    let parts: Vec<&str> = rel.split('/').collect();
    let file = parts.last().copied().unwrap_or_default();
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let (n, title) = number_and_title(stem);
    let (artist, album) = match parts.len() {
        0..=1 => (String::new(), String::new()),
        2 => (String::new(), parts[0].to_owned()),
        _ => (parts[parts.len() - 3].to_owned(), parts[parts.len() - 2].to_owned()),
    };
    let t = tags.unwrap_or_default();
    let or = |a: String, b: String| if a.is_empty() { b } else { a };
    Song {
        rel: rel.to_owned(),
        title: or(t.title, title),
        artist: or(t.artist, artist.clone()),
        album: or(t.album, album),
        album_artist: or(t.album_artist, artist),
        track: if t.track > 0 { t.track } else { n },
        year: t.year,
        seconds: t.seconds,
        youtube: t.youtube,
        ..Song::default()
    }
}

// ----------------------------------------------------------------- index ---

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub songs: Vec<Song>,
    /// Each folder's time, and its subfolders.
    dirs: HashMap<String, (i64, Vec<String>)>,
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

fn is_audio(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
}

impl Index {
    pub fn to_json(&self) -> Value {
        let dirs: serde_json::Map<String, Value> = self.dirs.iter().map(|(k, (t, subs))| (k.clone(), json!([t, subs]))).collect();
        json!({"v": 1, "dirs": dirs, "songs": self.songs.iter().map(Song::to_json).collect::<Vec<_>>()})
    }

    pub fn from_json(root: &Path, v: &Value) -> Index {
        let songs = v["songs"].as_array().map(|a| a.iter().filter_map(|s| Song::from_json(root, s)).collect()).unwrap_or_default();
        let dirs = v["dirs"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, d)| {
                        let t = d.get(0)?.as_i64()?;
                        let subs = d.get(1)?.as_array()?.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect();
                        Some((k.clone(), (t, subs)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Index { songs, dirs }
    }

    /// Brought up to date with the folders under `root`. `every_file`: each
    /// file's time looked at too, folders unchanged or not. `probe` reads a
    /// new or changed file's tags.
    pub fn refreshed(&self, root: &Path, every_file: bool, probe: &mut dyn FnMut(&Path) -> Option<Probed>) -> Index {
        let mut by_folder: HashMap<&str, Vec<&Song>> = HashMap::new();
        for s in &self.songs {
            by_folder.entry(s.folder()).or_default().push(s);
        }
        let mut out = Index::default();
        let mut stack = vec![String::new()];
        while let Some(rel) = stack.pop() {
            let full = if rel.is_empty() { root.to_path_buf() } else { root.join(&rel) };
            let Ok(meta) = std::fs::metadata(&full) else { continue };
            let t = mtime(&meta);
            match self.dirs.get(&rel) {
                // Unchanged: its songs as they were, its subfolders looked into.
                Some((was, subs)) if *was == t && !every_file => {
                    out.songs.extend(by_folder.get(rel.as_str()).into_iter().flatten().map(|s| (*s).clone()));
                    stack.extend(subs.iter().cloned());
                    out.dirs.insert(rel, (t, subs.clone()));
                }
                _ => {
                    let mut subs = Vec::new();
                    let mut new_here = false;
                    let mut entries: Vec<_> = std::fs::read_dir(&full).into_iter().flatten().flatten().collect();
                    entries.sort_by_key(|e| e.file_name());
                    for e in entries {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            continue;
                        }
                        let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                        let Ok(m) = e.metadata() else { continue };
                        if m.is_dir() {
                            subs.push(child);
                        } else if is_audio(&name) {
                            let (ft, size) = (mtime(&m), m.len());
                            let known = by_folder.get(rel.as_str()).and_then(|v| v.iter().find(|s| s.rel == child && s.mtime == ft && s.size == size));
                            match known {
                                Some(s) => out.songs.push((*s).clone()),
                                None => {
                                    let path = root.join(&child);
                                    let tags = probe(&path);
                                    let picture = tags.as_ref().and_then(|t| t.picture.clone());
                                    let mut song = from_path(&child, tags);
                                    song.path = path;
                                    song.mtime = ft;
                                    song.size = size;
                                    if let Some(pic) = picture {
                                        give_folder_a_cover(&full, Some(&pic));
                                        std::fs::remove_file(pic).ok();
                                    }
                                    out.songs.push(song);
                                    new_here = true;
                                }
                            }
                        }
                    }
                    if new_here {
                        give_folder_a_cover(&full, None);
                    }
                    stack.extend(subs.iter().cloned());
                    out.dirs.insert(rel, (t, subs));
                }
            }
        }
        out.songs.sort_by(|a, b| a.rel.cmp(&b.rel));
        out
    }
}

/// An album folder without its small cover gets one: from the folder's own
/// picture (cover.jpg, folder.jpg, front.png…), else from the cover inside
/// a song (`embedded`), written as cover.jpg too. None at all: one is drawn
/// when it is shown, apart from the folder.
fn give_folder_a_cover(dir: &Path, embedded: Option<&Path>) {
    let (big, small) = covers::folder_pair(dir);
    if small.exists() {
        return;
    }
    let image = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
        (name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".png")) && !name.starts_with("cover-small")
    });
    let picture = image.as_deref().or(embedded).and_then(|p| std::fs::read(p).ok()).and_then(|b| covers::decode_bytes(&b));
    if let Some(p) = picture {
        if image.is_some() {
            covers::write_jpeg(&covers::square(&p, covers::SMALL_PX), &small);
        } else {
            covers::write_pair(&p, &big, &small);
        }
    }
}

/// An album folder's small cover, or the one drawn for it.
pub fn small_cover(song_path: &Path) -> Option<PathBuf> {
    let dir = song_path.parent()?;
    let (_, small) = covers::folder_pair(dir);
    if small.exists() {
        return Some(small);
    }
    let s = song_for_path(song_path)?;
    Some(covers::ensure_made(&s.album, &s.album_artist_or_artist()))
}

// ------------------------------------------------------- the one index ---

static INDEX: Mutex<Option<Arc<Index>>> = Mutex::new(None);
static STAMP: AtomicU64 = AtomicU64::new(0);
static REFRESHING: AtomicBool = AtomicBool::new(false);
static REFRESHED_THIS_RUN: AtomicBool = AtomicBool::new(false);

fn index_file() -> PathBuf {
    store::library_dir().join(".index")
}

fn index() -> Arc<Index> {
    let idx = {
        let mut slot = INDEX.lock().unwrap();
        slot.get_or_insert_with(|| {
            let root = store::library_dir();
            let v = std::fs::read_to_string(index_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
            Arc::new(match v {
                Some(v) => Index::from_json(&root, &v),
                None if super::on_device() => Index::default(),
                None => demo_index(),
            })
        })
        .clone()
    };
    if !REFRESHED_THIS_RUN.swap(true, Ordering::Relaxed) && super::on_device() {
        refresh(false);
    }
    idx
}

fn set_index(idx: Index) {
    if super::on_device() {
        store::write_atomic(&index_file(), idx.to_json().to_string().as_bytes()).ok();
    }
    *INDEX.lock().unwrap() = Some(Arc::new(idx));
    STAMP.fetch_add(1, Ordering::Relaxed);
}

/// Counts changes to the library, for pages showing it.
pub fn stamp() -> u64 {
    STAMP.load(Ordering::Relaxed)
}

pub fn refreshing() -> bool {
    REFRESHING.load(Ordering::Relaxed)
}

/// The index brought up to date in the background (`every_file`: Settings'
/// look for new music).
pub fn refresh(every_file: bool) {
    if REFRESHING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::Builder::new()
        .name("music-library".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            polite::background_thread();
            let root = store::library_dir();
            std::fs::create_dir_all(&root).ok();
            let old = INDEX.lock().unwrap().clone().unwrap_or_default();
            let new = old.refreshed(&root, every_file, &mut probe);
            if new.songs != old.songs || new.dirs != old.dirs {
                set_index(new);
            }
            REFRESHING.store(false, Ordering::Relaxed);
            STAMP.fetch_add(1, Ordering::Relaxed);
        })
        .ok();
}

/// Every song, by artist, album and track.
pub fn songs() -> Vec<Song> {
    let mut all = index().songs.clone();
    all.sort_by(|a, b| sort_key(&a.album_artist_or_artist()).cmp(&sort_key(&b.album_artist_or_artist())).then(a.album.cmp(&b.album)).then(a.track.cmp(&b.track)).then(a.title.cmp(&b.title)));
    all
}

/// "The Northfields" sorts as "northfields".
fn sort_key(name: &str) -> String {
    let l = name.to_lowercase();
    l.strip_prefix("the ").map(str::to_owned).unwrap_or(l)
}

pub fn count() -> usize {
    index().songs.len()
}

/// What the library takes on the card, and how many songs.
pub fn bytes() -> (u64, usize) {
    let idx = index();
    (idx.songs.iter().map(|s| s.size).sum(), idx.songs.len())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Album {
    pub name: String,
    pub artist: String,
    pub year: u32,
    pub songs: Vec<Song>,
}

/// Albums (of one artist, or all), by name.
pub fn albums(artist: Option<&str>) -> Vec<Album> {
    let mut map: HashMap<(String, String), Album> = HashMap::new();
    for s in songs() {
        let a = s.album_artist_or_artist();
        if artist.is_some_and(|want| want != a) {
            continue;
        }
        let album = if s.album.is_empty() { "Singles".to_owned() } else { s.album.clone() };
        let e = map.entry((a.clone(), album.clone())).or_insert_with(|| Album { name: album, artist: a, year: 0, songs: Vec::new() });
        e.year = e.year.max(s.year);
        e.songs.push(s);
    }
    let mut out: Vec<Album> = map.into_values().collect();
    for a in &mut out {
        a.songs.sort_by(|x, y| x.track.cmp(&y.track).then(x.title.cmp(&y.title)));
    }
    out.sort_by(|a, b| sort_key(&a.name).cmp(&sort_key(&b.name)).then(a.artist.cmp(&b.artist)));
    out
}

/// Artists, by name, with how many albums and songs.
pub fn artists() -> Vec<(String, usize, usize)> {
    let mut map: HashMap<String, (usize, std::collections::HashSet<String>)> = HashMap::new();
    for s in index().songs.iter() {
        let e = map.entry(s.album_artist_or_artist()).or_default();
        e.0 += 1;
        e.1.insert(s.album.clone());
    }
    let mut out: Vec<(String, usize, usize)> = map.into_iter().filter(|(a, _)| !a.is_empty()).map(|(a, (n, albums))| (a, albums.len(), n)).collect();
    out.sort_by(|a, b| sort_key(&a.0).cmp(&sort_key(&b.0)));
    out
}

/// Playlists: `.m3u8` (or `.m3u`) files in `Playlists/`, their songs found
/// in the library.
pub fn playlists() -> Vec<(String, Vec<Song>)> {
    let root = store::library_dir();
    let dir = root.join("Playlists");
    let idx = index();
    let mut out: Vec<(String, Vec<Song>)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_owned();
            let stem = name.strip_suffix(".m3u8").or_else(|| name.strip_suffix(".m3u"))?.to_owned();
            let text = std::fs::read_to_string(&p).ok()?;
            let songs = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(|l| {
                    let path = if Path::new(l).is_absolute() { PathBuf::from(l) } else { dir.join(l) };
                    let path = normalise(&path);
                    idx.songs.iter().find(|s| s.path == path).cloned()
                })
                .collect();
            Some((stem, songs))
        })
        .collect();
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// `a/b/../c` as `a/c`, without asking the disk.
fn normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A song added to a playlist's file (made if it is new).
pub fn add_to_playlist(name: &str, song: &Path) {
    let dir = store::library_dir().join("Playlists");
    std::fs::create_dir_all(&dir).ok();
    let file = dir.join(format!("{}.m3u8", safe_name(name)));
    let line = song.strip_prefix(store::library_dir()).map(|r| format!("../{}", r.display().to_string().replace('\\', "/"))).unwrap_or_else(|_| song.display().to_string());
    let mut text = std::fs::read_to_string(&file).unwrap_or_else(|_| "#EXTM3U\n".into());
    if !text.lines().any(|l| l.trim() == line) {
        text.push_str(&line);
        text.push('\n');
        store::write_atomic(&file, text.as_bytes()).ok();
    }
}

/// A library song saved from YouTube Music, by its id there.
pub fn by_youtube(id: &str) -> Option<Song> {
    if id.is_empty() || id.starts_with("lib:") {
        return None;
    }
    let idx = INDEX.lock().unwrap().clone()?;
    idx.songs.iter().find(|s| s.youtube == id).cloned().filter(|s| s.path.exists())
}

pub fn song_for_path(path: &Path) -> Option<Song> {
    let idx = INDEX.lock().unwrap().clone()?;
    idx.songs.iter().find(|s| s.path == path).cloned()
}

pub fn song_for(track: &Track) -> Option<Song> {
    track.local.as_deref().and_then(song_for_path)
}

/// Songs whose name, artist or album hold every word of `query`.
pub fn search(query: &str) -> Vec<Track> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    songs()
        .into_iter()
        .filter(|s| {
            let hay = format!("{} {} {} {}", s.title, s.artist, s.album, s.album_artist).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|s| s.track())
        .collect()
}

// ---------------------------------------------------------------- saving ---

/// A name as a file or folder name: no slashes, nothing Windows refuses
/// (the card is read from computers too), not too long.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let t = cleaned.trim().trim_matches('.').trim();
    let t: String = t.chars().take(80).collect();
    if t.is_empty() {
        "Unknown".into()
    } else {
        t
    }
}

/// What a saved song is filed under, besides its own tags.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// The album's artist (a collection says it); else the song's first.
    pub album_artist: String,
    pub track_no: u32,
    pub year: u32,
}

/// Where a song is saved: `<Artist>/<Album>/<NN> <Title>.<ext>`.
pub fn place_for(track: &Track, meta: &Meta, ext: &str) -> PathBuf {
    let artist = if meta.album_artist.is_empty() { track.artist.split(", ").next().unwrap_or_default().to_owned() } else { meta.album_artist.clone() };
    let album = if track.album.is_empty() { "Singles".to_owned() } else { track.album.clone() };
    let file = if meta.track_no > 0 { format!("{:02} {}", meta.track_no, track.title) } else { track.title.clone() };
    store::library_dir().join(safe_name(&artist)).join(safe_name(&album)).join(format!("{}.{ext}", safe_name(&file)))
}

/// A song saved into the library from `src` (a played copy, or a
/// download), tagged, its album's cover beside it and inside it. Opus goes
/// into Ogg Opus as it is (no re-encoding); AAC stays M4A.
pub fn import(track: &Track, meta: &Meta, src: &Path, ext: &str, cover: Option<(&Path, &Path)>) -> Result<PathBuf, String> {
    let out_ext = if ext == "m4a" { "m4a" } else { "opus" };
    let dest = place_for(track, meta, out_ext);
    let dir = dest.parent().ok_or("no folder")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the album's folder: {e}"))?;
    // The album's covers, once for the album.
    let (big, small) = covers::folder_pair(&dir);
    if !small.exists() {
        if let Some((b, s)) = cover {
            std::fs::copy(b, &big).ok();
            std::fs::copy(s, &small).ok();
        }
    }
    if !dest.exists() {
        let artist = if meta.album_artist.is_empty() { track.artist.clone() } else { meta.album_artist.clone() };
        let mut tags = vec![("TITLE", track.title.clone()), ("ARTIST", track.artist.clone()), ("ALBUM", track.album.clone()), ("ALBUMARTIST", artist), ("COMMENT", format!("youtube:{}", track.id))];
        if meta.track_no > 0 {
            tags.push(("TRACKNUMBER", meta.track_no.to_string()));
        }
        if meta.year > 0 {
            tags.push(("DATE", meta.year.to_string()));
        }
        tags.retain(|(_, v)| !v.is_empty());
        let tmp = dir.join(format!(".{}.tmp", std::process::id()));
        let written = if !super::on_device() {
            std::fs::copy(src, &tmp).map(|_| ()).map_err(|e| e.to_string())
        } else if out_ext == "opus" {
            remux_opus(src, &tmp, &tags, big.exists().then_some(big.as_path()))
        } else {
            tag_m4a(src, &tmp, &tags, big.exists().then_some(big.as_path()))
        };
        if let Err(why) = written {
            std::fs::remove_file(&tmp).ok();
            return Err(why);
        }
        std::fs::rename(&tmp, &dest).map_err(|e| format!("could not save it: {e}"))?;
    }
    // Into the index now (its tags are known), no need to read it again.
    let rel = dest.strip_prefix(store::library_dir()).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let meta_now = std::fs::metadata(&dest).ok();
    let song = Song {
        path: dest.clone(),
        rel,
        mtime: meta_now.as_ref().map_or(0, mtime),
        size: meta_now.as_ref().map_or(0, |m| m.len()),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_artist: if meta.album_artist.is_empty() { track.artist.split(", ").next().unwrap_or_default().to_owned() } else { meta.album_artist.clone() },
        track: meta.track_no,
        year: meta.year,
        seconds: track.seconds.unwrap_or(0),
        youtube: track.id.clone(),
    };
    add(song, &dir);
    Ok(dest)
}

/// A song added to the index as saved (its folder's time noted, so the
/// next look does not open it again).
fn add(song: Song, dir: &Path) {
    let mut idx = (*index()).clone();
    idx.songs.retain(|s| s.rel != song.rel);
    idx.songs.push(song);
    let root = store::library_dir();
    // Each folder from the library down to the album's, noted with its time.
    let mut rel = String::new();
    let mut parent: Option<String> = Some(String::new());
    for part in dir.strip_prefix(&root).map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default() {
        let child = if rel.is_empty() { part.clone() } else { format!("{rel}/{part}") };
        if let Some(p) = parent.take() {
            let full = if p.is_empty() { root.clone() } else { root.join(&p) };
            let t = std::fs::metadata(&full).map(|m| mtime(&m)).unwrap_or(0);
            let e = idx.dirs.entry(p).or_insert((t, Vec::new()));
            e.0 = t;
            if !e.1.contains(&child) {
                e.1.push(child.clone());
            }
        }
        rel = child;
        parent = Some(rel.clone());
    }
    if let Some(p) = parent {
        let t = std::fs::metadata(root.join(&p)).map(|m| mtime(&m)).unwrap_or(0);
        idx.dirs.entry(p).or_insert((t, Vec::new())).0 = t;
    }
    set_index(idx);
}

fn tags_file(tags: &[(&str, String)]) -> Result<PathBuf, String> {
    let file = super::private_file("tags");
    let text: String = tags.iter().map(|(k, v)| format!("{k}={}\n", v.replace(['\n', '\r'], " "))).collect();
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

fn remux_opus(src: &Path, to: &Path, tags: &[(&str, String)], cover: Option<&Path>) -> Result<(), String> {
    let file = tags_file(tags)?;
    let mut cmd = Command::new(PLAYER);
    cmd.arg("--remux").arg(src).arg(to).arg("--tags").arg(&file);
    if let Some(c) = cover {
        cmd.arg("--picture").arg(c);
    }
    let out = polite::output_within(polite::polite(&mut cmd, Manner::Background), Duration::from_secs(120));
    std::fs::remove_file(&file).ok();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!("could not save it: {}", String::from_utf8_lossy(&o.stderr).trim().trim_start_matches("catalyst-play: "))),
        Err(why) => Err(format!("could not save it: {why}")),
    }
}

/// AAC kept as M4A, tagged by ffmpeg (stream copy), its cover inside if
/// ffmpeg takes it.
fn tag_m4a(src: &Path, to: &Path, tags: &[(&str, String)], cover: Option<&Path>) -> Result<(), String> {
    let names = |k: &str| match k {
        "TITLE" => "title",
        "ARTIST" => "artist",
        "ALBUM" => "album",
        "ALBUMARTIST" => "album_artist",
        "TRACKNUMBER" => "track",
        "DATE" => "date",
        _ => "comment",
    };
    let run = |with_cover: bool| {
        let mut cmd = Command::new(super::FFMPEG);
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(src);
        if let (true, Some(c)) = (with_cover, cover) {
            cmd.arg("-i").arg(c).args(["-map", "0:a", "-map", "1:v", "-disposition:v:0", "attached_pic"]);
        } else {
            cmd.args(["-map", "0:a"]);
        }
        cmd.args(["-c", "copy", "-map_metadata", "-1"]);
        for (k, v) in tags {
            cmd.arg("-metadata").arg(format!("{}={v}", names(k)));
        }
        cmd.args(["-f", "ipod"]).arg(to);
        polite::output_within(polite::polite(&mut cmd, Manner::Background), Duration::from_secs(120)).is_ok_and(|o| o.status.success())
    };
    if run(true) || run(false) {
        Ok(())
    } else {
        // Untagged, as it came: better than not saved.
        std::fs::copy(src, to).map(|_| ()).map_err(|e| format!("could not save it: {e}"))
    }
}

// ------------------------------------------------------------------ demo ---

/// The simulator's library: two of the made-up albums, as if saved.
fn demo_index() -> Index {
    let root = store::library_dir();
    let mut songs = Vec::new();
    for (i, t) in super::demo_catalogue().into_iter().enumerate() {
        if !matches!(t.album.as_str(), "Low Water" | "Orbit Songs" | "Night Bus" | "Copper Season") {
            continue;
        }
        let rel = format!("{}/{}/{:02} {}.opus", t.artist, t.album, i % 4 + 1, t.title);
        songs.push(Song { path: root.join(&rel), rel, mtime: 0, size: 3_400_000 + i as u64 * 91_000, title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), album_artist: t.artist.clone(), track: (i % 4 + 1) as u32, year: 2024, seconds: t.seconds.unwrap_or(0), youtube: t.id.clone() });
    }
    Index { songs, dirs: HashMap::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_songs_place_says_what_its_tags_do_not() {
        let s = from_path("Wren & the Tide/Low Water/03 Static Bloom.flac", None);
        assert_eq!((s.artist.as_str(), s.album.as_str(), s.track, s.title.as_str()), ("Wren & the Tide", "Low Water", 3, "Static Bloom"));
        let tags = Probed { title: "Static Bloom (Live)".into(), artist: "Wren".into(), track: 7, ..Probed::default() };
        let s = from_path("Wren & the Tide/Low Water/03 Static Bloom.flac", Some(tags));
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.album.as_str(), s.track), ("Static Bloom (Live)", "Wren", "Low Water", 7), "tags win");
        assert_eq!(from_path("loose.mp3", None).title, "loose");
        assert_eq!(number_and_title("1999"), (0, "1999".into()), "a title that is a year");
        assert_eq!(number_and_title("12 - Paper Satellites"), (12, "Paper Satellites".into()));
    }

    #[test]
    fn the_player_tools_tags_are_read() {
        let p = Probed::parse("title=Choosin' Texas\nartist=Ella Langley\nalbum=Dandelion\nalbum_artist=\nyoutube=xukbqwRuN5w\ntrack=3\nyear=2025\nseconds=231.8\ncodec=opus\nrate=48000\npicture=image/jpeg\n");
        assert_eq!((p.title.as_str(), p.track, p.year, p.seconds, p.youtube.as_str()), ("Choosin' Texas", 3, 2025, 232, "xukbqwRuN5w"));
    }

    #[test]
    fn saved_songs_are_filed_by_album_artist_album_and_number() {
        let t = Track { id: "abc".into(), title: "Kites: Over/Fremont?".into(), artist: "Hollis Gray, Mara Ellis".into(), album: "Fremont".into(), ..Track::default() };
        let p = place_for(&t, &Meta { track_no: 4, ..Meta::default() }, "opus");
        assert!(p.ends_with("Hollis Gray/Fremont/04 Kites_ Over_Fremont_.opus"), "{}", p.display());
        let single = Track { album: String::new(), ..t.clone() };
        assert!(place_for(&single, &Meta { album_artist: "Various".into(), ..Meta::default() }, "m4a").ends_with("Various/Singles/Kites_ Over_Fremont_.m4a"));
        assert_eq!((safe_name("  ..  "), safe_name("AC/DC")), ("Unknown".into(), "AC_DC".into()));
    }

    #[test]
    fn the_index_opens_only_what_is_new_and_keeps_what_has_not_moved() {
        let root = store::root().join("library-index-test");
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("Mara Ellis/Night Bus")).unwrap();
        std::fs::create_dir_all(root.join("Juniper Vale/Orbit Songs")).unwrap();
        std::fs::write(root.join("Mara Ellis/Night Bus/01 Slow Signal.mp3"), b"x").unwrap();
        std::fs::write(root.join("Mara Ellis/Night Bus/02 Afterglow Avenue.flac"), b"y").unwrap();
        std::fs::write(root.join("Juniper Vale/Orbit Songs/01 Paper Satellites.opus"), b"z").unwrap();
        std::fs::write(root.join("Juniper Vale/Orbit Songs/notes.txt"), b"not a song").unwrap();
        let mut opened = Vec::new();
        let mut probe = |p: &Path| {
            opened.push(p.file_name().unwrap().to_string_lossy().into_owned());
            Some(Probed { seconds: 200, ..Probed::default() })
        };
        let first = Index::default().refreshed(&root, false, &mut probe);
        assert_eq!(first.songs.len(), 3);
        assert_eq!(opened.len(), 3, "each song opened once");
        assert!(first.songs.iter().all(|s| s.seconds == 200));
        // Through its file and back: the same.
        let back = Index::from_json(&root, &first.to_json());
        assert_eq!(back.songs, first.songs);
        // Nothing changed: nothing opened.
        opened.clear();
        let mut probe = |p: &Path| {
            opened.push(p.file_name().unwrap().to_string_lossy().into_owned());
            None
        };
        let second = back.refreshed(&root, false, &mut probe);
        assert_eq!(second.songs, first.songs);
        assert!(opened.is_empty(), "{opened:?}");
        // A song added: only it is opened; one removed is gone.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(root.join("Mara Ellis/Night Bus/03 Harbour Lights.ogg"), b"w").unwrap();
        std::fs::remove_file(root.join("Juniper Vale/Orbit Songs/01 Paper Satellites.opus")).unwrap();
        let mut seen = Vec::new();
        let third = second.refreshed(&root, false, &mut |p: &Path| {
            seen.push(p.file_name().unwrap().to_string_lossy().into_owned());
            None
        });
        assert_eq!(seen, ["03 Harbour Lights.ogg"]);
        let titles: Vec<&str> = third.songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Slow Signal", "Afterglow Avenue", "Harbour Lights"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_playlists_lines_find_their_songs() {
        let p = normalise(Path::new("/data/music/library/Playlists/../Mara Ellis/Night Bus/01 Slow Signal.mp3"));
        assert_eq!(p, PathBuf::from("/data/music/library/Mara Ellis/Night Bus/01 Slow Signal.mp3"));
    }
}
