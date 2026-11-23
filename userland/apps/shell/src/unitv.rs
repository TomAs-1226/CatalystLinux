//! The smart camera's firmware and program, installed from the handheld over
//! USB, with no microSD card. The UnitV's USB-C goes into the handheld's
//! USB-A port; its K210 sits behind a USB serial bridge (an FTDI FT232 on
//! the UnitV M12, `0403:6001`; an FT2232 on other M5Stack boards and the
//! same two-port bridge Sipeed's CH552 boards show; a CH340 or CH9102 on
//! others), whose DTR and RTS lines reach the K210's reset and its boot pin.
//!
//! **Catalyst Coprocessor.** The firmware is Catalyst's own MaixPy
//! (tools/unitv/firmware: trimmed to the UnitV, sleeping delays, the
//! `catalyst` module's power and speech front end, a frozen boot that always
//! runs /flash/main.py and a safe program if it cannot). It goes on through
//! the ISP whenever the camera does not run it at this image's FIRMWARE
//! number; main.py and the models then come over the Grove port too.
//!
//! **Noticed.** A thread looks at the USB devices every two seconds. A K210
//! board (an FT2232, or a bridge whose name says M5Stack, UnitV, Sipeed or
//! Maix) is asked over MaixPy's REPL what it runs and what Catalyst put on
//! it (`/flash/catalyst.json`, the version stamp); the island then offers
//! "install on the camera" or "update on the camera". Settings › Camera ›
//! UnitV says the same and opens [`InstallScreen`].
//!
//! **Installed**, on the same thread, the page showing how far:
//! 1. The firmware, unless the camera already runs Catalyst Coprocessor at
//!    this image's number, through the K210's ISP ([`isp`], kflash.py's
//!    protocol): the boot ROM loads kflash.py's flash loader into SRAM,
//!    which writes the flash at 1.5 Mbaud (115 200 where the bridge will not).
//! 2. The face model at 0x300000, where main.py loads it, the same way.
//! 3. main.py and the keyword spotter (kws.kmodel) into the camera's
//!    /flash, through the raw REPL ([`repl`], mpremote's way), read back;
//!    an older Catalyst's /flash/boot.py taken away (the firmware's own boot
//!    does its work now).
//! 4. The stamp, then a soft reset, and a hello on the framed protocol over
//!    the same USB line: main.py answers there as on the Grove port.
//!
//! **Versioned.** The stamp names the firmware (by number), main.py, the
//! face model and the keyword spotter (each by its SHA-256, a dozen hex
//! digits). A later Catalyst whose firmware is newer says "update on the
//! camera · USB-A"; one whose program or models differ sends them over the
//! Grove port (or USB-A, when the camera is there).
//!
//! In the simulator a stand-in UnitV ([`stub`]) answers behind the same
//! bytes: `CATALYST_SIM_UNITV=bare|new|old|current` plugs one in.

pub mod isp;
pub mod repl;
pub mod serial;
#[cfg(any(test, not(target_arch = "aarch64")))]
pub mod stub;

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::smartcam::frame;
use isp::Wiring;
use serial::Serial;

/// What the image carries for the camera (catalyst-unitv).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub const DIR: &str = "/usr/share/catalyst/unitv";
/// Where main.py loads the face model from.
pub const MODEL_AT: u32 = 0x30_0000;
/// The flash loader's rate, where the bridge holds it (M5Stack's advice).
pub const FAST: u32 = 1_500_000;
const STAMP_FILE: &str = "/flash/catalyst.json";
/// How long each port is given to answer on MaixPy's REPL (a reset, then
/// MaixPy starting, about two seconds on a K210).
const PROBE_WAIT: Duration = if cfg!(test) { Duration::from_millis(800) } else { Duration::from_secs(5) };
/// What the install tidies on the camera. A pending update's trial ends
/// (this install is what runs now), an older Catalyst's boot.py goes (the
/// firmware's frozen boot runs main.py itself, and never a card's), the
/// leftovers of a transfer cut off go, and a face model sent over the Grove
/// port goes when the model is written to flash again. It still says
/// whether a card is in (the UnitV has no slot; another K210 board may).
const TIDY: &str = "import os\ndef _e(p):\n try:\n  os.stat(p)\n  return 1\n except OSError:\n  return 0\nfor n in ('catalyst.trial','catalyst.commit','catalyst.rolledback','boot.py','boot.py.old','boot.py.new','boot.py.part','main.py.new','main.py.part','face.kmodel.new','face.kmodel.part','kws.kmodel.new','kws.kmodel.part','kws.kmodel.old'@MODEL@):\n if _e('/flash/'+n):os.remove('/flash/'+n)\nc='none'\ntry:\n if 'sd' in os.listdir('/'):c='shim'\nexcept Exception:\n c='?'\nprint('CARD|'+c)";
/// What a probe prints: whether the camera runs Catalyst Coprocessor's
/// firmware at this image's number (or newer), with what main.py needs; its
/// MaixPy version; and the stamp. `@FW@` is the number ([`probe_code`]).
const PROBE: &str = "import sys\nok=1\ntry:\n import KPU,image,ujson\n from fpioa_manager import fm\n from machine import UART\n for n in ('find_apriltags','find_qrcodes','find_blobs'):\n  if not hasattr(image.Image,n):ok=0\n if not hasattr(KPU,'init_yolo2'):ok=0\n import catalyst\n if catalyst.FIRMWARE<@FW@:ok=0\nexcept Exception:\n ok=0\ntry:\n v='.'.join(str(x) for x in sys.implementation.version)\nexcept Exception:\n v='?'\ntry:\n s=open('/flash/catalyst.json').read()\nexcept Exception:\n s='{}'\nprint('CATALYST|%d|%s|%s'%(ok,v,s.replace('\\n','')))";

/// The probe, asking for this image's firmware number.
pub fn probe_code() -> String {
    PROBE.replace("@FW@", &firmware_number().max(1).to_string())
}

// ------------------------------------------------------------------ stamp ---

/// What Catalyst put on a camera: its version; the main.py, the face model
/// and the keyword spotter's model by their SHA-256's first twelve hex
/// digits; and the firmware it found or put, by name and number (Catalyst
/// Coprocessor's FIRMWARE; 0 for a MaixPy of Sipeed's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub catalyst: String,
    pub main: String,
    pub model: String,
    pub kws: String,
    pub firmware: String,
    pub fwv: u32,
}

