//! Setting the handheld up from a computer: for a few minutes it serves one
//! page on the local network (over the Ethernet cable or Wi-Fi), and shows
//! its address as text and a QR code with a one-time code beside it. The
//! computer fills the page in (team, Wi-Fi, keys, an SSH key, the look and
//! the choices) with a real keyboard, and sends it; the handheld takes it
//! from there.
//!
//! Nothing listens on the network except while this is showing, and then
//! for ten minutes at most. The code must be given with the page; five
//! wrong codes and it stops listening.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Accent, Color, Rect};

use crate::gfx::{Gfx, TextSize};
use crate::keys::{Keys, SERVICES};
use crate::shell::HomeStyle;

/// How long the page is offered.
const OPEN_FOR: Duration = Duration::from_secs(600);
/// Wrong codes before it stops listening.
const TRIES: u32 = 5;
/// Connections looked after at once. A browser opens a spare connection or
/// two beside the one it uses, to have one ready for its next request.
const AT_ONCE: usize = 8;
/// How long a connection may wait before its request starts: a browser
/// sends the form down the spare connection it opened when the page came,
/// however long the form took to fill in.
const IDLE_FOR: Duration = Duration::from_secs(300);
/// And how long a request may take once it has started.
const REQUEST_IN: Duration = Duration::from_secs(10);

/// What a computer sent. Anything left empty on the page is `None`: left as
/// it is on the handheld.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answers {
    pub team: Option<String>,
    pub wifi: Vec<(String, Option<String>)>,
    pub keys: Keys,
    pub ssh_key: Option<String>,
    pub accent: Option<Accent>,
    pub light: Option<bool>,
    pub text: Option<TextSize>,
    pub home: Option<HomeStyle>,
    pub open_robot: Option<bool>,
    pub match_report: Option<bool>,
    pub ssh: Option<bool>,
    pub calm: Option<bool>,
}

#[derive(Default)]
struct Shared {
    status: Mutex<String>,
    answers: Mutex<Option<Answers>>,
    stop: AtomicBool,
    changed: AtomicBool,
    /// Wrong codes so far, and connections open now.
    wrong: AtomicU32,
    open: AtomicUsize,
}

pub struct Remote {
    shared: Arc<Shared>,
    url: String,
    code: String,
    qr: qrcodegen::QrCode,
}

impl Remote {
    /// Start offering the page. `None` when the handheld has no address on
    /// any network to offer it on.
    pub fn start() -> Option<Self> {
        let ip = address()?;
        // Port 80 where it can be had, so the address is just the number.
        let (listener, port) = TcpListener::bind(("0.0.0.0", 80)).map(|l| (l, 80)).or_else(|_| TcpListener::bind(("0.0.0.0", 8080)).map(|l| (l, 8080))).ok()?;
        listener.set_nonblocking(true).ok()?;
        let url = if port == 80 { format!("http://{ip}") } else { format!("http://{ip}:{port}") };
        // Upper case is the QR code's compact alphabet: a smaller code, larger squares.
        let qr = qrcodegen::QrCode::encode_text(&url.to_uppercase(), qrcodegen::QrCodeEcc::Medium).ok()?;
        let code = one_time_code();
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "waiting for a computer".into();
        let (thread_shared, thread_code) = (shared.clone(), code.clone());
        std::thread::Builder::new()
            .name("setup page".into())
            .stack_size(256 * 1024)
            .spawn(move || serve(listener, thread_shared, thread_code))
            .ok()?;
        Some(Remote { shared, url, code, qr })
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }

    /// Whether anything changed since last asked, so the page is redrawn.
    pub fn tick(&mut self) -> bool {
        self.shared.changed.swap(false, Ordering::Relaxed)
    }

    /// What a computer sent, once.
    pub fn take(&mut self) -> Option<Answers> {
        self.shared.answers.lock().unwrap().take()
    }

    /// The address as a QR code in `r`: dark squares on a white ground with
    /// the quiet margin a reader needs, each square a whole number of pixels.
    pub fn draw_qr(&self, g: &mut Gfx, r: Rect, alpha: f32) {
        draw_code(g, &self.qr, r, alpha);
    }
}

