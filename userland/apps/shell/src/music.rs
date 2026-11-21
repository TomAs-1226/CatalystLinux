//! Music: YouTube Music on the handheld, playing on while other apps are
//! used, and with the screen off.
//!
//! - **Finding songs:** YouTube Music's own web API (InnerTube, the
//!   `WEB_REMIX` client): search (songs only), the home page's quick picks,
//!   your likes when signed in, and each song's radio for what plays next.
//!   Requests go through curl, as everything else here does.
//! - **Playing:** yt-dlp finds a song's audio-only stream (Opus, else AAC),
//!   answering YouTube's player challenge in QuickJS; ffmpeg fetches it over
//!   HTTPS, decodes it, resamples it to 48 kHz and plays it to ALSA's
//!   "default" (so the system volume and Bluetooth headphones apply). Pausing
//!   stops ffmpeg, which lets the speaker's amplifier go off; playing again
//!   starts it at the same place. The next song's stream is found while one
//!   plays, so the gap between them is short.
//! - **Signed out by default:** nothing is sent but the searches and the
//!   songs asked for. Signing in is a browser's cookies for
//!   music.youtube.com, put at `/data/music/cookies.txt` (0600); the app
//!   says plainly that a third-party client can break YouTube's terms.
//!   The cookies only ever go to curl and yt-dlp in files, never on a
//!   command line.
//!
//! On a computer (the simulator) a made-up catalogue stands in, with covers
//! drawn from each song's name, and songs "play" silently.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::git::Job;
use crate::photos::Picture;
use crate::platform::{Key, Media};
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const DIR: &str = "/data/music";
const COOKIES: &str = "/data/music/cookies.txt";
/// The image's yt-dlp, and a newer copy brought to /data (Account › Stream finder).
const RESOLVER: &str = "/usr/libexec/catalyst/yt-dlp";
const RESOLVER_UPDATED: &str = "/data/music/yt-dlp";
const QJS: &str = "/usr/bin/qjs";
const FFMPEG: &str = "/usr/bin/ffmpeg";
/// Covers, fetched for the page and kept only in memory once read.
const ART_DIR: &str = "/tmp/catalyst-music";
const ORIGIN: &str = "https://music.youtube.com";
/// Search's "songs" filter, as YouTube Music's own page asks for it.
const SONGS_ONLY: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
/// A stream's address is good for about six hours; found again after four.
const URL_KEEPS: Duration = Duration::from_secs(4 * 3600);
/// Back to the start of a song rather than the one before, after this long.
const RESTART_AFTER: f32 = 3.0;
const REQUEST_SECONDS: &str = "20";

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
            let third = col(2);
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
    std::fs::create_dir_all(crate::config_dir()).ok();
    let file = crate::config_dir().join(format!(".music-request-{}", std::process::id()));
    crate::sys::write_private(&file, headers.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    let url = format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false");
    let output = (|| {
        let mut child = Command::new("curl")
            .args(["-sS", "--compressed", "--max-time", REQUEST_SECONDS, "--proto", "=https", "-H", &format!("@{}", file.display()), "--data-binary", "@-", "-w", "\n%{http_code}", &url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "curl could not start".to_owned())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the request could not be sent".to_owned())?;
        }
        child.wait_with_output().map_err(|_| "the request stopped".to_owned())
    })();
    std::fs::remove_file(&file).ok();
    let output = output?;
    if !output.status.success() {
        return Err(match output.status.code() {
            Some(6) | Some(7) => "no internet".into(),
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

// -------------------------------------------------------------- streams ---

fn resolver() -> &'static str {
    if Path::new(RESOLVER_UPDATED).exists() {
        RESOLVER_UPDATED
    } else {
        RESOLVER
    }
}

/// A song's audio-only stream: Opus where there is one (YouTube's best for
/// its size, and 48 kHz already), else AAC. yt-dlp, a few seconds on the
/// handheld; its player-challenge work is kept in /data/music/cache.
fn resolve(id: &str) -> Result<String, String> {
    if !on_device() {
        std::thread::sleep(Duration::from_millis(600));
        return Ok(format!("demo:{id}"));
    }
    std::fs::create_dir_all(Path::new(DIR).join("cache")).ok();
    let js = format!("quickjs:{QJS}");
    let mut cmd = Command::new("python3");
    cmd.arg(resolver())
        .args(["--quiet", "--no-warnings", "--no-playlist", "--no-progress", "-f", "bestaudio[acodec=opus]/bestaudio[ext=m4a]/bestaudio", "-g"])
        .args(["--js-runtimes", &js, "--cache-dir", "/data/music/cache"]);
    if signed_in() {
        cmd.args(["--cookies", COOKIES]);
    }
    let out = cmd.arg(format!("{ORIGIN}/watch?v={id}")).stdin(Stdio::null()).output().map_err(|_| "the stream finder could not start".to_owned())?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().find(|l| l.starts_with("https://")) {
        Some(url) if out.status.success() => Ok(url.trim().to_owned()),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            let why = err.lines().filter(|l| l.contains("ERROR")).last().unwrap_or("").trim();
            Err(match why {
                w if w.contains("Sign in to confirm your age") => "this song is age-restricted: sign in to play it".into(),
                w if w.contains("Video unavailable") || w.contains("not available") => "this song is not available here".into(),
                w if w.contains("Unable to download") || w.contains("urlopen") => "no internet".into(),
                "" => "the song's stream could not be found".into(),
                w => format!("the stream finder: {}", w.trim_start_matches("ERROR:").trim().chars().take(80).collect::<String>()),
            })
        }
    }
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
    let out = Command::new("python3").args([RESOLVER_UPDATED, "--update-to", "stable"]).stdin(Stdio::null()).output().map_err(|_| "the stream finder could not start".to_owned())?;
    if !out.status.success() {
        return Err("the update could not be fetched".into());
    }
    resolver_version().ok_or_else(|| "the stream finder did not start after updating".into())
}

fn resolver_version() -> Option<String> {
    if !on_device() {
        return Some("2026.10.26".into());
    }
    let out = Command::new("python3").args([resolver(), "--version"]).stdin(Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------- player ---

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Nothing in the queue.
    Empty,
    /// Finding the stream, or its first sound on the way.
    Loading,
    Playing,
    Paused,
    /// Stopped by a problem, said here; Enter tries again.
    Failed(String),
}

/// What the player is doing, for every screen to read.
#[derive(Clone, Debug)]
pub struct Now {
    pub queue: Vec<Track>,
    pub at: usize,
    pub phase: Phase,
    /// Where in the song, in seconds.
    pub position: f32,
    /// Its stream is being found (the slow part: seconds on the handheld).
    pub finding: bool,
}

impl Now {
    pub fn track(&self) -> Option<&Track> {
        self.queue.get(self.at)
    }
}

enum Cmd {
    /// A list, played from one of its songs; the song's radio after it if
    /// it is alone.
    Play(Vec<Track>, usize),
    Toggle,
    Next,
    Previous,
    Seek(f32),
    Jump(usize),
    /// After the song playing now.
    PlayNext(Track),
    Remove(usize),
    Resolved(String, Result<String, String>),
    /// A song's radio arrived: added after it, if it is still the one.
    Radio(String, Vec<Track>),
}

/// The sound being made: ffmpeg playing to ALSA, or (in the simulator) a
/// clock.
enum Sound {
    Process { child: Child, error: Arc<Mutex<String>> },
    Silent { since: Instant },
}

struct Player {
    tx: Sender<Cmd>,
    state: Arc<Mutex<Shared>>,
}

struct Shared {
    queue: Vec<Track>,
    at: usize,
    phase: Phase,
    /// Where the song was when the sound now playing started.
    offset: f32,
    /// How far the sound now playing has got, in microseconds.
    played: Arc<AtomicU64>,
    playing_since: Option<Instant>,
    finding: bool,
}

impl Shared {
    fn position(&self) -> f32 {
        match self.phase {
            Phase::Playing => {
                let played = self.played.load(Ordering::Relaxed) as f32 / 1e6;
                // The simulator's songs run on the clock.
                let clock = if on_device() { 0.0 } else { self.playing_since.map_or(0.0, |t| t.elapsed().as_secs_f32()) };
                self.offset + played.max(clock)
            }
            _ => self.offset,
        }
    }
}

static PLAYER: OnceLock<Player> = OnceLock::new();

fn player() -> &'static Player {
    PLAYER.get_or_init(|| {
        let (tx, rx) = channel::<Cmd>();
        let state = Arc::new(Mutex::new(Shared { queue: Vec::new(), at: 0, phase: Phase::Empty, offset: 0.0, played: Arc::new(AtomicU64::new(0)), playing_since: None, finding: false }));
        let worker = Worker { tx: tx.clone(), state: state.clone(), sound: None, urls: HashMap::new(), finding: None, waiting: VecDeque::new() };
        std::thread::Builder::new().name("music".into()).stack_size(256 * 1024).spawn(move || worker.run(rx)).ok();
        Player { tx, state }
    })
}

/// The player now: its queue, the song, where it is.
pub fn now() -> Now {
    let Some(p) = PLAYER.get() else {
        return Now { queue: Vec::new(), at: 0, phase: Phase::Empty, position: 0.0, finding: false };
    };
    let s = p.state.lock().unwrap();
    Now { queue: s.queue.clone(), at: s.at, phase: s.phase.clone(), position: s.position(), finding: s.finding }
}

fn send(cmd: Cmd) {
    player().tx.send(cmd).ok();
}

/// Music is playing, or about to (a stream being found, or the first
/// sound on its way): deep standby waits for it to stop.
pub fn playing() -> bool {
    // The unit tests' player (the simulator's) plays beside tests of deep
    // standby, which must not see it.
    if cfg!(test) {
        return false;
    }
    PLAYER.get().is_some_and(|p| matches!(p.state.lock().unwrap().phase, Phase::Playing | Phase::Loading))
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
            Some(if n.phase == Phase::Playing { "paused".into() } else { format!("playing {}", track.title) })
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

struct Worker {
    tx: Sender<Cmd>,
    state: Arc<Mutex<Shared>>,
    sound: Option<Sound>,
    /// Streams found, and when.
    urls: HashMap<String, (String, Instant)>,
    /// The stream being found now, and those waiting their turn.
    finding: Option<String>,
    waiting: VecDeque<String>,
}

impl Worker {
    fn run(mut self, rx: std::sync::mpsc::Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.watch();
        }
        self.stop_sound();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play(list, at) => {
                let alone = list.len() == 1;
                let id = list.get(at).map(|t| t.id.clone());
                {
                    let mut s = self.state.lock().unwrap();
                    s.queue = list;
                    s.at = at;
                }
                self.start(0.0);
                if let (true, Some(id)) = (alone, id) {
                    let tx = self.tx.clone();
                    std::thread::Builder::new()
                        .name("music-radio".into())
                        .stack_size(256 * 1024)
                        .spawn(move || {
                            if let Ok(next) = up_next(&id) {
                                tx.send(Cmd::Radio(id, next)).ok();
                            }
                        })
                        .ok();
                }
            }
            Cmd::Radio(id, next) => {
                let mut s = self.state.lock().unwrap();
                if s.queue.len() == 1 && s.queue[0].id == id {
                    know(&next);
                    s.queue.extend(next.into_iter().filter(|t| t.id != id).take(40));
                }
                drop(s);
                self.prefetch();
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
            Cmd::Next => self.step(1),
            Cmd::Previous => {
                let (position, at) = {
                    let s = self.state.lock().unwrap();
                    (s.position(), s.at)
                };
                if position > RESTART_AFTER || at == 0 {
                    self.start(0.0);
                } else {
                    self.step(-1);
                }
            }
            Cmd::Seek(delta) => {
                let (position, length) = {
                    let s = self.state.lock().unwrap();
                    (s.position(), s.queue.get(s.at).and_then(|t| t.seconds).unwrap_or(u32::MAX) as f32)
                };
                self.start((position + delta).clamp(0.0, (length - 1.0).max(0.0)));
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
                } else {
                    self.prefetch();
                }
            }
            Cmd::Remove(i) => {
                let playing = {
                    let mut s = self.state.lock().unwrap();
                    if i >= s.queue.len() {
                        return;
                    }
                    s.queue.remove(i);
                    let playing = i == s.at;
                    if i < s.at {
                        s.at -= 1;
                    }
                    if s.queue.is_empty() {
                        s.phase = Phase::Empty;
                        s.at = 0;
                    }
                    s.at = s.at.min(s.queue.len().saturating_sub(1));
                    playing && !s.queue.is_empty()
                };
                if playing {
                    self.start(0.0);
                } else if self.state.lock().unwrap().queue.is_empty() {
                    self.stop_sound();
                }
            }
            Cmd::Resolved(id, found) => {
                if self.finding.as_deref() == Some(id.as_str()) {
                    self.finding = None;
                }
                self.find_next();
                let current = {
                    let s = self.state.lock().unwrap();
                    s.queue.get(s.at).map(|t| t.id.clone())
                };
                match found {
                    Ok(url) => {
                        self.urls.insert(id.clone(), (url, Instant::now()));
                        let loading = self.state.lock().unwrap().phase == Phase::Loading;
                        if current.as_deref() == Some(id.as_str()) && loading && self.sound.is_none() {
                            let at = self.state.lock().unwrap().offset;
                            self.play(&id, at);
                        }
                    }
                    Err(why) if current.as_deref() == Some(id.as_str()) => {
                        let mut s = self.state.lock().unwrap();
                        s.phase = Phase::Failed(why);
                        s.finding = false;
                    }
                    Err(_) => {}
                }
            }
        }
    }

    /// The song `step` along the queue, from its start; past the end, stopped.
    fn step(&mut self, step: i32) {
        let ok = {
            let mut s = self.state.lock().unwrap();
            let to = s.at as i32 + step;
            let ok = to >= 0 && (to as usize) < s.queue.len();
            if ok {
                s.at = to as usize;
            }
            ok
        };
        if ok {
            self.start(0.0);
        } else {
            self.stop_sound();
            let mut s = self.state.lock().unwrap();
            if s.phase != Phase::Empty {
                s.phase = Phase::Paused;
                s.offset = 0.0;
            }
        }
    }

    /// The song at the queue's place, from `at` seconds: its stream found
    /// first if it is not to hand.
    fn start(&mut self, at: f32) {
        self.stop_sound();
        let id = {
            let mut s = self.state.lock().unwrap();
            let Some(track) = s.queue.get(s.at) else {
                s.phase = Phase::Empty;
                return;
            };
            let id = track.id.clone();
            s.offset = at;
            s.phase = Phase::Loading;
            s.played.store(0, Ordering::Relaxed);
            id
        };
        if self.urls.get(&id).is_some_and(|(_, when)| when.elapsed() < URL_KEEPS) {
            self.play(&id, at);
        } else {
            self.urls.remove(&id);
            self.state.lock().unwrap().finding = true;
            self.ask(&id, true);
        }
    }

    /// A song's stream to be found. One at a time: finding one runs yt-dlp
    /// and QuickJS, which take up to 200 MB between them for a few seconds,
    /// so two at once could crowd out everything else. `now`: the song
    /// wanted this moment, ahead of any found in advance.
    fn ask(&mut self, id: &str, now: bool) {
        if self.finding.as_deref() == Some(id) {
            return;
        }
        self.waiting.retain(|w| w != id);
        if now {
            self.waiting.push_front(id.to_owned());
        } else {
            self.waiting.push_back(id.to_owned());
        }
        self.find_next();
    }

    fn find_next(&mut self) {
        if self.finding.is_some() {
            return;
        }
        let Some(id) = self.waiting.pop_front() else { return };
        self.finding = Some(id.clone());
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("music-stream".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let found = resolve(&id);
                tx.send(Cmd::Resolved(id, found)).ok();
            })
            .ok();
    }

    /// The next song's stream, found while this one plays.
    fn prefetch(&mut self) {
        let next = {
            let s = self.state.lock().unwrap();
            s.queue.get(s.at + 1).map(|t| t.id.clone())
        };
        if let Some(id) = next {
            if !self.urls.contains_key(&id) {
                self.ask(&id, false);
            }
        }
    }

    fn play(&mut self, id: &str, at: f32) {
        let Some((url, _)) = self.urls.get(id).cloned() else { return };
        let played = Arc::new(AtomicU64::new(0));
        let sound = if on_device() {
            match spawn_ffmpeg(&url, at, played.clone()) {
                Ok(sound) => sound,
                Err(why) => {
                    self.state.lock().unwrap().phase = Phase::Failed(why);
                    return;
                }
            }
        } else {
            Sound::Silent { since: Instant::now() }
        };
        self.sound = Some(sound);
        {
            let mut s = self.state.lock().unwrap();
            s.played = played;
            s.phase = Phase::Playing;
            s.playing_since = Some(Instant::now());
            s.finding = false;
        }
        self.prefetch();
    }

    fn pause(&mut self) {
        let position = self.state.lock().unwrap().position();
        self.stop_sound();
        let mut s = self.state.lock().unwrap();
        s.offset = position;
        s.phase = Phase::Paused;
    }

    /// ffmpeg stopped (the device closed, so the amplifier can go off).
    fn stop_sound(&mut self) {
        if let Some(Sound::Process { mut child, .. }) = self.sound.take() {
            stop_child(&mut child);
        }
    }

    /// A song that ended moves on; one that failed says why (a stream whose
    /// address went stale is found again, once).
    fn watch(&mut self) {
        let ended = match &mut self.sound {
            Some(Sound::Process { child, error, .. }) => match child.try_wait() {
                Ok(Some(status)) if status.success() => Some(Ok(())),
                Ok(Some(_)) => Some(Err(error.lock().unwrap().clone())),
                _ => None,
            },
            Some(Sound::Silent { since }) => {
                let s = self.state.lock().unwrap();
                let length = s.queue.get(s.at).and_then(|t| t.seconds).unwrap_or(200) as f32;
                (s.offset + since.elapsed().as_secs_f32() >= length).then_some(Ok(()))
            }
            None => None,
        };
        match ended {
            Some(Ok(())) => {
                self.sound = None;
                self.step(1);
            }
            Some(Err(why)) => {
                self.sound = None;
                let (id, position, played) = {
                    let s = self.state.lock().unwrap();
                    (s.queue.get(s.at).map(|t| t.id.clone()).unwrap_or_default(), s.position(), s.played.load(Ordering::Relaxed))
                };
                let stale = why.contains("403") || why.contains("Forbidden") || why.contains("404");
                if stale && self.urls.remove(&id).is_some() {
                    // Found again, and on from where it was.
                    self.start(position);
                } else {
                    let said = if played == 0 && why.is_empty() { "the song could not be played".into() } else { short_ffmpeg_error(&why) };
                    let mut s = self.state.lock().unwrap();
                    s.offset = position;
                    s.phase = Phase::Failed(said);
                }
            }
            None => {}
        }
    }
}

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        // ffmpeg finishes cleanly on SIGTERM, closing the sound device.
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.kill().ok();
    child.wait().ok();
}

