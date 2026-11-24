//! The gallery on the network: who may in, what each address answers, and
//! the photos streamed from the card a piece at a time.
//!
//! The way in is `/g/<token>` (the QR code's link) or `/g?c=<six digits>`
//! (typed): either is traded for a cookie and sent on to `/`. With the
//! cookie:
//!
//! | Address | Answer |
//! | --- | --- |
//! | `/` | the page, the album's list in it |
//! | `/list.json` | the list again, when the page comes back to the front |
//! | `/t/<name>` | its thumbnail (the album's own, `.thumbs`) as a small JPEG |
//! | `/p/<name>` | the file as it is, byte ranges too (a phone plays video by them) |
//! | `/d/<name>` | the same, to be saved |
//! | `/zip/all`, `/zip/selected` | a ZIP streamed (zip.rs); the selection is `POST /select` |
//! | `/ping` | still here (the page asks every 15 s while it shows) |
//! | `/f/sans.ttf`, `/f/code.ttf` | the handheld's two type voices |
//!
//! A name is served only when the album's listing has it, and only from
//! the album's folder; nothing else on the card is reachable.

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::page::{self, Look};
use super::{zip, Item};
use crate::lan::{self, Limits, Request};

/// How long it serves after it was last used.
pub const OPEN_FOR: Duration = Duration::from_secs(600);
/// Wrong codes before it stops.
pub const TRIES: u32 = 5;
/// Connections looked after at once; more wait their turn.
pub const AT_ONCE: usize = 8;
/// Phones let in at once: a ninth takes the place of the one heard from
/// longest ago.
const PHONES_MOST: usize = 8;
/// A phone counts as connected while its page has been heard from this
/// recently (it says so every 15 s while it shows).
pub const HEARD_WITHIN: Duration = Duration::from_secs(45);
/// How long the album's listing is kept before it is read again.
const LISTING_KEPT: Duration = Duration::from_secs(3);
/// A phone that stops taking what is sent is let go after this.
const WRITE_WAIT: Duration = Duration::from_secs(20);
const LIMITS: Limits = Limits { idle: Duration::from_secs(15), request_in: Duration::from_secs(10), body_most: 64 * 1024, passed_over_most: 256 * 1024 };
const COOKIE: &str = "catalyst_gallery";

/// Why sharing stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// Esc on the handheld.
    ByYou,
    /// Ten minutes without use.
    TimeUp,
    /// Five wrong codes.
    WrongCodes,
    /// The sheet was left (Home, another app).
    Left,
}

/// Why sharing could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// No address on the Wi-Fi or the cable.
    NoNetwork,
    /// Neither port 80 nor 8080 could be had.
    NoPort,
    /// No thread to listen on.
    NoThread,
}

struct Session {
    id: String,
    seen: Instant,
    selection: Vec<String>,
}

pub(super) struct Shared {
    pub(super) stop: AtomicBool,
    token: String,
    code: String,
    wrong: AtomicU32,
    open: AtomicUsize,
    sending: AtomicUsize,
    sessions: Mutex<Vec<Session>>,
    deadline: Mutex<Instant>,
    open_for: Duration,
    stopped: Mutex<Option<Stopped>>,
    changed: AtomicBool,
    streams: Mutex<Vec<(u64, TcpStream)>>,
    next_stream: AtomicU64,
    album: PathBuf,
    listing: Mutex<Option<(Instant, Arc<Vec<Item>>)>>,
    look: Look,
    /// One thumbnail made at a time (each is a photo decoded at an eighth).
    making: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 26 letters and digits of base 32: 130 random bits, all in a QR code's
/// compact alphabet.
fn token() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bytes = [0u8; 26];
    lan::random_bytes(&mut bytes);
    bytes.iter().map(|b| ALPHABET[(b & 31) as usize] as char).collect()
}

