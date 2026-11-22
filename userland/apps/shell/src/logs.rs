//! Match logs on the handheld: pulled from the robot's Catalyst Agent in the
//! background, summarised once, and cleared after 30 days.
//!
//! Pulling resumes: a log is appended to from wherever the local copy ends,
//! so an unplugged cable or a log still being written costs nothing. The
//! summary of each log is kept beside it as a small JSON digest.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::agent;
use crate::wpilog::{self, Level, Summary};

/// Logs older than this are cleared.
pub const KEEP_DAYS: u64 = 30;
/// Before this (2026-01-01) the clock has not been set, and ages mean nothing.
const CLOCK_TRUSTED: u64 = 1_767_225_600;
/// Space kept free on the data partition, whatever the age of the logs: this
/// much, or a quarter of the partition if that is less (a card whose data
/// partition never grew is 64 MB, and 256 MB free would clear every log).
const KEEP_FREE: u64 = 256 * 1024 * 1024;
/// Logs cleared from this folder, one name a line: the robot still has them,
/// and they are not pulled again.
const CLEARED: &str = ".cleared";

fn keep_free(partition: u64) -> u64 {
    KEEP_FREE.min(partition / 4)
}

/// The digest's layout: 2 added what trends across matches read (brownouts,
/// loop overruns, the hottest motor, CAN). A digest of an older layout is
/// made again when its log is next looked at.
const DIGEST_LAYOUT: u64 = 2;

/// What the screens show for one log.
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    pub seconds: f64,
    pub enabled_seconds: Option<f64>,
    pub battery_low: Option<f64>,
    pub worst_loop_ms: Option<f64>,
    pub highlights: Vec<(Level, String)>,
    /// For trends across matches; absent from a digest an older shell wrote.
    pub brownouts: Option<u32>,
    pub loop_overruns: Option<u32>,
    /// The hottest mechanism and its peak, °C.
    pub hottest: Option<(String, f64)>,
    /// The busiest CAN bus at its busiest, 0 to 1.
    pub can_peak: Option<f64>,
}

impl Digest {
    pub fn of(summary: &Summary) -> Self {
        Digest {
            seconds: summary.seconds,
            enabled_seconds: summary.enabled_seconds,
            battery_low: summary.battery_low,
            worst_loop_ms: summary.worst_loop_ms,
            highlights: summary.highlights(),
            brownouts: Some(summary.brownouts),
            loop_overruns: Some(summary.loop_overruns),
            hottest: summary.mechanisms.iter().find_map(|(name, temp, _)| temp.map(|t| (name.clone(), t))),
            can_peak: summary.can_peak,
        }
    }

    /// The single line a list row carries: the worst finding.
    pub fn headline(&self) -> (Level, String) {
        self.highlights.first().cloned().unwrap_or((Level::Fine, "nothing stood out".into()))
    }

    fn to_json(&self, bytes: u64) -> Value {
        let level = |l: &Level| match l {
            Level::Fault => "fault",
            Level::Warn => "warn",
            Level::Fine => "fine",
        };
        json!({
            "layout": DIGEST_LAYOUT,
            "bytes": bytes, "seconds": self.seconds, "enabledSeconds": self.enabled_seconds,
            "batteryLow": self.battery_low, "worstLoopMs": self.worst_loop_ms,
            "highlights": self.highlights.iter().map(|(l, text)| json!([level(l), text])).collect::<Vec<_>>(),
            "brownouts": self.brownouts, "loopOverruns": self.loop_overruns,
            "hottest": self.hottest.as_ref().map(|(name, temp)| json!([name, temp])),
            "canPeak": self.can_peak,
        })
    }

    /// Whether a digest on disk is of the current layout.
    fn current(v: &Value) -> bool {
        v["layout"].as_u64().is_some_and(|l| l >= DIGEST_LAYOUT)
    }

    /// The digest, if it was made from a file of exactly `bytes`.
    fn from_json(v: &Value, bytes: u64) -> Option<Self> {
        if v["bytes"].as_u64()? != bytes {
            return None;
        }
        let highlights = v["highlights"]
            .as_array()?
            .iter()
            .filter_map(|pair| {
                let level = match pair[0].as_str()? {
                    "fault" => Level::Fault,
                    "warn" => Level::Warn,
                    _ => Level::Fine,
                };
                Some((level, pair[1].as_str()?.to_owned()))
            })
            .collect();
        Some(Digest {
            seconds: v["seconds"].as_f64()?,
            enabled_seconds: v["enabledSeconds"].as_f64(),
            battery_low: v["batteryLow"].as_f64(),
            worst_loop_ms: v["worstLoopMs"].as_f64(),
            highlights,
            brownouts: v["brownouts"].as_u64().map(|n| n as u32),
            loop_overruns: v["loopOverruns"].as_u64().map(|n| n as u32),
            hottest: v["hottest"].as_array().and_then(|pair| Some((pair.first()?.as_str()?.to_owned(), pair.get(1)?.as_f64()?))),
            can_peak: v["canPeak"].as_f64(),
        })
    }
}

