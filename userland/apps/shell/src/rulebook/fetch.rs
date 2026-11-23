//! Getting FIRST's manual onto the handheld, and keeping it current.
//!
//! FIRST posts the game manual on its own file host in two forms: a PDF and
//! the same document as HTML (which Word writes from it). The HTML is read
//! here: its sections, rules and blue boxes are marked in it, where a PDF
//! would need a text extractor too heavy for this handheld. It is about
//! 1.2 MB, and read in well under a second.
//!
//! The manual is FIRST's, and not in the system image: each handheld
//! downloads it from FIRST, as a team member would, and keeps only what it
//! reads from it, for its own use. When it is online it asks FIRST whether
//! the file changed (its Last-Modified and ETag, a request of a few hundred
//! bytes), and when it did (a team update revises the manual), downloads it
//! again and notes which rules changed.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

use super::store::{self, Meta};

/// The 2026 manual (REBUILT), as FIRST posts it.
pub const MANUAL_URL: &str = "https://firstfrc.blob.core.windows.net/frc2026/Manual/HTML/2026GameManual.htm";
/// Its size when it was last looked at, for progress before the server says.
pub const SIZE_GUESS: u64 = 1_211_124;
/// How often a kept manual is looked at again for a newer one, when online.
pub const CHECK_EVERY: i64 = 6 * 3600;

/// How a download stands, shared with the page drawing it.
#[derive(Default)]
pub struct Progress {
    pub bytes: AtomicU64,
    pub total: AtomicU64,
    /// 0 asking, 1 downloading, 2 reading the rules.
    pub stage: AtomicU8,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(1);
        (self.bytes.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// A manual was kept: the first, or a newer one.
    Kept(Meta),
    /// The one kept is FIRST's latest.
    Current,
}

/// The simulator and the tests use the sample, unless told to fetch.
pub fn live() -> bool {
    !crate::ai::demo() || std::env::var_os("CATALYST_RULEBOOK_LIVE").is_some()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The headers of the last response in curl's `-I` output (after redirects).
pub fn headers(text: &str) -> (Option<String>, Option<String>, Option<u64>) {
    let last = text.rsplit("\nHTTP/").next().unwrap_or(text);
    let mut out = (None, None, None);
    for line in last.lines() {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim().to_owned();
        match name.trim().to_ascii_lowercase().as_str() {
            "last-modified" => out.0 = Some(value),
            "etag" => out.1 = Some(value),
            "content-length" => out.2 = value.parse().ok(),
            _ => {}
        }
    }
    out
}

fn curl_failure(code: Option<i32>) -> String {
    match code {
        Some(6) => "no internet: firstfrc.blob.core.windows.net did not resolve".into(),
        Some(7) => "could not reach FIRST's server".into(),
        Some(22) => "FIRST's server did not have the manual there".into(),
        Some(28) => "FIRST's server took too long".into(),
        Some(35 | 60) => "the secure connection failed · is the clock right?".into(),
        Some(23) => "the card is full".into(),
        _ => "the download failed".into(),
    }
}

/// Whether the manual kept is the one FIRST has now.
fn same(meta: &Meta, modified: &Option<String>, etag: &Option<String>) -> bool {
    match (&meta.etag, etag, &meta.modified, modified) {
        (Some(a), Some(b), _, _) => a == b,
        (_, _, Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Look for FIRST's manual and keep it if it is newer than the one here
/// (or `again`, or none is kept). Call it on a thread of its own.
pub fn fetch(dir: &Path, again: bool, progress: &Progress) -> Result<Outcome, String> {
    let kept = store::Book::open(dir).ok().map(|b| b.meta);
    if !live() {
        return sample(dir, kept, again, progress);
    }
    progress.stage.store(0, Ordering::Relaxed);
    let head = Command::new("curl")
        .args(["-sSIL", "--max-time", "20", "--proto", "=https", MANUAL_URL])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "curl could not start".to_owned())?;
    if !head.status.success() {
        return Err(curl_failure(head.status.code()));
    }
    let (modified, etag, length) = headers(&String::from_utf8_lossy(&head.stdout));
    if let Some(meta) = &kept {
        if !again && same(meta, &modified, &etag) {
            store::checked(dir, now());
            return Ok(Outcome::Current);
        }
    }
    std::fs::create_dir_all(dir).map_err(|_| "could not make a place for it on the card".to_owned())?;
    let part = dir.join(".manual.part");
    std::fs::remove_file(&part).ok();
    progress.total.store(length.unwrap_or(SIZE_GUESS), Ordering::Relaxed);
    progress.bytes.store(0, Ordering::Relaxed);
    progress.stage.store(1, Ordering::Relaxed);
    let mut child = Command::new("curl")
        .args(["-sSfL", "--max-time", "180", "--proto", "=https", "-o"])
        .arg(&part)
        .arg(MANUAL_URL)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "curl could not start".to_owned())?;
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break status;
        }
        progress.bytes.store(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        std::fs::remove_file(&part).ok();
        return Err(curl_failure(status.code()));
    }
    let bytes = std::fs::read(&part).map_err(|_| "the download could not be read".to_owned())?;
    std::fs::remove_file(&part).ok();
    progress.bytes.store(bytes.len() as u64, Ordering::Relaxed);
    keep(dir, &bytes, Meta { source: MANUAL_URL.into(), modified, etag, fetched: now(), checked: now(), ..Meta::default() }, progress)
}

/// Read a downloaded manual and keep it.
pub fn keep(dir: &Path, bytes: &[u8], mut meta: Meta, progress: &Progress) -> Result<Outcome, String> {
    progress.stage.store(2, Ordering::Relaxed);
    let manual = super::parse::parse(bytes)?;
    meta.title = manual.title.clone();
    store::write(dir, &manual, meta).map(Outcome::Kept)
}

/// The simulator's stand-in: the sample, as if downloaded.
fn sample(dir: &Path, kept: Option<Meta>, again: bool, progress: &Progress) -> Result<Outcome, String> {
    if kept.is_some() && !again {
        store::checked(dir, now());
        return Ok(Outcome::Current);
    }
    let bytes = super::sample::bytes();
    progress.total.store(bytes.len() as u64, Ordering::Relaxed);
    progress.stage.store(1, Ordering::Relaxed);
    for k in 1..=10u64 {
        progress.bytes.store(bytes.len() as u64 * k / 10, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(40));
    }
    let meta = Meta { source: MANUAL_URL.into(), modified: Some("Wed, 22 Apr 2026 00:08:20 GMT".into()), fetched: now(), checked: now(), sample: true, ..Meta::default() };
    keep(dir, &bytes, meta, progress)
}

/// "apr 22": a Last-Modified date, short.
pub fn short_date(modified: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc2822(modified).ok()?;
    Some(t.format("%b %-d").to_string().to_lowercase())
}
