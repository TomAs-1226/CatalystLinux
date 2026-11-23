//! Deep standby: after a while with the screen off and no key (half an hour,
//! unless chosen otherwise in Settings › Battery), the handheld goes as near
//! off as it can while a key can still wake it.
//!
//! The CM0 has no suspend, and only the slide switch cuts the power, so this
//! is everything the keyboard does not need, switched off. The screen is
//! already off and the processor at its slowest (standby.rs); deep standby
//! then takes these steps, in this order, and undoes each in the reverse
//! order as it wakes:
//!
//! 1. **Pollers**: the robot's link let go, and every thread that looks in
//!    on something (the robot's logs, the event schedule, an open Wi-Fi or
//!    agent page) parked on `rest` with no wakeups at all. Update checks
//!    and the weather are left until it wakes, as nothing ticks meanwhile.
//! 2. **Motion**: the motion sensor stops watching (no lift to wake: a key
//!    wakes it).
//! 3. **Lights**: the keyboard's three lights dark.
//! 4. **System**: `catalyst-power deep on`, on a thread of its own: Wi-Fi
//!    and the other radios off, the USB host (the hub and the Ethernet chip)
//!    stopped, the Grove and HAT 5 V outputs off, the red light dark.
//! 5. **Slack**: the shell's own wait stretched to a minute, and the kernel
//!    let fold its wakeup in with others (a second of timer slack).
//!
//! Any key wakes it, and is spent doing so (it, and its repeats while held,
//! go to no app). The island then says how long it slept and what it drew
//! on average, from the fuel gauge; the Battery page keeps the last figures.
//! Left long (12 hours) or low (10%) on battery, the red light blinks once a
//! minute, short: still on, slide the switch off. The kernel's timer trigger
//! blinks it, so nothing in the shell wakes for that.
//!
//! **Music standby.** Music keeps deep standby away (it would cut the sound
//! off), but not the rest of it: half a minute after the screen goes dark
//! with music playing, the same steps are taken with the sound left alone.
//! catalyst-power keeps the radios (Bluetooth headphones may be what plays)
//! and Wi-Fi while what plays next comes over the network (Music says, by
//! `nowplaying::needs_network`), and the USB host while USB headphones are
//! what plays (sound.rs says); otherwise the USB host goes off, with the 5 V
//! outputs and every light, the motion sensor stops, the shell's polling
//! parks and it looks in once a minute. I2S, the codec and the amplifier
//! belong to what plays, and the processor stays at its slowest, which
//! decodes with room to spare. The music keys (Fn + Q, W, E; A, S, D) work
//! without waking it.
//! Any other key wakes it with every step undone, as from deep standby.
//! Once the music has stopped for as long as the screen takes to go off, it
//! moves on: to deep standby in place if its wait is over (Wi-Fi and the
//! radios go too, nothing else is taken twice), or back to plain standby,
//! from which deep standby comes when its time does.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bezel::Rect;

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::power::Reading;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// How long with no key before deep standby, in minutes (0: never).
pub const AFTER_CHOICES: [u32; 4] = [10, 30, 60, 0];
pub const AFTER_DEFAULT: u32 = 30;

/// In deep standby the shell waits a minute at a time for a key.
pub const WAIT: Duration = Duration::from_secs(60);
/// The kernel may put the shell's wakeup off by this much to fold it in
/// with another, in deep standby.
#[cfg(target_os = "linux")]
const SLACK_NS: libc::c_ulong = 1_000_000_000;

/// How often, at most, the shell asks whether something stops it going deep.
const LOOK_EVERY: f32 = 10.0;

/// Music playing, the screen dark this long: music standby. Not at once, so
/// a screen put out and woken again straight away costs nothing.
pub const MUSIC_AFTER: f32 = 30.0;

/// The red light's reminder: after this long in deep standby, or at this
/// charge, on battery.
const REMIND_AFTER: f32 = 12.0 * 3600.0;
const REMIND_AT: u32 = 10;
/// Its blink: on this many milliseconds, then off for the rest of a minute.
pub const BLINK_MS: (u32, u32) = (150, 59_850);

/// What deep standby draws, until it has been measured here.
pub const ESTIMATE: &str = "0.6–1 W";
/// What a full battery holds, in watt hours: 1750 mAh at 3.7 V.
const BATTERY_WH: f32 = 6.5;

/// Measured over less than this, no draw is given (the gauge's steps are
/// too coarse).
const MEASURE_SAMPLED: f64 = 300.0;
const MEASURE_SLEPT: f32 = 600.0;

/// Where catalyst-power notes what it switched off.
const STATE: &str = "/run/catalyst-deep";

// ----------------------------------------------------------- the choice ---

/// "10 min", "1 hour", "never".
pub fn after_name(minutes: u32) -> String {
    match minutes {
        0 => "never".into(),
        60 => "1 hour".into(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} min"),
    }
}

/// The Battery page's value: "after 30 min", or "never".
pub fn after_value(minutes: u32) -> String {
    if minutes == 0 {
        "never".into()
    } else {
        format!("after {}", after_name(minutes))
    }
}

/// The next choice along (or back).
pub fn step_after(minutes: u32, step: i32) -> u32 {
    let at = AFTER_CHOICES.iter().position(|m| *m == minutes).unwrap_or(1) as i32;
    AFTER_CHOICES[(at + step).rem_euclid(AFTER_CHOICES.len() as i32) as usize]
}

/// A saved choice: one of the choices, or the default.
pub fn parse_after(value: &str) -> u32 {
    value.trim().parse().ok().filter(|m| AFTER_CHOICES.contains(m)).unwrap_or(AFTER_DEFAULT)
}

// -------------------------------------------------------- the gate ---

