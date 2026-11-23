//! The smart camera on the Grove port: an M5Stack UnitV (the K210 M12,
//! U078-V-M12), which runs `tools/unitv/main.py` (MaixPy) and does the
//! looking the handheld would otherwise spend its own processor on: faces
//! (on its KPU), QR codes, AprilTags with their pose, colour blobs. What it
//! finds arrives as small results; on request a small JPEG of what it sees
//! (QVGA), for the Camera's AI view.
//!
//! **The port.** Only when Settings › Camera › Grove port says Smart camera
//! (catalyst-grove), which takes it at once: the login stopped and any
//! session on the port hung up, the kernel's console quieted on it (and off
//! it from the next start's first line). The camera is powered only while
//! `catalyst-grove ready` says all of that holds. [`switch_grove`] runs it
//! off the screen's thread, and gives the port back first when it goes to
//! the console.
//!
//! **Power.** The port's 5 V is on only while something holds a [`Lease`]
//! (the Camera, for its face-aware exposure, the AI view and scan mode; the
//! field view for tags), then kept warm [`WARM_FOR`] so coming straight back
//! is instant, then off. A K210 starts in about a second and a half: the
//! time from power to its first answer is measured and shown ("waking the
//! smart camera…").
//!
//! **The link.** Frames (see [`frame`]): a sync word, the kind, a sequence
//! number, the length with a check of its own, the payload, and a CRC-32
//! over all of it; junk between frames (a camera starting up, a bit lost) is
//! skipped to the next sync word. It starts at 115 200 baud and then asks
//! for the fastest both ends hold (3 Mbaud is the PL011's limit at its
//! default clock; the K210's UART runs from a 195 MHz clock with a
//! fractional divider), stepping down whenever a rate fails to answer or
//! garbles too many frames.
//!
//! **The field view.** [`tags`] is the AprilTags the camera saw last, with
//! their id, where in its picture, and their pose (millimetres, from its
//! lens), for the field view to place the handheld with.
//!
//! In the simulator a stand-in for the camera ([`sim`]) runs in the shell
//! itself, behind the same bytes, at the same baud rates' pace.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::photos::Picture;

/// Kept powered this long after the last lease goes (a K210 starts fast).
pub const WARM_FOR: Duration = Duration::from_secs(15);
/// No answer this long after power on: not there (or not set up).
pub const WAKE_TIMEOUT: Duration = Duration::from_secs(6);
/// After a camera that did not answer, tried again this long after.
pub const RETRY_AFTER: Duration = Duration::from_secs(10);
/// The rates tried, fastest first.
pub const BAUDS: [u32; 7] = [3_000_000, 2_000_000, 1_500_000, 1_000_000, 921_600, 460_800, 115_200];
pub const START_BAUD: u32 = 115_200;
/// The 5 V stays off at least this long before it comes back (my own
/// tuning: the UnitV's capacitors drained, the K210 reset).
pub const OFF_FOR: Duration = Duration::from_millis(500);

// ------------------------------------------------------------------ frames ---

/// The link's frames: what goes over the wire and back.
pub mod frame {
    /// Every frame begins with these two bytes.
    pub const SYNC: [u8; 2] = [0xC5, 0x7A];
    /// The longest payload (a preview is 5 to 12 kB).
    pub const MAX: usize = 32 * 1024;
    /// Sync, kind, sequence, length (2), the header's check; CRC-32 after.
    pub const HEAD: usize = 7;
    pub const TAIL: usize = 4;

    // Asked of the camera.
    pub const HELLO: u8 = 0x01;
    pub const BAUD: u8 = 0x02;
    pub const WANT: u8 = 0x03;
    pub const PING: u8 = 0x04;
    // Its answers, and what it sends by itself once asked.
    pub const HELLO_BACK: u8 = 0x81;
    pub const BAUD_BACK: u8 = 0x82;
    pub const WANT_BACK: u8 = 0x83;
    pub const PONG: u8 = 0x84;
    pub const RESULTS: u8 = 0x85;
    pub const PREVIEW: u8 = 0x86;
    #[allow(dead_code)]
    pub const LOG: u8 = 0x87;

    /// CRC-32 (IEEE 802.3, as zlib and MicroPython's binascii have it).
    pub fn crc32(bytes: &[u8]) -> u32 {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let mut t = [0u32; 256];
            for (i, v) in t.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
                *v = c;
            }
            t
        });
        !bytes.iter().fold(!0u32, |c, b| table[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8))
    }

    fn head_check(kind: u8, seq: u8, len: u16) -> u8 {
        kind.wrapping_add(seq).wrapping_add(len as u8).wrapping_add((len >> 8) as u8).wrapping_add(0x5A)
    }

    /// One frame's bytes.
    pub fn encode(kind: u8, seq: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len().min(MAX) as u16;
        let mut out = Vec::with_capacity(HEAD + payload.len() + TAIL);
        out.extend_from_slice(&SYNC);
        out.extend_from_slice(&[kind, seq, len as u8, (len >> 8) as u8, head_check(kind, seq, len)]);
        out.extend_from_slice(&payload[..usize::from(len)]);
        let crc = crc32(&out[2..]);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// A frame taken off the wire.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Frame {
        pub kind: u8,
        pub seq: u8,
        pub payload: Vec<u8>,
    }

    /// Bytes in, frames out: whatever arrives, however it is cut up. Junk
    /// is skipped to the next sync word; a frame whose header or CRC fails
    /// is counted and its first byte skipped, so a frame inside it is
    /// still found.
    #[derive(Default)]
    pub struct Decoder {
        held: Vec<u8>,
        /// Frames refused (bad header or CRC) and bytes skipped as junk.
        pub bad: u64,
        pub junk: u64,
        pub good: u64,
    }

    impl Decoder {
        pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
            self.held.extend_from_slice(bytes);
            let mut out = Vec::new();
            let mut at = 0;
            loop {
                // The next sync word.
                let Some(start) = self.held[at..].windows(2).position(|w| w == SYNC).map(|p| at + p) else {
                    // Keep a last byte that may be the sync's first half.
                    let keep = usize::from(self.held.last() == Some(&SYNC[0]));
                    self.junk += (self.held.len() - at - keep) as u64;
                    at = self.held.len() - keep;
                    break;
                };
                self.junk += (start - at) as u64;
                at = start;
                if self.held.len() - at < HEAD {
                    break;
                }
                let h = &self.held[at..at + HEAD];
                let (kind, seq, len) = (h[2], h[3], u16::from_le_bytes([h[4], h[5]]));
                if h[6] != head_check(kind, seq, len) || usize::from(len) > MAX {
                    self.bad += 1;
                    at += 1;
                    continue;
                }
                let whole = HEAD + usize::from(len) + TAIL;
                if self.held.len() - at < whole {
                    break;
                }
                let body = &self.held[at + 2..at + HEAD + usize::from(len)];
                let crc = u32::from_le_bytes(self.held[at + whole - TAIL..at + whole].try_into().unwrap_or([0; 4]));
                if crc32(body) != crc {
                    self.bad += 1;
                    at += 1;
                    continue;
                }
                out.push(Frame { kind, seq, payload: self.held[at + HEAD..at + HEAD + usize::from(len)].to_vec() });
                self.good += 1;
                at += whole;
            }
            self.held.drain(..at);
            out
        }
    }
}

// ----------------------------------------------------------------- results ---

/// A face the camera saw: its box in the camera's picture (0 to 1 each way)
/// and how sure it is (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub at: [f32; 4],
    pub score: f32,
}

