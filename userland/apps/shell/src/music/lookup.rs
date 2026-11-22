//! Finding a song's audio stream.
//!
//! 1. **Quick** (the usual way, about a second): YouTube's own player API
//!    asked as its visionOS app, one HTTPS request through curl. That client
//!    is given stream addresses outright: no player JavaScript to run, no
//!    signature or throttling puzzle to answer (yt-dlp uses it for the same
//!    reason when it has no JavaScript runtime). It needs a visitor id,
//!    which YouTube hands out once and is kept.
//! 2. **Slower**: yt-dlp asked for that same client (Python: a few seconds,
//!    about 70 MB), for when YouTube answers the quick way differently.
//! 3. **Slowest**: yt-dlp as it is, answering the player's puzzle in
//!    QuickJS (15 to 25 s on the handheld, about 200 MB). Only when the
//!    others fail (an age-restricted song, signed in), only with 260 MB
//!    free, at nice 15, capped at 400 MB of address space, never two at
//!    once.
//!
//! Every helper is polite (polite.rs): low priority, ended first if memory
//! runs out, ended with the shell, and given a time limit.

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::polite::{self, Manner};

/// The client the quick way asks as (YouTube's visionOS app), as yt-dlp
/// 2026.10.26 has it.
const CLIENT: &str = "VISIONOS";
const CLIENT_NAME_ID: &str = "101";
const CLIENT_VERSION: &str = "1.02";
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const PLAYER: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const VISITOR: &str = "https://www.youtube.com/youtubei/v1/visitor_id?prettyPrint=false";
/// The visitor id YouTube gave, kept in Music's folder (0600).
fn visitor_file() -> std::path::PathBuf {
    super::store::root().join("visitor")
}

const QUICK_SECONDS: u64 = 15;
const SLOWER_SECONDS: u64 = 45;
const SLOWEST_SECONDS: u64 = 120;
/// The slow way's address-space cap, MB.
const SLOWEST_CAP_MB: u64 = 400;

/// A stream found.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub url: String,
    /// "webm" (Opus) or "m4a" (AAC).
    pub ext: String,
    /// Its sample rate: Opus is 48 kHz, as the codec runs; AAC 44.1.
    pub rate: u32,
    /// Its size, when YouTube says.
    pub bytes: Option<u64>,
    /// When the address stops working.
    pub expires: Instant,
}

/// Where a lookup has got, for the page to say.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    Asking,
    Slower,
    Slowest,
}

impl Step {
    pub fn words(self) -> &'static str {
        match self {
            Step::Asking => "finding it…",
            Step::Slower => "finding it the slower way…",
            Step::Slowest => "working out youtube's puzzle…",
        }
    }
}

/// Never two of the slow ways at once (each is a Python, the slowest a
/// QuickJS of 200 MB too).
static SLOW: Mutex<()> = Mutex::new(());
static VISITOR_ID: Mutex<Option<String>> = Mutex::new(None);

/// A song's stream: the quick way, then (if `slow_too`) the slower ones.
pub fn find(id: &str, slow_too: bool, signed_in: bool, step: &dyn Fn(Step)) -> Result<Stream, String> {
    step(Step::Asking);
    let quick = quick(id);
    let why = match quick {
        Ok(stream) => return Ok(stream),
        Err(why) => why,
    };
    if !slow_too || why == NO_INTERNET {
        return Err(why);
    }
    let _one_at_a_time = SLOW.lock().unwrap_or_else(|e| e.into_inner());
    step(Step::Slower);
    if let Ok(stream) = ytdlp(id, Some("visionos"), signed_in, Manner::Background, SLOWER_SECONDS) {
        return Ok(stream);
    }
    if polite::memory_available_mb().is_some_and(|mb| mb < polite::SLOW_LOOKUP_MB) {
        return Err("too little memory free to find this song: close something and try again".into());
    }
    step(Step::Slowest);
    ytdlp(id, None, signed_in, Manner::Capped(SLOWEST_CAP_MB), SLOWEST_SECONDS).map_err(|slow| if slow.is_empty() { why } else { slow })
}

pub const NO_INTERNET: &str = "no internet";

// ------------------------------------------------------------- the quick way ---

fn quick(id: &str) -> Result<Stream, String> {
    let visitor = visitor_id()?;
    match player(id, &visitor) {
        // A visitor id YouTube no longer takes: a new one, once.
        Err(why) if why.contains("confirm you") => {
            forget_visitor();
            player(id, &visitor_id()?)
        }
        other => other,
    }
}