impl Stamp {
    pub fn json(&self) -> String {
        serde_json::json!({"catalyst": self.catalyst, "main": self.main, "model": self.model, "kws": self.kws, "firmware": self.firmware, "fwv": self.fwv}).to_string()
    }

    pub fn from_value(v: &serde_json::Value) -> Option<Stamp> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
        Some(Stamp {
            catalyst: s("catalyst").unwrap_or_default(),
            main: s("main")?,
            model: s("model")?,
            kws: s("kws").unwrap_or_default(),
            firmware: s("firmware").unwrap_or_default(),
            fwv: v.get("fwv").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32,
        })
    }

    pub fn parse(text: &str) -> Option<Stamp> {
        Stamp::from_value(&serde_json::from_str(text).ok()?)
    }

    /// The same program and models (the firmware may differ).
    pub fn same_program(&self, other: &Stamp) -> bool {
        self.main == other.main && self.model == other.model && self.kws == other.kws
    }
}

fn short_sum(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// The files to install.
pub struct Payload {
    pub main: Vec<u8>,
    pub model: Vec<u8>,
    /// The keyword spotter (tools/unitv/speech/kws.kmodel).
    pub kws: Vec<u8>,
    pub firmware: Vec<u8>,
    pub firmware_name: String,
    /// Catalyst Coprocessor's firmware number (tools/unitv/firmware/FIRMWARE).
    pub firmware_version: u32,
    pub isp: Vec<u8>,
}

impl Payload {
    #[cfg(target_arch = "aarch64")]
    pub fn load() -> Result<Payload, String> {
        let read = |name: &str| std::fs::read(Path::new(DIR).join(name)).map_err(|_| format!("{DIR}/{name} is missing"));
        Ok(Payload {
            main: read("main.py")?,
            model: read("face.kmodel")?,
            kws: read("kws.kmodel")?,
            firmware: read("catalyst-coprocessor.bin")?,
            firmware_name: "Catalyst Coprocessor".into(),
            firmware_version: firmware_number(),
            isp: read("isp.bin")?,
        })
    }

    /// The simulator's: the real main.py and keyword spotter, and stand-ins
    /// of the others' sizes.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn load() -> Result<Payload, String> {
        let made = |n: usize, seed: u8| (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect::<Vec<u8>>();
        Ok(Payload {
            main: include_bytes!("../../../../tools/unitv/main.py").to_vec(),
            model: made(388_776, 7),
            kws: include_bytes!("../../../../tools/unitv/speech/kws.kmodel").to_vec(),
            firmware: made(1_844_800, 3),
            firmware_name: "Catalyst Coprocessor".into(),
            firmware_version: firmware_number(),
            isp: made(66_304, 11),
        })
    }

    /// Only what goes over the Grove port (main.py, the models): no
    /// firmware or loader read for it.
    #[cfg(target_arch = "aarch64")]
    pub fn program() -> Result<Payload, String> {
        let read = |name: &str| std::fs::read(Path::new(DIR).join(name)).map_err(|_| format!("{DIR}/{name} is missing"));
        Ok(Payload { main: read("main.py")?, model: read("face.kmodel")?, kws: read("kws.kmodel")?, firmware: Vec::new(), firmware_name: String::new(), firmware_version: firmware_number(), isp: Vec::new() })
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn program() -> Result<Payload, String> {
        Payload::load()
    }

    pub fn stamp(&self, firmware: &str) -> Stamp {
        Stamp { catalyst: crate::sysupdate::version(), main: short_sum(&self.main), model: short_sum(&self.model), kws: if self.kws.is_empty() { String::new() } else { short_sum(&self.kws) }, firmware: firmware.to_owned(), fwv: self.firmware_version }
    }
}

/// The firmware number this image carries (catalyst-unitv's
/// firmware.version; in the simulator, tools/unitv/firmware/FIRMWARE).
pub fn firmware_number() -> u32 {
    #[cfg(target_arch = "aarch64")]
    let text = std::fs::read_to_string(Path::new(DIR).join("firmware.version")).unwrap_or_default();
    #[cfg(not(target_arch = "aarch64"))]
    let text = include_str!("../../../../tools/unitv/firmware/FIRMWARE").to_owned();
    text.trim().parse().unwrap_or(0)
}

/// This image's program and model, as a stamp (worked out once, off the
/// screen's thread).
pub fn expected() -> Option<Stamp> {
    EXPECTED.get().cloned().flatten()
}

static EXPECTED: OnceLock<Option<Stamp>> = OnceLock::new();

/// Worked out now if it has not been (off the screen's thread: it reads the
/// image's files).
pub fn work_out_expected() {
    EXPECTED.get_or_init(|| Payload::load().ok().map(|p| p.stamp("")));
}

/// MaixPy's version as the firmware file names it ("0.6.3").
fn firmware_version(name: &str) -> String {
    name.split('_').find_map(|p| p.strip_prefix('v')).unwrap_or("0.6.3").to_owned()
}

// ------------------------------------------------------------------ boards ---

/// A USB serial bridge on USB-A that may be a K210 board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    /// Its place on the bus (sysfs: "1-1.3").
    pub id: String,
    /// The bridge ("FT2232", "CH340").
    pub chip: &'static str,
    /// What it calls itself, if anything.
    pub name: String,
    /// Its serial ports, the likeliest first (none: no driver for it).
    pub ttys: Vec<String>,
    /// Known as a K210 board (else only a serial bridge that may be one).
    pub sure: bool,
}

/// The bridges K210 boards carry, and whether the chip alone says so (the
/// FT2232: M5Stack's UnitV and M5StickV, Sipeed's boards' CH552 as it
/// presents itself).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
fn bridge(vid: u16, pid: u16) -> Option<(&'static str, bool)> {
    Some(match (vid, pid) {
        (0x0403, 0x6010) => ("FT2232", true),
        (0x0403, 0x6001 | 0x6014 | 0x6015) => ("FTDI", false),
        (0x1a86, 0x7523) => ("CH340", false),
        (0x1a86, 0x5523) => ("CH341", false),
        (0x1a86, 0x55d2..=0x55d8) => ("CH9102", false),
        _ => return None,
    })
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
/// sysfs names an interface "<device>:<config>.<interface>" ("1-1.2:1.0");
/// a computer without sysfs cannot name a file so, and its tests use "_".
const IFACE: char = if cfg!(windows) { '_' } else { ':' };

const K210_NAMES: [&str; 7] = ["m5stack", "unitv", "m5stickv", "sipeed", "k210", "maix", "kendryte"];

/// The first K210 board (else serial bridge) under `root`
/// (/sys/bus/usb/devices).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn scan(root: &Path) -> Option<Board> {
    let read = |p: PathBuf| std::fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_default();
    let mut entries: Vec<String> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    entries.sort();
    let mut found: Vec<Board> = Vec::new();
    for id in entries.iter().filter(|n| !n.contains(IFACE)) {
        let dir = root.join(id);
        let hex = |f: &str| u16::from_str_radix(&read(dir.join(f)), 16).ok();
        let (Some(vid), Some(pid)) = (hex("idVendor"), hex("idProduct")) else { continue };
        let Some((chip, by_chip)) = bridge(vid, pid) else { continue };
        let name = [read(dir.join("manufacturer")), read(dir.join("product"))].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
        let lower = name.to_lowercase();
        let sure = by_chip || K210_NAMES.iter().any(|k| lower.contains(k));
        // Its interfaces' serial ports, in interface order.
        let mut ttys: Vec<(String, String)> = Vec::new();
        for iface in entries.iter().filter(|n| n.starts_with(&format!("{id}{IFACE}"))) {
            let at = root.join(iface);
            for e in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("ttyUSB") {
                    ttys.push((iface.clone(), format!("/dev/{n}")));
                } else if n == "tty" {
                    for t in std::fs::read_dir(at.join("tty")).into_iter().flatten().flatten() {
                        ttys.push((iface.clone(), format!("/dev/{}", t.file_name().to_string_lossy())));
                    }
                }
            }
        }
        ttys.sort();
        // The FT2232's second port is the K210's UART (kflash.py's default).
        if chip == "FT2232" && ttys.len() > 1 {
            ttys.rotate_left(1);
        }
        found.push(Board { id: id.clone(), chip, name, ttys: ttys.into_iter().map(|(_, t)| t).collect(), sure });
    }
    found.iter().position(|b| b.sure).map(|i| found.swap_remove(i)).or_else(|| found.into_iter().next())
}

