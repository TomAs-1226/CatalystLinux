//! Controllers through Linux's evdev: the 24-byte `input_event` records a
//! `/dev/input/event*` node gives (64-bit: seconds, microseconds, type,
//! code, value), what a device says it has (its axes' ranges, its buttons,
//! rumble), and a pad's state as its reports arrive. The parsing here is
//! plain Rust over bytes, so the tests feed it streams they build; the
//! ioctls are in [`linux`].

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
pub const EV_FF: u16 = 0x15;
pub const SYN_REPORT: u16 = 0;
/// The kernel's buffer ran over: what came since the last report is lost,
/// and the state has to be read again.
pub const SYN_DROPPED: u16 = 3;
pub const EVENT_SIZE: usize = 24;

// Axes (linux/input-event-codes.h).
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_Z: u16 = 0x02;
pub const ABS_RX: u16 = 0x03;
pub const ABS_RY: u16 = 0x04;
pub const ABS_RZ: u16 = 0x05;
pub const ABS_GAS: u16 = 0x09;
pub const ABS_BRAKE: u16 = 0x0a;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;
pub const ABS_MT_SLOT: u16 = 0x2f;

// Buttons.
pub const BTN_JOYSTICK: u16 = 0x120;
pub const BTN_SOUTH: u16 = 0x130;
pub const BTN_EAST: u16 = 0x131;
/// BTN_NORTH and BTN_X share the code: the Xbox driver gives it the X
/// button (on the left), the PlayStation drivers the triangle (on top).
pub const BTN_NORTH: u16 = 0x133;
pub const BTN_WEST: u16 = 0x134;
pub const BTN_TL: u16 = 0x136;
pub const BTN_TR: u16 = 0x137;
pub const BTN_TL2: u16 = 0x138;
pub const BTN_TR2: u16 = 0x139;
pub const BTN_SELECT: u16 = 0x13a;
pub const BTN_START: u16 = 0x13b;
pub const BTN_MODE: u16 = 0x13c;
pub const BTN_THUMBL: u16 = 0x13d;
pub const BTN_THUMBR: u16 = 0x13e;
pub const BTN_DPAD_UP: u16 = 0x220;
pub const BTN_DPAD_DOWN: u16 = 0x221;
pub const BTN_DPAD_LEFT: u16 = 0x222;
pub const BTN_DPAD_RIGHT: u16 = 0x223;
pub const FF_RUMBLE: u16 = 0x50;

pub const SONY: u16 = 0x054c;

/// One record from an event node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub sec: i64,
    pub usec: i64,
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    pub fn new(micros: i64, kind: u16, code: u16, value: i32) -> Event {
        Event { sec: micros.div_euclid(1_000_000), usec: micros.rem_euclid(1_000_000), kind, code, value }
    }
    pub fn micros(&self) -> i64 {
        self.sec * 1_000_000 + self.usec
    }
    /// As the kernel lays it out on a 64-bit machine.
    pub fn bytes(&self) -> [u8; EVENT_SIZE] {
        let mut b = [0u8; EVENT_SIZE];
        b[0..8].copy_from_slice(&self.sec.to_le_bytes());
        b[8..16].copy_from_slice(&self.usec.to_le_bytes());
        b[16..18].copy_from_slice(&self.kind.to_le_bytes());
        b[18..20].copy_from_slice(&self.code.to_le_bytes());
        b[20..24].copy_from_slice(&self.value.to_le_bytes());
        b
    }
}

/// The whole records at the front of `bytes`, and how many bytes they took
/// (a read cut short leaves the rest for the next).
pub fn parse(bytes: &[u8]) -> (Vec<Event>, usize) {
    let n = bytes.len() / EVENT_SIZE;
    let events = (0..n)
        .map(|i| {
            let b = &bytes[i * EVENT_SIZE..(i + 1) * EVENT_SIZE];
            let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
            Event {
                sec: i64_at(0),
                usec: i64_at(8),
                kind: u16::from_le_bytes([b[16], b[17]]),
                code: u16::from_le_bytes([b[18], b[19]]),
                value: i32::from_le_bytes(b[20..24].try_into().unwrap()),
            }
        })
        .collect();
    (events, n * EVENT_SIZE)
}

