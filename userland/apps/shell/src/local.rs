//! The on-device model as the shell runs it: the assistant's "On this
//! handheld" (it calls `start` and posts OpenAI-style chats to the URL it
//! gets back), and Settings › On-device model, which lists the models
//! offered (Fastest, Balanced, Smartest), each downloaded, chosen, tried and
//! removed there, and shows whether one is loaded and the memory it holds.
//!
//! The rules it keeps, so that the model never clutters the handheld:
//! - Nothing runs until a question needs it: `start` brings up llama-server
//!   (from the image) on the chosen model, never at boot and never because a
//!   page is open.
//! - Two minutes (`IDLE`) after its last question or answer it is stopped,
//!   and all its memory comes back.
//! - When nobody is looking (the screen off, StandBy's faces, locked:
//!   `away`), it is stopped at once, or straight after the answer it is
//!   writing.
//! - An answer given up on (Esc, leaving the Assistant: `cancel`) stops
//!   being written at once: the request is cut, which llama-server takes as
//!   the end of it.
//! - It runs at a lower priority than the shell, on three of the four cores,
//!   and loaded but not answering it uses no processor time (measured).
//! - It never needs swap: only models that fit in RAM with 100 MB to spare
//!   are offered, one is started only with that much free now (else it says
//!   what to close), a waiting one gives way when memory runs short, and the
//!   kernel kills it first should memory run out.
//!
//! What the models are, where they live and how the server is run is in the
//! `localmodel` crate, which `ai` in the Terminal shares.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use localmodel::{self as lm, Model, MODELS};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Unasked for this long, the model is stopped and its memory given back.
/// Two minutes: a follow-up within them is answered by the model already
/// loaded, and starting it again after costs about ten seconds (measured on
/// the handheld: 9.6 s for LFM2.5 230M to load and say it is ready, 13 s
/// for the 350M), little beside the 20 to 35 s a question with a log in it
/// takes to read.
const IDLE: Duration = Duration::from_secs(120);
/// A question just started on counts as work this long, so the model is not
/// stopped between being started and being asked.
const JUST_ASKED: Duration = Duration::from_secs(5);
/// Processor time in a second (clock ticks) that means it is working: a
/// fifth of a core. Answering, it uses three; loaded and waiting, none.
const WORKING_TICKS: u64 = 20;
/// Loading a model from the card takes a while on the handheld's processor.
const START_WITHIN: Duration = Duration::from_secs(120);
const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

/// The server this shell started, when it last asked or answered, and the
/// processor time it had used when last looked at.
struct Running {
    child: Child,
    last: Instant,
    cpu: u64,
}

/// Held while a model loads (seconds): never taken on the UI's thread.
static SERVER: Mutex<Option<Running>> = Mutex::new(None);
static REAPING: AtomicBool = AtomicBool::new(false);
/// The server's process id while this shell has one running (0: none), for
/// the page to look at without waiting on `SERVER`.
static PID: AtomicU32 = AtomicU32::new(0);
/// The Assistant's request under way (its curl's process id; 0: none).
static CLIENT: AtomicU32 = AtomicU32::new(0);
/// Questions from Settings › On-device model under way.
static TRYING: AtomicU32 = AtomicU32::new(0);
/// Nobody is looking: stop once nothing is being written.
static AWAY: AtomicBool = AtomicBool::new(false);

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

/// The chosen model's name, "LFM2.5 230M".
#[allow(dead_code)]
pub fn model_name() -> &'static str {
    lm::chosen(&lm::dir()).name
}

/// The chosen model, for what else to know of it (whether llama-server
/// reads its tool calls, its context, its measured pace).
#[allow(dead_code)]
pub fn model() -> &'static Model {
    lm::chosen(&lm::dir())
}

