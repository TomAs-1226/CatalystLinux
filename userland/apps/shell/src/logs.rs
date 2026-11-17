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
/// Space kept free on the data partition, whatever the age of the logs.
const KEEP_FREE: u64 = 256 * 1024 * 1024;

/// What the screens show for one log.
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    pub seconds: f64,
    pub enabled_seconds: Option<f64>,
    pub battery_low: Option<f64>,
    pub worst_loop_ms: Option<f64>,
    pub highlights: Vec<(Level, String)>,
}

impl Digest {
    pub fn of(summary: &Summary) -> Self {
        Digest {
            seconds: summary.seconds,
            enabled_seconds: summary.enabled_seconds,
            battery_low: summary.battery_low,
            worst_loop_ms: summary.worst_loop_ms,
            highlights: summary.highlights(),
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
            "bytes": bytes, "seconds": self.seconds, "enabledSeconds": self.enabled_seconds,
            "batteryLow": self.battery_low, "worstLoopMs": self.worst_loop_ms,
            "highlights": self.highlights.iter().map(|(l, text)| json!([level(l), text])).collect::<Vec<_>>(),
        })
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

/// Which logs to clear: those past `KEEP_DAYS`, then the oldest until
/// `KEEP_FREE` is free. `files` is (name, modified, bytes); `free` is the
/// space left on the partition, when known.
pub fn to_clear(files: &[(String, u64, u64)], now: u64, free: Option<u64>) -> Vec<String> {
    let mut oldest_first: Vec<&(String, u64, u64)> = files.iter().collect();
    oldest_first.sort_by_key(|f| f.1);
    let mut clear = Vec::new();
    let mut free = free;
    for (name, modified, bytes) in oldest_first {
        // Ages only mean something once the clock has been set.
        let expired = now >= CLOCK_TRUSTED && *modified >= CLOCK_TRUSTED && now.saturating_sub(*modified) > KEEP_DAYS * 86_400;
        let cramped = free.is_some_and(|f| f < KEEP_FREE);
        if expired || cramped {
            clear.push(name.clone());
            free = free.map(|f| f + bytes);
        }
    }
    clear
}

#[cfg(target_os = "linux")]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(not(target_os = "linux"))]
fn free_space(_dir: &Path) -> Option<u64> {
    None
}

#[derive(Default)]
struct Shared {
    allowed: AtomicBool,
    stop: AtomicBool,
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
}

impl Logs {
    /// No robot: nothing to show or pull.
    pub fn none() -> Self {
        Logs { dir: None, shared: Arc::new(Shared::default()), demo: Vec::new(), listed: Mutex::new(None) }
    }

    /// Two scripted matches, for the demo robot.
    pub fn demo() -> Self {
        let sample = wpilog::sample();
        let digest = wpilog::summarize(&sample[..]).ok().map(|s| Digest::of(&s));
        let calm = Digest {
            seconds: 151.2,
            enabled_seconds: Some(135.0),
            battery_low: Some(11.4),
            worst_loop_ms: Some(9.8),
            highlights: vec![(Level::Fine, "battery low point 11.4 v".into()), (Level::Fine, "worst loop 9.8 ms".into()), (Level::Fine, "Elevator peak 58 °c · 41 a".into())],
        };
        let demo = vec![
            Row { name: "FRC_20261116_101500_NYRO_Q14.wpilog".into(), bytes: sample.len() as u64, digest },
            Row { name: "FRC_20261116_093200_NYRO_Q9.wpilog".into(), bytes: 18_400_000, digest: Some(calm) },
            Row { name: "FRC_20261115_171800.wpilog".into(), bytes: 31_250_000, digest: None },
        ];
        let logs = Logs { dir: None, shared: Arc::new(Shared::default()), demo, listed: Mutex::new(None) };
        *logs.shared.status.lock().unwrap() = "demo logs".into();
        logs
    }

    /// Keep the logs of the robot at `host`, pulling new ones in the background.
    pub fn for_robot(host: &str) -> Self {
        let dir = std::env::var_os("CATALYST_LOGS").map(PathBuf::from).unwrap_or_else(|| "/data/logs".into());
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "waiting for the robot".into();
        if fs::create_dir_all(&dir).is_err() {
            *shared.status.lock().unwrap() = "no place to keep logs".into();
            return Logs { dir: None, shared, demo: Vec::new(), listed: Mutex::new(None) };
        }
        let (thread_shared, thread_dir, host) = (shared.clone(), dir.clone(), host.to_owned());
        thread::Builder::new()
            .name("logs".into())
            .stack_size(512 * 1024)
            .spawn(move || pull_loop(&thread_shared, &thread_dir, &host))
            .expect("spawn logs thread");
        Logs { dir: Some(dir), shared, demo: Vec::new(), listed: Mutex::new(None) }
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
        if let Some((name, total)) = self.shared.pulling.lock().unwrap().clone() {
            let done = self.shared.pulled_bytes.load(Ordering::Relaxed);
            let row = Row { name, bytes: 0, digest: None };
            return format!("pulling {} · {}%", row.title(), (done * 100).checked_div(total).unwrap_or(0).min(100));
        }
        self.shared.status.lock().unwrap().clone()
    }

