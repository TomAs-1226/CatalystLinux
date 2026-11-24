//! The bridge: Claude Code on this computer, worked from the handheld.
//!
//! The handheld never talks to claude.ai, and nothing of the account ever
//! reaches it. It talks only to this bridge, which drives the `claude` CLI
//! headless ([`runner`]) and sends back text: the sessions ([`sessions`]),
//! a conversation's rows, what a turn writes as it writes it ([`stream`]),
//! and each permission request ([`approve`]), which the handheld allows or
//! denies.
//!
//! - **Where.** 127.0.0.1:[`PORT`] only, which Tailscale Funnel publishes
//!   at the computer's fixed `*.ts.net` address ([`tailscale`]); the
//!   permission tool's way in is a second port Funnel never sees.
//! - **Who.** Requests open only with the paired key (sealed end to end,
//!   companion-proto's [`claude`](catalyst_companion_proto::claude)), each
//!   once, within five minutes of this computer's clock. Anything else is
//!   refused before it is read. Pairing again or unpairing revokes the key.
//! - **What Claude may do.** Every request Claude Code would ask a person
//!   about (a command, a file outside the project, the web) goes to the
//!   handheld; unanswered for [`APPROVAL_SECONDS`] it is denied. A session
//!   asks for everything or accepts edits in its folder, nothing more.
//! - **Power.** Nothing runs between requests: no polling, no timers. A
//!   stream of events is held only while the handheld holds it.

pub mod approve;
pub mod http;
pub mod runner;
pub mod sessions;
pub mod setup;
pub mod store;
pub mod stream;
pub mod summary;
pub mod system;
pub mod tailscale;

use std::collections::{HashMap, VecDeque};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use catalyst_companion_proto::claude::{self as proto, Invite, Key, APPROVAL_SECONDS, APPROVER_PORT, PORT};
use serde_json::{json, Value};

use runner::{Launch, Mode, Running};
use sessions::Session;
use store::Paired;

/// Events kept for a handheld that comes back, newest last.
const EVENTS_KEPT: usize = 4000;
/// How long an event stream stays open with nothing to say, and how often
/// it says it is still there meanwhile.
const STREAM_IDLE: Duration = Duration::from_secs(90);
const KEEPALIVE: Duration = Duration::from_secs(15);
/// Words written into a row are sent together, this often at most.
const APPEND_EVERY: Duration = Duration::from_millis(120);
/// A session written to this recently, and not by the bridge, is being
/// worked in on the computer: the handheld does not write into it.
const IN_USE: Duration = Duration::from_secs(30);
/// Turns at once, across every session.
const TURNS_MOST: usize = 4;
/// Connections at once.
const CONNECTIONS_MOST: usize = 32;
/// A message's longest.
const MESSAGE_MOST: usize = 20_000;
/// A pairing code's life and tries (Companion's).
const CODE_SECONDS: u64 = catalyst_companion_proto::CODE_LIFETIME;
const CODE_TRIES: u32 = catalyst_companion_proto::TRIES;
/// How long a new pairing's key waits for the handheld's first request.
const PENDING_SECONDS: u64 = 30 * 60;

/// How the bridge is set up.
#[derive(Clone)]
pub struct Config {
    /// Its own folder: the pairing, and each turn's MCP config.
    pub state_dir: PathBuf,
    /// Claude Code's transcripts (`~/.claude/projects`).
    pub projects_dir: PathBuf,
    /// The `claude` executable.
    pub claude: PathBuf,
    /// The permission tool, as the MCP config names it: a program and its
    /// arguments (`catalyst-companion.exe --claude-approver`).
    pub approver: (PathBuf, Vec<String>),
    pub port: u16,
    pub approver_port: u16,
    /// This computer's name, as the handheld shows it.
    pub computer: String,
    pub approval_seconds: u64,
    /// Keep the key encrypted to the Windows account.
    pub protect: bool,
}