/// One log as the list shows it.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub bytes: u64,
    pub digest: Option<Digest>,
}

impl Row {
    /// "nov 16 · 10:15" from WPILib's `FRC_20261116_101500[_EVENT_Q12].wpilog`,
    /// with the match in front when the name carries one.
    pub fn title(&self) -> String {
        let stem = self.name.trim_end_matches(".wpilog");
        let parts: Vec<&str> = stem.split('_').collect();
        const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
        if let ["FRC", date, time, rest @ ..] = &parts[..] {
            if date.len() == 8 && time.len() == 6 && date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit()) {
                let month = date[4..6].parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))).copied().unwrap_or("?");
                let when = format!("{month} {} · {}:{}", date[6..8].trim_start_matches('0'), &time[..2], &time[2..4]);
                return if rest.is_empty() { when } else { format!("{} · {when}", rest.join(" ").to_lowercase()) };
            }
        }
        stem.to_owned()
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Which logs to clear: those past `KEEP_DAYS`, then the oldest until `keep`
/// is free. `files` is (name, modified, bytes); `free` is the space left on
/// the partition, when known.
pub fn to_clear(files: &[(String, u64, u64)], now: u64, free: Option<u64>, keep: u64) -> Vec<String> {
    let mut oldest_first: Vec<&(String, u64, u64)> = files.iter().collect();
    oldest_first.sort_by_key(|f| f.1);
    let mut clear = Vec::new();
    let mut free = free;
    for (name, modified, bytes) in oldest_first {
        // Ages only mean something once the clock has been set.
        let expired = now >= CLOCK_TRUSTED && *modified >= CLOCK_TRUSTED && now.saturating_sub(*modified) > KEEP_DAYS * 86_400;
        let cramped = free.is_some_and(|f| f < keep);
        if expired || cramped {
            clear.push(name.clone());
            free = free.map(|f| f + bytes);
        }
    }
    clear
}

/// The space left on the partition holding `dir`, and its size.
#[cfg(target_os = "linux")]
fn space(dir: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0).then(|| (stat.f_bavail as u64 * stat.f_frsize as u64, stat.f_blocks as u64 * stat.f_frsize as u64))
}

#[cfg(not(target_os = "linux"))]
fn space(_dir: &Path) -> Option<(u64, u64)> {
    None
}

/// The logs cleared from `dir`, which are not to be pulled again.
fn cleared(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join(CLEARED)).map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default()
}

fn remember_cleared(dir: &Path, names: &[String]) {
    let mut all = cleared(dir);
    all.extend(names.iter().cloned());
    // The robot keeps a season's logs at most: the last thousand names are plenty.
    let from = all.len().saturating_sub(1000);
    crate::sys::write_atomic(&dir.join(CLEARED), (all[from..].join("\n") + "\n").as_bytes()).ok();
}

#[derive(Default)]
struct Shared {
    allowed: AtomicBool,
    stop: AtomicBool,
    /// The wait between looks at the robot, which a stop ends.
    nap: crate::deep::Nap,
    generation: AtomicU64,
    status: Mutex<String>,
    /// The log being pulled: (name, bytes so far, bytes in all).
    pulling: Mutex<Option<(String, u64)>>,
    pulled_bytes: AtomicU64,
}

/// The logs for one robot.
pub struct Logs {
    dir: Option<PathBuf>,
    shared: Arc<Shared>,
    demo: Vec<Row>,
    /// The last listing and the generation it was read at: screens ask every
    /// frame, the folder changes a few times an hour.
    listed: Mutex<Option<(u64, Vec<Row>)>>,
    /// The demo's newest match, held back until this many seconds have
    /// passed, as if it were being pulled: so a demo robot plugged in shows
    /// what happens when a match's log arrives.
    demo_held: Option<f32>,
}

