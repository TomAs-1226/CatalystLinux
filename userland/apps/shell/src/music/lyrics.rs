//! Lyrics: the words of the song playing, in time with it.
//!
//! - **Where they come from:** LRCLIB (lrclib.net: free, no key). Its
//!   `/api/get` matches the title, the artist, the album and the length;
//!   failing that, `/api/search` by title and artist, the closest in length
//!   taken (time-synced lyrics before plain ones). If LRCLIB has nothing
//!   and you are signed in to YouTube Music, its own lyrics (plain text,
//!   from LyricFind). Only the song's name, artist, album and length are
//!   sent. The simulator has its own made-up songs' words.
//! - **Kept with the song** on /data, so they show offline after the first
//!   time: an `.lrc` beside the song in the library (where a song of your
//!   own may have one already), else beside it in the played cache, which
//!   lets them go with it. A song found to have none is noted beside it too
//!   (`.nolyrics`), and asked about again after a fortnight. Songs being
//!   saved to the library get theirs as they are saved.
//! - **Synced lyrics** are LRC: `[mm:ss.xx]` before a line (several, for a
//!   line sung more than once), `[offset:±ms]`, word timings (`<mm:ss.xx>`)
//!   left out. An empty line is a pause in the singing, and a first line
//!   well into the song has one before it.
//! - **One song at a time** is held in memory: the one asked for last (the
//!   song playing, or the one the lyrics page shows).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::polite::{self, Manner};
use super::{library, store, Track};

pub const LRCLIB: &str = "https://lrclib.net";
/// LRCLIB asks clients to say who they are.
const AGENT: &str = "CatalystLinux-Music/0.8 (handheld music player)";
const ASK_SECONDS: u64 = 12;
/// A song found to have none is asked about again after this long.
const NONE_FOR: i64 = 14 * 86_400;
/// After the network failed, asked again no sooner than this.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A first line this far into the song has a pause shown before it.
pub const INTRO: f32 = 4.0;
/// The line shown goes back only for a real move back (a seek, the song
/// again from its start), not for the clock's corrections of a moment.
const BACK_SLACK: f32 = 1.0;
/// LRCLIB's own match: lengths this close are the same recording.
const SAME_LENGTH: f32 = 3.0;

// ------------------------------------------------------------------ words ---

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// When it is sung, seconds into the song.
    pub at: f32,
    /// The words; empty for a pause.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    /// In time with the song (a pause first if the singing starts late).
    Synced(Vec<Line>),
    /// Words alone: scrolled by hand.
    Plain(Vec<String>),
    /// No words to it.
    Instrumental,
    /// None found.
    None,
}

impl Lyrics {
    /// The synced lines, if they are.
    pub fn lines(&self) -> Option<&[Line]> {
        match self {
            Lyrics::Synced(l) => Some(l),
            _ => None,
        }
    }
}

/// The line being sung at `t`: the last to have started (`None` before the
/// first).
pub fn line_at(lines: &[Line], t: f32) -> Option<usize> {
    lines.partition_point(|l| l.at <= t).checked_sub(1)
}

/// Which line shows, as the song plays: it moves on with the song, and goes
/// back only when the song really went back. The player's clock is put
/// right every few seconds by a fraction of one; at a line's start that
/// would show the line before again for a moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Follow {
    shown: Option<usize>,
}

impl Follow {
    pub fn at(&mut self, lines: &[Line], t: f32) -> Option<usize> {
        let now = line_at(lines, t);
        if let Some(shown) = self.shown.filter(|&s| s < lines.len()) {
            let earlier = now.is_none_or(|n| n < shown);
            if earlier && t > lines[shown].at - BACK_SLACK {
                return Some(shown);
            }
        }
        self.shown = now;
        now
    }

    /// A new song: nothing shown yet.
    pub fn reset(&mut self) {
        self.shown = None;
    }
}

