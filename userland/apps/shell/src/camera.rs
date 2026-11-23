//! The Camera: the IMX219's picture in colour, photos kept in the album
//! (Photos), and QR codes read.
//!
//! The viewfinder takes the panel's height at the photo's own shape (4:3),
//! with the controls in a rail beside it: the modes (scan, photo, pro: a
//! carousel, as a phone's camera has them), the shutter, and the last photo
//! taken (↓ opens the album on it). Enter takes a photo; `c` sets the
//! self-timer (2 or 10 s; `t` too, in photo mode), `g` lays a grid over the
//! picture (thirds, or squares), `h` the level, `o` frames it 3:2, 16:9 or
//! 1:1 (and crops the photo so), `n` lets the exposure run longer in low
//! light, `f` opens the looks, `z` the 1:1 focus check. camkit.rs has the
//! kit (0.9.4) and its keys in pro mode.
//!
//! On the handheld the picture comes from `catalyst-camera --colour` (the
//! image's own libcamera program): a 320 × 240 YUV 4:2:0 preview at up to 30
//! frames a second, turned the right way up for the CardputerZero, each with
//! what the camera said of it (exposure, gains, white balance, light; read
//! by pro.rs). A photo is asked of it by name, with what to take it with
//! ("still /data/photos/IMG_….jpg", then the exposure the preview settled
//! on, its white balance gains, the look's table): it stops the preview,
//! takes the sensor's full 3280 × 2464 with all of that fixed, so the first
//! frame is the photo and nothing settles again, copies the frame out and
//! starts the preview again ("captured"), then writes the JPEG (quality 95,
//! with Exif) on three cores behind it.
//!
//! Every photo is timed (camkit.rs's Timeline: the key, the order, the
//! sensor exposing, done, the picture in, written), in the log and, in pro
//! mode on `t`, a readout ("last shot: 0.42 s capture · 0.7 s save").
//! "Hold still" shows only while the sensor exposes, and only when that is
//! long enough to matter (a slow shutter, detail's burst, a bracketing
//! set), with the seconds left. 0.9.3 showed it from the key until the
//! picture was in, which could be ten seconds and more when the camera
//! program waited out a try that would never give a frame (catalyst-camera
//! 1.5 now gives such a try up at once).
//!
//! The shutter answers at once: the frame in the viewfinder as the key went
//! down, with the look over it, is the photo's thumbnail straight away (it
//! flies into the corner) while the full-size photo is taken and saved, and
//! the next can be taken as soon as the camera has the last one's picture,
//! before it is written.
//!
//! Pro mode (pro.rs): shutter (`s`), ISO (`i`), exposure compensation (`e`)
//! and white balance (`w`), dials in a bar along the bottom (← → turn the
//! one chosen, ↑ ↓ go to the next, `a` gives it back to auto). The aperture
//! is the lens's own f/2.0, said in the rail and never a dial. With them a
//! live histogram, zebras on highlights at 98% (`x`), RAW (`r`: a DNG of the
//! sensor's own frame beside the JPEG, dng.rs), and the shutter time always
//! in sight, a hand beside it when it is slower than 1/30 s (photo mode
//! shows that too, when its own exposure gets that long).
//!
//! Looks (filters.rs, `f`): a strip of live thumbnails along the bottom, ←
//! → to choose, the viewfinder showing the look as it will be saved, and
//! Tint's colour on ↑ ↓. The camera program lays it over the photo in its
//! background save and writes it into the Exif; the photo as taken is kept
//! beside it (Photos shows it with `o`).
//!
//! The focus check (`z`): the sensor's own pixels one to one on the panel
//! (the preview changes to the full sensor and is cropped), arrows to look
//! around, focus peaking marking what is sharp. The lens is fixed-focus:
//! 3.04 mm at f/2.0, set at its hyperfocal distance (about 2 m, for a
//! circle of confusion of two pixels, 2.2 µm), is sharp from about a metre
//! to the far distance. Many IMX219 modules have their lens in a thread,
//! held by a dot of glue, and the tool sold with them (or fine tweezers)
//! turns it to focus nearer; whether this one's does is down to the module.
//!
//! Crisp photos: in auto, the exposure is split for a hand (no slower than
//! 1/60 s while there is gain to raise, then 1/40, then 1/30); at little
//! gain the sensor's noise filter is off and its sharpening firmer; the
//! photo is the sensor's own 3280 × 2464, never scaled. Its colour is the
//! ISP's 4:2:0 at quality 95, all the ISP gives (4:4:4 would need a 24 MB
//! frame the camera's memory cannot hold beside the raw one).
//!
//! Detail mode (`d`: auto, on, off; Settings › Camera › Detail mode) takes a
//! burst of full-size frames instead, saves the first as the photo at once,
//! and merges the rest into it in the background (burst.rs): the thumbnail
//! wears a "refining" badge with the seconds left until the merged photo
//! takes its place. Its "auto" is for real low light only (pro.rs's
//! `low_light`: under 25 lux and the photo out of gain at 1/20 s, both;
//! 0.9.3 took either, so an evening room was a burst every time).
//!
//! Scan mode reads QR codes from the preview's brightness, a few times a
//! second on a thread of its own: a Wi-Fi code offers to join the network, a
//! web address to open it in the Web app, anything else shows as text. With
//! the smart camera on the Grove port (smartcam.rs) its codes are taken too,
//! read on its own processor.
//!
//! Doc mode (1.0, the carousel's fourth) scans documents: the page found
//! in the preview a few times a second on a thread of its own and outlined,
//! taken by itself once held still for a second (the shutter's ring
//! filling meanwhile) or at once with Enter, as the camera's own full-size
//! still; then flattened, enhanced and gathered into a document that is
//! saved as a PDF and PNGs (docscan/). ↓ shows the pages taken.
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
//! mode, a Wi-Fi code on a card), meters it and obeys the pro dials, and
//! its photos (and DNGs) are written by the shell, so everything works the
//! same; the smart camera is smartcam.rs's stand-in.
//!
//! Frames are developed on the camera's thread (colour conversion, look,
//! zebras and histogram in one pass, filters.rs), drawn as they arrive (the
//! thread wakes the main loop) and taken in as ambient change, not motion
//! (0.9.4): a camera at rest is drawn at its own frame rate, not the
//! panel's, and its waits (the timer, "hold still", the intervalometer)
//! look in only as often as what they show changes.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::camkit::{self, Aspect, Grid, HistMode, Kit, Mark, Program, Timeline};
use crate::filters::{Compiled, Look, LOOKS};
use crate::gfx::{Gfx, H, PAD, W};
use crate::photos::{self, draw_picture, whole, Photo, Picture};
use crate::pro::{self, Metered, Pro};
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
            Code::Text(text) if crate::strategy::is_board_code(text) => crate::strategy::card(text),
            Code::Text(text) if crate::team::is_team_code(text) => crate::team::card(text),
            Code::Text(text) => match crate::scout::scan_words(text) {
                // Scout's codes import as they are read.
                Some((title, action)) => ("download", title, action),
                None => ("notes", text.lines().next().unwrap_or("").to_owned(), "enter: read it all"),
            },
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
    /// The sensor is exposing the photo (catalyst-camera's "<ms> <frames>
    /// <what>"), and done with it.
    Exposing(String),
    Exposed,
    /// The photo's picture is in: the preview runs again while it is
    /// written; the camera program's timing marks.
    Captured(String),
    /// Saved: its size, and whether a detail merge now refines it (what it
    /// took, "shot 812 write 640", goes to the shell's log), and whether a
    /// RAW asked for found no room in the camera's memory.
    Saved { path: PathBuf, size: (u32, u32), detail: bool, raw_skipped: bool },
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
    /// What the camera said of the newest frame, its histogram, and the
    /// frame itself as it came (for the shutter's instant thumbnail).
    meta: Option<Metered>,
    histogram: pro::Histogram,
    /// Its red, green and blue, when the RGB histogram shows.
    rgb: Option<camkit::RgbHistogram>,
    yuv: Vec<u8>,
    yuv_size: (usize, usize),
    /// The looks' live thumbnails (while the strip is open), in its order.
    thumbs: Vec<Picture>,
    thumbs_made: u64,
}

/// How the camera's thread develops each frame, as the screen last asked.
#[derive(Clone)]
struct Develop {
    look: Arc<Compiled>,
    zebras: bool,
    /// Focus peaking, in this colour.
    peaking: Option<[u8; 3]>,
    /// The strip's looks, for their thumbnails.
    strip: Option<Arc<Vec<Compiled>>>,
    /// False colour in place of the picture; the RGB histogram counted.
    false_colour: bool,
    rgb: bool,
}

impl Default for Develop {
    fn default() -> Self {
        Develop { look: Arc::new(Look::Original.compiled(0.0)), zebras: false, peaking: None, strip: None, false_colour: false, rgb: false }
    }
}

/// The strip's thumbnails: 4:3, small.
const THUMB: (usize, usize) = (40, 30);

/// A frame developed as asked, with its histogram, into the shared state;
/// the strip's thumbnails every third frame while it is open.
fn develop_frame(live: &Mutex<Live>, develop: &Mutex<Develop>, yuv: &[u8], w: usize, h: usize, pic: &mut Picture, meta: Option<Metered>) {
    let opts = develop.lock().map(|d| d.clone()).unwrap_or_default();
    let frames = live.lock().map(|l| l.frames).unwrap_or(0);
    let zebras = opts.zebras && !opts.false_colour;
    let clipped = opts.look.develop(yuv, w, h, pic, zebras.then_some(frames as usize % 8));
    // False colour: the brightness's bands over the whole picture.
    if opts.false_colour {
        camkit::paint_false_colour(&yuv[..w * h], &mut pic.rgb);
    }
    let mut histogram = pro::Histogram::of_luma(&yuv[..w * h], w, h, 2);
    histogram.clipped = if zebras { clipped as f32 / (w * h) as f32 } else { histogram.clipped };
    let rgb = opts.rgb.then(|| camkit::RgbHistogram::of_yuv420(yuv, w, h, 2));
    if let Some(colour) = opts.peaking {
        let marks = pro::peaks(&yuv[..w * h], w, h);
        for (px, on) in pic.rgb.chunks_exact_mut(3).zip(marks) {
            if on {
                px.copy_from_slice(&colour);
            }
        }
    }
    let thumbs = opts.strip.filter(|_| frames % 3 == 0).map(|looks| {
        let small = shrink_yuv420(yuv, w, h, THUMB.0, THUMB.1);
        looks
            .iter()
            .map(|look| {
                let mut t = Picture::default();
                look.develop(&small, THUMB.0, THUMB.1, &mut t, None);
                t
            })
            .collect::<Vec<_>>()
    });
    if let Ok(mut l) = live.lock() {
        l.preview = Some(pic.clone());
        l.frames += 1;
        l.trouble = None;
        l.histogram = histogram;
        l.rgb = rgb;
        l.yuv.clear();
        l.yuv.extend_from_slice(yuv);
        l.yuv_size = (w, h);
        if meta.is_some() {
            l.meta = meta;
        }
        if let Some(t) = thumbs {
            l.thumbs = t;
            l.thumbs_made += 1;
        }
    }
}

/// A YUV 4:2:0 frame shrunk to `tw` × `th` (each pixel the mean of its
/// part, cropped to the new shape from the middle), for thumbnails.
fn shrink_yuv420(yuv: &[u8], w: usize, h: usize, tw: usize, th: usize) -> Vec<u8> {
    let (cw, ch) = (w / 2, h / 2);
    let mut out = vec![0u8; tw * th * 3 / 2];
    let plane = |src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize, dst: &mut [u8]| {
        for oy in 0..dh {
            let (y0, y1) = (oy * sh / dh, ((oy + 1) * sh / dh).max(oy * sh / dh + 1));
            for ox in 0..dw {
                let (x0, x1) = (ox * sw / dw, ((ox + 1) * sw / dw).max(ox * sw / dw + 1));
                let (mut sum, mut n) = (0u32, 0u32);
                for y in y0..y1.min(sh) {
                    for x in x0..x1.min(sw) {
                        sum += u32::from(src[y * sw + x]);
                        n += 1;
                    }
                }
                dst[oy * dw + ox] = (sum / n.max(1)) as u8;
            }
        }
    };
    let (oy, ouv) = out.split_at_mut(tw * th);
    let (ou, ov) = ouv.split_at_mut(tw * th / 4);
    plane(&yuv[..w * h], w, h, tw, th, oy);
    plane(&yuv[w * h..w * h + cw * ch], cw, ch, tw / 2, th / 2, ou);
    plane(&yuv[w * h + cw * ch..w * h + 2 * cw * ch], cw, ch, tw / 2, th / 2, ov);
    out
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
    /// A photo, with what to take it with (`exp=…`, `look=…`: see
    /// catalyst-camera's notes).
    Still(PathBuf, Vec<String>),
    /// Detail mode: a burst merged in the background (`true`: only in dim light).
    Detail(PathBuf, bool, Vec<String>),
    Night(bool),
    /// Where a face is in the picture (0 to 1), for the exposure; or none.
    Face(Option<[f32; 4]>),
    /// The preview's exposure and white balance (pro mode's dials).
    Pro(pro::PreviewControls),
    /// The focus check's crop, centred here (0 to 1 of the picture), or none.
    Focus(Option<(f32, f32)>),
}

impl Order {
    /// The line the camera program reads.
    fn line(&self) -> String {
        let with = |words: &[String]| words.iter().map(|w| format!("\t{w}")).collect::<String>();
        match self {
            Order::Still(path, words) => format!("still {}{}\n", path.display(), with(words)),
            Order::Detail(path, auto, words) => format!("detail {} {}{}\n", if *auto { "auto" } else { "on" }, path.display(), with(words)),
            Order::Night(on) => format!("night {}\n", if *on { "on" } else { "off" }),
            Order::Face(Some(b)) => format!("face {:.3} {:.3} {:.3} {:.3}\n", b[0], b[1], b[2], b[3]),
            Order::Face(None) => "face none\n".to_owned(),
            Order::Pro(c) => format!("{}\n", c.order()),
            Order::Focus(Some((x, y))) => format!("focus on {x:.4} {y:.4}\n"),
            Order::Focus(None) => "focus off\n".to_owned(),
        }
    }
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
                let line = order.line();
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
fn run_program(live: &Mutex<Live>, stop: &AtomicBool, slot: &Mutex<Option<ChildStdin>>, luma: &(Mutex<Luma>, Condvar), develop: &Mutex<Develop>) {
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
                        // Into the shell's log as well: each photo's marks,
                        // any wait that ran out (catalyst-camera 1.5).
                        eprintln!("catalyst-camera: {}", line.trim());
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
    // What the camera said of the frame that follows.
    let mut said_of = None;
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
                develop_frame(live, develop, &frame, w, h, &mut pic, said_of.take());
                give_luma(luma, &frame[..w * h]);
            }
            2 => {
                let (path, took, detail) = saved_line(&String::from_utf8_lossy(&frame));
                if !took.is_empty() {
                    eprintln!("camera: {} × {} {took}", number >> 16, number & 0xffff);
                }
                let raw_skipped = took.contains("raw-skipped");
                live.lock().map(|mut l| l.events.push(Event::Saved { path, size: (number >> 16, number & 0xffff), detail, raw_skipped })).ok();
            }
            5 => said_of = Metered::parse(&String::from_utf8_lossy(&frame)),
            3 => {
                let why = String::from_utf8_lossy(&frame).trim().to_owned();
                live.lock().map(|mut l| l.events.push(Event::Failed(why))).ok();
            }
            4 => {
                let text = String::from_utf8_lossy(&frame).into_owned();
                let marks = text.split_once('\t').map_or(String::new(), |(_, m)| m.to_owned());
                live.lock().map(|mut l| l.events.push(Event::Captured(marks))).ok();
            }
            6 => {
                let line = String::from_utf8_lossy(&frame).trim().to_owned();
                live.lock().map(|mut l| l.events.push(Event::Exposing(line))).ok();
            }
            7 => {
                live.lock().map(|mut l| l.events.push(Event::Exposed)).ok();
            }
            _ => {}
        }
        // Drawn as it arrives, not on the frame beat (what the camera says of
        // a frame comes just before it).
        if kind != 5 {
            crate::platform::wake();
        }
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

/// The simulated room's light: a lit room in the evening.
const SIM_LUX: f32 = 320.0;
/// The simulated scene's light, as a white balance would find it.
const SIM_CT: u32 = 5200;

/// The whole exposure (µs at a gain of 1) the simulated scene needs, by the
/// IMX219 tuning's own reference (27.7 ms at gain 1 for 998 lux), with
/// compensation in stops.
fn sim_needs(ev: f32) -> f32 {
    27_685.0 * 998.0 / SIM_LUX * 2f32.powf(ev)
}

/// What the simulated camera does with the preview's controls: its own
/// exposure where they leave it free, and how much brighter than right the
/// result is (1 is the scene as drawn).
fn sim_meter(c: &pro::PreviewControls, focus: bool) -> (Metered, f32) {
    let needs = sim_needs(c.ev);
    let (exposure_us, gain) = match (c.exposure_us, c.gain) {
        (Some(e), Some(g)) => (e, g),
        (Some(e), None) => (e, (needs / e as f32).clamp(1.0, pro::MAX_GAIN)),
        (None, Some(g)) => (((needs / g).round() as u32).clamp(100, c.frame_max_us), g),
        // The camera's own exposure, as the tuning's normal profile has it.
        (None, None) => {
            let e = (needs / 4.0).clamp(100.0, 30_000.0f32.min(c.frame_max_us as f32));
            (e as u32, (needs / e).clamp(1.0, pro::MAX_GAIN))
        }
    };
    let ct = c.ct.unwrap_or(SIM_CT);
    let (r, b) = crate::dng::Tuning::stock().white(f64::from(ct));
    let m = Metered { exposure_us, analogue: gain, digital: 1.0, gains: [(1.0 / r) as f32, (1.0 / b) as f32], ct, lux: SIM_LUX, settled: true, fom: 0, zoom: if focus { 1.0 } else { 0.0 } };
    (m, exposure_us as f32 * gain / sim_needs(0.0))
}