// ------------------------------------------------------------------- state ---

/// What the camera runs, as its REPL said.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// Not asked yet, or being asked.
    Looking,
    /// Nothing answered: no MaixPy (or the line is not the K210's).
    NoMaixPy,
    MaixPy { full: bool, version: String, stamp: Option<Stamp> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Firmware,
    Model,
    Program,
    Check,
}

/// What an install does: flash MaixPy, write the model (main.py always).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub firmware: bool,
    pub model: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Idle,
    /// Under way: the step, how far through it (0 to 1).
    Running { step: Step, done: f32, plan: Plan },
    /// Done; whether it answered over USB afterwards.
    Done { answered: bool, plan: Plan },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub board: Option<Board>,
    pub found: Found,
    pub job: Job,
}

/// What the page and Settings offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Unplugged,
    /// A bridge with no serial port: its driver did not load.
    NoDriver,
    Looking,
    /// A serial bridge not known as a K210 board: installed only when asked.
    Unsure,
    Install,
    Update,
    Current,
}

pub fn need(state: &State, expected: Option<&Stamp>) -> Need {
    let Some(board) = &state.board else { return Need::Unplugged };
    if board.ttys.is_empty() {
        return Need::NoDriver;
    }
    match &state.found {
        Found::Looking if !board.sure => Need::Unsure,
        Found::Looking => Need::Looking,
        Found::NoMaixPy | Found::MaixPy { full: false, .. } | Found::MaixPy { stamp: None, .. } => Need::Install,
        // The keyword spotter comes over the Grove port: not USB-A's to send.
        Found::MaixPy { stamp: Some(s), .. } => match expected {
            Some(e) if s.main != e.main || s.model != e.model => Need::Update,
            _ => Need::Current,
        },
    }
}

struct Hub {
    state: Mutex<State>,
    /// An install asked for.
    asked: Mutex<bool>,
    wake: Condvar,
}

fn hub() -> &'static Hub {
    static HUB: OnceLock<Hub> = OnceLock::new();
    HUB.get_or_init(|| Hub { state: Mutex::new(State { board: None, found: Found::Looking, job: Job::Idle }), asked: Mutex::new(false), wake: Condvar::new() })
}

pub fn state() -> State {
    hub().state.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(change: impl FnOnce(&mut State)) {
    {
        let mut s = hub().state.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut s);
    }
    crate::platform::wake();
}

static SAID: Mutex<Option<String>> = Mutex::new(None);

fn say(words: impl Into<String>) {
    *SAID.lock().unwrap_or_else(|e| e.into_inner()) = Some(words.into());
    crate::platform::wake();
}