/// A QR code: its box (0 to 1) and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub at: [f32; 4],
    pub text: String,
}

/// An AprilTag (36h11): its id, its centre in the camera's picture (0 to 1),
/// its turn (radians), and where it is from the camera's lens (millimetres:
/// x right, y down, z out), as MaixPy's `find_apriltags` gives it with the
/// UnitV M12's lens figures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tag {
    pub id: u16,
    pub centre: [f32; 2],
    pub turn: f32,
    pub pose_mm: [f32; 3],
}

/// A colour blob: which of the camera's colours (0 orange, 1 yellow,
/// 2 red, 3 blue), its box (0 to 1), its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub colour: u8,
    pub at: [f32; 4],
    pub pixels: u32,
}

/// What the camera found in one of its frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    pub frame: u32,
    pub faces: Vec<Face>,
    pub codes: Vec<Code>,
    pub tags: Vec<Tag>,
    pub blobs: Vec<Blob>,
    /// The camera's own pace, frames a second.
    pub fps: f32,
}

/// Results from the camera's JSON: `{"n":frame,"w":320,"h":240,"fps":..,
/// "f":[[x,y,w,h,score%]], "q":[[x,y,w,h,"text"]], "a":[[id,cx,cy,rot_mrad,
/// tx,ty,tz]], "b":[[colour,x,y,w,h,pixels]]}` (pixels of its picture,
/// millimetres): short keys, as it is sent many times a second.
pub fn parse_results(json: &[u8]) -> Option<Results> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let (w, h) = (v["w"].as_f64().unwrap_or(320.0) as f32, v["h"].as_f64().unwrap_or(240.0) as f32);
    let num = |a: &serde_json::Value, i: usize| a.get(i).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
    let boxed = |a: &serde_json::Value| [num(a, 0) / w, num(a, 1) / h, (num(a, 0) + num(a, 2)) / w, (num(a, 1) + num(a, 3)) / h];
    let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
    Some(Results {
        frame: v["n"].as_u64().unwrap_or(0) as u32,
        fps: v["fps"].as_f64().unwrap_or(0.0) as f32,
        faces: list("f").iter().map(|a| Face { at: boxed(a), score: num(a, 4) / 100.0 }).collect(),
        codes: list("q").iter().map(|a| Code { at: boxed(a), text: a.get(4).and_then(|t| t.as_str()).unwrap_or("").to_owned() }).collect(),
        tags: list("a")
            .iter()
            .map(|a| Tag { id: num(a, 0) as u16, centre: [num(a, 1) / w, num(a, 2) / h], turn: num(a, 3) / 1000.0, pose_mm: [num(a, 4), num(a, 5), num(a, 6)] })
            .collect(),
        blobs: list("b").iter().map(|a| Blob { colour: num(a, 0) as u8, at: [num(a, 1) / w, num(a, 2) / h, (num(a, 1) + num(a, 3)) / w, (num(a, 2) + num(a, 4)) / h], pixels: num(a, 5) as u32 }).collect(),
    })
}

// ------------------------------------------------------------------- power ---

/// The port's power over time, kept apart from the link so its timing can
/// be checked: on while wanted, warm a while after, off; and how long the
/// camera took to answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Power {
    Off,
    /// Powered at `since`, no answer yet.
    Waking { since: Instant },
    /// Answered `took` after power on.
    On { took: Duration },
    /// Nobody wants it now; off at `until` unless wanted again.
    Warm { took: Duration, until: Instant },
    /// No answer in time: off, tried again after `at` + RETRY_AFTER.
    Failed { at: Instant },
}

/// What the power wants done to the port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
    Keep,
}

impl Power {
    /// One step: `wanted` (a lease is held), `answered` (the camera spoke
    /// for the first time since power on).
    pub fn step(&mut self, now: Instant, wanted: bool, answered: bool) -> Switch {
        let (next, switch) = match self.clone() {
            Power::Off if wanted => (Power::Waking { since: now }, Switch::On),
            Power::Off => (Power::Off, Switch::Keep),
            Power::Waking { since } if answered => (if wanted { Power::On { took: now - since } } else { Power::Warm { took: now - since, until: now + WARM_FOR } }, Switch::Keep),
            Power::Waking { since } if now - since >= WAKE_TIMEOUT => (Power::Failed { at: now }, Switch::Off),
            Power::Waking { since } if !wanted && now - since >= WARM_FOR => (Power::Off, Switch::Off),
            w @ Power::Waking { .. } => (w, Switch::Keep),
            Power::On { took } if !wanted => (Power::Warm { took, until: now + WARM_FOR }, Switch::Keep),
            on @ Power::On { .. } => (on, Switch::Keep),
            Power::Warm { took, .. } if wanted => (Power::On { took }, Switch::Keep),
            Power::Warm { until, .. } if now >= until => (Power::Off, Switch::Off),
            warm @ Power::Warm { .. } => (warm, Switch::Keep),
            Power::Failed { at } if wanted && now - at >= RETRY_AFTER => (Power::Waking { since: now }, Switch::On),
            Power::Failed { .. } if !wanted => (Power::Off, Switch::Keep),
            failed @ Power::Failed { .. } => (failed, Switch::Keep),
        };
        *self = next;
        switch
    }

    pub fn powered(&self) -> bool {
        matches!(self, Power::Waking { .. } | Power::On { .. } | Power::Warm { .. })
    }

    /// Whether a step now, wanted, would power the port on.
    pub fn would_power(&self, now: Instant) -> bool {
        match self {
            Power::Off => true,
            Power::Failed { at } => now - *at >= RETRY_AFTER,
            _ => false,
        }
    }
}

// ----------------------------------------------------------------- status ---

/// How the smart camera stands, for the Camera and Settings.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// The Grove port is the console (Settings › Camera › Grove port).
    PortIsConsole,
    /// Chosen, but not free yet: a login or the kernel's console still on
    /// it (catalyst-grove ready says no), or the port changing hands.
    PortBusy,
    /// Not wanted: unpowered.
    Asleep,
    /// Powered, waiting for its first answer (since then).
    Waking { since: Instant },
    /// Answering, at `baud`, having taken `woke_ms` to start.
    Ready { baud: u32, woke_ms: u32 },
    /// Powered and never answered: why.
    Absent(String),
}

/// The link's figures: bytes and frames a second, and frames refused.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub bytes_per_s: f32,
    pub previews_per_s: f32,
    pub results_per_s: f32,
    pub refused: u64,
}

/// What the camera last sent.
#[derive(Clone, Default)]
pub struct Seen {
    pub results: Option<(Instant, Results)>,
    /// The newest preview, decoded, and how many have come.
    pub preview: Option<Picture>,
    pub previews: u64,
    pub tags: Option<(Instant, Vec<Tag>)>,
}

/// What a lease is for: what the camera is asked to look for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// The Camera's face-aware exposure.
    Faces,
    /// Scan mode: QR codes.
    Codes,
    /// The AI view: everything, and its picture.
    View,
    /// The field view: AprilTags.
    Tags,
}

struct Shared {
    leases: Vec<(u64, Use)>,
    next_lease: u64,
    status: Status,
    rates: Rates,
    seen: Seen,
    /// The camera's own description (from its HELLO).
    about: Option<String>,
}

struct Hub {
    shared: Mutex<Shared>,
    wake: Condvar,
}