/// An axis's range, as `EVIOCGABS` gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AbsInfo {
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub fuzz: i32,
    pub flat: i32,
}

/// Who made it and what it is (`EVIOCGID`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ident {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub code: u16,
    pub info: AbsInfo,
    pub value: i32,
}

impl Axis {
    /// −1 to 1 about the middle of its range (a hat's −1, 0, 1 as they are).
    pub fn centred(&self) -> f32 {
        let (lo, hi) = (self.info.min as f32, self.info.max as f32);
        if hi <= lo {
            return 0.0;
        }
        (2.0 * (self.value as f32 - lo) / (hi - lo) - 1.0).clamp(-1.0, 1.0)
    }
    /// 0 to 1 from the bottom of its range (a trigger).
    pub fn pulled(&self) -> f32 {
        let (lo, hi) = (self.info.min as f32, self.info.max as f32);
        if hi <= lo {
            return 0.0;
        }
        ((self.value as f32 - lo) / (hi - lo)).clamp(0.0, 1.0)
    }
}

/// Whether what a device has makes it a game controller: buttons of a
/// gamepad's or a joystick's, and a stick. Its touchpad and motion
/// sensors (a DualShock gives three devices) have neither.
pub fn is_controller(keys: &[u16], axes: &[u16], accelerometer: bool) -> bool {
    let buttons = keys.iter().any(|k| (BTN_JOYSTICK..=BTN_THUMBR).contains(k));
    !accelerometer && buttons && axes.contains(&ABS_X) && axes.contains(&ABS_Y) && !axes.contains(&ABS_MT_SLOT)
}

/// Where a pad's parts are among its axes and buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub left: Option<(u16, u16)>,
    pub right: Option<(u16, u16)>,
    /// Analog triggers, left and right.
    pub triggers: [Option<u16>; 2],
    pub hat: Option<(u16, u16)>,
    /// Numbered buttons (a DirectInput pad: BTN_TRIGGER and on) rather
    /// than a gamepad's named ones.
    pub numbered: bool,
}

impl Layout {
    /// Two sticks where they usually are: X/Y, then RX/RY; Z/RZ are the
    /// triggers beside RX/RY, or the right stick without them (a
    /// DirectInput pad such as the F310 switched to D). As SDL guesses.
    pub fn of(axes: &[u16], keys: &[u16]) -> Layout {
        let has = |c: u16| axes.contains(&c);
        let pair = |a: u16, b: u16| (has(a) && has(b)).then_some((a, b));
        let left = pair(ABS_X, ABS_Y);
        let rxy = pair(ABS_RX, ABS_RY);
        let zrz = pair(ABS_Z, ABS_RZ);
        let (right, triggers) = match (rxy, zrz) {
            (Some(r), _) => (Some(r), [has(ABS_Z).then_some(ABS_Z), has(ABS_RZ).then_some(ABS_RZ)]),
            (None, Some(z)) => (Some(z), [has(ABS_BRAKE).then_some(ABS_BRAKE), has(ABS_GAS).then_some(ABS_GAS)]),
            (None, None) => (None, [has(ABS_BRAKE).then_some(ABS_BRAKE), has(ABS_GAS).then_some(ABS_GAS)]),
        };
        let numbered = !keys.contains(&BTN_SOUTH) && keys.iter().any(|k| (BTN_JOYSTICK..BTN_SOUTH).contains(k));
        Layout { left, right, triggers, hat: pair(ABS_HAT0X, ABS_HAT0Y), numbered }
    }
}

