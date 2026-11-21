//! Tailscale: the handheld on your tailnet, so it reaches your other
//! machines from anywhere (a laptop over SSH, a dashboard at home) and they
//! reach it. Off unless switched on in Settings › Network › Tailscale; only
//! then does tailscaled run (the catalyst-tailscale service).
//!
//! Signing in happens on another device: tailscaled hands out a link, shown
//! here as a QR code for a phone's camera. The sign-in and the machine's key
//! live in /data/tailscale, so it stays signed in across restarts and
//! updates. No firewall rules and no DNS changes: the handheld's own network
//! settings stay as they are.
//!
//! The page reads tailscaled's status from its local API, over its socket,
//! every two seconds while it shows. On a computer (the simulator) a
//! made-up tailnet stands in, signing in by itself a few seconds after its
//! code is shown, as if a phone had scanned it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use bezel::{Align, Rect};
use serde_json::Value;

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::git::Job;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const SCRIPT: &str = "/usr/sbin/catalyst-tailscale";
const FLAG: &str = "/data/catalyst/tailscale-enabled";
#[cfg(unix)]
const SOCKET: &str = "/run/tailscale/tailscaled.sock";
const PIDFILE: &str = "/run/tailscaled.pid";
/// How often the page reads the status while it shows, in seconds.
const EVERY: f32 = 2.0;
/// How long a sign-in link is waited for before asking again, in seconds.
const ASK_AGAIN: f32 = 20.0;

fn on_device() -> bool {
    Path::new(SCRIPT).exists()
}

/// Whether Tailscale is switched on.
pub fn enabled() -> bool {
    if on_device() {
        Path::new(FLAG).exists()
    } else {
        DEMO.load(Ordering::Relaxed) != OFF
    }
}

// ---------------------------------------------------------------- status ---

/// Another machine on the tailnet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    pub os: String,
    pub online: bool,
    /// When it was last connected (seconds since 1970), if ever.
    pub seen: Option<i64>,
}

/// What tailscaled says of itself and the tailnet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// NoState, NeedsLogin, NeedsMachineAuth, Stopped, Starting or Running.
    pub state: String,
    /// The link to sign in with, while it waits for one.
    pub auth_url: String,
    /// This handheld on the tailnet: its name, full name and address.
    pub name: String,
    pub dns: String,
    pub ip: String,
    pub tailnet: String,
    pub peers: Vec<Peer>,
    /// Anything tailscaled thinks is wrong.
    pub health: Vec<String>,
    /// tailscaled's resident memory, in MB.
    pub memory: Option<f32>,
}

