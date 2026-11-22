//! Bluetooth: headphones and speakers. Off unless switched on in Settings ›
//! Bluetooth; only then do D-Bus, bluetoothd and bluealsa run (the
//! catalyst-bluetooth service).
//!
//! The first time it is switched on the handheld restarts once: the radio
//! shares the processor's PL011 serial port with the serial console, and is
//! moved to the other UART at boot (config.txt), so the console stays where
//! it is. After that, on and off are immediate.
//!
//! Pairing asks nothing: headphones pair "just works", with no code to
//! compare. Paired devices are trusted, so they connect again by
//! themselves; while the one chosen is connected, sound goes to it
//! (catalyst-audio). Pairings live on /data/bluetooth.
//!
//! On a computer (the simulator) a made-up room stands in: a pair of
//! headphones paired before, a speaker and earbuds to find.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::git::Job;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const SCRIPT: &str = "/usr/sbin/catalyst-bluetooth";
const FLAG: &str = "/data/catalyst/bluetooth-enabled";
/// How long a look round for devices lasts, in seconds.
const SCAN_SECONDS: u32 = 8;

fn on_device() -> bool {
    Path::new(SCRIPT).exists()
}

/// Whether Bluetooth is switched on (it may still wait for a restart).
pub fn enabled() -> bool {
    if on_device() {
        Path::new(FLAG).exists()
    } else {
        demo().as_ref().is_some_and(|d| d.enabled)
    }
}

/// Settings' one word for it: off, waiting for a restart, on, or the device
/// it plays to.
pub fn summary() -> String {
    if !enabled() {
        return "off".into();
    }
    let st = LAST.lock().unwrap().clone();
    match st {
        Some(s) if s.radio == Radio::Restart => "after a restart".into(),
        Some(Status { connected: Some(address), .. }) => name_of(&address),
        _ => "on".into(),
    }
}

// --------------------------------------------------------------- devices ---

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Headphones,
    Headset,
    Speaker,
    Other,
}

impl Kind {
    fn from_icon(icon: &str) -> Self {
        match icon {
            "audio-headphones" => Kind::Headphones,
            "audio-headset" => Kind::Headset,
            "audio-card" | "audio-speakers" => Kind::Speaker,
            _ => Kind::Other,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::Headphones | Kind::Headset => "headphones",
            Kind::Speaker => "speaker",
            Kind::Other => "bluetooth",
        }
    }

    fn words(self) -> &'static str {
        match self {
            Kind::Headphones => "headphones",
            Kind::Headset => "a headset",
            Kind::Speaker => "a speaker",
            Kind::Other => "audio",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub address: String,
    pub paired: bool,
    pub connected: bool,
    pub kind: Kind,
    pub name: String,
}

/// `catalyst-bluetooth devices`: address, paired|new, connected|-, icon and
/// name, separated by tabs.
pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.splitn(5, '\t');
            let address = f.next()?.trim();
            let paired = f.next()? == "paired";
            let connected = f.next()? == "connected";
            let kind = Kind::from_icon(f.next()?);
            let name = f.next()?.trim();
            (address.len() == 17 && !name.is_empty()).then(|| Device { address: address.to_owned(), paired, connected, kind, name: name.to_owned() })
        })
        .collect()
}

/// Names of devices seen, for pages that only know an address.
static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn remember(devices: &[Device]) {
    let mut names = NAMES.lock().unwrap();
    let names = names.get_or_insert_with(HashMap::new);
    for d in devices {
        names.insert(d.address.to_uppercase(), d.name.clone());
    }
}

/// A device's name, if it has been seen; else its address.
pub fn name_of(address: &str) -> String {
    NAMES.lock().unwrap().as_ref().and_then(|n| n.get(&address.to_uppercase()).cloned()).unwrap_or_else(|| address.to_owned())
}

// ---------------------------------------------------------------- status ---

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Radio {
    #[default]
    Off,
    /// Switched on, the radio comes up at the next start.
    Restart,
    On,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub enabled: bool,
    pub radio: Radio,
    pub powered: bool,
    /// The handheld as others see it.
    pub name: String,
    pub connected: Option<String>,
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "enabled" => s.enabled = value == "yes",
            "radio" => {
                s.radio = match value {
                    "on" => Radio::On,
                    "restart" => Radio::Restart,
                    _ => Radio::Off,
                }
            }
            "powered" => s.powered = value == "yes",
            "name" => s.name = value.to_owned(),
            "connected" if !value.is_empty() => s.connected = Some(value.to_owned()),
            _ => {}
        }
    }
    s
}

