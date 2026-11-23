//! Team: messages between Catalyst handhelds at an event, for the pit
//! crew, the drive team and the scouts, over the pit's Wi-Fi or a phone's
//! hotspot, with no server and no internet.
//!
//! - **Joining** (join.rs, crypto.rs). One handheld makes a team space: a
//!   random 256-bit key and a name, shown as a QR code that the others read
//!   with the Camera, or handed on by a six-digit code (SPAKE2, as
//!   Companion pairs). Everything between members is sealed with that key.
//! - **Talking** (wire.rs, engine.rs). UDP broadcast and multicast on
//!   [`PORT`]: hellos to find each other, messages resent until each member
//!   there has acknowledged them, each taken once, shown in one order
//!   everywhere, and what a member missed while away handed on from anyone's
//!   last [`engine::KEEP`].
//! - **Saying**: six canned calls on the number keys (editable), free text,
//!   and a Strategy board or Scout's records sent straight to the team.
//! - **Hearing**: on the island, the urgent calls with the signal; the Team
//!   page shows the thread, who is there and who has seen it, and the next
//!   match.
//! - **Battery**: the listener runs only while this handheld is in a team.
//!   It waits on the socket itself (no polling), wakes for a hello every 12
//!   s and for resends while they are due, and in deep standby parks on
//!   deep.rs's gate with the Wi-Fi off.

pub mod crypto;
pub mod engine;
pub mod join;
pub mod wire;
mod ui;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::json;

use crypto::TeamKey;
use engine::{Engine, Event, Out, To};
use join::Invite;
use wire::{Body, MemberId, ShareKind};

pub use ui::{JoinPage, TeamScreen};

/// UDP: Team's packets (beside Companion's 47315 and 47316).
pub const PORT: u16 = 47317;
/// Multicast, kept to the site (239.255/16), for networks that pass it
/// where they drop broadcast.
pub const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 58, 5);
/// A team's QR code: this, the key in hex, and the team's name.
pub const QR_PREFIX: &str = "CATTEAM1 ";
/// Parts a share may take.
pub const SHARE_MOST: usize = 60;

/// Where Team keeps its files: `/data/team` on the handheld.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CATALYST_TEAM") {
        return dir.into();
    }
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/team")
    } else {
        crate::config_dir().join("team")
    }
}

/// Milliseconds by the system's clock.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------ membership ---

/// This handheld in its team.
#[derive(Clone, Debug, PartialEq)]
pub struct Membership {
    pub key: TeamKey,
    pub team: String,
    /// This handheld's id in it: new at each join.
    pub me: MemberId,
    pub name: String,
}

impl Membership {
    pub fn load() -> Option<Membership> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir().join("team.json")).ok()?).ok()?;
        let key: [u8; 32] = catalyst_companion_proto::unhex(v["key"].as_str()?)?.try_into().ok()?;
        Some(Membership { key: TeamKey::from_bytes(key), team: v["team"].as_str()?.to_owned(), me: v["me"].as_str()?.parse().ok()?, name: v["name"].as_str()?.to_owned() })
    }

    fn save(&self) -> Result<(), String> {
        let d = dir();
        std::fs::create_dir_all(&d).map_err(|_| "the card would not take it")?;
        let v = json!({"key": catalyst_companion_proto::hex(self.key.bytes()), "team": self.team, "me": self.me.to_string(), "name": self.name});
        crate::sys::write_atomic(&d.join("team.json"), v.to_string().as_bytes()).map_err(|_| "the card would not take it".to_owned())
    }

    /// The QR code's text.
    pub fn qr_text(&self) -> String {
        format!("{QR_PREFIX}{} {}", catalyst_companion_proto::hex(self.key.bytes()), self.team)
    }
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    crypto::random(&mut b);
    u64::from_le_bytes(b).max(1)
}

/// A team's QR code read: its key and name.
pub fn read_qr(text: &str) -> Option<(TeamKey, String)> {
    let rest = text.trim().strip_prefix(QR_PREFIX)?;
    let (hex, name) = rest.split_once(' ').unwrap_or((rest, ""));
    let key: [u8; 32] = catalyst_companion_proto::unhex(hex)?.try_into().ok()?;
    Some((TeamKey::from_bytes(key), wire::clean_name(name)))
}

/// Whether a code the camera read is a team's.
pub fn is_team_code(text: &str) -> bool {
    read_qr(text).is_some()
}

/// The camera's card for a team's code: its symbol, its line, what Enter does.
pub fn card(text: &str) -> (&'static str, String, &'static str) {
    let name = read_qr(text).map(|(_, n)| n).unwrap_or_default();
    ("groups", format!("Team · {name}"), "enter: join this team")
}

