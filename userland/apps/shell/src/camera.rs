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
//! preview, takes the sensor's full 3280 × 2464, copies the frame out and
//! starts the preview again ("captured"), then writes the JPEG (quality 95,
//! with Exif) through libjpeg-turbo behind it. The preview is held only for
//! the capture itself, under the shutter's flash.
//!
//! Detail mode (`d`: auto, on, off; Settings › Camera › Detail mode) takes a
//! burst of full-size frames instead, saves the first as the photo at once,
//! and merges the rest into it in the background (burst.rs): the thumbnail
//! wears a "refining" badge with the seconds left until the merged photo
//! takes its place.
//!
//! Scan mode reads QR codes from the preview's brightness, a few times a
//! second on a thread of its own: a Wi-Fi code offers to join the network, a
//! web address to open it in the Web app, anything else shows as text. With
//! the smart camera on the Grove port (smartcam.rs) its codes are taken too,
//! read on its own processor.
//!
//! The smart camera: with the Grove port set to it, the Camera holds it
//! while open. Its faces lean the exposure toward the people in the picture
//! (mapped into this camera's picture, `a` in the AI view to align the two),
//! and the lens switch (↑, the way a phone switches 1× and 0.5×) shows its
//! AI view: what it sees, at its own small size, with what it finds drawn
//! on it. It takes no photos: at 0.3 MP it is a seeing camera, not a
//! photographing one.
//!
//! On a computer, a simulated camera shows a scene of its own (and, in scan
//! mode, a Wi-Fi code on a card), and its photos are written by a JPEG
//! encoder in the shell, so everything works the same; the smart camera is
//! smartcam.rs's stand-in.
//!
//! Frames are drawn as they arrive (the camera's thread wakes the main
//! loop), and nothing here keeps the frame beat but motion: a camera at rest
//! is drawn at its own frame rate, not the panel's.

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
    /// The photo's picture is in: the preview runs again while it is written.
    Captured,
    /// Saved: its size, and whether a detail merge now refines it (what it
    /// took, "shot 812 write 640", goes to the shell's log).
    Saved { path: PathBuf, size: (u32, u32), detail: bool },
    Failed(String),
}

/// A saved photo's line from the camera program: its path, then after a tab
/// what it took and "detail N" when it is being refined.
fn saved_line(text: &str) -> (PathBuf, String, bool) {
    let (path, took) = text.split_once('\t').unwrap_or((text, ""));
    (PathBuf::from(path), took.to_owned(), took.contains("detail "))
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
    /// Detail mode: a burst merged in the background (`true`: only in dim light).
    Detail(PathBuf, bool),
    Night(bool),
    /// Where a face is in the picture (0 to 1), for the exposure; or none.
    Face(Option<[f32; 4]>),
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
                    Order::Detail(path, auto) => format!("detail {} {}\n", if auto { "auto" } else { "on" }, path.display()),
                    Order::Night(on) => format!("night {}\n", if on { "on" } else { "off" }),
                    Order::Face(Some(b)) => format!("face {:.3} {:.3} {:.3} {:.3}\n", b[0], b[1], b[2], b[3]),
                    Order::Face(None) => "face none\n".to_owned(),
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
            Err(_) if waited.elapsed() < Duration::from_secs(10) && !stop.load(Ordering::Relaxed) => std::thread::sleep(Duration::from_millis(40)),
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
                let (path, took, detail) = saved_line(&String::from_utf8_lossy(&frame));
                if !took.is_empty() {
                    eprintln!("camera: {} × {} {took}", number >> 16, number & 0xffff);
                }
                live.lock().map(|mut l| l.events.push(Event::Saved { path, size: (number >> 16, number & 0xffff), detail })).ok();
            }
            3 => {
                let why = String::from_utf8_lossy(&frame).trim().to_owned();
                live.lock().map(|mut l| l.events.push(Event::Failed(why))).ok();
            }
            4 => {
                live.lock().map(|mut l| l.events.push(Event::Captured)).ok();
            }
            _ => {}
        }
        // Drawn as it arrives, not on the frame beat.
        crate::platform::wake();
    };
    // Its input closed, it stops the camera and ends: given a moment for
    // that (and to finish writing a photo whose picture it has) before it
    // is made to.
    slot.lock().map(|mut s| s.take()).ok();
    let deadline = Instant::now() + Duration::from_secs(8);
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
    let (mut night, mut face) = (false, None);
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
        crate::platform::wake();
        while let Ok(order) = orders.try_recv() {
            match order {
                Order::Still(path) => sim_photo(live, path, t, scan, 0, None),
                // "Dim light" in the simulator: night on.
                Order::Detail(path, auto) => sim_photo(live, path, t, scan, if auto && !night { 0 } else { SIM_BURST }, face),
                Order::Night(on) => night = on,
                Order::Face(f) => face = f,
            }
        }
    }
    finish_luma(luma);
}

/// The simulator's burst: as many frames as the handheld takes with room to spare.
pub const SIM_BURST: usize = 6;
/// The sensor's full size, as every photo is taken.
pub const FULL: (usize, usize) = (3280, 2464);

/// The simulator's photo, as catalyst-camera takes one: the picture in
/// ("captured"), the JPEG written behind it; for detail mode a burst of a
/// dim, noisy scene (the robot still driving) merged on a thread of its own.
fn sim_photo(live: &Mutex<Live>, path: PathBuf, t: f32, scan: bool, burst: usize, face: Option<[f32; 4]>) {
    let asked = Instant::now();
    let (w, h) = FULL;
    let mut noise = 0x2545_F491_4F6C_DD1Du64 ^ (t.to_bits() as u64);
    let mut frames = Vec::new();
    for i in 0..burst.max(1) {
        // About 8 frames a second at full size, a hand's drift between them.
        let mut yuv = rgb_to_yuv420(&scene(w, h, t + i as f32 * 0.12, scan));
        if burst > 0 {
            dim_and_noisy(&mut yuv, w * h, &mut noise);
        }
        frames.push(yuv);
    }
    live.lock().map(|mut l| l.events.push(Event::Captured)).ok();
    crate::platform::wake();
    let shot = asked.elapsed().as_millis();
    let began = Instant::now();
    let event = match crate::burst::write_jpeg(&path, &frames[0], w, h, None) {
        Ok(()) => {
            let detail = burst > 0;
            if detail {
                std::fs::write(crate::burst::marker(&path), format!("began={}\nexpect={}\nprogress=0\n", crate::burst::now_ms(), crate::burst::expect_ms(burst, w, h))).ok();
                let (p, faces) = (path.clone(), face.into_iter().collect::<Vec<_>>());
                std::thread::Builder::new()
                    .name("merge".into())
                    .spawn(move || {
                        crate::sched::enter(crate::sched::Class::Idle);
                        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
                        if let Err(why) = crate::burst::refine(&p, &refs, w, h, &faces, 3) {
                            eprintln!("camera: detail merge: {why}");
                        }
                        crate::platform::wake();
                    })
                    .ok();
            }
            let took = format!("shot {shot} write {}{}", began.elapsed().as_millis(), if detail { format!(" detail {burst}") } else { String::new() });
            eprintln!("camera: {w} × {h} {took} (simulated)");
            Event::Saved { path, size: (w as u32, h as u32), detail }
        }
        Err(why) => Event::Failed(why),
    };
    live.lock().map(|mut l| l.events.push(event)).ok();
    crate::platform::wake();
}

