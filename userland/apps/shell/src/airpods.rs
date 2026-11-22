//! AirPods (and Beats): what an iPhone has of them beside the sound, over
//! Apple's own protocol (AAP, the `aap` crate): the battery of each bud and
//! the case, noise control, ear detection, and the head tracking spatial
//! sound turns with.
//!
//! A thread of its own, asleep until sound goes to Bluetooth headphones.
//! If they are Apple's (their Device ID's vendor, 0x004C, in the modalias
//! BlueZ shows), it opens AAP's L2CAP channel beside the audio and keeps it
//! while they play. Other headphones, and AirPods too old to answer, have
//! none of it: the pages show what they have and no more.
//!
//! - **Battery**: Settings › Bluetooth shows each bud and the case; the
//!   island shows them for a moment as the AirPods connect.
//! - **Noise control** (off, noise cancelling, transparency, adaptive where
//!   they have it): Settings › Sound, while they are connected.
//! - **Ear detection**: a bud taken out pauses Music; put back within ten
//!   minutes, it plays on.
//! - **Head tracking**: only while spatial sound plays to them, which is
//!   while the player listens on its pose socket (audiofx::head): asked
//!   for then, stopped as soon as nothing listens (the song paused, spatial
//!   sound off), so the buds' sensors and the radio's extra traffic cost
//!   nothing the rest of the time. Each pose goes on to the player as it
//!   comes. AirPods that never send one (AirPods 1 and 2) are left alone.
//!
//! Also here: Bluetooth's codec put right. bluez-alsa offers AAC ahead of
//! SBC, so headphones that take it get it; if a pair connected on SBC
//! anyway, it is moved to AAC once (`catalyst-bluetooth codec`).

// The thread runs on the handheld alone; a computer's build has the demo.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use aap::{Cell, Charge, Ear, NoiseMode, Part};

/// The island shows the battery this long after they connect.
const ACTIVITY_FOR: Duration = Duration::from_secs(6);
/// A bud put back this soon after it paused Music plays it on.
const RESUME_WITHIN: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    /// Not asked for.
    Off,
    /// Asked for; no pose yet.
    Starting,
    Tracking,
    /// Asked for twice and nothing came: these do not track the head.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub address: String,
    pub name: String,
    pub model: String,
    pub battery: Vec<Cell>,
    pub ears: Option<(Ear, Ear)>,
    /// Their noise control mode, once they have said it (none: they have none).
    pub noise: Option<NoiseMode>,
    pub head: Head,
    /// The head's last pose (degrees), while tracking.
    pub pose: (f32, f32),
    /// When their battery was first said, this connection.
    pub battery_at: Option<Instant>,
}

impl State {
    fn new(address: &str) -> State {
        State { address: address.to_owned(), name: crate::bluetooth::name_of(address), model: String::new(), battery: Vec::new(), ears: None, noise: None, head: Head::Off, pose: (0.0, 0.0), battery_at: None }
    }

    /// The noise control modes these have, in the order ←→ steps through.
    pub fn noise_modes(&self) -> Vec<NoiseMode> {
        let mut modes = vec![NoiseMode::Off, NoiseMode::Cancelling, NoiseMode::Transparency];
        if aap::has_adaptive(&self.model) || self.noise == Some(NoiseMode::Adaptive) {
            modes.push(NoiseMode::Adaptive);
        }
        modes
    }

