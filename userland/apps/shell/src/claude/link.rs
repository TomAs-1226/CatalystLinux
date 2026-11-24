//! The handheld's side of the bridge: the pairing kept, every request
//! sealed and sent with curl over HTTPS, the stream of events held while it
//! is needed, and what came of them, kept for the pages ([`State`]).
//!
//! - **Held only while needed.** The stream is open while a Claude page is
//!   ([`hold`]) or a reply or a permission request is still to come; then
//!   curl is stopped and nothing more is sent or polled. A request is one
//!   curl, on a thread of its own, when a page asks.
//! - **The pages never wait on the network.** They read [`State`] under its
//!   lock; what arrives changes it, bumps [`generation`] and wakes the shell
//!   (`platform::wake`), which looks in on its next turn.
//! - **Away from the app**, a reply finishing or a permission request
//!   waiting is told by a notification (said in the island) and a live
//!   activity while the turn runs.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use catalyst_companion_proto::claude::{self as proto, Key};
use serde_json::{json, Value};

use super::Paired;

// ---------------------------------------------------------------- trouble ---

/// Why the laptop could not be reached, as the pages say it.
#[derive(Clone, Debug, PartialEq)]
pub enum Trouble {
    /// No internet on the handheld.
    Offline,
    /// The laptop asleep, off, or its bridge not running.
    Asleep,
    /// The bridge does not know this handheld's key.
    NotPaired,
    /// Anything else, in a few words.
    Other(String),
}

impl Trouble {
    pub fn words(&self) -> &str {
        match self {
            Trouble::Offline => "no internet on the handheld",
            Trouble::Asleep => "laptop asleep or unreachable",
            Trouble::NotPaired => "the laptop no longer knows this handheld",
            Trouble::Other(w) => w,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Trouble::Offline => "wifi_off",
            Trouble::Asleep => "bedtime",
            Trouble::NotPaired => "link_off",
            Trouble::Other(_) => "error",
        }
    }
}

/// Why curl got no answer.
pub fn curl_trouble(code: Option<i32>) -> Trouble {
    match code {
        // The name did not resolve: the handheld has no internet.
        Some(6) => Trouble::Offline,
        // Refused, timed out, TLS cut, nothing or a broken answer: the
        // laptop (or the bridge on it) is not there.
        Some(7 | 28 | 35 | 52 | 55 | 56) => Trouble::Asleep,
        Some(60) => Trouble::Other("the laptop's certificate was not trusted · is the clock right?".into()),
        _ => Trouble::Asleep,
    }
}

/// What an HTTP status other than 200 means.
pub fn status_trouble(status: u16, body: &str) -> Trouble {
    match status {
        401 => Trouble::NotPaired,
        // Funnel answers for a laptop whose bridge is not running.
        502..=504 | 0 => Trouble::Asleep,
        _ => Trouble::Other(format!("the laptop answered {status}{}", if body.trim().is_empty() { String::new() } else { format!(": {}", body.trim()) })),
    }
}

// ----------------------------------------------------------------- rows ---

/// One row of a conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    /// "you", "claude", "tool" or "note".
    pub kind: String,
    pub text: String,
    pub icon: String,
    /// A tool's: "run", "ok", "error", "denied", "cut".
    pub state: String,
    pub detail: String,
    /// Bumped whenever it changes, for its layout to be made again.
    pub rev: u64,
}

static REV: AtomicU64 = AtomicU64::new(1);

fn rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}

impl Row {
    pub fn from_json(v: &Value) -> Row {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        Row { id: s("id"), kind: s("k"), text: s("text"), icon: s("icon"), state: s("state"), detail: s("detail"), rev: rev() }
    }
}

/// A permission request waiting for an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Approval {
    pub id: String,
    pub session: String,
    pub what: String,
    pub line: String,
    pub place: String,
    pub detail: String,
    /// When it is denied unanswered (seconds since 1970, the laptop's clock).
    pub until: i64,
}

impl Approval {
    fn from_json(session: &str, v: &Value) -> Approval {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        Approval { id: s("id"), session: session.to_owned(), what: s("what"), line: s("line"), place: s("where"), detail: s("detail"), until: v["until"].as_i64().unwrap_or(0) }
    }
}

