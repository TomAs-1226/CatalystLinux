//! Sound: the volume, the speaker and headphones, the microphone.
//!
//! The system's side is `catalyst-audio` (an ES8389 codec on I2S; ALSA's
//! "default" mixes every program at the system volume). This module keeps
//! the volume where the volume keys, the control centre and Settings ›
//! Sound all see the same value: it is read once from catalyst-audio's own
//! files on /data, changed here at once, and handed to the script by a
//! thread of its own, so a held key never waits on a process (and a run of
//! presses is applied as the last of them).
//!
//! **Where sound goes** is catalyst-audio's to decide: the speaker (or
//! headphones on the jack, which the kernel switches), USB headphones while
//! they are plugged in, Bluetooth headphones while they are connected; of
//! USB and Bluetooth, the one connected last. It writes its answer to
//! /run/catalyst-audio, and a thread here sleeps on a watch of that folder
//! (mdev runs catalyst-audio as a USB sound card comes or goes), so nothing
//! is polled. As sound moves, Music follows: on where it was through the
//! new headphones, or paused when the USB headphones it played through come
//! out (as a phone does, rather than carry on aloud); the island says so,
//! and the volume keys move the volume of what plays now. USB headphones
//! keep a volume of their own; the speaker, the jack and Bluetooth share
//! the main one, as before.
//!
//! On a computer (the simulator) nothing plays: the level is kept in memory,
//! the microphone's meter moves as if someone were talking, and USB
//! headphones are plugged in and out by the gallery.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bezel::Rect;

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::git::Job;
use crate::platform::{Key, Media};
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const SCRIPT: &str = "/usr/sbin/catalyst-audio";
const STATE: &str = "/data/catalyst";
/// Where catalyst-audio says where sound goes: `output` (local, bluetooth,
/// or "usb N" for card N) and `usb` (the USB headphones there).
const RUN: &str = "/run/catalyst-audio";
/// What the volume keys move it by.
pub const STEP: u32 = 5;
/// catalyst-audio's own starting level, for a handheld that has never set one.
const DEFAULT_VOLUME: u32 = 65;
/// And for USB headphones, close to the ear.
const DEFAULT_USB_VOLUME: u32 = 40;
/// The microphone's amplifier settings the codec allows (4 and 5 are not),
/// and what each gives, by the ES8389's datasheet.
const MIC_GAINS: [(u32, &str); 13] = [
    (0, "0 dB"),
    (1, "3.5 dB"),
    (2, "6.5 dB"),
    (3, "9.5 dB"),
    (6, "12.5 dB"),
    (7, "15.5 dB"),
    (8, "18.5 dB"),
    (9, "21.5 dB"),
    (10, "24.5 dB"),
    (11, "27.5 dB"),
    (12, "30.5 dB"),
    (13, "33.5 dB"),
    (14, "36.5 dB"),
];
const DEFAULT_MIC_GAIN: u32 = 8;

pub fn on_device() -> bool {
    Path::new(SCRIPT).exists()
}

// ----------------------------------------------------------------- volume ---

/// The volume (0 to 100) and whether it is muted, as the system has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub volume: u32,
    pub muted: bool,
}

impl Level {
    /// What plays: nothing while muted.
    pub fn heard(self) -> u32 {
        if self.muted {
            0
        } else {
            self.volume
        }
    }

    /// The icon for it: off, low or full.
    pub fn icon(self) -> &'static str {
        match self.heard() {
            0 => "volume_off",
            1..=40 => "volume_down",
            _ => "volume_up",
        }
    }

    /// "volume 60%", or "sound off", for the island.
    pub fn words(self) -> String {
        if self.heard() == 0 {
            "sound off".into()
        } else {
            format!("volume {}%", self.volume)
        }
    }
}

/// Whose volume: the main one (the speaker, the jack's headphones and
/// Bluetooth, as catalyst-audio's "Master") or USB headphones' own. Each is
/// kept for its own (audio-volume, audio-volume-usb); silence is one for all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Main,
    Usb,
}

impl Owner {
    /// Its name to catalyst-audio.
    fn word(self) -> &'static str {
        match self {
            Owner::Main => "main",
            Owner::Usb => "usb",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Owner::Main => "audio-volume",
            Owner::Usb => "audio-volume-usb",
        }
    }

    fn start_at(self) -> u32 {
        match self {
            Owner::Main => DEFAULT_VOLUME,
            Owner::Usb => DEFAULT_USB_VOLUME,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The volumes as known here (each read once from catalyst-audio's files),
/// and silence.
struct Levels {
    volume: [Option<u32>; 2],
    muted: Option<bool>,
}

static LEVELS: Mutex<Levels> = Mutex::new(Levels { volume: [None, None], muted: None });

/// Tests that change the volume hold this, one at a time: it is the one
/// volume the whole shell shares.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());
static WORKER: OnceLock<Mutex<Sender<(Owner, Level)>>> = OnceLock::new();

/// A volume as saved by catalyst-audio (its files on /data).
fn saved_volume(owner: Owner) -> u32 {
    if !on_device() {
        return owner.start_at();
    }
    std::fs::read_to_string(Path::new(STATE).join(owner.file())).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(owner.start_at()).min(100)
}

fn saved_muted() -> bool {
    on_device() && Path::new(STATE).join("audio-muted").exists()
}

/// The volume now: of what plays now.
pub fn level() -> Level {
    level_for(route().owner())
}

fn level_for(owner: Owner) -> Level {
    let mut l = LEVELS.lock().unwrap();
    let volume = *l.volume[owner.index()].get_or_insert_with(|| saved_volume(owner));
    let muted = *l.muted.get_or_insert_with(saved_muted);
    Level { volume, muted }
}

/// Set the volume of what plays now, and mute, together: at once here and
/// soon on the handheld.
pub fn set_level(level: Level) -> Level {
    let owner = route().owner();
    let level = Level { volume: level.volume.min(100), muted: level.muted };
    {
        let mut l = LEVELS.lock().unwrap();
        l.volume[owner.index()] = Some(level.volume);
        l.muted = Some(level.muted);
    }
    if on_device() {
        let sender = WORKER.get_or_init(|| Mutex::new(start_worker()));
        sender.lock().unwrap().send((owner, level)).ok();
    }
    level
}

/// The volume up or down by `delta`. Turning it up while muted unmutes, as
/// catalyst-audio does.
pub fn step(delta: i32) -> Level {
    let now = level();
    let volume = (now.volume as i32 + delta).clamp(0, 100) as u32;
    set_level(Level { volume, muted: now.muted && delta <= 0 })
}

pub fn toggle_mute() -> Level {
    let now = level();
    set_level(Level { muted: !now.muted, ..now })
}

/// What the island says of a level: "volume 60%", or "headphones 60%"
/// while USB headphones play (it is theirs), or "sound off".
pub fn said(level: Level) -> String {
    match route().owner() {
        Owner::Usb if level.heard() > 0 => format!("headphones {}%", level.volume),
        _ => level.words(),
    }
}

/// The volume's icon: headphones while USB ones play and are heard.
pub fn icon() -> &'static str {
    let l = level();
    if route().owner() == Owner::Usb && l.heard() > 0 {
        "headphones"
    } else {
        l.icon()
    }
}

/// The thread that hands levels to catalyst-audio, the last of a run of
/// them for each output.
fn start_worker() -> Sender<(Owner, Level)> {
    let (tx, rx): (Sender<(Owner, Level)>, Receiver<(Owner, Level)>) = channel();
    std::thread::Builder::new()
        .name("sound".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            // As saved when the shell started: only what changes is sent.
            let mut applied = [saved_volume(Owner::Main), saved_volume(Owner::Usb)];
            let mut applied_muted = saved_muted();
            while let Ok((owner, level)) = rx.recv() {
                // Presses that came meanwhile: only the last counts.
                let mut want = [None, None];
                want[owner.index()] = Some(level.volume);
                let mut muted = level.muted;
                while let Ok((owner, newer)) = rx.try_recv() {
                    want[owner.index()] = Some(newer.volume);
                    muted = newer.muted;
                }
                // Muting first and unmuting last, so a level set while
                // muted is never heard on the way.
                if muted && !applied_muted {
                    run(&["mute", "on"]).ok();
                }
                for owner in [Owner::Main, Owner::Usb] {
                    if let Some(volume) = want[owner.index()].filter(|v| *v != applied[owner.index()]) {
                        run(&["volume", &volume.to_string(), owner.word()]).ok();
                        applied[owner.index()] = volume;
                    }
                }
                if !muted && applied_muted {
                    run(&["mute", "off"]).ok();
                }
                applied_muted = muted;
            }
        })
        .ok();
    tx
}

/// The script, waited for; its last line of complaint if it fails.
fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(SCRIPT).args(args).stdin(Stdio::null()).output().map_err(|_| "sound is not installed".to_owned())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default().trim().to_owned();
        Err(if why.is_empty() { format!("catalyst-audio {} failed", args.join(" ")) } else { why })
    }
}

// ------------------------------------------------------------------ where ---

/// What plays, as catalyst-audio routed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Out {
    /// The codec: the speaker, or headphones on the jack.
    #[default]
    Handheld,
    Bluetooth,
    Usb,
}

/// USB headphones plugged in: a USB sound card that plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usb {
    pub card: u32,
    /// What they call themselves ("USB-C to 3.5mm Headphone Jack Adapter").
    pub name: String,
    /// A volume control of their own (else catalyst-audio's software one).
    pub own_volume: bool,
}

/// Where sound goes, and the USB headphones there (playing or not).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    pub out: Out,
    pub usb: Option<Usb>,
}

