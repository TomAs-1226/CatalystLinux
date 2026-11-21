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
//! On a computer (the simulator) nothing plays: the level is kept in memory
//! and the microphone's meter moves as if someone were talking.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bezel::Rect;

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::git::Job;
use crate::platform::{Key, Media};
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const SCRIPT: &str = "/usr/sbin/catalyst-audio";
const STATE: &str = "/data/catalyst";
/// What the volume keys move it by.
pub const STEP: u32 = 5;
/// catalyst-audio's own starting level, for a handheld that has never set one.
const DEFAULT_VOLUME: u32 = 65;
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

static LEVEL: Mutex<Option<Level>> = Mutex::new(None);

/// Tests that change the volume hold this, one at a time: it is the one
/// volume the whole shell shares.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());
static WORKER: OnceLock<Mutex<Sender<Level>>> = OnceLock::new();

/// The level as saved by catalyst-audio (its files on /data).
fn read_saved() -> Level {
    if !on_device() {
        return Level { volume: DEFAULT_VOLUME, muted: false };
    }
    let volume = std::fs::read_to_string(Path::new(STATE).join("audio-volume")).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(DEFAULT_VOLUME).min(100);
    Level { volume, muted: Path::new(STATE).join("audio-muted").exists() }
}

/// The volume now.
pub fn level() -> Level {
    *LEVEL.lock().unwrap().get_or_insert_with(read_saved)
}