fn short_ffmpeg_error(text: &str) -> String {
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    match line {
        l if l.contains("Device or resource busy") => "the sound device is busy".into(),
        l if l.contains("Connection") || l.contains("resolve") || l.contains("Network") => "the connection dropped".into(),
        "" => "the song stopped".into(),
        l => format!("the song stopped: {}", l.chars().take(70).collect::<String>()),
    }
}

/// ffmpeg playing a stream to ALSA's "default" from `at` seconds, telling
/// how far it has got on its standard output.
fn spawn_ffmpeg(url: &str, at: f32, played: Arc<AtomicU64>) -> Result<Sound, String> {
    let start = format!("{at:.2}");
    let mut child = Command::new(FFMPEG)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "4"])
        .args(["-ss", &start, "-i", url, "-vn", "-af", "aresample=48000", "-ac", "2"])
        .args(["-progress", "pipe:1", "-stats_period", "0.5", "-f", "alsa", "default"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "ffmpeg could not start".to_owned())?;
    if let Some(out) = child.stdout.take() {
        std::thread::Builder::new()
            .name("music-progress".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                        played.store(us.max(0) as u64, Ordering::Relaxed);
                    }
                }
            })
            .ok();
    }
    let error = Arc::new(Mutex::new(String::new()));
    if let Some(mut err) = child.stderr.take() {
        let error = error.clone();
        std::thread::Builder::new()
            .name("music-errors".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let mut text = String::new();
                err.read_to_string(&mut text).ok();
                *error.lock().unwrap() = text.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
            })
            .ok();
    }
    Ok(Sound::Process { child, error })
}