impl Route {
    /// From catalyst-audio's `output` and `usb` files.
    pub fn parse(output: &str, usb: &str) -> Route {
        let field = |key: &str| usb.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim);
        let usb = field("card").and_then(|c| c.parse().ok()).map(|card| Usb {
            card,
            name: field("name").filter(|n| !n.is_empty()).unwrap_or("USB audio").to_owned(),
            own_volume: field("mixer").is_some_and(|m| !m.is_empty()),
        });
        let out = match output.split_whitespace().next() {
            Some("bluetooth") => Out::Bluetooth,
            Some("usb") if usb.is_some() => Out::Usb,
            _ => Out::Handheld,
        };
        Route { out, usb }
    }

    /// Whose volume plays.
    pub fn owner(&self) -> Owner {
        if self.out == Out::Usb {
            Owner::Usb
        } else {
            Owner::Main
        }
    }

    /// The card USB headphones play through, while they are what plays.
    pub fn usb_playing(&self) -> Option<u32> {
        if self.out == Out::Usb {
            self.usb.as_ref().map(|u| u.card)
        } else {
            None
        }
    }

    fn usb_card(&self) -> Option<u32> {
        self.usb.as_ref().map(|u| u.card)
    }
}

// The route as last read, and what the island is to say of it: one for the
// whole shell, or one a test (each test's shell is its own).
#[cfg(not(test))]
static ROUTE: Mutex<Option<Route>> = Mutex::new(None);
#[cfg(not(test))]
static SAID: Mutex<Option<String>> = Mutex::new(None);
#[cfg(test)]
thread_local! {
    static ROUTE: std::cell::RefCell<Option<Route>> = const { std::cell::RefCell::new(None) };
    static SAID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn with_route<T>(f: impl FnOnce(&mut Option<Route>) -> T) -> T {
    #[cfg(not(test))]
    return f(&mut ROUTE.lock().unwrap_or_else(|e| e.into_inner()));
    #[cfg(test)]
    ROUTE.with(|r| f(&mut r.borrow_mut()))
}

fn set_said(words: String) {
    #[cfg(not(test))]
    {
        *SAID.lock().unwrap_or_else(|e| e.into_inner()) = Some(words);
    }
    #[cfg(test)]
    SAID.with(|s| *s.borrow_mut() = Some(words));
}

/// What the island is to say of sound moving, since the shell last asked.
pub fn take_said() -> Option<String> {
    #[cfg(not(test))]
    return SAID.lock().unwrap_or_else(|e| e.into_inner()).take();
    #[cfg(test)]
    SAID.with(|s| s.borrow_mut().take())
}

/// Times sound has moved, for a page showing where it goes to look again.
static MOVES: AtomicU32 = AtomicU32::new(0);

pub fn moves() -> u32 {
    MOVES.load(Ordering::Relaxed)
}

/// Where sound goes now.
pub fn route() -> Route {
    with_route(|r| r.get_or_insert_with(read_route).clone())
}

fn read_route() -> Route {
    if !on_device() {
        return Route::default();
    }
    let read = |name: &str| std::fs::read_to_string(Path::new(RUN).join(name)).unwrap_or_default();
    Route::parse(&read("output"), &read("usb"))
}

/// What Music does as sound moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// On from where it is, through what plays now.
    Carry,
    /// Paused: the USB headphones it played through came out.
    Pause,
    /// As it is.
    Stay,
}

/// Headphones that take over carry the music on; USB headphones pulled out
/// pause it. Bluetooth headphones going away are left as before (the player
/// finds its sound gone, and says so).
pub fn follow(old: &Route, new: &Route) -> Follow {
    if let Some(played) = old.usb_playing() {
        if new.usb_card() != Some(played) {
            return Follow::Pause;
        }
    }
    if old.out == new.out && old.usb_playing() == new.usb_playing() {
        return Follow::Stay;
    }
    match (old.out, new.out) {
        (Out::Bluetooth, Out::Handheld) => Follow::Stay,
        _ => Follow::Carry,
    }
}

/// The island's words as sound moves (`music`: Music was playing, or has
/// just been paused by the headphones coming out).
pub const PLUGGED: &str = "playing on usb headphones";
pub const UNPLUGGED: &str = "usb headphones unplugged";
pub const UNPLUGGED_PAUSED: &str = "headphones unplugged · paused";

pub fn announce(old: &Route, new: &Route, music: bool) -> Option<&'static str> {
    if new.out == Out::Usb && old.usb_playing() != new.usb_playing() {
        return Some(PLUGGED);
    }
    if old.usb_card().is_some() && old.usb_card() != new.usb_card() {
        return Some(if music && old.out == Out::Usb { UNPLUGGED_PAUSED } else { UNPLUGGED });
    }
    None
}

/// Sound went somewhere else: Music follows, the island is to say so, and
/// the shell wakes to.
fn moved(new: Route) {
    let old = with_route(|r| std::mem::replace(r, Some(new.clone()))).unwrap_or_default();
    if old == new {
        return;
    }
    MOVES.fetch_add(1, Ordering::Relaxed);
    // (The tests' player is the music tests' own: it is left to them.)
    let music = !cfg!(test) && (crate::music::playing() | crate::music::paused_unplugged());
    if !cfg!(test) {
        crate::music::moved(follow(&old, &new));
    }
    if let Some(words) = announce(&old, &new, music) {
        set_said(words.into());
    }
    // AirPods come and go with Bluetooth: their protocol's thread looks.
    crate::airpods::look();
    crate::platform::wake();
}

/// The USB card is no longer there (pulled out): Music's player asks, as its
/// sound stops, whether that is why.
pub fn usb_gone(card: u32) -> bool {
    if on_device() {
        !Path::new(&format!("/proc/asound/card{card}")).exists()
    } else {
        route().usb_card() != Some(card)
    }
}

/// catalyst-audio's word of where sound goes, as it changes: a watch on its
/// folder, read when it puts a new `output` there (USB headphones come or
/// go, Bluetooth headphones connect). The thread sleeps in the read: nothing
/// is polled, and nothing of it runs on the frame.
#[cfg(target_os = "linux")]
fn watch() {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    std::fs::create_dir_all(RUN).ok();
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return;
    }
    let mut events = unsafe { std::fs::File::from_raw_fd(fd) };
    let dir = std::ffi::CString::new(RUN).unwrap();
    if unsafe { libc::inotify_add_watch(fd, dir.as_ptr(), libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE) } < 0 {
        return;
    }
    // Anything routed between the start's look and the watch.
    moved(read_route());
    let mut buf = [0u8; 1024];
    loop {
        match events.read(&mut buf) {
            Ok(n) if n > 0 => {
                if names_in(&buf[..n]).iter().any(|name| name == "output") {
                    moved(read_route());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            _ => return,
        }
    }
}

/// The file names in a run of inotify events (each a 16-byte header, the
/// last field the length of the name after it, padded with NULs).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn names_in(buf: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut at = 0;
    while at + 16 <= buf.len() {
        let len = u32::from_ne_bytes([buf[at + 12], buf[at + 13], buf[at + 14], buf[at + 15]]) as usize;
        let name = buf.get(at + 16..at + 16 + len).unwrap_or_default();
        names.push(String::from_utf8_lossy(name).trim_end_matches('\0').to_owned());
        at += 16 + len;
    }
    names
}

/// The simulator's USB headphones plugged in (Some) or pulled out, as the
/// handheld's hotplug would: sound follows, and the island says so.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_plug(usb: Option<Usb>) {
    let out = match (&usb, crate::bluetooth::demo_connected()) {
        (Some(_), _) => Out::Usb,
        (None, Some(_)) => Out::Bluetooth,
        (None, None) => Out::Handheld,
    };
    moved(Route { out, usb });
}

/// The simulator's USB headphones: an adapter with a volume of its own.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_usb() -> Usb {
    Usb { card: 1, name: "USB-C to 3.5mm Headphone Jack Adapter".into(), own_volume: true }
}

/// With USB and Bluetooth headphones both there: which plays.
fn prefer(usb: bool) -> Result<(), String> {
    if on_device() {
        return run(&["prefer", if usb { "usb" } else { "bluetooth" }]).map(|_| ());
    }
    let now = route();
    moved(Route { out: if usb { Out::Usb } else { Out::Bluetooth }, ..now });
    Ok(())
}

// ------------------------------------------------------------- USB's setup ---

/// How USB headphones stand: playing, there, or what they wait for. They
/// need the CM0's one USB port running the internal hub, which carries
/// USB-C1 and the USB-A port: Settings › USB on charging only (a computer's
/// modes make the port a device, from the next start) and the board's USB
/// switch on the hub's side. USB-C2 goes to the CM0 alone, for a computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setup {
    Playing,
    /// Plugged in, with Bluetooth headphones playing (connected after them).
    Connected,
    /// Ready for them: plugged into USB-C1 or USB-A, they play.
    #[default]
    Ready,
    /// The host runs but no hub answers: the USB switch is on USB-C2's side.
    NoHub,
    /// A mode for a computer is chosen in Settings › USB.
    DeviceMode,
    /// Back on charging only, the port is a device until the next start.
    Restart,
}

impl Setup {
    /// From what is there (USB headphones, playing or not), the USB mode
    /// chosen, whether the port runs as a device, and whether the hub answers.
    pub fn of(usb: bool, playing: bool, mode: usize, as_device: bool, hub: bool) -> Setup {
        match () {
            _ if usb && playing => Setup::Playing,
            _ if usb => Setup::Connected,
            _ if mode != 0 => Setup::DeviceMode,
            _ if as_device => Setup::Restart,
            _ if !hub => Setup::NoHub,
            _ => Setup::Ready,
        }
    }

    /// Its value on a row.
    pub fn value(self) -> &'static str {
        match self {
            Setup::Playing => "playing",
            Setup::Connected => "plugged in",
            Setup::Ready => "not plugged in",
            Setup::NoHub => "switch to hub",
            Setup::DeviceMode => "set up usb",
            Setup::Restart => "after a restart",
        }
    }

    /// Plugged in, playing or not.
    pub fn there(self) -> bool {
        matches!(self, Setup::Playing | Setup::Connected)
    }
}

