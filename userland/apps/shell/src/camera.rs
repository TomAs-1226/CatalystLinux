//! The Camera: the IMX219's picture in colour, photos kept in the album
//! (Photos), and QR codes read.
//!
//! The viewfinder takes the panel's height at the photo's own shape (4:3),
//! with the controls in a rail beside it: the mode (photo or scan), the
//! shutter, and the last photo taken (↓ opens the album on it). Enter takes a
//! photo; `t` sets a timer (3 or 10 s), `g` lays a rule-of-thirds grid over
//! the picture, `n` lets the exposure run longer in low light.
//!
//! On the handheld the picture comes from `catalyst-camera --colour` (the
//! image's own libcamera program): a 320 × 240 YUV 4:2:0 preview at up to 30
//! frames a second, turned the right way up for the CardputerZero. A photo is
//! asked of it by name ("still /data/photos/IMG_….jpg"): it stops the
//! preview, takes the sensor's full 3280 × 2464 (or its binned 1640 × 1232
//! where the camera's memory will not stretch to the full one), writes the
//! JPEG through libjpeg-turbo straight from the camera's buffer, and carries
//! on. The preview is held meanwhile, under the shutter's flash.
//!
//! Scan mode reads QR codes from the preview's brightness, a few times a
//! second on a thread of its own: a Wi-Fi code offers to join the network, a
//! web address to open it in the Web app, anything else shows as text.
//!
//! On a computer, a simulated camera shows a scene of its own (and, in scan
//! mode, a Wi-Fi code on a card), and its photos are written by a JPEG
//! encoder in the shell, so everything works the same.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::photos::{self, draw_picture, whole, Photo, Picture};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The preview's size: the panel's height and a little more, at 4:3.
const PREVIEW_W: usize = 320;
const PREVIEW_H: usize = 240;
/// Reading QR codes: at most this often.
const SCAN_EVERY: Duration = Duration::from_millis(220);
/// How long a note over the picture stays.
const NOTE_FOR: f32 = 2.2;

// ------------------------------------------------------------------ codes ---

/// What a QR code holds, as the camera offers it.
#[derive(Clone, Debug, PartialEq)]
pub enum Code {
    /// A Wi-Fi network: its name, its password (none for an open one), and
    /// whether it hides its name.
    Wifi { ssid: String, password: Option<String>, hidden: bool },
    /// A web address.
    Url(String),
    Text(String),
}

impl Code {
    /// The card's title and line.
    fn words(&self) -> (&'static str, String, &'static str) {
        match self {
            Code::Wifi { ssid, password, .. } => ("wifi", format!("Wi-Fi · {ssid}"), if password.is_some() { "enter: join" } else { "enter: join · open network" }),
            Code::Url(url) => ("language", url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_owned(), "enter: open in Web"),
            Code::Text(text) => ("notes", text.lines().next().unwrap_or("").to_owned(), "enter: read it all"),
        }
    }
}

/// What a QR code's text is: a Wi-Fi network (`WIFI:T:WPA;S:name;P:pass;;`,
/// with `\` before any `;`, `,`, `:`, `\` or `"` in a value), a web address,
/// or text.
pub fn read_code(text: &str) -> Code {
    let trimmed = text.trim();
    if trimmed.len() > 5 && trimmed[..5].eq_ignore_ascii_case("WIFI:") {
        let mut fields: Vec<(String, String)> = Vec::new();
        let (mut key, mut value, mut in_value, mut escaped) = (String::new(), String::new(), false, false);
        for c in trimmed[5..].chars() {
            match (escaped, c) {
                (true, c) => {
                    value.push(c);
                    escaped = false;
                }
                (false, '\\') if in_value => escaped = true,
                (false, ':') if !in_value => in_value = true,
                (false, ';') => {
                    if in_value {
                        fields.push((std::mem::take(&mut key).to_ascii_uppercase(), std::mem::take(&mut value)));
                    }
                    key.clear();
                    in_value = false;
                }
                (false, c) if in_value => value.push(c),
                (false, c) => key.push(c),
            }
        }
        if in_value {
            fields.push((key.to_ascii_uppercase(), value));
        }
        let field = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.clone());
        if let Some(ssid) = field("S").filter(|s| !s.is_empty()) {
            let open = field("T").is_some_and(|t| t.eq_ignore_ascii_case("nopass"));
            let password = field("P").filter(|p| !p.is_empty() && !open);
            return Code::Wifi { ssid, password, hidden: field("H").is_some_and(|h| h.eq_ignore_ascii_case("true")) };
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://")) && !trimmed.contains(char::is_whitespace) && trimmed.len() > 8 {
        return Code::Url(trimmed.to_owned());
    }
    Code::Text(trimmed.to_owned())
}

/// A string for a shell command line, in single quotes.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A code read, where it was in the preview (corners, in its pixels).
#[derive(Clone, Debug)]
struct Found {
    code: Code,
    corners: [(f32, f32); 4],
}

// ----------------------------------------------------------------- source ---

/// What happened that the screen is told of.
#[derive(Debug)]
enum Event {
    Saved { path: PathBuf, size: (u32, u32) },
    Failed(String),
}

/// What the camera's thread hands the screen.
#[derive(Default)]
struct Live {
    preview: Option<Picture>,
    frames: u64,
    trouble: Option<String>,
    events: Vec<Event>,
    found: Option<Found>,
    found_count: u64,
    /// When a code was last read (any code).
    found_at: Option<Instant>,
}

/// The newest picture's brightness, for the QR reader.
#[derive(Default)]
struct Luma {
    y: Vec<u8>,
    fresh: bool,
    done: bool,
}

/// What the screen asks of the camera.
enum Order {
    Still(PathBuf),
    Night(bool),
}

/// Where orders go: the camera program's input, or the simulated camera.
enum Control {
    Program(Arc<Mutex<Option<ChildStdin>>>),
    Simulated(Sender<Order>),
}

impl Control {
    fn send(&self, order: Order) -> bool {
        match self {
            Control::Program(stdin) => {
                let line = match order {
                    Order::Still(path) => format!("still {}\n", path.display()),
                    Order::Night(on) => format!("night {}\n", if on { "on" } else { "off" }),
                };
                stdin.lock().ok().and_then(|mut s| s.as_mut().map(|s| s.write_all(line.as_bytes()).and_then(|()| s.flush()).is_ok())).unwrap_or(false)
            }
            Control::Simulated(tx) => tx.send(order).is_ok(),
        }
    }
}

/// YUV 4:2:0 (full range, as JPEG has it) into RGB, `w` × `h`.
pub fn yuv420_to_rgb(data: &[u8], w: usize, h: usize, out: &mut Picture) {
    out.w = w;
    out.h = h;
    out.rgb.resize(w * h * 3, 0);
    let (ys, us, vs) = (&data[..w * h], &data[w * h..w * h * 5 / 4], &data[w * h * 5 / 4..w * h * 3 / 2]);
    let cw = w / 2;
    for y in 0..h {
        for x in 0..w {
            let l = i32::from(ys[y * w + x]);
            let c = (y / 2) * cw + x / 2;
            let (u, v) = (i32::from(us[c]) - 128, i32::from(vs[c]) - 128);
            let at = (y * w + x) * 3;
            out.rgb[at] = (l + ((359 * v) >> 8)).clamp(0, 255) as u8;
            out.rgb[at + 1] = (l - ((88 * u + 183 * v) >> 8)).clamp(0, 255) as u8;
            out.rgb[at + 2] = (l + ((454 * u) >> 8)).clamp(0, 255) as u8;
        }
    }
}