/// A picture as YUV 4:2:0 (full-range BT.601, as the camera's), packed planes.
pub fn rgb_to_yuv420(pic: &Picture) -> Vec<u8> {
    let (w, h) = (pic.w, pic.h);
    let (cw, ch) = (w / 2, h / 2);
    let mut out = vec![0u8; w * h + 2 * cw * ch];
    let (y, uv) = out.split_at_mut(w * h);
    let (u, v) = uv.split_at_mut(cw * ch);
    for (i, p) in pic.rgb.chunks_exact(3).enumerate() {
        let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
        y[i] = ((77 * r + 150 * g + 29 * b + 128) >> 8).clamp(0, 255) as u8;
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = ((2 * cy + dy) * w + 2 * cx + dx) * 3;
                r += i32::from(pic.rgb[at]);
                g += i32::from(pic.rgb[at + 1]);
                b += i32::from(pic.rgb[at + 2]);
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            u[cy * cw + cx] = (128 + ((-43 * r - 85 * g + 128 * b + 128) >> 8)).clamp(0, 255) as u8;
            v[cy * cw + cx] = (128 + ((128 * r - 107 * g - 21 * b + 128) >> 8)).clamp(0, 255) as u8;
        }
    }
    out
}

/// The simulator's dim light: the picture darker, with a small sensor's
/// noise in it at high gain (brightness and, less, colour).
fn dim_and_noisy(yuv: &mut [u8], ysize: usize, seed: &mut u64) {
    let mut next = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        // Roughly normal: four uniforms.
        let u = |s: u64, k: u32| ((s >> k) & 0xFFFF) as f32 / 65535.0;
        (u(*seed, 0) + u(*seed, 16) + u(*seed, 32) + u(*seed, 48) - 2.0) * 1.732
    };
    for (i, p) in yuv.iter_mut().enumerate() {
        let (sigma, gain) = if i < ysize { (7.0, 0.62) } else { (4.0, 1.0) };
        let v = if i < ysize { f32::from(*p) * gain } else { 128.0 + (f32::from(*p) - 128.0) * 0.8 };
        *p = (v + next() * sigma).round().clamp(0.0, 255.0) as u8;
    }
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
    /// The photo's picture is in (the preview runs again); still writing.
    captured: bool,
    /// The newest photo while detail mode refines it, and when it was last
    /// looked at (its marker, twice a second).
    refining: Option<(PathBuf, crate::burst::Refining)>,
    refine_look: Instant,
    /// The dots of "saving…" as last drawn (only a change is a new frame).
    dots_drawn: usize,
    /// The lens in use, and the switch's spring between them (0 main, 1 AI).
    lens: Lens,
    lens_at: Animated,
    lens_fade: Animated,
    /// The smart camera, held while this is open and the port is its.
    smart: Option<(crate::smartcam::Use, crate::smartcam::Lease)>,
    smart_status: crate::smartcam::Status,
    /// Its picture (the AI view) and what it found, as last taken in.
    ai_seen: u64,
    ai_preview: Option<Picture>,
    ai_results: Option<crate::smartcam::Results>,
    /// Where its picture sits in this one's, and the alignment step.
    align: crate::smartcam::Align,
    aligning: bool,
    /// The face the exposure leans toward (in this camera's picture), and
    /// when it was last told to the camera program.
    face: Option<[f32; 4]>,
    face_told: (Instant, Option<[f32; 4]>),
    /// The seconds of "waking the smart camera" as last drawn.
    waking_drawn: u64,
    /// Held as set (the shots): nothing taken in, nothing moving.
    frozen: bool,
}

/// Settings › Camera › Smart camera: how it stands, and how fast its link
/// runs and it started, once it has.
pub fn smart_words() -> String {
    use crate::smartcam::Status;
    match crate::smartcam::status() {
        _ if crate::smartcam::switching() => "the port is changing over…".into(),
        _ if !crate::smartcam::chosen() => "off · the port is the console".into(),
        Status::PortIsConsole => "off · the port is the console".into(),
        Status::PortBusy => "waiting for the port".into(),
        Status::Asleep => "asleep".into(),
        Status::Waking { .. } => "waking…".into(),
        Status::Ready { baud, .. } => format!("ready · {}", baud_words(baud)),
        Status::Absent(_) => "not answering".into(),
    }
}

/// A baud rate as the Camera says it.
fn baud_words(baud: u32) -> String {
    if baud >= 1_000_000 {
        format!("{:.1} Mbaud", baud as f32 / 1e6)
    } else {
        format!("{} kbaud", baud / 1000)
    }
}