/// Bring the model up if it is not (waiting while it loads) and the base URL
/// to post chats to, "http://127.0.0.1:8088/v1". Call it from a thread of
/// its own, only when a question needs it: loading takes seconds.
pub fn start() -> Result<String, String> {
    let dir = lm::dir();
    let model = lm::chosen(&dir);
    // Before anything that answers on the port is taken for it.
    if !lm::present(&dir, model) {
        return Err(format!("{} is not downloaded: Settings › On-device model", model.name));
    }
    // Someone is asking: someone is looking.
    AWAY.store(false, Ordering::SeqCst);
    let mut running = server();
    if let Some(r) = running.as_mut() {
        // Held still while the processor cools, it cannot say it is well:
        // it is, and carries on once let go.
        if matches!(r.child.try_wait(), Ok(None)) && (lm::cooling() || lm::healthy()) {
            r.last = Instant::now();
            return Ok(lm::base_url());
        }
        if let Some(mut gone) = running.take() {
            PID.store(0, Ordering::SeqCst);
            lm::stop_server(&mut gone.child);
        }
    }
    // One started by `ai` in the Terminal: asked too, and left to it to stop.
    if lm::healthy() {
        return Ok(lm::base_url());
    }
    // A live activity while it loads (seconds, from the card).
    let _loading = crate::liveactivity::task(crate::liveactivity::Kind::Assistant, format!("Loading {}", model.name), "on-device model");
    let mut child = lm::spawn_server(&dir, model)?;
    lm::wait_ready(&mut child, START_WITHIN)?;
    let (pid, cpu) = (child.id(), cpu_time(child.id()));
    *running = Some(Running { child, last: Instant::now(), cpu });
    PID.store(pid, Ordering::SeqCst);
    drop(running);
    reap_when_done();
    Ok(lm::base_url())
}

/// Stop the model now, if this shell started it: its memory back.
pub fn stop() {
    if let Some(mut r) = server().take() {
        PID.store(0, Ordering::SeqCst);
        lm::stop_server(&mut r.child);
    }
}

/// Settings' "unload now": the answer under way given up on and the model
/// stopped, without the page waiting for it.
pub fn unload_now() {
    cancel();
    std::thread::Builder::new().name("local-model-stop".into()).stack_size(64 * 1024).spawn(stop).ok();
}

/// The model was just asked something: its idle time starts again.
pub fn touch() {
    if let Some(r) = server().as_mut() {
        r.last = Instant::now();
    }
}

/// The Assistant's request to the model has begun (`Some`: its curl's
/// process id, so it can be cut short) or is over (`None`).
pub fn answering(client: Option<u32>) {
    CLIENT.store(client.unwrap_or(0), Ordering::SeqCst);
    touch();
}