/// One camera program at a time: a Camera opened again just after one was
/// closed waits a moment for the last program to let go of the sensor.
static CAMERA: Mutex<()> = Mutex::new(());

/// The camera program, read: frames and what it says, until it stops.
fn run_program(live: &Mutex<Live>, stop: &AtomicBool, slot: &Mutex<Option<ChildStdin>>, luma: &(Mutex<Luma>, Condvar)) {
    let waited = Instant::now();
    let _held = loop {
        match CAMERA.try_lock() {
            Ok(held) => break Some(held),
            Err(std::sync::TryLockError::Poisoned(held)) => break Some(held.into_inner()),
            Err(_) if waited.elapsed() < Duration::from_secs(4) && !stop.load(Ordering::Relaxed) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => break None,
        }
    };
    if stop.load(Ordering::Relaxed) {
        return finish_luma(luma);
    }
    crate::apriltags::ready_the_isp();
    let args = ["--colour", "--width", "320", "--height", "240", "--fps", "30", "--slowest", "15", "--orientation", "180"];
    let spawned = std::process::Command::new("catalyst-camera")
        .args(args)
        .env("LIBCAMERA_LOG_LEVELS", "*:WARN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            let why = if e.kind() == std::io::ErrorKind::NotFound { "no camera program in this system".into() } else { format!("the camera would not start: {e}") };
            live.lock().map(|mut l| l.trouble = Some(why)).ok();
            return finish_luma(luma);
        }
    };
    if let Ok(mut s) = slot.lock() {
        *s = child.stdin.take();
    }
    // What it says, read as it goes (a full pipe would stop it), the last
    // line kept for when it stops.
    let said: Arc<Mutex<String>> = Arc::default();
    let listening = child.stderr.take().and_then(|err| {
        let said = said.clone();
        std::thread::Builder::new()
            .name("camera-log".into())
            .spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        said.lock().map(|mut s| *s = line).ok();
                    }
                }
            })
            .ok()
    });
    let Some(mut out) = child.stdout.take() else { return finish_luma(luma) };
    let mut frame = Vec::new();
    let mut pic = Picture::default();
    let mut head = [0u8; 16];
    let trouble = loop {
        if stop.load(Ordering::Relaxed) {
            break None;
        }
        if out.read_exact(&mut head).is_err() {
            break Some(());
        }
        if &head[..4] != b"CCAM" {
            live.lock().map(|mut l| l.trouble = Some("the camera's picture is garbled".into())).ok();
            break None;
        }
        let word = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (kind, length, number) = (word(4), word(8) as usize, word(12));
        if length > 64 << 20 {
            break Some(());
        }
        frame.resize(length, 0);
        if out.read_exact(&mut frame).is_err() {
            break Some(());
        }
        let (w, h) = ((number >> 16) as usize, (number & 0xffff) as usize);
        match kind {
            1 if w * h * 3 / 2 == length && w > 0 => {
                yuv420_to_rgb(&frame, w, h, &mut pic);
                give_luma(luma, &frame[..w * h]);
                if let Ok(mut l) = live.lock() {
                    l.preview = Some(pic.clone());
                    l.frames += 1;
                    l.trouble = None;
                }
            }
            2 => {
                let path = PathBuf::from(String::from_utf8_lossy(&frame).into_owned());
                live.lock().map(|mut l| l.events.push(Event::Saved { path, size: (number >> 16, number & 0xffff) })).ok();
            }
            3 => {
                let why = String::from_utf8_lossy(&frame).trim().to_owned();
                live.lock().map(|mut l| l.events.push(Event::Failed(why))).ok();
            }
            _ => {}
        }
    };
    // Its input closed, it stops the camera and ends: given a moment for
    // that before it is made to.
    slot.lock().map(|mut s| s.take()).ok();
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().ok();
    child.wait().ok();
    if let Some(listening) = listening {
        listening.join().ok();
    }
    // The program's last word, if it stopped of itself.
    if trouble.is_some() && !stop.load(Ordering::Relaxed) {
        let last = said.lock().map(|s| s.trim().to_lowercase()).unwrap_or_default();
        let last = match last.as_str() {
            "" => "the camera stopped".to_owned(),
            l if l.contains("no cameras available") || l.contains("no camera") => "no camera found".to_owned(),
            _ => last,
        };
        live.lock().map(|mut l| l.trouble = Some(last)).ok();
    }
    finish_luma(luma);
}

/// The simulated camera: its scene at the camera's pace, photos written by
/// the shell's own JPEG encoder.
fn run_simulated(live: &Mutex<Live>, stop: &AtomicBool, orders: &Receiver<Order>, scanning: &AtomicBool, luma: &(Mutex<Luma>, Condvar)) {
    let started = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(33));
        let t = started.elapsed().as_secs_f32();
        let scan = scanning.load(Ordering::Relaxed);
        let pic = scene(PREVIEW_W, PREVIEW_H, t, scan);
        give_luma(luma, &brightness(&pic));
        if let Ok(mut l) = live.lock() {
            l.preview = Some(pic);
            l.frames += 1;
        }
        while let Ok(order) = orders.try_recv() {
            if let Order::Still(path) = order {
                // The photo at half the sensor's size, as a camera short of
                // memory would take it.
                let photo = scene(1640, 1232, t, scan);
                let event = match save_jpeg(&path, &photo) {
                    Ok(()) => Event::Saved { path, size: (1640, 1232) },
                    Err(why) => Event::Failed(why),
                };
                live.lock().map(|mut l| l.events.push(event)).ok();
            }
        }
    }
    finish_luma(luma);
}

fn give_luma(luma: &(Mutex<Luma>, Condvar), y: &[u8]) {
    let (lock, ready) = luma;
    if let Ok(mut l) = lock.lock() {
        l.y.clear();
        l.y.extend_from_slice(y);
        l.fresh = true;
    }
    ready.notify_one();
}

fn finish_luma(luma: &(Mutex<Luma>, Condvar)) {
    luma.0.lock().map(|mut l| l.done = true).ok();
    luma.1.notify_one();
}

/// A picture's brightness (BT.601), as the camera's Y plane would be.
fn brightness(pic: &Picture) -> Vec<u8> {
    pic.rgb.chunks_exact(3).map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2])) >> 8) as u8).collect()
}