fn hub() -> &'static Arc<Hub> {
    static HUB: std::sync::OnceLock<Arc<Hub>> = std::sync::OnceLock::new();
    HUB.get_or_init(|| {
        let hub = Arc::new(Hub {
            shared: Mutex::new(Shared { leases: Vec::new(), next_lease: 1, status: Status::Asleep, rates: Rates::default(), seen: Seen::default(), about: None }),
            wake: Condvar::new(),
        });
        let h = hub.clone();
        std::thread::Builder::new()
            .name("smartcam".into())
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Batch(5));
                link::run(&h)
            })
            .ok();
        hub
    })
}

/// Held while something uses the camera; dropped, it lets go.
pub struct Lease(u64);

impl Drop for Lease {
    fn drop(&mut self) {
        let hub = hub();
        if let Ok(mut s) = hub.shared.lock() {
            s.leases.retain(|(id, _)| *id != self.0);
            CHANGED.fetch_add(1, Ordering::AcqRel);
        }
        hub.wake.notify_all();
    }
}

/// Counted up, under the hub's lock, whenever what the link must act on
/// changes (a lease, the port changing hands): its waits end at the next
/// change even when the change lands just before it starts waiting.
static CHANGED: AtomicU64 = AtomicU64::new(0);

fn changed() {
    let _held = hub().shared.lock();
    CHANGED.fetch_add(1, Ordering::AcqRel);
    drop(_held);
    hub().wake.notify_all();
}

/// The camera, for `what`, while the lease is held.
pub fn lease(what: Use) -> Lease {
    let hub = hub();
    let id = {
        let mut s = hub.shared.lock().unwrap_or_else(|e| e.into_inner());
        let id = s.next_lease;
        s.next_lease += 1;
        s.leases.push((id, what));
        CHANGED.fetch_add(1, Ordering::AcqRel);
        id
    };
    hub.wake.notify_all();
    Lease(id)
}

/// Whether the Grove port is the smart camera's (as chosen).
pub fn chosen() -> bool {
    if cfg!(target_arch = "aarch64") {
        std::fs::read_to_string("/data/catalyst/grove-port").is_ok_and(|m| m.trim() == "camera")
    } else {
        sim::chosen()
    }
}

// ------------------------------------------------------------ the switch ---

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The port is changing hands: the link lets go of it and keeps off it.
static HOLD_OFF: AtomicBool = AtomicBool::new(false);
static SWITCHING: AtomicBool = AtomicBool::new(false);
/// The link has ttyAMA0 open.
static PORT_OPEN: AtomicBool = AtomicBool::new(false);
static SAID: Mutex<Option<String>> = Mutex::new(None);

/// The Grove port given to the smart camera, or back to the console, at
/// once, on a thread of its own (catalyst-grove waits on OpenRC, a hung-up
/// session and the boot partition). Going to the console, the link first
/// lets go of the port and cuts the camera's power, so the getty starts on
/// a port nobody else holds. The island says how it went.
pub fn switch_grove(camera: bool) {
    if SWITCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    HOLD_OFF.store(true, Ordering::Release);
    changed();
    crate::platform::wake();
    std::thread::Builder::new()
        .name("grove".into())
        .spawn(move || {
            // The link lets go (within its next turn: half a second at most).
            let until = Instant::now() + Duration::from_secs(3);
            while PORT_OPEN.load(Ordering::Acquire) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            let said = run_switch(camera);
            HOLD_OFF.store(false, Ordering::Release);
            SWITCHING.store(false, Ordering::Release);
            changed();
            *SAID.lock().unwrap_or_else(|e| e.into_inner()) = Some(said);
            crate::platform::wake();
        })
        .ok();
}

fn run_switch(camera: bool) -> String {
    if !cfg!(target_arch = "aarch64") {
        sim::choose(camera);
        return if camera { "Grove port: smart camera".into() } else { "Grove port: console · login on it again".into() };
    }
    match std::process::Command::new("catalyst-grove").arg(if camera { "camera" } else { "console" }).output() {
        Ok(out) if out.status.success() => match (camera, String::from_utf8_lossy(&out.stdout).trim()) {
            (true, "busy") => "Grove port: smart camera · the port is still busy".into(),
            (true, _) => "Grove port: smart camera".into(),
            (false, _) => "Grove port: console · login on it again".into(),
        },
        Ok(out) => {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            if why.is_empty() {
                "the Grove port could not be changed".into()
            } else {
                why
            }
        }
        Err(_) => "the Grove port is set on the handheld".into(),
    }
}

/// The port changing hands now.
pub fn switching() -> bool {
    SWITCHING.load(Ordering::Acquire)
}

/// What the island is to say of the switch, since the shell last asked.
pub fn take_said() -> Option<String> {
    SAID.lock().unwrap_or_else(|e| e.into_inner()).take()
}

pub fn status() -> Status {
    hub().shared.lock().map(|s| s.status.clone()).unwrap_or(Status::Asleep)
}

#[allow(dead_code)] // for diagnostics and the field view
pub fn rates() -> Rates {
    hub().shared.lock().map(|s| s.rates).unwrap_or_default()
}

#[allow(dead_code)] // the camera's own description, for diagnostics
pub fn about() -> Option<String> {
    hub().shared.lock().ok().and_then(|s| s.about.clone())
}

/// The newest results, if they are no older than `fresh`.
pub fn results(fresh: Duration) -> Option<Results> {
    let s = hub().shared.lock().ok()?;
    s.seen.results.as_ref().filter(|(at, _)| at.elapsed() <= fresh).map(|(_, r)| r.clone())
}

/// The newest preview, and how many have come (a new one when it changes).
pub fn preview() -> (u64, Option<Picture>) {
    hub().shared.lock().map(|s| (s.seen.previews, s.seen.preview.clone())).unwrap_or((0, None))
}

/// For the field view: the AprilTags the camera last saw, and when. Hold a
/// `lease(Use::Tags)` while looking, or nothing new arrives.
#[allow(dead_code)] // the field view's to use
pub fn tags() -> Option<(Instant, Vec<Tag>)> {
    hub().shared.lock().ok().and_then(|s| s.seen.tags.clone())
}

// ------------------------------------------------------------- alignment ---

/// How the smart camera's picture sits in the main camera's: a point at
/// (u, v) in its picture (0 to 1) is at (0.5 + (u − 0.5)·sx + ox, 0.5 +
/// (v − 0.5)·sy + oy) in the IMX219's. Side by side in a case, they look
/// the same way a few centimetres apart, so a scale (its 80° lens against
/// the IMX219's 62°) and an offset are all it takes; the Camera's alignment
/// step (`a` in the AI view) sets them by eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Align {
    pub sx: f32,
    pub sy: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Default for Align {
    /// From the lenses alone: the M12's 80° taken as diagonal (67.4° by
    /// 53.1° at 4:3) against the IMX219's 62.2° by 48.8°, as tan(½ its
    /// field) to tan(½ the IMX219's).
    fn default() -> Self {
        let t = |deg: f32| (deg.to_radians() * 0.5).tan();
        let diag = t(80.0);
        let (kh, kv) = ((diag * 0.8).atan() * 2.0, (diag * 0.6).atan() * 2.0);
        Align { sx: (kh * 0.5).tan() / t(62.2), sy: (kv * 0.5).tan() / t(48.8), ox: 0.0, oy: 0.0 }
    }
}

impl Align {
    pub fn map(&self, b: [f32; 4]) -> [f32; 4] {
        let u = |x: f32| 0.5 + (x - 0.5) * self.sx + self.ox;
        let v = |y: f32| 0.5 + (y - 0.5) * self.sy + self.oy;
        [u(b[0]), v(b[1]), u(b[2]), v(b[3])]
    }

    fn file() -> std::path::PathBuf {
        crate::photos::dir().parent().map(|d| d.join("catalyst/smartcam-align")).unwrap_or_else(|| "smartcam-align".into())
    }