fn player(id: &str, visitor: &str) -> Result<Stream, String> {
    let body = json!({
        "context": {"client": {"clientName": CLIENT, "clientVersion": CLIENT_VERSION, "deviceMake": "Apple", "deviceModel": "RealityDevice17,1", "osName": "visionOS", "osVersion": "26.5.23O471", "hl": "en", "visitorData": visitor}},
        "videoId": id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let answer = post(PLAYER, &body, &[("X-Goog-Visitor-Id", visitor)])?;
    stream_from(&answer)
}

/// The best audio-only stream in a player answer: Opus first (smaller for
/// the same sound, 48 kHz as the codec runs, so nothing to resample), AAC
/// if there is none.
pub fn stream_from(v: &Value) -> Result<Stream, String> {
    let status = v["playabilityStatus"]["status"].as_str().unwrap_or_default();
    if status != "OK" {
        let reason = v["playabilityStatus"]["reason"].as_str().unwrap_or(status);
        return Err(if reason.is_empty() { "youtube would not play this song".into() } else { reason.to_owned() });
    }
    let expires = v["streamingData"]["expiresInSeconds"].as_str().and_then(|s| s.parse::<u64>().ok()).unwrap_or(6 * 3600);
    let formats = v["streamingData"]["adaptiveFormats"].as_array().ok_or("no streams")?;
    let audio = |itag: u64| formats.iter().find(|f| f["itag"].as_u64() == Some(itag) && f["url"].is_string());
    let (f, ext, rate) = [(251, "webm", 48_000), (250, "webm", 48_000), (249, "webm", 48_000), (140, "m4a", 44_100), (139, "m4a", 44_100)]
        .into_iter()
        .find_map(|(itag, ext, rate)| audio(itag).map(|f| (f, ext, rate)))
        .ok_or("no audio stream this player can use")?;
    Ok(Stream {
        url: f["url"].as_str().unwrap_or_default().to_owned(),
        ext: ext.into(),
        rate: f["audioSampleRate"].as_str().and_then(|r| r.parse().ok()).unwrap_or(rate),
        bytes: f["contentLength"].as_str().and_then(|b| b.parse().ok()),
        // A margin, so an address is never used in its last minutes.
        expires: Instant::now() + Duration::from_secs(expires.saturating_sub(600)),
    })
}

fn visitor_id() -> Result<String, String> {
    if let Some(v) = VISITOR_ID.lock().unwrap().clone() {
        return Ok(v);
    }
    if let Some(v) = std::fs::read_to_string(visitor_file()).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
        *VISITOR_ID.lock().unwrap() = Some(v.clone());
        return Ok(v);
    }
    let answer = post(VISITOR, &json!({"context": {"client": {"clientName": CLIENT, "clientVersion": CLIENT_VERSION, "hl": "en"}}}), &[])?;
    let v = answer["responseContext"]["visitorData"].as_str().filter(|s| !s.is_empty()).ok_or("youtube gave no visitor id")?.to_owned();
    std::fs::create_dir_all(super::store::root()).ok();
    crate::sys::write_private(&visitor_file(), v.as_bytes()).ok();
    *VISITOR_ID.lock().unwrap() = Some(v.clone());
    Ok(v)
}

fn forget_visitor() {
    *VISITOR_ID.lock().unwrap() = None;
    std::fs::remove_file(visitor_file()).ok();
}

/// A JSON request as the visionOS app makes it, through curl (headers in a
/// file of the request's own, the body on standard input).
fn post(url: &str, body: &Value, extra: &[(&str, &str)]) -> Result<Value, String> {
    let mut headers = format!("Content-Type: application/json\nUser-Agent: {USER_AGENT}\nX-YouTube-Client-Name: {CLIENT_NAME_ID}\nX-YouTube-Client-Version: {CLIENT_VERSION}\nOrigin: https://www.youtube.com\n");
    for (k, v) in extra {
        headers.push_str(&format!("{k}: {v}\n"));
    }
    let file = super::private_file("player");
    crate::sys::write_private(&file, headers.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    let body_file = super::private_file("player-body");
    let written = std::fs::write(&body_file, body.to_string());
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--compressed", "--max-time", &QUICK_SECONDS.to_string(), "--proto", "=https", "-w", "\n%{http_code}"])
        .arg("-H")
        .arg(format!("@{}", file.display()))
        .arg("--data-binary")
        .arg(format!("@{}", body_file.display()))
        .arg(url);
    let out = written.map_err(|e| e.to_string()).and_then(|_| polite::output_within(polite::polite(&mut cmd, Manner::Background), Duration::from_secs(QUICK_SECONDS + 5)));
    std::fs::remove_file(&file).ok();
    std::fs::remove_file(&body_file).ok();
    let out = out?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) | Some(7) => NO_INTERNET.into(),
            Some(28) => "youtube did not answer in time".into(),
            _ => "the request failed".into(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    if code.trim() != "200" {
        return Err(format!("youtube answered {}", code.trim()));
    }
    serde_json::from_str(body).map_err(|_| "youtube's answer could not be read".to_owned())
}

// -------------------------------------------------------- the slower ways ---

/// yt-dlp: `client` (None: its own choice, with QuickJS for the puzzle).
fn ytdlp(id: &str, client: Option<&str>, signed_in: bool, manner: Manner, seconds: u64) -> Result<Stream, String> {
    std::fs::create_dir_all(super::store::root()).ok();
    let mut cmd = Command::new("python3");
    cmd.arg(super::resolver())
        .args(["--quiet", "--no-warnings", "--no-playlist", "--no-progress", "-f", "bestaudio[acodec=opus]/bestaudio[ext=m4a]/bestaudio"])
        .args(["--print", "%(ext)s|%(asr)s|%(filesize)s", "--print", "url", "--cache-dir", "/data/music/ytdlp-cache"]);
    match client {
        Some(c) => {
            cmd.args(["--no-js-runtimes", "--extractor-args", &format!("youtube:player_client={c}")]);
        }
        None => {
            cmd.args(["--js-runtimes", &format!("quickjs:{}", super::QJS)]);
        }
    }
    // Signed in, the cookies as a private copy (yt-dlp writes its jar back).
    let jar = (signed_in && client.is_none()).then(|| std::fs::read(super::cookies_file()).ok()).flatten().and_then(|cookies| {
        let path = super::private_file("cookies");
        crate::sys::write_private(&path, &cookies).ok().map(|_| path)
    });
    if let Some(path) = &jar {
        cmd.arg("--cookies").arg(path);
    }
    cmd.arg(format!("{}/watch?v={id}", super::ORIGIN));
    let out = polite::output_within(polite::polite(&mut cmd, manner), Duration::from_secs(seconds));
    if let Some(path) = &jar {
        std::fs::remove_file(path).ok();
    }
    let out = out?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let (info, url) = (lines.next().unwrap_or_default(), lines.find(|l| l.starts_with("https://")));
    match url {
        Some(url) if out.status.success() => {
            let mut f = info.split('|');
            let ext = f.next().filter(|e| !e.is_empty()).unwrap_or("webm").to_owned();
            let rate = f.next().and_then(|r| r.parse().ok()).unwrap_or(if ext == "webm" { 48_000 } else { 44_100 });
            let bytes = f.next().and_then(|b| b.parse().ok());
            Ok(Stream { url: url.trim().to_owned(), ext, rate, bytes, expires: Instant::now() + Duration::from_secs(5 * 3600) })
        }
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            let why = err.lines().filter(|l| l.contains("ERROR")).last().unwrap_or("").trim();
            Err(match why {
                w if w.contains("Sign in to confirm your age") => "this song is age-restricted: sign in to play it".into(),
                w if w.contains("Video unavailable") || w.contains("not available") => "this song is not available here".into(),
                w if w.contains("Unable to download") || w.contains("urlopen") => NO_INTERNET.into(),
                "" => String::new(),
                w => format!("the stream finder: {}", w.trim_start_matches("ERROR:").trim().chars().take(80).collect::<String>()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_is_chosen_over_aac_and_a_refusal_is_said() {
        let v = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"expiresInSeconds": "21540", "adaptiveFormats": [
            {"itag": 140, "url": "https://x/140", "mimeType": "audio/mp4", "contentLength": "3752751", "audioSampleRate": "44100"},
            {"itag": 251, "url": "https://x/251", "mimeType": "audio/webm; codecs=\"opus\"", "contentLength": "4115008", "audioSampleRate": "48000"},
            {"itag": 250, "signatureCipher": "s=…"}
        ]}});
        let s = stream_from(&v).unwrap();
        assert_eq!((s.url.as_str(), s.ext.as_str(), s.rate, s.bytes), ("https://x/251", "webm", 48_000, Some(4_115_008)));
        let left = s.expires.saturating_duration_since(Instant::now()).as_secs();
        assert!((20_900..=20_940).contains(&left), "{left}");
        // Only AAC: that, at 44.1 kHz.
        let v = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [{"itag": 140, "url": "https://x/140"}]}});
        assert_eq!((stream_from(&v).unwrap().ext.as_str(), stream_from(&v).unwrap().rate), ("m4a", 44_100));
        let v = json!({"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm you're not a bot"}});
        assert_eq!(stream_from(&v).unwrap_err(), "Sign in to confirm you're not a bot");
        let v = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [{"itag": 251, "signatureCipher": "s=…"}]}});
        assert!(stream_from(&v).is_err(), "a ciphered address is no use without the player's script");
    }

    /// YouTube itself: a song's stream the quick way, and how long it took.
    /// Over the network: `cargo test -- --ignored live_quick`.
    #[test]
    #[ignore]
    fn live_quick_lookup_finds_opus_in_about_a_second() {
        forget_visitor();
        for id in ["xukbqwRuN5w", "9bZkp7q19f0", "2Vv-BfVoq4g"] {
            let started = Instant::now();
            let s = quick(id).unwrap();
            println!("{id}: {:.2} s, {} {} Hz, {:?} bytes", started.elapsed().as_secs_f32(), s.ext, s.rate, s.bytes);
            assert_eq!(s.ext, "webm");
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }
}
