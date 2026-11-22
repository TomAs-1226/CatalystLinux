//! Music: YouTube Music on the handheld, playing on while other apps are
//! used, and with the screen off.
//!
//! - **Finding songs:** YouTube Music's own web API (InnerTube, the
//!   `WEB_REMIX` client): search (songs only), the home page's quick picks,
//!   your likes when signed in, and each song's radio for what plays next.
//!   Requests go through curl, as everything else here does.
//! - **Finding a song's stream** (lookup.rs): YouTube's own player API asked
//!   as its visionOS app, which is given stream addresses outright, so there
//!   is no player JavaScript to run: about a second, a curl's worth of
//!   memory. yt-dlp (the same client, then with QuickJS for YouTube's
//!   puzzle) only if that fails, at low priority, never two at once.
//! - **Playing** (player.rs): from the card whenever the song is there (the
//!   library, else the played cache); otherwise fetched (fetch.rs) into the
//!   cache in 1 MiB ranges and played from that file as it grows, by
//!   `catalyst-play` (symphonia and libopus in one small process, to ALSA's
//!   "default": the system volume and Bluetooth apply). Pausing stops it,
//!   which lets the speaker's amplifier go off. The next few songs come
//!   ahead, so the next starts at once and Wi-Fi can rest between.
//! - **On the card** (store.rs, library.rs): every song played is kept to
//!   play again offline (a few GB, the oldest making way); songs saved go
//!   into the library with their tags and covers, with your own music.
//!   Downloads (downloads.rs) save albums, playlists, an artist's top songs,
//!   your likes or everything played, one at a time and politely.
//! - **Signed out by default:** nothing is sent but the searches and the
//!   songs asked for. Signing in is a browser's cookies for
//!   music.youtube.com, put at `/data/music/cookies.txt` (0600); the app
//!   says plainly that a third-party client can break YouTube's terms.
//!   The cookies only ever go to curl and yt-dlp in files, never on a
//!   command line.
//!
//! On a computer (the simulator) a made-up catalogue stands in, with covers
//! drawn from each song's name, and songs "play" silently.

mod covers;
mod downloads;
mod fetch;
mod library;
pub mod lookup;
mod player;
pub mod polite;
mod screen;
mod settings;
mod store;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::photos::Picture;
use crate::robot::Robot;
use crate::shell::{Cx, Screen};

pub use player::{media_key, playing, tile};

