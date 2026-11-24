//! Setting the handheld up from a computer: for a few minutes it serves one
//! page on the local network (over the Ethernet cable or Wi-Fi), and shows
//! its address as text and a QR code with a one-time code beside it. The
//! computer fills the page in (team, Wi-Fi, keys, an SSH key, YouTube
//! Music's sign-in, the look and the choices) with a real keyboard, and
//! sends it; the handheld takes it from there.
//!
//! Nothing listens on the network except while this is showing, and then
//! for ten minutes at most. The code must be given with the page; five
//! wrong codes and it stops listening. What is sent is never written to a
//! log, and the cookies for YouTube Music never come back on a page.

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Accent, Color, Rect};

use crate::gfx::{Gfx, TextSize};
use crate::keys::{Keys, SERVICES};
use crate::lan::{read_request, respond, Limits, Request};
use crate::shell::HomeStyle;

pub use crate::lan::parse_form;

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
/// The most a form may send: the page's fields and a cookies.txt (64 KB at
/// most, as a file and pasted both), with room for the form's own wrapping.
const BODY_MOST: usize = 160 * 1024;
/// More than that is read and passed over up to this much, so the browser
/// is told why rather than finding the connection gone.
const PASSED_OVER_MOST: usize = 2 * 1024 * 1024;
const LIMITS: Limits = Limits { idle: IDLE_FOR, request_in: REQUEST_IN, body_most: BODY_MOST, passed_over_most: PASSED_OVER_MOST };

/// What a computer sent. Anything left empty on the page is `None`: left as
/// it is on the handheld.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answers {
    pub team: Option<String>,
    pub wifi: Vec<(String, Option<String>)>,
    pub keys: Keys,
    pub ssh_key: Option<String>,
    /// YouTube Music's sign-in: a browser's cookies.txt, checked.
    pub cookies: Option<crate::music::Jar>,
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
        // The cable first, then the Wi-Fi (lan.rs).
        let ip = crate::lan::address(&["eth0", "wlan0"])?;
        // Port 80 where it can be had, so the address is just the number.
        let (listener, port) = crate::lan::bind()?;
        let url = crate::lan::url(&ip, port);
        // Upper case is the QR code's compact alphabet: a smaller code, larger squares.
        let qr = qrcodegen::QrCode::encode_text(&url.to_uppercase(), qrcodegen::QrCodeEcc::Medium).ok()?;
        let code = crate::lan::one_time_code();
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
            // In deep standby, parked here: no wakeups until it wakes.
            crate::deep::rest();
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
    let Some(request) = read_request(&stream, &shared.stop, &LIMITS) else { return };
    respond(stream, reply(&request, shared, code));
}

