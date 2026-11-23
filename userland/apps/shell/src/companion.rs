//! Settings › Computer companion: Catalyst Companion, the app for a Windows
//! computer, and the handheld's side of it (`catalyst-companiond`, a
//! service of its own).
//!
//! - **The page.** The switch (off, the service is not running and nothing
//!   listens), Pair a computer (a six-digit code, good for five minutes),
//!   the computers paired (Enter twice unpairs one), and where the
//!   handheld can be found.
//! - **Pairing.** The code is written to /run for the service, which runs
//!   SPAKE2 with it (the code itself never crosses the network) and says
//!   how it went in a file beside it; this page shows the outcome.
//! - **What only the shell does.** The service asks the shell, on a socket
//!   only root can open, for what it has (the look, the keys as they are
//!   shown, whether Music is signed in) and to put in place what a
//!   computer sent: the same fields, checks and code as the page for
//!   setting up from a computer (remote.rs `answers`, setup.rs
//!   `apply_now` and `apply_look`). The look and the team wait for the
//!   next frame, which has the settings in hand.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bezel::{Rect, TextStyle};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::keys::{Keys, SERVICES};
use crate::platform::Key;
use crate::remote;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen, Settings};

/// How long a pairing code is good for.
const CODE_FOR: Duration = Duration::from_secs(300);
/// The most the service may send the shell in one request (a cookies.txt
/// of 64 KB, form-encoded).
const REQUEST_MOST: u64 = 512 * 1024;
/// The service's script: there, it is on the handheld.
const SERVICE: &str = "/etc/init.d/catalyst-companion";

// ----------------------------------------------------------------- places ---

/// The companion's own folder on /data (its computers, its certificate).
fn own_dir() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join(format!("catalyst-companion-test-{}", std::process::id())).join("companion")
    } else if crate::sys::on_handheld() {
        "/data/companion".into()
    } else {
        crate::config_dir().join("companion")
    }
}

/// Where the code and the outcome are passed, and the shell's socket.
fn run_dir() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join(format!("catalyst-companion-test-{}", std::process::id())).join("run")
    } else if crate::sys::on_handheld() {
        "/run/catalyst".into()
    } else {
        crate::config_dir().join("run")
    }
}

/// Switched on (it is, unless switched off).
pub fn enabled() -> bool {
    !own_dir().join("off").exists()
}

/// The service started or stopped, and the choice kept across starts.
pub fn switch(on: bool) -> Result<(), String> {
    let flag = own_dir().join("off");
    if on {
        std::fs::remove_file(&flag).ok();
    } else {
        stop_pairing();
        std::fs::create_dir_all(own_dir()).ok();
        std::fs::write(&flag, b"").map_err(|e| format!("could not switch it off: {e}"))?;
    }
    if crate::sys::on_handheld() && std::path::Path::new(SERVICE).exists() {
        // Restart, not start: the service counts as started even while it is off.
        let ok = std::process::Command::new("rc-service").args(["catalyst-companion", if on { "restart" } else { "stop" }]).stdin(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("the companion did not {}", if on { "start" } else { "stop" }));
        }
    }
    Ok(())
}

// -------------------------------------------------------------- computers ---

/// A computer paired with this handheld.
#[derive(Clone, Debug, PartialEq)]
pub struct Computer {
    pub id: String,
    pub name: String,
    pub paired: i64,
}

pub fn computers() -> Vec<Computer> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(own_dir().join("computers.json")).unwrap_or_default()).unwrap_or(Value::Null);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some(Computer { id: c["id"].as_str()?.to_owned(), name: c["name"].as_str().unwrap_or("a computer").to_owned(), paired: c["paired"].as_i64().unwrap_or(0) }))
        .collect()
}

