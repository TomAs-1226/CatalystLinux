//! The processor's pace: as little of it as can be at rest and for work
//! nobody is waiting on, and all of it from the first frame of anything that
//! moves.
//!
//! **The boost.** The CM0's four cores share one clock (one cpufreq policy),
//! so the only question is when that clock runs fast, and the shell knows
//! the answer exactly: from a key going down, or the first frame of a motion
//! on a lit screen, until the motion settles. For that long the clock's
//! floor (`scaling_min_freq`) is raised to its ceiling, so every part of a
//! frame's path runs at speed: the drawing thread, the kernel's DRM worker
//! that sends the changed rows, and the SPI transfer (the firmware's turbo
//! raises the core clock the SPI divides along with the ARM's). It comes
//! down `TAIL` after the last frame, time for that frame's rows to reach the
//! panel. A motion that runs on by itself for `CAP` with no key and nothing
//! new begun lets it go, and the governor judges from there. It is never
//! raised with the screen dark, nor by the motion of a dimming screen or of
//! StandBy's faces (a key there still raises it). The battery saver's
//! ceiling stays the ceiling: the kernel holds the floor under it.
//!
//! A thread of its own writes sysfs, so the drawing thread never waits on
//! cpufreq's lock or the firmware's mailbox. The write only asks; a kernel
//! worker switches the clock while the first frame is already being drawn.
//! `/run/catalyst-noboost` (or `CATALYST_BOOST=off`) switches it off, to
//! compare.
//!
//! Why not the governor alone: schedutil sets the clock from the drawing
//! thread's utilisation (PELT), which after a rest is near zero and takes
//! about 74 ms (a 32 ms half-life) to reach the 80% at which it asks for the
//! top; and a thread busy 13 ms of a 25 ms beat at 600 MHz never asks for
//! more than 600 (1.25 × 0.52 × 0.6 is 39% of the top). The rate limit
//! (1.5 ms here, from cpufreq-dt's default latency of 1 ms) is not what holds
//! it back. Why not uclamp.min on the drawing thread: the kernel has no
//! CONFIG_UCLAMP_TASK (a rebuild, with M5Stack's modules rebuilt against
//! it); a task's clamp does not cover the DRM worker or the SPI; and the
//! clamp is dropped whenever the thread sleeps between frames, so the clock
//! would go up and down with every frame.
//!
//! **Classes.** Work off the drawing thread runs in one of a few classes
//! (`Class`): SCHED_IDLE for what nobody waits on and shares no lock with
//! the shell, SCHED_BATCH (which never preempts a frame on waking) for the
//! rest, at nice 19 or 10, with timer slack so its wakeups fold into
//! others'. The sound is not here: catalyst-play runs at nice −10 under the
//! ordinary scheduler (music/polite.rs), which with 341 ms of ALSA buffer is
//! enough; real time would also have schedutil run the clock at the top
//! whenever the decoder ran.
//!
//! **Governors.** Awake, schedutil (standby.rs), its rate limit as the
//! kernel sets it. Power saving (and the battery saver), conservative with
//! `ignore_nice_load`: work at nice > 0 never climbs the clock there. The
//! screen dark, powersave, at 600 MHz. Cores cannot be parked: the CM0's
//! start its cores by spin table, which has no way to stop one
//! (`op_cpu_disable` in arch/arm64/kernel/smp.c returns EOPNOTSUPP), and an
//! idle core already waits in WFI, there being no deeper idle state.

use std::time::{Duration, Instant};

// ------------------------------------------------------------- classes ---