/// A controller and how it stands.
#[derive(Clone, Debug)]
pub struct Pad {
    /// Its node's name under /dev/input (`event5`).
    pub node: String,
    pub name: String,
    pub ident: Ident,
    /// Its serial number or address, where it has one.
    pub serial: String,
    pub axes: Vec<Axis>,
    /// Every button it has, and whether it is down.
    pub buttons: Vec<(u16, bool)>,
    pub rumble: bool,
    pub layout: Layout,
    /// Events lost to an overrun: skip to the next report, then read again.
    pub dropped: bool,
    /// When each report since last taken arrived (µs), and the sticks then.
    pub reports: Vec<Sample>,
    /// The last few gaps between reports (µs), newest last.
    pub gaps: Vec<i64>,
    last_report: Option<i64>,
}

/// The sticks at one report, −1 to 1 each way.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Sample {
    pub at_us: i64,
    pub sticks: [(f32, f32); 2],
}

/// How many reports are kept for whoever takes them, and gaps for the rate.
const KEEP_REPORTS: usize = 4096;
const KEEP_GAPS: usize = 64;

impl Pad {
    pub fn new(node: &str, name: &str, ident: Ident, serial: &str, axes: Vec<(u16, AbsInfo)>, keys: Vec<u16>, rumble: bool) -> Pad {
        let codes: Vec<u16> = axes.iter().map(|(c, _)| *c).collect();
        let layout = Layout::of(&codes, &keys);
        Pad {
            node: node.into(),
            name: name.into(),
            ident,
            serial: serial.into(),
            axes: axes.into_iter().map(|(code, info)| Axis { code, info, value: info.value }).collect(),
            buttons: keys.into_iter().map(|k| (k, false)).collect(),
            rumble,
            layout,
            dropped: false,
            reports: Vec::new(),
            gaps: Vec::new(),
            last_report: None,
        }
    }

    /// The name it is kept under: vendor and product, and its serial.
    pub fn id(&self) -> String {
        let base = format!("{:04x}:{:04x}", self.ident.vendor, self.ident.product);
        let serial: String = self.serial.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
        if serial.is_empty() {
            base
        } else {
            format!("{base}:{}", serial.to_lowercase())
        }
    }

    pub fn sony(&self) -> bool {
        self.ident.vendor == SONY
    }

    pub fn axis(&self, code: u16) -> Option<&Axis> {
        self.axes.iter().find(|a| a.code == code)
    }

    pub fn down(&self, code: u16) -> bool {
        self.buttons.iter().any(|(c, d)| *c == code && *d)
    }

    pub fn has(&self, code: u16) -> bool {
        self.buttons.iter().any(|(c, _)| *c == code)
    }

    /// A stick, −1 to 1 each way (down positive, as evdev has it).
    pub fn stick(&self, which: usize) -> (f32, f32) {
        let pair = if which == 0 { self.layout.left } else { self.layout.right };
        pair.map_or((0.0, 0.0), |(x, y)| (self.axis(x).map_or(0.0, Axis::centred), self.axis(y).map_or(0.0, Axis::centred)))
    }