/// The tour: nothing playing (to show what Music opens on then).
#[cfg(not(target_arch = "aarch64"))]
pub fn stop_for_shots() {
    player::clear();
    for _ in 0..50 {
        if player::now().track().is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub use screen::MusicScreen;
pub use settings::{summary, MusicSettings};

const DIR: &str = "/data/music";
const COOKIES: &str = "/data/music/cookies.txt";
/// The image's yt-dlp, and a newer copy brought to /data (Account › Stream finder).
const RESOLVER: &str = "/usr/libexec/catalyst/yt-dlp";
const RESOLVER_UPDATED: &str = "/data/music/yt-dlp";
const QJS: &str = "/usr/bin/qjs";
const FFMPEG: &str = "/usr/bin/ffmpeg";
/// Files of one request (headers, the cookies' copy), in memory, not on the card.
const ART_DIR: &str = "/tmp/catalyst-music";
const ORIGIN: &str = "https://music.youtube.com";
/// Search's "songs" filter, as YouTube Music's own page asks for it.
const SONGS_ONLY: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const REQUEST_SECONDS: &str = "20";
/// Search's other filters, as YouTube Music's own page asks for them.
const ALBUMS_ONLY: &str = "EgWKAQIYAWoMEA4QChADEAQQCRAF";
const ARTISTS_ONLY: &str = "EgWKAQIgAWoMEA4QChADEAQQCRAF";
const PLAYLISTS_ONLY: &str = "EgWKAQIoAWoMEA4QChADEAQQCRAF";
/// A long playlist, or all your likes: at most this many pages of it.
const MAX_PAGES: usize = 30;

/// The handheld (or its emulator): the real catalogue and player. Elsewhere
/// the simulator's made-up ones.
fn on_device() -> bool {
    cfg!(target_arch = "aarch64")
}

// ----------------------------------------------------------------- songs ---

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub seconds: Option<u32>,
    /// The cover's address (a square JPEG).
    pub art: String,
    /// On the card: the file it plays from (the library's songs).
    pub local: Option<PathBuf>,
}

/// "3:07", "1:02:03".
pub fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// "3:07" or "1:02:03" back into seconds.
fn parse_clock(text: &str) -> Option<u32> {
    let parts: Vec<u32> = text.trim().split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        [m, s] if *s < 60 => Some(m * 60 + s),
        [h, m, s] if *m < 60 && *s < 60 => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

fn runs_text(v: &Value) -> String {
    match v["runs"].as_array() {
        Some(runs) => runs.iter().filter_map(|r| r["text"].as_str()).collect(),
        None => v["simpleText"].as_str().unwrap_or_default().to_owned(),
    }
}

/// Every value under `key`, anywhere in `v`: YouTube moves its wrappers
/// about, the items themselves keep their names.
fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if k == key {
                    out.push(x);
                } else {
                    find_all(x, key, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|x| find_all(x, key, out)),
        _ => {}
    }
}

/// The largest of a set of thumbnails, as a 226 px square JPEG where the
/// server can make one (Google's image server takes its size in the name).
fn art_url(thumbnails: &Value) -> String {
    let url = thumbnails.as_array().and_then(|t| t.last()).and_then(|t| t["url"].as_str()).unwrap_or_default();
    match url.rfind("=w") {
        Some(at) if url.contains("googleusercontent.com") => format!("{}=w226-h226-l90-rj", &url[..at]),
        _ => url.to_owned(),
    }
}

/// A byline's parts ("Artist • Album • 3:07"), its kind ("Song") left off.
fn byline_parts(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = text.split(" • ").map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect();
    if parts.first().is_some_and(|p| matches!(p.as_str(), "Song" | "Video" | "Episode")) {
        parts.remove(0);
    }
    parts
}

/// "375M plays", "1.2B views": a count, not a name.
fn is_count(text: &str) -> bool {
    let t = text.to_lowercase();
    ["plays", "views", "listeners", "likes"].iter().any(|w| t.ends_with(w)) && t.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// The songs in a search, a home page or a list of likes: each a
/// `musicResponsiveListItemRenderer` that plays (has a video id).
pub fn parse_list(v: &Value) -> Vec<Track> {
    let mut items = Vec::new();
    find_all(v, "musicResponsiveListItemRenderer", &mut items);
    let mut tracks: Vec<Track> = items
        .into_iter()
        .filter_map(|r| {
            let cols = r["flexColumns"].as_array()?;
            let col = |i: usize| cols.get(i).map(|c| runs_text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).unwrap_or_default();
            let first = &cols.first()?["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"][0];
            let id = r["playlistItemData"]["videoId"]
                .as_str()
                .or_else(|| first["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str())
                .or_else(|| r["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]["videoId"].as_str())?;
            let title = col(0);
            if id.is_empty() || title.is_empty() {
                return None;
            }
            // How often it was played is not the album.
            let mut parts: Vec<String> = byline_parts(&col(1)).into_iter().filter(|p| !is_count(p)).collect();
            // Likes keep the album in a column of its own, the length in a fixed one.
            let fixed = r["fixedColumns"].as_array().and_then(|f| f.first()).map(|c| runs_text(&c["musicResponsiveListItemFixedColumnRenderer"]["text"])).unwrap_or_default();
            let mut seconds = parse_clock(&fixed);
            if seconds.is_none() && parts.last().is_some_and(|p| parse_clock(p).is_some()) {
                seconds = parts.pop().and_then(|p| parse_clock(&p));
            }
            let third = if is_count(&col(2)) { col(3) } else { col(2) };
            let album = if !third.is_empty() && !is_count(&third) && parse_clock(&third).is_none() {
                third
            } else if parts.len() >= 2 {
                parts.pop().unwrap_or_default()
            } else {
                String::new()
            };
            Some(Track {
                id: id.to_owned(),
                title,
                artist: parts.join(", "),
                album,
                seconds,
                art: art_url(&r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]),
                local: None,
            })
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    tracks.retain(|t| seen.insert(t.id.clone()));
    tracks
}

/// A song's radio, what plays after it: each a `playlistPanelVideoRenderer`.
pub fn parse_up_next(v: &Value) -> Vec<Track> {
    let mut items = Vec::new();
    find_all(v, "playlistPanelVideoRenderer", &mut items);
    items
        .into_iter()
        .filter_map(|r| {
            let id = r["videoId"].as_str()?;
            let parts = byline_parts(&runs_text(&r["longBylineText"]));
            Some(Track {
                id: id.to_owned(),
                title: runs_text(&r["title"]),
                artist: parts.first().cloned().unwrap_or_default(),
                album: parts.get(1).filter(|p| !p.chars().all(|c| c.is_ascii_digit())).cloned().unwrap_or_default(),
                seconds: parse_clock(&runs_text(&r["lengthText"])),
                art: art_url(&r["thumbnail"]["thumbnails"]),
                local: None,
            })
        })
        .collect()
}

// --------------------------------------------------------------- account ---

/// Signed in: a browser's cookies for YouTube are on /data.
pub fn signed_in() -> bool {
    if on_device() {
        Path::new(COOKIES).exists()
    } else {
        false
    }
}

/// The cookies for youtube.com from a Netscape cookies.txt, and the one
/// YouTube's request signature is made from.
fn cookie_header(text: &str) -> Option<(String, String)> {
    let mut pairs = Vec::new();
    let mut sapisid = None;
    for line in text.lines() {
        // curl marks HTTP-only cookies with this prefix; they count all the same.
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 || !f[0].trim_start_matches('.').ends_with("youtube.com") {
            continue;
        }
        let (name, value) = (f[5].trim(), f[6].trim());
        if name == "SAPISID" || (name == "__Secure-3PAPISID" && sapisid.is_none()) {
            sapisid = Some(value.to_owned());
        }
        pairs.push(format!("{name}={value}"));
    }
    Some((pairs.join("; "), sapisid?)).filter(|(c, _)| !c.is_empty())
}

/// YouTube's signature for a signed-in request: SHA-1 of the time, the
/// SAPISID cookie and the origin.
fn sapisidhash(sapisid: &str, now: i64) -> String {
    use sha1::{Digest, Sha1};
    let digest = Sha1::digest(format!("{now} {sapisid} {ORIGIN}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {now}_{hex}")
}

fn sign_out() -> Result<(), String> {
    if on_device() {
        std::fs::remove_file(COOKIES).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(format!("could not sign out: {e}")) })?;
    }
    Ok(())
}

// ------------------------------------------------------------- innertube ---

/// The web client's version, as YouTube Music's page sends it: its build
/// date, which YouTube accepts for a long while.
fn client_version() -> String {
    format!("1.{}.01.00", chrono::Utc::now().format("%Y%m%d"))
}

/// A request to YouTube Music's web API, its answer as JSON.
fn innertube(endpoint: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client": {"clientName": "WEB_REMIX", "clientVersion": client_version(), "hl": "en", "gl": "US"}});
    let mut headers = format!("Content-Type: application/json\nOrigin: {ORIGIN}\nX-Origin: {ORIGIN}\nReferer: {ORIGIN}/\nUser-Agent: Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36\n");
    if let Some((cookie, sapisid)) = std::fs::read_to_string(COOKIES).ok().as_deref().and_then(cookie_header) {
        headers.push_str(&format!("Cookie: {cookie}\nAuthorization: {}\nX-Goog-AuthUser: 0\n", sapisidhash(&sapisid, chrono::Utc::now().timestamp())));
    }
    // Headers (the cookies with them) go to curl in a file only root
    // reads, never on its command line.
    let file = private_file("request");
    crate::sys::write_private(&file, headers.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    let url = format!("{ORIGIN}/youtubei/v1/{endpoint}{}prettyPrint=false", if endpoint.contains('?') { '&' } else { '?' });
    let output = (|| {
        let mut cmd = Command::new("curl");
        cmd.args(["-sS", "--compressed", "--max-time", REQUEST_SECONDS, "--proto", "=https", "-H", &format!("@{}", file.display()), "--data-binary", "@-", "-w", "\n%{http_code}", &url]);
        let mut child = polite::polite(&mut cmd, polite::Manner::Background).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_| "curl could not start".to_owned())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the request could not be sent".to_owned())?;
        }
        child.wait_with_output().map_err(|_| "the request stopped".to_owned())
    })();
    std::fs::remove_file(&file).ok();
    let output = output?;
    if !output.status.success() {
        return Err(match output.status.code() {
            Some(6) | Some(7) => lookup::NO_INTERNET.into(),
            Some(28) => "youtube music did not answer in time".into(),
            Some(35) | Some(60) => "the secure connection failed · is the clock right?".into(),
            _ => "the request failed".into(),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    match code.trim() {
        "200" => serde_json::from_str(body).map_err(|_| "youtube music's answer could not be read".to_owned()),
        "401" | "403" if signed_in() => Err("the sign-in was refused: sign in again".into()),
        code => Err(format!("youtube music answered {code}")),
    }
}

/// Songs for a search.
fn search(query: &str) -> Result<Vec<Track>, String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(700));
        let q = query.to_lowercase();
        let found: Vec<Track> = demo_catalogue().into_iter().filter(|t| t.title.to_lowercase().contains(&q) || t.artist.to_lowercase().contains(&q)).collect();
        return Ok(if found.is_empty() { demo_catalogue().into_iter().rev().take(6).collect() } else { found });
    }
    Ok(parse_list(&innertube("search", json!({"query": query, "params": SONGS_ONLY}))?))
}

/// The first playlist a page offers (its browse id and its name).
fn first_playlist(v: &Value) -> Option<(String, String)> {
    let mut items = Vec::new();
    find_all(v, "musicTwoRowItemRenderer", &mut items);
    items.into_iter().find_map(|r| {
        let id = r["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()?;
        id.starts_with("VL").then(|| (id.to_owned(), runs_text(&r["title"])))
    })
}

/// What the start page lists, and what it is: your likes signed in; signed
/// out, the home page's quick picks, or (when it has only playlists to
/// offer, as it often does a visitor it does not know) its first playlist.
fn start_page() -> Result<(String, Vec<Track>), String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(500));
        return Ok(("quick picks".into(), demo_catalogue()));
    }
    let browse = |id: &str| innertube("browse", json!({"browseId": id}));
    if signed_in() {
        return Ok(("your likes".into(), parse_list(&browse("FEmusic_liked_videos")?)));
    }
    let home = browse("FEmusic_home")?;
    let picks = parse_list(&home);
    if !picks.is_empty() {
        return Ok(("quick picks".into(), picks));
    }
    match first_playlist(&home) {
        Some((id, name)) => Ok((name.to_lowercase(), parse_list(&browse(&id)?))),
        None => Ok(("quick picks".into(), Vec::new())),
    }
}

/// What plays after a song: its radio.
fn up_next(id: &str) -> Result<Vec<Track>, String> {
    if !on_device() {
        let all = demo_catalogue();
        let at = all.iter().position(|t| t.id == id).unwrap_or(0);
        return Ok(all.iter().cycle().skip(at).take(all.len()).cloned().collect());
    }
    let body = json!({"videoId": id, "playlistId": format!("RDAMVM{id}"), "isAudioOnly": true, "enablePersistentPlaylistPanel": true, "tunerSettingValue": "AUTOMIX_SETTING_NORMAL"});
    Ok(parse_up_next(&innertube("next", body)?))
}

// ------------------------------------------------------- albums and lists ---

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Album,
    Artist,
    Playlist,
    /// Your likes (signed in).
    Liked,
}

/// An album, artist or playlist on YouTube Music.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    pub kind: Kind,
    /// Its browse id ("MPREb_…", "UC…", "VL…").
    pub id: String,
    pub title: String,
    /// "Daft Punk", "misterepicpants · 5.9M views".
    pub byline: String,
    pub art: String,
    pub year: u32,
}

/// Albums, artists or playlists in a search.
pub fn parse_collections(v: &Value) -> Vec<Collection> {
    let mut items = Vec::new();
    find_all(v, "musicResponsiveListItemRenderer", &mut items);
    items
        .into_iter()
        .filter_map(|r| {
            let nav = &r["navigationEndpoint"]["browseEndpoint"];
            let id = nav["browseId"].as_str()?;
            let page = nav["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"].as_str().unwrap_or_default();
            let kind = match page {
                "MUSIC_PAGE_TYPE_ALBUM" => Kind::Album,
                "MUSIC_PAGE_TYPE_ARTIST" => Kind::Artist,
                "MUSIC_PAGE_TYPE_PLAYLIST" => Kind::Playlist,
                _ if id.starts_with("MPREb_") => Kind::Album,
                _ if id.starts_with("UC") => Kind::Artist,
                _ if id.starts_with("VL") => Kind::Playlist,
                _ => return None,
            };
            let cols = r["flexColumns"].as_array()?;
            let col = |i: usize| cols.get(i).map(|c| runs_text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).unwrap_or_default();
            let title = col(0);
            if title.is_empty() {
                return None;
            }
            let mut parts = byline_parts(&col(1));
            if parts.first().is_some_and(|p| matches!(p.as_str(), "Album" | "Single" | "EP" | "Artist" | "Playlist")) {
                parts.remove(0);
            }
            let year = parts.last().and_then(|p| p.parse::<u32>().ok()).filter(|y| (1900..2100).contains(y)).unwrap_or(0);
            if year > 0 {
                parts.pop();
            }
            Some(Collection { kind, id: id.to_owned(), title, byline: parts.join(" · "), art: art_url(&r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]), year })
        })
        .collect()
}