impl Logs {
    /// No robot: nothing to show or pull.
    pub fn none() -> Self {
        Logs { dir: None, shared: Arc::new(Shared::default()), demo: Vec::new(), listed: Mutex::new(None), demo_held: None }
    }

    /// An event's worth of matches, for the demo robot: a day and a half of
    /// qualifying in which the robot slowly gets worse (the battery sags
    /// further, the elevator runs hotter, the loop starts to overrun) until
    /// the newest match browns out. Each is a whole log made up as a robot
    /// writes one (`offline::synth`), summarised as a real one is, so Logs,
    /// Trends and the assistant offline tell the same story. And a practice
    /// log not yet read.
    pub fn demo() -> Self {
        let mut demo: Vec<Row> = crate::offline::demo_logs().iter().map(|(name, bytes, summary)| Row { name: name.clone(), bytes: *bytes, digest: Some(Digest::of(summary)) }).collect();
        demo.insert(3.min(demo.len()), Row { name: "FRC_20261115_171800.wpilog".into(), bytes: 31_250_000, digest: None });
        let logs = Logs { dir: None, shared: Arc::new(Shared::default()), demo, listed: Mutex::new(None), demo_held: None };
        *logs.shared.status.lock().unwrap() = "demo logs".into();
        logs
    }

    /// The demo logs, with the newest match arriving `seconds` from now as
    /// though it were being pulled from a robot just back from that match.
    pub fn demo_arriving(seconds: f32) -> Self {
        let mut logs = Self::demo();
        logs.demo_held = Some(seconds);
        logs
    }

    /// Time passes for the demo's held-back match.
    pub fn advance(&mut self, dt: f32) {
        if let Some(left) = &mut self.demo_held {
            *left -= dt;
            if *left <= 0.0 {
                self.demo_held = None;
                self.shared.generation.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Keep the logs of the robot at `host`, pulling new ones in the background.
    pub fn for_robot(host: &str) -> Self {
        let dir = kept_dir();
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "waiting for the robot".into();
        if fs::create_dir_all(&dir).is_err() {
            *shared.status.lock().unwrap() = "no place to keep logs".into();
            return Logs { dir: None, shared, demo: Vec::new(), listed: Mutex::new(None), demo_held: None };
        }
        let (thread_shared, thread_dir, host) = (shared.clone(), dir.clone(), host.to_owned());
        thread::Builder::new()
            .name("logs".into())
            .stack_size(512 * 1024)
            .spawn(move || pull_loop(&thread_shared, &thread_dir, &host))
            .expect("spawn logs thread");
        Logs { dir: Some(dir), shared, demo: Vec::new(), listed: Mutex::new(None), demo_held: None }
    }

    /// Pulling only happens while this is true: linked, and not enabled.
    pub fn set_allowed(&self, allowed: bool) {
        self.shared.allowed.store(allowed, Ordering::Relaxed);
    }

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed) + self.shared.pulled_bytes.load(Ordering::Relaxed) / (256 * 1024)
    }

    /// What the puller is doing, in a few words.
    pub fn status(&self) -> String {
        if let (Some(left), Some(row)) = (self.demo_held, self.demo.first()) {
            return format!("pulling {} · {}%", row.title(), ((1.0 - left / 4.0).clamp(0.0, 0.99) * 100.0) as u32);
        }
        if let Some((name, total)) = self.shared.pulling.lock().unwrap().clone() {
            let done = self.shared.pulled_bytes.load(Ordering::Relaxed);
            let row = Row { name, bytes: 0, digest: None };
            return format!("pulling {} · {}%", row.title(), (done * 100).checked_div(total).unwrap_or(0).min(100));
        }
        self.shared.status.lock().unwrap().clone()
    }

    /// The logs kept here, newest first.
    pub fn list(&self) -> Vec<Row> {
        let Some(dir) = &self.dir else {
            let held = if self.demo_held.is_some() { 1 } else { 0 };
            return self.demo[held.min(self.demo.len())..].to_vec();
        };
        let generation = self.shared.generation.load(Ordering::Relaxed);
        if let Some((at, rows)) = &*self.listed.lock().unwrap() {
            if *at == generation {
                return rows.clone();
            }
        }
        let rows = rows_in(dir);
        *self.listed.lock().unwrap() = Some((generation, rows.clone()));
        rows
    }

    /// The folder the logs are kept in, for a robot that has one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The demo robot's made-up event.
    pub fn is_demo(&self) -> bool {
        self.dir.is_none() && !self.demo.is_empty()
    }
}

/// Where logs are kept: `/data/logs` on the handheld (`CATALYST_LOGS` elsewhere).
pub fn kept_dir() -> PathBuf {
    std::env::var_os("CATALYST_LOGS").map(PathBuf::from).unwrap_or_else(|| "/data/logs".into())
}

/// The logs in a folder, newest first, with their digests where made.
pub fn rows_in(dir: &Path) -> Vec<Row> {
    let mut rows: Vec<Row> = local_files(dir)
        .into_iter()
        .map(|(name, _, bytes)| {
            let digest = fs::read(dir.join(format!("{name}.json"))).ok().and_then(|text| serde_json::from_slice::<Value>(&text).ok()).and_then(|v| Digest::from_json(&v, bytes));
            Row { name, bytes, digest }
        })
        .collect();
    // WPILib names sort by time.
    rows.sort_by(|a, b| b.name.cmp(&a.name));
    rows
}

impl Drop for Logs {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.nap.stop();
    }
}

