//! Your library on YouTube Music, signed in: your playlists (Liked music
//! among them), the albums you saved and your library's artists, as YouTube
//! Music's own Library page lists them. Your likes (`FEmusic_liked_videos`)
//! are opened as they always were, from the list itself.
//!
//! - **Asked for** in the background, never on the drawing thread: when
//!   Music opens online, at most once a minute. Each list page by page, as
//!   ytmusicapi asks for it and reads it: `FEmusic_liked_playlists` and
//!   `FEmusic_liked_albums` are a `gridRenderer` of `musicTwoRowItemRenderer`s,
//!   `FEmusic_library_corpus_track_artists` (the artists of your library's
//!   songs) and `FEmusic_library_corpus_artists` (those you subscribe to) a
//!   `musicShelfRenderer` of `musicResponsiveListItemRenderer`s.
//! - **Kept on the card** (`/data/music/your-library.json`, and each list's
//!   small cover in the cache), so the lists show at once, and offline.
//! - **Signed out**, none of it shows, and signing out (or in again, perhaps
//!   as someone else) forgets it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{art_url, byline_parts, continuation, covers, find_all, innertube, polite, runs_text, store, Collection, Kind, Track, MAX_PAGES};

/// The browse ids, as ytmusicapi has them.
pub const PLAYLISTS: &str = "FEmusic_liked_playlists";
pub const ALBUMS: &str = "FEmusic_liked_albums";
pub const ARTISTS: &str = "FEmusic_library_corpus_track_artists";
pub const SUBSCRIPTIONS: &str = "FEmusic_library_corpus_artists";
/// Liked music, the playlist YouTube Music keeps of the songs you like.
pub const LIKED_MUSIC: &str = "VLLM";

/// Not asked again sooner than this after the last time (Music opened and
/// closed and opened again).
const AGAIN_AFTER: Duration = Duration::from_secs(60);
/// Covers fetched in one refresh, at most.
const COVERS_AT_ONCE: usize = 300;

/// The lists, and when they were last asked for whole (unix seconds).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lists {
    pub playlists: Vec<Collection>,
    pub albums: Vec<Collection>,
    pub artists: Vec<Collection>,
    pub saved: i64,
}

impl Lists {
    pub fn is_empty(&self) -> bool {
        self.playlists.is_empty() && self.albums.is_empty() && self.artists.is_empty()
    }

    fn all(&self) -> impl Iterator<Item = &Collection> {
        self.playlists.iter().chain(&self.albums).chain(&self.artists)
    }

    fn to_json(&self) -> Value {
        let list = |l: &[Collection]| Value::Array(l.iter().map(|c| json!([c.id, c.title, c.byline, c.art, c.year])).collect());
        json!({"saved": self.saved, "playlists": list(&self.playlists), "albums": list(&self.albums), "artists": list(&self.artists)})
    }

    fn from_json(v: &Value) -> Lists {
        let list = |key: &str, kind: Kind| -> Vec<Collection> {
            v[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            let s = |i: usize| c[i].as_str().unwrap_or_default().to_owned();
                            let id = s(0);
                            (!id.is_empty()).then(|| Collection { kind, id, title: s(1), byline: s(2), art: s(3), year: c[4].as_u64().unwrap_or(0) as u32 })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Lists { playlists: list("playlists", Kind::Playlist), albums: list("albums", Kind::Album), artists: list("artists", Kind::Artist), saved: v["saved"].as_i64().unwrap_or(0) }
    }
}

static LISTS: Mutex<Option<Arc<Lists>>> = Mutex::new(None);
static STAMP: AtomicU64 = AtomicU64::new(0);
static REFRESHING: AtomicBool = AtomicBool::new(false);
/// What went wrong the last time, said on the page.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);
static LAST_ASKED: Mutex<Option<Instant>> = Mutex::new(None);
/// Counts forgettings: a refresh begun before one is not kept.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn file() -> PathBuf {
    store::root().join("your-library.json")
}

/// The lists as last seen: none signed out.
pub fn lists() -> Arc<Lists> {
    if !super::signed_in() {
        return Arc::default();
    }
    LISTS.lock().unwrap().get_or_insert_with(|| Arc::new(std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| Lists::from_json(&v)).unwrap_or_default())).clone()
}

/// Counts changes, for the pages showing the lists.
pub fn stamp() -> u64 {
    STAMP.load(Ordering::Relaxed)
}