/// The internal hub answers: a hub on the bus other than the controller's
/// own (`usb1`), as the GL852G is when the switch is on its side.
fn hub_present() -> bool {
    std::fs::read_dir("/sys/bus/usb/devices").into_iter().flatten().flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        !name.starts_with("usb") && !name.contains(':') && std::fs::read_to_string(e.path().join("bDeviceClass")).is_ok_and(|c| c.trim() == "09")
    })
}

/// The gallery's choice of how USB stands, in the simulator.
static DEMO_SETUP: Mutex<Option<Setup>> = Mutex::new(None);

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_setup(setup: Option<Setup>) {
    *DEMO_SETUP.lock().unwrap() = setup;
}

// ----------------------------------------------------------------- chimes ---

const SOUNDS: &str = "/usr/share/catalyst/sounds";

/// What plays as reminders come due (reminders.rs calls it): the chime, or
/// for the pit's a brighter, quicker one that cuts through a busy pit. At
/// the system volume, over whatever else plays; nothing while muted.
pub fn reminder_chime(rang: &[crate::reminders::Ring]) {
    let file = if rang.iter().any(|r| r.pit) { "pit.wav" } else { "chime.wav" };
    play_file(file);
}

/// A sound from the image's own, played to "default" on a thread of its
/// own (waited for there, so nothing is left behind).
fn play_file(name: &'static str) {
    if !on_device() || level().heard() == 0 {
        return;
    }
    std::thread::Builder::new()
        .name("chime".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let path = Path::new(SOUNDS).join(name);
            Command::new("aplay").args(["-q", "-D", "default"]).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok();
        })
        .ok();
}

/// At start: the reminders' chime, where sound goes (as catalyst-audio left
/// it: the shell may have restarted with headphones in), and its watch.
pub fn start() {
    crate::reminders::set_chime(reminder_chime);
    #[cfg(target_os = "linux")]
    if on_device() {
        with_route(|r| *r = Some(read_route()));
        std::thread::Builder::new().name("sound-watch".into()).stack_size(64 * 1024).spawn(watch).ok();
        crate::airpods::start();
    }
}

// ------------------------------------------------------------------- keys ---

/// A sound or music key, over any screen: what the island says.
pub fn media_key(key: Media) -> Option<String> {
    match key {
        Media::VolumeUp => Some(said(step(STEP as i32))),
        Media::VolumeDown => Some(said(step(-(STEP as i32)))),
        Media::Mute => Some(said(toggle_mute())),
        Media::PlayPause | Media::Next | Media::Previous => crate::music::media_key(key),
    }
}

/// A sound or music key as Show key presses names it.
pub fn key_name(key: Media) -> &'static str {
    match key {
        Media::Mute => "mute",
        Media::VolumeDown => "vol −",
        Media::VolumeUp => "vol +",
        Media::PlayPause => "play",
        Media::Previous => "prev",
        Media::Next => "next",
    }
}

// ----------------------------------------------------------------- status ---

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Output {
    #[default]
    Speaker,
    Headphones,
    Bluetooth,
    Usb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mic {
    /// The handheld's own: a MEMS microphone beside the keyboard.
    #[default]
    Builtin,
    /// A headset's, on the jack.
    Headset,
}

/// What catalyst-audio says of things (`catalyst-audio status`), and how
/// USB headphones stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub card: bool,
    pub output: Output,
    pub bluetooth: Option<String>,
    pub bluetooth_connected: bool,
    /// The codec and rate it plays with (AAC, SBC).
    pub bluetooth_codec: Option<String>,
    pub bluetooth_rate: Option<u32>,
    /// USB headphones plugged in (playing or not): what they call themselves.
    pub usb: Option<String>,
    pub usb_own_volume: bool,
    pub volume: u32,
    pub volume_usb: u32,
    pub muted: bool,
    pub mic: Mic,
    pub mic_gain: u32,
    pub playing: bool,
    pub setup: Setup,
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status { volume: DEFAULT_VOLUME, volume_usb: DEFAULT_USB_VOLUME, mic_gain: DEFAULT_MIC_GAIN, ..Status::default() };
    let mut usb_name = None;
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match name.trim() {
            "card" => s.card = value != "none" && !value.is_empty(),
            "output" => {
                s.output = match value {
                    "headphones" => Output::Headphones,
                    "bluetooth" => Output::Bluetooth,
                    "usb" => Output::Usb,
                    _ => Output::Speaker,
                }
            }
            "bluetooth" if !value.is_empty() => s.bluetooth = Some(value.to_owned()),
            "bluetooth_connected" => s.bluetooth_connected = value == "yes",
            "bluetooth_codec" if !value.is_empty() => s.bluetooth_codec = Some(value.to_owned()),
            "bluetooth_rate" => s.bluetooth_rate = value.parse().ok(),
            "usb" if !value.is_empty() => s.usb = Some(value.to_owned()),
            "usb_name" if !value.is_empty() => usb_name = Some(value.to_owned()),
            "usb_mixer" => s.usb_own_volume = value == "own",
            "volume" => s.volume = value.parse().unwrap_or(s.volume).min(100),
            "volume_usb" => s.volume_usb = value.parse().unwrap_or(s.volume_usb).min(100),
            "muted" => s.muted = value == "yes",
            "mic" => s.mic = if value == "headset" { Mic::Headset } else { Mic::Builtin },
            "mic_gain" => s.mic_gain = value.parse().unwrap_or(s.mic_gain),
            "playing" => s.playing = value == "yes",
            _ => {}
        }
    }
    // Named by what they call themselves, where they say.
    if let (Some(usb), Some(name)) = (&mut s.usb, usb_name) {
        *usb = name;
    }
    s
}

/// How things stand now: off the drawing thread.
fn read_status() -> Result<Status, String> {
    let mut s = if on_device() {
        run(&["status"]).map(|text| parse_status(&text))?
    } else {
        let r = route();
        let mic = *DEMO_MIC.lock().unwrap();
        let bluetooth = crate::bluetooth::demo_connected();
        let output = match r.out {
            Out::Usb => Output::Usb,
            _ if bluetooth.is_some() => Output::Bluetooth,
            _ => Output::Speaker,
        };
        let (main, usb_level) = (level_for(Owner::Main), level_for(Owner::Usb));
        Status {
            card: true,
            output,
            bluetooth_connected: bluetooth.is_some(),
            bluetooth_codec: bluetooth.as_ref().map(|_| "AAC".to_owned()),
            bluetooth_rate: bluetooth.as_ref().map(|_| 48_000),
            bluetooth,
            usb: r.usb.as_ref().map(|u| u.name.clone()),
            usb_own_volume: r.usb.as_ref().is_some_and(|u| u.own_volume),
            volume: main.volume,
            volume_usb: usb_level.volume,
            muted: main.muted,
            mic: mic.0,
            mic_gain: mic.1,
            playing: false,
            setup: Setup::Ready,
        }
    };
    s.setup = match *DEMO_SETUP.lock().unwrap() {
        Some(setup) if !on_device() && s.usb.is_none() => setup,
        _ => Setup::of(s.usb.is_some(), s.output == Output::Usb, crate::usb::mode(), crate::usb::running_as_device(), !on_device() || hub_present()),
    };
    Ok(s)
}

static DEMO_MIC: Mutex<(Mic, u32)> = Mutex::new((Mic::Builtin, DEFAULT_MIC_GAIN));

fn set_mic(mic: Mic) -> Result<(), String> {
    if !on_device() {
        DEMO_MIC.lock().unwrap().0 = mic;
        return Ok(());
    }
    run(&["mic", if mic == Mic::Headset { "headset" } else { "builtin" }]).map(|_| ())
}

fn set_mic_gain(gain: u32) -> Result<(), String> {
    if !on_device() {
        DEMO_MIC.lock().unwrap().1 = gain;
        return Ok(());
    }
    run(&["mic-gain", &gain.to_string()]).map(|_| ())
}

fn test_tone() -> Result<(), String> {
    if !on_device() {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        return Ok(());
    }
    run(&["test-tone"]).map(|_| ())
}

/// The amplifier setting's words, and the next one up or down.
fn mic_gain_words(gain: u32) -> &'static str {
    MIC_GAINS.iter().find(|(g, _)| *g == gain).map_or("–", |(_, w)| w)
}

fn mic_gain_step(gain: u32, step: i32) -> u32 {
    let at = MIC_GAINS.iter().position(|(g, _)| *g >= gain).unwrap_or(MIC_GAINS.len() - 1) as i32;
    MIC_GAINS[(at + step).clamp(0, MIC_GAINS.len() as i32 - 1) as usize].0
}

// ------------------------------------------------------------- microphone ---

/// The microphone's level, live: `arecord` on ALSA's "default" at 16 kHz
/// mono (what speech programs ask for), its loudest sample each 50 ms kept.
/// Recording stops when this is dropped.
pub struct MicMeter {
    child: Option<Child>,
    /// The latest peak, as a fraction of full scale (0 to 1), in thousandths.
    peak: Arc<AtomicU32>,
    alive: Arc<AtomicBool>,
    started: Instant,
}

impl MicMeter {
    pub fn start() -> Self {
        let peak = Arc::new(AtomicU32::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let mut child = None;
        if on_device() {
            let spawned = Command::new("arecord")
                .args(["-q", "-D", "default", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut c) = spawned {
                if let Some(mut out) = c.stdout.take() {
                    let (peak, alive) = (peak.clone(), alive.clone());
                    std::thread::Builder::new()
                        .name("mic".into())
                        .stack_size(64 * 1024)
                        .spawn(move || {
                            use std::io::Read;
                            // 50 ms at 16 kHz, 16-bit.
                            let mut buf = [0u8; 1600];
                            while out.read_exact(&mut buf).is_ok() {
                                peak.store((peak_of(&buf) * 1000.0) as u32, Ordering::Relaxed);
                            }
                            alive.store(false, Ordering::Relaxed);
                        })
                        .ok();
                }
                child = Some(c);
            } else {
                alive.store(false, Ordering::Relaxed);
            }
        }
        MicMeter { child, peak, alive, started: Instant::now() }
    }

    /// The level to show, 0 to 1: the peak in decibels, from -60 dBFS (0) to
    /// full scale (1), so speech at arm's length reads clearly.
    pub fn level(&self) -> f32 {
        let peak = if on_device() {
            self.peak.load(Ordering::Relaxed) as f32 / 1000.0
        } else {
            // Someone talking: syllables over a quiet room.
            let t = self.started.elapsed().as_secs_f32();
            let syllable = ((t * 5.3).sin() * (t * 1.7).sin()).abs();
            0.004 + 0.3 * syllable * syllable
        };
        level_of(peak)
    }

    pub fn working(&self) -> bool {
        !on_device() || self.alive.load(Ordering::Relaxed)
    }
}

impl Drop for MicMeter {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            c.kill().ok();
            c.wait().ok();
        }
    }
}