/// The status as tailscaled's local API gives it (`tailscale status --json`).
pub fn parse_status(v: &Value) -> Status {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    // Its IPv4 address where it has one: the shorter to type.
    let ip = |p: &Value| {
        let ips: Vec<&str> = p["TailscaleIPs"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        ips.iter().find(|ip| ip.contains('.')).or(ips.first()).copied().unwrap_or_default().to_owned()
    };
    // The name others reach it by: the first part of its MagicDNS name.
    let name = |p: &Value| p["DNSName"].as_str().and_then(|d| d.split('.').next()).filter(|n| !n.is_empty()).map_or_else(|| s(&p["HostName"]), str::to_owned);
    let mut peers: Vec<Peer> = v["Peer"]
        .as_object()
        .map(|m| {
            m.values()
                .map(|p| Peer { name: name(p), ip: ip(p), os: s(&p["OS"]), online: p["Online"].as_bool().unwrap_or(false), seen: seen(&p["LastSeen"]) })
                .collect()
        })
        .unwrap_or_default();
    peers.sort_by_key(|p| (!p.online, p.name.to_lowercase()));
    let me = &v["Self"];
    Status {
        state: s(&v["BackendState"]),
        auth_url: s(&v["AuthURL"]),
        name: name(me),
        dns: s(&me["DNSName"]).trim_end_matches('.').to_owned(),
        ip: ip(me),
        tailnet: v["CurrentTailnet"]["Name"].as_str().filter(|n| !n.is_empty()).or(v["MagicDNSSuffix"].as_str()).unwrap_or_default().to_owned(),
        peers,
        health: v["Health"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default(),
        memory: None,
    }
}

/// A last-seen time; Go's zero time ("0001-01-01…") is never.
fn seen(v: &Value) -> Option<i64> {
    let t = chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok()?.timestamp();
    (t > 946_684_800).then_some(t)
}

/// "seen 3 h ago", "seen nov 20".
fn seen_words(time: i64, now: i64) -> String {
    let ago = crate::git::ago(time, now);
    match ago.as_str() {
        "now" => "seen just now".into(),
        a if a.starts_with(|c: char| c.is_ascii_digit()) => format!("seen {a} ago"),
        a => format!("seen {a}"),
    }
}

/// The status now, with tailscaled's memory: off the drawing thread.
fn read() -> Result<Status, String> {
    if !on_device() {
        return Ok(demo_status());
    }
    let mut status = parse_status(&local_api("/localapi/v0/status")?);
    status.memory = memory();
    Ok(status)
}

/// A GET from tailscaled's local API, on its socket.
#[cfg(unix)]
fn local_api(path: &str) -> Result<Value, String> {
    use std::io::{Read, Write};
    let mut socket = std::os::unix::net::UnixStream::connect(SOCKET).map_err(|_| "tailscaled is not running yet".to_owned())?;
    let limit = Some(std::time::Duration::from_secs(5));
    socket.set_read_timeout(limit).ok();
    socket.set_write_timeout(limit).ok();
    // HTTP/1.0: the answer ends when the connection closes, never chunked.
    write!(socket, "GET {path} HTTP/1.0\r\nHost: local-tailscaled.sock\r\n\r\n").map_err(|_| "tailscaled did not answer".to_owned())?;
    let mut answer = Vec::new();
    socket.read_to_end(&mut answer).map_err(|_| "tailscaled did not answer".to_owned())?;
    let text = String::from_utf8_lossy(&answer);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("tailscaled did not answer")?;
    if !head.split_whitespace().nth(1).is_some_and(|code| code == "200") {
        return Err(format!("tailscaled: {}", body.lines().next().unwrap_or(head).trim()));
    }
    serde_json::from_str(body).map_err(|_| "tailscaled's answer could not be read".to_owned())
}

#[cfg(not(unix))]
fn local_api(_path: &str) -> Result<Value, String> {
    Err("tailscale runs on the handheld".into())
}

/// tailscaled's resident memory, in MB.
fn memory() -> Option<f32> {
    let pid: u32 = std::fs::read_to_string(PIDFILE).ok()?.trim().parse().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kb: f32 = status.lines().find(|l| l.starts_with("VmRSS:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

/// Switch Tailscale on or off, waited for: off the drawing thread.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn switch(on: bool) -> Result<(), String> {
    run(if on { "on" } else { "off" })
}

/// The service script, waited for (it waits for tailscaled's socket).
fn run(verb: &'static str) -> Result<(), String> {
    if !on_device() {
        demo(verb);
        return Ok(());
    }
    let out = Command::new(SCRIPT).arg(verb).stdin(Stdio::null()).output().map_err(|_| "tailscale is not installed".to_owned())?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default().trim().to_owned();
        Err(if why.is_empty() { format!("tailscale {verb} failed") } else { why })
    }
}

// ------------------------------------------------------------------ demo ---

const OFF: u8 = 0;
const SIGNING_IN: u8 = 1;
const SIGNED_IN: u8 = 2;
static DEMO: AtomicU8 = AtomicU8::new(OFF);
static DEMO_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
/// The made-up tailnet: starting this long, then its code shown this long
/// before a "phone" signs in.
const DEMO_STARTING: f32 = 1.2;
const DEMO_SCANNED: f32 = 9.0;

fn demo(verb: &str) {
    let stage = match verb {
        "on" | "logout" => SIGNING_IN,
        "off" => OFF,
        _ => return,
    };
    DEMO.store(stage, Ordering::Relaxed);
    *DEMO_SINCE.lock().unwrap() = Some(Instant::now());
}

/// The gallery's made-up tailnet, signed in at once.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_signed_in() {
    DEMO.store(SIGNED_IN, Ordering::Relaxed);
}

fn demo_status() -> Status {
    let since = DEMO_SINCE.lock().unwrap().map_or(f32::MAX, |t| t.elapsed().as_secs_f32());
    if DEMO.load(Ordering::Relaxed) == SIGNING_IN && since > DEMO_STARTING + DEMO_SCANNED {
        DEMO.store(SIGNED_IN, Ordering::Relaxed);
    }
    match DEMO.load(Ordering::Relaxed) {
        OFF => Status::default(),
        SIGNING_IN if since < DEMO_STARTING => Status { state: "Starting".into(), ..Status::default() },
        SIGNING_IN => Status { state: "NeedsLogin".into(), auth_url: "https://login.tailscale.com/a/5e1d7c2a9b3f80".into(), ..Status::default() },
        _ => {
            let now = chrono::Utc::now().timestamp();
            let me: String = crate::device::name().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            let peer = |name: &str, os: &str, ip: &str, online: bool, ago: i64| Peer { name: name.into(), ip: ip.into(), os: os.into(), online, seen: Some(now - ago) };
            Status {
                state: "Running".into(),
                dns: format!("{me}.tail5805.ts.net"),
                name: me,
                ip: "100.101.7.23".into(),
                tailnet: "team5805.github".into(),
                peers: vec![
                    peer("drive-station", "windows", "100.88.14.2", true, 0),
                    peer("pit-laptop", "macOS", "100.71.203.9", true, 0),
                    peer("scouting-tablet", "android", "100.94.5.61", true, 0),
                    peer("build-server", "linux", "100.120.33.4", false, 3 * 3600),
                ],
                memory: Some(24.6),
                ..Status::default()
            }
        }
    }
}

// ---------------------------------------------------------------- screen ---

/// Where things stand, as the page shows it.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Off,
    Starting,
    SignIn,
    Approval,
    Paused,
    Running,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Switch,
    SignIn,
    Device,
    Tailnet,
    Peer(usize),
    Memory,
    SignOut,
}

#[derive(Clone, Copy, PartialEq)]
enum Work {
    On,
    Off,
    Login,
    Logout,
}

/// Settings › Network › Tailscale: switching it on, signing in with a QR
/// code, this handheld's address, the machines on the tailnet, memory, and
/// signing out.
pub struct TailscaleScreen {
    list: ListState,
    status: Option<Result<Status, String>>,
    reading: Option<Job<Result<Status, String>>>,
    /// When the last read started: reads follow the clock on the wall, since
    /// the page's own time stands still while nothing on it moves.
    read_at: Instant,
    /// The script at work, off the drawing thread.
    working: Option<(Work, Job<Result<(), String>>)>,
    /// When a sign-in link was last asked for.
    asked: Option<Instant>,
    /// The sign-in link as a QR code, made once per link.
    qr: Option<(String, qrcodegen::QrCode)>,
    /// Sign out was pressed once: a second press signs out.
    armed: bool,
    changed: bool,
    now: i64,
}

impl TailscaleScreen {
    pub fn new() -> Self {
        let mut page = TailscaleScreen {
            list: ListState::default(),
            status: None,
            reading: None,
            read_at: Instant::now(),
            working: None,
            asked: None,
            qr: None,
            armed: false,
            changed: true,
            now: chrono::Utc::now().timestamp(),
        };
        page.read_now();
        page
    }

    fn read_now(&mut self) {
        self.read_at = Instant::now();
        self.reading = enabled().then(|| Job::start(read));
    }

    fn start(&mut self, work: Work) {
        if self.working.is_some() {
            return;
        }
        let verb = match work {
            Work::On => "on",
            Work::Off => "off",
            Work::Login => "login",
            Work::Logout => "logout",
        };
        if work != Work::Off {
            self.asked = Some(Instant::now());
        }
        self.working = Some((work, Job::start(move || run(verb))));
        self.changed = true;
    }

    fn status(&self) -> Option<&Status> {
        self.status.as_ref().and_then(|s| s.as_ref().ok())
    }

    fn phase(&self) -> Phase {
        if !enabled() {
            return Phase::Off;
        }
        match self.status().map(|s| s.state.as_str()) {
            Some("NeedsLogin") => Phase::SignIn,
            Some("NeedsMachineAuth") => Phase::Approval,
            Some("Stopped") => Phase::Paused,
            Some("Running") => Phase::Running,
            _ => Phase::Starting,
        }
    }

    fn rows(&self, phase: Phase) -> Vec<Row> {
        match phase {
            Phase::Off | Phase::Starting | Phase::Paused => vec![Row::Switch],
            Phase::SignIn | Phase::Approval => vec![Row::Switch, Row::SignIn],
            Phase::Running => {
                let peers = self.status().map_or(0, |s| s.peers.len());
                [Row::Switch, Row::Device, Row::Tailnet].into_iter().chain((0..peers).map(Row::Peer)).chain([Row::Memory, Row::SignOut]).collect()
            }
        }
    }

    /// A row's name, its value, and whether the value is lit.
    fn describe(&self, row: Row, phase: Phase) -> (String, String, bool) {
        let st = self.status();
        match row {
            Row::Switch => {
                let value = match (self.working.as_ref().map(|w| w.0), phase) {
                    (Some(Work::On), _) => "starting…",
                    (Some(Work::Off), _) => "stopping…",
                    (_, Phase::Off) => "off",
                    (_, Phase::Starting) => "starting…",
                    (_, Phase::Paused) => "paused",
                    _ => "on",
                };
                ("Tailscale".into(), value.into(), phase != Phase::Off)
            }
            Row::SignIn => {
                let value = match phase {
                    Phase::Approval => "waiting for approval",
                    _ if self.qr.is_some() => "scan it",
                    _ => "asking…",
                };
                ("Sign in".into(), value.into(), true)
            }
            Row::Device => ("This handheld".into(), st.map_or_else(String::new, |s| s.ip.clone()), true),
            Row::Tailnet => ("Tailnet".into(), st.map_or_else(String::new, |s| s.tailnet.clone()), true),
            Row::Peer(i) => match st.and_then(|s| s.peers.get(i)) {
                Some(p) if p.online => (p.name.clone(), p.ip.clone(), true),
                Some(p) => (p.name.clone(), p.seen.map_or_else(|| "offline".into(), |t| seen_words(t, self.now)), false),
                None => Default::default(),
            },
            Row::Memory => ("Memory in use".into(), st.and_then(|s| s.memory).map_or_else(|| "–".into(), |mb| format!("{mb:.0} MB")), true),
            Row::SignOut => ("Sign out".into(), (if self.armed { "enter again" } else { "" }).into(), true),
        }
    }

    fn hint(&self, row: Row, phase: Phase) -> String {
        let st = self.status();
        match row {
            Row::Switch => match phase {
                Phase::Off => "enter: switch on".into(),
                Phase::Starting => match &self.status {
                    Some(Err(why)) => why.clone(),
                    _ => "starting tailscaled".into(),
                },
                Phase::Paused => "enter: connect again".into(),
                Phase::SignIn | Phase::Approval => "enter: switch off".into(),
                _ => match st.and_then(|s| s.health.first()) {
                    Some(trouble) => trouble.to_lowercase(),
                    None => "enter: switch off · stays signed in".into(),
                },
            },
            Row::SignIn => match (phase, &self.qr) {
                (Phase::Approval, _) => "an admin approves it in the tailnet's admin console".into(),
                (_, Some((url, _))) => format!("or open {}", url.trim_start_matches("https://")),
                _ => "asking tailscale for a sign-in link".into(),
            },
            Row::Device => st.map_or_else(String::new, |s| format!("others reach it as {}", if s.dns.is_empty() { &s.name } else { &s.dns })),
            Row::Tailnet => "the tailnet this handheld is in".into(),
            Row::Peer(i) => st.and_then(|s| s.peers.get(i)).map_or_else(String::new, |p| [p.os.to_lowercase(), p.ip.clone()].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" · ")),
            Row::Memory => "tailscaled's own · none while it is off".into(),
            Row::SignOut if self.armed => "enter again: sign out · esc: stay".into(),
            Row::SignOut => "forgets this handheld's sign-in".into(),
        }
    }
}