/// Where background work waits while deep standby is on: parked on a
/// condition variable, so it costs no wakeups until the handheld wakes.
struct Gate {
    paused: Mutex<bool>,
    resumed: Condvar,
}

impl Gate {
    const fn new() -> Gate {
        Gate { paused: Mutex::new(false), resumed: Condvar::new() }
    }

    fn rest(&self) {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        while *paused {
            paused = self.resumed.wait(paused).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn set(&self, paused: bool) {
        *self.paused.lock().unwrap_or_else(PoisonError::into_inner) = paused;
        self.resumed.notify_all();
    }
}

static GATE: Gate = Gate::new();

/// Called by background work each time round: in deep standby it waits
/// here until the handheld wakes; otherwise it returns at once.
pub fn rest() {
    GATE.rest();
}

/// A background loop's wait between rounds, and its stop signal: one
/// wakeup a round, where a loop that slept in short steps to see whether it
/// had been stopped woke five or ten times a second; stopped, it ends at
/// once all the same. In deep standby the loop then parks (`rest`).
#[derive(Default)]
pub struct Nap {
    stopped: Mutex<bool>,
    woken: Condvar,
}

impl Nap {
    /// Stop the loop: a wait under way ends now.
    pub fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.woken.notify_all();
    }

    /// Wait `round`, or until stopped; then, in deep standby, until it
    /// wakes. True if the loop is stopped.
    pub fn nap(&self, round: Duration) -> bool {
        let guard = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        let (guard, _) = self.woken.wait_timeout_while(guard, round, |stopped| !*stopped).unwrap_or_else(PoisonError::into_inner);
        let stopped = *guard;
        drop(guard);
        if !stopped {
            rest();
        }
        stopped
    }
}

/// Park background work (or let it go on). The tests run many shells side
/// by side, so there it parks nothing (one shell must not hold up another's
/// threads); the gate itself is tested on its own.
fn pause_background(paused: bool) {
    #[cfg(not(test))]
    GATE.set(paused);
    #[cfg(test)]
    let _ = paused;
}

/// The shell's own wakeups, in deep standby: the kernel may put one off by
/// up to a second to fold it in with another.
fn timer_slack(deep: bool) {
    #[cfg(target_os = "linux")]
    unsafe {
        // 0 is the thread's default again.
        libc::prctl(libc::PR_SET_TIMERSLACK, if deep { SLACK_NS } else { 0 });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = deep;
}

// ----------------------------------------------------------- the record ---

/// One deep standby, as the island says it and the Battery page keeps it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Slept {
    /// When it woke, in Unix seconds.
    pub woke: i64,
    pub seconds: u64,
    /// What it drew from the battery on average, in watts: none if it was
    /// too short to tell, or on the charger.
    pub watts: Option<f32>,
    /// On the charger for some of it.
    pub charging: bool,
}

impl Slept {
    fn line(&self) -> String {
        let watts = self.watts.map_or("-".into(), |w| format!("{w:.3}"));
        format!("{} {} {watts} {}\n", self.woke, self.seconds, if self.charging { "c" } else { "d" })
    }

    pub fn parse(text: &str) -> Option<Slept> {
        let mut f = text.split_whitespace();
        let woke = f.next()?.parse().ok()?;
        let seconds = f.next()?.parse().ok()?;
        let watts = match f.next()? {
            "-" => None,
            w => Some(w.parse::<f32>().ok().filter(|w| w.is_finite() && *w >= 0.0)?),
        };
        let charging = f.next()? == "c";
        Some(Slept { woke, seconds, watts, charging })
    }

    /// "7 h 12 m", "4 m".
    pub fn length(&self) -> String {
        crate::sys::minutes_text((self.seconds / 60).max(1))
    }

    /// The Battery page's figures: "7 h 12 m · 0.68 W".
    pub fn summary(&self) -> String {
        match (self.watts, self.charging) {
            (_, true) => format!("{} · charging", self.length()),
            (Some(w), false) => format!("{} · {w:.2} W", self.length()),
            (None, false) => self.length(),
        }
    }

    /// What the island says on waking: "slept 7 h 12 m · ~0.68 W" (short,
    /// so the title beside it keeps a few letters).
    pub fn island(&self) -> String {
        match (self.watts, self.charging) {
            (_, true) => format!("slept {} · charging", self.length()),
            (Some(w), false) => format!("slept {} · ~{w:.2} W", self.length()),
            (None, false) => format!("slept {}", self.length()),
        }
    }
}

/// Where the last deep standby is kept, beside the settings.
pub fn record_path() -> PathBuf {
    crate::config_dir().join("deep-standby")
}

/// The last deep standby, if there has been one.
pub fn last() -> Option<Slept> {
    Slept::parse(&std::fs::read_to_string(record_path()).ok()?)
}

pub fn keep(slept: &Slept) {
    crate::sys::write_atomic(&record_path(), slept.line().as_bytes()).ok();
}

/// A full battery's hours at `watts`: "about 9 h".
fn hours_from_full(watts: f32) -> String {
    format!("about {:.0}\u{a0}h", (BATTERY_WH / watts.max(0.05)).min(999.0))
}

/// What Settings says of deep standby: what it does, what it still draws
/// (measured here once it has been, an estimate until then), and that only
/// the switch is off. Numbers keep to their units (no-break spaces).
pub fn words(after: u32, last: Option<Slept>) -> String {
    if after == 0 {
        return "Never: with the screen off, Wi-Fi, USB and the lights stay on. For days, slide the switch off.".into();
    }
    let draw = match last.and_then(|s| s.watts) {
        Some(w) => format!("Last time {w:.2}\u{a0}W: {} from full.", hours_from_full(w)),
        None => format!("Estimated {}: a night from full.", ESTIMATE.replace(' ', "\u{a0}")),
    };
    let after = after_name(after).replace(' ', "\u{a0}");
    format!("After {after} with no key, Wi-Fi, USB and the lights go off; a key wakes it. {draw} For days, slide the switch off.")
}

// ---------------------------------------------------- measuring the draw ---

/// What deep standby drew, from the gauge: the readings taken while it
/// slept (once a minute), weighted by the time each stood for; failing
/// those, the charge it lost over the time. The readings come first: the
/// gauge corrects its charge from the battery's resting voltage after a
/// long rest, which can move it in a step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
    /// The charge going in (mAh) and the voltage.
    start: Option<(f32, f32)>,
    /// Watt seconds and seconds, from readings on battery.
    energy: f64,
    sampled: f64,
    charged: bool,
}