/// A session, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub project: String,
    pub cwd: String,
    /// Seconds since 1970, the laptop's clock.
    pub last: i64,
    pub mode: String,
}

/// The conversation open on the handheld.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Convo {
    /// None for a new session until its first message is sent.
    pub session: Option<String>,
    pub cwd: String,
    pub project: String,
    pub title: String,
    pub rows: Vec<Row>,
    /// Its history has come.
    pub loaded: bool,
    /// "ask" or "edits".
    pub mode: String,
    /// What it said went wrong last, until the next message.
    pub error: Option<String>,
    /// The event its history was read at: only later events add to it.
    pub since: (String, u64),
    /// Its last message, kept to send again if it could not go.
    pub unsent: Option<String>,
    /// How Claude's turn is going, when the laptop says ("retrying · attempt 2").
    pub status: String,
}

// ---------------------------------------------------------------- state ---

/// Everything the pages show, kept here.
#[derive(Default)]
pub struct State {
    pub paired: Option<Paired>,
    pub trouble: Option<Trouble>,
    /// A request or the stream is on its way.
    pub asking: bool,
    pub sessions: Option<Vec<Session>>,
    /// The laptop's clock when the list came (for "5 m ago").
    pub laptop_now: i64,
    pub projects: Option<Vec<(String, String, i64)>>,
    pub convo: Option<Convo>,
    /// Sessions Claude is working in, and their project's name.
    pub working: HashMap<String, String>,
    pub approvals: Vec<Approval>,
    /// The newest thing Claude wrote in each session, for its notification.
    last_words: HashMap<String, String>,
    epoch: String,
    seq: u64,
    holds: usize,
    streaming: bool,
    tasks: HashMap<String, crate::liveactivity::TaskHandle>,
    /// Pairing: how it went ("" while under way).
    pub pairing: Option<Result<String, String>>,
}

impl State {
    /// Whether the stream should be open.
    fn needed(&self) -> bool {
        self.paired.is_some() && (self.holds > 0 || !self.working.is_empty() || !self.approvals.is_empty())
    }

    pub fn is_working(&self, session: &str) -> bool {
        self.working.contains_key(session)
    }

    /// The oldest request waiting in `session`.
    pub fn approval_in(&self, session: &str) -> Option<&Approval> {
        self.approvals.iter().find(|a| a.session == session)
    }
}

#[cfg(not(test))]
static STATE: Mutex<Option<State>> = Mutex::new(None);
// Each test thread its own, as the tests run side by side.
#[cfg(test)]
thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}
static GENERATION: AtomicU64 = AtomicU64::new(1);
/// The laptop's clock less this one's, in milliseconds.
static OFFSET: AtomicI64 = AtomicI64::new(0);
/// Wakes the stream's thread from its wait before trying again.
static NUDGE: Condvar = Condvar::new();
static NUDGE_LOCK: Mutex<bool> = Mutex::new(false);
/// The stream's curl, to stop when it is no longer needed.
static STREAM: Mutex<Option<Child>> = Mutex::new(None);
/// The tests and the screenshots: nothing leaves the handheld.
static OFFLINE_DEMO: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The state, read or changed.
#[cfg(not(test))]
pub fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = lock(&STATE);
    let s = g.get_or_insert_with(|| State { paired: super::load(), ..State::default() });
    f(s)
}

#[cfg(test)]
pub fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|cell| f(cell.borrow_mut().get_or_insert_with(State::default)))
}

/// Bumped whenever something the pages show changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn changed() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    crate::platform::wake();
}

/// Nothing leaves the handheld from now on (the tests and the shots), and
/// the state starts again from `state`.
#[allow(dead_code)]
pub fn install_demo(f: impl FnOnce(&mut State)) {
    OFFLINE_DEMO.store(true, Ordering::Relaxed);
    with(|s| {
        *s = State::default();
        f(s);
    });
    changed();
}

fn demo() -> bool {
    OFFLINE_DEMO.load(Ordering::Relaxed)
}