/// The lens switch: this camera, or the smart camera's AI view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    Main,
    Ai,
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
            captured: false,
            refining: None,
            refine_look: Instant::now(),
            dots_drawn: 0,
            lens: Lens::Main,
            // Bezel's mode buttons: the shape on SwiftUI's snappy (Detent's
            // release, 0.5 s, bounce 0.15), the colour and fade on Material
            // 3's default effects spring (stiffness 1600, no bounce).
            lens_at: Animated::new(0.0, role::release()).with_rest(0.002),
            lens_fade: Animated::new(0.0, role::effect()).with_rest(0.002),
            smart: None,
            smart_status: crate::smartcam::Status::Asleep,
            ai_seen: 0,
            ai_preview: None,
            ai_results: None,
            align: crate::smartcam::Align::load(),
            aligning: false,
            face: None,
            face_told: (Instant::now(), None),
            waking_drawn: 0,
            frozen: false,
        };
        camera.hold_smart();
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
        self.hold_smart();
    }

    /// The smart camera held for what this shows now (its codes in scan
    /// mode, everything in the AI view, faces otherwise), when the port is
    /// its. The new lease is taken before the old goes: no gap in its power.
    fn hold_smart(&mut self) {
        use crate::smartcam::Use;
        if !crate::smartcam::chosen() {
            self.smart = None;
            return;
        }
        let want = match (self.lens, self.mode) {
            (Lens::Ai, _) => Use::View,
            (_, Mode::Scan) => Use::Codes,
            _ => Use::Faces,
        };
        if self.smart.as_ref().map(|(u, _)| *u) != Some(want) {
            let lease = crate::smartcam::lease(want);
            self.smart = Some((want, lease));
        }
    }

    /// The lens switch, as a phone switches 1× and 0.5×: a spring between
    /// this camera's picture and the smart camera's.
    pub fn set_lens(&mut self, lens: Lens) {
        if self.lens == lens {
            return;
        }
        self.lens = lens;
        self.lens_at.set(if lens == Lens::Ai { 1.0 } else { 0.0 });
        self.lens_fade.set(if lens == Lens::Ai { 1.0 } else { 0.0 });
        self.aligning = false;
        self.hold_smart();
        if lens == Lens::Ai {
            self.say(match self.smart_status {
                crate::smartcam::Status::Ready { .. } => "AI view · a: align with the main camera",
                _ => "AI view: what the smart camera sees",
            });
        }
    }

    /// The face boxes, codes, tags and blobs the smart camera found, in
    /// its own picture's terms (0 to 1).
    fn smart_found(&self) -> Option<&crate::smartcam::Results> {
        self.ai_results.as_ref()
    }

    /// A box in the picture (0 to 1) on the viewfinder.
    fn on_viewfinder(vf: Rect, b: [f32; 4]) -> Rect {
        let (x0, y0) = (vf.x + b[0].min(b[2]) * vf.w, vf.y + b[1].min(b[3]) * vf.h);
        let (x1, y1) = (vf.x + b[0].max(b[2]) * vf.w, vf.y + b[1].max(b[3]) * vf.h);
        Rect::new(x0, y0, (x1 - x0).max(2.0), (y1 - y0).max(2.0))
    }

    /// The code being offered: read, and not put away.
    fn offered(&self) -> Option<&Found> {
        self.found.as_ref().filter(|_| self.mode == Mode::Scan && self.found_rise.target() > 0.5)
    }

    fn shoot(&mut self, calm: bool, detail: crate::burst::DetailMode) {
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
        let order = match detail {
            crate::burst::DetailMode::Off => Order::Still(path),
            crate::burst::DetailMode::On => Order::Detail(path, false),
            crate::burst::DetailMode::Auto => Order::Detail(path, true),
        };
        if !self.control.send(order) {
            self.say("the camera is not ready yet");
            return;
        }
        self.saving = Some(Instant::now());
        self.captured = false;
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

    /// Only the countdown's pulse keeps the frame beat: frames are drawn as
    /// they arrive (the camera wakes the loop), springs as they move.
    fn busy(&self) -> bool {
        self.countdown.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        // The alignment step: the smart camera's picture over this one's,
        // moved and sized by eye until they agree.
        if self.aligning {
            let step = 0.004;
            match key {
                Key::Left => self.align.ox -= step,
                Key::Right => self.align.ox += step,
                Key::Up => self.align.oy -= step,
                Key::Down => self.align.oy += step,
                Key::Char('+' | '=') => (self.align.sx, self.align.sy) = (self.align.sx * 1.01, self.align.sy * 1.01),
                Key::Char('-' | '_') => (self.align.sx, self.align.sy) = (self.align.sx / 1.01, self.align.sy / 1.01),
                Key::Char('r') => self.align = crate::smartcam::Align::default(),
                Key::Enter => {
                    self.aligning = false;
                    let kept = self.align.save().is_ok();
                    self.say(if kept { "aligned: faces now map onto this camera" } else { "could not keep the alignment" });
                }
                Key::Back | Key::Leave => {
                    self.aligning = false;
                    self.align = crate::smartcam::Align::load();
                    self.say("alignment left as it was");
                }
                _ => {}
            }
            return Nav::Stay;
        }
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
                Mode::Photo => self.shoot(cx.settings.calm, cx.settings.detail),
            },
            // The lens switch (↑, or l): this camera, or the smart camera's AI view.
            Key::Up | Key::Char('l') if !self.quick => self.set_lens(if self.lens == Lens::Main { Lens::Ai } else { Lens::Main }),
            Key::Char('d') => {
                cx.settings.detail = cx.settings.detail.step(1);
                crate::save_settings(cx.settings);
                self.say(match cx.settings.detail {
                    crate::burst::DetailMode::Auto => "detail: auto · merged from a burst in dim light",
                    crate::burst::DetailMode::On => "detail: on · every photo merged from a burst",
                    crate::burst::DetailMode::Off => "detail: off · one frame a photo",
                });
            }
            Key::Char('a') if self.lens == Lens::Ai => {
                if matches!(self.smart_status, crate::smartcam::Status::Ready { .. }) && self.preview.is_some() {
                    self.aligning = true;
                    self.say("arrows · +/− · enter: keep");
                } else {
                    self.say("align once both cameras show a picture");
                }
            }
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
            self.shoot(cx.settings.calm, cx.settings.detail);
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
        if self.frozen {
            return false;
        }
        self.clock += dt;
        let mut changed = self.mode_at.tick(dt) | self.flash.tick(dt) | self.press.tick(dt) | self.last_pop.tick(dt) | self.found_rise.tick(dt) | self.lens_at.tick(dt) | self.lens_fade.tick(dt);
        // The countdown pulses on the frame beat; the saving dots and the
        // seconds left are drawn only when they change.
        changed |= self.countdown.is_some();
        if self.saving.is_some() {
            let dots = (self.clock * 3.0) as usize % 3 + 1;
            changed |= dots != self.dots_drawn;
            self.dots_drawn = dots;
        }
        if let Some((_, age)) = &mut self.note {
            *age += dt;
            // Drawn while it fades in and out; held still between.
            let fading = *age < 0.2 || *age > NOTE_FOR;
            changed |= fading;
            if *age > NOTE_FOR + 0.4 {
                self.note = None;
                changed = true;
            }
        }
        changed |= self.tick_smart();
        changed |= self.tick_refining();
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
            match event {
                Event::Captured => self.captured = true,
                Event::Saved { path, size, detail } => {
                    self.saving = None;
                    photos::changed();
                    // Quick capture's photo: said in the island, and the camera gives way.
                    if self.quick {
                        crate::capture::photo_taken(path.clone());
                        self.done = true;
                    }
                    if detail {
                        // Refined in the background: its badge until the merged photo is in.
                        let expect = crate::burst::expect_ms(SIM_BURST, size.0 as usize, size.1 as usize);
                        let marking = crate::burst::read_marker(&path).unwrap_or(crate::burst::Refining { began_ms: crate::burst::now_ms(), expect_ms: expect, progress: 0.0 });
                        self.say(format!("saved · refining, about {} s", marking.left_s(crate::burst::now_ms())));
                        self.refining = Some((path.clone(), marking));
                        self.refine_look = Instant::now();
                    } else {
                        self.say(if size.0 >= 3000 { "saved to photos".to_owned() } else { format!("saved · {} × {} (memory was short)", size.0, size.1) });
                    }
                    self.load_last(Some(path));
                }
                Event::Failed(why) => {
                    self.saving = None;
                    self.say(why);
                }
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

        // The pictures: this camera's, and the smart camera's AI view, the
        // lens switch a spring between them. Going to the wider lens, this
        // picture draws back into the middle as it fades and the AI view
        // settles from the size this one shows of it, as a phone's 0.5×.
        if g.calm {
            self.lens_at.jump(self.lens_at.target());
            self.lens_fade.jump(self.lens_fade.target());
        }
        let k = self.lens_at.value();
        let ka = self.lens_fade.value().clamp(0.0, 1.0);
        if ka < 0.999 || self.aligning {
            match &self.preview {
                Some(pic) => {
                    let s = 1.0 - (1.0 - 1.0 / self.align.sx.max(1.0)) * k;
                    let dest = Rect::new(vf.cx() - vf.w * s * 0.5, vf.cy() - vf.h * s * 0.5, vf.w * s, vf.h * s);
                    draw_picture(g, dest, pic, whole(pic), 0.0, if self.aligning { 1.0 } else { 1.0 - ka });
                }
                None => {
                    let words = self.trouble.clone().unwrap_or_else(|| "starting the camera".into());
                    let words = g.text.fit(&words, &ty.caption, vf.w - 20.0);
                    let a = 1.0 - ka;
                    g.icon_alpha("photo_camera", false, vf.cx(), vf.cy() - 12.0, 20.0, Color(120, 120, 128), a);
                    let half = g.measure(&words, ty.caption) * 0.5;
                    g.text_alpha(vf.cx() - half, vf.cy() + 12.0, &words, ty.caption, Color(170, 170, 178), a);
                }
            }
        }
        if ka > 0.001 || self.aligning {
            self.draw_ai(g, vf, k, ka);
        }
        let white = Color(255, 255, 255);
        // The face the exposure leans toward, in this camera's picture.
        if let (Some(face), true) = (self.face, ka < 0.5 && self.mode == Mode::Photo) {
            let r = Self::on_viewfinder(vf, face);
            brackets(g, r, t.signal, 0.85 * (1.0 - 2.0 * ka));
        }
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
        // Taking it (the picture held), then writing it (a detail photo's
        // merge after that is the thumbnail's badge).
        let h = ty.caption.size + 8.0;
        let pill = |g: &mut Gfx, icon: Option<&str>, words: &str, widest: &str| {
            let lead = if icon.is_some() { h - 2.0 } else { 0.0 };
            let w = g.measure(widest, ty.caption) + 16.0 + lead;
            g.canvas.rrect(Rect::new(vf.x + 7.0 + g.ox, vf.bottom() - 7.0 - h, w, h), h * 0.5, Color(0, 0, 0), 0.55);
            if let Some(icon) = icon {
                g.icon(icon, true, vf.x + 7.0 + h * 0.5 + 1.0, vf.bottom() - 7.0 - h * 0.5, h * 0.55, t.signal);
            }
            g.text_at(vf.x + 15.0 + lead, vf.bottom() - 7.0 - h * 0.5, Align::Left, words, ty.caption, white);
        };
        if self.saving.is_some() {
            let dots = ".".repeat(self.dots_drawn.max(1));
            if self.captured {
                pill(g, None, &format!("saving{dots}"), "saving...");
            } else {
                pill(g, None, &format!("hold still{dots}"), "hold still...");
            }
        }
        // The lens switch, when there is a smart camera to switch to.
        if self.smart.is_some() && !self.quick {
            self.draw_lens_switch(g, vf);
        }

        self.draw_rail(g, rail);
    }
}