/// What a request is answered with: the page, the page with what went
/// wrong, or (the form taken) the page saying so.
fn reply(request: &Request, shared: &Shared, code: &str) -> (&'static str, &'static str, String) {
    match (request.method.as_str(), request.path.as_str()) {
        // Sent already: a second press of the button is told so, not refused.
        ("POST", "/") if shared.answers.lock().unwrap().is_some() => done_page(),
        _ if shared.stop.load(Ordering::Relaxed) && shared.answers.lock().unwrap().is_none() => {
            page(Some("The page has closed. Start setup from a computer again on the handheld."))
        }
        ("GET", "/") => {
            set_status(shared, "a computer has the page open");
            page(None)
        }
        ("POST", "/") if request.too_large => page(Some("That was more than the page takes. A cookies.txt is a few KB: export the cookies for music.youtube.com alone, and send it again.")),
        ("POST", "/") => {
            let form = parse_body(&request.kind, &request.body);
            let given = form.iter().find(|(k, _)| k == "code").map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
            if given == code {
                // Cookies that would not sign in: said so, and nothing taken
                // yet, so the page can be sent again with them put right.
                match cookies_given(&form) {
                    Err(why) => {
                        set_status(shared, "the cookies sent would not sign in");
                        page(Some(&format!("Nothing was changed: the YouTube Music cookies would not sign in ({why}).")))
                    }
                    Ok(_) => {
                        *shared.answers.lock().unwrap() = Some(answers(&form));
                        set_status(shared, "received: putting it in place");
                        shared.stop.store(true, Ordering::Relaxed);
                        done_page()
                    }
                }
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
    }
}

/// A form's fields, as either kind of form sends them: urlencoded, or
/// `multipart/form-data` (the page's, so a file can come with it).
pub fn parse_body(kind: &str, body: &str) -> Vec<(String, String)> {
    let boundary = kind
        .split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("boundary="))
        .map(|b| b.trim_matches('"'))
        .filter(|_| kind.trim_start().to_ascii_lowercase().starts_with("multipart/form-data"));
    match boundary {
        Some(b) if !b.is_empty() => parse_multipart(b, body),
        _ => parse_form(body),
    }
}

/// `multipart/form-data` into name and value pairs: each part's name from
/// its Content-Disposition, its value what follows its headers (a file's
/// contents, for a file).
pub fn parse_multipart(boundary: &str, body: &str) -> Vec<(String, String)> {
    let delimiter = format!("--{boundary}");
    let mut out = Vec::new();
    for part in body.split(delimiter.as_str()).skip(1) {
        // The last delimiter ends "--".
        if part.starts_with("--") {
            break;
        }
        let part = part.strip_prefix("\r\n").or_else(|| part.strip_prefix('\n')).unwrap_or(part);
        let Some((headers, value)) = part.split_once("\r\n\r\n").or_else(|| part.split_once("\n\n")) else { continue };
        let value = value.strip_suffix("\r\n").or_else(|| value.strip_suffix('\n')).unwrap_or(value);
        let name = headers.lines().find(|l| l.to_ascii_lowercase().starts_with("content-disposition:")).and_then(|l| {
            l.split(';').map(str::trim).find_map(|p| p.strip_prefix("name=")).map(|n| n.trim_matches('"').to_owned())
        });
        if let Some(name) = name {
            out.push((name, value.to_owned()));
        }
    }
    out
}

/// The cookies.txt sent, chosen as a file or pasted (the file if both):
/// none, checked and taken, or why it would not sign in.
pub fn cookies_given(form: &[(String, String)]) -> Result<Option<crate::music::Jar>, String> {
    let given = ["cookies_file", "cookies"].iter().find_map(|name| form.iter().find(|(k, v)| k == name && !v.trim().is_empty()).map(|(_, v)| v.as_str()));
    match given {
        Some(text) => crate::music::check_cookies(text, chrono::Utc::now().timestamp()).map(Some),
        None => Ok(None),
    }
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
    // Cookies that would not sign in are none (the page has said why).
    a.cookies = cookies_given(form).ok().flatten();
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
    // That it came, and no more.
    if a.cookies.is_some() {
        rows.push(("YouTube Music".to_owned(), "signed in".to_owned()));
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

/// What the handheld has now, as the page shows it, so that the page sent
/// back as it came changes nothing. (It showed its own defaults instead:
/// sent to add one key, it turned SSH off and put the look back to grid,
/// dark and standard text.)
#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    pub accent: Accent,
    pub light: bool,
    pub text: TextSize,
    pub home: HomeStyle,
    pub open_robot: bool,
    pub match_report: bool,
    pub ssh: bool,
    pub calm: bool,
    /// The services a key is set for already.
    pub keys: Vec<&'static str>,
    /// Signed in to YouTube Music already.
    pub music: bool,
}

impl Now {
    /// Read afresh each time the page is asked for: from the saved settings,
    /// SSH as it is, and the keys file.
    pub fn read() -> Now {
        let s = crate::saved_settings();
        let keys = Keys::load();
        Now {
            accent: s.accent,
            light: s.light,
            text: s.text,
            home: s.home,
            open_robot: s.open_robot,
            match_report: s.match_report,
            ssh: crate::ssh_enabled(),
            calm: s.calm,
            keys: SERVICES.iter().filter(|(name, _)| keys.get(name).is_some()).map(|(name, _)| *name).collect(),
            music: crate::music::signed_in(),
        }
    }
}

/// Text made safe to put in the page.
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The page as a computer is given it, for the assistant's shots.
#[cfg(not(target_arch = "aarch64"))]
pub fn page_for_shots(now: &Now, error: Option<&str>) -> String {
    let error = error.map_or(String::new(), |e| format!("<p class=\"error\">{}</p>", escaped(e)));
    PAGE.replace("{{body}}", &form(now)).replace("{{error}}", &error)
}

fn page(error: Option<&str>) -> (&'static str, &'static str, String) {
    let error = error.map_or(String::new(), |e| format!("<p class=\"error\">{}</p>", escaped(e)));
    let body = form(&Now::read());
    ("200 OK", "text/html", PAGE.replace("{{body}}", &body).replace("{{error}}", &error))
}

/// The form, showing what the handheld has now: the look's choices chosen,
/// the boxes ticked as they are, and a key already set said to be.
fn form(now: &Now) -> String {
    let option = |value: &str, current: bool| format!("<option value=\"{value}\"{}>{value}</option>", if current { " selected" } else { "" });
    let accents: String = Accent::ALL.iter().map(|a| option(&a.name().to_lowercase(), *a == now.accent)).collect();
    let modes = option("dark", !now.light) + &option("light", now.light);
    let texts: String = [TextSize::Standard, TextSize::Compact, TextSize::Large].iter().map(|t| option(t.name(), *t == now.text)).collect();
    let homes: String = HomeStyle::ALL.iter().map(|h| option(h.name(), *h == now.home)).collect();
    let checked = |on: bool| if on { " checked" } else { "" };
    let mut form = FORM
        .replace("{{accents}}", &accents)
        .replace("{{modes}}", &modes)
        .replace("{{texts}}", &texts)
        .replace("{{homes}}", &homes)
        .replace("{{open_robot}}", checked(now.open_robot))
        .replace("{{match_report}}", checked(now.match_report))
        .replace("{{ssh}}", checked(now.ssh))
        .replace("{{calm}}", checked(now.calm));
    for (name, _) in SERVICES {
        let set = if now.keys.contains(&name) { " placeholder=\"set already: left empty, it is kept\"" } else { "" };
        form = form.replace(&format!("{{{{{name}}}}}"), set);
    }
    // Signed in already: said so, and kept unless other cookies are sent.
    let (state, placeholder) = if now.music {
        ("<p class=\"state\">Signed in already. Left empty, the sign-in is kept.</p>", " placeholder=\"signed in already: left empty, it is kept\"")
    } else {
        ("", " placeholder=\"# Netscape HTTP Cookie File …\"")
    };
    form.replace("{{music_state}}", state).replace("{{cookies}}", placeholder)
}

const DONE: &str = r#"<section class="done"><h1>That's everything.</h1><p>The handheld is putting it in place. You can close this page.</p></section>"#;

const FORM: &str = r#"
<form method="post" action="/" enctype="multipart/form-data" autocomplete="off">
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
    <label>The Blue Alliance <small>read API key</small><input name="tba" spellcheck="false"{{tba}}></label>
    <label>FRC Nexus <small>for live queueing</small><input name="nexus" spellcheck="false"{{nexus}}></label>
  </section>
  <section>
    <h2>Assistant</h2>
    <p class="hint">The assistant answers questions about the robot with either service; one key is enough. Each is paid for by use on the account it comes from.</p>
    <label>OpenAI <small>GPT-6.2 Sol · a key from platform.openai.com, API keys</small><input name="openai" spellcheck="false"{{openai}}></label>
    <label>Claude <small>Claude Opus 5.5 · a key from platform.claude.com, API keys</small><input name="claude" spellcheck="false"{{claude}}></label>
  </section>
  <section>
    <h2>Git</h2>
    <p class="hint">For Git on the handheld: your repositories, their history, pulling and pushing.</p>
    <label>GitHub <small>a token with repository access, from github.com, Settings, Developer settings</small><input name="github" spellcheck="false"{{github}}></label>
  </section>
  <section>
    <h2>YouTube Music</h2>
    <p class="hint">Sign in once, and your playlists, albums and artists are in Music on the handheld. On this computer, open music.youtube.com signed in and export its cookies as cookies.txt with a browser extension (Get cookies.txt LOCALLY in Chrome or Edge, cookies.txt in Firefox).</p>
    {{music_state}}
    <label>cookies.txt <small>the file</small><input type="file" name="cookies_file" accept=".txt,text/plain"></label>
    <label>Or its text <small>pasted</small><textarea name="cookies" rows="3" spellcheck="false" autocomplete="off"{{cookies}}></textarea></label>
    <p class="hint small">For a sign-in that lasts, export from a private window, then close it: a browser that stays on YouTube renews its cookies, and the copy stops working. Third-party players like this one can break YouTube's terms, which could put the account at risk.</p>
  </section>
  <section>
    <h2>SSH</h2>
    <label>Your public key <small>ssh-ed25519 …</small><textarea name="ssh_key" rows="2" spellcheck="false"></textarea></label>
  </section>
  <section>
    <h2>Choices</h2>
    <p class="hint">As the handheld has them now.</p>
    <input type="hidden" name="choices" value="1">
    <label class="check"><input type="checkbox" name="open_robot"{{open_robot}}> Open Robot when one is plugged in</label>
    <label class="check"><input type="checkbox" name="match_report"{{match_report}}> Show how the last match went</label>
    <label class="check"><input type="checkbox" name="ssh"{{ssh}}> SSH access</label>
    <label class="check"><input type="checkbox" name="calm"{{calm}}> Calm version: less motion, no transparency</label>
  </section>
  <section>
    <h2>Look</h2>
    <p class="hint">As the handheld has it now.</p>
    <div class="quad">
      <label>Accent<select name="accent">{{accents}}</select></label>
      <label>Mode<select name="mode">{{modes}}</select></label>
      <label>Text<select name="text">{{texts}}</select></label>
      <label>Home<select name="home">{{homes}}</select></label>
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
.hint{color:var(--thin);margin:-4px 0 12px}.hint.small{font-size:13px;margin:0}
.state{margin:-4px 0 12px;font-weight:500}.state::before{content:"";display:inline-block;width:8px;height:8px;border-radius:50%;background:#4cc38a;margin-right:8px;vertical-align:1px}
input[type=file]{padding:8px 10px;color:var(--thin)}input[type=file]::file-selector-button{font:inherit;color:var(--ink);background:var(--line);border:0;border-radius:8px;padding:6px 12px;margin-right:12px;cursor:pointer}
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
    use std::io::{Read, Write};

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
    fn the_page_shows_what_the_handheld_has_so_sent_as_it_came_it_changes_nothing() {
        let now = Now {
            accent: Accent::Blue,
            light: true,
            text: TextSize::Large,
            home: HomeStyle::Flow,
            open_robot: false,
            match_report: true,
            ssh: true,
            calm: false,
            keys: vec!["claude"],
            music: false,
        };
        let html = form(&now);
        // The look as it is, one choice each.
        for chosen in ["value=\"blue\" selected", "value=\"light\" selected", "value=\"large\" selected", "value=\"flow\" selected"] {
            assert!(html.contains(chosen), "{chosen}");
        }
        assert_eq!(html.matches(" selected>").count(), 4);
        // The boxes ticked as they are: SSH stays on.
        assert!(html.contains("name=\"ssh\" checked") && html.contains("name=\"match_report\" checked"));
        assert!(!html.contains("name=\"open_robot\" checked") && !html.contains("name=\"calm\" checked"));
        // A key set already is said to be; the others are not.
        assert!(html.contains("name=\"claude\" spellcheck=\"false\" placeholder=\"set already"));
        assert!(html.contains("name=\"openai\" spellcheck=\"false\">"));
        assert!(!html.contains("{{"), "every blank filled");
        // What a browser sends for the page as it came, a key pasted in:
        // the look and choices as they were, the other keys left alone.
        let sent = answers(&parse_form("code=123456&tba=&nexus=&openai=sk-new&claude=&github=&choices=1&match_report=on&ssh=on&accent=blue&mode=light&text=large&home=flow"));
        assert_eq!((sent.accent, sent.light, sent.text, sent.home), (Some(now.accent), Some(now.light), Some(now.text), Some(now.home)));
        assert_eq!((sent.open_robot, sent.match_report, sent.ssh, sent.calm), (Some(now.open_robot), Some(now.match_report), Some(now.ssh), Some(now.calm)));
        assert_eq!((sent.keys.get("openai"), sent.keys.get("claude")), (Some("sk-new"), None), "the Claude key set already is kept");
    }

    use crate::music::tests::MADE_UP_JAR;

    /// The page's form as a browser sends it, a file and all.
    fn multipart(fields: &[(&str, Option<&str>, &str)]) -> (String, String) {
        let boundary = "----CatalystFormBoundary7MA4YWxkTrZu0gW";
        let mut body = String::new();
        for (name, file, value) in fields {
            body.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""));
            match file {
                Some(f) => body.push_str(&format!("; filename=\"{f}\"\r\nContent-Type: text/plain\r\n\r\n{value}\r\n")),
                None => body.push_str(&format!("\r\n\r\n{value}\r\n")),
            }
        }
        body.push_str(&format!("--{boundary}--\r\n"));
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    fn request(kind: &str, body: &str) -> Request {
        Request { method: "POST".into(), path: "/".into(), kind: kind.into(), body: body.into(), ..Request::default() }
    }

    #[test]
    fn a_form_with_a_file_reads_as_the_same_fields() {
        let (kind, body) = multipart(&[("code", None, "123456"), ("team", None, "5805"), ("ssh_key", None, ""), ("cookies_file", Some("cookies.txt"), &MADE_UP_JAR.replace('\n', "\r\n")), ("cookies", None, ""), ("choices", None, "1"), ("ssh", None, "on")]);
        let form = parse_body(&kind, &body);
        let names: Vec<&str> = form.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["code", "team", "ssh_key", "cookies_file", "cookies", "choices", "ssh"]);
        let a = answers(&form);
        assert_eq!((a.team.as_deref(), a.ssh, a.open_robot), (Some("5805"), Some(true), Some(false)));
        assert!(a.cookies.is_some(), "the file signs in");
        // Not a file chosen: the text pasted. Neither: nothing changes.
        let (kind, body) = multipart(&[("code", None, "1"), ("cookies_file", Some(""), ""), ("cookies", None, MADE_UP_JAR)]);
        assert!(answers(&parse_body(&kind, &body)).cookies.is_some());
        let (kind, body) = multipart(&[("code", None, "1"), ("cookies_file", Some(""), ""), ("cookies", None, "")]);
        assert_eq!(cookies_given(&parse_body(&kind, &body)), Ok(None));
        // Urlencoded, as before (and as the tests send it).
        assert_eq!(parse_body("application/x-www-form-urlencoded", "team=254&code=1"), parse_form("team=254&code=1"));
    }

    #[test]
    fn the_sign_in_arrives_as_signed_in_and_nothing_more() {
        let a = answers(&[("cookies".into(), MADE_UP_JAR.into())]);
        let rows = received(&a);
        assert_eq!(rows, [("YouTube Music".to_owned(), "signed in".to_owned())]);
        // Nowhere in what the handheld shows, nor in the answers shown for a test.
        assert!(!format!("{rows:?}{a:?}").contains("made-up"), "{a:?}");
        // Cookies that would not sign in are none.
        assert_eq!(answers(&[("cookies".into(), "not cookies".into())]).cookies, None);
    }

    #[test]
    fn cookies_that_would_not_sign_in_leave_the_page_open_and_nothing_taken() {
        let shared = Shared::default();
        let (kind, body) = multipart(&[("code", None, "424242"), ("team", None, "5805"), ("cookies_file", Some("cookies.json"), "[{\"name\": \"SID\", \"value\": \"secret-in-json\"}]")]);
        let (status, _, page) = reply(&request(&kind, &body), &shared, "424242");
        assert_eq!(status, "200 OK");
        assert!(page.contains("Nothing was changed: the YouTube Music cookies would not sign in") && page.contains("JSON"), "said why");
        assert!(!page.contains("secret-in-json"), "never echoed");
        assert!(shared.answers.lock().unwrap().is_none() && !shared.stop.load(Ordering::Relaxed), "nothing taken, still offered");
        assert_eq!(shared.wrong.load(Ordering::Relaxed), 0, "not a wrong code");
        // Sent again put right: taken, the page done, and the cookies never on it.
        let (kind, body) = multipart(&[("code", None, "424242"), ("team", None, "5805"), ("cookies", None, MADE_UP_JAR)]);
        let (_, _, page) = reply(&request(&kind, &body), &shared, "424242");
        assert!(page.contains("That's everything") && !page.contains("made-up"));
        let taken = shared.answers.lock().unwrap().clone().unwrap();
        assert!(taken.cookies.is_some() && taken.team.as_deref() == Some("5805"));
        // Too much sent: said so.
        let shared = Shared::default();
        let (_, _, page) = reply(&Request { too_large: true, ..request("", "") }, &shared, "424242");
        assert!(page.contains("more than the page takes"));
        assert!(shared.answers.lock().unwrap().is_none());
    }

    #[test]
    fn a_cookies_txt_fits_what_the_page_takes_and_more_is_told_so() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = AtomicBool::new(false);
        let send = |body: Vec<u8>| {
            std::thread::spawn(move || {
                let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                let head = format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Type: multipart/form-data; boundary=b\r\nContent-Length: {}\r\n\r\n", body.len());
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(&body).ok();
                s
            })
        };
        // A large cookies.txt, as a file, is read whole.
        let jar = MADE_UP_JAR.repeat(crate::music::COOKIES_MOST / MADE_UP_JAR.len());
        let body = format!("--b\r\nContent-Disposition: form-data; name=\"cookies_file\"; filename=\"c.txt\"\r\n\r\n{jar}\r\n--b--\r\n");
        assert!(body.len() > 32 * 1024, "more than the old limit");
        let sender = send(body.into_bytes());
        let (stream, _) = listener.accept().unwrap();
        let got = read_request(&stream, &stop, &LIMITS).unwrap();
        assert!(!got.too_large);
        assert_eq!(parse_body(&got.kind, &got.body)[0].1, jar);
        drop(sender.join());
        // Far more: passed over, and said so rather than cut off.
        let sender = send(vec![b'x'; BODY_MOST + 1]);
        let (stream, _) = listener.accept().unwrap();
        let got = read_request(&stream, &stop, &LIMITS).unwrap();
        assert!(got.too_large && got.body.is_empty());
        drop(sender.join());
    }

    #[test]
    fn the_page_says_when_youtube_music_is_signed_in_already() {
        let mut now = Now { accent: Accent::Rose, light: false, text: TextSize::Standard, home: HomeStyle::Grid, open_robot: false, match_report: false, ssh: false, calm: false, keys: vec![], music: true };
        let html = form(&now);
        assert!(html.contains("Signed in already. Left empty, the sign-in is kept."));
        assert!(html.contains("name=\"cookies\" rows=\"3\" spellcheck=\"false\" autocomplete=\"off\" placeholder=\"signed in already: left empty, it is kept\""));
        assert!(html.contains("enctype=\"multipart/form-data\"") && html.contains("type=\"file\" name=\"cookies_file\""));
        now.music = false;
        let html = form(&now);
        assert!(!html.contains("signed in already") && !html.contains("Signed in already"));
        assert!(!html.contains("{{"), "every blank filled");
        // Sent as it came (nothing chosen, nothing pasted): the sign-in is left as it is.
        let (kind, body) = multipart(&[("code", None, "1"), ("cookies_file", Some(""), ""), ("cookies", None, "")]);
        assert_eq!(answers(&parse_body(&kind, &body)).cookies, None);
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