fn now_ms() -> u64 {
    let local = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    (local + OFFSET.load(Ordering::Relaxed)).max(0) as u64
}

/// The laptop's clock, as near as this one can tell.
pub fn laptop_now() -> i64 {
    (now_ms() / 1000) as i64
}

// -------------------------------------------------------------- holding ---

/// A Claude page is open: the stream is held (and the list fetched the
/// first time). Balanced by [`release`].
pub fn hold() {
    let first = with(|s| {
        s.holds += 1;
        s.holds == 1
    });
    if first {
        ensure_stream();
    }
}

/// A Claude page has closed or been left.
pub fn release() {
    with(|s| s.holds = s.holds.saturating_sub(1));
    settle();
}

/// The stream opened if it is needed and is not open, or stopped if it is
/// open and no longer needed.
fn settle() {
    let (needed, streaming) = with(|s| (s.needed(), s.streaming));
    if !needed && streaming {
        if let Some(mut c) = lock(&STREAM).take() {
            let _ = c.kill();
        }
        nudge();
    } else if needed {
        ensure_stream();
    }
}

fn nudge() {
    *lock(&NUDGE_LOCK) = true;
    NUDGE.notify_all();
}

/// Wait up to `d`, or until nudged.
fn rest(d: Duration) {
    let g = lock(&NUDGE_LOCK);
    let (mut g, _) = NUDGE.wait_timeout_while(g, d, |nudged| !*nudged).unwrap_or_else(|e| e.into_inner());
    *g = false;
}

// ------------------------------------------------------------ transport ---

fn handheld_name() -> String {
    crate::sys::hostname().map(|h| h.trim().to_owned()).filter(|h| !h.is_empty()).unwrap_or_else(|| "catalyst".into())
}

/// One sealed request to the bridge at `address` with `key`, its answer.
/// A clock the laptop says is off is set right from its, and the request
/// sent again once.
pub fn request_to(address: &str, key: &Key, op: &Value) -> Result<Value, Trouble> {
    if demo() {
        return Err(Trouble::Asleep);
    }
    for _ in 0..2 {
        let mut op = op.clone();
        op["name"] = Value::from(handheld_name());
        let (rid, body) = key.seal_request(&op, now_ms()).ok_or(Trouble::Other("no random numbers".into()))?;
        let (status, text) = post(address, proto::PATH, &body, 25)?;
        match status {
            200 => {
                let v = key.open_reply(&rid, &text).ok_or_else(|| Trouble::Other("the answer was not sealed for this handheld".into()))?;
                if v["error"] == "clock" {
                    if let Some(laptop) = v["now"].as_i64() {
                        let local = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
                        OFFSET.store(laptop - local, Ordering::Relaxed);
                        continue;
                    }
                }
                return Ok(v);
            }
            // Taken already: a request made again goes with a new id.
            409 => continue,
            s => return Err(status_trouble(s, &text)),
        }
    }
    Err(Trouble::Other("the laptop did not take the request".into()))
}

