//! The on-device model as the shell runs it: the assistant's "On this
//! handheld" (it calls `start` and posts OpenAI-style chats to the URL it
//! gets back), and Settings › On-device model, where one is chosen,
//! downloaded, tried and removed.
//!
//! Nothing runs until something asks: `start` brings up llama-server (from
//! the image) on the chosen model, and it is stopped again after
//! `IDLE` without being asked anything (`touch` marks a question). What the
//! models are, where they live and how the server is run is in the
//! `localmodel` crate, which `ai` in the Terminal shares.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use localmodel::{self as lm, Model, MODELS};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Unasked for this long, the model is stopped and its memory given back.
const IDLE: Duration = Duration::from_secs(180);
/// Loading a model from the card takes a while on the handheld's processor.
const START_WITHIN: Duration = Duration::from_secs(120);
const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

/// The server this shell started, and when it was last asked something.
struct Running {
    child: Child,
    last: Instant,
}

static SERVER: Mutex<Option<Running>> = Mutex::new(None);
static REAPING: AtomicBool = AtomicBool::new(false);

fn server() -> std::sync::MutexGuard<'static, Option<Running>> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

// --------------------------------------------------------------- interface ---

/// Whether the chosen model is downloaded and checked. (The assistant's
/// "On this handheld" asks; until that lands, nothing here does.)
#[allow(dead_code)]
pub fn ready() -> bool {
    let dir = lm::dir();
    lm::present(&dir, lm::chosen(&dir))
}

/// The chosen model's name, "SmolLM2 135M".
#[allow(dead_code)]
pub fn model_name() -> &'static str {
    lm::chosen(&lm::dir()).name
}

/// Bring the model up if it is not (waiting while it loads) and the base URL
/// to post chats to, "http://127.0.0.1:8088/v1". Call it from a thread of
/// its own: loading takes seconds.
pub fn start() -> Result<String, String> {
    let dir = lm::dir();
    let model = lm::chosen(&dir);
    // Before anything that answers on the port is taken for it.
    if !lm::present(&dir, model) {
        return Err(format!("{} is not downloaded: Settings › On-device model", model.name));
    }
    let mut running = server();
    if let Some(r) = running.as_mut() {
        if matches!(r.child.try_wait(), Ok(None)) && lm::healthy() {
            r.last = Instant::now();
            return Ok(lm::base_url());
        }
        if let Some(mut gone) = running.take() {
            lm::stop_server(&mut gone.child);
        }
    }
    // One started by `ai` in the Terminal: asked too, and left to it to stop.
    if lm::healthy() {
        return Ok(lm::base_url());
    }
    let mut child = lm::spawn_server(&dir, model)?;
    lm::wait_ready(&mut child, START_WITHIN)?;
    *running = Some(Running { child, last: Instant::now() });
    drop(running);
    reap_when_idle();
    Ok(lm::base_url())
}

/// Stop the model now, if this shell started it.
pub fn stop() {
    if let Some(mut r) = server().take() {
        lm::stop_server(&mut r.child);
    }
}

/// The model was just asked something: its idle time starts again.
pub fn touch() {
    if let Some(r) = server().as_mut() {
        r.last = Instant::now();
    }
}

/// Whether a model unasked since `last` has waited long enough to stop.
fn idle_too_long(last: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last) >= IDLE
}

/// A thread that stops the server once it has been idle `IDLE` (one at a
/// time; it ends with the server).
fn reap_when_idle() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("local-model".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let mut running = server();
                let done = match running.as_mut() {
                    None => true,
                    Some(r) => !matches!(r.child.try_wait(), Ok(None)) || idle_too_long(r.last, Instant::now()),
                };
                if done {
                    if let Some(mut r) = running.take() {
                        lm::stop_server(&mut r.child);
                    }
                    break;
                }
            }
            REAPING.store(false, Ordering::SeqCst);
        })
        .ok();
}

/// Settings' line for it: which model and whether it is here.
pub fn summary() -> String {
    let dir = lm::dir();
    let model = lm::chosen(&dir);
    if lm::present(&dir, model) {
        format!("{} · ready", short(model))
    } else {
        "none yet".into()
    }
}

/// "135M", from "SmolLM2 135M".
fn short(model: &Model) -> &'static str {
    model.name.rsplit(' ').next().unwrap_or(model.name)
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", (bytes + 500_000) / 1_000_000)
}

// ---------------------------------------------------------------- download ---

/// A download's progress, shared with the page that shows it.
#[derive(Default)]
struct Progress {
    bytes: AtomicU64,
    checking: AtomicBool,
    cancel: AtomicBool,
}