/// The last status read, for Settings' summary.
static LAST: Mutex<Option<Status>> = Mutex::new(None);

/// What a read finds: the status, and the devices while it is on.
#[derive(Clone, Debug, Default)]
struct Reading {
    status: Status,
    devices: Vec<Device>,
    /// The codec the device playing plays with (catalyst-audio's word).
    codec: Option<Codec>,
}

/// A Bluetooth stream's codec and rate: "AAC", 48000.
#[derive(Clone, Debug, PartialEq)]
pub struct Codec {
    pub address: String,
    pub name: String,
    pub rate: Option<u32>,
}

impl Codec {
    /// catalyst-audio's /run/catalyst-audio/bluetooth.
    pub fn parse(text: &str) -> Option<Codec> {
        let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).map(str::trim).filter(|v| !v.is_empty());
        Some(Codec { address: field("address")?.to_uppercase(), name: field("codec")?.to_owned(), rate: field("rate").and_then(|r| r.parse().ok()) })
    }

    /// "AAC · 48 kHz".
    pub fn words(&self) -> String {
        match self.rate {
            Some(r) if r % 1000 == 0 => format!("{} · {} kHz", self.name, r / 1000),
            Some(r) => format!("{} · {:.1} kHz", self.name, r as f32 / 1000.0),
            None => self.name.clone(),
        }
    }
}

fn read(scan: bool) -> Result<Reading, String> {
    let reading = if on_device() {
        let status = parse_status(&run(&["status"])?);
        let devices = if status.radio == Radio::On && status.enabled {
            let seconds = SCAN_SECONDS.to_string();
            let args: Vec<&str> = if scan { vec!["scan", &seconds] } else { vec!["devices"] };
            parse_devices(&run(&args).unwrap_or_default())
        } else {
            Vec::new()
        };
        let codec = std::fs::read_to_string("/run/catalyst-audio/bluetooth").ok().and_then(|t| Codec::parse(&t));
        Reading { status, devices, codec }
    } else {
        demo_read(scan)
    };
    remember(&reading.devices);
    *LAST.lock().unwrap() = Some(reading.status.clone());
    Ok(reading)
}

fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(SCRIPT).args(args).stdin(Stdio::null()).output().map_err(|_| "bluetooth is not installed".to_owned())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default().trim().to_owned();
        Err(if why.is_empty() { format!("bluetooth {} failed", args.first().unwrap_or(&"")) } else { why })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Work {
    On,
    Off,
    Pair,
    Connect,
    Disconnect,
    Forget,
    Restart,
}

impl Work {
    fn verb(self) -> &'static str {
        match self {
            Work::On => "on",
            Work::Off => "off",
            Work::Pair => "pair",
            Work::Connect => "connect",
            Work::Disconnect => "disconnect",
            Work::Forget => "forget",
            Work::Restart => "restart",
        }
    }
}

/// The script at work, waited for: what it printed.
fn act(work: Work, address: String) -> Result<String, String> {
    if !on_device() {
        return demo_act(work, &address);
    }
    if work == Work::Restart {
        return Command::new("reboot").status().map(|_| String::new()).map_err(|e| format!("reboot: {e}"));
    }
    if address.is_empty() {
        run(&[work.verb()])
    } else {
        run(&[work.verb(), &address])
    }
}

/// Bluetooth switched on or off, waited for (the assistant, on a thread of
/// its own, once the person said yes): what the script said ("restart"
/// the first time it is switched on).
pub fn switch(on: bool) -> Result<String, String> {
    act(if on { Work::On } else { Work::Off }, String::new())
}

// ------------------------------------------------------------------ demo ---

struct Demo {
    enabled: bool,
    restarted: bool,
    devices: Vec<Device>,
    found: bool,
}

static DEMO: Mutex<Option<Demo>> = Mutex::new(None);