pub fn search_collections(query: &str, kind: Kind) -> Result<Vec<Collection>, String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(500));
        return Ok(demo_collections(kind));
    }
    search_collections_live(query, kind)
}

fn search_collections_live(query: &str, kind: Kind) -> Result<Vec<Collection>, String> {
    let params = match kind {
        Kind::Album => ALBUMS_ONLY,
        Kind::Artist => ARTISTS_ONLY,
        _ => PLAYLISTS_ONLY,
    };
    Ok(parse_collections(&innertube("search", json!({"query": query, "params": params}))?).into_iter().filter(|c| c.kind == kind).collect())
}

/// An album, playlist, artist or your likes, opened: its songs, each with
/// what it is saved under.
#[derive(Clone, Debug, Default)]
pub struct Opened {
    pub byline: String,
    pub tracks: Vec<Track>,
    pub metas: Vec<library::Meta>,
    /// Downloaded, it becomes a playlist of this name.
    pub playlist: Option<String>,
}

/// An album or playlist page's header: its title, the line under it, its
/// cover.
fn header(v: &Value) -> (String, String, String, String) {
    let mut h = Vec::new();
    find_all(v, "musicResponsiveHeaderRenderer", &mut h);
    let Some(h) = h.first() else { return Default::default() };
    let mut thumbs = Vec::new();
    find_all(h, "thumbnails", &mut thumbs);
    (runs_text(&h["title"]), runs_text(&h["straplineTextOne"]), runs_text(&h["subtitle"]), thumbs.first().map(|t| art_url(t)).unwrap_or_default())
}