/// Stop the answer being written at once (Esc, or leaving the Assistant),
/// rather than letting it finish where nobody reads it: the request is cut,
/// and llama-server stops writing when its asker goes.
pub fn cancel() {
    let client = CLIENT.swap(0, Ordering::SeqCst);
    #[cfg(target_os = "linux")]
    if client != 0 {
        // SAFETY: a signal to the curl this shell started for the request.
        unsafe {
            libc::kill(client as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = client;
}

/// Nobody is looking (the screen off, StandBy's faces, locked): the model
/// is stopped within a second, or straight after the answer it is writing.
/// Cheap, for the shell to call every frame while it is so.
pub fn away() {
    if PID.load(Ordering::Relaxed) != 0 {
        AWAY.store(true, Ordering::Relaxed);
    }
}

/// The model this shell has loaded now: the memory it holds (MB), if one is.
pub fn loaded() -> Option<u64> {
    match PID.load(Ordering::Relaxed) {
        0 => None,
        pid => std::fs::read_to_string(format!("/proc/{pid}/status")).ok().and_then(|s| resident_mb(&s)).or(Some(0)),
    }
}

/// Tests: a model loaded (a process standing in for it), or none.
#[cfg(test)]
pub(crate) fn pretend_loaded(pid: Option<u32>) {
    PID.store(pid.unwrap_or(0), Ordering::SeqCst);
    if pid.is_none() {
        AWAY.store(false, Ordering::SeqCst);
    }
}

/// Tests: whether the model is to be let go once it is done.
#[cfg(test)]
pub(crate) fn let_go() -> bool {
    AWAY.load(Ordering::SeqCst)
}

/// The resident memory (MB) in a /proc/<pid>/status: the model's mapped
/// weights it has read, its cache and its working space.
fn resident_mb(status: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| (kb + 512) / 1024)
}

/// Whether a model last working at `last` is to stop now: idle `IDLE`, or
/// done with its answer while nobody is looking.
fn time_to_stop(last: Instant, now: Instant, away: bool) -> bool {
    let since = now.saturating_duration_since(last);
    since >= IDLE || (away && since >= JUST_ASKED)
}

/// The processor time a process has used (clock ticks), 0 if unknown.
fn cpu_time(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    cpu_ticks(&stat)
}

/// utime + stime from a /proc/<pid>/stat line (after the name, which may
/// hold spaces and brackets of its own).
fn cpu_ticks(stat: &str) -> u64 {
    let fields: Vec<&str> = stat.rsplit_once(')').map_or("", |(_, rest)| rest).split_whitespace().collect();
    let tick = |i: usize| fields.get(i).and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
    tick(11) + tick(12)
}

/// A thread that looks at the server once a second and stops it when its
/// time has come (`time_to_stop`). Working is: a request under way, its
/// processor time going up, or held still to cool. One at a time; it ends
/// with the server.
fn reap_when_done() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("local-model".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let mut running = server();
                let now = Instant::now();
                let done = match running.as_mut() {
                    None => true,
                    Some(r) => {
                        let cpu = cpu_time(r.child.id());
                        let working = cpu > r.cpu + WORKING_TICKS
                            || lm::cooling()
                            || CLIENT.load(Ordering::SeqCst) != 0
                            || TRYING.load(Ordering::SeqCst) != 0;
                        r.cpu = cpu;
                        if working {
                            r.last = now;
                        }
                        // Something else needs the memory (Music, an app):
                        // a model only waiting gives it back now.
                        let short = !working && lm::memory_short(lm::available_mb());
                        !matches!(r.child.try_wait(), Ok(None)) || short || time_to_stop(r.last, now, AWAY.load(Ordering::SeqCst))
                    }
                };
                if done {
                    if let Some(mut r) = running.take() {
                        PID.store(0, Ordering::SeqCst);
                        lm::stop_server(&mut r.child);
                    }
                    AWAY.store(false, Ordering::SeqCst);
                    // Still holding `SERVER`: a model started next finds no
                    // reaper and starts its own.
                    REAPING.store(false, Ordering::SeqCst);
                    break;
                }
            }
        })
        .ok();
}