/// One computer unpaired: its token is no good from now on. With none
/// left, the certificate goes too, and the next pairing makes a new one.
pub fn unpair(id: &str) -> Result<(), String> {
    let path = own_dir().join("computers.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or(json!([]));
    let left: Vec<Value> = v.as_array().into_iter().flatten().filter(|c| c["id"].as_str() != Some(id)).cloned().collect();
    crate::sys::write_private(&path, serde_json::to_string_pretty(&left).unwrap_or_default().as_bytes()).map_err(|e| format!("could not unpair it: {e}"))?;
    if left.is_empty() {
        for f in ["tls-cert.der", "tls-key.der"] {
            std::fs::remove_file(own_dir().join(f)).ok();
        }
    }
    Ok(())
}

/// Settings' one line for it.
pub fn summary() -> String {
    if !enabled() {
        return "off".into();
    }
    match computers().len() {
        0 => "on".into(),
        1 => "1 computer".into(),
        n => format!("{n} computers"),
    }
}

// ---------------------------------------------------------------- pairing ---

/// A new six-digit code, shown until it is used, it runs out, or the page
/// stops it.
pub fn start_pairing() -> String {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).ok();
    let code = format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000);
    let expires = chrono::Utc::now().timestamp() as u64 + CODE_FOR.as_secs();
    std::fs::create_dir_all(run_dir()).ok();
    std::fs::remove_file(run_dir().join("companion-paired")).ok();
    crate::sys::write_private(&run_dir().join("companion-pair"), format!("{code}\n{expires}\n").as_bytes()).ok();
    code
}

pub fn stop_pairing() {
    std::fs::remove_file(run_dir().join("companion-pair")).ok();
}

/// How the pairing ended, once it has: the computer's name, or refused.
fn pairing_result() -> Option<Result<String, ()>> {
    let text = std::fs::read_to_string(run_dir().join("companion-paired")).ok()?;
    Some(text.trim().strip_prefix("paired\t").map(str::to_owned).ok_or(()))
}

// ----------------------------------------------------------------- socket ---

/// What a computer sent for the look and the team, for the next frame.
static PENDING: Mutex<Option<remote::Answers>> = Mutex::new(None);

/// The look and the team a computer sent, put in place with the settings
/// in hand (`Shell::tick`). True if anything was.
pub fn apply_pending(robot: &mut Robot, settings: &mut Settings) -> bool {
    let Some(a) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take() else { return false };
    if let Some(team) = &a.team {
        crate::setup::save_team(team, robot);
    }
    crate::setup::apply_look(&a, settings);
    true
}

/// The team as kept, if it is a team number.
fn team() -> Option<String> {
    let kept = std::fs::read_to_string(crate::config_path()).ok()?;
    let kept = kept.trim();
    (!kept.is_empty() && kept.len() <= 5 && kept.chars().all(|c| c.is_ascii_digit())).then(|| kept.to_owned())
}

/// What the handheld has now, for the app to show (keys only as Settings
/// shows them: whether set, and their last characters).
pub fn now_json() -> Value {
    let mut now = remote::Now::read();
    let pending = PENDING.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // Sent and not yet put in place (the shell in standby): as it will be.
    if let Some(a) = &pending {
        now.accent = a.accent.unwrap_or(now.accent);
        now.light = a.light.unwrap_or(now.light);
        now.text = a.text.unwrap_or(now.text);
        now.home = a.home.unwrap_or(now.home);
        now.open_robot = a.open_robot.unwrap_or(now.open_robot);
        now.match_report = a.match_report.unwrap_or(now.match_report);
        now.calm = a.calm.unwrap_or(now.calm);
    }
    let keys = Keys::load();
    let shown: serde_json::Map<String, Value> = SERVICES.iter().map(|(name, _)| ((*name).to_owned(), json!(keys.shown(name)))).collect();
    let accents: Vec<String> = bezel::Accent::ALL.iter().map(|a| a.name().to_lowercase()).collect();
    let texts: Vec<&str> = [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large].iter().map(|t| t.name()).collect();
    let homes: Vec<&str> = [crate::shell::HomeStyle::Grid, crate::shell::HomeStyle::Carousel, crate::shell::HomeStyle::Flow].iter().map(|h| h.name()).collect();
    json!({
        "name": crate::device::name(),
        "team": pending.and_then(|a| a.team).or_else(team),
        "accent": now.accent.name().to_lowercase(),
        "mode": if now.light { "light" } else { "dark" },
        "text": now.text.name(),
        "home": now.home.name(),
        "open_robot": now.open_robot, "match_report": now.match_report, "ssh": now.ssh, "calm": now.calm,
        "keys": shown,
        "music": now.music,
        "github": crate::github::account().map(|a| a.login),
        "accents": accents,
        "texts": texts,
        "homes": homes,
    })
}