    pub fn load() -> Align {
        let text = std::fs::read_to_string(Self::file()).unwrap_or_default();
        let v: Vec<f32> = text.split_whitespace().filter_map(|w| w.parse().ok()).collect();
        match v[..] {
            [sx, sy, ox, oy] if (0.3..3.0).contains(&sx) && (0.3..3.0).contains(&sy) => Align { sx, sy, ox, oy },
            _ => Align::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let file = Self::file();
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, format!("{:.4} {:.4} {:.4} {:.4}\n", self.sx, self.sy, self.ox, self.oy))
    }
}

/// The face the main camera's exposure should lean toward: the largest the
/// smart camera sees, in the IMX219's picture, if it falls inside it.
pub fn main_face(results: &Results, align: &Align) -> Option<[f32; 4]> {
    let best = results.faces.iter().filter(|f| f.score >= 0.5).max_by(|a, b| {
        let area = |f: &Face| (f.at[2] - f.at[0]) * (f.at[3] - f.at[1]);
        area(a).total_cmp(&area(b))
    })?;
    let m = align.map(best.at);
    let inside = m[2] > 0.02 && m[0] < 0.98 && m[3] > 0.02 && m[1] < 0.98;
    inside.then(|| [m[0].clamp(0.0, 1.0), m[1].clamp(0.0, 1.0), m[2].clamp(0.0, 1.0), m[3].clamp(0.0, 1.0)])
}

// -------------------------------------------------------------------- link ---

/// The port: on the handheld ttyAMA0 and catalyst-grove; in the simulator
/// the stand-in.
pub trait Port: Send {
    /// Whether the camera may be powered now (else why not).
    fn ready(&mut self) -> Result<(), Status>;
    fn power(&mut self, on: bool);
    fn set_baud(&mut self, baud: u32) -> bool;
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// What has arrived, waiting up to `wait` for something.
    fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize;
}

mod link {
    use super::*;

    /// What a lease set wants of the camera, as its WANT payload.
    fn wants(leases: &[(u64, Use)]) -> String {
        let has = |u: Use| leases.iter().any(|(_, w)| *w == u);
        let view = has(Use::View);
        format!(
            r#"{{"faces":{},"codes":{},"tags":{},"blobs":{},"preview":{}}}"#,
            u8::from(view || has(Use::Faces)),
            u8::from(view || has(Use::Codes)),
            u8::from(view || has(Use::Tags)),
            u8::from(view),
            u8::from(view)
        )
    }

    pub fn run(hub: &Hub) {
        let mut port: Box<dyn Port> = if cfg!(target_arch = "aarch64") { device::open() } else { Box::new(sim::port()) };
        let mut power = Power::Off;
        let mut decoder = frame::Decoder::default();
        let mut seq = 0u8;
        let mut baud = START_BAUD;
        // The camera answered since power on; the rate settled; what was last asked of it.
        let mut answered = false;
        let mut settled = false;
        let mut asked = String::new();
        let mut last_hello = Instant::now() - Duration::from_secs(1);
        let mut window = (Instant::now(), 0u64, 0u64, 0u64, 0u64);
        let mut buf = vec![0u8; 8192];
        let mut send = |port: &mut Box<dyn Port>, kind: u8, payload: &[u8]| {
            seq = seq.wrapping_add(1);
            port.write(&frame::encode(kind, seq, payload))
        };
        // When the 5 V last went off: it stays off a moment before it comes
        // back, so the K210 starts afresh (at 115 200) rather than riding a
        // short gap on its capacitors at the rate it had.
        let mut off_at: Option<Instant> = None;
        loop {
            let (leases, chosen, gen) = {
                let s = hub.shared.lock().unwrap_or_else(|e| e.into_inner());
                (s.leases.clone(), super::chosen() && !HOLD_OFF.load(Ordering::Acquire), CHANGED.load(Ordering::Acquire))
            };
            // Not the camera's (any more, or for now): unpowered and let go
            // of at once, no warm time.
            if !chosen && power.powered() {
                port.power(false);
                off_at = Some(Instant::now());
                power = Power::Off;
                (answered, settled) = (false, false);
            }
            let wanted = !leases.is_empty() && chosen;
            // Not to be powered (the port is the console, or not free yet:
            // a login or the console still on it): said so, and nothing more.
            if wanted && power.would_power(Instant::now()) {
                if let Err(why) = port.ready() {
                    set_status(hub, why);
                    wait(hub, Duration::from_secs(2), gen);
                    continue;
                }
            }
            match power.step(Instant::now(), wanted, answered && matches!(power, Power::Waking { .. })) {
                Switch::On => {
                    if let Some(rest) = off_at.and_then(|at| OFF_FOR.checked_sub(at.elapsed())) {
                        std::thread::sleep(rest);
                    }
                    port.power(true);
                    port.set_baud(START_BAUD);
                    baud = START_BAUD;
                    (answered, settled) = (false, false);
                    asked.clear();
                    decoder = frame::Decoder::default();
                }
                Switch::Off => {
                    port.power(false);
                    off_at = Some(Instant::now());
                    (answered, settled) = (false, false);
                }
                Switch::Keep => {}
            }
            set_status(
                hub,
                match &power {
                    Power::Off if HOLD_OFF.load(Ordering::Acquire) => Status::PortBusy,
                    Power::Off if !chosen => Status::PortIsConsole,
                    Power::Off => Status::Asleep,
                    Power::Waking { since } => Status::Waking { since: *since },
                    Power::On { took } | Power::Warm { took, .. } => Status::Ready { baud, woke_ms: took.as_millis() as u32 },
                    Power::Failed { .. } => Status::Absent("no answer from the smart camera on the Grove port".into()),
                },
            );
            if !power.powered() {
                wait(hub, if wanted { Duration::from_millis(500) } else { Duration::from_secs(30) }, gen);
                continue;
            }
            // Hello until it answers.
            if !answered && last_hello.elapsed() >= Duration::from_millis(250) {
                send(&mut port, frame::HELLO, br#"{"v":1}"#);
                last_hello = Instant::now();
            }
            let n = port.read(&mut buf, Duration::from_millis(40));
            for f in decoder.push(&buf[..n]) {
                window.1 += (f.payload.len() + frame::HEAD + frame::TAIL) as u64;
                match f.kind {
                    frame::HELLO_BACK => {
                        answered = true;
                        if let Ok(mut s) = hub.shared.lock() {
                            s.about = Some(String::from_utf8_lossy(&f.payload).into_owned());
                        }
                        if !settled {
                            baud = negotiate(&mut port, &mut decoder, &mut send, baud);
                            settled = true;
                        }
                    }
                    frame::RESULTS => {
                        window.3 += 1;
                        if let Some(r) = parse_results(&f.payload) {
                            if let Ok(mut s) = hub.shared.lock() {
                                if !r.tags.is_empty() {
                                    s.seen.tags = Some((Instant::now(), r.tags.clone()));
                                }
                                s.seen.results = Some((Instant::now(), r));
                            }
                            crate::platform::wake();
                        }
                    }
                    frame::PREVIEW => {
                        window.2 += 1;
                        if let Some(pic) = decode_jpeg(&f.payload) {
                            if let Ok(mut s) = hub.shared.lock() {
                                s.seen.preview = Some(pic);
                                s.seen.previews += 1;
                            }
                            crate::platform::wake();
                        }
                    }
                    _ => {}
                }
            }
            // Ask for what the leases want, once it answers (and again when that changes).
            let want = wants(&leases);
            if answered && settled && (want != asked) && wanted {
                send(&mut port, frame::WANT, want.as_bytes());
                asked = want;
            }
            if !wanted && answered && !asked.is_empty() {
                // Warm: it keeps quiet meanwhile.
                send(&mut port, frame::WANT, br#"{"faces":0,"codes":0,"tags":0,"blobs":0,"preview":0}"#);
                asked.clear();
            }
            // The figures, every second; too many frames refused at this
            // rate: one step slower.
            if window.0.elapsed() >= Duration::from_secs(1) {
                let secs = window.0.elapsed().as_secs_f32();
                let refused = decoder.bad.saturating_sub(window.4);
                if let Ok(mut s) = hub.shared.lock() {
                    s.rates = Rates { bytes_per_s: window.1 as f32 / secs, previews_per_s: window.2 as f32 / secs, results_per_s: window.3 as f32 / secs, refused: decoder.bad };
                }
                if refused >= 3 && settled && baud > START_BAUD {
                    let slower = BAUDS.iter().copied().find(|b| *b < baud).unwrap_or(START_BAUD);
                    baud = switch_to(&mut port, &mut decoder, &mut send, baud, slower).unwrap_or(START_BAUD);
                    asked.clear();
                }
                window = (Instant::now(), 0, 0, 0, decoder.bad);
            }
        }
    }

    fn set_status(hub: &Hub, status: Status) {
        if let Ok(mut s) = hub.shared.lock() {
            if s.status != status {
                s.status = status;
                drop(s);
                crate::platform::wake();
            }
        }
    }

    /// Up to `d`, or until something changed since `gen` was read.
    fn wait(hub: &Hub, d: Duration, gen: u64) {
        if let Ok(s) = hub.shared.lock() {
            hub.wake.wait_timeout_while(s, d, |_| CHANGED.load(Ordering::Acquire) == gen).ok();
        }
    }

    type Sender<'a> = dyn FnMut(&mut Box<dyn Port>, u8, &[u8]) -> bool + 'a;

    /// The fastest rate both ends hold: each tried from the top until one
    /// answers a ping, falling back to where it was.
    fn negotiate(port: &mut Box<dyn Port>, decoder: &mut frame::Decoder, send: &mut Sender, from: u32) -> u32 {
        for &rate in BAUDS.iter().filter(|b| **b > from) {
            if let Some(b) = switch_to(port, decoder, send, from, rate) {
                if b == rate {
                    return rate;
                }
            }
        }
        from
    }

    /// Ask the camera to move to `to` and follow it; a ping must come back
    /// at the new rate, or both go back to 115 200 (the camera does so by
    /// itself when no frame reaches it at the new rate). The rate in use.
    fn switch_to(port: &mut Box<dyn Port>, decoder: &mut frame::Decoder, send: &mut Sender, from: u32, to: u32) -> Option<u32> {
        send(port, frame::BAUD, &to.to_le_bytes());
        if !await_kind(port, decoder, frame::BAUD_BACK, Duration::from_millis(400)) {
            return None;
        }
        port.set_baud(to);
        std::thread::sleep(Duration::from_millis(30));
        for _ in 0..3 {
            send(port, frame::PING, b"catalyst");
            if await_kind(port, decoder, frame::PONG, Duration::from_millis(200)) {
                return Some(to);
            }
        }
        // Not held: back to the start rate, where the camera returns too.
        port.set_baud(START_BAUD);
        std::thread::sleep(Duration::from_millis(1200));
        let _ = from;
        Some(START_BAUD)
    }

    fn await_kind(port: &mut Box<dyn Port>, decoder: &mut frame::Decoder, kind: u8, within: Duration) -> bool {
        let until = Instant::now() + within;
        let mut buf = [0u8; 2048];
        while Instant::now() < until {
            let n = port.read(&mut buf, Duration::from_millis(20));
            if decoder.push(&buf[..n]).iter().any(|f| f.kind == kind) {
                return true;
            }
        }
        false
    }

    fn decode_jpeg(bytes: &[u8]) -> Option<Picture> {
        let mut d = jpeg_decoder::Decoder::new(bytes);
        let px = d.decode().ok()?;
        let info = d.info()?;
        let (w, h) = (usize::from(info.width), usize::from(info.height));
        let rgb = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => px,
            jpeg_decoder::PixelFormat::L8 => px.iter().flat_map(|v| [*v, *v, *v]).collect(),
            _ => return None,
        };
        (rgb.len() == w * h * 3).then_some(Picture { w, h, rgb })
    }
}

// ------------------------------------------------------------------ device ---

#[cfg(target_os = "linux")]
mod device {
    use super::*;