/// Settings' line for it: which model and whether it is here.
pub fn summary() -> String {
    let dir = lm::dir();
    let model = lm::chosen(&dir);
    if lm::present(&dir, model) {
        format!("{} · ready", model.label.to_lowercase())
    } else {
        "none yet".into()
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", (bytes + 500_000) / 1_000_000)
}

/// What a model is, measured, for its first line: the longest that fits
/// first ("LFM2.5 230M · 8 tokens/s · 149 MB"); its `about` is the second.
fn facts(model: &Model) -> [String; 3] {
    let (speed, size) = (format!("{:.0} tokens/s", model.writes_per_s), megabytes(model.size));
    [format!("{} · {speed} · {size}", model.name), format!("{} · {speed}", model.name), model.name.to_owned()]
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

/// Settings › On-device model: the models offered, a row each (Fastest,
/// Balanced, Smartest), what the one focused is like above them. Enter
/// downloads one, uses one downloaded, and asks the one in use a question;
/// d twice removes one, or stops its download. The one in use says when it
/// is loaded and the memory it holds; u unloads it now.
pub struct ModelPage {
    dir: PathBuf,
    list: ListState,
    /// A download under way: its progress and how it ended.
    downloading: Option<(&'static Model, Arc<Progress>, Receiver<Result<(), String>>)>,
    /// A question to try it with, under way: the answer and its speed.
    trying: Option<(&'static Model, Receiver<Result<(String, Option<f64>), String>>)>,
    /// The last answer to show (and whose), or what went wrong.
    answer: Option<(&'static Model, String)>,
    /// The model `d` was pressed on once: a second press removes it (or
    /// stops its download).
    armed: Option<&'static str>,
    /// The memory the loaded model holds (MB), as last looked at, and how
    /// long ago that was; `held` keeps it as given (the gallery).
    memory: Option<u64>,
    looked: f32,
    held: bool,
    clock: f32,
}

/// How often the page looks again at whether the model is loaded.
const LOOK_EVERY: f32 = 0.5;

const TRY: &str = "In one short sentence: what is a brownout on a robot?";

/// How a model stands on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not here: Enter downloads it.
    Away,
    Downloading,
    /// Here, and not the one asked.
    Here,
    /// Here and chosen: the one the Assistant and `ai` ask.
    InUse,
}

impl ModelPage {
    pub fn new() -> Self {
        Self::at(lm::dir())
    }

    /// On the models kept in `dir`.
    fn at(dir: PathBuf) -> Self {
        lm::clear_retired(&dir);
        // Open on the one in use.
        let chosen = lm::chosen(&dir);
        let mut list = ListState::default();
        list.focus = MODELS.iter().position(|m| m.id == chosen.id).unwrap_or(lm::DEFAULT);
        ModelPage { dir, list, downloading: None, trying: None, answer: None, armed: None, memory: loaded(), looked: 0.0, held: false, clock: 0.0 }
    }

    fn focused(&self) -> &'static Model {
        &MODELS[self.list.focus.min(MODELS.len() - 1)]
    }

    fn state(&self, model: &Model) -> State {
        match &self.downloading {
            Some((m, ..)) if m.id == model.id => State::Downloading,
            _ if !lm::present(&self.dir, model) => State::Away,
            _ if lm::chosen(&self.dir).id == model.id => State::InUse,
            _ => State::Here,
        }
    }

    fn start_download(&mut self, model: &'static Model) {
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

    /// This one is the one asked from now on: the one running stops, and the
    /// next question starts this one.
    fn use_it(&mut self, model: &'static Model, cx: &mut Cx) {
        match lm::set_chosen(&self.dir, model) {
            Ok(()) => {
                stop();
                self.answer = None;
                *cx.say = Some(format!("{} is in use · enter asks it a question", model.name));
            }
            Err(e) => *cx.say = Some(format!("could not choose it: {e}")),
        }
    }

    fn start_trying(&mut self, model: &'static Model) {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("model-try".into())
            .spawn(move || {
                // Working, for the rules on stopping, until it has answered.
                TRYING.fetch_add(1, Ordering::SeqCst);
                let asked = start().and_then(|_| {
                    let body = lm::chat_body("You are a small assistant on a handheld. Answer in one short sentence.", &[("user", TRY)], 80);
                    let answer = lm::ask(&body, Duration::from_secs(120));
                    touch();
                    answer
                });
                TRYING.fetch_sub(1, Ordering::SeqCst);
                tx.send(asked).ok();
            })
            .ok();
        self.trying = Some((model, rx));
        self.answer = None;
    }

    fn remove(&mut self, model: &'static Model, cx: &mut Cx) {
        if lm::chosen(&self.dir).id == model.id {
            stop();
        }
        lm::remove(&self.dir, model);
        if matches!(&self.answer, Some((m, _)) if m.id == model.id) {
            self.answer = None;
        }
        *cx.say = Some(format!("{} removed · {} back", model.name, megabytes(model.size)));
    }

    /// The row's value: how it stands, or how far its download is.
    fn value(&self, model: &Model) -> String {
        match (self.state(model), &self.downloading) {
            (State::Downloading, Some((_, p, _))) if p.checking.load(Ordering::Relaxed) => "checking".into(),
            (State::Downloading, Some((m, p, _))) => {
                let got = p.bytes.load(Ordering::Relaxed);
                format!("{}% · {} of {}", got * 100 / m.size.max(1), megabytes(got), megabytes(m.size))
            }
            (State::InUse, _) => match self.memory {
                Some(mb) => format!("loaded · {mb} MB"),
                None => "in use · not loaded".into(),
            },
            (State::Here, _) => "downloaded".into(),
            _ => megabytes(model.size),
        }
    }

    /// Settings' "unload now": its memory back at once.
    fn unload(&mut self, cx: &mut Cx) {
        match self.memory.take() {
            Some(mb) => {
                unload_now();
                *cx.say = Some(format!("unloaded · {mb} MB back"));
            }
            None => *cx.say = Some("not loaded · it loads when asked".into()),
        }
    }

    /// In the gallery: a download partway, as it would show.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_download(at: usize, fraction: f64) -> Self {
        let mut page = Self::new();
        let model = &MODELS[at];
        let progress = Arc::new(Progress::default());
        progress.bytes.store((model.size as f64 * fraction) as u64, Ordering::Relaxed);
        let (_tx, rx) = mpsc::channel();
        std::mem::forget(_tx);
        page.downloading = Some((model, progress, rx));
        page.list.focus = at;
        page
    }

    /// In the gallery: an answer from trying the one in use.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_answer(answer: &str) -> Self {
        let mut page = Self::new();
        page.answer = Some((lm::chosen(&page.dir), answer.to_owned()));
        page
    }

    /// In the gallery: the focus on another row.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn focused_on(mut self, at: usize) -> Self {
        self.list.focus = at;
        self
    }

    /// In the gallery: the one in use loaded, holding `mb`.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_loaded(mut self, mb: u64) -> Self {
        self.memory = Some(mb);
        self.held = true;
        self
    }

    /// A download or a question under way.
    fn working(&self) -> bool {
        self.downloading.is_some() || self.trying.is_some()
    }
}