/// The next page of a long list, if it has one.
fn continuation(v: &Value) -> Option<String> {
    let mut found = Vec::new();
    find_all(v, "continuationCommand", &mut found);
    if let Some(t) = found.iter().find_map(|c| c["token"].as_str()) {
        return Some(format!("c:{t}"));
    }
    found.clear();
    find_all(v, "nextContinuationData", &mut found);
    found.iter().find_map(|c| c["continuation"].as_str()).map(|t| format!("n:{t}"))
}

/// A playlist's songs (or your likes), page after page.
fn all_pages(first: Value) -> Result<Vec<Track>, String> {
    let mut tracks = parse_list(&first);
    let mut next = continuation(&first);
    let mut pages = 1;
    while let Some(token) = next.take() {
        if pages >= MAX_PAGES {
            break;
        }
        pages += 1;
        let v = match token.split_once(':') {
            Some(("c", t)) => innertube("browse", json!({"continuation": t}))?,
            Some((_, t)) => innertube(&format!("browse?ctoken={t}&continuation={t}&type=next"), json!({}))?,
            None => break,
        };
        let more = parse_list(&v);
        if more.is_empty() {
            break;
        }
        tracks.extend(more);
        next = continuation(&v);
    }
    let mut seen = std::collections::HashSet::new();
    tracks.retain(|t| seen.insert(t.id.clone()));
    Ok(tracks)
}

/// Your likes, all of them (signed in).
pub fn liked_all() -> Result<Vec<Track>, String> {
    all_pages(innertube("browse", json!({"browseId": "FEmusic_liked_videos"}))?)
}

pub fn open_collection(c: &Collection) -> Result<Opened, String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(400));
        return Ok(demo_opened(c));
    }
    open_live(c)
}

fn open_live(c: &Collection) -> Result<Opened, String> {
    let browse = |id: &str| innertube("browse", json!({"browseId": id}));
    match c.kind {
        Kind::Album => {
            let v = browse(&c.id)?;
            let (title, artist, subtitle, art) = header(&v);
            let title = if title.is_empty() { c.title.clone() } else { title };
            let year = subtitle.split(" • ").filter_map(|p| p.trim().parse::<u32>().ok()).find(|y| (1900..2100).contains(y)).unwrap_or(c.year);
            let artist = if artist.is_empty() { c.byline.clone() } else { artist };
            // The album's own numbers, as its page gives them.
            let mut items = Vec::new();
            find_all(&v, "musicResponsiveListItemRenderer", &mut items);
            let numbers: std::collections::HashMap<String, u32> = items
                .iter()
                .filter_map(|r| Some((r["playlistItemData"]["videoId"].as_str()?.to_owned(), runs_text(&r["index"]).trim().parse().ok()?)))
                .collect();
            let mut tracks = parse_list(&v);
            let mut metas = Vec::new();
            for (i, t) in tracks.iter_mut().enumerate() {
                t.album = title.clone();
                if t.artist.is_empty() {
                    t.artist = artist.clone();
                }
                if t.art.is_empty() {
                    t.art = if art.is_empty() { c.art.clone() } else { art.clone() };
                }
                metas.push(library::Meta { album_artist: artist.clone(), track_no: numbers.get(&t.id).copied().unwrap_or(i as u32 + 1), year });
            }
            let byline = if year > 0 { format!("{artist} · {year}") } else { artist };
            Ok(Opened { byline, metas, tracks, playlist: None })
        }
        Kind::Playlist | Kind::Liked => {
            let first = browse(&c.id)?;
            let (title, _, subtitle, _) = header(&first);
            let tracks = all_pages(first)?;
            let name = if title.is_empty() { c.title.clone() } else { title };
            let byline = if subtitle.is_empty() { format!("{} songs", tracks.len()) } else { format!("{} · {} songs", subtitle.replace(" • ", " · "), tracks.len()) };
            Ok(Opened { byline, metas: vec![library::Meta::default(); tracks.len()], tracks, playlist: Some(name) })
        }
        Kind::Artist => {
            let v = browse(&c.id)?;
            // "Top songs": the shelf's link to all of them, else the few it shows.
            let mut shelves = Vec::new();
            find_all(&v, "musicShelfRenderer", &mut shelves);
            let shelf = shelves.first().copied();
            let more = shelf.and_then(|s| s["bottomEndpoint"]["browseEndpoint"]["browseId"].as_str().map(str::to_owned));
            let mut tracks = match more {
                Some(id) => all_pages(browse(&id)?)?,
                None => shelf.map(parse_list).unwrap_or_default(),
            };
            tracks.truncate(100);
            Ok(Opened { byline: format!("top songs · {}", tracks.len()), metas: vec![library::Meta::default(); tracks.len()], tracks, playlist: Some(format!("{} · top songs", c.title)) })
        }
    }
}