/// The loudest sample in a run of 16-bit little-endian samples, 0 to 1.
fn peak_of(bytes: &[u8]) -> f32 {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0
}

/// A peak (0 to 1) on the meter's scale: -60 dBFS and below at 0, full scale at 1.
fn level_of(peak: f32) -> f32 {
    if peak <= 0.001 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

// ------------------------------------------------------- music's sound ---

/// Music's sound settings (spatial sound, the equalizer, loudness
/// matching, the crossfade), as the player reads them from /data
/// (audiofx::settings). Kept here once read; changed here at once.
static FX: Mutex<Option<audiofx::settings::Fx>> = Mutex::new(None);

pub fn fx() -> audiofx::settings::Fx {
    FX.lock().unwrap().get_or_insert_with(|| if on_device() { audiofx::settings::load(Path::new(audiofx::settings::PATH)) } else { Default::default() }).clone()
}

/// A setting changed: written for the player (which reads it within half
/// a second, while a song plays), and the AirPods' head tracking looked at.
pub fn set_fx(change: impl FnOnce(&mut audiofx::settings::Fx)) -> audiofx::settings::Fx {
    let mut fx = fx();
    change(&mut fx);
    *FX.lock().unwrap() = Some(fx.clone());
    if on_device() {
        audiofx::settings::save(Path::new(audiofx::settings::PATH), &fx).ok();
    }
    crate::airpods::look();
    fx
}

/// What the player is playing, and how (/run/catalyst-music/stream).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    pub codec: String,
    pub rate: u32,
    pub bits: u32,
    pub out_rate: u32,
    pub bit_perfect: bool,
    pub processing: Vec<String>,
    pub tracking: bool,
}

pub fn parse_stream(text: &str) -> Stream {
    let mut s = Stream::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "codec" => s.codec = v.to_owned(),
            "rate" => s.rate = v.parse().unwrap_or(0),
            "bits" => s.bits = v.parse().unwrap_or(0),
            "out_rate" => s.out_rate = v.parse().unwrap_or(0),
            "bitperfect" => s.bit_perfect = v == "yes",
            "processing" if v != "none" => s.processing = v.split(',').map(str::to_owned).collect(),
            "tracking" => s.tracking = v == "yes",
            _ => {}
        }
    }
    s
}

static DEMO_STREAM: Mutex<Option<Stream>> = Mutex::new(None);

/// The simulator's song, as the player would say it (the galleries).
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_stream(s: Option<Stream>) {
    *DEMO_STREAM.lock().unwrap() = s;
}

/// What plays now, while Music plays.
pub fn stream() -> Option<Stream> {
    if !on_device() {
        return DEMO_STREAM.lock().unwrap().clone();
    }
    if !crate::music::playing() {
        return None;
    }
    std::fs::read_to_string("/run/catalyst-music/stream").ok().map(|t| parse_stream(&t)).filter(|s| s.rate > 0)
}

/// "44.1 kHz", "48 kHz", "96 kHz".
fn khz(rate: u32) -> String {
    let k = rate as f32 / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{} kHz", k.round() as u32)
    } else {
        format!("{k:.1} kHz")
    }
}

/// How a song plays, the longest first: "44.1 kHz · 16-bit · bit-perfect".
pub fn quality_words(s: &Stream) -> Vec<String> {
    let how = if s.bit_perfect {
        "bit-perfect".to_owned()
    } else if s.processing.iter().any(|p| p == "spatial") {
        if s.tracking { "spatial, head tracked" } else { "spatial" }.to_owned()
    } else if s.processing.iter().any(|p| p == "eq") {
        "equalized".into()
    } else if s.processing.iter().any(|p| p == "loudness") {
        "level matched".into()
    } else if s.processing.iter().any(|p| p == "resampled") {
        format!("to {}", khz(s.out_rate))
    } else {
        "mixed".into()
    };
    let short = how.split(',').next().unwrap_or(&how).to_owned();
    vec![format!("{} · {}-bit · {how}", khz(s.rate), s.bits), format!("{} · {short}", khz(s.rate)), short]
}

// ----------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Volume,
    Mute,
    Output,
    Usb,
    /// How the song playing plays: its rate and depth, bit-perfect or not.
    Quality,
    Spatial,
    /// The AirPods' noise control, while they are connected.
    Noise,
    Equalizer,
    Loudness,
    Crossfade,
    Test,
    Microphone,
    MicGain,
    Bluetooth,
}

/// Every row the page can have, in order; some only now and then (rows()).
const ALL: [Row; 14] = [Row::Volume, Row::Mute, Row::Output, Row::Usb, Row::Quality, Row::Spatial, Row::Noise, Row::Equalizer, Row::Loudness, Row::Crossfade, Row::Test, Row::Microphone, Row::MicGain, Row::Bluetooth];

/// Search's places on the page (screens.rs): rows of `ALL`.
pub const USB_ROW: usize = 3;
pub const SPATIAL_ROW: usize = 5;
pub const EQ_ROW: usize = 7;
pub const LOUDNESS_ROW: usize = 8;
pub const CROSSFADE_ROW: usize = 9;
pub const TEST_ROW: usize = 10;
pub const MIC_ROW: usize = 11;

/// How long the microphone's level is shown after the last key, in seconds.
const LISTEN_FOR: f32 = 30.0;

/// Settings › Sound: the volume, silence, where sound goes, USB headphones,
/// how the song playing plays, Music's sound (spatial sound, the AirPods'
/// noise control, the equalizer, loudness matching, the crossfade), a test
/// sound, the microphone (with its level, live, while it is chosen) and its
/// gain, and Bluetooth.
pub struct SoundScreen {
    list: ListState,
    status: Option<Status>,
    reading: Option<Job<Result<Status, String>>>,
    read_at: Instant,
    /// Sound's moves seen: another, and it is read again at once.
    moves: u32,
    /// The volume bar, gliding to the level.
    bar: detent::Animated,
    /// The microphone's meter, while its row is chosen.
    meter: Option<MicMeter>,
    meter_bar: detent::Animated,
    /// The last key on the page: the microphone stops listening a while
    /// after it, so a page left open as the screen goes off (or locks, or
    /// sleeps deep) does not record on.
    last_key: Instant,
    working: Option<(Row, Job<Result<(), String>>)>,
    /// The song playing, and the AirPods there, as last looked at.
    stream: Option<Stream>,
    pods: Option<crate::airpods::State>,
    looked: Instant,
    changed: bool,
}

impl SoundScreen {
    pub fn new() -> Self {
        let mut page = SoundScreen {
            list: ListState::default(),
            status: None,
            reading: None,
            read_at: Instant::now(),
            moves: moves(),
            bar: detent::Animated::new(level().volume as f32 / 100.0, detent::role::hold()).with_rest(0.003),
            meter: None,
            meter_bar: detent::Animated::new(0.0, detent::role::tick()).with_rest(0.003),
            working: None,
            last_key: Instant::now(),
            stream: stream(),
            pods: crate::airpods::now(),
            looked: Instant::now(),
            changed: true,
        };
        page.read_now();
        page
    }

    fn read_now(&mut self) {
        self.read_at = Instant::now();
        self.moves = moves();
        self.reading = Some(Job::start(read_status));
    }

    /// The rows there are now: how the song plays only while one does, the
    /// noise control only while AirPods that have it are connected.
    fn rows(&self) -> Vec<Row> {
        let noise = self.pods.as_ref().is_some_and(|p| p.noise.is_some());
        ALL.into_iter()
            .filter(|r| match r {
                Row::Quality => self.stream.is_some(),
                Row::Noise => noise,
                _ => true,
            })
            .collect()
    }

    fn row(&self) -> Row {
        let rows = self.rows();
        rows[self.list.focus.min(rows.len() - 1)]
    }

    fn start(&mut self, row: Row, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        if self.working.is_none() {
            self.working = Some((row, Job::start(work)));
        }
    }

    /// The Bluetooth headphones there, by name, if they are connected.
    fn bluetooth_there(&self) -> Option<String> {
        let st = self.status.as_ref()?;
        st.bluetooth.as_deref().filter(|_| st.bluetooth_connected).map(crate::bluetooth::name_of)
    }

    /// Spatial sound turned by the head now.
    fn tracking(&self) -> bool {
        self.stream.as_ref().is_some_and(|s| s.tracking) || (!on_device() && fx().spatial && self.pods.as_ref().is_some_and(|p| p.head == crate::airpods::Head::Tracking))
    }