// ------------------------------------------------------------------- art ---

/// Covers, fetched one at a time and kept decoded (a few dozen KB each).
struct Covers {
    ready: HashMap<String, Arc<Picture>>,
    order: VecDeque<String>,
    wanted: VecDeque<(String, String)>,
    busy: bool,
}

static COVERS: Mutex<Option<Covers>> = Mutex::new(None);
const COVERS_KEPT: usize = 24;
/// Covers are decoded at least this big (the now-playing page's is 96 px).
const COVER_PX: usize = 112;

/// A song's cover if it is to hand; else asked for, and `None` for now.
fn cover(track: &Track) -> Option<Arc<Picture>> {
    let mut covers = COVERS.lock().unwrap();
    let c = covers.get_or_insert_with(|| Covers { ready: HashMap::new(), order: VecDeque::new(), wanted: VecDeque::new(), busy: false });
    if let Some(p) = c.ready.get(&track.id) {
        return Some(p.clone());
    }
    if !on_device() {
        let p = Arc::new(demo_cover(track));
        c.ready.insert(track.id.clone(), p.clone());
        return Some(p);
    }
    if track.art.is_empty() || c.wanted.iter().any(|(id, _)| *id == track.id) {
        return None;
    }
    c.wanted.push_back((track.id.clone(), track.art.clone()));
    if !c.busy {
        c.busy = true;
        std::thread::Builder::new().name("music-covers".into()).stack_size(256 * 1024).spawn(fetch_covers).ok();
    }
    None
}