/// Set the volume and mute together, at once here and soon on the handheld.
pub fn set_level(level: Level) -> Level {
    let level = Level { volume: level.volume.min(100), muted: level.muted };
    *LEVEL.lock().unwrap() = Some(level);
    if on_device() {
        let sender = WORKER.get_or_init(|| Mutex::new(start_worker()));
        sender.lock().unwrap().send(level).ok();
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

/// The thread that hands levels to catalyst-audio, the last of a run of them.
fn start_worker() -> Sender<Level> {
    let (tx, rx): (Sender<Level>, Receiver<Level>) = channel();
    std::thread::Builder::new()
        .name("sound".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            // As saved when the shell started: only what changes is sent.
            let mut applied = read_saved();
            while let Ok(mut want) = rx.recv() {
                // Presses that came meanwhile: only the last counts.
                while let Ok(newer) = rx.try_recv() {
                    want = newer;
                }
                // Muting first and unmuting last, so a level set while
                // muted is never heard on the way.
                if want.muted && !applied.muted {
                    run(&["mute", "on"]).ok();
                }
                if want.volume != applied.volume {
                    run(&["volume", &want.volume.to_string()]).ok();
                }
                if !want.muted && applied.muted {
                    run(&["mute", "off"]).ok();
                }
                applied = want;
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

/// At start: the reminders' chime.
pub fn start() {
    crate::reminders::set_chime(reminder_chime);
}

// ------------------------------------------------------------------- keys ---

/// A sound or music key, over any screen: what the island says.
pub fn media_key(key: Media) -> Option<String> {
    match key {
        Media::VolumeUp => Some(step(STEP as i32).words()),
        Media::VolumeDown => Some(step(-(STEP as i32)).words()),
        Media::Mute => Some(toggle_mute().words()),
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mic {
    /// The handheld's own: a MEMS microphone beside the keyboard.
    #[default]
    Builtin,
    /// A headset's, on the jack.
    Headset,
}

/// What catalyst-audio says of things (`catalyst-audio status`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub card: bool,
    pub output: Output,
    pub bluetooth: Option<String>,
    pub volume: u32,
    pub muted: bool,
    pub mic: Mic,
    pub mic_gain: u32,
    pub playing: bool,
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status { volume: DEFAULT_VOLUME, mic_gain: DEFAULT_MIC_GAIN, ..Status::default() };
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match name.trim() {
            "card" => s.card = value != "none" && !value.is_empty(),
            "output" => {
                s.output = match value {
                    "headphones" => Output::Headphones,
                    "bluetooth" => Output::Bluetooth,
                    _ => Output::Speaker,
                }
            }
            "bluetooth" if !value.is_empty() => s.bluetooth = Some(value.to_owned()),
            "volume" => s.volume = value.parse().unwrap_or(s.volume).min(100),
            "muted" => s.muted = value == "yes",
            "mic" => s.mic = if value == "headset" { Mic::Headset } else { Mic::Builtin },
            "mic_gain" => s.mic_gain = value.parse().unwrap_or(s.mic_gain),
            "playing" => s.playing = value == "yes",
            _ => {}
        }
    }
    s
}

/// How things stand now: off the drawing thread.
fn read_status() -> Result<Status, String> {
    if !on_device() {
        let l = level();
        let mic = *DEMO_MIC.lock().unwrap();
        let bluetooth = crate::bluetooth::demo_connected();
        let output = if bluetooth.is_some() { Output::Bluetooth } else { Output::Speaker };
        return Ok(Status { card: true, output, bluetooth, volume: l.volume, muted: l.muted, mic: mic.0, mic_gain: mic.1, playing: false });
    }
    run(&["status"]).map(|text| parse_status(&text))
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

// ----------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Volume,
    Mute,
    Output,
    Test,
    Microphone,
    MicGain,
    Bluetooth,
}

const ROWS: [Row; 7] = [Row::Volume, Row::Mute, Row::Output, Row::Test, Row::Microphone, Row::MicGain, Row::Bluetooth];

/// Settings › Sound: the volume, silence, where sound goes, a test sound,
/// the microphone (with its level, live, while it is chosen) and its gain,
/// and Bluetooth.
pub struct SoundScreen {
    list: ListState,
    status: Option<Status>,
    reading: Option<Job<Result<Status, String>>>,
    read_at: Instant,
    /// The volume bar, gliding to the level.
    bar: detent::Animated,
    /// The microphone's meter, while its row is chosen.
    meter: Option<MicMeter>,
    meter_bar: detent::Animated,
    working: Option<(Row, Job<Result<(), String>>)>,
    changed: bool,
}

impl SoundScreen {
    pub fn new() -> Self {
        let mut page = SoundScreen {
            list: ListState::default(),
            status: None,
            reading: None,
            read_at: Instant::now(),
            bar: detent::Animated::new(level().volume as f32 / 100.0, detent::role::hold()).with_rest(0.003),
            meter: None,
            meter_bar: detent::Animated::new(0.0, detent::role::tick()).with_rest(0.003),
            working: None,
            changed: true,
        };
        page.read_now();
        page
    }

    fn read_now(&mut self) {
        self.read_at = Instant::now();
        self.reading = Some(Job::start(read_status));
    }

    fn row(&self) -> Row {
        ROWS[self.list.focus.min(ROWS.len() - 1)]
    }

    fn start(&mut self, row: Row, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        if self.working.is_none() {
            self.working = Some((row, Job::start(work)));
        }
    }

    fn describe(&self, row: Row) -> (&'static str, String, bool) {
        let st = self.status.as_ref();
        let l = level();
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
                        Output::Bluetooth => crate::bluetooth::name_of(s.bluetooth.as_deref().unwrap_or_default()),
                    },
                };
                ("Playing on", value, st.is_some_and(|s| s.card))
            }
            Row::Test => ("Test sound", (if matches!(self.working, Some((Row::Test, _))) { "playing…" } else { "" }).into(), true),
            Row::Microphone => ("Microphone", (if st.is_some_and(|s| s.mic == Mic::Headset) { "headset's" } else { "handheld's" }).into(), true),
            Row::MicGain => ("Mic gain", mic_gain_words(st.map_or(DEFAULT_MIC_GAIN, |s| s.mic_gain)).into(), true),
            Row::Bluetooth => ("Bluetooth", crate::bluetooth::summary(), crate::bluetooth::enabled()),
        }
    }

    /// What the keys do on a row: the longest that fits whole first.
    fn hint(&self, row: Row) -> &'static [&'static str] {
        match row {
            Row::Volume => &["←→: volume · fn + s, fn + d anywhere", "←→: volume"],
            Row::Mute => &["enter: mute or unmute · fn + a anywhere", "enter: mute or unmute"],
            Row::Output => match self.status.as_ref().map(|s| s.output) {
                Some(Output::Headphones) => &["the speaker is off while headphones are in", "speaker off: headphones in"],
                Some(Output::Bluetooth) => &["bluetooth headphones, while connected", "bluetooth, while connected"],
                _ => &["headphones take over when plugged in", "headphones take over"],
            },
            Row::Test => &["enter: a short chime, at this volume", "enter: a short chime"],
            Row::Microphone if self.meter.as_ref().is_some_and(|m| !m.working()) => &["the microphone could not be opened", "no microphone"],
            Row::Microphone => &["←→: the handheld's or a headset's", "←→: which microphone"],
            Row::MicGain => &["←→: how much the mic is amplified", "←→: mic gain"],
            Row::Bluetooth => &["enter: headphones and speakers", "enter: bluetooth"],
        }
    }
}