/// One request from the service: `command\nbody`.
pub fn answer(request: &str) -> Result<Value, String> {
    let (command, body) = request.split_once('\n').unwrap_or((request, ""));
    match command.trim() {
        "now" => Ok(now_json()),
        "apply" => {
            let form = remote::parse_form(body);
            // YouTube Music's cookies checked as the setup page checks them, and why not said.
            remote::cookies_given(&form)?;
            let a = remote::answers(&form);
            let received = remote::received(&a);
            let github = a.keys.get("github").is_some();
            let mut keys = Keys::load();
            if let Some(why) = crate::setup::apply_now(&a, &mut keys) {
                return Err(why);
            }
            if github {
                // Whose key it is, for Git › Account (asked of GitHub, away from here).
                std::thread::Builder::new().name("github-sign-in".into()).spawn(|| crate::github::sign_in().ok()).ok();
            }
            let look = a.team.is_some() || a.accent.is_some() || a.light.is_some() || a.text.is_some() || a.home.is_some() || a.open_robot.is_some();
            if look {
                *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(a);
                crate::shell::settings_changed();
                // A moment for the next frame to put it in place, so what the
                // app asks next is what the handheld has (in standby it waits).
                let until = Instant::now() + Duration::from_millis(1500);
                while !cfg!(test) && Instant::now() < until && PENDING.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            Ok(json!({ "received": received }))
        }
        "forget" if body.trim() == "github" => {
            let mut keys = Keys::load();
            keys.set("github", "");
            keys.save().map_err(|e| format!("could not forget the key: {e}"))?;
            crate::github::sign_out();
            Ok(json!({ "forgotten": "github" }))
        }
        "music-sign-out" => crate::music::sign_out().map(|()| json!({ "signed_out": true })),
        "music-rescan" => {
            crate::music::rescan_library();
            Ok(json!({ "looking": true }))
        }
        other => Err(format!("{other} is not something the shell does")),
    }
}

fn reply(request: &[u8]) -> String {
    match answer(&String::from_utf8_lossy(request)) {
        Ok(v) => format!("ok\n{v}"),
        Err(why) => format!("error {why}\n"),
    }
}

/// Listen for the service, on a thread that sleeps until asked: a socket
/// only root can open on the handheld; on a computer, for the simulator,
/// the loopback address in `CATALYST_COMPANION_SHELL` if it is set.
pub fn serve() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let path = run_dir().join("companion.sock");
        std::fs::create_dir_all(run_dir()).ok();
        std::fs::remove_file(&path).ok();
        let Ok(listener) = UnixListener::bind(&path) else { return };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
        std::thread::Builder::new()
            .name("companion-socket".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    let mut request = Vec::new();
                    if (&mut stream).take(REQUEST_MOST).read_to_end(&mut request).is_ok() {
                        stream.write_all(reply(&request).as_bytes()).ok();
                    }
                }
            })
            .ok();
    }
    #[cfg(not(unix))]
    {
        let Some(addr) = std::env::var("CATALYST_COMPANION_SHELL").ok().and_then(|a| a.parse::<std::net::SocketAddr>().ok()).filter(|a| a.ip().is_loopback()) else { return };
        let Ok(listener) = std::net::TcpListener::bind(addr) else { return };
        std::thread::Builder::new()
            .name("companion-socket".into())
            .spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    let mut request = Vec::new();
                    if (&mut stream).take(REQUEST_MOST).read_to_end(&mut request).is_ok() {
                        stream.write_all(reply(&request).as_bytes()).ok();
                    }
                }
            })
            .ok();
    }
}

// ------------------------------------------------------------------- page ---

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Switch,
    Pair,
    Computer(Computer),
    Found,
}

#[derive(Clone, Debug, PartialEq)]
enum Showing {
    Code { code: String, until: Instant },
    Paired(String),
    Refused,
    RanOut,
}

pub struct CompanionScreen {
    list: ListState,
    rows: Vec<Row>,
    /// The computer Enter was pressed on once, to unpair on the second.
    armed: Option<String>,
    /// The code, or how pairing went, over the page.
    showing: Option<Showing>,
    /// The code's card coming up (0 → 1), and when it was shown.
    card: Animated,
    shown_at: Instant,
    /// Read again now and then (the computers, the addresses).
    read_at: Instant,
    seconds_left: u64,
    changed: bool,
}

fn date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0).map(|d| d.with_timezone(&chrono::Local).format("%b %-d").to_string().to_lowercase()).unwrap_or_default()
}