impl Camera {
    /// The smart camera's news: its status, its picture, what it found;
    /// the face handed to the exposure; its codes to scan mode. True if any
    /// of it shows.
    fn tick_smart(&mut self) -> bool {
        use crate::smartcam::Status;
        let mut changed = false;
        if self.smart.is_none() {
            return false;
        }
        let status = crate::smartcam::status();
        if status != self.smart_status {
            // It answered: how long it took to start, and how fast the link runs.
            if let (Status::Waking { .. } | Status::Asleep, Status::Ready { baud, woke_ms }) = (&self.smart_status, &status) {
                if self.lens == Lens::Ai {
                    self.say(format!("woke in {:.1} s · {}", *woke_ms as f32 / 1000.0, baud_words(*baud)));
                }
            }
            self.smart_status = status;
            changed = true;
        }
        // The seconds of waking, counted once a second.
        if let Status::Waking { since } = &self.smart_status {
            let s = since.elapsed().as_secs();
            if s != self.waking_drawn && self.lens == Lens::Ai {
                self.waking_drawn = s;
                changed = true;
            }
        }
        let showing_ai = self.lens == Lens::Ai || self.lens_at.value() > 0.01;
        let (n, pic) = crate::smartcam::preview();
        if n != self.ai_seen {
            self.ai_seen = n;
            self.ai_preview = pic;
            changed |= showing_ai;
        }
        let results = crate::smartcam::results(Duration::from_millis(700));
        if results != self.ai_results {
            self.ai_results = results;
            changed |= showing_ai;
        }
        // The face the exposure leans toward, told to the camera program
        // at most five times a second (and at once when it goes).
        let face = if self.lens == Lens::Main && self.mode == Mode::Photo { self.ai_results.as_ref().and_then(|r| crate::smartcam::main_face(r, &self.align)) } else { None };
        if face != self.face {
            self.face = face;
            changed = true;
        }
        let told = self.face_told.1;
        if (face.is_some() && face != told && self.face_told.0.elapsed() >= Duration::from_millis(200)) || (face.is_none() && told.is_some()) {
            self.control.send(Order::Face(face));
            self.face_told = (Instant::now(), face);
        }
        // Scan mode: its codes, read on its own processor; the handheld's
        // own reader rests while it answers.
        if self.mode == Mode::Scan {
            let ready = matches!(self.smart_status, Status::Ready { .. });
            self.scanning.store(!ready, Ordering::Relaxed);
            if let Some(code) = self.ai_results.as_ref().and_then(|r| r.codes.first()).cloned() {
                let read = read_code(&code.text);
                if self.dismissed.as_ref() != Some(&read) {
                    let fresh = self.found_rise.target() < 0.5 || self.found.as_ref().map(|f| &f.code) != Some(&read);
                    let b = self.align.map(code.at);
                    let (w, h) = (PREVIEW_W as f32, PREVIEW_H as f32);
                    let corners = [(b[0] * w, b[1] * h), (b[2] * w, b[1] * h), (b[2] * w, b[3] * h), (b[0] * w, b[3] * h)];
                    self.found = Some(Found { code: read, corners });
                    self.found_at = Some(Instant::now());
                    if fresh {
                        self.found_rise.jump(0.0);
                        self.found_rise.set(1.0);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// The AI view: the smart camera's picture (settling from the part of it
    /// this camera shows to the whole, `k`), at `alpha`, with what it found;
    /// or, before it has a picture, how it stands, calmly. In the alignment
    /// step, its picture at half strength over this camera's, where the
    /// alignment puts it.
    fn draw_ai(&mut self, g: &mut Gfx, vf: Rect, k: f32, alpha: f32) {
        use crate::smartcam::Status;
        let ty = g.ty;
        if self.aligning {
            if let Some(pic) = &self.ai_preview {
                let a = self.align;
                // The part of its picture that falls inside this one's.
                let (u0, u1) = (0.5 + (-0.5 - a.ox) / a.sx, 0.5 + (0.5 - a.ox) / a.sx);
                let (v0, v1) = (0.5 + (-0.5 - a.oy) / a.sy, 0.5 + (0.5 - a.oy) / a.sy);
                let (pw, ph) = (pic.w as f32, pic.h as f32);
                draw_picture(g, vf, pic, [u0 * pw, v0 * ph, (u1 - u0) * pw, (v1 - v0) * ph], 0.0, 0.5);
            }
            return;
        }
        let ready = matches!(self.smart_status, Status::Ready { .. });
        if let (true, Some(pic)) = (ready, &self.ai_preview) {
            let s = 1.0 + (self.align.sx - 1.0).max(0.0) * (1.0 - k);
            let (vw, vh) = (pic.w as f32 / s, pic.h as f32 / s);
            draw_picture(g, vf, pic, [(pic.w as f32 - vw) * 0.5, (pic.h as f32 - vh) * 0.5, vw, vh], 0.0, alpha);
            let shown = ((alpha - 0.6) / 0.4).clamp(0.0, 1.0);
            if shown > 0.0 {
                if let Some(found) = self.smart_found().cloned() {
                    self.draw_found(g, vf, &found, shown);
                }
            }
            return;
        }
        // How it stands, over the dark viewfinder.
        g.canvas.fill_rect(vf.offset(g.ox, 0.0), Color(6, 6, 8), alpha);
        let (icon, line, more) = match &self.smart_status {
            Status::PortIsConsole => ("cable", "the Grove port is the console".to_owned(), "Settings › Camera › Grove port".to_owned()),
            Status::PortBusy => ("cable", "the Grove port is not free yet".to_owned(), "a login or the console still on it".to_owned()),
            Status::Absent(_) => ("developer_board", "no smart camera answers".to_owned(), "its program: plug it into USB-A once".to_owned()),
            Status::Waking { since } => ("developer_board", "waking the smart camera…".to_owned(), format!("{} s", since.elapsed().as_secs())),
            Status::Ready { .. } => ("developer_board", "waiting for its picture".to_owned(), String::new()),
            Status::Asleep => ("developer_board", "waking the smart camera…".to_owned(), String::new()),
        };
        g.icon_alpha(icon, false, vf.cx(), vf.cy() - 18.0, 22.0, Color(150, 150, 160), alpha);
        let line = g.text.fit(&line, &ty.caption, vf.w - 20.0);
        let half = g.measure(&line, ty.caption) * 0.5;
        g.text_alpha(vf.cx() - half, vf.cy() + 6.0, &line, ty.caption, Color(214, 214, 220), alpha);
        if !more.is_empty() {
            let more = g.text.fit(&more, &ty.caption, vf.w - 20.0);
            let half = g.measure(&more, ty.caption) * 0.5;
            g.text_alpha(vf.cx() - half, vf.cy() + 10.0 + ty.caption.size, &more, ty.caption, Color(140, 140, 150), alpha);
        }
    }

    /// What the smart camera found, on its picture: faces in brackets, codes
    /// and colour blobs outlined, tags marked with their id and distance.
    fn draw_found(&self, g: &mut Gfx, vf: Rect, found: &crate::smartcam::Results, alpha: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let white = Color(255, 255, 255);
        // A label at (x, y), or ending at x when left.
        let label = |g: &mut Gfx, x: f32, y: f32, words: &str, left: bool| {
            let words = g.text.fit(words, &ty.caption, vf.w * 0.55);
            let w = g.measure(&words, ty.caption) + 10.0;
            let h = ty.caption.size + 6.0;
            let x = if left { x - w } else { x }.clamp(vf.x + 2.0, vf.right() - w - 2.0);
            let y = y.clamp(vf.y + 2.0, vf.bottom() - h - 2.0);
            g.canvas.rrect(Rect::new(x + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.6 * alpha);
            g.text_alpha(x + 5.0, y + h * 0.5, &words, ty.caption, white, alpha);
        };
        for face in &found.faces {
            brackets(g, Self::on_viewfinder(vf, face.at), t.signal, 0.9 * alpha);
        }
        for code in &found.codes {
            let r = Self::on_viewfinder(vf, code.at);
            outline(g, r, 1.5, white, 0.9 * alpha);
            // Beside it, on the side with more room, level with its middle.
            let left = r.cx() > vf.cx();
            label(g, if left { r.x - 4.0 } else { r.right() + 4.0 }, r.cy() - (ty.caption.size + 6.0) * 0.5, &code.text, left);
        }
        for tag in &found.tags {
            let (cx, cy) = (vf.x + tag.centre[0] * vf.w, vf.y + tag.centre[1] * vf.h);
            let r = Rect::new(cx - 12.0, cy - 12.0, 24.0, 24.0);
            outline(g, r, 2.0, t.ice, alpha);
            let metres = (tag.pose_mm[0].powi(2) + tag.pose_mm[1].powi(2) + tag.pose_mm[2].powi(2)).sqrt() / 1000.0;
            label(g, r.right() + 4.0, cy - (ty.caption.size + 6.0) * 0.5, &format!("tag {} · {metres:.1} m", tag.id), false);
        }
        for blob in &found.blobs {
            let colour = [Color(255, 149, 0), Color(255, 214, 10), Color(255, 69, 58), Color(10, 132, 255)][usize::from(blob.colour.min(3))];
            outline(g, Self::on_viewfinder(vf, blob.at), 1.5, colour, 0.9 * alpha);
        }
    }

    /// The lens switch: Bezel's mode buttons in small, on clear glass over
    /// the picture (it floats over media), the one in use an ice capsule
    /// that slides between them.
    fn draw_lens_switch(&mut self, g: &mut Gfx, vf: Rect) {
        let t = g.tokens;
        let cap = g.ty.caption;
        let d = (cap.size + 10.0).round();
        let pad = 3.0;
        let (w, h) = (2.0 * d + 3.0 * pad, d + 2.0 * pad);
        let r = Rect::new((vf.right() - 8.0 - w).round(), (vf.bottom() - 8.0 - h).round(), w, h);
        g.glass(&[bezel::GlassShape { clear: true, ..bezel::GlassShape::new(r.offset(g.ox, 0.0), h * 0.5, 11) }]);
        // Concentric: the capsule's radius is the track's less its padding.
        let k = self.lens_at.value();
        let x = r.x + pad + (d + pad) * k;
        g.canvas.rrect(Rect::new(x + g.ox, r.y + pad, d, d), h * 0.5 - pad, t.ice, 1.0);
        let fade = self.lens_fade.value().clamp(0.0, 1.0);
        for (i, words) in ["1×", "AI"].iter().enumerate() {
            let on = if i == 0 { 1.0 - fade } else { fade };
            let ink = mix(t.ink, t.on_ice, on);
            let cx = r.x + pad + d * 0.5 + i as f32 * (d + pad);
            g.text_at(cx, r.cy(), Align::Center, words, cap, ink);
        }
    }

    /// The newest photo being refined: its marker looked at twice a second;
    /// gone, the merged photo is in and its thumbnail is taken again.
    fn tick_refining(&mut self) -> bool {
        let Some((path, was)) = self.refining.clone() else { return false };
        if self.refine_look.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.refine_look = Instant::now();
        let now = crate::burst::now_ms();
        match crate::burst::read_marker(&path) {
            Some(r) => {
                let moved = r.left_s(now) != was.left_s(now) || (r.progress - was.progress).abs() > 0.05;
                self.refining = Some((path, r));
                moved || was.left_s(now) != was.left_s(now.saturating_sub(500))
            }
            // Not written yet (the merge just starting): given a moment.
            None if now.saturating_sub(was.began_ms) < 3000 => false,
            None => {
                self.refining = None;
                photos::changed();
                self.load_last(Some(path));
                self.say("detail merged into the photo");
                true
            }
        }
    }

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
        // Being refined: a badge with the seconds left where the ↓ is.
        match &self.refining {
            Some((_, r)) => {
                let s = format!("{}s", r.left_s(crate::burst::now_ms()));
                let bh = cap.size + 4.0;
                let bw = g.measure(&s, cap) + bh + 2.0;
                let b = Rect::new((thumb.cx() - bw * 0.5).round(), thumb.y - 2.0 - bh, bw, bh);
                g.canvas.rrect(b.offset(g.ox, 0.0), bh * 0.5, t.surface2, 1.0);
                g.icon("auto_awesome", true, b.x + bh * 0.5, b.cy(), bh * 0.6, t.signal);
                g.text_at(b.x + bh - 1.0, b.cy(), Align::Left, &s, cap, t.ink);
            }
            None => {
                g.text_at(thumb.cx(), thumb.y - 2.0 - cap.size * 0.5, Align::Center, "↓", cap, t.faint);
            }
        }
        // In the AI view: the smart camera's pace and the link's rate.
        if let (Lens::Ai, crate::smartcam::Status::Ready { baud, .. }) = (self.lens, &self.smart_status) {
            let fps = self.ai_results.as_ref().map_or(0.0, |r| r.fps);
            let full = format!("{fps:.0} fps · {}M", (*baud as f32 / 1e6 * 10.0).round() / 10.0);
            let words = if g.measure(&full, cap) <= rail.w - 8.0 { full } else { format!("{fps:.0} fps") };
            g.text_at(rail.cx(), PAD + cap.size * 2.0 + 10.0, Align::Center, &words, cap, t.faint);
        }
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
        // Detail mode: on in ink, auto dimmer, off faint.
        let color = match g.detail {
            crate::burst::DetailMode::On => t.ink,
            crate::burst::DetailMode::Auto => t.dim,
            crate::burst::DetailMode::Off => t.faint,
        };
        let y = y - cap.size - 3.0;
        g.icon("auto_awesome", g.detail == crate::burst::DetailMode::On, right - 18.0, y, 11.0, color);
        g.text_at(right - 6.0, y, Align::Center, "d", cap, color);
    }
}

/// A face's corners, as a camera marks one: four short brackets.
fn brackets(g: &mut Gfx, r: Rect, color: Color, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let arm = (r.w.min(r.h) * 0.28).clamp(4.0, 14.0);
    let th = 2.0;
    for (cx, cy, sx, sy) in [(r.x, r.y, 1.0, 1.0), (r.right(), r.y, -1.0, 1.0), (r.right(), r.bottom(), -1.0, -1.0), (r.x, r.bottom(), 1.0, -1.0)] {
        let h = Rect::new(if sx > 0.0 { cx } else { cx - arm }, if sy > 0.0 { cy } else { cy - th }, arm, th);
        let v = Rect::new(if sx > 0.0 { cx } else { cx - th }, if sy > 0.0 { cy } else { cy - arm }, th, arm);
        g.canvas.rrect(h.offset(g.ox, 0.0), 1.0, color, alpha);
        g.canvas.rrect(v.offset(g.ox, 0.0), 1.0, color, alpha);
    }
}

/// A box's outline, `width` thick.
fn outline(g: &mut Gfx, r: Rect, width: f32, color: Color, alpha: f32) {
    for side in [Rect::new(r.x, r.y, r.w, width), Rect::new(r.x, r.bottom() - width, r.w, width), Rect::new(r.x, r.y, width, r.h), Rect::new(r.right() - width, r.y, width, r.h)] {
        g.canvas.fill_rect(side.offset(g.ox, 0.0), color, alpha);
    }
}

/// `a` toward `b` by `k`.
fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * k).round() as u8;
    Color(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
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

/// The Camera's 0.9 pictures (`catalyst-shell --camera-shots DIR`): the
/// lens switch at rest and on its way, the smart camera waking and its AI
/// view, taking, saving and refining a photo, the alignment step, and
/// Settings › Camera, at each text size; and a detail merge's before and
/// after (a 1:1 crop, twice up), timed.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    use crate::smartcam::{sim, Blob, Code, Face, Results, Status, Tag};
    std::fs::create_dir_all(dir).ok();
    sim::choose(true);
    let t = 3.1;
    let face = [0.42, 0.28];
    let ai_pic = sim::view(t, face);
    // The same person in this camera's narrower picture, where the
    // alignment puts them.
    let mut main_pic = scene(PREVIEW_W, PREVIEW_H, t, false);
    {
        let a = crate::smartcam::Align::default();
        let (cx, cy) = ((face[0] * 320.0 + 29.0) / 320.0, (face[1] * 240.0 + 35.0) / 240.0);
        let m = a.map([cx, cy, cx, cy]);
        sim::person(&mut main_pic, m[0] * PREVIEW_W as f32, m[1] * PREVIEW_H as f32, a.sx);
    }
    let found = Results {
        frame: 1,
        fps: 14.6,
        faces: vec![Face { at: [face[0], face[1], face[0] + 58.0 / 320.0, face[1] + 70.0 / 240.0], score: 0.87 }],
        codes: vec![Code { at: [212.0 / 320.0, 128.0 / 240.0, 264.0 / 320.0, 180.0 / 240.0], text: "SCOUT 5805 Q12 auto=3 tele=11".into() }],
        tags: vec![Tag { id: 7, centre: [70.0 / 320.0, 62.0 / 240.0], turn: 0.1, pose_mm: [-412.0, -88.0, 2640.0] }],
        blobs: vec![Blob { colour: 0, at: [40.0 / 320.0, 176.0 / 240.0, 74.0 / 320.0, 204.0 / 240.0], pixels: 640 }],
    };
    let ready = Status::Ready { baud: 3_000_000, woke_ms: 1420 };
    type Setup = Box<dyn Fn(&mut Camera)>;
    let (m, a, f, r) = (main_pic.clone(), ai_pic.clone(), found.clone(), ready.clone());
    let base = move |c: &mut Camera| {
        c.frozen = true;
        c.preview = Some(m.clone());
        c.ai_preview = Some(a.clone());
        c.ai_results = Some(f.clone());
        c.smart_status = r.clone();
        c.note = None;
        c.align = crate::smartcam::Align::default();
    };
    let base = std::rc::Rc::new(base);
    let with = |base: &std::rc::Rc<dyn Fn(&mut Camera)>, more: Box<dyn Fn(&mut Camera)>| -> Setup {
        let base = base.clone();
        Box::new(move |c: &mut Camera| {
            base(c);
            more(c);
        })
    };
    let base: std::rc::Rc<dyn Fn(&mut Camera)> = base;
    let lens = |c: &mut Camera, k: f32, fade: f32| {
        c.lens_at.jump(k);
        c.lens_fade.jump(fade);
    };
    let note = |c: &mut Camera, words: &str| c.note = Some((words.to_owned(), 1.0));
    let shots: Vec<(&str, crate::burst::DetailMode, Setup)> = vec![
        ("1-main-face", crate::burst::DetailMode::Auto, with(&base, Box::new(move |c| {
            let aligned = crate::smartcam::main_face(c.ai_results.as_ref().unwrap(), &c.align);
            c.face = aligned;
        }))),
        ("2-switching", crate::burst::DetailMode::Auto, with(&base, Box::new(move |c| {
            c.lens = Lens::Ai;
            lens(c, 0.55, 0.5);
        }))),
        ("3-waking", crate::burst::DetailMode::Auto, with(&base, Box::new(move |c| {
            c.lens = Lens::Ai;
            lens(c, 1.0, 1.0);
            c.ai_preview = None;
            c.smart_status = Status::Waking { since: Instant::now() - Duration::from_millis(1100) };
        }))),
        ("4-ai-view", crate::burst::DetailMode::Auto, with(&base, Box::new(move |c| {
            c.lens = Lens::Ai;
            lens(c, 1.0, 1.0);
            note(c, "woke in 1.4 s · 3.0 Mbaud");
        }))),
        ("5-hold-still", crate::burst::DetailMode::On, with(&base, Box::new(move |c| {
            c.saving = Some(Instant::now());
            c.dots_drawn = 3;
            c.flash.jump(0.0);
        }))),
        ("6-saving", crate::burst::DetailMode::On, with(&base, Box::new(move |c| {
            c.saving = Some(Instant::now());
            c.captured = true;
            c.dots_drawn = 2;
        }))),
        ("7-refining", crate::burst::DetailMode::On, with(&base, Box::new(move |c| {
            let began = crate::burst::now_ms() - 1500;
            let r = crate::burst::Refining { began_ms: began, expect_ms: 7500, progress: 0.2 };
            c.note = Some((format!("saved · refining, about {} s", r.left_s(crate::burst::now_ms())), 1.0));
            c.refining = Some((PathBuf::from("IMG_refining.jpg"), r));
            c.last = Some((Photo { path: PathBuf::from("IMG_refining.jpg"), name: "IMG_refining.jpg".into(), taken: chrono::Local::now().naive_local(), bytes: 0 }, scene(72, 54, t, false)));
        }))),
        ("8-aligning", crate::burst::DetailMode::Off, with(&base, Box::new(move |c| {
            c.lens = Lens::Ai;
            lens(c, 1.0, 1.0);
            c.aligning = true;
            c.align.ox = 0.03;
            note(c, "arrows · +/− · enter: keep");
        }))),
    ];
    for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
        for (name, detail, setup) in &shots {
            let mut camera = Camera::new();
            setup(&mut camera);
            let mut shell = Shell::new(crate::robot::Robot::demo());
            (shell.settings.text, shell.settings.detail) = (size, *detail);
            shell.apply_settings();
            shell.show(Box::new(camera));
            for _ in 0..90 {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
            shell.draw();
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tag}.png")), 3);
        }
        let mut shell = Shell::new(crate::robot::Robot::demo());
        shell.settings.text = size;
        shell.apply_settings();
        shell.show(Box::new(crate::screens::SettingsApp::camera_page()));
        for _ in 0..90 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("9-settings-camera-{tag}.png")), 3);
    }
    // Detail mode's before and after: six full-size frames of the dim pit,
    // the robot driving through them, merged as the handheld merges them.
    let (w, h) = FULL;
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let frames: Vec<Vec<u8>> = (0..SIM_BURST)
        .map(|i| {
            let mut yuv = rgb_to_yuv420(&scene(w, h, t + i as f32 * 0.12, false));
            dim_and_noisy(&mut yuv, w * h, &mut seed);
            yuv
        })
        .collect();
    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let began = Instant::now();
    let (merged, report) = crate::burst::merge(&refs, w, h, &[], 3, &mut |_| {});
    let took = began.elapsed();
    let crop = |yuv: &[u8]| {
        let mut rgb = Picture::default();
        yuv420_to_rgb(yuv, w, h, &mut rgb);
        // The robot's bumper and the carpet's lines, 1:1.
        let (cw, ch, x0, y0) = (480, 320, 1300, 1720);
        let mut out = Picture::new(cw, ch);
        for y in 0..ch {
            let from = ((y0 + y) * w + x0) * 3;
            out.rgb[y * cw * 3..(y + 1) * cw * 3].copy_from_slice(&rgb.rgb[from..from + cw * 3]);
        }
        out
    };
    for (name, pic) in [("detail-before", crop(&frames[0])), ("detail-after", crop(&merged))] {
        let mut canvas = bezel::Canvas::new(pic.w as i32, pic.h as i32);
        for (i, p) in pic.rgb.chunks_exact(3).enumerate() {
            canvas.px[i] = (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]);
        }
        crate::platform::sim::save_png(&canvas, &dir.join(format!("{name}.png")), 2);
    }
    println!(
        "camera shots in {} · detail merge of {} frames at {w} × {h}: {} ms here on 3 threads (align {} + merge {} + finish {}), {:.0}% of the other frames kept, noise {:.1}",
        dir.display(),
        report.frames,
        took.as_millis(),
        report.align_ms,
        report.merge_ms,
        report.finish_ms,
        report.kept * 100.0,
        report.noise
    );
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
        // The handheld's own reader: no smart camera on the port.
        let _port = crate::smartcam::sim::exclusive(false);
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
        let _port = crate::smartcam::sim::exclusive(false);
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
        let mut captured = false;
        while saved.is_none() && started.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(30));
            saved = camera.live.lock().unwrap().events.drain(..).find_map(|e| match e {
                Event::Captured => {
                    captured = true;
                    None
                }
                Event::Saved { path, size, detail } => Some((path, size, detail)),
                Event::Failed(why) => panic!("{why}"),
            });
        }
        let (got, size, detail) = saved.expect("the photo is taken");
        assert!(captured, "its picture was in before it was written");
        assert_eq!(got, path);
        // Every photo at the sensor's full size.
        assert_eq!(size, (3280, 2464));
        assert_eq!(photos::dimensions(&path), Some((3280, 2464)));
        assert!(!detail, "a plain photo is not refined");
        drop(camera);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_camera_programs_saved_line_is_read_with_its_timing() {
        let (path, took, detail) = saved_line("/data/photos/IMG_1.jpg\tshot 812 write 640 detail 6");
        assert_eq!(path, PathBuf::from("/data/photos/IMG_1.jpg"));
        assert_eq!(took, "shot 812 write 640 detail 6");
        assert!(detail);
        // An older program's line: the path alone.
        assert_eq!(saved_line("/data/photos/IMG_2.jpg"), (PathBuf::from("/data/photos/IMG_2.jpg"), String::new(), false));
    }

    #[test]
    fn the_lens_switch_springs_to_the_ai_view_and_back_and_holds_what_it_shows() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(true);
        let mut camera = Camera::new();
        run(&mut camera, 100);
        assert_eq!(camera.lens, Lens::Main);
        assert_eq!(camera.smart.as_ref().map(|(u, _)| *u), Some(crate::smartcam::Use::Faces), "faces for the exposure");
        // ↑: to the AI view, on a spring (part way after a frame, there after a while).
        assert!(matches!(press(&mut camera, Key::Up), Nav::Stay));
        assert_eq!(camera.lens, Lens::Ai);
        camera.tick(1.0 / 60.0);
        let early = camera.lens_at.value();
        assert!(early > 0.0 && early < 0.9, "moving: {early}");
        assert_eq!(camera.smart.as_ref().map(|(u, _)| *u), Some(crate::smartcam::Use::View), "everything, with its picture");
        run(&mut camera, 900);
        assert!((camera.lens_at.value() - 1.0).abs() < 0.01);
        // Scan mode from there still shows the AI view; back with ↑ (or l).
        assert!(matches!(press(&mut camera, Key::Char('l')), Nav::Stay));
        assert_eq!(camera.lens, Lens::Main);
        press(&mut camera, Key::Right);
        assert_eq!(camera.smart.as_ref().map(|(u, _)| *u), Some(crate::smartcam::Use::Codes), "codes in scan mode");
    }

    #[test]
    fn the_ai_view_wakes_the_smart_camera_and_shows_what_it_finds() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(true);
        let mut camera = Camera::new();
        press(&mut camera, Key::Up);
        let started = Instant::now();
        while (camera.ai_preview.is_none() || camera.ai_results.is_none()) && started.elapsed() < Duration::from_secs(15) {
            run(&mut camera, 50);
        }
        assert!(matches!(camera.smart_status, crate::smartcam::Status::Ready { .. }), "{:?}", camera.smart_status);
        let found = camera.ai_results.clone().expect("results");
        assert!(!found.faces.is_empty() && !found.tags.is_empty());
        // Back on the main lens, the face reaches this camera's picture.
        press(&mut camera, Key::Up);
        run(&mut camera, 300);
        assert!(camera.face.is_some(), "the face for the exposure");
    }
}