/// Room left on the filesystem `dir` is on, in bytes.
fn free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: a valid path and a struct to fill.
        if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
            return None;
        }
        Some(s.f_bavail as u64 * s.f_frsize as u64)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        None
    }
}

fn len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// Fetch `url` into `part` (carrying on from what it already holds) until it
/// is `size` bytes, picking up after a dropped connection for as long as each
/// try gets further; then its SHA-256.
fn fetch(url: &str, size: u64, part: &Path, progress: &Progress) -> Result<String, String> {
    if len(part) > size {
        std::fs::remove_file(part).ok();
    }
    let mut stalls = 0;
    while len(part) < size {
        let before = len(part);
        let mut command = Command::new("curl");
        command.args(["-fsSL", "--connect-timeout", "20", "-C", "-", "-o"]).arg(part).arg(url);
        if Path::new(CA_BUNDLE).exists() {
            command.args(["--cacert", CA_BUNDLE]);
        }
        let mut child = command.spawn().map_err(|e| format!("could not run curl: {e}"))?;
        let status = loop {
            progress.bytes.store(len(part), Ordering::Relaxed);
            if progress.cancel.load(Ordering::Relaxed) {
                child.kill().ok();
                child.wait().ok();
                return Err("stopped".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(250)),
                Err(e) => return Err(format!("the download: {e}")),
            }
        };
        progress.bytes.store(len(part), Ordering::Relaxed);
        if len(part) > size {
            std::fs::remove_file(part).ok();
        }
        stalls = if len(part) > before { 0 } else { stalls + 1 };
        if stalls >= 6 {
            return Err(format!("the download stopped ({status})"));
        }
        if len(part) < size {
            std::thread::sleep(Duration::from_secs(1 + 2 * stalls));
        }
    }
    progress.checking.store(true, Ordering::Relaxed);
    sha256_of(part)
}

fn sha256_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| format!("checking the model: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("checking the model: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download a model into `dir`, check it, and keep it.
fn download(model: &'static Model, dir: &Path, progress: &Progress) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    if lm::present(dir, model) {
        return Ok(());
    }
    let part = lm::partial(dir, model);
    if let Some(free) = free_bytes(dir) {
        let need = model.size.saturating_sub(len(&part)) + 16_000_000;
        if free < need {
            return Err(format!("{} more room is needed on the card", megabytes(need - free)));
        }
    }
    let sum = fetch(model.url, model.size, &part, progress)?;
    lm::accept(dir, model, &sum)
}

// -------------------------------------------------------------------- page ---

/// Settings › On-device model.
pub struct ModelPage {
    dir: PathBuf,
    list: ListState,
    /// A download under way: its progress and how it ended.
    downloading: Option<(&'static Model, Arc<Progress>, Receiver<Result<(), String>>)>,
    /// A question to try it with, under way: the answer and its speed.
    trying: Option<Receiver<Result<(String, Option<f64>), String>>>,
    /// The last answer to show, or what went wrong.
    answer: Option<String>,
    /// Enter once arms removing (or stopping a download); again does it.
    armed: bool,
    clock: f32,
}

const ROWS: usize = 3;
const TRY: &str = "In one short sentence: what is a brownout on a robot?";

impl ModelPage {
    pub fn new() -> Self {
        ModelPage { dir: lm::dir(), list: ListState::default(), downloading: None, trying: None, answer: None, armed: false, clock: 0.0 }
    }

    fn model(&self) -> &'static Model {
        lm::chosen(&self.dir)
    }

    fn present(&self) -> bool {
        lm::present(&self.dir, self.model())
    }

    /// The other model, chosen (not while one is coming down).
    fn turn(&mut self, by: isize, cx: &mut Cx) {
        if self.downloading.is_some() {
            *cx.say = Some("one download at a time".into());
            return;
        }
        let at = MODELS.iter().position(|m| m.id == self.model().id).unwrap_or(0) as isize;
        let next = &MODELS[(at + by).rem_euclid(MODELS.len() as isize) as usize];
        lm::set_chosen(&self.dir, next).ok();
        // Another model: the one running stops, the next question starts this one.
        stop();
        self.answer = None;
    }