/// The QR reader's thread: while scanning, the newest picture's brightness,
/// a few times a second.
fn scan(live: &Mutex<Live>, scanning: &AtomicBool, luma: &(Mutex<Luma>, Condvar)) {
    let mut y = Vec::new();
    loop {
        {
            let (lock, ready) = luma;
            let Ok(mut l) = lock.lock() else { return };
            while !l.fresh && !l.done {
                l = match ready.wait_timeout(l, Duration::from_millis(300)) {
                    Ok((l, _)) => l,
                    Err(_) => return,
                };
            }
            if l.done {
                return;
            }
            l.fresh = false;
            if !scanning.load(Ordering::Relaxed) {
                continue;
            }
            std::mem::swap(&mut y, &mut l.y);
        }
        if y.len() == PREVIEW_W * PREVIEW_H {
            if let Some(found) = read_qr(&y, PREVIEW_W, PREVIEW_H) {
                if let Ok(mut l) = live.lock() {
                    l.found = Some(found);
                    l.found_count += 1;
                    l.found_at = Some(Instant::now());
                }
            }
        }
        std::thread::sleep(SCAN_EVERY);
    }
}

/// The first QR code in a picture's brightness.
fn read_qr(y: &[u8], w: usize, h: usize) -> Option<Found> {
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, row| y[row * w + x]);
    for grid in img.detect_grids() {
        if let Ok((_, text)) = grid.decode() {
            let b = grid.bounds;
            let corners = [b[0], b[1], b[2], b[3]].map(|p| (p.x as f32, p.y as f32));
            return Some(Found { code: read_code(&text), corners });
        }
    }
    None
}

// ----------------------------------------------------------------- screen ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Photo,
    Scan,
}

/// The timer's choices, in turn.
const TIMERS: [u32; 3] = [0, 3, 10];

pub struct Camera {
    live: Arc<Mutex<Live>>,
    stop: Arc<AtomicBool>,
    control: Control,
    scanning: Arc<AtomicBool>,
    mode: Mode,
    /// Where the mode's mark is: 0 under photo, 1 under scan.
    mode_at: Animated,
    preview: Option<Picture>,
    seen: u64,
    trouble: Option<String>,
    timer: u32,
    /// When the timer's photo is taken (kept by the clock, not the frames).
    countdown: Option<Instant>,
    grid: bool,
    night: bool,
    /// The shutter's flash over the picture, and its button pressed in.
    flash: Animated,
    press: Animated,
    saving: Option<Instant>,
    /// The newest photo and its thumbnail, and the thumbnail arriving.
    last: Option<(Photo, Picture)>,
    last_rx: Option<Receiver<Option<(Photo, Picture)>>>,
    last_pop: Animated,
    note: Option<(String, f32)>,
    /// The code read, the card it rises on, and when any code was last seen
    /// (the card goes once none has been for a few seconds).
    found: Option<Found>,
    found_seen: u64,
    found_rise: Animated,
    found_at: Option<Instant>,
    /// A code put away with Esc: not offered again while it stays in sight.
    dismissed: Option<Code>,
    clock: f32,
    /// Opened by quick capture (`capture.rs`): one photo, and it gives way.
    quick: bool,
    done: bool,
}

/// A code's card stays this long after the code was last seen; its outline
/// only while it is being seen.
const CARD_FOR: Duration = Duration::from_secs(4);
const OUTLINE_FOR: Duration = Duration::from_millis(700);