impl Config {
    /// As Companion runs it: its folder in `%APPDATA%`, the transcripts
    /// where Claude Code keeps them, the claude it finds.
    pub fn standard(state_dir: PathBuf, approver: (PathBuf, Vec<String>), computer: String) -> Result<Config, String> {
        let claude = runner::find_claude().ok_or("Claude Code is not installed: install it, run claude once and sign in")?;
        Ok(Config { state_dir, projects_dir: sessions::projects_dir(), claude, approver, port: PORT, approver_port: APPROVER_PORT, computer, approval_seconds: APPROVAL_SECONDS, protect: cfg!(windows) })
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn now_s() -> i64 {
    (now_ms() / 1000) as i64
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    catalyst_companion_proto::seal::random(&mut b);
    catalyst_companion_proto::hex(&b)
}

/// A random UUID (version 4), for a new session's id.
pub fn new_uuid() -> String {
    let mut b = [0u8; 16];
    catalyst_companion_proto::seal::random(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = catalyst_companion_proto::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

// ---------------------------------------------------------------- pairing ---

/// A pairing being made: its key (the QR code's), the code shown with it,
/// and the code's tries.
struct Offer {
    key: Key,
    made: Instant,
    code: String,
    tries: u32,
}

#[derive(Default)]
struct Pairing {
    paired: Option<Paired>,
    offer: Option<Offer>,
}

/// How pairing stands, for the computer's own window.
#[derive(Clone, Debug, PartialEq)]
pub struct PairState {
    /// The handheld paired, by name.
    pub paired: Option<String>,
    /// A pairing waiting for the handheld: the code's seconds left.
    pub offering: Option<u64>,
}

fn new_code() -> String {
    loop {
        let mut b = [0u8; 4];
        catalyst_companion_proto::seal::random(&mut b);
        let n = u32::from_le_bytes(b);
        if n < u32::MAX - u32::MAX % 1_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

// ----------------------------------------------------------------- events ---

struct EventLog {
    /// Which run of the bridge: a handheld's place in another run's log
    /// means nothing here.
    epoch: String,
    next: u64,
    kept: VecDeque<Value>,
}

impl EventLog {
    /// Events after `after`, or None if some were let go meanwhile (or the
    /// place is another run's): the handheld reads the history again.
    fn since(&self, epoch: &str, after: u64) -> Option<Vec<Value>> {
        if epoch != self.epoch || after >= self.next {
            return None;
        }
        let oldest = self.kept.front().and_then(|e| e["seq"].as_u64()).unwrap_or(self.next);
        if after + 1 < oldest {
            return None;
        }
        Some(self.kept.iter().filter(|e| e["seq"].as_u64().is_some_and(|s| s > after)).cloned().collect())
    }
}

// -------------------------------------------------------------- approvals ---

/// A permission request waiting on the handheld.
pub struct Approval {
    pub id: String,
    pub session: String,
    turn: String,
    pub tool_use_id: String,
    pub asked: summary::Asked,
    pub tool: String,
    pub until: i64,
    answer: Option<bool>,
}

impl Approval {
    fn json(&self) -> Value {
        json!({"id": self.id, "session": self.session, "tool": self.tool, "what": self.asked.what, "line": self.asked.line, "where": self.asked.place, "detail": self.asked.detail, "until": self.until})
    }
}

// ------------------------------------------------------------------ turns ---

struct TurnState {
    id: String,
    session: String,
    cwd: String,
    ticket: String,
    running: Option<Running>,
    stopping: bool,
    mcp_file: PathBuf,
}

// ----------------------------------------------------------------- bridge ---

pub(crate) struct Shared {
    pub(crate) config: Config,
    pairing: Mutex<Pairing>,
    seen: Mutex<HashMap<String, u64>>,
    events: Mutex<EventLog>,
    events_moved: Condvar,
    cache: Mutex<sessions::Cache>,
    modes: Mutex<HashMap<String, Mode>>,
    /// When each session's last turn from here ended: its transcript was
    /// written by the bridge, not by someone at the computer.
    ended: Mutex<HashMap<String, Instant>>,
    turns: Mutex<Vec<TurnState>>,
    pub(crate) approvals: Mutex<Vec<Approval>>,
    pub(crate) answered: Condvar,
    /// Tool calls not allowed, by turn, for their rows to say so.
    denied: Mutex<Vec<(String, String)>>,
    next_approval: AtomicU64,
    connections: AtomicUsize,
    changed: Mutex<Option<Box<dyn Fn() + Send>>>,
    stop: AtomicBool,
}

/// The bridge, running.
#[derive(Clone)]
pub struct Bridge {
    shared: Arc<Shared>,
}

impl Bridge {
    /// Start it: both ports bound (or an error saying which is taken), the
    /// pairing read back.
    pub fn start(config: Config) -> Result<Bridge, String> {
        std::fs::create_dir_all(config.state_dir.join("run")).map_err(|e| format!("its folder: {e}"))?;
        // A turn's leftover config from a run that ended badly holds a
        // dead ticket: gone.
        for e in std::fs::read_dir(config.state_dir.join("run")).into_iter().flatten().flatten() {
            std::fs::remove_file(e.path()).ok();
        }
        let public = TcpListener::bind(("127.0.0.1", config.port)).map_err(|e| format!("port {} is taken ({e}): is the bridge running already?", config.port))?;
        let local = TcpListener::bind(("127.0.0.1", config.approver_port)).map_err(|e| format!("port {} is taken ({e})", config.approver_port))?;
        let paired = store::load(&config.state_dir, config.protect);
        let shared = Arc::new(Shared {
            pairing: Mutex::new(Pairing { paired, offer: None }),
            seen: Mutex::new(HashMap::new()),
            events: Mutex::new(EventLog { epoch: random_hex(8), next: 1, kept: VecDeque::new() }),
            events_moved: Condvar::new(),
            cache: Mutex::new(sessions::Cache::default()),
            modes: Mutex::new(HashMap::new()),
            ended: Mutex::new(HashMap::new()),
            turns: Mutex::new(Vec::new()),
            approvals: Mutex::new(Vec::new()),
            answered: Condvar::new(),
            denied: Mutex::new(Vec::new()),
            next_approval: AtomicU64::new(1),
            connections: AtomicUsize::new(0),
            changed: Mutex::new(None),
            stop: AtomicBool::new(false),
            config,
        });
        for (listener, local_only) in [(public, false), (local, true)] {
            let s = shared.clone();
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if s.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    if s.connections.load(Ordering::Relaxed) >= CONNECTIONS_MOST {
                        http::text(&conn, 429, "busy");
                        continue;
                    }
                    let s = s.clone();
                    s.connections.fetch_add(1, Ordering::Relaxed);
                    std::thread::spawn(move || {
                        if local_only {
                            approve::serve(&s, &conn);
                        } else {
                            serve(&s, &conn);
                        }
                        s.connections.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            });
        }
        Ok(Bridge { shared })
    }

    /// Called whenever pairing changes (a handheld paired), from any thread.
    pub fn on_change(&self, f: impl Fn() + Send + 'static) {
        *lock(&self.shared.changed) = Some(Box::new(f));
    }

    /// Begin pairing: a new key for the QR code at `address`, and a code.
    /// The handheld paired before stays paired until the new one's first
    /// request, which revokes it.
    pub fn begin_pairing(&self, address: &str) -> Result<(Invite, String), String> {
        let address = proto::clean_address(address).ok_or("that is not an https address")?;
        let key = Key::random().ok_or("no random numbers")?;
        let code = new_code();
        lock(&self.shared.pairing).offer = Some(Offer { key: key.clone(), made: Instant::now(), code: code.clone(), tries: 0 });
        Ok((Invite { address, key, computer: self.shared.config.computer.clone() }, code))
    }

    /// Stop offering the pairing shown (its window closed).
    pub fn end_pairing(&self) {
        let mut p = lock(&self.shared.pairing);
        // Kept a while for a handheld that read the code but has not yet
        // reached the bridge; only the code itself stops working.
        if let Some(o) = p.offer.as_mut() {
            o.tries = CODE_TRIES;
        }
    }

    pub fn pair_state(&self) -> PairState {
        let p = lock(&self.shared.pairing);
        let offering = p.offer.as_ref().filter(|o| o.tries < CODE_TRIES).map(|o| CODE_SECONDS.saturating_sub(o.made.elapsed().as_secs())).filter(|s| *s > 0);
        PairState { paired: p.paired.as_ref().map(|x| x.handheld.clone()), offering }
    }

    /// Forget the handheld: its key opens nothing from now on.
    pub fn unpair(&self) {
        self.shared.unpair();
    }

    /// Turns running and requests waiting, for the tray's tooltip.
    pub fn busy(&self) -> (usize, usize) {
        (lock(&self.shared.turns).len(), lock(&self.shared.approvals).iter().filter(|a| a.answer.is_none()).count())
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in lock(&self.shared.turns).iter() {
            if let Some(r) = &t.running {
                r.stop();
            }
        }
    }
}

impl Shared {
    fn unpair(&self) {
        {
            let mut p = lock(&self.pairing);
            p.paired = None;
            p.offer = None;
        }
        store::forget(&self.config.state_dir);
        self.changed();
    }

    fn changed(&self) {
        if let Some(f) = lock(&self.changed).as_ref() {
            f();
        }
    }

    pub(crate) fn push(&self, mut ev: Value) {
        let mut log = lock(&self.events);
        ev["seq"] = Value::from(log.next);
        ev["epoch"] = Value::from(log.epoch.clone());
        log.next += 1;
        log.kept.push_back(ev);
        while log.kept.len() > EVENTS_KEPT {
            log.kept.pop_front();
        }
        drop(log);
        self.events_moved.notify_all();
    }

    fn seq(&self) -> (String, u64) {
        let log = lock(&self.events);
        (log.epoch.clone(), log.next - 1)
    }

    /// The key a request's id names: the paired one, or the one a pairing
    /// is offering (which then becomes the paired one).
    fn key_for(&self, id: &[u8; 8]) -> Option<(Key, bool)> {
        let p = lock(&self.pairing);
        if let Some(paired) = p.paired.as_ref().filter(|x| x.key.id == *id) {
            return Some((paired.key.clone(), false));
        }
        p.offer.as_ref().filter(|o| o.key.id == *id && o.made.elapsed().as_secs() < PENDING_SECONDS).map(|o| (o.key.clone(), true))
    }

    /// The offered key's first request: it is the paired one now, and the
    /// one before it is revoked.
    fn complete_pairing(&self, key: &Key, handheld: &str) {
        let paired = Paired { key: key.clone(), handheld: if handheld.is_empty() { "handheld".into() } else { proto::clean_name(handheld) }, since: now_s() };
        {
            let mut p = lock(&self.pairing);
            if p.offer.as_ref().is_some_and(|o| o.key == *key) {
                p.offer = None;
                p.paired = Some(paired.clone());
            }
        }
        store::save(&self.config.state_dir, self.config.protect, &paired).ok();
        self.changed();
    }

    /// Each request once, and only near this computer's time.
    fn fresh(&self, rid: &str, t: u64) -> Result<(), &'static str> {
        let now = now_ms();
        if now.abs_diff(t) > proto::REPLAY_WINDOW_MS {
            return Err("clock");
        }
        let mut seen = lock(&self.seen);
        seen.retain(|_, at| now.saturating_sub(*at) < 2 * proto::REPLAY_WINDOW_MS);
        if seen.insert(rid.to_owned(), now).is_some() {
            return Err("seen");
        }
        Ok(())
    }

    fn sessions(&self) -> Vec<Session> {
        sessions::list(&self.config.projects_dir, &mut lock(&self.cache))
    }

    fn find(&self, id: &str) -> Option<Session> {
        sessions::find(&self.config.projects_dir, &mut lock(&self.cache), id)
    }

    fn mode(&self, session: &str) -> Mode {
        lock(&self.modes).get(session).copied().unwrap_or(Mode::Ask)
    }

    fn running(&self, session: &str) -> bool {
        lock(&self.turns).iter().any(|t| t.session == session)
    }

    /// A ticket's turn: its session and folder.
    pub(crate) fn turn_of_ticket(&self, ticket: &str) -> Option<(String, String, String)> {
        lock(&self.turns).iter().find(|t| catalyst_companion_proto::same(t.ticket.as_bytes(), ticket.as_bytes())).map(|t| (t.id.clone(), t.session.clone(), t.cwd.clone()))
    }

    pub(crate) fn turn_alive(&self, turn: &str) -> bool {
        lock(&self.turns).iter().any(|t| t.id == turn && !t.stopping)
    }

    pub(crate) fn new_approval(&self, turn: &str, session: &str, tool: &str, tool_use_id: &str, asked: summary::Asked) -> String {
        let id = format!("a{}", self.next_approval.fetch_add(1, Ordering::Relaxed));
        let a = Approval { id: id.clone(), session: session.to_owned(), turn: turn.to_owned(), tool_use_id: tool_use_id.to_owned(), asked, tool: tool.to_owned(), until: now_s() + self.config.approval_seconds as i64, answer: None };
        let ev = json!({"ev": "approval", "session": session, "approval": a.json()});
        lock(&self.approvals).push(a);
        self.push(ev);
        id
    }

    /// The handheld's answer.
    fn answer(&self, id: &str, allow: bool) -> bool {
        let mut list = lock(&self.approvals);
        let Some(a) = list.iter_mut().find(|a| a.id == id && a.answer.is_none()) else { return false };
        a.answer = Some(allow);
        drop(list);
        self.answered.notify_all();
        true
    }

    /// Wait for an approval's answer until its time is up or its turn ends;
    /// it is taken off the list either way.
    pub(crate) fn wait_answer(&self, id: &str) -> (bool, &'static str) {
        let deadline = Instant::now() + Duration::from_secs(self.config.approval_seconds);
        let mut list = lock(&self.approvals);
        let outcome = loop {
            let Some(a) = list.iter().find(|a| a.id == id) else { break (false, "gone") };
            if let Some(allow) = a.answer {
                break (allow, "answered");
            }
            let turn = a.turn.clone();
            let now = Instant::now();
            if now >= deadline {
                break (false, "timeout");
            }
            if !self.turn_alive(&turn) {
                break (false, "stopped");
            }
            list = self.answered.wait_timeout(list, (deadline - now).min(Duration::from_secs(1))).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        };
        let session = list.iter().find(|a| a.id == id).map(|a| a.session.clone()).unwrap_or_default();
        list.retain(|a| a.id != id);
        drop(list);
        self.push(json!({"ev": "approval_done", "session": session, "id": id, "allowed": outcome.0, "why": outcome.1}));
        outcome
    }

    /// A tool call not allowed: its row will say so.
    pub(crate) fn denied(&self, turn: &str, tool_use_id: &str) {
        lock(&self.denied).push((turn.to_owned(), tool_use_id.to_owned()));
    }
}

// --------------------------------------------------------------- requests ---

fn serve(s: &Arc<Shared>, conn: &TcpStream) {
    let Some(req) = http::read(conn) else {
        http::text(conn, 400, "no");
        return;
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", proto::PATH) => sealed(s, conn, &req.body),
        ("POST", proto::PAIR_PATH) => pair_by_code(s, conn, &req.body),
        _ => http::text(conn, 404, "not here"),
    }
}

/// A six-digit code tried.
fn pair_by_code(s: &Arc<Shared>, conn: &TcpStream, body: &[u8]) {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let Some(ask) = v["ask"].as_str().and_then(catalyst_companion_proto::unhex) else {
        return http::text(conn, 400, "no ask");
    };
    let mut p = lock(&s.pairing);
    let Some(offer) = p.offer.as_mut().filter(|o| o.tries < CODE_TRIES && o.made.elapsed().as_secs() < CODE_SECONDS) else {
        drop(p);
        return http::text(conn, 403, "no code is showing on the computer");
    };
    offer.tries += 1;
    let answer = proto::pairing::answer(&offer.code, &ask, &offer.key, &s.config.computer);
    drop(p);
    match answer {
        Ok((answer, sealed)) => http::reply(conn, 200, "application/json", json!({"answer": catalyst_companion_proto::hex(&answer), "sealed": catalyst_companion_proto::hex(&sealed)}).to_string().as_bytes()),
        Err(_) => http::text(conn, 400, "not a pairing message"),
    }
}

/// A sealed request: refused unless it opens with the paired key.
fn sealed(s: &Arc<Shared>, conn: &TcpStream, body: &[u8]) {
    let body = String::from_utf8_lossy(body);
    let Some((id, sealed_part)) = proto::request_id(&body) else {
        return http::text(conn, 401, "not paired");
    };
    let Some((key, pending)) = s.key_for(&id) else {
        return http::text(conn, 401, "not paired");
    };
    let Some(req) = key.open_request(sealed_part) else {
        return http::text(conn, 401, "not paired");
    };
    let rid = req["rid"].as_str().unwrap_or_default().to_owned();
    let reply = |v: Value| match key.seal_reply(&rid, &v) {
        Some(text) => http::reply(conn, 200, "text/plain", text.as_bytes()),
        None => http::text(conn, 400, "no"),
    };
    match s.fresh(&rid, req["t"].as_u64().unwrap_or(0)) {
        Ok(()) => {}
        // The handheld's clock is off: it is told this one's, sealed.
        Err("clock") => return reply(json!({"error": "clock", "now": now_ms()})),
        Err(_) => return http::text(conn, 409, "seen"),
    }
    if pending {
        s.complete_pairing(&key, req["name"].as_str().unwrap_or("handheld"));
    }
    let op = req["op"].as_str().unwrap_or_default();
    if op == "events" {
        return events(s, conn, &key, &rid, &req);
    }
    reply(handle(s, op, &req));
}

/// One request's answer.
fn handle(s: &Arc<Shared>, op: &str, req: &Value) -> Value {
    match op {
        "hello" => json!({"computer": s.config.computer, "ok": true}),
        "sessions" => {
            let list: Vec<Value> = s
                .sessions()
                .iter()
                .take(60)
                .map(|x| {
                    let mut v = x.json();
                    v["running"] = Value::from(s.running(&x.id));
                    v["mode"] = Value::from(s.mode(&x.id).word());
                    v
                })
                .collect();
            let (epoch, seq) = s.seq();
            json!({"sessions": list, "epoch": epoch, "seq": seq, "now": now_s()})
        }
        "projects" => {
            let list: Vec<Value> = sessions::projects(&s.sessions()).iter().take(30).map(|(cwd, name, last)| json!({"cwd": cwd, "name": name, "last": last})).collect();
            json!({"projects": list})
        }
        "history" => {
            let id = req["session"].as_str().unwrap_or_default();
            let (epoch, seq) = s.seq();
            let Some(session) = s.find(id) else { return json!({"error": "that session is not on the computer any more"}) };
            let waiting: Vec<Value> = lock(&s.approvals).iter().filter(|a| a.session == id && a.answer.is_none()).map(Approval::json).collect();
            json!({"session": session.json(), "rows": sessions::history(&session, 120), "running": s.running(id), "mode": s.mode(id).word(), "approvals": waiting, "epoch": epoch, "seq": seq})
        }
        "mode" => {
            let id = req["session"].as_str().unwrap_or_default();
            match Mode::from_word(req["mode"].as_str().unwrap_or_default()) {
                Some(m) if sessions::is_session_id(id) => {
                    lock(&s.modes).insert(id.to_owned(), m);
                    json!({"ok": true, "mode": m.word()})
                }
                _ => json!({"error": "a session asks for everything or accepts edits, nothing else"}),
            }
        }
        "send" => send(s, req),
        "interrupt" => {
            let id = req["session"].as_str().unwrap_or_default();
            let mut turns = lock(&s.turns);
            match turns.iter_mut().find(|t| t.session == id) {
                Some(t) => {
                    t.stopping = true;
                    if let Some(r) = &t.running {
                        r.stop();
                    }
                    drop(turns);
                    s.answered.notify_all();
                    json!({"ok": true})
                }
                None => json!({"ok": false, "error": "Claude is not working on it"}),
            }
        }
        "answer" => {
            let ok = s.answer(req["approval"].as_str().unwrap_or_default(), req["allow"] == true);
            json!({"ok": ok})
        }
        "unpair" => {
            s.unpair();
            json!({"ok": true})
        }
        _ => json!({"error": "not something the bridge does"}),
    }
}

/// A message to Claude: a session carried on, or a new one begun in one
/// of the folders sessions have worked in.
fn send(s: &Arc<Shared>, req: &Value) -> Value {
    let text = req["text"].as_str().unwrap_or_default().trim();
    if text.is_empty() {
        return json!({"error": "nothing to send"});
    }
    if text.chars().count() > MESSAGE_MOST {
        return json!({"error": "that message is too long"});
    }
    let mode = req["mode"].as_str().and_then(Mode::from_word);
    let (session, cwd, resume) = match req["session"].as_str() {
        Some(id) => {
            let Some(found) = s.find(id) else { return json!({"error": "that session is not on the computer any more"}) };
            if s.running(id) {
                return json!({"error": "busy", "words": "Claude is still working on it"});
            }
            // Someone at the computer is in it.
            let written = std::fs::metadata(&found.path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            let ours = lock(&s.ended).get(id).is_some_and(|e| e.elapsed() < IN_USE + Duration::from_secs(5));
            if written.is_some_and(|w| w < IN_USE) && !ours {
                return json!({"error": "in use", "words": "it is being worked in on the computer right now"});
            }
            (found.id, found.cwd, true)
        }
        None => {
            let asked = req["cwd"].as_str().unwrap_or_default();
            let known = sessions::projects(&s.sessions()).into_iter().find(|(c, ..)| c.eq_ignore_ascii_case(asked));
            let Some((cwd, ..)) = known else { return json!({"error": "a new session begins in a folder Claude Code has worked in"}) };
            (new_uuid(), cwd, false)
        }
    };
    if !Path::new(&cwd).is_dir() {
        return json!({"error": "its folder is not on the computer any more"});
    }
    if lock(&s.turns).len() >= TURNS_MOST {
        return json!({"error": "busy", "words": "Claude is already working in several sessions"});
    }
    if let Some(m) = mode {
        lock(&s.modes).insert(session.clone(), m);
    }
    let mode = s.mode(&session);
    match start_turn(s, &session, &cwd, resume, mode, text) {
        Ok(turn) => json!({"ok": true, "session": session, "turn": turn, "mode": mode.word()}),
        Err(why) => json!({"error": why}),
    }
}

fn start_turn(s: &Arc<Shared>, session: &str, cwd: &str, resume: bool, mode: Mode, text: &str) -> Result<String, String> {
    let turn = random_hex(8);
    let ticket = random_hex(32);
    let mcp_file = s.config.state_dir.join("run").join(format!("{turn}.json"));
    let config = runner::mcp_config(&s.config.approver.0, &s.config.approver.1, s.config.approver_port, &ticket);
    std::fs::write(&mcp_file, config.to_string()).map_err(|_| "the bridge could not write its permission tool's settings".to_owned())?;
    store::restrict(&mcp_file);
    // Installed after the bridge started (Set up installs it): found now.
    let claude = if s.config.claude.is_file() { s.config.claude.clone() } else { runner::find_claude().unwrap_or_else(|| s.config.claude.clone()) };
    let launch = Launch { claude, cwd: PathBuf::from(cwd), session: session.to_owned(), resume, mode, mcp_config: mcp_file.clone() };
    lock(&s.turns).push(TurnState { id: turn.clone(), session: session.to_owned(), cwd: cwd.to_owned(), ticket, running: None, stopping: false, mcp_file: mcp_file.clone() });
    let you = format!("you-{turn}");
    s.push(json!({"ev": "row", "session": session, "row": sessions::row(&you, "you", &summary::cut(text, summary::DETAIL_MOST))}));
    s.push(json!({"ev": "turn", "session": session, "state": "working", "words": "", "new": !resume}));
    // Lines go to the turn's own thread, which sends words written on
    // together every APPEND_EVERY at most.
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let tx_exit = tx.clone();
    let on_line = move |line: &str| {
        let _ = tx.send(Some(line.to_owned()));
    };
    let exit: Arc<Mutex<Option<(Option<i32>, String)>>> = Arc::new(Mutex::new(None));
    let exit_to = exit.clone();
    let on_exit = move |code: Option<i32>, said: String| {
        *lock(&exit_to) = Some((code, said));
        let _ = tx_exit.send(None);
    };
    let running = match runner::start(&launch, text, on_line, on_exit) {
        Ok(r) => r,
        Err(e) => {
            lock(&s.turns).retain(|t| t.id != turn);
            std::fs::remove_file(&mcp_file).ok();
            s.push(json!({"ev": "turn", "session": session, "state": "error", "words": format!("claude would not start: {e}")}));
            return Err("claude would not start".into());
        }
    };
    if let Some(t) = lock(&s.turns).iter_mut().find(|t| t.id == turn) {
        t.running = Some(running);
    }
    let s2 = s.clone();
    let (turn2, session2, cwd2) = (turn.clone(), session.to_owned(), cwd.to_owned());
    std::thread::spawn(move || follow(&s2, &turn2, &session2, &cwd2, rx, exit));
    Ok(turn)
}

/// A turn's lines, as they come, into events; its end.
fn follow(s: &Arc<Shared>, turn: &str, session: &str, cwd: &str, rx: std::sync::mpsc::Receiver<Option<String>>, exit: Arc<Mutex<Option<(Option<i32>, String)>>>) {
    let mut t = stream::Turn::new(cwd);
    let mut pending: Option<(String, String, Instant)> = None;
    let mut ended: Option<(bool, String)> = None;
    let flush = |pending: &mut Option<(String, String, Instant)>| {
        if let Some((id, text, _)) = pending.take() {
            s.push(json!({"ev": "append", "session": session, "id": id, "text": text}));
        }
    };
    loop {
        let wait = pending.as_ref().map_or(Duration::from_secs(3600), |(.., at)| APPEND_EVERY.saturating_sub(at.elapsed()));
        match rx.recv_timeout(wait) {
            Ok(Some(line)) => {
                lock(&s.denied).retain(|(tn, id)| {
                    if tn == turn {
                        t.denied.insert(id.clone());
                        false
                    } else {
                        true
                    }
                });
                for out in t.line(&line) {
                    match out {
                        stream::Out::Append { id, text } => match pending.as_mut() {
                            Some((pid, ptext, _)) if *pid == id => ptext.push_str(&text),
                            _ => {
                                flush(&mut pending);
                                pending = Some((id, text, Instant::now()));
                            }
                        },
                        other => {
                            flush(&mut pending);
                            match other {
                                stream::Out::Row(row) => s.push(json!({"ev": "row", "session": session, "row": row})),
                                stream::Out::Status(words) => s.push(json!({"ev": "turn", "session": session, "state": "working", "words": words})),
                                stream::Out::Done { ok, words } => ended = Some((ok, words)),
                                stream::Out::Session(_) | stream::Out::Append { .. } => {}
                            }
                        }
                    }
                }
                if pending.as_ref().is_some_and(|(.., at)| at.elapsed() >= APPEND_EVERY) {
                    flush(&mut pending);
                }
            }
            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => flush(&mut pending),
        }
    }
    flush(&mut pending);
    let (code, said) = lock(&exit).take().unwrap_or((None, String::new()));
    let stopping = {
        let mut turns = lock(&s.turns);
        let stopping = turns.iter().find(|x| x.id == turn).is_some_and(|x| x.stopping);
        if let Some(x) = turns.iter().find(|x| x.id == turn) {
            std::fs::remove_file(&x.mcp_file).ok();
        }
        turns.retain(|x| x.id != turn);
        stopping
    };
    s.answered.notify_all();
    lock(&s.ended).insert(session.to_owned(), Instant::now());
    let (state, words) = match ended {
        _ if stopping => {
            s.push(json!({"ev": "row", "session": session, "row": stream::note(&format!("stop-{turn}"), "stopped from the handheld")}));
            ("stopped", String::new())
        }
        Some((true, _)) => ("done", String::new()),
        Some((false, words)) => ("error", words),
        None => {
            let last = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
            ("error", if last.is_empty() { format!("claude stopped ({})", code.map_or("no code".into(), |c| c.to_string())) } else { summary::cut(&last, 160) })
        }
    };
    s.push(json!({"ev": "turn", "session": session, "state": state, "words": words}));
}

/// The stream of events after the handheld's place, held while it holds it.
fn events(s: &Arc<Shared>, conn: &TcpStream, key: &Key, rid: &str, req: &Value) {
    let epoch = req["epoch"].as_str().unwrap_or_default().to_owned();
    let mut after = req["after"].as_u64().unwrap_or(0);
    if !http::events_head(conn) {
        return;
    }
    conn.set_write_timeout(Some(Duration::from_secs(20))).ok();
    let mut index = 0u64;
    let mut send = |v: &Value| -> bool {
        let Some(text) = key.seal_event(rid, index, v) else { return false };
        index += 1;
        http::event(conn, Some(&text))
    };
    let mut idle_since = Instant::now();
    let mut epoch_now = epoch;
    loop {
        if s.stop.load(Ordering::Relaxed) {
            return;
        }
        let batch = {
            let mut log = lock(&s.events);
            loop {
                match log.since(&epoch_now, after) {
                    None => break None,
                    Some(evs) if !evs.is_empty() => break Some(evs),
                    Some(_) => {}
                }
                let waited = idle_since.elapsed();
                if waited >= STREAM_IDLE {
                    return;
                }
                let (g, timeout) = s.events_moved.wait_timeout(log, KEEPALIVE.min(STREAM_IDLE - waited)).unwrap_or_else(|e| e.into_inner());
                log = g;
                if timeout.timed_out() {
                    drop(log);
                    if !http::event(conn, None) {
                        return;
                    }
                    log = lock(&s.events);
                }
            }
        };
        match batch {
            // The handheld's place is gone (or another run's): it starts again.
            None => {
                let (e, seq) = s.seq();
                if !send(&json!({"ev": "reset", "epoch": e, "seq": seq})) {
                    return;
                }
                epoch_now = e;
                after = seq;
            }
            Some(evs) => {
                for ev in &evs {
                    if !send(ev) {
                        return;
                    }
                    after = ev["seq"].as_u64().unwrap_or(after);
                }
                idle_since = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_is_a_session_id() {
        let u = new_uuid();
        assert!(sessions::is_session_id(&u), "{u}");
        assert_eq!(&u[14..15], "4");
        assert_ne!(new_uuid(), u);
    }

    #[test]
    fn the_log_says_when_a_place_is_lost() {
        let mut log = EventLog { epoch: "e".into(), next: 1, kept: VecDeque::new() };
        assert_eq!(log.since("e", 0), Some(vec![]), "nothing yet");
        for seq in 1..=5u64 {
            log.kept.push_back(json!({"seq": seq}));
            log.next = seq + 1;
        }
        assert_eq!(log.since("e", 3).map(|v| v.len()), Some(2));
        assert_eq!(log.since("e", 5), Some(vec![]));
        assert_eq!(log.since("other", 3), None, "another run's place");
        assert_eq!(log.since("e", 9), None, "a place past the end");
        log.kept.pop_front();
        log.kept.pop_front();
        assert_eq!(log.since("e", 0), None, "events let go");
        assert_eq!(log.since("e", 2).map(|v| v.len()), Some(3));
    }
}