    fn start_download(&mut self) {
        let model = self.model();
        let progress = Arc::new(Progress::default());
        progress.bytes.store(len(&lm::partial(&self.dir, model)), Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let (dir, shared) = (self.dir.clone(), progress.clone());
        std::thread::Builder::new()
            .name("model-download".into())
            .spawn(move || {
                tx.send(download(model, &dir, &shared)).ok();
            })
            .ok();
        self.downloading = Some((model, progress, rx));
        self.answer = None;
    }

    fn start_trying(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("model-try".into())
            .spawn(move || {
                let asked = start().and_then(|_| {
                    let body = lm::chat_body("You are a small assistant on a handheld. Answer in one short sentence.", &[("user", TRY)], 80);
                    let answer = lm::ask(&body, Duration::from_secs(120));
                    touch();
                    answer
                });
                tx.send(asked).ok();
            })
            .ok();
        self.trying = Some(rx);
        self.answer = None;
    }

    /// What the action row says and does: download, the download under way,
    /// or remove.
    fn action(&self) -> (String, String) {
        let model = self.model();
        match &self.downloading {
            Some((m, p, _)) if p.checking.load(Ordering::Relaxed) => ("Checking".into(), format!("{} · its sum", short(m))),
            Some((m, p, _)) => {
                let got = p.bytes.load(Ordering::Relaxed);
                ("Downloading".into(), format!("{}% · {} of {}", got * 100 / m.size.max(1), megabytes(got), megabytes(m.size)))
            }
            None if self.present() => ("Remove".into(), format!("{} on the card", megabytes(model.size))),
            None => ("Download".into(), megabytes(model.size)),
        }
    }

    /// In the gallery: a download partway, as it would show.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_download(fraction: f64) -> Self {
        let mut page = Self::new();
        let model = page.model();
        let progress = Arc::new(Progress::default());
        progress.bytes.store((model.size as f64 * fraction) as u64, Ordering::Relaxed);
        let (_tx, rx) = mpsc::channel();
        std::mem::forget(_tx);
        page.downloading = Some((model, progress, rx));
        page.list.focus = 1;
        page
    }

    /// In the gallery: an answer from trying it.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_answer(answer: &str) -> Self {
        let mut page = Self::new();
        page.answer = Some(answer.to_owned());
        page.list.focus = 2;
        page
    }
}

impl Screen for ModelPage {
    fn title(&self) -> &str {
        // "On-device model" runs into the island.
        "On device"
    }

