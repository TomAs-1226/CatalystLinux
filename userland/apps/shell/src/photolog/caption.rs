//! Captions by Claude, for the build log: opt-in, in the background, within
//! a monthly limit.
//!
//! **What is sent.** Each new photo shrunk to 1,024 pixels on its long side
//! (a JPEG of about 100 KB: 37 × 28 = 1,036 of Claude's 28-pixel image
//! tokens), with a short instruction for a robotics team's notebook and, as
//! context, when it was taken, its named place and the event and match.
//! Nothing else: no names, no location, no Wi-Fi.
//!
//! **How.** The Messages API (`2026-10-21`), Claude Opus 5.5, effort low
//! (thinking cannot be turned off on Opus 5.5; low keeps it short), through
//! the Message Batches API: half the price of a request made at once, and
//! a caption can wait. Photos waiting are sent together once the newest is
//! two minutes old; the batch is looked at after a minute, then less often
//! (usually done within the hour, at most a day), its answers read by each
//! `custom_id`, and the batch deleted at Anthropic once read (its requests
//! would otherwise be kept 29 days). "Caption now" in a photo's sheet asks
//! at once instead, at the full price, with the server's fallback on a
//! refusal as the Assistant has it.
//!
//! **What it costs.** From each answer's `usage` at Opus 5.5's prices ($4 a
//! million tokens in, $20 out; batches half that). About $0.004 a photo in a
//! batch. Before sending, the most a photo could cost (its image and the
//! instruction in, every allowed token out) is held against the month's
//! limit, so the limit is never passed: past it, captions wait for the next
//! month.
//!
//! **When.** Only while captions are on, there is a Claude key, the
//! handheld is online, not short of memory or in deep standby, and on the
//! charger or above 40%. Offline, photos wait. A failure tries again later,
//! a minute, then two, four… up to six hours; after eight tries it gives up.
//! A refused key waits for the keys to change.
//!
//! The key is the Assistant's (Settings › Keys), read when a request is
//! made and handed to curl on its input (net.rs): never logged, never on a
//! command line, never in another file.

use std::path::Path;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::net::{self, Reply};
use super::store::{month_of, Batch, Job, Ledger};

pub const MODEL: &str = "claude-opus-5-5";
const VERSION: &str = "2026-10-21";
/// Server-side fallback on a refusal, for a caption asked at once.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const ANTHROPIC: &str = "https://api.anthropic.com";
const AGENT: &str = "CatalystLinux-Photos/1.4";

/// The long side photos are shrunk to.
pub const LONG: usize = 1024;
/// Room for thinking (at low effort, short) and a caption of two sentences.
pub const MAX_TOKENS: u32 = 1500;
/// A caption kept at most this long.
pub const CAPTION_MOST: usize = 320;
/// The instruction's own tokens, a generous count.
const PROMPT_TOKENS: u64 = 420;
/// Photos in one batch at most (each about 140 KB as base64).
const BATCH_MOST: usize = 16;
/// A batch waits until the newest photo in it is this old (s): a burst of
/// photos goes together.
const SETTLE: i64 = 120;
/// Tries before a photo's caption is given up.
pub const TRIES: u32 = 8;

pub const SYSTEM: &str = "You caption photos for an FRC robotics team's build log: the engineering notebook of how the team designed, built and tested its robot.

Write one or two plain sentences, at most 40 words: what the photo shows, and the mechanism, part or task in it (for example the intake, a swerve module, the elevator, wiring, bumpers, a gearbox, the drive base, a test on the practice field, work in the pit). Be concrete and factual.

Never guess or invent the names of people, teams, sponsors or places. Use a name only if it can be read in the photo or is given in the context. If something is unclear, describe what can be seen.

Reply with the caption alone: no quotation marks, no Markdown, nothing before or after it.";

// ------------------------------------------------------------ the request ---