impl Meter {
    pub fn begin(r: Option<&Reading>) -> Meter {
        let start = r.and_then(|r| Some((r.charge_mah?, r.volts?)));
        Meter { start, charged: r.is_some_and(|r| !r.flow.on_battery()), ..Meter::default() }
    }

    /// A reading taken `dt` seconds after the last.
    pub fn sample(&mut self, r: &Reading, dt: f32) {
        if !r.flow.on_battery() {
            self.charged = true;
            return;
        }
        if let Some(w) = r.watts().filter(|w| w.is_finite() && *w >= 0.0) {
            self.energy += w as f64 * dt as f64;
            self.sampled += dt as f64;
        }
    }

    /// The average draw over `slept` seconds, ending at `end`.
    pub fn watts(&self, end: Option<&Reading>, slept: f32) -> Option<f32> {
        if self.charged || end.is_some_and(|r| !r.flow.on_battery()) {
            return None;
        }
        if self.sampled >= MEASURE_SAMPLED {
            return Some((self.energy / self.sampled) as f32);
        }
        let ((from, v0), (to, v1)) = (self.start?, end.and_then(|r| Some((r.charge_mah?, r.volts?)))?);
        let lost = from - to;
        (slept >= MEASURE_SLEPT && lost >= 2.0).then(|| lost / 1000.0 * (v0 + v1) * 0.5 / (slept / 3600.0))
    }

    pub fn charged(&self) -> bool {
        self.charged
    }
}

// --------------------------------------------------------- the blockers ---

/// What would be cut off by going deep now.
#[derive(Clone, Copy, Default, Debug)]
pub struct Busy {
    /// A robot is linked (not the demo one).
    pub robot: bool,
    /// A computer has the handheld as its device over USB.
    pub computer: bool,
    /// Someone is logged in over SSH.
    pub ssh: bool,
    /// An update is being checked for, downloaded or installed.
    pub update: bool,
    /// A Clock timer is running: it must be said when it runs out.
    pub timer: bool,
    /// A screenshot is being saved, or the battery is running out.
    pub saving: bool,
    /// Music is playing (it streams over Wi-Fi, which deep standby stops).
    pub music: bool,
    /// Songs are downloading (over Wi-Fi, which deep standby stops).
    pub downloads: bool,
    /// On the charger: nothing to save, and it stays reachable.
    pub charger: bool,
}

/// Why not, if anything stops it.
pub fn blocker(b: Busy) -> Option<&'static str> {
    [
        (b.robot, "a robot is linked"),
        (b.computer, "a computer has it over USB"),
        (b.ssh, "someone is logged in over SSH"),
        (b.update, "an update is under way"),
        (b.timer, "a timer is running"),
        (b.saving, "something is being saved"),
        (b.music, "music is playing"),
        (b.downloads, "songs are downloading"),
        (b.charger, "it is on the charger"),
    ]
    .into_iter()
    .find_map(|(busy, why)| busy.then_some(why))
}

// ---------------------------------------------------------------- steps ---

/// Deep standby's own steps, in the order they are taken going in; coming
/// out, each is undone in the reverse order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Pollers,
    Motion,
    Lights,
    System,
    Slack,
}

pub const STEPS: [Step; 5] = [Step::Pollers, Step::Motion, Step::Lights, Step::System, Step::Slack];

/// Coming out: these at once (the last ones taken, back to System)...
pub fn waking_first() -> impl Iterator<Item = Step> {
    let at = STEPS.iter().position(|s| *s == Step::System).unwrap_or(0);
    STEPS[at..].iter().rev().copied()
}

/// ...and these once catalyst-power has brought the system back.
pub fn waking_after() -> impl Iterator<Item = Step> {
    let at = STEPS.iter().position(|s| *s == Step::System).unwrap_or(0);
    STEPS[..at].iter().rev().copied()
}

/// Steps the shell carries out itself (the rest touch its own parts: the
/// robot, the lights).
pub fn take(step: Step, deep: bool, deep_state: &mut Deep) {
    match step {
        Step::Pollers => pause_background(deep),
        Step::Motion => {
            // Back: watched again from the next tick, as the settings ask.
            if deep {
                crate::imu::set_watch(false);
            }
        }
        Step::System => deep_state.system(deep),
        Step::Slack => timer_slack(deep),
        Step::Lights => {}
    }
    #[cfg(test)]
    deep_state.log.push((step, deep));
}

// -------------------------------------------------- catalyst-power, apart ---

/// `catalyst-power deep on|off`, one at a time on a thread of its own, so
/// the frame never waits for it (Wi-Fi stopping takes half a second). An
/// "off" sent while an "on" is still running waits its turn.
#[derive(Default)]
struct Worker {
    tx: Option<Sender<Vec<&'static str>>>,
    done: Option<Receiver<()>>,
    pending: usize,
    /// What was asked of catalyst-power, in order, for the tests.
    #[cfg(test)]
    asked: Vec<Vec<&'static str>>,
}