pub fn refreshing() -> bool {
    REFRESHING.load(Ordering::Relaxed)
}

pub fn problem() -> Option<String> {
    PROBLEM.lock().unwrap().clone()
}

/// The lists asked for again in the background, signed in and online: not
/// if they were asked for less than a minute ago (unless `now`), nor while
/// they are being.
pub fn refresh(now: bool) {
    if !super::signed_in() || !polite::online() {
        return;
    }
    {
        let mut last = LAST_ASKED.lock().unwrap();
        if !now && last.is_some_and(|t| t.elapsed() < AGAIN_AFTER) {
            return;
        }
        if REFRESHING.swap(true, Ordering::Relaxed) {
            return;
        }
        *last = Some(Instant::now());
    }
    STAMP.fetch_add(1, Ordering::Relaxed);
    let generation = GENERATION.load(Ordering::Relaxed);
    let spawned = std::thread::Builder::new()
        .name("music-yours".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            polite::background_thread();
            let old = lists();
            let got = if super::on_device() {
                fetch(&old)
            } else {
                std::thread::sleep(Duration::from_millis(700));
                (demo(), None)
            };
            // Signed out (or in as someone else) while it was asked for: not kept.
            if GENERATION.load(Ordering::Relaxed) == generation && super::signed_in() {
                *PROBLEM.lock().unwrap() = got.1;
                if got.0 != *old {
                    keep(got.0.clone());
                }
                STAMP.fetch_add(1, Ordering::Relaxed);
                if super::on_device() {
                    keep_covers(&old, &got.0, generation);
                }
            }
            REFRESHING.store(false, Ordering::Relaxed);
            STAMP.fetch_add(1, Ordering::Relaxed);
        });
    if spawned.is_err() {
        REFRESHING.store(false, Ordering::Relaxed);
    }
}

fn keep(lists: Lists) {
    store::write_atomic(&file(), lists.to_json().to_string().as_bytes()).ok();
    *LISTS.lock().unwrap() = Some(Arc::new(lists));
}

/// Signed out, or signed in again: the lists, and their covers, forgotten.
pub fn forget() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    let old = LISTS.lock().unwrap().take().or_else(|| std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| Arc::new(Lists::from_json(&v))));
    if let Some(old) = old {
        for c in old.all() {
            let (big, small) = covers::cache_pair(&c.id);
            std::fs::remove_file(big).ok();
            std::fs::remove_file(small).ok();
        }
    }
    std::fs::remove_file(file()).ok();
    *PROBLEM.lock().unwrap() = None;
    *LAST_ASKED.lock().unwrap() = None;
    STAMP.fetch_add(1, Ordering::Relaxed);
}

