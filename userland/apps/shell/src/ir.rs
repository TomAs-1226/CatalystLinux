//! Infrared, for the Remote app: the CardputerZero's transmitter (an IR LED
//! on GPIO12, the kernel's `gpio-ir-tx`) and its receiver (a demodulating
//! receiver on GPIO13, `gpio-ir-recv`), through the kernel's LIRC devices.
//!
//! A burst of infrared is kept as it is heard: how long the light was on, then
//! off, then on, in microseconds, starting and ending with light (a pulse).
//! Any remote's button can be learned and sent back that way, whatever its
//! protocol. The common protocols (NEC and its 16-bit-address form, Samsung,
//! Sony SIRC, Philips RC5 and RC6) are also recognised: to name what was
//! learned ("NEC · 0x04 · 0x08"), to keep one clean frame of it rather than
//! every repeat the remote sent while its button was down, and to send it at
//! its own carrier with its own timing (RC5 and RC6 flip their toggle bit on
//! each new press, as their remotes do; a held NEC button sends repeat codes).
//!
//! Neither driver runs while it is not needed: both are kept from loading at
//! start (`/etc/modprobe.d/catalyst.conf`), the transmitter is loaded while
//! the Remote app is open and the receiver only while a button is being
//! learned (every edge of infrared in the room is an interrupt to it). On a
//! computer there is no infrared: sends are only timed, and learning hears a
//! simulated remote.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// The carrier most remotes use, and the one a learned button is sent at
/// when its protocol does not say otherwise (a demodulating receiver hears
/// the light, not the carrier it was flashed at).
pub const CARRIER: u32 = 38_000;

/// A space this long ends a frame: anything longer is the gap before the next.
const FRAME_GAP: u32 = 15_000;
/// A burst ends after this long with no light.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const BURST_END: u32 = 100_000;
/// The most a learned burst keeps: lengths, and time (the kernel takes up to
/// 500 ms in one send).
const MOST_LENGTHS: usize = 511;
const MOST_TIME: u32 = 450_000;
/// Fewer lengths than this is not a remote's button but a flicker of light.
const FEWEST_LENGTHS: usize = 9;

// --------------------------------------------------------------- protocols ---

/// A protocol recognised in a learned frame, with what it said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// NEC: an 8-bit address and a command, each followed by its inverse.
    Nec { address: u8, command: u8 },
    /// NEC with a 16-bit address (no inverse) and a command.
    NecX { address: u16, command: u8 },
    /// NEC's timing with 32 bits that are neither.
    Nec32 { data: u32 },
    /// Samsung: NEC's bits after a shorter leader; the address twice.
    Samsung { address: u8, command: u8 },
    /// Sony SIRC: 12, 15 or 20 bits, a 7-bit command then the address.
    Sony { bits: u8, address: u16, command: u8 },
    /// Philips RC5: 5-bit address, 7-bit command (its sixth bit carried in
    /// the second start bit), and the toggle.
    Rc5 { address: u8, command: u8, toggle: bool },
    /// Philips RC6: a mode, the data bits after the trailer (the toggle).
    Rc6 { mode: u8, bits: u8, data: u32, toggle: bool },
}

/// Within a remote's tolerance of `nominal` µs: receivers stretch light and
/// shorten dark by a hundred or so, and remotes are not exact either.
fn near(v: u32, nominal: u32) -> bool {
    v.abs_diff(nominal) <= (nominal * 3 / 10).max(180)
}

impl Protocol {
    /// What it is, as people write it: `NEC · 0x04 · 0x08`.
    pub fn label(&self) -> String {
        match *self {
            Protocol::Nec { address, command } => format!("NEC · 0x{address:02X} · 0x{command:02X}"),
            Protocol::NecX { address, command } => format!("NECx · 0x{address:04X} · 0x{command:02X}"),
            Protocol::Nec32 { data } => format!("NEC32 · 0x{data:08X}"),
            Protocol::Samsung { address, command } => format!("Samsung · 0x{address:02X} · 0x{command:02X}"),
            Protocol::Sony { bits, address, command } => format!("Sony {bits} · 0x{address:02X} · 0x{command:02X}"),
            Protocol::Rc5 { address, command, .. } => format!("RC5 · 0x{address:02X} · 0x{command:02X}"),
            Protocol::Rc6 { mode: 0, bits: 16, data, .. } => format!("RC6 · 0x{:02X} · 0x{:02X}", data >> 8, data & 0xFF),
            Protocol::Rc6 { mode, bits, data, .. } => format!("RC6-{mode}-{bits} · 0x{data:0w$X}", w = (bits as usize).div_ceil(4)),
        }
    }

    /// The same, short and lowercase, for a row: `nec 04 08`.
    pub fn short(&self) -> String {
        match *self {
            Protocol::Nec { address, command } => format!("nec {address:02x} {command:02x}"),
            Protocol::NecX { address, command } => format!("necx {address:04x} {command:02x}"),
            Protocol::Nec32 { data } => format!("nec32 {data:08x}"),
            Protocol::Samsung { address, command } => format!("samsung {address:02x} {command:02x}"),
            Protocol::Sony { bits, address, command } => format!("sony{bits} {address:02x} {command:02x}"),
            Protocol::Rc5 { address, command, .. } => format!("rc5 {address:02x} {command:02x}"),
            Protocol::Rc6 { mode: 0, bits: 16, data, .. } => format!("rc6 {:02x} {:02x}", data >> 8, data & 0xFF),
            Protocol::Rc6 { data, .. } => format!("rc6 {data:x}"),
        }
    }

