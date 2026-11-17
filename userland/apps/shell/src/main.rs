//! catalyst-shell: the GUI of Catalyst Linux.
//!
//! ```text
//! catalyst-shell                 on the device: /dev/fb0 and evdev keys
//! catalyst-shell --sim           a desktop window showing the 320 × 170 panel
//! catalyst-shell --shots <dir>   render a scripted tour to PNG files
//! catalyst-shell --bench         draw 600 frames off screen; print time and memory
//! catalyst-shell --smooth        how even the frames of each transition are, at the
//!                                handheld's estimated speed and at worse
//!     --robot <team|ip|host>     robot to connect to (overrides the saved one)
//!     --demo                     a scripted robot instead of a real one
//!     --light, --calm            start in light mode / the calm version
//!     --text <size>              compact, standard or large
//!     --accent <name>            orange, blue, green, violet or rose
//!     --speed <device|worse|pc>  (simulator) run as slowly as the handheld would,
//!                                or slower; the window title shows the cost
//!     --perf                     also print that cost once a second
//! ```

mod agent;
mod gfx;
mod logs;
mod platform;
mod robot;
mod screens;
mod shell;
mod sys;
mod wifi;
mod wpilog;

use std::time::{Duration, Instant};

use platform::{Key, Platform};
use robot::Robot;
use shell::Shell;

fn config_path() -> std::path::PathBuf {
    std::env::var_os("CATALYST_CONF").map(Into::into).unwrap_or_else(|| "/etc/catalyst/robot".into())
}

/// Remember the robot address across boots. False if it could not be written.
pub fn save_robot_address(address: &str) -> bool {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(path, address).is_ok()
}

/// Appearance settings live beside the robot address, on the data partition.
fn settings_path() -> std::path::PathBuf {
    config_path().with_file_name("settings")
}

pub fn save_settings(settings: &shell::Settings) {
    let text = format!("light={}\ncalm={}\ntext={}\naccent={}\n", settings.light, settings.calm, settings.text.name(), settings.accent.name());
    std::fs::write(settings_path(), text).ok();
}

fn load_settings(settings: &mut shell::Settings) {
    let Ok(text) = std::fs::read_to_string(settings_path()) else { return };
    for line in text.lines() {
        match line.split_once('=') {
            Some(("light", v)) => settings.light = v == "true",
            Some(("calm", v)) => settings.calm = v == "true",
            Some(("text", v)) => settings.text = gfx::TextSize::from_name(v).unwrap_or(settings.text),
            Some(("accent", v)) => settings.accent = bezel::Accent::from_name(v).unwrap_or(settings.accent),
            _ => {}
        }
    }
}

const SSH_FLAG: &str = "/data/catalyst/ssh-enabled";
const SSH_TOGGLE: &str = "/usr/sbin/catalyst-ssh-toggle";

/// Whether the SSH server is switched on (it is off unless chosen).
pub fn ssh_enabled() -> bool {
    std::path::Path::new(SSH_FLAG).exists()
}

/// Start or stop the SSH server and remember the choice across boots.
pub fn set_ssh(on: bool) -> Result<(), String> {
    if !std::path::Path::new(SSH_TOGGLE).exists() {
        return Err("ssh is set on the device".into());
    }
    match std::process::Command::new(SSH_TOGGLE).arg(if on { "on" } else { "off" }).status() {
        Ok(status) if status.success() => Ok(()),
        _ => Err("could not change ssh".into()),
    }
}