    pub fn cell(&self, part: Part) -> Option<Cell> {
        self.battery.iter().copied().find(|c| c.part == part)
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static TX: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();

enum Cmd {
    /// Where sound goes changed, or spatial sound was switched: look again.
    Look,
    Noise(NoiseMode),
}

fn send(cmd: Cmd) {
    if let Some(tx) = TX.get() {
        tx.lock().unwrap().send(cmd).ok();
    }
}

/// The AirPods connected now, if any (and what they have said).
pub fn now() -> Option<State> {
    if !crate::sound::on_device() {
        return demo_with(|d| d.clone());
    }
    STATE.lock().unwrap().clone()
}

/// The thread, at the shell's start (on the handheld).
pub fn start() {
    if !crate::sound::on_device() || TX.get().is_some() {
        return;
    }
    let (tx, rx) = channel();
    TX.set(Mutex::new(tx)).ok();
    std::thread::Builder::new().name("airpods".into()).stack_size(128 * 1024).spawn(move || run(rx)).ok();
}

/// Sound moved, or spatial sound was switched on or off.
pub fn look() {
    send(Cmd::Look);
}

/// Their noise control set (the Sound page's ←→).
pub fn set_noise(mode: NoiseMode) {
    if !crate::sound::on_device() {
        demo_with(|d| {
            if let Some(s) = d.as_mut() {
                s.noise = Some(mode);
            }
        });
        return;
    }
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        s.noise = Some(mode);
    }
    send(Cmd::Noise(mode));
}

// ------------------------------------------------------------- words ----

/// "80 %", with "charging" where it is.
pub fn cell_words(c: Cell) -> String {
    match c.charge {
        Charge::Charging => format!("{} % · charging", c.level),
        _ => format!("{} %", c.level),
    }
}

/// The buds' and case's levels in a line, shortest last: for the island.
pub fn battery_line(s: &State) -> Vec<String> {
    let pick = |p: Part| s.cell(p).map(|c| c.level);
    match (pick(Part::Left), pick(Part::Right), pick(Part::Case), pick(Part::Headphones)) {
        (_, _, _, Some(h)) => vec![format!("{h} %")],
        (Some(l), Some(r), case, _) => {
            let case_words = case.map(|c| format!(" · case {c} %")).unwrap_or_default();
            vec![format!("left {l} % · right {r} %{case_words}"), format!("left {l} · right {r}{case_words}"), format!("L {l} · R {r}")]
        }
        (one_l, one_r, case, _) => {
            let mut parts = Vec::new();
            if let Some(l) = one_l {
                parts.push(format!("left {l} %"));
            }
            if let Some(r) = one_r {
                parts.push(format!("right {r} %"));
            }
            if let Some(c) = case {
                parts.push(format!("case {c} %"));
            }
            vec![parts.join(" · ")]
        }
    }
}

/// The lowest bud (or the headphones): the island's ring and short value.
fn lowest(s: &State) -> Option<u8> {
    s.battery.iter().filter(|c| c.part != Part::Case).map(|c| c.level).min()
}

/// The island's activity as they connect: their battery, for a moment.
pub fn activity() -> Option<crate::liveactivity::Activity> {
    use crate::liveactivity::{Activity, Kind, Mark, Meter, Tint};
    let s = now()?;
    let at = s.battery_at?;
    if at.elapsed() > ACTIVITY_FOR {
        return None;
    }
    let low = lowest(&s)?;
    // The case first: where the line is too long for the card, the card
    // leaves out what comes before its first " · ", and the buds stay.
    let detail = match (s.cell(Part::Case), s.cell(Part::Left), s.cell(Part::Right)) {
        (case, Some(l), Some(r)) => {
            let buds = format!("left {} · right {}", l.level, r.level);
            case.map_or(buds.clone(), |c| format!("case {} % · {buds}", c.level))
        }
        _ => battery_line(&s).into_iter().last().unwrap_or_default(),
    };
    Some(Activity {
        kind: Kind::Headphones,
        id: "airpods".into(),
        priority: 45,
        mark: Mark::Icon(if aap::is_max(&s.model) { "headphones" } else { "earbuds" }, Tint::Ice),
        title: s.name.clone(),
        detail,
        short: format!("{low} %"),
        value: Some(format!("{low} %")),
        meter: Some(Meter::Fraction(low as f32 / 100.0)),
        playing: None,
        urgent: false,
        state: format!("connected {}", s.address),
        seed: None,
        sleep: None,
        sung: None,
    })
}

// ------------------------------------------------------ ear detection ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarAction {
    Pause,
    Resume,
    Nothing,
}