/// The photo, shrunk: a JPEG at most [`LONG`] on its long side, and its size.
pub fn shrink(path: &Path) -> Result<(Vec<u8>, usize, usize), String> {
    let (w, h) = crate::photos::dimensions(path).ok_or("not a photo this reads")?;
    let long = w.max(h).max(1);
    let k = if long > LONG { LONG as f64 / long as f64 } else { 1.0 };
    let (tw, th) = (((w as f64 * k).round() as usize).max(1), ((h as f64 * k).round() as usize).max(1));
    let pic = crate::photos::decode(path, tw, th)?;
    let small = if (pic.w, pic.h) == (tw, th) { pic } else { pic.cover(tw, th) };
    let mut out = Vec::with_capacity(160 * 1024);
    let encoder = jpeg_encoder::Encoder::new(&mut out, 82);
    encoder.encode(&small.rgb, small.w as u16, small.h as u16, jpeg_encoder::ColorType::Rgb).map_err(|e| format!("the photo could not be shrunk: {e}"))?;
    Ok((out, small.w, small.h))
}

/// Claude's image tokens for a picture: one for each 28-pixel square.
pub fn image_tokens(w: usize, h: usize) -> u64 {
    (w.div_ceil(28) * h.div_ceil(28)) as u64
}

/// What the photo's own words tell Claude: when, where, the event and match.
pub fn context(at: i64, about: &super::meta::About) -> String {
    let when = chrono::DateTime::from_timestamp(at, 0).map(|d| d.with_timezone(&chrono::Local).format("%a %-d %b %Y, %H:%M").to_string()).unwrap_or_default();
    let mut words = format!("Taken {when}");
    if !about.place.is_empty() {
        words.push_str(&format!(", at the place the team calls \"{}\"", about.place));
    }
    if !about.event.is_empty() {
        let venue = if about.venue.is_empty() { String::new() } else { format!(" in {}", about.venue) };
        words.push_str(&format!(", at the {}{venue}", about.event));
        if !about.context.is_empty() {
            words.push_str(&format!(", {} (the team's match)", about.context));
        }
    }
    words.push('.');
    format!("{words} Write the caption.")
}

/// A Messages request for one photo's caption.
pub fn params(jpeg: &[u8], context: &str) -> Value {
    json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "output_config": {"effort": "low"},
        "system": SYSTEM,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": base64(jpeg)}},
                {"type": "text", "text": context},
            ],
        }],
    })
}

/// A photo's name as a batch's `custom_id` (`^[a-zA-Z0-9_-]{1,64}$`).
pub fn custom_id(name: &str, n: usize) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let clean: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(56).collect();
    format!("{clean}-{n}")
}

pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(ABC[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// --------------------------------------------------------------- the cost ---

/// Prices, millionths of a dollar a token (= dollars a million tokens):
/// in, written to the cache, read from it, out. Opus 5.5's from Anthropic's
/// pricing page; another model's (a fallback) at Opus 5's, the higher.
fn prices(model: &str) -> (f64, f64, f64, f64) {
    match model {
        m if m.starts_with("claude-opus-5-5") => (4.0, 5.0, 0.2, 20.0),
        m if m.starts_with("claude-sonnet-5") => (2.0, 2.5, 0.2, 10.0),
        _ => (5.0, 6.25, 0.5, 25.0),
    }
}

/// What an answer cost, millionths of a dollar: its `usage` at its model's
/// prices, half in a batch.
pub fn cost(usage: &Value, model: &str, batch: bool) -> u64 {
    let n = |k: &str| usage[k].as_u64().unwrap_or(0) as f64;
    let (input, write, read, output) = prices(model);
    let full = n("input_tokens") * input + n("cache_creation_input_tokens") * write + n("cache_read_input_tokens") * read + n("output_tokens") * output;
    (if batch { full * 0.5 } else { full }).round() as u64
}

/// The most a photo of `w` × `h` (shrunk) can cost.
pub fn most(w: usize, h: usize, batch: bool) -> u64 {
    let (input, _, _, output) = prices(MODEL);
    let full = (image_tokens(w, h) + PROMPT_TOKENS) as f64 * input + f64::from(MAX_TOKENS) * output;
    (if batch { full * 0.5 } else { full }).round() as u64
}

/// "$0.42", "0.4¢".
pub fn money(micro: u64) -> String {
    if micro < 10_000 {
        format!("{:.1}¢", micro as f64 / 10_000.0)
    } else {
        format!("${:.2}", micro as f64 / 1e6)
    }
}

// ------------------------------------------------------------- the answer ---

/// What came of one photo's request.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Caption(String),
    /// Declined on safety grounds: not asked again.
    Declined,
    /// No caption in it (cut short): asked again.
    Empty,
}

/// A Messages answer's caption, tidied for the panel and the log.
pub fn caption_of(message: &Value) -> Outcome {
    if message["stop_reason"] == "refusal" {
        return Outcome::Declined;
    }
    let text: String = message["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(" ");
    let tidy = tidy(&text);
    if tidy.is_empty() {
        Outcome::Empty
    } else {
        Outcome::Caption(tidy)
    }
}

/// One line, no quotes or Markdown round it, at most [`CAPTION_MOST`].
pub fn tidy(text: &str) -> String {
    let mut s: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for lead in ["Caption:", "caption:"] {
        if let Some(rest) = s.strip_prefix(lead) {
            s = rest.trim().to_owned();
        }
    }
    let s = s.trim_matches(|c: char| matches!(c, '"' | '“' | '”' | '*' | '_' | '`' | '\'')).trim().to_owned();
    if s.chars().count() <= CAPTION_MOST {
        return s;
    }
    let cut: String = s.chars().take(CAPTION_MOST - 1).collect();
    format!("{}…", cut.trim_end())
}

// ---------------------------------------------------------- the transport ---

/// Where requests go, and with what key (the Assistant's).
pub struct Api {
    pub base: String,
    pub key: String,
    /// The simulator's: answered here, nothing sent.
    pub demo: bool,
    /// When the keys were last changed, as the key was read: a key refused
    /// is not tried again until this moves.
    pub stamp: i64,
}

impl Api {
    /// The real service, with the key from Settings › Keys.
    pub fn from_keys() -> Option<Api> {
        let stamp = keys_time();
        let key = crate::keys::Keys::load().get("claude")?.to_owned();
        Some(Api { base: ANTHROPIC.into(), key, demo: crate::ai::demo(), stamp })
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>, beta: Option<&str>) -> Result<Reply, String> {
        if self.demo {
            return Ok(demo_reply(method, path, body));
        }
        let mut headers = format!("x-api-key: {}\nanthropic-version: {VERSION}\n", self.key);
        if body.is_some() {
            headers.push_str("content-type: application/json\n");
        }
        if let Some(b) = beta {
            headers.push_str(&format!("anthropic-beta: {b}\n"));
        }
        let bytes = body.map(|b| b.to_string().into_bytes());
        net::call(method, &format!("{}{path}", self.base), &headers, bytes.as_deref(), AGENT)
    }
}

/// Why a reply is not what was asked for: whether to wait for the keys,
/// try again later, or give up on what was sent.
#[derive(Clone, Debug, PartialEq)]
pub enum Trouble {
    Key,
    Later(String),
    Bad(String),
}

fn trouble(r: &Reply) -> Trouble {
    let message = serde_json::from_str::<Value>(&r.body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_owned)).unwrap_or_default();
    match r.code {
        401 | 403 => Trouble::Key,
        408 | 409 | 429 | 500..=599 | 0 => Trouble::Later(if r.code == 429 { "busy, or out of credit".into() } else { "anthropic is busy".into() }),
        _ => Trouble::Bad(if message.is_empty() { format!("anthropic answered {}", r.code) } else { crate::ai::short(&message, 100) }),
    }
}

// ---------------------------------------------------------------- the work ---

/// What the handheld is like now, for background work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    pub online: bool,
    /// On the charger, or with battery to spare.
    pub power: bool,
    /// Not in deep standby, not short of memory.
    pub rested: bool,
}

impl Conditions {
    pub fn now() -> Conditions {
        use crate::music::polite;
        let power = polite::battery().is_none_or(|(charging, percent)| charging || percent >= 40);
        Conditions { online: polite::online(), power, rested: !polite::deep() && !polite::memory_short() }
    }

    #[cfg(test)]
    pub fn all() -> Conditions {
        Conditions { online: true, power: true, rested: true }
    }

    /// Why work waits, if it does.
    pub fn waits(&self) -> Option<&'static str> {
        match () {
            _ if !self.online => Some("waiting to be online"),
            _ if !self.power => Some("waiting for the charger or more battery"),
            _ if !self.rested => Some("waiting for the handheld to be idle"),
            _ => None,
        }
    }
}