/// How work off the drawing thread is run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Class {
    /// Nobody waits on it, and it shares no lock with the shell: the
    /// system update, the on-device model's download. SCHED_IDLE (it runs
    /// only where no other thread wants the processor, and gives way at
    /// once), nice 19 (counted as nice time, which conservative leaves out),
    /// the card's idle class, 50 ms of timer slack.
    Idle,
    /// The shell's own background threads, which take locks the drawing
    /// thread takes too (Spotlight's index, catalogue installs, the robot's
    /// logs): SCHED_BATCH at nice 19, so they never preempt a frame on
    /// waking yet are never starved while holding a lock; 50 ms of slack.
    Background,
    /// Work someone may be waiting on, off the drawing thread: Music's
    /// fetching and lookups, the offline assistant reading the logs, the
    /// on-device model. SCHED_BATCH at this nice (10: the frames kept their
    /// pace with llama-server's threads at it), 10 ms of slack.
    Batch(i32),
    /// A thread that looks in on something now and then (the event
    /// schedule, a page's poll): its priority as it was, 20 ms of slack.
    Poller,
}

const MS: u64 = 1_000_000;

/// What a class sets: the scheduling policy, the nice, the timer slack (ns)
/// and whether the card serves it last.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Parts {
    policy: Option<i32>,
    nice: Option<i32>,
    slack_ns: u64,
    idle_io: bool,
}

/// Linux's SCHED_BATCH and SCHED_IDLE.
const SCHED_BATCH: i32 = 3;
const SCHED_IDLE: i32 = 5;

impl Class {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn parts(self) -> Parts {
        match self {
            Class::Idle => Parts { policy: Some(SCHED_IDLE), nice: Some(19), slack_ns: 50 * MS, idle_io: true },
            Class::Background => Parts { policy: Some(SCHED_BATCH), nice: Some(19), slack_ns: 50 * MS, idle_io: false },
            Class::Batch(nice) => Parts { policy: Some(SCHED_BATCH), nice: Some(nice.clamp(-20, 19)), slack_ns: 10 * MS, idle_io: false },
            Class::Poller => Parts { policy: None, nice: None, slack_ns: 20 * MS, idle_io: false },
        }
    }
}

/// Run the calling thread as `class` from now on; what it starts inherits
/// it. Only system calls, nothing allocated: safe between fork and exec.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn enter(class: Class) {
    // SAFETY: plain system calls on the calling thread's own settings.
    #[cfg(target_os = "linux")]
    unsafe {
        enter_here(class)
    }
}

#[cfg(target_os = "linux")]
unsafe fn enter_here(class: Class) {
    let p = class.parts();
    // Linux keeps each of these for each thread; 0 is the calling one.
    if let Some(nice) = p.nice {
        libc::setpriority(libc::PRIO_PROCESS, 0, nice);
    }
    if let Some(policy) = p.policy {
        // The system call itself: musl's sched_setscheduler only says ENOSYS.
        // The kernel's sched_param is one int, the priority (0 for these
        // policies). It keeps the nice just set.
        let priority: libc::c_int = 0;
        libc::syscall(libc::SYS_sched_setscheduler, 0, policy, &priority as *const libc::c_int);
    }
    libc::prctl(libc::PR_SET_TIMERSLACK, p.slack_ns as libc::c_ulong);
    if p.idle_io {
        // IOPRIO_WHO_PROCESS (1), this thread (0), IOPRIO_CLASS_IDLE (3) << 13.
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
}

/// Start `cmd` as `class`: set in the child, before it runs anything.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn command(cmd: &mut std::process::Command, class: Class) -> &mut std::process::Command {
    #[cfg(target_os = "linux")]
    // SAFETY: between fork and exec only system calls on the child itself.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            enter_here(class);
            Ok(())
        });
    }
    cmd
}

// ------------------------------------------------------------- the boost ---

/// After the last frame of a motion, how long the clock stays up: that
/// frame's rows reaching the panel (17 to 21 ms for a whole one, after
/// waiting up to a refresh, 16 ms, for the scan).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const TAIL: Duration = Duration::from_millis(50);
/// A motion running on with no key and nothing new begun lets the boost go
/// after this long: no transition lasts it (an app opening rests in 35
/// frames, under a second), so what still moves is the screen's own life,
/// for the governor to judge.
pub const CAP: Duration = Duration::from_secs(4);