    /// ttyAMA0, raw, and the port's power through catalyst-grove (which
    /// refuses it unless the port is safely the camera's).
    pub struct Uart {
        fd: i32,
    }

    pub fn open() -> Box<dyn Port> {
        Box::new(Uart { fd: -1 })
    }

    fn speed(baud: u32) -> libc::speed_t {
        match baud {
            3_000_000 => libc::B3000000,
            2_000_000 => libc::B2000000,
            1_500_000 => libc::B1500000,
            1_000_000 => libc::B1000000,
            921_600 => libc::B921600,
            460_800 => libc::B460800,
            230_400 => libc::B230400,
            _ => libc::B115200,
        }
    }

    impl Uart {
        fn ensure_open(&mut self) -> bool {
            if self.fd >= 0 {
                return true;
            }
            // SAFETY: plain system calls on a descriptor this owns.
            unsafe {
                let fd = libc::open(c"/dev/ttyAMA0".as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC);
                if fd < 0 {
                    return false;
                }
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(fd, &mut t);
                libc::cfmakeraw(&mut t);
                t.c_cflag |= libc::CLOCAL | libc::CREAD;
                t.c_cflag &= !(libc::CRTSCTS | libc::CSTOPB | libc::PARENB);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::cfsetispeed(&mut t, libc::B115200);
                libc::cfsetospeed(&mut t, libc::B115200);
                libc::tcsetattr(fd, libc::TCSANOW, &t);
                libc::tcflush(fd, libc::TCIOFLUSH);
                self.fd = fd;
                PORT_OPEN.store(true, Ordering::Release);
            }
            true
        }
    }

    impl Port for Uart {
        fn ready(&mut self) -> Result<(), Status> {
            let ok = std::process::Command::new("catalyst-grove").arg("ready").status().is_ok_and(|s| s.success());
            if ok {
                return Ok(());
            }
            if super::chosen() {
                Err(Status::PortBusy)
            } else {
                Err(Status::PortIsConsole)
            }
        }

        /// On: the port opened raw first (nothing the camera sends is ever
        /// read as anything but bytes), then its 5 V, which catalyst-grove
        /// gives only while the port is safely the camera's. Off: the 5 V,
        /// then the port let go of.
        fn power(&mut self, on: bool) {
            if on {
                self.ensure_open();
            }
            let given = std::process::Command::new("catalyst-grove").args(["power", if on { "on" } else { "off" }]).status().is_ok_and(|s| s.success());
            if (!on || !given) && self.fd >= 0 {
                // SAFETY: the descriptor this opened, closed once.
                unsafe { libc::close(self.fd) };
                self.fd = -1;
                PORT_OPEN.store(false, Ordering::Release);
            }
        }

        fn set_baud(&mut self, baud: u32) -> bool {
            if !self.ensure_open() {
                return false;
            }
            // SAFETY: termios on a descriptor this owns.
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(self.fd, &mut t);
                libc::cfsetispeed(&mut t, speed(baud));
                libc::cfsetospeed(&mut t, speed(baud));
                libc::tcdrain(self.fd);
                libc::tcsetattr(self.fd, libc::TCSANOW, &t) == 0
            }
        }