fn fetch_covers() {
    loop {
        let next = {
            let mut covers = COVERS.lock().unwrap();
            let c = covers.as_mut().unwrap();
            match c.wanted.pop_front() {
                Some(w) => w,
                None => {
                    c.busy = false;
                    return;
                }
            }
        };
        let (id, url) = next;
        let picture = fetch_cover(&id, &url).unwrap_or_default();
        let mut covers = COVERS.lock().unwrap();
        let c = covers.as_mut().unwrap();
        c.ready.insert(id.clone(), Arc::new(picture));
        c.order.push_back(id);
        while c.order.len() > COVERS_KEPT {
            if let Some(old) = c.order.pop_front() {
                c.ready.remove(&old);
            }
        }
    }
}

fn fetch_cover(id: &str, url: &str) -> Option<Picture> {
    if !url.starts_with("https://") {
        return None;
    }
    std::fs::create_dir_all(ART_DIR).ok();
    let path = PathBuf::from(ART_DIR).join(format!("{}.jpg", id.replace(['/', '.'], "_")));
    let ok = Command::new("curl").args(["-sS", "-f", "--max-time", "15", "--proto", "=https", "-o"]).arg(&path).arg(url).stdin(Stdio::null()).status().is_ok_and(|s| s.success());
    let picture = if ok { crate::photos::decode(&path, COVER_PX, COVER_PX).ok() } else { None };
    std::fs::remove_file(&path).ok();
    picture
}

// ------------------------------------------------------------------ demo ---

fn demo_catalogue() -> Vec<Track> {
    let t = |id: &str, title: &str, artist: &str, album: &str, s: u32| Track { id: id.into(), title: title.into(), artist: artist.into(), album: album.into(), seconds: Some(s), art: String::new() };
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
        KNOWN.lock().unwrap().iter().map(|t| Item::new(format!("song:{}", t.id), t.title.clone(), "music_note", weight::OTHER).detail(format!("song · {}", t.artist.to_lowercase())).words(t.artist.clone())).collect()
    }

    fn stamp(&self) -> Option<u64> {
        Some(KNOWN_CHANGED.load(Ordering::Relaxed))
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
        let id = item.id.strip_prefix("song:").unwrap_or(&item.id);
        let track = KNOWN.lock().unwrap().iter().find(|t| t.id == id).cloned().ok_or_else(|| "that song is not to hand: search for it in Music".to_owned())?;
        send(Cmd::Play(vec![track], 0));
        Ok(Box::new(MusicScreen::new()))
    }
}