    /// The carrier its remotes flash at.
    pub fn carrier(&self) -> u32 {
        match self {
            Protocol::Nec { .. } | Protocol::NecX { .. } | Protocol::Nec32 { .. } | Protocol::Samsung { .. } => 38_000,
            Protocol::Sony { .. } => 40_000,
            Protocol::Rc5 { .. } | Protocol::Rc6 { .. } => 36_000,
        }
    }

    /// From the start of one frame to the start of the next, as its remotes send them.
    fn period(&self) -> u32 {
        match self {
            Protocol::Nec { .. } | Protocol::NecX { .. } | Protocol::Nec32 { .. } | Protocol::Samsung { .. } => 108_000,
            Protocol::Sony { .. } => 45_000,
            Protocol::Rc5 { .. } => 113_778,
            Protocol::Rc6 { .. } => 107_000,
        }
    }

    /// The same press with its toggle bit set to `toggle` (RC5 and RC6; for
    /// the others, unchanged).
    fn with_toggle(self, toggle: bool) -> Protocol {
        match self {
            Protocol::Rc5 { address, command, .. } => Protocol::Rc5 { address, command, toggle },
            // RC6's MCE remotes (mode 6, 32 bits) carry theirs in bit 15 of
            // the data, and leave the trailer's at 0.
            Protocol::Rc6 { mode: 6, bits: 32, data, toggle: trailer } => {
                Protocol::Rc6 { mode: 6, bits: 32, data: if toggle { data | 0x8000 } else { data & !0x8000 }, toggle: trailer }
            }
            Protocol::Rc6 { mode, bits, data, .. } => Protocol::Rc6 { mode, bits, data, toggle },
            other => other,
        }
    }

    /// One frame of it, at its nominal timing.
    pub fn encode(&self) -> Vec<u32> {
        match *self {
            Protocol::Nec { address, command } => nec_frame(9000, u32::from_le_bytes([address, !address, command, !command])),
            Protocol::NecX { address, command } => {
                let [lo, hi] = address.to_le_bytes();
                nec_frame(9000, u32::from_le_bytes([lo, hi, command, !command]))
            }
            Protocol::Nec32 { data } => nec_frame(9000, data),
            Protocol::Samsung { address, command } => nec_frame(4500, u32::from_le_bytes([address, address, command, !command])),
            Protocol::Sony { bits, address, command } => {
                let data = (command as u32 & 0x7F) | ((address as u32) << 7);
                let mut t = vec![2400, 600];
                for i in 0..bits {
                    t.push(if data >> i & 1 == 1 { 1200 } else { 600 });
                    t.push(600);
                }
                t.pop();
                t
            }
            Protocol::Rc5 { address, command, toggle } => {
                // Start, field (the inverse of the command's sixth bit), toggle,
                // five of address, six of command; most significant first. A 1 is
                // dark then light, a 0 light then dark, each half 889 µs.
                let mut bits = vec![true, command & 0x40 == 0, toggle];
                bits.extend((0..5).rev().map(|i| address >> i & 1 == 1));
                bits.extend((0..6).rev().map(|i| command >> i & 1 == 1));
                let halves: Vec<bool> = bits.iter().flat_map(|&b| [!b, b]).collect();
                runs(&halves, 889)
            }
            Protocol::Rc6 { mode, bits, data, toggle } => {
                // Leader, then halves of 444 µs: a 1 is light then dark. The
                // start bit is a 1; the trailer (the toggle) is twice as long.
                let mut halves = vec![true, false];
                for i in (0..3).rev() {
                    let b = mode >> i & 1 == 1;
                    halves.extend([b, !b]);
                }
                halves.extend([toggle, toggle, !toggle, !toggle]);
                for i in (0..bits).rev() {
                    let b = data >> i & 1 == 1;
                    halves.extend([b, !b]);
                }
                let mut t = vec![2666, 889];
                t.extend(runs(&halves, 444));
                t
            }
        }
    }
}

/// NEC's frame (or Samsung's, after its own leader): 32 bits, least
/// significant first, each a 560 µs pulse and a short or long space.
fn nec_frame(leader: u32, data: u32) -> Vec<u32> {
    let mut t = vec![leader, leader / 2];
    if leader == 4500 {
        t[1] = 4500;
    }
    for i in 0..32 {
        t.push(560);
        t.push(if data >> i & 1 == 1 { 1690 } else { 560 });
    }
    t.push(560);
    t
}

/// NEC's repeat code: sent every 108 ms for as long as the button is held.
const NEC_REPEAT: [u32; 3] = [9000, 2250, 560];

/// Halves of light (true) and dark into lengths: runs of each, `unit` µs a
/// half, from the first light to the last.
fn runs(halves: &[bool], unit: u32) -> Vec<u32> {
    let first = halves.iter().position(|&h| h).unwrap_or(halves.len());
    let last = halves.iter().rposition(|&h| h).map_or(0, |i| i + 1);
    let mut out: Vec<u32> = Vec::new();
    let mut level = None;
    for &h in &halves[first..last.max(first)] {
        if level == Some(h) {
            *out.last_mut().expect("a run under way") += unit;
        } else {
            out.push(unit);
            level = Some(h);
        }
    }
    out
}