    /// A trigger, 0 to 1: its axis, or its button where it has no axis.
    pub fn trigger(&self, which: usize) -> f32 {
        match self.layout.triggers[which].and_then(|c| self.axis(c)) {
            Some(a) => a.pulled(),
            None => {
                let b = if self.layout.numbered { BTN_JOYSTICK + 6 + which as u16 } else { BTN_TL2 + which as u16 };
                if self.down(b) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// The D-pad: (x, y), each −1, 0 or 1, from its hat or its buttons.
    pub fn dpad(&self) -> (i32, i32) {
        let mut x = 0;
        let mut y = 0;
        if let Some((hx, hy)) = self.layout.hat {
            x = self.axis(hx).map_or(0, |a| a.value.signum());
            y = self.axis(hy).map_or(0, |a| a.value.signum());
        }
        if self.down(BTN_DPAD_LEFT) {
            x = -1;
        }
        if self.down(BTN_DPAD_RIGHT) {
            x = 1;
        }
        if self.down(BTN_DPAD_UP) {
            y = -1;
        }
        if self.down(BTN_DPAD_DOWN) {
            y = 1;
        }
        (x, y)
    }

    /// Take a run of events. Values change as they come; a report
    /// (`SYN_REPORT`) closes a frame, noting when, and after an overrun
    /// everything up to the next report is let go. True if any report closed.
    pub fn feed(&mut self, events: &[Event]) -> bool {
        let mut any = false;
        for e in events {
            match (e.kind, e.code) {
                (EV_SYN, SYN_DROPPED) => self.dropped = true,
                (EV_SYN, SYN_REPORT) if self.dropped => {
                    // The frame after the loss is whole again; the caller
                    // reads the state afresh (`linux::Device::resync`).
                    self.dropped = false;
                    self.last_report = None;
                }
                (EV_SYN, SYN_REPORT) => {
                    let at = e.micros();
                    if let Some(prev) = self.last_report {
                        if self.gaps.len() >= KEEP_GAPS {
                            self.gaps.remove(0);
                        }
                        self.gaps.push((at - prev).max(0));
                    }
                    self.last_report = Some(at);
                    if self.reports.len() >= KEEP_REPORTS {
                        self.reports.drain(..KEEP_REPORTS / 2);
                    }
                    self.reports.push(Sample { at_us: at, sticks: [self.stick(0), self.stick(1)] });
                    any = true;
                }
                _ if self.dropped => {}
                (EV_ABS, code) => {
                    if let Some(a) = self.axes.iter_mut().find(|a| a.code == code) {
                        a.value = e.value;
                    }
                }
                (EV_KEY, code) => {
                    if let Some(b) = self.buttons.iter_mut().find(|(c, _)| *c == code) {
                        b.1 = e.value != 0;
                    }
                }
                _ => {}
            }
        }
        any
    }

    /// Reports a second, from the middle gap of the last few.
    pub fn rate(&self) -> Option<f32> {
        super::health::median(&self.gaps).filter(|g| *g > 0).map(|g| 1e6 / g as f32)
    }
}

/// A button's name on this pad.
pub fn button_name(code: u16, sony: bool, numbered: bool) -> String {
    if numbered && (BTN_JOYSTICK..BTN_SOUTH).contains(&code) {
        return format!("{}", code - BTN_JOYSTICK + 1);
    }
    let (xbox, ps) = match code {
        BTN_SOUTH => ("A", "cross"),
        BTN_EAST => ("B", "circle"),
        BTN_NORTH => ("X", "triangle"),
        BTN_WEST => ("Y", "square"),
        BTN_TL => ("LB", "L1"),
        BTN_TR => ("RB", "R1"),
        BTN_TL2 => ("LT", "L2"),
        BTN_TR2 => ("RT", "R2"),
        BTN_SELECT => ("view", "share"),
        BTN_START => ("menu", "options"),
        BTN_MODE => ("guide", "PS"),
        BTN_THUMBL => ("LS", "L3"),
        BTN_THUMBR => ("RS", "R3"),
        BTN_DPAD_UP => ("up", "up"),
        BTN_DPAD_DOWN => ("down", "down"),
        BTN_DPAD_LEFT => ("left", "left"),
        BTN_DPAD_RIGHT => ("right", "right"),
        // The Xbox Series share button (xpad gives it KEY_RECORD).
        167 => ("share", "share"),
        0x14a => ("touch", "touch"),
        0x2c0..=0x2e7 => return format!("extra {}", code - 0x2c0 + 1),
        _ => return format!("0x{code:03x}"),
    };
    (if sony { ps } else { xbox }).to_owned()
}

/// An axis's name.
pub fn axis_name(code: u16) -> String {
    match code {
        ABS_X => "x".into(),
        ABS_Y => "y".into(),
        ABS_Z => "z".into(),
        ABS_RX => "rx".into(),
        ABS_RY => "ry".into(),
        ABS_RZ => "rz".into(),
        0x06 => "throttle".into(),
        0x07 => "rudder".into(),
        0x08 => "wheel".into(),
        ABS_GAS => "gas".into(),
        ABS_BRAKE => "brake".into(),
        0x10..=0x17 => format!("hat{}{}", (code - 0x10) / 2, if code % 2 == 0 { "x" } else { "y" }),
        _ => format!("abs 0x{code:02x}"),
    }
}

// ------------------------------------------------------------- the node ---

/// A controller's node, opened: its events read, its rumble played.
#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    /// `_IOC(dir, 'E', nr, size)`.
    const fn ioc(dir: u64, nr: u64, size: u64) -> u64 {
        (dir << 30) | (size << 16) | ((b'E' as u64) << 8) | nr
    }
    const READ: u64 = 2;
    const WRITE: u64 = 1;
    const EVIOCGID: u64 = ioc(READ, 0x02, 8);
    const fn eviocgname(len: u64) -> u64 {
        ioc(READ, 0x06, len)
    }
    const fn eviocguniq(len: u64) -> u64 {
        ioc(READ, 0x08, len)
    }
    const fn eviocgprop(len: u64) -> u64 {
        ioc(READ, 0x09, len)
    }
    const fn eviocgkey(len: u64) -> u64 {
        ioc(READ, 0x18, len)
    }
    const fn eviocgbit(ev: u64, len: u64) -> u64 {
        ioc(READ, 0x20 + ev, len)
    }
    const fn eviocgabs(abs: u64) -> u64 {
        ioc(READ, 0x40 + abs, 24)
    }
    /// `struct ff_effect` is 48 bytes on a 64-bit machine.
    const EVIOCSFF: u64 = ioc(WRITE, 0x80, 48);
    const EVIOCSCLOCKID: u64 = ioc(WRITE, 0xa0, 4);
    const INPUT_PROP_ACCELEROMETER: usize = 0x06;

    fn bits(buf: &[u8]) -> impl Iterator<Item = usize> + '_ {
        buf.iter().enumerate().flat_map(|(i, b)| (0..8).filter(move |k| b & (1 << k) != 0).map(move |k| i * 8 + k))
    }

    pub struct Device {
        file: File,
        writable: bool,
        rumble_id: Option<i16>,
        partial: Vec<u8>,
    }

    fn get(file: &File, request: u64, buf: &mut [u8]) -> bool {
        // SAFETY: each request is sized to the buffer handed to it.
        unsafe { libc::ioctl(file.as_raw_fd(), request as _, buf.as_mut_ptr()) >= 0 }
    }

    fn text(file: &File, request: fn(u64) -> u64) -> String {
        let mut buf = [0u8; 128];
        if !get(file, request(buf.len() as u64), &mut buf) {
            return String::new();
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).trim().to_owned()
    }

    fn absinfo(file: &File, code: u16) -> Option<AbsInfo> {
        let mut b = [0u8; 24];
        get(file, eviocgabs(code as u64), &mut b).then(|| {
            let at = |o: usize| i32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
            AbsInfo { value: at(0), min: at(4), max: at(8), fuzz: at(12), flat: at(16) }
        })
    }

    /// The node, if it is a controller, and the pad as it stands.
    pub fn open(node: &str) -> Option<(Device, Pad)> {
        let path = format!("/dev/input/{node}");
        let (file, writable) = match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path) {
            Ok(f) => (f, true),
            Err(_) => (OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path).ok()?, false),
        };
        let mut keys = [0u8; 96];
        let mut abs = [0u8; 8];
        let mut ff = [0u8; 16];
        let mut props = [0u8; 4];
        get(&file, eviocgbit(EV_KEY as u64, 96), &mut keys);
        get(&file, eviocgbit(EV_ABS as u64, 8), &mut abs);
        get(&file, eviocgprop(4), &mut props);
        let keys: Vec<u16> = bits(&keys).map(|k| k as u16).collect();
        let axes: Vec<u16> = bits(&abs).map(|a| a as u16).collect();
        let accelerometer = bits(&props).any(|p| p == INPUT_PROP_ACCELEROMETER);
        if !is_controller(&keys, &axes, accelerometer) {
            return None;
        }
        let rumble = writable && get(&file, eviocgbit(EV_FF as u64, 16), &mut ff) && bits(&ff).any(|b| b == FF_RUMBLE as usize);
        let mut id = [0u8; 8];
        get(&file, EVIOCGID, &mut id);
        let at = |o: usize| u16::from_ne_bytes([id[o], id[o + 1]]);
        let ident = Ident { bus: at(0), vendor: at(2), product: at(4), version: at(6) };
        // Stamped on the monotonic clock, as the shell's own reads are.
        let clock: libc::c_int = libc::CLOCK_MONOTONIC;
        // SAFETY: EVIOCSCLOCKID takes an int.
        unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID as _, &clock) };
        let mut serial = text(&file, eviocguniq);
        if serial.is_empty() {
            // A USB controller's serial is its USB device's (two levels up
            // from the input device: the interface, then the device).
            serial = std::fs::read_to_string(format!("/sys/class/input/{node}/device/device/../serial")).unwrap_or_default().trim().to_owned();
        }
        let axes: Vec<(u16, AbsInfo)> = axes.into_iter().filter_map(|c| absinfo(&file, c).map(|i| (c, i))).collect();
        let mut pad = Pad::new(node, &text(&file, eviocgname), ident, &serial, axes, keys, rumble);
        let mut device = Device { file, writable, rumble_id: None, partial: Vec::new() };
        device.resync(&mut pad);
        Some((device, pad))
    }

    impl Device {
        /// What has arrived; `Err` once the controller is gone.
        pub fn read(&mut self) -> Result<Vec<Event>, ()> {
            let mut buf = [0u8; EVENT_SIZE * 64];
            let mut events = Vec::new();
            loop {
                match self.file.read(&mut buf) {
                    Ok(0) => return Err(()),
                    Ok(n) => {
                        self.partial.extend_from_slice(&buf[..n]);
                        let (got, used) = parse(&self.partial);
                        events.extend(got);
                        self.partial.drain(..used);
                        if n < buf.len() {
                            return Ok(events);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(events),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return Err(()),
                }
            }
        }

        /// The axes and buttons read afresh (on opening, after an overrun).
        pub fn resync(&mut self, pad: &mut Pad) {
            for a in &mut pad.axes {
                if let Some(info) = absinfo(&self.file, a.code) {
                    a.value = info.value;
                }
            }
            let mut keys = [0u8; 96];
            if get(&self.file, eviocgkey(96), &mut keys) {
                let down: Vec<usize> = bits(&keys).collect();
                for b in &mut pad.buttons {
                    b.1 = down.contains(&(b.0 as usize));
                }
            }
        }

        /// Both motors at three quarters for `ms`.
        pub fn rumble(&mut self, ms: u16) -> bool {
            if !self.writable {
                return false;
            }
            let mut effect = [0u8; 48];
            effect[0..2].copy_from_slice(&FF_RUMBLE.to_ne_bytes());
            effect[2..4].copy_from_slice(&self.rumble_id.unwrap_or(-1).to_ne_bytes());
            effect[10..12].copy_from_slice(&ms.to_ne_bytes());
            effect[16..18].copy_from_slice(&0xc000u16.to_ne_bytes());
            effect[18..20].copy_from_slice(&0xc000u16.to_ne_bytes());
            if !get(&self.file, EVIOCSFF, &mut effect) {
                return false;
            }
            let id = i16::from_ne_bytes([effect[2], effect[3]]);
            self.rumble_id = Some(id);
            let play = Event { sec: 0, usec: 0, kind: EV_FF, code: id as u16, value: 1 };
            self.file.write_all(&play.bytes()).is_ok()
        }
    }

    /// A watch on /dev/input, readable when a node comes or goes.
    pub fn watch() -> Option<File> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain inotify calls; the descriptor is owned by the File.
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return None;
            }
            let file = File::from_raw_fd(fd);
            (libc::inotify_add_watch(fd, c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB) >= 0).then_some(file)
        }
    }
}
