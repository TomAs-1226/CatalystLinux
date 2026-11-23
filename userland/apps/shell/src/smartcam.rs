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
//! is instant, then off. A K210 with main.py answers two or three seconds
//! after power (MaixPy, the sensor, the face model): it is given
//! [`WAKE_TIMEOUT`], then powered off a moment and tried again. The time
//! from power to its first answer is measured and shown ("waking the smart
//! camera…"); one far longer than a start takes means it started more than
//! once (short of power: the Grove 5 V is rated for 0.5 A, the UnitV's own
//! figure).
//!
//! **The link.** Frames (see [`frame`]): a sync word, the kind, a sequence
//! number, the length with a check of its own, the payload, and a CRC-32
//! over all of it; junk between frames (a camera starting up, a bit lost) is
//! skipped to the next sync word. It starts at 115 200 baud and climbs a
//! step at a time ([`CLIMB`]), each step confirmed both ways (asked and
//! answered at the old rate, a ping heard and answered at the new); the
//! first that fails sends both back to the last that held (3 Mbaud is the
//! PL011's limit at its default clock; the K210's UART runs from a 195 MHz
//! clock with a fractional divider). Too many frames garbled at a rate: one
//! step down. Quiet for [`LOST_AFTER`]: both back to 115 200 and found again
//! (main.py goes back by itself after five quiet seconds).
//!
//! **Its program.** When the camera answers with an older program or model
//! than this image carries (`/usr/share/catalyst/unitv`), the new ones go
//! over the link ([`update`]) and the camera starts again with them; boot.py
//! there rolls back a program that never starts. MaixPy itself cannot go
//! over the Grove port (it needs the K210's boot pin, which only the USB
//! bridge reaches): when the camera's lacks what the program needs, it is
//! said so, and USB-A does it.
//!
//! **What it has seen** ([`diag`]): the 5 V, the bytes and frames, the
//! rates tried, the last error, the camera's hello, for Settings › Camera ›
//! Smart camera (`smartcam_status`).
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
/// No answer this long after power on: not there (or not set up). A K210
/// with main.py answers in two or three seconds; this leaves room for a
/// slow start (a fresh /flash, a sensor slow to wake) several times over.
pub const WAKE_TIMEOUT: Duration = Duration::from_secs(12);
/// After a camera that did not answer, tried again this long after.
pub const RETRY_AFTER: Duration = Duration::from_secs(10);
/// The rates stepped down through when one garbles, fastest first.
pub const BAUDS: [u32; 7] = [3_000_000, 2_000_000, 1_500_000, 1_000_000, 921_600, 460_800, 115_200];
pub const START_BAUD: u32 = 115_200;
/// The 5 V stays off at least this long before it comes back (my own
/// tuning: the UnitV's capacitors drained, the K210 reset).
pub const OFF_FOR: Duration = Duration::from_secs(1);
/// Nothing heard this long from a camera that answered: the link is lost
/// (pinged meanwhile, so a quiet camera still answers).
pub const LOST_AFTER: Duration = Duration::from_secs(5);
/// From power to main.py's first line, a start takes two or three seconds
/// (MaixPy loading); longer than this, the K210 started more than once.
pub const RESTARTED_MS: u32 = 7000;
/// What the handheld's hello says of itself.
pub const HELLO_PAYLOAD: &[u8] = br#"{"v":2}"#;

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
    /// The Grove update: a file begun (name, size, CRC, id), a chunk of it
    /// (its offset, LE, then bytes), its end (checked whole), and all of
    /// them moved into place with the stamp (the camera then starts again).
    pub const PUT: u8 = 0x05;
    pub const CHUNK: u8 = 0x06;
    pub const DONE: u8 = 0x07;
    pub const COMMIT: u8 = 0x08;
    // Its answers, and what it sends by itself once asked.
    pub const HELLO_BACK: u8 = 0x81;
    pub const BAUD_BACK: u8 = 0x82;
    pub const WANT_BACK: u8 = 0x83;
    pub const PONG: u8 = 0x84;
    pub const RESULTS: u8 = 0x85;
    pub const PREVIEW: u8 = 0x86;
    #[allow(dead_code)]
    pub const LOG: u8 = 0x87;
    /// How much of the file it already holds (to resume from), or why not.
    pub const PUT_BACK: u8 = 0x88;
    /// How much of the file it holds now (LE; 0xFFFFFFFF: no file under way).
    pub const CHUNK_BACK: u8 = 0x89;
    pub const DONE_BACK: u8 = 0x8A;
    pub const COMMIT_BACK: u8 = 0x8B;

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
    /// Settings › Camera › Smart camera: powered and linked, asked for
    /// nothing (what it says of itself, its program brought up to date).
    Status,
}

// --------------------------------------------------------------- diagnosis ---

/// The camera's hello, as main.py says it (0.9.3's, version 1, says less).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hello {
    /// main.py's VERSION.
    pub version: u32,
    /// It takes its program over the link (version 2 on).
    pub updates: bool,
    /// MaixPy's version ("0.6.3").
    pub maixpy: String,
    /// Whether its MaixPy has what main.py needs (None: it did not say).
    pub full: Option<bool>,
    /// "ok", or why its image sensor did not start.
    pub sensor: Option<String>,
    pub kpu: String,
    /// From main.py's first line to this hello.
    pub up_ms: u32,
    /// From main.py's first line to its listening on the link.
    pub listen_ms: Option<u32>,
    pub usb: bool,
    /// Where MaixPy started it from: /flash, or /sd (a card in it).
    pub cwd: String,
    /// A new program's starts so far (None: no update on trial).
    pub trial: Option<u32>,
    /// The last update never started, and boot.py put the old one back.
    pub rolled_back: bool,
    pub stamp: Option<crate::unitv::Stamp>,
}

impl Hello {
    pub fn parse(json: &[u8]) -> Option<Hello> {
        let v: serde_json::Value = serde_json::from_slice(json).ok()?;
        let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
        let num = |k: &str| v.get(k).and_then(serde_json::Value::as_u64);
        let flag = |k: &str| v.get(k).and_then(|x| x.as_bool().or_else(|| x.as_u64().map(|n| n != 0)));
        Some(Hello {
            version: num("v").unwrap_or(1) as u32,
            updates: flag("upd").unwrap_or(false),
            maixpy: text("mp").unwrap_or_default(),
            full: flag("full"),
            sensor: text("sensor"),
            kpu: text("kpu").unwrap_or_default(),
            up_ms: num("up_ms").unwrap_or(0) as u32,
            listen_ms: num("listen_ms").map(|n| n as u32),
            usb: flag("usb").unwrap_or(false),
            cwd: text("cwd").unwrap_or_default(),
            trial: num("trial").map(|n| n as u32),
            rolled_back: flag("rolled_back").unwrap_or(false),
            stamp: v.get("stamp").and_then(crate::unitv::Stamp::from_value),
        })
    }
}

/// The camera's program brought up to date over the link, as it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Update {
    #[default]
    Idle,
    /// Older than this image's, and it cannot take one over the link: why
    /// (USB-A does it, once).
    UsbOnce(String),
    /// A file on its way; `done` is of the whole update, 0 to 1.
    Sending { file: String, done: f32 },
    /// Committed: the camera starting again with it.
    Restarting,
    /// On it now: the release whose program it is.
    Done(String),
    Failed(String),
    /// It never started, and the old program is back.
    RolledBack,
}

/// What the link has seen since the shell started, for the status page and
/// its report.
#[derive(Clone, Debug, Default)]
pub struct Diag {
    /// The Grove 5 V as its switch reads back (None: no switch to read).
    pub five_volt: Option<bool>,
    /// Powered since.
    pub powered_at: Option<Instant>,
    pub wakes: u32,
    /// Powered and never answered (each a wake timed out).
    pub unanswered: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub frames: u64,
    /// Frames refused (header or CRC), and bytes between frames.
    pub refused: u64,
    pub junk: u64,
    /// The rate in use, the fastest tried, and the last that did not hold.
    pub baud: u32,
    pub baud_tried: u32,
    pub baud_failed: Option<u32>,
    pub last_error: Option<(Instant, String)>,
    pub hello: Option<Hello>,
    pub hello_at: Option<Instant>,
    /// Power to main.py's first line, at the last start.
    pub start_ms: Option<u32>,
    /// It started again by itself (a long start, or after the link went quiet).
    pub restarts: u32,
    /// The link went quiet and was found again.
    pub lost: u32,
    /// The port's own trouble: it would not open, catalyst-grove's word.
    pub port: Option<String>,
    pub update: Update,
}