/// Lengths back into halves of `unit` µs, each run one to `most` halves long;
/// `None` if a run is not a whole number of them.
fn halves(lengths: &[u32], unit: u32, most: u32, starts_lit: bool) -> Option<Vec<bool>> {
    let mut out = Vec::new();
    for (i, &v) in lengths.iter().enumerate() {
        let n = (v + unit / 2) / unit;
        if n == 0 || n > most || v.abs_diff(n * unit) > unit * 2 / 5 {
            return None;
        }
        let lit = (i % 2 == 0) == starts_lit;
        out.extend(std::iter::repeat_n(lit, n as usize));
    }
    Some(out)
}

/// Pairs of halves into bits: `one` is the pair a 1 is made of.
fn manchester(halves: &[bool], one: (bool, bool)) -> Option<Vec<bool>> {
    halves
        .chunks(2)
        .map(|pair| match (pair[0], pair.get(1).copied()?) {
            p if p == one => Some(true),
            p if p == (one.1, one.0) => Some(false),
            _ => None,
        })
        .collect()
}

/// What protocol, if any, one frame is (pulse first, ending in a pulse).
pub fn decode(frame: &[u32]) -> Option<Protocol> {
    decode_nec(frame).or_else(|| decode_sony(frame)).or_else(|| decode_rc5(frame)).or_else(|| decode_rc6(frame))
}

fn decode_nec(f: &[u32]) -> Option<Protocol> {
    if f.len() < 67 {
        return None;
    }
    let samsung = match (f[0], f[1]) {
        (p, s) if near(p, 9000) && near(s, 4500) => false,
        (p, s) if near(p, 4500) && near(s, 4500) => true,
        _ => return None,
    };
    let mut data = 0u32;
    for i in 0..32 {
        let (pulse, space) = (f[2 + 2 * i], f[3 + 2 * i]);
        if !near(pulse, 560) {
            return None;
        }
        if near(space, 1690) {
            data |= 1 << i;
        } else if !near(space, 560) {
            return None;
        }
    }
    if !near(f[66], 560) {
        return None;
    }
    let [b0, b1, b2, b3] = data.to_le_bytes();
    Some(match (samsung, b2 ^ b3 == 0xFF) {
        (true, true) if b0 == b1 => Protocol::Samsung { address: b0, command: b2 },
        (true, _) => return None,
        (false, true) if b0 ^ b1 == 0xFF => Protocol::Nec { address: b0, command: b2 },
        (false, true) => Protocol::NecX { address: u16::from_le_bytes([b0, b1]), command: b2 },
        (false, false) => Protocol::Nec32 { data },
    })
}

/// A frame that is only NEC's repeat code.
pub fn is_nec_repeat(f: &[u32]) -> bool {
    f.len() == 3 && near(f[0], 9000) && near(f[1], 2250) && near(f[2], 560)
}

fn decode_sony(f: &[u32]) -> Option<Protocol> {
    if f.len() < 3 || f.len() % 2 == 0 || !near(f[0], 2400) || !near(f[1], 600) {
        return None;
    }
    let bits = (f.len() - 1) / 2;
    if ![12, 15, 20].contains(&bits) {
        return None;
    }
    let mut data = 0u32;
    for i in 0..bits {
        let pulse = f[2 + 2 * i];
        if near(pulse, 1200) {
            data |= 1 << i;
        } else if !near(pulse, 600) {
            return None;
        }
        if let Some(&space) = f.get(3 + 2 * i) {
            if !near(space, 600) {
                return None;
            }
        }
    }
    Some(Protocol::Sony { bits: bits as u8, address: (data >> 7) as u16, command: (data & 0x7F) as u8 })
}

fn decode_rc5(f: &[u32]) -> Option<Protocol> {
    // The first start bit's dark half cannot be heard: the frame starts lit.
    let mut h = vec![false];
    h.extend(halves(f, 889, 2, true)?);
    if h.len() % 2 == 1 {
        // And the last bit's dark half, if it ended in one.
        h.push(false);
    }
    let bits = manchester(&h, (false, true))?;
    if bits.len() != 14 || !bits[0] {
        return None;
    }
    let number = |range: std::ops::Range<usize>| bits[range].iter().fold(0u8, |n, &b| n << 1 | b as u8);
    let command = number(8..14) | if bits[1] { 0 } else { 0x40 };
    Some(Protocol::Rc5 { address: number(3..8), command, toggle: bits[2] })
}

fn decode_rc6(f: &[u32]) -> Option<Protocol> {
    if f.len() < 4 || !near(f[0], 2666) || !near(f[1], 889) {
        return None;
    }
    let mut h = halves(&f[2..], 444, 3, true)?;
    if h.len() % 2 == 1 {
        h.push(false);
    }
    // Start (2 halves), mode (6), trailer (4), then the data.
    if h.len() < 12 + 32 || h[0..2] != [true, false] {
        return None;
    }
    let mode = manchester(&h[2..8], (true, false))?.iter().fold(0u8, |n, &b| n << 1 | b as u8);
    let toggle = match h[8..12] {
        [true, true, false, false] => true,
        [false, false, true, true] => false,
        _ => return None,
    };
    let data_bits = manchester(&h[12..], (true, false))?;
    if ![16, 20, 24, 32].contains(&data_bits.len()) {
        return None;
    }
    let data = data_bits.iter().fold(0u32, |n, &b| n << 1 | b as u32);
    Some(Protocol::Rc6 { mode, bits: data_bits.len() as u8, data, toggle })
}