fn in_ears(e: (Ear, Ear)) -> usize {
    [e.0, e.1].iter().filter(|&&x| x == Ear::In).count()
}

/// What a change of where the buds are does to Music: one taken out while
/// it plays pauses it; all of them back again soon after plays it on.
/// `paused`: how many were in when ear detection paused it, and when.
pub fn ears_changed(before: Option<(Ear, Ear)>, now: (Ear, Ear), playing: bool, paused: Option<(usize, Instant)>) -> EarAction {
    let Some(before) = before else { return EarAction::Nothing };
    let (was, is) = (in_ears(before), in_ears(now));
    if is < was && playing {
        return EarAction::Pause;
    }
    match paused {
        Some((wanted, at)) if is > was && is >= wanted && !playing && at.elapsed() < RESUME_WITHIN => EarAction::Resume,
        _ => EarAction::Nothing,
    }
}

// --------------------------------------------------------------- codec ----

/// Bluetooth codecs, best first: AAC (AirPods, most headphones), then
/// aptX HD and aptX where an image has them, then SBC, which all have.
const CODECS: [&str; 4] = ["AAC", "aptX-HD", "aptX", "SBC"];

/// The codec to move a pair to, if a better one than the one they play
/// with is on offer.
pub fn better_codec(selected: &str, available: &[&str]) -> Option<&'static str> {
    // One this does not rank (Opus, chosen by hand) is left as it is.
    let now = CODECS.iter().position(|c| c.eq_ignore_ascii_case(selected))?;
    CODECS.iter().take(now).find(|c| available.iter().any(|a| a.eq_ignore_ascii_case(c))).copied()
}

// -------------------------------------------------------------- thread ----

/// The Bluetooth headphones sound goes to now, if it does.
fn bluetooth_target() -> Option<String> {
    if crate::sound::route().out != crate::sound::Out::Bluetooth {
        return None;
    }
    let chosen = std::fs::read_to_string("/data/catalyst/audio-output").ok()?;
    let mut f = chosen.split_whitespace();
    (f.next()? == "bluetooth").then(|| f.next().map(str::to_owned)).flatten()
}

fn run(rx: Receiver<Cmd>) {
    let mut apple: Vec<(String, bool)> = Vec::new();
    let mut codec_checked: Option<String> = None;
    let mut failures = 0u32;
    loop {
        let target = bluetooth_target();
        if let Some(addr) = &target {
            if codec_checked.as_deref() != Some(addr.as_str()) {
                codec_checked = Some(addr.clone());
                put_codec_right(addr);
            }
        } else {
            codec_checked = None;
            failures = 0;
        }
        let is_apple = target.as_ref().is_some_and(|a| {
            if let Some((_, yes)) = apple.iter().find(|(x, _)| x == a) {
                return *yes;
            }
            let yes = modalias(a).is_some_and(|m| aap::is_apple(&m));
            apple.push((a.clone(), yes));
            yes
        });
        if let (Some(addr), true, true) = (&target, is_apple, failures < 3) {
            #[cfg(not(target_os = "linux"))]
            let _ = addr;
            #[cfg(target_os = "linux")]
            match session(addr, &rx) {
                Ok(()) => failures = 0,
                Err(e) => {
                    eprintln!("catalyst-shell: airpods: {e}");
                    failures += 1;
                }
            }
            *STATE.lock().unwrap() = None;
            crate::platform::wake();
            // A moment before trying again (they may be switching to a phone).
            if failures > 0 {
                match rx.recv_timeout(Duration::from_secs(5 * failures as u64)) {
                    Err(RecvTimeoutError::Disconnected) => return,
                    _ => continue,
                }
            }
            continue;
        }
        if rx.recv().is_err() {
            return;
        }
    }
}