impl Worker {
    fn send(&mut self, args: Vec<&'static str>) {
        #[cfg(test)]
        self.asked.push(args.clone());
        let script = crate::power::SCRIPT;
        if !Path::new(script).exists() {
            return;
        }
        if self.tx.is_none() {
            let (tx, rx) = mpsc::channel::<Vec<&'static str>>();
            let (done_tx, done_rx) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("deep standby".into()).stack_size(64 * 1024).spawn(move || {
                for args in rx {
                    std::process::Command::new(script).args(args).status().ok();
                    done_tx.send(()).ok();
                }
            });
            if spawned.is_err() {
                return;
            }
            (self.tx, self.done) = (Some(tx), Some(done_rx));
        }
        if self.tx.as_ref().is_some_and(|tx| tx.send(args).is_ok()) {
            self.pending += 1;
        }
    }

    /// Nothing sent is still running.
    fn settled(&mut self) -> bool {
        if let Some(done) = &self.done {
            while done.try_recv().is_ok() {
                self.pending = self.pending.saturating_sub(1);
            }
        }
        self.pending == 0
    }
}

/// A shell started while the system is still deep (the last one stopped
/// there): everything brought back.
pub fn recover() {
    if Path::new(STATE).exists() && Path::new(crate::power::SCRIPT).exists() {
        std::process::Command::new(crate::power::SCRIPT).args(["deep", "off"]).status().ok();
    }
}

// ------------------------------------------------------------ the state ---

struct Asleep {
    /// Seconds asleep so far, from the shell's ticks, and when the last
    /// was (the time from it to the waking key counts too).
    slept: f32,
    last_tick: Instant,
    meter: Meter,
    /// Seconds in music standby before this (deep standby's own time
    /// starts when it deepens): the screen's idle time, all of it.
    listened: f32,
}

/// Which standby: deep, or music standby (the sound left playing).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    #[default]
    Deep,
    /// Music playing; Wi-Fi kept while what plays next needs the network.
    Music { wifi: bool },
}

/// What music standby does next, after a minute's look.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MusicNext {
    /// Playing on, as it is.
    Stay,
    /// What plays next is all here now: Wi-Fi off too.
    WifiOff,
    /// Back to plain standby (the screen stays dark): the music needs the
    /// network that music standby took away, or it stopped before deep
    /// standby's wait was over.
    Leave,
    /// The music stopped, and deep standby's wait is over: deep, in place.
    Deepen,
}

/// Waking: what the island says, if anything, and what to undo first.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Woke {
    /// Deep standby's figures (none from music standby).
    pub slept: Option<Slept>,
    /// The red light was reminding: put it out first.
    pub reminding: bool,
    /// Seconds the screen was dark in here, for standby's idle time.
    pub seconds: f32,
}

/// Where deep standby stands, ticked by the shell.
#[derive(Default)]
pub struct Deep {
    asleep: Option<Asleep>,
    kind: Kind,
    /// Woken, and waiting for catalyst-power to bring the system back
    /// before the last steps are undone.
    waking: bool,
    worker: Worker,
    reminding: bool,
    /// Seconds until it next asks whether anything stops it going deep.
    look: f32,
    /// Seconds the screen has been dark with music playing, and until the
    /// next look at whether music standby can begin.
    dark_with_music: f32,
    music_look: f32,
    /// In music standby: seconds since the music stopped.
    stopped: f32,
    /// The steps taken, in order, for the tests.
    #[cfg(test)]
    pub log: Vec<(Step, bool)>,
}

impl Deep {
    pub fn new() -> Deep {
        Deep::default()
    }

    /// In deep standby.
    pub fn on(&self) -> bool {
        self.asleep.is_some()
    }

    /// Woken, the last steps still to undo.
    pub fn waking(&self) -> bool {
        self.waking
    }

    /// Whether to see if it can go deep now: the screen has been off (and
    /// no key) for `idle` seconds of the `after` minutes chosen. Asked
    /// every tick; true at most every ten seconds.
    pub fn due(&mut self, idle: f32, after: u32, dt: f32) -> bool {
        if self.on() || self.waking || after == 0 || idle < after as f32 * 60.0 {
            self.look = 0.0;
            return false;
        }
        self.look -= dt;
        if self.look > 0.0 {
            return false;
        }
        self.look = LOOK_EVERY;
        true
    }

    /// Going deep: the time starts, with the gauge's reading as it does.
    pub fn begin(&mut self, r: Option<&Reading>) {
        self.asleep = Some(Asleep { slept: 0.0, last_tick: Instant::now(), meter: Meter::begin(r), listened: 0.0 });
        self.kind = Kind::Deep;
        self.reminding = false;
    }