fn demo() -> std::sync::MutexGuard<'static, Option<Demo>> {
    let mut d = DEMO.lock().unwrap();
    if d.is_none() {
        let dev = |address: &str, paired, connected, kind, name: &str| Device { address: address.into(), paired, connected, kind, name: name.into() };
        *d = Some(Demo {
            enabled: false,
            restarted: false,
            devices: vec![
                dev("0C:A6:94:12:34:56", true, false, Kind::Headphones, "WH-1000XM4"),
                dev("11:22:33:44:55:66", false, false, Kind::Speaker, "Kitchen Speaker"),
                dev("98:52:3D:00:7A:02", false, false, Kind::Headset, "Galaxy Buds2 Pro"),
            ],
            found: false,
        });
    }
    d
}

/// The made-up room's device playing now, if one is (the simulator's sound
/// plays there, as the handheld's would).
pub fn demo_connected() -> Option<String> {
    if on_device() {
        return None;
    }
    let d = demo();
    let d = d.as_ref()?;
    (d.enabled && d.restarted).then(|| d.devices.iter().find(|x| x.connected).map(|x| x.address.clone())).flatten()
}

/// The gallery's made-up room as it starts again: off, nothing found.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_reset() {
    *DEMO.lock().unwrap() = None;
}

fn demo_read(scan: bool) -> Reading {
    if scan {
        std::thread::sleep(std::time::Duration::from_millis(1800));
    }
    let mut d = demo();
    let d = d.as_mut().unwrap();
    d.found |= scan;
    let radio = match (d.enabled, d.restarted) {
        (false, _) => Radio::Off,
        (true, false) => Radio::Restart,
        (true, true) => Radio::On,
    };
    let devices: Vec<Device> = if radio == Radio::On { d.devices.iter().filter(|x| x.paired || d.found).cloned().collect() } else { Vec::new() };
    let mut devices = devices;
    devices.sort_by_key(|x| (!x.paired, x.name.to_lowercase()));
    Reading {
        status: Status {
            enabled: d.enabled,
            radio,
            powered: radio == Radio::On,
            name: crate::device::name(),
            connected: devices.iter().find(|x| x.connected).map(|x| x.address.clone()),
        },
        // As bluealsa would play to them: AAC to AirPods, SBC to the rest.
        codec: devices.iter().find(|x| x.connected).map(|x| Codec { address: x.address.clone(), name: if x.name.contains("AirPods") { "AAC".into() } else { "SBC".into() }, rate: Some(48_000) }),
        devices,
    }
}

/// The made-up room with AirPods Pro paired and playing (the galleries).
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_airpods() {
    let mut d = demo();
    let d = d.as_mut().unwrap();
    (d.enabled, d.restarted, d.found) = (true, true, true);
    if !d.devices.iter().any(|x| x.address == "F4:34:F0:12:34:56") {
        d.devices.insert(0, Device { address: "F4:34:F0:12:34:56".into(), paired: true, connected: true, kind: Kind::Headphones, name: "AirPods Pro".into() });
    }
}

fn demo_act(work: Work, address: &str) -> Result<String, String> {
    std::thread::sleep(std::time::Duration::from_millis(if matches!(work, Work::Pair | Work::Connect) { 1500 } else { 300 }));
    let mut d = demo();
    let d = d.as_mut().unwrap();
    let find = |d: &mut Demo| d.devices.iter_mut().position(|x| x.address == address);
    match work {
        Work::On => {
            d.enabled = true;
            return Ok(if d.restarted { String::new() } else { "restart\n".into() });
        }
        Work::Off => d.enabled = false,
        // The simulator "restarts" at once.
        Work::Restart => d.restarted = true,
        Work::Pair | Work::Connect => {
            let at = find(d).ok_or("no such device")?;
            for x in d.devices.iter_mut() {
                x.connected = false;
            }
            d.devices[at].paired = true;
            d.devices[at].connected = true;
        }
        Work::Disconnect => {
            if let Some(at) = find(d) {
                d.devices[at].connected = false;
            }
        }
        Work::Forget => {
            if let Some(at) = find(d) {
                d.devices[at].paired = false;
                d.devices[at].connected = false;
            }
        }
    }
    Ok(String::new())
}

// ---------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Switch,
    Restart,
    Seen,
    Device(usize),
    /// Under the device playing: the codec it plays with.
    Codec,
    /// Under AirPods playing: a bud's battery, or the case's.
    Battery(aap::Part),
    Look,
}