/// Enter on a team's code in the camera: the page to join it.
pub fn scanned(text: &str) -> Box<dyn crate::shell::Screen> {
    match read_qr(text) {
        Some((key, team)) => Box::new(JoinPage::scanned(key, team)),
        None => Box::new(TeamScreen::new()),
    }
}

// ----------------------------------------------------------------- calls ---

/// The six calls on the number keys: their words, and whether each breaks
/// through as urgent.
pub const CALLS: [(&str, bool); 6] =
    [("robot to queue", true), ("battery needed", true), ("bring the cart", false), ("need a driver", true), ("match in 5", true), ("all good", false)];

pub fn calls() -> Vec<(String, bool)> {
    let kept: Option<Vec<(String, bool)>> = std::fs::read_to_string(dir().join("calls.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| {
        v.as_array()?.iter().map(|c| Some((wire::clean_text(c["words"].as_str()?), c["urgent"].as_bool()?))).collect()
    });
    match kept {
        Some(k) if k.len() == CALLS.len() && k.iter().all(|(w, _)| !w.is_empty()) => k,
        _ => CALLS.iter().map(|(w, u)| ((*w).to_owned(), *u)).collect(),
    }
}

pub fn save_calls(calls: &[(String, bool)]) {
    let d = dir();
    std::fs::create_dir_all(&d).ok();
    let v: Vec<serde_json::Value> = calls.iter().map(|(w, u)| json!({"words": w, "urgent": u})).collect();
    crate::sys::write_atomic(&d.join("calls.json"), serde_json::Value::from(v).to_string().as_bytes()).ok();
}

// --------------------------------------------------------------- service ---

struct Service {
    engine: Arc<Mutex<Engine>>,
    team: String,
    socket: Option<Arc<UdpSocket>>,
    stop: Arc<AtomicBool>,
    trouble: Arc<Mutex<Option<String>>>,
}

static SERVICE: Mutex<Option<Service>> = Mutex::new(None);
static INVITE: Mutex<Option<Invite>> = Mutex::new(None);
/// Bumped whenever the thread or the members change, for the pages.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Whether this handheld is in a team (its listener running).
pub fn member() -> bool {
    lock(&SERVICE).is_some()
}

/// The team's name.
pub fn team_name() -> Option<String> {
    lock(&SERVICE).as_ref().map(|s| s.team.clone())
}

/// Why the listener is not hearing anything, if it is not.
pub fn trouble() -> Option<String> {
    lock(&SERVICE).as_ref().and_then(|s| lock(&s.trouble).clone())
}

/// Look at (or change) the engine.
pub fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    let engine = lock(&SERVICE).as_ref()?.engine.clone();
    let mut e = lock(&engine);
    Some(f(&mut e))
}

/// At start: the listener, if this handheld is in a team.
pub fn start() {
    if lock(&SERVICE).is_some() {
        return;
    }
    if let Some(m) = Membership::load() {
        run(m);
    }
}

/// The listener for `m`, on a thread of its own.
fn run(m: Membership) {
    let mut engine = Engine::new(m.key.clone(), m.me, &m.name, random_u64() as u32);
    if let Some(v) = std::fs::read_to_string(dir().join("thread.json")).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        engine.restore(&v);
    }
    let engine = Arc::new(Mutex::new(engine));
    let stop = Arc::new(AtomicBool::new(false));
    let trouble = Arc::new(Mutex::new(None));
    let socket = match open_socket(PORT) {
        Ok(s) => Some(Arc::new(s)),
        Err(why) => {
            *lock(&trouble) = Some(why);
            None
        }
    };
    if let Some(sock) = &socket {
        let (engine, sock, stop, trouble, team) = (engine.clone(), sock.clone(), stop.clone(), trouble.clone(), m.team.clone());
        std::thread::Builder::new().name("team".into()).stack_size(256 * 1024).spawn(move || listen(&engine, &sock, &stop, &trouble, &team)).ok();
    }
    *lock(&SERVICE) = Some(Service { engine, team: m.team, socket, stop, trouble });
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn open_socket(port: u16) -> Result<UdpSocket, String> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).map_err(|_| format!("port {port} is taken"))?;
    sock.set_broadcast(true).ok();
    sock.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED).ok();
    sock.set_multicast_loop_v4(false).ok();
    Ok(sock)
}

/// Everyone: broadcast, and the multicast group.
fn everyone(port: u16) -> [SocketAddr; 2] {
    [SocketAddr::from((Ipv4Addr::BROADCAST, port)), SocketAddr::from((GROUP, port))]
}