impl Screen for ModelPage {
    fn title(&self) -> &str {
        // "On-device model" runs into the island.
        "On device"
    }

    /// Looked at every beat while something moves on it, or while a model
    /// is loaded (to see it go when it is unloaded).
    fn busy(&self) -> bool {
        self.working() || self.memory.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.working()
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.looked += dt;
        let mut changed = false;
        if self.looked >= LOOK_EVERY && !self.held {
            self.looked = 0.0;
            let memory = loaded();
            changed = memory != self.memory;
            self.memory = memory;
        }
        self.working() || changed
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some((model, _, rx)) = &self.downloading {
            match rx.try_recv() {
                Ok(result) => {
                    let model = *model;
                    self.downloading = None;
                    match result {
                        // The first one down, with none chosen yet: the one in use.
                        Ok(()) if !lm::was_chosen(&self.dir) || !lm::present(&self.dir, lm::chosen(&self.dir)) => {
                            lm::set_chosen(&self.dir, model).ok();
                            stop();
                            *cx.say = Some(format!("{} is ready and in use", model.name));
                        }
                        Ok(()) => *cx.say = Some(format!("{} is ready · enter uses it", model.name)),
                        Err(why) => *cx.say = Some(why),
                    }
                }
                Err(TryRecvError::Disconnected) => self.downloading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((model, rx)) = &self.trying {
            match rx.try_recv() {
                Ok(Ok((answer, speed))) => {
                    if let Some(s) = speed {
                        *cx.say = Some(format!("answered at {s:.1} tokens a second"));
                    }
                    self.answer = Some((*model, answer));
                    self.trying = None;
                }
                Ok(Err(why)) => {
                    *cx.say = Some(why.clone());
                    self.answer = Some((*model, why));
                    self.trying = None;
                }
                Err(TryRecvError::Disconnected) => self.trying = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.armed.take();
        let model = self.focused();
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter | Key::Right => match self.state(model) {
                State::Downloading => *cx.say = Some("downloading · d twice stops it".into()),
                State::Away if self.downloading.is_some() => *cx.say = Some("one download at a time".into()),
                State::Away => self.start_download(model),
                State::Here if self.trying.is_some() => *cx.say = Some("still answering".into()),
                State::Here => self.use_it(model, cx),
                State::InUse if self.trying.is_some() => *cx.say = Some("still answering".into()),
                State::InUse => self.start_trying(model),
            },
            Key::Char('d') => match self.state(model) {
                State::Downloading if armed == Some(model.id) => {
                    if let Some((_, progress, _)) = &self.downloading {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                    *cx.say = Some("download stopped · it carries on from here next time".into());
                }
                State::Downloading => {
                    self.armed = Some(model.id);
                    *cx.say = Some("d again stops the download".into());
                }
                State::Here | State::InUse if self.trying.is_some() => *cx.say = Some("still answering".into()),
                State::Here | State::InUse if armed == Some(model.id) => self.remove(model, cx),
                State::Here | State::InUse => {
                    self.armed = Some(model.id);
                    *cx.say = Some(format!("d again removes {} from the card", model.name));
                }
                State::Away => *cx.say = Some("not downloaded".into()),
            },
            Key::Char('u') => self.unload(cx),
            other => crate::screens::list_key(&mut self.list, other, MODELS.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let model = self.focused();
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let cap_lh = ty.caption.size + 4.0;

        // What the focused one is (two lines, so all three rows fit: its
        // name and pace, then what it is like), or the answer it just gave
        // (three).
        let width = W - 2.0 * PAD;
        let (mut lines, most, color) = match (&self.answer, &self.trying) {
            (_, Some((m, _))) => {
                let dots = ".".repeat(1 + (self.clock * 2.0) as usize % 3);
                let doing = if lm::cooling() { "is letting the processor cool" } else { "is thinking" };
                (crate::help::wrap(g, &format!("{} {doing}{dots}", m.name), ty.caption, width), 2, t.dim)
            }
            (Some((m, answer)), None) if m.id == model.id => (crate::help::wrap(g, &format!("“{answer}”"), ty.caption, width), 3, t.ink),
            _ => {
                let facts = facts(model);
                let first = facts.iter().find(|f| g.measure(f, ty.caption) <= width).unwrap_or(&facts[2]).clone();
                (vec![first, model.about.to_owned()], 2, t.dim)
            }
        };
        if lines.len() > most {
            lines.truncate(most);
            if let Some(last) = lines.last_mut() {
                *last = g.text.fit(&format!("{last}…"), &ty.caption, width);
            }
        }
        for (k, line) in lines.iter().enumerate() {
            g.text_at(PAD + 2.0, top + 1.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, color);
        }
        let rows_top = top + 4.0 + lines.len().max(2) as f32 * cap_lh;

        for (i, y) in self.list.layout(g, MODELS.len(), rows_top, row_h, 3.0, bottom) {
            let m = &MODELS[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let value_color = match self.state(m) {
                State::InUse => g.good(),
                State::Away => t.signal,
                _ => t.dim,
            };
            g.row(r, m.label, &self.value(m), value_color, i == self.list.focus);
            if let Some((d, p, _)) = &self.downloading {
                if d.id == m.id {
                    let fraction = p.bytes.load(Ordering::Relaxed) as f32 / d.size.max(1) as f32;
                    g.meter(Rect::new(r.x + 12.0, r.bottom() - 3.5, r.w - 24.0, 2.0), fraction, t.signal);
                }
            }
        }
        self.list.scrollbar(g, MODELS.len(), rows_top, bottom);
        match self.state(model) {
            State::Downloading => g.hint_any(&["d twice: stop · it carries on later", "d twice: stop"]),
            State::Away => g.hint_any(&["enter: download from Hugging Face", "enter: download"]),
            State::Here => g.hint_any(&["enter: use this one · d twice: remove", "enter: use it · d d: remove"]),
            State::InUse if self.memory.is_some() => g.hint_any(&["enter: ask it · u: unload now · d d: remove", "enter: ask · u: unload now", "u: unload now"]),
            State::InUse => g.hint_any(&["enter: ask it a question · d twice: remove", "enter: ask it · d d: remove"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unasked_model_stops_after_two_minutes_and_at_once_when_nobody_looks() {
        let start = Instant::now();
        let after = |s: u64| start + Duration::from_secs(s);
        assert_eq!(IDLE, Duration::from_secs(120), "two minutes");
        assert!(!time_to_stop(start, after(60), false));
        assert!(!time_to_stop(start, start + IDLE - Duration::from_secs(1), false));
        assert!(time_to_stop(start, start + IDLE, false));
        assert!(!time_to_stop(after(10), start, false), "a clock that went back is not idle");
        // Nobody looking: as soon as it is done, though not in the moment
        // between being started and being asked.
        assert!(!time_to_stop(start, after(2), true));
        assert!(time_to_stop(start, start + JUST_ASKED, true));
        assert!(JUST_ASKED < Duration::from_secs(10));
    }

    #[test]
    fn the_memory_a_model_holds_is_read_from_the_kernel() {
        let status = "Name:\tllama-server\nVmHWM:\t  270000 kB\nVmRSS:\t  195584 kB\nRssAnon:\t   41000 kB\n";
        assert_eq!(resident_mb(status), Some(191));
        assert_eq!(resident_mb("Name:\tx\n"), None);
    }

    #[test]
    fn an_answer_given_up_on_is_cut_short() {
        // The request under way is known, and forgotten once over.
        answering(Some(4242));
        assert_eq!(CLIENT.load(Ordering::SeqCst), 4242);
        answering(None);
        assert_eq!(CLIENT.load(Ordering::SeqCst), 0);
        cancel();
        // Given up on: the request (a process of its own) is stopped.
        #[cfg(target_os = "linux")]
        {
            let mut request = Command::new("sleep").arg("30").spawn().unwrap();
            answering(Some(request.id()));
            cancel();
            assert_eq!(CLIENT.load(Ordering::SeqCst), 0);
            let started = Instant::now();
            while request.try_wait().unwrap().is_none() {
                assert!(started.elapsed() < Duration::from_secs(5), "the request was not stopped");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[test]
    fn nothing_is_loaded_until_a_question_needs_it() {
        // Opening the pages starts nothing; with nothing running, being
        // looked away from has nothing to let go.
        crate::shell::tests::sandbox();
        let dir = std::env::temp_dir().join(format!("catalyst-local-none-{}", std::process::id()));
        let page = ModelPage::at(dir.clone());
        assert!(SERVER.try_lock().map_or(true, |s| s.is_none()), "the page started nothing");
        assert_eq!(page.value(&MODELS[0]), megabytes(MODELS[0].size));
        std::fs::remove_dir_all(&dir).ok();
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
    fn a_working_model_is_seen_by_its_processor_time() {
        // A name with spaces and a bracket of its own, as /proc may show.
        let stat = "4242 (llama-server (x)) R 1 4242 4242 0 -1 4194304 900 0 0 0 1234 56 0 0 30 10 7 0 100 0";
        assert_eq!(cpu_ticks(stat), 1290);
        assert_eq!(cpu_ticks(""), 0);
        assert_eq!(cpu_time(u32::MAX), 0, "no such process");
    }

    #[test]
    fn each_model_is_described_by_what_was_measured() {
        let m = &MODELS[1];
        let [whole, shorter, name] = facts(m);
        assert_eq!(whole, format!("{} · {:.0} tokens/s · {}", m.name, m.writes_per_s, megabytes(m.size)));
        assert!(whole.starts_with(&shorter) && shorter.starts_with(&name), "each shorter one a start of the one before");
        for m in &MODELS {
            // What fits a line of captions at the largest text size.
            assert!(facts(m)[0].chars().count() <= 36, "{}", facts(m)[0]);
            assert!(m.about.chars().count() <= 36 && !m.about.ends_with('.'), "{}", m.about);
        }
    }

    #[test]
    fn settings_says_which_model_and_whether_it_is_here() {
        assert_eq!(megabytes(144_811_360), "145 MB");
        crate::shell::tests::sandbox();
        let dir = std::env::temp_dir().join(format!("catalyst-local-summary-{}", std::process::id()));
        std::env::set_var("CATALYST_MODELS", &dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(summary(), "none yet");
        assert!(!ready());
        assert_eq!(model_name(), MODELS[lm::DEFAULT].name);
        assert!(start().unwrap_err().contains("not downloaded"), "nothing to start");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A stand-in file of the model's size, accepted as checked.
    fn stand_in(dir: &Path, model: &Model) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::File::create(lm::partial(dir, model)).unwrap().set_len(model.size).unwrap();
        lm::accept(dir, model, model.sha256).unwrap();
    }

    #[test]
    fn the_page_downloads_uses_and_removes_each_model() {
        crate::shell::tests::sandbox();
        let dir = std::env::temp_dir().join(format!("catalyst-local-page-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        stand_in(&dir, &MODELS[0]);
        stand_in(&dir, &MODELS[2]);
        std::fs::write(dir.join(lm::RETIRED[0]), b"an old model").unwrap();
        let mut page = ModelPage::at(dir.clone());
        // What is loaded is the process's own (other tests pretend): none here.
        page.memory = None;
        page.held = true;
        assert!(!dir.join(lm::RETIRED[0]).exists(), "the one no longer offered is taken off the card");
        assert_eq!(page.list.focus, lm::DEFAULT, "it opens on the one in use");
        assert_eq!([0, 1, 2].map(|i| page.state(&MODELS[i])), [State::Here, State::Away, State::Here]);
        assert_eq!(page.value(&MODELS[1]), megabytes(MODELS[1].size), "what it would download");
        assert_eq!(page.value(&MODELS[0]), "downloaded");

        let (mut robot, mut settings, mut say) = (Robot::demo(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        // Enter on one downloaded: it is the one in use.
        page.list.focus = 0;
        page.key(Key::Enter, &mut cx);
        assert_eq!(lm::chosen(&dir).id, MODELS[0].id);
        assert_eq!(page.state(&MODELS[0]), State::InUse);
        assert_eq!(page.value(&MODELS[0]), "in use · not loaded");
        assert!(cx.say.take().unwrap().contains("in use"));
        // Loaded: it says so, and what it holds; u gives that back.
        page.memory = Some(191);
        assert_eq!(page.value(&MODELS[0]), "loaded · 191 MB");
        page.key(Key::Char('u'), &mut cx);
        assert_eq!(cx.say.take().unwrap(), "unloaded · 191 MB back");
        assert_eq!(page.value(&MODELS[0]), "in use · not loaded");
        page.key(Key::Char('u'), &mut cx);
        assert!(cx.say.take().unwrap().contains("not loaded"));

        // d once warns; anything else between disarms it.
        page.list.focus = 2;
        page.key(Key::Char('d'), &mut cx);
        assert!(cx.say.take().unwrap().contains("d again removes"));
        page.key(Key::Up, &mut cx);
        page.key(Key::Down, &mut cx);
        page.key(Key::Char('d'), &mut cx);
        assert!(lm::present(&dir, &MODELS[2]), "not removed: the second d was not straight after");
        page.key(Key::Char('d'), &mut cx);
        assert!(!lm::present(&dir, &MODELS[2]), "d twice removes it");
        assert!(cx.say.take().unwrap().contains("removed"));
        assert_eq!(page.state(&MODELS[2]), State::Away);
        // d on one not here does nothing to it.
        page.key(Key::Char('d'), &mut cx);
        page.key(Key::Char('d'), &mut cx);
        assert_eq!(lm::chosen(&dir).id, MODELS[0].id, "the one in use is untouched");
        std::fs::remove_dir_all(&dir).ok();
    }
}