/// Settings › Bluetooth: switching it on (once with a restart), the devices
/// paired and found, pairing, connecting and forgetting them, and a look
/// round for more. Under the device playing, the codec it plays with; under
/// AirPods, the battery of each bud and the case (as they say it, over
/// their own protocol: airpods.rs).
pub struct BluetoothScreen {
    list: ListState,
    reading: Option<Job<Result<Reading, String>>>,
    last: Option<Reading>,
    read_at: Instant,
    scanning: bool,
    /// Looked round once already since the page opened.
    looked: bool,
    working: Option<(Work, String, Job<Result<String, String>>)>,
    /// Asked once: forget this device (or restart); a second press does it.
    armed: Option<Row>,
    /// The AirPods connected, as last looked at.
    pods: Option<crate::airpods::State>,
    /// Each battery row's bar, gliding to its level.
    bars: [detent::Animated; 4],
    changed: bool,
}

const PARTS: [aap::Part; 4] = [aap::Part::Left, aap::Part::Right, aap::Part::Case, aap::Part::Headphones];

fn part_name(p: aap::Part) -> &'static str {
    match p {
        aap::Part::Left => "Left bud",
        aap::Part::Right => "Right bud",
        aap::Part::Case => "Case",
        aap::Part::Headphones => "Battery",
    }
}

impl BluetoothScreen {
    pub fn new() -> Self {
        let mut page = BluetoothScreen {
            list: ListState::default(),
            reading: None,
            last: None,
            read_at: Instant::now(),
            scanning: false,
            looked: false,
            working: None,
            armed: None,
            pods: crate::airpods::now(),
            bars: std::array::from_fn(|_| detent::Animated::new(0.0, detent::role::hold()).with_rest(0.003)),
            changed: true,
        };
        page.read_now(false);
        page
    }

    /// The AirPods connected, if they are this device.
    fn pods_for(&self, address: &str) -> Option<&crate::airpods::State> {
        self.pods.as_ref().filter(|p| p.address.eq_ignore_ascii_case(address))
    }

    /// The codec the device playing plays with, if it is this one.
    fn codec_for(&self, address: &str) -> Option<&Codec> {
        self.last.as_ref()?.codec.as_ref().filter(|c| c.address.eq_ignore_ascii_case(address))
    }

    /// The device the rows under it belong to: the one playing.
    fn playing(&self) -> Option<&Device> {
        self.devices().iter().find(|d| d.connected)
    }

    fn read_now(&mut self, scan: bool) {
        if self.reading.is_some() {
            return;
        }
        self.read_at = Instant::now();
        self.scanning = scan;
        self.reading = Some(Job::start(move || read(scan)));
    }

    fn status(&self) -> Status {
        self.last.as_ref().map(|r| r.status.clone()).unwrap_or(Status { enabled: enabled(), ..Status::default() })
    }

    fn devices(&self) -> &[Device] {
        self.last.as_ref().map_or(&[][..], |r| &r.devices)
    }

    fn rows(&self) -> Vec<Row> {
        let st = self.status();
        let mut rows = vec![Row::Switch];
        match (st.enabled, st.radio) {
            (true, Radio::Restart) => rows.push(Row::Restart),
            (true, Radio::On) => {
                if !st.name.is_empty() {
                    rows.push(Row::Seen);
                }
                for (i, d) in self.devices().iter().enumerate() {
                    rows.push(Row::Device(i));
                    if !d.connected {
                        continue;
                    }
                    if self.codec_for(&d.address).is_some() {
                        rows.push(Row::Codec);
                    }
                    if let Some(p) = self.pods_for(&d.address) {
                        rows.extend(PARTS.iter().filter(|part| p.cell(**part).is_some()).map(|part| Row::Battery(*part)));
                    }
                }
                rows.push(Row::Look);
            }
            _ => {}
        }
        rows
    }

    fn start(&mut self, work: Work, address: String) {
        if self.working.is_some() {
            return;
        }
        let a = address.clone();
        self.working = Some((work, address, Job::start(move || act(work, a))));
        self.changed = true;
    }

    fn busy_with(&self, address: &str) -> Option<Work> {
        self.working.as_ref().filter(|(_, a, _)| a == address).map(|(w, _, _)| *w)
    }