/// A burst cut into frames at its long spaces: each frame, and the space
/// after it (0 after the last).
pub fn frames(burst: &[u32]) -> Vec<(&[u32], u32)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 1;
    while i < burst.len() {
        if burst[i] >= FRAME_GAP {
            out.push((&burst[start..i], burst[i]));
            start = i + 1;
        }
        i += 2;
    }
    if start < burst.len() {
        out.push((&burst[start..], 0));
    }
    out
}

/// Two frames that are the same press sent again.
fn alike(a: &[u32], b: &[u32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| x.abs_diff(y) <= (x.max(y) / 4).max(200))
}

/// What was learned from a burst: the lengths kept, and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Learned {
    pub lengths: Vec<u32>,
    pub protocol: Option<Protocol>,
}

/// A burst made into a button: the first frame a protocol is recognised in;
/// failing that, the frames of the first press (up to the first one that
/// repeats it, or a long pause). `None` for a flicker that is no button.
pub fn learn(burst: &[u32]) -> Option<Learned> {
    let frames = frames(burst);
    for (frame, _) in &frames {
        if let Some(protocol) = decode(frame) {
            return Some(Learned { lengths: frame.to_vec(), protocol: Some(protocol) });
        }
    }
    let mut kept: Vec<u32> = Vec::new();
    let mut first: Option<&[u32]> = None;
    for (frame, gap) in frames.iter().filter(|(f, _)| !is_nec_repeat(f)) {
        if let Some(first) = first {
            if alike(first, frame) {
                break;
            }
        }
        if kept.len() + frame.len() + 1 > MOST_LENGTHS || kept.iter().sum::<u32>() + frame.iter().sum::<u32>() > MOST_TIME {
            break;
        }
        first.get_or_insert(*frame);
        kept.extend_from_slice(frame);
        if *gap == 0 || *gap > 50_000 {
            break;
        }
        kept.push(*gap);
    }
    if kept.len() % 2 == 0 {
        kept.pop();
    }
    (kept.len() >= FEWEST_LENGTHS).then_some(Learned { lengths: kept, protocol: None })
}

/// A burst of light to send: lengths (odd in number, light first) at a carrier.
#[derive(Clone, Debug, PartialEq)]
pub struct Burst {
    pub lengths: Vec<u32>,
    pub carrier: u32,
}

impl Burst {
    /// How long it takes to send.
    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.lengths.iter().map(|&l| l as u64).sum())
    }
}

/// A learned button as it is sent: `held` when it is the same button still
/// held down (a NEC remote sends its repeat code then; RC5 and RC6 keep their
/// toggle), with `toggle` the toggle bit this press carries.
pub fn burst(lengths: &[u32], carrier: u32, held: bool, toggle: bool) -> Burst {
    let Some(protocol) = decode(lengths) else {
        return Burst { lengths: lengths.to_vec(), carrier };
    };
    let frame = match protocol {
        Protocol::Nec { .. } | Protocol::NecX { .. } | Protocol::Nec32 { .. } if held => NEC_REPEAT.to_vec(),
        other => other.with_toggle(toggle).encode(),
    };
    // Sony's receivers want to see a press three times over.
    let copies = if matches!(protocol, Protocol::Sony { .. }) { 3 } else { 1 };
    let mut out = frame.clone();
    let gap = protocol.period().saturating_sub(frame.iter().sum()).max(10_000);
    for _ in 1..copies {
        out.push(gap);
        out.extend_from_slice(&frame);
    }
    Burst { lengths: out, carrier: protocol.carrier() }
}

// ---------------------------------------------------------------- the parts ---

/// Which infrared parts this system has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub transmitter: bool,
    pub receiver: bool,
    /// On a computer: nothing is really sent or heard.
    pub simulated: bool,
}

/// The parts here. On the handheld, config.txt's overlays put a node for
/// each in the device tree whether or not its driver is loaded.
pub fn parts() -> Parts {
    if !cfg!(target_arch = "aarch64") {
        return Parts { transmitter: true, receiver: true, simulated: true };
    }
    let names: Vec<String> = std::fs::read_dir("/proc/device-tree")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let has = |prefix: &str| names.iter().any(|n| n.starts_with(prefix));
    Parts { transmitter: has("gpio-ir-transmitter"), receiver: has("ir-receiver"), simulated: false }
}