    fn busy(&self) -> bool {
        self.downloading.is_some() || self.trying.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.busy()
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.busy()
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some((model, _, rx)) = &self.downloading {
            match rx.try_recv() {
                Ok(result) => {
                    *cx.say = Some(match result {
                        Ok(()) => format!("{} is ready", model.name),
                        Err(why) => why,
                    });
                    self.downloading = None;
                }
                Err(TryRecvError::Disconnected) => self.downloading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.trying {
            match rx.try_recv() {
                Ok(Ok((answer, speed))) => {
                    if let Some(s) = speed {
                        *cx.say = Some(format!("answered at {s:.1} tokens a second"));
                    }
                    self.answer = Some(answer);
                    self.trying = None;
                }
                Ok(Err(why)) => {
                    *cx.say = Some(why.clone());
                    self.answer = Some(why);
                    self.trying = None;
                }
                Err(TryRecvError::Disconnected) => self.trying = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = std::mem::take(&mut self.armed);
        match (key, self.list.focus) {
            (Key::Back | Key::Leave, _) => return Nav::Pop,
            (Key::Left, 0) => self.turn(-1, cx),
            (Key::Right | Key::Enter, 0) => self.turn(1, cx),
            (Key::Enter | Key::Right, 1) => match &self.downloading {
                Some((_, progress, _)) if armed => {
                    progress.cancel.store(true, Ordering::Relaxed);
                    *cx.say = Some("download stopped · it carries on from here next time".into());
                }
                Some(_) => {
                    self.armed = true;
                    *cx.say = Some("enter again stops the download".into());
                }
                None if self.present() && armed => {
                    stop();
                    lm::remove(&self.dir, self.model());
                    self.answer = None;
                    *cx.say = Some(format!("{} removed", self.model().name));
                }
                None if self.present() => {
                    self.armed = true;
                    *cx.say = Some(format!("enter again removes {} from the card", self.model().name));
                }
                None => self.start_download(),
            },
            (Key::Enter | Key::Right, 2) if self.present() && self.trying.is_none() => self.start_trying(),
            (Key::Enter | Key::Right, 2) if self.trying.is_some() => *cx.say = Some("still answering".into()),
            (Key::Enter | Key::Right, 2) => *cx.say = Some("download it first".into()),
            (other, _) => crate::screens::list_key(&mut self.list, other, ROWS),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let model = self.model();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let cap_lh = ty.caption.size + 4.0;

        // What it is (two lines, so all three rows fit), or the answer it
        // just gave (three).
        let (words, most) = match (&self.answer, &self.trying) {
            (_, Some(_)) => {
                let dots = ".".repeat(1 + (self.clock * 2.0) as usize % 3);
                (format!("{} is thinking{dots}", model.name), 2)
            }
            (Some(answer), None) => (format!("“{answer}”"), 3),
            (None, None) => ("Answers offline, on this handheld. Short ones; often wrong on facts; blind to the robot.".into(), 2),
        };
        let mut lines = crate::help::wrap(g, &words, ty.caption, W - 2.0 * PAD);
        if lines.len() > most {
            lines.truncate(most);
            if let Some(last) = lines.last_mut() {
                *last = g.text.fit(&format!("{last}…"), &ty.caption, W - 2.0 * PAD);
            }
        }
        let color = if self.answer.is_some() && self.trying.is_none() { t.ink } else { t.dim };
        for (k, line) in lines.iter().enumerate() {
            g.text_at(PAD + 2.0, top + 1.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, color);
        }
        let rows_top = top + 4.0 + lines.len().max(2) as f32 * cap_lh;

        let (action, action_value) = self.action();
        for (i, y) in self.list.layout(g, ROWS, rows_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match i {
                0 => g.row(r, "Model", model.name, t.ink, focused),
                1 => {
                    g.row(r, &action, &action_value, t.dim, focused);
                    if let Some((m, p, _)) = &self.downloading {
                        let fraction = p.bytes.load(Ordering::Relaxed) as f32 / m.size.max(1) as f32;
                        g.meter(Rect::new(r.x + 12.0, r.bottom() - 3.5, r.w - 24.0, 2.0), fraction, t.signal);
                    }
                }
                _ => {
                    let value = if self.trying.is_some() {
                        "answering"
                    } else if self.present() {
                        "one question"
                    } else {
                        "after downloading"
                    };
                    g.row(r, "Try it", value, if self.present() { t.dim } else { t.faint }, focused);
                }
            }
        }
        self.list.scrollbar(g, ROWS, rows_top, bottom);
        let hint = match self.list.focus {
            0 => format!("{} · {} MB in use", model.about, model.peak_mb),
            1 if self.downloading.is_some() => "enter twice: stop · it carries on later".into(),
            1 if self.present() => "enter twice: remove it".into(),
            1 => "enter: download from Hugging Face".into(),
            _ => "enter: one question · stops 3 min after".into(),
        };
        g.hint(&hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unasked_model_stops_after_a_while_and_not_before() {
        let start = Instant::now();
        assert!(!idle_too_long(start, start + Duration::from_secs(60)));
        assert!(!idle_too_long(start, start + IDLE - Duration::from_secs(1)));
        assert!(idle_too_long(start, start + IDLE));
        assert!(!idle_too_long(start + Duration::from_secs(10), start), "a clock that went back is not idle");
    }

    #[test]
    fn a_download_carries_on_and_its_sum_is_what_decides() {
        if !Command::new("curl").arg("--version").output().is_ok_and(|o| o.status.success()) {
            eprintln!("no curl here: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("catalyst-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let whole: Vec<u8> = (0..400_000u32).map(|i| (i * 7 % 253) as u8).collect();
        let source = dir.join("model.gguf");
        std::fs::write(&source, &whole).unwrap();
        let url = format!("file:///{}", source.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
        // Part of it from before, as a dropped download leaves it.
        let part = dir.join("got.part");
        std::fs::write(&part, &whole[..150_000]).unwrap();
        let progress = Progress::default();
        let sum = fetch(&url, whole.len() as u64, &part, &progress).unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), whole, "picked up where it stopped");
        assert_eq!(sum, sha256_of(&source).unwrap());
        assert_eq!(progress.bytes.load(Ordering::Relaxed), whole.len() as u64);
        assert!(progress.checking.load(Ordering::Relaxed));
        // Stopped: it says so and keeps what it has.
        std::fs::write(&part, &whole[..1000]).unwrap();
        let stopping = Progress::default();
        stopping.cancel.store(true, Ordering::Relaxed);
        assert_eq!(fetch(&url, whole.len() as u64, &part, &stopping).unwrap_err(), "stopped");
        assert!(part.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_known_sum_of_nothing_checks_out() {
        let dir = std::env::temp_dir().join(format!("catalyst-local-sum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("empty"), b"").unwrap();
        assert_eq!(sha256_of(&dir.join("empty")).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn settings_says_which_model_and_whether_it_is_here() {
        assert_eq!(short(&MODELS[0]), "135M");
        assert_eq!(megabytes(144_811_360), "145 MB");
        crate::shell::tests::sandbox();
        let dir = std::env::temp_dir().join(format!("catalyst-local-summary-{}", std::process::id()));
        std::env::set_var("CATALYST_MODELS", &dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(summary(), "none yet");
        assert!(!ready());
        assert_eq!(model_name(), "SmolLM2 135M");
        assert!(start().unwrap_err().contains("not downloaded"), "nothing to start");
        std::fs::remove_dir_all(&dir).ok();
    }
}