/// (name, modified, bytes) of every log in the folder.
fn local_files(dir: &Path) -> Vec<(String, u64, u64)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            (name.ends_with(".wpilog") && meta.is_file()).then_some((name, modified, meta.len()))
        })
        .collect()
}

/// Clear what is due, and as many of the oldest as it takes for `room` more
/// bytes to fit as well, never the log `making_room_for`. Returns how many
/// logs went.
pub fn clean(dir: &Path, room: u64, making_room_for: Option<&str>) -> usize {
    let mut files = local_files(dir);
    files.retain(|(name, ..)| Some(name.as_str()) != making_room_for);
    let time = now();
    // A log pulled before the clock was set gets its age from when the clock came good.
    if time >= CLOCK_TRUSTED {
        for (name, modified, _) in &files {
            if *modified < CLOCK_TRUSTED {
                if let Ok(f) = File::options().write(true).open(dir.join(name)) {
                    f.set_modified(SystemTime::now()).ok();
                }
            }
        }
    }
    let space = space(dir);
    let keep = space.map_or(KEEP_FREE, |(_, size)| keep_free(size));
    let clear = to_clear(&files, time, space.map(|(free, _)| free), keep + room);
    for name in &clear {
        fs::remove_file(dir.join(name)).ok();
        fs::remove_file(dir.join(format!("{name}.json"))).ok();
        fs::remove_file(crate::offline::facts::sheet_path(dir, name)).ok();
    }
    if !clear.is_empty() {
        remember_cleared(dir, &clear);
    }
    clear.len()
}

/// Write the digest of a log beside it. (Its sheet of facts for the
/// assistant offline is not made here: only when a question first needs it,
/// so nothing is worked out that nobody asks about.)
fn digest_file(dir: &Path, name: &str) {
    let path = dir.join(name);
    let Ok(bytes) = fs::metadata(&path).map(|m| m.len()) else { return };
    let Ok(file) = File::open(&path) else { return };
    if let Ok(summary) = wpilog::summarize(std::io::BufReader::with_capacity(64 * 1024, file)) {
        let json = Digest::of(&summary).to_json(bytes).to_string();
        crate::sys::write_atomic(&dir.join(format!("{name}.json")), json.as_bytes()).ok();
    }
}

fn pull_loop(shared: &Shared, dir: &Path, host: &str) {
    let set = |text: String| {
        let mut status = shared.status.lock().unwrap();
        if *status != text {
            *status = text;
            shared.generation.fetch_add(1, Ordering::Relaxed);
        }
    };
    let mut last_clean = 0u64;
    loop {
        if now().saturating_sub(last_clean) >= 3600 || last_clean == 0 {
            last_clean = now().max(1);
            if clean(dir, 0, None) > 0 {
                shared.generation.fetch_add(1, Ordering::Relaxed);
            }
        }
        if shared.allowed.load(Ordering::Relaxed) {
            match pull_once(shared, dir, host) {
                Ok(count) => set(format!("up to date · {count} kept")),
                Err(why) => set(why),
            }
        } else {
            set("waiting: robot away or enabled".into());
        }
        // Fifteen seconds in one wait (a stop ends it; in deep standby it
        // parks after): one wakeup, where steps of 200 ms were seventy-five.
        if shared.nap.nap(Duration::from_secs(15)) || shared.stop.load(Ordering::Relaxed) {
            return;
        }
    }
}