// ---------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    /// The start: the song playing, search, quick picks or likes, the account.
    Start,
    Search,
    Playing,
    Queue,
    Account,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum StartRow {
    NowPlaying,
    Search,
    Song(usize),
    Account,
}

/// The Music app: what is playing, search, the queue, the account.
pub struct MusicScreen {
    page: Page,
    list: ListState,
    /// The start page's songs (quick picks, or likes), and their loading.
    picks: Vec<Track>,
    /// What they are: "quick picks", "your likes", a playlist's name.
    picks_are: String,
    picks_job: Option<Job<Result<(String, Vec<Track>), String>>>,
    picks_error: Option<String>,
    /// Search: what is typed, whether typing, what was found.
    query: String,
    typing: bool,
    results: Vec<Track>,
    searched: String,
    search_job: Option<Job<Result<Vec<Track>, String>>>,
    /// Account: the stream finder's version, and work on it.
    version: Option<Job<Option<String>>>,
    version_seen: Option<String>,
    updating: Option<Job<Result<String, String>>>,
    armed: bool,
    /// The progress bar's fill, gliding.
    bar: detent::Animated,
    last_phase: Phase,
    changed: bool,
}

impl MusicScreen {
    /// Opened on a search for `query` (from Spotlight).
    pub fn searching(query: &str) -> Self {
        let mut screen = Self::new();
        screen.go(Page::Search);
        screen.query = query.to_owned();
        screen.run_search();
        screen
    }

    pub fn new() -> Self {
        let playing = now().track().is_some();
        let mut screen = MusicScreen {
            page: if playing { Page::Playing } else { Page::Start },
            list: ListState::default(),
            picks: Vec::new(),
            picks_are: "quick picks".into(),
            picks_job: Some(Job::start(start_page)),
            picks_error: None,
            query: String::new(),
            typing: false,
            results: Vec::new(),
            searched: String::new(),
            search_job: None,
            version: None,
            version_seen: None,
            updating: None,
            armed: false,
            bar: detent::Animated::new(0.0, detent::role::hold()).with_rest(0.002),
            last_phase: Phase::Empty,
            changed: true,
        };
        screen.list.focus = if playing { 0 } else { 1 };
        screen
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.list = ListState::default();
        self.armed = false;
        match page {
            Page::Start => self.list.focus = if now().track().is_some() { 0 } else { 1 },
            Page::Queue => self.list.focus = now().at,
            Page::Account => {
                if self.version_seen.is_none() && self.version.is_none() {
                    self.version = Some(Job::start(resolver_version));
                }
            }
            _ => {}
        }
        self.changed = true;
    }

    fn start_rows(&self) -> Vec<StartRow> {
        let mut rows = Vec::new();
        if now().track().is_some() {
            rows.push(StartRow::NowPlaying);
        }
        rows.push(StartRow::Search);
        rows.extend((0..self.picks.len()).map(StartRow::Song));
        rows.push(StartRow::Account);
        rows
    }

    /// A song from a list, played with the rest of the list after it (or
    /// its radio, alone).
    fn play(&mut self, list: &[Track], at: usize, radio: bool) {
        let Some(track) = list.get(at) else { return };
        if radio {
            send(Cmd::Play(vec![track.clone()], 0));
        } else {
            send(Cmd::Play(list.to_vec(), at));
        }
        self.go(Page::Playing);
    }

    fn run_search(&mut self) {
        let q = self.query.trim().to_owned();
        if q.is_empty() {
            return;
        }
        self.searched = q.clone();
        self.results.clear();
        self.search_job = Some(Job::start(move || search(&q)));
        self.list = ListState::default();
    }

    fn title_for(&self) -> &'static str {
        match self.page {
            Page::Start | Page::Playing => "Music",
            Page::Search => "Search",
            Page::Queue => "Up next",
            Page::Account => "Account",
        }
    }
}

impl Screen for MusicScreen {
    fn title(&self) -> &str {
        self.title_for()
    }