impl Camera {
    pub fn new() -> Self {
        let live = Arc::new(Mutex::new(Live::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let scanning = Arc::new(AtomicBool::new(false));
        let luma: Arc<(Mutex<Luma>, Condvar)> = Arc::default();
        let simulated = !cfg!(target_arch = "aarch64");
        let control = if simulated {
            let (tx, rx) = mpsc::channel();
            let (l, s, sc, lu) = (live.clone(), stop.clone(), scanning.clone(), luma.clone());
            std::thread::Builder::new().name("camera".into()).spawn(move || run_simulated(&l, &s, &rx, &sc, &lu)).ok();
            Control::Simulated(tx)
        } else {
            let slot: Arc<Mutex<Option<ChildStdin>>> = Arc::default();
            let (l, s, sl, lu) = (live.clone(), stop.clone(), slot.clone(), luma.clone());
            std::thread::Builder::new().name("camera".into()).spawn(move || run_program(&l, &s, &sl, &lu)).ok();
            Control::Program(slot)
        };
        {
            let (l, sc, lu) = (live.clone(), scanning.clone(), luma.clone());
            std::thread::Builder::new().name("qr".into()).stack_size(1 << 20).spawn(move || scan(&l, &sc, &lu)).ok();
        }
        let mut camera = Camera {
            live,
            stop,
            control,
            scanning,
            mode: Mode::Photo,
            mode_at: Animated::new(0.0, role::detent()).with_rest(0.002),
            preview: None,
            seen: 0,
            trouble: None,
            timer: 0,
            countdown: None,
            grid: false,
            night: false,
            flash: Animated::new(0.0, role::smooth()).with_rest(0.01),
            press: Animated::new(0.0, role::layer()).with_rest(0.01),
            saving: None,
            last: None,
            last_rx: None,
            last_pop: Animated::new(1.0, role::wobble()).with_rest(0.002),
            note: Some(("enter: take a photo · ↓ photos".into(), 0.0)),
            found: None,
            found_seen: 0,
            found_rise: Animated::new(0.0, role::release()).with_rest(0.002),
            found_at: None,
            dismissed: None,
            clock: 0.0,
            quick: false,
            done: false,
        };
        // The simulator's album has a few photos to begin with.
        #[cfg(not(target_arch = "aarch64"))]
        photos::seed_demo();
        camera.load_last(None);
        camera
    }

    /// For quick capture: the camera for one photo. Enter (or Ctrl-P, the
    /// key that opened it) takes it into Photos, and the camera gives way
    /// at once; Esc goes back with none.
    pub fn quick() -> Self {
        let mut camera = Camera::new();
        camera.quick = true;
        camera.note = Some(("enter: take it · esc: none".into(), 0.0));
        camera
    }

    /// The newest photo's thumbnail, on a thread (it may need making).
    fn load_last(&mut self, newest: Option<PathBuf>) {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("last-photo".into())
            .spawn(move || {
                let photos = photos::list();
                let photo = match newest {
                    Some(path) => photos.into_iter().find(|p| p.path == path),
                    None => photos.into_iter().next(),
                };
                tx.send(photo.and_then(|p| photos::thumbnail(&p).map(|t| (p, t)))).ok();
            })
            .ok();
        self.last_rx = Some(rx);
    }

    fn say(&mut self, words: impl Into<String>) {
        self.note = Some((words.into(), 0.0));
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.mode_at.set(if mode == Mode::Scan { 1.0 } else { 0.0 });
        self.scanning.store(mode == Mode::Scan, Ordering::Relaxed);
        self.countdown = None;
        // Whatever was read before is forgotten: scanning starts afresh.
        self.found = None;
        self.found_at = None;
        self.dismissed = None;
        self.found_rise.jump(0.0);
        if let Ok(mut l) = self.live.lock() {
            l.found = None;
            l.found_at = None;
            self.found_seen = l.found_count;
        }
        if mode == Mode::Scan {
            self.say("point it at a QR code");
        }
    }

    /// The code being offered: read, and not put away.
    fn offered(&self) -> Option<&Found> {
        self.found.as_ref().filter(|_| self.mode == Mode::Scan && self.found_rise.target() > 0.5)
    }

    fn shoot(&mut self, calm: bool) {
        if self.saving.is_some() {
            return;
        }
        if self.preview.is_none() {
            self.say("the camera is not ready yet");
            return;
        }
        let path = photos::new_path(chrono::Local::now().naive_local());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        if !self.control.send(Order::Still(path)) {
            self.say("the camera is not ready yet");
            return;
        }
        self.saving = Some(Instant::now());
        self.press.jump(1.0);
        self.press.set(0.0);
        if !calm {
            self.flash.jump(1.0);
            self.flash.set(0.0);
        }
    }

    /// Enter on what a QR code holds.
    fn act(&mut self) -> Nav {
        let Some(found) = self.offered().cloned() else {
            self.say("point it at a QR code");
            return Nav::Stay;
        };
        match found.code {
            Code::Wifi { ssid, password, .. } => Nav::Push(Box::new(crate::screens::WifiScreen::joining(&ssid, password))),
            Code::Url(url) => {
                let command = format!("lynx -accept_all_cookies -nopause {}", quote(&url));
                Nav::Push(Box::new(crate::terminal::Terminal::running("Web", &command, crate::connect::session_dir())))
            }
            Code::Text(text) => Nav::Push(Box::new(TextPage::new(text))),
        }
    }

    /// The viewfinder: the panel's height at 4:3, at its left.
    fn viewfinder() -> Rect {
        let w = (H * 4.0 / 3.0).round();
        Rect::new(0.0, 0.0, w, H)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Closing the program's input ends it (it reads its orders until then).
        if let Control::Program(slot) = &self.control {
            slot.lock().map(|mut s| s.take()).ok();
        }
    }
}

impl Screen for Camera {
    fn title(&self) -> &str {
        "Camera"
    }

    fn full_screen(&self) -> bool {
        true
    }

    /// Quick capture's photo is in: it gives way to what it opened over.
    fn finished(&self) -> bool {
        self.done
    }

    /// Leaving it stops the camera, which would otherwise run on unseen.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn busy(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave if self.countdown.is_some() => {
                self.countdown = None;
                self.say("timer stopped");
            }
            // The card put away; the code not offered again while it stays
            // in sight.
            Key::Back if self.offered().is_some() => {
                self.dismissed = self.found.as_ref().map(|f| f.code.clone());
                self.found_rise.set(0.0);
            }
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Left | Key::Right | Key::Tab => self.set_mode(if self.mode == Mode::Photo { Mode::Scan } else { Mode::Photo }),
            Key::Enter | Key::Char(' ') | Key::Ctrl('p' | 'P') if self.quick || matches!(key, Key::Enter | Key::Char(' ')) => match self.mode {
                Mode::Scan => return self.act(),
                Mode::Photo if self.countdown.is_some() => {}
                Mode::Photo if self.timer > 0 => self.countdown = Some(Instant::now() + Duration::from_secs(self.timer.into())),
                Mode::Photo => self.shoot(cx.settings.calm),
            },
            Key::Down => {
                return Nav::Push(Box::new(match &self.last {
                    Some((photo, _)) => photos::Album::at(&photo.path),
                    None => photos::Album::new(),
                }));
            }
            Key::Char('t') => {
                let at = TIMERS.iter().position(|t| *t == self.timer).unwrap_or(0);
                self.timer = TIMERS[(at + 1) % TIMERS.len()];
                self.say(if self.timer == 0 { "timer off".to_owned() } else { format!("timer {} s", self.timer) });
            }
            Key::Char('g') => {
                self.grid = !self.grid;
                self.say(if self.grid { "grid on" } else { "grid off" });
            }
            Key::Char('n') => {
                self.night = !self.night;
                self.control.send(Order::Night(self.night));
                self.say(if self.night { "night: longer exposures in low light" } else { "night off" });
            }
            _ => {}
        }
        Nav::Stay
    }

    fn resume(&mut self) {
        // Back from the album: a photo may have gone.
        self.load_last(None);
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        // The timer ran out: the photo now.
        if self.countdown.is_some_and(|at| Instant::now() >= at) {
            self.countdown = None;
            self.shoot(cx.settings.calm);
        }
        // The camera never answered (its program gone quiet): say so rather
        // than saving for ever.
        if self.saving.is_some_and(|at| at.elapsed() > Duration::from_secs(20)) {
            self.saving = None;
            self.say("the camera did not answer: no photo");
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        let mut changed = self.mode_at.tick(dt) | self.flash.tick(dt) | self.press.tick(dt) | self.last_pop.tick(dt) | self.found_rise.tick(dt);
        // Counting down and saving are drawn moving (the number's pulse, the
        // dots), and keep the frames coming while they last.
        changed |= self.countdown.is_some() || self.saving.is_some();
        if let Some((_, age)) = &mut self.note {
            *age += dt;
            changed = true;
            if *age > NOTE_FOR + 0.4 {
                self.note = None;
            }
        }
        if let Some(rx) = &self.last_rx {
            if let Ok(last) = rx.try_recv() {
                self.last_rx = None;
                let arrived = last.as_ref().map(|(p, _)| p.path.clone()) != self.last.as_ref().map(|(p, _)| p.path.clone());
                self.last = last;
                if arrived && self.last.is_some() {
                    self.last_pop.jump(0.6);
                    self.last_pop.set(1.0);
                }
                changed = true;
            }
        }
        let events = {
            let Ok(mut l) = self.live.lock() else { return changed };
            if l.frames != self.seen {
                self.seen = l.frames;
                if let Some(p) = l.preview.take() {
                    self.preview = Some(p);
                }
                changed = true;
            }
            if l.trouble != self.trouble {
                self.trouble.clone_from(&l.trouble);
                // The camera stopped: say why rather than hold its last picture.
                if self.trouble.is_some() {
                    self.preview = None;
                    self.saving = None;
                }
                changed = true;
            }
            if l.found_count != self.found_seen && self.mode == Mode::Scan {
                self.found_seen = l.found_count;
                self.found_at = l.found_at;
                let code = l.found.as_ref().map(|f| &f.code);
                // A code put away and still in sight is not offered again.
                if code.is_none() || code != self.dismissed.as_ref() {
                    let fresh = self.found_rise.target() < 0.5 || self.found.as_ref().map(|f| &f.code) != code;
                    self.found.clone_from(&l.found);
                    if fresh {
                        self.found_rise.jump(0.0);
                        self.found_rise.set(1.0);
                    }
                }
                changed = true;
            }
            std::mem::take(&mut l.events)
        };
        // No code seen for a while: its card goes, and one put away may be
        // offered again when it comes back.
        let unseen = self.found_at.map_or(Duration::MAX, |t| t.elapsed());
        if unseen > CARD_FOR && self.found_rise.target() > 0.5 {
            self.found_rise.set(0.0);
            changed = true;
        }
        if unseen > Duration::from_millis(1500) {
            self.dismissed = None;
        }
        if self.found.is_some() && self.found_rise.target() < 0.5 && self.found_rise.value() < 0.01 {
            self.found = None;
            changed = true;
        }
        for event in events {
            self.saving = None;
            match event {
                Event::Saved { path, size } => {
                    photos::changed();
                    // Quick capture's photo: said in the island, and the camera gives way.
                    if self.quick {
                        crate::capture::photo_taken(path.clone());
                        self.done = true;
                    }
                    self.say(if size.0 >= 3000 { "saved to photos".to_owned() } else { format!("saved · {} × {}", size.0, size.1) });
                    self.load_last(Some(path));
                }
                Event::Failed(why) => self.say(why),
            }
            changed = true;
        }
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if g.calm {
            for a in [&mut self.mode_at, &mut self.press, &mut self.last_pop, &mut self.found_rise] {
                a.jump(a.target());
            }
            self.flash.jump(0.0);
        }
        let vf = Self::viewfinder();
        let rail = Rect::new(vf.right(), 0.0, W - vf.right(), H);
        g.canvas.fill_rect(Rect::new(g.ox, 0.0, W, H), t.ground, 1.0);
        g.canvas.fill_rect(vf.offset(g.ox, 0.0), Color(6, 6, 8), 1.0);

        // The picture.
        match &self.preview {
            Some(pic) => draw_picture(g, vf, pic, whole(pic), 0.0, 1.0),
            None => {
                let words = self.trouble.clone().unwrap_or_else(|| "starting the camera".into());
                let words = g.text.fit(&words, &ty.caption, vf.w - 20.0);
                g.icon("photo_camera", false, vf.cx(), vf.cy() - 12.0, 20.0, Color(120, 120, 128));
                g.text_at(vf.cx(), vf.cy() + 12.0, Align::Center, &words, ty.caption, Color(170, 170, 178));
            }
        }
        let white = Color(255, 255, 255);
        if self.grid && self.mode == Mode::Photo {
            for i in 1..3 {
                let x = vf.x + vf.w * i as f32 / 3.0;
                let y = vf.y + vf.h * i as f32 / 3.0;
                g.canvas.fill_rect(Rect::new(x.round() + g.ox, vf.y, 1.0, vf.h), white, 0.35);
                g.canvas.fill_rect(Rect::new(vf.x + g.ox, y.round(), vf.w, 1.0), white, 0.35);
            }
        }
        // Scan mode: a square to aim with, the code outlined once read.
        let scan = self.mode_at.value().clamp(0.0, 1.0);
        if scan > 0.01 {
            let side = (vf.h * 0.62).round();
            let sq = Rect::new(vf.cx() - side * 0.5, vf.cy() - side * 0.5, side, side);
            let arm = side * 0.18;
            for (cx, cy, sx, sy) in [(sq.x, sq.y, 1.0, 1.0), (sq.right(), sq.y, -1.0, 1.0), (sq.right(), sq.bottom(), -1.0, -1.0), (sq.x, sq.bottom(), 1.0, -1.0)] {
                let h = Rect::new(if sx > 0.0 { cx } else { cx - arm }, if sy > 0.0 { cy } else { cy - 2.0 }, arm, 2.0);
                let v = Rect::new(if sx > 0.0 { cx } else { cx - 2.0 }, if sy > 0.0 { cy } else { cy - arm }, 2.0, arm);
                g.canvas.rrect(h.offset(g.ox, 0.0), 1.0, white, 0.9 * scan);
                g.canvas.rrect(v.offset(g.ox, 0.0), 1.0, white, 0.9 * scan);
            }
            let in_sight = self.found_at.is_some_and(|t| t.elapsed() < OUTLINE_FOR);
            if let (Some(found), Some(pic), true) = (self.offered(), &self.preview, in_sight) {
                let (sx, sy) = (vf.w / pic.w as f32, vf.h / pic.h as f32);
                let pts: Vec<(f32, f32)> = found.corners.iter().map(|(x, y)| (vf.x + x * sx + g.ox, vf.y + y * sy)).collect();
                for i in 0..4 {
                    line(g, pts[i], pts[(i + 1) % 4], 2.0, t.signal);
                }
            }
        }
        // The timer counting down.
        if let Some(at) = self.countdown {
            let left = at.saturating_duration_since(Instant::now()).as_secs_f32();
            let n = left.ceil().max(1.0) as u32;
            // Each second lands large and eases back, fading a touch.
            let into = 1.0 - (left - left.floor());
            let pulse = if g.calm { 0.0 } else { (1.0 - into / 0.35).clamp(0.0, 1.0).powi(2) };
            let big = ty.display(44.0 + 10.0 * pulse);
            let words = n.to_string();
            let side = 64.0;
            g.canvas.rrect(Rect::new(vf.cx() - side * 0.5 + g.ox, vf.cy() - side * 0.5, side, side), side * 0.5, Color(0, 0, 0), 0.45);
            let half = g.measure(&words, big) * 0.5;
            g.text_alpha(vf.cx() - half, vf.cy(), &words, big, white, 1.0 - 0.25 * pulse);
        }
        // The shutter's flash.
        let flash = self.flash.value();
        if flash > 0.01 {
            g.canvas.fill_rect(vf.offset(g.ox, 0.0), white, flash.min(1.0) * 0.85);
        }
        // What a QR code holds, rising onto the picture.
        if let Some(found) = self.found.as_ref().filter(|_| self.mode == Mode::Scan) {
            let rise = self.found_rise.value();
            let (icon, title, action) = found.code.words();
            let h = ty.body.size + ty.caption.size + 22.0;
            let card = Rect::new(vf.x + 8.0, vf.bottom() - 8.0 - h + (1.0 - rise) * (h + 12.0), vf.w - 16.0, h);
            let radius = g.radius(12.0).min(h * 0.5);
            g.canvas.rrect(card.offset(g.ox, 0.0), radius, t.surface1, 0.96 * rise.clamp(0.0, 1.0));
            g.icon_alpha(icon, true, card.x + 18.0, card.cy(), 16.0, t.signal, rise.clamp(0.0, 1.0));
            let room = card.w - 44.0;
            let title = g.text.fit(&title, &ty.body, room);
            g.text_alpha(card.x + 34.0, card.y + 8.0 + ty.body.size * 0.5, &title, ty.body, t.ink, rise.clamp(0.0, 1.0));
            g.text_alpha(card.x + 34.0, card.bottom() - 8.0 - ty.caption.size * 0.5, action, ty.caption, t.dim, rise.clamp(0.0, 1.0));
        }
        // A note, at the top of the picture, fading as it goes.
        if let Some((words, age)) = &self.note {
            let a = ((NOTE_FOR + 0.4 - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0);
            let words = g.text.fit(words, &ty.caption, vf.w - 24.0);
            let w = g.measure(&words, ty.caption) + 16.0;
            let h = ty.caption.size + 8.0;
            g.canvas.rrect(Rect::new(vf.cx() - w * 0.5 + g.ox, 7.0, w, h), h * 0.5, Color(0, 0, 0), 0.55 * a);
            g.text_alpha(vf.cx() - w * 0.5 + 8.0, 7.0 + h * 0.5, &words, ty.caption, white, a);
        }
        if self.saving.is_some() {
            let dots = ".".repeat((self.clock * 3.0) as usize % 3 + 1);
            let words = format!("saving{dots}");
            let w = g.measure("saving...", ty.caption) + 16.0;
            let h = ty.caption.size + 8.0;
            g.canvas.rrect(Rect::new(vf.x + 7.0 + g.ox, vf.bottom() - 7.0 - h, w, h), h * 0.5, Color(0, 0, 0), 0.55);
            g.text_at(vf.x + 15.0, vf.bottom() - 7.0 - h * 0.5, Align::Left, &words, ty.caption, white);
        }

        self.draw_rail(g, rail);
    }
}

impl Camera {
    /// The rail beside the picture: the modes, the shutter, the last photo
    /// and the choices.
    fn draw_rail(&mut self, g: &mut Gfx, rail: Rect) {
        let t = g.tokens;
        let ty = g.ty;
        // The modes, a mark sliding under the one in use.
        let cap = ty.caption;
        let (photo_w, scan_w) = (g.measure("photo", cap), g.measure("scan", cap));
        let gap = 10.0;
        let x0 = rail.cx() - (photo_w + gap + scan_w) * 0.5;
        let cy = PAD + cap.size * 0.5 + 2.0;
        let at = self.mode_at.value();
        g.text_at(x0, cy, Align::Left, "photo", cap, if self.mode == Mode::Photo { t.ink } else { t.dim });
        g.text_at(x0 + photo_w + gap, cy, Align::Left, "scan", cap, if self.mode == Mode::Scan { t.ink } else { t.dim });
        let (mx, mw) = (x0 + (photo_w + gap) * at, photo_w + (scan_w - photo_w) * at);
        g.surface(Rect::new(mx, cy + cap.size * 0.5 + 3.0, mw, 2.0), 1.0, t.signal);

        // The shutter: a ring, and in it a disc that presses in.
        let (sx, sy) = (rail.cx(), (H * 0.5).round());
        let ring = 21.0;
        g.canvas.rrect(Rect::new(sx - ring + g.ox, sy - ring, ring * 2.0, ring * 2.0), ring, t.ink, 1.0);
        g.canvas.rrect(Rect::new(sx - ring + 2.5 + g.ox, sy - ring + 2.5, ring * 2.0 - 5.0, ring * 2.0 - 5.0), ring - 2.5, t.ground, 1.0);
        let inner = (ring - 5.5) * (1.0 - 0.12 * self.press.value().clamp(0.0, 1.0));
        match self.mode {
            Mode::Photo => {
                let a = if self.saving.is_some() { 0.45 } else { 1.0 };
                g.canvas.rrect(Rect::new(sx - inner + g.ox, sy - inner, inner * 2.0, inner * 2.0), inner, t.ink, a);
                if self.timer > 0 && self.countdown.is_none() {
                    g.text_at(sx, sy, Align::Center, &format!("{}s", self.timer), ty.caption, t.ground);
                }
            }
            Mode::Scan => g.icon("qr_code_scanner", false, sx, sy, 18.0, t.ink),
        }

        // The last photo, opening the album (↓).
        let (tw, th) = (36.0, 27.0);
        let thumb = Rect::new(rail.x + 10.0, H - PAD - th - 2.0, tw, th);
        let pop = self.last_pop.value();
        let grown = Rect::new(thumb.cx() - tw * pop * 0.5, thumb.cy() - th * pop * 0.5, tw * pop, th * pop);
        match &self.last {
            Some((_, pic)) => draw_picture(g, grown, pic, whole(pic), 5.0, 1.0),
            None => {
                g.surface(thumb, 5.0, t.surface1);
                g.icon("photo_library", false, thumb.cx(), thumb.cy(), 13.0, t.faint);
            }
        }
        g.text_at(thumb.cx(), thumb.y - 2.0 - cap.size * 0.5, Align::Center, "↓", cap, t.faint);
        // The choices, each with its key: on in ink, off faint.
        let right = rail.right() - 9.0;
        let row = |i: usize| thumb.y + 3.0 + i as f32 * (th / 2.0 - 1.0);
        let choices = [("timer", "t", self.timer > 0), ("grid_on", "g", self.grid), ("bedtime", "n", self.night)];
        for (i, (icon, key, on)) in choices.iter().enumerate().take(2) {
            let y = row(i) + 2.0;
            let color = if *on { t.ink } else { t.faint };
            g.icon(icon, *on, right - 18.0, y, 11.0, color);
            g.text_at(right - 6.0, y, Align::Center, key, cap, color);
        }
        let (icon, key, on) = choices[2];
        let color = if on { t.ink } else { t.faint };
        let y = thumb.y - 2.0 - cap.size * 0.5;
        g.icon(icon, on, right - 18.0, y, 11.0, color);
        g.text_at(right - 6.0, y, Align::Center, key, cap, color);
    }
}

/// A straight line `width` pixels wide.
fn line(g: &mut Gfx, a: (f32, f32), b: (f32, f32), width: f32, color: Color) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
    g.canvas.polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], color, 1.0);
}