/// Where the shell is, for the boost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// Lit and in use: keys and motion raise it.
    Lit,
    /// Lit but not in use: dimming toward off, going out, or StandBy's
    /// faces on the charger. A key raises it; their own motion does not.
    Quiet,
    /// Dark (standby, music standby, deep standby): never.
    Dark,
}

/// When the clock should be boosted: the decisions alone, made on the
/// drawing thread with no system call (`Booster` carries them out).
#[derive(Debug, Default)]
pub struct Boost {
    on: bool,
    /// The last key, or the start of the motion in flight.
    since: Option<Instant>,
    moving: bool,
}

impl Boost {
    /// A key, before it is handled. `Some(true)` if the boost comes on now.
    pub fn key(&mut self, now: Instant, place: Place) -> Option<bool> {
        if place == Place::Dark {
            return None;
        }
        self.since = Some(now);
        (!self.on).then(|| {
            self.on = true;
            true
        })
    }

    /// The end of a turn of the loop (its frame drawn, if any): whether
    /// anything still moves, and where. The change to make, if any.
    pub fn after(&mut self, now: Instant, moving: bool, place: Place) -> Option<bool> {
        if moving && !self.moving && place == Place::Lit {
            self.since = Some(now);
        }
        self.moving = moving;
        let fresh = self.since.is_some_and(|since| now.saturating_duration_since(since) < CAP);
        let want = place != Place::Dark && moving && fresh;
        (want != self.on).then(|| {
            self.on = want;
            want
        })
    }

    pub fn on(&self) -> bool {
        self.on
    }
}

/// The boost carried out: the decisions (`Boost`), and on the handheld the
/// thread that moves the clock's floor for them.
pub struct Booster {
    decide: Boost,
    #[cfg(target_os = "linux")]
    shared: Option<std::sync::Arc<Shared>>,
}

#[cfg(target_os = "linux")]
struct Shared {
    want: std::sync::Mutex<bool>,
    bell: std::sync::Condvar,
    floors: Vec<Floor>,
}

/// One cpufreq policy's floor: where it belongs, and the top.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Floor {
    file: std::path::PathBuf,
    least: String,
    top: String,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Floor {
    /// Every cpufreq policy (the CM0 has one, for its four cores).
    fn all() -> Vec<Floor> {
        let mut floors: Vec<Floor> = std::fs::read_dir("/sys/devices/system/cpu/cpufreq")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("policy"))
            .filter_map(|e| {
                let dir = e.path();
                let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
                Some(Floor { least: read("cpuinfo_min_freq")?, top: read("cpuinfo_max_freq")?, file: dir.join("scaling_min_freq") })
            })
            .collect();
        floors.sort_by(|a, b| a.file.cmp(&b.file));
        floors
    }

    /// Up to the top: the kernel holds it under the ceiling (the saver's).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn raise(&self) {
        std::fs::write(&self.file, &self.top).ok();
    }

    fn lower(&self) {
        std::fs::write(&self.file, &self.least).ok();
    }
}

/// The boost switched off from outside, to compare with and without it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn switched_off() -> bool {
    std::path::Path::new("/run/catalyst-noboost").exists() || std::env::var("CATALYST_BOOST").is_ok_and(|v| v == "off")
}

impl Booster {
    /// On the handheld, the thread that moves the floor (and the floor put
    /// back where it belongs, should a shell before this one have stopped
    /// with it raised). Elsewhere, the same decisions with nothing to move.
    pub fn start() -> Booster {
        #[cfg(target_os = "linux")]
        {
            let floors = Floor::all();
            for floor in &floors {
                floor.lower();
            }
            let shared = (!floors.is_empty()).then(|| std::sync::Arc::new(Shared { want: std::sync::Mutex::new(false), bell: std::sync::Condvar::new(), floors }));
            let started = shared.as_ref().is_some_and(|s| {
                let s = s.clone();
                std::thread::Builder::new().name("boost".into()).stack_size(64 * 1024).spawn(move || carry_out(&s)).is_ok()
            });
            Booster { decide: Boost::default(), shared: shared.filter(|_| started) }
        }
        #[cfg(not(target_os = "linux"))]
        Booster { decide: Boost::default() }
    }