struct Shared {
    leases: Vec<(u64, Use)>,
    next_lease: u64,
    status: Status,
    rates: Rates,
    seen: Seen,
    /// The camera's own description (from its HELLO).
    about: Option<String>,
    diag: Diag,
}

struct Hub {
    shared: Mutex<Shared>,
    wake: Condvar,
}

fn hub() -> &'static Arc<Hub> {
    static HUB: std::sync::OnceLock<Arc<Hub>> = std::sync::OnceLock::new();
    HUB.get_or_init(|| {
        let hub = Arc::new(Hub {
            shared: Mutex::new(Shared { leases: Vec::new(), next_lease: 1, status: Status::Asleep, rates: Rates::default(), seen: Seen::default(), about: None, diag: Diag::default() }),
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

/// The camera's own description (its hello's JSON, as it came).
pub fn about() -> Option<String> {
    hub().shared.lock().ok().and_then(|s| s.about.clone())
}

/// What the link has seen (the status page reads it four times a second).
pub fn diag() -> Diag {
    hub().shared.lock().map(|s| s.diag.clone()).unwrap_or_default()
}

/// The update tried once each start of the shell (for one program); asked
/// again from the status page.
static TRIED: Mutex<Option<String>> = Mutex::new(None);

/// The camera's program sent again, if it needs it, at its next answer (the
/// status page's `u`).
pub fn retry_update() {
    *TRIED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    RECHECK.store(true, Ordering::Release);
    changed();
}

static RECHECK: AtomicBool = AtomicBool::new(false);

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
    /// The 5 V as its switch reads back (None: no switch to read).
    fn five_volt(&mut self) -> Option<bool> {
        None
    }
    /// What last went wrong with the port itself (it would not open, the 5 V
    /// was refused, why it is not free), if anything.
    fn trouble(&mut self) -> Option<String> {
        None
    }
}

/// How long the link waits for things.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// For the camera's first answer after power on (or after it restarts).
    pub answer: Duration,
    /// The 5 V kept off before it comes back.
    pub off_for: Duration,
    /// For the answer to a request.
    pub ack: Duration,
    /// For the camera to go back to the rate before, after a move that did
    /// not hold (main.py: a second with no frame at the new rate).
    pub settle: Duration,
}

pub const TIMING: Timing = Timing { answer: WAKE_TIMEOUT, off_for: OFF_FOR, ack: Duration::from_millis(1500), settle: Duration::from_millis(1200) };

/// The rates climbed through from 115 200, one step at a time.
pub const CLIMB: [u32; 6] = [460_800, 921_600, 1_000_000, 1_500_000, 2_000_000, 3_000_000];

/// The port with the link's frames on it, what goes each way counted.
pub struct Link {
    pub port: Box<dyn Port>,
    pub decoder: frame::Decoder,
    pub baud: u32,
    seq: u8,
    pub sent: u64,
    pub got: u64,
}

impl Link {
    pub fn new(port: Box<dyn Port>) -> Link {
        Link { port, decoder: frame::Decoder::default(), baud: START_BAUD, seq: 0, sent: 0, got: 0 }
    }

    pub fn send(&mut self, kind: u8, payload: &[u8]) -> bool {
        self.seq = self.seq.wrapping_add(1);
        let bytes = frame::encode(kind, self.seq, payload);
        self.sent += bytes.len() as u64;
        self.port.write(&bytes)
    }

    pub fn set_baud(&mut self, baud: u32) {
        self.baud = baud;
        self.port.set_baud(baud);
    }

    /// The frames that arrive within `wait`.
    pub fn read(&mut self, wait: Duration) -> Vec<frame::Frame> {
        let mut buf = [0u8; 8192];
        let n = self.port.read(&mut buf, wait);
        self.got += n as u64;
        self.decoder.push(&buf[..n])
    }

    /// The first frame of `kind` within `within` (any other let go).
    pub fn await_kind(&mut self, kind: u8, within: Duration) -> Option<frame::Frame> {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if let Some(f) = self.read(Duration::from_millis(20)).into_iter().find(|f| f.kind == kind) {
                return Some(f);
            }
        }
        None
    }

    /// Sent, and its answer waited for, up to `tries` times.
    pub fn ask(&mut self, kind: u8, payload: &[u8], back: u8, within: Duration, tries: u32) -> Option<frame::Frame> {
        (0..tries).find_map(|_| {
            self.send(kind, payload);
            self.await_kind(back, within)
        })
    }

    /// Hellos four times a second until the camera answers, for up to
    /// `within`.
    pub fn hello(&mut self, within: Duration) -> Option<frame::Frame> {
        let until = Instant::now() + within;
        while Instant::now() < until {
            self.send(frame::HELLO, HELLO_PAYLOAD);
            let left = until.saturating_duration_since(Instant::now());
            if let Some(f) = self.await_kind(frame::HELLO_BACK, Duration::from_millis(250).min(left)) {
                return Some(f);
            }
        }
        None
    }
}

/// Where a climb ended: the rate in use, the fastest tried, the one that
/// did not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Climbed {
    pub baud: u32,
    pub tried: u32,
    pub failed: Option<u32>,
}

/// Up from the rate in use, a step at a time; the first step that does not
/// hold sends both back to the last that did.
pub fn climb(link: &mut Link, t: &Timing) -> Climbed {
    let (mut tried, from) = (link.baud, link.baud);
    for &rate in CLIMB.iter().filter(|r| **r > from) {
        tried = rate;
        if !step_to(link, rate, t) {
            return Climbed { baud: link.baud, tried, failed: Some(rate) };
        }
    }
    Climbed { baud: link.baud, tried, failed: None }
}

/// One move, confirmed both ways: asked and answered at the rate in use,
/// then a ping heard and answered at `to`. Not held: both back to the rate
/// before (the camera goes back by itself after a second with no frame).
/// Whether it held.
pub fn step_to(link: &mut Link, to: u32, t: &Timing) -> bool {
    let from = link.baud;
    if link.ask(frame::BAUD, &to.to_le_bytes(), frame::BAUD_BACK, Duration::from_millis(400), 1).is_none() {
        // It may have moved with its answer lost: it comes back by itself.
        std::thread::sleep(t.settle);
        link.decoder = frame::Decoder::default();
        return false;
    }
    link.set_baud(to);
    std::thread::sleep(Duration::from_millis(30));
    if link.ask(frame::PING, b"catalyst", frame::PONG, Duration::from_millis(200), 3).is_some() {
        return true;
    }
    link.set_baud(from);
    std::thread::sleep(t.settle);
    link.decoder = frame::Decoder::default();
    false
}

/// A baud rate in a few characters ("3 Mbaud", "921.6 kbaud").
pub fn baud_words(baud: u32) -> String {
    match baud {
        b if b >= 1_000_000 && b % 1_000_000 == 0 => format!("{} Mbaud", b / 1_000_000),
        b if b >= 1_000_000 => format!("{:.1} Mbaud", f64::from(b) / 1e6),
        b if b % 1000 == 0 => format!("{} kbaud", b / 1000),
        b => format!("{:.1} kbaud", f64::from(b) / 1e3),
    }
}

// ------------------------------------------------------------------ update ---