/// What the island is to say, since the shell last asked.
pub fn take_said() -> Option<String> {
    SAID.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// An install, on the watching thread (one at a time).
pub fn install() {
    if matches!(state().job, Job::Running { .. }) {
        return;
    }
    set(|s| s.job = Job::Running { step: Step::Firmware, done: 0.0, plan: Plan::default() });
    *hub().asked.lock().unwrap_or_else(|e| e.into_inner()) = true;
    hub().wake.notify_all();
}

/// The stamp last seen on the camera: on USB-A now, in its hello on the
/// Grove port, or as last installed.
pub fn known_stamp() -> Option<Stamp> {
    if let Found::MaixPy { stamp: Some(s), .. } = state().found {
        return Some(s);
    }
    let hello = crate::smartcam::about().and_then(|a| serde_json::from_str::<serde_json::Value>(&a).ok());
    if let Some(s) = hello.as_ref().and_then(|v| v.get("stamp")).and_then(Stamp::from_value) {
        return Some(s);
    }
    std::fs::read_to_string(saved_stamp()).ok().and_then(|t| Stamp::parse(&t))
}

fn saved_stamp() -> PathBuf {
    if cfg!(target_arch = "aarch64") {
        "/data/catalyst/unitv-stamp.json".into()
    } else {
        crate::config_dir().join("unitv-stamp.json")
    }
}

/// Settings › Camera › UnitV, in a few words.
pub fn words() -> String {
    let s = state();
    let expected = expected();
    if let Job::Running { step, done, plan } = &s.job {
        return format!("installing · {}%", (overall(*step, *done, *plan) * 100.0) as u32);
    }
    match need(&s, expected.as_ref()) {
        Need::Unplugged => match (known_stamp(), expected) {
            // 0.9.4's program on it takes newer ones over the Grove port.
            (Some(k), Some(e)) if !k.same_program(&e) && crate::smartcam::diag().hello.is_some_and(|h| h.updates) => "updates over the Grove port".into(),
            (Some(k), Some(e)) if !k.same_program(&e) => "update on the camera · USB-A".into(),
            (Some(_), _) => "up to date".into(),
            _ => "plug it into USB-A".into(),
        },
        Need::NoDriver => "no driver for its USB".into(),
        Need::Looking => "checking…".into(),
        Need::Unsure => "a USB serial device".into(),
        Need::Install => "install · on USB-A".into(),
        Need::Update => "update on the camera".into(),
        Need::Current => "up to date".into(),
    }
}

/// How far through the whole install, from the step and how far through it.
pub fn overall(step: Step, done: f32, plan: Plan) -> f32 {
    // Each step's share, by the time it takes (my own estimate: the
    // firmware ~35 s, the model ~8 s, main.py ~6 s, the check ~4 s).
    let shares = [(Step::Firmware, if plan.firmware { 35.0 } else { 0.0 }), (Step::Model, if plan.model { 8.0 } else { 0.0 }), (Step::Program, 6.0), (Step::Check, 4.0)];
    let total: f32 = shares.iter().map(|(_, s)| s).sum();
    let mut before = 0.0;
    for (s, share) in shares {
        if s == step {
            return ((before + share * done.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0);
        }
        before += share;
    }
    1.0
}

// ------------------------------------------------------------------- watch ---

/// Watching USB-A for a K210 board, from start (on its own thread).
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("unitv".into())
            .spawn(|| {
                crate::sched::enter(crate::sched::Class::Batch(5));
                work_out_expected();
                watch();
            })
            .ok();
    });
}

fn scan_now() -> Option<Board> {
    #[cfg(not(target_arch = "aarch64"))]
    return sim::board();
    #[cfg(target_arch = "aarch64")]
    scan(Path::new("/sys/bus/usb/devices"))
}

fn open(tty: &str) -> Option<Box<dyn Serial>> {
    #[cfg(not(target_arch = "aarch64"))]
    return sim::open(tty);
    #[cfg(target_arch = "aarch64")]
    serial::Tty::open(tty).map(|t| Box::new(t) as Box<dyn Serial>)
}

fn watch() {
    // The port that answered, per board, for this run.
    let mut answering: Option<(String, String)> = None;
    loop {
        {
            let asked = hub().asked.lock().unwrap_or_else(|e| e.into_inner());
            let (mut asked, _) = hub().wake.wait_timeout_while(asked, Duration::from_secs(2), |a| !*a).unwrap_or_else(|e| e.into_inner());
            if *asked {
                *asked = false;
                drop(asked);
                if let Some(board) = state().board {
                    let first = answering.as_ref().filter(|(id, _)| *id == board.id).map(|(_, t)| t.clone());
                    run_install(&board, first);
                } else {
                    set(|s| s.job = Job::Failed("nothing on USB-A".into()));
                }
                continue;
            }
        }
        let board = scan_now();
        let before = state().board;
        if board == before {
            continue;
        }
        let arrived = board.as_ref().is_some_and(|b| before.as_ref().is_none_or(|o| o.id != b.id));
        set(|s| {
            s.board = board.clone();
            if arrived || board.is_none() {
                s.found = Found::Looking;
                if !matches!(s.job, Job::Running { .. }) {
                    s.job = Job::Idle;
                }
            }
        });
        let Some(board) = board.filter(|b| arrived && b.sure && !b.ttys.is_empty()) else {
            if arrived && state().board.as_ref().is_some_and(|b| b.ttys.is_empty()) {
                say("a USB serial device · no driver for it");
            }
            continue;
        };
        let (tty, found) = probe(&board, None);
        if let Some(t) = tty {
            answering = Some((board.id.clone(), t));
        }
        set(|s| s.found = found);
        announce();
    }
}

/// What the island says of a camera just plugged in.
fn announce() {
    let s = state();
    let what = match need(&s, expected().as_ref()) {
        Need::Install => "install on the camera",
        Need::Update => "update on the camera",
        Need::Current => {
            say("smart camera on USB · its program is up to date");
            return;
        }
        _ => return,
    };
    crate::notify::post(crate::notify::Notification::new("Camera", "developer_board", "smart camera on USB").body(what).tag("unitv").action("unitv:install"));
}

/// Each of the board's ports asked over MaixPy's REPL (the one that
/// answered before first): which answered, and what it said. The camera is
/// left running main.py again.
fn probe(board: &Board, first: Option<String>) -> (Option<String>, Found) {
    let mut ttys = board.ttys.clone();
    if let Some(f) = first {
        ttys.retain(|t| *t != f);
        ttys.insert(0, f);
    }
    for tty in ttys {
        let Some(mut port) = open(&tty) else { continue };
        if let Ok(mut repl) = repl::Repl::enter(port.as_mut(), PROBE_WAIT) {
            let said = repl.exec(&probe_code(), Duration::from_secs(5));
            repl.restart();
            if let Ok(out) = said {
                if let Some(found) = parse_probe(&out) {
                    return (Some(tty), found);
                }
            }
        }
    }
    (None, Found::NoMaixPy)
}

pub fn parse_probe(out: &str) -> Option<Found> {
    let line = out.lines().find_map(|l| l.trim().strip_prefix("CATALYST|"))?;
    let mut parts = line.splitn(3, '|');
    let full = parts.next()? == "1";
    let version = parts.next()?.to_owned();
    let stamp = Stamp::parse(parts.next().unwrap_or("{}"));
    Some(Found::MaixPy { full, version, stamp })
}

// ----------------------------------------------------------------- install ---