/// A POST with curl: the body on its input, never on its command line.
fn post(address: &str, path: &str, body: &str, seconds: u32) -> Result<(u16, String), Trouble> {
    let url = format!("{address}{path}");
    let mut child = crate::tools::quiet("curl")
        .args(["-sS", "--proto", "=https", "--max-time", &seconds.to_string(), "-H", "Content-Type: text/plain", "--data-binary", "@-", "-w", "\n%{http_code}", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Trouble::Other("curl could not start".into()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.as_bytes()).ok();
    }
    let out = child.wait_with_output().map_err(|_| Trouble::Asleep)?;
    if !out.status.success() {
        return Err(curl_trouble(out.status.code()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok((code.trim().parse().unwrap_or(0), body.to_owned()))
}

/// A request with the pairing kept, on a thread of its own; `then` with
/// its answer (on that thread).
fn spawn(op: Value, then: impl FnOnce(Result<Value, Trouble>) + Send + 'static) {
    let Some(p) = with(|s| s.paired.clone()) else { return };
    // The tests and the shots: nothing is asked.
    if demo() {
        return;
    }
    with(|s| s.asking = true);
    changed();
    std::thread::spawn(move || {
        let r = request_to(&p.address, &p.key, &op);
        with(|s| {
            s.asking = false;
            match &r {
                Ok(_) => s.trouble = None,
                Err(t) => s.trouble = Some(t.clone()),
            }
        });
        then(r);
        changed();
    });
}

// ------------------------------------------------------------- requests ---

/// The sessions, fetched again.
pub fn refresh() {
    spawn(json!({"op": "sessions"}), |r| {
        let Ok(v) = r else { return };
        with(|s| {
            let list: Vec<Session> = v["sessions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| {
                    let st = |k: &str| x[k].as_str().unwrap_or_default().to_owned();
                    if x["running"] == true && !s.working.contains_key(&st("id")) {
                        s.working.insert(st("id"), st("project"));
                    }
                    Session { id: st("id"), title: st("title"), project: st("project"), cwd: st("cwd"), last: x["last"].as_i64().unwrap_or(0), mode: st("mode") }
                })
                .collect();
            s.sessions = Some(list);
            s.laptop_now = v["now"].as_i64().unwrap_or_else(laptop_now);
            take_place(s, &v);
        });
        settle();
    });
}

/// Where the stream starts from, if it has not begun.
fn take_place(s: &mut State, v: &Value) {
    if !s.streaming || s.epoch.is_empty() {
        if let (Some(e), Some(q)) = (v["epoch"].as_str(), v["seq"].as_u64()) {
            s.epoch = e.to_owned();
            s.seq = q;
        }
    }
}

/// The folders to begin a session in.
pub fn fetch_projects() {
    spawn(json!({"op": "projects"}), |r| {
        if let Ok(v) = r {
            let list = v["projects"].as_array().into_iter().flatten().map(|p| (p["cwd"].as_str().unwrap_or_default().to_owned(), p["name"].as_str().unwrap_or_default().to_owned(), p["last"].as_i64().unwrap_or(0))).collect();
            with(|s| s.projects = Some(list));
        }
    });
}

/// Open a session: its rows fetched.
pub fn open(session: &Session) {
    let id = session.id.clone();
    with(|s| {
        s.convo = Some(Convo { session: Some(id.clone()), cwd: session.cwd.clone(), project: session.project.clone(), title: session.title.clone(), mode: if session.mode.is_empty() { "ask".into() } else { session.mode.clone() }, ..Convo::default() })
    });
    changed();
    spawn(json!({"op": "history", "session": id}), move |r| {
        with(|s| {
            let Some(c) = s.convo.as_mut().filter(|c| c.session.as_deref() == Some(id.as_str())) else { return };
            match r {
                Ok(v) if v["error"].is_string() => c.error = v["error"].as_str().map(str::to_owned),
                Ok(v) => {
                    c.rows = v["rows"].as_array().into_iter().flatten().map(Row::from_json).collect();
                    c.loaded = true;
                    c.mode = v["mode"].as_str().unwrap_or("ask").to_owned();
                    c.since = (v["epoch"].as_str().unwrap_or_default().to_owned(), v["seq"].as_u64().unwrap_or(0));
                    let project = c.project.clone();
                    if v["running"] == true {
                        s.working.entry(id.clone()).or_insert(project);
                    } else {
                        s.working.remove(&id);
                    }
                    for a in v["approvals"].as_array().into_iter().flatten() {
                        let a = Approval::from_json(&id, a);
                        if !s.approvals.iter().any(|x| x.id == a.id) {
                            s.approvals.push(a);
                        }
                    }
                    take_place(s, &v);
                }
                Err(_) => {}
            }
        });
        settle();
    });
}

/// Begin a conversation in a folder: nothing is sent until its first message.
pub fn begin(cwd: &str, project: &str) {
    with(|s| s.convo = Some(Convo { cwd: cwd.to_owned(), project: project.to_owned(), title: format!("new in {project}"), loaded: true, mode: "ask".into(), ..Convo::default() }));
    changed();
}

/// A message to Claude in the open conversation.
pub fn send(text: &str) {
    let Some((session, cwd, mode, project)) = with(|s| s.convo.as_ref().map(|c| (c.session.clone(), c.cwd.clone(), c.mode.clone(), c.project.clone()))) else { return };
    let mut op = json!({"op": "send", "text": text, "mode": mode});
    match &session {
        Some(id) => op["session"] = Value::from(id.clone()),
        None => op["cwd"] = Value::from(cwd),
    }
    with(|s| {
        if let Some(c) = s.convo.as_mut() {
            c.error = None;
            c.unsent = Some(text.to_owned());
        }
        // Working from now: the stream is held for its reply.
        s.working.insert(session.clone().unwrap_or_else(|| "new".into()), project.clone());
    });
    settle();
    let text = text.to_owned();
    spawn(op, move |r| {
        with(|s| {
            let pending = session.clone().unwrap_or_else(|| "new".into());
            let ok = matches!(&r, Ok(v) if v["ok"] == true);
            if let Some(c) = s.convo.as_mut().filter(|c| c.session == session) {
                match &r {
                    Ok(v) if ok => {
                        c.unsent = None;
                        if c.session.is_none() {
                            c.session = v["session"].as_str().map(str::to_owned);
                            c.title = super::ui::first_words(&text);
                        }
                    }
                    Ok(v) => c.error = Some(v["words"].as_str().or(v["error"].as_str()).unwrap_or("it could not be sent").to_owned()),
                    Err(t) => c.error = Some(t.words().to_owned()),
                }
            }
            s.working.remove(&pending);
            if let (true, Ok(v)) = (ok, &r) {
                if let Some(id) = v["session"].as_str() {
                    s.working.insert(id.to_owned(), project.clone());
                }
            }
        });
        settle();
    });
}

/// Stop Claude in the open conversation.
pub fn interrupt() {
    let Some(id) = with(|s| s.convo.as_ref().and_then(|c| c.session.clone())) else { return };
    spawn(json!({"op": "interrupt", "session": id}), |_| {});
}

/// The answer to a permission request.
pub fn answer(approval: &str, allow: bool) {
    let id = approval.to_owned();
    // Taken off at once: the bridge says when it is done with it.
    with(|s| s.approvals.retain(|a| a.id != id));
    changed();
    spawn(json!({"op": "answer", "approval": approval, "allow": allow}), |_| {});
}

/// The open conversation's way of asking: "ask" or "edits".
pub fn set_mode(mode: &str) {
    let session = with(|s| {
        let c = s.convo.as_mut()?;
        c.mode = mode.to_owned();
        c.session.clone()
    });
    changed();
    if let Some(id) = session {
        spawn(json!({"op": "mode", "session": id, "mode": mode}), |_| {});
    }
}

/// Unpair: the laptop told to forget this handheld's key, and it forgotten here.
pub fn unpair() {
    if let Some(p) = with(|s| s.paired.clone()) {
        std::thread::spawn(move || {
            let _ = request_to(&p.address, &p.key, &json!({"op": "unpair"}));
        });
    }
    super::forget();
    with(|s| *s = State { holds: s.holds, ..State::default() });
    settle();
    changed();
}

/// Pair with what the QR code said, or the code gave: kept, then the
/// laptop greeted (its first request completes the pairing there).
pub fn pair(p: Paired) {
    with(|s| {
        s.pairing = Some(Ok(String::new()));
        s.trouble = None;
    });
    changed();
    if demo() {
        with(|s| s.pairing = Some(Err(Trouble::Asleep.words().to_owned())));
        return;
    }
    std::thread::spawn(move || {
        let r = request_to(&p.address, &p.key, &json!({"op": "hello"}));
        let outcome = match r {
            Ok(v) => {
                let computer = v["computer"].as_str().map(proto::clean_name).filter(|c| !c.is_empty()).unwrap_or(p.computer.clone());
                let kept = Paired { computer, ..p };
                match super::save(&kept) {
                    Ok(()) => {
                        let name = kept.computer.clone();
                        with(|s| {
                            *s = State { holds: s.holds, paired: Some(kept), ..State::default() };
                        });
                        Ok(name)
                    }
                    Err(why) => Err(why),
                }
            }
            Err(t) => Err(t.words().to_owned()),
        };
        with(|s| s.pairing = Some(outcome));
        changed();
        settle();
        refresh();
    });
}

/// Pair with a six-digit code: SPAKE2 with the bridge at `address`.
pub fn pair_by_code(address: &str, code: &str) {
    let (address, code) = (address.to_owned(), code.to_owned());
    with(|s| s.pairing = Some(Ok(String::new())));
    changed();
    if demo() {
        with(|s| s.pairing = Some(Err(Trouble::Asleep.words().to_owned())));
        return;
    }
    std::thread::spawn(move || {
        let (asking, ask) = proto::pairing::Asking::new(&code);
        let body = json!({"ask": catalyst_companion_proto::hex(&ask)}).to_string();
        let got = post(&address, proto::PAIR_PATH, &body, 20);
        let outcome = match got {
            Ok((200, text)) => {
                let v: Value = serde_json::from_str(&text).unwrap_or_default();
                let part = |k: &str| v[k].as_str().and_then(catalyst_companion_proto::unhex).unwrap_or_default();
                match asking.finish(&part("answer"), &part("sealed")) {
                    Some((key, computer)) => Ok(Paired { address: address.clone(), key, computer, since: laptop_now() }),
                    None => Err("that code did not match: check it on the laptop".to_owned()),
                }
            }
            Ok((403, _)) => Err("the laptop is not showing a code: choose Pair a handheld there".into()),
            Ok((s, text)) => Err(status_trouble(s, &text).words().to_owned()),
            Err(t) => Err(t.words().to_owned()),
        };
        match outcome {
            Ok(p) => pair(p),
            Err(why) => {
                with(|s| s.pairing = Some(Err(why)));
                changed();
            }
        }
    });
}

// --------------------------------------------------------------- stream ---

fn ensure_stream() {
    let start = with(|s| {
        if s.streaming || !s.needed() {
            return false;
        }
        s.streaming = true;
        true
    });
    if start && !demo() {
        std::thread::spawn(stream_loop);
    } else if start {
        with(|s| s.streaming = false);
    }
}

/// While it is needed: the stream opened, read, and opened again when it
/// ends; after trouble, again in 2, 5, 10, then 30 s (sooner if a page
/// asks), and given up 15 minutes on with no page open.
fn stream_loop() {
    let mut failures = 0u32;
    let mut failing_since: Option<Instant> = None;
    loop {
        let go = with(|s| {
            if !s.needed() {
                s.streaming = false;
                return None;
            }
            Some((s.paired.clone()?, s.epoch.clone(), s.seq))
        });
        let Some((p, epoch, after)) = go else { break };
        // The list (and so a place in the events) first, if there is none.
        if epoch.is_empty() {
            refresh_now(&p);
        }
        let (epoch, after) = with(|s| (s.epoch.clone(), s.seq.max(after)));
        let r = stream_once(&p, &epoch, after);
        match r {
            Ok(()) => {
                failures = 0;
                failing_since = None;
            }
            Err(t) => {
                with(|s| s.trouble = Some(t.clone()));
                changed();
                if t == Trouble::NotPaired {
                    with(|s| {
                        s.working.clear();
                        s.approvals.clear();
                        s.tasks.clear();
                        s.streaming = false;
                    });
                    changed();
                    break;
                }
                failures += 1;
                let since = *failing_since.get_or_insert_with(Instant::now);
                // With no page open, a reply that cannot come is let go.
                let open = with(|s| s.holds > 0);
                if !open && since.elapsed() > Duration::from_secs(15 * 60) {
                    with(|s| {
                        s.working.clear();
                        s.approvals.clear();
                        s.tasks.clear();
                        s.streaming = false;
                    });
                    changed();
                    break;
                }
                rest(Duration::from_secs([2, 5, 10, 30][(failures as usize - 1).min(3)]));
            }
        }
    }
}

fn refresh_now(p: &Paired) {
    if let Ok(v) = request_to(&p.address, &p.key, &json!({"op": "sessions"})) {
        with(|s| take_place(s, &v));
    }
}

/// One stream: events from `after` until it ends.
fn stream_once(p: &Paired, epoch: &str, after: u64) -> Result<(), Trouble> {
    let op = json!({"op": "events", "after": after, "epoch": epoch, "name": handheld_name()});
    let (rid, body) = p.key.seal_request(&op, now_ms()).ok_or(Trouble::Other("no random numbers".into()))?;
    let url = format!("{}{}", p.address, proto::PATH);
    let mut child = crate::tools::quiet("curl")
        .args(["-sS", "-N", "--proto", "=https", "--max-time", "600", "--speed-time", "45", "--speed-limit", "1", "-H", "Content-Type: text/plain", "--data-binary", "@-", "-w", "\n%{http_code}\n", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Trouble::Other("curl could not start".into()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.as_bytes()).ok();
    }
    let stdout = child.stdout.take();
    *lock(&STREAM) = Some(child);
    with(|s| s.asking = true);
    changed();
    let mut index = 0u64;
    let mut last = String::new();
    let mut got_any = false;
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if let Some(data) = line.strip_prefix("data: ") {
                let Some(ev) = p.key.open_event(&rid, index, data) else { break };
                index += 1;
                if !got_any {
                    got_any = true;
                    with(|s| {
                        s.trouble = None;
                        s.asking = false;
                    });
                }
                apply(&ev);
            } else if !line.trim().is_empty() {
                last = line.trim().to_owned();
            }
        }
    }
    with(|s| s.asking = false);
    let child = lock(&STREAM).take();
    let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
    let status: u16 = last.parse().unwrap_or(0);
    match (code, status) {
        // Stopped here (no longer needed), or ended by the bridge: fine.
        (None, _) | (Some(0), 200) => Ok(()),
        (Some(0), s) => Err(status_trouble(s, "")),
        (Some(c), _) => Err(curl_trouble(Some(c))),
    }
}

// ---------------------------------------------------------------- events ---

/// One event from the bridge, taken in.
pub fn apply(ev: &Value) {
    let session = ev["session"].as_str().unwrap_or_default().to_owned();
    let seq = ev["seq"].as_u64().unwrap_or(0);
    let epoch = ev["epoch"].as_str().unwrap_or_default().to_owned();
    let mut posts: Vec<crate::notify::Notification> = Vec::new();
    let mut reset = false;
    with(|s| {
        if ev["ev"] == "reset" {
            s.epoch = epoch.clone();
            s.seq = seq;
            reset = true;
            return;
        }
        s.epoch = epoch.clone();
        s.seq = s.seq.max(seq);
        // For the conversation open on it, if its history is older.
        let open = s.convo.as_mut().filter(|c| c.session.as_deref() == Some(session.as_str()) && c.loaded && (c.since.0 != epoch || seq > c.since.1));
        match ev["ev"].as_str() {
            Some("row") => {
                let row = Row::from_json(&ev["row"]);
                if row.kind == "claude" && !row.text.trim().is_empty() {
                    s.last_words.insert(session.clone(), row.text.clone());
                }
                if let Some(c) = open {
                    put_row(c, row, ev["row"]["replaces"].as_str());
                }
            }
            Some("append") => {
                if let Some(c) = open {
                    let id = ev["id"].as_str().unwrap_or_default();
                    let text = ev["text"].as_str().unwrap_or_default();
                    match c.rows.iter_mut().find(|r| r.id == id) {
                        Some(r) => {
                            r.text.push_str(text);
                            r.rev = rev();
                        }
                        None => c.rows.push(Row { id: id.to_owned(), kind: "claude".into(), text: text.to_owned(), rev: rev(), ..Row::default() }),
                    }
                }
            }
            Some("turn") => {
                let state = ev["state"].as_str().unwrap_or_default();
                let words = ev["words"].as_str().unwrap_or_default().to_owned();
                let looking = s.holds > 0 && s.convo.as_ref().is_some_and(|c| c.session.as_deref() == Some(session.as_str()));
                if let Some(c) = s.convo.as_mut().filter(|c| c.session.as_deref() == Some(session.as_str())) {
                    c.status = if state == "working" { words.clone() } else { String::new() };
                }
                if state == "working" {
                    let project = s.sessions.iter().flatten().find(|x| x.id == session).map(|x| x.project.clone()).or_else(|| s.convo.as_ref().filter(|c| c.session.as_deref() == Some(session.as_str())).map(|c| c.project.clone())).unwrap_or_default();
                    s.working.insert(session.clone(), project.clone());
                    s.tasks.entry(session.clone()).or_insert_with(|| crate::liveactivity::task(crate::liveactivity::Kind::Claude, "Claude", working_words(&project)));
                } else {
                    let project = s.working.remove(&session).unwrap_or_default();
                    s.approvals.retain(|a| a.session != session);
                    let (title, body) = match state {
                        "done" => ("reply ready".to_owned(), s.last_words.get(&session).map(|w| super::ui::first_words(w)).unwrap_or_default()),
                        "stopped" => ("stopped".to_owned(), project.clone()),
                        _ => ("could not finish".to_owned(), words.clone()),
                    };
                    if let Some(task) = s.tasks.remove(&session) {
                        task.done(title.clone());
                    }
                    if let Some(c) = s.convo.as_mut().filter(|c| c.session.as_deref() == Some(session.as_str())) {
                        if state == "error" && !words.is_empty() {
                            c.error = Some(words.clone());
                        }
                    }
                    if !looking && state != "stopped" {
                        posts.push(crate::notify::Notification::new("Claude", super::ICON, title).body(if body.is_empty() { project } else { body }).tag(format!("claude-turn-{session}")).action("builtin:Claude"));
                    }
                }
            }
            Some("approval") => {
                let a = Approval::from_json(&session, &ev["approval"]);
                let project = s.working.get(&session).cloned().unwrap_or_default();
                if let Some(t) = s.tasks.get(&session) {
                    t.detail(format!("approval waiting · {project}"));
                }
                let looking = s.holds > 0 && s.convo.as_ref().is_some_and(|c| c.session.as_deref() == Some(session.as_str()));
                if !looking {
                    posts.push(crate::notify::Notification::new("Claude", super::ICON, "approval waiting").body(format!("{} · {}", a.what, super::ui::first_words(&a.line))).tag(format!("claude-ask-{}", a.id)).urgent().action("builtin:Claude"));
                }
                if !s.approvals.iter().any(|x| x.id == a.id) {
                    s.approvals.push(a);
                }
            }
            Some("approval_done") => {
                let id = ev["id"].as_str().unwrap_or_default();
                s.approvals.retain(|a| a.id != id);
                if let Some(t) = s.tasks.get(&session) {
                    let project = s.working.get(&session).cloned().unwrap_or_default();
                    t.detail(working_words(&project));
                }
            }
            _ => {}
        }
    });
    if reset {
        refresh();
        if let Some(c) = with(|s| s.convo.clone()) {
            if let Some(id) = c.session {
                open(&Session { id, title: c.title, project: c.project, cwd: c.cwd, last: 0, mode: c.mode });
            }
        }
    }
    for n in posts {
        crate::notify::post(n);
    }
    changed();
    settle();
}

fn working_words(project: &str) -> String {
    if project.is_empty() {
        "working".into()
    } else {
        format!("working · {project}")
    }
}

/// A row into the conversation: in place of the one with its id (or the
/// streaming row it `replaces`), or of the same words already there (the
/// history read while it was being written), else at the end.
fn put_row(c: &mut Convo, row: Row, replaces: Option<&str>) {
    let at = c.rows.iter().position(|r| r.id == row.id || replaces.is_some_and(|x| r.id == x)).or_else(|| (row.kind == "claude").then(|| c.rows.iter().position(|r| r.kind == "claude" && r.text.trim() == row.text.trim())).flatten());
    match at {
        Some(i) => c.rows[i] = row,
        None => c.rows.push(row),
    }
}

impl Default for Row {
    fn default() -> Row {
        Row { id: String::new(), kind: String::new(), text: String::new(), icon: String::new(), state: String::new(), detail: String::new(), rev: 0 }
    }
}

#[cfg(test)]
pub(super) fn put_row_for_test(c: &mut Convo, row: Row, replaces: Option<&str>) {
    put_row(c, row, replaces);
}