// --------------------------------------------------------------- the text ---

/// What a QR code said, all of it.
pub struct TextPage {
    lines: Option<(f32, Vec<String>)>,
    text: String,
    scroll: Animated,
}

impl TextPage {
    fn new(text: String) -> Self {
        TextPage { lines: None, text, scroll: Animated::new(0.0, role::settle()).with_rest(0.2) }
    }
}

impl Screen for TextPage {
    fn title(&self) -> &str {
        "QR code"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let lines = self.lines.as_ref().map_or(0, |(_, l)| l.len()) as f32;
        let step = 16.0;
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Down => self.scroll.set((self.scroll.target() + step * 2.0).min((lines * step - 80.0).max(0.0))),
            Key::Up => self.scroll.set((self.scroll.target() - step * 2.0).max(0.0)),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let width = W - 2.0 * PAD - 6.0;
        if self.lines.as_ref().is_none_or(|(w, _)| *w != ty.body.size) {
            let mut lines = Vec::new();
            for paragraph in self.text.split('\n') {
                lines.extend(crate::help::wrap(g, paragraph, ty.body, width));
            }
            self.lines = Some((ty.body.size, lines));
        }
        let top = g.top();
        let bottom = g.above_hint();
        let lh = ty.body.size + 4.0;
        g.canvas.set_clip(Rect::new(0.0, top, W, bottom - top));
        if let Some((_, lines)) = &self.lines {
            for (i, line) in lines.iter().enumerate() {
                let cy = top + (i as f32 + 0.5) * lh - self.scroll.value();
                if cy > top - lh && cy < bottom + lh {
                    g.text_at(PAD + 2.0, cy, Align::Left, line, ty.body, t.ink);
                }
            }
        }
        g.canvas.clear_clip();
        g.hint("↑↓: scroll · esc: back");
    }
}