    /// A row's name, value, whether it is lit, and its icon.
    fn describe(&self, row: Row) -> (String, String, bool) {
        let st = self.status();
        match row {
            Row::Switch => {
                let value = match (self.working.as_ref().map(|w| w.0), st.enabled, st.radio) {
                    (Some(Work::On), ..) => "switching on…",
                    (Some(Work::Off), ..) => "switching off…",
                    (_, false, _) => "off",
                    (_, true, Radio::Restart) => "after a restart",
                    _ => "on",
                };
                ("Bluetooth".into(), value.into(), st.enabled)
            }
            Row::Restart => ("Restart now".into(), (if self.armed == Some(Row::Restart) { "enter again" } else { "" }).into(), true),
            Row::Seen => ("Seen as".into(), st.name.clone(), true),
            Row::Device(i) => match self.devices().get(i) {
                Some(d) => {
                    let value = match self.busy_with(&d.address) {
                        Some(Work::Pair) => "pairing…",
                        Some(Work::Connect) => "connecting…",
                        Some(Work::Disconnect) => "disconnecting…",
                        Some(Work::Forget) => "forgetting…",
                        _ if self.armed == Some(row) => "del again: forget",
                        _ if d.connected => "playing",
                        _ if d.paired => "paired",
                        _ => "pair",
                    };
                    (d.name.clone(), value.into(), d.connected || !d.paired)
                }
                None => Default::default(),
            },
            Row::Codec => {
                let codec = self.playing().and_then(|d| self.codec_for(&d.address));
                ("Codec".into(), codec.map(Codec::words).unwrap_or_default(), true)
            }
            Row::Battery(part) => {
                let cell = self.playing().and_then(|d| self.pods_for(&d.address)).and_then(|p| p.cell(part));
                (part_name(part).into(), cell.map(crate::airpods::cell_words).unwrap_or_default(), cell.is_some_and(|c| c.level > 10))
            }
            Row::Look => ("Look for devices".into(), (if self.scanning { "looking…" } else { "" }).into(), true),
        }
    }

    /// What the keys do on a row: the longest that fits whole first.
    fn hint(&self, row: Row) -> Vec<String> {
        let st = self.status();
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect();
        match row {
            Row::Switch if !st.enabled => words(&["enter: switch on · the first time, it restarts", "enter: switch on"]),
            Row::Switch => words(&["enter: switch off"]),
            Row::Restart if self.armed == Some(Row::Restart) => words(&["enter again: restart · esc: later", "enter again: restart"]),
            Row::Restart => words(&["the radio starts with the handheld, once", "enter: restart"]),
            Row::Seen => words(&["the handheld's name, as headphones see it", "the handheld's name"]),
            Row::Device(i) => match self.devices().get(i) {
                Some(d) if d.connected => vec![format!("{} · enter: disconnect · del: forget", d.kind.words()), "enter: disconnect · del: forget".into()],
                Some(d) if d.paired => vec![format!("{} · enter: connect · del: forget", d.kind.words()), "enter: connect · del: forget".into()],
                Some(d) => vec![format!("{} · enter: pair and play here", d.kind.words()), "enter: pair and play here".into()],
                None => Vec::new(),
            },
            Row::Codec => match self.playing().and_then(|d| self.codec_for(&d.address)).map(|c| c.name.as_str()) {
                Some("AAC") => words(&["aac, as an iphone sends it, 256 kb/s", "aac, as an iphone sends it", "aac"]),
                Some("SBC") => words(&["sbc: the codec every pair takes", "sbc, which every pair takes", "sbc"]),
                Some(other) => vec![format!("{}, chosen with these headphones", other.to_lowercase()), other.to_lowercase()],
                None => Vec::new(),
            },
            Row::Battery(aap::Part::Case) => words(&["the case's, while it is open near them", "the case's, while open", "the case"]),
            Row::Battery(_) => words(&["as the airpods say it, over their own link", "as the airpods say it", "the airpods'"]),
            Row::Look if self.scanning => words(&["put headphones in pairing mode now", "pairing mode now"]),
            Row::Look => words(&["enter: look again · headphones in pairing mode", "enter: look again"]),
        }
    }
}