/// The scene exposed `bright` times over and white-balanced for `ct` (the
/// light being [`SIM_CT`]), in place: in linear light, then back.
fn sim_expose(pic: &mut Picture, bright: f32, ct: u32) {
    let tuning = crate::dng::Tuning::stock();
    let (r0, b0) = tuning.white(f64::from(SIM_CT));
    let (r1, b1) = tuning.white(f64::from(ct));
    let k = [bright * (r1 / r0) as f32, bright, bright * (b1 / b0) as f32];
    if k.iter().all(|k| (k - 1.0).abs() < 1e-3) {
        return;
    }
    let mut table = [[0u8; 256]; 3];
    for (c, t) in table.iter_mut().enumerate() {
        for (v, o) in t.iter_mut().enumerate() {
            let lin = (v as f32 / 255.0).powf(2.2) * k[c];
            *o = (lin.min(1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
        }
    }
    for p in pic.rgb.chunks_exact_mut(3) {
        for (c, v) in p.iter_mut().enumerate() {
            *v = table[c][usize::from(*v)];
        }
    }
}

/// The focus check's part of the picture (0 to 1 each way): one photo
/// pixel to a preview pixel, centred at (`x`, `y`) as far as the edges allow.
pub fn focus_view(x: f32, y: f32) -> [f32; 4] {
    let (hw, hh) = (PREVIEW_W as f32 / FULL.0 as f32 * 0.5, PREVIEW_H as f32 / FULL.1 as f32 * 0.5);
    let (x, y) = (x.clamp(hw, 1.0 - hw), y.clamp(hh, 1.0 - hh));
    [x - hw, y - hh, x + hw, y + hh]
}

/// The simulated camera: its scene at the camera's pace, metered and
/// exposed as the dials say and developed as the screen asks; photos
/// written by the shell's own JPEG encoder behind the preview.
fn run_simulated(live: &Arc<Mutex<Live>>, stop: &AtomicBool, orders: &Receiver<Order>, scanning: &AtomicBool, luma: &(Mutex<Luma>, Condvar), develop: &Mutex<Develop>, paging: &AtomicBool) {
    let started = Instant::now();
    let (mut night, mut face) = (false, None);
    let mut controls = pro::preview(None, false);
    let mut focus: Option<(f32, f32)> = None;
    let mut out = Picture::default();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(33));
        let t = started.elapsed().as_secs_f32();
        let scan = scanning.load(Ordering::Relaxed);
        let view = focus.map_or([0.0, 0.0, 1.0, 1.0], |(x, y)| focus_view(x, y));
        let mut pic = scene_part(PREVIEW_W, PREVIEW_H, t, scan && focus.is_none(), view);
        // Doc mode: a page on the table (docscan's).
        if paging.load(Ordering::Relaxed) {
            crate::docscan::sim::paint(&mut pic, view, t);
        }
        let (mut meta, bright) = sim_meter(&controls, focus.is_some());
        sim_expose(&mut pic, bright, meta.ct);
        let yuv = rgb_to_yuv420(&pic);
        meta.fom = yuv[..PREVIEW_W * PREVIEW_H].windows(2).map(|p| u32::from(p[0].abs_diff(p[1]))).sum::<u32>() / 64;
        develop_frame(live, develop, &yuv, PREVIEW_W, PREVIEW_H, &mut out, Some(meta));
        give_luma(luma, &yuv[..PREVIEW_W * PREVIEW_H]);
        crate::platform::wake();
        while let Ok(order) = orders.try_recv() {
            match order {
                Order::Still(path, words) => sim_photo(live, path, t, scan, 0, None, &words),
                // "Dim light" in the simulator: night on.
                Order::Detail(path, auto, words) => sim_photo(live, path, t, scan, if auto && !night { 0 } else { SIM_BURST }, face, &words),
                Order::Night(on) => night = on,
                Order::Face(f) => face = f,
                Order::Pro(c) => controls = c,
                Order::Focus(f) => focus = f,
            }
        }
    }
    finish_luma(luma);
}

/// The simulator's burst: as many frames as the handheld takes with room to spare.
pub const SIM_BURST: usize = 6;
/// The sensor's full size, as every photo is taken.
pub const FULL: (usize, usize) = (3280, 2464);

/// A still order's word for `key` (each is `key=value`).
fn still_word<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.iter().find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// The simulator's photo, as catalyst-camera takes one: the picture in
/// ("captured") with the exposure and white balance it was given, then
/// written behind the preview with the look's table over it, the photo as
/// taken kept beside it and a DNG if asked; for detail mode a burst of a
/// dim, noisy scene (the robot still driving), merged on a thread of its own.
fn sim_photo(live: &Arc<Mutex<Live>>, path: PathBuf, t: f32, scan: bool, burst: usize, face: Option<[f32; 4]>, words: &[String]) {
    let asked = Instant::now();
    let (fw, fh) = FULL;
    let exposure = still_word(words, "exp").and_then(|v| v.parse::<f32>().ok());
    let gain = still_word(words, "gain").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let bright = exposure.map_or(1.0, |e| e * gain / sim_needs(0.0));
    let ct = still_word(words, "ct").and_then(|v| v.parse().ok()).unwrap_or(SIM_CT);
    // The aspect's crop, and a bracketing set's other frames ("also=µs,gain,thirds,path").
    let crop = Aspect::from_name(still_word(words, "aspect").unwrap_or("4:3")).crop(fw as u32, fh as u32);
    let (w, h) = (crop.2 as usize, crop.3 as usize);
    let also: Vec<(f32, PathBuf)> = words
        .iter()
        .filter_map(|w| w.strip_prefix("also="))
        .filter_map(|v| {
            let mut parts = v.splitn(4, ',');
            let (e, g) = (parts.next()?.parse::<f32>().ok()?, parts.next()?.parse::<f32>().ok()?);
            parts.next()?;
            Some((e * g / sim_needs(0.0), PathBuf::from(parts.next()?)))
        })
        .collect();
    // The sensor exposing it, as catalyst-camera says: a frame (or a burst,
    // or a set) at its exposure, two frame lengths each at the most.
    let frames_n = burst.max(1) + also.len();
    let frame_ms = exposure.map_or(66.7, |e| (e / 1000.0).max(66.7));
    let what = if !also.is_empty() { "bracket" } else if burst > 0 { "detail" } else { "still" };
    live.lock().map(|mut l| l.events.push(Event::Exposing(format!("{:.0} {frames_n} {what}", 2.0 * frame_ms * frames_n as f32)))).ok();
    crate::platform::wake();
    let mut noise = 0x2545_F491_4F6C_DD1Du64 ^ (t.to_bits() as u64);
    let mut frames = Vec::new();
    let mut raw_rgb = None;
    let cut = |yuv: Vec<u8>| if (w, h) == (fw, fh) { yuv } else { camkit::crop_yuv420(&yuv, fw, fh, crop) };
    for i in 0..burst.max(1) {
        // About 8 frames a second at full size, a hand's drift between them.
        let mut pic = scene(fw, fh, t + i as f32 * 0.12, scan);
        if still_word(words, "doc").is_some() {
            crate::docscan::sim::paint(&mut pic, [0.0, 0.0, 1.0, 1.0], t + i as f32 * 0.12);
        }
        sim_expose(&mut pic, bright, ct);
        let mut yuv = rgb_to_yuv420(&pic);
        if burst > 0 {
            dim_and_noisy(&mut yuv, fw * fh, &mut noise);
        }
        if i == 0 && still_word(words, "raw").is_some() {
            raw_rgb = Some(pic);
        }
        frames.push(cut(yuv));
    }
    let set: Vec<(PathBuf, Vec<u8>)> = also
        .into_iter()
        .enumerate()
        .map(|(i, (b, p))| {
            let mut pic = scene(fw, fh, t + (i + 1) as f32 * 0.12, scan);
            sim_expose(&mut pic, b, ct);
            (p, cut(rgb_to_yuv420(&pic)))
        })
        .collect();
    let shot = asked.elapsed().as_millis();
    let marks = format!("room 0 stop 0 config 0 start 0 frame {shot} copy {shot} captured {shot}");
    {
        let Ok(mut l) = live.lock() else { return };
        l.events.push(Event::Exposed);
        l.events.push(Event::Captured(marks));
    }
    crate::platform::wake();
    let (live, words) = (live.clone(), words.to_vec());
    std::thread::Builder::new()
        .name("photo-write".into())
        .spawn(move || {
            let event = sim_write(&path, frames, burst, face, &words, raw_rgb, shot, (w, h));
            live.lock().map(|mut l| l.events.push(event)).ok();
            crate::platform::wake();
            // The set's others, each a photo of its own, the look over each.
            let table = still_word(&words, "look").and_then(|f| std::fs::read(f).ok()).and_then(|b| crate::filters::read_lut_file(&b));
            let exif = still_word(&words, "lookname").map(crate::filters::exif_with_look);
            for (p, mut yuv) in set {
                if let Some(table) = &table {
                    crate::filters::apply_yuv420(table, &mut yuv, w, h);
                }
                let event = match crate::burst::write_jpeg(&p, &yuv, w, h, exif.clone()) {
                    Ok(()) => Event::Saved { path: p, size: (w as u32, h as u32), detail: false, raw_skipped: false },
                    Err(why) => Event::Failed(why),
                };
                live.lock().map(|mut l| l.events.push(event)).ok();
                crate::platform::wake();
            }
        })
        .ok();
}

/// The simulator's photo written: the look over it, the photo as taken
/// kept beside it, the DNG, and a detail merge started.
#[allow(clippy::too_many_arguments)]
fn sim_write(path: &std::path::Path, mut frames: Vec<Vec<u8>>, burst: usize, face: Option<[f32; 4]>, words: &[String], raw_rgb: Option<Picture>, shot: u128, (w, h): (usize, usize)) -> Event {
    let began = Instant::now();
    let table = still_word(words, "look").and_then(|f| std::fs::read(f).ok()).and_then(|b| crate::filters::read_lut_file(&b));
    let name = still_word(words, "lookname").map(str::to_owned);
    let exif = name.as_deref().map(crate::filters::exif_with_look);
    let original = still_word(words, "orig").map(PathBuf::from);
    let mut photo = frames[0].clone();
    if let Some(table) = &table {
        crate::filters::apply_yuv420(table, &mut photo, w, h);
    }
    if let Err(why) = crate::burst::write_jpeg(path, &photo, w, h, exif.clone()) {
        return Event::Failed(why);
    }
    if let (Some(original), true) = (&original, table.is_some()) {
        if let Some(dir) = original.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        crate::burst::write_jpeg(original, &frames[0], w, h, None).ok();
    }
    let mut raw_note = String::new();
    if let (Some(dng), Some(rgb)) = (still_word(words, "raw"), raw_rgb) {
        raw_note = match sim_dng(std::path::Path::new(dng), &rgb, words, name.clone()) {
            Ok(()) => " raw".into(),
            Err(why) => {
                eprintln!("camera: raw: {why}");
                " raw-skipped".into()
            }
        };
    }
    let detail = burst > 0;
    if detail {
        std::fs::write(crate::burst::marker(path), format!("began={}\nexpect={}\nprogress=0\n", crate::burst::now_ms(), crate::burst::expect_ms(burst, w, h))).ok();
        let (p, faces) = (path.to_path_buf(), face.into_iter().collect::<Vec<_>>());
        let original = original.filter(|_| table.is_some());
        frames.shrink_to_fit();
        std::thread::Builder::new()
            .name("merge".into())
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Idle);
                let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
                if let Err(why) = crate::burst::refine_with(&p, &refs, w, h, &faces, 3, table.as_deref(), original.as_deref()) {
                    eprintln!("camera: detail merge: {why}");
                }
                crate::platform::wake();
            })
            .ok();
    }
    let took = format!("shot {shot} write {}{}{raw_note}", began.elapsed().as_millis(), if detail { format!(" detail {burst}") } else { String::new() });
    eprintln!("camera: {w} × {h} {took} (simulated)");
    Event::Saved { path: path.to_path_buf(), size: (w as u32, h as u32), detail, raw_skipped: raw_note.contains("skipped") }
}

/// The simulator's RAW: the picture back to the sensor's own light (its
/// gamma undone, its white balance gains taken off) as a BGGR mosaic of
/// 10-bit values, written by the same DNG writer as the handheld's.
fn sim_dng(path: &std::path::Path, rgb: &Picture, words: &[String], look: Option<String>) -> Result<(), String> {
    let gains = still_word(words, "gains").map(|g| g.split(',').filter_map(|v| v.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|g| g.len() == 2).map_or([1.9, 1.5], |g| [g[0], g[1]]);
    let info = crate::dng::RawInfo {
        w: rgb.w,
        h: rgb.h,
        stride: rgb.w * 2,
        format: "SBGGR10".into(),
        black16: 4096,
        gains,
        exposure_us: still_word(words, "exp").and_then(|v| v.parse().ok()).unwrap_or(10_000),
        gain: still_word(words, "gain").and_then(|v| v.parse().ok()).unwrap_or(1.0),
        time: chrono::Local::now().timestamp(),
        turned: false,
        look,
    };
    let mut lin = [[0u16; 256]; 3];
    for (c, t) in lin.iter_mut().enumerate() {
        let g = [gains[0], 1.0, gains[1]][c];
        for (v, o) in t.iter_mut().enumerate() {
            *o = (64.0 + 959.0 * ((v as f32 / 255.0).powf(2.2) / g)).round() as u16;
        }
    }
    let mut px = vec![0u16; rgb.w * rgb.h];
    for y in 0..rgb.h {
        for x in 0..rgb.w {
            // BGGR: blue at the top left, red at the bottom right.
            let c = match (y % 2, x % 2) {
                (0, 0) => 2,
                (1, 1) => 0,
                _ => 1,
            };
            px[y * rgb.w + x] = lin[c][usize::from(rgb.rgb[(y * rgb.w + x) * 3 + c])];
        }
    }
    crate::dng::write(path, &info, &px, &crate::dng::Tuning::load())
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
#[cfg(test)]
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

/// The modes, in the carousel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Scan,
    Photo,
    Pro,
    /// The document scanner (docscan/, 1.0).
    Doc,
}

const MODES: [(Mode, &str); 4] = [(Mode::Scan, "scan"), (Mode::Photo, "photo"), (Mode::Pro, "pro"), (Mode::Doc, "doc")];

impl Mode {
    fn index(self) -> usize {
        MODES.iter().position(|(m, _)| *m == self).unwrap_or(1)
    }

    /// A photo is taken in it (doc mode takes pages, with none of its kit).
    fn shoots(self) -> bool {
        matches!(self, Mode::Photo | Mode::Pro)
    }
}

/// Pro mode's dials, in the bar's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dial {
    Shutter,
    Iso,
    Ev,
    Wb,
}

const DIALS: [Dial; 4] = [Dial::Shutter, Dial::Iso, Dial::Ev, Dial::Wb];

impl Dial {
    fn index(self) -> usize {
        DIALS.iter().position(|d| *d == self).unwrap_or(0)
    }
}

/// What the Camera keeps between visits (its own file beside the settings):
/// the dials, the look, RAW and the zebras, and the kit's settings and
/// presets (camkit.rs). It always opens in photo mode.
struct Prefs {
    pro: Pro,
    look: Look,
    hue: f32,
    raw: bool,
    zebras: bool,
    kit: Kit,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { pro: Pro::default(), look: Look::Original, hue: 32.0, raw: false, zebras: true, kit: Kit::default() }
    }
}

impl Prefs {
    fn file() -> PathBuf {
        crate::config_dir().join("camera")
    }

    fn load() -> Prefs {
        let mut p = Prefs::default();
        for line in std::fs::read_to_string(Self::file()).unwrap_or_default().lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "pro" => p.pro = Pro::from_line(&v.replace(',', " ")),
                "look" => p.look = Look::from_id(v).unwrap_or_default(),
                "hue" => p.hue = v.parse().unwrap_or(p.hue),
                "raw" => p.raw = v == "1",
                "zebras" => p.zebras = v != "0",
                _ => p.kit.read(k, v),
            }
        }
        p
    }

    fn save(&self) {
        let text = format!(
            "pro={}\nlook={}\nhue={:.0}\nraw={}\nzebras={}\n{}",
            self.pro.to_line().replace(' ', ","),
            self.look.id(),
            self.hue,
            u8::from(self.raw),
            u8::from(self.zebras),
            self.kit.lines()
        );
        crate::sys::write_atomic(&Self::file(), text.as_bytes()).ok();
    }
}