/// Where the handheld is: its Wi-Fi address, or the USB cable's.
fn found_at() -> String {
    let addr = |iface: &str| crate::sys::address(iface).filter(|a| a.contains('.'));
    match (addr("wlan0"), addr("eth0")) {
        (Some(w), Some(_)) => format!("{w} · usb too"),
        (Some(w), None) => w,
        (None, Some(u)) => format!("usb {u}"),
        (None, None) if crate::sys::on_handheld() => "no network yet".into(),
        (None, None) => "this computer".into(),
    }
}

impl CompanionScreen {
    pub fn new() -> Self {
        let mut s = CompanionScreen {
            list: ListState::default(),
            rows: Vec::new(),
            armed: None,
            showing: None,
            card: Animated::new(0.0, role::release()).with_rest(0.002),
            shown_at: Instant::now(),
            read_at: Instant::now(),
            seconds_left: 0,
            changed: true,
        };
        s.read();
        s
    }

    /// Opened at one of its rows (from search).
    pub fn at(mut self, row: usize) -> Self {
        self.list.focus = row.min(self.rows.len() - 1);
        self
    }

    /// The simulator's shots: the code showing, then the outcome.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_for_shots(mut self, code: Option<&str>, paired: Option<&str>) -> Self {
        self.showing = match (code, paired) {
            (Some(code), _) => Some(Showing::Code { code: code.into(), until: Instant::now() + CODE_FOR }),
            (None, Some(name)) => Some(Showing::Paired(name.into())),
            (None, None) => Some(Showing::Refused),
        };
        self.seconds_left = CODE_FOR.as_secs() - 8;
        self.card.jump(1.0);
        self
    }

    fn read(&mut self) {
        let mut rows = vec![Row::Switch, Row::Pair];
        if enabled() {
            rows.extend(computers().into_iter().map(Row::Computer));
            rows.push(Row::Found);
        }
        if rows != self.rows {
            self.rows = rows;
            self.list.focus = self.list.focus.min(self.rows.len() - 1);
            self.changed = true;
        }
        self.read_at = Instant::now();
    }

    fn show(&mut self, showing: Option<Showing>) {
        if showing.is_some() && self.showing.is_none() {
            self.card.jump(0.0);
            self.card.set(1.0);
        }
        self.showing = showing;
        self.shown_at = Instant::now();
        self.changed = true;
    }

    fn pair(&mut self, cx: &mut Cx) {
        if !enabled() {
            if let Err(why) = switch(true) {
                *cx.say = Some(why);
                return;
            }
            self.read();
        }
        let code = start_pairing();
        self.show(Some(Showing::Code { code, until: Instant::now() + CODE_FOR }));
    }

    fn describe(&self, row: &Row) -> (String, String, bool) {
        match row {
            Row::Switch => ("Computer companion".into(), (if enabled() { "on" } else { "off" }).into(), enabled()),
            Row::Pair => ("Pair a computer".into(), String::new(), true),
            Row::Computer(c) if self.armed.as_deref() == Some(c.id.as_str()) => (c.name.clone(), "enter: unpair".into(), true),
            Row::Computer(c) => (c.name.clone(), format!("paired {}", date(c.paired)), true),
            Row::Found => ("Found at".into(), found_at(), false),
        }
    }

    fn hint(&self, row: &Row) -> &'static [&'static str] {
        match (row, &self.showing) {
            (_, Some(Showing::Code { .. })) => &["esc: stop · the code is good for 5 minutes", "esc: stop"],
            (_, Some(Showing::Paired(_))) => &["enter: done"],
            (_, Some(_)) => &["enter: a new code · esc: back", "enter: a new code"],
            (Row::Switch, _) => &["off, nothing on the network answers", "enter: on or off"],
            (Row::Pair, _) => &["catalyst companion for windows · from the releases", "enter: show a code"],
            (Row::Computer(c), _) if self.armed.as_deref() == Some(c.id.as_str()) => &["enter again: unpair · it must pair again", "enter again: unpair"],
            (Row::Computer(_), _) => &["enter twice: unpair", "enter twice: unpair"],
            (Row::Found, _) => &["the app finds it by itself, wi-fi or usb", "found by itself"],
        }
    }

    fn draw_card(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, bottom) = (g.top(), g.above_hint());
        let a = self.card.value().clamp(0.0, 1.2);
        let lift = (1.0 - a) * 10.0;
        let card = Rect::new(PAD, top + 2.0 + lift, W - 2.0 * PAD, bottom - top - 4.0);
        g.shadow(card, g.radius(16.0), 0.5 * a.min(1.0));
        g.canvas.rrect(card, g.radius(16.0), t.surface2, a.min(1.0));
        let alpha = a.min(1.0);
        let lines = card.h;
        let inner = card.w - 28.0;
        // A line centred on the card, cut to fit it, fading in with it.
        let centred = |g: &mut Gfx, cy: f32, s: &str, style: TextStyle, color: bezel::Color| {
            let s = fit(g, s, style, inner);
            let w = g.measure(&s, style);
            g.text_alpha(card.cx() - w * 0.5, cy, &s, style, color, alpha);
        };
        match self.showing.clone() {
            Some(Showing::Code { code, .. }) => {
                centred(g, card.y + lines * 0.2, "Type this in Catalyst Companion", ty.body, t.dim);
                let big = TextStyle { tracking: 0.06, ..ty.display(34.0) };
                centred(g, card.y + lines * 0.52, &format!("{} {}", &code[..3], &code[3..]), big, t.ink);
                let left = format!("on your computer · {}:{:02} left", self.seconds_left / 60, self.seconds_left % 60);
                centred(g, card.y + lines * 0.84, &left, ty.caption, t.faint);
            }
            Some(Showing::Paired(name)) => {
                let good = g.good();
                g.icon_alpha("check_circle", true, card.cx(), card.y + lines * 0.36, 30.0, good, alpha);
                centred(g, card.y + lines * 0.66, &format!("Paired with {name}"), ty.name, t.ink);
                centred(g, card.y + lines * 0.85, "it can reach this handheld now", ty.caption, t.dim);
            }
            Some(other) => {
                let head = if other == Showing::Refused { "Too many wrong codes" } else { "The code ran out" };
                g.icon_alpha("error", false, card.cx(), card.y + lines * 0.36, 28.0, t.dim, alpha);
                centred(g, card.y + lines * 0.66, head, ty.name, t.ink);
                centred(g, card.y + lines * 0.85, "nothing was paired", ty.caption, t.dim);
            }
            None => {}
        }
    }
}