/// What one round did.
#[derive(Clone, Debug, PartialEq)]
pub enum Did {
    /// Nothing to do now; the next thing is due then (Unix s), if any.
    Nothing(Option<i64>),
    /// Work waits on this: offline, the limit, no key, captions off.
    Waits(&'static str),
    Sent(usize),
    /// A batch was looked at and is not done yet.
    Polled,
    /// Captions taken in.
    Took(usize),
    /// Asking failed; it is tried again later.
    Failed(String),
}

fn keys_time() -> i64 {
    std::fs::metadata(crate::keys::path()).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

fn lock(m: &Mutex<Ledger>) -> std::sync::MutexGuard<'_, Ledger> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Back off: the next try of a photo's caption.
pub(super) fn retry(job: &mut Job, now: i64, why: &str) {
    let tries = match job {
        Job::Waiting { tries, .. } => *tries + 1,
        _ => 1,
    };
    *job = if tries >= TRIES { Job::Failed(format!("gave up after {TRIES} tries: {why}")) } else { Job::Waiting { tries, next: now + backoff(tries) } };
}

/// 1, 2, 4… minutes, at most six hours.
pub fn backoff(tries: u32) -> i64 {
    (60i64 << tries.min(9)).min(6 * 3600)
}

/// One round of captioning: a batch looked at, or one sent.
pub fn step(ledger: &Mutex<Ledger>, api: Option<&Api>, now: i64, cond: Conditions) -> Did {
    if !lock(ledger).prefs.captions {
        return Did::Waits("captions are off");
    }
    let Some(api) = api else { return Did::Waits("no claude key") };
    {
        let mut l = lock(ledger);
        if l.refused == Some(api.stamp) {
            return Did::Waits("claude refused the key");
        }
        // The keys changed since: tried again.
        if l.refused.is_some() {
            l.refused = None;
            l.save();
        }
        if let Some(why) = cond.waits() {
            return Did::Waits(why);
        }
        if now < l.pause_until {
            return Did::Nothing(Some(l.pause_until));
        }
    }
    // A batch due a look.
    let due = lock(ledger).batches.iter().find(|b| b.next_poll <= now).cloned();
    if let Some(batch) = due {
        return poll(ledger, api, &batch, now);
    }
    send(ledger, api, now)
}

/// When the next round is due, if anything waits.
pub fn next_due(l: &Ledger) -> Option<i64> {
    let polls = l.batches.iter().map(|b| b.next_poll);
    let newest = l.records.values().filter(|r| matches!(r.caption, Job::Waiting { .. })).map(|r| r.at).max();
    let photos = l.records.values().filter_map(|r| match r.caption {
        Job::Waiting { next, .. } => Some(next.max(newest.unwrap_or(0) + SETTLE)),
        _ => None,
    });
    polls.chain(photos).min().map(|t| t.max(l.pause_until))
}

/// The photos due, newest settled, as many as fit in a batch and under the
/// month's limit: (name, its path, when, its sidecar's words).
fn send(ledger: &Mutex<Ledger>, api: &Api, now: i64) -> Did {
    let (due, room) = {
        let l = lock(ledger);
        let newest = l.records.values().filter(|r| matches!(r.caption, Job::Waiting { .. })).map(|r| r.at).max().unwrap_or(0);
        if now < newest + SETTLE {
            return Did::Nothing(next_due(&l));
        }
        let mut due: Vec<(String, i64)> = l.records.iter().filter(|(_, r)| matches!(r.caption, Job::Waiting { next, .. } if next <= now)).map(|(n, r)| (n.clone(), r.at)).collect();
        if due.is_empty() {
            return Did::Nothing(next_due(&l));
        }
        due.sort_by_key(|(_, at)| *at);
        due.truncate(BATCH_MOST);
        (due.into_iter().map(|(n, at)| (l.photo_path(&n), n, at)).collect::<Vec<_>>(), l.room(&month_of(now)))
    };
    // Shrunk and put into requests outside the lock: the decoding takes a while.
    let mut requests = Vec::new();
    let mut items = Vec::new();
    let mut reserved = 0u64;
    let mut over = false;
    let mut broken = Vec::new();
    for (k, (path, name, at)) in due.iter().enumerate() {
        let (jpeg, w, h) = match shrink(path) {
            Ok(s) => s,
            Err(why) => {
                broken.push((name.clone(), why));
                continue;
            }
        };
        let most = most(w, h, true);
        if reserved + most > room {
            over = true;
            break;
        }
        reserved += most;
        let about = super::meta::read(path);
        let id = custom_id(name, k);
        requests.push(json!({"custom_id": id, "params": params(&jpeg, &context(*at, &about))}));
        items.push((id, name.clone()));
    }
    {
        let mut l = lock(ledger);
        for (name, why) in &broken {
            if let Some(r) = l.records.get_mut(name) {
                r.caption = Job::Failed(why.clone());
            }
        }
        if !broken.is_empty() {
            l.save();
        }
    }
    if requests.is_empty() {
        return if over { Did::Waits("the month's limit is reached") } else { Did::Nothing(next_due(&lock(ledger))) };
    }
    let reply = api.call("POST", "/v1/messages/batches", Some(&json!({"requests": requests})), None);
    let mut l = lock(ledger);
    let result = match reply {
        Ok(r) if r.code == 200 => serde_json::from_str::<Value>(&r.body).ok().and_then(|v| v["id"].as_str().map(str::to_owned)).ok_or(Trouble::Later("its answer could not be read".into())),
        Ok(r) => Err(trouble(&r)),
        Err(why) => Err(Trouble::Later(why)),
    };
    let did = match result {
        Ok(id) => {
            for (_, name) in &items {
                if let Some(r) = l.records.get_mut(name) {
                    r.caption = Job::Sent(id.clone());
                }
            }
            l.batches.push(Batch { id, sent: now, next_poll: now + 60, polls: 0, items: items.clone(), reserved });
            l.pauses = 0;
            Did::Sent(items.len())
        }
        Err(t) => fail_all(&mut l, &items, now, t, api.stamp),
    };
    l.save();
    did
}

fn fail_all(l: &mut Ledger, items: &[(String, String)], now: i64, t: Trouble, stamp: i64) -> Did {
    match t {
        Trouble::Key => {
            l.refused = Some(stamp);
            Did::Waits("claude refused the key")
        }
        Trouble::Later(why) => {
            l.pauses += 1;
            l.pause_until = now + backoff(l.pauses);
            Did::Failed(why)
        }
        Trouble::Bad(why) => {
            for (_, name) in items {
                if let Some(r) = l.records.get_mut(name) {
                    retry(&mut r.caption, now, &why);
                }
            }
            Did::Failed(why)
        }
    }
}

/// How long until a batch is looked at again: 1, 2, 4, 8 minutes, then
/// every ten.
fn poll_after(polls: u32) -> i64 {
    (60i64 << polls.min(4)).min(600)
}

fn poll(ledger: &Mutex<Ledger>, api: &Api, batch: &Batch, now: i64) -> Did {
    let path = format!("/v1/messages/batches/{}", batch.id);
    let status = api.call("GET", &path, None, None);
    let ended = match status {
        Ok(r) if r.code == 200 => serde_json::from_str::<Value>(&r.body).ok().is_some_and(|v| v["processing_status"] == "ended"),
        Ok(r) if r.code == 404 => {
            // Gone at Anthropic (deleted, or past its 29 days): ask again.
            let mut l = lock(ledger);
            for (_, name) in &batch.items {
                if let Some(rec) = l.records.get_mut(name) {
                    if rec.caption == Job::Sent(batch.id.clone()) {
                        retry(&mut rec.caption, now, "the batch was gone");
                    }
                }
            }
            l.batches.retain(|b| b.id != batch.id);
            l.save();
            return Did::Failed("the batch was gone".into());
        }
        Ok(r) => return later(ledger, batch, now, trouble(&r), api.stamp),
        Err(why) => return later(ledger, batch, now, Trouble::Later(why), api.stamp),
    };
    if !ended {
        let mut l = lock(ledger);
        if let Some(b) = l.batches.iter_mut().find(|b| b.id == batch.id) {
            b.polls += 1;
            b.next_poll = now + poll_after(b.polls);
        }
        l.save();
        return Did::Polled;
    }
    let results = match api.call("GET", &format!("{path}/results"), None, None) {
        Ok(r) if r.code == 200 => r.body,
        Ok(r) => return later(ledger, batch, now, trouble(&r), api.stamp),
        Err(why) => return later(ledger, batch, now, Trouble::Later(why), api.stamp),
    };
    let took = take_results(ledger, batch, &results, now);
    // Read: nothing more is kept at Anthropic than must be.
    api.call("DELETE", &path, None, None).ok();
    took
}

/// A batch could not be looked at: looked at again later.
fn later(ledger: &Mutex<Ledger>, batch: &Batch, now: i64, t: Trouble, stamp: i64) -> Did {
    let mut l = lock(ledger);
    if let Some(b) = l.batches.iter_mut().find(|b| b.id == batch.id) {
        b.polls += 1;
        b.next_poll = now + poll_after(b.polls).max(300);
    }
    if t == Trouble::Key {
        l.refused = Some(stamp);
    }
    l.save();
    Did::Failed(match t {
        Trouble::Key => "claude refused the key".into(),
        Trouble::Later(w) | Trouble::Bad(w) => w,
    })
}

/// A batch's answers (JSON lines, in any order), each taken by its
/// `custom_id`.
pub fn take_results(ledger: &Mutex<Ledger>, batch: &Batch, jsonl: &str, now: i64) -> Did {
    let mut l = lock(ledger);
    let month = month_of(now);
    let mut took = 0;
    let mut answered: Vec<&str> = Vec::new();
    for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(id) = v["custom_id"].as_str() else { continue };
        let Some((cid, name)) = batch.items.iter().find(|(c, _)| c == id) else { continue };
        answered.push(cid.as_str());
        let path = l.photo_path(name);
        let result = &v["result"];
        let (job, spent) = match result["type"].as_str() {
            Some("succeeded") => {
                let message = &result["message"];
                let spent = cost(&message["usage"], message["model"].as_str().unwrap_or(MODEL), true);
                match caption_of(message) {
                    Outcome::Caption(text) => {
                        let mut about = super::meta::read(&path);
                        // A caption written by hand meanwhile stays.
                        if about.caption.is_empty() || about.by != "you" {
                            about.caption = text;
                            about.by = MODEL.into();
                            super::meta::write(&path, &about).ok();
                        }
                        took += 1;
                        (Job::Done, spent)
                    }
                    Outcome::Declined => (Job::Skipped("claude declined to caption it".into()), spent),
                    Outcome::Empty => {
                        let mut j = l.records.get(name).map(|r| r.caption.clone()).unwrap_or_default();
                        retry(&mut j, now, "no caption came back");
                        (j, spent)
                    }
                }
            }
            Some("errored") => {
                // The API's error shape: {"type": "error", "error": {"type", "message"}}.
                let e = if result["error"]["error"].is_object() { &result["error"]["error"] } else { &result["error"] };
                let kind = e["type"].as_str().unwrap_or("");
                let mut j = Job::Waiting { tries: 0, next: now };
                if kind.contains("invalid_request") {
                    let why = e["message"].as_str().unwrap_or("anthropic could not read the request");
                    j = Job::Failed(crate::ai::short(why, 100));
                } else {
                    retry(&mut j, now, "anthropic had trouble with it");
                }
                (j, 0)
            }
            // Canceled, or expired after a day: asked again.
            _ => {
                let mut j = Job::Waiting { tries: 0, next: now };
                retry(&mut j, now, "the batch ran out of time");
                (j, 0)
            }
        };
        if let Some(r) = l.records.get_mut(name) {
            r.caption = job;
            r.cost += spent;
        }
        *l.spend.entry(month.clone()).or_insert(0) += spent;
    }
    // Any it said nothing of: asked again.
    for (cid, name) in &batch.items {
        if !answered.contains(&cid.as_str()) {
            if let Some(r) = l.records.get_mut(name) {
                if r.caption == Job::Sent(batch.id.clone()) {
                    retry(&mut r.caption, now, "no answer for it");
                }
            }
        }
    }
    l.batches.retain(|b| b.id != batch.id);
    l.save();
    Did::Took(took)
}

/// One photo's caption asked for at once (full price), whatever else waits.
pub fn now_for(ledger: &Mutex<Ledger>, api: &Api, name: &str, now: i64) -> Result<String, String> {
    let (path, at, room, month) = {
        let l = lock(ledger);
        if !l.prefs.captions {
            return Err("captions are off: Settings › Photos".into());
        }
        super::captionable(name).map_err(str::to_owned)?;
        let month = month_of(now);
        let at = l.records.get(name).map_or(now, |r| r.at);
        (l.photo_path(name), at, l.room(&month), month)
    };
    let (jpeg, w, h) = shrink(&path)?;
    if most(w, h, false) > room {
        return Err("the month's limit for captions is reached".into());
    }
    let about = super::meta::read(&path);
    let mut body = params(&jpeg, &context(at, &about));
    body["fallbacks"] = json!("default");
    let reply = api.call("POST", "/v1/messages", Some(&body), Some(FALLBACK_BETA));
    let mut l = lock(ledger);
    let r = match reply {
        Ok(r) if r.code == 200 => r,
        Ok(r) => {
            return Err(match trouble(&r) {
                Trouble::Key => {
                    l.refused = Some(api.stamp);
                    l.save();
                    "claude refused the key · Settings › Keys".into()
                }
                Trouble::Later(w) | Trouble::Bad(w) => w,
            })
        }
        Err(why) => return Err(why),
    };
    let message: Value = serde_json::from_str(&r.body).map_err(|_| "the answer could not be read".to_owned())?;
    let spent = cost(&message["usage"], message["model"].as_str().unwrap_or(MODEL), false);
    *l.spend.entry(month).or_insert(0) += spent;
    let record = l.records.entry(name.to_owned()).or_insert_with(|| super::store::Record { at, ..Default::default() });
    record.cost += spent;
    let result = match caption_of(&message) {
        Outcome::Caption(text) => {
            let mut about = super::meta::read(&path);
            about.caption = text.clone();
            about.by = MODEL.into();
            match super::meta::write(&path, &about) {
                Ok(()) => {
                    record.caption = Job::Done;
                    Ok(text)
                }
                Err(_) => Err("the caption could not be kept".to_owned()),
            }
        }
        Outcome::Declined => {
            record.caption = Job::Skipped("claude declined to caption it".into());
            Err("claude declined to caption it".into())
        }
        Outcome::Empty => Err("no caption came back · try again".into()),
    };
    l.save();
    result
}

// ------------------------------------------------------------------ demo ---

/// The simulator's Anthropic: answers in the API's own shapes (so the real
/// reading runs), nothing sent anywhere.
fn demo_reply(method: &str, path: &str, body: Option<&Value>) -> Reply {
    let usage = json!({"input_tokens": 1311, "output_tokens": 96, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0});
    let message = |text: &str| json!({"id": "msg_demo", "type": "message", "role": "assistant", "model": MODEL, "content": [{"type": "text", "text": text}], "stop_reason": "end_turn", "usage": usage});
    let ok = |v: Value| Reply { code: 200, body: v.to_string() };
    match (method, path) {
        ("POST", "/v1/messages") => ok(message(DEMO_CAPTIONS[0])),
        ("POST", "/v1/messages/batches") => {
            let ids: Vec<String> = body.and_then(|b| b["requests"].as_array()).map(|a| a.iter().filter_map(|r| r["custom_id"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
            DEMO_BATCH.lock().map(|mut b| *b = ids).ok();
            ok(json!({"id": "msgbatch_demo", "type": "message_batch", "processing_status": "in_progress"}))
        }
        ("GET", p) if p.ends_with("/results") => {
            let ids = DEMO_BATCH.lock().map(|b| b.clone()).unwrap_or_default();
            let lines: Vec<String> = ids.iter().enumerate().map(|(k, id)| json!({"custom_id": id, "result": {"type": "succeeded", "message": message(DEMO_CAPTIONS[k % DEMO_CAPTIONS.len()])}}).to_string()).collect();
            Reply { code: 200, body: lines.join("\n") }
        }
        ("GET", _) => ok(json!({"id": "msgbatch_demo", "type": "message_batch", "processing_status": "ended"})),
        ("DELETE", _) => ok(json!({"id": "msgbatch_demo", "type": "message_batch_deleted"})),
        _ => Reply { code: 404, body: String::new() },
    }
}

static DEMO_BATCH: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the simulator's photos are said to show.
pub const DEMO_CAPTIONS: [&str; 6] = [
    "The robot on the practice field at the end of the afternoon, bumpers on, lined up for a drive test.",
    "Drive base on the field after a run, with the intake lowered at the front.",
    "The robot parked by the field's edge between practice runs, wiring tucked behind the bumpers.",
    "A practice run under way, the robot turning toward the far end of the field.",
    "The robot at rest on the practice field in the evening light, ready for the next test.",
    "Lining the robot up at the starting line before an autonomous test.",
];