pub struct Camera {
    live: Arc<Mutex<Live>>,
    stop: Arc<AtomicBool>,
    control: Control,
    scanning: Arc<AtomicBool>,
    mode: Mode,
    /// Where the modes' carousel is: 0 at scan, 1 at photo, 2 at pro.
    mode_at: Animated,
    /// Pro mode's dials, the one being turned (its mark sliding between).
    pro: Pro,
    dial: Option<Dial>,
    dial_at: Animated,
    dial_fade: Animated,
    /// The look, Tint's colour, the looks' strip (open, where it has slid
    /// to, how far in), and the thumbnails it shows.
    look: Look,
    hue: f32,
    strip: bool,
    strip_at: Animated,
    strip_fade: Animated,
    thumbs: Vec<Picture>,
    thumbs_seen: u64,
    raw: bool,
    zebras: bool,
    /// The focus check: where its crop is centred (0 to 1), and its fade.
    focus: Option<(f32, f32)>,
    focus_fade: Animated,
    /// What the camera said of the newest frame, and its histogram.
    metered: Option<Metered>,
    histogram: pro::Histogram,
    /// How the camera's thread develops frames, and the preview's controls
    /// as last told.
    develop: Arc<Mutex<Develop>>,
    told: Option<pro::PreviewControls>,
    /// Photos asked for and not yet saved, and their names.
    pending: usize,
    pending_paths: Vec<PathBuf>,
    /// The photo whose thumbnail is the shutter's instant one.
    instant: Option<PathBuf>,
    /// The shot, the moment it was taken, flying into the thumbnail.
    flying: Option<Picture>,
    fly: Animated,
    /// When the slow shutter was last said, so it is said once.
    shaky_said: bool,
    /// Focus peaking's colour: the signal colour, as last drawn.
    peak: [u8; 3],
    preview: Option<Picture>,
    seen: u64,
    trouble: Option<String>,
    timer: u32,
    /// When the timer's photo is taken (kept by the clock, not the frames).
    countdown: Option<Instant>,
    night: bool,
    /// The kit (camkit.rs): grids, level, aspect, bracketing, the readout,
    /// presets, the intervalometer, and the photo under way, timed.
    kit: Kit,
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
    /// Doc mode: the page found, held still and taken (docscan/), and the
    /// simulated camera told to lay a page on its table.
    doc: Option<crate::docscan::Finding>,
    paging: Arc<AtomicBool>,
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
        let paging = Arc::new(AtomicBool::new(false));
        let luma: Arc<(Mutex<Luma>, Condvar)> = Arc::default();
        let prefs = Prefs::load();
        let develop = Arc::new(Mutex::new(Develop { look: Arc::new(prefs.look.compiled(prefs.hue)), ..Develop::default() }));
        let simulated = !cfg!(target_arch = "aarch64");
        let control = if simulated {
            let (tx, rx) = mpsc::channel();
            let (l, s, sc, lu, d, pg) = (live.clone(), stop.clone(), scanning.clone(), luma.clone(), develop.clone(), paging.clone());
            std::thread::Builder::new().name("camera".into()).spawn(move || run_simulated(&l, &s, &rx, &sc, &lu, &d, &pg)).ok();
            Control::Simulated(tx)
        } else {
            let slot: Arc<Mutex<Option<ChildStdin>>> = Arc::default();
            let (l, s, sl, lu, d) = (live.clone(), stop.clone(), slot.clone(), luma.clone(), develop.clone());
            std::thread::Builder::new().name("camera".into()).spawn(move || run_program(&l, &s, &sl, &lu, &d)).ok();
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
            mode_at: Animated::new(1.0, role::detent()).with_rest(0.002),
            pro: prefs.pro,
            dial: None,
            // The dial's mark on Bezel's mode buttons' spring, its fade on
            // the effects spring (as the lens switch).
            dial_at: Animated::new(0.0, role::release()).with_rest(0.002),
            dial_fade: Animated::new(0.0, role::effect()).with_rest(0.002),
            look: prefs.look,
            hue: prefs.hue,
            strip: false,
            strip_at: Animated::new(LOOKS.iter().position(|l| *l == prefs.look).unwrap_or(0) as f32, role::release()).with_rest(0.002),
            strip_fade: Animated::new(0.0, role::effect()).with_rest(0.002),
            thumbs: Vec::new(),
            thumbs_seen: 0,
            raw: prefs.raw,
            zebras: prefs.zebras,
            focus: None,
            focus_fade: Animated::new(0.0, role::effect()).with_rest(0.002),
            metered: None,
            histogram: pro::Histogram::default(),
            develop,
            told: None,
            pending: 0,
            pending_paths: Vec::new(),
            instant: None,
            flying: None,
            // The shot into the corner on a smooth, unbouncing spring.
            fly: Animated::new(1.0, role::smooth()).with_rest(0.002),
            shaky_said: false,
            peak: [255, 140, 40],
            preview: None,
            seen: 0,
            trouble: None,
            timer: 0,
            countdown: None,
            night: false,
            kit: prefs.kit,
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
            doc: None,
            paging,
        };
        camera.hold_smart();
        // The level, if it was on when the Camera was last left.
        if camera.kit.level {
            camera.kit.imu = Some(crate::imu::hold(crate::imu::Use::Motion));
        }
        camera.tell_develop();
        camera.tell_pro();
        // The simulator's album has a few photos to begin with.
        #[cfg(not(target_arch = "aarch64"))]
        photos::seed_demo();
        camera.load_last(None);
        camera
    }

    fn prefs(&self) -> Prefs {
        let kit = Kit {
            grid: self.kit.grid,
            aspect: self.kit.aspect,
            level: self.kit.level,
            bracket: self.kit.bracket,
            false_colour: self.kit.false_colour,
            hist: self.kit.hist,
            readout: self.kit.readout,
            presets: self.kit.presets.clone(),
            interval: self.kit.interval,
            ..Kit::default()
        };
        Prefs { pro: self.pro, look: self.look, hue: self.hue, raw: self.raw, zebras: self.zebras, kit }
    }

    /// How the camera's thread is to develop frames now: the look, the
    /// zebras (pro mode), peaking (the focus check), the strip's thumbnails.
    fn tell_develop(&mut self) {
        let strip = self.strip.then(|| Arc::new(LOOKS.iter().map(|l| l.compiled(self.hue)).collect::<Vec<_>>()));
        let look = Arc::new(self.look.compiled(self.hue));
        let pro = self.mode == Mode::Pro;
        let zebras = pro && self.zebras;
        let peaking = self.focus.map(|_| self.peak);
        let false_colour = pro && self.kit.false_colour && self.focus.is_none();
        let rgb = pro && self.kit.hist == HistMode::Rgb;
        if let Ok(mut d) = self.develop.lock() {
            *d = Develop { look, zebras, peaking, strip, false_colour, rgb };
        }
    }

    /// The preview's exposure and white balance for the mode and dials,
    /// told to the camera when they change.
    fn tell_pro(&mut self) {
        let controls = pro::preview((self.mode == Mode::Pro).then_some(&self.pro), self.night);
        if self.told != Some(controls) && self.control.send(Order::Pro(controls)) {
            self.told = Some(controls);
        }
    }

    fn keep_prefs(&self) {
        self.prefs().save();
    }

    /// Straight into scan mode: for reading Scout's codes.
    pub fn scanning() -> Self {
        let mut camera = Camera::new();
        camera.set_mode(Mode::Scan);
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
        let was = self.mode;
        self.mode = mode;
        self.mode_at.set(mode.index() as f32);
        self.scanning.store(mode == Mode::Scan, Ordering::Relaxed);
        self.paging.store(mode == Mode::Doc, Ordering::Relaxed);
        self.countdown = None;
        self.set_dial(None);
        if matches!(mode, Mode::Scan | Mode::Doc) {
            self.set_strip(false);
            self.set_focus(None);
        }
        // Doc mode looks for the page (its finder's thread waits while not
        // given frames); its pages wait in the work folder meanwhile.
        if mode == Mode::Doc {
            let finding = self.doc.get_or_insert_with(crate::docscan::Finding::new);
            let waiting = finding.doc.borrow().pages.len();
            self.say(if waiting > 0 { format!("{waiting} page{} waiting · ↓ to see", if waiting == 1 { "" } else { "s" }) } else { "point it at a page".to_owned() });
        }
        if (was == Mode::Pro) != (mode == Mode::Pro) {
            self.tell_develop();
            self.tell_pro();
        }
        if mode == Mode::Pro {
            self.say("keys s i e w: the dials");
        }
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

    /// What a photo taken now is asked for with: the exposure the preview
    /// settled on (or the dials), split for a steady hand, its white
    /// balance, the look's table and where the photo as taken goes, the RAW;
    /// and whether detail mode merges a burst for it.
    fn still_words(&self, path: &std::path::Path, detail: crate::burst::DetailMode) -> (Vec<String>, Option<bool>, Vec<PathBuf>) {
        let in_pro = self.mode == Mode::Pro;
        let pro = in_pro.then_some(&self.pro);
        let mut words = Vec::new();
        let mut set = Vec::new();
        let still = self.metered.map(|m| pro::still(pro, &m, self.night));
        if let Some(s) = &still {
            words = s.words();
            if in_pro {
                let program = Program::of(&self.pro);
                words.push(format!("program={}", program.exif()));
                if self.pro.ev != 0 && !self.pro.manual() {
                    words.push(format!("bias={}", self.pro.ev));
                }
                // Bracketing: the set's other frames, each its exposure and file.
                if let Some(b) = self.kit.bracket {
                    for (ev, us, gain) in b.exposures(s, program).into_iter().skip(1) {
                        let p = camkit::set_path(path, ev);
                        let bias = ev * 3 + if self.pro.manual() { 0 } else { i32::from(self.pro.ev) };
                        words.push(format!("also={us},{gain:.3},{bias},{}", p.display()));
                        set.push(p);
                    }
                }
            }
        }
        if let Some(word) = self.kit.aspect.word() {
            words.push(word);
        }
        // Some(auto) for a burst: decided here from the metering when there
        // is any, else left to the camera program ("auto").
        let raw = in_pro && self.raw;
        let mut burst = camkit::detail_burst(detail, self.metered.as_ref(), still.as_ref(), raw, !set.is_empty());
        if self.look != Look::Original {
            if let Some(table) = crate::filters::ensure_lut(self.look, self.hue) {
                words.push(format!("look={}", table.display()));
                let name = if self.look == Look::Tint { format!("Tint (hue {:.0})", self.hue.rem_euclid(360.0)) } else { self.look.name().to_owned() };
                words.push(format!("lookname={name}"));
                words.push(format!("orig={}", crate::filters::original_of(path).display()));
            }
        }
        if raw && set.is_empty() {
            words.push(format!("raw={}", path.with_extension("dng").display()));
            // A RAW needs the camera's memory a burst would.
            burst = None;
        }
        (words, burst, set)
    }

    /// A name for the next photo, never one already taken and not yet written.
    fn fresh_path(&self) -> PathBuf {
        let mut path = photos::new_path(chrono::Local::now().naive_local());
        let mut n = 2;
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        while self.pending_paths.contains(&path) {
            path = path.with_file_name(format!("{stem}_{n}.jpg"));
            n += 1;
        }
        path
    }

    fn shoot(&mut self, calm: bool, detail: crate::burst::DetailMode) {
        self.shoot_at(calm, detail, Instant::now());
    }

    /// A photo asked for, timed from `key` (the shutter's key going down).
    fn shoot_at(&mut self, calm: bool, detail: crate::burst::DetailMode, key: Instant) {
        // One photo's picture at a time: the next as soon as the camera has
        // the last one's, while that is still being written.
        if self.saving.is_some() && !self.captured {
            return;
        }
        if self.preview.is_none() {
            self.say("the camera is not ready yet");
            return;
        }
        let path = self.fresh_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let (words, burst, set) = self.still_words(&path, detail);
        let order = match burst {
            None => Order::Still(path.clone(), words),
            Some(auto) => Order::Detail(path.clone(), auto, words),
        };
        // Timed from the key (or the timer, or the intervalometer) on.
        let mut timeline = Timeline::new(path.clone(), key);
        if !self.control.send(order) {
            self.say("the camera is not ready yet");
            return;
        }
        timeline.mark(Mark::Request, Instant::now());
        self.kit.shot = Some(timeline);
        self.instant_shot(&path, calm);
        self.saving = Some(Instant::now());
        self.captured = false;
        self.pending += 1 + set.len();
        self.pending_paths.push(path);
        self.pending_paths.extend(set);
        self.press.jump(1.0);
        self.press.set(0.0);
        if !calm {
            self.flash.jump(1.0);
            self.flash.set(0.0);
        }
    }

    /// Doc mode's photo: the camera's own still at the sensor's full size,
    /// with the exposure the preview settled on and none of the photo's
    /// kit (no look, aspect, burst or RAW), into the scans' work folder;
    /// the page found in it goes with it.
    fn shoot_doc(&mut self) {
        let busy = self.saving.is_some() && !self.captured;
        let Some(finding) = &mut self.doc else { return };
        if finding.shot.is_some() || busy {
            return;
        }
        if self.preview.is_none() {
            self.say("the camera is not ready yet");
            return;
        }
        let path = crate::docscan::Finding::shot_path();
        let corners = finding.corners();
        let mut words = self.metered.map(|m| pro::still(None, &m, self.night).words()).unwrap_or_default();
        words.push("doc=1".into());
        if !self.control.send(Order::Still(path.clone(), words)) {
            self.say("the camera is not ready yet");
            return;
        }
        finding.shot = Some((path.clone(), corners));
        self.saving = Some(Instant::now());
        self.captured = false;
        self.pending += 1;
        self.pending_paths.push(path);
        self.press.jump(1.0);
        self.press.set(0.0);
        self.flash.jump(1.0);
        self.flash.set(0.0);
    }

    /// The shutter's answer, at once: the frame in sight as it was taken,
    /// with the look over it (and nothing else: no zebras, no peaking), as
    /// the photo's thumbnail, flying into the corner, while the full-size
    /// photo is taken and saved. The focus check's frame is a crop, not the
    /// photo: there the thumbnail waits for the photo.
    fn instant_shot(&mut self, path: &std::path::Path, calm: bool) {
        if self.focus.is_some() {
            return;
        }
        let (yuv, (w, h)) = match self.live.lock() {
            Ok(l) => (l.yuv.clone(), l.yuv_size),
            Err(_) => return,
        };
        if w == 0 || yuv.len() < w * h * 3 / 2 {
            return;
        }
        let mut pic = Picture::default();
        self.look.compiled(self.hue).develop(&yuv, w, h, &mut pic, None);
        let thumb = pic.cover(photos::THUMB_W, photos::THUMB_H);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.last = Some((Photo { path: path.to_path_buf(), name, taken: chrono::Local::now().naive_local(), bytes: 0 }, thumb));
        self.instant = Some(path.to_path_buf());
        if calm {
            self.flying = None;
            self.fly.jump(1.0);
        } else {
            self.flying = Some(pic);
            self.fly.jump(0.0);
            self.fly.set(1.0);
        }
    }

    /// A dial chosen to turn (its mark slides to it), or none.
    fn set_dial(&mut self, dial: Option<Dial>) {
        if let Some(d) = dial {
            // From none, the mark appears where it is, not sliding in.
            if self.dial.is_none() {
                self.dial_at.jump(d.index() as f32);
            }
            self.dial_at.set(d.index() as f32);
        }
        self.dial_fade.set(if dial.is_some() { 1.0 } else { 0.0 });
        self.dial = dial;
    }

    /// A dial's key: it is chosen; pressed again, it goes back to auto (the
    /// white balance's steps through its presets instead).
    fn dial_key(&mut self, dial: Dial) {
        if dial == Dial::Ev && self.pro.manual() {
            self.say("ev shows the meter here");
            return;
        }
        if self.dial != Some(dial) {
            self.set_dial(Some(dial));
            if dial == Dial::Wb {
                self.say("y: from a grey card");
            }
            return;
        }
        match dial {
            Dial::Wb => {
                self.pro.wb = self.pro.wb.next_preset();
                let words = self.pro.wb.preset().map_or_else(|| "wb: auto".to_owned(), |p| format!("wb: {p} · {}", self.pro.wb.words()));
                self.say(words);
            }
            _ => self.dial_auto(),
        }
        self.pro_changed();
    }

    /// The dial chosen back to auto.
    fn dial_auto(&mut self) {
        match self.dial {
            Some(Dial::Shutter) => self.pro.shutter = None,
            Some(Dial::Iso) => self.pro.iso = None,
            Some(Dial::Ev) => self.pro.ev = 0,
            Some(Dial::Wb) => self.pro.wb = pro::Wb::Auto,
            None => return,
        }
        self.pro_changed();
    }

    /// The chosen dial turned a step: → for more light (a longer shutter, a
    /// higher ISO, more compensation) or a warmer white balance. From auto,
    /// it starts at what the camera had chosen.
    fn turn_dial(&mut self, by: i32) {
        let m = self.metered.unwrap_or_default();
        let step = |at: usize, len: usize| (at as i32 + by).clamp(0, len as i32 - 1) as usize;
        match self.dial {
            Some(Dial::Shutter) => self.pro.shutter = Some(step(self.pro.shutter.unwrap_or_else(|| pro::nearest_shutter(m.exposure_us)), pro::SHUTTERS.len())),
            Some(Dial::Iso) => self.pro.iso = Some(step(self.pro.iso.unwrap_or_else(|| pro::nearest_iso(m.analogue * m.digital)), pro::ISOS.len())),
            Some(Dial::Ev) => self.pro.ev = (i32::from(self.pro.ev) + by).clamp(-i32::from(pro::EV_THIRDS), i32::from(pro::EV_THIRDS)) as i8,
            Some(Dial::Wb) => {
                let from = match self.pro.wb {
                    pro::Wb::Kelvin(k) => k,
                    pro::Wb::Auto | pro::Wb::Grey(..) => (m.ct + pro::KELVIN_STEP / 2) / pro::KELVIN_STEP * pro::KELVIN_STEP,
                };
                let k = (from as i32 + by * pro::KELVIN_STEP as i32).clamp(pro::KELVIN_MIN as i32, pro::KELVIN_MAX as i32) as u32;
                self.pro.wb = pro::Wb::Kelvin(k);
            }
            None => return,
        }
        self.pro_changed();
    }

    fn pro_changed(&mut self) {
        self.tell_pro();
        self.keep_prefs();
    }

    /// The looks' strip shown or put away.
    fn set_strip(&mut self, open: bool) {
        if self.strip == open {
            return;
        }
        self.strip = open;
        self.strip_fade.set(if open { 1.0 } else { 0.0 });
        if open {
            self.set_dial(None);
            self.say(if self.look == Look::Tint { "← →: looks · ↑ ↓: colour" } else { "← → looks · f: done" });
        }
        self.tell_develop();
    }

    /// The look `by` along the strip.
    fn choose_look(&mut self, by: i32) {
        let at = LOOKS.iter().position(|l| *l == self.look).unwrap_or(0) as i32;
        let to = (at + by).clamp(0, LOOKS.len() as i32 - 1) as usize;
        if to as i32 != at + by {
            // At an end: the strip leans a little and comes back.
            self.strip_at.nudge(by as f32 * 3.0);
        }
        self.look = LOOKS[to];
        self.strip_at.set(to as f32);
        self.tell_develop();
        self.keep_prefs();
    }

    /// Tint's colour turned by `by` degrees.
    fn turn_hue(&mut self, by: f32) {
        self.hue = (self.hue + by).rem_euclid(360.0);
        self.tell_develop();
        self.keep_prefs();
    }

    /// The focus check on (its crop centred here) or off.
    fn set_focus(&mut self, at: Option<(f32, f32)>) {
        let was = self.focus.is_some();
        self.focus = at;
        self.focus_fade.set(if at.is_some() { 1.0 } else { 0.0 });
        self.control.send(Order::Focus(at));
        if at.is_some() != was {
            self.tell_develop();
            if at.is_some() {
                self.set_strip(false);
                self.say("arrows: look around");
            }
        }
    }

    /// The focus check's crop moved a step (half of what is in sight).
    fn pan_focus(&mut self, dx: f32, dy: f32) {
        if let Some((x, y)) = self.focus {
            let v = focus_view(x, y);
            let (hw, hh) = ((v[2] - v[0]) * 0.5, (v[3] - v[1]) * 0.5);
            let to = ((x + dx * hw).clamp(hw, 1.0 - hw), (y + dy * hh).clamp(hh, 1.0 - hh));
            self.set_focus(Some(to));
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
            Code::Text(text) if crate::scout::share::is_code(&text) => {
                self.say("read in already · esc: back");
                Nav::Stay
            }
            // A team's code (Team): the page to join it.
            Code::Text(text) if crate::team::is_team_code(&text) => Nav::Push(crate::team::scanned(&text)),
            Code::Text(text) if crate::strategy::is_board_code(&text) => match crate::strategy::scanned(&text) {
                Ok(board) => Nav::Push(board),
                Err(words) => {
                    self.say(words);
                    Nav::Stay
                }
            },
            // A battery's QR label (Batteries): its page.
            Code::Text(text) => crate::batteries::scanned(&text).map_or_else(|| Nav::Push(Box::new(TextPage::new(text))), Nav::Push),
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

    /// Nothing here keeps the frame beat but springs moving: frames are
    /// drawn as they arrive (the camera wakes the loop) and the waits look
    /// in through [`Screen::ambient`] (0.9.4).
    fn busy(&self) -> bool {
        false
    }

    /// How soon what time alone changes needs looking at again: a note
    /// fading, the timer's second landing, "hold still"'s tenths, the saving
    /// dots, the intervalometer's next, the level. None: nothing does.
    fn ambient(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut soonest: Option<Duration> = None;
        let mut want = |d: Duration| soonest = Some(soonest.map_or(d, |s| s.min(d)));
        let frame = Duration::from_millis(16);
        if let Some((_, age)) = &self.note {
            want(if *age < 0.2 || *age > NOTE_FOR { frame } else { Duration::from_secs_f32((NOTE_FOR - age).max(0.016)) });
        }
        if let Some(at) = self.countdown {
            // Each second's number lands large and eases back over 0.35 s.
            let left = at.saturating_duration_since(now).as_secs_f32();
            let into = 1.0 - (left - left.floor());
            want(if into < 0.4 { frame } else { Duration::from_secs_f32((1.0 - into).max(0.016)) });
        }
        if self.kit.shot.as_ref().is_some_and(|t| t.hold_still(now).is_some()) {
            want(Duration::from_millis(100));
        }
        if self.saving.is_some() {
            want(Duration::from_millis(333));
        }
        if let Some(run) = &self.kit.run {
            want(run.next.saturating_duration_since(now).min(Duration::from_secs(1)).max(Duration::from_millis(50)));
        }
        if self.kit.level && self.kit.imu.is_some() {
            // My own tuning: 15 a second follows a hand turning it.
            want(Duration::from_millis(66));
        }
        if self.refining.is_some() {
            want(Duration::from_millis(500));
        }
        // Doc mode: the hold filling while the page keeps still.
        if self.mode == Mode::Doc && self.doc.as_ref().is_some_and(|f| f.hold().is_some()) {
            want(Duration::from_millis(33));
        }
        soonest
    }

    /// The intervalometer running: the screen dims (and never goes off),
    /// the handheld stays awake.
    fn holds_dim(&self) -> bool {
        self.kit.run.is_some()
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
        let main = self.lens == Lens::Main && self.mode.shoots();
        // The intervalometer running: Enter or Esc stops it, and nothing
        // else is taken (a stray key must not change what it is taking).
        if self.kit.run.is_some() {
            if matches!(key, Key::Enter | Key::Char(' ') | Key::Back | Key::Leave | Key::Char('v')) {
                self.stop_run("stopped");
            }
            return Nav::Stay;
        }
        // Its panel: ← → the value, ↑ ↓ the row, Enter starts, Esc or v closes.
        if let Some(row) = self.kit.panel {
            match key {
                Key::Left | Key::Right => {
                    let by = if key == Key::Right { 1 } else { -1 };
                    if row == 0 { self.kit.interval.step_every(by) } else { self.kit.interval.step_count(by) }
                    self.keep_prefs();
                }
                Key::Up | Key::Down => self.kit.panel = Some(1 - row),
                Key::Enter | Key::Char(' ') => self.start_run(),
                Key::Back | Key::Leave | Key::Char('v') => self.kit.panel = None,
                _ => {}
            }
            return Nav::Stay;
        }
        let pro_mode = self.mode == Mode::Pro && main;
        match key {
            Key::Back | Key::Leave if self.countdown.is_some() => {
                self.countdown = None;
                self.say("timer stopped");
            }
            // Esc puts away what is out first: a dial, the looks, the focus check.
            Key::Back if self.dial.is_some() => self.set_dial(None),
            Key::Back if self.strip => self.set_strip(false),
            Key::Back if self.focus.is_some() => self.set_focus(None),
            // The card put away; the code not offered again while it stays
            // in sight.
            Key::Back if self.offered().is_some() => {
                self.dismissed = self.found.as_ref().map(|f| f.code.clone());
                self.found_rise.set(0.0);
            }
            Key::Back | Key::Leave => return Nav::Pop,
            // The focus check: the arrows look around.
            Key::Left if self.focus.is_some() => self.pan_focus(-1.0, 0.0),
            Key::Right if self.focus.is_some() => self.pan_focus(1.0, 0.0),
            Key::Up if self.focus.is_some() => self.pan_focus(0.0, -1.0),
            Key::Down if self.focus.is_some() => self.pan_focus(0.0, 1.0),
            // The looks: ← → along the strip, ↑ ↓ Tint's colour.
            Key::Left if self.strip => self.choose_look(-1),
            Key::Right if self.strip => self.choose_look(1),
            Key::Up if self.strip && self.look == Look::Tint => self.turn_hue(15.0),
            Key::Down if self.strip && self.look == Look::Tint => self.turn_hue(-15.0),
            Key::Up | Key::Down if self.strip => {}
            // A dial: ← → turn it, ↑ ↓ to the next.
            Key::Left if self.dial.is_some() => self.turn_dial(-1),
            Key::Right if self.dial.is_some() => self.turn_dial(1),
            Key::Up | Key::Down if self.dial.is_some() => {
                let at = self.dial.map_or(0, Dial::index) as i32 + if key == Key::Down { 1 } else { -1 };
                self.set_dial(Some(DIALS[at.rem_euclid(DIALS.len() as i32) as usize]));
            }
            Key::Char('a') if self.dial.is_some() => self.dial_auto(),
            Key::Left => self.set_mode(MODES[self.mode.index().saturating_sub(1)].0),
            Key::Right => self.set_mode(MODES[(self.mode.index() + 1).min(MODES.len() - 1)].0),
            Key::Tab => self.set_mode(MODES[(self.mode.index() + 1) % MODES.len()].0),
            Key::Enter | Key::Char(' ') | Key::Ctrl('p' | 'P') if self.quick || matches!(key, Key::Enter | Key::Char(' ')) => match self.mode {
                Mode::Scan => return self.act(),
                Mode::Doc => self.shoot_doc(),
                _ if self.countdown.is_some() => {}
                _ if self.timer > 0 => self.countdown = Some(Instant::now() + Duration::from_secs(self.timer.into())),
                _ => self.shoot(cx.settings.calm, cx.settings.detail),
            },
            // Pro mode's dials, each on its key.
            Key::Char('s') if self.mode == Mode::Pro => self.dial_key(Dial::Shutter),
            Key::Char('i') if self.mode == Mode::Pro => self.dial_key(Dial::Iso),
            Key::Char('e') if self.mode == Mode::Pro => self.dial_key(Dial::Ev),
            Key::Char('w') if self.mode == Mode::Pro => self.dial_key(Dial::Wb),
            Key::Char('r') if self.mode == Mode::Pro => {
                self.raw = !self.raw;
                self.keep_prefs();
                self.say(if self.raw { "raw on: a .dng too" } else { "RAW off" });
            }
            // Exposure's aids, in turn: zebras, false colour, neither.
            Key::Char('x') if self.mode == Mode::Pro => {
                (self.zebras, self.kit.false_colour) = match (self.zebras, self.kit.false_colour) {
                    (true, _) => (false, true),
                    (false, true) => (false, false),
                    (false, false) => (true, false),
                };
                self.tell_develop();
                self.keep_prefs();
                self.say(match (self.zebras, self.kit.false_colour) {
                    (true, _) => "zebras: highlights at 98%",
                    (_, true) => "false colour",
                    _ => "exposure aids off",
                });
            }
            Key::Char('f') if main && !self.quick => self.set_strip(!self.strip),
            Key::Char('z') if main && !self.quick => self.set_focus(if self.focus.is_some() { None } else { Some((0.5, 0.5)) }),
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
            // Doc mode: the pages taken so far.
            Key::Down if self.mode == Mode::Doc && self.doc.as_ref().is_some_and(|f| !f.doc.borrow().pages.is_empty()) => {
                if let Some(f) = &self.doc {
                    return Nav::Push(Box::new(crate::docscan::Review::new(f.doc.clone())));
                }
            }
            Key::Down => {
                return Nav::Push(Box::new(match &self.last {
                    Some((photo, _)) => photos::Album::at(&photo.path),
                    None => photos::Album::new(),
                }));
            }
            // The readout of the last photo's timing (pro mode's `t`).
            Key::Char('t') if pro_mode => {
                self.kit.readout = !self.kit.readout;
                self.keep_prefs();
                self.say(if self.kit.readout { "timing readout on" } else { "timing readout off" });
            }
            Key::Char('c' | 't') => {
                self.timer = camkit::next_timer(self.timer);
                self.say(if self.timer == 0 { "self-timer off".to_owned() } else { format!("self-timer {} s", self.timer) });
            }
            Key::Char('g') if main => {
                self.kit.grid = self.kit.grid.next();
                self.keep_prefs();
                self.say(match self.kit.grid {
                    Grid::Off => "grid off",
                    Grid::Thirds => "grid: thirds",
                    Grid::Square => "grid: squares",
                });
            }
            Key::Char('h') if main => self.set_level(!self.kit.level),
            Key::Char('o') if main && !self.quick => {
                self.kit.aspect = self.kit.aspect.next();
                self.kit.overlay = camkit::Overlay::default();
                self.keep_prefs();
                let (_, _, cw, ch) = self.kit.aspect.crop(FULL.0 as u32, FULL.1 as u32);
                self.say(format!("{} · {cw} × {ch}", self.kit.aspect.name()));
            }
            // Pro mode's kit (camkit.rs).
            Key::Char('m') if pro_mode => {
                let program = Program::of(&self.pro).next();
                let still = self.still_now();
                program.apply(&mut self.pro, still.as_ref(), self.metered.as_ref());
                self.pro_changed();
                self.say(program.words());
            }
            Key::Char('b') if pro_mode => {
                self.kit.bracket = camkit::next_bracket(self.kit.bracket);
                self.keep_prefs();
                self.say(match self.kit.bracket {
                    Some(b) => format!("bracketing {} × ±{} ev", b.frames, b.stops),
                    None => "bracketing off".to_owned(),
                });
            }
            Key::Char('y') if pro_mode => self.grey_card(),
            Key::Char('u') if pro_mode => {
                self.kit.hist = self.kit.hist.next();
                self.tell_develop();
                self.keep_prefs();
                self.say(match self.kit.hist {
                    HistMode::Luma => "histogram: brightness",
                    HistMode::Rgb => "histogram: rgb",
                    HistMode::Off => "histogram off",
                });
            }
            Key::Char('v') if pro_mode && !self.quick => {
                self.set_dial(None);
                self.set_strip(false);
                self.kit.panel = Some(0);
                self.say("enter: start");
            }
            Key::Char(c) if pro_mode && camkit::preset_key(c).is_some() => {
                let Some((i, keep)) = camkit::preset_key(c) else { return Nav::Stay };
                if keep {
                    let setup = self.setup();
                    eprintln!("camera: c{} kept: {}", i + 1, setup.summary());
                    self.kit.presets[i] = Some(setup);
                    self.keep_prefs();
                    self.say(format!("kept as c{}", i + 1));
                } else if let Some(setup) = self.kit.presets[i].clone() {
                    self.recall(&setup);
                    eprintln!("camera: c{} recalled: {}", i + 1, setup.summary());
                    self.say(format!("c{} recalled", i + 1));
                } else {
                    self.say(format!("c{} is empty", i + 1));
                }
            }
            Key::Char('n') => {
                self.night = !self.night;
                self.control.send(Order::Night(self.night));
                self.tell_pro();
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
        let now = Instant::now();
        // Doc mode's page is in: its review opens while it is flattened.
        if let Some(f) = self.doc.as_mut() {
            if f.take_open() {
                return Nav::Push(Box::new(crate::docscan::Review::new(f.doc.clone())));
            }
        }
        // The timer ran out: the photo now, timed from then.
        if let Some(at) = self.countdown.filter(|at| now >= *at) {
            self.countdown = None;
            self.shoot_at(cx.settings.calm, cx.settings.detail, at);
        }
        // The intervalometer: the next photo when it is due and the camera
        // has the last one's picture (a beat missed is skipped, not made up).
        if let Some(run) = &self.kit.run {
            if run.finished() {
                let n = run.taken;
                self.stop_run(&format!("done · {n} taken"));
            } else if run.due(now) && !(self.saving.is_some() && !self.captured) && self.preview.is_some() {
                self.shoot_at(true, crate::burst::DetailMode::Off, now);
                if let Some(run) = &mut self.kit.run {
                    run.took(now);
                }
            }
        }
        // A photo the camera never finished: given up on at the step it
        // stopped at (each bounded: catalyst-camera's own waits are shorter),
        // said, and logged with its marks.
        if let Some(why) = self.kit.shot.as_ref().and_then(|t| t.overdue(now)) {
            if let Some(t) = self.kit.shot.take() {
                eprintln!("{} · given up: {why}", t.log_line());
            }
            self.saving = None;
            self.pending = 0;
            self.pending_paths.clear();
            self.captured = true;
            if self.instant.take().is_some() {
                self.load_last(None);
            }
            self.say(format!("{why}: no photo"));
        }
        // Taken, and never written (its program gone quiet): said rather
        // than saving for ever.
        if self.kit.shot.is_none() && self.saving.is_some_and(|at| at.elapsed() > Duration::from_secs(45)) {
            self.saving = None;
            self.pending = 0;
            self.pending_paths.clear();
            self.kit.writing.clear();
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
        changed |= self.dial_at.tick(dt) | self.dial_fade.tick(dt) | self.strip_at.tick(dt) | self.strip_fade.tick(dt) | self.focus_fade.tick(dt);
        if let Some(f) = &mut self.doc {
            changed |= f.tick(dt);
        }
        // The shot lands in the corner, and the thumbnail pops as it does.
        let flying = self.flying.is_some();
        changed |= self.fly.tick(dt);
        if flying && self.fly.value() > 0.97 {
            self.flying = None;
            self.last_pop.jump(0.7);
            self.last_pop.set(1.0);
            changed = true;
        }
        changed
    }

    /// What arrives from outside and what time alone changes, drawn as it
    /// comes and never holding the frame beat (0.9.4): the camera's frames
    /// and news, a note fading, the timer, "hold still", the saving dots,
    /// the intervalometer, the level.
    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        if self.frozen {
            return false;
        }
        let mut changed = false;
        if let Some((_, age)) = &mut self.note {
            *age += dt.min(0.25);
            // Drawn while it fades in and out; held still between.
            let fading = *age < 0.2 || *age > NOTE_FOR;
            changed |= fading;
            if *age > NOTE_FOR + 0.4 {
                self.note = None;
                changed = true;
            }
        }
        changed |= self.tick_kit(dt);
        changed |= self.tick_smart();
        changed |= self.tick_refining();
        if let Some(rx) = &self.last_rx {
            if let Ok(last) = rx.try_recv() {
                self.last_rx = None;
                let path = last.as_ref().map(|(p, _)| p.path.clone());
                // A photo still being written keeps its instant thumbnail:
                // an older one from the album does not take its place.
                let waiting = self.instant.as_ref().is_some_and(|i| self.pending_paths.contains(i) && path.as_ref() != Some(i));
                if !waiting {
                    let arrived = path != self.last.as_ref().map(|(p, _)| p.path.clone());
                    self.last = last;
                    if arrived && self.last.is_some() {
                        self.last_pop.jump(0.6);
                        self.last_pop.set(1.0);
                    }
                    changed = true;
                }
            }
        }
        let events = {
            let Ok(mut l) = self.live.lock() else { return changed };
            if l.frames != self.seen {
                self.seen = l.frames;
                if let Some(p) = l.preview.take() {
                    self.preview = Some(p);
                }
                if l.meta.is_some() {
                    self.metered = l.meta;
                }
                self.histogram.clone_from(&l.histogram);
                if let Some(rgb) = l.rgb.take() {
                    self.kit.rgb = rgb;
                }
                if l.thumbs_made != self.thumbs_seen {
                    self.thumbs_seen = l.thumbs_made;
                    self.thumbs = std::mem::take(&mut l.thumbs);
                }
                // Doc mode: the frame's brightness to the page's finder, a
                // few times a second.
                if let (Mode::Doc, Some(f)) = (self.mode, &mut self.doc) {
                    let (w, h) = l.yuv_size;
                    if w > 0 && l.yuv.len() >= w * h && f.wants() {
                        f.give(&l.yuv[..w * h], w, h);
                    }
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
                // A Scout code: its records merge in as it is read.
                if let Some(Code::Text(text)) = code {
                    crate::scout::scanned(text);
                }
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
        // The camera program was not yet listening when the dials were first
        // told: told again once its frames come.
        if self.told.is_none() && self.preview.is_some() {
            self.tell_pro();
        }
        // A photo's exposure slower than a hand holds, said once as it gets so.
        let shaky = self.mode.shoots() && self.shutter_now().is_some_and(pro::shaky);
        if shaky && !self.shaky_said {
            self.say("slow shutter: hold steady");
        }
        self.shaky_said = shaky;
        // Doc mode: the page found, and held still long enough to take.
        let mut take_page = false;
        if let (Mode::Doc, Some(f)) = (self.mode, &mut self.doc) {
            let (ch, take) = f.ambient(dt.min(0.25));
            changed |= ch | f.doc.borrow_mut().poll();
            take_page = take;
        }
        if take_page && !self.frozen {
            self.shoot_doc();
        }
        for event in events {
            // Doc mode's photo: its page goes to the document, not the album.
            if let Event::Saved { path, .. } = &event {
                if self.doc.as_mut().is_some_and(|f| f.took(path)) {
                    self.pending = self.pending.saturating_sub(1);
                    self.pending_paths.retain(|p| p != path);
                    if self.pending == 0 {
                        self.saving = None;
                    }
                    changed = true;
                    continue;
                }
            }
            if let (Event::Failed(_), Some(f)) = (&event, &mut self.doc) {
                f.failed();
            }
            match event {
                Event::Exposing(line) => {
                    if let Some(t) = &mut self.kit.shot {
                        t.exposing = Some(camkit::read_exposing(&line));
                        t.mark(Mark::Exposing, Instant::now());
                    }
                }
                Event::Exposed => {
                    if let Some(t) = &mut self.kit.shot {
                        t.mark(Mark::Exposed, Instant::now());
                    }
                }
                Event::Captured(marks) => {
                    self.captured = true;
                    // Timed on to its writing, out of the way of the next.
                    if let Some(mut t) = self.kit.shot.take() {
                        t.mark(Mark::Captured, Instant::now());
                        t.camera = marks;
                        self.kit.writing.push(t);
                    }
                }
                Event::Saved { path, size, detail, raw_skipped } => {
                    self.pending = self.pending.saturating_sub(1);
                    self.pending_paths.retain(|p| *p != path);
                    if self.pending == 0 {
                        self.saving = None;
                    }
                    if let Some(at) = self.kit.writing.iter().position(|t| t.path == path) {
                        let mut t = self.kit.writing.remove(at);
                        t.mark(Mark::Written, Instant::now());
                        eprintln!("{}", t.log_line());
                        self.kit.last_shot = Some(t);
                    }
                    if raw_skipped {
                        self.say("raw skipped: no room");
                    }
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
                    } else if size.0.max(size.1) < 2000 {
                        self.say(format!("saved · {} × {} (memory was short)", size.0, size.1));
                    }
                    self.load_last(Some(path));
                }
                Event::Failed(why) => {
                    self.pending = self.pending.saturating_sub(1);
                    if self.pending == 0 {
                        self.saving = None;
                        self.pending_paths.clear();
                    }
                    // The instant thumbnail was of a photo that never came.
                    if self.instant.take().is_some() {
                        self.load_last(None);
                    }
                    if let Some(t) = self.kit.shot.take() {
                        eprintln!("{} · failed: {why}", t.log_line());
                    }
                    self.captured = true;
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
            for a in [&mut self.mode_at, &mut self.press, &mut self.last_pop, &mut self.found_rise, &mut self.dial_at, &mut self.dial_fade, &mut self.strip_at, &mut self.strip_fade, &mut self.focus_fade, &mut self.fly] {
                a.jump(a.target());
            }
            self.flash.jump(0.0);
            self.flying = None;
        }
        // Focus peaking in the signal colour.
        let signal = [t.signal.0, t.signal.1, t.signal.2];
        if self.peak != signal {
            self.peak = signal;
            if self.focus.is_some() {
                self.tell_develop();
            }
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
                // The focus check's crop: its middle, one pixel to one.
                Some(pic) if self.cropped() => {
                    let view = [((pic.w as f32 - vf.w) * 0.5).max(0.0), ((pic.h as f32 - vf.h) * 0.5).max(0.0), vf.w.min(pic.w as f32), vf.h.min(pic.h as f32)];
                    draw_picture(g, vf, pic, view, 0.0, 1.0 - ka);
                }
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
        let whole_view = !self.cropped();
        if let (Some(face), true) = (self.face, ka < 0.5 && self.mode.shoots() && whole_view) {
            let r = Self::on_viewfinder(vf, face);
            brackets(g, r, t.signal, 0.85 * (1.0 - 2.0 * ka));
        }
        // The white balance dial out in pro mode: where the grey card is read
        // (the middle sixth of the picture each way, camkit's middle_mean).
        if self.mode == Mode::Pro && self.dial == Some(Dial::Wb) && whole_view && ka < 0.5 {
            let r = Self::on_viewfinder(vf, [5.0 / 12.0, 5.0 / 12.0, 7.0 / 12.0, 7.0 / 12.0]);
            brackets(g, r, white, 0.85 * self.dial_fade.value().clamp(0.0, 1.0));
        }
        // The aspect's frame (the rest dimmed), its grid and the level: the
        // shapes worked out once for the viewfinder and kept until a setting
        // changes (camkit's Overlay), the level only as it turns.
        if self.mode.shoots() && whole_view && ka < 0.999 {
            let a = 1.0 - ka;
            self.kit.overlay.update(self.kit.grid, self.kit.aspect, (vf.x, vf.y, vf.w, vf.h));
            for &(x, y, w, h) in &self.kit.overlay.bars {
                g.canvas.fill_rect(Rect::new(x + g.ox, y, w, h), Color(0, 0, 0), 0.55 * a);
            }
            let (fx, fy, fw, fh) = self.kit.overlay.frame;
            let frame = Rect::new(fx, fy, fw, fh);
            if self.kit.aspect != Aspect::Full {
                outline(g, frame, 1.0, white, 0.5 * a);
            }
            for &(x, y, w, h) in &self.kit.overlay.lines {
                g.canvas.fill_rect(Rect::new(x + g.ox, y, w, h), white, 0.35 * a);
            }
            if let (true, Some(h)) = (self.kit.level, self.kit.horizon) {
                draw_level(g, frame, h, a);
            }
        }
        // Scan mode: a square to aim with, the code outlined once read.
        let scan = (1.0 - self.mode_at.value()).clamp(0.0, 1.0);
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
        // Doc mode: the page found, outlined.
        if let (Mode::Doc, Some(f), true) = (self.mode, &self.doc, ka < 0.5) {
            f.draw(g, vf, t.signal);
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
        // Pro mode's histogram and dials, the looks' strip, the focus
        // check's chrome, and what the photo will wear (all on the main lens).
        let main = 1.0 - ka;
        let mut bottom = 0.0;
        if main > 0.01 && self.mode.shoots() {
            // The top row gives way to a note while it shows.
            let note = self.note.as_ref().map_or(0.0, |(_, age)| ((NOTE_FOR + 0.4 - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0));
            bottom = self.draw_overlays(g, vf, main, main * (1.0 - note));
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
        // (Above the dials or the strip, when they are out.)
        let h = ty.caption.size + 8.0;
        let base = vf.bottom() - 7.0 - bottom;
        let pill = |g: &mut Gfx, icon: Option<&str>, words: &str, widest: &str| {
            let lead = if icon.is_some() { h - 2.0 } else { 0.0 };
            let w = g.measure(widest, ty.caption) + 16.0 + lead;
            g.canvas.rrect(Rect::new(vf.x + 7.0 + g.ox, base - h, w, h), h * 0.5, Color(0, 0, 0), 0.55);
            if let Some(icon) = icon {
                g.icon(icon, true, vf.x + 7.0 + h * 0.5 + 1.0, base - h * 0.5, h * 0.55, t.signal);
            }
            g.text_at(vf.x + 15.0 + lead, base - h * 0.5, Align::Left, words, ty.caption, white);
        };
        // "Hold still" only while the sensor exposes something long enough
        // to matter, with the seconds left; else the camera at it, then
        // saving; then a slow shutter's warning; then the timing readout.
        let now = Instant::now();
        let dots = ".".repeat(self.dots_drawn.max(1));
        let mut said = true;
        let room = vf.w - 14.0 - 16.0 - (h - 2.0);
        if let Some((words, left)) = self.kit.shot.as_ref().and_then(|t| t.hold_still(now)) {
            // The longest form that fits at this text size (its width kept
            // as the seconds count down).
            let forms = [(format!("{words} · {left:.1} s"), format!("{words} · 88.8 s")), (format!("hold still · {left:.1} s"), "hold still · 88.8 s".to_owned()), (format!("{left:.1} s"), "88.8 s".to_owned())];
            let (shown, widest) = forms.iter().find(|(_, w)| g.measure(w, ty.caption) <= room).unwrap_or(&forms[2]);
            pill(g, Some("back_hand"), shown, widest);
        } else if self.kit.shot.as_ref().is_some_and(|t| self.waiting_long(t, now)) {
            pill(g, None, &format!("taking it{dots}"), "taking it...");
        } else if self.saving.is_some() && (self.captured || self.kit.shot.is_none()) {
            pill(g, None, &format!("saving{dots}"), "saving...");
        } else if let (Some(us), true) = (self.shutter_now(), self.mode == Mode::Photo && main > 0.5 && !self.strip && self.saving.is_none()) {
            // Photo mode's own exposure, when it gets too slow to hold.
            said = pro::shaky(us);
            if said {
                let words = pro::shutter_words(us);
                pill(g, Some("back_hand"), &words, &words);
            }
        } else {
            said = self.saving.is_some();
        }
        // Pro mode's `t`: the last photo's timing, to report real numbers.
        if !said && self.kit.readout && self.mode == Mode::Pro && main > 0.5 && !self.strip && self.kit.panel.is_none() {
            let forms = self.kit.last_shot.as_ref().map_or_else(|| ["last shot: none yet".to_owned(), "no shot yet".to_owned(), "no shot yet".to_owned(), "none".to_owned()], Timeline::readout);
            let words = forms.iter().find(|f| g.measure(f, ty.caption) <= room).unwrap_or(&forms[3]).clone();
            pill(g, Some("timer"), &words, &words);
        }
        // The lens switch, when there is a smart camera to switch to.
        if self.smart.is_some() && !self.quick {
            // Above the dials or the strip: glass apart from glass.
            self.draw_lens_switch(g, Rect::new(vf.x, vf.y, vf.w, vf.h - bottom));
        }

        self.draw_rail(g, rail);
        self.draw_fly(g, vf, rail);
    }
}

impl Camera {
    /// The level on (the motion sensor held at its full rate while it is)
    /// or off.
    fn set_level(&mut self, on: bool) {
        self.kit.level = on;
        self.kit.imu = on.then(|| crate::imu::hold(crate::imu::Use::Motion));
        self.kit.horizon = None;
        self.keep_prefs();
        self.say(match (on, crate::imu::has_motion()) {
            (false, _) => "level off",
            (true, true) => "level on",
            (true, false) => "no motion sensor",
        });
    }

    /// The white balance from a grey card: what is in the middle of the
    /// picture now, made as red and blue as it is green.
    fn grey_card(&mut self) {
        let (Some(pic), Some(m)) = (&self.preview, self.metered) else {
            self.say("the camera is not ready yet");
            return;
        };
        let mean = camkit::middle_mean(&pic.rgb, pic.w, pic.h);
        // The preview's own gains, or the card's if one is set already.
        let current = self.pro.wb.gains().unwrap_or(m.gains);
        match camkit::grey_gains(mean, current) {
            Ok(gains) => {
                self.pro.wb = pro::Wb::grey(gains);
                self.pro_changed();
                eprintln!("camera: grey card: gains r {:.3} b {:.3}", gains[0], gains[1]);
                self.say("wb from the grey card");
            }
            Err(why) => self.say(why),
        }
    }

    /// Everything as set now, for a preset.
    fn setup(&self) -> camkit::Setup {
        camkit::Setup {
            pro: self.pro,
            look: self.look,
            hue: self.hue,
            raw: self.raw,
            zebras: self.zebras,
            false_colour: self.kit.false_colour,
            hist: self.kit.hist,
            grid: self.kit.grid,
            aspect: self.kit.aspect,
            bracket: self.kit.bracket,
            timer: self.timer,
            level: self.kit.level,
            night: self.night,
        }
    }

    /// A preset recalled: everything as it was kept.
    fn recall(&mut self, s: &camkit::Setup) {
        self.pro = s.pro;
        self.look = s.look;
        self.hue = s.hue;
        self.raw = s.raw;
        self.zebras = s.zebras;
        self.kit.false_colour = s.false_colour;
        self.kit.hist = s.hist;
        self.kit.grid = s.grid;
        self.kit.aspect = s.aspect;
        self.kit.bracket = s.bracket;
        self.timer = s.timer;
        if self.kit.level != s.level {
            self.kit.level = s.level;
            self.kit.imu = s.level.then(|| crate::imu::hold(crate::imu::Use::Motion));
            self.kit.horizon = None;
        }
        if self.night != s.night {
            self.night = s.night;
            self.control.send(Order::Night(self.night));
        }
        self.strip_at.set(LOOKS.iter().position(|l| *l == self.look).unwrap_or(0) as f32);
        self.tell_develop();
        self.pro_changed();
    }

    /// The intervalometer started: its first photo at once, the panel put
    /// away, the screen dimming after a few seconds.
    fn start_run(&mut self) {
        self.kit.panel = None;
        self.set_dial(None);
        self.kit.run = Some(camkit::Run::start(self.kit.interval, Instant::now()));
        self.keep_prefs();
        self.say(format!("every {} · {}", self.kit.interval.every_words(), self.kit.interval.count_words()));
    }

    fn stop_run(&mut self, words: &str) {
        let taken = self.kit.run.take().map_or(0, |r| r.taken);
        self.say(if words.contains("taken") { words.to_owned() } else { format!("{words} · {taken} taken") });
    }

    /// The kit's share of what time changes: the level's reading, and the
    /// words that count down (only a change is a new frame).
    fn tick_kit(&mut self, dt: f32) -> bool {
        let mut changed = false;
        if self.kit.level {
            if let Some(read) = crate::imu::reading().and_then(|r| camkit::Horizon::of(r.up)) {
                let before = self.kit.horizon;
                match &mut self.kit.horizon {
                    Some(h) => h.ease(read, dt.clamp(0.0, 0.25)),
                    None => self.kit.horizon = Some(read),
                }
                // Redrawn when it has turned a tenth of a degree, or crossed level.
                changed |= match (before, self.kit.horizon) {
                    (Some(a), Some(b)) => (a.roll - b.roll).abs() > 0.1 || (a.pitch - b.pitch).abs() > 0.3 || a.level() != b.level() || a.flat != b.flat,
                    (a, b) => a.is_some() != b.is_some(),
                };
            }
        }
        // The words that change with time, as they would be drawn now.
        let now = Instant::now();
        let mut words = String::new();
        if let Some(at) = self.countdown {
            words.push_str(&format!("t{}", at.saturating_duration_since(now).as_millis() / 1000));
        }
        if let Some((w, left)) = self.kit.shot.as_ref().and_then(|t| t.hold_still(now)) {
            words.push_str(&format!("h{w}{left:.1}"));
        }
        if let Some(run) = &self.kit.run {
            words.push_str(&run.words(now));
        }
        if let Some(since) = self.saving {
            let dots = (now.saturating_duration_since(since).as_secs_f32() * 3.0) as usize % 3 + 1;
            self.dots_drawn = dots;
            words.push_str(&format!("d{dots}"));
        }
        if self.kit.shot.as_ref().is_some_and(|t| self.waiting_long(t, now)) {
            words.push('w');
        }
        if words != self.kit.drawn {
            self.kit.drawn = words;
            changed = true;
        }
        // The countdown's pulse is motion of a kind: drawn while it lands.
        if let Some(at) = self.countdown {
            let left = at.saturating_duration_since(now).as_secs_f32();
            changed |= 1.0 - (left - left.floor()) < 0.4;
        }
        changed
    }

    /// A photo asked for and not yet exposing (or exposed, not yet in) for
    /// long enough to say the camera is at it.
    fn waiting_long(&self, t: &Timeline, now: Instant) -> bool {
        let since = |m: Mark| t.at(m).map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        match t.reached() {
            Mark::Key | Mark::Request => since(Mark::Key) > Duration::from_millis(600),
            Mark::Exposed => since(Mark::Exposed) > Duration::from_millis(600),
            Mark::Exposing => t.hold_still(now).is_none() && since(Mark::Exposing) > Duration::from_millis(600),
            _ => false,
        }
    }

    /// The focus check's frames are in (a crop, not the whole picture).
    fn cropped(&self) -> bool {
        self.focus.is_some() && self.metered.is_some_and(|m| m.zoom > 0.0)
    }

    /// What a photo taken now would be taken with.
    fn still_now(&self) -> Option<pro::StillControls> {
        self.metered.map(|m| pro::still((self.mode == Mode::Pro).then_some(&self.pro), &m, self.night))
    }

    fn shutter_now(&self) -> Option<u32> {
        self.still_now().map(|s| s.exposure_us)
    }

    /// The newest photo's place in the rail.
    fn thumb_rect(rail: Rect) -> Rect {
        let (tw, th) = (36.0, 27.0);
        Rect::new(rail.x + 10.0, H - PAD - th - 2.0, tw, th)
    }

    /// A word on a dark chip over the picture, at (x, y); its width.
    fn chip(g: &mut Gfx, x: f32, y: f32, icon: Option<&str>, words: &str, color: Color, alpha: f32) -> f32 {
        let cap = g.ty.caption;
        let h = cap.size + 8.0;
        let lead = if icon.is_some() { cap.size + 2.0 } else { 0.0 };
        let w = (g.measure(words, cap) + 14.0 + lead).round();
        g.canvas.rrect(Rect::new(x + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.55 * alpha);
        if let Some(icon) = icon {
            g.icon_alpha(icon, true, x + 7.0 + cap.size * 0.5, y + h * 0.5, cap.size, color, alpha);
        }
        g.text_alpha(x + 7.0 + lead, y + h * 0.5, words, cap, Color(255, 255, 255), alpha);
        w
    }

    /// The overlays on this camera's picture: what the photo will wear (its
    /// look, RAW), pro mode's histogram and dials, the looks' strip, the
    /// focus check's scale and where it is (the top row at `top`, which
    /// gives way to a note). Gives how much of the viewfinder's foot they take.
    fn draw_overlays(&mut self, g: &mut Gfx, vf: Rect, alpha: f32, top: f32) -> f32 {
        let t = g.tokens;
        let m = 6.0;
        let mut x = vf.x + m;
        if top > 0.01 {
            if self.focus.is_some() {
                let words = match self.metered.map(|m| m.zoom) {
                    Some(z) if z > 0.0 && z < 0.99 => "1:2",
                    _ => "1:1",
                };
                x += Self::chip(g, x, vf.y + m, Some("center_focus_strong"), words, t.signal, top) + 4.0;
            }
            if self.look != Look::Original && !self.strip {
                let name = self.look.name().to_lowercase();
                x += Self::chip(g, x, vf.y + m, Some("photo_filter"), &name, t.ice, top) + 4.0;
            }
            if self.mode == Mode::Pro && self.raw && self.kit.bracket.is_none() {
                x += Self::chip(g, x, vf.y + m, None, "raw", t.ink, top) + 4.0;
            }
            // The kit's state, as far as there is room before the histogram:
            // the intervalometer first (it is running), the aspect, the set.
            let cap = g.ty.caption;
            let mut hist = self.mode == Mode::Pro && self.kit.hist != HistMode::Off;
            let run_w = self.kit.run.as_ref().map_or(0.0, |r| g.measure(&r.words(Instant::now()), cap) + 16.0 + cap.size);
            // The intervalometer's count comes before the histogram, where
            // there is no room for both (it is running: that matters most).
            if hist && x + run_w > vf.right() - 6.0 - pro::BINS as f32 - 8.0 - 4.0 {
                hist = false;
            }
            let limit = if hist { vf.right() - 6.0 - pro::BINS as f32 - 8.0 - 4.0 } else { vf.right() - m };
            let mut chips: Vec<(Option<&str>, String, Color)> = Vec::new();
            if let Some(run) = &self.kit.run {
                chips.push((Some("timer"), run.words(Instant::now()), t.signal));
            }
            if self.kit.aspect != Aspect::Full {
                chips.push((None, self.kit.aspect.name().to_owned(), t.ink));
            }
            if let (Some(b), Mode::Pro) = (self.kit.bracket, self.mode) {
                chips.push((None, b.words(), t.ink));
            }
            for (icon, words, color) in chips {
                let w = g.measure(&words, cap) + 14.0 + if icon.is_some() { cap.size + 2.0 } else { 0.0 };
                if x + w <= limit {
                    x += Self::chip(g, x, vf.y + m, icon, &words, color, top) + 4.0;
                }
            }
            if hist {
                self.draw_histogram(g, vf, top);
            }
        }
        let strip = self.strip_fade.value().clamp(0.0, 1.0);
        let mut foot = 0.0f32;
        if self.kit.panel.is_some() {
            foot = foot.max(self.draw_panel(g, vf, alpha));
        } else if self.mode == Mode::Pro && strip < 0.99 {
            foot = foot.max(self.draw_dials(g, vf, alpha * (1.0 - strip)));
        }
        if strip > 0.01 {
            foot = foot.max(self.draw_strip(g, vf, alpha * strip));
        }
        if let Some((fx, fy)) = self.focus {
            // Where the crop is in the whole picture.
            let fade = self.focus_fade.value().clamp(0.0, 1.0) * alpha;
            let (bw, bh) = (40.0, 30.0);
            let b = Rect::new(vf.right() - m - bw, vf.bottom() - m - foot - bh, bw, bh);
            g.canvas.rrect(b.inset(-3.0).offset(g.ox, 0.0), 6.0, Color(0, 0, 0), 0.55 * fade);
            outline(g, b, 1.0, Color(255, 255, 255), 0.5 * fade);
            let (vw, vh) = ((vf.w / FULL.0 as f32 * bw).max(3.0), (vf.h / FULL.1 as f32 * bh).max(2.0));
            let at = Rect::new(b.x + fx * bw - vw * 0.5, b.y + fy * bh - vh * 0.5, vw, vh);
            g.canvas.fill_rect(at.offset(g.ox, 0.0), t.signal, fade);
            foot = foot.max(foot + bh + 6.0);
        }
        foot
    }

    /// The live histogram, top right: the camera's own brightness before any
    /// look, the clipping mark at its right in the signal colour.
    fn draw_histogram(&self, g: &mut Gfx, vf: Rect, alpha: f32) {
        let t = g.tokens;
        let (hw, hh) = (pro::BINS as f32, 24.0);
        let pad = 4.0;
        let b = Rect::new(vf.right() - 6.0 - hw - 2.0 * pad, vf.y + 6.0, hw + 2.0 * pad, hh + 2.0 * pad);
        g.canvas.rrect(b.offset(g.ox, 0.0), 8.0, Color(0, 0, 0), 0.55 * alpha);
        let base = b.bottom() - pad;
        // Red, green and blue, each its own colour, laid over each other
        // (where they agree they add toward white).
        if self.kit.hist == HistMode::Rgb {
            let colours = [Color(255, 86, 76), Color(92, 214, 112), Color(96, 150, 255)];
            for (heights, colour) in self.kit.rgb.heights().iter().zip(colours) {
                for (i, k) in heights.iter().enumerate() {
                    let height = (k * hh).max(if *k > 0.0 { 1.0 } else { 0.0 });
                    if height > 0.0 {
                        g.canvas.fill_rect(Rect::new(b.x + pad + i as f32 + g.ox, base - height, 1.0, height), colour, 0.6 * alpha);
                    }
                }
            }
            return;
        }
        for (i, k) in self.histogram.heights().iter().enumerate() {
            let height = (k * hh).max(if *k > 0.0 { 1.0 } else { 0.0 });
            if height > 0.0 {
                g.canvas.fill_rect(Rect::new(b.x + pad + i as f32 + g.ox, base - height, 1.0, height), Color(236, 238, 242), 0.85 * alpha);
            }
        }
        // False colour's key: each brightness's colour along the foot of
        // the histogram, where that brightness is counted.
        if self.kit.false_colour && self.focus.is_none() {
            let table = camkit::false_colour();
            for i in 0..pro::BINS {
                let [r, gr, bl] = table[(i * 256 + 128) / pro::BINS];
                g.canvas.fill_rect(Rect::new(b.x + pad + i as f32 + g.ox, base + 1.0, 1.0, 2.0), Color(r, gr, bl), alpha);
            }
        }
        if self.histogram.clipped > 0.002 {
            g.canvas.fill_rect(Rect::new(b.right() - pad - 2.0 + g.ox, b.y + pad, 2.0, hh), t.signal, alpha);
        }
    }

    /// Pro mode's dials: shutter, ISO, compensation and white balance in a
    /// bar of glass along the foot, the one being turned an ice capsule that
    /// slides between them (Bezel's mode buttons). Values set by hand in
    /// ink, the camera's own choice dimmer; a hand beside a shutter too slow
    /// to hold. Gives its height with its margin.
    fn draw_dials(&mut self, g: &mut Gfx, vf: Rect, alpha: f32) -> f32 {
        if alpha <= 0.01 {
            return 0.0;
        }
        let t = g.tokens;
        let cap = g.ty.caption;
        let m = 6.0;
        let bh = (cap.size + 12.0).round();
        let pad = 3.0;
        let r = Rect::new(vf.x + m, vf.bottom() - m - bh, vf.w - 2.0 * m, bh);
        g.glass(&[bezel::GlassShape::new(r.offset(g.ox, 0.0), bh * 0.5, 12)]);
        let cell = (r.w - 2.0 * pad) / DIALS.len() as f32;
        let metered = self.metered.unwrap_or_default();
        let still = self.still_now();
        let shutter_us = self.pro.shutter.map_or_else(|| still.map_or(metered.exposure_us, |s| s.exposure_us), |i| pro::SHUTTERS[i].us);
        let iso = self.pro.iso.map_or_else(|| still.map_or(metered.iso(), |s| (s.gain * 100.0).round() as u32), |i| pro::ISOS[i]);
        let iso_word = |long: bool| {
            let n = pro::iso_words(iso).trim_start_matches("ISO ").to_owned();
            if long { format!("iso {n}") } else { n }
        };
        let iso_text = if g.measure(&iso_word(true), cap) <= cell - 6.0 { iso_word(true) } else { iso_word(false) };
        let wb = match self.pro.wb {
            pro::Wb::Auto => "awb".to_owned(),
            k => k.words(),
        };
        // Fully manual, the compensation's place shows the meter instead.
        let ev = if self.pro.manual() {
            let stops = pro::meter(&metered, shutter_us, iso as f32 / 100.0);
            pro::ev_words((stops * 3.0).round().clamp(-9.0, 9.0) as i8)
        } else {
            self.pro.ev_words()
        };
        let cells = [
            (pro::shutter_words(shutter_us), self.pro.shutter.is_some()),
            (iso_text, self.pro.iso.is_some()),
            (ev, self.pro.ev != 0 && !self.pro.manual()),
            (wb, self.pro.wb != pro::Wb::Auto),
        ];
        // The capsule behind the dial being turned.
        let fade = self.dial_fade.value().clamp(0.0, 1.0);
        if fade > 0.01 {
            let x = r.x + pad + cell * self.dial_at.value();
            g.canvas.rrect(Rect::new(x + g.ox, r.y + pad, cell, bh - 2.0 * pad), bh * 0.5 - pad, t.ice, fade * alpha);
        }
        let shaky = pro::shaky(shutter_us);
        for (i, (words, set)) in cells.iter().enumerate() {
            let on = if self.dial.map(Dial::index) == Some(i) { fade } else { 0.0 };
            let rest = if *set { t.ink } else { t.dim };
            let ink = mix(rest, t.on_ice, on);
            let cx = r.x + pad + cell * (i as f32 + 0.5);
            let words = g.text.fit(words, &cap, cell - 4.0);
            let tw = g.measure(&words, cap);
            let icon = if i == 0 && shaky { cap.size + 2.0 } else { 0.0 };
            let left = cx - (tw + icon) * 0.5;
            if icon > 0.0 {
                g.icon_alpha("back_hand", true, left + cap.size * 0.5, r.cy(), cap.size, mix(t.signal, t.on_ice, on), alpha);
            }
            g.text_alpha(left + icon, r.cy(), &words, cap, ink, alpha);
        }
        bh + m
    }

    /// The intervalometer's panel: how often and how many, in a bar of
    /// glass along the foot as the dials are, the row being changed an ice
    /// capsule. Gives its height with its margin.
    fn draw_panel(&mut self, g: &mut Gfx, vf: Rect, alpha: f32) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let m = 6.0;
        let bh = (cap.size + 12.0).round();
        let pad = 3.0;
        let r = Rect::new(vf.x + m, vf.bottom() - m - bh, vf.w - 2.0 * m, bh);
        g.glass(&[bezel::GlassShape::new(r.offset(g.ox, 0.0), bh * 0.5, 12)]);
        let cell = (r.w - 2.0 * pad) / 2.0;
        let row = self.kit.panel.unwrap_or(0);
        g.canvas.rrect(Rect::new(r.x + pad + cell * row as f32 + g.ox, r.y + pad, cell, bh - 2.0 * pad), bh * 0.5 - pad, t.ice, alpha);
        let count = if self.kit.interval.count == 0 { "no end".to_owned() } else { format!("{} photos", self.kit.interval.count) };
        for (i, words) in [format!("every {}", self.kit.interval.every_words()), count].iter().enumerate() {
            let ink = if i == row { t.on_ice } else { t.ink };
            let words = g.text.fit(words, &cap, cell - 8.0);
            let half = g.measure(&words, cap) * 0.5;
            g.text_alpha(r.x + pad + cell * (i as f32 + 0.5) - half, r.cy(), &words, cap, ink, alpha);
        }
        bh + m
    }

    /// The looks' strip along the foot: each look's live thumbnail, the one
    /// chosen ringed in ice and named above, sliding to keep it in the
    /// middle. Gives its height with the name and margins.
    fn draw_strip(&mut self, g: &mut Gfx, vf: Rect, alpha: f32) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let m = 6.0;
        let (tw, th) = (THUMB.0 as f32, THUMB.1 as f32);
        let gap = 6.0;
        let rise = (1.0 - alpha) * 12.0;
        let y = vf.bottom() - m - th + rise;
        let at = self.strip_at.value();
        g.canvas.set_clip(vf.offset(g.ox, 0.0));
        let chosen = LOOKS.iter().position(|l| *l == self.look).unwrap_or(0);
        for (i, _) in LOOKS.iter().enumerate() {
            let cx = vf.cx() + (i as f32 - at) * (tw + gap);
            if cx + tw < vf.x || cx - tw > vf.right() {
                continue;
            }
            let r = Rect::new((cx - tw * 0.5).round(), y, tw, th);
            if i == chosen {
                // Concentric: the ring's corners are the thumbnail's and its gap.
                g.canvas.rrect(r.inset(-3.0).offset(g.ox, 0.0), 9.0, t.ice, alpha);
                g.canvas.rrect(r.inset(-1.5).offset(g.ox, 0.0), 7.5, Color(6, 6, 8), alpha);
            }
            // Pictures are drawn past any clip: cut to the viewfinder here.
            let (x0, x1) = (r.x.max(vf.x), r.right().min(vf.right()));
            if x1 - x0 < 1.0 {
                continue;
            }
            let part = Rect::new(x0, r.y, x1 - x0, r.h);
            let whole_one = x0 <= r.x && x1 >= r.right();
            match self.thumbs.get(i) {
                Some(p) => {
                    let k = p.w as f32 / r.w;
                    draw_picture(g, part, p, [(x0 - r.x) * k, 0.0, (x1 - x0) * k, p.h as f32], if whole_one { 6.0 } else { 0.0 }, alpha);
                }
                None => g.canvas.rrect(part.offset(g.ox, 0.0), 6.0, t.surface1, alpha),
            }
        }
        g.canvas.clear_clip();
        // The chosen look's name (and Tint's colour), above the strip.
        let name = self.look.name().to_lowercase();
        let h = cap.size + 8.0;
        let dot = if self.look == Look::Tint { cap.size } else { 0.0 };
        let w = g.measure(&name, cap) + 16.0 + if dot > 0.0 { dot + 4.0 } else { 0.0 };
        let ly = y - 7.0 - h;
        g.canvas.rrect(Rect::new(vf.cx() - w * 0.5 + g.ox, ly, w, h), h * 0.5, Color(0, 0, 0), 0.55 * alpha);
        let mut tx = vf.cx() - w * 0.5 + 8.0;
        if dot > 0.0 {
            let c = crate::filters::tint_colour(self.hue).map(|v| (v * 255.0).round() as u8);
            g.canvas.rrect(Rect::new(tx + g.ox, ly + (h - dot) * 0.5, dot, dot), dot * 0.5, Color(c[0], c[1], c[2]), alpha);
            tx += dot + 4.0;
        }
        g.text_alpha(tx, ly + h * 0.5, &name, cap, Color(255, 255, 255), alpha);
        th + m + 7.0 + h - rise
    }

    /// The shot flying from the viewfinder into the rail's corner.
    fn draw_fly(&self, g: &mut Gfx, vf: Rect, rail: Rect) {
        let Some(pic) = &self.flying else { return };
        let k = self.fly.value().clamp(0.0, 1.0);
        let to = Self::thumb_rect(rail);
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        let r = Rect::new(lerp(vf.x, to.x), lerp(vf.y, to.y), lerp(vf.w, to.w), lerp(vf.h, to.h));
        draw_picture(g, r, pic, whole(pic), lerp(0.0, 5.0), 1.0);
    }

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
            Status::Absent(why) => ("developer_board", "no smart camera answers".to_owned(), why.clone()),
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
        // The modes: the one in use named, sliding aside and fading as the
        // next comes in (← →), over a pager's dots, the one in use in ink.
        let cap = ty.caption;
        let at = self.mode_at.value().clamp(0.0, (MODES.len() - 1) as f32);
        let cy = PAD + cap.size * 0.5 + 2.0;
        for (i, (_, name)) in MODES.iter().enumerate() {
            let d = i as f32 - at;
            let a = (1.0 - d.abs() * 1.4).clamp(0.0, 1.0);
            if a > 0.01 {
                let half = g.measure(name, cap) * 0.5;
                g.text_alpha(rail.cx() + d * 22.0 - half, cy, name, cap, t.ink, a);
            }
        }
        let (dot, gap) = (4.0, 5.0);
        let row = MODES.len() as f32 * dot + (MODES.len() - 1) as f32 * gap;
        let dy = cy + cap.size * 0.5 + 6.0;
        for i in 0..MODES.len() {
            let x = rail.cx() - row * 0.5 + i as f32 * (dot + gap);
            g.canvas.rrect(Rect::new(x + g.ox, dy, dot, dot), dot * 0.5, t.faint, 1.0);
        }
        // The one in use: a capsule that stretches as it slides.
        let stretch = (1.0 - (at - at.round()).abs() * 2.0).clamp(0.0, 1.0);
        let w = dot + (dot + gap) * (1.0 - stretch);
        let x = rail.cx() - row * 0.5 + at * (dot + gap) - (w - dot) * 0.5;
        g.canvas.rrect(Rect::new(x + g.ox, dy, w, dot), dot * 0.5, t.ink, 1.0);
        // The lens's aperture: fixed, so said, never a dial; and who sets
        // the exposure (p, s, iso, m), where there is room for both.
        let pro_k = (1.0 - (at - 2.0).abs()).clamp(0.0, 1.0);
        if pro_k > 0.01 && self.lens == Lens::Main {
            let program = Program::of(&self.pro).word();
            let both = format!("{program} · {}", pro::APERTURE);
            let words = if g.measure(&both, cap) <= rail.w - 8.0 { both } else { pro::APERTURE.to_owned() };
            let half = g.measure(&words, cap) * 0.5;
            g.text_alpha(rail.cx() - half, dy + dot + 4.0 + cap.size * 0.5, &words, cap, t.dim, pro_k);
        }

        // The shutter: a ring, and in it a disc that presses in.
        let (sx, sy) = (rail.cx(), (H * 0.5).round());
        let ring = 21.0;
        g.canvas.rrect(Rect::new(sx - ring + g.ox, sy - ring, ring * 2.0, ring * 2.0), ring, t.ink, 1.0);
        g.canvas.rrect(Rect::new(sx - ring + 2.5 + g.ox, sy - ring + 2.5, ring * 2.0 - 5.0, ring * 2.0 - 5.0), ring - 2.5, t.ground, 1.0);
        let inner = (ring - 5.5) * (1.0 - 0.12 * self.press.value().clamp(0.0, 1.0));
        match self.mode {
            Mode::Photo | Mode::Pro => {
                let a = if self.saving.is_some() && !self.captured { 0.45 } else { 1.0 };
                g.canvas.rrect(Rect::new(sx - inner + g.ox, sy - inner, inner * 2.0, inner * 2.0), inner, t.ink, a);
                if self.timer > 0 && self.countdown.is_none() {
                    g.text_at(sx, sy, Align::Center, &format!("{}s", self.timer), ty.caption, t.ground);
                }
            }
            Mode::Scan => g.icon("qr_code_scanner", false, sx, sy, 18.0, t.ink),
            // Doc mode: the page held still fills the ring, and is taken as it closes.
            Mode::Doc => {
                g.icon("document_scanner", false, sx, sy, 18.0, t.ink);
                if let Some(hold) = self.doc.as_ref().and_then(crate::docscan::Finding::hold).filter(|h| *h > 0.01) {
                    g.canvas.arc(sx + g.ox, sy, ring - 1.25, 2.5, -std::f32::consts::FRAC_PI_2, hold * std::f32::consts::TAU, t.signal, 1.0);
                }
            }
        }

        // The last photo, opening the album (↓); while a shot flies into
        // it, its place waits.
        let thumb = Self::thumb_rect(rail);
        let (tw, th) = (thumb.w, thumb.h);
        let pop = self.last_pop.value();
        let grown = Rect::new(thumb.cx() - tw * pop * 0.5, thumb.cy() - th * pop * 0.5, tw * pop, th * pop);
        // Doc mode: the newest page in the thumbnail's place, and how many.
        let pages = self.doc.as_ref().filter(|_| self.mode == Mode::Doc).map(|f| {
            let mut doc = f.doc.borrow_mut();
            let n = doc.pages.len();
            (n, doc.pages.last_mut().and_then(|p| p.shown().cloned()))
        });
        if let Some((n, page)) = pages {
            g.surface(thumb, 5.0, t.surface1);
            match &page {
                Some(pic) => {
                    let (w, h) = crate::editor::fit_size(pic.w as f32, pic.h as f32, tw - 6.0, th - 6.0);
                    draw_picture(g, Rect::new((thumb.cx() - w * 0.5).round(), (thumb.cy() - h * 0.5).round(), w.round(), h.round()), pic, whole(pic), 2.0, 1.0);
                }
                None => g.icon("description", false, thumb.cx(), thumb.cy(), 13.0, if n > 0 { t.ink } else { t.faint }),
            }
            if n > 0 {
                let words = n.to_string();
                let bh = cap.size + 4.0;
                let bw = (g.measure(&words, cap) + 8.0).max(bh);
                let b = Rect::new(thumb.right() - bw + 3.0, thumb.y - 3.0, bw, bh);
                g.surface(b, bh * 0.5, t.ice);
                g.text_at(b.cx(), b.cy(), Align::Center, &words, cap, t.on_ice);
            }
        } else {
            match &self.last {
                Some((_, pic)) if self.flying.is_none() => draw_picture(g, grown, pic, whole(pic), 5.0, 1.0),
                Some(_) => g.surface(thumb, 5.0, t.surface1),
                None => {
                    g.surface(thumb, 5.0, t.surface1);
                    g.icon("photo_library", false, thumb.cx(), thumb.cy(), 13.0, t.faint);
                }
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
        let choices = [("timer", "c", self.timer > 0), ("grid_on", "g", self.kit.grid != Grid::Off), ("bedtime", "n", self.night)];
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
/// The level over the frame `r`: the true horizon through its middle (two
/// arms either side of a gap, turned as the horizon lies in the picture)
/// against short fixed marks at the frame's sides, ice once level, and the
/// degrees off below it; lying flat, a ring and a bubble that settles in
/// it. Sizes are my own tuning for the 227 px viewfinder.
fn draw_level(g: &mut Gfx, r: Rect, h: camkit::Horizon, alpha: f32) {
    let t = g.tokens;
    let white = Color(255, 255, 255);
    let level = h.level();
    let color = if level { t.ice } else { white };
    let (cx, cy) = (r.cx() + g.ox, r.cy());
    let cap = g.ty.caption;
    if h.flat {
        let ring = 11.0;
        g.canvas.stroke_rrect(Rect::new(cx - ring, cy - ring, ring * 2.0, ring * 2.0), ring, 1.5, white, 0.6 * alpha);
        let (bx, by) = ((h.roll * 1.6).clamp(-30.0, 30.0), (-h.pitch * 1.6).clamp(-30.0, 30.0));
        g.canvas.circle(cx + bx, cy + by, 4.0, color, 0.9 * alpha);
        return;
    }
    // The handheld turned clockwise turns the picture's horizon the other way.
    let a = h.roll.to_radians();
    let (dx, dy) = (a.cos(), -a.sin());
    let (gap, arm) = (10.0, (r.w * 0.3).round());
    let width = if level { 2.0 } else { 1.5 };
    for s in [-1.0f32, 1.0] {
        let (x0, y0) = (cx + s * gap * dx, cy + s * gap * dy);
        let (x1, y1) = (cx + s * (gap + arm) * dx, cy + s * (gap + arm) * dy);
        // A shadow beneath, so it reads over a bright sky as over the ground.
        stroke(g, (x0, y0 + 1.0), (x1, y1 + 1.0), width + 1.0, Color(0, 0, 0), 0.35 * alpha);
        stroke(g, (x0, y0), (x1, y1), width, color, 0.95 * alpha);
        // The fixed marks: where level would lie.
        let mx = cx + s * (gap + arm + 6.0);
        g.canvas.fill_rect(Rect::new(mx.min(mx + s * 8.0), cy - 0.5, 8.0, 1.0), white, 0.5 * alpha);
    }
    if !level {
        // The degrees off, on a dark chip below the middle.
        let words = format!("{:.1}°", h.roll.abs());
        let (w, ch) = (g.measure(&words, cap) + 10.0, cap.size + 6.0);
        let top = cy + 6.0;
        g.canvas.rrect(Rect::new(cx - w * 0.5, top, w, ch), ch * 0.5, Color(0, 0, 0), 0.5 * alpha);
        g.text_alpha(cx - g.ox - w * 0.5 + 5.0, top + ch * 0.5, &words, cap, white, alpha);
    }
}

/// A line from `a` to `b` (already offset), `width` wide.
fn stroke(g: &mut Gfx, a: (f32, f32), b: (f32, f32), width: f32, color: Color, alpha: f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
    g.canvas.polygon(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], color, alpha);
}

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
    scene_part(w, h, t, with_code, [0.0, 0.0, 1.0, 1.0])
}

/// Part of the scene (`view`: x0, y0, x1, y1, 0 to 1 of the whole) drawn
/// at `w` × `h`: the focus check's crop at the photo's own scale.
pub fn scene_part(w: usize, h: usize, t: f32, with_code: bool, view: [f32; 4]) -> Picture {
    let mut pic = Picture::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    let (sx, sy) = ((view[2] - view[0]) / fw, (view[3] - view[1]) / fh);
    // The whole picture's shape (the sun is round in it).
    let aspect = 4.0 / 3.0;
    // A hand's drift, and the light changing with the time.
    let (jx, jy) = ((t * 0.7).sin() * 0.012, (t * 0.53).cos() * 0.01);
    let warm = 0.5 + 0.5 * (t * 0.11).sin();
    let horizon = 0.58 + jy;
    let sun = (0.72 + jx + 0.05 * (t * 0.05).sin(), 0.3 + jy);
    for y in 0..h {
        let v = view[1] + y as f32 * sy;
        for x in 0..w {
            let u = view[0] + x as f32 * sx;
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
                let d = (((u - sun.0) * aspect).powi(2) + (v - sun.1).powi(2)).sqrt();
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
                // Its weave, fine enough to show only near the photo's own scale.
                let (px, py) = ((u * 3280.0) as u32, (v * 2464.0) as u32);
                let weave = (px.wrapping_mul(2_654_435_761) ^ py.wrapping_mul(40_503)).wrapping_mul(2_246_822_519) >> 28;
                // (Averaged away as a smaller picture would: each of its
                // pixels stands for many.)
                let fibre = (weave as f32 - 7.5) / (sx * 3280.0).max(1.0);
                (r, g, b) = (70.0 + 20.0 * k + fibre, 78.0 + 18.0 * k + fibre, 86.0 + 14.0 * k + fibre);
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
        ..Results::default()
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
            // Detail's burst being exposed: "hold still" with its seconds.
            let now = Instant::now();
            let mut tl = Timeline::new(PathBuf::from("IMG_burst.jpg"), now - Duration::from_millis(500));
            tl.mark(Mark::Request, now - Duration::from_millis(490));
            tl.exposing = Some((camkit::Exposing::Detail, Duration::from_millis(1_800), 6));
            tl.mark(Mark::Exposing, now - Duration::from_millis(300));
            c.kit.shot = Some(tl);
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

/// The Camera's 0.9.2 pictures (`catalyst-shell --camera-pro-shots DIR`):
/// pro mode's viewfinder (a dial being turned, a slow shutter, the zebras
/// and histogram; and all in auto), the looks' strip, the focus check with
/// peaking, photo mode's slow-shutter warning and the shutter's instant
/// thumbnail flying in, in dark and light at each text size; each look on
/// the sample scene in one sheet; and a sheet of each screen's six.
#[cfg(not(target_arch = "aarch64"))]
pub fn pro_shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).ok();
    crate::smartcam::sim::choose(false);
    let t = 3.1;
    // The sample scene: the pit at the end of the afternoon, the robot on the carpet.
    let sample = scene(PREVIEW_W, PREVIEW_H, t, false);
    let yuv_of = |pic: &Picture| rgb_to_yuv420(pic);
    let developed = |yuv: &[u8], look: Look, hue: f32, zebras: bool| {
        let mut out = Picture::default();
        look.compiled(hue).develop(yuv, PREVIEW_W, PREVIEW_H, &mut out, zebras.then_some(0));
        out
    };
    let metered = |exposure_us: u32, gain: f32, zoom: f32| Metered { exposure_us, analogue: gain, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom };
    let thumbs = |yuv: &[u8], hue: f32| {
        let small = shrink_yuv420(yuv, PREVIEW_W, PREVIEW_H, THUMB.0, THUMB.1);
        LOOKS
            .iter()
            .map(|l| {
                let mut p = Picture::default();
                l.compiled(hue).develop(&small, THUMB.0, THUMB.1, &mut p, None);
                p
            })
            .collect::<Vec<_>>()
    };
    let base_yuv = yuv_of(&sample);
    // Overexposed half a stop and more for the zebras (the sun, its glow and the lines clip).
    let mut bright = sample.clone();
    sim_expose(&mut bright, 1.6, SIM_CT);
    let bright_yuv = yuv_of(&bright);
    // The focus check's crop: the robot's bumper and the carpet's lines, 1:1.
    let crop = focus_view(0.47, 0.72);
    let mut crop_pic = scene_part(PREVIEW_W, PREVIEW_H, t, false, crop);
    sim_expose(&mut crop_pic, 1.0, SIM_CT);
    let crop_yuv = yuv_of(&crop_pic);

    type Setup = Box<dyn Fn(&mut Camera)>;
    let snap = |c: &mut Camera| {
        for a in [&mut c.mode_at, &mut c.dial_at, &mut c.dial_fade, &mut c.strip_at, &mut c.strip_fade, &mut c.focus_fade] {
            a.jump(a.target());
        }
    };
    let screens: Vec<(&str, Setup)> = vec![
        ("1-pro-dial", {
            let (pic, hist) = (developed(&bright_yuv, Look::Original, 0.0, true), pro::Histogram::of_luma(&bright_yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2));
            Box::new(move |c: &mut Camera| {
                c.set_mode(Mode::Pro);
                c.pro = Pro { shutter: Some(pro::nearest_shutter(66_667)), iso: Some(0), ev: 0, wb: pro::Wb::Kelvin(5500) };
                c.set_dial(Some(Dial::Shutter));
                c.raw = true;
                c.preview = Some(pic.clone());
                c.histogram = hist.clone();
                c.histogram.clipped = 0.08;
                c.metered = Some(metered(66_667, 1.0, 0.0));
                c.note = None;
                snap(c);
            })
        }),
        ("2-pro-auto", {
            let (pic, hist) = (developed(&base_yuv, Look::Contemporary, 0.0, true), pro::Histogram::of_luma(&base_yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2));
            Box::new(move |c: &mut Camera| {
                c.set_mode(Mode::Pro);
                c.pro = Pro { ev: -1, ..Pro::default() };
                c.look = Look::Contemporary;
                c.set_dial(Some(Dial::Ev));
                c.preview = Some(pic.clone());
                c.histogram = hist.clone();
                c.metered = Some(metered(16_667, 3.2, 0.0));
                c.note = None;
                snap(c);
            })
        }),
        ("3-looks-strip", {
            let (pic, th) = (developed(&base_yuv, Look::VividWarm, 0.0, false), thumbs(&base_yuv, 32.0));
            Box::new(move |c: &mut Camera| {
                c.look = Look::VividWarm;
                c.strip = true;
                c.strip_fade.set(1.0);
                c.strip_at.set(2.0);
                c.thumbs = th.clone();
                c.preview = Some(pic.clone());
                c.metered = Some(metered(16_667, 2.0, 0.0));
                c.note = None;
                snap(c);
            })
        }),
        ("4-looks-tint", {
            let (pic, th) = (developed(&base_yuv, Look::Tint, 205.0, false), thumbs(&base_yuv, 205.0));
            Box::new(move |c: &mut Camera| {
                c.look = Look::Tint;
                c.hue = 205.0;
                c.strip = true;
                c.strip_fade.set(1.0);
                c.strip_at.set(14.0);
                c.thumbs = th.clone();
                c.preview = Some(pic.clone());
                c.metered = Some(metered(16_667, 2.0, 0.0));
                c.note = Some(("← →: looks · ↑ ↓: colour".into(), 1.0));
                snap(c);
            })
        }),
        ("5-focus-check", {
            let pic = developed(&crop_yuv, Look::Original, 0.0, false);
            Box::new(move |c: &mut Camera| {
                c.focus = Some((0.47, 0.72));
                c.focus_fade.set(1.0);
                let colour = [c.peak[0], c.peak[1], c.peak[2]];
                let mut pic = pic.clone();
                for (px, on) in pic.rgb.chunks_exact_mut(3).zip(pro::peaks(&crop_yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H)) {
                    if on {
                        px.copy_from_slice(&colour);
                    }
                }
                c.preview = Some(pic.clone());
                c.metered = Some(metered(16_667, 2.0, 1.0));
                c.note = None;
                snap(c);
            })
        }),
        ("6-photo-slow-shutter", {
            let mut dim = sample.clone();
            sim_expose(&mut dim, 0.55, SIM_CT);
            let pic = developed(&yuv_of(&dim), Look::Classic, 0.0, false);
            Box::new(move |c: &mut Camera| {
                c.look = Look::Classic;
                c.preview = Some(pic.clone());
                // Dim enough that photo mode's own exposure gets to 1/15.
                c.metered = Some(metered(66_667, 10.0, 0.0));
                c.night = true;
                c.note = None;
                snap(c);
            })
        }),
        ("7-instant-shot", {
            let pic = developed(&base_yuv, Look::Vivid, 0.0, false);
            Box::new(move |c: &mut Camera| {
                c.look = Look::Vivid;
                c.preview = Some(pic.clone());
                c.metered = Some(metered(8_000, 1.0, 0.0));
                c.flying = Some(pic.clone());
                c.fly.jump(0.42);
                c.saving = Some(Instant::now());
                c.captured = true;
                c.dots_drawn = 2;
                c.last = Some((Photo { path: PathBuf::from("IMG_new.jpg"), name: "IMG_new.jpg".into(), taken: chrono::Local::now().naive_local(), bytes: 0 }, pic.cover(photos::THUMB_W, photos::THUMB_H)));
                c.note = None;
                snap(c);
            })
        }),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..screens.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
            let tone = if light { "light" } else { "dark" };
            for (n, (name, setup)) in screens.iter().enumerate() {
                let mut camera = Camera::new();
                camera.frozen = true;
                camera.flash.jump(0.0);
                // As set here, whatever this computer's Camera kept.
                (camera.look, camera.hue, camera.raw, camera.zebras, camera.pro) = (Look::Original, 32.0, false, true, Pro::default());
                camera.last = None;
                camera.last_rx = None;
                let mut shell = Shell::new(crate::robot::Robot::demo());
                (shell.settings.text, shell.settings.light) = (size, light);
                shell.apply_settings();
                // The peaking colour is the theme's signal.
                let s = shell.gfx.tokens.signal;
                camera.peak = [s.0, s.1, s.2];
                setup(&mut camera);
                shell.show(Box::new(camera));
                for _ in 0..90 {
                    if shell.tick(1.0 / 60.0) {
                        shell.draw();
                    }
                }
                shell.draw();
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tone}-{tag}.png")), 3);
                sheets[n].push(shell.gfx.canvas.clone());
            }
        }
    }
    // Each screen's six (dark above light; compact, standard, large).
    for ((name, _), shots) in screens.iter().zip(&sheets) {
        let (w, h) = (W as i32, H as i32);
        let gap = 6;
        let mut sheet = bezel::Canvas::new(3 * w + 2 * gap, 2 * h + gap);
        sheet.px.fill(0x0080_8080);
        for (i, shot) in shots.iter().enumerate() {
            let (ox, oy) = ((i % 3) as i32 * (w + gap), (i / 3) as i32 * (h + gap));
            for y in 0..h {
                let from = (y * shot.w) as usize;
                let to = ((oy + y) * sheet.w + ox) as usize;
                sheet.px[to..to + w as usize].copy_from_slice(&shot.px[from..from + w as usize]);
            }
        }
        crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{name}.png")), 2);
    }
    // Every look on the sample scene, named, five across.
    let (cw, ch) = (PREVIEW_W as i32, PREVIEW_H as i32);
    let label = 22;
    let mut sheet = bezel::Canvas::new(5 * cw, 3 * (ch + label));
    sheet.px.fill(0x0010_1214);
    let mut shell = Shell::new(crate::robot::Robot::demo());
    shell.apply_settings();
    for (i, look) in LOOKS.iter().enumerate() {
        let pic = developed(&base_yuv, *look, 32.0, false);
        let (ox, oy) = ((i % 5) as i32 * cw, (i / 5) as i32 * (ch + label));
        for y in 0..ch {
            for x in 0..cw {
                let p = &pic.rgb[((y * cw + x) * 3) as usize..];
                sheet.px[((oy + y) * sheet.w + ox + x) as usize] = (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]);
            }
        }
        let g = &mut shell.gfx;
        std::mem::swap(&mut g.canvas, &mut sheet);
        let ty = g.ty;
        g.text_at(ox as f32 + 8.0, (oy + ch) as f32 + label as f32 * 0.5, Align::Left, &look.name().to_lowercase(), ty.caption, Color(236, 238, 242));
        std::mem::swap(&mut g.canvas, &mut sheet);
    }
    crate::platform::sim::save_png(&sheet, &dir.join("looks-on-the-sample-scene.png"), 2);
    println!("camera 0.9.2 shots in {}", dir.display());
}

/// Doc mode as set for the shots (docscan's): the simulated page in the
/// preview, found by the finder itself, held still for `hold` seconds.
#[cfg(not(target_arch = "aarch64"))]
pub fn doc_camera(hold: f32, t: f32) -> Camera {
    let mut c = Camera::new();
    c.frozen = true;
    c.flash.jump(0.0);
    c.last = None;
    c.last_rx = None;
    c.set_mode(Mode::Doc);
    let mut pic = scene_part(PREVIEW_W, PREVIEW_H, t, false, [0.0, 0.0, 1.0, 1.0]);
    crate::docscan::sim::paint(&mut pic, [0.0, 0.0, 1.0, 1.0], t);
    sim_expose(&mut pic, 1.0, SIM_CT);
    let luma = brightness_of(&pic);
    if let (Some(f), Some(q)) = (&mut c.doc, crate::docscan::quad::find(&luma, PREVIEW_W, PREVIEW_H)) {
        f.place(q, hold);
        // The pages waiting, read back for the rail.
        let began = Instant::now();
        while f.doc.borrow().working() && began.elapsed() < Duration::from_secs(20) {
            f.doc.borrow_mut().poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    c.preview = Some(pic);
    c.note = None;
    c.mode_at.jump(c.mode_at.target());
    c
}

/// A picture's brightness (BT.601), as the camera's Y plane would be.
#[cfg(not(target_arch = "aarch64"))]
pub fn brightness_of(pic: &Picture) -> Vec<u8> {
    pic.rgb.chunks_exact(3).map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2])) >> 8) as u8).collect()
}

/// The kit's overlays and modes (0.9.4), each at the three text sizes in
/// both tones: grids, the level, the aspects, "hold still" with its
/// seconds, bracketing, the RGB histogram and the timing readout, false
/// colour, the intervalometer, the grey card, a preset, the self-timer.
#[cfg(not(target_arch = "aarch64"))]
pub fn kit_shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).ok();
    crate::smartcam::sim::choose(false);
    let t = 3.1;
    let sample = scene(PREVIEW_W, PREVIEW_H, t, false);
    let yuv = rgb_to_yuv420(&sample);
    let developed = |false_colour: bool| {
        let mut out = Picture::default();
        Look::Original.compiled(0.0).develop(&yuv, PREVIEW_W, PREVIEW_H, &mut out, None);
        if false_colour {
            camkit::paint_false_colour(&yuv[..PREVIEW_W * PREVIEW_H], &mut out.rgb);
        }
        out
    };
    let (plain, coloured) = (developed(false), developed(true));
    let hist = pro::Histogram::of_luma(&yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2);
    let rgb = camkit::RgbHistogram::of_yuv420(&yuv, PREVIEW_W, PREVIEW_H, 2);
    let metered = Metered { exposure_us: 16_667, analogue: 2.0, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom: 0.0 };
    // A photo under way: asked `asked` ago, the sensor exposing for `ms` since `since` ago.
    let exposing = |what: camkit::Exposing, ms: u64, since: u64| {
        let now = Instant::now();
        let mut tl = Timeline::new(PathBuf::from("IMG_shot.jpg"), now - Duration::from_millis(since + 200));
        tl.mark(Mark::Request, now - Duration::from_millis(since + 190));
        tl.exposing = Some((what, Duration::from_millis(ms), 1));
        tl.mark(Mark::Exposing, now - Duration::from_millis(since));
        tl
    };
    type Setup = Box<dyn Fn(&mut Camera)>;
    let level = |roll: f32, pitch: f32, flat: bool| Some(camkit::Horizon { roll, pitch, flat });
    let screens: Vec<(&str, Setup)> = vec![
        ("01-grid-thirds-level", Box::new(move |c: &mut Camera| {
            c.kit.grid = Grid::Thirds;
            c.kit.level = true;
            c.kit.horizon = level(2.4, 0.0, false);
        })),
        ("02-grid-squares-level-true", Box::new(move |c: &mut Camera| {
            c.kit.grid = Grid::Square;
            c.kit.level = true;
            c.kit.horizon = level(0.2, 0.0, false);
        })),
        ("03-aspect-16x9", Box::new(move |c: &mut Camera| {
            c.kit.aspect = Aspect::SixteenNine;
            c.kit.grid = Grid::Thirds;
        })),
        ("04-aspect-1x1-flat-level", Box::new(move |c: &mut Camera| {
            c.kit.aspect = Aspect::Square;
            c.kit.level = true;
            c.kit.horizon = level(4.0, -3.0, true);
        })),
        ("05-hold-still-long-exposure", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            c.pro = Pro { shutter: Some(pro::nearest_shutter(2_000_000)), iso: Some(0), ..Pro::default() };
            c.kit.shot = Some(exposing(camkit::Exposing::Still, 4_200, 2_600));
            c.saving = Some(Instant::now());
            c.note = None;
        })),
        ("06-detail-hold-still", Box::new(move |c: &mut Camera| {
            c.kit.shot = Some(exposing(camkit::Exposing::Detail, 1_800, 600));
            c.saving = Some(Instant::now());
            c.note = None;
        })),
        ("07-bracketing-set", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            c.pro = Pro { shutter: Some(pro::nearest_shutter(4_000)), ..Pro::default() };
            c.kit.bracket = Some(camkit::Bracket { frames: 5, stops: 1 });
            c.kit.shot = Some(exposing(camkit::Exposing::Bracket, 2_000, 700));
            c.saving = Some(Instant::now());
            c.note = None;
        })),
        ("08-rgb-histogram-readout", {
            let rgb = rgb.clone();
            Box::new(move |c: &mut Camera| {
                c.set_mode(Mode::Pro);
                c.kit.hist = HistMode::Rgb;
                c.kit.rgb = rgb.clone();
                c.kit.readout = true;
                let t0 = Instant::now() - Duration::from_secs(3);
                let mut tl = Timeline::new(PathBuf::from("IMG_last.jpg"), t0);
                tl.mark(Mark::Request, t0 + Duration::from_millis(2));
                tl.mark(Mark::Captured, t0 + Duration::from_millis(420));
                tl.mark(Mark::Written, t0 + Duration::from_millis(1120));
                c.kit.last_shot = Some(tl);
                c.note = None;
            })
        }),
        ("09-false-colour", {
            let coloured = coloured.clone();
            Box::new(move |c: &mut Camera| {
                c.set_mode(Mode::Pro);
                c.zebras = false;
                c.kit.false_colour = true;
                c.preview = Some(coloured.clone());
                c.note = Some(("false colour".into(), 1.0));
            })
        }),
        ("10-intervalometer-panel", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            c.kit.panel = Some(0);
            c.kit.interval = camkit::Interval { every_s: 10, count: 50 };
            c.note = Some(("enter: start".into(), 1.0));
        })),
        ("11-intervalometer-running", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            c.kit.run = Some(camkit::Run { interval: camkit::Interval { every_s: 10, count: 50 }, taken: 3, next: Instant::now() + Duration::from_millis(6_600) });
            c.note = None;
        })),
        ("12-grey-card", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            c.pro.wb = pro::Wb::grey([1.92, 1.55]);
            c.set_dial(Some(Dial::Wb));
            c.note = Some(("wb from the grey card".into(), 1.0));
        })),
        ("13-preset-recalled", Box::new(move |c: &mut Camera| {
            c.set_mode(Mode::Pro);
            let s = camkit::Setup { pro: Pro { shutter: Some(pro::nearest_shutter(4_000)), ..Pro::default() }, aspect: Aspect::ThreeTwo, bracket: Some(camkit::Bracket { frames: 3, stops: 1 }), grid: Grid::Thirds, ..camkit::Setup::default() };
            c.pro = s.pro;
            c.kit.aspect = s.aspect;
            c.kit.bracket = s.bracket;
            c.kit.grid = s.grid;
            c.note = Some(("c2 recalled".into(), 1.0));
        })),
        ("14-self-timer", Box::new(move |c: &mut Camera| {
            c.timer = 2;
            c.countdown = Some(Instant::now() + Duration::from_millis(1_600));
            c.note = None;
        })),
    ];
    let mut sheets: Vec<Vec<bezel::Canvas>> = (0..screens.len()).map(|_| Vec::new()).collect();
    for light in [false, true] {
        for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
            let tone = if light { "light" } else { "dark" };
            for (n, (name, setup)) in screens.iter().enumerate() {
                let mut camera = Camera::new();
                camera.frozen = true;
                camera.flash.jump(0.0);
                (camera.look, camera.hue, camera.raw, camera.zebras, camera.pro) = (Look::Original, 32.0, false, true, Pro::default());
                camera.kit = Kit::default();
                camera.last = None;
                camera.last_rx = None;
                camera.preview = Some(plain.clone());
                camera.histogram = hist.clone();
                camera.metered = Some(metered);
                camera.note = None;
                let mut shell = Shell::new(crate::robot::Robot::demo());
                (shell.settings.text, shell.settings.light) = (size, light);
                shell.apply_settings();
                setup(&mut camera);
                for a in [&mut camera.mode_at, &mut camera.dial_at, &mut camera.dial_fade, &mut camera.strip_at, &mut camera.strip_fade, &mut camera.focus_fade] {
                    a.jump(a.target());
                }
                shell.show(Box::new(camera));
                for _ in 0..90 {
                    if shell.tick(1.0 / 60.0) {
                        shell.draw();
                    }
                }
                shell.draw();
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tone}-{tag}.png")), 3);
                sheets[n].push(shell.gfx.canvas.clone());
            }
        }
    }
    for ((name, _), shots) in screens.iter().zip(&sheets) {
        let (w, h) = (W as i32, H as i32);
        let gap = 6;
        let mut sheet = bezel::Canvas::new(3 * w + 2 * gap, 2 * h + gap);
        sheet.px.fill(0x0080_8080);
        for (i, shot) in shots.iter().enumerate() {
            let (ox, oy) = ((i % 3) as i32 * (w + gap), (i / 3) as i32 * (h + gap));
            for y in 0..h {
                let from = (y * shot.w) as usize;
                let to = ((oy + y) * sheet.w + ox) as usize;
                sheet.px[to..to + w as usize].copy_from_slice(&shot.px[from..from + w as usize]);
            }
        }
        crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{name}.png")), 2);
    }
    println!("camera 0.9.4 shots in {}", dir.display());
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
    /// shell would tick the screen: its motion, then what arrived.
    fn run(camera: &mut Camera, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        let robot = Robot::none();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(16));
            camera.tick(1.0 / 60.0);
            camera.ambient_tick(1.0 / 60.0, &robot);
        }
    }

    /// Ticked and updated as the shell does (the timer, the intervalometer
    /// and a photo given up on are seen to in `update`).
    fn run_updating(camera: &mut Camera, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(16));
            camera.tick(1.0 / 60.0);
            camera.ambient_tick(1.0 / 60.0, &robot);
            camera.update(&mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        }
    }

    #[test]
    fn a_code_read_is_offered_put_away_and_not_offered_again_while_in_sight() {
        crate::shell::tests::sandbox();
        // The handheld's own reader: no smart camera on the port.
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = Camera::new();
        run(&mut camera, 300);
        assert!(matches!(press(&mut camera, Key::Left), Nav::Stay));
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
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Left);
        let started = Instant::now();
        while camera.offered().is_none() && started.elapsed() < Duration::from_secs(8) {
            run(&mut camera, 50);
        }
        assert!(camera.offered().is_some(), "offered again after scanning anew");
        assert!(matches!(press(&mut camera, Key::Enter), Nav::Push(_)));
        // With nothing to put away, Esc leaves.
        press(&mut camera, Key::Right);
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
        assert!(camera.control.send(Order::Still(path.clone(), Vec::new())));
        let mut saved = None;
        // What the camera says, in the order it says it: the sensor
        // exposing, done, the picture in, then written.
        let mut said = Vec::new();
        while saved.is_none() && started.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(30));
            saved = camera.live.lock().unwrap().events.drain(..).find_map(|e| match e {
                Event::Exposing(line) => {
                    said.push(if line.ends_with(" 1 still") { "exposing" } else { "exposing?" });
                    None
                }
                Event::Exposed => {
                    said.push("exposed");
                    None
                }
                Event::Captured(marks) => {
                    said.push(if marks.contains("captured") { "captured" } else { "captured?" });
                    None
                }
                Event::Saved { path, size, detail, .. } => Some((path, size, detail)),
                Event::Failed(why) => panic!("{why}"),
            });
        }
        let (got, size, detail) = saved.expect("the photo is taken");
        assert_eq!(said, ["exposing", "exposed", "captured"], "its picture was in before it was written");
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
        press(&mut camera, Key::Left);
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

    /// A camera with its first metered frames in.
    fn ready() -> Camera {
        // From the Camera's own defaults: what another test kept is not this one's.
        Prefs::default().save();
        let mut camera = Camera::new();
        let started = Instant::now();
        while (camera.metered.is_none() || camera.preview.is_none()) && started.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 40);
        }
        assert!(camera.metered.is_some(), "metered frames");
        camera
    }

    #[test]
    fn the_shutter_answers_with_the_preview_at_once_then_the_full_photo_is_saved() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        let before = camera.last.as_ref().map(|(p, _)| p.path.clone());
        assert!(matches!(press(&mut camera, Key::Enter), Nav::Stay));
        // At once, before the camera has even taken it: the thumbnail is the
        // new photo's, made from the preview, and flying into the corner.
        let path = camera.last.as_ref().map(|(p, _)| p.path.clone()).expect("an instant thumbnail");
        assert_ne!(Some(&path), before.as_ref());
        assert_eq!(camera.instant.as_ref(), Some(&path));
        assert!(camera.flying.is_some(), "the shot flies into the corner");
        assert!(!path.exists() && !camera.captured && camera.pending == 1, "nothing written yet");
        // Then the camera has its picture (the next could be taken), then
        // the full-size photo is saved: in that order.
        let started = Instant::now();
        let (mut captured_at, mut saved_at) = (None, None);
        while saved_at.is_none() && started.elapsed() < Duration::from_secs(30) {
            run(&mut camera, 20);
            if camera.captured && captured_at.is_none() {
                captured_at = Some(started.elapsed());
            }
            if camera.pending == 0 {
                saved_at = Some(started.elapsed());
            }
        }
        let (captured_at, saved_at) = (captured_at.expect("captured"), saved_at.expect("saved"));
        assert!(captured_at <= saved_at, "{captured_at:?} then {saved_at:?}");
        assert_eq!(photos::dimensions(&path), Some(FULL));
        // Its thumbnail is still the same photo's, now from the file.
        assert_eq!(camera.last.as_ref().map(|(p, _)| p.path.clone()), Some(path.clone()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_pro_photo_takes_the_dials_and_keeps_its_look_its_original_and_its_raw() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        // Pro mode, to the right of photo: 1/250 s by hand, RAW on, Vivid.
        press(&mut camera, Key::Right);
        assert_eq!(camera.mode, Mode::Pro);
        press(&mut camera, Key::Char('s'));
        assert_eq!(camera.dial, Some(Dial::Shutter));
        camera.pro.shutter = Some(pro::nearest_shutter(4_000));
        camera.pro_changed();
        press(&mut camera, Key::Char('r'));
        assert!(camera.raw);
        press(&mut camera, Key::Back);
        assert_eq!(camera.dial, None, "esc puts the dial away first");
        press(&mut camera, Key::Char('f'));
        assert!(camera.strip);
        press(&mut camera, Key::Right);
        assert_eq!(camera.look, Look::Vivid);
        press(&mut camera, Key::Back);
        assert!(!camera.strip && camera.mode == Mode::Pro, "esc closes the strip, not the camera");
        // The preview runs at the dial's time (the simulated camera says so).
        let started = Instant::now();
        while camera.metered.map(|m| m.exposure_us) != Some(4_000) && started.elapsed() < Duration::from_secs(3) {
            run(&mut camera, 40);
        }
        assert_eq!(camera.metered.map(|m| m.exposure_us), Some(4_000));
        let (words, burst, _) = camera.still_words(std::path::Path::new("/x/IMG_1.jpg"), crate::burst::DetailMode::Auto);
        assert!(words.iter().any(|w| w == "exp=4000") && words.iter().any(|w| w.starts_with("look=")) && words.iter().any(|w| w == "raw=/x/IMG_1.dng"), "{words:?}");
        assert_eq!(burst, None, "a RAW photo takes no burst");
        press(&mut camera, Key::Enter);
        let path = camera.pending_paths.first().cloned().expect("taken");
        let started = Instant::now();
        while camera.pending > 0 && started.elapsed() < Duration::from_secs(40) {
            run(&mut camera, 30);
        }
        // The photo wears its look (said in its Exif), the original is kept,
        // and the DNG beside it is the sensor's size.
        assert_eq!(crate::filters::look_of(&path).as_deref(), Some("Vivid"));
        let original = crate::filters::original_of(&path);
        assert_eq!(photos::dimensions(&original), Some(FULL));
        let dng = std::fs::read(path.with_extension("dng")).expect("a DNG");
        assert_eq!(&dng[..4], b"II*\0");
        assert!(dng.len() > FULL.0 * FULL.1 * 2);
        // Deleting the photo takes what was kept beside it too.
        let photo = photos::list().into_iter().find(|p| p.path == path).expect("in the album");
        photos::delete(&photo).unwrap();
        assert!(!original.exists() && !path.with_extension("dng").exists());
        // The dials are kept for the next visit.
        assert_eq!(Prefs::load().pro.shutter, Some(pro::nearest_shutter(4_000)));
        assert!(Prefs::load().raw);
    }

    #[test]
    fn the_focus_check_shows_the_photos_own_pixels_and_moves_with_the_arrows() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        press(&mut camera, Key::Char('z'));
        assert_eq!(camera.focus, Some((0.5, 0.5)));
        let started = Instant::now();
        while !camera.cropped() && started.elapsed() < Duration::from_secs(3) {
            run(&mut camera, 40);
        }
        assert!(camera.cropped(), "the crop's frames arrive");
        press(&mut camera, Key::Right);
        let (x, _) = camera.focus.unwrap();
        assert!(x > 0.5 && x < 0.6, "{x}");
        assert_eq!(camera.mode, Mode::Photo, "the arrows look around, not change mode");
        press(&mut camera, Key::Back);
        assert_eq!(camera.focus, None);
        // Its crop never reaches past the picture's edge.
        let v = focus_view(0.0, 1.0);
        assert!(v[0] >= 0.0 && v[3] <= 1.0 + 1e-6 && ((v[2] - v[0]) * FULL.0 as f32 - PREVIEW_W as f32).abs() < 0.5);
    }

    #[test]
    fn developing_a_frame_lays_the_look_and_zebras_over_it_and_counts_the_histogram() {
        let live = Mutex::new(Live::default());
        let mut pic = scene(PREVIEW_W, PREVIEW_H, 2.0, false);
        // Overexposed four times: the sky clips.
        sim_expose(&mut pic, 4.0, SIM_CT);
        let yuv = rgb_to_yuv420(&pic);
        let develop = Mutex::new(Develop { look: Arc::new(Look::Mono.compiled(0.0)), zebras: true, ..Develop::default() });
        let mut out = Picture::default();
        develop_frame(&live, &develop, &yuv, PREVIEW_W, PREVIEW_H, &mut out, None);
        let l = live.lock().unwrap();
        let shown = l.preview.as_ref().unwrap();
        assert!(shown.rgb.chunks_exact(3).all(|p| p[0] == p[1] && p[1] == p[2]), "mono");
        assert!(l.histogram.clipped > 0.05, "{}", l.histogram.clipped);
        assert!(shown.rgb.chunks_exact(3).any(|p| p == [36, 36, 36]), "zebra stripes");
        assert_eq!(l.histogram.bins.iter().sum::<u32>() as usize, PREVIEW_W * PREVIEW_H / 4);
        assert_eq!(l.yuv.len(), yuv.len(), "the frame kept for the shutter");
    }

    /// Waits (ticking as the shell does) until nothing is being saved.
    fn until_saved(camera: &mut Camera) {
        let started = Instant::now();
        while (camera.pending > 0 || camera.saving.is_some()) && started.elapsed() < Duration::from_secs(60) {
            run_updating(camera, 30);
        }
        assert_eq!(camera.pending, 0, "saved");
    }

    #[test]
    fn a_photo_is_timed_along_the_still_path_and_hold_still_is_only_for_a_long_exposure() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        // A metered photo at a hand-held shutter: never a "hold still".
        assert!(matches!(press(&mut camera, Key::Enter), Nav::Stay));
        let shot = camera.kit.shot.as_ref().expect("timed from the key");
        assert_eq!(shot.reached(), Mark::Request);
        let mut held = false;
        let started = Instant::now();
        while (camera.pending > 0 || camera.kit.last_shot.is_none()) && started.elapsed() < Duration::from_secs(30) {
            run_updating(&mut camera, 20);
            held |= camera.kit.shot.as_ref().is_some_and(|t| t.hold_still(Instant::now()).is_some());
        }
        assert!(!held, "a short exposure says nothing to hold still for");
        let last = camera.kit.last_shot.clone().expect("the last shot, timed");
        let order: Vec<Mark> = last.since_key().iter().map(|(m, _)| *m).collect();
        assert_eq!(order, [Mark::Key, Mark::Request, Mark::Exposing, Mark::Exposed, Mark::Captured, Mark::Written]);
        assert!(last.capture_s().unwrap() <= last.since_key().last().unwrap().1);
        assert!(last.readout()[0].starts_with("last shot: ") && last.readout()[0].contains(" capture · "), "{:?}", last.readout());
        assert!(last.camera.contains("captured"), "the camera program's marks kept: {}", last.camera);
        std::fs::remove_file(&last.path).ok();
        // The readout on pro mode's t; the self-timer on c (and t in photo mode).
        press(&mut camera, Key::Char('c'));
        assert_eq!(camera.timer, 2);
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Char('t'));
        assert!(camera.kit.readout && camera.timer == 2, "t in pro mode is the readout");
        // A one-second exposure by hand: "hold still" while it is taken, its seconds counting down.
        camera.pro.shutter = Some(pro::SHUTTERS.iter().position(|s| s.us == 1_000_000).unwrap());
        camera.timer = 0;
        press(&mut camera, Key::Enter);
        let started = Instant::now();
        let mut left = None;
        while left.is_none() && started.elapsed() < Duration::from_secs(20) {
            run_updating(&mut camera, 10);
            left = camera.kit.shot.as_ref().and_then(|t| t.hold_still(Instant::now())).map(|(_, s)| s);
        }
        assert!(left.is_some_and(|s| s > 0.5 && s <= 2.2), "{left:?}");
        until_saved(&mut camera);
        if let Some(t) = &camera.kit.last_shot {
            std::fs::remove_file(&t.path).ok();
        }
    }

    #[test]
    fn a_photo_is_cropped_to_its_aspect_and_a_bracketing_set_is_saved_beside_it() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        // 3:2, on o (twice from 4:3 is 16:9; once is 3:2).
        press(&mut camera, Key::Char('o'));
        assert_eq!(camera.kit.aspect, Aspect::ThreeTwo);
        press(&mut camera, Key::Enter);
        let path = camera.instant.clone().expect("a photo");
        until_saved(&mut camera);
        assert_eq!(photos::dimensions(&path), Some((3280, 2188)));
        std::fs::remove_file(&path).ok();
        // Pro mode, bracketing 3 at ±1: three photos, a stop apart, named as a set.
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Char('b'));
        assert_eq!(camera.kit.bracket, Some(camkit::Bracket { frames: 3, stops: 1 }));
        let (words, burst, set) = camera.still_words(std::path::Path::new("/p/IMG_9.jpg"), crate::burst::DetailMode::Auto);
        assert_eq!(burst, None, "a set is never a burst");
        assert_eq!(set, [PathBuf::from("/p/IMG_9_ev-1.jpg"), PathBuf::from("/p/IMG_9_ev+1.jpg")]);
        let exp = |w: &str| still_word(&[w.to_owned()], "also").and_then(|v| v.split(',').next()?.parse::<f32>().ok());
        let base = still_word(&words, "exp").and_then(|v| v.parse::<f32>().ok()).unwrap();
        let alsos: Vec<f32> = words.iter().filter_map(|w| exp(w)).collect();
        assert_eq!(alsos.len(), 2);
        assert!((alsos[0] / base - 0.5).abs() < 0.01 && (alsos[1] / base - 2.0).abs() < 0.01, "{base} {alsos:?}");
        press(&mut camera, Key::Enter);
        let path = camera.instant.clone().expect("a photo");
        assert_eq!(camera.pending, 3, "the set's three");
        until_saved(&mut camera);
        for p in [path.clone(), camkit::set_path(&path, -1), camkit::set_path(&path, 1)] {
            assert_eq!(photos::dimensions(&p), Some((3280, 2188)), "{}", p.display());
            std::fs::remove_file(&p).ok();
        }
    }

    #[test]
    fn presets_keep_and_recall_the_dials_and_the_kit() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Char('m'));
        assert_eq!(Program::of(&camera.pro), Program::Shutter, "shutter priority");
        press(&mut camera, Key::Char('g'));
        press(&mut camera, Key::Char('b'));
        press(&mut camera, Key::Char('!'));
        let kept = camera.setup();
        assert_eq!(camera.kit.presets[0].as_ref(), Some(&kept));
        // Changed, then C1 recalled: all of it back.
        press(&mut camera, Key::Char('m'));
        press(&mut camera, Key::Char('g'));
        press(&mut camera, Key::Char('b'));
        assert_ne!(camera.setup(), kept);
        press(&mut camera, Key::Char('1'));
        assert_eq!(camera.setup(), kept);
        // And kept with the Camera's file.
        assert_eq!(Prefs::load().kit.presets[0].as_ref(), Some(&kept));
        press(&mut camera, Key::Char('2'));
        assert!(camera.note.as_ref().is_some_and(|(n, _)| n.contains("empty")));
    }

    #[test]
    fn the_intervalometer_dims_the_screen_and_keeps_shooting_until_its_count() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        assert!(!camera.busy() && !camera.holds_dim());
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Char('v'));
        assert_eq!(camera.kit.panel, Some(0));
        // Every 2 s (the least), 2 photos.
        for _ in 0..12 {
            press(&mut camera, Key::Left);
        }
        press(&mut camera, Key::Down);
        for _ in 0..12 {
            press(&mut camera, Key::Left);
        }
        assert_eq!(camera.kit.interval, camkit::Interval { every_s: 2, count: 10 });
        press(&mut camera, Key::Enter);
        assert!(camera.kit.run.is_some() && camera.holds_dim(), "running: the screen dims, never off");
        camera.kit.run.as_mut().unwrap().interval.count = 2;
        // A stray key does nothing while it runs.
        press(&mut camera, Key::Char('g'));
        let started = Instant::now();
        while camera.kit.run.is_some() && started.elapsed() < Duration::from_secs(30) {
            run_updating(&mut camera, 30);
        }
        assert!(camera.kit.run.is_none() && !camera.holds_dim(), "done after its count");
        until_saved(&mut camera);
        assert!(camera.note.as_ref().is_some_and(|(n, _)| n.contains("2 taken")), "{:?}", camera.note);
    }

    #[test]
    fn waiting_never_holds_the_frame_beat() {
        crate::shell::tests::sandbox();
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready();
        camera.note = None;
        assert!(!camera.busy());
        // At rest, only the saving dots or nothing at all; the frames wake the loop themselves.
        assert_eq!(camera.ambient(), None);
        camera.countdown = Some(Instant::now() + Duration::from_millis(9500));
        let wait = camera.ambient().expect("the timer looks in");
        assert!(wait <= Duration::from_secs(1));
        assert!(!camera.busy(), "and never by the frame beat");
        camera.countdown = None;
        // The level looks in 15 times a second while it shows.
        press(&mut camera, Key::Char('h'));
        camera.note = None;
        assert!(camera.kit.level && camera.ambient() == Some(Duration::from_millis(66)));
        run(&mut camera, 100);
        assert!(camera.kit.horizon.is_some(), "read from the motion sensor");
        press(&mut camera, Key::Char('h'));
        assert!(camera.kit.imu.is_none(), "the sensor let go");
    }
}