    /// In music standby (a kind of deep standby: `on` is true in it too).
    pub fn music(&self) -> bool {
        self.on() && matches!(self.kind, Kind::Music { .. })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Whether music standby should begin: asked every tick with whether
    /// the screen is dark and music playing; true once both have held for
    /// `MUSIC_AFTER`.
    pub fn music_due(&mut self, dark: bool, playing: bool, dt: f32) -> bool {
        if self.on() || self.waking || !dark || !playing {
            self.dark_with_music = 0.0;
            self.music_look = 0.0;
            return false;
        }
        self.dark_with_music += dt;
        if self.dark_with_music < MUSIC_AFTER {
            return false;
        }
        // Then at most every ten seconds (something may be stopping it).
        self.music_look -= dt;
        if self.music_look > 0.0 {
            return false;
        }
        self.music_look = LOOK_EVERY;
        true
    }

    /// Into music standby: Wi-Fi kept if what plays next needs it.
    pub fn begin_music(&mut self, wifi: bool) {
        self.asleep = Some(Asleep { slept: 0.0, last_tick: Instant::now(), meter: Meter::default(), listened: 0.0 });
        self.kind = Kind::Music { wifi };
        self.stopped = 0.0;
        self.reminding = false;
    }

    /// A minute's look in music standby: whether the music plays, whether
    /// it needs the network, how long the screen takes to go off (the
    /// wait once it stops), and whether deep standby could begin now.
    pub fn music_next(&mut self, playing: bool, needs_network: bool, linger: f32, deep_ok: bool, dt: f32) -> MusicNext {
        let Kind::Music { wifi } = self.kind else { return MusicNext::Stay };
        if !self.on() {
            return MusicNext::Stay;
        }
        if playing {
            self.stopped = 0.0;
            return match (wifi, needs_network) {
                // Read ahead or from the library: the radio can rest.
                (true, false) => {
                    self.kind = Kind::Music { wifi: false };
                    MusicNext::WifiOff
                }
                // Streaming again with Wi-Fi off: back, and in again with it.
                (false, true) => MusicNext::Leave,
                _ => MusicNext::Stay,
            };
        }
        self.stopped += dt;
        if self.stopped < linger {
            MusicNext::Stay
        } else if deep_ok {
            MusicNext::Deepen
        } else {
            MusicNext::Leave
        }
    }

    /// Music standby becomes deep standby, in place: deep's own time (and
    /// what it draws) is counted from now.
    pub fn deepen(&mut self, r: Option<&Reading>) {
        if let Some(a) = &mut self.asleep {
            a.listened += a.slept + a.last_tick.elapsed().as_secs_f32();
            a.slept = 0.0;
            a.last_tick = Instant::now();
            a.meter = Meter::begin(r);
        }
        self.kind = Kind::Deep;
        self.reminding = false;
    }

    /// Seconds the screen has been dark in here so far.
    pub fn dark_for(&self) -> f32 {
        self.asleep.as_ref().map_or(0.0, |a| a.listened + a.slept + a.last_tick.elapsed().as_secs_f32())
    }

    /// Time passing in deep standby, with the reading taken meanwhile.
    pub fn sample(&mut self, r: Option<&Reading>, dt: f32) {
        let Some(a) = &mut self.asleep else { return };
        a.slept += dt;
        a.last_tick = Instant::now();
        if let Some(r) = r {
            a.meter.sample(r, dt);
        }
    }

    /// The red light's reminder: Some(on or off) when it should change.
    /// On battery, after twelve hours or at 10%; never on the charger, and
    /// not while music plays (whoever listens knows it is on).
    pub fn reminder(&mut self, r: Option<&Reading>) -> Option<bool> {
        if self.kind != Kind::Deep {
            return None;
        }
        let a = self.asleep.as_ref()?;
        let on_battery = r.is_none_or(|r| r.flow.on_battery());
        let low = r.is_some_and(|r| r.flow.on_battery() && r.percent <= REMIND_AT);
        let want = on_battery && (a.slept >= REMIND_AFTER || low);
        (want != self.reminding).then(|| {
            self.reminding = want;
            want
        })
    }

    /// Woken (or, from music standby, back to plain standby): how long it
    /// slept and what it drew (kept for the Battery page; deep standby
    /// only), and whether the red light was reminding (to be put out first).
    pub fn end(&mut self, r: Option<&Reading>) -> Option<Woke> {
        let a = self.asleep.take()?;
        let slept = a.slept + a.last_tick.elapsed().as_secs_f32();
        let record = (self.kind == Kind::Deep).then(|| Slept {
            woke: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64),
            seconds: slept as u64,
            watts: a.meter.watts(r, slept),
            charging: a.meter.charged() || r.is_some_and(|r| !r.flow.on_battery()),
        });
        if let Some(record) = &record {
            keep(record);
        }
        self.waking = true;
        self.dark_with_music = 0.0;
        Some(Woke { slept: record, reminding: std::mem::take(&mut self.reminding), seconds: a.listened + slept })
    }

    /// catalyst-power's part, on its thread: what this kind keeps on.
    fn system(&mut self, deep: bool) {
        let mut args = match (deep, self.kind) {
            (false, _) => vec!["deep", "off"],
            (true, Kind::Deep) => vec!["deep", "on"],
            (true, Kind::Music { wifi: true }) => vec!["deep", "on", "keep-wifi", "keep-radios"],
            (true, Kind::Music { wifi: false }) => vec!["deep", "on", "keep-radios"],
        };
        // USB headphones playing hang off the hub: the USB host stays.
        if deep && matches!(self.kind, Kind::Music { .. }) && crate::sound::route().usb_playing().is_some() {
            args.push("keep-usb");
        }
        self.worker.send(args);
    }

    /// What was asked of catalyst-power, in order (the tests').
    #[cfg(test)]
    pub fn asked(&self) -> &[Vec<&'static str>] {
        &self.worker.asked
    }

    /// Waking, and catalyst-power is done: the last steps can be undone.
    pub fn ready_to_finish(&mut self) -> bool {
        self.waking && self.worker.settled()
    }

    pub fn woke(&mut self) {
        self.waking = false;
    }
}

// ------------------------------------------------------- Settings' page ---

/// Settings › Battery › Deep standby: what it does and still draws (the
/// last time it slept, once it has), and the choice of when.
pub struct DeepPage {
    last: Option<Slept>,
}

impl DeepPage {
    pub fn new() -> DeepPage {
        DeepPage { last: last() }
    }
}

impl Default for DeepPage {
    fn default() -> Self {
        Self::new()
    }
}