fn run_install(board: &Board, first: Option<String>) {
    let report = |step: Step, done: f32, plan: Plan| set(|s| s.job = Job::Running { step, done, plan });
    let result = Payload::load().and_then(|payload| install_on(board, first, &payload, &report));
    match result {
        Ok((answered, plan, stamp)) => {
            std::fs::create_dir_all(saved_stamp().parent().unwrap_or(Path::new("/"))).ok();
            crate::sys::write_atomic(&saved_stamp(), stamp.json().as_bytes()).ok();
            set(|s| {
                s.job = Job::Done { answered, plan };
                if let Found::MaixPy { stamp: st, full, .. } = &mut s.found {
                    *st = Some(stamp.clone());
                    *full = true;
                } else {
                    s.found = Found::MaixPy { full: true, version: firmware_version(&stamp.firmware), stamp: Some(stamp.clone()) };
                }
            });
            let card = CARD_OWN.load(std::sync::atomic::Ordering::Acquire);
            say(match (card, answered, crate::smartcam::chosen()) {
                (true, ..) => "installed · take the microSD card out of the camera",
                (false, true, true) => "installed on the camera · move it to the Grove port",
                (false, true, false) => "installed · now Settings › Camera › Grove port › Smart camera",
                (false, false, _) => "installed · not heard over USB: the camera's status page has more",
            });
        }
        Err(why) => {
            set(|s| s.job = Job::Failed(why.clone()));
            say(format!("not installed · {why}"));
        }
    }
}

/// The whole install on `board`: (whether it answered after, the plan, the
/// stamp written).
pub fn install_on(board: &Board, first: Option<String>, payload: &Payload, report: &dyn Fn(Step, f32, Plan)) -> Result<(bool, Plan, Stamp), String> {
    // 1. What it runs now.
    let (tty, found) = probe(board, first);
    let expected = payload.stamp("");
    let (full, stamp, version) = match &found {
        Found::MaixPy { full, stamp, version } => (*full, stamp.clone(), version.clone()),
        _ => (false, None, String::new()),
    };
    let plan = Plan { firmware: !full, model: !full || stamp.as_ref().is_none_or(|s| s.model != expected.model) };
    report(Step::Firmware, 0.0, plan);
    let mut ttys = board.ttys.clone();
    if let Some(t) = &tty {
        ttys.retain(|x| x != t);
        ttys.insert(0, t.clone());
    }

    // 2. Through the ISP: MaixPy and the model, as needed.
    let mut firmware_name = stamp.as_ref().map(|s| s.firmware.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| format!("MaixPy {version} (as found)"));
    let mut tty = tty;
    if plan.firmware || plan.model {
        let mut last = String::from("the camera did not enter its ISP");
        let mut done = false;
        'ports: for t in &ttys {
            for fast in [true, false] {
                let Some(mut port) = open(t) else { continue 'ports };
                match flash(port.as_mut(), payload, plan, fast, report) {
                    Ok(()) => {
                        done = true;
                        tty = Some(t.clone());
                        break 'ports;
                    }
                    Err(why) => last = why,
                }
            }
        }
        if !done {
            return Err(last);
        }
        if plan.firmware {
            firmware_name = format!("{} (installed by Catalyst)", payload.firmware_name);
        }
    }

    // 3. main.py, through the REPL (MaixPy starting again after a reset).
    let tty = tty.ok_or("no MaixPy answered on its USB")?;
    let mut port = open(&tty).ok_or("its USB port went away")?;
    report(Step::Program, 0.0, plan);
    // Its firmware is this image's now (flashed, or found at its number).
    // The keyword spotter (94 kB) is not written over the REPL (as a bytes
    // literal there and read back as hex, near a minute at 115 200): its
    // stamp is left empty, so the Grove port sends it at 3 Mbaud, in under
    // a second, the first time the camera answers there.
    let stamp = Stamp { firmware: firmware_name, fwv: payload.firmware_version, kws: String::new(), ..expected };
    let card: String;
    {
        // A fresh MaixPy makes its /flash filesystem at its first start.
        let mut repl = repl::Repl::enter(port.as_mut(), Duration::from_secs(if plan.firmware { 40 } else { 12 }))?;
        let total = (payload.main.len() + stamp.json().len()) as f32;
        let mut sent = 0usize;
        repl.write_file("/flash/main.py", &payload.main, &mut |n| report(Step::Program, (sent + n) as f32 / total, plan))?;
        sent += payload.main.len();
        repl.write_file(STAMP_FILE, stamp.json().as_bytes(), &mut |n| report(Step::Program, (sent + n) as f32 / total, plan))?;
        let tidy = TIDY.replace("@MODEL@", if plan.model { ",'face.kmodel','face.kmodel.old'" } else { "" });
        card = repl.exec(&tidy, Duration::from_secs(5)).ok().and_then(|out| out.lines().find_map(|l| l.trim().strip_prefix("CARD|").map(str::to_owned))).unwrap_or_default();
        report(Step::Program, 1.0, plan);
        repl.restart();
    }

    // 4. Does it answer, as main.py, on the same line?
    report(Step::Check, 0.0, plan);
    let answered = check(port.as_mut(), &mut |f| report(Step::Check, f, plan));
    report(Step::Check, 1.0, plan);
    // A card with a program of its own would start instead of this one.
    CARD_OWN.store(card == "own", std::sync::atomic::Ordering::Release);
    Ok((answered, plan, stamp))
}

/// The last install found a microSD card in the camera with a main.py of
/// its own, which MaixPy starts instead of /flash's.
pub static CARD_OWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// MaixPy and the model written through the ISP, then the camera reset
/// into its flash.
fn flash(port: &mut dyn Serial, payload: &Payload, plan: Plan, fast: bool, report: &dyn Fn(Step, f32, Plan)) -> Result<(), String> {
    let first = if plan.firmware { Step::Firmware } else { Step::Model };
    let image = plan.firmware.then(|| isp::firmware_image(&payload.firmware));
    // The loader is a slice of the first step: a tenth of it.
    let loader = payload.isp.len() as f32;
    let mut session = isp::Isp::enter(port, Wiring::Dan)?;
    session.load(&payload.isp, &mut |n| report(first, 0.1 * n as f32 / loader, plan))?;
    if fast && !session.faster(FAST) {
        return Err("the camera's loader did not hold 1.5 Mbaud".into());
    }
    session.init_flash()?;
    if let Some(image) = &image {
        let len = image.len() as f32;
        session.write_flash(0, image, &mut |n| report(Step::Firmware, 0.1 + 0.9 * n as f32 / len, plan))?;
    }
    if plan.model {
        let len = payload.model.len() as f32;
        let (from, span) = if plan.firmware { (0.0, 1.0) } else { (0.1, 0.9) };
        session.write_flash(MODEL_AT, &payload.model, &mut |n| report(Step::Model, from + span * n as f32 / len, plan))?;
    }
    session.finish();
    Ok(())
}