/// Sent; false when nothing would go anywhere (no network).
fn send_out(sock: &UdpSocket, engine: &Engine, out: Out) -> bool {
    let mut any = out.is_empty();
    for (to, bytes) in out {
        match engine.peers.get(&match to {
            To::Peer(p) => p,
            To::All => 0,
        }) {
            Some(engine::Peer { addr: Some(addr), .. }) => any |= sock.send_to(&bytes, addr).is_ok(),
            _ => {
                for addr in everyone(PORT) {
                    any |= sock.send_to(&bytes, addr).is_ok();
                }
            }
        }
    }
    any
}

/// The listener: blocked on the socket until a packet comes or the engine
/// has something due; in deep standby, parked on its gate.
fn listen(engine: &Mutex<Engine>, sock: &UdpSocket, stop: &AtomicBool, trouble: &Mutex<Option<String>>, team: &str) {
    let mut buf = vec![0u8; wire::MOST + 64];
    let mut saved_at = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let wait = lock(engine).next_due(now_ms()).clamp(20, engine::HELLO_EVERY);
        sock.set_read_timeout(Some(Duration::from_millis(wait))).ok();
        let heard = sock.recv_from(&mut buf);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = now_ms();
        let mut e = lock(engine);
        if let Ok((n, from)) = &heard {
            let packet = &buf[..*n];
            if join::is_join(packet) {
                let answer = lock(&INVITE).as_mut().and_then(|i| i.answer(packet, &e.key, team, now));
                if let Some(a) = answer {
                    sock.send_to(&a, from).ok();
                }
            } else {
                let out = e.receive(packet, Some(*from), now);
                send_out(sock, &e, out);
            }
        }
        let out = e.poll(now);
        // Sends failing everywhere: no network (the Wi-Fi off, or deep standby).
        if !out.is_empty() {
            let ok = send_out(sock, &e, out);
            *lock(trouble) = (!ok).then(|| "no network · messages wait".to_owned());
        }
        let events = std::mem::take(&mut e.events);
        let changed = std::mem::take(&mut e.changed) || !events.is_empty();
        let arrived = events.iter().any(|ev| matches!(ev, Event::Arrived { .. }));
        let shares = shares_done(&e, &events);
        if changed && (arrived || now.saturating_sub(saved_at) > 2_000) {
            save_thread(&e);
            saved_at = now;
        }
        drop(e);
        if changed {
            GENERATION.fetch_add(1, Ordering::Relaxed);
            crate::platform::wake();
        }
        tell(&events, shares);
        // In deep standby, parked here: no wakeups until it wakes.
        crate::deep::rest();
    }
    save_thread(&lock(engine));
}

fn save_thread(e: &Engine) {
    let d = dir();
    std::fs::create_dir_all(&d).ok();
    crate::sys::write_atomic(&d.join("thread.json"), e.to_json().to_string().as_bytes()).ok();
}

/// Wake the listener: something was sent with resends due before it would
/// look again.
fn wake(sock: &UdpSocket) {
    if let Ok(local) = sock.local_addr() {
        sock.send_to(b"W", SocketAddr::from((Ipv4Addr::LOCALHOST, local.port()))).ok();
    }
}