impl Screen for SoundScreen {
    fn title(&self) -> &str {
        "Sound"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let row = self.row();
        let left_right = matches!(key, Key::Left | Key::Right);
        let step = if key == Key::Left { -1 } else { 1 };
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right if row == Row::Volume => {
                let l = step_volume(step * STEP as i32);
                self.bar.set(l.volume as f32 / 100.0);
                *cx.say = Some(l.words());
            }
            Key::Enter | Key::Char(' ') if row == Row::Mute => {
                *cx.say = Some(toggle_mute().words());
            }
            Key::Enter if row == Row::Test => {
                if self.working.is_none() {
                    self.start(Row::Test, test_tone);
                }
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
            Key::Enter | Key::Right if row == Row::Bluetooth => return Nav::Push(Box::new(crate::bluetooth::BluetoothScreen::new())),
            Key::Enter if row == Row::Output && self.status.as_ref().is_some_and(|s| s.output == Output::Bluetooth) => {
                return Nav::Push(Box::new(crate::bluetooth::BluetoothScreen::new()))
            }
            Key::Char('r') => self.read_now(),
            other if !left_right => crate::screens::list_key(&mut self.list, other, ROWS.len()),
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
        if self.read_at.elapsed().as_secs_f32() >= 2.0 && self.reading.is_none() && self.working.is_none() {
            self.read_now();
        }
        // The microphone is listened to only while its row is chosen.
        let listening = self.row() == Row::Microphone && self.working.is_none();
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
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom + 2.0) {
            let row = ROWS[i];
            let r = Rect::new(PAD, y, width, row_h);
            let (name, value, lit) = self.describe(row);
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
        self.list.scrollbar(g, ROWS.len(), top, bottom);
        let hint = self.hint(self.row());
        g.hint_any(hint);
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
    /// Opened at one of its rows (from search).
    pub fn at(mut self, row: usize) -> Self {
        self.list.focus = row.min(ROWS.len() - 1);
        self
    }
}

/// The volume moved from a page or the control centre (as the keys move it).
pub fn step_volume(delta: i32) -> Level {
    step(delta)
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
        assert_eq!((s.output, s.volume, s.mic, s.mic_gain), (Output::Speaker, DEFAULT_VOLUME, Mic::Builtin, DEFAULT_MIC_GAIN));
        assert_eq!(parse_status("output=headphones").output, Output::Headphones);
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
        let page = SoundScreen::new();
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let caption = g.ty.caption;
            for row in ROWS {
                let shortest = page.hint(row).last().unwrap();
                assert!(g.measure(shortest, caption) <= W - 2.0 * PAD, "{shortest:?} at {size:?}");
            }
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
}