    /// A key, before it is handled (and again once handled, should it have
    /// woken the screen).
    pub fn key(&mut self, now: Instant, place: Place) {
        if let Some(on) = self.decide.key(now, place) {
            self.send(on);
        }
    }

    /// The end of a turn of the loop.
    pub fn after(&mut self, now: Instant, moving: bool, place: Place) {
        if let Some(on) = self.decide.after(now, moving, place) {
            self.send(on);
        }
    }

    /// Let go now (an external app takes the panel and the keys).
    pub fn release(&mut self) {
        if self.decide.on {
            self.decide.on = false;
            self.decide.since = None;
            self.send(false);
        }
    }

    pub fn on(&self) -> bool {
        self.decide.on()
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn send(&self, on: bool) {
        #[cfg(target_os = "linux")]
        if let Some(s) = &self.shared {
            *s.want.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = on;
            s.bell.notify_one();
        }
    }
}

impl Drop for Booster {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(s) = &self.shared {
            for floor in &s.floors {
                floor.lower();
            }
        }
    }
}

/// The boost's thread: up at once when wanted, down `TAIL` after it no
/// longer is (unless wanted again meanwhile). It holds no lock while it
/// writes, so the drawing thread never waits on it.
#[cfg(target_os = "linux")]
fn carry_out(s: &Shared) {
    use std::sync::PoisonError;
    // SAFETY: this thread's own priority. Ahead of the shell's other
    // threads: it only writes a number, and a frame is waiting on it.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, -10);
    }
    let mut raised = false;
    let mut want = s.want.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if *want && !raised {
            drop(want);
            raised = true;
            if !switched_off() {
                for floor in &s.floors {
                    floor.raise();
                }
            }
        } else if !*want && raised {
            let (still, _) = s.bell.wait_timeout_while(want, TAIL, |w| !*w).unwrap_or_else(PoisonError::into_inner);
            if *still {
                want = still;
                continue;
            }
            drop(still);
            raised = false;
            for floor in &s.floors {
                floor.lower();
            }
        } else {
            want = s.bell.wait(want).unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        want = s.want.lock().unwrap_or_else(PoisonError::into_inner);
    }
}

// ------------------------------------------------------------- governors ---

/// The governor just chosen, tuned (a governor's tunables are the kernel's
/// defaults again each time it is chosen): conservative leaves work at
/// nice > 0 out of its load, so in power saving and the battery saver the
/// background never climbs the clock. schedutil and ondemand keep the
/// kernel's settings.
pub fn tune_governor() {
    let Ok(governor) = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor") else { return };
    if governor.trim() == "conservative" {
        governor_tunable("conservative", "ignore_nice_load", "1");
    }
}

/// A governor's tunable: one set for every policy, or one in each.
fn governor_tunable(governor: &str, name: &str, value: &str) {
    let root = std::path::Path::new("/sys/devices/system/cpu/cpufreq");
    let mut dirs = vec![root.join(governor)];
    dirs.extend(std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path().join(governor)));
    for dir in dirs {
        let file = dir.join(name);
        if file.exists() {
            std::fs::write(file, value).ok();
        }
    }
}

// ------------------------------------------------- the drawing thread's wait ---

/// What the drawing thread is waiting for, and so how late the kernel may
/// end its wait to fold the wakeup in with another (timer slack). A key or
/// another thread's wake ends a wait at once whatever the slack; only the
/// timeout moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    /// The next frame's beat: the thread's own slack (50 µs).
    Frame,
    /// The idle look, four times a second: 10 ms.
    Idle,
    /// The look every two seconds with the screen dark: 100 ms.
    Dark,
    /// Deep or music standby: deep.rs sets its own (a second).
    Deep,
}