impl Screen for TailscaleScreen {
    fn title(&self) -> &str {
        "Tailscale"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let phase = self.phase();
        let rows = self.rows(phase);
        let row = rows.get(self.list.focus).copied().unwrap_or(Row::Switch);
        // Sign out asks twice; any other key, or Esc, lets it be.
        let armed = std::mem::take(&mut self.armed);
        match key {
            Key::Back if armed => {}
            Key::Back => return Nav::Pop,
            Key::Char('r') => self.read_now(),
            Key::Enter | Key::Right => match row {
                _ if self.working.is_some() => *cx.say = Some("a moment…".into()),
                Row::Switch => match phase {
                    Phase::Off => self.start(Work::On),
                    Phase::Paused => self.start(Work::Login),
                    _ => self.start(Work::Off),
                },
                Row::SignIn if self.qr.is_none() && phase == Phase::SignIn => self.start(Work::Login),
                Row::SignOut if armed => self.start(Work::Logout),
                Row::SignOut => self.armed = true,
                _ => {}
            },
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        self.changed = true;
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.working.as_ref().and_then(|(_, job)| job.get()) {
            let work = self.working.take().map(|(w, _)| w);
            *cx.say = Some(match (work, done) {
                (_, Err(why)) => why,
                (Some(Work::On), _) => "tailscale on".into(),
                (Some(Work::Off), _) => "tailscale off · still signed in".into(),
                (Some(Work::Logout), _) => "signed out".into(),
                _ => "asking for a sign-in link".into(),
            });
            self.status = None;
            self.qr = None;
            self.list.focus = 0;
            self.read_now();
            self.changed = true;
        }
        if let Some(done) = self.reading.as_ref().and_then(Job::get) {
            self.reading = None;
            self.now = chrono::Utc::now().timestamp();
            let url = done.as_ref().map_or("", |s| s.auth_url.as_str());
            if url.is_empty() {
                self.qr = None;
            } else if self.qr.as_ref().is_none_or(|(had, _)| had != url) {
                // A link's own letters: the path is case-sensitive.
                self.qr = qrcodegen::QrCode::encode_text(url, qrcodegen::QrCodeEcc::Low).ok().map(|qr| (url.to_owned(), qr));
            }
            self.status = Some(done);
            // Signed out with no link coming: ask for one (now and then, not every read).
            if self.phase() == Phase::SignIn && self.qr.is_none() && self.working.is_none() && self.asked.is_none_or(|t| t.elapsed().as_secs_f32() > ASK_AGAIN) {
                self.start(Work::Login);
            }
            let len = self.rows(self.phase()).len();
            self.list.focus = self.list.focus.min(len.saturating_sub(1));
            self.changed = true;
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        if self.read_at.elapsed().as_secs_f32() >= EVERY && self.reading.is_none() && self.working.is_none() {
            self.read_now();
        }
        std::mem::take(&mut self.changed)
    }

    fn busy(&self) -> bool {
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let phase = self.phase();
        let rows = self.rows(phase);
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        // The sign-in code takes the right of the page, as tall as it has room for.
        let code = if phase == Phase::SignIn { self.qr.as_ref().map(|(_, qr)| qr) } else { None };
        let side = if code.is_some() { (bottom - top).min(124.0) } else { 0.0 };
        let width = W - 2.0 * PAD - 5.0 - if side > 0.0 { side + 8.0 } else { 0.0 };
        let value_style = if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label };
        let mut below = top;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let (name, value, lit) = self.describe(rows[i], phase);
            let room = r.w - g.measure(&name, ty.body) - 34.0;
            let value = g.text.fit(&value, &value_style, room.max(30.0));
            g.row(r, &name, &value, if lit { t.ink } else { t.dim }, i == self.list.focus);
            below = below.max(r.bottom());
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        if let Some(qr) = code {
            crate::remote::draw_code(g, qr, Rect::new(W - PAD - side + g.ox, top, side, bottom - top), 1.0);
        }
        // A few words under a short page: what it is, or what to do now.
        let words = match phase {
            Phase::Off => "Puts the handheld on your tailnet: reach your other machines from anywhere, and they reach it. Off until switched on.",
            Phase::SignIn if code.is_some() => "Scan with a phone's camera and sign in to Tailscale there.",
            Phase::Approval => "Signed in. The tailnet's admin approves new machines first.",
            _ => "",
        };
        let mut y = below + 10.0 + ty.caption.size * 0.5;
        for line in crate::help::wrap(g, words, ty.caption, width - 8.0) {
            if y + ty.caption.size * 0.5 > bottom {
                break;
            }
            g.text_at(PAD + 4.0, y, Align::Left, &line, ty.caption, t.dim);
            y += ty.caption.size + 4.0;
        }
        let hint = rows.get(self.list.focus).map_or_else(String::new, |row| self.hint(*row, phase));
        g.hint(&hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_status_is_read_with_this_handheld_and_its_peers() {
        let v = json!({
            "Version": "1.102.3",
            "BackendState": "Running",
            "AuthURL": "",
            "TailscaleIPs": ["100.101.7.23", "fd7a:115c:a1e0::1"],
            "Self": {"HostName": "atlas", "DNSName": "atlas.tail5805.ts.net.", "OS": "linux", "TailscaleIPs": ["fd7a:115c:a1e0::1", "100.101.7.23"], "Online": true},
            "Health": ["not connected to home DERP region 1"],
            "MagicDNSSuffix": "tail5805.ts.net",
            "CurrentTailnet": {"Name": "team5805.github", "MagicDNSSuffix": "tail5805.ts.net", "MagicDNSEnabled": true},
            "Peer": {
                "nodekey:b": {"HostName": "Pit Laptop", "DNSName": "pit-laptop.tail5805.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.71.203.9"], "Online": true, "LastSeen": "0001-01-01T00:00:00Z"},
                "nodekey:a": {"HostName": "build", "DNSName": "build-server.tail5805.ts.net.", "OS": "linux", "TailscaleIPs": ["100.120.33.4"], "Online": false, "LastSeen": "2026-11-20T18:00:00Z"},
                "nodekey:c": {"HostName": "drive-station", "DNSName": "", "OS": "windows", "TailscaleIPs": ["100.88.14.2"], "Online": true}
            }
        });
        let s = parse_status(&v);
        assert_eq!((s.state.as_str(), s.name.as_str(), s.dns.as_str(), s.ip.as_str()), ("Running", "atlas", "atlas.tail5805.ts.net", "100.101.7.23"));
        assert_eq!(s.tailnet, "team5805.github");
        assert_eq!(s.health, ["not connected to home DERP region 1"]);
        // Online first, then by name; a name from MagicDNS, else the host's.
        let names: Vec<&str> = s.peers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["drive-station", "pit-laptop", "build-server"]);
        assert_eq!(s.peers[1].seen, None);
        let seen = s.peers[2].seen.unwrap();
        assert_eq!(seen_words(seen, seen + 3 * 3600), "seen 3 h ago");
        assert_eq!(seen_words(seen, seen + 20), "seen just now");
    }

    #[test]
    fn signing_in_gives_its_link_and_an_older_tailscaled_its_tailnet_by_suffix() {
        let v = json!({"BackendState": "NeedsLogin", "AuthURL": "https://login.tailscale.com/a/5e1d7c2a9b3f80", "Self": {"HostName": "atlas", "TailscaleIPs": null}, "Peer": null, "MagicDNSSuffix": "example.ts.net", "CurrentTailnet": null});
        let s = parse_status(&v);
        assert_eq!((s.state.as_str(), s.auth_url.as_str(), s.name.as_str(), s.ip.as_str()), ("NeedsLogin", "https://login.tailscale.com/a/5e1d7c2a9b3f80", "atlas", ""));
        assert_eq!(s.tailnet, "example.ts.net");
        assert!(s.peers.is_empty());
        // The link fits a small code a phone reads off the panel.
        let qr = qrcodegen::QrCode::encode_text(&s.auth_url, qrcodegen::QrCodeEcc::Low).unwrap();
        assert!(qr.size() <= 33, "{}", qr.size());
        assert_eq!(parse_status(&json!({"message": "nope"})), Status::default());
    }
}