/// LRC (or plain words) read: synced if any line has a time.
pub fn parse(text: &str) -> Lyrics {
    let mut offset = 0.0f32;
    let mut timed: Vec<(f32, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        let mut rest = line;
        let mut times = Vec::new();
        let mut tag_only = false;
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            if let Some(t) = timestamp(tag) {
                times.push(t);
                rest = &inner[end + 1..];
                continue;
            }
            // [ar:…], [ti:…], [offset:+200] on a line of their own; anything
            // else in brackets ("[Chorus]") is part of the words.
            let meta = tag.split_once(':').filter(|(k, _)| (1..=8).contains(&k.len()) && k.chars().all(|c| c.is_ascii_alphabetic() || c == '#'));
            if let Some((key, value)) = meta.filter(|_| times.is_empty() && inner[end + 1..].trim().is_empty()) {
                if key.eq_ignore_ascii_case("offset") {
                    offset = value.trim().trim_start_matches('+').parse::<f32>().map_or(0.0, |ms| ms / 1000.0);
                }
                tag_only = true;
            }
            break;
        }
        if tag_only {
            continue;
        }
        let words = strip_word_times(rest);
        if times.is_empty() {
            plain.push(words);
        } else {
            timed.extend(times.into_iter().map(|t| (t, words.clone())));
        }
    }
    if !timed.is_empty() {
        return synced(timed, offset);
    }
    // Plain: blank lines kept as one between verses, none at the ends.
    let mut lines: Vec<String> = Vec::new();
    for l in plain {
        if !l.is_empty() || lines.last().is_some_and(|p| !p.is_empty()) {
            lines.push(l);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.is_empty() {
        Lyrics::None
    } else {
        Lyrics::Plain(lines)
    }
}

fn synced(mut timed: Vec<(f32, String)>, offset: f32) -> Lyrics {
    // A positive offset brings the words in sooner.
    for (t, _) in timed.iter_mut() {
        *t = (*t - offset).max(0.0);
    }
    timed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lines: Vec<Line> = Vec::new();
    for (at, text) in timed {
        // One pause for several empty lines; none before the first words.
        if text.is_empty() && lines.last().is_none_or(|l| l.text.is_empty()) {
            continue;
        }
        lines.push(Line { at, text });
    }
    while lines.last().is_some_and(|l| l.text.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return Lyrics::None;
    }
    if lines[0].at >= INTRO {
        lines.insert(0, Line { at: 0.0, text: String::new() });
    }
    Lyrics::Synced(lines)
}

/// "02:14.30", "2:14.305", "02:14", "02:14:30" (a colon before the
/// hundredths, as some write it): seconds.
fn timestamp(tag: &str) -> Option<f32> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (m, s) = tag.split_once(':')?;
    if !digits(m) || m.len() > 3 {
        return None;
    }
    let (whole, frac) = s.split_once(['.', ':']).unwrap_or((s, ""));
    if !digits(whole) || whole.len() > 2 || (!frac.is_empty() && (!digits(frac) || frac.len() > 3)) {
        return None;
    }
    let secs: f32 = whole.parse().ok()?;
    if secs >= 60.0 {
        return None;
    }
    let frac = if frac.is_empty() { 0.0 } else { frac.parse::<f32>().ok()? / 10f32.powi(frac.len() as i32) };
    Some(m.parse::<f32>().ok()? * 60.0 + secs + frac)
}