    fn describe(&self, row: Row) -> (&'static str, String, bool) {
        let st = self.status.as_ref();
        let l = level();
        let fx = fx();
        let on_off = |b: bool| -> String { (if b { "on" } else { "off" }).into() };
        match row {
            Row::Volume => ("Volume", if l.muted { "off".into() } else { format!("{}%", l.volume) }, !l.muted),
            Row::Mute => ("Sound", (if l.muted { "muted" } else { "on" }).into(), !l.muted),
            Row::Output => {
                let value = match st {
                    None => "…".into(),
                    Some(s) if !s.card => "no sound card".into(),
                    Some(s) => match s.output {
                        Output::Speaker => "speaker".into(),
                        Output::Headphones => "headphones".into(),
                        Output::Usb => "usb headphones".into(),
                        Output::Bluetooth => crate::bluetooth::name_of(s.bluetooth.as_deref().unwrap_or_default()),
                    },
                };
                ("Playing on", value, st.is_some_and(|s| s.card))
            }
            Row::Usb => {
                let setup = st.map(|s| s.setup);
                let value = setup.map_or("…", Setup::value);
                ("USB headphones", value.into(), setup.is_some_and(Setup::there))
            }
            Row::Quality => ("Quality", self.stream.as_ref().map(|s| quality_words(s).remove(0)).unwrap_or_default(), self.stream.as_ref().is_some_and(|s| s.bit_perfect)),
            Row::Spatial => ("Spatial sound", if !fx.spatial { "off".into() } else if self.tracking() { "head tracked".into() } else { "on".into() }, fx.spatial),
            Row::Noise => ("Noise control", self.pods.as_ref().and_then(|p| p.noise).map_or("", |m| m.word()).into(), self.pods.as_ref().is_some_and(|p| p.noise.is_some_and(|m| m != aap::NoiseMode::Off))),
            Row::Equalizer => ("Equalizer", if fx.eq == audiofx::eq::Preset::Flat { "off".into() } else { fx.eq.word().into() }, fx.eq != audiofx::eq::Preset::Flat),
            Row::Loudness => ("Loudness match", on_off(fx.loudness), fx.loudness),
            Row::Crossfade => ("Crossfade", if fx.crossfade == 0 { "off".into() } else { format!("{} s", fx.crossfade) }, fx.crossfade > 0),
            Row::Test => ("Test sound", (if matches!(self.working, Some((Row::Test, _))) { "playing…" } else { "" }).into(), true),
            Row::Microphone => ("Microphone", (if st.is_some_and(|s| s.mic == Mic::Headset) { "headset's" } else { "handheld's" }).into(), true),
            Row::MicGain => ("Mic gain", mic_gain_words(st.map_or(DEFAULT_MIC_GAIN, |s| s.mic_gain)).into(), true),
            Row::Bluetooth => ("Bluetooth", crate::bluetooth::summary(), crate::bluetooth::enabled()),
        }
    }

    /// A row's value as it fits beside its name in `room`: the quality's
    /// longest wording that does; the rest as they are.
    fn fitted(&self, g: &mut Gfx, row: Row, room: f32) -> String {
        let (name, value, _) = self.describe(row);
        if row != Row::Quality {
            return value;
        }
        let bezel = g.theme == crate::look::Theme::Bezel;
        let (name_style, value_style) = if bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
        let name = if bezel { name.to_lowercase() } else { name.to_owned() };
        let free = room - g.measure(&name, name_style) - 30.0;
        let words = self.stream.as_ref().map(quality_words).unwrap_or_default();
        words.iter().find(|w| g.measure(w, value_style) <= free).or(words.last()).cloned().unwrap_or(value)
    }

    /// What the keys do on a row: the longest that fits whole first.
    fn hint(&self, row: Row) -> Vec<String> {
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let usb_plays = self.status.as_ref().is_some_and(|s| s.output == Output::Usb);
        let fx = fx();
        match row {
            Row::Volume if usb_plays => words(&["←→: the headphones' own · fn + s, fn + d", "←→: the headphones' volume"]),
            Row::Volume => words(&["←→: volume · fn + s, fn + d anywhere", "←→: volume"]),
            Row::Mute => words(&["enter: mute or unmute · fn + a anywhere", "enter: mute or unmute"]),
            Row::Output => match self.status.as_ref().map(|s| s.output) {
                Some(Output::Headphones) => words(&["the speaker is off while headphones are in", "speaker off: headphones in"]),
                Some(Output::Usb) => words(&["usb headphones · the speaker stays off", "usb headphones"]),
                Some(Output::Bluetooth) => match self.status.as_ref().and_then(|s| s.bluetooth_codec.as_deref()) {
                    Some(codec) => vec![format!("bluetooth, {} · enter: more", codec.to_lowercase()), format!("bluetooth, {}", codec.to_lowercase())],
                    None => words(&["bluetooth headphones, while connected", "bluetooth, while connected"]),
                },
                _ => words(&["headphones take over when plugged in", "headphones take over"]),
            },
            Row::Usb => match (self.status.as_ref().map(|s| s.setup), self.bluetooth_there()) {
                (Some(Setup::Playing), Some(bt)) => vec![format!("enter: play on {bt} instead"), "enter: play on bluetooth".into()],
                (Some(Setup::Playing), None) => words(&["their own volume: fn + s, fn + d · enter: more", "their own volume: fn + s, fn + d", "their own volume"]),
                (Some(Setup::Connected), _) => words(&["bluetooth came after them · enter: play here", "enter: play on these instead", "enter: play here"]),
                (Some(Setup::NoHub), _) => words(&["usb switch to the hub's side · enter: more", "usb switch to the hub's side"]),
                (Some(Setup::DeviceMode), _) => words(&["usb › charging only, switch to hub, restart", "enter: how to set them up"]),
                (Some(Setup::Restart), _) => words(&["restart, the usb switch on the hub's side", "restart for usb headphones"]),
                _ => words(&["into usb-c1 or usb-a · enter: more", "into usb-c1 or usb-a"]),
            },
            Row::Quality => match &self.stream {
                Some(s) if s.bit_perfect => words(&["the song as it was made, to the headphones", "the song as it was made"]),
                Some(_) if usb_plays => words(&["spatial, equalizer, loudness off: bit-perfect", "effects off: bit-perfect"]),
                _ => words(&["bit-perfect through usb headphones", "bit-perfect over usb"]),
            },
            Row::Spatial if !fx.spatial => words(&["enter: speakers around you · headphones only", "enter: speakers around you", "enter: on"]),
            Row::Spatial if self.tracking() => {
                let yaw = self.pods.as_ref().map_or(0.0, |p| p.pose.0).round() as i32;
                let side = match yaw {
                    0 => "ahead".to_owned(),
                    y if y > 0 => format!("{y}° right"),
                    y => format!("{}° left", -y),
                };
                vec![format!("head tracked · {side} · enter: off"), format!("head {side} · enter: off"), "enter: off".into()]
            }
            Row::Spatial if self.pods.as_ref().is_some_and(|p| p.head == crate::airpods::Head::Unsupported) => words(&["these airpods do not track the head", "no head tracking here"]),
            Row::Spatial => words(&["virtual speakers, for headphones · enter: off", "virtual speakers · enter: off", "enter: off"]),
            Row::Noise => match self.pods.as_ref().map(|p| p.noise_modes().len()) {
                Some(4) => words(&["←→: off, cancelling, transparency, adaptive", "←→: noise control"]),
                _ => words(&["←→: off, noise cancelling or transparency", "←→: noise control"]),
            },
            Row::Equalizer => words(&["enter: presets, and five bands by hand", "enter: presets and bands"]),
            Row::Loudness => words(&["enter: every song as loud as the last", "enter: on or off"]),
            Row::Crossfade => words(&["←→: up to 8 s · never within an album", "←→: up to 8 s"]),
            Row::Test => words(&["enter: a short chime, at this volume", "enter: a short chime"]),
            Row::Microphone if self.meter.as_ref().is_some_and(|m| !m.working()) => words(&["the microphone could not be opened", "no microphone"]),
            Row::Microphone => words(&["←→: the handheld's or a headset's", "←→: which microphone"]),
            Row::MicGain => words(&["←→: how much the mic is amplified", "←→: mic gain"]),
            Row::Bluetooth => words(&["enter: headphones and speakers", "enter: bluetooth"]),
        }
    }
}