// -------------------------------------------------------------- simulated ---

/// The simulated camera's picture at `t` seconds, `w` × `h`: a field at the
/// end of an afternoon (sky, sun, hills, carpet and its lines, a robot's
/// bumper), drifting a little as a hand would; in scan mode a card with a
/// Wi-Fi QR code held up in front.
pub fn scene(w: usize, h: usize, t: f32, with_code: bool) -> Picture {
    let mut pic = Picture::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    // A hand's drift, and the light changing with the time.
    let (jx, jy) = ((t * 0.7).sin() * 0.012, (t * 0.53).cos() * 0.01);
    let warm = 0.5 + 0.5 * (t * 0.11).sin();
    let horizon = 0.58 + jy;
    let sun = (0.72 + jx + 0.05 * (t * 0.05).sin(), 0.3 + jy);
    for y in 0..h {
        let v = y as f32 / fh;
        for x in 0..w {
            let u = x as f32 / fw;
            let (mut r, mut g, mut b);
            let hill1 = horizon - 0.07 - 0.05 * ((u + jx) * 7.0).sin() - 0.03 * ((u + jx) * 17.0 + 1.0).sin();
            let hill2 = horizon - 0.02 - 0.03 * ((u + jx) * 11.0 + 2.0).sin();
            if v < hill1 {
                // The sky: deep blue at the top to a warm glow at the horizon.
                let k = (v / hill1).clamp(0.0, 1.0);
                r = 40.0 + 200.0 * k * (0.6 + 0.4 * warm);
                g = 90.0 + 110.0 * k * (0.7 + 0.2 * warm);
                b = 190.0 - 60.0 * k;
                // The sun and its glow.
                let d = (((u - sun.0) * fw / fh).powi(2) + (v - sun.1).powi(2)).sqrt();
                let glow = (1.0 - d / 0.35).max(0.0).powi(2);
                r += 120.0 * glow;
                g += 90.0 * glow;
                b += 40.0 * glow;
                if d < 0.055 {
                    (r, g, b) = (255.0, 244.0, 214.0);
                }
            } else if v < hill2 {
                (r, g, b) = (58.0 + 30.0 * warm, 92.0, 84.0);
            } else if v < horizon {
                (r, g, b) = (44.0 + 20.0 * warm, 74.0, 66.0);
            } else {
                // The carpet, with white lines meeting at the far end.
                let k = (v - horizon) / (1.0 - horizon);
                (r, g, b) = (70.0 + 20.0 * k, 78.0 + 18.0 * k, 86.0 + 14.0 * k);
                let lane = ((u - 0.5 - jx) / (0.08 + 0.9 * k)).abs();
                if (lane - 0.5).abs() < 0.03 + 0.01 * k || (k * 9.0).fract() < 0.035 {
                    (r, g, b) = (226.0, 228.0, 232.0);
                }
            }
            // A robot on the carpet, driving slowly from side to side: its
            // shadow, an elevator's two aluminium uprights and crossbar with
            // the carriage on them, the chassis, and its red bumper in front,
            // lit from above.
            let (rx, ry) = (u - 0.35 - 0.14 * (t * 0.25).sin() - jx, v - jy);
            let within = |x0: f32, x1: f32, y0: f32, y1: f32| (x0..x1).contains(&rx) && (y0..y1).contains(&ry);
            if within(-0.01, 0.31, 0.83, 0.87) {
                let k = 0.55 + 0.45 * ((ry - 0.83) / 0.04);
                (r, g, b) = (r * k, g * k, b * k);
            }
            let aluminium = |side: bool| if side { (128.0, 133.0, 142.0) } else { (184.0, 188.0, 196.0) };
            for x0 in [0.06, 0.225] {
                if within(x0, x0 + 0.015, 0.47, 0.70) {
                    (r, g, b) = aluminium(rx < x0 + 0.005);
                }
            }
            if within(0.06, 0.24, 0.47, 0.484) {
                (r, g, b) = aluminium(ry > 0.479);
            }
            if within(0.085, 0.215, 0.575, 0.625) {
                (r, g, b) = if ry < 0.581 { (96.0, 102.0, 112.0) } else { (62.0, 66.0, 74.0) };
            }
            if within(0.02, 0.28, 0.70, 0.745) {
                (r, g, b) = if ry < 0.707 { (98.0, 104.0, 114.0) } else { (44.0, 48.0, 54.0) };
            }
            // The bumper, its corners rounded, darker toward the carpet.
            let corner = 0.014;
            let (cx, cy) = (rx.clamp(corner, 0.30 - corner), ry.clamp(0.74 + corner, 0.83 - corner));
            if within(0.0, 0.30, 0.74, 0.83) && ((rx - cx) * 4.0 / 3.0).hypot(ry - cy) <= corner {
                let k = (ry - 0.74) / 0.09;
                (r, g, b) = if k < 0.09 { (236.0, 92.0, 98.0) } else { (212.0 * (1.06 - 0.36 * k), 40.0 * (1.06 - 0.36 * k), 52.0 * (1.06 - 0.36 * k)) };
            }
            let at = (y * w + x) * 3;
            pic.rgb[at] = r.clamp(0.0, 255.0) as u8;
            pic.rgb[at + 1] = g.clamp(0.0, 255.0) as u8;
            pic.rgb[at + 2] = b.clamp(0.0, 255.0) as u8;
        }
    }
    if with_code {
        draw_code(&mut pic, "WIFI:T:WPA;S:Pit 5805;P:catalyst-5805;;", jx, jy);
    }
    pic
}