/// A QR code at the right of `r` (on the canvas, so with the page's slide
/// already added), centred down it: dark squares on a white ground with the
/// quiet margin a reader needs, each square a whole number of pixels.
pub fn draw_code(g: &mut Gfx, qr: &qrcodegen::QrCode, r: Rect, alpha: f32) {
    let n = qr.size();
    let cell = (r.w.min(r.h) / (n + 4) as f32).floor().max(1.0);
    let side = cell * (n + 4) as f32;
    let (x0, y0) = ((r.right() - side).round(), (r.y + (r.h - side) * 0.5).round());
    g.canvas.rrect(Rect::new(x0, y0, side, side), cell * 1.5, Color::WHITE, alpha);
    for y in 0..n {
        for x in 0..n {
            if qr.get_module(x, y) {
                let at = Rect::new(x0 + (x + 2) as f32 * cell, y0 + (y + 2) as f32 * cell, cell, cell);
                g.canvas.fill_rect(at, Color::hex(0x0d0e12), alpha);
            }
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// The handheld's address on whichever network it is on. Screenshots name
/// one of their own (`CATALYST_SETUP_ADDRESS`), not the computer's.
fn address() -> Option<String> {
    if let Ok(stand_in) = std::env::var("CATALYST_SETUP_ADDRESS") {
        return Some(stand_in);
    }
    for iface in ["eth0", "wlan0"] {
        if let Some(ip) = crate::sys::address(iface).filter(|a| a.contains('.')) {
            return Some(ip);
        }
    }
    // Off the handheld: the address this computer would reach others from.
    // Connecting a UDP socket sends nothing; it only picks the route.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Six digits from the process's random hashing keys, which the system
/// seeds afresh for every process.
fn one_time_code() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    format!("{:06}", h.finish() % 1_000_000)
}

fn set_status(shared: &Shared, text: &str) {
    *shared.status.lock().unwrap() = text.to_owned();
    shared.changed.store(true, Ordering::Relaxed);
}

/// Take connections while the page is offered, each looked after on a
/// thread of its own: a browser's spare connection waiting quietly for the
/// form never holds up the one the page or the form arrives on.
fn serve(listener: TcpListener, shared: Arc<Shared>, code: String) {
    let started = Instant::now();
    while !shared.stop.load(Ordering::Relaxed) {
        if started.elapsed() > OPEN_FOR {
            set_status(&shared, "the page has closed: start it again from Settings");
            shared.stop.store(true, Ordering::Relaxed);
            return;
        }
        let Ok((stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(60));
            continue;
        };
        if shared.open.load(Ordering::Relaxed) >= AT_ONCE {
            continue; // dropped: a browser opens another
        }
        shared.open.fetch_add(1, Ordering::Relaxed);
        let (shared, code) = (shared.clone(), code.clone());
        let spawned = std::thread::Builder::new().name("setup request".into()).stack_size(128 * 1024).spawn(move || {
            answer(stream, &shared, &code);
            shared.open.fetch_sub(1, Ordering::Relaxed);
        });
        if spawned.is_err() {
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

/// One connection: its request, and the reply.
fn answer(stream: TcpStream, shared: &Shared, code: &str) {
    let Some((method, path, body)) = read_request(&stream, &shared.stop) else { return };
    let reply = match (method.as_str(), path.as_str()) {
        // Sent already: a second press of the button is told so, not refused.
        ("POST", "/") if shared.answers.lock().unwrap().is_some() => done_page(),
        _ if shared.stop.load(Ordering::Relaxed) && shared.answers.lock().unwrap().is_none() => {
            page(Some("The page has closed. Start setup from a computer again on the handheld."))
        }
        ("GET", "/") => {
            set_status(shared, "a computer has the page open");
            page(None)
        }
        ("POST", "/") => {
            let form = parse_form(&body);
            let given = form.iter().find(|(k, _)| k == "code").map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
            if given == code {
                *shared.answers.lock().unwrap() = Some(answers(&form));
                set_status(shared, "received: putting it in place");
                shared.stop.store(true, Ordering::Relaxed);
                done_page()
            } else {
                let wrong = shared.wrong.fetch_add(1, Ordering::Relaxed) + 1;
                if wrong >= TRIES {
                    set_status(shared, "too many wrong codes: the page has closed");
                    shared.stop.store(true, Ordering::Relaxed);
                    page(Some("Too many wrong codes. Start setup from a computer again on the handheld."))
                } else {
                    set_status(shared, "a wrong code was given");
                    page(Some("That code is not the one on the handheld's screen."))
                }
            }
        }
        _ => ("404 Not Found", "text/plain", "not here".to_owned()),
    };
    respond(stream, reply);
}

/// One request: method, path, and body, read with a size limit. A
/// connection may wait quietly for a while before its request starts (a
/// browser's spare one), checking now and then that the page is still
/// offered; once a request has started it must arrive whole in a few
/// seconds, so a client sending a byte now and then cannot hold a thread.
fn read_request(mut stream: &TcpStream, stop: &AtomicBool) -> Option<(String, String, String)> {
    stream.set_nonblocking(false).ok();
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    // The first byte: up to IDLE_FOR, a second at a time.
    let idle_until = Instant::now() + IDLE_FOR;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
    let first = loop {
        match stream.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => break n,
            // Nothing yet: wait on, unless the page has closed (or the form
            // came on another connection) or it has waited long enough.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if Instant::now() > idle_until || stop.load(Ordering::Relaxed) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    };
    data.extend_from_slice(&buf[..first]);
    let deadline = Instant::now() + REQUEST_IN;
    let read = |stream: &mut &TcpStream, buf: &mut [u8]| -> Option<usize> {
        let left = deadline.checked_duration_since(Instant::now())?;
        stream.set_read_timeout(Some(left.max(Duration::from_millis(1)))).ok();
        stream.read(buf).ok()
    };
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if data.len() > 16 * 1024 {
            return None;
        }
        let n = read(&mut stream, &mut buf)?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if data.len() > 16 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut first = head.lines().next()?.split_whitespace();
    let (method, path) = (first.next()?.to_owned(), first.next()?.split('?').next()?.to_owned());
    let length = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().ok())).flatten().unwrap_or(0);
    if length > 32 * 1024 {
        return None;
    }
    while data.len() < head_end + length {
        let n = read(&mut stream, &mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let body = String::from_utf8_lossy(&data[head_end..(head_end + length).min(data.len())]).into_owned();
    Some((method, path, body))
}

fn respond(mut stream: TcpStream, (status, kind, body): (&str, &str, String)) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(body.as_bytes()).ok();
}

/// `application/x-www-form-urlencoded` into name and value pairs.
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    let decode = |s: &str| {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < bytes.len() => {
                    match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                        Ok(b) => {
                            out.push(b);
                            i += 2;
                        }
                        Err(_) => out.push(b'%'),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    body.split('&').filter(|p| !p.is_empty()).map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k), decode(v))
    }).collect()
}

/// What the form says, field by field; blank fields change nothing.
pub fn answers(form: &[(String, String)]) -> Answers {
    let field = |name: &str| form.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_owned()).filter(|v| !v.is_empty());
    let checked = |name: &str| Some(form.iter().any(|(k, v)| k == name && v == "on"));
    let mut a = Answers { team: field("team").filter(|t| t.len() <= 5 && t.chars().all(|c| c.is_ascii_digit())), ..Answers::default() };
    for i in 1..=2 {
        if let Some(ssid) = form.iter().find(|(k, _)| *k == format!("ssid{i}")).map(|(_, v)| v.clone()).filter(|v| !v.trim().is_empty()) {
            let password = form.iter().find(|(k, _)| *k == format!("psk{i}")).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
            a.wifi.push((ssid, password));
        }
    }
    for (name, _) in SERVICES {
        if let Some(value) = field(name) {
            a.keys.set(name, &value);
        }
    }
    a.ssh_key = field("ssh_key").filter(|k| ["ssh-ed25519 ", "ssh-rsa ", "ecdsa-sha2-"].iter().any(|p| k.starts_with(p)) && !k.contains('\n'));
    a.accent = field("accent").and_then(|v| Accent::from_name(&v));
    a.light = field("mode").map(|v| v == "light");
    a.text = field("text").and_then(|v| TextSize::from_name(&v));
    a.home = field("home").and_then(|v| HomeStyle::from_name(&v));
    // A form sends a checkbox only when it is ticked; the page marks that it was sent.
    if field("choices").is_some() {
        a.open_robot = checked("open_robot");
        a.match_report = checked("match_report");
        a.ssh = checked("ssh");
        a.calm = checked("calm");
    }
    a
}

/// What arrived, a row each, as the handheld shows it being put in place:
/// a name and a short value (keys shown only by their last characters).
pub fn received(a: &Answers) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(team) = &a.team {
        rows.push(("Team".to_owned(), team.clone()));
    }
    for (ssid, password) in &a.wifi {
        rows.push(("Wi-Fi network".to_owned(), format!("{ssid}{}", if password.is_some() { "" } else { " · open" })));
    }
    for (name, label) in SERVICES {
        if a.keys.get(name).is_some() {
            rows.push((format!("{label} key"), a.keys.shown(name)));
        }
    }
    if let Some(key) = &a.ssh_key {
        let kind = key.split_whitespace().next().unwrap_or("").trim_start_matches("ssh-");
        rows.push(("SSH key".to_owned(), kind.to_owned()));
    }
    let look: Vec<String> = [
        a.accent.map(|v| v.name().to_lowercase()),
        a.light.map(|l| (if l { "light" } else { "dark" }).to_owned()),
        a.text.map(|t| t.name().to_owned()),
        a.home.map(|h| h.name().to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !look.is_empty() {
        rows.push(("Look".to_owned(), look.join(" · ")));
    }
    if a.open_robot.is_some() {
        let on = [a.open_robot, a.match_report, a.ssh, a.calm].iter().filter(|v| **v == Some(true)).count();
        rows.push(("Choices".to_owned(), format!("{on} of 4 on")));
    }
    if rows.is_empty() {
        rows.push(("Nothing to change".to_owned(), String::new()));
    }
    rows
}

fn done_page() -> (&'static str, &'static str, String) {
    ("200 OK", "text/html", PAGE.replace("{{body}}", DONE).replace("{{error}}", ""))
}

fn page(error: Option<&str>) -> (&'static str, &'static str, String) {
    let error = error.map_or(String::new(), |e| format!("<p class=\"error\">{e}</p>"));
    let accents: String = Accent::ALL.iter().map(|a| format!("<option value=\"{0}\">{0}</option>", a.name().to_lowercase())).collect();
    let body = FORM.replace("{{accents}}", &accents);
    ("200 OK", "text/html", PAGE.replace("{{body}}", &body).replace("{{error}}", &error))
}

const DONE: &str = r#"<section class="done"><h1>That's everything.</h1><p>The handheld is putting it in place. You can close this page.</p></section>"#;

const FORM: &str = r#"
<form method="post" action="/" autocomplete="off">
  <section>
    <h2>Code</h2>
    <label>The six digits on the handheld's screen
      <input name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required autofocus></label>
  </section>
  <section>
    <h2>Robot</h2>
    <label>Team number <small>the robot is then at 10.TE.AM.2</small>
      <input name="team" inputmode="numeric" pattern="[0-9]{1,5}" maxlength="5"></label>
  </section>
  <section>
    <h2>Wi-Fi</h2>
    <p class="hint">For updates and the event schedule. The robot only needs its cable.</p>
    <div class="pair"><label>Network<input name="ssid1"></label><label>Password<input name="psk1" type="password"></label></div>
    <div class="pair"><label>Another network<input name="ssid2"></label><label>Password<input name="psk2" type="password"></label></div>
  </section>
  <section>
    <h2>Keys for online services</h2>
    <label>The Blue Alliance <small>read API key</small><input name="tba" spellcheck="false"></label>
    <label>FRC Nexus <small>for live queueing</small><input name="nexus" spellcheck="false"></label>
  </section>
  <section>
    <h2>Assistant</h2>
    <p class="hint">The assistant answers questions about the robot with either service; one key is enough. Each is paid for by use on the account it comes from.</p>
    <label>OpenAI <small>GPT-6.2 Sol · a key from platform.openai.com, API keys</small><input name="openai" spellcheck="false"></label>
    <label>Claude <small>Claude Opus 5.5 · a key from platform.claude.com, API keys</small><input name="claude" spellcheck="false"></label>
  </section>
  <section>
    <h2>Git</h2>
    <p class="hint">For Git on the handheld: your repositories, their history, pulling and pushing.</p>
    <label>GitHub <small>a token with repository access, from github.com, Settings, Developer settings</small><input name="github" spellcheck="false"></label>
  </section>
  <section>
    <h2>SSH</h2>
    <label>Your public key <small>ssh-ed25519 …</small><textarea name="ssh_key" rows="2" spellcheck="false"></textarea></label>
  </section>
  <section>
    <h2>Choices</h2>
    <input type="hidden" name="choices" value="1">
    <label class="check"><input type="checkbox" name="open_robot" checked> Open Robot when one is plugged in</label>
    <label class="check"><input type="checkbox" name="match_report" checked> Show how the last match went</label>
    <label class="check"><input type="checkbox" name="ssh"> SSH access</label>
    <label class="check"><input type="checkbox" name="calm"> Calm version: less motion, no transparency</label>
  </section>
  <section>
    <h2>Look</h2>
    <div class="quad">
      <label>Accent<select name="accent">{{accents}}</select></label>
      <label>Mode<select name="mode"><option value="dark">dark</option><option value="light">light</option></select></label>
      <label>Text<select name="text"><option value="standard">standard</option><option value="compact">compact</option><option value="large">large</option></select></label>
      <label>Home<select name="home"><option value="grid">grid</option><option value="carousel">carousel</option></select></label>
    </div>
  </section>
  <button type="submit">Send to the handheld</button>
</form>"#;

const PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Set up Catalyst Linux</title>
<style>
:root{--ground:#0d0e12;--card:#16171c;--line:#2b2d35;--ink:#f4f4f6;--thin:#8d919a;--crimson:#e94560}
*{box-sizing:border-box}
body{margin:0;background:radial-gradient(120% 80% at 20% 0%,#1b1c22 0%,var(--ground) 60%);color:var(--ink);font:15px/1.45 "Segoe UI Variable Text","Segoe UI",system-ui,-apple-system,sans-serif;min-height:100vh}
main{max-width:640px;margin:0 auto;padding:40px 20px 64px}
header{display:flex;align-items:center;gap:14px;margin-bottom:28px}
header svg{width:44px;height:44px;flex:none}
.word{font-size:26px;letter-spacing:-.02em}.word b{font-weight:700}.word span{font-weight:300;color:var(--thin)}
.rule{height:1px;background:linear-gradient(90deg,var(--crimson),transparent);margin:-14px 0 28px}
section{background:var(--card);border:1px solid var(--line);border-radius:16px;padding:18px 20px;margin:0 0 14px}
h2{font-size:13px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:var(--thin);margin:0 0 12px}
label{display:block;margin:0 0 12px;font-weight:500}
label small{font-weight:400;color:var(--thin);margin-left:6px}
input,select,textarea{display:block;width:100%;margin-top:6px;background:#0f1014;border:1px solid var(--line);border-radius:10px;color:var(--ink);font:inherit;padding:10px 12px}
textarea{font-family:ui-monospace,"Cascadia Code",monospace;font-size:13px;resize:vertical}
input:focus,select:focus,textarea:focus{outline:2px solid var(--ink);outline-offset:1px;border-color:transparent}
.check{display:flex;align-items:center;gap:10px;font-weight:400}.check input{width:18px;height:18px;margin:0;accent-color:var(--crimson)}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}.quad{display:grid;grid-template-columns:repeat(4,1fr);gap:12px}
@media(max-width:520px){.pair,.quad{grid-template-columns:1fr 1fr}}
.hint{color:var(--thin);margin:-4px 0 12px}
button{width:100%;margin-top:8px;padding:14px;border:0;border-radius:999px;background:var(--crimson);color:#fff;font:600 16px/1 inherit;font-family:inherit;cursor:pointer}
button:focus-visible{outline:2px solid var(--ink);outline-offset:3px}
.error{background:#3a1a22;border:1px solid #6e2a3a;color:#ffd7de;border-radius:12px;padding:12px 16px}
.done{text-align:center;padding:40px 20px}.done h1{font-size:28px;margin:0 0 8px}
</style></head><body><main>
<header><svg viewBox="0 0 512 512" aria-hidden="true"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#191a20"/><stop offset=".55" stop-color="#101116"/><stop offset="1" stop-color="#08090b"/></linearGradient><radialGradient id="l"><stop offset="0" stop-color="#e94560" stop-opacity=".3"/><stop offset="1" stop-color="#e94560" stop-opacity="0"/></radialGradient></defs><rect width="512" height="512" rx="114" fill="url(#g)"/><circle cx="256" cy="256" r="196" fill="url(#l)"/><path transform="translate(88 88) scale(14)" d="M13.5 2 5 13h5l-1.5 9L19 10h-5.5z" fill="#e94560"/></svg>
<div class="word"><b>Catalyst</b> <span>Linux</span></div></header>
<div class="rule"></div>
{{error}}
{{body}}
</main></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_becomes_answers_and_blank_fields_change_nothing() {
        let body = "code=123456&team=5805&ssid1=Pit+%225805%22&psk1=pa%26ss&ssid2=&psk2=&tba=abc%3D%3D&nexus=&openai=&claude=sk-ant-k1&ssh_key=ssh-ed25519+AAAAC3+me%40laptop&choices=1&open_robot=on&ssh=on&accent=blue&mode=light&text=large&home=carousel";
        let a = answers(&parse_form(body));
        assert_eq!(a.team.as_deref(), Some("5805"));
        assert_eq!(a.wifi, [("Pit \"5805\"".to_owned(), Some("pa&ss".to_owned()))]);
        assert_eq!(a.keys.get("tba"), Some("abc=="));
        assert_eq!(a.keys.get("nexus"), None);
        assert_eq!((a.keys.get("openai"), a.keys.get("claude")), (None, Some("sk-ant-k1")), "the assistant's keys come with the rest");
        assert_eq!(a.ssh_key.as_deref(), Some("ssh-ed25519 AAAAC3 me@laptop"));
        assert_eq!((a.open_robot, a.match_report, a.ssh, a.calm), (Some(true), Some(false), Some(true), Some(false)));
        assert_eq!((a.accent, a.light, a.text, a.home), (Some(Accent::Blue), Some(true), Some(TextSize::Large), Some(HomeStyle::Carousel)));
        // Something that is not a public key is not taken as one; a team must be a number.
        let a = answers(&parse_form("ssh_key=rm+-rf+%2F&team=abc"));
        assert_eq!((a.ssh_key, a.team), (None, None));
        assert_eq!(a.open_robot, None, "no choices section sent: choices left alone");
    }

    #[test]
    fn the_page_is_served_and_only_the_right_code_is_taken() {
        let Some(mut remote) = Remote::start() else { return }; // no network here
        let port = remote.url().rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(80);
        let send = |request: String| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).ok();
            out
        };
        assert!(send("GET / HTTP/1.1\r\nHost: x\r\n\r\n".into()).contains("Set up Catalyst Linux"));
        let post = |code: &str| {
            let body = format!("code={code}&team=254");
            format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len())
        };
        assert!(send(post("000000x")).contains("not the one"));
        assert!(remote.take().is_none());
        let code = remote.code().to_owned();
        assert!(send(post(&code)).contains("That's everything"));
        assert_eq!(remote.take().and_then(|a| a.team).as_deref(), Some("254"));
    }

    #[test]
    fn a_browsers_spare_connection_carries_the_form_however_long_it_waited() {
        let Some(mut remote) = Remote::start() else { return }; // no network here
        let port = remote.url().rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(80);
        // As a browser does: the page on one connection, and a spare opened
        // beside it that sits quiet while the form is filled in.
        let mut spare = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut page = TcpStream::connect(("127.0.0.1", port)).unwrap();
        page.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        page.read_to_string(&mut out).ok();
        assert!(out.contains("Set up Catalyst Linux"), "the page comes while the spare waits");
        // Longer than the old six-second limit, which closed the spare.
        std::thread::sleep(Duration::from_secs(7));
        let body = format!("code={}&team=5805&ssid1=Workshop&psk1=hunter22", remote.code());
        let request = format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        // Sent in two parts, as the head and the body often go.
        let (head, rest) = request.split_at(request.len() - body.len());
        spare.write_all(head.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        spare.write_all(rest.as_bytes()).unwrap();
        let mut out = String::new();
        spare.read_to_string(&mut out).ok();
        assert!(out.contains("That's everything"), "the form's reply: {}", &out[..out.len().min(120)]);
        let got = remote.take().unwrap();
        assert_eq!(got.team.as_deref(), Some("5805"));
        assert_eq!(got.wifi, vec![("Workshop".to_owned(), Some("hunter22".to_owned()))]);
    }
}