/// Bring every log the robot has up to its full length here, newest first.
fn pull_once(shared: &Shared, dir: &Path, host: &str) -> Result<usize, String> {
    let listing = agent::get(host, "/api/logs").map_err(|why| if why.contains("404") { "robot's agent is too old for logs".to_string() } else { why })?;
    let empty = Vec::new();
    let gone = cleared(dir);
    for entry in listing["logs"].as_array().unwrap_or(&empty) {
        let (Some(name), Some(total)) = (entry["name"].as_str(), entry["bytes"].as_u64()) else { continue };
        // Names come from the robot: take only a plain file name.
        if !name.ends_with(".wpilog") || name.contains(['/', '\\']) || name.starts_with('.') {
            continue;
        }
        // Cleared here for its age or for room: the robot still has it, but it stays gone.
        if gone.iter().any(|g| g == name) {
            continue;
        }
        let path = dir.join(name);
        let have = fs::metadata(&path).map_or(0, |m| m.len());
        if have >= total {
            // Not read yet, or read by an older shell into an older layout.
            let current = fs::read(dir.join(format!("{name}.json"))).ok().and_then(|t| serde_json::from_slice::<Value>(&t).ok()).is_some_and(|v| Digest::current(&v));
            if !current {
                digest_file(dir, name);
                shared.generation.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }
        if !shared.allowed.load(Ordering::Relaxed) || shared.stop.load(Ordering::Relaxed) {
            break;
        }
        // Room for the rest of it first, oldest logs making way; a log the
        // card could never hold is not started.
        let need = total - have;
        if let Some((_, size)) = space(dir) {
            if need + keep_free(size) > size {
                continue;
            }
            clean(dir, need, Some(name));
            if space(dir).is_some_and(|(free, _)| free < need) {
                return Err("card full: logs wait for room".into());
            }
        }
        shared.pulled_bytes.store(have, Ordering::Relaxed);
        *shared.pulling.lock().unwrap() = Some((name.to_owned(), total));
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot save logs: {e}"))
            .and_then(|mut file| agent::download(host, name, have, &mut file, &shared.pulled_bytes));
        *shared.pulling.lock().unwrap() = None;
        shared.generation.fetch_add(1, Ordering::Relaxed);
        result?;
        digest_file(dir, name);
    }
    Ok(local_files(dir).len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    const TODAY: u64 = CLOCK_TRUSTED + 400 * DAY;

    fn file(name: &str, age_days: u64, mb: u64) -> (String, u64, u64) {
        (name.to_owned(), TODAY - age_days * DAY, mb * 1024 * 1024)
    }

    #[test]
    fn logs_go_after_thirty_days() {
        let files = [file("fresh", 2, 20), file("month", 29, 20), file("old", 31, 20), file("ancient", 90, 20)];
        assert_eq!(to_clear(&files, TODAY, Some(10 * KEEP_FREE), KEEP_FREE), ["ancient", "old"]);
        assert_eq!(to_clear(&files, TODAY, None, KEEP_FREE), ["ancient", "old"]);
    }

    #[test]
    fn the_oldest_go_first_when_the_card_fills() {
        let files = [file("a", 1, 100), file("b", 3, 100), file("c", 2, 100)];
        // 100 MB free: one 100 MB log must go, then 200 MB is still under the floor, so a second.
        assert_eq!(to_clear(&files, TODAY, Some(100 * 1024 * 1024), KEEP_FREE), ["b", "c"]);
    }

    #[test]
    fn a_small_data_partition_keeps_its_logs() {
        // The 64 MB partition of a card that never grew, 30 MB free: a quarter
        // of it is kept free, which 30 MB already is, so nothing goes.
        const MB: u64 = 1024 * 1024;
        let files = [file("a", 1, 10), file("b", 2, 10)];
        assert_eq!(keep_free(64 * MB), 16 * MB);
        assert!(to_clear(&files, TODAY, Some(30 * MB), keep_free(64 * MB)).is_empty());
        // Room for a 20 MB log as well: the oldest makes way.
        assert_eq!(to_clear(&files, TODAY, Some(30 * MB), keep_free(64 * MB) + 20 * MB), ["b"]);
        // A big card keeps the full 256 MB.
        assert_eq!(keep_free(29 * 1024 * MB), KEEP_FREE);
    }

    #[test]
    fn a_cleared_log_is_remembered_and_not_pulled_again() {
        let dir = std::env::temp_dir().join(format!("catalyst-cleared-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        remember_cleared(&dir, &["one.wpilog".into()]);
        remember_cleared(&dir, &["two.wpilog".into()]);
        assert_eq!(cleared(&dir), ["one.wpilog", "two.wpilog"]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nothing_is_cleared_by_age_while_the_clock_is_unset() {
        // A handheld that booted with no clock reads 1970.
        let files = [("pulled-today".to_owned(), 5_000, 1024)];
        assert!(to_clear(&files, 9_000_000, Some(10 * KEEP_FREE), KEEP_FREE).is_empty());
        // And a log stamped 1970 is not thirty days old the moment the clock is set.
        assert!(to_clear(&files, TODAY, Some(10 * KEEP_FREE), KEEP_FREE).is_empty());
    }

    #[test]
    fn a_digest_survives_its_file_and_knows_when_it_is_stale() {
        let summary = wpilog::summarize(&wpilog::sample()[..]).unwrap();
        let digest = Digest::of(&summary);
        let json: Value = serde_json::from_str(&digest.to_json(4096).to_string()).unwrap();
        assert!(Digest::current(&json));
        assert_eq!(digest.brownouts, Some(1));
        assert_eq!(digest.hottest.as_ref().map(|(name, _)| name.as_str()), Some("Elevator"));
        assert!(digest.can_peak.is_some());
        assert_eq!(Digest::from_json(&json, 4096), Some(digest));
        assert_eq!(Digest::from_json(&json, 4097), None);
        // One an older shell wrote still reads, with what it lacks absent, and is made again.
        let old = json!({"bytes": 10, "seconds": 150.0, "batteryLow": 11.2, "highlights": []});
        let read = Digest::from_json(&old, 10).unwrap();
        assert_eq!((read.battery_low, read.brownouts, read.can_peak), (Some(11.2), None, None));
        assert!(!Digest::current(&old));
    }

    /// Against a real catalyst-agent serving a folder that holds the sample log:
    /// `CATALYST_AGENT_HOST=… CATALYST_SAMPLE_DIR=<that folder> cargo test live_pull -- --ignored`.
    /// With only CATALYST_SAMPLE_DIR set it writes the sample log there and stops.
    #[test]
    #[ignore = "needs a running catalyst-agent with the log routes"]
    fn live_pull() {
        let robot_dir = PathBuf::from(std::env::var("CATALYST_SAMPLE_DIR").expect("set CATALYST_SAMPLE_DIR"));
        let name = "FRC_20261116_101500_NYRO_Q14.wpilog";
        let sample = wpilog::sample();
        fs::create_dir_all(&robot_dir).unwrap();
        fs::write(robot_dir.join(name), &sample).unwrap();
        let Ok(host) = std::env::var("CATALYST_AGENT_HOST") else { return };

        let here = std::env::temp_dir().join(format!("catalyst-live-pull-{}", std::process::id()));
        fs::create_dir_all(&here).unwrap();
        let shared = Shared::default();
        shared.allowed.store(true, Ordering::Relaxed);

        // A whole pull, byte for byte, with its digest beside it.
        assert_eq!(pull_once(&shared, &here, &host).unwrap(), 1);
        assert_eq!(fs::read(here.join(name)).unwrap(), sample);
        let digest: Value = serde_json::from_slice(&fs::read(here.join(format!("{name}.json"))).unwrap()).unwrap();
        assert_eq!(digest["highlights"][0][1], "1 brownout");

        // Cut the local copy in half, as a pulled cable would: the next pull resumes.
        let half = &sample[..sample.len() / 2];
        fs::write(here.join(name), half).unwrap();
        shared.pulled_bytes.store(0, Ordering::Relaxed);
        pull_once(&shared, &here, &host).unwrap();
        assert_eq!(fs::read(here.join(name)).unwrap(), sample);
        // Only the missing half crossed the wire.
        assert_eq!(shared.pulled_bytes.load(Ordering::Relaxed), sample.len() as u64);
        println!("pulled {} bytes, resumed from {}", sample.len(), half.len());
        fs::remove_dir_all(&here).ok();
    }

    #[test]
    fn names_read_as_dates_and_matches() {
        let title = |name: &str| Row { name: name.into(), bytes: 0, digest: None }.title();
        assert_eq!(title("FRC_20261116_101500.wpilog"), "nov 16 · 10:15");
        assert_eq!(title("FRC_20260305_153012_NYRO_Q12.wpilog"), "nyro q12 · mar 5 · 15:30");
        assert_eq!(title("practice.wpilog"), "practice");
    }
}