/// The camera's program (and face model) over the link.
///
/// Each file: PUT (name, size, CRC-32, an id of its contents), answered with
/// how much of it the camera already holds (a transfer cut off before, by a
/// lost link or the power, resumes there); CHUNKs, each its offset and up to
/// 4 kB (1 kB below 921.6 kbaud), each answered with how much it holds now
/// (one out of place is not written, so a resent chunk is harmless); DONE,
/// answered once the camera has read the file back and checked its CRC (a
/// file that fails is thrown away and sent again). Then COMMIT with the
/// stamp: main.py moves them all into place with the old ones kept, begins a
/// trial for boot.py and starts again; if the new program does not answer,
/// the camera is powered off and on until boot.py (on its third start) has
/// the old one back.
pub mod update {
    use super::*;
    use crate::unitv::{Payload, Stamp};

    /// What the camera needs, from its hello and this image's program.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Plan {
        Nothing,
        /// Over USB-A, once: why.
        UsbOnce(String),
        /// Over the link: the program (main.py and boot.py), the model.
        Send { program: bool, model: bool },
    }

    pub fn plan(hello: &Hello, expected: Option<&Stamp>) -> Plan {
        if hello.full == Some(false) {
            return Plan::UsbOnce("its MaixPy lacks what the program needs: a full one over USB-A".into());
        }
        let Some(e) = expected else { return Plan::Nothing };
        let (program, model) = match &hello.stamp {
            Some(s) => (s.main != e.main, s.model != e.model),
            None => (true, true),
        };
        if !program && !model {
            return Plan::Nothing;
        }
        if !hello.updates {
            return Plan::UsbOnce("its program is from before updates over the Grove port: once over USB-A".into());
        }
        if hello.cwd == "/sd" {
            return Plan::UsbOnce("it starts from a microSD card: take the card out, then once over USB-A".into());
        }
        Plan::Send { program, model }
    }

    /// How an update went: the camera's hello after, the files that resumed
    /// part way (and from where), the files sent again.
    #[derive(Debug)]
    #[allow(dead_code)] // the tests read how it went
    pub struct Sent {
        pub hello: Hello,
        pub resumed: Vec<(String, usize)>,
        pub again: u32,
    }

    /// The files, then the commit, then the camera started again.
    pub fn run(link: &mut Link, payload: &Payload, stamp: &Stamp, program: bool, model: bool, t: &Timing, progress: &mut dyn FnMut(Update)) -> Result<Sent, String> {
        let mut files: Vec<(&str, &[u8])> = Vec::new();
        if program {
            files.push(("main.py", payload.main.as_slice()));
            files.push(("boot.py", payload.boot.as_slice()));
        }
        if model {
            files.push(("face.kmodel", payload.model.as_slice()));
        }
        let total = files.iter().map(|(_, d)| d.len()).sum::<usize>().max(1);
        let (mut before, mut resumed, mut again) = (0usize, Vec::new(), 0u32);
        // Nothing streams while files arrive.
        link.send(frame::WANT, br#"{"faces":0,"codes":0,"tags":0,"blobs":0,"preview":0}"#);
        for (name, data) in &files {
            let id = format!("{:08x}{:x}", frame::crc32(data), data.len());
            let mut tries = 0;
            loop {
                let r = send_file(link, name, data, &id, t, &mut |at| progress(Update::Sending { file: (*name).to_owned(), done: (before + at) as f32 / total as f32 }));
                match r {
                    Ok(from) => {
                        if from > 0 {
                            resumed.push(((*name).to_owned(), from));
                        }
                        break;
                    }
                    Err(_) if tries < 3 => {
                        tries += 1;
                        again += 1;
                        relink(link, t)?;
                    }
                    Err(why) => return Err(why),
                }
            }
            before += data.len();
        }
        let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
        let stamp_value: serde_json::Value = serde_json::from_str(&stamp.json()).unwrap_or_default();
        let commit = serde_json::json!({ "files": names, "stamp": stamp_value }).to_string();
        let f = link.ask(frame::COMMIT, commit.as_bytes(), frame::COMMIT_BACK, t.ack * 2, 2).ok_or("no answer to the commit")?;
        let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
        if v["ok"].as_u64() != Some(1) {
            return Err(format!("the camera would not take it ({})", v["err"].as_str().unwrap_or("?")));
        }
        progress(Update::Restarting);
        // It starts again by itself, at 115 200.
        link.set_baud(START_BAUD);
        link.decoder = frame::Decoder::default();
        let hello = restart(link, t, 4).ok_or("the camera did not start again")?;
        Ok(Sent { hello, resumed, again })
    }

    /// One file: from where the camera has got to, chunk by chunk, checked
    /// whole at the end. Where it began (more than 0: resumed).
    pub fn send_file(link: &mut Link, name: &str, data: &[u8], id: &str, t: &Timing, progress: &mut dyn FnMut(usize)) -> Result<usize, String> {
        let begin = serde_json::json!({ "name": name, "size": data.len(), "crc": frame::crc32(data), "id": id }).to_string();
        let f = link.ask(frame::PUT, begin.as_bytes(), frame::PUT_BACK, t.ack, 3).ok_or_else(|| format!("no answer to {name}'s start"))?;
        let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
        if let Some(err) = v["err"].as_str() {
            return Err(match err {
                "room" => "no room for it on the camera's flash".to_owned(),
                "card" => "it starts from a microSD card".to_owned(),
                other => format!("the camera refused {name} ({other})"),
            });
        }
        let from = (v["have"].as_u64().unwrap_or(0) as usize).min(data.len());
        let piece = if link.baud >= 921_600 { 4096 } else { 1024 };
        let (mut at, mut stalls) = (from, 0);
        progress(at);
        while at < data.len() {
            let end = (at + piece).min(data.len());
            let mut chunk = (at as u32).to_le_bytes().to_vec();
            chunk.extend_from_slice(&data[at..end]);
            let wire = Duration::from_secs_f64((chunk.len() + 11) as f64 * 10.0 / f64::from(link.baud.max(1)));
            match link.ask(frame::CHUNK, &chunk, frame::CHUNK_BACK, wire + t.ack, 1) {
                Some(f) if f.payload.len() >= 4 => {
                    let have = u32::from_le_bytes([f.payload[0], f.payload[1], f.payload[2], f.payload[3]]);
                    if have == u32::MAX {
                        return Err(format!("the camera lost {name}"));
                    }
                    let have = (have as usize).min(data.len());
                    stalls = if have > at { 0 } else { stalls + 1 };
                    at = have;
                }
                _ => stalls += 1,
            }
            if stalls >= 4 {
                return Err(format!("{name}: no answer at {at} of {} bytes", data.len()));
            }
            progress(at);
        }
        // It reads the file back for its CRC: about 20 kB a millisecond's worth, slow.
        let check = t.ack + Duration::from_millis(data.len() as u64 / 20);
        let done = serde_json::json!({ "name": name }).to_string();
        let f = link.ask(frame::DONE, done.as_bytes(), frame::DONE_BACK, check, 2).ok_or_else(|| format!("no answer to {name}'s end"))?;
        let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
        if v["ok"].as_u64() == Some(1) {
            Ok(from)
        } else {
            Err(format!("{name} arrived damaged ({})", v["err"].as_str().unwrap_or("?")))
        }
    }

    /// Hellos until it answers; powered off and on between tries (a new
    /// program that never answers is rolled back by boot.py on its third
    /// start). Its hello.
    pub fn restart(link: &mut Link, t: &Timing, tries: u32) -> Option<Hello> {
        for i in 0..tries {
            if i > 0 {
                link.port.power(false);
                std::thread::sleep(t.off_for);
                link.port.power(true);
                link.set_baud(START_BAUD);
                link.decoder = frame::Decoder::default();
            }
            if let Some(f) = link.hello(t.answer) {
                return Hello::parse(&f.payload);
            }
        }
        None
    }

    /// Found again at 115 200 (it may have started again), and up to speed.
    fn relink(link: &mut Link, t: &Timing) -> Result<(), String> {
        link.set_baud(START_BAUD);
        link.decoder = frame::Decoder::default();
        restart(link, t, 2).ok_or("the camera stopped answering")?;
        climb(link, t);
        Ok(())
    }
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

    fn note(hub: &Hub, change: impl FnOnce(&mut Diag)) {
        if let Ok(mut s) = hub.shared.lock() {
            change(&mut s.diag);
        }
    }

    fn fault(hub: &Hub, words: impl Into<String>) {
        let words = words.into();
        note(hub, |d| d.last_error = Some((Instant::now(), words)));
    }

    /// Why a camera that never answered did not, in a few words (the AI view
    /// says it under "no smart camera answers").
    fn absent_words(d: &Diag) -> String {
        match (d.five_volt, d.bytes_in, d.frames) {
            (Some(false), ..) => "the Grove 5 V did not come on".into(),
            (_, 0, _) => "no bytes from it · its cable, or its own power".into(),
            (_, _, 0) => "bytes, no frames · its program or the speed".into(),
            _ => "it stopped answering · Settings › Camera".into(),
        }
    }

    pub fn run(hub: &Hub) {
        let port: Box<dyn Port> = if cfg!(target_arch = "aarch64") { device::open() } else { Box::new(sim::port()) };
        let mut link = Link::new(port);
        let t = TIMING;
        crate::unitv::work_out_expected();
        let mut power = Power::Off;
        // The camera answered since power on; the rate settled; its program
        // compared with this image's; what was last asked of it.
        let (mut answered, mut settled, mut checked) = (false, false, false);
        let mut asked = String::new();
        let mut last_hello = Instant::now() - Duration::from_secs(1);
        let (mut last_sent, mut last_frame) = (Instant::now(), Instant::now());
        // Since when the link was quiet, when it was lost (to tell a camera
        // that started again by itself); and an update just made it start
        // again (that start is not counted against it).
        let mut lost_at: Option<Instant> = None;
        let mut updated = false;
        let mut window = (Instant::now(), 0u64, 0u64, 0u64, 0u64);
        let mut seen_bad = 0u64;
        // When the 5 V last went off: it stays off a moment before it comes
        // back, so the K210 starts afresh (at 115 200) rather than riding a
        // short gap on its capacitors at the rate it had.
        let mut off_at: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let (leases, chosen, gen) = {
                let s = hub.shared.lock().unwrap_or_else(|e| e.into_inner());
                (s.leases.clone(), super::chosen() && !HOLD_OFF.load(Ordering::Acquire), CHANGED.load(Ordering::Acquire))
            };
            // Not the camera's (any more, or for now): unpowered and let go
            // of at once, no warm time.
            if !chosen && power.powered() {
                link.port.power(false);
                off_at = Some(now);
                power = Power::Off;
                (answered, settled) = (false, false);
                let five = link.port.five_volt();
                note(hub, |d| (d.powered_at, d.five_volt) = (None, five));
            }
            let wanted = !leases.is_empty() && chosen;
            // Not to be powered (the port is the console, or not free yet:
            // a login or the console still on it): said so, and nothing more.
            if wanted && power.would_power(now) {
                if let Err(why) = link.port.ready() {
                    let trouble = link.port.trouble();
                    note(hub, |d| d.port = trouble);
                    set_status(hub, why);
                    wait(hub, Duration::from_secs(2), gen);
                    continue;
                }
            }
            let was_waking = matches!(power, Power::Waking { .. });
            match power.step(now, wanted, answered && was_waking) {
                Switch::On => {
                    if let Some(rest) = off_at.and_then(|at| t.off_for.checked_sub(at.elapsed())) {
                        std::thread::sleep(rest);
                    }
                    link.port.power(true);
                    link.set_baud(START_BAUD);
                    (answered, settled, checked) = (false, false, false);
                    asked.clear();
                    link.decoder = frame::Decoder::default();
                    (seen_bad, lost_at, updated) = (0, None, false);
                    let (five, trouble) = (link.port.five_volt(), link.port.trouble());
                    note(hub, |d| {
                        (d.powered_at, d.five_volt, d.port, d.baud) = (Some(Instant::now()), five, trouble, START_BAUD);
                        d.wakes += 1;
                        if five == Some(false) {
                            d.last_error = Some((Instant::now(), "the Grove 5 V did not come on".into()));
                        }
                    });
                }
                Switch::Off => {
                    link.port.power(false);
                    off_at = Some(now);
                    (answered, settled) = (false, false);
                    let five = link.port.five_volt();
                    let failed = matches!(power, Power::Failed { .. });
                    note(hub, |d| {
                        (d.powered_at, d.five_volt) = (None, five);
                        if failed {
                            d.unanswered += 1;
                            d.last_error = Some((now, format!("no answer within {} s of power on", t.answer.as_secs())));
                        }
                    });
                }
                Switch::Keep => {}
            }
            let absent = hub.shared.lock().map(|s| absent_words(&s.diag)).unwrap_or_default();
            set_status(
                hub,
                match &power {
                    Power::Off if HOLD_OFF.load(Ordering::Acquire) => Status::PortBusy,
                    Power::Off if !chosen => Status::PortIsConsole,
                    Power::Off => Status::Asleep,
                    Power::Waking { since } => Status::Waking { since: *since },
                    Power::On { took } | Power::Warm { took, .. } => Status::Ready { baud: link.baud, woke_ms: took.as_millis() as u32 },
                    Power::Failed { .. } => Status::Absent(absent),
                },
            );
            if !power.powered() {
                wait(hub, if wanted { Duration::from_millis(500) } else { Duration::from_secs(30) }, gen);
                continue;
            }
            // Hello until it answers.
            if !answered && last_hello.elapsed() >= Duration::from_millis(250) {
                link.send(frame::HELLO, HELLO_PAYLOAD);
                (last_hello, last_sent) = (Instant::now(), Instant::now());
            }
            for f in link.read(Duration::from_millis(40)) {
                last_frame = Instant::now();
                window.1 += (f.payload.len() + frame::HEAD + frame::TAIL) as u64;
                match f.kind {
                    frame::HELLO_BACK => {
                        let first = !answered;
                        answered = true;
                        let hello = Hello::parse(&f.payload);
                        if let Ok(mut s) = hub.shared.lock() {
                            s.about = Some(String::from_utf8_lossy(&f.payload).into_owned());
                            let d = &mut s.diag;
                            if let (true, false, Some(h)) = (first, updated, &hello) {
                                match lost_at {
                                    // Found again after it went quiet: it
                                    // started again by itself if it has been
                                    // up for less time than it was quiet.
                                    Some(quiet) if u128::from(h.up_ms) < quiet.elapsed().as_millis() => {
                                        d.restarts += 1;
                                        d.last_error = Some((Instant::now(), "the camera started again by itself".into()));
                                    }
                                    Some(_) => {}
                                    None => {
                                        let start = d.powered_at.map(|p| (p.elapsed().as_millis() as u32).saturating_sub(h.up_ms));
                                        d.start_ms = start;
                                        if start.is_some_and(|s| s > RESTARTED_MS) {
                                            d.restarts += 1;
                                            d.last_error = Some((Instant::now(), format!("{:.1} s from power to its program: it started more than once", f64::from(start.unwrap_or(0)) / 1000.0)));
                                        }
                                    }
                                }
                            }
                            if hello.is_some() {
                                (d.hello, d.hello_at) = (hello.clone(), Some(Instant::now()));
                            }
                        }
                        (lost_at, updated) = (None, false);
                        if !settled {
                            let c = climb(&mut link, &t);
                            note(hub, |d| {
                                d.baud = c.baud;
                                d.baud_tried = d.baud_tried.max(c.tried);
                                if let Some(fail) = c.failed {
                                    d.baud_failed = Some(fail);
                                    d.last_error = Some((Instant::now(), format!("{} did not hold · {} in use", baud_words(fail), baud_words(c.baud))));
                                }
                            });
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
                    frame::LOG => fault(hub, format!("the camera said: {}", String::from_utf8_lossy(&f.payload))),
                    _ => {}
                }
            }
            // Its program against this image's, once each time it answers
            // after power on (or when the status page asks again).
            if RECHECK.swap(false, Ordering::AcqRel) {
                checked = false;
            }
            if answered && settled && !checked {
                checked = true;
                let hello = hub.shared.lock().ok().and_then(|s| s.diag.hello.clone());
                if let Some(h) = hello {
                    if bring_up_to_date(hub, &mut link, &h, &t) {
                        // It started again at 115 200: found again, and its
                        // rate climbed afresh.
                        (answered, settled, updated) = (false, false, true);
                        asked.clear();
                        last_frame = Instant::now();
                    }
                }
            }
            // Ask for what the leases want, once it answers (and again when that changes).
            let want = wants(&leases);
            if answered && settled && (want != asked) && wanted {
                link.send(frame::WANT, want.as_bytes());
                (asked, last_sent) = (want, Instant::now());
            }
            if !wanted && answered && !asked.is_empty() {
                // Warm: it keeps quiet meanwhile.
                link.send(frame::WANT, br#"{"faces":0,"codes":0,"tags":0,"blobs":0,"preview":0}"#);
                (asked, last_sent) = (String::new(), Instant::now());
            }
            // Something every two seconds, whatever arrives: main.py goes
            // back to 115 200 after five without a frame from here, and a
            // quiet camera answers the ping. One quiet too long is lost: both
            // back to 115 200, and found again (or powered off and on, if it
            // does not answer in time).
            if answered && last_sent.elapsed() > Duration::from_secs(2) {
                link.send(frame::PING, b"alive");
                last_sent = Instant::now();
            }
            if answered && last_frame.elapsed() > LOST_AFTER {
                let at = link.baud;
                (answered, settled, lost_at) = (false, false, Some(last_frame));
                asked.clear();
                link.set_baud(START_BAUD);
                link.decoder = frame::Decoder::default();
                seen_bad = 0;
                power = Power::Waking { since: Instant::now() };
                note(hub, |d| {
                    d.lost += 1;
                    d.baud = START_BAUD;
                    d.last_error = Some((Instant::now(), format!("the link went quiet at {}: finding it again", baud_words(at))));
                });
            }
            // The figures, every second; too many frames refused at this
            // rate: one step slower.
            if window.0.elapsed() >= Duration::from_secs(1) {
                let secs = window.0.elapsed().as_secs_f32();
                let refused = link.decoder.bad.saturating_sub(window.4);
                let five = link.port.five_volt();
                if let Ok(mut s) = hub.shared.lock() {
                    s.rates = Rates { bytes_per_s: window.1 as f32 / secs, previews_per_s: window.2 as f32 / secs, results_per_s: window.3 as f32 / secs, refused: link.decoder.bad };
                    let d = &mut s.diag;
                    (d.bytes_in, d.bytes_out, d.frames, d.refused, d.junk, d.five_volt) = (link.got, link.sent, link.decoder.good, link.decoder.bad, link.decoder.junk, five);
                    if link.decoder.bad > seen_bad {
                        let n = link.decoder.bad - seen_bad;
                        d.last_error = Some((Instant::now(), format!("{n} frame{} refused (header or CRC) at {}", if n == 1 { "" } else { "s" }, baud_words(link.baud))));
                    }
                }
                seen_bad = link.decoder.bad;
                if refused >= 3 && settled && link.baud > START_BAUD {
                    let slower = BAUDS.iter().copied().find(|b| *b < link.baud).unwrap_or(START_BAUD);
                    let from = link.baud;
                    if !step_to(&mut link, slower, &t) {
                        // Not even the step down: found again from the start.
                        link.set_baud(START_BAUD);
                        (answered, settled) = (false, false);
                    }
                    let now_at = link.baud;
                    note(hub, |d| {
                        d.baud = now_at;
                        d.baud_failed = Some(from);
                        d.last_error = Some((Instant::now(), format!("too many frames refused at {}: {} now", baud_words(from), baud_words(now_at))));
                    });
                    asked.clear();
                }
                window = (Instant::now(), 0, 0, 0, link.decoder.bad);
            } else if let Ok(mut s) = hub.shared.lock() {
                // The byte counts live for the status page.
                (s.diag.bytes_in, s.diag.bytes_out) = (link.got, link.sent);
            }
        }
    }

    /// The camera's program compared with this image's, and sent over the
    /// link when older (once each start of the shell for one program).
    /// Whether the camera was touched (it has started again).
    fn bring_up_to_date(hub: &Hub, link: &mut Link, hello: &Hello, t: &Timing) -> bool {
        let expected = crate::unitv::expected();
        let (program, model) = match update::plan(hello, expected.as_ref()) {
            update::Plan::Nothing => {
                note(hub, |d| {
                    if matches!(d.update, Update::UsbOnce(_)) {
                        d.update = Update::Idle;
                    }
                });
                return false;
            }
            update::Plan::UsbOnce(why) => {
                let fresh = hub.shared.lock().is_ok_and(|s| s.diag.update != Update::UsbOnce(why.clone()));
                note(hub, |d| d.update = Update::UsbOnce(why.clone()));
                if fresh {
                    crate::notify::post(crate::notify::Notification::new("Camera", "developer_board", "smart camera").body("update on the camera · USB-A, once").tag("unitv").action("unitv:install"));
                }
                return false;
            }
            update::Plan::Send { program, model } => (program, model),
        };
        let Some(expected) = expected else { return false };
        let target = format!("{}{}", expected.main, expected.model);
        {
            let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
            if tried.as_deref() == Some(target.as_str()) {
                return false;
            }
            *tried = Some(target);
        }
        let payload = match crate::unitv::Payload::program() {
            Ok(p) => p,
            Err(why) => {
                note(hub, |d| d.update = Update::Failed(why));
                return false;
            }
        };
        let firmware = hello.stamp.as_ref().map(|s| s.firmware.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| format!("MaixPy {} (as found)", hello.maixpy));
        let stamp = crate::unitv::Stamp { firmware, ..expected.clone() };
        let task = crate::liveactivity::task(crate::liveactivity::Kind::Download, "Smart camera", "updating its program");
        let outcome = update::run(link, &payload, &stamp, program, model, t, &mut |u| {
            match &u {
                Update::Sending { file, done } => {
                    task.progress(*done);
                    task.detail(format!("sending {file}"));
                }
                Update::Restarting => task.detail("starting it again"),
                _ => {}
            }
            note(hub, |d| d.update = u);
            crate::platform::wake();
        });
        let release = expected.catalyst.clone();
        let (state, said) = match outcome {
            Ok(sent) if sent.hello.stamp.as_ref().is_some_and(|s| s.same_program(&expected)) => (Update::Done(release.clone()), format!("smart camera updated · Catalyst {release}'s program")),
            Ok(sent) if sent.hello.rolled_back => (Update::RolledBack, "smart camera update did not start · its old program is back".to_owned()),
            Ok(_) => (Update::Failed("it answered with another program".into()), "smart camera update did not take".to_owned()),
            Err(why) => (Update::Failed(why.clone()), format!("smart camera update stopped · {why}")),
        };
        if matches!(state, Update::Done(_)) {
            task.done("updated");
        } else {
            drop(task);
            crate::notify::post(crate::notify::Notification::new("Camera", "developer_board", "smart camera").body(said.clone()).tag("smartcam").action("smartcam:status"));
        }
        note(hub, |d| {
            if !matches!(state, Update::Done(_)) {
                d.last_error = Some((Instant::now(), said.clone()));
            }
            d.update = state;
        });
        *SAID.lock().unwrap_or_else(|e| e.into_inner()) = Some(said);
        crate::platform::wake();
        true
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

    /// The Grove 5 V's switch (gpio-leds on the board's I/O expander, active
    /// high: 1 is on).
    const FIVE_VOLT: &str = "/sys/class/leds/grove_5v_out/brightness";

    /// ttyAMA0, raw, and the port's power through catalyst-grove (which
    /// refuses it unless the port is safely the camera's).
    pub struct Uart {
        fd: i32,
        trouble: Option<String>,
    }

    pub fn open() -> Box<dyn Port> {
        Box::new(Uart { fd: -1, trouble: None })
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
                    self.trouble = Some(format!("/dev/ttyAMA0 would not open: {}", std::io::Error::last_os_error()));
                    return false;
                }
                // All raw, whatever a getty left set: no flow control either
                // way (no XON/XOFF in the bytes, no RTS/CTS), no line
                // editing, no echo, no CR/LF turned, no break read as a
                // signal; 8 bits, no parity, one stop bit.
                let mut t: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(fd, &mut t) != 0 {
                    self.trouble = Some(format!("ttyAMA0's settings would not read: {}", std::io::Error::last_os_error()));
                }
                libc::cfmakeraw(&mut t);
                t.c_iflag = 0;
                t.c_oflag = 0;
                t.c_lflag = 0;
                t.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::cfsetispeed(&mut t, libc::B115200);
                libc::cfsetospeed(&mut t, libc::B115200);
                if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                    self.trouble = Some(format!("ttyAMA0 would not go raw: {}", std::io::Error::last_os_error()));
                }
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
            // Why not, for the status page: catalyst-grove's own account.
            if let Ok(out) = std::process::Command::new("catalyst-grove").arg("status").output() {
                let said = String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
                self.trouble = Some(format!("catalyst-grove: {said}"));
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
            let out = std::process::Command::new("catalyst-grove").args(["power", if on { "on" } else { "off" }]).output();
            let given = out.as_ref().is_ok_and(|o| o.status.success());
            if on && given {
                self.trouble = None;
            }
            if on && !given {
                let why = out.map(|o| String::from_utf8_lossy(&o.stderr).trim().to_owned()).unwrap_or_default();
                self.trouble = Some(if why.is_empty() { "catalyst-grove would not power the port".into() } else { why });
            }
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

        fn five_volt(&mut self) -> Option<bool> {
            match std::fs::read_to_string(FIVE_VOLT) {
                Ok(level) => Some(level.trim() != "0"),
                Err(_) => {
                    self.trouble = Some("no switch for the Grove 5 V (grove_5v_out): the board's driver did not load".into());
                    None
                }
            }
        }

        fn trouble(&mut self) -> Option<String> {
            self.trouble.clone()
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
/// sees them over the wire. It starts [`sim::BOOT`] after power on, keeps
/// quiet until spoken to, answers the same frames, moves to the baud rates
/// asked (bytes then take as long as they would at that rate, so the
/// preview's rate is the real link's; a rate it does not hold garbles both
/// ways), takes a program over the link into a /flash of its own with
/// main.py's and boot.py's rules (the trial, the rollback), and sees a pit:
/// a face drifting about, a QR code with a scouting note, AprilTag 7 on the
/// field wall, an orange game piece. Now and then it sends a burst of junk,
/// as a camera starting up does, which the link skips. [`sim::Spec`] makes
/// it awkward for the tests (a slow start, a slow link, a chunk lost, the
/// power cut part way, a new program that never starts);
/// `CATALYST_SIM_SMARTCAM` does in the simulator (old, v1, slow, slowlink,
/// brownout, silent, sensor).
pub mod sim {
    use super::*;
    use std::collections::HashMap;
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

    /// How the stand-in behaves.
    #[derive(Clone, Debug)]
    pub struct Spec {
        /// Power to listening.
        pub boot: Duration,
        /// The fastest rate it holds; above it, both ways garble.
        pub max_baud: u32,
        /// Its program's VERSION (1: 0.9.3's, which takes no update over the link).
        pub version: u32,
        /// Its /flash/catalyst.json.
        pub stamp: Option<String>,
        pub full: bool,
        pub sensor_fails: bool,
        /// No program answers at all.
        pub silent: bool,
        /// A program sent over the link never starts.
        pub breaks_new: bool,
        /// The nth chunk (from 1) lost on the wire.
        pub drop_chunk: Option<u32>,
        /// The nth chunk written wrong (its file fails its CRC).
        pub bad_write: Option<u32>,
        /// The power gone once a file has this many bytes, the first time.
        pub cut_at: Option<usize>,
        /// It starts again by itself this long after each start (short of power).
        pub restarts_after: Option<Duration>,
    }

    impl Spec {
        /// No program of Catalyst's yet: a stamp of none.
        pub fn bare() -> Spec {
            Spec { boot: BOOT, max_baud: 3_000_000, version: 2, stamp: None, full: true, sensor_fails: false, silent: false, breaks_new: false, drop_chunk: None, bad_write: None, cut_at: None, restarts_after: None }
        }

        /// This image's program on it, up to date (the simulator's).
        pub fn current() -> Spec {
            let stamp = crate::unitv::Payload::load().ok().map(|p| p.stamp("MaixPy 0.6.3 (simulated)").json());
            Spec { stamp, ..Spec::bare() }
        }

        /// An older release's program on it.
        pub fn old() -> Spec {
            Spec { stamp: Some(r#"{"catalyst":"0.9.3","main":"0a1b2c3d4e5f","model":"000000000000","firmware":"MaixPy 0.6.3 (installed by Catalyst)"}"#.into()), ..Spec::bare() }
        }

        /// As CATALYST_SIM_SMARTCAM says (else up to date).
        pub fn from_env() -> Spec {
            match std::env::var("CATALYST_SIM_SMARTCAM").unwrap_or_default().as_str() {
                "old" => Spec::old(),
                "v1" => Spec { version: 1, ..Spec::old() },
                "slow" => Spec { boot: Duration::from_secs(7), ..Spec::current() },
                "slowlink" => Spec { max_baud: START_BAUD, ..Spec::current() },
                "brownout" => Spec { restarts_after: Some(Duration::from_secs(6)), ..Spec::current() },
                "silent" => Spec { silent: true, ..Spec::current() },
                "sensor" => Spec { sensor_fails: true, ..Spec::current() },
                _ => Spec::current(),
            }
        }
    }

    /// The stand-in's /flash, as main.py and boot.py keep it.
    #[derive(Debug, Default)]
    pub struct Flash {
        pub files: HashMap<String, Vec<u8>>,
        pub old: HashMap<String, Vec<u8>>,
        /// Files on their way: size, CRC, id, what has arrived.
        pub new: HashMap<String, (usize, u32, String, Vec<u8>)>,
        /// A new program's starts, and its files (and whether each was there before).
        pub trial: Option<(u32, Vec<(String, bool)>)>,
        pub rolled_back: bool,
        /// The program running came over the link.
        pub from_link: bool,
        pub starts: u32,
        /// Files begun again part way: their name and where.
        pub resumed: Vec<(String, usize)>,
        pub chunks: u32,
        /// Files refused at their end (their CRC).
        pub damaged: u32,
        cut: bool,
    }

    impl Flash {
        fn with(spec: &Spec) -> Flash {
            let mut f = Flash::default();
            f.files.insert("main.py".into(), b"# the program it came with\n".to_vec());
            f.files.insert("boot.py".into(), b"# its boot.py\n".to_vec());
            if let Some(s) = &spec.stamp {
                f.files.insert("catalyst.json".into(), s.as_bytes().to_vec());
            }
            f
        }

        /// A start: boot.py's count, and the old files back on the third
        /// start of a program never heard. Whether a program answers.
        fn start(&mut self, spec: &Spec) -> bool {
            self.starts += 1;
            if let Some((n, list)) = self.trial.take() {
                if n >= 2 {
                    for (name, had) in &list {
                        match (had, self.old.remove(name)) {
                            (true, Some(old)) => {
                                self.files.insert(name.clone(), old);
                            }
                            (true, None) => {}
                            (false, _) => {
                                self.files.remove(name);
                            }
                        }
                    }
                    (self.rolled_back, self.from_link) = (true, false);
                } else {
                    self.trial = Some((n + 1, list));
                }
            }
            !spec.silent && !(spec.breaks_new && self.from_link)
        }

        fn commit(&mut self, names: &[String], stamp: &[u8]) -> Result<(), String> {
            if let Some(missing) = names.iter().find(|n| !self.new.contains_key(*n)) {
                return Err(format!("missing {missing}"));
            }
            let mut list = Vec::new();
            for name in names {
                let (.., data) = self.new.remove(name).unwrap_or_default();
                list.push((name.clone(), self.put(name, data)));
            }
            list.push(("catalyst.json".to_owned(), self.put("catalyst.json", stamp.to_vec())));
            self.trial = Some((0, list));
            self.rolled_back = false;
            self.from_link |= names.iter().any(|n| n == "main.py");
            Ok(())
        }

        /// In place, what was there kept as .old: whether there was.
        fn put(&mut self, name: &str, data: Vec<u8>) -> bool {
            match self.files.insert(name.to_owned(), data) {
                Some(was) => {
                    self.old.insert(name.to_owned(), was);
                    true
                }
                None => {
                    self.old.remove(name);
                    false
                }
            }
        }
    }

    pub struct SimPort {
        to_cam: mpsc::Sender<(u32, Vec<u8>)>,
        from_cam: mpsc::Receiver<(u32, Vec<u8>)>,
        held: Vec<u8>,
        baud: u32,
        powered: Arc<Mutex<Option<Instant>>>,
    }

    /// The simulator's stand-in, as CATALYST_SIM_SMARTCAM says.
    pub fn port() -> SimPort {
        port_with(Spec::from_env()).0
    }

    /// A stand-in as `spec` says, and its /flash to look into.
    pub fn port_with(spec: Spec) -> (SimPort, Arc<Mutex<Flash>>) {
        let (to_cam, cam_rx) = mpsc::channel::<(u32, Vec<u8>)>();
        let (cam_tx, from_cam) = mpsc::channel::<(u32, Vec<u8>)>();
        let powered = Arc::new(Mutex::new(None));
        let flash = Arc::new(Mutex::new(Flash::with(&spec)));
        let (p, f) = (powered.clone(), flash.clone());
        std::thread::Builder::new().name("smartcam-sim".into()).spawn(move || Cam::new(spec, f, cam_tx).run(cam_rx, p)).ok();
        (SimPort { to_cam, from_cam, held: Vec::new(), baud: START_BAUD, powered }, flash)
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
            self.baud = baud;
            true
        }
        fn write(&mut self, bytes: &[u8]) -> bool {
            // Sent at a rate: the camera hears it only at the same.
            self.to_cam.send((self.baud, bytes.to_vec())).is_ok()
        }
        fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize {
            if self.held.is_empty() {
                let mut take = |(rate, bytes): (u32, Vec<u8>)| {
                    if rate == self.baud {
                        self.held.extend_from_slice(&bytes);
                    } else {
                        // Heard at the wrong rate: a few bytes of nothing.
                        self.held.extend_from_slice(&vec![0xFF; bytes.len().div_ceil(8)]);
                    }
                };
                if let Ok(chunk) = self.from_cam.recv_timeout(wait) {
                    take(chunk);
                }
                while let Ok(more) = self.from_cam.try_recv() {
                    take(more);
                }
            }
            let n = self.held.len().min(buf.len());
            buf[..n].copy_from_slice(&self.held[..n]);
            self.held.drain(..n);
            n
        }
        fn five_volt(&mut self) -> Option<bool> {
            Some(self.powered.lock().map(|p| p.is_some()).unwrap_or(false))
        }
    }

    /// The stand-in's program (as tools/unitv/main.py does it).
    struct Cam {
        spec: Spec,
        flash: Arc<Mutex<Flash>>,
        tx: mpsc::Sender<(u32, Vec<u8>)>,
        decoder: frame::Decoder,
        baud: u32,
        before: u32,
        since_baud: Option<Instant>,
        heard_at: Instant,
        want: (bool, bool, bool, bool, bool),
        seq: u8,
        n: u32,
        /// Powered (or started again by itself) at; listening from.
        on_since: Option<Instant>,
        booted_at: Option<Instant>,
        alive: bool,
        last_frame: Instant,
        started: Instant,
        file: Option<String>,
    }

    impl Cam {
        fn new(spec: Spec, flash: Arc<Mutex<Flash>>, tx: mpsc::Sender<(u32, Vec<u8>)>) -> Cam {
            let now = Instant::now();
            Cam { spec, flash, tx, decoder: frame::Decoder::default(), baud: START_BAUD, before: START_BAUD, since_baud: None, heard_at: now, want: (false, false, false, false, false), seq: 0, n: 0, on_since: None, booted_at: None, alive: false, last_frame: now, started: now, file: None }
        }

        /// From its first line again (power, or a reset of its own).
        fn reset(&mut self, at: Option<Instant>) {
            (self.on_since, self.booted_at, self.alive) = (at, None, false);
            (self.baud, self.before, self.since_baud) = (START_BAUD, START_BAUD, None);
            self.want = (false, false, false, false, false);
            self.decoder = frame::Decoder::default();
            self.file = None;
        }

        /// Sent at its rate: as long as the bytes take on the wire; garbled
        /// at a rate it does not hold.
        fn send(&mut self, kind: u8, payload: &[u8]) {
            self.seq = self.seq.wrapping_add(1);
            let mut bytes = frame::encode(kind, self.seq, payload);
            std::thread::sleep(Duration::from_secs_f64(bytes.len() as f64 * 10.0 / f64::from(self.baud)));
            if self.baud > self.spec.max_baud {
                bytes.iter_mut().for_each(|b| *b = b.rotate_left(3) ^ 0x5A);
            }
            self.tx.send((self.baud, bytes)).ok();
        }

        fn hello(&mut self) -> String {
            // main.py's first line comes before it listens (the sensor, the model).
            let up = self.booted_at.map_or(0, |b| b.elapsed().as_millis() + self.spec.boot.as_millis().min(880));
            let f = self.flash.lock().unwrap_or_else(|e| e.into_inner());
            let stamp = f.files.get("catalyst.json").map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_else(|| "null".into());
            let trial = f.trial.as_ref().map_or("null".to_owned(), |(n, _)| n.to_string());
            let (v, rolled) = (self.spec.version, f.rolled_back);
            let sensor = if self.spec.sensor_fails { "failed: Reset Failed" } else { "ok" };
            if v < 2 {
                return format!(r#"{{"v":1,"fw":"catalyst-unitv 1 (simulated)","kpu":"face yolo2 320x240","crc":"native","fov":80,"w":320,"h":240,"up_ms":{up},"usb":false,"stamp":{stamp}}}"#);
            }
            format!(
                r#"{{"v":{v},"fw":"catalyst-unitv {v} (simulated)","upd":1,"mp":"0.6.3","full":{},"sensor":"{sensor}","kpu":"face yolo2 320x240 (flash 0x300000)","crc":"native","fov":80,"w":320,"h":240,"up_ms":{up},"listen_ms":880,"usb":false,"cwd":"/flash","free":2850000,"trial":{trial},"rolled_back":{rolled},"stamp":{stamp}}}"#,
                u8::from(self.spec.full)
            )
        }

        fn run(mut self, rx: mpsc::Receiver<(u32, Vec<u8>)>, powered: Arc<Mutex<Option<Instant>>>) {
            let mut seen_power: Option<Instant> = None;
            loop {
                let power = *powered.lock().unwrap_or_else(|e| e.into_inner());
                if power != seen_power {
                    seen_power = power;
                    self.reset(power);
                }
                let Some(since) = self.on_since else {
                    if !drain(&rx) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                    continue;
                };
                if since.elapsed() < self.spec.boot {
                    if !drain(&rx) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                if self.booted_at.is_none() {
                    self.booted_at = Some(Instant::now());
                    self.heard_at = Instant::now();
                    self.alive = self.flash.lock().map(|mut f| f.start(&self.spec)).unwrap_or(false);
                    // The K210's boot ROM says something at its own rate: junk to the link.
                    self.tx.send((0, b"\x00\xffK210 boot\r\n\xc5".to_vec())).ok();
                }
                // Short of power: it starts again by itself, now and then.
                if self.spec.restarts_after.is_some_and(|every| self.booted_at.is_some_and(|b| b.elapsed() >= every)) {
                    self.reset(Some(Instant::now()));
                    continue;
                }
                // What the handheld sent, heard only at the rate this listens
                // at, and only at a rate it holds.
                loop {
                    match rx.try_recv() {
                        Ok((rate, bytes)) => {
                            if !self.alive || rate != self.baud || self.baud > self.spec.max_baud {
                                continue;
                            }
                            for f in self.decoder.push(&bytes) {
                                self.heard_at = Instant::now();
                                if !self.hear(f) {
                                    break;
                                }
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return,
                    }
                    if self.booted_at.is_none() {
                        break;
                    }
                }
                if self.booted_at.is_none() {
                    continue;
                }
                // No frame at a new rate for a second: back to the rate
                // before; nothing for five at a raised rate: to the start.
                if self.since_baud.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                    (self.baud, self.since_baud) = (self.before, None);
                }
                if self.baud != START_BAUD && self.since_baud.is_none() && self.heard_at.elapsed() > Duration::from_secs(5) {
                    self.baud = START_BAUD;
                }
                self.look();
            }
        }

        /// One frame from the handheld; false if it started again (the rest
        /// of what arrived is lost with it).
        fn hear(&mut self, f: frame::Frame) -> bool {
            match f.kind {
                frame::HELLO => {
                    self.flash.lock().unwrap_or_else(|e| e.into_inner()).trial = None;
                    let hello = self.hello();
                    self.send(frame::HELLO_BACK, hello.as_bytes());
                }
                frame::BAUD if f.payload.len() >= 4 => {
                    let to = u32::from_le_bytes([f.payload[0], f.payload[1], f.payload[2], f.payload[3]]);
                    self.send(frame::BAUD_BACK, &f.payload);
                    (self.before, self.baud, self.since_baud) = (self.baud, to, Some(Instant::now()));
                }
                frame::PING => {
                    self.since_baud = None;
                    self.send(frame::PONG, &f.payload);
                }
                frame::WANT => {
                    let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
                    let on = |k: &str| v[k].as_u64().unwrap_or(0) != 0;
                    self.want = (on("faces"), on("codes"), on("tags"), on("blobs"), on("preview"));
                    self.send(frame::WANT_BACK, &f.payload);
                }
                frame::PUT => {
                    self.want = (false, false, false, false, false);
                    let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
                    let (name, size, crc, id) = (v["name"].as_str().unwrap_or("").to_owned(), v["size"].as_u64().unwrap_or(0) as usize, v["crc"].as_u64().unwrap_or(0) as u32, v["id"].as_str().unwrap_or("").to_owned());
                    let said = if ["main.py", "boot.py", "face.kmodel"].contains(&name.as_str()) {
                        let mut fl = self.flash.lock().unwrap_or_else(|e| e.into_inner());
                        let have = match fl.new.get(&name) {
                            Some((s, c, i, data)) if *s == size && *c == crc && *i == id && data.len() <= size => data.len(),
                            _ => {
                                fl.new.insert(name.clone(), (size, crc, id, Vec::new()));
                                0
                            }
                        };
                        if have > 0 {
                            fl.resumed.push((name.clone(), have));
                        }
                        self.file = Some(name.clone());
                        format!(r#"{{"name":"{name}","have":{have}}}"#)
                    } else {
                        self.file = None;
                        format!(r#"{{"name":"{name}","err":"name"}}"#)
                    };
                    self.send(frame::PUT_BACK, said.as_bytes());
                }
                frame::CHUNK if f.payload.len() >= 4 => {
                    let at = u32::from_le_bytes([f.payload[0], f.payload[1], f.payload[2], f.payload[3]]) as usize;
                    let mut fl = self.flash.lock().unwrap_or_else(|e| e.into_inner());
                    fl.chunks += 1;
                    let count = fl.chunks;
                    if self.spec.drop_chunk == Some(count) {
                        return true;
                    }
                    let Some((size, _, _, data)) = self.file.as_ref().and_then(|n| fl.new.get_mut(n)) else {
                        drop(fl);
                        self.send(frame::CHUNK_BACK, &u32::MAX.to_le_bytes());
                        return true;
                    };
                    let mut piece = f.payload[4..].to_vec();
                    if at == data.len() && data.len() + piece.len() <= *size {
                        if self.spec.bad_write == Some(count) {
                            piece[0] ^= 0x01;
                        }
                        data.extend_from_slice(&piece);
                    }
                    let have = data.len();
                    let cut = self.spec.cut_at.is_some_and(|c| have >= c) && !fl.cut;
                    if cut {
                        // The power gone: what arrived is kept, the rest lost.
                        fl.cut = true;
                        drop(fl);
                        self.reset(Some(Instant::now()));
                        return false;
                    }
                    drop(fl);
                    self.send(frame::CHUNK_BACK, &(have as u32).to_le_bytes());
                }
                frame::DONE => {
                    let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
                    let name = v["name"].as_str().unwrap_or("").to_owned();
                    let mut fl = self.flash.lock().unwrap_or_else(|e| e.into_inner());
                    let ok = fl.new.get(&name).is_some_and(|(size, crc, _, data)| data.len() == *size && frame::crc32(data) == *crc);
                    if !ok {
                        fl.new.remove(&name);
                        fl.damaged += 1;
                    }
                    drop(fl);
                    self.file = None;
                    let said = if ok { format!(r#"{{"name":"{name}","ok":1}}"#) } else { format!(r#"{{"name":"{name}","ok":0,"err":"crc"}}"#) };
                    self.send(frame::DONE_BACK, said.as_bytes());
                }
                frame::COMMIT => {
                    let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap_or_default();
                    let names: Vec<String> = v["files"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_owned)).collect();
                    let stamp = v["stamp"].to_string();
                    let done = self.flash.lock().unwrap_or_else(|e| e.into_inner()).commit(&names, stamp.as_bytes());
                    let said = match &done {
                        Ok(()) => r#"{"ok":1}"#.to_owned(),
                        Err(why) => format!(r#"{{"ok":0,"err":"{why}"}}"#),
                    };
                    self.send(frame::COMMIT_BACK, said.as_bytes());
                    if done.is_ok() {
                        // The new program from its first line.
                        self.reset(Some(Instant::now()));
                        return false;
                    }
                }
                _ => {}
            }
            true
        }

        /// Its looking, at about 15 frames a second while asked to.
        fn look(&mut self) {
            let w = self.want;
            let any = w.0 || w.1 || w.2 || w.3 || w.4;
            if !self.alive || self.spec.sensor_fails || !any || self.last_frame.elapsed() < Duration::from_millis(66) {
                std::thread::sleep(Duration::from_millis(8));
                return;
            }
            self.last_frame = Instant::now();
            self.n += 1;
            let n = self.n;
            let t = self.started.elapsed().as_secs_f32();
            let face = [0.42 + 0.12 * (t * 0.4).sin(), 0.28 + 0.04 * (t * 0.7).cos()];
            let mut json = format!(r#"{{"n":{n},"w":320,"h":240,"fps":15.0"#);
            if w.0 {
                json += &format!(r#","f":[[{:.0},{:.0},58,70,87]]"#, face[0] * 320.0, face[1] * 240.0);
            }
            if w.1 {
                json += r#","q":[[212,128,52,52,"SCOUT 5805 Q12 auto=3 tele=11 climb=deep"]]"#;
            }
            if w.2 {
                json += &format!(r#","a":[[7,{:.0},62,{:.0},-412,-88,2640]]"#, 70.0 + 6.0 * (t * 0.3).sin(), 120.0 * (t * 0.2).sin());
            }
            if w.3 {
                json += r#","b":[[0,40,176,34,28,640]]"#;
            }
            json.push('}');
            self.send(frame::RESULTS, json.as_bytes());
            // A burst of junk now and then (a hand on the cable).
            if n % 97 == 0 {
                self.tx.send((self.baud, vec![0xC5, 0x7A, 0x85, 0x01, 0xFF, 0xFF, 0x00, 1, 2, 3])).ok();
            }
            if w.4 {
                let pic = view(t, face);
                if let Some(jpeg) = jpeg(&pic) {
                    self.send(frame::PREVIEW, &jpeg);
                }
            }
        }
    }

    /// What arrived while it was not listening, dropped; false once the
    /// handheld's end is gone (the stand-in ends with it).
    fn drain(rx: &mpsc::Receiver<(u32, Vec<u8>)>) -> bool {
        loop {
            match rx.try_recv() {
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
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
mod tests;