/// Each list's small cover kept on the card, so the lists show them offline
/// too; those of lists no longer there let go.
fn keep_covers(old: &Lists, new: &Lists, generation: u64) {
    for gone in old.all().filter(|o| !new.all().any(|n| n.id == o.id)) {
        let (big, small) = covers::cache_pair(&gone.id);
        std::fs::remove_file(big).ok();
        std::fs::remove_file(small).ok();
    }
    let wanted = new.all().filter(|c| !c.art.is_empty() && !covers::cache_pair(&c.id).1.exists()).take(COVERS_AT_ONCE);
    for c in wanted {
        // Forgotten meanwhile, offline, or memory short: the rest another time.
        if GENERATION.load(Ordering::Relaxed) != generation || !polite::online() || polite::memory_short() {
            return;
        }
        if covers::keep_small(&Track { id: c.id.clone(), art: c.art.clone(), title: c.title.clone(), ..Track::default() }) {
            STAMP.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Every list, from YouTube Music: what could not be had kept as it was,
/// and what went wrong.
fn fetch(old: &Lists) -> (Lists, Option<String>) {
    let mut problem = None;
    // After one goes wrong (no internet, the sign-in refused), the rest would too.
    let mut get = |id: &str, part: Part, read: fn(&Value) -> Vec<Collection>| -> Option<Vec<Collection>> {
        if problem.is_some() {
            return None;
        }
        pages(id, part, read).map_err(|why| problem = Some(why)).ok()
    };
    let playlists = get(PLAYLISTS, Part::Grid, parse_playlists);
    let albums = get(ALBUMS, Part::Grid, parse_albums);
    let artists = get(ARTISTS, Part::Shelf, parse_artists);
    // The artists you subscribe to, after those of your songs: a list that
    // could not be had leaves the rest as they are.
    let subscribed = if problem.is_none() { pages(SUBSCRIPTIONS, Part::Shelf, parse_artists).unwrap_or_default() } else { Vec::new() };
    let lists = Lists {
        playlists: playlists.map(with_liked_music).unwrap_or_else(|| old.playlists.clone()),
        albums: albums.unwrap_or_else(|| old.albums.clone()),
        artists: artists.map(|a| merged(a, subscribed)).unwrap_or_else(|| old.artists.clone()),
        saved: if problem.is_none() { store::now_unix() } else { old.saved },
    };
    (lists, problem)
}

/// Where a list's items are: a grid (playlists, albums) or a shelf (artists).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Grid,
    Shelf,
}

impl Part {
    /// On the first page, then on the pages after it: the old way's
    /// continuation, or the newer way's items to add.
    fn keys(self) -> [&'static str; 3] {
        match self {
            Part::Grid => ["gridRenderer", "gridContinuation", "continuationItems"],
            Part::Shelf => ["musicShelfRenderer", "musicShelfContinuation", "continuationItems"],
        }
    }
}

/// A page's part holding the list, if it has one.
fn part_of(v: &Value, part: Part) -> Option<&Value> {
    let mut found = Vec::new();
    for key in part.keys() {
        find_all(v, key, &mut found);
        if let Some(p) = found.first() {
            return Some(p);
        }
    }
    None
}

/// A list's items, and the next page's token if there is one.
fn page(v: &Value, part: Part, read: fn(&Value) -> Vec<Collection>) -> (Vec<Collection>, Option<String>) {
    match part_of(v, part) {
        Some(p) => (read(p), continuation(p)),
        None => (Vec::new(), None),
    }
}

/// A list, page after page: an old-style token goes back with the first
/// request's own body (as ytmusicapi sends it), a newer one on its own.
fn pages(id: &str, part: Part, read: fn(&Value) -> Vec<Collection>) -> Result<Vec<Collection>, String> {
    let body = json!({"browseId": id});
    let first = innertube("browse", body.clone())?;
    let (mut items, mut next) = page(&first, part, read);
    let mut asked = 1;
    while let Some(token) = next.take() {
        if asked >= MAX_PAGES {
            break;
        }
        asked += 1;
        let v = match token.split_once(':') {
            Some(("c", t)) => innertube("browse", json!({"continuation": t}))?,
            Some((_, t)) => innertube(&format!("browse?ctoken={t}&continuation={t}"), body.clone())?,
            None => break,
        };
        let (more, after) = page(&v, part, read);
        if more.is_empty() {
            break;
        }
        items.extend(more);
        next = after;
    }
    let mut seen = std::collections::HashSet::new();
    items.retain(|c| seen.insert(c.id.clone()));
    Ok(items)
}

/// Liked music first, even when YouTube leaves it off the list.
fn with_liked_music(mut playlists: Vec<Collection>) -> Vec<Collection> {
    if !playlists.iter().any(|p| p.id == LIKED_MUSIC) {
        playlists.insert(0, Collection { kind: Kind::Playlist, id: LIKED_MUSIC.into(), title: "Liked music".into(), byline: "Auto playlist".into(), art: String::new(), year: 0 });
    }
    playlists
}

/// Your songs' artists, then those you subscribe to that are not among them.
fn merged(mut artists: Vec<Collection>, subscribed: Vec<Collection>) -> Vec<Collection> {
    for s in subscribed {
        if !artists.iter().any(|a| a.id == s.id) {
            artists.push(s);
        }
    }
    artists
}

/// What a two-row item (a playlist's or album's tile) links to.
fn two_row_link(r: &Value) -> Option<&str> {
    r["title"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str().or_else(|| r["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str())
}

/// Your playlists: a grid of two-row items, each linking to "VL…" (the
/// first, "New playlist", links to nothing of the kind and is passed by).
pub fn parse_playlists(v: &Value) -> Vec<Collection> {
    let mut items = Vec::new();
    find_all(v, "musicTwoRowItemRenderer", &mut items);
    items
        .into_iter()
        .filter_map(|r| {
            let id = two_row_link(r).filter(|id| id.starts_with("VL"))?;
            let title = runs_text(&r["title"]);
            if title.is_empty() {
                return None;
            }
            let mut parts = byline_parts(&runs_text(&r["subtitle"]));
            if parts.first().is_some_and(|p| p == "Playlist") {
                parts.remove(0);
            }
            Some(Collection { kind: Kind::Playlist, id: id.to_owned(), title, byline: parts.join(" · "), art: art_url(&r["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]), year: 0 })
        })
        .collect()
}

/// Your saved albums: a grid of two-row items, each with its kind, its
/// artists and its year under it ("Album • Daft Punk • 2013").
pub fn parse_albums(v: &Value) -> Vec<Collection> {
    let mut items = Vec::new();
    find_all(v, "musicTwoRowItemRenderer", &mut items);
    items
        .into_iter()
        .filter_map(|r| {
            let id = two_row_link(r).filter(|id| !id.starts_with("VL"))?;
            let title = runs_text(&r["title"]);
            if title.is_empty() {
                return None;
            }
            let mut parts = byline_parts(&runs_text(&r["subtitle"]));
            if parts.first().is_some_and(|p| matches!(p.as_str(), "Album" | "Single" | "EP" | "Audiobook")) {
                parts.remove(0);
            }
            let year = parts.last().and_then(|p| p.parse::<u32>().ok()).filter(|y| (1900..2100).contains(y)).unwrap_or(0);
            if year > 0 {
                parts.pop();
            }
            Some(Collection { kind: Kind::Album, id: id.to_owned(), title, byline: parts.join(" · "), art: art_url(&r["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]), year })
        })
        .collect()
}

/// Your library's artists: a shelf of list items, each linking to the
/// artist ("MPLA" and their channel, for the library's own page of them:
/// their channel alone opens their page with its top songs).
pub fn parse_artists(v: &Value) -> Vec<Collection> {
    let mut items = Vec::new();
    find_all(v, "musicResponsiveListItemRenderer", &mut items);
    items
        .into_iter()
        .filter_map(|r| {
            let id = r["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()?;
            let id = id.strip_prefix("MPLA").unwrap_or(id);
            let cols = r["flexColumns"].as_array()?;
            let col = |i: usize| cols.get(i).map(|c| runs_text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).unwrap_or_default();
            let title = col(0);
            if id.is_empty() || title.is_empty() {
                return None;
            }
            let mut parts = byline_parts(&col(1));
            if parts.first().is_some_and(|p| p == "Artist") {
                parts.remove(0);
            }
            Some(Collection { kind: Kind::Artist, id: id.to_owned(), title, byline: parts.join(" · "), art: art_url(&r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]), year: 0 })
        })
        .collect()
}

/// The simulator's library.
fn demo() -> Lists {
    let c = |kind: Kind, id: &str, title: &str, byline: &str, year: u32| Collection { kind, id: id.into(), title: title.into(), byline: byline.into(), art: String::new(), year };
    Lists {
        playlists: vec![
            c(Kind::Playlist, LIKED_MUSIC, "Liked music", "Auto playlist", 0),
            c(Kind::Playlist, "VLdemolist1", "Pit Crew Mix", "Thomas · 10 songs", 0),
            c(Kind::Playlist, "VLdemolist2", "Late Night Drive", "Thomas · 8 songs", 0),
            c(Kind::Playlist, "VLdemolist3", "Workshop Focus", "Catalyst · 24 songs", 0),
            c(Kind::Playlist, "VLdemolist4", "Match Day Warm-up", "Thomas · 15 songs", 0),
        ],
        albums: vec![
            c(Kind::Album, "demoalbum1", "Orbit Songs", "Juniper Vale", 2023),
            c(Kind::Album, "demoalbum2", "Low Water", "Wren & the Tide", 2021),
            c(Kind::Album, "demoalbum3", "Fremont", "Hollis Gray", 2024),
            c(Kind::Album, "demoalbum4", "Night Bus", "Mara Ellis", 2022),
            c(Kind::Album, "demoalbum5", "Copper Season", "The Northfields", 2020),
            c(Kind::Album, "demoalbum6", "Glasshouse", "Ocean Avenue Club", 2019),
        ],
        artists: vec![
            c(Kind::Artist, "demoartist1", "Juniper Vale", "1.2M subscribers", 0),
            c(Kind::Artist, "demoartist2", "Mara Ellis", "860K subscribers", 0),
            c(Kind::Artist, "demoartist3", "Wren & the Tide", "412K subscribers", 0),
            c(Kind::Artist, "demoartist4", "Hollis Gray", "98K subscribers", 0),
            c(Kind::Artist, "demoartist5", "The Northfields", "2.4M subscribers", 0),
            c(Kind::Artist, "demoartist6", "Ocean Avenue Club", "640K subscribers", 0),
        ],
        saved: store::now_unix(),
    }
}

/// The simulator's library, at once, as if it had been asked for: for the
/// assistant's shots, and the tests.
#[cfg(any(test, not(target_arch = "aarch64")))]
pub fn seed_demo_for_shots() {
    *PROBLEM.lock().unwrap() = None;
    keep(demo());
    *LAST_ASKED.lock().unwrap() = Some(Instant::now());
    STAMP.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(parts: &[&str]) -> Value {
        json!({"runs": parts.iter().map(|p| json!({"text": p})).collect::<Vec<_>>()})
    }

    fn thumbs(url: &str) -> Value {
        json!({"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": url}]}}})
    }

    /// A tile in a library grid, as YouTube Music sends one.
    fn tile(id: &str, title: &str, subtitle: &[&str]) -> Value {
        json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": title, "navigationEndpoint": {"browseEndpoint": {"browseId": id,
                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_PLAYLIST"}}}}}]},
            "subtitle": runs(subtitle),
            "thumbnailRenderer": thumbs(&format!("https://lh3.googleusercontent.com/{id}=w226-h226-l90-rj")),
            "navigationEndpoint": {"browseEndpoint": {"browseId": id}}
        }})
    }

    /// The library's playlists page: its tabs, the section, the item
    /// section, the grid (as ytmusicapi finds it), and an old-style token.
    fn playlists_page() -> Value {
        json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"itemSectionRenderer": {"contents": [{"gridRenderer": {
                "items": [
                    // "New playlist" leads the grid and is no playlist.
                    {"musicTwoRowItemRenderer": {"title": runs(&["New playlist"]), "navigationEndpoint": {"createPlaylistEndpoint": {}}, "subtitle": {}}},
                    tile("VLLM", "Liked music", &["Auto playlist"]),
                    tile("VLPLabc123", "Pit Crew Mix", &["Thomas", " • ", "25 songs"]),
                    tile("VLPLdef456", "Late Night Drive", &["Playlist", " • ", "Thomas", " • ", "8 songs"])
                ],
                "continuations": [{"nextContinuationData": {"continuation": "PAGE2", "clickTrackingParams": "x"}}]
            }}]}}
        ]}}}}]}}})
    }

    #[test]
    fn your_playlists_read_as_ytmusicapi_reads_them() {
        let (items, next) = page(&playlists_page(), Part::Grid, parse_playlists);
        let titles: Vec<&str> = items.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Liked music", "Pit Crew Mix", "Late Night Drive"], "New playlist is passed by");
        assert_eq!((items[1].id.as_str(), items[1].byline.as_str(), items[1].kind), ("VLPLabc123", "Thomas · 25 songs", Kind::Playlist));
        assert_eq!(items[2].byline, "Thomas · 8 songs", "its kind left off");
        assert_eq!(items[0].art, "https://lh3.googleusercontent.com/VLLM=w226-h226-l90-rj");
        assert_eq!(next.as_deref(), Some("n:PAGE2"));
        // The page after it, the old way: the grid's continuation.
        let more = json!({"continuationContents": {"gridContinuation": {"items": [tile("VLPLghi789", "Workshop Focus", &["Catalyst", " • ", "24 songs"])]}}});
        let (items, next) = page(&more, Part::Grid, parse_playlists);
        assert_eq!((items.len(), items[0].title.as_str(), next), (1, "Workshop Focus", None));
        // And the newer way: items to add, a token in the last.
        let newer = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {"continuationItems": [
            tile("VLPLjkl012", "Match Day", &["Thomas", " • ", "15 songs"]),
            {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "PAGE3", "request": "CONTINUATION_REQUEST_TYPE_BROWSE"}}}}
        ]}}]});
        let (items, next) = page(&newer, Part::Grid, parse_playlists);
        assert_eq!((items.len(), next.as_deref()), (1, Some("c:PAGE3")));
    }

    #[test]
    fn liked_music_is_always_first_among_the_playlists() {
        let without = parse_playlists(&json!({"items": [tile("VLPLabc123", "Pit Crew Mix", &["Thomas", " • ", "25 songs"])]}));
        let lists = with_liked_music(without);
        assert_eq!((lists[0].id.as_str(), lists[0].title.as_str(), lists.len()), (LIKED_MUSIC, "Liked music", 2));
        assert_eq!(with_liked_music(lists.clone()), lists, "not twice");
    }

    #[test]
    fn saved_albums_keep_their_artist_and_year() {
        let album = |id: &str, title: &str, subtitle: &[&str]| {
            json!({"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": title, "navigationEndpoint": {"browseEndpoint": {"browseId": id,
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}]},
                "subtitle": runs(subtitle),
                "thumbnailRenderer": thumbs("https://lh3.googleusercontent.com/cover=w120-h120-l90-rj"),
                "menu": {"menuRenderer": {"items": [{"menuNavigationItemRenderer": {"navigationEndpoint": {"watchPlaylistEndpoint": {"playlistId": "OLAK5uy_x"}}}}]}}
            }})
        };
        let v = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"gridRenderer": {"items": [
                album("MPREb_abc", "Random Access Memories", &["Album", " • ", "Daft Punk", " • ", "2013"]),
                album("MPREb_def", "Night Bus", &["EP", " • ", "Mara Ellis", " & ", "Hollis Gray", " • ", "2022"]),
                album("MPREb_ghi", "Copper Season", &["Single", " • ", "The Northfields"])
            ]}}
        ]}}}}]}}});
        let (a, next) = page(&v, Part::Grid, parse_albums);
        assert_eq!(next, None);
        assert_eq!((a[0].kind, a[0].id.as_str(), a[0].title.as_str(), a[0].byline.as_str(), a[0].year), (Kind::Album, "MPREb_abc", "Random Access Memories", "Daft Punk", 2013));
        assert_eq!((a[1].byline.as_str(), a[1].year), ("Mara Ellis & Hollis Gray", 2022));
        assert_eq!((a[2].byline.as_str(), a[2].year), ("The Northfields", 0));
        assert_eq!(a[0].art, "https://lh3.googleusercontent.com/cover=w226-h226-l90-rj");
    }

    #[test]
    fn your_artists_open_on_their_own_page_and_pages_follow() {
        let artist = |id: &str, name: &str, line: &str| {
            json!({"musicResponsiveListItemRenderer": {
                "navigationEndpoint": {"browseEndpoint": {"browseId": id, "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_LIBRARY_ARTIST"}}}},
                "flexColumns": [
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": name}]}}},
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": line}]}}}
                ],
                "thumbnail": thumbs("https://yt3.ggpht.com/face=s120-c-k")
            }})
        };
        let v = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"itemSectionRenderer": {"contents": [{"musicShelfRenderer": {
                "contents": [artist("MPLAUCabc", "Daft Punk", "12 songs"), artist("UCdef", "Mara Ellis", "2.91K subscribers")],
                "continuations": [{"nextContinuationData": {"continuation": "MORE"}}]
            }}]}}
        ]}}}}]}}});
        let (a, next) = page(&v, Part::Shelf, parse_artists);
        assert_eq!((a[0].kind, a[0].id.as_str(), a[0].title.as_str(), a[0].byline.as_str()), (Kind::Artist, "UCabc", "Daft Punk", "12 songs"), "the library's own page left for the artist's");
        assert_eq!((a[1].id.as_str(), a[1].byline.as_str()), ("UCdef", "2.91K subscribers"));
        assert_eq!(next.as_deref(), Some("n:MORE"));
        let more = json!({"continuationContents": {"musicShelfContinuation": {"contents": [artist("MPLAUCghi", "Hollis Gray", "3 songs")]}}});
        assert_eq!(page(&more, Part::Shelf, parse_artists).0[0].id, "UCghi");
        // Those you subscribe to follow your songs' artists, once each.
        let subscribed = parse_artists(&json!([artist("UCabc", "Daft Punk", "9M subscribers"), artist("UCxyz", "Juniper Vale", "1.2M subscribers")]));
        let all = merged(a, subscribed);
        assert_eq!(all.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Daft Punk", "Mara Ellis", "Juniper Vale"]);
        // An empty library: nothing, and no page to follow.
        assert_eq!(page(&json!({"contents": {}}), Part::Shelf, parse_artists), (Vec::new(), None));
    }

    #[test]
    fn the_lists_are_kept_on_the_card_as_they_were() {
        let lists = demo();
        assert_eq!(Lists::from_json(&lists.to_json()), lists);
        assert!(Lists::default().is_empty() && !lists.is_empty());
    }
}