/// Its one row, at the foot above the keys.
fn row(g: &Gfx) -> Rect {
    let row_h = g.row_h();
    Rect::new(PAD, H - PAD - g.hint_h() - row_h, W - 2.0 * PAD, row_h)
}

impl Screen for DeepPage {
    fn title(&self) -> &str {
        // "Deep standby" does not fit beside the island at the title's size.
        "Deep sleep"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("deep standby", &[("← →", "Sooner or later"), ("enter", "Later")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right | Key::Enter => {
                let c = &mut cx.settings.battery;
                c.deep_after = step_after(c.deep_after, if key == Key::Left { -1 } else { 1 });
                crate::save_settings(cx.settings);
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let after = g.battery.deep_after;
        // What it does and draws, above the row.
        let line_h = ty.caption.size + 4.0;
        let lines = crate::help::wrap(g, &words(after, self.last), ty.caption, W - 2.0 * PAD);
        for (i, line) in lines.iter().enumerate() {
            g.text_alpha(PAD, top + 2.0 + line_h * (i as f32 + 0.5), line, ty.caption, t.dim, 1.0);
        }
        g.row(row(g), "Deep standby", &after_value(after), if after > 0 { t.ink } else { t.dim }, true);
        g.hint("←→ change · esc back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::Flow;

    fn on_battery(percent: u32, milliamps: f32, volts: f32, charge_mah: f32) -> Reading {
        Reading { percent, flow: Flow::Discharging, milliamps: Some(milliamps), volts: Some(volts), charge_mah: Some(charge_mah), ..Reading::default() }
    }

    #[test]
    fn the_choices_go_round_and_read_back() {
        assert_eq!(AFTER_CHOICES.map(after_value), ["after 10 min", "after 30 min", "after 1 hour", "never"].map(String::from));
        assert_eq!(step_after(30, 1), 60);
        assert_eq!(step_after(0, 1), 10, "round from never");
        assert_eq!(step_after(10, -1), 0);
        for (text, minutes) in [("10", 10), ("60", 60), ("0", 0), (" 30 ", 30), ("45", 30), ("soon", 30), ("", 30)] {
            assert_eq!(parse_after(text), minutes, "{text:?}");
        }
    }

    #[test]
    fn it_goes_deep_after_the_wait_and_asks_only_now_and_then() {
        let mut d = Deep::new();
        assert!(!d.due(29.0 * 60.0, 30, 1.0));
        assert!(d.due(30.0 * 60.0, 30, 1.0), "half an hour with the screen off");
        assert!(!d.due(30.0 * 60.0 + 1.0, 30, 1.0), "asked again a second later: not yet");
        assert!(d.due(30.0 * 60.0 + 11.0, 30, 10.0), "ten seconds on");
        assert!(!d.due(1e9, 0, 1.0), "never");
        d.begin(None);
        assert!(!d.due(1e9, 10, 100.0), "deep already");
    }

    #[test]
    fn what_stops_it_going_deep() {
        assert_eq!(blocker(Busy::default()), None);
        assert_eq!(blocker(Busy { robot: true, ..Busy::default() }), Some("a robot is linked"));
        assert_eq!(blocker(Busy { computer: true, ..Busy::default() }), Some("a computer has it over USB"));
        assert_eq!(blocker(Busy { ssh: true, ..Busy::default() }), Some("someone is logged in over SSH"));
        assert_eq!(blocker(Busy { update: true, ..Busy::default() }), Some("an update is under way"));
        assert_eq!(blocker(Busy { timer: true, ..Busy::default() }), Some("a timer is running"));
        assert_eq!(blocker(Busy { saving: true, ..Busy::default() }), Some("something is being saved"));
        assert_eq!(blocker(Busy { downloads: true, ..Busy::default() }), Some("songs are downloading"));
        assert_eq!(blocker(Busy { charger: true, ..Busy::default() }), Some("it is on the charger"));
        assert_eq!(blocker(Busy { music: true, ..Busy::default() }), Some("music is playing"));
    }

    #[test]
    fn waking_undoes_the_steps_in_the_reverse_order() {
        let order: Vec<Step> = waking_first().chain(waking_after()).collect();
        let mut back = STEPS.to_vec();
        back.reverse();
        assert_eq!(order, back);
        assert_eq!(waking_first().last(), Some(Step::System), "the system comes back before the rest");
    }

    #[test]
    fn music_standby_comes_half_a_minute_into_the_dark_and_asks_only_now_and_then() {
        let mut d = Deep::new();
        assert!(!d.music_due(true, true, 29.0));
        assert!(d.music_due(true, true, 1.0), "half a minute dark with music");
        assert!(!d.music_due(true, true, 1.0), "asked again a second later: not yet");
        assert!(d.music_due(true, true, 10.0), "ten seconds on");
        assert!(!d.music_due(false, true, 1.0), "the screen lit: the count starts again");
        assert!(!d.music_due(true, true, 20.0));
        assert!(!d.music_due(true, false, 20.0), "paused: not while nothing plays");
        d.begin_music(true);
        assert!(d.on() && d.music());
        assert!(!d.music_due(true, true, 60.0), "in it already");
        assert!(!d.due(1e9, 10, 100.0), "and not deep standby");
    }

    #[test]
    fn music_standby_keeps_what_plays_and_moves_on_once_the_music_stops() {
        crate::shell::tests::sandbox();
        let mut d = Deep::new();
        d.begin_music(true);
        d.system(true);
        assert_eq!(d.asked(), [vec!["deep", "on", "keep-wifi", "keep-radios"]]);
        // Streaming: as it is. Then all read ahead: Wi-Fi off.
        assert_eq!(d.music_next(true, true, 60.0, true, 60.0), MusicNext::Stay);
        assert_eq!(d.music_next(true, false, 60.0, true, 60.0), MusicNext::WifiOff);
        assert_eq!(d.kind(), Kind::Music { wifi: false });
        d.system(true);
        assert_eq!(d.asked()[1], vec!["deep", "on", "keep-radios"]);
        // Streaming again with Wi-Fi off: back out (and in again, with it).
        assert_eq!(d.music_next(true, true, 60.0, true, 60.0), MusicNext::Leave);
        // Stopped: a while (as long as the screen takes to go off), then on.
        let mut d = Deep::new();
        d.begin_music(true);
        d.sample(None, 1800.0);
        assert_eq!(d.music_next(false, true, 60.0, true, 30.0), MusicNext::Stay, "paused for a moment");
        assert_eq!(d.music_next(true, true, 60.0, true, 60.0), MusicNext::Stay, "and playing again");
        assert_eq!(d.music_next(false, true, 60.0, false, 60.0), MusicNext::Leave, "stopped, deep standby's wait not over");
        let mut d = Deep::new();
        d.begin_music(true);
        d.sample(None, 1800.0);
        assert_eq!(d.music_next(false, true, 60.0, true, 60.0), MusicNext::Deepen, "stopped, and deep standby's time");
        d.deepen(None);
        assert_eq!(d.kind(), Kind::Deep);
        assert!(d.dark_for() >= 1800.0, "the music's time counts as the screen's dark time");
        d.system(true);
        assert_eq!(d.asked().last().unwrap(), &vec!["deep", "on"], "the rest: catalyst-power takes only what is left");
        d.sample(None, 600.0);
        let woke = d.end(None).unwrap();
        assert_eq!(woke.slept.map(|s| s.seconds), Some(600), "deep standby's own ten minutes");
        assert!(woke.seconds >= 2400.0);
        // From music standby: nothing to say, the time given back.
        let mut d = Deep::new();
        d.begin_music(false);
        d.sample(None, 900.0);
        assert_eq!(d.reminder(Some(&on_battery(5, 100.0, 3.5, 50.0))), None, "no reminder while music plays");
        let woke = d.end(None).unwrap();
        assert_eq!(woke.slept, None);
        assert!((woke.seconds - 900.0).abs() < 1.0);
        d.system(false);
        assert_eq!(d.asked().last().unwrap(), &vec!["deep", "off"]);
    }

    #[test]
    fn music_standby_keeps_the_usb_host_while_usb_headphones_play() {
        crate::shell::tests::sandbox();
        crate::sound::demo_plug(Some(crate::sound::demo_usb()));
        let mut d = Deep::new();
        d.begin_music(false);
        d.system(true);
        assert_eq!(d.asked(), [vec!["deep", "on", "keep-radios", "keep-usb"]]);
        // The music stopped: deep standby takes the USB host too.
        d.deepen(None);
        d.system(true);
        assert_eq!(d.asked().last().unwrap(), &vec!["deep", "on"]);
        crate::sound::demo_plug(None);
        let mut d = Deep::new();
        d.begin_music(false);
        d.system(true);
        assert_eq!(d.asked(), [vec!["deep", "on", "keep-radios"]], "the speaker playing: the USB host goes");
    }

    #[test]
    fn the_draw_is_the_readings_weighted_by_time() {
        let start = on_battery(80, 200.0, 3.9, 1400.0);
        let mut m = Meter::begin(Some(&start));
        // Ten minutes at 0.5 W, then ten at 1 W: 0.75 W on average.
        for _ in 0..10 {
            m.sample(&on_battery(80, 128.2, 3.9, 1390.0), 60.0);
        }
        for _ in 0..10 {
            m.sample(&on_battery(79, 256.4, 3.9, 1380.0), 60.0);
        }
        let w = m.watts(Some(&on_battery(79, 200.0, 3.9, 1380.0)), 1200.0).unwrap();
        assert!((w - 0.75).abs() < 0.01, "{w}");
    }

    #[test]
    fn with_too_few_readings_the_charge_lost_tells_it() {
        let mut m = Meter::begin(Some(&on_battery(80, 200.0, 3.8, 1500.0)));
        // An hour, read twice: 180 mAh lost at 3.8 V and 3.7 V is 0.675 W.
        m.sample(&on_battery(75, 0.0, 3.75, 1410.0), 60.0);
        let w = m.watts(Some(&on_battery(70, 170.0, 3.7, 1320.0)), 3600.0).unwrap();
        assert!((w - 0.675).abs() < 0.001, "{w}");
        // Too short, or too little lost, to tell.
        let m = Meter::begin(Some(&on_battery(80, 200.0, 3.8, 1500.0)));
        assert_eq!(m.watts(Some(&on_battery(80, 200.0, 3.8, 1499.0)), 3600.0), None);
        assert_eq!(m.watts(Some(&on_battery(80, 200.0, 3.8, 1480.0)), 300.0), None);
        assert_eq!(m.watts(None, 3600.0), None);
    }

    #[test]
    fn on_the_charger_no_draw_is_given() {
        let mut m = Meter::begin(Some(&on_battery(50, 200.0, 3.8, 900.0)));
        for _ in 0..30 {
            m.sample(&on_battery(50, 150.0, 3.8, 890.0), 60.0);
        }
        m.sample(&Reading { flow: Flow::Charging, ..on_battery(51, 500.0, 4.0, 920.0) }, 60.0);
        assert!(m.charged());
        assert_eq!(m.watts(Some(&on_battery(51, 150.0, 3.8, 920.0)), 1860.0), None);
    }

    #[test]
    fn the_red_light_reminds_when_long_or_low_on_battery() {
        crate::shell::tests::sandbox();
        let mut d = Deep::new();
        d.begin(None);
        let fine = on_battery(60, 150.0, 3.8, 1000.0);
        d.sample(Some(&fine), 11.0 * 3600.0);
        assert_eq!(d.reminder(Some(&fine)), None, "eleven hours: dark");
        d.sample(Some(&fine), 3600.0);
        assert_eq!(d.reminder(Some(&fine)), Some(true), "twelve: once a minute");
        assert_eq!(d.reminder(Some(&fine)), None, "already blinking");
        let charging = Reading { flow: Flow::Charging, ..fine.clone() };
        assert_eq!(d.reminder(Some(&charging)), Some(false), "plugged in: dark again");
        let mut d = Deep::new();
        d.begin(None);
        d.sample(Some(&fine), 60.0);
        assert_eq!(d.reminder(Some(&on_battery(11, 150.0, 3.6, 190.0))), None);
        assert_eq!(d.reminder(Some(&on_battery(10, 150.0, 3.6, 170.0))), Some(true), "at 10%");
        let woke = d.end(None).unwrap();
        assert!(woke.reminding, "waking puts it out first");
    }

    #[test]
    fn a_sleep_is_kept_and_said() {
        crate::shell::tests::sandbox();
        let s = Slept { woke: 1_700_000_000, seconds: 7 * 3600 + 12 * 60 + 30, watts: Some(0.684), charging: false };
        assert_eq!(Slept::parse(&s.line()), Some(Slept { watts: Some(0.684), ..s }));
        assert_eq!(s.island(), "slept 7 h 12 m · ~0.68 W");
        assert_eq!(s.summary(), "7 h 12 m · 0.68 W");
        let short = Slept { seconds: 20, watts: None, ..s };
        assert_eq!(short.island(), "slept 1 m");
        assert_eq!(Slept::parse(&short.line()), Some(short));
        let charging = Slept { charging: true, watts: None, ..s };
        assert_eq!(charging.island(), "slept 7 h 12 m · charging");
        for bad in ["", "1 2", "1 2 x d", "1 2 -1 d", "a 2 - d"] {
            assert_eq!(Slept::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn settings_says_what_it_draws_honestly() {
        let estimate = words(30, None).replace('\u{a0}', " ");
        assert!(estimate.starts_with("After 30 min with no key, Wi-Fi, USB and the lights go off"), "{estimate}");
        assert!(estimate.contains("Estimated 0.6–1 W: a night from full.") && estimate.ends_with("For days, slide the switch off."), "{estimate}");
        let measured = words(60, Some(Slept { woke: 0, seconds: 8 * 3600, watts: Some(0.65), charging: false })).replace('\u{a0}', " ");
        assert!(measured.contains("After 1 hour") && measured.contains("Last time 0.65 W: about 10 h from full."), "{measured}");
        assert!(words(0, None).starts_with("Never:"));
    }

    #[test]
    fn numbers_keep_to_their_units_when_the_words_wrap() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let ty = g.ty;
        let measured = Some(Slept { woke: 0, seconds: 3600, watts: Some(0.68), charging: false });
        for width in (120..=300).step_by(7) {
            for line in crate::help::wrap(&mut g, &words(30, None), ty.caption, width as f32).into_iter().chain(crate::help::wrap(&mut g, &words(30, measured), ty.caption, width as f32)) {
                assert!(!line.contains('\u{a0}'), "drawn as a plain space: {line:?}");
                for cut in ["0.6–1", "0.68", " 30", " 10"] {
                    assert!(!line.ends_with(cut), "{line:?} at {width}");
                }
            }
        }
    }

    #[test]
    fn its_page_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let ty = g.ty;
        // Its title beside the island: no wider than Settings' own "Appearance".
        assert!(g.measure(DeepPage::new().title(), ty.title) <= g.measure("Appearance", ty.title) + 9.0);
        let longest = Slept { woke: 0, seconds: 23 * 3600 + 59 * 60, watts: Some(0.888), charging: false };
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            // What the island says on waking, whole (it holds 250 px of words).
            for s in [longest, Slept { charging: true, ..longest }] {
                assert!(g.measure(&s.island(), ty.caption) <= 250.0, "{} at {size:?}", s.island());
            }
            let r = row(&g);
            let slow = Slept { watts: Some(0.05), ..longest };
            for (after, last) in [(30, None), (60, Some(longest)), (60, Some(slow)), (0, Some(longest)), (10, Some(Slept { charging: true, ..longest }))] {
                let lines = crate::help::wrap(&mut g, &words(after, last), ty.caption, W - 2.0 * PAD).len() as f32;
                assert!(g.top() + 2.0 + lines * (ty.caption.size + 4.0) <= r.y - 4.0, "{lines} lines at {size:?}: {}", words(after, last));
                assert!(g.measure("Deep standby", ty.body) + 30.0 + g.measure(&after_value(after), ty.label) <= r.w);
            }
            assert!(g.measure("←→ change · esc back", ty.caption) <= W - 2.0 * PAD);
        }
    }

    #[test]
    fn parked_work_waits_for_the_gate_and_goes_on_after() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        static GATE: Gate = Gate::new();
        let rounds = Arc::new(AtomicU32::new(0));
        GATE.set(true);
        let counted = rounds.clone();
        let worker = std::thread::spawn(move || {
            for _ in 0..3 {
                GATE.rest();
                counted.fetch_add(1, Ordering::SeqCst);
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(rounds.load(Ordering::SeqCst), 0, "parked while deep");
        GATE.set(false);
        worker.join().unwrap();
        assert_eq!(rounds.load(Ordering::SeqCst), 3, "and on again after");
    }
}