/// `s` cut with an ellipsis to fit `width`.
fn fit(g: &mut Gfx, s: &str, style: TextStyle, width: f32) -> String {
    if g.measure(s, style) <= width {
        return s.to_owned();
    }
    let mut cut: String = s.to_owned();
    while !cut.is_empty() && g.measure(&format!("{cut}…"), style) > width {
        cut.pop();
    }
    format!("{}…", cut.trim_end())
}

impl Screen for CompanionScreen {
    fn title(&self) -> &str {
        "Computer companion"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        match &self.showing {
            Some(Showing::Code { .. }) => return vec![keys("pairing", &[("esc", "Stop pairing")])],
            Some(Showing::Paired(_)) => return vec![keys("paired", &[("enter", "Done")])],
            Some(_) => return vec![keys("not paired", &[("enter", "Try again")])],
            None => {}
        }
        let here = match self.rows.get(self.list.focus.min(self.rows.len().saturating_sub(1))) {
            Some(Row::Switch) => Some(keys("companion", &[("enter · ← →", "On or off")])),
            Some(Row::Pair) => Some(keys("pairing", &[("enter", "Pair a computer")])),
            Some(Row::Computer(_)) => Some(keys("this computer", &[("enter twice", "Unpair it")])),
            _ => None,
        };
        here.into_iter().chain([keys("moving", crate::screens::LIST_KEYS)]).collect()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        self.changed = true;
        if let Some(showing) = self.showing.clone() {
            match (key, showing) {
                (Key::Back, Showing::Code { .. }) => {
                    stop_pairing();
                    self.show(None);
                }
                (Key::Enter | Key::Back, Showing::Paired(_)) => self.show(None),
                (Key::Enter, Showing::Refused | Showing::RanOut) => self.pair(cx),
                (Key::Back, _) => self.show(None),
                _ => {}
            }
            self.read();
            return Nav::Stay;
        }
        let row = self.rows[self.list.focus.min(self.rows.len() - 1)].clone();
        let armed = self.armed.take();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Left | Key::Right if row == Row::Switch => {
                let on = !enabled();
                match switch(on) {
                    Ok(()) => *cx.say = Some(format!("computer companion {}", if on { "on" } else { "off" })),
                    Err(why) => *cx.say = Some(why),
                }
                self.read();
            }
            Key::Enter if row == Row::Pair => self.pair(cx),
            Key::Enter => {
                if let Row::Computer(c) = row {
                    if armed.as_deref() == Some(c.id.as_str()) {
                        match unpair(&c.id) {
                            Ok(()) => *cx.say = Some(format!("{} unpaired", c.name)),
                            Err(why) => *cx.say = Some(why),
                        }
                        self.read();
                    } else {
                        self.armed = Some(c.id);
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.rows.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.card.tick(dt);
        if let Some(Showing::Code { until, .. }) = &self.showing {
            let left = until.saturating_duration_since(Instant::now()).as_secs();
            if left != self.seconds_left {
                self.seconds_left = left;
                self.changed = true;
            }
            match pairing_result() {
                Some(Ok(name)) => {
                    std::fs::remove_file(run_dir().join("companion-paired")).ok();
                    self.show(Some(Showing::Paired(name)));
                    self.read();
                }
                Some(Err(())) => {
                    std::fs::remove_file(run_dir().join("companion-paired")).ok();
                    self.show(Some(Showing::Refused));
                }
                None if left == 0 => {
                    stop_pairing();
                    self.show(Some(Showing::RanOut));
                }
                None => {}
            }
        }
        if matches!(self.showing, Some(Showing::Paired(_))) && self.shown_at.elapsed() > Duration::from_secs(3) {
            self.show(None);
        }
        if self.read_at.elapsed() > Duration::from_secs(2) {
            self.read();
        }
        moving |= std::mem::take(&mut self.changed);
        moving
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        let row = self.rows[self.list.focus.min(self.rows.len() - 1)].clone();
        if self.showing.is_some() {
            self.draw_card(g);
        } else {
            let rows = self.rows.clone();
            for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
                let (name, value, lit) = self.describe(&rows[i]);
                g.row(Rect::new(PAD, y, width, row_h), &name, &value, if lit { t.ink } else { t.dim }, i == self.list.focus);
            }
            self.list.scrollbar(g, rows.len(), top, bottom);
        }
        g.hint_any(self.hint(&row));
    }
}

/// Settings › Computer companion in the simulator (`CATALYST_SHOTS_ONLY=
/// companion catalyst-shell --shots DIR`): the page with a computer paired,
/// the code, paired and refused, in both looks and at each text size.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(own_dir()).ok();
    std::fs::remove_file(own_dir().join("off")).ok();
    std::fs::write(own_dir().join("computers.json"), r#"[{"id":"5a1c","name":"THOMAS-DESKTOP","hash":"-","paired":1763800000}]"#).ok();
    for (look, light, size, tag) in [
        (crate::look::Theme::Bezel, false, TextSize::Standard, "bezel-dark-standard"),
        (crate::look::Theme::Bezel, true, TextSize::Large, "bezel-light-large"),
        (crate::look::Theme::Bezel, false, TextSize::Compact, "bezel-dark-compact"),
        (crate::look::Theme::Catalyst, false, TextSize::Large, "catalyst-dark-large"),
    ] {
        for (name, page) in [
            ("1-page", CompanionScreen::new()),
            ("2-code", CompanionScreen::new().showing_for_shots(Some("482913"), None)),
            ("3-paired", CompanionScreen::new().showing_for_shots(None, Some("THOMAS-DESKTOP"))),
            ("4-refused", CompanionScreen::new().showing_for_shots(None, None)),
        ] {
            let mut shell = Shell::new(Robot::demo());
            (shell.settings.theme, shell.settings.light, shell.settings.text) = (look, light, size);
            shell.apply_settings();
            shell.show(Box::new(page));
            for _ in 0..90 {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
            shell.draw();
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("companion-{name}-{tag}.png")), 3);
        }
    }
    println!("companion shots in {}", dir.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The companion's folders emptied for one test at a time (the tests
    /// share them, and run side by side): held until the test ends.
    fn fresh() -> std::sync::MutexGuard<'static, ()> {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        let held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::remove_dir_all(own_dir()).ok();
        std::fs::remove_dir_all(run_dir()).ok();
        held
    }

    #[test]
    fn pairing_writes_a_code_for_the_service_and_reads_back_how_it_went() {
        let _held = fresh();
        let code = start_pairing();
        assert!(catalyst_ok(&code), "{code}");
        let file = std::fs::read_to_string(run_dir().join("companion-pair")).unwrap();
        assert!(file.starts_with(&format!("{code}\n")));
        assert_eq!(pairing_result(), None);
        std::fs::write(run_dir().join("companion-paired"), "paired\tTHOMAS-PC\n").unwrap();
        assert_eq!(pairing_result(), Some(Ok("THOMAS-PC".into())));
        std::fs::write(run_dir().join("companion-paired"), "refused\n").unwrap();
        assert_eq!(pairing_result(), Some(Err(())));
        stop_pairing();
        assert!(!run_dir().join("companion-pair").exists());
    }

    fn catalyst_ok(code: &str) -> bool {
        code.len() == 6 && code.chars().all(|c| c.is_ascii_digit())
    }

    #[test]
    fn unpairing_the_last_computer_lets_its_certificate_go() {
        let _held = fresh();
        std::fs::create_dir_all(own_dir()).unwrap();
        std::fs::write(own_dir().join("computers.json"), r#"[{"id":"a1","name":"THOMAS-PC","hash":"x","paired":1763800000},{"id":"b2","name":"LAPTOP","hash":"y","paired":1763800000}]"#).unwrap();
        std::fs::write(own_dir().join("tls-cert.der"), b"c").unwrap();
        assert_eq!(computers().len(), 2);
        assert_eq!(summary(), "2 computers");
        unpair("a1").unwrap();
        assert_eq!(computers().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["LAPTOP"]);
        assert!(std::fs::read_to_string(own_dir().join("computers.json")).unwrap().contains("\"hash\": \"y\""), "the others' tokens still work");
        assert!(own_dir().join("tls-cert.der").exists());
        unpair("b2").unwrap();
        assert!(!own_dir().join("tls-cert.der").exists());
    }

    #[test]
    fn the_service_is_answered_with_the_setup_pages_own_checks() {
        let now = answer("now").unwrap();
        assert!(now["keys"]["tba"].is_string() && now["accents"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(now.to_string().len() < 4096);
        let why = answer("apply\ncookies=not%20cookies").unwrap_err();
        assert!(why.contains("cookies.txt"), "{why}");
        let got = answer("apply\naccent=rose&team=5805").unwrap();
        assert_eq!(got["received"][0][0], "Team");
        assert!(PENDING.lock().unwrap().take().is_some_and(|a| a.team.as_deref() == Some("5805")), "the look and team wait for the next frame");
        assert!(answer("reboot").is_err());
    }

    #[test]
    fn every_row_hint_and_card_fits_at_every_text_size() {
        let _held = fresh();
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut s = CompanionScreen::new();
        s.rows.push(Row::Computer(Computer { id: "a".into(), name: "THOMAS-DESKTOP".into(), paired: 1_763_800_000 }));
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for row in s.rows.clone() {
                let (name, value, _) = s.describe(&row);
                assert!(g.measure(&name, ty.body) + g.measure(&value, ty.body) + 40.0 <= W - 2.0 * PAD, "{name} {value} at {size:?}");
                assert!(s.hint(&row).iter().any(|h| g.measure(h, ty.caption) <= W - 2.0 * PAD), "{row:?} at {size:?}");
            }
            let digits = TextStyle { tracking: 0.06, ..ty.display(34.0) };
            assert!(g.measure("482 913", digits) <= W - 2.0 * PAD - 28.0, "the code at {size:?}");
            assert!(g.measure("Type this in Catalyst Companion", ty.body) <= W - 2.0 * PAD - 28.0, "at {size:?}");
            assert!(g.measure("on your computer · 4:59 left", ty.caption) <= W - 2.0 * PAD - 28.0, "at {size:?}");
            assert!(g.measure("Too many wrong codes", ty.name) <= W - 2.0 * PAD - 28.0, "at {size:?}");
            // The card's three lines stand clear of each other between the title and the hint.
            let card_h = g.above_hint() - g.top() - 4.0;
            assert!(card_h * (0.52 - 0.2) >= ty.body.size * 0.5 + 34.0 * ty.display_scale * 0.5, "the caption clears the code at {size:?}");
            assert!(card_h * (0.84 - 0.52) >= ty.caption.size * 0.5 + 34.0 * ty.display_scale * 0.36, "the time clears the code at {size:?}");
        }
    }
}