/// The drawing thread's slack, for its wait alone: raised just before it
/// waits and put back as it wakes, so no thread it starts afterwards (they
/// take their slack from it) inherits the wait's.
#[derive(Default)]
pub struct LoopSlack {
    raised: bool,
}

impl LoopSlack {
    /// Before the wait.
    pub fn wait(&mut self, wait: Wait) {
        let ns = match wait {
            Wait::Idle => 10 * MS,
            Wait::Dark => 100 * MS,
            Wait::Frame | Wait::Deep => return,
        };
        set_slack(ns);
        self.raised = true;
    }

    /// Woken: the thread's own again (0 means its default).
    pub fn woke(&mut self) {
        if std::mem::take(&mut self.raised) {
            set_slack(0);
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
fn set_slack(ns: u64) {
    #[cfg(target_os = "linux")]
    // SAFETY: this thread's own timer slack.
    unsafe {
        libc::prctl(libc::PR_SET_TIMERSLACK, ns as libc::c_ulong);
    }
}

// -------------------------------------------------- a model, for --smooth ---

/// How the handheld's clock would follow a motion, for `--smooth` (which
/// measures frames on a computer and scales them to the handheld at its
/// top speed). A model, not a measurement: the kernel's rules (PELT's 32 ms
/// half-life, frequency-invariant; schedutil's 1.25 margin, 1.5 ms rate
/// limit, a look at each wakeup and each 4 ms tick; the 100 MHz steps from
/// 600 to 1000) with a clock switch assumed to land 1 ms after it is asked
/// for (cpufreq-dt's default latency, until the handheld measures it:
/// `catalyst-shell --sched-bench`).
pub mod model {
    /// The clock's steps, MHz.
    pub const STEPS: [f32; 5] = [600.0, 700.0, 800.0, 900.0, 1000.0];
    const TOP: f32 = 1000.0;
    const SWITCH_MS: f32 = 1.0;
    const RATE_LIMIT_MS: f32 = 1.5;
    const TICK_MS: f32 = 4.0;
    const HALF_LIFE_MS: f32 = 32.0;
    const DT: f32 = 0.05;

    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum Policy {
        /// The floor raised as the key goes down.
        Boost,
        /// schedutil alone, from rest: the keypad's interrupt thread (real
        /// time) asks for the top as the key comes, and the next look, with
        /// the drawing thread's utilisation near zero, asks for the least.
        Schedutil,
    }

    pub struct Clock {
        policy: Policy,
        /// Milliseconds since the key.
        t: f32,
        mhz: f32,
        /// A switch asked for and not yet landed: when, and to what.
        pending: Option<(f32, f32)>,
        /// The drawing thread's utilisation, 0 to 1024.
        util: f32,
        asked: f32,
    }

    impl Clock {
        /// At rest, at the least clock, as the key goes down.
        pub fn new(policy: Policy) -> Clock {
            Clock { policy, t: 0.0, mhz: STEPS[0], pending: Some((SWITCH_MS, TOP)), util: 10.0, asked: 0.0 }
        }

        #[cfg(test)]
        pub fn mhz(&self) -> f32 {
            self.mhz
        }

        /// The step schedutil asks for at this utilisation: the least at or
        /// above 1.25 × the top × util.
        pub fn schedutil_step(util: f32) -> f32 {
            let want = 1.25 * TOP * util / 1024.0;
            STEPS.iter().copied().find(|s| *s >= want).unwrap_or(TOP)
        }

        fn look(&mut self) {
            if self.policy != Policy::Schedutil || self.pending.is_some() || self.t - self.asked < RATE_LIMIT_MS {
                return;
            }
            let want = Self::schedutil_step(self.util);
            if want != self.mhz {
                self.pending = Some((self.t + SWITCH_MS, want));
                self.asked = self.t;
            }
        }

        fn step(&mut self, running: bool) {
            if let Some((at, mhz)) = self.pending {
                if self.t >= at {
                    self.mhz = mhz;
                    self.pending = None;
                }
            }
            let target = if running { 1024.0 * self.mhz / TOP } else { 0.0 };
            self.util += (target - self.util) * (1.0 - 0.5f32.powf(DT / HALF_LIFE_MS));
            self.t += DT;
        }

        /// Idle until `at` ms after the key.
        pub fn idle_until(&mut self, at: f32) {
            while self.t < at {
                self.step(false);
            }
        }

        /// Run `work` ms of processor time (as it takes at the top clock);
        /// returns how long it took at the clock it had.
        pub fn run(&mut self, work: f32) -> f32 {
            let began = self.t;
            // A wakeup is a look; so is every tick while it runs.
            self.look();
            let mut done = 0.0;
            let mut tick = began + TICK_MS;
            while done < work {
                done += DT * self.mhz / TOP;
                self.step(true);
                if self.t >= tick {
                    tick += TICK_MS;
                    self.look();
                }
            }
            self.t - began
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn a_key_raises_it_before_it_is_handled_and_rest_lets_it_go() {
        let t0 = Instant::now();
        let mut b = Boost::default();
        assert_eq!(b.key(t0, Place::Lit), Some(true), "up as the key goes down");
        assert_eq!(b.key(at(t0, 5), Place::Lit), None, "already up");
        // The motion the key began: held while it moves.
        for ms in (10..900).step_by(25) {
            assert_eq!(b.after(at(t0, ms), true, Place::Lit), None);
        }
        assert!(b.on());
        // Settled: let go (the thread waits out the tail).
        assert_eq!(b.after(at(t0, 925), false, Place::Lit), Some(false));
        // A key that moves nothing: up for its frame, down after it.
        assert_eq!(b.key(at(t0, 2000), Place::Lit), Some(true));
        assert_eq!(b.after(at(t0, 2010), false, Place::Lit), Some(false));
    }

    #[test]
    fn motion_begun_by_itself_raises_it_on_a_lit_screen_only() {
        let t0 = Instant::now();
        let mut b = Boost::default();
        // The island opening for a message.
        assert_eq!(b.after(t0, true, Place::Lit), Some(true));
        assert_eq!(b.after(at(t0, 400), false, Place::Lit), Some(false));
        // The screen dimming, or StandBy's faces stepping their second hand: no.
        let mut b = Boost::default();
        assert_eq!(b.after(t0, true, Place::Quiet), None);
        assert_eq!(b.after(at(t0, 500), true, Place::Quiet), None);
        // But a key there does, and holds while what it began moves.
        assert_eq!(b.key(at(t0, 600), Place::Quiet), Some(true));
        assert_eq!(b.after(at(t0, 620), true, Place::Quiet), None);
        assert_eq!(b.after(at(t0, 900), false, Place::Quiet), Some(false));
        // Dark: never, key or not (music keys in a pocket).
        let mut b = Boost::default();
        assert_eq!(b.key(t0, Place::Dark), None);
        assert_eq!(b.after(t0, true, Place::Dark), None);
        // Up, then the screen goes dark: down.
        assert_eq!(b.key(t0, Place::Lit), Some(true));
        assert_eq!(b.after(at(t0, 30), true, Place::Dark), Some(false));
    }

    #[test]
    fn motion_that_runs_on_by_itself_lets_it_go_after_the_cap() {
        let t0 = Instant::now();
        let mut b = Boost::default();
        assert_eq!(b.key(t0, Place::Lit), Some(true));
        let cap = CAP.as_millis() as u64;
        // The motion it began, from its first frame.
        assert_eq!(b.after(at(t0, 2), true, Place::Lit), None);
        assert_eq!(b.after(at(t0, cap - 10), true, Place::Lit), None, "within the cap");
        assert_eq!(b.after(at(t0, cap + 10), true, Place::Lit), Some(false), "past it, with no key");
        assert_eq!(b.after(at(t0, cap + 900), true, Place::Lit), None, "and it stays down while that motion runs on");
        // A key brings it back.
        assert_eq!(b.key(at(t0, cap + 1000), Place::Lit), Some(true));
        // So does a new motion after a rest.
        let mut b = Boost::default();
        b.after(t0, true, Place::Lit);
        b.after(at(t0, cap + 10), true, Place::Lit);
        assert!(!b.on());
        b.after(at(t0, cap + 20), false, Place::Lit);
        assert_eq!(b.after(at(t0, cap + 30), true, Place::Lit), Some(true));
    }

    #[test]
    fn held_keys_keep_it_up_through_a_long_scroll() {
        let t0 = Instant::now();
        let mut b = Boost::default();
        b.key(t0, Place::Lit);
        // A key repeating every 50 ms for ten seconds, the list gliding throughout.
        for ms in (0..10_000).step_by(50) {
            b.key(at(t0, ms), Place::Lit);
            assert_eq!(b.after(at(t0, ms + 1), true, Place::Lit), None);
        }
        assert!(b.on(), "the cap counts from the last key");
    }

    #[test]
    fn the_boosters_decisions_work_with_nothing_to_move() {
        // A computer has no cpufreq: the same decisions, no thread.
        let mut booster = Booster { decide: Boost::default(), #[cfg(target_os = "linux")] shared: None };
        let t0 = Instant::now();
        booster.key(t0, Place::Lit);
        assert!(booster.on());
        booster.release();
        assert!(!booster.on(), "let go before an external app takes the panel");
    }

    #[test]
    fn classes_say_what_they_set() {
        let idle = Class::Idle.parts();
        assert_eq!((idle.policy, idle.nice, idle.idle_io), (Some(SCHED_IDLE), Some(19), true));
        let bg = Class::Background.parts();
        assert_eq!((bg.policy, bg.nice, bg.idle_io), (Some(SCHED_BATCH), Some(19), false), "a lock it holds is never starved");
        let batch = Class::Batch(10).parts();
        assert_eq!((batch.policy, batch.nice, batch.slack_ns), (Some(SCHED_BATCH), Some(10), 10 * MS));
        assert_eq!(Class::Batch(40).parts().nice, Some(19));
        let poll = Class::Poller.parts();
        assert_eq!((poll.policy, poll.nice, poll.slack_ns), (None, None, 20 * MS));
    }

    #[test]
    fn schedutil_from_rest_leaves_a_frame_at_the_least_clock() {
        use model::{Clock, Policy};
        // A thread busy half of each beat at 600 MHz asks for no more.
        assert_eq!(Clock::schedutil_step(1024.0 * 0.52 * 0.6), 600.0);
        assert_eq!(Clock::schedutil_step(1024.0 * 0.85), 1000.0);
        // An 8 ms frame (at the top clock), then the beat's rest, for a while.
        let mut today = Clock::new(Policy::Schedutil);
        let mut boosted = Clock::new(Policy::Boost);
        let first_today = today.run(8.0);
        let first_boosted = boosted.run(8.0);
        assert!(first_boosted < 9.5, "1 ms to switch, then the top: {first_boosted}");
        // The keypad's spike lasts to the first tick's look (4 ms here; in
        // the kernel any processor's event looks, so sooner): then 600 MHz.
        assert!(first_today > first_boosted + 1.5, "{first_today} against {first_boosted}");
        let mut start = 25.0;
        for _ in 0..30 {
            today.idle_until(start);
            boosted.idle_until(start);
            today.run(8.0);
            boosted.run(8.0);
            start += 25.0;
        }
        assert_eq!(boosted.mhz(), 1000.0);
        assert!(today.mhz() <= 700.0, "half a beat busy never climbs far: {}", today.mhz());
    }
}