/// A card held up with a QR code on it, in the middle of the picture.
fn draw_code(pic: &mut Picture, text: &str, jx: f32, jy: f32) {
    let Ok(qr) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else { return };
    let n = qr.size();
    let (fw, fh) = (pic.w as f32, pic.h as f32);
    // The code about half the picture's height, with a quiet zone of four.
    let module = (fh * 0.5 / (n + 8) as f32).floor().max(1.0);
    let side = module * (n + 8) as f32;
    let (x0, y0) = ((fw * (0.5 + jx) - side * 0.5).round(), (fh * (0.5 + jy) - side * 0.5).round());
    for y in 0..side as usize {
        for x in 0..side as usize {
            let (px, py) = (x0 as i64 + x as i64, y0 as i64 + y as i64);
            if px < 0 || py < 0 || px >= pic.w as i64 || py >= pic.h as i64 {
                continue;
            }
            let (mx, my) = ((x as f32 / module) as i32 - 4, (y as f32 / module) as i32 - 4);
            let dark = qr.get_module(mx, my);
            let v = if dark { 24 } else { 236 };
            let at = (py as usize * pic.w + px as usize) * 3;
            pic.rgb[at..at + 3].copy_from_slice(&[v, v, v]);
        }
    }
}

/// A picture written as a JPEG (beside `path`, renamed into place). The
/// simulator's photos; on the handheld catalyst-camera writes them.
pub fn save_jpeg(path: &std::path::Path, pic: &Picture) -> Result<(), String> {
    #[cfg(not(target_arch = "aarch64"))]
    {
        let part = path.with_extension("jpg.part");
        let encoder = jpeg_encoder::Encoder::new_file(&part, 90).map_err(|e| format!("the photo cannot be written: {e}"))?;
        encoder.encode(&pic.rgb, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Rgb).map_err(|e| format!("the photo could not be made: {e}"))?;
        std::fs::rename(&part, path).map_err(|e| format!("the photo could not be kept: {e}"))
    }
    #[cfg(target_arch = "aarch64")]
    {
        let _ = (path, pic);
        Err("photos are written by the camera program here".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_codes_are_read_with_their_escapes() {
        assert_eq!(
            read_code("WIFI:T:WPA;S:Pit 5805;P:catalyst-5805;;"),
            Code::Wifi { ssid: "Pit 5805".into(), password: Some("catalyst-5805".into()), hidden: false }
        );
        // Escaped ; , : and \ in a value, fields in any order, a hidden one.
        assert_eq!(
            read_code(r#"WIFI:S:My\;Net\:work;P:pa\\ss\,word;T:WPA;H:true;;"#),
            Code::Wifi { ssid: "My;Net:work".into(), password: Some(r"pa\ss,word".into()), hidden: true }
        );
        assert_eq!(read_code("WIFI:T:nopass;S:Guest;P:;;"), Code::Wifi { ssid: "Guest".into(), password: None, hidden: false });
        assert_eq!(read_code("wifi:S:lower;;"), Code::Wifi { ssid: "lower".into(), password: None, hidden: false });
        assert!(matches!(read_code("WIFI:T:WPA;P:x;;"), Code::Text(_)), "no name: only text");
    }

    #[test]
    fn web_addresses_and_text_are_told_apart() {
        assert_eq!(read_code("https://www.thebluealliance.com/team/5805"), Code::Url("https://www.thebluealliance.com/team/5805".into()));
        assert_eq!(read_code(" http://10.58.5.2:5800 "), Code::Url("http://10.58.5.2:5800".into()));
        assert!(matches!(read_code("https://a b"), Code::Text(_)));
        assert!(matches!(read_code("ftp://x"), Code::Text(_)));
        assert_eq!(read_code("Pit 12, by the north door"), Code::Text("Pit 12, by the north door".into()));
        // A web address goes to the shell in single quotes, whatever it holds.
        assert_eq!(quote("https://x.org/a'b;rm -rf"), r"'https://x.org/a'\''b;rm -rf'");
    }

    #[test]
    fn full_range_yuv_turns_into_rgb() {
        // 2 × 2: white, black, mid grey, and a red whose U and V are shared.
        let mut data = vec![255u8, 0, 128, 76];
        data.extend_from_slice(&[128]); // U
        data.extend_from_slice(&[128]); // V
        let mut out = Picture::default();
        yuv420_to_rgb(&data, 2, 2, &mut out);
        assert_eq!(&out.rgb[0..3], &[255, 255, 255]);
        assert_eq!(&out.rgb[3..6], &[0, 0, 0]);
        assert_eq!(&out.rgb[6..9], &[128, 128, 128]);
        // Pure red (255, 0, 0) is Y 76, U 85, V 255 in full range.
        let red = [76u8, 76, 76, 76, 85, 255];
        yuv420_to_rgb(&red, 2, 2, &mut out);
        let px = &out.rgb[0..3];
        assert!(px[0] > 245 && px[1] < 10 && px[2] < 10, "{px:?}");
    }

    #[test]
    fn the_simulated_scan_reads_its_wifi_code() {
        let pic = scene(PREVIEW_W, PREVIEW_H, 1.3, true);
        let found = read_qr(&brightness(&pic), PREVIEW_W, PREVIEW_H).expect("the code is read at the preview's size");
        assert_eq!(found.code, Code::Wifi { ssid: "Pit 5805".into(), password: Some("catalyst-5805".into()), hidden: false });
        // Its corners are where it was drawn: in the middle of the picture.
        let cx = found.corners.iter().map(|c| c.0).sum::<f32>() / 4.0;
        assert!((cx - PREVIEW_W as f32 * 0.5).abs() < 30.0, "{:?}", found.corners);
        assert!(read_qr(&brightness(&scene(PREVIEW_W, PREVIEW_H, 1.3, false)), PREVIEW_W, PREVIEW_H).is_none(), "no code, nothing read");
    }

    fn press(screen: &mut dyn Screen, key: Key) -> Nav {
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut say = None;
        screen.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true })
    }

    /// Frames for `ms` of real time (the simulated camera keeps it), as the
    /// shell would tick the screen.
    fn run(camera: &mut Camera, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(16));
            camera.tick(1.0 / 60.0);
        }
    }

    #[test]
    fn a_code_read_is_offered_put_away_and_not_offered_again_while_in_sight() {
        crate::shell::tests::sandbox();
        let mut camera = Camera::new();
        run(&mut camera, 300);
        assert!(matches!(press(&mut camera, Key::Right), Nav::Stay));
        assert_eq!(camera.mode, Mode::Scan);
        // The reader runs a few times a second on the simulated Wi-Fi card.
        let started = Instant::now();
        while camera.offered().is_none() && started.elapsed() < Duration::from_secs(8) {
            run(&mut camera, 50);
        }
        let code = camera.offered().map(|f| f.code.clone());
        assert!(matches!(code, Some(Code::Wifi { .. })), "{code:?}");
        // Esc puts the card away; the camera stays open, and the code (still
        // in sight) is not offered again.
        assert!(matches!(press(&mut camera, Key::Back), Nav::Stay));
        run(&mut camera, 1200);
        assert!(camera.offered().is_none() && camera.found.is_none(), "put away");
        // Back to photos and to scanning again: it is offered afresh, and
        // Enter takes it to the Wi-Fi page.
        press(&mut camera, Key::Left);
        press(&mut camera, Key::Right);
        let started = Instant::now();
        while camera.offered().is_none() && started.elapsed() < Duration::from_secs(8) {
            run(&mut camera, 50);
        }
        assert!(camera.offered().is_some(), "offered again after scanning anew");
        assert!(matches!(press(&mut camera, Key::Enter), Nav::Push(_)));
        // With nothing to put away, Esc leaves.
        press(&mut camera, Key::Left);
        assert!(matches!(press(&mut camera, Key::Back), Nav::Pop));
    }

    #[test]
    fn a_photo_taken_lands_in_the_album() {
        crate::shell::tests::sandbox();
        let camera = Camera::new();
        // The simulated camera's first pictures.
        let started = Instant::now();
        while camera.live.lock().unwrap().frames < 3 && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let path = photos::new_path(chrono::Local::now().naive_local());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(camera.control.send(Order::Still(path.clone())));
        let mut saved = None;
        while saved.is_none() && started.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(30));
            saved = camera.live.lock().unwrap().events.drain(..).find_map(|e| match e {
                Event::Saved { path, size } => Some((path, size)),
                Event::Failed(why) => panic!("{why}"),
            });
        }
        let (got, size) = saved.expect("the photo is taken");
        assert_eq!(got, path);
        assert_eq!(size, (1640, 1232));
        assert_eq!(photos::dimensions(&path), Some((1640, 1232)));
        drop(camera);
        std::fs::remove_file(&path).ok();
    }
}