/// The words without enhanced LRC's word timings (`<00:12.30>`), spaced once.
fn strip_word_times(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        match rest[at + 1..].find('>') {
            Some(end) if timestamp(&rest[at + 1..at + 1 + end]).is_some() => {
                out.push_str(&rest[..at]);
                out.push(' ');
                rest = &rest[at + 2 + end..];
            }
            _ => {
                out.push_str(&rest[..=at]);
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- asking ---

/// What asking found.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// LRC, or plain words.
    Words(String),
    Instrumental,
    Nothing,
}

impl Found {
    pub fn lyrics(&self) -> Lyrics {
        match self {
            Found::Words(w) => parse(w),
            Found::Instrumental => Lyrics::Instrumental,
            Found::Nothing => Lyrics::None,
        }
    }
}

/// One of LRCLIB's records: synced words before plain ones.
fn from_record(v: &Value) -> Option<Found> {
    if v["instrumental"].as_bool() == Some(true) {
        return Some(Found::Instrumental);
    }
    ["syncedLyrics", "plainLyrics"].iter().find_map(|k| v[*k].as_str().filter(|t| !t.trim().is_empty()).map(|t| Found::Words(t.to_owned())))
}

/// The best of a search's records for a song this long: only those of its
/// length (when it is known), synced first, then the closest.
pub fn best(records: &[Value], seconds: Option<u32>) -> Option<Found> {
    let off = |v: &Value| match (seconds, v["duration"].as_f64()) {
        (Some(s), Some(d)) => (d as f32 - s as f32).abs(),
        (Some(_), None) => f32::MAX,
        (None, _) => 0.0,
    };
    let synced = |v: &Value| v["syncedLyrics"].as_str().is_some_and(|t| !t.trim().is_empty());
    records
        .iter()
        .filter(|v| off(v) <= SAME_LENGTH)
        .filter(|v| from_record(v).is_some())
        .min_by(|a, b| (!synced(a), off(a)).partial_cmp(&(!synced(b), off(b))).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(from_record)
}

/// A title as LRCLIB knows it: YouTube's "(Official Video)", "[Remastered
/// 2011]" and the like left off.
pub fn clean_title(title: &str) -> String {
    const NOISE: [&str; 9] = ["official", "video", "audio", "lyric", "remaster", "visuali", " mv", "hd", "4k"];
    let noisy = |s: &str| {
        let s = format!(" {}", s.to_lowercase());
        NOISE.iter().any(|n| s.contains(n))
    };
    let mut out = String::new();
    let mut rest = title;
    while let Some(at) = rest.find(['(', '[']) {
        let close = if rest[at..].starts_with('(') { ')' } else { ']' };
        match rest[at..].find(close) {
            Some(end) if noisy(&rest[at + 1..at + end]) => {
                out.push_str(&rest[..at]);
                rest = &rest[at + end + 1..];
            }
            _ => {
                out.push_str(&rest[..=at]);
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    let out = match out.split_once(" - ") {
        Some((head, tail)) if noisy(tail) => head.to_owned(),
        _ => out,
    };
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first of several artists ("A & B", "A, B", "A feat. B").
pub fn first_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let cut = [", ", " & ", " feat. ", " ft. ", " x ", " and "].iter().filter_map(|s| lower.find(s)).min();
    cut.map_or(artist, |at| &artist[..at]).trim().to_owned()
}

/// A GET to LRCLIB (or the tests' stand-in for it) through curl: the
/// status, and the answer as JSON (null if it is not).
fn ask(base: &str, path: &str, query: &[(&str, String)]) -> Result<(u16, Value), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-G", "--compressed", "--max-time", &ASK_SECONDS.to_string(), "-A", AGENT, "-w", "\n%{http_code}"]);
    if base.starts_with("https://") {
        cmd.args(["--proto", "=https"]);
    }
    for (k, v) in query {
        cmd.arg("--data-urlencode").arg(format!("{k}={v}"));
    }
    cmd.arg(format!("{base}{path}"));
    polite::polite(&mut cmd, Manner::Background);
    let out = polite::output_within(&mut cmd, Duration::from_secs(ASK_SECONDS + 3))?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) | Some(7) => super::lookup::NO_INTERNET.into(),
            Some(28) => "lrclib did not answer in time".into(),
            _ => "the lyrics could not be asked for".into(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let code: u16 = code.trim().parse().map_err(|_| "lrclib's answer could not be read".to_owned())?;
    Ok((code, serde_json::from_str(body).unwrap_or(Value::Null)))
}

/// LRCLIB's lyrics for a song: matched whole, else searched for.
pub fn from_lrclib(base: &str, t: &Track) -> Result<Found, String> {
    if t.title.trim().is_empty() {
        return Ok(Found::Nothing);
    }
    if let (Some(secs), false) = (t.seconds, t.album.trim().is_empty()) {
        let (code, v) = ask(base, "/api/get", &[("track_name", t.title.clone()), ("artist_name", t.artist.clone()), ("album_name", t.album.clone()), ("duration", secs.to_string())])?;
        if code == 200 {
            if let Some(found) = from_record(&v) {
                return Ok(found);
            }
        }
    }
    let (code, v) = ask(base, "/api/search", &[("track_name", clean_title(&t.title)), ("artist_name", first_artist(&t.artist))])?;
    match code {
        200 => Ok(v.as_array().and_then(|records| best(records, t.seconds)).unwrap_or(Found::Nothing)),
        c if c >= 500 => Err(format!("lrclib answered {c}")),
        _ => Ok(Found::Nothing),
    }
}

/// YouTube Music's lyrics page for a song, from what plays next for it.
pub fn youtube_browse_id(next: &Value) -> Option<String> {
    let mut found = Vec::new();
    super::find_all(next, "browseEndpoint", &mut found);
    found.iter().filter_map(|e| e["browseId"].as_str()).find(|id| id.starts_with("MPLY")).map(str::to_owned)
}

/// The words on YouTube Music's lyrics page.
pub fn youtube_words(page: &Value) -> Option<String> {
    let mut shelves = Vec::new();
    super::find_all(page, "musicDescriptionShelfRenderer", &mut shelves);
    shelves.iter().map(|s| super::runs_text(&s["description"])).find(|t| !t.trim().is_empty())
}

fn from_youtube(id: &str) -> Result<Found, String> {
    let next = super::innertube("next", json!({"videoId": id, "isAudioOnly": true}))?;
    let Some(browse) = youtube_browse_id(&next) else { return Ok(Found::Nothing) };
    let page = super::innertube("browse", json!({"browseId": browse}))?;
    Ok(youtube_words(&page).map_or(Found::Nothing, Found::Words))
}

/// A song's lyrics, from wherever has them.
fn fetch_for(t: &Track) -> Result<Found, String> {
    if !super::on_device() {
        if !cfg!(test) {
            std::thread::sleep(Duration::from_millis(350));
        }
        return Ok(demo(t));
    }
    let lrclib = from_lrclib(LRCLIB, t);
    if matches!(lrclib, Ok(Found::Words(_)) | Ok(Found::Instrumental)) {
        return lrclib;
    }
    let youtube = !t.id.is_empty() && !t.id.starts_with("lib:");
    if youtube && super::signed_in() {
        if let Ok(Found::Words(w)) = from_youtube(&t.id) {
            return Ok(Found::Words(w));
        }
    }
    lrclib
}

// -------------------------------------------------------------- the card ---

/// Where a song's lyrics are kept: beside it in the library, else beside
/// it in the played cache.
fn home(t: &Track) -> PathBuf {
    let beside = t.local.clone().or_else(|| library::by_youtube(&t.id).map(|s| s.path));
    match beside.filter(|p| p.parent().is_some_and(Path::exists)) {
        Some(song) => song.with_extension("lrc"),
        None => cached_home(t),
    }
}

fn cached_home(t: &Track) -> PathBuf {
    store::cache_dir().join(format!("{}.lrc", store::safe_id(&t.id)))
}

/// The note that a song has none.
fn marker(lrc: &Path) -> PathBuf {
    lrc.with_extension("nolyrics")
}

/// The lyrics kept for a song, if they are.
pub fn on_card(t: &Track) -> Option<Lyrics> {
    let mut places = vec![home(t)];
    if !t.id.is_empty() && !t.id.starts_with("lib:") {
        places.push(cached_home(t));
    }
    places.dedup();
    for lrc in places {
        if let Ok(text) = std::fs::read_to_string(&lrc) {
            return Some(parse(&text));
        }
        if let Ok(note) = std::fs::read_to_string(marker(&lrc)) {
            let mut words = note.split_whitespace();
            let kind = words.next().unwrap_or_default();
            let when: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
            if kind == "instrumental" {
                return Some(Lyrics::Instrumental);
            }
            if store::now_unix() - when < NONE_FOR {
                return Some(Lyrics::None);
            }
        }
    }
    None
}

/// What was found, kept at `lrc`.
fn keep(lrc: &Path, found: &Found) {
    match found {
        Found::Words(w) => {
            if store::write_atomic(lrc, w.as_bytes()).is_ok() {
                std::fs::remove_file(marker(lrc)).ok();
            }
        }
        Found::Instrumental => {
            store::write_atomic(&marker(lrc), format!("instrumental {}\n", store::now_unix()).as_bytes()).ok();
        }
        Found::Nothing => {
            store::write_atomic(&marker(lrc), format!("none {}\n", store::now_unix()).as_bytes()).ok();
        }
    }
}

/// A song just saved to the library: its lyrics beside it, carried from
/// the played cache if they were found while it played, else asked for
/// (online). On the download's own thread.
pub fn keep_for_saved(t: &Track, song: &Path) {
    let lrc = song.with_extension("lrc");
    if lrc.exists() || marker(&lrc).exists() {
        return;
    }
    let cached = cached_home(t);
    if std::fs::copy(&cached, &lrc).is_ok() || std::fs::copy(marker(&cached), marker(&lrc)).is_ok() {
        return;
    }
    if super::on_device() && !polite::online() {
        return;
    }
    if let Ok(found) = fetch_for(t) {
        keep(&lrc, &found);
    }
}

/// A played song let go from the cache: its lyrics with it.
pub fn forget_cached(id: &str) {
    let lrc = store::cache_dir().join(format!("{}.lrc", store::safe_id(id)));
    std::fs::remove_file(marker(&lrc)).ok();
    std::fs::remove_file(lrc).ok();
}

// ------------------------------------------------------------- in memory ---

/// The lyrics of the song asked for last, as they stand.
#[derive(Clone, Debug)]
pub enum State {
    /// Being looked for.
    Looking,
    Ready(Arc<Lyrics>),
    /// Not on the card, and no internet to ask.
    Offline,
    /// Asking failed (said); asked again a minute later.
    Failed(String),
}

struct Held {
    id: String,
    state: State,
    since: Instant,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);
/// Counts changes, so a page showing lyrics draws again.
pub static STAMP: AtomicU64 = AtomicU64::new(0);

fn hold(id: &str, state: State) {
    *HELD.lock().unwrap() = Some(Held { id: id.to_owned(), state, since: Instant::now() });
    STAMP.fetch_add(1, Ordering::Relaxed);
}

/// A song's lyrics wanted: from the card at once if they are there, else
/// asked for on a thread of its own (online). Nothing if they are already
/// held, or being looked for.
pub fn want(t: &Track) {
    if t.id.is_empty() {
        return;
    }
    if let Some(h) = HELD.lock().unwrap().as_ref().filter(|h| h.id == t.id) {
        let again = match &h.state {
            State::Offline => polite::online(),
            State::Failed(_) => h.since.elapsed() >= RETRY_AFTER,
            _ => false,
        };
        if !again {
            return;
        }
    }
    if let Some(l) = on_card(t) {
        return hold(&t.id, State::Ready(Arc::new(l)));
    }
    if super::on_device() && !polite::online() {
        return hold(&t.id, State::Offline);
    }
    hold(&t.id, State::Looking);
    let track = t.clone();
    std::thread::Builder::new()
        .name("music-lyrics".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            polite::background_thread();
            let state = match fetch_for(&track) {
                Ok(found) => {
                    keep(&home(&track), &found);
                    State::Ready(Arc::new(found.lyrics()))
                }
                Err(why) if why == super::lookup::NO_INTERNET => State::Offline,
                Err(why) => State::Failed(why),
            };
            // Kept only if it is still the song wanted.
            if HELD.lock().unwrap().as_ref().is_some_and(|h| h.id == track.id) {
                hold(&track.id, state);
            }
        })
        .ok();
}

/// The lyrics held for this song, if they are.
pub fn held(id: &str) -> Option<State> {
    HELD.lock().unwrap().as_ref().filter(|h| h.id == id).map(|h| h.state.clone())
}

/// The synced lyrics held for this song, ready.
pub fn synced_for(id: &str) -> Option<Arc<Lyrics>> {
    match held(id)? {
        State::Ready(l) if l.lines().is_some() => Some(l),
        _ => None,
    }
}

// ------------------------------------------------------------------ demo ---

/// The simulator's made-up songs' words (written for them).
fn demo(t: &Track) -> Found {
    match t.title.as_str() {
        "Harbour Lights" => Found::Words(DEMO_HARBOUR.into()),
        "Slow Signal" => Found::Words(DEMO_SIGNAL.into()),
        "Midnight Telemetry" | "Glasshouse" => Found::Instrumental,
        _ => Found::Nothing,
    }
}

const DEMO_HARBOUR: &str = "[ti:Harbour Lights]
[ar:Wren & the Tide]
[al:Low Water]
[00:14.20]The ferry hums a low goodnight
[00:18.60]The water holds the town in light
[00:23.10]I count the windows one by one
[00:27.40]And wait for you to cross the bay
[00:32.20]
[00:36.00]Harbour lights, harbour lights
[00:40.30]Keep me steady through the night
[00:44.70]Every lantern on the tide
[00:49.00]Is a way back to your side
[00:54.00]
[01:02.00]Salt on the railing, rope on the pier
[01:06.40]Gulls on the mast say you're almost here
[01:10.90]The tide comes in with a slower song
[01:15.20]I've been waiting here all along
[01:20.00]
[01:24.00][02:27.00]Harbour lights, harbour lights
[01:28.30][02:31.30]Keep me steady through the night
[01:32.70][02:35.70]Every lantern on the tide
[01:37.00][02:40.00]Is a way back to your side
[01:42.00]
[02:05.00]And if the fog rolls in tonight
[02:09.40]I'll follow every little light
[02:13.80]Past the buoys and the breakwater wall
[02:18.20]Until I hear you call
[02:23.00]
[02:45.00]A way back to your side
[02:52.00]
";

const DEMO_SIGNAL: &str = "Static on the radio
A city falling into blue
I turn the dial a little slow
And every song is about you

Slow signal, come in clear
Through the miles and the years
Slow signal, I can hear
You're still out there

Headlights on the overpass
The night bus sighing at the stop
I keep a station for the past
And I never let it drop

Slow signal, come in clear
Through the miles and the years
Slow signal, I can hear
You're still out there
";

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn lines(l: &Lyrics) -> Vec<(f32, &str)> {
        l.lines().unwrap().iter().map(|l| (l.at, l.text.as_str())).collect()
    }

    #[test]
    fn lrc_reads_in_time_with_repeats_offsets_and_word_timings_left_out() {
        let lrc = "[ar:Somebody]\n[offset:+500]\n[00:12.00]First <00:12.20>line <00:12.80>here\n[00:15.5][01:02.250]Chorus\n\n[00:20.00]\n[00:21.00]\n[00:30:40]Last [ad lib]\n";
        let l = parse(lrc);
        // Half a second sooner, a pause before the first words, the chorus twice.
        assert_eq!(lines(&l), [(0.0, ""), (11.5, "First line here"), (15.0, "Chorus"), (19.5, ""), (29.9, "Last [ad lib]"), (61.75, "Chorus")]);
    }

    #[test]
    fn words_alone_are_plain_with_one_blank_between_verses() {
        let l = parse("\n[Chorus]\nOne\n\n\n\nTwo\nThree\n\n");
        assert_eq!(l, Lyrics::Plain(vec!["[Chorus]".into(), "One".into(), "".into(), "Two".into(), "Three".into()]));
        assert_eq!(parse("[ti:Only tags]\n\n"), Lyrics::None);
        assert_eq!(parse("[00:01.00]\n[00:02.00]"), Lyrics::None);
    }

    #[test]
    fn a_song_that_starts_singing_at_once_has_no_pause_first() {
        let l = parse("[00:01.50]Straight in\n[00:04.00]Next");
        assert_eq!(lines(&l), [(1.5, "Straight in"), (4.0, "Next")]);
    }

    #[test]
    fn times_are_read_as_lrc_writes_them_and_nothing_else_is() {
        let near = |tag: &str, want: f32| assert!(timestamp(tag).is_some_and(|t| (t - want).abs() < 1e-3), "{tag}: {:?}", timestamp(tag));
        near("01:02.5", 62.5);
        near("01:02.345", 62.345);
        near("101:00", 6060.0);
        near("00:02:50", 2.5);
        for bad in ["ar:Name", "1:60.00", "a1:00", ":12", "01:02.3456", "01:"] {
            assert_eq!(timestamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_line_sung_follows_the_song_and_goes_back_only_for_a_seek() {
        let Lyrics::Synced(l) = parse("[00:10.00]a\n[00:20.00]b\n[00:30.00]c") else { panic!() };
        assert_eq!(line_at(&l, 5.0), Some(0), "the pause before the singing");
        assert_eq!(line_at(&l, 10.0), Some(1));
        assert_eq!(line_at(&l, 29.99), Some(2));
        assert_eq!(line_at(&l, 999.0), Some(3));
        let mut f = Follow::default();
        assert_eq!(f.at(&l, 20.1), Some(2));
        // The clock put right by a few tenths, back over the line's start: kept.
        assert_eq!(f.at(&l, 19.7), Some(2));
        assert_eq!(f.at(&l, 25.0), Some(2));
        assert_eq!(f.at(&l, 31.0), Some(3));
        // A seek back ten seconds: back it goes.
        assert_eq!(f.at(&l, 21.0), Some(2));
        assert_eq!(f.at(&l, 2.0), Some(0));
        f.reset();
        assert_eq!(f.at(&l, 12.0), Some(1));
    }

    #[test]
    fn titles_and_artists_are_asked_for_as_lrclib_knows_them() {
        assert_eq!(clean_title("Harbour Lights (Official Video)"), "Harbour Lights");
        assert_eq!(clean_title("Copper Season [Remastered 2011] (Live)"), "Copper Season (Live)");
        assert_eq!(clean_title("Static Bloom - 2019 Remaster"), "Static Bloom");
        assert_eq!(clean_title("Kites (Over Fremont)"), "Kites (Over Fremont)");
        assert_eq!(first_artist("Juniper Vale & Mara Ellis"), "Juniper Vale");
        assert_eq!(first_artist("Wren, The Tide"), "Wren");
        assert_eq!(first_artist("Hollis Gray feat. Someone"), "Hollis Gray");
        assert_eq!(first_artist("Ocean Avenue Club"), "Ocean Avenue Club");
    }

    #[test]
    fn of_a_search_the_synced_record_of_the_songs_length_is_taken() {
        let records = vec![
            json!({"duration": 214.0, "plainLyrics": "plain, right length", "syncedLyrics": null}),
            json!({"duration": 260.0, "plainLyrics": "x", "syncedLyrics": "[00:01.00]too long"}),
            json!({"duration": 216.0, "plainLyrics": "x", "syncedLyrics": "[00:01.00]synced, close"}),
            json!({"duration": 213.0, "instrumental": false, "plainLyrics": "", "syncedLyrics": ""}),
        ];
        assert_eq!(best(&records, Some(214)), Some(Found::Words("[00:01.00]synced, close".into())));
        assert_eq!(best(&records[..1], Some(214)), Some(Found::Words("plain, right length".into())));
        assert_eq!(best(&records[1..2], Some(214)), None, "another recording's words are not this one's");
        assert_eq!(best(&[json!({"duration": 100.0, "instrumental": true})], Some(101)), Some(Found::Instrumental));
    }

    /// A stand-in for LRCLIB on this computer: answers each request by its
    /// path, and keeps what was asked.
    fn stub(answer: fn(&str, &str) -> (u16, String)) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).ok();
                let mut header = String::new();
                let mut agent = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    if header.to_lowercase().starts_with("user-agent:") {
                        agent = header.trim().to_owned();
                    }
                    header.clear();
                }
                let target = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let query = decode(query);
                seen.lock().unwrap().push(format!("{path}?{query} {agent}"));
                let (code, body) = answer(path, &query);
                let mut stream = stream;
                write!(stream, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).ok();
            }
        });
        (base, asked)
    }

    fn decode(q: &str) -> String {
        let bytes = q.replace('+', " ").into_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn song() -> Track {
        Track { id: "abc".into(), title: "Harbour Lights (Official Video)".into(), artist: "Wren & the Tide".into(), album: "Low Water".into(), seconds: Some(214), ..Track::default() }
    }

    #[test]
    fn lrclib_is_asked_for_the_song_whole_first() {
        let (base, asked) = stub(|path, _| match path {
            "/api/get" => (200, json!({"id": 1, "duration": 214.0, "instrumental": false, "plainLyrics": "a", "syncedLyrics": "[00:14.20]The ferry hums"}).to_string()),
            _ => (500, String::new()),
        });
        assert_eq!(from_lrclib(&base, &song()), Ok(Found::Words("[00:14.20]The ferry hums".into())));
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "{asked:?}");
        let a = &asked[0];
        for part in ["track_name=Harbour Lights (Official Video)", "artist_name=Wren & the Tide", "album_name=Low Water", "duration=214", "CatalystLinux-Music"] {
            assert!(a.contains(part), "{part} in {a}");
        }
    }

    #[test]
    fn not_matched_whole_lrclib_is_searched_and_the_closest_taken() {
        let (base, asked) = stub(|path, query| match path {
            "/api/get" => (404, json!({"code": 404, "name": "TrackNotFound"}).to_string()),
            "/api/search" if query.contains("track_name=Harbour Lights&") || query.ends_with("track_name=Harbour Lights") => (
                200,
                json!([
                    {"duration": 180.0, "syncedLyrics": "[00:01.00]another one"},
                    {"duration": 215.0, "plainLyrics": "words", "syncedLyrics": "[00:14.20]this one"}
                ])
                .to_string(),
            ),
            _ => (200, "[]".into()),
        });
        assert_eq!(from_lrclib(&base, &song()), Ok(Found::Words("[00:14.20]this one".into())));
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 2);
        assert!(asked[1].contains("artist_name=Wren"), "the first artist alone: {}", asked[1]);
        assert!(!asked[1].contains("Official"), "the title cleaned: {}", asked[1]);
    }

    #[test]
    fn nothing_found_is_said_as_nothing_and_a_broken_server_as_a_failure() {
        let (base, _) = stub(|path, _| match path {
            "/api/get" => (404, "{}".into()),
            _ => (200, "[]".into()),
        });
        assert_eq!(from_lrclib(&base, &song()), Ok(Found::Nothing));
        let (base, _) = stub(|_, _| (503, "down".into()));
        assert!(from_lrclib(&base, &song()).is_err());
        // No album or length: straight to the search.
        let (base, asked) = stub(|_, _| (200, json!([{"duration": 99.0, "plainLyrics": "found"}]).to_string()));
        let loose = Track { album: String::new(), seconds: None, ..song() };
        assert_eq!(from_lrclib(&base, &loose), Ok(Found::Words("found".into())));
        assert!(asked.lock().unwrap()[0].starts_with("/api/search"));
    }

    #[test]
    fn youtube_musics_own_lyrics_are_read_from_its_lyrics_tab() {
        let next = json!({"contents": {"singleColumnMusicWatchNextResultsRenderer": {"tabbedRenderer": {"watchNextTabbedResultsRenderer": {"tabs": [
            {"tabRenderer": {"title": "Up next", "endpoint": {"browseEndpoint": {"browseId": "MPTRt_queue"}}}},
            {"tabRenderer": {"title": "Lyrics", "endpoint": {"browseEndpoint": {"browseId": "MPLYt_abc-1", "browseEndpointContextSupportedConfigs": {}}}}},
            {"tabRenderer": {"title": "Related", "endpoint": {"browseEndpoint": {"browseId": "MPTRt_rel"}}}}
        ]}}}}});
        assert_eq!(youtube_browse_id(&next).as_deref(), Some("MPLYt_abc-1"));
        let none = json!({"tabs": [{"tabRenderer": {"title": "Lyrics", "unselectable": true}}]});
        assert_eq!(youtube_browse_id(&none), None);
        let page = json!({"contents": {"sectionListRenderer": {"contents": [{"musicDescriptionShelfRenderer": {
            "description": {"runs": [{"text": "First line\nSecond line\n\nThird"}]},
            "footer": {"runs": [{"text": "Source: LyricFind"}]}
        }}]}}});
        let words = youtube_words(&page).unwrap();
        assert_eq!(parse(&words), Lyrics::Plain(vec!["First line".into(), "Second line".into(), "".into(), "Third".into()]));
    }

    #[test]
    fn lyrics_are_kept_with_the_song_and_carried_into_the_library() {
        let t = Track { id: "keep-test".into(), title: "Kept".into(), ..Track::default() };
        assert_eq!(on_card(&t), None);
        keep(&cached_home(&t), &Found::Words("[00:01.00]Kept".into()));
        assert_eq!(on_card(&t).and_then(|l| l.lines().map(|l| l[0].text.clone())), Some("Kept".into()));
        // Saved to the library: beside the song there, without asking again.
        let dir = store::library_dir().join("Kept Artist").join("Kept Album");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01 Kept.opus");
        keep_for_saved(&t, &file);
        assert!(std::fs::read_to_string(dir.join("01 Kept.lrc")).unwrap().contains("Kept"));
        // Let go from the cache: the lyrics with it.
        forget_cached(&t.id);
        assert!(!cached_home(&t).exists());
        // A song with none is noted, and not asked again for a fortnight.
        let quiet = Track { id: "none-test".into(), title: "Quiet".into(), ..Track::default() };
        keep(&cached_home(&quiet), &Found::Nothing);
        assert_eq!(on_card(&quiet), Some(Lyrics::None));
        store::write_atomic(&marker(&cached_home(&quiet)), format!("none {}\n", store::now_unix() - NONE_FOR - 1).as_bytes()).unwrap();
        assert_eq!(on_card(&quiet), None, "asked again after a fortnight");
        keep(&cached_home(&quiet), &Found::Instrumental);
        assert_eq!(on_card(&quiet), Some(Lyrics::Instrumental));
    }

    #[test]
    fn the_simulators_songs_have_words_of_their_own() {
        let t = |title: &str| Track { title: title.into(), ..Track::default() };
        assert!(matches!(demo(&t("Harbour Lights")).lyrics(), Lyrics::Synced(l) if l.len() > 20 && l[0].text.is_empty()));
        assert!(matches!(demo(&t("Slow Signal")).lyrics(), Lyrics::Plain(_)));
        assert_eq!(demo(&t("Glasshouse")).lyrics(), Lyrics::Instrumental);
        assert_eq!(demo(&t("Paper Satellites")).lyrics(), Lyrics::None);
    }
}