/// A time far enough back that the next look at head tracking is due now.
#[cfg(target_os = "linux")]
fn look_now() -> Instant {
    Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or_else(Instant::now)
}

/// BlueZ's modalias for a device ("bluetooth:v004Cp2014d0E10").
fn modalias(address: &str) -> Option<String> {
    let out = std::process::Command::new("bluetoothctl").args(["info", address]).stderr(std::process::Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().strip_prefix("Modalias:").map(|m| m.trim().to_owned()))
}

/// A pair on SBC that offers better, moved to it (once a connection).
fn put_codec_right(address: &str) {
    let ask = |extra: Option<&str>| {
        let mut cmd = std::process::Command::new("/usr/sbin/catalyst-bluetooth");
        cmd.args(["codec", address]);
        if let Some(c) = extra {
            cmd.arg(c);
        }
        cmd.stderr(std::process::Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    // bluealsa offers the stream a moment after the link.
    for _ in 0..5 {
        if let Some(text) = ask(None) {
            let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).unwrap_or("").to_owned();
            let (selected, available) = (field("selected"), field("available"));
            if !selected.is_empty() {
                if let Some(better) = better_codec(&selected, &available.split_whitespace().collect::<Vec<_>>()) {
                    ask(Some(better));
                }
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(target_os = "linux")]
fn session(address: &str, rx: &Receiver<Cmd>) -> std::io::Result<()> {
    use aap::{l2cap::Channel, Message};
    let ch = Channel::connect(address, aap::PSM, Duration::from_secs(5))?;
    ch.send(&aap::HANDSHAKE)?;
    std::thread::sleep(Duration::from_millis(100));
    ch.send(&aap::SET_FEATURES)?;
    ch.send(&aap::REQUEST_NOTIFICATIONS)?;
    *STATE.lock().unwrap() = Some(State::new(address));
    let mut orientation = aap::Orientation::new();
    let pose_out = std::os::unix::net::UnixDatagram::unbound()?;
    let mut buf = [0u8; 1024];
    let mut checked = look_now();
    // Head tracking: asked for when, and in which spelling.
    let mut asked: Option<(Instant, bool)> = None;
    let mut unsupported = false;
    let mut paused_by_ears: Option<(usize, Instant)> = None;
    let update = |f: &mut dyn FnMut(&mut State)| {
        if let Some(s) = STATE.lock().unwrap().as_mut() {
            f(s);
        }
    };
    let result = loop {
        // Woken by each packet; else once a second (the look at head
        // tracking below), so with the screen off and nothing tracked this
        // thread costs one wake-up a second (ALSA's alone are 23).
        match ch.recv(&mut buf, Duration::from_secs(1)) {
            Ok(Some(n)) => match aap::parse(&buf[..n]) {
                Some(Message::Battery(cells)) if !cells.is_empty() => {
                    update(&mut |s| {
                        s.battery = cells.clone();
                        s.battery_at.get_or_insert_with(Instant::now);
                    });
                    crate::platform::wake();
                }
                Some(Message::Ears(a, b)) => {
                    let before = STATE.lock().unwrap().as_ref().and_then(|s| s.ears);
                    update(&mut |s| s.ears = Some((a, b)));
                    let playing = crate::music::playing();
                    match ears_changed(before, (a, b), playing, paused_by_ears) {
                        EarAction::Pause => {
                            paused_by_ears = before.map(|e| (in_ears(e), Instant::now()));
                            crate::music::media_key(crate::platform::Media::PlayPause);
                        }
                        EarAction::Resume => {
                            paused_by_ears = None;
                            crate::music::media_key(crate::platform::Media::PlayPause);
                        }
                        EarAction::Nothing => {}
                    }
                }
                Some(Message::Noise(mode)) => {
                    update(&mut |s| s.noise = Some(mode));
                    crate::platform::wake();
                }
                Some(Message::Info(info)) => update(&mut |s| {
                    if !info.name.is_empty() {
                        s.name = info.name.clone();
                    }
                    s.model = info.model.clone();
                }),
                Some(Message::Head(raw)) => {
                    if let Some((yaw, pitch)) = orientation.update(raw) {
                        update(&mut |s| {
                            s.head = Head::Tracking;
                            s.pose = (yaw, pitch);
                        });
                        let pose = audiofx::head::encode(audiofx::head::Pose { yaw, pitch });
                        if pose_out.send_to(&pose, audiofx::head::SOCKET).is_err() {
                            // Nothing listening any more: stopped at the next look.
                            checked = look_now();
                        }
                    }
                }
                _ => {}
            },
            Ok(None) => {}
            Err(e) => break Err(e),
        }
        loop {
            match rx.try_recv() {
                // They answer with the mode they took (a notification).
                Ok(Cmd::Noise(mode)) => ch.send(&aap::noise_control(mode))?,
                Ok(Cmd::Look) => checked = look_now(),
                Err(_) => break,
            }
        }
        if bluetooth_target().as_deref() != Some(address) {
            break Ok(());
        }
        // Once a second: head tracking while (and only while) spatial
        // sound plays and the player listens for the pose.
        if checked.elapsed() >= Duration::from_secs(1) {
            checked = Instant::now();
            let listening = crate::sound::fx().spatial && pose_out.send_to(&[0u8], audiofx::head::SOCKET).is_ok();
            match (listening && !unsupported, asked) {
                (true, None) => {
                    orientation.reset();
                    ch.send(&aap::head_tracking(true, false))?;
                    asked = Some((Instant::now(), false));
                    update(&mut |s| s.head = Head::Starting);
                }
                // Nothing came: the older firmware's spelling, then give up.
                (true, Some((at, alternate))) if STATE.lock().unwrap().as_ref().is_some_and(|s| s.head == Head::Starting) && at.elapsed() > Duration::from_secs(3) => {
                    if alternate {
                        unsupported = true;
                        update(&mut |s| s.head = Head::Unsupported);
                        asked = None;
                    } else {
                        ch.send(&aap::head_tracking(true, true))?;
                        asked = Some((Instant::now(), true));
                    }
                }
                (false, Some((_, alternate))) => {
                    ch.send(&aap::head_tracking(false, alternate))?;
                    asked = None;
                    update(&mut |s| {
                        if s.head != Head::Unsupported {
                            s.head = Head::Off;
                        }
                    });
                }
                _ => {}
            }
        }
    };
    if let Some((_, alternate)) = asked {
        ch.send(&aap::head_tracking(false, alternate)).ok();
    }
    result
}

// --------------------------------------------------------- simulator ----

// The simulator's AirPods: one for the whole shell, or one a test thread
// (the tests run side by side, and others' islands must not see them).
#[cfg(not(test))]
static DEMO: Mutex<Option<State>> = Mutex::new(None);

#[cfg(not(test))]
fn demo_with<T>(f: impl FnOnce(&mut Option<State>) -> T) -> T {
    f(&mut DEMO.lock().unwrap())
}

#[cfg(test)]
thread_local! {
    static DEMO: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn demo_with<T>(f: impl FnOnce(&mut Option<State>) -> T) -> T {
    DEMO.with(|d| f(&mut d.borrow_mut()))
}

/// The simulator's AirPods Pro, connected a while ago (or not), for the
/// galleries.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo(connected: bool) {
    demo_connected_at(connected, Instant::now().checked_sub(ACTIVITY_FOR * 2));
}

/// The simulator's AirPods Pro, connecting now: the island shows them.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn demo_connecting() {
    demo_connected_at(true, Some(Instant::now()));
}

fn demo_connected_at(connected: bool, battery_at: Option<Instant>) {
    let state = connected.then(|| State {
        address: "F4:34:F0:12:34:56".into(),
        name: "AirPods Pro".into(),
        model: "A2698".into(),
        battery: vec![
            Cell { part: Part::Left, level: 82, charge: Charge::Discharging },
            Cell { part: Part::Right, level: 78, charge: Charge::Discharging },
            Cell { part: Part::Case, level: 46, charge: Charge::Charging },
        ],
        ears: Some((Ear::In, Ear::In)),
        noise: Some(NoiseMode::Cancelling),
        head: Head::Tracking,
        pose: (12.0, -3.0),
        battery_at,
    });
    demo_with(|d| *d = state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bud_out_pauses_and_back_in_plays_on() {
        let both = (Ear::In, Ear::In);
        let one = (Ear::In, Ear::Out);
        let now = Instant::now();
        assert_eq!(ears_changed(Some(both), one, true, None), EarAction::Pause);
        // Paused by it, with two in: both back plays on.
        assert_eq!(ears_changed(Some(one), both, false, Some((2, now))), EarAction::Resume);
        // Only back to one of two: still paused.
        assert_eq!(ears_changed(Some((Ear::Out, Ear::Out)), one, false, Some((2, now))), EarAction::Nothing);
        // Paused by hand, not by the ears: put back, nothing.
        assert_eq!(ears_changed(Some(one), both, false, None), EarAction::Nothing);
        // Not playing: taking one out does nothing.
        assert_eq!(ears_changed(Some(both), one, false, None), EarAction::Nothing);
        // The first word after connecting is only where they are.
        assert_eq!(ears_changed(None, one, true, None), EarAction::Nothing);
        // Into the case from the ear counts as out.
        assert_eq!(ears_changed(Some(both), (Ear::InCase, Ear::In), true, None), EarAction::Pause);
        // Long after, a bud put back leaves Music alone.
        let long_ago = Instant::now().checked_sub(Duration::from_secs(700)).unwrap_or(now);
        if long_ago != now {
            assert_eq!(ears_changed(Some(one), both, false, Some((2, long_ago))), EarAction::Nothing);
        }
    }

    #[test]
    fn a_better_codec_is_chosen_where_there_is_one() {
        assert_eq!(better_codec("SBC", &["SBC", "AAC"]), Some("AAC"));
        assert_eq!(better_codec("AAC", &["SBC", "AAC"]), None);
        assert_eq!(better_codec("SBC", &["SBC"]), None);
        assert_eq!(better_codec("SBC", &["SBC", "aptX"]), Some("aptX"));
        assert_eq!(better_codec("aptX", &["SBC", "aptX", "AAC"]), Some("AAC"));
        // A codec it does not rank (Opus, chosen by hand): left as it is.
        assert_eq!(better_codec("Opus", &["Opus", "SBC", "AAC"]), None);
    }

    #[test]
    fn the_battery_reads_bud_by_bud() {
        demo_connecting();
        let s = now().unwrap();
        assert_eq!(battery_line(&s)[0], "left 82 % · right 78 % · case 46 %");
        assert_eq!(lowest(&s), Some(78));
        assert_eq!(cell_words(s.cell(Part::Case).unwrap()), "46 % · charging");
        assert!(s.noise_modes().contains(&NoiseMode::Adaptive));
        let mut old = s.clone();
        old.model = "A2084".into();
        old.noise = Some(NoiseMode::Cancelling);
        assert_eq!(old.noise_modes(), [NoiseMode::Off, NoiseMode::Cancelling, NoiseMode::Transparency]);
        let a = activity().unwrap();
        assert_eq!((a.title.as_str(), a.short.as_str(), a.detail.as_str()), ("AirPods Pro", "78 %", "case 46 % · left 82 · right 78"));
        // Connected a while: the island has let them go.
        demo(true);
        assert!(activity().is_none());
        demo(false);
        assert!(activity().is_none());
    }
}