/// Stop listening (at leaving, or for a test).
fn stop() {
    if let Some(s) = lock(&SERVICE).take() {
        s.stop.store(true, Ordering::Relaxed);
        if let Some(sock) = &s.socket {
            if let Some(bye) = lock(&s.engine).bye() {
                for addr in everyone(PORT) {
                    sock.send_to(&bye, addr).ok();
                }
            }
            wake(sock);
        }
    }
    *lock(&INVITE) = None;
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Make a team space: a new key, this handheld its first member.
pub fn create(team: &str, name: &str) -> Result<(), String> {
    let key = TeamKey::random().ok_or("no random numbers from the system")?;
    join_team(key, team, name)
}

/// Join (or start) the team of `key`: a new member id, kept, and the
/// listener started.
pub fn join_team(key: TeamKey, team: &str, name: &str) -> Result<(), String> {
    let (team, name) = (wire::clean_name(team), wire::clean_name(name));
    if team.is_empty() || name.is_empty() {
        return Err("a name is needed".into());
    }
    stop();
    std::fs::remove_file(dir().join("thread.json")).ok();
    let m = Membership { key, team, me: random_u64(), name };
    m.save()?;
    run(m);
    Ok(())
}

/// Leave the team: goodbye, and its key and thread gone from this handheld.
pub fn leave() {
    stop();
    for f in ["team.json", "thread.json"] {
        std::fs::remove_file(dir().join(f)).ok();
    }
}

/// Say something to the team.
pub fn send(body: Body, urgent: bool) -> Result<(), String> {
    let guard = lock(&SERVICE);
    let s = guard.as_ref().ok_or("not in a team")?;
    let mut e = lock(&s.engine);
    let out = e.send(body, urgent, chrono::Utc::now().timestamp(), now_ms());
    if let Some(sock) = &s.socket {
        send_out(sock, &e, out);
        wake(sock);
    }
    if s.socket.is_none() {
        // The demo (and a port that was taken): kept, as if sent.
        e.changed = false;
    }
    save_thread(&e);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// The thread read to its newest message (the page is looked at).
pub fn read_all() {
    let guard = lock(&SERVICE);
    let Some(s) = guard.as_ref() else { return };
    let mut e = lock(&s.engine);
    let before = e.seen;
    e.read_all(now_ms());
    if e.seen != before {
        if let Some(sock) = &s.socket {
            wake(sock);
        }
    }
}

/// The invitation while Invite shows: its code, made new when spent.
pub fn invite_code() -> Option<(String, u64)> {
    let now = now_ms();
    let mut invite = lock(&INVITE);
    if invite.as_ref().is_none_or(|i| i.spent(now)) {
        *invite = Invite::new(now);
    }
    invite.as_ref().map(|i| (i.code.clone(), i.left(now)))
}

/// Invite has closed: no more asks answered.
pub fn invite_closed() {
    *lock(&INVITE) = None;
}

// ---------------------------------------------------------------- shares ---

/// Texts packed into parts, whole, `\x1e` between them.
pub fn pack(texts: &[String]) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for t in texts {
        let t: String = t.chars().filter(|&c| c != '\u{1e}').collect();
        match parts.last_mut() {
            Some(p) if p.len() + 1 + t.len() <= wire::DATA_MOST => {
                p.push('\u{1e}');
                p.push_str(&t);
            }
            _ => parts.push(t),
        }
    }
    parts
}

/// Send a share: `texts` are what the app would show as QR codes, read on
/// the other handhelds with that app's own decoder. How many parts it took.
pub fn share(kind: ShareKind, label: &str, texts: &[String]) -> Result<usize, String> {
    if !member() {
        return Err("join a team first, in Team".into());
    }
    let parts = pack(texts);
    if parts.is_empty() {
        return Err("nothing to send".into());
    }
    if parts.len() > SHARE_MOST || parts.iter().any(|p| p.len() > wire::DATA_MOST) {
        return Err("too much for Team: share it by QR code".into());
    }
    let set = random_u64() as u32;
    let of = parts.len() as u8;
    for (k, data) in parts.into_iter().enumerate() {
        send(Body::Share { kind, set, part: k as u8 + 1, of, label: wire::clean_name(label), data }, false)?;
    }
    Ok(of as usize)
}

/// A share whose parts have all come: who, what, and its texts in order.
struct Done {
    kind: ShareKind,
    by: String,
    label: String,
    texts: Vec<String>,
    late: bool,
}

/// The texts of share `set` from `origin`, if every part is in the log.
pub fn share_texts(e: &Engine, origin: MemberId, set: u32) -> Option<Vec<String>> {
    let mut parts: Vec<(u8, u8, &str)> = e
        .log
        .iter()
        .filter_map(|en| match &en.m.body {
            Body::Share { set: s, part, of, data, .. } if *s == set && en.m.id.origin == origin => Some((*part, *of, data.as_str())),
            _ => None,
        })
        .collect();
    parts.sort_by_key(|p| p.0);
    parts.dedup_by_key(|p| p.0);
    let of = parts.first()?.1;
    (parts.len() == of as usize).then(|| parts.iter().flat_map(|p| p.2.split('\u{1e}')).map(str::to_owned).collect())
}

fn shares_done(e: &Engine, events: &[Event]) -> Vec<Done> {
    let mut done = Vec::new();
    for ev in events {
        if let Event::Arrived { m, late } = ev {
            if let Body::Share { kind, set, label, .. } = &m.body {
                if let Some(texts) = share_texts(e, m.id.origin, *set) {
                    if !done.iter().any(|d: &Done| d.label == *label && d.by == m.by) {
                        done.push(Done { kind: *kind, by: m.by.clone(), label: label.clone(), texts, late: *late });
                    }
                }
            }
        }
    }
    done
}

/// What arrived, said: on the island (urgent calls breaking through), and
/// shares taken into their apps.
fn tell(events: &[Event], shares: Vec<Done>) {
    let mut late = 0;
    for ev in events {
        let Event::Arrived { m, late: was_late } = ev else { continue };
        if matches!(m.body, Body::Share { .. }) {
            continue;
        }
        if *was_late {
            late += 1;
            continue;
        }
        let mut n = crate::notify::Notification::new("Team", "groups", m.by.clone()).body(m.words()).action("builtin:Team");
        if m.urgent {
            n = n.urgent();
        }
        crate::notify::post(n);
    }
    if late > 0 {
        crate::notify::post(crate::notify::Notification::new("Team", "groups", format!("{late} message{} while away", if late == 1 { "" } else { "s" })).action("builtin:Team"));
    }
    for d in shares {
        let words = take_share(&d);
        if !d.late {
            crate::notify::post(crate::notify::Notification::new("Team", "groups", d.by.clone()).body(format!("shared {} · {words}", d.label)).action("builtin:Team"));
        }
    }
}

/// A share taken in by its app: Scout's records merged, a board kept.
fn take_share(d: &Done) -> String {
    match d.kind {
        ShareKind::Scout => {
            let (mut added, mut newer) = (0, 0);
            for t in &d.texts {
                if let Some((a, u)) = crate::scout::from_team(t, &d.by) {
                    added += a;
                    newer += u;
                }
            }
            match (added, newer) {
                (0, 0) => "nothing new".into(),
                (a, 0) => format!("{a} new in Scout"),
                (a, u) => format!("{a} new · {u} newer in Scout"),
            }
        }
        ShareKind::Strategy => {
            let texts: Vec<&str> = d.texts.iter().map(String::as_str).collect();
            match crate::strategy::take_team_board(&texts) {
                Ok(_) => "in Strategy".into(),
                Err(why) => why,
            }
        }
    }
}

/// The tile on Home.
pub fn tile() -> (String, Option<&'static str>) {
    let now = now_ms();
    match with(|e| (e.here(now).count(), e.log.iter().filter(|x| x.m.lamport > e.seen && x.m.id.origin != e.me).count())) {
        None => ("pit · drive · scouts".into(), None),
        Some((_, unread)) if unread > 0 => (format!("{unread} new"), None),
        Some((0, _)) => ("no one nearby".into(), None),
        Some((n, _)) => (format!("{} here", n + 1), None),
    }
}

// ------------------------------------------------------------------ demo ---

/// A team as the screenshots show it, with no network: this handheld in
/// it, and the engine given to look at.
pub fn install_demo(engine: Engine, team: &str) {
    stop();
    *lock(&SERVICE) = Some(Service {
        engine: Arc::new(Mutex::new(engine)),
        team: team.to_owned(),
        socket: None,
        stop: Arc::new(AtomicBool::new(false)),
        trouble: Arc::new(Mutex::new(None)),
    });
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// No team (the screenshots' welcome).
pub fn remove_demo() {
    *lock(&SERVICE) = None;
    *lock(&INVITE) = None;
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Join with a code typed, on a thread of its own: asks on the network for
/// a few seconds. What happened, for the page.
pub fn join_by_code(code: String, name: String) -> std::sync::mpsc::Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("team join".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = ask_with_code(&code).and_then(|(key, team)| join_team(key, &team, &name).map(|_| team));
            tx.send(result).ok();
            crate::platform::wake();
        })
        .ok();
    rx
}

/// Ask for the team showing `code`, for up to six seconds: a new ask each
/// two (an answer that would not open spends that ask).
fn ask_with_code(code: &str) -> Result<(TeamKey, String), String> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|_| "no network")?;
    sock.set_broadcast(true).ok();
    sock.set_read_timeout(Some(Duration::from_millis(400))).ok();
    let mut buf = vec![0u8; 512];
    let mut answered_wrong = false;
    for _ in 0..3 {
        let (asking, ask) = join::Asking::new(code).ok_or("no random numbers from the system")?;
        let mut asking = Some(asking);
        for round in 0..5 {
            if round % 2 == 0 {
                let mut sent = false;
                for addr in everyone(PORT) {
                    sent |= sock.send_to(&ask, addr).is_ok();
                }
                if !sent {
                    return Err("no network: join the pit's Wi-Fi first".into());
                }
            }
            while let Ok((n, _)) = sock.recv_from(&mut buf) {
                let Some(a) = asking.take() else { break };
                if !a.answers(&buf[..n]) {
                    asking = Some(a);
                    continue;
                }
                match a.finish(&buf[..n]) {
                    Some(found) => return Ok(found),
                    None => answered_wrong = true,
                }
            }
            if asking.is_none() {
                break;
            }
        }
    }
    Err(if answered_wrong { "that code is not the one showing".into() } else { "no team showing that code nearby".into() })
}