// -------------------------------------------------------------- streams ---

/// Which yt-dlp finds streams the slow way: the newer of the image's and
/// the one brought to /data (an update later than that copy can come with
/// the image). Asked once a run (each asking is a second of Python on the
/// handheld), and again after an update.
static RESOLVER_CHOSEN: Mutex<Option<&'static str>> = Mutex::new(None);

fn resolver() -> &'static str {
    *RESOLVER_CHOSEN.lock().unwrap().get_or_insert_with(|| {
        if !Path::new(RESOLVER_UPDATED).exists() {
            return RESOLVER;
        }
        // yt-dlp's versions are dates, zero-padded: newer sorts later.
        match (version_of(RESOLVER_UPDATED), version_of(RESOLVER)) {
            (Some(copy), Some(image)) if copy >= image => RESOLVER_UPDATED,
            (Some(_), None) => RESOLVER_UPDATED,
            _ => RESOLVER,
        }
    })
}

fn version_of(exe: &str) -> Option<String> {
    let mut cmd = Command::new("python3");
    cmd.args([exe, "--version"]);
    let out = polite::output_within(polite::polite(&mut cmd, polite::Manner::Background), Duration::from_secs(30)).ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|v| out.status.success() && !v.is_empty())
}

/// A newer yt-dlp to /data/music, from its own releases (yt-dlp checks
/// what it fetches against the release's sums).
fn update_resolver() -> Result<String, String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(900));
        return Ok("2026.11.20".into());
    }
    std::fs::create_dir_all(DIR).map_err(|e| format!("music: {e}"))?;
    if !Path::new(RESOLVER_UPDATED).exists() {
        std::fs::copy(RESOLVER, RESOLVER_UPDATED).map_err(|e| format!("could not copy the stream finder: {e}"))?;
    }
    let mut cmd = Command::new("python3");
    cmd.args([RESOLVER_UPDATED, "--update-to", "stable"]);
    let updated = polite::output_within(polite::polite(&mut cmd, polite::Manner::Background), Duration::from_secs(180)).is_ok_and(|o| o.status.success());
    *RESOLVER_CHOSEN.lock().unwrap() = None;
    match version_of(RESOLVER_UPDATED) {
        Some(v) if updated => Ok(v),
        // Nothing half-done left to be chosen.
        _ => {
            std::fs::remove_file(RESOLVER_UPDATED).ok();
            Err("the update could not be fetched".into())
        }
    }
}

fn resolver_version() -> Option<String> {
    if !on_device() {
        return Some("2026.10.26".into());
    }
    version_of(resolver())
}