/// A hello on the framed protocol until main.py answers (up to eight
/// seconds after its soft reset). The hello's bytes hold no 03: the REPL
/// would take one as Ctrl-C and stop main.py.
pub fn check(port: &mut dyn Serial, progress: &mut dyn FnMut(f32)) -> bool {
    let hello = (1..=255u8).map(|seq| frame::encode(frame::HELLO, seq, br#"{"v":1}"#)).find(|f| !f.contains(&0x03)).unwrap_or_default();
    let began = Instant::now();
    let within = Duration::from_secs(8);
    let mut decoder = frame::Decoder::default();
    let mut buf = [0u8; 512];
    let mut last = began - Duration::from_secs(1);
    while began.elapsed() < within {
        if last.elapsed() >= Duration::from_millis(300) {
            port.write(&hello);
            last = Instant::now();
            progress(began.elapsed().as_secs_f32() / within.as_secs_f32());
        }
        let n = port.read(&mut buf, Duration::from_millis(40));
        if decoder.push(&buf[..n]).iter().any(|f| f.kind == frame::HELLO_BACK) {
            return true;
        }
    }
    false
}

// --------------------------------------------------------------------- sim ---

/// The simulator's UnitV, plugged in when asked.
#[cfg(not(target_arch = "aarch64"))]
pub mod sim {
    use super::stub::{Fake, Firmware, Port};
    use super::*;
    use std::sync::Arc;

    static FAKE: Mutex<Option<Arc<Mutex<Fake>>>> = Mutex::new(None);

    /// For tests: a stand-in of their own on USB-A (or none).
    #[cfg(test)]
    pub fn plug_with(fake: Option<Arc<Mutex<Fake>>>) {
        *FAKE.lock().unwrap_or_else(|e| e.into_inner()) = fake;
    }

    /// For tests: one at a time on the simulator's USB-A.
    #[cfg(test)]
    pub fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static ONE: Mutex<()> = Mutex::new(());
        ONE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A UnitV on USB-A: "bare" (nothing on it that answers), "new" (MaixPy,
    /// nothing of Catalyst's), "old" (an older Catalyst's program), "current".
    pub fn plug(kind: Option<&str>) {
        let fake = kind.map(|k| {
            let mut f = Fake::new(if k == "bare" { Firmware::None } else { Firmware::MaixPy { full: true } });
            f.pace = true;
            f.boot = Duration::from_millis(1400);
            if k == "old" || k == "current" {
                let payload = Payload::load().ok();
                let mut stamp = payload.as_ref().map(|p| p.stamp("MaixPy 0.6.3 (as found)")).unwrap_or_default();
                if k == "old" {
                    stamp.main = "0a1b2c3d4e5f".into();
                    stamp.catalyst = "0.9.0".into();
                }
                f.files.insert("/flash/main.py".into(), b"# an older main.py\n".to_vec());
                f.files.insert(STAMP_FILE.into(), stamp.json().into_bytes());
            }
            f.running().shared()
        });
        *FAKE.lock().unwrap_or_else(|e| e.into_inner()) = fake;
    }

    pub fn board_for_shots() -> Board {
        Board { id: "1-1.3".into(), chip: "FT2232", name: "M5Stack UnitV".into(), ttys: vec!["/dev/ttyUSB1".into(), "/dev/ttyUSB0".into()], sure: true }
    }

    pub fn board() -> Option<Board> {
        static FROM_ENV: OnceLock<()> = OnceLock::new();
        FROM_ENV.get_or_init(|| {
            if let Ok(kind) = std::env::var("CATALYST_SIM_UNITV") {
                plug(Some(&kind));
            }
        });
        FAKE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|_| board_for_shots())
    }

    pub fn open(tty: &str) -> Option<Box<dyn Serial>> {
        // The FT2232's first port is the K210's JTAG side: nothing answers.
        let fake = FAKE.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        if tty.ends_with('0') {
            return Some(Box::new(Port(Fake::new(Firmware::None).shared())));
        }
        Some(Box::new(Port(fake)))
    }

    /// For the screenshots: the state as given, this image's stamp known.
    pub fn set_for_shots(state: State) {
        work_out_expected();
        set(|s| *s = state);
    }
}

// --------------------------------------------------------------------- page ---

/// The camera's program: what is on USB-A, what it runs, and the install
/// with how far it has come. Opened from the island's notice and Settings ›
/// Camera › UnitV.
///
/// Two lines say what is plugged in and how it stands, a bar under them how
/// far the whole install has come, and a row for each part (MaixPy, the
/// face model, main.py) what happens to it; the part under way is ringed and
/// the list follows it (at the largest text, the rows scroll).
pub struct InstallScreen {
    state: State,
    expected: Option<Stamp>,
    /// The handheld's USB controller in the device role (Settings › USB):
    /// USB-A has no host behind it.
    usb_off: bool,
    /// The bar's fill, gliding to how far the install has come.
    bar: Animated,
    list: ListState,
}

impl InstallScreen {
    pub fn new() -> InstallScreen {
        start();
        let state = state();
        let mut page = InstallScreen { usb_off: crate::usb::running_as_device(), expected: expected(), bar: Animated::new(0.0, role::smooth()).with_rest(0.002), list: ListState::default(), state };
        page.follow();
        page
    }

    fn need(&self) -> Need {
        need(&self.state, self.expected.as_ref())
    }

    /// The step under way, by its row.
    fn active(&self) -> Option<usize> {
        match &self.state.job {
            Job::Running { step: Step::Firmware, .. } => Some(0),
            Job::Running { step: Step::Model, .. } => Some(1),
            Job::Running { .. } => Some(2),
            _ => None,
        }
    }

    /// The list kept on the step under way.
    fn follow(&mut self) {
        if let Some(at) = self.active() {
            self.list.focus = at;
        }
    }

    fn refresh(&mut self) -> bool {
        let fresh = state();
        let expected = expected();
        let changed = fresh != self.state || expected != self.expected;
        match &fresh.job {
            Job::Running { step, done, plan } => self.bar.set(overall(*step, *done, *plan)),
            Job::Done { .. } => self.bar.set(1.0),
            _ => {}
        }
        self.state = fresh;
        self.expected = expected;
        self.follow();
        changed
    }

    /// The two lines at the top: what is plugged in, and how it stands, each
    /// longest first (the first that fits whole is drawn).
    fn lines(&self) -> (Vec<String>, Vec<String>) {
        let board = match &self.state.board {
            Some(b) if b.sure => {
                let port = b.ttys.first().map_or("no port", |t| t.trim_start_matches("/dev/"));
                vec![format!("UnitV on USB-A · {} · {port}", b.chip), format!("UnitV on USB-A · {port}"), "UnitV on USB-A".into()]
            }
            Some(b) if !b.name.is_empty() => vec![format!("{} serial device on USB-A · {}", b.chip, b.name), format!("{} serial device on USB-A", b.chip)],
            Some(b) => vec![format!("{} serial device on USB-A", b.chip), "a serial device on USB-A".into()],
            None if self.usb_off => vec!["USB-A is off while a computer uses the handheld".into(), "USB-A is off (Settings › USB)".into()],
            None => vec!["nothing on USB-A".into()],
        };
        let words = |all: &[&str]| all.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
        let release = self.expected.as_ref().map_or_else(String::new, |e| e.catalyst.clone());
        let status = match (&self.state.job, self.need()) {
            (Job::Running { step, plan, .. }, _) => words(&[match step {
                Step::Firmware if plan.firmware => "flashing MaixPy…",
                Step::Firmware => "asking what it runs…",
                Step::Model => "writing the face model…",
                Step::Program => "writing main.py…",
                Step::Check => "listening for it over USB…",
            }]),
            (Job::Done { .. }, _) if CARD_OWN.load(std::sync::atomic::Ordering::Acquire) => words(&["installed · take the microSD card out of it", "installed · take its card out"]),
            (Job::Done { answered: true, .. }, _) => words(&["installed, and answering over USB", "installed, and answering"]),
            (Job::Done { answered: false, .. }, _) => words(&["installed · not heard over USB: a card in it?", "installed · not heard over USB", "installed"]),
            (Job::Failed(why), _) => vec![why.clone(), "not installed".into()],
            (_, Need::Unplugged) if self.usb_off => words(&["Settings › USB › charging only frees it", "charging only frees it"]),
            (_, Need::Unplugged) => words(&["plug the UnitV's USB-C in here, no card needed", "plug the UnitV's USB-C in here"]),
            (_, Need::NoDriver) => words(&["no driver took its USB serial bridge", "no driver for its USB"]),
            (_, Need::Looking) => words(&["asking what it runs…"]),
            (_, Need::Unsure) => words(&["not known as a UnitV: install only if it is one", "not known as a UnitV"]),
            (_, Need::Install) => match &self.state.found {
                Found::NoMaixPy => words(&["no MaixPy answers: it gets MaixPy first", "no MaixPy: it gets one first"]),
                Found::MaixPy { full: false, version, .. } => vec![format!("MaixPy {version} lacks what it needs: a full one first"), format!("MaixPy {version}: a full one first")],
                Found::MaixPy { version, .. } => vec![format!("MaixPy {version} · no Catalyst program on it yet"), format!("MaixPy {version} · no Catalyst program yet"), "no Catalyst program yet".into()],
                Found::Looking => Vec::new(),
            },
            (_, Need::Update) => match &self.state.found {
                Found::MaixPy { stamp: Some(s), .. } if !s.catalyst.is_empty() => {
                    let v = &s.catalyst;
                    vec![format!("Catalyst {v}'s program on it · a newer one here"), format!("Catalyst {v}'s program · a newer one here"), format!("Catalyst {v}'s program: an older one"), "a newer program here".into()]
                }
                _ => words(&["an older program on it · a newer one here", "a newer program here"]),
            },
            (_, Need::Current) => vec![format!("Catalyst {release}'s program on it, as this one's"), format!("Catalyst {release}'s program, as this one's"), "its program is up to date".into()],
        };
        (board, status)
    }

    /// The parts' rows: name, and what happens to it.
    fn rows(&self) -> [(&'static str, String); 3] {
        let found_full = matches!(self.state.found, Found::MaixPy { full: true, .. });
        let found_model_ok = match (&self.state.found, &self.expected) {
            (Found::MaixPy { stamp: Some(s), full: true, .. }, Some(e)) => s.model == e.model,
            _ => false,
        };
        let version = match &self.state.found {
            Found::MaixPy { version, .. } => version.clone(),
            _ => String::new(),
        };
        let pct = |d: f32| format!("{}%", (d * 100.0) as u32);
        let unknown = self.state.board.is_none() || self.state.found == Found::Looking;
        let (firmware, model, program) = match &self.state.job {
            Job::Running { step, done, plan } => {
                let past = |s: Step| (*step as u8) > (s as u8);
                let firmware = match step {
                    Step::Firmware if plan.firmware => pct(*done),
                    Step::Firmware if *done == 0.0 => "checking…".into(),
                    _ if plan.firmware => "flashed".into(),
                    _ => format!("kept {version}"),
                };
                let model = match step {
                    Step::Model => pct(*done),
                    _ if !plan.model => "up to date".into(),
                    _ if past(Step::Model) => "written".into(),
                    _ => "next".into(),
                };
                let program = match step {
                    Step::Program => pct(*done),
                    Step::Check => "written".into(),
                    _ => "next".into(),
                };
                (firmware, model, program)
            }
            Job::Done { plan, .. } => (
                if plan.firmware { "flashed 0.6.3".into() } else { format!("kept {version}") },
                if plan.model { "written".into() } else { "up to date".into() },
                "written".into(),
            ),
            _ if unknown => ("–".into(), "–".into(), "–".into()),
            _ => (
                if found_full { format!("keep {version}") } else { "flash 0.6.3 · 2.1 MB".into() },
                if found_model_ok { "up to date".into() } else { "write · 0.4 MB".into() },
                if self.need() == Need::Update { "update · 13 kB".into() } else { "write · 13 kB".into() },
            ),
        };
        [("MaixPy", firmware), ("Face model", model), ("main.py", program)]
    }
}

impl Screen for InstallScreen {
    fn title(&self) -> &str {
        "UnitV"
    }

    fn keeps_awake(&self) -> bool {
        matches!(self.state.job, Job::Running { .. })
    }

    /// The bar glides on the beat only while it moves.
    fn tick(&mut self, dt: f32) -> bool {
        self.bar.tick(dt)
    }

    /// The install's progress looked at four times a second, off the beat:
    /// waiting on the camera never holds the frame beat.
    fn ambient(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        self.refresh()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter => match (&self.state.job, self.need()) {
                (Job::Running { .. }, _) => *cx.say = Some("installing · keep it plugged in".into()),
                // Installed with the port still the console: the port given
                // to the camera; after that, the camera's status page.
                (Job::Done { .. }, _) if !crate::smartcam::chosen() => crate::smartcam::switch_grove(true),
                (Job::Done { .. }, _) => return Nav::Push(Box::new(crate::smartcam_status::StatusPage::new())),
                (_, Need::Unplugged) => *cx.say = Some("plug the UnitV into USB-A".into()),
                (_, Need::NoDriver) => *cx.say = Some("no driver for its USB serial bridge".into()),
                (_, Need::Looking) => *cx.say = Some("still asking what it runs".into()),
                _ => {
                    self.bar.jump(0.0);
                    install();
                    self.refresh();
                }
            },
            // The rows scroll where they do not all fit.
            other if self.active().is_none() => crate::screens::list_key(&mut self.list, other, 3),
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD;
        let cap_lh = ty.caption.size + 4.0;
        let (board, status) = self.lines();
        let failed = matches!(self.state.job, Job::Failed(_));
        let mut whole = |choices: &[String]| -> String {
            let fits = choices.iter().find(|s| g.measure(s, ty.caption) <= width).cloned();
            fits.unwrap_or_else(|| g.text.fit(choices.last().map_or("", String::as_str), &ty.caption, width))
        };
        let (board, status) = (whole(&board), whole(&status));
        g.text_at(PAD + 2.0, top + 1.0 + 0.5 * cap_lh, Align::Left, &board, ty.caption, t.dim);
        g.text_at(PAD + 2.0, top + 1.0 + 1.5 * cap_lh, Align::Left, &status, ty.caption, if failed { g.fault() } else { t.ink });

        // The whole install's bar, under the words, while it runs and once done.
        let bar_y = top + 5.0 + 2.0 * cap_lh;
        if matches!(self.state.job, Job::Running { .. } | Job::Done { .. }) {
            g.meter(Rect::new(PAD + 2.0, bar_y, width - 4.0, 4.0), self.bar.value(), t.signal);
        }
        let rows_top = bar_y + 8.0;
        let rows = self.rows();
        let active = self.active();
        for (i, y) in self.list.layout(g, rows.len(), rows_top, row_h, 3.0, bottom) {
            let (name, value) = &rows[i];
            let lit = active == Some(i);
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            g.row(r, name, value, if lit { t.ink } else { t.dim }, lit);
        }
        self.list.scrollbar(g, rows.len(), rows_top, bottom);
        let hint: &[&str] = match (&self.state.job, self.need()) {
            (Job::Running { .. }, _) => &["keep it plugged in · about a minute", "keep it plugged in"],
            (Job::Done { .. }, _) if !crate::smartcam::chosen() => &["enter: Grove port to smart camera, then move it", "enter: Grove port to smart camera", "enter: smart camera"],
            (Job::Done { .. }, _) => &["done · move it to the Grove port · enter: status", "done · to the Grove port · enter: status", "enter: status"],
            (Job::Failed(_), _) => &["enter: try again", "enter: again"],
            (_, Need::Unplugged) => &["plug the UnitV's USB-C into USB-A", "plug it into USB-A"],
            (_, Need::NoDriver | Need::Looking) => &["esc: back"],
            (_, Need::Unsure) => &["enter: install, if this is the UnitV", "enter: install"],
            (_, Need::Install) => &["enter: install on the camera", "enter: install"],
            (_, Need::Update) => &["enter: update on the camera", "enter: update"],
            (_, Need::Current) => &["enter: install it again", "enter: again"],
        };
        g.hint_any(hint);
    }
}

// ------------------------------------------------------------------- shots ---

/// The install's pages (on USB-A, installing, done, an update) and
/// Settings › Camera, at each text size.
/// `catalyst-shell --unitv-shots <dir>`.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).ok();
    work_out_expected();
    let expected = expected().unwrap_or_default();
    let board = sim::board_for_shots();
    let old = Stamp { catalyst: "0.9.0".into(), main: "0a1b2c3d4e5f".into(), ..expected.clone() };
    let plan = Plan { firmware: true, model: true };
    let states: Vec<(&str, State)> = vec![
        ("1-detected", State { board: Some(board.clone()), found: Found::MaixPy { full: true, version: "0.6.3".into(), stamp: None }, job: Job::Idle }),
        ("1b-detected-bare", State { board: Some(board.clone()), found: Found::NoMaixPy, job: Job::Idle }),
        ("2-installing", State { board: Some(board.clone()), found: Found::NoMaixPy, job: Job::Running { step: Step::Firmware, done: 0.46, plan } }),
        ("2b-installing-program", State { board: Some(board.clone()), found: Found::NoMaixPy, job: Job::Running { step: Step::Program, done: 0.62, plan } }),
        ("3-done", State { board: Some(board.clone()), found: Found::MaixPy { full: true, version: "0.6.3".into(), stamp: Some(expected.clone()) }, job: Job::Done { answered: true, plan } }),
        ("4-update", State { board: Some(board.clone()), found: Found::MaixPy { full: true, version: "0.6.3".into(), stamp: Some(old) }, job: Job::Idle }),
        ("5-unplugged", State { board: None, found: Found::Looking, job: Job::Idle }),
    ];
    let settle = |shell: &mut Shell| {
        for _ in 0..90 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        shell.draw();
    };
    for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
        for (name, st) in &states {
            sim::set_for_shots(st.clone());
            let mut shell = Shell::new(Robot::demo());
            shell.settings.text = size;
            shell.apply_settings();
            let mut page = InstallScreen::new();
            page.bar.jump(match &st.job {
                Job::Running { step, done, plan } => overall(*step, *done, *plan),
                Job::Done { .. } => 1.0,
                _ => 0.0,
            });
            shell.show(Box::new(page));
            settle(&mut shell);
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tag}.png")), 3);
        }
        // Settings › Camera: the port as the camera's, the program to update.
        for (name, st, focus) in [("6-settings-camera", states[5].1.clone(), 3usize), ("7-settings-grove", states[4].1.clone(), 1)] {
            sim::set_for_shots(st);
            let mut shell = Shell::new(Robot::demo());
            shell.settings.text = size;
            shell.apply_settings();
            shell.show(Box::new(crate::screens::SettingsApp::camera_page().focused(focus)));
            settle(&mut shell);
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tag}.png")), 3);
        }
    }
    sim::set_for_shots(State { board: None, found: Found::Looking, job: Job::Idle });
}

#[cfg(test)]
mod tests;