#[cfg(target_os = "linux")]
mod lirc {
    //! The kernel's LIRC character devices, `/dev/lirc*`: ioctls from
    //! `<linux/lirc.h>`.
    use std::fs::{File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    pub const GET_FEATURES: u32 = 0x8004_6900;
    pub const SET_REC_MODE: u32 = 0x4004_6912;
    pub const SET_SEND_CARRIER: u32 = 0x4004_6913;
    pub const SET_REC_TIMEOUT: u32 = 0x4004_6918;
    pub const CAN_SEND_PULSE: u32 = 0x0000_0002;
    pub const CAN_SET_SEND_CARRIER: u32 = 0x0000_0100;
    pub const CAN_REC_MODE2: u32 = 0x0004_0000;
    pub const CAN_SET_REC_TIMEOUT: u32 = 0x1000_0000;
    pub const MODE_MODE2: u32 = 0x0000_0004;
    /// What a mode2 sample is, in its top byte; its length in the rest.
    pub const PULSE: u32 = 0x0100_0000;
    pub const SPACE: u32 = 0x0000_0000;
    pub const TIMEOUT: u32 = 0x0300_0000;
    pub const OVERFLOW: u32 = 0x0400_0000;
    pub const KIND: u32 = 0xFF00_0000;
    pub const VALUE: u32 = 0x00FF_FFFF;

    pub fn features(file: &File) -> u32 {
        let mut features = 0u32;
        // SAFETY: LIRC_GET_FEATURES writes one u32 through the pointer.
        let ok = unsafe { libc::ioctl(file.as_raw_fd(), GET_FEATURES as _, &mut features as *mut u32) } == 0;
        if ok {
            features
        } else {
            0
        }
    }

    pub fn set(file: &File, request: u32, value: u32) -> bool {
        // SAFETY: LIRC's set requests read one u32 through the pointer.
        unsafe { libc::ioctl(file.as_raw_fd(), request as _, &value as *const u32) == 0 }
    }

    /// The first `/dev/lirc*` that can do `can`, waiting up to `patience`
    /// for one to appear (a driver just loaded takes a moment).
    pub fn find(can: u32, write: bool, patience: Duration) -> Option<File> {
        let until = Instant::now() + patience;
        loop {
            let mut names: Vec<String> = std::fs::read_dir("/dev")
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("lirc"))
                .collect();
            names.sort();
            for name in names {
                let mut options = OpenOptions::new();
                options.read(!write).write(write);
                if !write {
                    options.custom_flags(libc::O_NONBLOCK);
                }
                if let Ok(file) = options.open(format!("/dev/{name}")) {
                    if features(&file) & can == can {
                        return Some(file);
                    }
                }
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Each driver this shell loaded: how many are using it, and whether it
    /// was loaded here (one that was in already is left in). Loading and
    /// unloading happen under the lock, so a page closing and another opening
    /// at once never unloads a driver from under the newcomer.
    static DRIVERS: std::sync::Mutex<Vec<(&'static str, usize, bool)>> = std::sync::Mutex::new(Vec::new());

    /// A driver wanted: loaded now unless it is in already.
    pub fn acquire(module: &'static str) {
        let Ok(mut drivers) = DRIVERS.lock() else { return };
        if let Some(entry) = drivers.iter_mut().find(|(m, ..)| *m == module) {
            if entry.1 > 0 {
                entry.1 += 1;
                return;
            }
        }
        let ours = !std::path::Path::new("/sys/module").join(module).exists()
            && crate::tools::quiet("modprobe").arg(module).status().is_ok_and(|s| s.success());
        drivers.retain(|(m, ..)| *m != module);
        drivers.push((module, 1, ours));
    }

    /// A driver no longer wanted: unloaded when nothing else here wants it,
    /// if it was loaded here.
    pub fn release(module: &'static str) {
        let Ok(mut drivers) = DRIVERS.lock() else { return };
        let Some(entry) = drivers.iter_mut().find(|(m, ..)| *m == module) else { return };
        entry.1 = entry.1.saturating_sub(1);
        if entry.1 == 0 && entry.2 {
            crate::tools::quiet("modprobe").args(["-r", module]).status().ok();
            entry.2 = false;
        }
    }
}

// ----------------------------------------------------------- the transmitter ---

/// How the transmitter stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sending {
    /// It can send (its driver is loaded and its device open).
    pub ready: bool,
    /// A burst is on its way out now.
    pub busy: bool,
    /// Bursts sent since it was opened.
    pub sent: u64,
    /// What went wrong last, if anything did.
    pub trouble: Option<String>,
}

/// The transmitter, on a thread of its own: a send holds its writer until
/// the light has gone out. Bursts asked for faster than they go (a key held
/// down) are not queued up: only the newest waiting is sent.
pub struct Transmitter {
    to: Option<mpsc::Sender<Burst>>,
    state: Arc<Mutex<Sending>>,
}

impl Transmitter {
    pub fn open() -> Transmitter {
        let (to, from) = mpsc::channel::<Burst>();
        let state = Arc::new(Mutex::new(Sending::default()));
        let shared = state.clone();
        std::thread::Builder::new().name("ir-send".into()).spawn(move || transmit(&from, &shared)).ok();
        Transmitter { to: Some(to), state }
    }

    pub fn send(&self, burst: Burst) {
        if let Some(to) = &self.to {
            to.send(burst).ok();
        }
    }

    pub fn state(&self) -> Sending {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Transmitter {
    fn drop(&mut self) {
        // The thread sees the channel close, and lets its driver go.
        self.to = None;
    }
}

/// Where bursts go.
enum Output {
    #[cfg(target_os = "linux")]
    Device { file: std::fs::File },
    Simulated,
    Missing(String),
}

impl Output {
    fn open() -> Output {
        if parts().simulated {
            return Output::Simulated;
        }
        Self::device()
    }

    #[cfg(target_os = "linux")]
    fn device() -> Output {
        lirc::acquire("gpio_ir_tx");
        match lirc::find(lirc::CAN_SEND_PULSE, true, Duration::from_secs(3)) {
            Some(file) => Output::Device { file },
            None => {
                lirc::release("gpio_ir_tx");
                Output::Missing("no IR transmitter found".into())
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn device() -> Output {
        Output::Missing("no IR on this system".into())
    }

    fn send(&mut self, burst: &Burst) -> Result<(), String> {
        match self {
            #[cfg(target_os = "linux")]
            Output::Device { file } => {
                use std::io::Write;
                if lirc::features(file) & lirc::CAN_SET_SEND_CARRIER != 0 {
                    lirc::set(file, lirc::SET_SEND_CARRIER, burst.carrier);
                }
                let mut lengths = burst.lengths.clone();
                if lengths.len() % 2 == 0 {
                    lengths.pop();
                }
                let bytes: Vec<u8> = lengths.iter().flat_map(|l| l.to_ne_bytes()).collect();
                file.write_all(&bytes).map_err(|e| format!("the IR transmitter failed: {e}"))
            }
            Output::Simulated => {
                std::thread::sleep(burst.duration());
                Ok(())
            }
            Output::Missing(why) => Err(why.clone()),
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Output::Device { .. } = self {
            lirc::release("gpio_ir_tx");
        }
    }
}

fn transmit(from: &mpsc::Receiver<Burst>, state: &Mutex<Sending>) {
    let mut out = Output::open();
    if let Ok(mut s) = state.lock() {
        s.ready = !matches!(out, Output::Missing(_));
        if let Output::Missing(why) = &out {
            s.trouble = Some(why.clone());
        }
    }
    while let Ok(mut burst) = from.recv() {
        while let Ok(newer) = from.try_recv() {
            burst = newer;
        }
        if let Ok(mut s) = state.lock() {
            s.busy = true;
        }
        let result = out.send(&burst);
        if let Ok(mut s) = state.lock() {
            s.busy = false;
            match result {
                Ok(()) => {
                    s.sent += 1;
                    s.trouble = None;
                }
                Err(why) => s.trouble = Some(why),
            }
        }
        // The dark a receiver needs between one frame and the next.
        std::thread::sleep(Duration::from_millis(40));
    }
}

// -------------------------------------------------------------- the receiver ---

/// How the receiver stands.
#[derive(Default)]
struct Hearing {
    ready: bool,
    trouble: Option<String>,
    /// The newest burst heard and not yet taken.
    heard: Option<Vec<u32>>,
    /// When the simulated remote is pressed next.
    next_press: Option<Instant>,
}

/// The receiver, listening on a thread of its own while it is open. Its
/// driver is loaded when it opens and let go when it closes.
pub struct Receiver {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<Hearing>>,
}

/// After the learn page opens (or is asked to listen again), how long the
/// simulated remote waits before its button is pressed.
pub const SIMULATED_PRESS: Duration = Duration::from_millis(1400);

impl Receiver {
    pub fn open() -> Receiver {
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(Hearing { next_press: Some(Instant::now() + SIMULATED_PRESS), ..Hearing::default() }));
        let (s, st) = (stop.clone(), state.clone());
        std::thread::Builder::new().name("ir-hear".into()).spawn(move || listen(&s, &st)).ok();
        Receiver { stop, state }
    }

    /// The newest burst heard since the last one taken.
    pub fn take(&self) -> Option<Vec<u32>> {
        self.state.lock().ok()?.heard.take()
    }

    /// Forget anything heard, and listen afresh.
    pub fn clear(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.heard = None;
            s.next_press = Some(Instant::now() + SIMULATED_PRESS);
        }
    }

    /// Listening (true), still starting (false), or what stopped it.
    pub fn ready(&self) -> Result<bool, String> {
        let s = self.state.lock().map_err(|_| "the receiver stopped".to_owned())?;
        match &s.trouble {
            Some(why) => Err(why.clone()),
            None => Ok(s.ready),
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Mode2 samples into a burst: lengths of light and dark, light first. A
/// burst is done at the driver's timeout or a long dark. (On a computer, only
/// its tests use it: the simulated remote hands over whole bursts.)
#[derive(Default)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Bursts {
    lengths: Vec<u32>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Bursts {
    /// One length of light (`lit`) or dark. Returns a burst that it ended.
    fn add(&mut self, lit: bool, length: u32) -> Option<Vec<u32>> {
        let expecting_light = self.lengths.len() % 2 == 0;
        match (lit, expecting_light) {
            // Dark before any light is just the room.
            (false, _) if self.lengths.is_empty() => None,
            (false, _) if length >= BURST_END => self.end(),
            (l, e) if l == e => {
                self.lengths.push(length);
                // Too long to be a button: what is kept so far is the burst.
                (self.lengths.len() > MOST_LENGTHS).then(|| self.end()).flatten()
            }
            _ => {
                *self.lengths.last_mut().expect("not empty") += length;
                None
            }
        }
    }

    fn end(&mut self) -> Option<Vec<u32>> {
        let mut burst = std::mem::take(&mut self.lengths);
        if burst.len() % 2 == 0 {
            burst.pop();
        }
        (burst.len() >= FEWEST_LENGTHS).then_some(burst)
    }
}

fn listen(stop: &AtomicBool, state: &Mutex<Hearing>) {
    if parts().simulated {
        return listen_simulated(stop, state);
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        lirc::acquire("gpio_ir_recv");
        let Some(mut file) = lirc::find(lirc::CAN_REC_MODE2, false, Duration::from_secs(3)) else {
            if let Ok(mut s) = state.lock() {
                s.trouble = Some("no IR receiver found".into());
            }
            lirc::release("gpio_ir_recv");
            return;
        };
        lirc::set(&file, lirc::SET_REC_MODE, lirc::MODE_MODE2);
        if lirc::features(&file) & lirc::CAN_SET_REC_TIMEOUT != 0 {
            lirc::set(&file, lirc::SET_REC_TIMEOUT, BURST_END);
        }
        if let Ok(mut s) = state.lock() {
            s.ready = true;
        }
        let mut bursts = Bursts::default();
        let mut buf = [0u8; 4 * 256];
        while !stop.load(Ordering::Relaxed) {
            let mut fds = libc::pollfd { fd: file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd, for a file open for as long as this runs.
            if unsafe { libc::poll(&mut fds, 1, 100) } <= 0 {
                continue;
            }
            let n = match file.read(&mut buf) {
                Ok(0) => 0,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => 0,
            };
            if n == 0 || fds.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                // The device went (its driver unloaded under it).
                if let Ok(mut s) = state.lock() {
                    s.trouble = Some("the IR receiver stopped".into());
                }
                break;
            }
            for sample in buf[..n - n % 4].chunks_exact(4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]])) {
                let (kind, value) = (sample & lirc::KIND, sample & lirc::VALUE);
                let ended = match kind {
                    lirc::PULSE => bursts.add(true, value),
                    lirc::SPACE => bursts.add(false, value),
                    lirc::TIMEOUT | lirc::OVERFLOW => bursts.end(),
                    _ => None,
                };
                if let Some(burst) = ended {
                    if let Ok(mut s) = state.lock() {
                        s.heard = Some(burst);
                    }
                }
            }
        }
        drop(file);
        lirc::release("gpio_ir_recv");
    }
}

/// The buttons the simulated remote has, pressed in turn: a TV's (NEC,
/// address 0x04), as an LG's are.
const SIMULATED: [u8; 6] = [0x08, 0x02, 0x03, 0x09, 0x0B, 0x44];
static SIMULATED_NEXT: AtomicUsize = AtomicUsize::new(0);

/// A simulated press as a receiver hears it: light a little long and dark a
/// little short, never quite even, and the repeat code once after it.
pub fn simulated_press() -> Vec<u32> {
    let i = SIMULATED_NEXT.fetch_add(1, Ordering::Relaxed);
    let frame = Protocol::Nec { address: 0x04, command: SIMULATED[i % SIMULATED.len()] }.encode();
    let mut out: Vec<u32> = frame
        .iter()
        .enumerate()
        .map(|(k, &l)| {
            let wobble = ((k * 37 + i * 11) % 9) as i32 * 9 - 36;
            let skew = if k % 2 == 0 { 70 } else { -70 };
            (l as i32 + skew + wobble).max(100) as u32
        })
        .collect();
    out.push(40_000);
    out.extend([9070, 2190, 630]);
    out
}

fn listen_simulated(stop: &AtomicBool, state: &Mutex<Hearing>) {
    // A moment to start, as the driver would take.
    std::thread::sleep(Duration::from_millis(150));
    if let Ok(mut s) = state.lock() {
        s.ready = true;
    }
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(40));
        if let Ok(mut s) = state.lock() {
            if s.heard.is_none() && s.next_press.is_some_and(|at| Instant::now() >= at) {
                s.heard = Some(simulated_press());
                s.next_press = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A remote's timing as a receiver hears it: light stretched by `skew`
    /// µs, dark shortened by as much.
    fn heard(lengths: &[u32], skew: i32) -> Vec<u32> {
        lengths.iter().enumerate().map(|(i, &l)| (l as i32 + if i % 2 == 0 { skew } else { -skew }) as u32).collect()
    }

    #[test]
    fn each_protocol_reads_back_what_it_sends() {
        let all = [
            Protocol::Nec { address: 0x04, command: 0x08 },
            Protocol::NecX { address: 0x1234, command: 0x12 },
            Protocol::Nec32 { data: 0xDEAD_BEEF },
            Protocol::Samsung { address: 0x07, command: 0x02 },
            Protocol::Sony { bits: 12, address: 0x01, command: 0x15 },
            Protocol::Sony { bits: 15, address: 0x97, command: 0x2A },
            Protocol::Sony { bits: 20, address: 0x1A3, command: 0x7F },
            Protocol::Rc5 { address: 0x00, command: 0x0C, toggle: false },
            Protocol::Rc5 { address: 0x14, command: 0x4C, toggle: true },
            Protocol::Rc6 { mode: 0, bits: 16, data: 0x040C, toggle: true },
            Protocol::Rc6 { mode: 6, bits: 32, data: 0x800F_040C, toggle: false },
        ];
        for p in all {
            let frame = p.encode();
            assert_eq!(frame.len() % 2, 1, "{p:?}: light first and last");
            for skew in [0, 90, -60] {
                assert_eq!(decode(&heard(&frame, skew)), Some(p), "{p:?} heard {skew} µs off");
            }
        }
    }

    #[test]
    fn labels_say_what_it_is() {
        assert_eq!(Protocol::Nec { address: 0x04, command: 0x08 }.label(), "NEC · 0x04 · 0x08");
        assert_eq!(Protocol::NecX { address: 0x1234, command: 0x12 }.label(), "NECx · 0x1234 · 0x12");
        assert_eq!(Protocol::Sony { bits: 12, address: 1, command: 0x15 }.label(), "Sony 12 · 0x01 · 0x15");
        assert_eq!(Protocol::Rc6 { mode: 0, bits: 16, data: 0x040C, toggle: false }.label(), "RC6 · 0x04 · 0x0C");
        assert_eq!(Protocol::Rc6 { mode: 6, bits: 32, data: 0x800F_040C, toggle: false }.label(), "RC6-6-32 · 0x800F040C");
        assert_eq!(Protocol::Nec { address: 0x04, command: 0x08 }.short(), "nec 04 08");
    }

    #[test]
    fn a_held_nec_button_is_learned_as_one_frame() {
        let press = simulated_press();
        let learned = learn(&press).expect("a button");
        assert!(matches!(learned.protocol, Some(Protocol::Nec { address: 0x04, .. })));
        assert_eq!(learned.lengths.len(), 67, "the frame alone, not its repeat code");
        // Sent, it is the clean frame; held, the repeat code.
        let sent = burst(&learned.lengths, CARRIER, false, false);
        assert_eq!(decode(&sent.lengths), learned.protocol);
        assert_eq!(sent.carrier, 38_000);
        assert_eq!(burst(&learned.lengths, CARRIER, true, false).lengths, NEC_REPEAT.to_vec());
    }

    #[test]
    fn rc5_flips_its_toggle_on_a_new_press_and_sony_goes_three_times() {
        let rc5 = Protocol::Rc5 { address: 0x14, command: 0x0C, toggle: false }.encode();
        let a = burst(&rc5, CARRIER, false, true);
        let b = burst(&rc5, CARRIER, false, false);
        assert_eq!(decode(&a.lengths), Some(Protocol::Rc5 { address: 0x14, command: 0x0C, toggle: true }));
        assert_eq!(decode(&b.lengths), Some(Protocol::Rc5 { address: 0x14, command: 0x0C, toggle: false }));
        assert_eq!(a.carrier, 36_000);
        let sony = Protocol::Sony { bits: 12, address: 1, command: 0x15 }.encode();
        let sent = burst(&sony, CARRIER, false, false);
        let frames = frames(&sent.lengths);
        assert_eq!(frames.len(), 3);
        assert!(frames.iter().all(|(f, _)| decode(f) == decode(&sony)));
        // Frame to frame, 45 ms.
        let (first, gap) = frames[0];
        assert_eq!(first.iter().sum::<u32>() + gap, 45_000);
        assert_eq!(sent.lengths.len() % 2, 1);
        let mce = Protocol::Rc6 { mode: 6, bits: 32, data: 0x800F_040C, toggle: false };
        assert_eq!(decode(&burst(&mce.encode(), CARRIER, false, true).lengths), Some(Protocol::Rc6 { mode: 6, bits: 32, data: 0x800F_840C, toggle: false }));
    }

    #[test]
    fn an_unknown_remote_is_kept_raw_up_to_its_first_repeat() {
        // An air conditioner's sort of frame: long, its own timing.
        let frame: Vec<u32> = [3400, 1700].into_iter().chain((0..40).flat_map(|i| [420, if i % 3 == 0 { 1300 } else { 420 }])).chain([420]).collect();
        assert_eq!(decode(&frame), None);
        let mut burst_heard = frame.clone();
        burst_heard.push(30_000);
        burst_heard.extend(&frame);
        let learned = learn(&burst_heard).expect("a button");
        assert_eq!(learned.protocol, None);
        assert_eq!(learned.lengths, frame, "once, not again");
        // And sent as it was heard, at the usual carrier.
        assert_eq!(burst(&learned.lengths, CARRIER, false, false), Burst { lengths: frame, carrier: CARRIER });
        // A flicker is no button.
        assert_eq!(learn(&[500, 300, 500]), None);
    }

    #[test]
    fn samples_become_bursts() {
        let mut b = Bursts::default();
        assert_eq!(b.add(false, 1_000_000), None, "the room's dark before anything");
        let frame = Protocol::Nec { address: 1, command: 2 }.encode();
        for (i, &l) in frame.iter().enumerate() {
            // The driver may split one length in two.
            if i == 10 {
                b.add(i % 2 == 0, l / 2);
                b.add(i % 2 == 0, l - l / 2);
            } else {
                assert_eq!(b.add(i % 2 == 0, l), None);
            }
        }
        assert_eq!(b.add(false, BURST_END), Some(frame));
        assert_eq!(b.end(), None, "and nothing left over");
    }
}