/// This process's and the system's memory use, where the kernel reports it.
pub fn memory_line() -> Option<String> {
    let field = |text: &str, name: &str| -> Option<u64> {
        text.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse().ok()
    };
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let rss = field(&status, "VmRSS:")?;
    let (total, available) = (field(&meminfo, "MemTotal:")?, field(&meminfo, "MemAvailable:")?);
    Some(format!("shell {:.1} mb · system {:.0} of {:.0} mb used", rss as f64 / 1024.0, (total - available) as f64 / 1024.0, total as f64 / 1024.0))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();

    let saved = std::fs::read_to_string(config_path()).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let robot = match (flag("--demo"), value("--robot").or(saved)) {
        (true, _) => Robot::demo(),
        (_, Some(input)) if input == "demo" => Robot::demo(),
        (_, Some(input)) => Robot::connect(&screens::resolve_address(&input)),
        _ => Robot::none(),
    };
    let mut shell = Shell::new(robot);
    load_settings(&mut shell.settings);
    shell.settings.light |= flag("--light");
    shell.settings.calm |= flag("--calm");
    if let Some(size) = value("--text").and_then(|name| gfx::TextSize::from_name(&name)) {
        shell.settings.text = size;
    }
    if let Some(accent) = value("--accent").and_then(|name| bezel::Accent::from_name(&name)) {
        shell.settings.accent = accent;
    }
    shell.key(Key::Char('\0')); // apply the settings

    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--shots") {
        return shots(shell, std::path::Path::new(&dir));
    }

    if flag("--bench") {
        return bench(shell);
    }
    if flag("--smooth") {
        return smooth(shell);
    }
    if flag("--about") {
        // What Settings › About and Network would show, as text.
        let lines = [
            ("device", sys::model()), ("kernel", sys::kernel()), ("name", sys::hostname()), ("up for", sys::uptime()),
            ("memory in use", sys::memory()), ("this shell", sys::shell_memory()), ("storage", sys::storage()),
            ("battery", sys::battery()), ("processor", sys::cpu_temperature()), ("ethernet", sys::address("eth0")),
            ("backlight", sys::brightness().map(|b| format!("{b}%"))),
        ];
        for (name, value) in lines {
            println!("{name:>14}: {}", value.unwrap_or_else(|| "–".into()));
        }
        return;
    }

    let mut platform: Box<dyn Platform> = open_platform(flag("--sim"));
    let speed = Speed::named(value("--speed").as_deref());
    let mut beat = Beat::new();
    let mut meter = Meter::default();
    let perf = flag("--perf");
    let mut last = Instant::now();
    // While anything moves, frames start on a steady beat, so motion is even
    // rather than as fast as each frame happens to allow. Idle, nothing is
    // drawn at all.
    let mut next_frame = Instant::now();
    loop {
        let moving = shell.animating();
        let timeout = if moving { next_frame.saturating_duration_since(Instant::now()) } else { Duration::from_millis(250) };
        let Some(keys) = platform.poll(timeout) else { break };
        // Keys are taken in whenever they arrive; the frame waits for its beat.
        for key in keys {
            shell.key(key);
        }
        if moving && Instant::now() < next_frame {
            continue;
        }
        let work = Instant::now();
        let dt = (work - last).as_secs_f32().min(0.25);
        last = work;
        let changed = shell.tick(dt);
        if changed {
            shell.draw();
        }
        let cost = speed.busy(work);
        let part = if changed || platform.present_always() { platform.present(&shell.gfx.canvas) } else { 0.0 };
        if let Some(line) = meter.record(cost, part > 0.0) {
            platform.set_status(&format!("Catalyst Linux · {} · {line}", speed.name));
            if perf {
                eprintln!("{line}");
            }
        }
        // What this frame needed: its own work, and the panel taking what
        // was sent. The beat slows to one the handheld can hold, so frames
        // stay evenly spaced rather than alternating fast and slow.
        let beats = if part > 0.0 { beat.frame_took(cost + speed.panel.mul_f32(part)) } else { 1 };
        next_frame += beat.interval() * beats;
        if !shell.animating() {
            beat.rest();
            next_frame = Instant::now();
        } else if next_frame < Instant::now() {
            next_frame = Instant::now();
        }
    }
}

/// The time from one frame's start to the next while something is moving.
///
/// Frames evenly spaced look smooth; frames that alternate fast and slow do
/// not, however good their average. So the beat is the fastest of a few
/// steady rates that the handheld is actually holding: 40 frames a second
/// (25 ms, or `CATALYST_FRAME_MS`), then 30, 20 and 10. One late frame is
/// let go: it takes an extra beat and the rhythm carries on. A second soon
/// after means the beat is too fast, and it steps down. It steps back up only
/// after a long run of frames with room to spare, so it settles rather than
/// hunts.
struct Beat {
    steps: [Duration; 4],
    level: usize,
    /// Frames in a row that would have fitted the next faster beat.
    roomy: u32,
    /// Frames since one ran late.
    since_late: u32,
}

impl Beat {
    fn new() -> Beat {
        let ms = std::env::var("CATALYST_FRAME_MS").ok().and_then(|v| v.parse::<f32>().ok()).filter(|ms| (5.0..=100.0).contains(ms)).unwrap_or(25.0);
        let base = Duration::from_secs_f32(ms / 1000.0);
        Beat { steps: [base, base.mul_f32(4.0 / 3.0), base * 2, base * 4], level: 0, roomy: 0, since_late: u32::MAX }
    }

    fn interval(&self) -> Duration {
        self.steps[self.level]
    }