fn session_id() -> String {
    let mut bytes = [0u8; 16];
    lan::random_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Shared {
    pub(super) fn new(album: PathBuf, look: Look, open_for: Duration) -> Shared {
        Shared {
            stop: AtomicBool::new(false),
            token: token(),
            code: lan::one_time_code(),
            wrong: AtomicU32::new(0),
            open: AtomicUsize::new(0),
            sending: AtomicUsize::new(0),
            sessions: Mutex::new(Vec::new()),
            deadline: Mutex::new(Instant::now() + open_for),
            open_for,
            stopped: Mutex::new(None),
            changed: AtomicBool::new(true),
            streams: Mutex::new(Vec::new()),
            next_stream: AtomicU64::new(1),
            album,
            listing: Mutex::new(None),
            look,
            making: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub(super) fn secrets(&self) -> (&str, &str) {
        (&self.token, &self.code)
    }

    /// Stop taking requests: those under way end at their next piece.
    fn halt(&self, why: Stopped) {
        lock(&self.stopped).get_or_insert(why);
        self.stop.store(true, Ordering::Relaxed);
        self.changed.store(true, Ordering::Relaxed);
    }

    /// Stop at once: every connection closed now, mid-photo or not.
    fn stop_now(&self, why: Stopped) {
        self.halt(why);
        for (_, stream) in lock(&self.streams).iter() {
            stream.shutdown(Shutdown::Both).ok();
        }
    }

    fn renew(&self) {
        *lock(&self.deadline) = Instant::now() + self.open_for;
        self.changed.store(true, Ordering::Relaxed);
    }

    fn left(&self) -> Duration {
        lock(&self.deadline).saturating_duration_since(Instant::now())
    }

    fn phones(&self) -> usize {
        lock(&self.sessions).iter().filter(|s| s.seen.elapsed() < HEARD_WITHIN).count()
    }

    /// A phone let in: its cookie's value.
    fn let_in(&self) -> String {
        let id = session_id();
        let mut sessions = lock(&self.sessions);
        if sessions.len() >= PHONES_MOST {
            if let Some(oldest) = sessions.iter().enumerate().min_by_key(|(_, s)| s.seen).map(|(i, _)| i) {
                sessions.remove(oldest);
            }
        }
        sessions.push(Session { id: id.clone(), seen: Instant::now(), selection: Vec::new() });
        self.changed.store(true, Ordering::Relaxed);
        id
    }

    /// Whether a cookie is one this sharing gave, noting that it was heard from.
    fn heard_from(&self, id: &str) -> bool {
        let mut sessions = lock(&self.sessions);
        let mut found = false;
        for s in sessions.iter_mut() {
            if lan::same(&s.id, id) {
                if s.seen.elapsed() >= HEARD_WITHIN {
                    self.changed.store(true, Ordering::Relaxed);
                }
                s.seen = Instant::now();
                found = true;
            }
        }
        found
    }

    fn track(&self, stream: &TcpStream) -> Option<u64> {
        let id = self.next_stream.fetch_add(1, Ordering::Relaxed);
        let clone = stream.try_clone().ok()?;
        lock(&self.streams).push((id, clone));
        Some(id)
    }

    fn untrack(&self, id: Option<u64>) {
        if let Some(id) = id {
            lock(&self.streams).retain(|(i, _)| *i != id);
        }
    }

    /// The album as its listing has it now (read again after a few seconds).
    pub(super) fn listing(&self) -> Arc<Vec<Item>> {
        let mut kept = lock(&self.listing);
        if let Some((at, items)) = kept.as_ref() {
            if at.elapsed() < LISTING_KEPT {
                return items.clone();
            }
        }
        let fresh = Arc::new(super::items(&self.album));
        *kept = Some((Instant::now(), fresh.clone()));
        fresh
    }

    fn find(&self, name: &str) -> Option<Item> {
        self.listing().iter().find(|i| i.photo.name == name).cloned()
    }
}

/// Sharing, while it lasts: dropped, it stops at once.
pub struct Gallery {
    shared: Arc<Shared>,
    address: String,
    /// The code's link, token and all: in the QR code, the tests and the screenshots.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    link: String,
    qr: qrcodegen::QrCode,
}

impl Gallery {
    /// Start sharing the album at `album` on the Wi-Fi (or the cable).
    pub fn start(album: PathBuf, look: Look) -> Result<Gallery, Trouble> {
        // The Wi-Fi first: a phone is on it, or is it (its hotspot).
        let ip = lan::address_on(&["wlan0", "eth0"]).or_else(|| (!crate::sys::on_handheld()).then(lan::routed_address).flatten()).ok_or(Trouble::NoNetwork)?;
        let (listener, port) = lan::bind().ok_or(Trouble::NoPort)?;
        Self::serve_on(listener, &ip, port, album, look, OPEN_FOR)
    }

    /// Serve on `listener` (not blocking), reached at `ip`:`port`.
    pub(super) fn serve_on(listener: TcpListener, ip: &str, port: u16, album: PathBuf, look: Look, open_for: Duration) -> Result<Gallery, Trouble> {
        let shared = Arc::new(Shared::new(album, look, open_for));
        let address = if port == 80 { ip.to_owned() } else { format!("{ip}:{port}") };
        let link = format!("{}/g/{}", lan::url(ip, port), shared.token);
        // Upper case is the QR code's compact alphabet: a smaller code, larger squares.
        let qr = qrcodegen::QrCode::encode_text(&link.to_uppercase(), qrcodegen::QrCodeEcc::Medium).map_err(|_| Trouble::NoPort)?;
        let thread_shared = shared.clone();
        std::thread::Builder::new().name("phone gallery".into()).stack_size(128 * 1024).spawn(move || take(listener, thread_shared)).map_err(|_| Trouble::NoThread)?;
        Ok(Gallery { shared, address, link, qr })
    }

    /// What a phone types: the address, with its port when not 80.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The six digits typed with it.
    pub fn code(&self) -> &str {
        &self.shared.code
    }

    /// The code's link, token and all (the QR code).
    pub fn qr(&self) -> &qrcodegen::QrCode {
        &self.qr
    }

    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub(super) fn link(&self) -> &str {
        &self.link
    }

    /// As it is mid-way (the screenshots): photos on their way, and this
    /// long left.
    #[cfg(not(target_arch = "aarch64"))]
    pub(super) fn pretend(&self, sending: bool, left: Duration) {
        self.shared.sending.store(usize::from(sending), Ordering::Relaxed);
        *lock(&self.shared.deadline) = Instant::now() + left;
    }

    #[cfg(test)]
    pub(super) fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Phones with the page open.
    pub fn phones(&self) -> usize {
        self.shared.phones()
    }

    /// Photos on their way to a phone now.
    pub fn sending(&self) -> bool {
        self.shared.sending.load(Ordering::Relaxed) > 0
    }

    /// How long until it stops by itself.
    pub fn left(&self) -> Duration {
        self.shared.left()
    }

    pub fn open_for(&self) -> Duration {
        self.shared.open_for
    }

    /// Ten more minutes from now.
    pub fn renew(&self) {
        self.shared.renew();
    }

    pub fn stop(&self, why: Stopped) {
        self.shared.stop_now(why);
    }

    pub fn stopped(&self) -> Option<Stopped> {
        *lock(&self.shared.stopped)
    }

    /// Whether anything changed since last asked.
    pub fn tick(&self) -> bool {
        self.shared.changed.swap(false, Ordering::Relaxed)
    }
}

impl Drop for Gallery {
    fn drop(&mut self) {
        self.shared.stop_now(Stopped::Left);
    }
}

/// Take connections while sharing lasts, each looked after on a thread of
/// its own; past `AT_ONCE`, the next waits its turn in the listener's queue.
fn take(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        if shared.left().is_zero() {
            shared.halt(Stopped::TimeUp);
            return;
        }
        if shared.open.load(Ordering::Relaxed) >= AT_ONCE {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let Ok((stream, _)) = listener.accept() else {
            // In deep standby, parked here: no wakeups until it wakes.
            crate::deep::rest();
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        shared.open.fetch_add(1, Ordering::Relaxed);
        let s = shared.clone();
        let spawned = std::thread::Builder::new().name("gallery request".into()).stack_size(256 * 1024).spawn(move || {
            answer(stream, &s);
            s.open.fetch_sub(1, Ordering::Relaxed);
        });
        if spawned.is_err() {
            shared.open.fetch_sub(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

/// One connection: its request, and the reply.
fn answer(stream: TcpStream, shared: &Shared) {
    // Below the frames: a photo on its way never costs the panel a beat.
    crate::sched::enter(crate::sched::Class::Background);
    stream.set_write_timeout(Some(WRITE_WAIT)).ok();
    let id = shared.track(&stream);
    if let Some(request) = lan::read_request(&stream, &shared.stop, &LIMITS) {
        let reply = route(&request, shared);
        send(&stream, &request, reply, shared);
    }
    shared.untrack(id);
}

/// What a reply carries.
pub(super) enum Body {
    Bytes(Vec<u8>),
    Static(&'static [u8]),
    /// A file's bytes from `from`, `len` of them.
    File { path: PathBuf, from: u64, len: u64 },
    Zip(zip::Plan),
}

pub(super) struct Reply {
    pub status: &'static str,
    pub kind: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Reply {
    fn new(status: &'static str, kind: &'static str, body: impl Into<Vec<u8>>) -> Reply {
        Reply { status, kind, headers: vec![("Cache-Control", "no-store".into())], body: Body::Bytes(body.into()) }
    }

    fn text(status: &'static str, words: &str) -> Reply {
        Reply::new(status, "text/plain; charset=utf-8", words.as_bytes())
    }

    fn html(status: &'static str, html: String) -> Reply {
        let mut r = Reply::new(status, "text/html; charset=utf-8", html);
        r.headers.push(("Content-Security-Policy", page::CSP.into()));
        r.headers.push(("X-Frame-Options", "DENY".into()));
        r
    }

    pub(super) fn len(&self) -> u64 {
        match &self.body {
            Body::Bytes(b) => b.len() as u64,
            Body::Static(b) => b.len() as u64,
            Body::File { len, .. } => *len,
            Body::Zip(plan) => plan.len(),
        }
    }

    #[cfg(test)]
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The bytes of a reply held whole (the tests).
    #[cfg(test)]
    pub(super) fn bytes(&self) -> &[u8] {
        match &self.body {
            Body::Bytes(b) => b,
            Body::Static(b) => b,
            _ => &[],
        }
    }
}

fn not_found() -> Reply {
    Reply::text("404 Not Found", "Not in the album.")
}

/// A browser may keep what does not change: each address carries the
/// file's size, so a photo changed is a new address.
fn kept_a_day() -> (&'static str, String) {
    ("Cache-Control", "private, max-age=86400".into())
}

/// Whether a request names the handheld as an address (or `.local`), as a
/// phone reaching it does: a page elsewhere that had its own name point here
/// (DNS rebinding) is refused.
pub(super) fn host_ok(host: Option<&str>) -> bool {
    let Some(host) = host.map(str::trim) else { return true };
    let name = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        match host.rsplit_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host,
        }
    };
    let lower = name.to_ascii_lowercase();
    name.parse::<std::net::IpAddr>().is_ok() || lower == "localhost" || lower.ends_with(".local")
}

fn cookie_of(req: &Request) -> Option<&str> {
    req.header("cookie")?.split(';').find_map(|p| p.trim().strip_prefix(COOKIE).and_then(|rest| rest.strip_prefix('=')))
}

/// A name as the album could have it: one file in its folder, no more.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0'])
}

/// What a request is answered with.
pub(super) fn route(req: &Request, shared: &Shared) -> Reply {
    if shared.stop.load(Ordering::Relaxed) {
        return Reply::html("410 Gone", page::closed_page(&shared.look, *lock(&shared.stopped)));
    }
    if !host_ok(req.header("host")) {
        return Reply::text("421 Misdirected Request", "Open this page by the address on the handheld's screen.");
    }
    let method = req.method.as_str();
    let reading = matches!(method, "GET" | "HEAD");
    let lower = req.path.to_ascii_lowercase();
    // The way in.
    if reading && (lower == "/g" || lower.starts_with("/g/")) {
        let given = req.path.get(3..).and_then(lan::decode_path).filter(|g| !g.is_empty()).or_else(|| req.query_field("c"));
        return enter(shared, given.as_deref().unwrap_or(""));
    }
    let Some(session) = cookie_of(req).filter(|id| shared.heard_from(id)).map(str::to_owned) else {
        return if reading && req.path == "/" { Reply::html("200 OK", page::code_page(&shared.look, None)) } else { Reply::text("403 Forbidden", "Scan the code on the handheld's screen.") };
    };
    // Read-only: looking, saving, and choosing what to save.
    if !reading && !(method == "POST" && req.path == "/select") {
        let mut r = Reply::text("405 Method Not Allowed", "This page only shows photos.");
        r.headers.push(("Allow", "GET, HEAD".into()));
        return r;
    }
    match req.path.as_str() {
        "/" => {
            shared.renew();
            Reply::html("200 OK", page::page(&shared.look, &shared.listing()))
        }
        "/list.json" => {
            shared.renew();
            Reply::new("200 OK", "application/json", page::items_json(&shared.look, &shared.listing()))
        }
        "/ping" => Reply { status: "204 No Content", kind: "", headers: vec![("Cache-Control", "no-store".into())], body: Body::Bytes(Vec::new()) },
        "/select" if method == "POST" => {
            shared.renew();
            select(shared, &session, &req.body)
        }
        "/zip/all" => {
            shared.renew();
            let all: Vec<Item> = shared.listing().iter().cloned().collect();
            let name = format!("{} photos {}.zip", shared.look.name, chrono::Local::now().format("%Y-%m-%d"));
            zip_reply(&all, &name)
        }
        "/zip/selected" => {
            shared.renew();
            let chosen: Vec<String> = lock(&shared.sessions).iter().find(|s| lan::same(&s.id, &session)).map(|s| s.selection.clone()).unwrap_or_default();
            let listing = shared.listing();
            let items: Vec<Item> = chosen.iter().filter_map(|n| listing.iter().find(|i| i.photo.name == *n).cloned()).collect();
            if items.is_empty() {
                return Reply::text("409 Conflict", "Choose the photos again: those chosen are no longer in the album.");
            }
            let name = format!("{} {} photos {}.zip", shared.look.name, items.len(), chrono::Local::now().format("%Y-%m-%d"));
            zip_reply(&items, &name)
        }
        "/f/sans.ttf" | "/f/code.ttf" => {
            let (sans, code) = bezel::text::font_files();
            let bytes = if req.path == "/f/sans.ttf" { sans } else { code };
            Reply { status: "200 OK", kind: "font/ttf", headers: vec![("Cache-Control", "private, max-age=31536000, immutable".into())], body: Body::Static(bytes) }
        }
        path => {
            let (Some(kind), Some(rest)) = (path.get(..3), path.get(3..)) else { return not_found() };
            if !matches!(kind, "/t/" | "/p/" | "/d/") {
                return not_found();
            }
            let Some(item) = lan::decode_path(rest).filter(|n| plain_name(n)).and_then(|n| shared.find(&n)) else { return not_found() };
            shared.renew();
            match kind {
                "/t/" => thumb(shared, &item),
                "/p/" => file(req, &item, false),
                _ => file(req, &item, true),
            }
        }
    }
}

/// The token or the six digits: a cookie, and on to the page; anything
/// else is a wrong code, and the fifth stops sharing.
fn enter(shared: &Shared, given: &str) -> Reply {
    let given: String = given.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if given.is_empty() {
        return Reply::html("200 OK", page::code_page(&shared.look, None));
    }
    let right = lan::same(&given.to_ascii_uppercase(), &shared.token) || lan::same(&given, &shared.code);
    if right {
        let id = shared.let_in();
        shared.renew();
        let mut r = Reply::text("303 See Other", "In.");
        r.headers.push(("Location", "/".into()));
        r.headers.push(("Set-Cookie", format!("{COOKIE}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age=3600")));
        return r;
    }
    let wrong = shared.wrong.fetch_add(1, Ordering::Relaxed) + 1;
    shared.changed.store(true, Ordering::Relaxed);
    if wrong >= TRIES {
        shared.halt(Stopped::WrongCodes);
        return Reply::html("403 Forbidden", page::closed_page(&shared.look, Some(Stopped::WrongCodes)));
    }
    Reply::html("403 Forbidden", page::code_page(&shared.look, Some("That isn't the code on the handheld's screen.")))
}

/// A thumbnail: the album's own (made now, and kept, if it has none yet),
/// as a JPEG of a few KB.
fn thumb(shared: &Shared, item: &Item) -> Reply {
    let jpeg = {
        let _one = lock(&shared.making);
        thumb_jpeg(&shared.album, item)
    };
    match jpeg {
        Some(jpeg) => Reply { status: "200 OK", kind: "image/jpeg", headers: vec![kept_a_day()], body: Body::Bytes(jpeg) },
        None => not_found(),
    }
}

/// The album's thumbnail of a photo as a JPEG.
pub(super) fn thumb_jpeg(album: &std::path::Path, item: &Item) -> Option<Vec<u8>> {
    let pic = crate::photos::thumbnail_in(album, &item.photo).filter(|p| !p.is_empty())?;
    let mut jpeg = Vec::with_capacity(8 * 1024);
    jpeg_encoder::Encoder::new(&mut jpeg, 88).encode(&pic.rgb, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Rgb).ok()?;
    Some(jpeg)
}

/// `Content-Disposition` for a file: shown, or saved, under its own name.
fn disposition(save: bool, name: &str) -> String {
    let plain: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("{}; filename=\"{plain}\"; filename*=UTF-8''{encoded}", if save { "attachment" } else { "inline" })
}

/// Which bytes of a file a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Span {
    Whole,
    /// From and to, both included.
    Part(u64, u64),
    Unsatisfiable,
}

/// A `Range` header, one range of bytes at most (several, or one not
/// understood, is the whole file, as RFC 9110 allows).
pub(super) fn span(header: Option<&str>, size: u64) -> Span {
    let Some(spec) = header.map(str::trim).and_then(|h| h.strip_prefix("bytes=")) else { return Span::Whole };
    if spec.contains(',') {
        return Span::Whole;
    }
    let Some((from, to)) = spec.split_once('-') else { return Span::Whole };
    let (from, to) = (from.trim(), to.trim());
    if from.is_empty() {
        // The last `n` bytes.
        let Ok(n) = to.parse::<u64>() else { return Span::Whole };
        if n == 0 || size == 0 {
            return Span::Unsatisfiable;
        }
        return Span::Part(size - n.min(size), size - 1);
    }
    let Ok(from) = from.parse::<u64>() else { return Span::Whole };
    if from >= size {
        return Span::Unsatisfiable;
    }
    let to = if to.is_empty() {
        size - 1
    } else {
        match to.parse::<u64>() {
            Ok(to) if to >= from => to.min(size - 1),
            _ => return Span::Whole,
        }
    };
    Span::Part(from, to)
}

/// A photo, a loop or a video as it is on the card, or the part asked for.
fn file(req: &Request, item: &Item, save: bool) -> Reply {
    let size = std::fs::metadata(&item.photo.path).map_or(item.photo.bytes, |m| m.len());
    let mut headers = vec![("Accept-Ranges", "bytes".to_owned()), kept_a_day(), ("Content-Disposition", disposition(save, &item.photo.name))];
    let path = item.photo.path.clone();
    match span(req.header("range"), size) {
        Span::Whole => Reply { status: "200 OK", kind: item.mime, headers, body: Body::File { path, from: 0, len: size } },
        Span::Part(from, to) => {
            headers.push(("Content-Range", format!("bytes {from}-{to}/{size}")));
            Reply { status: "206 Partial Content", kind: item.mime, headers, body: Body::File { path, from, len: to - from + 1 } }
        }
        Span::Unsatisfiable => {
            headers.push(("Content-Range", format!("bytes */{size}")));
            Reply { status: "416 Range Not Satisfiable", kind: "text/plain; charset=utf-8", headers, body: Body::Bytes(Vec::new()) }
        }
    }
}

/// The photos chosen on a phone, by name; only the album's are kept.
fn select(shared: &Shared, session: &str, body: &str) -> Reply {
    let listing = shared.listing();
    let known: std::collections::HashSet<&str> = listing.iter().map(|i| i.photo.name.as_str()).collect();
    let mut chosen: Vec<String> = Vec::new();
    for name in body.lines().map(str::trim) {
        if known.contains(name) && !chosen.iter().any(|c| c == name) {
            chosen.push(name.to_owned());
        }
    }
    if chosen.is_empty() {
        return Reply::text("400 Bad Request", "None of those are in the album.");
    }
    if let Some(s) = lock(&shared.sessions).iter_mut().find(|s| lan::same(&s.id, session)) {
        s.selection = chosen;
    }
    Reply { status: "204 No Content", kind: "", headers: vec![("Cache-Control", "no-store".into())], body: Body::Bytes(Vec::new()) }
}

fn zip_reply(items: &[Item], filename: &str) -> Reply {
    let entries = items
        .iter()
        .map(|i| zip::Entry { name: i.photo.name.clone(), path: i.photo.path.clone(), size: std::fs::metadata(&i.photo.path).map_or(i.photo.bytes, |m| m.len()), when: i.photo.taken })
        .collect();
    let plan = zip::Plan::new(entries);
    Reply {
        status: "200 OK",
        kind: "application/zip",
        headers: vec![("Cache-Control", "no-store".into()), ("Content-Disposition", disposition(true, filename))],
        body: Body::Zip(plan),
    }
}

/// Write a reply: its head, then its body, a piece at a time for a file
/// or a ZIP, ending at once if sharing stops.
pub(super) fn send(mut stream: &TcpStream, request: &Request, reply: Reply, shared: &Shared) {
    let mut head = format!("HTTP/1.1 {}\r\n", reply.status);
    if !reply.kind.is_empty() {
        head.push_str(&format!("Content-Type: {}\r\n", reply.kind));
    }
    head.push_str(&format!("Content-Length: {}\r\n", reply.len()));
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n");
    if stream.write_all(head.as_bytes()).is_err() || request.method == "HEAD" {
        return;
    }
    match reply.body {
        Body::Bytes(b) => {
            stream.write_all(&b).ok();
        }
        Body::Static(b) => {
            stream.write_all(b).ok();
        }
        Body::File { path, from, len } => {
            sending(shared, 1);
            stream_file(stream, &path, from, len, &shared.stop).ok();
            sending(shared, -1);
        }
        Body::Zip(plan) => {
            sending(shared, 1);
            let mut out = BufWriter::with_capacity(zip::CHUNK, stream);
            plan.write(&mut out, &shared.stop).ok();
            sending(shared, -1);
        }
    }
    stream.flush().ok();
}

fn sending(shared: &Shared, by: isize) {
    if by > 0 {
        shared.sending.fetch_add(1, Ordering::Relaxed);
    } else {
        shared.sending.fetch_sub(1, Ordering::Relaxed);
    }
    shared.changed.store(true, Ordering::Relaxed);
}

/// `len` bytes of a file from `from`, read and sent 64 KB at a time.
pub(super) fn stream_file(mut out: impl Write, path: &std::path::Path, from: u64, len: u64, stop: &AtomicBool) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut buf = vec![0u8; zip::CHUNK];
    let mut left = len;
    while left > 0 {
        if stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "sharing stopped"));
        }
        let n = file.read(&mut buf[..left.min(zip::CHUNK as u64) as usize])?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        left -= n as u64;
    }
    Ok(())
}