    fn wants_text(&self) -> bool {
        self.typing
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        self.changed = true;
        let n = now();
        // Typing a search: letters into the field, Enter looks.
        if self.typing {
            match key {
                Key::Char(c) => self.query.push(c),
                Key::Backspace => {
                    self.query.pop();
                }
                Key::Enter => {
                    self.typing = false;
                    self.run_search();
                }
                Key::Back | Key::Leave => {
                    self.typing = false;
                    if self.results.is_empty() {
                        self.go(Page::Start);
                    }
                }
                Key::Down if !self.results.is_empty() => self.typing = false,
                _ => {}
            }
            return Nav::Stay;
        }
        match self.page {
            Page::Playing => match key {
                Key::Back => return Nav::Pop,
                Key::Enter | Key::Char(' ') => send(Cmd::Toggle),
                Key::Left => send(Cmd::Previous),
                Key::Right => send(Cmd::Next),
                Key::Char(',') => send(Cmd::Seek(-10.0)),
                Key::Char('.') => send(Cmd::Seek(10.0)),
                Key::Down | Key::Char('u') => self.go(Page::Queue),
                Key::Up | Key::Tab => self.go(Page::Start),
                Key::Char('/') => {
                    self.go(Page::Search);
                    self.typing = true;
                }
                _ => {}
            },
            Page::Start => {
                let rows = self.start_rows();
                let row = rows.get(self.list.focus).copied().unwrap_or(StartRow::Search);
                match key {
                    Key::Back => return Nav::Pop,
                    Key::Tab if n.track().is_some() => self.go(Page::Playing),
                    Key::Enter | Key::Right => match row {
                        StartRow::NowPlaying => self.go(Page::Playing),
                        StartRow::Search => {
                            self.go(Page::Search);
                            self.typing = true;
                        }
                        StartRow::Song(i) => {
                            let picks = self.picks.clone();
                            // Quick picks are a mix already: played as a list; likes too.
                            self.play(&picks, i, false);
                        }
                        StartRow::Account => self.go(Page::Account),
                    },
                    Key::Char('n') if matches!(row, StartRow::Song(_)) => {
                        if let StartRow::Song(i) = row {
                            send(Cmd::PlayNext(self.picks[i].clone()));
                            *cx.say = Some(format!("next: {}", self.picks[i].title));
                        }
                    }
                    // Typing anywhere on the start page starts a search.
                    Key::Char(c) if c.is_alphanumeric() => {
                        self.go(Page::Search);
                        self.query = c.to_string();
                        self.typing = true;
                    }
                    other => crate::screens::list_key(&mut self.list, other, rows.len()),
                }
            }
            Page::Search => match key {
                Key::Back => self.go(Page::Start),
                Key::Up if self.list.focus == 0 => self.typing = true,
                Key::Char('/') => self.typing = true,
                Key::Enter | Key::Right => {
                    let results = self.results.clone();
                    self.play(&results, self.list.focus, true);
                }
                Key::Char('n') => {
                    if let Some(t) = self.results.get(self.list.focus).cloned() {
                        *cx.say = Some(format!("next: {}", t.title));
                        send(Cmd::PlayNext(t));
                    }
                }
                Key::Char(c) if c.is_alphanumeric() => {
                    self.query = c.to_string();
                    self.typing = true;
                }
                other => crate::screens::list_key(&mut self.list, other, self.results.len()),
            },
            Page::Queue => match key {
                Key::Back => self.go(Page::Playing),
                Key::Enter | Key::Right => {
                    send(Cmd::Jump(self.list.focus));
                    self.go(Page::Playing);
                }
                Key::Delete | Key::Backspace | Key::Char('x') => send(Cmd::Remove(self.list.focus)),
                other => crate::screens::list_key(&mut self.list, other, n.queue.len()),
            },
            Page::Account => {
                let armed = std::mem::take(&mut self.armed);
                match key {
                    Key::Back => self.go(Page::Start),
                    Key::Enter if self.list.focus == 1 && self.updating.is_none() => self.updating = Some(Job::start(update_resolver)),
                    Key::Enter if self.list.focus == 2 && signed_in() => {
                        if armed {
                            match sign_out() {
                                Ok(()) => *cx.say = Some("signed out".into()),
                                Err(why) => *cx.say = Some(why),
                            }
                            self.picks_job = Some(Job::start(start_page));
                        } else {
                            self.armed = true;
                        }
                    }
                    other => crate::screens::list_key(&mut self.list, other, if signed_in() { 3 } else { 2 }),
                }
            }
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.picks_job.as_ref().and_then(Job::get) {
            self.picks_job = None;
            match done {
                Ok((what, t)) => {
                    know(&t);
                    self.picks = t;
                    self.picks_are = what;
                    self.picks_error = None;
                }
                Err(why) => self.picks_error = Some(why),
            }
            self.changed = true;
        }
        if let Some(done) = self.search_job.as_ref().and_then(Job::get) {
            self.search_job = None;
            match done {
                Ok(t) if t.is_empty() => *cx.say = Some("no songs found".into()),
                Ok(t) => {
                    know(&t);
                    self.results = t;
                }
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        if let Some(done) = self.version.as_ref().and_then(Job::get) {
            self.version = None;
            self.version_seen = done.or(Some("–".into()));
            self.changed = true;
        }
        if let Some(done) = self.updating.as_ref().and_then(Job::get) {
            self.updating = None;
            match done {
                Ok(v) => {
                    *cx.say = Some(format!("stream finder {v}"));
                    self.version_seen = Some(v);
                }
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        // A problem playing is said once, as it happens.
        let phase = now().phase;
        if phase != self.last_phase {
            if let Phase::Failed(why) = &phase {
                *cx.say = Some(why.clone());
            }
            self.last_phase = phase;
            self.changed = true;
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let n = now();
        let fraction = match n.track().and_then(|t| t.seconds) {
            Some(len) if len > 0 => n.position / len as f32,
            _ => 0.0,
        };
        if (self.bar.target() - fraction).abs() > 0.0005 {
            // A jump (a new song, a seek) glides; play itself moves smoothly on its own.
            if (self.bar.target() - fraction).abs() > 0.05 {
                self.bar.set(fraction);
            } else {
                self.bar.jump(fraction);
            }
        }
        let moving = self.bar.tick(dt);
        std::mem::take(&mut self.changed) | moving | (self.page == Page::Playing && n.phase == Phase::Playing) | self.picks_job.is_some() | self.search_job.is_some()
    }

    fn busy(&self) -> bool {
        true
    }

    fn keeps_awake(&self) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        match self.page {
            Page::Playing => self.draw_playing(g),
            Page::Start => self.draw_start(g),
            Page::Search => self.draw_search(g),
            Page::Queue => self.draw_queue(g),
            Page::Account => self.draw_account(g),
        }
    }
}

impl MusicScreen {
    fn draw_playing(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let n = now();
        let Some(track) = n.track().cloned() else {
            g.empty("music_note", "nothing playing · esc, then pick a song");
            return;
        };
        let top = g.top() + 2.0;
        let side = 96.0;
        let art = Rect::new(PAD, top, side, side);
        match cover(&track) {
            Some(p) if !p.is_empty() => crate::photos::draw_picture(g, art, &p, [0.0, 0.0, p.w as f32, p.h as f32], 14.0, 1.0),
            _ => {
                g.surface(art, 14.0, t.surface2);
                g.icon("music_note", true, art.cx(), art.cy(), 34.0, t.faint);
            }
        }
        // To the right: the song, the artist and album, where it is, the transport.
        let x = art.right() + 12.0;
        let w = W - PAD - x;
        g.text_fit(x, top + ty.name.size * 0.6, &track.title, ty.name, t.ink, w);
        g.text_fit(x, top + ty.name.size + 4.0 + ty.body.size * 0.55, &track.artist, ty.body, t.dim, w);
        if !track.album.is_empty() {
            g.text_fit(x, top + ty.name.size + ty.body.size + 8.0 + ty.caption.size * 0.5, &track.album.to_lowercase(), ty.caption, t.faint, w);
        }
        let bar_y = top + 56.0;
        g.meter(Rect::new(x, bar_y, w, 5.0), self.bar.value(), t.ice);
        let length = track.seconds.map(|s| s as f32);
        let left = clock(n.position);
        let right = match length {
            Some(l) => format!("-{}", clock(l - n.position)),
            None => String::new(),
        };
        let times_y = bar_y + 7.0 + ty.caption.size * 0.5;
        g.text_at(x, times_y, Align::Left, &left, ty.caption, t.dim);
        g.text_at(W - PAD, times_y, Align::Right, &right, ty.caption, t.dim);
        // The transport: previous, play or pause (the primary action, in signal), next.
        let cy = times_y + 25.0;
        let mid = x + w * 0.5;
        let playing = matches!(n.phase, Phase::Playing | Phase::Loading);
        for (dx, icon, r) in [(-46.0, "skip_previous", 13.0), (0.0, if playing { "pause" } else { "play_arrow" }, 16.0), (46.0, "skip_next", 13.0)] {
            let primary = dx == 0.0;
            g.dot(mid + dx, cy, r, if primary { t.signal } else { t.surface2 });
            g.icon(icon, true, mid + dx, cy, if primary { 20.0 } else { 17.0 }, if primary { t.on_signal } else { t.ink });
        }
        if n.phase == Phase::Loading {
            g.text_at(mid, cy - 25.0, Align::Center, if n.finding { "finding it…" } else { "loading…" }, ty.caption, t.dim);
        }
        let upcoming = n.queue.get(n.at + 1).map(|t| format!("next: {}", t.title.to_lowercase()));
        let hint = match (&n.phase, upcoming) {
            (Phase::Failed(_), _) => vec!["enter: try again · ←→: skip".to_owned(), "enter: try again".to_owned()],
            (_, Some(next)) => vec![format!("{next} · ↓ up next"), "enter: pause · ←→: skip · ↓: up next".to_owned(), "enter · ←→ · ↓".to_owned()],
            _ => vec!["enter: pause · ←: back · /: search".to_owned(), "enter · ← · /".to_owned()],
        };
        let choices: Vec<&str> = hint.iter().map(String::as_str).collect();
        g.hint_any(&choices);
    }

    fn draw_start(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let rows = self.start_rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        let n = now();
        let focus = self.list.focus;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            match rows[i] {
                StartRow::NowPlaying => {
                    let track = n.track().cloned().unwrap_or_default();
                    let icon = if n.phase == Phase::Playing { "graphic_eq" } else { "pause" };
                    crate::bluetooth::icon_row(g, r, icon, n.phase == Phase::Playing, &track.title, &clock(n.position), t.ink, i == focus);
                }
                StartRow::Search => crate::bluetooth::icon_row(g, r, "search", false, "Search songs", "", t.dim, i == focus),
                StartRow::Song(s) => {
                    let track = &self.picks[s];
                    song_row(g, r, track, i == focus);
                }
                StartRow::Account => g.row(r, "Account", if signed_in() { "signed in ›" } else { "signed out ›" }, t.dim, i == focus),
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let row = rows.get(focus).copied();
        let songs = format!("{} · enter: play · n: play next", self.picks_are);
        let hint: &[&str] = match row {
            Some(StartRow::Song(_)) => &[&songs, "enter: play · n: next"],
            Some(StartRow::NowPlaying) => &["enter: now playing · tab, anywhere here", "enter: now playing"],
            Some(StartRow::Account) if signed_in() => &["signed in with your browser's cookies", "signed in"],
            Some(StartRow::Account) => &["signed out unless you sign in", "signed out"],
            _ => &["type to search youtube music", "type to search"],
        };
        if self.picks_job.is_some() {
            g.hint("loading…");
        } else if let Some(why) = &self.picks_error {
            let why = why.clone();
            g.hint(&why);
        } else {
            g.hint_any(hint);
        }
    }

    fn draw_search(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        // The field: what is typed, with a caret while typing.
        let field = Rect::new(PAD, top, W - 2.0 * PAD, g.row_h());
        g.surface(field, field.h * 0.5, if self.typing { t.surface2 } else { t.surface1 });
        g.icon("search", false, field.x + 14.0, field.cy(), 14.0, t.dim);
        let shown = if self.query.is_empty() && !self.typing { "songs, artists".to_owned() } else { self.query.clone() };
        let text_w = g.text_fit(field.x + 27.0, field.cy(), &shown, ty.body, if self.query.is_empty() { t.faint } else { t.ink }, field.w - 40.0);
        if self.typing {
            let caret_x = field.x + 27.0 + text_w.min(field.w - 40.0) + 2.0;
            g.surface(Rect::new(caret_x, field.cy() - ty.body.size * 0.45, 1.5, ty.body.size * 0.9), 0.5, t.signal);
            g.focus_ring(field, field.h * 0.5);
        }
        let (list_top, row_h, bottom) = (field.bottom() + 5.0, g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if self.search_job.is_some() {
            g.text_at(W * 0.5, (list_top + bottom) * 0.5, Align::Center, "looking…", ty.caption, t.dim);
        }
        let focus = if self.typing { usize::MAX } else { self.list.focus };
        let results = self.results.clone();
        for (i, y) in self.list.layout(g, results.len(), list_top, row_h, 3.0, bottom + 2.0) {
            song_row(g, Rect::new(PAD, y, width, row_h), &results[i], i == focus);
        }
        self.list.scrollbar(g, results.len(), list_top, bottom);
        if self.typing {
            g.hint_any(&["enter: search · esc: back", "enter: search"]);
        } else if !results.is_empty() {
            g.hint_any(&["enter: play, then its radio · n: play next", "enter: play · n: next"]);
        } else {
            g.hint("/: search again");
        }
    }

    fn draw_queue(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let n = now();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if n.queue.is_empty() {
            g.empty("queue_music", "nothing in the queue");
            return;
        }
        let focus = self.list.focus;
        for (i, y) in self.list.layout(g, n.queue.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let track = &n.queue[i];
            if i == n.at {
                let icon = if n.phase == Phase::Playing { "graphic_eq" } else { "pause" };
                crate::bluetooth::icon_row(g, r, icon, true, &track.title, "now", t.ink, i == focus);
            } else {
                song_row(g, r, track, i == focus);
            }
        }
        self.list.scrollbar(g, n.queue.len(), top, bottom);
        g.hint_any(&["enter: play this · x: take it out", "enter: play · x: out"]);
    }

    fn draw_account(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        let signed = signed_in();
        let version = match (&self.updating, &self.version_seen) {
            (Some(_), _) => "updating…".to_owned(),
            (_, Some(v)) => v.clone(),
            _ => "…".to_owned(),
        };
        let mut rows: Vec<(&str, String, bool)> = vec![("YouTube Music", (if signed { "signed in" } else { "signed out" }).into(), signed), ("Stream finder", version, true)];
        if signed {
            rows.push(("Sign out", (if self.armed { "enter again" } else { "" }).into(), true));
        }
        let mut below = top;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let (name, value, lit) = &rows[i];
            let r = Rect::new(PAD, y, width, row_h);
            g.row(r, name, value, if *lit { t.ink } else { t.dim }, i == self.list.focus);
            below = below.max(r.bottom());
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        // The warning first, so it is the part that always shows.
        let sentences: &[&str] = if signed {
            &["Third-party players like this one can break YouTube's terms, which could put this account at risk.", "Sign out to keep it out of reach."]
        } else {
            &["Third-party players like this one can break YouTube's terms.", "Signed out, nothing is at risk.", "To sign in, put cookies.txt from your browser in /data/music."]
        };
        // Whole sentences, the warning first: as many as there is room for.
        let mut y = below + 9.0 + ty.caption.size * 0.5;
        let step = ty.caption.size + 4.0;
        let mut lines: Vec<String> = Vec::new();
        for sentence in sentences {
            let mut with = lines.clone();
            let joined = if with.is_empty() { (*sentence).to_owned() } else { format!("{} {sentence}", with.join(" ")) };
            with = crate::help::wrap(g, &joined, ty.caption, width - 8.0);
            if y + step * (with.len() as f32 - 1.0) + ty.caption.size * 0.5 > bottom {
                break;
            }
            lines = with;
        }
        for line in &lines {
            g.text_at(PAD + 4.0, y, Align::Left, line, ty.caption, t.dim);
            y += step;
        }
        let hint: &[&str] = match self.list.focus {
            1 => &["enter: fetch the newest yt-dlp", "enter: update"],
            2 if self.armed => &["enter again: sign out · esc: stay", "enter again: sign out"],
            2 => &["forgets the cookies on this handheld", "enter: sign out"],
            _ if signed => &["signed in with your browser's cookies", "signed in"],
            _ => &["signed out unless you sign in", "signed out"],
        };
        g.hint_any(hint);
    }
}

/// A song in a list: its cover small, its title, and artist with length.
fn song_row(g: &mut Gfx, r: Rect, track: &Track, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let length = track.seconds.map(|s| clock(s as f32)).unwrap_or_default();
    g.row(r, "", &length, t.dim, focused);
    let side = r.h - 8.0;
    let art = Rect::new(r.x + 5.0, r.cy() - side * 0.5, side, side);
    match cover(track) {
        Some(p) if !p.is_empty() => crate::photos::draw_picture(g, art, &p, [0.0, 0.0, p.w as f32, p.h as f32], 5.0, 1.0),
        _ => g.surface(art, 5.0, t.surface2),
    }
    let x = art.right() + 7.0;
    let room = r.right() - 14.0 - g.measure(&length, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label }) - x;
    let title_w = g.text_fit(x, r.cy(), &track.title, ty.body, t.ink, room);
    if title_w + 30.0 < room && !track.artist.is_empty() {
        g.text_fit(x + title_w + 6.0, r.cy(), &track.artist.to_lowercase(), ty.caption, t.dim, room - title_w - 6.0);
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

    #[test]
    fn the_simulators_player_plays_pauses_and_moves_along() {
        let songs = demo_catalogue();
        send(Cmd::Play(songs.clone(), 2));
        let wait = |want: &dyn Fn(&Now) -> bool| {
            for _ in 0..80 {
                if want(&now()) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        };
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 2), "{:?}", now().phase);
        send(Cmd::Toggle);
        assert!(wait(&|n| n.phase == Phase::Paused));
        send(Cmd::Next);
        assert!(wait(&|n| n.phase == Phase::Playing && n.at == 3));
        // Back from the start of a song is the song before.
        send(Cmd::Previous);
        assert!(wait(&|n| n.at == 2));
        send(Cmd::PlayNext(songs[9].clone()));
        assert!(wait(&|n| n.queue.get(3).is_some_and(|t| t.id == "demo10")));
        send(Cmd::Remove(3));
        assert!(wait(&|n| n.queue.len() == songs.len()));
        assert_eq!(media_key(Media::PlayPause).as_deref(), Some("paused"));
        assert!(wait(&|n| n.phase == Phase::Paused));
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
        println!(
            "search: {:?}\nhome: {} quick picks; {name:?}: {} songs\nradio: {:?}",
            found.iter().take(3).map(|t| (&t.title, &t.artist, &t.album, t.seconds)).collect::<Vec<_>>(),
            picks.len(),
            list.len(),
            radio.iter().take(3).map(|t| (&t.title, &t.artist, t.seconds)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_hint_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let shortest = ["enter · ←→ · ↓", "enter · ← · /", "enter: try again", "enter: play · n: next", "enter: now playing", "type to search", "signed out unless you sign in", "enter: search", "enter: play · x: out", "enter: update", "enter again: sign out", "enter: sign out", "signed in", "signed out"];
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let caption = g.ty.caption;
            for s in shortest {
                assert!(g.measure(s, caption) <= W - 2.0 * PAD, "{s:?} at {size:?}");
            }
        }
    }
}