    /// Note what a frame needed. Returns how many beats it takes up: one,
    /// unless it ran late.
    fn frame_took(&mut self, needed: Duration) -> u32 {
        if needed > self.interval() {
            self.roomy = 0;
            if self.since_late < 8 && self.level + 1 < self.steps.len() {
                // The second late frame in a short while: this beat is too fast.
                self.level += 1;
                self.since_late = u32::MAX;
            } else {
                self.since_late = 0;
            }
        } else {
            self.since_late = self.since_late.saturating_add(1);
            if self.level > 0 && needed.mul_f32(1.15) < self.steps[self.level - 1] {
                self.roomy += 1;
            } else {
                self.roomy = 0;
            }
        }
        (needed.as_secs_f32() / self.interval().as_secs_f32()).ceil().max(1.0) as u32
    }

    /// Motion has stopped. Only now may the beat quicken, so that it never
    /// changes pace in the middle of an animation.
    fn rest(&mut self) {
        if self.roomy >= 40 && self.level > 0 {
            self.level -= 1;
            self.roomy = 0;
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn open_platform(_sim: bool) -> Box<dyn Platform> {
    Box::new(platform::device::Device::open().expect("open /dev/fb0"))
}

#[cfg(not(target_arch = "aarch64"))]
fn open_platform(sim: bool) -> Box<dyn Platform> {
    #[cfg(target_os = "linux")]
    if !sim {
        return Box::new(platform::device::Device::open().expect("open /dev/fb0"));
    }
    let _ = sim;
    Box::new(platform::sim::Sim::open(gfx::W as usize, gfx::H as usize))
}

/// A scripted tour: press keys, let motion settle, save each screen.
#[cfg(not(target_arch = "aarch64"))]
fn shots(mut shell: Shell, dir: &std::path::Path) {
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let tour: &[(&str, &[Key])] = &[
        ("01-home", &[]),
        ("02-robot", &[Enter]),
        ("03-health", &[Back, Right, Enter]),
        ("04-logs", &[Back, Right, Enter]),
        ("04b-match", &[Enter]),
        ("04c-tables", &[Back, Back, Down, Left, Left, Enter]),
        ("05-tables-elevator", &[Down, Down, Enter]),
        ("06-loop", &[Back, Back, Right, Enter]),
        ("07-motors", &[Back, Right, Enter, Down]),
        ("08-console", &[Back, Right, Enter]),
        ("09-settings", &[Back, Right, Enter]),
        ("09b-appearance", &[Enter]),
        ("09c-light-blue", &[Enter, Down, Enter]),
        ("09d-network", &[Back, Down, Down, Enter]),
        ("09e-wifi", &[Enter, Down, Down, Down]),
        ("09f-wifi-password", &[Enter, Char('c'), Char('a'), Char('t'), Char('a'), Char('l'), Char('y'), Char('s'), Char('t'), Char('7')]),
        ("09g-about", &[Back, Back, Back, Down, Down, Enter]),
        ("10-home-page-2-light", &[Back, Back]),
    ];
    let frame = |shell: &mut Shell| {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    };
    let started = Instant::now();
    let mut frames = 0u32;
    // Home's tiles cascade in when it first appears: one shot of that, then let it finish.
    for _ in 0..14 {
        frame(&mut shell);
    }
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("00-cascade.png"), 3);
    for _ in 0..90 {
        frame(&mut shell);
    }
    for (name, keys) in tour {
        for key in keys.iter() {
            shell.key(*key);
            // Mid-turn frames show the page spring; settle before the next key.
            for _ in 0..90 {
                frame(&mut shell);
                frames += 1;
            }
        }
        shell.key(Key::Char('\0'));
        frame(&mut shell);
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    }
    // An app caught opening out of its tile, then caught again on its way back.
    shell.key(Key::Enter);
    for _ in 0..8 {
        frame(&mut shell);
    }
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("11-app-opening.png"), 3);
    for _ in 0..90 {
        frame(&mut shell);
    }
    // And a page inside the app, caught sliding in.
    shell.key(Key::Enter);
    for _ in 0..9 {
        frame(&mut shell);
    }
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("12-page-sliding.png"), 3);
    println!("{} screens in {}, {frames} frames at {:.2} ms each", tour.len() + 1, dir.display(), started.elapsed().as_secs_f64() * 1e3 / frames as f64);
}