        fn write(&mut self, bytes: &[u8]) -> bool {
            if !self.ensure_open() {
                return false;
            }
            let mut at = 0;
            while at < bytes.len() {
                // SAFETY: writing from a live slice to a descriptor this owns.
                let n = unsafe { libc::write(self.fd, bytes[at..].as_ptr().cast(), bytes.len() - at) };
                if n <= 0 {
                    let mut p = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: one pollfd.
                    if unsafe { libc::poll(&mut p, 1, 100) } <= 0 {
                        return false;
                    }
                    continue;
                }
                at += n as usize;
            }
            true
        }

        fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize {
            if self.fd < 0 {
                std::thread::sleep(wait);
                return 0;
            }
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd; reading into a live buffer.
            unsafe {
                if libc::poll(&mut p, 1, wait.as_millis() as i32) <= 0 {
                    return 0;
                }
                let n = libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len());
                n.max(0) as usize
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod device {
    pub fn open() -> Box<dyn super::Port> {
        Box::new(super::sim::port())
    }
}

// --------------------------------------------------------------------- sim ---

/// The simulator's smart camera: the UnitV and its program as the handheld
/// sees them over the wire. It starts about 1.4 s after power on, keeps
/// quiet until spoken to, answers the same frames, moves to the baud rates
/// asked (bytes then take as long as they would at that rate, so the
/// preview's rate is the real link's), and sees a pit: a face drifting
/// about, a QR code with a scouting note, AprilTag 7 on the field wall, an
/// orange game piece. Now and then it sends a burst of junk, as a camera
/// starting up does, which the link skips.
pub mod sim {
    use super::*;
    use std::sync::mpsc;

    static CHOSEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

    /// The simulator's Grove port setting (on unless a test says otherwise).
    pub fn chosen() -> bool {
        CHOSEN.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn choose(on: bool) {
        CHOSEN.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// For tests: the port set as `on` says for as long as the guard is
    /// held, one test at a time (the setting is the whole simulator's).
    #[cfg(test)]
    pub fn exclusive(on: bool) -> std::sync::MutexGuard<'static, ()> {
        static ONE: Mutex<()> = Mutex::new(());
        let guard = ONE.lock().unwrap_or_else(|e| e.into_inner());
        choose(on);
        guard
    }

    pub const BOOT: Duration = Duration::from_millis(1400);

    pub struct SimPort {
        to_cam: mpsc::Sender<Vec<u8>>,
        from_cam: mpsc::Receiver<Vec<u8>>,
        held: Vec<u8>,
        baud: Arc<Mutex<u32>>,
        powered: Arc<Mutex<Option<Instant>>>,
    }

    pub fn port() -> SimPort {
        let (to_cam, cam_rx) = mpsc::channel::<Vec<u8>>();
        let (cam_tx, from_cam) = mpsc::channel::<Vec<u8>>();
        let baud = Arc::new(Mutex::new(START_BAUD));
        let powered = Arc::new(Mutex::new(None));
        let (b, p) = (baud.clone(), powered.clone());
        std::thread::Builder::new().name("smartcam-sim".into()).spawn(move || camera(cam_rx, cam_tx, b, p)).ok();
        SimPort { to_cam, from_cam, held: Vec::new(), baud, powered }
    }

    impl Port for SimPort {
        fn ready(&mut self) -> Result<(), Status> {
            if chosen() {
                Ok(())
            } else {
                Err(Status::PortIsConsole)
            }
        }
        fn power(&mut self, on: bool) {
            *self.powered.lock().unwrap_or_else(|e| e.into_inner()) = on.then(Instant::now);
        }
        fn set_baud(&mut self, baud: u32) -> bool {
            *self.baud.lock().unwrap_or_else(|e| e.into_inner()) = baud;
            true
        }
        fn write(&mut self, bytes: &[u8]) -> bool {
            // Tagged with the rate it was sent at: the camera hears it only
            // if it listens at the same.
            let mut tagged = self.baud.lock().map(|b| b.to_le_bytes().to_vec()).unwrap_or_default();
            tagged.extend_from_slice(bytes);
            self.to_cam.send(tagged).is_ok()
        }
        fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize {
            if self.held.is_empty() {
                if let Ok(chunk) = self.from_cam.recv_timeout(wait) {
                    self.held = chunk;
                }
                while let Ok(more) = self.from_cam.try_recv() {
                    self.held.extend_from_slice(&more);
                }
            }
            let n = self.held.len().min(buf.len());
            buf[..n].copy_from_slice(&self.held[..n]);
            self.held.drain(..n);
            n
        }
    }

    /// The stand-in camera's program (as tools/unitv/main.py does it).
    fn camera(rx: mpsc::Receiver<Vec<u8>>, tx: mpsc::Sender<Vec<u8>>, _baud: Arc<Mutex<u32>>, powered: Arc<Mutex<Option<Instant>>>) {
        let mut decoder = frame::Decoder::default();
        let mut my_baud = START_BAUD;
        let mut want = (false, false, false, false, false);
        let mut seq = 0u8;
        let mut n = 0u32;
        let mut booted_at: Option<Instant> = None;
        let mut last_frame = Instant::now();
        let mut since_baud: Option<Instant> = None;
        let started = Instant::now();
        // Sent at the camera's rate: as long as the bytes take on the wire.
        let send = |kind: u8, payload: &[u8], seq: &mut u8, rate: u32| {
            *seq = seq.wrapping_add(1);
            let bytes = frame::encode(kind, *seq, payload);
            std::thread::sleep(Duration::from_secs_f64(bytes.len() as f64 * 10.0 / f64::from(rate)));
            tx.send(bytes).ok();
        };
        loop {
            let on = *powered.lock().unwrap_or_else(|e| e.into_inner());
            let Some(since) = on else {
                (booted_at, my_baud, want) = (None, START_BAUD, (false, false, false, false, false));
                decoder = frame::Decoder::default();
                while rx.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(30));
                continue;
            };
            if since.elapsed() < BOOT {
                while rx.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            if booted_at.is_none() {
                booted_at = Some(Instant::now());
                // The K210's boot ROM says something at its own rate: junk to the link.
                tx.send(b"\x00\xffK210 boot\r\n\xc5".to_vec()).ok();
            }
            // What the handheld sent, heard only at the rate this listens at.
            while let Ok(chunk) = rx.try_recv() {
                let (rate, bytes) = (u32::from_le_bytes(chunk[..4].try_into().unwrap_or([0; 4])), &chunk[4..]);
                if rate != my_baud {
                    continue;
                }
                for f in decoder.push(bytes) {
                    match f.kind {
                        frame::HELLO => {
                            let up = booted_at.map_or(0, |b| b.elapsed().as_millis());
                            let hello = format!(r#"{{"v":1,"fw":"catalyst-unitv 1 (simulated)","kpu":"face yolo2 320x240","crc":"native","fov":80,"w":320,"h":240,"up_ms":{up}}}"#);
                            send(frame::HELLO_BACK, hello.as_bytes(), &mut seq, my_baud);
                        }
                        frame::BAUD => {
                            let to = u32::from_le_bytes(f.payload[..4].try_into().unwrap_or([0; 4]));
                            send(frame::BAUD_BACK, &f.payload, &mut seq, my_baud);
                            // The K210 holds every rate the link asks of it here.
                            my_baud = to;
                            since_baud = Some(Instant::now());
                        }
                        frame::PING => {
                            since_baud = None;
                            send(frame::PONG, &f.payload, &mut seq, my_baud);
                        }
                        frame::WANT => {
                            let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
                            let on = |k: &str| v[k].as_u64().unwrap_or(0) != 0;
                            want = (on("faces"), on("codes"), on("tags"), on("blobs"), on("preview"));
                            send(frame::WANT_BACK, &f.payload, &mut seq, my_baud);
                        }
                        _ => {}
                    }
                }
            }
            // No frame at a new rate for a second: back to the start rate.
            if since_baud.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                (my_baud, since_baud) = (START_BAUD, None);
            }
            // Its looking, at about 15 frames a second while asked to.
            let any = want.0 || want.1 || want.2 || want.3 || want.4;
            if !any || last_frame.elapsed() < Duration::from_millis(66) {
                std::thread::sleep(Duration::from_millis(8));
                continue;
            }
            last_frame = Instant::now();
            n += 1;
            let t = started.elapsed().as_secs_f32();
            let face = [0.42 + 0.12 * (t * 0.4).sin(), 0.28 + 0.04 * (t * 0.7).cos()];
            let mut json = format!(r#"{{"n":{n},"w":320,"h":240,"fps":15.0"#);
            if want.0 {
                json += &format!(r#","f":[[{:.0},{:.0},58,70,87]]"#, face[0] * 320.0, face[1] * 240.0);
            }
            if want.1 {
                json += r#","q":[[212,128,52,52,"SCOUT 5805 Q12 auto=3 tele=11 climb=deep"]]"#;
            }
            if want.2 {
                json += &format!(r#","a":[[7,{:.0},62,{:.0},-412,-88,2640]]"#, 70.0 + 6.0 * (t * 0.3).sin(), 120.0 * (t * 0.2).sin());
            }
            if want.3 {
                json += r#","b":[[0,40,176,34,28,640]]"#;
            }
            json.push('}');
            send(frame::RESULTS, json.as_bytes(), &mut seq, my_baud);
            // A burst of junk now and then (a hand on the cable).
            if n % 97 == 0 {
                tx.send(vec![0xC5, 0x7A, 0x85, 0x01, 0xFF, 0xFF, 0x00, 1, 2, 3]).ok();
            }
            if want.4 {
                let pic = view(t, face);
                if let Some(jpeg) = jpeg(&pic) {
                    send(frame::PREVIEW, &jpeg, &mut seq, my_baud);
                }
            }
        }
    }

    fn dot(pic: &mut Picture, x: f32, y: f32, rx: f32, ry: f32, c: [u8; 3]) {
        for py in (y - ry) as i32..(y + ry) as i32 {
            for px in (x - rx) as i32..(x + rx) as i32 {
                let (dx, dy) = ((px as f32 - x) / rx, (py as f32 - y) / ry);
                if dx * dx + dy * dy <= 1.0 && px >= 0 && py >= 0 && (px as usize) < pic.w && (py as usize) < pic.h {
                    let at = (py as usize * pic.w + px as usize) * 3;
                    pic.rgb[at..at + 3].copy_from_slice(&c);
                }
            }
        }
    }

    /// A person in the pit, their face centred at (x, y), `s` times the
    /// size the stand-in's own lens sees them.
    pub fn person(pic: &mut Picture, x: f32, y: f32, s: f32) {
        dot(pic, x, y + 70.0 * s, 62.0 * s, 38.0 * s, [44, 62, 98]);
        dot(pic, x, y + 38.0 * s, 12.0 * s, 14.0 * s, [196, 150, 122]);
        dot(pic, x, y, 27.0 * s, 34.0 * s, [214, 168, 140]);
        dot(pic, x, y - 26.0 * s, 28.0 * s, 14.0 * s, [58, 40, 30]);
    }

    /// What the stand-in sees, 320 × 240: the pit through its wider lens,
    /// with a person's face, the code card, the tag on the wall and a game
    /// piece in it.
    pub fn view(t: f32, face: [f32; 2]) -> Picture {
        let mut pic = crate::camera::scene(320, 240, t, false);
        fn rect(pic: &mut Picture, x0: usize, y0: usize, s: usize, c: u8) {
            for y in y0..(y0 + s).min(pic.h) {
                for x in x0..(x0 + s).min(pic.w) {
                    let at = (y * pic.w + x) * 3;
                    pic.rgb[at..at + 3].copy_from_slice(&[c, c, c]);
                }
            }
        }
        person(&mut pic, face[0] * 320.0 + 29.0, face[1] * 240.0 + 35.0, 1.0);
        // The tag: black with its white border, on the wall.
        let tx = (70.0 + 6.0 * (t * 0.3).sin()) as usize;
        rect(&mut pic, tx - 16, 46, 32, 235);
        rect(&mut pic, tx - 12, 50, 24, 20);
        rect(&mut pic, tx - 6, 56, 6, 235);
        rect(&mut pic, tx + 2, 60, 5, 235);
        // The code card and the game piece.
        rect(&mut pic, 212, 128, 52, 236);
        for i in 0..5 {
            rect(&mut pic, 218 + i * 9, 134 + (i % 2) * 18, 7, 26);
            rect(&mut pic, 218 + ((i + 2) % 5) * 9, 158 + (i % 3) * 6, 6, 26);
        }
        dot(&mut pic, 57.0, 190.0, 17.0, 14.0, [236, 132, 40]);
        pic
    }

    #[cfg(not(target_arch = "aarch64"))]
    fn jpeg(pic: &Picture) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut out, 55);
        encoder.encode(&pic.rgb, pic.w as u16, pic.h as u16, jpeg_encoder::ColorType::Rgb).ok()?;
        Some(out)
    }

    #[cfg(target_arch = "aarch64")]
    fn jpeg(_pic: &Picture) -> Option<Vec<u8>> {
        None
    }
}

// ------------------------------------------------------------------- tests ---

#[cfg(test)]
mod tests {
    use super::frame::*;
    use super::*;

    #[test]
    fn crc32_is_the_standard_one() {
        // The check value every CRC-32 (IEEE) has for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn the_wire_bytes_are_the_cameras() {
        // tools/unitv/test_protocol.py checks main.py makes these same bytes.
        let bytes = encode(RESULTS, 1, br#"{"n":1}"#);
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "c57a85010700e77b226e223a317d5ed2d160");
    }

    #[test]
    fn frames_survive_junk_a_split_read_and_a_corrupted_neighbour() {
        let a = encode(RESULTS, 1, br#"{"n":1}"#);
        let b = encode(PREVIEW, 2, &[0xC5, 0x7A, 1, 2, 3, 0xC5]);
        let mut bad = encode(PONG, 3, b"x");
        let last = bad.len() - 1;
        bad[last] ^= 0x40;
        let c = encode(LOG, 4, b"after the bad one");
        let mut wire = b"boot noise \xc5".to_vec();
        wire.extend_from_slice(&a);
        wire.extend_from_slice(&[0x00, 0xC5, 0xC5, 0x7A]);
        wire.extend_from_slice(&b);
        wire.extend_from_slice(&bad);
        wire.extend_from_slice(&c);
        // Every way of cutting it up gives the same frames.
        for cut in [1usize, 2, 3, 5, 7, 11, 64, wire.len()] {
            let mut d = Decoder::default();
            let mut got = Vec::new();
            for chunk in wire.chunks(cut) {
                got.extend(d.push(chunk));
            }
            let kinds: Vec<(u8, u8)> = got.iter().map(|f| (f.kind, f.seq)).collect();
            assert_eq!(kinds, vec![(RESULTS, 1), (PREVIEW, 2), (LOG, 4)], "cut into {cut}s");
            assert_eq!(got[1].payload, vec![0xC5, 0x7A, 1, 2, 3, 0xC5], "a payload that holds the sync word");
            assert!(d.bad >= 1, "the corrupted frame was refused");
            assert!(d.junk > 0);
        }
    }

    #[test]
    fn a_false_sync_with_a_huge_length_is_refused_at_once() {
        let good = encode(PONG, 9, b"ok");
        // A sync word in junk, a header claiming 60 kB: refused by the
        // header's own check, not waited on.
        let mut wire = vec![0xC5, 0x7A, 0x85, 0x01, 0x60, 0xEA, 0x00];
        wire.extend_from_slice(&good);
        let mut d = Decoder::default();
        let got = d.push(&wire);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].payload, b"ok");
    }

    #[test]
    fn power_is_on_while_wanted_warm_after_and_then_off() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut p = Power::Off;
        assert_eq!(p.step(at(0), false, false), Switch::Keep, "nobody wants it: off");
        assert_eq!(p.step(at(10), true, false), Switch::On, "wanted: on at once");
        assert_eq!(p.step(at(500), true, false), Switch::Keep);
        assert!(matches!(p, Power::Waking { .. }));
        // It answers 1.4 s after power: that is its start time.
        p.step(at(1410), true, true);
        assert_eq!(p, Power::On { took: Duration::from_millis(1400) });
        // Let go: warm for WARM_FOR, wanted again meanwhile, no restart.
        assert_eq!(p.step(at(5000), false, false), Switch::Keep);
        assert!(matches!(p, Power::Warm { .. }));
        assert_eq!(p.step(at(9000), true, false), Switch::Keep, "back within the warm time: still on");
        assert!(matches!(p, Power::On { .. }));
        p.step(at(10_000), false, false);
        let off_at = 10_000 + WARM_FOR.as_millis() as u64;
        assert_eq!(p.step(at(off_at - 1), false, false), Switch::Keep);
        assert_eq!(p.step(at(off_at), false, false), Switch::Off, "off once the warm time is out");
        assert_eq!(p, Power::Off);
    }

    #[test]
    fn a_camera_that_never_answers_is_powered_off_and_tried_again_later() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut p = Power::Off;
        p.step(at(0), true, false);
        let timeout = WAKE_TIMEOUT.as_millis() as u64;
        assert_eq!(p.step(at(timeout), true, false), Switch::Off);
        assert!(matches!(p, Power::Failed { .. }));
        assert_eq!(p.step(at(timeout + 1000), true, false), Switch::Keep, "not hammered");
        assert_eq!(p.step(at(timeout + RETRY_AFTER.as_millis() as u64), true, false), Switch::On, "tried again");
        // Nobody wants it any more: off, and stays so.
        let mut q = Power::Failed { at: at(0) };
        q.step(at(10), false, false);
        assert_eq!(q, Power::Off);
    }

    #[test]
    fn results_are_read_into_the_cameras_picture() {
        let r = parse_results(br#"{"n":42,"w":320,"h":240,"fps":14.5,"f":[[160,120,32,24,91]],"q":[[0,0,64,48,"SCOUT 5805"]],"a":[[7,80,60,250,-412,-88,2640]],"b":[[0,40,176,34,28,640]]}"#).unwrap();
        assert_eq!(r.frame, 42);
        assert_eq!(r.faces, vec![Face { at: [0.5, 0.5, 0.6, 0.6], score: 0.91 }]);
        assert_eq!(r.codes[0].text, "SCOUT 5805");
        assert_eq!(r.tags[0].id, 7);
        assert!((r.tags[0].turn - 0.25).abs() < 1e-6 && r.tags[0].pose_mm == [-412.0, -88.0, 2640.0]);
        assert_eq!(r.blobs[0].pixels, 640);
        assert!(parse_results(b"not json").is_none());
    }

    #[test]
    fn a_face_maps_into_the_main_cameras_picture() {
        let a = Align::default();
        // The wider lens: its picture's middle is the main one's, its edges
        // further out.
        assert!(a.sx > 1.05 && a.sx < 1.2 && a.sy > 1.05 && a.sy < 1.2, "{a:?}");
        let mid = a.map([0.45, 0.45, 0.55, 0.55]);
        assert!((mid[0] - (0.5 - 0.05 * a.sx)).abs() < 1e-5);
        let offset = Align { ox: 0.1, ..a };
        assert!((offset.map([0.5, 0.5, 0.5, 0.5])[0] - 0.6).abs() < 1e-5);
        // The largest sure face, if it falls in the main picture.
        let r = Results { faces: vec![Face { at: [0.4, 0.4, 0.5, 0.5], score: 0.9 }, Face { at: [0.1, 0.1, 0.12, 0.12], score: 0.95 }, Face { at: [0.3, 0.3, 0.7, 0.7], score: 0.3 }], ..Results::default() };
        let f = main_face(&r, &a).unwrap();
        assert!(f[0] > 0.35 && f[2] < 0.6);
        let outside = Results { faces: vec![Face { at: [0.0, 0.0, 0.03, 0.03], score: 0.9 }], ..Results::default() };
        assert!(main_face(&outside, &a).is_none(), "a face beyond the main picture's edge");
    }

    /// The whole link against the stand-in: woken, its start time measured,
    /// the fastest rate taken, results and previews arriving, then warm.
    #[test]
    fn the_stand_in_wakes_and_answers_at_the_fastest_rate() {
        let _port = sim::exclusive(true);
        let lease = lease(Use::View);
        let started = Instant::now();
        while !matches!(status(), Status::Ready { baud: 3_000_000, .. }) && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let Status::Ready { baud, woke_ms } = status() else { panic!("{:?}", status()) };
        assert_eq!(baud, 3_000_000);
        assert!((1300..4000).contains(&woke_ms), "it took {woke_ms} ms to start");
        while preview().0 < 3 && started.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(preview().0 >= 3, "previews arrive");
        // Room for a busy machine: the whole suite runs beside it.
        let r = results(Duration::from_secs(5)).expect("results arrive");
        assert!(!r.faces.is_empty() && !r.codes.is_empty() && !r.tags.is_empty());
        assert!(tags().is_some_and(|(_, t)| t[0].id == 7), "the field view's tags");
        drop(lease);
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(status(), Status::Ready { .. }), "warm: still answering a moment after");
    }

    /// The port changes hands at once, both ways: to the console, the
    /// camera in use is let go of and unpowered at once (no warm time), and
    /// nothing powers it while the port changes; back, it wakes again with
    /// no restart.
    #[test]
    fn the_port_switches_at_once_and_lets_go_first() {
        let _port = sim::exclusive(true);
        let _lease = lease(Use::View);
        let wait = |what: &dyn Fn() -> bool| {
            let started = Instant::now();
            while !what() && started.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(10));
            }
            what()
        };
        assert!(wait(&|| matches!(status(), Status::Ready { .. })), "{:?}", status());
        switch_grove(false);
        assert!(wait(&|| !switching()), "the switch finished");
        assert!(!chosen());
        assert!(!PORT_OPEN.load(Ordering::Acquire));
        assert!(wait(&|| status() == Status::PortIsConsole), "unpowered at once, no warm time: {:?}", status());
        switch_grove(true);
        assert!(wait(&|| !switching()));
        assert!(chosen());
        assert!(wait(&|| matches!(status(), Status::Ready { .. })), "awake again without a restart: {:?}", status());
    }
}