    /// The logs kept here, newest first.
    pub fn list(&self) -> Vec<Row> {
        let Some(dir) = &self.dir else { return self.demo.clone() };
        let generation = self.shared.generation.load(Ordering::Relaxed);
        if let Some((at, rows)) = &*self.listed.lock().unwrap() {
            if *at == generation {
                return rows.clone();
            }
        }
        let mut rows: Vec<Row> = local_files(dir)
            .into_iter()
            .map(|(name, _, bytes)| {
                let digest = fs::read(dir.join(format!("{name}.json")))
                    .ok()
                    .and_then(|text| serde_json::from_slice::<Value>(&text).ok())
                    .and_then(|v| Digest::from_json(&v, bytes));
                Row { name, bytes, digest }
            })
            .collect();
        // WPILib names sort by time.
        rows.sort_by(|a, b| b.name.cmp(&a.name));
        *self.listed.lock().unwrap() = Some((generation, rows.clone()));
        rows
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
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

/// Clear what is due. Returns how many logs went.
pub fn clean(dir: &Path) -> usize {
    let files = local_files(dir);
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
    let clear = to_clear(&files, time, free_space(dir));
    for name in &clear {
        fs::remove_file(dir.join(name)).ok();
        fs::remove_file(dir.join(format!("{name}.json"))).ok();
    }
    clear.len()
}

/// Write the digest of a log beside it.
fn digest_file(dir: &Path, name: &str) {
    let path = dir.join(name);
    let Ok(bytes) = fs::metadata(&path).map(|m| m.len()) else { return };
    let Ok(file) = File::open(&path) else { return };
    if let Ok(summary) = wpilog::summarize(std::io::BufReader::with_capacity(64 * 1024, file)) {
        let json = Digest::of(&summary).to_json(bytes).to_string();
        fs::write(dir.join(format!("{name}.json")), json).ok();
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
            if clean(dir) > 0 {
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
        for _ in 0..75 {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Bring every log the robot has up to its full length here, newest first.
fn pull_once(shared: &Shared, dir: &Path, host: &str) -> Result<usize, String> {
    let listing = agent::get(host, "/api/logs").map_err(|why| if why.contains("404") { "robot's agent is too old for logs".to_string() } else { why })?;
    let empty = Vec::new();
    for entry in listing["logs"].as_array().unwrap_or(&empty) {
        let (Some(name), Some(total)) = (entry["name"].as_str(), entry["bytes"].as_u64()) else { continue };
        // Names come from the robot: take only a plain file name.
        if !name.ends_with(".wpilog") || name.contains(['/', '\\']) || name.starts_with('.') {
            continue;
        }
        let path = dir.join(name);
        let have = fs::metadata(&path).map_or(0, |m| m.len());
        if have >= total {
            if !dir.join(format!("{name}.json")).exists() {
                digest_file(dir, name);
                shared.generation.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }
        if !shared.allowed.load(Ordering::Relaxed) || shared.stop.load(Ordering::Relaxed) {
            break;
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
        // Make room as the card fills rather than once an hour.
        clean(dir);
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
        assert_eq!(to_clear(&files, TODAY, Some(10 * KEEP_FREE)), ["ancient", "old"]);
        assert_eq!(to_clear(&files, TODAY, None), ["ancient", "old"]);
    }

    #[test]
    fn the_oldest_go_first_when_the_card_fills() {
        let files = [file("a", 1, 100), file("b", 3, 100), file("c", 2, 100)];
        // 100 MB free: one 100 MB log must go, then 200 MB is still under the floor, so a second.
        assert_eq!(to_clear(&files, TODAY, Some(100 * 1024 * 1024)), ["b", "c"]);
    }

    #[test]
    fn nothing_is_cleared_by_age_while_the_clock_is_unset() {
        // A handheld that booted with no clock reads 1970.
        let files = [("pulled-today".to_owned(), 5_000, 1024)];
        assert!(to_clear(&files, 9_000_000, Some(10 * KEEP_FREE)).is_empty());
        // And a log stamped 1970 is not thirty days old the moment the clock is set.
        assert!(to_clear(&files, TODAY, Some(10 * KEEP_FREE)).is_empty());
    }

    #[test]
    fn a_digest_survives_its_file_and_knows_when_it_is_stale() {
        let summary = wpilog::summarize(&wpilog::sample()[..]).unwrap();
        let digest = Digest::of(&summary);
        let json: Value = serde_json::from_str(&digest.to_json(4096).to_string()).unwrap();
        assert_eq!(Digest::from_json(&json, 4096), Some(digest));
        assert_eq!(Digest::from_json(&json, 4097), None);
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