/// Draw every screen repeatedly without a display, to time the renderer and
/// see what the process weighs on the hardware it runs on.
fn bench(mut shell: Shell) {
    use Key::*;
    let keys = [Enter, Back, Right, Enter, Down, Back, Right, Enter, Down, Enter, Back, Back];
    let started = Instant::now();
    let mut frames = 0u32;
    for round in 0..50 {
        shell.key(keys[round % keys.len()]);
        for _ in 0..12 {
            shell.tick(1.0 / 60.0);
            shell.draw();
            frames += 1;
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!("{frames} frames, {ms:.2} ms each ({:.0} fps of headroom)", 1000.0 / ms);
    // On a PC, what the simulator's two speeds make of that. Every frame here
    // is a full redraw with a page turn in flight: the worst case.
    if cfg!(not(target_arch = "aarch64")) {
        for name in ["device", "worse"] {
            let speed = Speed::named(Some(name));
            let mut beat = Beat::new();
            let needed = Duration::from_secs_f32(ms as f32 * speed.cpu / 1e3) + speed.panel;
            beat.frame_took(needed);
            let interval = beat.interval().as_secs_f32() * 1e3;
            println!("  {}: {:.1} ms processor + {} ms panel holds a {interval:.0} ms beat = {:.0} fps for a full-screen redraw", speed.name, ms as f32 * speed.cpu, speed.panel.as_millis(), 1000.0 / interval);
        }
    }
    println!("{}", memory_line().unwrap_or_else(|| "no /proc here".into()));
}

/// The handheld's speed, imposed on a development PC so the simulator shows
/// what the device will: how smooth motion really is, and how much of the
/// processor a screen costs (which is what the battery pays for).
///
/// The factors are estimates until a CardputerZero is measured: a 1 GHz
/// Cortex-A53 against one core of this PC, and the time the 60 MHz SPI link
/// takes to carry one 320 × 170 frame. On the device, `--bench` gives the
/// real figure to set them from.
struct Speed {
    name: &'static str,
    /// How many times longer the handheld takes over the same work.
    cpu: f32,
    /// How long the panel takes to accept a whole frame; part of a frame
    /// takes that part of it.
    panel: Duration,
}

impl Speed {
    fn named(name: Option<&str>) -> Speed {
        match name {
            Some("device") => Speed { name: "device speed (est.)", cpu: 20.0, panel: Duration::from_millis(15) },
            Some("worse") => Speed { name: "worse than device", cpu: 40.0, panel: Duration::from_millis(25) },
            _ => Speed { name: "full pc speed", cpu: 1.0, panel: Duration::ZERO },
        }
    }

    /// Hold the loop until the handheld would have finished the work begun at
    /// `work`. Returns the time the handheld's processor was busy.
    fn busy(&self, work: Instant) -> Duration {
        let cost = work.elapsed().mul_f32(self.cpu);
        if self.cpu > 1.0 {
            // Busy, as the device would be: spin rather than sleep.
            while work.elapsed() < cost {
                std::hint::spin_loop();
            }
        }
        cost
    }
}

/// Once a second: frames drawn, what a frame costs, and how busy the
/// processor was. An idle screen should read 0 fps and about 0%.
#[derive(Default)]
struct Meter {
    since: Option<Instant>,
    frames: u32,
    busy: Duration,
    frame_cost: Duration,
}

impl Meter {
    fn record(&mut self, cost: Duration, drew: bool) -> Option<String> {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.busy += cost;
        if drew {
            self.frames += 1;
            self.frame_cost += cost;
        }
        let elapsed = since.elapsed();
        if elapsed < Duration::from_secs(1) {
            return None;
        }
        let frame_ms = if self.frames > 0 { self.frame_cost.as_secs_f32() * 1e3 / self.frames as f32 } else { 0.0 };
        let line = format!(
            "{:.0} fps · {frame_ms:.1} ms a frame · processor {:.0}% busy",
            self.frames as f32 / elapsed.as_secs_f32(),
            (self.busy.as_secs_f32() / elapsed.as_secs_f32() * 100.0).min(100.0)
        );
        *self = Meter::default();
        Some(line)
    }
}

/// How even motion is, not how fast on average: for each kind of transition,
/// the time between one frame and the next as the handheld would see it.
/// A transition that averages 40 frames a second but stalls for one long
/// frame looks worse than a steady 30, so this reports the first frame, the
/// typical frame and the worst frame separately.
///
/// Time is simulated: each frame's measured cost is scaled to the handheld's
/// speed, the panel's time is added, and the next frame is told that much
/// time has passed, exactly as the device's own loop would.
fn smooth(mut shell: Shell) {
    use Key::*;
    let steps: [(&str, Key); 5] = [("app opens", Enter), ("app closes", Back), ("app opens", Enter), ("page slides in", Enter), ("page slides out", Back)];
    let settle = |shell: &mut Shell| {
        for _ in 0..240 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    // Settings: an app with pages inside it.
    for key in [Right, Right, Right, Right] {
        shell.key(key);
    }
    settle(&mut shell);
    for name in ["device", "worse"] {
        let speed = Speed::named(Some(name));
        let mut beat = Beat::new();
        println!("{}: processor x{}, panel {} ms a full frame", speed.name, speed.cpu, speed.panel.as_millis());
        println!("  {:<16} {:>6} {:>10} {:>10} {:>10} {:>9}", "", "frames", "first", "typical", "worst", "even");
        for (label, key) in steps {
            // Several runs, because a 0.5 ms frame on a PC is easily disturbed.
            let (mut firsts, mut rest, mut counts) = (Vec::new(), Vec::new(), Vec::new());
            for _ in 0..7 {
                let mut shown = Vec::new();
                platform::changed_rows(&mut shown, &shell.gfx.canvas);
                shell.key(key);
                let mut dt = 0.0f32;
                let mut frames = 0usize;
                loop {
                    let started = Instant::now();
                    let drew = shell.tick(dt);
                    if drew {
                        shell.draw();
                    }
                    let work = started.elapsed().as_secs_f32() * speed.cpu;
                    let part = platform::changed_rows(&mut shown, &shell.gfx.canvas).map_or(0.0, |(first, end)| (end - first) as f32 / gfx::H);
                    // The beat in force once this frame's needs are known.
                    let beats = if part > 0.0 { beat.frame_took(Duration::from_secs_f32(work + speed.panel.as_secs_f32() * part)) } else { 1 };
                    let interval = beat.interval().as_secs_f32() * beats as f32;
                    if frames == 0 { firsts.push(interval) } else { rest.push(interval) }
                    frames += 1;
                    dt = interval;
                    if !shell.animating() || frames > 400 {
                        break;
                    }
                }
                counts.push(frames);
                beat.rest();
                settle(&mut shell);
                // Undo, so every run starts from the same place.
                let undo = if matches!(key, Enter) { Back } else { Enter };
                shell.key(undo);
                settle(&mut shell);
            }
            // Leave the step done, for the next one.
            shell.key(key);
            settle(&mut shell);
            let median = |v: &mut Vec<f32>| {
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() / 2] * 1e3
            };
            let worst = rest.iter().cloned().fold(0.0f32, f32::max) * 1e3;
            let typical = median(&mut rest);
            // How many frames came exactly a typical frame after the one before.
            let even = rest.iter().filter(|i| (**i * 1e3 - typical).abs() < 0.5).count() as f32 / rest.len().max(1) as f32;
            counts.sort();
            println!(
                "  {label:<16} {:>6} {:>8.0}ms {:>8.0}ms {:>8.0}ms {:>8.0}%",
                counts[counts.len() / 2],
                median(&mut firsts),
                typical,
                worst,
                even * 100.0
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: f32) -> Duration {
        Duration::from_secs_f32(v / 1000.0)
    }

    #[test]
    fn the_beat_slows_to_one_the_frames_hold_and_stays_there() {
        let mut beat = Beat::new();
        assert_eq!(beat.interval(), ms(25.0));
        // Frames that fit keep the fastest beat.
        for _ in 0..100 {
            beat.frame_took(ms(21.0));
        }
        assert_eq!(beat.level, 0);
        // One late frame is let go: it takes two beats, and the beat stays.
        assert_eq!(beat.frame_took(ms(27.0)), 2);
        assert_eq!(beat.level, 0);
        for _ in 0..20 {
            assert_eq!(beat.frame_took(ms(21.0)), 1);
        }
        // Two late frames close together: the beat is too fast, so down a step.
        beat.frame_took(ms(27.0));
        beat.frame_took(ms(21.0));
        assert_eq!(beat.frame_took(ms(27.0)), 1, "which the slower beat holds");
        assert_eq!(beat.level, 1);
        // Frames hovering around the faster beat do not send it back and forth.
        for i in 0..200 {
            beat.frame_took(ms(if i % 3 == 0 { 26.0 } else { 24.0 }));
        }
        assert_eq!(beat.level, 1, "it settles rather than hunts");
        // Frames that keep running late take it down further, a step at a time.
        for _ in 0..4 {
            beat.frame_took(ms(60.0));
        }
        assert_eq!(beat.level, 3);
        // Only a long run with real room to spare brings it back up, a step at a time.
        for _ in 0..39 {
            beat.frame_took(ms(20.0));
        }
        beat.rest();
        assert_eq!(beat.level, 3);
        // And never mid-animation: only once motion has stopped.
        beat.frame_took(ms(20.0));
        assert_eq!(beat.level, 3);
        beat.rest();
        assert_eq!(beat.level, 2);
    }
}