impl Screen for SoundScreen {
    fn title(&self) -> &str {
        "Sound"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        self.last_key = Instant::now();
        let row = self.row();
        let left_right = matches!(key, Key::Left | Key::Right);
        let step = if key == Key::Left { -1 } else { 1 };
        let setup = self.status.as_ref().map(|s| s.setup);
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right if row == Row::Volume => {
                let l = step_volume(step * STEP as i32);
                self.bar.set(l.volume as f32 / 100.0);
                *cx.say = Some(said(l));
            }
            Key::Enter | Key::Char(' ') if row == Row::Mute => {
                *cx.say = Some(said(toggle_mute()));
            }
            Key::Enter if row == Row::Test => {
                if self.working.is_none() {
                    self.start(Row::Test, test_tone);
                }
            }
            Key::Enter | Key::Left | Key::Right if row == Row::Spatial => {
                let on = set_fx(|f| f.spatial = !f.spatial).spatial;
                *cx.say = Some((if on { "spatial sound on" } else { "spatial sound off" }).into());
            }
            Key::Enter | Key::Left | Key::Right if row == Row::Noise => {
                if let Some(p) = &self.pods {
                    let modes = p.noise_modes();
                    let at = p.noise.and_then(|m| modes.iter().position(|x| *x == m)).unwrap_or(0) as i32;
                    let to = modes[(at + step).rem_euclid(modes.len() as i32) as usize];
                    crate::airpods::set_noise(to);
                    self.pods = crate::airpods::now();
                    *cx.say = Some(to.word().into());
                }
            }
            Key::Enter | Key::Right if row == Row::Equalizer => return Nav::Push(Box::new(crate::equalizer::EqualizerPage::new())),
            Key::Enter | Key::Left | Key::Right if row == Row::Loudness => {
                let on = set_fx(|f| f.loudness = !f.loudness).loudness;
                *cx.say = Some((if on { "loudness matched" } else { "loudness as mastered" }).into());
            }
            Key::Left | Key::Right if row == Row::Crossfade => {
                let s = set_fx(|f| f.crossfade = (f.crossfade as i32 + step).clamp(0, audiofx::crossfade::MOST as i32) as u8).crossfade;
                *cx.say = Some(if s == 0 { "gapless, no crossfade".into() } else { format!("crossfade {s} s") });
            }
            Key::Enter | Key::Left | Key::Right if row == Row::Microphone => {
                let mic = if self.status.as_ref().is_some_and(|s| s.mic == Mic::Headset) { Mic::Builtin } else { Mic::Headset };
                if let Some(s) = &mut self.status {
                    s.mic = mic;
                }
                // The meter starts again on the microphone now chosen.
                self.meter = None;
                self.start(Row::Microphone, move || set_mic(mic));
            }
            Key::Left | Key::Right if row == Row::MicGain => {
                let now = self.status.as_ref().map_or(DEFAULT_MIC_GAIN, |s| s.mic_gain);
                let gain = mic_gain_step(now, step);
                if let Some(s) = &mut self.status {
                    s.mic_gain = gain;
                }
                self.start(Row::MicGain, move || set_mic_gain(gain));
            }
            // Both there: Enter plays on the other.
            Key::Enter if row == Row::Usb && setup == Some(Setup::Playing) && self.bluetooth_there().is_some() => {
                if let Some(bt) = self.bluetooth_there() {
                    *cx.say = Some(format!("playing on {bt}"));
                }
                self.start(Row::Usb, || prefer(false));
            }
            Key::Enter if row == Row::Usb && setup == Some(Setup::Connected) => self.start(Row::Usb, || prefer(true)),
            Key::Enter | Key::Right if row == Row::Usb => return Nav::Push(Box::new(UsbAudioPage::new())),
            Key::Enter if row == Row::Output && setup == Some(Setup::Playing) => return Nav::Push(Box::new(UsbAudioPage::new())),
            Key::Enter | Key::Right if row == Row::Bluetooth => return Nav::Push(Box::new(crate::bluetooth::BluetoothScreen::new())),
            Key::Enter if row == Row::Output && self.status.as_ref().is_some_and(|s| s.output == Output::Bluetooth) => {
                return Nav::Push(Box::new(crate::bluetooth::BluetoothScreen::new()))
            }
            Key::Char('r') => self.read_now(),
            other if !left_right => {
                let n = self.rows().len();
                crate::screens::list_key(&mut self.list, other, n)
            }
            _ => {}
        }
        self.changed = true;
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.working.as_ref().and_then(|(_, job)| job.get()) {
            if let Err(why) = done {
                *cx.say = Some(why);
            }
            self.working = None;
            self.read_now();
            self.changed = true;
        }
        if let Some(done) = self.reading.as_ref().and_then(Job::get) {
            self.reading = None;
            match done {
                Ok(s) => self.status = Some(s),
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        Nav::Stay
    }

    fn resume(&mut self) {
        self.read_now();
        self.changed = true;
    }

    fn tick(&mut self, dt: f32) -> bool {
        // Read again every two seconds, and at once as sound moves.
        let moved = self.moves != moves();
        if (moved || self.read_at.elapsed().as_secs_f32() >= 2.0) && self.reading.is_none() && self.working.is_none() {
            self.read_now();
        }
        // The song and the AirPods, twice a second: rows come and go with
        // them, the focus kept on the row it was on.
        if self.looked.elapsed().as_secs_f32() >= 0.5 {
            self.looked = Instant::now();
            let (stream, pods) = (stream(), crate::airpods::now());
            if stream != self.stream || pods != self.pods {
                let row = self.row();
                self.stream = stream;
                self.pods = pods;
                let rows = self.rows();
                self.list.focus = rows.iter().position(|r| *r == row).unwrap_or(self.list.focus.min(rows.len() - 1));
                self.changed = true;
            }
        }
        // The microphone is listened to only while its row is chosen.
        let listening = self.row() == Row::Microphone && self.working.is_none() && self.last_key.elapsed().as_secs_f32() < LISTEN_FOR;
        if listening && self.meter.is_none() {
            self.meter = Some(MicMeter::start());
        } else if !listening && self.meter.is_some() {
            self.meter = None;
            self.meter_bar.set(0.0);
        }
        if let Some(m) = &self.meter {
            self.meter_bar.set(m.level());
        }
        let target = level().volume as f32 / 100.0;
        if self.bar.target() != target {
            self.bar.set(target);
        }
        let moving = self.bar.tick(dt) | self.meter_bar.tick(dt);
        std::mem::take(&mut self.changed) | moving | self.meter.is_some()
    }

    fn busy(&self) -> bool {
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        let rows = self.rows();
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let row = rows[i];
            let r = Rect::new(PAD, y, width, row_h);
            let (name, _, lit) = self.describe(row);
            let value = self.fitted(g, row, r.w);
            let focused = i == self.list.focus;
            g.row(r, name, &value, if lit { t.ink } else { t.dim }, focused);
            // The volume and the microphone carry a bar between name and value.
            let bar = match row {
                Row::Volume => Some((self.bar.value(), if level().muted { t.faint } else { t.ice }, "100%")),
                Row::Microphone if self.meter.is_some() => Some((self.meter_bar.value(), t.ice, "handheld's")),
                _ => None,
            };
            if let Some((fraction, fill, widest)) = bar {
                let name_w = g.measure(&name.to_lowercase(), g.ty.label).max(g.measure(name, g.ty.body)) + 22.0;
                let value_w = g.measure(widest, g.ty.body).max(g.measure(&value, g.ty.body)) + 22.0;
                let room = r.w - name_w - value_w;
                if room > 24.0 {
                    g.meter(Rect::new(r.x + name_w, r.cy() - 3.0, room, 6.0), fraction, fill);
                }
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let hint = self.hint(self.row());
        let choices: Vec<&str> = hint.iter().map(String::as_str).collect();
        g.hint_any(&choices);
    }
}

/// Settings' one word for it: the volume, or that it is muted.
pub fn summary() -> String {
    let l = level();
    if l.muted {
        "muted".into()
    } else {
        format!("{}%", l.volume)
    }
}

impl SoundScreen {
    /// Opened at one of its rows (from search): a row of `ALL`, wherever it
    /// sits among the rows there are now.
    pub fn at(mut self, place: usize) -> Self {
        let want = ALL[place.min(ALL.len() - 1)];
        self.list.focus = self.rows().iter().position(|r| *r == want).unwrap_or(0);
        self
    }
}

/// The volume moved from a page or the control centre (as the keys move it).
pub fn step_volume(delta: i32) -> Level {
    step(delta)
}

// --------------------------------------------------------- USB audio page ---

/// Settings › Sound › USB headphones: where they plug in and what they do,
/// or what they wait for (the USB mode, the switch, a restart), with how
/// they stand now in the row at the foot.
pub struct UsbAudioPage {
    status: Option<Status>,
    reading: Option<Job<Result<Status, String>>>,
    read_at: Instant,
    moves: u32,
    changed: bool,
}

impl UsbAudioPage {
    pub fn new() -> Self {
        let mut page = UsbAudioPage { status: None, reading: None, read_at: Instant::now(), moves: moves(), changed: true };
        page.read_now();
        page
    }

    fn read_now(&mut self) {
        self.read_at = Instant::now();
        self.moves = moves();
        self.reading = Some(Job::start(read_status));
    }
}

/// The page's words for how USB headphones stand (`name`: what the ones
/// plugged in call themselves).
pub fn usb_words(setup: Setup, name: &str) -> String {
    match setup {
        Setup::Playing => format!("{name}. Sound plays here at their own volume (fn\u{a0}+\u{a0}s, fn\u{a0}+\u{a0}d); pulled out, it goes back to the speaker and music pauses."),
        Setup::Connected => format!("{name}, plugged in. Bluetooth headphones connected after them play: the ones connected last do."),
        Setup::Ready => "USB-C headphones, or a USB headphone adapter, plug into USB-C1 or the USB-A port, not USB-C2 (it charges). Sound goes to them at their own volume.".into(),
        Setup::NoHub => "The USB switch is on USB-C2's side, a computer's. Slide it to the hub's side, then plug the headphones into USB-C1 or USB-A.".into(),
        Setup::DeviceMode => "A computer's USB mode has the port. For headphones: Settings › USB on charging only, the USB switch to the hub's side, and restart.".into(),
        Setup::Restart => "USB is on charging only now. Restart with the USB switch on the hub's side, and headphones in USB-C1 or USB-A play.".into(),
    }
}

impl Screen for UsbAudioPage {
    fn title(&self) -> &str {
        // Short enough to sit whole beside the island at the largest text.
        "USB"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Char('r') => {
                self.read_now();
                Nav::Stay
            }
            // The next step, asked for: a computer's mode has the port, so
            // charging only (the controller hosts from the next start); set,
            // a restart. Nothing changes unless Enter is pressed here.
            Key::Enter => {
                match self.status.as_ref().map(|s| s.setup) {
                    Some(Setup::DeviceMode) => {
                        *cx.say = Some(match crate::usb::set(0) {
                            Ok(()) => "usb on charging only · restart with the switch on the hub's side".into(),
                            Err(why) => why,
                        });
                        self.read_now();
                    }
                    Some(Setup::Restart) if on_device() => {
                        std::process::Command::new("reboot").spawn().ok();
                    }
                    _ => {}
                }
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.reading.as_ref().and_then(Job::get) {
            self.reading = None;
            match done {
                Ok(s) => {
                    self.changed |= self.status.as_ref() != Some(&s);
                    self.status = Some(s);
                }
                Err(why) => *cx.say = Some(why),
            }
        }
        Nav::Stay
    }

    fn resume(&mut self) {
        self.read_now();
        self.changed = true;
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // Read again every two seconds, and at once as sound moves.
        if (self.moves != moves() || self.read_at.elapsed().as_secs_f32() >= 2.0) && self.reading.is_none() {
            self.read_now();
        }
        std::mem::take(&mut self.changed)
    }

    fn busy(&self) -> bool {
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let setup = self.status.as_ref().map_or(Setup::Ready, |s| s.setup);
        let name = self.status.as_ref().and_then(|s| s.usb.clone()).unwrap_or_default();
        let line_h = ty.caption.size + 4.0;
        let lines = crate::help::wrap(g, &usb_words(setup, &name), ty.caption, W - 2.0 * PAD);
        for (i, line) in lines.iter().enumerate() {
            g.text_alpha(PAD, top + 2.0 + line_h * (i as f32 + 0.5), line, ty.caption, t.dim, 1.0);
        }
        let row_h = g.row_h();
        let r = Rect::new(PAD, crate::gfx::H - PAD - g.hint_h() - row_h, W - 2.0 * PAD, row_h);
        let lit = setup.there();
        crate::bluetooth::icon_row(g, r, "headphones", lit, "USB headphones", setup.value(), if lit { t.ink } else { t.dim }, true);
        match setup {
            Setup::DeviceMode => g.hint_any(&["enter: usb on charging only · esc back", "enter: charging only", "enter: set up"]),
            Setup::Restart => g.hint_any(&["enter: restart now · esc back", "enter: restart now"]),
            _ => g.hint("esc back"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_read_line_by_line() {
        let s = parse_status("card=ES8389Audio\noutput=bluetooth\nbluetooth=0C:A6:94:12:34:56\nvolume=40\nmuted=yes\nmic=headset\nmic_gain=10\nplaying=yes\n");
        assert!(s.card && s.muted && s.playing);
        assert_eq!((s.output, s.bluetooth.as_deref(), s.volume, s.mic, s.mic_gain), (Output::Bluetooth, Some("0C:A6:94:12:34:56"), 40, Mic::Headset, 10));
        // No card, and anything it does not say, as it starts.
        let s = parse_status("card=none\nsomething=else\n");
        assert!(!s.card);
        assert_eq!((s.output, s.volume, s.volume_usb, s.mic, s.mic_gain, s.usb), (Output::Speaker, DEFAULT_VOLUME, DEFAULT_USB_VOLUME, Mic::Builtin, DEFAULT_MIC_GAIN, None));
        assert_eq!(parse_status("output=headphones").output, Output::Headphones);
        // USB headphones, by what they call themselves, and Bluetooth there too.
        let s = parse_status("output=usb\nbluetooth=0C:A6:94:12:34:56\nbluetooth_connected=yes\nusb=Adapter\nusb_name=USB-C to 3.5mm Adapter\nusb_mixer=own\nvolume_usb=45\n");
        assert_eq!((s.output, s.usb.as_deref(), s.usb_own_volume, s.volume_usb, s.bluetooth_connected), (Output::Usb, Some("USB-C to 3.5mm Adapter"), true, 45, true));
        assert_eq!(parse_status("usb=Device\nusb_mixer=software").usb.as_deref(), Some("Device"), "no name: its card's");
    }

    #[test]
    fn where_sound_goes_is_read_from_catalyst_audios_files() {
        let usb = "card=1\nid=Adapter\ndevice=0\nusbid=05ac:110a\nname=USB-C to 3.5mm Headphone Jack Adapter\nmixer=PCM,0\nswitch=yes\n";
        let r = Route::parse("usb 1\n", usb);
        assert_eq!((r.out, r.usb_playing(), r.owner()), (Out::Usb, Some(1), Owner::Usb));
        assert_eq!(r.usb, Some(Usb { card: 1, name: "USB-C to 3.5mm Headphone Jack Adapter".into(), own_volume: true }));
        let r = Route::parse("bluetooth\n", "card=2\nmixer=\n");
        assert_eq!((r.out, r.usb_playing(), r.owner(), r.usb.map(|u| (u.name, u.own_volume))), (Out::Bluetooth, None, Owner::Main, Some(("USB audio".into(), false))));
        assert_eq!(Route::parse("local\n", ""), Route::default());
        assert_eq!(Route::parse("", ""), Route::default(), "nothing routed yet");
        assert_eq!(Route::parse("usb 1\n", "").out, Out::Handheld, "no card described: not USB");
    }

    fn usb_route(card: u32) -> Route {
        Route { out: Out::Usb, usb: Some(Usb { card, ..demo_usb() }) }
    }

    #[test]
    fn music_carries_on_through_headphones_and_pauses_as_usb_ones_come_out() {
        let speaker = Route::default();
        let bluetooth = Route { out: Out::Bluetooth, usb: None };
        let usb = usb_route(1);
        let both_bt = Route { out: Out::Bluetooth, usb: usb.usb.clone() };
        assert_eq!(follow(&speaker, &usb), Follow::Carry, "plugged in");
        assert_eq!(follow(&usb, &speaker), Follow::Pause, "pulled out");
        assert_eq!(follow(&usb, &bluetooth), Follow::Pause, "pulled out, Bluetooth there");
        assert_eq!(follow(&usb, &both_bt), Follow::Carry, "Bluetooth connected after them");
        assert_eq!(follow(&both_bt, &usb), Follow::Carry, "chosen again");
        assert_eq!(follow(&usb, &usb_route(2)), Follow::Pause, "another card: these came out");
        assert_eq!(follow(&speaker, &bluetooth), Follow::Carry, "Bluetooth connected");
        assert_eq!(follow(&bluetooth, &speaker), Follow::Stay, "Bluetooth gone: as before");
        assert_eq!(follow(&bluetooth, &usb), Follow::Carry);
        assert_eq!(follow(&usb, &usb), Follow::Stay);
        assert_eq!(follow(&speaker, &Route { out: Out::Handheld, usb: usb.usb.clone() }), Follow::Stay, "plugged in, not yet playing");
    }

    #[test]
    fn the_island_says_headphones_in_and_out() {
        let speaker = Route::default();
        let usb = usb_route(1);
        let both_bt = Route { out: Out::Bluetooth, usb: usb.usb.clone() };
        assert_eq!(announce(&speaker, &usb, false), Some(PLUGGED));
        assert_eq!(announce(&usb, &speaker, true), Some(UNPLUGGED_PAUSED));
        assert_eq!(announce(&usb, &speaker, false), Some(UNPLUGGED));
        assert_eq!(announce(&both_bt, &Route { out: Out::Bluetooth, usb: None }, true), Some(UNPLUGGED), "not what played: nothing paused");
        assert_eq!(announce(&both_bt, &usb, false), Some(PLUGGED), "chosen over Bluetooth");
        assert_eq!(announce(&usb, &both_bt, false), None, "Bluetooth's page says so");
        assert_eq!(announce(&speaker, &Route { out: Out::Bluetooth, usb: None }, false), None);
        // Whole in the island at every text size (it gives a message 250 px).
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            for words in [PLUGGED, UNPLUGGED, UNPLUGGED_PAUSED, "headphones 100%"] {
                assert!(g.measure(words, g.ty.caption) <= 250.0, "{words:?} at {size:?}");
            }
        }
    }

    #[test]
    fn sound_moving_is_said_and_the_volume_keys_follow_it() {
        let _one_at_a_time = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_level(Level { volume: 50, muted: false });
        LEVELS.lock().unwrap().volume[Owner::Usb.index()] = None;
        take_said();
        let before = moves();
        demo_plug(Some(demo_usb()));
        assert_eq!(take_said().as_deref(), Some(PLUGGED));
        assert!(moves() > before, "pages look again");
        // Their own volume: it starts where theirs does, and moves alone.
        assert_eq!(level().volume, DEFAULT_USB_VOLUME);
        assert_eq!(media_key(Media::VolumeUp).as_deref(), Some("headphones 45%"));
        assert_eq!(icon(), "headphones");
        demo_plug(None);
        assert_eq!(take_said().as_deref(), Some(UNPLUGGED));
        assert_eq!(level().volume, 50, "the speaker's, as it was");
        assert_eq!(media_key(Media::VolumeDown).as_deref(), Some("volume 45%"));
        demo_plug(Some(demo_usb()));
        assert_eq!(level().volume, 45, "theirs, remembered");
        demo_plug(None);
        assert!(take_said().is_some());
        demo_plug(None);
        assert_eq!(take_said(), None, "nothing moved: nothing said");
    }

    #[test]
    fn usb_headphones_say_what_they_wait_for() {
        assert_eq!(Setup::of(true, true, 1, true, false), Setup::Playing, "there and playing, whatever else");
        assert_eq!(Setup::of(true, false, 0, false, true), Setup::Connected);
        assert_eq!(Setup::of(false, false, 2, true, false), Setup::DeviceMode, "a computer's mode: set USB first");
        assert_eq!(Setup::of(false, false, 2, false, true), Setup::DeviceMode, "chosen, not yet restarted");
        assert_eq!(Setup::of(false, false, 0, true, false), Setup::Restart, "charging only again, still a device");
        assert_eq!(Setup::of(false, false, 0, false, false), Setup::NoHub);
        assert_eq!(Setup::of(false, false, 0, false, true), Setup::Ready);
    }

    #[test]
    fn inotify_names_are_read_from_its_events() {
        let mut buf = Vec::new();
        for name in ["output.123", "output"] {
            let len = 16usize;
            buf.extend_from_slice(&1i32.to_ne_bytes());
            buf.extend_from_slice(&0x80u32.to_ne_bytes());
            buf.extend_from_slice(&0u32.to_ne_bytes());
            buf.extend_from_slice(&(len as u32).to_ne_bytes());
            let mut padded = name.as_bytes().to_vec();
            padded.resize(len, 0);
            buf.extend_from_slice(&padded);
        }
        assert_eq!(names_in(&buf), ["output.123", "output"]);
        assert_eq!(names_in(&buf[..20]), [""], "cut short: no name");
        assert!(names_in(&buf[..10]).is_empty());
    }

    #[test]
    fn the_volume_steps_within_bounds_and_mutes() {
        let _one_at_a_time = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_level(Level { volume: 50, muted: false });
        assert_eq!(step(5).volume, 55);
        assert_eq!(step(-100).volume, 0);
        assert_eq!(step(500).volume, 100);
        let muted = toggle_mute();
        assert!(muted.muted && muted.heard() == 0 && muted.words() == "sound off");
        assert_eq!(muted.icon(), "volume_off");
        // Down while muted stays muted; up unmutes.
        assert!(step(-5).muted);
        let up = step(5);
        assert!(!up.muted);
        assert_eq!(up.words(), "volume 100%");
        assert_eq!(set_level(Level { volume: 30, muted: false }).icon(), "volume_down");
    }

    #[test]
    fn every_hint_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut page = SoundScreen::new();
        let sizes = [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large];
        let setups = [Setup::Playing, Setup::Connected, Setup::Ready, Setup::NoHub, Setup::DeviceMode, Setup::Restart];
        for output in [Output::Speaker, Output::Headphones, Output::Bluetooth, Output::Usb] {
            for setup in setups {
                for bt in [false, true] {
                    page.status = Some(Status { output, setup, bluetooth: Some("0C:A6:94:12:34:56".into()), bluetooth_connected: bt, card: true, ..Status::default() });
                    for size in sizes {
                        g.set_text_size(size);
                        let caption = g.ty.caption;
                        for row in ALL {
                            let hints = page.hint(row);
                            let shortest = hints.last().unwrap();
                            assert!(g.measure(shortest, caption) <= W - 2.0 * PAD, "{shortest:?} at {size:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_row_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut page = SoundScreen::new();
        let width = W - 2.0 * PAD - 5.0;
        for look in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = look;
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let (name_style, value_style) = if look == crate::look::Theme::Bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                for setup in [Setup::Playing, Setup::Connected, Setup::Ready, Setup::NoHub, Setup::DeviceMode, Setup::Restart] {
                    page.status = Some(Status { output: Output::Usb, setup, card: true, ..Status::default() });
                    for row in ALL {
                        let (name, value, _) = page.describe(row);
                        let name = if look == crate::look::Theme::Bezel { name.to_lowercase() } else { name.to_owned() };
                        let need = g.measure(&name, name_style) + g.measure(&value, value_style) + 30.0;
                        assert!(need <= width, "{name} · {value} at {size:?} in {look:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_usb_page_says_it_all_above_its_row_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let name = "USB-C to 3.5mm Headphone Jack Adapter";
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            // Clear of the row by a little air, not touching it.
            let room = crate::gfx::H - PAD - g.hint_h() - g.row_h() - g.top() - 2.0 - 8.0;
            for setup in [Setup::Playing, Setup::Connected, Setup::Ready, Setup::NoHub, Setup::DeviceMode, Setup::Restart] {
                let lines = crate::help::wrap(&mut g, &usb_words(setup, name), ty.caption, W - 2.0 * PAD);
                let tall = lines.len() as f32 * (ty.caption.size + 4.0);
                assert!(tall <= room, "{setup:?} at {size:?}: {} lines", lines.len());
            }
            // Its title whole beside the island at rest with no robot set.
            let rest = g.measure("no robot set", ty.caption) + 34.0;
            let title = UsbAudioPage::new().title().to_owned();
            assert!(g.measure(&title, ty.title) <= (W - rest) * 0.5 - PAD - 8.0, "{title} at {size:?}");
        }
    }

    #[test]
    fn the_microphone_gain_skips_what_the_codec_forbids() {
        assert_eq!(mic_gain_step(3, 1), 6);
        assert_eq!(mic_gain_step(6, -1), 3);
        assert_eq!(mic_gain_step(14, 1), 14);
        assert_eq!(mic_gain_step(0, -1), 0);
        assert_eq!(mic_gain_words(8), "18.5 dB");
    }

    #[test]
    fn the_meter_reads_peaks_in_decibels() {
        let full = [0xffu8, 0x7f, 0x00, 0x00];
        assert!((peak_of(&full) - 1.0).abs() < 0.001);
        assert_eq!(peak_of(&[0, 0, 0, 0]), 0.0);
        assert_eq!(level_of(1.0), 1.0);
        assert!((level_of(0.031_6) - 0.5).abs() < 0.01, "-30 dBFS is half way");
        assert_eq!(level_of(0.0), 0.0);
    }

    #[test]
    fn enter_on_the_usb_page_takes_the_next_step_toward_headphones() {
        crate::shell::tests::sandbox();
        let _turn = crate::usb::MODE_FILE.lock().unwrap_or_else(|e| e.into_inner());
        // A computer's mode has the port (serial, as the handheld had it).
        crate::usb::set(1).unwrap();
        let mut page = UsbAudioPage::new();
        page.status = Some(Status { setup: Setup::DeviceMode, ..Status::default() });
        let (mut robot, mut settings, mut say) = (crate::robot::Robot::demo(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Enter, &mut cx);
        assert_eq!(crate::usb::mode(), 0, "charging only, asked for here");
        assert!(say.as_deref().is_some_and(|s| s.contains("charging only") && s.contains("restart")), "{say:?}");
        // Set and waiting for a start: Enter restarts, on the handheld alone
        // (here it changes nothing).
        page.status = Some(Status { setup: Setup::Restart, ..Status::default() });
        let mut say = None;
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Enter, &mut cx);
        assert_eq!(crate::usb::mode(), 0);
        // Ready or playing: Enter changes nothing.
        crate::usb::set(1).unwrap();
        page.status = Some(Status { setup: Setup::Ready, ..Status::default() });
        page.key(Key::Enter, &mut cx);
        assert_eq!(crate::usb::mode(), 1, "left as it was");
        crate::usb::set(0).unwrap();
    }

    #[test]
    fn how_a_song_plays_is_said_shortest_last() {
        let s = parse_stream("codec=flac\nrate=44100\nbits=16\nout_rate=44100\nout_bits=16\ndevice=hw\noutput=usb\nbitperfect=yes\nprocessing=none\ntracking=no\n");
        assert!(s.bit_perfect && s.processing.is_empty());
        assert_eq!(quality_words(&s), ["44.1 kHz · 16-bit · bit-perfect", "44.1 kHz · bit-perfect", "bit-perfect"]);
        let spatial = parse_stream("rate=48000\nbits=16\nout_rate=48000\nbitperfect=no\nprocessing=spatial,loudness\ntracking=yes\n");
        assert_eq!(quality_words(&spatial)[0], "48 kHz · 16-bit · spatial, head tracked");
        assert_eq!(quality_words(&spatial)[2], "spatial");
        let resampled = parse_stream("rate=96000\nbits=24\nout_rate=48000\nprocessing=resampled\n");
        assert_eq!(quality_words(&resampled)[0], "96 kHz · 24-bit · to 48 kHz");
        assert_eq!(quality_words(&parse_stream("rate=48000\nbits=16\nprocessing=none\n"))[0], "48 kHz · 16-bit · mixed");
    }

    #[test]
    fn rows_come_with_the_song_and_the_airpods_and_search_finds_them() {
        let mut page = SoundScreen::new();
        page.stream = None;
        page.pods = None;
        assert!(!page.rows().contains(&Row::Quality) && !page.rows().contains(&Row::Noise));
        assert_eq!(page.rows()[USB_ROW], Row::Usb, "the USB row stays where search expects it");
        page.stream = Some(parse_stream("rate=44100\nbits=16\nbitperfect=yes\n"));
        crate::airpods::demo(true);
        page.pods = crate::airpods::now();
        crate::airpods::demo(false);
        assert_eq!(page.rows().len(), ALL.len());
        let page = page.at(TEST_ROW);
        assert_eq!(page.row(), Row::Test);
        let page = SoundScreen::new().at(SPATIAL_ROW);
        assert_eq!(page.row(), Row::Spatial);
    }

    #[test]
    fn the_quality_and_noise_rows_fit_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut page = SoundScreen::new();
        let width = W - 2.0 * PAD - 5.0;
        crate::airpods::demo(true);
        page.pods = crate::airpods::now();
        crate::airpods::demo(false);
        let streams = ["rate=44100\nbits=16\nbitperfect=yes\n", "rate=48000\nbits=16\nprocessing=spatial\ntracking=yes\n", "rate=192000\nbits=24\nout_rate=48000\nprocessing=resampled\n"];
        for look in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = look;
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let (name_style, value_style) = if look == crate::look::Theme::Bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                for s in streams {
                    page.stream = Some(parse_stream(s));
                    for mode in [aap::NoiseMode::Off, aap::NoiseMode::Cancelling, aap::NoiseMode::Transparency, aap::NoiseMode::Adaptive] {
                        page.pods.as_mut().unwrap().noise = Some(mode);
                        for row in [Row::Quality, Row::Noise, Row::Spatial, Row::Equalizer, Row::Loudness, Row::Crossfade] {
                            let (name, _, _) = page.describe(row);
                            let value = page.fitted(&mut g, row, width);
                            let name = if look == crate::look::Theme::Bezel { name.to_lowercase() } else { name.to_owned() };
                            let need = g.measure(&name, name_style) + g.measure(&value, value_style) + 30.0;
                            assert!(need <= width, "{name} · {value} at {size:?} in {look:?}");
                            let hints = page.hint(row);
                            assert!(g.measure(hints.last().unwrap(), g.ty.caption) <= W - 2.0 * PAD, "{hints:?} at {size:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn music_settings_change_with_their_keys() {
        let (mut robot, mut settings, mut say) = (crate::robot::Robot::demo(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        set_fx(|f| f.crossfade = 0);
        let mut page = SoundScreen::new().at(CROSSFADE_ROW);
        page.key(Key::Right, &mut cx);
        page.key(Key::Right, &mut cx);
        assert_eq!(fx().crossfade, 2);
        for _ in 0..3 {
            page.key(Key::Left, &mut cx);
        }
        assert_eq!(fx().crossfade, 0, "never below none");
        let mut page = SoundScreen::new().at(SPATIAL_ROW);
        let was = fx().spatial;
        page.key(Key::Enter, &mut cx);
        assert_eq!(fx().spatial, !was);
        page.key(Key::Enter, &mut cx);
        assert_eq!(fx().spatial, was);
    }
}
