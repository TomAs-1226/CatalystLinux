//! catalyst-shell: the GUI of Catalyst Linux.
//!
//! ```text
//! catalyst-shell                 on the device: /dev/fb0 and evdev keys
//! catalyst-shell --sim           a desktop window showing the 320 × 170 panel
//! catalyst-shell --shots <dir>   render a scripted tour to PNG files
//! catalyst-shell --bench         draw 600 frames off screen; print time and memory
//!     --robot <team|ip|host>     robot to connect to (overrides the saved one)
//!     --demo                     a scripted robot instead of a real one
//!     --light, --calm            start in light mode / the calm version
//!     --text <size>              compact, standard or large
//! ```

mod agent;
mod gfx;
mod logs;
mod platform;
mod robot;
mod screens;
mod shell;
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
    let text = format!("light={}\ncalm={}\ntext={}\n", settings.light, settings.calm, settings.text.name());
    std::fs::write(settings_path(), text).ok();
}

fn load_settings(settings: &mut shell::Settings) {
    let Ok(text) = std::fs::read_to_string(settings_path()) else { return };
    for line in text.lines() {
        match line.split_once('=') {
            Some(("light", v)) => settings.light = v == "true",
            Some(("calm", v)) => settings.calm = v == "true",
            Some(("text", v)) => settings.text = gfx::TextSize::from_name(v).unwrap_or(settings.text),
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
    shell.key(Key::Char('\0')); // apply the settings

    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--shots") {
        return shots(shell, std::path::Path::new(&dir));
    }

    if flag("--bench") {
        return bench(shell);
    }

    let mut platform: Box<dyn Platform> = open_platform(flag("--sim"));
    let mut last = Instant::now();
    loop {
        // Idle costs nothing: wake for keys, or four times a second for data.
        let timeout = if shell.animating() { Duration::from_millis(4) } else { Duration::from_millis(250) };
        let Some(keys) = platform.poll(timeout) else { break };
        for key in keys {
            shell.key(key);
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;
        let changed = shell.tick(dt);
        if changed {
            shell.draw();
        }
        if changed || platform.present_always() {
            platform.present(&shell.gfx.canvas);
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
        ("09-system", &[Back, Right, Enter]),
        ("10-home-page-2-light", &[Enter, Back]),
    ];
    let frame = |shell: &mut Shell| {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    };
    let started = Instant::now();
    let mut frames = 0u32;
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
    // One frame caught mid page turn, to show the motion.
    shell.key(Key::Enter);
    for _ in 0..7 {
        frame(&mut shell);
    }
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("11-page-turn.png"), 3);
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
    println!("{}", memory_line().unwrap_or_else(|| "no /proc here".into()));
}