/// A file of this run's own, for one request: in /tmp (memory, not the
/// card), named so no two requests ever share one.
fn private_file(what: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = if on_device() { PathBuf::from(ART_DIR) } else { std::env::temp_dir().join("catalyst-music") };
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!(".{what}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

// ------------------------------------------------------------------ demo ---

fn demo_catalogue() -> Vec<Track> {
    let t = |id: &str, title: &str, artist: &str, album: &str, s: u32| Track { id: id.into(), title: title.into(), artist: artist.into(), album: album.into(), seconds: Some(s), art: String::new(), local: None };
    vec![
        t("demo01", "Harbour Lights", "Wren & the Tide", "Low Water", 214),
        t("demo02", "Paper Satellites", "Juniper Vale", "Orbit Songs", 187),
        t("demo03", "Copper Season", "The Northfields", "Copper Season", 243),
        t("demo04", "Slow Signal", "Mara Ellis", "Night Bus", 201),
        t("demo05", "Glasshouse", "Ocean Avenue Club", "Glasshouse", 226),
        t("demo06", "Weekend in Lisbon", "Juniper Vale", "Orbit Songs", 195),
        t("demo07", "Kites Over Fremont", "Hollis Gray", "Fremont", 258),
        t("demo08", "Static Bloom", "Wren & the Tide", "Low Water", 179),
        t("demo09", "Midnight Telemetry", "The Pit Crew", "Match Day", 232),
        t("demo10", "Afterglow Avenue", "Mara Ellis", "Night Bus", 207),
    ]
}

/// The simulator's albums, artists and playlists for a search.
fn demo_collections(kind: Kind) -> Vec<Collection> {
    let c = |id: &str, title: &str, byline: &str, year: u32| Collection { kind, id: id.into(), title: title.into(), byline: byline.into(), art: String::new(), year };
    match kind {
        Kind::Album => vec![c("demoalbum1", "Orbit Songs", "Juniper Vale", 2023), c("demoalbum2", "Low Water", "Wren & the Tide", 2021), c("demoalbum3", "Fremont", "Hollis Gray", 2024)],
        Kind::Artist => vec![c("demoartist1", "Juniper Vale", "", 0), c("demoartist2", "Mara Ellis", "", 0)],
        _ => vec![c("demolist1", "Pit Crew Mix", "Catalyst · 10 songs", 0), c("demolist2", "Late Night Drive", "Catalyst · 8 songs", 0)],
    }
}

fn demo_opened(c: &Collection) -> Opened {
    let all = demo_catalogue();
    let tracks: Vec<Track> = match c.kind {
        Kind::Album => all.into_iter().filter(|t| t.album == c.title).collect(),
        Kind::Artist => all.into_iter().filter(|t| t.artist == c.title).collect(),
        _ => all,
    };
    let metas = tracks.iter().enumerate().map(|(i, t)| library::Meta { album_artist: if c.kind == Kind::Album { t.artist.clone() } else { String::new() }, track_no: if c.kind == Kind::Album { i as u32 + 1 } else { 0 }, year: c.year }).collect();
    let byline = if c.kind == Kind::Album { format!("{} · {}", c.byline, c.year) } else { format!("{} songs", tracks.len()) };
    Opened { byline, metas, playlist: (c.kind != Kind::Album).then(|| c.title.clone()), tracks }
}

/// A cover for the simulator's made-up songs: two colours from the song's
/// name, a soft gradient and a disc of light.
fn demo_cover(track: &Track) -> Picture {
    let hash = track.title.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
    // A colour from a hue (degrees), its saturation and value.
    let hsv = |h: u32, s: f32, v: f32| -> [f32; 3] {
        let f = |n: f32| {
            let k = (n + (h % 360) as f32 / 60.0) % 6.0;
            v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
        };
        [f(5.0), f(3.0), f(1.0)]
    };
    let first = hash % 360;
    let second = (first + 110 + (hash >> 12) % 100) % 360;
    let (a, b) = (hsv(first, 0.62, 0.82), hsv(second, 0.70, 0.36));
    let n = COVER_PX as f32;
    let (sx, sy) = (n * (0.3 + (hash >> 20 & 7) as f32 * 0.05), n * (0.3 + (hash >> 24 & 7) as f32 * 0.04));
    let mut rgb = Vec::with_capacity(COVER_PX * COVER_PX * 3);
    for y in 0..COVER_PX {
        for x in 0..COVER_PX {
            let (fx, fy) = (x as f32, y as f32);
            let t = ((fx * 0.4 + fy) / (1.4 * n)).clamp(0.0, 1.0);
            let d = ((fx - sx).powi(2) + (fy - sy).powi(2)).sqrt();
            // A low sun, and a ring of light around it.
            let sun = (1.0 - d / (n * 0.24)).clamp(0.0, 1.0).powf(0.7) * 0.55;
            let ring = (1.0 - ((d - n * 0.36).abs() / 1.6)).clamp(0.0, 1.0) * 0.22;
            for k in 0..3 {
                let base = a[k] * (1.0 - t) + b[k] * t;
                rgb.push(((base + sun + ring).min(1.0) * 255.0) as u8);
            }
        }
    }
    Picture { w: COVER_PX, h: COVER_PX, rgb }
}

/// The demo covers' size.
const COVER_PX: usize = 112;

// ------------------------------------------------------------- spotlight ---

/// Songs the app has listed (the start page, searches, radios): Spotlight
/// finds them by name and artist without asking YouTube again.
static KNOWN: Mutex<Vec<Track>> = Mutex::new(Vec::new());
static KNOWN_CHANGED: AtomicU64 = AtomicU64::new(0);
const KNOWN_KEPT: usize = 200;

fn know(tracks: &[Track]) {
    let mut known = KNOWN.lock().unwrap();
    for t in tracks {
        known.retain(|k| k.id != t.id);
        known.push(t.clone());
    }
    let over = known.len().saturating_sub(KNOWN_KEPT);
    known.drain(..over);
    KNOWN_CHANGED.fetch_add(1, Ordering::Relaxed);
}

/// Music in Spotlight: the songs it knows, played from there, and, for
/// anything typed, a search of YouTube Music, after the rest.
pub struct Spotlight;

/// Spotlight finds Music's songs from now on.
pub fn register_spotlight() {
    crate::spotlight::register(Arc::new(Spotlight));
}

impl crate::spotlight::Provider for Spotlight {
    fn name(&self) -> &'static str {
        "music"
    }

    fn items(&self) -> Vec<crate::spotlight::Item> {
        use crate::spotlight::{weight, Item};
        let mut items: Vec<Item> = KNOWN.lock().unwrap().iter().map(|t| Item::new(format!("song:{}", t.id), t.title.clone(), "music_note", weight::OTHER).detail(format!("song · {}", t.artist.to_lowercase())).words(t.artist.clone())).collect();
        // The library's songs, found offline too.
        for s in library::songs().into_iter().take(2000) {
            let id = format!("card:{}", s.rel);
            items.push(Item::new(id, s.title.clone(), "music_note", weight::OTHER).detail(format!("on the card · {}", s.artist.to_lowercase())).words(format!("{} {}", s.artist, s.album)));
        }
        items
    }

    fn stamp(&self) -> Option<u64> {
        Some(KNOWN_CHANGED.load(Ordering::Relaxed) + library::stamp())
    }

    fn answer(&self, query: &crate::spotlight::Query, _robot: &Robot, out: &mut Vec<crate::spotlight::Item>) {
        use crate::spotlight::{weight, Item, TAIL};
        let q = query.raw.trim();
        if q.chars().count() >= 2 {
            out.push(Item::new(format!("search:{q}"), format!("Search YouTube Music for “{q}”"), "music_note", weight::OTHER).detail("music").ranked(TAIL));
        }
    }

    fn open(&self, item: &crate::spotlight::Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        if let Some(q) = item.id.strip_prefix("search:") {
            return Ok(Box::new(MusicScreen::searching(q)));
        }
        if let Some(rel) = item.id.strip_prefix("card:") {
            let song = library::songs().into_iter().find(|s| s.rel == rel).ok_or_else(|| "that song is no longer on the card".to_owned())?;
            // The rest of its album after it.
            let album: Vec<Track> = library::albums(Some(&song.album_artist_or_artist())).into_iter().find(|a| a.songs.iter().any(|s| s.rel == rel)).map(|a| a.songs.iter().map(library::Song::track).collect()).unwrap_or_else(|| vec![song.track()]);
            let at = album.iter().position(|t| t.local.as_deref() == Some(song.path.as_path())).unwrap_or(0);
            player::play(album, at, false);
            return Ok(Box::new(MusicScreen::new()));
        }
        let id = item.id.strip_prefix("song:").unwrap_or(&item.id);
        let track = KNOWN.lock().unwrap().iter().find(|t| t.id == id).cloned().ok_or_else(|| "that song is not to hand: search for it in Music".to_owned())?;
        player::play(vec![track], 0, true);
        Ok(Box::new(MusicScreen::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flex(text: &str) -> Value {
        json!({"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": text}]}}})
    }

    #[test]
    fn a_search_reads_as_songs_with_artist_album_and_length() {
        let v = json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [{"musicShelfRenderer": {"contents": [
            {"musicResponsiveListItemRenderer": {
                "playlistItemData": {"videoId": "abc123"},
                "flexColumns": [flex("Harbour Lights"), {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Wren"}, {"text": " & "}, {"text": "the Tide"}, {"text": " • "}, {"text": "Low Water"}, {"text": " • "}, {"text": "3:34"}]}}}],
                "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj"}, {"url": "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"}]}}}
            }},
            {"musicResponsiveListItemRenderer": {
                "flexColumns": [{"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Static Bloom", "navigationEndpoint": {"watchEndpoint": {"videoId": "def456"}}}]}}}, flex("Song • Mara Ellis • 1:02:03")]
            }},
            // An artist, which does not play: left out.
            {"musicResponsiveListItemRenderer": {"flexColumns": [flex("Mara Ellis"), flex("Artist • 2M monthly audience")]}},
            // The same song twice: once.
            {"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "abc123"}, "flexColumns": [flex("Harbour Lights"), flex("Wren • 3:34")]}}
        ]}}]}}}}]}}});
        let t = parse_list(&v);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].id.as_str(), t[0].title.as_str(), t[0].artist.as_str(), t[0].album.as_str(), t[0].seconds), ("abc123", "Harbour Lights", "Wren & the Tide", "Low Water", Some(214)));
        assert_eq!(t[0].art, "https://lh3.googleusercontent.com/abc=w226-h226-l90-rj");
        assert_eq!((t[1].id.as_str(), t[1].artist.as_str(), t[1].album.as_str(), t[1].seconds), ("def456", "Mara Ellis", "", Some(3723)));
    }

    #[test]
    fn likes_keep_the_album_and_length_in_columns_of_their_own() {
        let v = json!({"musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "x1"},
            "flexColumns": [flex("Copper Season"), flex("The Northfields"), flex("Copper Season (Deluxe)")],
            "fixedColumns": [{"musicResponsiveListItemFixedColumnRenderer": {"text": {"runs": [{"text": "4:03"}]}}}]
        }});
        let t = parse_list(&v);
        assert_eq!((t[0].artist.as_str(), t[0].album.as_str(), t[0].seconds), ("The Northfields", "Copper Season (Deluxe)", Some(243)));
    }

    #[test]
    fn quick_picks_say_how_often_they_were_played_which_is_not_the_album() {
        let v = json!({"musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "x2"},
            "flexColumns": [flex("Slow Signal"), flex("Mara Ellis • 375M plays"), flex("Night Bus")]
        }});
        let t = parse_list(&v);
        assert_eq!((t[0].artist.as_str(), t[0].album.as_str(), t[0].seconds), ("Mara Ellis", "Night Bus", None));
        let v = json!({"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "x3"}, "flexColumns": [flex("Glasshouse"), flex("Ocean Avenue Club • 1.2B views")]}});
        assert_eq!(parse_list(&v)[0].album, "");
    }

    #[test]
    fn a_radio_reads_as_what_plays_next() {
        let v = json!({"contents": {"x": {"playlistPanelRenderer": {"contents": [
            {"playlistPanelVideoRenderer": {"videoId": "a", "title": {"runs": [{"text": "Slow Signal"}]}, "longBylineText": {"runs": [{"text": "Mara Ellis"}, {"text": " • "}, {"text": "Night Bus"}, {"text": " • "}, {"text": "2024"}]}, "lengthText": {"runs": [{"text": "3:21"}]}, "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/a/sddefault.jpg"}]}}},
            {"playlistPanelVideoWrapperRenderer": {"primaryRenderer": {"playlistPanelVideoRenderer": {"videoId": "b", "title": {"runs": [{"text": "Glasshouse"}]}, "longBylineText": {"runs": [{"text": "Ocean Avenue Club"}, {"text": " • "}, {"text": "2019"}]}, "lengthText": {"simpleText": "3:46"}}}}}
        ]}}}});
        let t = parse_up_next(&v);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].artist.as_str(), t[0].album.as_str(), t[0].seconds, t[0].art.as_str()), ("Mara Ellis", "Night Bus", Some(201), "https://i.ytimg.com/vi/a/sddefault.jpg"));
        // A year is not an album.
        assert_eq!((t[1].title.as_str(), t[1].album.as_str(), t[1].seconds), ("Glasshouse", "", Some(226)));
    }

    #[test]
    fn spotlight_finds_the_songs_listed_and_offers_a_search() {
        use crate::spotlight::{Provider, Query};
        know(&demo_catalogue()[..2]);
        let items = Spotlight.items();
        assert!(items.iter().any(|i| i.id == "song:demo01" && i.title == "Harbour Lights" && i.detail == "song · wren & the tide"), "{items:?}");
        let before = Spotlight.stamp();
        know(&demo_catalogue()[2..3]);
        assert_ne!(Spotlight.stamp(), before, "read again once more are known");
        let mut out = Vec::new();
        Spotlight.answer(&Query::new("daft punk"), &Robot::demo(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].title.as_str(), out[0].id.as_str()), ("Search YouTube Music for “daft punk”", "search:daft punk"));
        // A single letter is no search yet.
        out.clear();
        Spotlight.answer(&Query::new("d"), &Robot::demo(), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn times_read_and_write_both_ways() {
        assert_eq!((parse_clock("3:07"), parse_clock("1:02:03"), parse_clock("3:75"), parse_clock("x")), (Some(187), Some(3723), None, None));
        assert_eq!((clock(187.9), clock(3723.0), clock(-4.0)), ("3:07".to_owned(), "1:02:03".to_owned(), "0:00".to_owned()));
    }

    #[test]
    fn the_cookies_sign_requests_without_going_on_a_command_line() {
        let jar = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tabc\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tHSID\tdef\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsecret/xyz\n.google.com\tTRUE\t/\tTRUE\t0\tNID\tnope\n";
        let (cookie, sapisid) = cookie_header(jar).unwrap();
        assert_eq!(cookie, "SID=abc; HSID=def; SAPISID=secret/xyz");
        assert_eq!(sapisid, "secret/xyz");
        // SHA-1 of "1700000000 secret/xyz https://music.youtube.com".
        let signed = sapisidhash("secret/xyz", 1_700_000_000);
        assert!(signed.starts_with("SAPISIDHASH 1700000000_") && signed.len() == "SAPISIDHASH 1700000000_".len() + 40);
        assert_eq!(cookie_header("# nothing\n"), None);
    }

    /// YouTube Music itself, signed out: a search, the home page and a radio
    /// read as songs. Over the network: `cargo test -- --ignored live_`.
    #[test]
    #[ignore]
    fn live_youtube_music_reads_as_songs() {
        let found = parse_list(&innertube("search", json!({"query": "daft punk", "params": SONGS_ONLY})).unwrap());
        assert!(found.len() >= 5, "{found:?}");
        assert!(found.iter().all(|t| t.id.len() == 11 && !t.title.is_empty() && !t.artist.is_empty() && t.seconds.is_some()), "{found:#?}");
        assert!(found.iter().filter(|t| t.art.starts_with("https://")).count() >= found.len() - 1);
        // The home page's quick picks, or its first playlist (the home page
        // often has only playlists for a visitor it does not know).
        let home = innertube("browse", json!({"browseId": "FEmusic_home"})).unwrap();
        let picks = parse_list(&home);
        let (id, name) = first_playlist(&home).expect("a playlist on the home page");
        let list = parse_list(&innertube("browse", json!({"browseId": id})).unwrap());
        assert!(list.len() >= 10, "{name}: {list:#?}");
        assert!(picks.iter().chain(&list).all(|t| !is_count(&t.album) && t.id.len() == 11 && !t.artist.is_empty()), "{picks:#?}");
        let radio = parse_up_next(&innertube("next", json!({"videoId": found[0].id, "playlistId": format!("RDAMVM{}", found[0].id), "isAudioOnly": true})).unwrap());
        assert!(radio.len() >= 10 && radio.iter().all(|t| !t.title.is_empty()), "{radio:#?}");
        // An album, an artist's top songs and a playlist, opened.
        let albums = search_collections_live("daft punk", Kind::Album).unwrap();
        assert!(albums.len() >= 3 && albums.iter().all(|a| a.id.starts_with("MPREb_") && a.year > 1990), "{albums:#?}");
        let album = open_live(&albums[0]).unwrap();
        assert!(album.tracks.len() >= 5 && album.tracks.iter().all(|t| t.album == albums[0].title && !t.art.is_empty()), "{album:#?}");
        assert!(album.metas.iter().enumerate().all(|(i, m)| m.track_no == i as u32 + 1 && m.year == albums[0].year && !m.album_artist.is_empty()), "{:#?}", album.metas);
        let artist = &search_collections_live("daft punk", Kind::Artist).unwrap()[0];
        let top = open_live(artist).unwrap();
        assert!(top.tracks.len() >= 5, "{top:#?}");
        let pl = &search_collections_live("daft punk", Kind::Playlist).unwrap()[0];
        let opened = open_live(pl).unwrap();
        assert!(opened.tracks.len() >= 5 && opened.playlist.is_some(), "{opened:#?}");
        println!("album {:?}: {} songs; top songs: {}; playlist {:?}: {} songs", albums[0].title, album.tracks.len(), top.tracks.len(), pl.title, opened.tracks.len());
        println!(
            "search: {:?}\nhome: {} quick picks; {name:?}: {} songs\nradio: {:?}",
            found.iter().take(3).map(|t| (&t.title, &t.artist, &t.album, t.seconds)).collect::<Vec<_>>(),
            picks.len(),
            list.len(),
            radio.iter().take(3).map(|t| (&t.title, &t.artist, t.seconds)).collect::<Vec<_>>()
        );
    }


}