impl Screen for BluetoothScreen {
    fn title(&self) -> &str {
        "Bluetooth"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = self.rows();
        let row = rows.get(self.list.focus).copied().unwrap_or(Row::Switch);
        let armed = self.armed.take();
        match key {
            Key::Back if armed.is_some() => {}
            Key::Back => return Nav::Pop,
            _ if self.working.is_some() && matches!(key, Key::Enter | Key::Delete) => *cx.say = Some("a moment…".into()),
            Key::Enter => match row {
                Row::Switch => self.start(if self.status().enabled { Work::Off } else { Work::On }, String::new()),
                Row::Restart if armed == Some(Row::Restart) => self.start(Work::Restart, String::new()),
                Row::Restart => self.armed = Some(Row::Restart),
                Row::Device(i) => {
                    if let Some(d) = self.devices().get(i).cloned() {
                        let work = if d.connected {
                            Work::Disconnect
                        } else if d.paired {
                            Work::Connect
                        } else {
                            Work::Pair
                        };
                        self.start(work, d.address);
                    }
                }
                Row::Look if !self.scanning => {
                    self.reading = None;
                    self.read_now(true);
                }
                _ => {}
            },
            Key::Delete => {
                if let Row::Device(i) = row {
                    if let Some(d) = self.devices().get(i).filter(|d| d.paired).cloned() {
                        if armed == Some(row) {
                            self.start(Work::Forget, d.address);
                        } else {
                            self.armed = Some(row);
                        }
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        self.changed = true;
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.working.as_ref().and_then(|(_, _, job)| job.get()) {
            let (work, address, _) = self.working.take().unwrap();
            let name = name_of(&address);
            *cx.say = Some(match (work, done) {
                (_, Err(why)) => why.to_lowercase(),
                (Work::On, Ok(out)) if out.contains("restart") => "on after a restart".into(),
                (Work::On, _) => "bluetooth on".into(),
                (Work::Off, _) => "bluetooth off".into(),
                (Work::Pair | Work::Connect, _) => format!("playing on {name}"),
                (Work::Disconnect, _) => format!("{name} disconnected"),
                (Work::Forget, _) => format!("{name} forgotten"),
                (Work::Restart, _) => "restarting…".into(),
            });
            self.reading = None;
            self.read_now(false);
        }
        if let Some(done) = self.reading.as_ref().and_then(Job::get) {
            self.reading = None;
            self.scanning = false;
            match done {
                Ok(r) => self.last = Some(r),
                Err(why) => *cx.say = Some(why),
            }
            // Opened on: look round once, so what is in pairing mode shows.
            let st = self.status();
            if !self.looked && st.enabled && st.radio == Radio::On {
                self.looked = true;
                self.read_now(true);
            }
            let len = self.rows().len();
            self.list.focus = self.list.focus.min(len.saturating_sub(1));
            self.changed = true;
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        if self.read_at.elapsed().as_secs_f32() >= 3.0 && self.reading.is_none() && self.working.is_none() {
            self.read_now(false);
        }
        let pods = crate::airpods::now();
        if pods != self.pods {
            let row = self.rows().get(self.list.focus).copied();
            self.pods = pods;
            let rows = self.rows();
            self.list.focus = row.and_then(|r| rows.iter().position(|x| *x == r)).unwrap_or(self.list.focus.min(rows.len().saturating_sub(1)));
            self.changed = true;
        }
        let mut moving = false;
        for (bar, part) in self.bars.iter_mut().zip(PARTS) {
            let level = self.pods.as_ref().and_then(|p| p.cell(part)).map_or(0.0, |c| c.level as f32 / 100.0);
            if bar.target() != level {
                bar.set(level);
            }
            moving |= bar.tick(dt);
        }
        std::mem::take(&mut self.changed) || self.scanning || self.working.is_some() || moving
    }

    fn busy(&self) -> bool {
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = self.rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        let mut below = top;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let row = rows[i];
            let (name, value, lit) = self.describe(row);
            // A device's kind leads its row: headphones, a speaker.
            let icon = match row {
                // AirPods by their own shape, as the island shows them.
                Row::Device(d) => self.devices().get(d).map(|d| if self.pods_for(&d.address).is_some_and(|p| !aap::is_max(&p.model)) { "earbuds" } else { d.kind.icon() }),
                Row::Look => Some(if self.scanning { "bluetooth_searching" } else { "search" }),
                _ => None,
            };
            let r = Rect::new(PAD, y, width, row_h);
            let value_color = if lit { t.ink } else { t.dim };
            match icon {
                Some(icon) => {
                    let on = matches!(row, Row::Device(d) if self.devices().get(d).is_some_and(|d| d.connected));
                    icon_row(g, r, icon, on, &name, &value, value_color, i == self.list.focus);
                }
                None => g.row(r, &name, &value, value_color, i == self.list.focus),
            }
            // A battery's level as a bar between its name and value, as the
            // volume's is on Sound.
            if let Row::Battery(part) = row {
                let at = PARTS.iter().position(|p| *p == part).unwrap_or(0);
                // The same for every part, so the bars line up.
                let name_w = PARTS.iter().map(|p| part_name(*p)).map(|n| g.measure(&n.to_lowercase(), ty.label).max(g.measure(n, ty.body))).fold(0.0, f32::max) + 22.0;
                let value_w = g.measure("100 % · charging", ty.body).max(g.measure(&value, ty.label)) + 22.0;
                let room = r.w - name_w - value_w;
                if room > 24.0 {
                    g.meter(Rect::new(r.x + name_w, r.cy() - 3.0, room, 6.0), self.bars[at].value(), t.ice);
                }
            }
            below = below.max(r.bottom());
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        // A few words under a short page.
        let st = self.status();
        let words = match (st.enabled, st.radio) {
            (false, _) => "Plays to Bluetooth headphones and speakers. Off until switched on; the first time, the handheld restarts once to start the radio.",
            (true, Radio::Restart) => "The radio shares a port with the serial console, and is moved to another as the handheld starts.",
            (true, Radio::On) if self.devices().is_empty() && !self.scanning => "Nothing found yet: put the headphones in pairing mode and look again.",
            _ => "",
        };
        let mut y = below + 10.0 + ty.caption.size * 0.5;
        for line in crate::help::wrap(g, words, ty.caption, width - 8.0) {
            if y + ty.caption.size * 0.5 > bottom {
                break;
            }
            g.text_at(PAD + 4.0, y, Align::Left, &line, ty.caption, t.dim);
            y += ty.caption.size + 4.0;
        }
        let hint = rows.get(self.list.focus).map_or_else(Vec::new, |row| self.hint(*row));
        let choices: Vec<&str> = hint.iter().map(String::as_str).collect();
        g.hint_any(&choices);
    }
}

/// A list row led by an icon: the row as every list draws it, its name moved
/// along to make room. A lit icon (filled, in ice) is one that is on.
#[allow(clippy::too_many_arguments)]
pub fn icon_row(g: &mut Gfx, r: Rect, icon: &str, lit: bool, name: &str, value: &str, value_color: bezel::Color, focused: bool) {
    let t = g.tokens;
    g.row(r, "", value, value_color, focused);
    // Named as the row would name it: lowercase mono in Bezel, Sans in Catalyst.
    let bezel = g.theme == crate::look::Theme::Bezel;
    let (style, text, value_style) = if bezel { (g.ty.label, name.to_lowercase(), g.ty.body) } else { (g.ty.body, name.to_owned(), g.ty.label) };
    let value_w = g.measure(value, value_style);
    g.icon(icon, lit, r.x + 17.0, r.cy(), 14.0, if lit { t.ice } else { t.dim });
    g.text_fit(r.x + 30.0, r.cy(), &text, style, t.ink, r.w - 50.0 - value_w);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_read_from_the_scripts_lines() {
        let text = "11:22:33:44:55:66\tpaired\tconnected\taudio-card\tKitchen Speaker\n0C:A6:94:12:34:56\tnew\t-\taudio-headphones\tWH-1000XM4\nbroken line\n";
        let d = parse_devices(text);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].paired, d[0].connected, d[0].kind, d[0].name.as_str()), (true, true, Kind::Speaker, "Kitchen Speaker"));
        assert_eq!((d[1].paired, d[1].connected, d[1].kind), (false, false, Kind::Headphones));
        remember(&d);
        assert_eq!(name_of("0c:a6:94:12:34:56"), "WH-1000XM4");
        assert_eq!(name_of("00:00:00:00:00:00"), "00:00:00:00:00:00");
    }

    #[test]
    fn every_hint_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut page = BluetoothScreen::new();
        let dev = |paired, connected| Device { address: "11:22:33:44:55:66".into(), paired, connected, kind: Kind::Headphones, name: "x".into() };
        page.last = Some(Reading { status: Status { enabled: true, radio: Radio::On, name: "atlas".into(), ..Status::default() }, devices: vec![dev(true, true), dev(true, false), dev(false, false)], codec: Some(Codec { address: "11:22:33:44:55:66".into(), name: "AAC".into(), rate: Some(44_100) }) });
        let rows = [Row::Switch, Row::Restart, Row::Seen, Row::Device(0), Row::Device(1), Row::Device(2), Row::Codec, Row::Battery(aap::Part::Left), Row::Battery(aap::Part::Case), Row::Look];
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let caption = g.ty.caption;
            for row in rows {
                let hints = page.hint(row);
                let shortest = hints.last().unwrap();
                assert!(g.measure(shortest, caption) <= W - 2.0 * PAD, "{shortest:?} at {size:?}");
            }
        }
    }

    #[test]
    fn the_codec_and_airpods_battery_sit_under_the_device_playing() {
        assert_eq!(Codec::parse("address=f4:34:f0:12:34:56\ncodec=AAC\nrate=44100\nicon=audio-headphones\n").map(|c| c.words()), Some("AAC · 44.1 kHz".into()));
        assert_eq!(Codec::parse("address=F4:34:F0:12:34:56\ncodec=SBC\nrate=48000\n").map(|c| c.words()), Some("SBC · 48 kHz".into()));
        assert_eq!(Codec::parse("codec=\n"), None);
        let mut page = BluetoothScreen::new();
        let pods = Device { address: "F4:34:F0:12:34:56".into(), paired: true, connected: true, kind: Kind::Headphones, name: "AirPods Pro".into() };
        let other = Device { address: "0C:A6:94:12:34:56".into(), paired: true, connected: false, kind: Kind::Headphones, name: "WH-1000XM4".into() };
        page.last = Some(Reading { status: Status { enabled: true, radio: Radio::On, ..Status::default() }, devices: vec![pods, other], codec: Codec::parse("address=F4:34:F0:12:34:56\ncodec=AAC\nrate=48000\n") });
        crate::airpods::demo(true);
        page.pods = crate::airpods::now();
        crate::airpods::demo(false);
        assert_eq!(page.rows(), [Row::Switch, Row::Device(0), Row::Codec, Row::Battery(aap::Part::Left), Row::Battery(aap::Part::Right), Row::Battery(aap::Part::Case), Row::Device(1), Row::Look]);
        assert_eq!(page.describe(Row::Codec).1, "AAC · 48 kHz");
        assert_eq!(page.describe(Row::Battery(aap::Part::Right)).1, "78 %");
        assert_eq!(page.describe(Row::Battery(aap::Part::Case)).1, "46 % · charging");
        // Every row whole, at every text size, in both looks.
        let mut g = Gfx::new(bezel::Tokens::dark());
        let width = W - 2.0 * PAD - 5.0;
        for look in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = look;
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let (name_style, value_style) = if look == crate::look::Theme::Bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                for row in page.rows() {
                    let (name, value, _) = page.describe(row);
                    let name = if look == crate::look::Theme::Bezel { name.to_lowercase() } else { name };
                    let need = g.measure(&name, name_style) + g.measure(&value, value_style) + 50.0;
                    assert!(need <= width, "{name} · {value} at {size:?} in {look:?}");
                }
            }
        }
    }

    #[test]
    fn the_status_says_when_a_restart_is_needed() {
        let s = parse_status("enabled=yes\nradio=restart\n");
        assert!(s.enabled && s.radio == Radio::Restart && s.connected.is_none());
        let s = parse_status("enabled=yes\nradio=on\npowered=yes\nname=atlas\nconnected=11:22:33:44:55:66\n");
        assert_eq!((s.radio, s.powered, s.name.as_str(), s.connected.as_deref()), (Radio::On, true, "atlas", Some("11:22:33:44:55:66")));
        assert_eq!(parse_status("enabled=no\nradio=off\nconnected=\n"), Status::default());
    }
}
