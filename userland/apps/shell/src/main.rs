//! catalyst-shell: the GUI of Catalyst Linux.
//!
//! ```text
//! catalyst-shell                 on the device: /dev/fb0 and evdev keys
//! catalyst-shell --sim           a desktop window showing the 320 × 170 panel
//! catalyst-shell --shots <dir>   render a scripted tour to PNG files
//! catalyst-shell --bench         draw 600 frames off screen; print time and memory
//! catalyst-shell --smooth        how even the frames of each transition are, at the
//!                                handheld's estimated speed and at worse
//! catalyst-shell --passcode-bench   what making and checking the lock screen's
//!                                passcode takes on this processor
//! catalyst-shell --standby-shots <dir>   StandBy's faces and the lock screen, to PNG
//! catalyst-shell --cjk-shots <dir>   Chinese, Japanese and Korean text on the screens, to PNG
//! catalyst-shell --activity-shots <dir>  live activities, Tab held, the lock screen with a song
//! catalyst-shell --notify-shots <dir>    notifications: the centre, Quiet, the island, the lock screen
//!     --robot <team|ip|host>     robot to connect to (overrides the saved one)
//!     --demo                     a scripted robot instead of a real one
//!     --arrive <seconds>         the scripted robot, plugged in that long after start
//!     --no-intro                 straight to Home, without the start animation
//!     --setup                    first-boot setup again, as if never set up
//! catalyst-shell --start-movie <file.png>   the start animation as an animated PNG
//!     --light, --calm            start in light mode / the calm version
//!     --text <size>              compact, standard or large
//!     --accent <name>            orange, blue, green, violet or rose
//!     --speed <device|worse|pc>  (simulator) run as slowly as the handheld would,
//!                                or slower; the window title shows the cost
//!     --perf                     also print that cost once a second
//! ```
//!
//! Built with the `packed` feature (each release's `catalyst-sim.exe`), it is
//! the simulator for a computer with nothing installed: it always opens the
//! window, on the demo robot, at the computer's full speed in step with its
//! screen (`--speed device` for the handheld's pacing).

// The packed simulator is a windowed program on Windows: double-clicked, no
// console comes up beside it.
#![cfg_attr(all(windows, feature = "packed"), windows_subsystem = "windows")]

mod about;
mod agent;
mod ai;
mod bluetooth;
mod browse;
mod apps;
mod apriltags;
mod brand;
mod camera;
mod calc;
mod catalogue;
mod changelog;
#[cfg(not(target_arch = "aarch64"))]
mod cjk_shots;
mod clock;
mod commands;
mod connect;
mod control;
mod deep;
mod device;
mod faces;
mod files;
#[cfg(not(target_arch = "aarch64"))]
mod gallery3d;
mod fullheight;
mod gfx;
mod git;
mod github;
mod flow;
mod glance;
mod gpu;
mod graphs;
mod help;
mod hex;
mod http;
mod hwcheck;
mod imu;
mod intro;
mod ir;
mod irremote;
mod keys;
mod lights;
mod liveactivity;
mod level;
mod local;
mod lock;
mod lockpages;
mod logs;
mod look;
mod monitor;
mod music;
mod nettools;
mod notes;
mod notify;
mod nowplaying;
mod object3d;
mod offline;

// The tests count each thread's heap, for what the offline answers cost.
#[cfg(test)]
#[global_allocator]
static METERED: offline::meter::Metered = offline::meter::Metered;
mod panel;
mod passcode;
mod photos;
mod pit;
mod power;
mod pty;
mod platform;
mod recents;
mod reminders;
mod remote;
mod robot;
mod robot3d;
mod schedule;
mod screens;
mod screenshot;
mod setup;
mod share;
mod shell;
mod sound;
mod spotlight;
mod standby;
mod sys;
mod tailscale;
mod sysupdate;
mod usb;
mod term;
mod terminal;
mod tools;
mod trends;
mod units;
mod upnext;
mod voice;
mod weather;
mod wifi;
mod wpilog;

use std::time::{Duration, Instant};

use platform::{Key, Platform};
use robot::Robot;
use shell::Shell;

fn config_path() -> std::path::PathBuf {
    std::env::var_os("CATALYST_CONF").map(Into::into).unwrap_or_else(|| "/etc/catalyst/robot".into())
}

/// The folder the handheld's own state is kept in, beside the robot address:
/// `/data/catalyst` on the device.
pub fn config_dir() -> std::path::PathBuf {
    config_path().parent().map(std::path::Path::to_path_buf).unwrap_or_default()
}

/// Allow a public key to sign in over SSH: one line added to the keys
/// Dropbear reads (`/data/ssh/authorized_keys`), unless it is there already.
pub fn add_ssh_key(key: &str) -> std::io::Result<()> {
    let key = key.trim();
    if key.contains(['\n', '\r']) || !["ssh-ed25519 ", "ssh-rsa ", "ecdsa-sha2-"].iter().any(|p| key.starts_with(p)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a public key"));
    }
    let dir = if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() { std::path::PathBuf::from("/data/ssh") } else { config_dir().join("ssh") };
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("authorized_keys");
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();
    if text.lines().any(|l| l.trim() == key) {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(key);
    text.push('\n');
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    sys::write_private(&file, text.as_bytes())
}

/// First-boot setup has been finished (or put off), so it is not shown again.
pub fn setup_done() -> bool {
    config_dir().join("setup-done").exists()
}

pub fn mark_setup_done() {
    std::fs::create_dir_all(config_dir()).ok();
    sys::write_atomic(&config_dir().join("setup-done"), b"").ok();
}

/// Remember the robot address across boots. False if it could not be written.
pub fn save_robot_address(address: &str) -> bool {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    sys::write_atomic(&path, address.as_bytes()).is_ok()
}

/// Appearance settings live beside the robot address, on the data partition.
fn settings_path() -> std::path::PathBuf {
    config_path().with_file_name("settings")
}

pub fn save_settings(settings: &shell::Settings) {
    sys::write_atomic(&settings_path(), settings_text(settings).as_bytes()).ok();
}

/// The settings as the file keeps them: one `name=value` a line.
fn settings_text(s: &shell::Settings) -> String {
    let lines = [
        ("light", s.light.to_string()),
        ("calm", s.calm.to_string()),
        ("text", s.text.name().to_owned()),
        ("accent", s.accent.key()),
        ("theme", s.theme.name().to_owned()),
        ("wallpaper", s.wallpaper.name().to_owned()),
        ("corners", s.corners.name().to_owned()),
        ("letters", s.letters.name().to_owned()),
        ("width", s.width.name().to_owned()),
        ("columns", s.columns.to_string()),
        ("home", s.home.name().to_owned()),
        ("open_robot", s.open_robot.to_string()),
        ("match_report", s.match_report.to_string()),
        ("glance", s.glance.iter().map(|w| w.name()).collect::<Vec<_>>().join(",")),
        ("pinned", s.pinned.clone().unwrap_or_default()),
        // App names cannot hold a line break, and commas are rare: a '|' between them.
        ("order", s.order.join("|")),
        ("hidden", s.hidden.join("|")),
        ("favourites", s.favourites.join("|")),
        ("clock24", s.clock24.to_string()),
        ("seconds", s.seconds.to_string()),
        ("island", s.island.name().to_owned()),
        ("greeting", s.greeting.to_string()),
        ("robot_name", s.robot_name.clone().unwrap_or_default()),
        ("graphics", (if s.gpu { "auto" } else { "processor" }).to_owned()),
        ("developer", s.developer.to_string()),
        ("motion_scale", s.motion_scale.to_string()),
        ("show_keys", s.show_keys.to_string()),
        ("toured", s.toured.to_string()),
        ("screen_off", s.screen_off.to_string()),
        ("power", s.power.key().to_owned()),
        ("lift_wake", s.lift_wake.to_string()),
        ("face_down", s.face_down.to_string()),
        ("brightness", s.brightness.to_string()),
    ];
    lines.iter().chain(s.battery.lines().iter()).chain(s.lock.lines().iter()).map(|(k, v)| format!("{k}={v}\n")).collect()
}

fn load_settings(settings: &mut shell::Settings) {
    if let Ok(text) = std::fs::read_to_string(settings_path()) {
        parse_settings(&text, settings);
    }
}

/// Read settings from their file's text; anything not understood is left as it was.
fn parse_settings(text: &str, settings: &mut shell::Settings) {
    for line in text.lines() {
        if line.split_once('=').is_some_and(|(k, v)| settings.battery.parse(k, v) || settings.lock.parse(k, v)) {
            continue;
        }
        match line.split_once('=') {
            Some(("light", v)) => settings.light = v == "true",
            Some(("calm", v)) => settings.calm = v == "true",
            Some(("text", v)) => settings.text = gfx::TextSize::from_name(v).unwrap_or(settings.text),
            Some(("accent", v)) => settings.accent = bezel::Accent::from_name(v).unwrap_or(settings.accent),
            Some(("home", v)) => settings.home = shell::HomeStyle::from_name(v).unwrap_or(settings.home),
            Some(("open_robot", v)) => settings.open_robot = v != "false",
            Some(("match_report", v)) => settings.match_report = v != "false",
            Some(("glance", v)) => settings.glance = shell::Widget::list_from(v),
            Some(("pinned", v)) => settings.pinned = Some(v.trim().to_owned()).filter(|k| !k.is_empty()),
            Some(("theme", v)) => settings.theme = look::Theme::from_name(v).unwrap_or(settings.theme),
            Some(("wallpaper", v)) => settings.wallpaper = look::Wallpaper::from_name(v).unwrap_or(settings.wallpaper),
            Some(("corners", v)) => settings.corners = look::Corners::from_name(v).unwrap_or(settings.corners),
            Some(("letters", v)) => settings.letters = look::Letters::from_name(v).unwrap_or(settings.letters),
            Some(("width", v)) => settings.width = look::Width::from_name(v).unwrap_or(settings.width),
            Some(("columns", v)) => settings.columns = v.parse::<usize>().ok().filter(|c| (2..=3).contains(c)).unwrap_or(settings.columns),
            Some(("order", v)) => settings.order = v.split('|').filter(|n| !n.is_empty()).map(str::to_owned).collect(),
            Some(("hidden", v)) => settings.hidden = v.split('|').filter(|n| !n.is_empty()).map(str::to_owned).collect(),
            Some(("favourites", v)) => settings.favourites = v.split('|').filter(|n| !n.is_empty()).map(str::to_owned).collect(),
            Some(("clock24", v)) => settings.clock24 = v != "false",
            Some(("seconds", v)) => settings.seconds = v == "true",
            Some(("island", v)) => settings.island = shell::IslandRest::from_name(v).unwrap_or(settings.island),
            Some(("greeting", v)) => settings.greeting = v != "false",
            Some(("robot_name", v)) => settings.robot_name = Some(v.trim().to_owned()).filter(|n| !n.is_empty()),
            Some(("graphics", v)) => settings.gpu = v != "processor",
            Some(("developer", v)) => settings.developer = v == "true",
            Some(("motion_scale", v)) => {
                settings.motion_scale = v.parse::<f32>().ok().filter(|s| shell::MOTION_SCALES.contains(s)).unwrap_or(settings.motion_scale)
            }
            Some(("show_keys", v)) => settings.show_keys = v == "true",
            Some(("toured", v)) => settings.toured = v == "true",
            Some(("screen_off", v)) => {
                settings.screen_off = v.parse::<u32>().ok().filter(|s| standby::SCREEN_OFF_CHOICES.contains(s)).unwrap_or(settings.screen_off)
            }
            Some(("power", v)) => settings.power = standby::Power::from_key(v).unwrap_or(settings.power),
            Some(("brightness", v)) => settings.brightness = v.parse::<u32>().ok().filter(|b| (10..=100).contains(b)).unwrap_or(settings.brightness),
            Some(("lift_wake", v)) => settings.lift_wake = v != "false",
            Some(("face_down", v)) => settings.face_down = v != "false",
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

/// Where the simulator keeps what it saves on a computer: the user's own
/// folder for programs' data, never the computer's `/etc` or `/data`.
fn user_state_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("CatalystLinux"))
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/CatalystLinux"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".local/share"))).map(|d| d.join("catalyst-linux"))
    }
}

/// Run from a terminal, the packed simulator prints there (`--about`, `--bench`).
#[cfg(all(windows, feature = "packed"))]
fn attach_console() {
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: a plain Win32 call; failing (double-clicked, no console) is fine.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    #[cfg(all(windows, feature = "packed"))]
    attach_console();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // The packed simulator is always the simulator, double-clicked or started
    // from a terminal with options of its own (`--no-intro`, `--light`). It
    // runs at the computer's full speed, in step with its screen, unless asked
    // for the handheld's pacing (`--speed device`), which is only an estimate
    // scaled from the computer it runs on.
    if cfg!(feature = "packed") && !args.iter().any(|a| a == "--sim") {
        args.push("--sim".into());
    }
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    // What making and checking the lock screen's passcode takes here.
    // A demo event's match logs (seven matches with known faults: brownouts,
    // a hot elevator, loop overruns, CAN spikes), written where Logs reads
    // them, to try Logs, Trends and the offline Assistant without a robot.
    // `catalyst-shell --demo-logs` on the handheld; a folder may be given.
    if flag("--demo-logs") {
        let dir = value("--demo-logs").filter(|v| !v.starts_with("--")).map(std::path::PathBuf::from).unwrap_or_else(logs::kept_dir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("catalyst-shell: making {}: {e}", dir.display());
            std::process::exit(1);
        }
        for spec in offline::synth::demo_event() {
            let path = dir.join(&spec.name);
            match std::fs::write(&path, offline::synth::write(&spec)) {
                Ok(()) => println!("{}", path.display()),
                Err(e) => {
                    eprintln!("catalyst-shell: writing {}: {e}", path.display());
                    std::process::exit(1);
                }
            }
        }
        return;
    }
    if flag("--passcode-bench") {
        println!("{}", passcode::bench());
        return;
    }
    // The offline assistant's evaluation: its questions asked of the model
    // at CATALYST_LOCAL_URL (or the facts alone), the results to a file.
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(path) = value("--offline-eval") {
        offline::eval::main(std::path::Path::new(&path));
        return;
    }
    // StandBy's and the lock screen's "next reminder": from Reminders.
    upnext::set_reminders(reminders::next_for_upnext);

    // The simulator on a computer saves in the user's folder unless told otherwise.
    if flag("--sim") && std::env::var_os("CATALYST_CONF").is_none() {
        if let Some(dir) = user_state_dir() {
            std::env::set_var("CATALYST_CONF", dir.join("robot"));
            if std::env::var_os("CATALYST_LOGS").is_none() {
                std::env::set_var("CATALYST_LOGS", dir.join("logs"));
            }
        }
    }

    // The packed simulator starts on the demo robot every time, whatever
    // address setup or Settings saved; `--robot` connects a real one.
    let saved = std::fs::read_to_string(config_path()).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).filter(|_| !cfg!(feature = "packed"));
    // The demo robot plugged in a while after start, to see what its arrival does.
    let mut arrive = value("--arrive").and_then(|s| s.parse::<f32>().ok()).map(|s| Instant::now() + Duration::from_secs_f32(s.max(0.0)));
    let robot = match (flag("--demo"), value("--robot").or(saved)) {
        _ if arrive.is_some() => Robot::none(),
        (true, _) => Robot::demo(),
        (_, Some(input)) if input == "demo" => Robot::demo(),
        (_, Some(input)) => Robot::connect(&screens::resolve_address(&input)),
        // The packed simulator has a robot to show until it is given a real one.
        _ if cfg!(feature = "packed") => Robot::demo(),
        _ => Robot::none(),
    };
    // The scripted tour brings its own files and apps to show.
    #[cfg(not(target_arch = "aarch64"))]
    if flag("--shots") || flag("--voice-shots") || flag("--cjk-shots") {
        tour_fixtures();
    }
    // The voice screens keep their state in a folder of their own: a tour
    // run at the same time clears the shared one as it starts.
    #[cfg(not(target_arch = "aarch64"))]
    if flag("--voice-shots") {
        let own = std::env::temp_dir().join(format!("catalyst-voice-shots-{}", std::process::id()));
        std::fs::create_dir_all(&own).ok();
        std::env::set_var("CATALYST_CONF", own.join("robot"));
    }
    // The motion sensor starts on a thread of its own (its program takes
    // most of a second over the bus), so Home knows what it offers.
    imu::start();
    // Reminders chime through the speaker.
    sound::start();
    let mut shell = Shell::new(robot);
    load_settings(&mut shell.settings);
    shell.settings.light |= flag("--light");
    shell.settings.calm |= flag("--calm");
    // Everything on the processor, to compare against the GPU.
    shell.settings.gpu &= !flag("--cpu");
    if let Some(size) = value("--text").and_then(|name| gfx::TextSize::from_name(&name)) {
        shell.settings.text = size;
    }
    if let Some(accent) = value("--accent").and_then(|name| bezel::Accent::from_name(&name)) {
        shell.settings.accent = accent;
    }
    shell.apply_settings();

    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--shots") {
        return shots(shell, std::path::Path::new(&dir));
    }
    // Every screen at the panel's whole height, in a look at a text size.
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--full-height") {
        tour_fixtures();
        let size = value("--text").and_then(|name| gfx::TextSize::from_name(&name)).unwrap_or(gfx::TextSize::Standard);
        let look = value("--look").unwrap_or_else(|| "catalyst".into());
        return fullheight::gallery(std::path::Path::new(&dir), &look, size);
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--activity-shots") {
        return activity_shots(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--notify-shots") {
        return notify_shots(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--standby-shots") {
        return standby_shots(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--cjk-shots") {
        return cjk_shots::shots(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--looks") {
        return looks(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--motion") {
        return motion(std::path::Path::new(&dir));
    }
    // Flow's previews (Home's album covers): stills, strips and clips.
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--flow-shots") {
        return flow::gallery::shots(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if flag("--flow-bench") {
        return flow::gallery::bench();
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--objects") {
        return gallery3d::run(std::path::Path::new(&dir));
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(path) = value("--start-movie") {
        return start_movie(shell, std::path::Path::new(&path));
    }
    // The app catalogue's screens, from its demo catalogue.
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--catalogue-shots") {
        return catalogue::shots(std::path::Path::new(&dir));
    }
    // The voice screens alone (push-to-talk, dictation, spoken answers),
    // as the end of the full tour (`--shots`) shows them.
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(dir) = value("--voice-shots") {
        std::fs::create_dir_all(&dir).expect("create the screenshot folder");
        for _ in 0..120 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        voice::gallery(&mut shell, std::path::Path::new(&dir));
        // And every voice island over every title it meets, in both looks
        // at each text size: a sheet each.
        for (what, settled) in voice::island_sheets(std::path::Path::new(&dir)) {
            if !settled {
                eprintln!("the title had not settled whole or gone: {what}");
            }
        }
        std::fs::remove_dir_all(config_dir()).ok();
        return;
    }

    if flag("--bench") || flag("--about") {
        // What draws is part of the answer: let the GPU finish opening.
        shell.gfx.wait_gpu();
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
            ("graphics", Some(gpu::note())),
        ];
        for (name, value) in lines {
            println!("{name:>14}: {}", value.unwrap_or_else(|| "–".into()));
        }
        return;
    }

    if !flag("--sim") {
        keep_in_memory();
    }
    let mut platform: Box<dyn Platform> = open_platform(flag("--sim"));
    // The time zone chosen, in use for every clock this draws (on the handheld).
    device::apply_saved();
    // A shell before this one stopped in deep standby: Wi-Fi, USB and the
    // rest brought back.
    deep::recover();
    // The first start sets the handheld up; `--setup` shows that again.
    if flag("--setup") || !setup_done() {
        shell.start_setup();
    }
    // The start the tear-free panel is tried in asks whether to keep it.
    shell.check_the_panel();
    if !flag("--no-intro") {
        shell.start_intro(if flag("--setup") || !setup_done() { "let's get you set up".to_owned() } else { start_caption(&shell) });
    }
    let speed = Speed::named(value("--speed").as_deref());
    // The simulator at full speed keeps time with the computer's own screen
    // (60 pictures a second on most), where the handheld's beat of 40 would
    // land unevenly on its refreshes and judder. Paced as the handheld
    // (`--speed device`), it keeps the handheld's beat.
    #[cfg(not(target_arch = "aarch64"))]
    let mut beat = if flag("--sim") && speed.cpu <= 1.0 && std::env::var_os("CATALYST_FRAME_MS").is_none() {
        Beat::every(platform::sim::screen_frame_ms())
    } else {
        Beat::new()
    };
    #[cfg(target_arch = "aarch64")]
    let mut beat = Beat::new();
    let mut meter = Meter::default();
    let perf = flag("--perf");
    let mut last = Instant::now();
    // While anything moves, frames start on a steady beat, so motion is even
    // rather than as fast as each frame happens to allow. Idle, nothing is
    // drawn at all.
    let mut next_frame = Instant::now();
    // The panel asleep with the screen dark (standby).
    let mut panel_asleep = false;
    // The backlight is dark from power-on until the first picture is sent.
    let mut lit = false;
    loop {
        let moving = shell.animating();
        // Idle, a look four times a second; with the screen dark, every two
        // seconds (a lift wakes it sooner: see platform::wake); in deep or
        // music standby, once a minute (sooner when a reminder or the queue
        // comes first).
        let timeout = if moving {
            next_frame.saturating_duration_since(Instant::now())
        } else if shell.deep_asleep() {
            shell.deep_wait()
        } else if shell.panel_off() {
            standby::DARK_LOOK
        } else {
            Duration::from_millis(250)
        };
        let Some(keys) = platform.poll(timeout) else { break };
        // Keys are taken in whenever they arrive; the frame waits for its beat.
        for key in keys {
            shell.key(key);
        }
        // A key that woke the screen wakes the panel before anything is drawn.
        if panel_asleep && !shell.panel_off() {
            platform.panel_sleep(false);
            panel_asleep = false;
        }
        beat.set_floor(shell.frame_floor());
        if moving && Instant::now() < next_frame {
            continue;
        }
        let work = Instant::now();
        // In deep standby a tick stands for the whole minute that passed,
        // and with the screen dark for the seconds between looks.
        let elapsed = (work - last).as_secs_f32();
        let dt = if shell.deep_asleep() || shell.panel_off() { elapsed } else { elapsed.min(0.25) };
        last = work;
        if arrive.is_some_and(|at| work >= at) {
            arrive = None;
            shell.robot = Robot::demo_back_from_a_match();
        }
        let changed = shell.tick(dt);
        if shell.panel_off() != panel_asleep {
            panel_asleep = !panel_asleep;
            platform.panel_sleep(panel_asleep);
        }
        if changed {
            shell.draw();
        }
        let cost = speed.busy(work);
        let part = if changed || platform.present_always() { platform.present(&shell.gfx.canvas) } else { 0.0 };
        if !lit && part > 0.0 {
            lit = true;
            shell.light_up();
        }
        // An external app that draws for itself: once its page has arrived,
        // the panel is its own until it ends or is left.
        if let Some(app) = shell.take_launch() {
            let result = apps::run(&app, &shell.settings, platform.as_mut(), &shell.gfx.canvas);
            // What it left on the panel: its window closes from that picture.
            let left = platform.capture();
            shell.launch_ended(result, left);
            beat.rest();
            last = Instant::now();
            next_frame = last;
            continue;
        }
        if let Some(line) = meter.record(cost, part > 0.0) {
            platform.set_status(&format!("Catalyst Linux · {} · {line}", speed.name));
            if perf {
                eprintln!("{line}");
            }
            // Asked for at run time (`touch /run/catalyst-perf`): the same
            // line, timed, into /run/catalyst-perf.log, for a test to read.
            if cfg!(target_os = "linux") && std::path::Path::new("/run/catalyst-perf").exists() {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/run/catalyst-perf.log") {
                    writeln!(f, "{} {line}", chrono::Local::now().format("%H:%M:%S")).ok();
                }
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
/// (25 ms, or `CATALYST_FRAME_MS`), then 30, 20 and 10; the simulator at full
/// speed starts from the computer's screen rate instead. One late frame is
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
    /// No faster than this (the battery saver), or zero.
    floor: Duration,
}

impl Beat {
    /// The handheld's beat: 25 ms, or `CATALYST_FRAME_MS`.
    fn new() -> Beat {
        let ms = std::env::var("CATALYST_FRAME_MS").ok().and_then(|v| v.parse::<f32>().ok()).filter(|ms| (5.0..=100.0).contains(ms)).unwrap_or(25.0);
        Beat::every(ms)
    }

    /// A beat whose fastest rate is a frame every `ms`.
    fn every(ms: f32) -> Beat {
        let base = Duration::from_secs_f32(ms.clamp(4.0, 100.0) / 1000.0);
        Beat { steps: [base, base.mul_f32(4.0 / 3.0), base * 2, base * 4], level: 0, roomy: 0, since_late: u32::MAX, floor: Duration::ZERO }
    }

    /// The battery saver: no faster than a frame every `ms` (none: as fast
    /// as the beat allows). Taken at once when it slows the beat; when it
    /// is lifted, the beat quickens at rest, as ever.
    fn set_floor(&mut self, ms: Option<f32>) {
        // A little under, so 2 × 16.7 ms counts as 30 a second.
        let floor = ms.map_or(Duration::ZERO, |ms| Duration::from_secs_f32((ms - 0.5).max(0.0) / 1000.0));
        if floor == self.floor {
            return;
        }
        self.floor = floor;
        while self.interval() < floor && self.level + 1 < self.steps.len() {
            self.level += 1;
        }
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
        if self.roomy >= 40 && self.level > 0 && self.steps[self.level - 1] >= self.floor {
            self.level -= 1;
            self.roomy = 0;
        }
    }
}

/// The shell's own memory stays in RAM, as it is touched and from then on.
/// Under memory pressure (a model loading, Music finding a song) the kernel
/// would otherwise swap the shell out to zram, and every frame would wait
/// for its pages to come back: the handheld ran at about 10 frames a second
/// so. Its own memory is about 30 MB; other programs swap instead (a lock is
/// not passed on to the programs it starts).
#[cfg(target_arch = "aarch64")]
fn keep_in_memory() {
    if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE | libc::MCL_ONFAULT) } != 0 {
        eprintln!("catalyst-shell: could not keep its memory in RAM: {}", std::io::Error::last_os_error());
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn keep_in_memory() {}

#[cfg(target_arch = "aarch64")]
fn open_platform(_sim: bool) -> Box<dyn Platform> {
    Box::new(platform::device::Device::open().expect("the panel"))
}

#[cfg(not(target_arch = "aarch64"))]
fn open_platform(sim: bool) -> Box<dyn Platform> {
    #[cfg(target_os = "linux")]
    if !sim {
        return Box::new(platform::device::Device::open().expect("the panel"));
    }
    let _ = sim;
    Box::new(platform::sim::Sim::open(gfx::W as usize, gfx::H as usize))
}

/// Files for the Files app and apps for Home to show in the tour.
#[cfg(not(target_arch = "aarch64"))]
fn tour_fixtures() {
    // What the tour saves (the team, a name, a time zone, its settings) goes
    // to a folder of its own, fresh each run: never the computer's `/etc`,
    // and never a simulator's own state.
    let state = std::env::temp_dir().join("catalyst-tour-state");
    std::fs::remove_dir_all(&state).ok();
    std::fs::create_dir_all(&state).ok();
    std::env::set_var("CATALYST_CONF", state.join("robot"));
    let files = std::env::temp_dir().join("catalyst-tour-files");
    std::fs::remove_dir_all(&files).ok();
    std::fs::create_dir_all(files.join("logs")).ok();
    std::fs::write(files.join("notes.txt"), "pit notes, nov 16\n\nelevator ran hot in q14 (80 c)\n  - check the follower's belt\n  - swap battery 3, it sagged to 6 v\ncan_s0 at 71%: move the intake off it\n").ok();
    std::fs::write(files.join("check.sh"), "#!/bin/sh\nping -c 1 10.58.5.2\n").ok();
    std::env::set_var("CATALYST_FILES", &files);
    // Three external apps: a script, one that draws for itself, one in the terminal.
    let apps = std::env::temp_dir().join("catalyst-tour-apps");
    std::fs::remove_dir_all(&apps).ok();
    std::fs::create_dir_all(apps.join("clock")).ok();
    std::fs::create_dir_all(apps.join("processes")).ok();
    std::fs::write(apps.join("check.sh"), "#!/bin/sh\nping -c 1 10.58.5.2\n").ok();
    std::fs::write(apps.join("clock/app"), "name = Big clock\nicon = schedule\nabout = big digits\nrun = ./clock\nscreen = own\n").ok();
    std::fs::write(apps.join("processes/app"), "name = Processes\nicon = memory\nabout = what is running\nrun = top\n").ok();
    std::env::set_var("CATALYST_APPS", &apps);
    // Setup's page for a computer, at the address a handheld on team 5805's cable would have.
    std::env::set_var("CATALYST_SETUP_ADDRESS", "10.58.5.77");
}

/// The line under the name as the handheld starts: whose it is when that is
/// known (the team, and the robot it last linked to), otherwise what it is.
fn start_caption(shell: &Shell) -> String {
    let team = schedule::team_of(&shell.robot.address);
    match (team, &shell.settings.robot_name) {
        (Some(team), Some(name)) => format!("team {team} · {name}"),
        (Some(team), None) => format!("team {team}"),
        _ => format!("{} · cardputer zero", sysupdate::version()),
    }
}

/// The start animation as an animated PNG at the frame beat the handheld
/// keeps (25 ms), from black through to Home's tiles arriving.
#[cfg(not(target_arch = "aarch64"))]
fn start_movie(mut shell: Shell, path: &std::path::Path) {
    const BEAT: f32 = 0.025;
    shell.start_intro(start_caption(&shell));
    let mut frames = Vec::new();
    for _ in 0..((intro::DONE + 1.3) / BEAT) as usize {
        shell.tick(BEAT);
        shell.draw();
        frames.push(shell.gfx.canvas.px.clone());
    }
    platform::sim::save_apng(&frames, gfx::W as usize, gfx::H as usize, path, 2, 25);
    // And stills: the strike, the ignition, the name arriving, the name set.
    for (name, at) in [("1-strike", 0.44), ("2-ignite", 0.66), ("3-rising", 1.27), ("4-name", 2.2)] {
        let i = ((at / BEAT) as usize).min(frames.len() - 1);
        shell.gfx.canvas.px.copy_from_slice(&frames[i]);
        platform::sim::save_png(&shell.gfx.canvas, &path.with_file_name(format!("start-{name}.png")), 3);
    }
    println!("{} frames written to {}", frames.len(), path.display());
}

/// The same screens in a range of looks: each theme, the wallpapers, your
/// own colour, the letter styles, corners and tiles per row.
#[cfg(not(target_arch = "aarch64"))]
fn looks(dir: &std::path::Path) {
    use look::{Corners, Letters, Theme, Wallpaper, Width};
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    // (name, theme, wallpaper, accent, light, letters, width, corners, columns)
    let looks: [(&str, Theme, Wallpaper, bezel::Accent, bool, Letters, Width, Corners, usize); 8] = [
        ("1-catalyst", Theme::Catalyst, Wallpaper::Catalyst, bezel::Accent::Rose, false, Letters::Crisp, Width::Normal, Corners::Standard, 3),
        ("2-catalyst-light", Theme::Catalyst, Wallpaper::Catalyst, bezel::Accent::Rose, true, Letters::Crisp, Width::Normal, Corners::Standard, 3),
        ("3-bezel", Theme::Bezel, Wallpaper::Glow, bezel::Accent::Orange, false, Letters::Crisp, Width::Normal, Corners::Standard, 3),
        ("4-aurora-violet", Theme::Catalyst, Wallpaper::Aurora, bezel::Accent::Violet, false, Letters::Soft, Width::Normal, Corners::Standard, 3),
        ("5-own-teal-deep", Theme::Catalyst, Wallpaper::Deep, bezel::Accent::Hue(36), false, Letters::Crisp, Width::Normal, Corners::Standard, 3),
        ("6-round-big-tiles", Theme::Catalyst, Wallpaper::Catalyst, bezel::Accent::Hue(6), false, Letters::Round, Width::Wide, Corners::Round, 2),
        ("7-square-narrow", Theme::Bezel, Wallpaper::Plain, bezel::Accent::Blue, false, Letters::Crisp, Width::Narrow, Corners::Square, 3),
        ("8-own-green-light", Theme::Bezel, Wallpaper::Aurora, bezel::Accent::Hue(24), true, Letters::Soft, Width::Normal, Corners::Round, 3),
    ];
    for (name, theme, wallpaper, accent, light, letters, width, corners, columns) in looks {
        let mut shell = Shell::new(Robot::demo());
        let s = &mut shell.settings;
        (s.theme, s.wallpaper, s.accent, s.light, s.letters, s.width, s.corners, s.columns) = (theme, wallpaper, accent, light, letters, width, corners, columns);
        shell.apply_settings();
        let settle = |shell: &mut Shell, n: usize| {
            for _ in 0..n {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
        };
        settle(&mut shell, 120);
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-home.png")), 3);
        // The Robot page, and Settings › Appearance.
        shell.key(Enter);
        settle(&mut shell, 120);
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-robot.png")), 3);
        shell.key(Back);
        settle(&mut shell, 120);
        for key in std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter, Down, Enter, Down, Down, Down]) {
            shell.key(key);
            settle(&mut shell, 60);
        }
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-appearance.png")), 3);
    }
    println!("{} looks in {}", looks.len(), dir.display());
}

/// The ring's path over time, plotted: its top and bottom edges (ink), where
/// they come to rest (faint lines), one dot a frame.
#[cfg(not(target_arch = "aarch64"))]
fn plot_ring(path: &[bezel::Rect], beat: f32, file: &std::path::Path) {
    let (w, h) = (320i32, 170i32);
    let mut c = bezel::Canvas::new(w, h);
    c.px.fill(0x0b0b0e);
    let Some(last) = path.last() else { return };
    let (lo, hi) = path.iter().fold((f32::MAX, f32::MIN), |(lo, hi), r| (lo.min(r.y), hi.max(r.bottom())));
    let (lo, hi) = (lo - 4.0, hi + 4.0);
    let y = |v: f32| 10.0 + (v - lo) / (hi - lo) * (h as f32 - 20.0);
    let x = |i: usize| 10.0 + i as f32 * (w as f32 - 20.0) / (path.len().max(2) - 1) as f32;
    let faint = bezel::Color::hex(0x3a3a44);
    for rest in [last.y, last.bottom()] {
        c.rrect(bezel::Rect::new(0.0, y(rest) - 0.5, w as f32, 1.0), 0.0, faint, 1.0);
    }
    // A tick every 100 ms along the bottom.
    let mut t = 0.0;
    while t <= beat * path.len() as f32 {
        let i = (t / beat).round() as usize;
        c.rrect(bezel::Rect::new(x(i) - 0.5, h as f32 - 6.0, 1.0, 6.0), 0.0, faint, 1.0);
        t += 0.1;
    }
    for (i, r) in path.iter().enumerate() {
        c.circle(x(i), y(r.y), 2.0, bezel::Color::hex(0xf2f2f5), 1.0);
        c.circle(x(i), y(r.bottom()), 2.0, bezel::Color::hex(0xe94560), 1.0);
    }
    platform::sim::save_png(&c, file, 3);
}

/// The motion, to be looked at: each piece as a strip of frames 50 ms apart
/// (and as an animated PNG at the handheld's 25 ms beat), and the Home
/// settings as stills.
#[cfg(not(target_arch = "aarch64"))]
fn motion(dir: &std::path::Path) {
    use Key::*;
    const BEAT: f32 = 0.025;
    const PW: usize = gfx::W as usize;
    const PH: usize = gfx::H as usize;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");

    // Frames from now on, one every beat, for `seconds`.
    fn film(shell: &mut Shell, seconds: f32) -> Vec<Vec<u32>> {
        (0..(seconds / BEAT).round() as usize)
            .map(|_| {
                shell.tick(BEAT);
                shell.draw();
                shell.gfx.canvas.px.clone()
            })
            .collect()
    }
    // Some of those frames side by side, three to a row, and the whole run animated.
    fn strip(frames: &[Vec<u32>], at: &[usize], dir: &std::path::Path, name: &str) {
        const GAP: usize = 4;
        let cols = 3;
        let rows = at.len().div_ceil(cols);
        let (sw, sh) = (cols * PW + (cols - 1) * GAP, rows * PH + (rows - 1) * GAP);
        let mut sheet = bezel::Canvas::new(sw as i32, sh as i32);
        sheet.px.fill(0x0b0b0e);
        for (n, &i) in at.iter().enumerate() {
            let frame = &frames[i.min(frames.len() - 1)];
            let (ox, oy) = ((n % cols) * (PW + GAP), (n / cols) * (PH + GAP));
            for y in 0..PH {
                sheet.px[(oy + y) * sw + ox..][..PW].copy_from_slice(&frame[y * PW..][..PW]);
            }
        }
        platform::sim::save_png(&sheet, &dir.join(format!("{name}-strip.png")), 2);
        platform::sim::save_apng(frames, PW, PH, &dir.join(format!("{name}.png")), 2, 25);
    }
    fn still(shell: &mut Shell, dir: &std::path::Path, name: &str) {
        film(shell, 1.5);
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    }
    fn press(shell: &mut Shell, keys: &[Key]) {
        for &key in keys {
            shell.key(key);
            film(shell, 0.6);
        }
    }
    let fresh = |robot: Robot| {
        let mut shell = Shell::new(robot);
        shell.apply_settings();
        film(&mut shell, 1.5);
        shell
    };
    // 50 ms apart from the key, then where it comes to rest.
    let beats = [0, 2, 4, 6, 9, 30];

    // Settings growing out of its tile, its rows rising inside the window.
    let mut shell = fresh(Robot::demo());
    let to_settings = vec![Right; steps_to("Settings")];
    press(&mut shell, &to_settings);
    shell.key(Enter);
    let frames = film(&mut shell, 1.0);
    strip(&frames, &beats, dir, "1-open-settings");
    // A page sliding in, its rows rising one after another.
    press(&mut shell, &[Down, Down, Down]);
    shell.key(Enter);
    let frames = film(&mut shell, 1.0);
    strip(&frames, &beats, dir, "2-page-rises");
    // Enter on a switch: the row takes the press.
    press(&mut shell, &[Down, Down, Down, Down, Down, Down, Down]);
    shell.key(Enter);
    let frames = film(&mut shell, 0.6);
    strip(&frames, &[0, 1, 2, 4, 8, 20], dir, "3-press-row");
    shell.key(Enter);
    film(&mut shell, 0.6);

    // The focus moving across Home: the tile it leaves settles, the next lifts.
    let mut shell = fresh(Robot::demo());
    shell.key(Right);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &beats, dir, "4-focus-lift");

    // The ring going down a list, a frame every beat: it stretches on its
    // way, runs a little past and draws back. Its path is plotted too.
    let every = [0, 1, 2, 3, 4, 5, 6, 8, 12];
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &to_settings);
    shell.key(Enter);
    film(&mut shell, 1.2);
    shell.key(Down);
    let mut path = Vec::new();
    let frames: Vec<Vec<u32>> = (0..24)
        .map(|_| {
            shell.tick(BEAT);
            shell.draw();
            path.push(shell.gfx.last_ring);
            shell.gfx.canvas.px.clone()
        })
        .collect();
    strip(&frames, &every, dir, "4b-ring-down-a-row");
    plot_ring(&path, BEAT, &dir.join("4b-ring-path.png"));
    // Down held: the key repeats every 75 ms and the ring keeps its speed.
    let mut frames = Vec::new();
    for _ in 0..4 {
        shell.key(Down);
        frames.extend(film(&mut shell, 0.075));
    }
    frames.extend(film(&mut shell, 0.5));
    strip(&frames, &[0, 3, 6, 9, 12, 14, 16, 19, 30], dir, "4c-ring-key-held");
    // A page pushed, and Back before it has landed: it turns round where it is.
    press(&mut shell, &[Up, Up, Up, Up, Up]);
    shell.key(Enter);
    let mut frames = film(&mut shell, 0.1);
    shell.key(Back);
    frames.extend(film(&mut shell, 0.8));
    strip(&frames, &[0, 2, 4, 6, 8, 10, 14, 20, 34], dir, "4d-back-while-arriving");
    // Network, then Wi-Fi from it before Network has landed: Network carries on beneath.
    press(&mut shell, &[Down, Down, Down, Down, Down, Down]);
    shell.key(Enter);
    let mut frames = film(&mut shell, 0.1);
    shell.key(Enter);
    frames.extend(film(&mut shell, 0.9));
    strip(&frames, &[0, 2, 4, 6, 8, 10, 14, 20, 38], dir, "4e-push-while-arriving");

    // Home's pages turning: Right past the last tile of the first page.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &[Right, Right, Right, Right, Right]);
    shell.key(Right);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &every, dir, "4f-home-page-turn");
    // Enter on a tile: it presses in as the app grows out of it.
    shell.key(Enter);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &beats, dir, "5-press-tile");
    // Back: the app shrinks into its tile, melting into the tile's own
    // picture on the way, so the tile is what lands.
    shell.key(Back);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &[0, 4, 7, 9, 11, 13, 15, 18, 30], dir, "5b-close-app");

    // The recent apps: Ctrl-Tab over Logs, the cards rising in, Robot's
    // offered first; Enter, and Robot grows out of its card in Logs' place.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &[Enter, Back, Right, Right, Enter]);
    film(&mut shell, 0.6);
    shell.key(Ctrl('\t'));
    let frames = film(&mut shell, 0.8);
    strip(&frames, &beats, dir, "5c-recent-apps");
    shell.key(Enter);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &beats, dir, "5d-recent-app-back");

    // Keys faster than the motion. Health closing, and Logs opened 100 ms
    // in: Health goes on into its tile beneath Logs growing out of its own.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &[Right, Enter]);
    shell.key(Back);
    let mut frames = film(&mut shell, 0.1);
    shell.key(Right);
    shell.key(Enter);
    frames.extend(film(&mut shell, 0.8));
    strip(&frames, &[0, 2, 4, 6, 8, 10, 13, 17, 30], dir, "5e-close-then-open-another");
    // Esc twice, 50 ms apart, over a Settings page: the page slides away
    // inside the window as Settings goes back into its tile.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &to_settings);
    press(&mut shell, &[Enter, Enter]);
    shell.key(Back);
    let mut frames = film(&mut shell, 0.05);
    shell.key(Back);
    frames.extend(film(&mut shell, 0.8));
    strip(&frames, &[0, 2, 4, 6, 8, 10, 13, 17, 30], dir, "5f-esc-twice");
    // Ctrl-Tab 75 ms into opening Logs: it goes on growing behind the cards,
    // and is there, whole, when Esc puts them away.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &[Right, Right]);
    shell.key(Enter);
    let mut frames = film(&mut shell, 0.2);
    shell.key(Ctrl('\t'));
    frames.extend(film(&mut shell, 0.3));
    shell.key(Back);
    frames.extend(film(&mut shell, 0.6));
    strip(&frames, &[6, 8, 10, 13, 16, 20, 22, 26, 40], dir, "5g-ctrl-tab-mid-zoom");

    // A robot back from a match: the Logs tile pulses as the match's log
    // arrives and its warning comes with it.
    let mut shell = fresh(Robot::demo_back_from_a_match());
    let frames = film(&mut shell, 6.0);
    // A plain stretch of the Logs tile (the third across): when it changes, the pulse has begun.
    let spot = |f: &Vec<u32>| f[34 * PW + 262];
    let from = frames.windows(2).position(|w| spot(&w[0]) != spot(&w[1])).unwrap_or(0);
    let pulse: Vec<Vec<u32>> = frames[from..].iter().take(48).cloned().collect();
    strip(&pulse, &[0, 2, 4, 8, 14, 40], dir, "6-tile-reacts");

    // Settings › Home, Arrange apps, and Home arranged.
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &to_settings);
    press(&mut shell, &[Enter, Down, Down, Down, Enter]);
    still(&mut shell, dir, "7-settings-home");
    press(&mut shell, &[Down, Down, Down, Down, Down, Down, Down]);
    still(&mut shell, dir, "8-settings-home-end");
    press(&mut shell, &[Up, Up, Up, Up, Up, Enter]);
    // Schedule moved to the front; Tables hidden; Logs a favourite, so first of all.
    press(&mut shell, &[Down, Down, Down, Left, Left, Left, Down, Down, Down, Down, Enter, Up, Char(' ')]);
    still(&mut shell, dir, "9-arrange-apps");
    // Home, arranged so, from its first page of apps: back from Settings (the
    // twelfth tile now, Tables hidden) to the fourth.
    press(&mut shell, &[Back, Back, Back, Left, Left, Left, Left, Left, Left, Left, Left]);
    still(&mut shell, dir, "10-home-arranged");

    // Settings › Display: brightness, and what draws (here, the processor).
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &to_settings);
    press(&mut shell, &[Enter, Down, Down, Down, Down, Enter, Down]);
    still(&mut shell, dir, "14-settings-display");

    // Settings › System update, part-way through installing (a pretend status).
    let update_dir = config_dir().join("update");
    std::fs::create_dir_all(&update_dir).ok();
    std::fs::write(update_dir.join("status.json"), r####"{"state":"installing","version":"0.3.0","progress":0.46,"detail":"","size":6800000,"checked":0,"notes":"### New\n- Faster glass.\n- Favourites, pinned first on Home."}"####).ok();
    let mut shell = fresh(Robot::demo());
    press(&mut shell, &to_settings);
    press(&mut shell, &[Enter, Down, Down, Down, Down, Down, Down, Down, Down, Enter]);
    still(&mut shell, dir, "15-system-update");
    std::fs::write(update_dir.join("status.json"), r####"{"state":"ready","version":"0.3.0","progress":1.0,"detail":"","size":6800000,"checked":0,"notes":"### New\n- Faster glass.\n- Favourites, pinned first on Home."}"####).ok();
    // The page reads it again every two seconds, of real time.
    std::thread::sleep(std::time::Duration::from_millis(2100));
    film(&mut shell, 0.5);
    still(&mut shell, dir, "16-system-update-ready");
    std::fs::remove_dir_all(&update_dir).ok();

    // The island at rest as a 12-hour clock with seconds.
    let mut shell = fresh(Robot::demo());
    (shell.settings.island, shell.settings.clock24, shell.settings.seconds) = (shell::IslandRest::Clock, false, true);
    shell.apply_settings();
    still(&mut shell, dir, "11-island-clock");

    // A personal start: the team and its robot under the name, and Home's greeting.
    let mut shell = Shell::new(Robot::demo());
    shell.robot.address = "10.58.5.2".into();
    shell.settings.robot_name = Some("atlas".into());
    shell.apply_settings();
    shell.start_intro(start_caption(&shell));
    film(&mut shell, 2.4);
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("12-start-personal.png"), 3);
    film(&mut shell, intro::DONE - 2.4 + 0.9);
    platform::sim::save_png(&shell.gfx.canvas, &dir.join("13-greeting.png"), 3);
    println!("motion in {}", dir.display());
}

/// Settings › On-device model: its row, the page with nothing downloaded,
/// a download part way, the default in use with an answer it gave, and the
/// fastest and smartest focused (stand-in files of the models' sizes,
/// accepted as checked). At the text size given, else the one set.
#[cfg(not(target_arch = "aarch64"))]
fn model_shots(dir: &std::path::Path, text: Option<gfx::TextSize>) {
    use Key::*;
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    let models = config_dir().join("models");
    std::fs::remove_dir_all(&models).ok();
    std::env::set_var("CATALYST_MODELS", &models);
    let mut shell = Shell::new(Robot::demo());
    if let Some(size) = text {
        shell.settings.text = size;
    }
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Settings")], 20);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &vec![Down; screens::settings_row("On-device model")], 20);
    save(&mut shell, "112-settings-on-device-model");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "112b-on-device-model");
    shell.show(Box::new(local::ModelPage::showing_download(localmodel::DEFAULT, 0.37)));
    settle(&mut shell, 90);
    save(&mut shell, "112c-on-device-model-downloading");
    // The default and the fastest here, the default in use.
    std::fs::create_dir_all(&models).ok();
    for model in [&localmodel::MODELS[localmodel::DEFAULT], &localmodel::MODELS[0]] {
        if let Ok(stand_in) = std::fs::File::create(localmodel::partial(&models, model)) {
            stand_in.set_len(model.size).ok();
        }
        localmodel::accept(&models, model, model.sha256).ok();
    }
    localmodel::set_chosen(&models, &localmodel::MODELS[localmodel::DEFAULT]).ok();
    shell.show(Box::new(local::ModelPage::showing_answer("A brownout is when the battery sags so low that the robot's controller cuts power to the motors to keep itself running.")));
    settle(&mut shell, 90);
    save(&mut shell, "112d-on-device-model-answer");
    shell.show(Box::new(local::ModelPage::new().showing_loaded(191)));
    settle(&mut shell, 90);
    save(&mut shell, "112g-on-device-model-loaded");
    shell.show(Box::new(local::ModelPage::new().focused_on(0)));
    settle(&mut shell, 90);
    save(&mut shell, "112e-on-device-model-fastest");
    shell.show(Box::new(local::ModelPage::new().focused_on(localmodel::MODELS.len() - 1)));
    settle(&mut shell, 90);
    save(&mut shell, "112f-on-device-model-smartest");
    std::fs::remove_dir_all(&models).ok();
    std::env::remove_var("CATALYST_MODELS");
}

/// The assistant offline: with no key it answers on this handheld, from the
/// demo robot's match logs. The questions it offers, an answer and what it
/// rests on, a follow-up, a match's report (no model), two matches compared,
/// a hot motor, the robot just now, its choices; light, and the large text
/// size. With `CATALYST_LOCAL_URL` naming a llama-server (and
/// `CATALYST_LOCAL_MODEL` its name), its model words the answers, and the
/// pictures are named "-worded".
#[cfg(not(target_arch = "aarch64"))]
fn offline_shots(dir: &std::path::Path) {
    use Key::*;
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let worded = if std::env::var_os("CATALYST_LOCAL_URL").is_some() { "-worded" } else { "" };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}{worded}.png")), 3);
    };
    let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    // Until the answer is in (on its thread; a model takes a few seconds).
    let answered = |shell: &mut Shell| {
        for _ in 0..800 {
            std::thread::sleep(Duration::from_millis(20));
            settle(shell, 2);
            if !shell.animating() {
                break;
            }
        }
        settle(shell, 60);
    };
    let ask = |shell: &mut Shell, text: &str| {
        for c in text.chars() {
            shell.key(Char(c));
        }
        settle(shell, 4);
        shell.key(Enter);
    };
    keys::Keys::default().save().ok();
    std::fs::remove_file(config_dir().join("assistant")).ok();
    std::fs::remove_dir_all(config_dir().join("assistant-chats")).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Assistant")], 20);
    settle(&mut shell, 60);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "120-offline");
    press(&mut shell, &[Down], 40);
    save(&mut shell, "120b-offline-choosing");
    // The question offered: why Q14 browned out, worked out from its log.
    shell.key(Enter);
    settle(&mut shell, 3);
    save(&mut shell, "120c-offline-working");
    answered(&mut shell);
    save(&mut shell, "121-offline-brownout");
    ask(&mut shell, "and Q12?");
    answered(&mut shell);
    save(&mut shell, "121b-offline-follow-up");
    // The newest match's report: Ctrl R, no model.
    shell.key(Ctrl('r'));
    answered(&mut shell);
    save(&mut shell, "122-offline-report");
    press(&mut shell, &[Up, Up, Up, Up, Up], 40);
    save(&mut shell, "122b-offline-report-scrolled");
    press(&mut shell, &[Up, Up], 40);
    save(&mut shell, "122c-offline-report-top");
    for question in ["compare Q14 and Q12", "any motor running hot?", "how is the robot doing?"] {
        press(&mut shell, &[Ctrl('n')], 30);
        ask(&mut shell, question);
        answered(&mut shell);
        let name = match question.split(' ').next() {
            Some("compare") => "123-offline-compare",
            Some("any") => "123b-offline-hot-motor",
            _ => "123c-offline-robot-now",
        };
        save(&mut shell, name);
    }
    press(&mut shell, &[Tab], 90);
    save(&mut shell, "124-offline-choices");
    press(&mut shell, &[Back], 90);
    // Light, and the large text size.
    shell.settings.light = true;
    shell.apply_settings();
    press(&mut shell, &[Ctrl('n')], 60);
    save(&mut shell, "125-offline-light");
    ask(&mut shell, "why did we brown out in Q14?");
    answered(&mut shell);
    save(&mut shell, "125b-offline-light-answer");
    shell.settings.light = false;
    shell.settings.text = gfx::TextSize::Large;
    shell.apply_settings();
    press(&mut shell, &[Ctrl('n')], 60);
    save(&mut shell, "126-offline-large");
    shell.key(Ctrl('r'));
    answered(&mut shell);
    save(&mut shell, "126b-offline-large-report");
    shell.settings.text = gfx::TextSize::Standard;
    shell.apply_settings();
    std::fs::remove_dir_all(config_dir().join("assistant-chats")).ok();
}
/// Spotlight's part of the tour: growing out of the island, results as they
/// are typed (apps and values, a sum, a conversion, a match, a team), a
/// preview, over an app, a result opening out of its row, the recent picks,
/// light mode and the large text size.
#[cfg(not(target_arch = "aarch64"))]
fn spotlight_shots(dir: &std::path::Path) {
    use Key::*;
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    let typed = |shell: &mut Shell, text: &str| {
        shell.key(Delete);
        for c in text.chars() {
            shell.key(Char(c));
            settle(shell, 2);
        }
        settle(shell, 50);
    };
    // A note, two hosts and the album's photos, for it to find.
    let notes = notes::dir();
    std::fs::create_dir_all(&notes).ok();
    std::fs::write(notes.join("2026-11-20-101500.txt"), "Swerve offsets\n\nfront left 0.412 rad\nfront right 1.873 rad\nback left 2.551 rad\nback right 5.090 rad\n\n— Q31 · nov 20 10:15\n").ok();
    std::fs::write(config_dir().join("ssh-hosts"), "lvuser@10.58.5.2\nthomas@build.local\n").ok();
    photos::seed_demo();
    spotlight::index().refresh(true);
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    // Out of the island: caught growing, then at rest with nothing typed yet.
    shell.key(Ctrl('\n'));
    settle(&mut shell, 6);
    save(&mut shell, "32-spotlight-growing");
    settle(&mut shell, 60);
    save(&mut shell, "32b-spotlight");
    typed(&mut shell, "lo");
    save(&mut shell, "32c-spotlight-lo");
    typed(&mut shell, "12*7");
    save(&mut shell, "32d-spotlight-sum");
    typed(&mut shell, "3 in to mm");
    save(&mut shell, "32e-spotlight-units");
    typed(&mut shell, "q31");
    save(&mut shell, "32f-spotlight-match");
    typed(&mut shell, "254");
    save(&mut shell, "32g-spotlight-team");
    // Tab: a robot value's preview, then a note's (read on its thread).
    typed(&mut shell, "volts");
    shell.key(Tab);
    settle(&mut shell, 40);
    save(&mut shell, "32h-spotlight-preview-value");
    shell.key(Left);
    typed(&mut shell, "swerve");
    shell.key(Tab);
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(10));
        settle(&mut shell, 2);
    }
    settle(&mut shell, 40);
    save(&mut shell, "32i-spotlight-preview-note");
    shell.key(Back);
    settle(&mut shell, 8);
    save(&mut shell, "32j-spotlight-back-into-the-island");
    settle(&mut shell, 90);
    // Over an app: Logs. A setting found, and opened out of its row.
    for key in [Right, Right, Enter] {
        shell.key(key);
        settle(&mut shell, 60);
    }
    shell.key(Ctrl('\n'));
    settle(&mut shell, 40);
    typed(&mut shell, "bright");
    save(&mut shell, "32k-spotlight-over-an-app");
    shell.key(Enter);
    settle(&mut shell, 7);
    save(&mut shell, "32l-spotlight-opening");
    settle(&mut shell, 90);
    save(&mut shell, "32m-spotlight-opened");
    // Back to Home; with nothing typed, the picks made lately.
    for _ in 0..3 {
        shell.key(Back);
        settle(&mut shell, 60);
    }
    shell.key(Ctrl('\n'));
    settle(&mut shell, 60);
    save(&mut shell, "32n-spotlight-recent");
    shell.key(Back);
    settle(&mut shell, 90);
    // Light mode, and the large text size.
    shell.settings.light = true;
    shell.apply_settings();
    settle(&mut shell, 60);
    shell.key(Ctrl('\n'));
    settle(&mut shell, 40);
    typed(&mut shell, "net");
    save(&mut shell, "32o-spotlight-light");
    shell.key(Back);
    settle(&mut shell, 90);
    shell.settings.light = false;
    shell.settings.text = gfx::TextSize::Large;
    shell.apply_settings();
    settle(&mut shell, 60);
    shell.key(Ctrl('\n'));
    settle(&mut shell, 40);
    typed(&mut shell, "wi");
    save(&mut shell, "32p-spotlight-large-text");
}

/// A scripted tour: press keys, let motion settle, save each screen.
#[cfg(not(target_arch = "aarch64"))]
fn shots(mut shell: Shell, dir: &std::path::Path) {
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    // Only Spotlight's part, when that is all that is wanted.
    if std::env::var("CATALYST_SHOTS_ONLY").as_deref() == Ok("spotlight") {
        return spotlight_shots(dir);
    }
    if std::env::var("CATALYST_SHOTS_ONLY").as_deref() == Ok("offline") {
        return offline_shots(dir);
    }
    // Or only Music's.
    if std::env::var("CATALYST_SHOTS_ONLY").as_deref() == Ok("music") {
        let settle = |shell: &mut Shell, frames: usize| {
            for _ in 0..frames {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
        };
        let save = |shell: &mut Shell, name: &str| {
            shell.draw();
            platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
        };
        let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
            for key in keys {
                shell.key(*key);
                settle(shell, frames);
            }
        };
        return music_shots(&mut shell, &press, &save, &settle);
    }
    // From Files to Settings, however many apps lie between.
    let files_to_settings: Vec<Key> =
        [Ctrl('q'), Back].into_iter().chain(std::iter::repeat_n(Right, steps_to("Settings") - steps_to("Files"))).chain([Enter]).collect();
    // From the Wi-Fi password back out to Settings' list (on Network), and on to About.
    let network_to_about: Vec<Key> = [Back, Back, Back]
        .into_iter()
        .chain(std::iter::repeat_n(Down, screens::settings_row("About this handheld") - screens::settings_row("Network")))
        .chain([Enter])
        .collect();
    // Along Home's tiles from one app to another, by name: Left and Right go
    // through them in turn, so apps added between never throw the tour off.
    let walk = |from: &str, to: &str| -> Vec<Key> {
        let (a, b) = (steps_to(from), steps_to(to));
        vec![if b >= a { Right } else { Left }; a.abs_diff(b)]
    };
    let typed = |text: &str| -> Vec<Key> { text.chars().map(|c| if c == '\n' { Enter } else { Char(c) }).collect() };
    // Down Settings' list to a section, by name.
    let down_to = |from: &str, to: &str| vec![Down; screens::settings_row(to) - screens::settings_row(from)];
    let elevator = robot_tables(&shell.robot).iter().position(|t| t == "Elevator").expect("the demo robot's Elevator table");
    let tour: Vec<(&str, Vec<Key>)> = vec![
        ("01-home", vec![]),
        ("02-robot", vec![Enter]),
        ("03-health", [vec![Back], walk("Robot", "Health"), vec![Enter]].concat()),
        ("04-logs", [vec![Back], walk("Health", "Logs"), vec![Enter]].concat()),
        ("04b-match", vec![Enter]),
        ("04c-tables", [vec![Back, Back], walk("Logs", "Tables"), vec![Enter]].concat()),
        ("05-tables-elevator", [vec![Down; elevator], vec![Enter]].concat()),
        ("06-loop", [vec![Back, Back], walk("Tables", "Loop"), vec![Enter]].concat()),
        ("06b-schedule", [vec![Back], walk("Loop", "Schedule"), vec![Enter]].concat()),
        ("07-motors", [vec![Back], walk("Schedule", "Motors"), vec![Enter, Down]].concat()),
        ("08-console", [vec![Back], walk("Motors", "Console"), vec![Enter]].concat()),
        ("08b-terminal", [vec![Back], walk("Console", "Terminal"), vec![Enter], typed("ls\ncolors\nuname\n")].concat()),
        ("08c-files", [vec![Ctrl('q')], walk("Terminal", "Files"), vec![Enter]].concat()),
        ("08d-editor", vec![Down, Down, Enter, Down, Down, Right, Right, Right, Right]),
        ("09-settings", files_to_settings),
        ("09b-appearance", [down_to("Robot settings", "Appearance"), vec![Enter]].concat()),
        ("09c-light-blue", vec![Down, Enter, Down, Enter, Enter]),
        ("09d-network", [vec![Back], down_to("Appearance", "Network"), vec![Enter]].concat()),
        ("09e-wifi", vec![Enter, Down, Down, Down]),
        ("09f-wifi-password", [vec![Enter], typed("catalyst7")].concat()),
        ("09g-about", network_to_about),
        ("10-home-page-3-light", vec![Back, Back]),
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
    for (name, keys) in &tour {
        for key in keys.iter() {
            shell.key(*key);
            // Mid-turn frames show the page spring; settle before the next key.
            for _ in 0..90 {
                frame(&mut shell);
                frames += 1;
            }
        }
        shell.apply_settings();
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
    // External apps: their tiles on Home, one in the terminal, one that draws for itself.
    let apps: &[(&str, &[Key])] = &[("13-home-apps", &[Back, Back, Right]), ("14-app-in-terminal", &[Enter]), ("15-app-own-screen", &[Ctrl('q'), Right, Enter])];
    for (name, keys) in apps {
        for key in keys.iter() {
            shell.key(*key);
            for _ in 0..90 {
                frame(&mut shell);
            }
        }
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    }
    // The control centre, caught coming down over Home and then at rest.
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    shell.key(Back);
    settle(&mut shell, 90);
    shell.key(Ctrl(' '));
    settle(&mut shell, 7);
    save(&mut shell, "16-control-coming-down");
    settle(&mut shell, 90);
    shell.key(Down);
    settle(&mut shell, 90);
    save(&mut shell, "17-control-centre");
    shell.key(Back);
    settle(&mut shell, 90);
    // The glance page: one step left of the first tile.
    for _ in 0..20 {
        if shell.top_title() == "Glance" {
            break;
        }
        shell.key(Left);
        settle(&mut shell, 90);
    }
    save(&mut shell, "18-glance");
    // The other Home: the carousel, at rest and caught turning.
    shell.key(Right);
    shell.settings.home = shell::HomeStyle::Carousel;
    shell.apply_settings();
    settle(&mut shell, 90);
    save(&mut shell, "19-carousel");
    shell.key(Right);
    settle(&mut shell, 6);
    save(&mut shell, "20-carousel-turning");
    // A robot back from a match plugged in while Home is showing: the Robot
    // app, caught opening by itself; then, once the match's log is pulled,
    // that match's summary. Then the switches for both, in Settings › Robot.
    let mut shell = Shell::new(Robot::none());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.robot = Robot::demo_back_from_a_match();
    settle(&mut shell, 9);
    save(&mut shell, "21-robot-arriving");
    settle(&mut shell, 60 * 5);
    save(&mut shell, "21b-match-report");
    let keys = [Back, Back].into_iter().chain(std::iter::repeat_n(Right, steps_to("Settings"))).chain([Enter, Enter]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 90);
    }
    save(&mut shell, "22-robot-settings");
    // First-start setup, page by page.
    let mut shell = Shell::new(Robot::none());
    shell.apply_settings();
    shell.start_setup();
    let pages: &[(&str, &[Key])] = &[
        ("23-setup-welcome", &[]),
        ("24-setup-computer", &[Down, Enter]),
        // The hardware check: each part on a chip, then one looked at.
        // Wi-Fi first (its radio switched on as the page arrives), then on.
        ("26-setup-wifi", &[Back, Enter]),
        ("84-setup-hardware", &[Down, Enter]),
        ("84b-setup-hardware-battery", &[Right, Right]),
        // The keyboard lesson: its five cards, each practised a little.
        ("85-setup-lesson-modifiers", &[Tab]),
        ("85b-setup-lesson-moving", &[Enter, Char('x'), Char('c')]),
        ("85c-setup-lesson-capitals", &[Enter, Char('G'), Char('o'), Char('!')]),
        ("85d-setup-lesson-text-arrows", &[Enter, Left, Left, Left]),
        ("85e-setup-lesson-fn-keys", &[Enter, End, F(5)]),
        ("25-setup-team", &[Enter, Char('5'), Char('8'), Char('0'), Char('5')]),
        // The name (offered with the team in it) and the time zone, chosen by search.
        ("86-setup-name-and-time", &[Enter]),
        ("87-time-zones", &[Enter, Enter]),
        ("87b-time-zone-search", &[Char(' '), Char('n'), Char('e'), Char('w')]),
        ("86b-setup-name-and-time-chosen", &[Enter]),
        ("27-setup-keys", &[Tab, Down]),
        ("28-setup-choices", &[Tab, Down]),
        // Battery and power: the screen off after two minutes.
        ("84c-setup-power", &[Tab, Right]),
        ("29-setup-look", &[Tab, Right, Right]),
        ("30-setup-working", &[Down, Down, Down, Down, Enter]),
    ];
    for (name, keys) in pages {
        for key in keys.iter() {
            shell.key(*key);
            settle(&mut shell, 4);
        }
        settle(&mut shell, if *name == "30-setup-working" { 20 } else { 60 });
        save(&mut shell, name);
    }
    settle(&mut shell, 70);
    save(&mut shell, "31-setup-all-set");
    // Settings › Name & time, as setup left it; then the shortcut list's
    // part on the keyboard, its last.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let (name_row, help_row) = (screens::settings_row("Name & time"), screens::settings_row("Help"));
    for key in std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, name_row)).chain([Enter]) {
        shell.key(key);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 60);
    save(&mut shell, "88-settings-name-and-time");
    // On to Help; its second row, the shortcuts; End, their last part.
    let keys = [Back].into_iter().chain(std::iter::repeat_n(Down, help_row - name_row)).chain([Enter, Down, Enter, End]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 40);
    }
    settle(&mut shell, 60);
    save(&mut shell, "89-shortcuts-keyboard");
    // Spotlight: from Home and over an app, as it is typed into.
    spotlight_shots(dir);
    // Git, with no repositories yet.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let keys = std::iter::repeat_n(Right, steps_to("Git")).chain([Enter]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 60);
    }
    save(&mut shell, "33-git");
    // Signed in to GitHub (the simulator's made-up account): the account, its
    // repositories, one cloned, its page, its history with the branches drawn,
    // a commit, the branches, and a commit's message being typed. Then gone
    // again, so the shots after see Git as before.
    let mut held = keys::Keys::load();
    held.set("github", "ghp_demo0000000000000000000000000000demo");
    held.save().ok();
    std::fs::remove_dir_all(git::code_dir().join("robot-2027")).ok();
    let go = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    go(&mut shell, &[Enter], 60);
    go(&mut shell, &[Down, Enter], 30);
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 40);
    save(&mut shell, "33b-git-account");
    go(&mut shell, &[Back, Down, Enter], 60);
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 40);
    save(&mut shell, "33c-git-your-repositories");
    go(&mut shell, &[Enter], 4);
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        settle(&mut shell, 2);
    }
    save(&mut shell, "33d-git-cloning");
    for _ in 0..80 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        settle(&mut shell, 2);
    }
    settle(&mut shell, 60);
    go(&mut shell, &[Enter], 60);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 30);
    save(&mut shell, "33e-git-repository");
    go(&mut shell, &[Down, Down, Down, Down, Enter], 30);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 40);
    save(&mut shell, "33f-git-history");
    go(&mut shell, &[Down, Down], 30);
    settle(&mut shell, 40);
    save(&mut shell, "33g-git-history-down");
    go(&mut shell, &[Enter], 60);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 30);
    save(&mut shell, "33h-git-commit");
    go(&mut shell, &[Back, Back, Down, Enter], 60);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 4);
    }
    settle(&mut shell, 30);
    save(&mut shell, "33i-git-branches");
    go(&mut shell, &[Back, Up, Up, Up, Enter], 60);
    for c in "Shooter at 4300 rpm".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 30);
    save(&mut shell, "33j-git-commit-message");
    go(&mut shell, &[Back, Up, Up], 30);
    save(&mut shell, "33k-git-repository-top");
    let mut held = keys::Keys::load();
    held.set("github", "");
    held.save().ok();
    crate::github::sign_out();
    std::fs::remove_dir_all(git::code_dir().join("robot-2027")).ok();

    // Tailscale, in Settings › Network (the simulator's made-up tailnet):
    // off, switched on and showing its sign-in code, then signed in with the
    // tailnet's machines; and the code again in Bezel's look, light, at the
    // large text size. Off again after, for the shots that follow.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let keys = std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, screens::settings_row("Network"))).chain([Enter, Down]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 60);
    save(&mut shell, "90-network");
    let real_time = |shell: &mut Shell, seconds: f32| {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            settle(shell, 6);
            std::thread::sleep(Duration::from_millis(40));
        }
    };
    shell.key(Enter);
    real_time(&mut shell, 0.4);
    settle(&mut shell, 60);
    save(&mut shell, "90b-tailscale-off");
    shell.key(Enter);
    real_time(&mut shell, 2.6);
    settle(&mut shell, 30);
    save(&mut shell, "90c-tailscale-sign-in");
    tailscale::demo_signed_in();
    real_time(&mut shell, 2.4);
    settle(&mut shell, 40);
    save(&mut shell, "90d-tailscale-on");
    for key in [Down, Down, Down] {
        shell.key(key);
        settle(&mut shell, 20);
    }
    settle(&mut shell, 40);
    save(&mut shell, "90e-tailscale-machines");
    shell.key(End);
    settle(&mut shell, 30);
    shell.key(Enter);
    settle(&mut shell, 40);
    save(&mut shell, "90f-tailscale-sign-out");
    shell.key(Back);
    tailscale::switch(false).ok();
    let mut shell = Shell::new(Robot::demo());
    (shell.settings.theme, shell.settings.light, shell.settings.text) = (look::Theme::Bezel, true, gfx::TextSize::Large);
    shell.apply_settings();
    settle(&mut shell, 100);
    tailscale::switch(true).ok();
    shell.show(Box::new(tailscale::TailscaleScreen::new()));
    real_time(&mut shell, 2.6);
    settle(&mut shell, 40);
    save(&mut shell, "90g-tailscale-sign-in-bezel-light-large");
    tailscale::switch(false).ok();

    // About, laid out as Android's; seven presses on Build number make a
    // developer, and Developer options appear.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    press(&mut shell, &vec![Right; steps_to("Settings")], 30);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &vec![Down; screens::settings_row("About this handheld")], 20);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "34-about");
    // Down to Build number (past version, status, hardware check, licences…).
    let build_number = crate::about::row_of("Build number").unwrap_or(5);
    press(&mut shell, &vec![Down; build_number], 30);
    press(&mut shell, &[Enter; 4], 12);
    save(&mut shell, "35-build-number");
    press(&mut shell, &[Enter; 3], 12);
    settle(&mut shell, 30);
    save(&mut shell, "35b-a-developer");
    press(&mut shell, &[Back, Down, Enter], 90);
    press(&mut shell, &[Down, Right, Right, Down], 30);
    save(&mut shell, "36-developer-options");
    press(&mut shell, &[Enter, Up], 10);
    save(&mut shell, "37-show-keys");
    press(&mut shell, &[Down, Enter, Up, Left, Left, Back, Up, Enter, Down, Down, Enter], 60);
    save(&mut shell, "38-version");
    // Three quick presses on the version: the Easter egg, caught mid-burst and settled.
    for _ in 0..3 {
        shell.key(Enter);
        settle(&mut shell, 2);
    }
    settle(&mut shell, 70);
    save(&mut shell, "39-easter-egg");
    settle(&mut shell, 60 * 5);
    save(&mut shell, "39b-easter-egg-settled");
    // From the version (third) down to the licences (seventh).
    press(&mut shell, &[Back, Back, Down, Down, Down, Down, Enter], 60);
    save(&mut shell, "40-licences");
    // ? held for five seconds: it repeats as a held key does, and the island counts down.
    shell.key(Char('?'));
    settle(&mut shell, 24);
    for i in 0..80 {
        shell.key(Char('?'));
        settle(&mut shell, 4);
        if i == 40 {
            save(&mut shell, "41a-hold-for-shortcuts");
        }
    }
    settle(&mut shell, 90);
    save(&mut shell, "41-shortcuts");
    press(&mut shell, &[Down; 9], 20);
    save(&mut shell, "41b-shortcuts-home");

    // The AprilTag reader, on the simulated camera (which runs in real time,
    // so the shot waits for its pictures).
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("AprilTag")], 20);
    press(&mut shell, &[Enter], 60);
    for name in ["80-apriltag", "80b-apriltag-later"] {
        std::thread::sleep(std::time::Duration::from_millis(1600));
        settle(&mut shell, 4);
        save(&mut shell, name);
    }

    // The Camera on the simulated one (real time again: its pictures, its
    // photos written on a thread, its QR codes read on another), then the
    // album and a photo in it.
    let wait = |shell: &mut Shell, ms: u64| {
        let until = Instant::now() + std::time::Duration::from_millis(ms);
        while Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(16));
            settle(shell, 1);
        }
    };
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Camera")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "80bc-home-camera");
    press(&mut shell, &[Enter], 60);
    wait(&mut shell, 3000);
    save(&mut shell, "80c-camera");
    // The grid, the three-second timer, and the timer counting down.
    press(&mut shell, &[Char('g'), Char('t')], 30);
    wait(&mut shell, 2600);
    press(&mut shell, &[Enter], 1);
    wait(&mut shell, 1150);
    save(&mut shell, "80d-camera-timer");
    // The shutter's flash, caught as it goes, then the photo kept.
    wait(&mut shell, 1950);
    settle(&mut shell, 4);
    save(&mut shell, "80e-camera-shutter");
    wait(&mut shell, 2500);
    save(&mut shell, "80f-camera-saved");
    // Scan mode, and the Wi-Fi code on the card read.
    press(&mut shell, &[Char('g'), Char('t'), Char('t'), Right], 30);
    wait(&mut shell, 3000);
    save(&mut shell, "80g-camera-scan");
    // The album, on the photo just taken; that photo; closer; its details;
    // deleting it, asked.
    press(&mut shell, &[Left, Down], 1);
    wait(&mut shell, 2000);
    save(&mut shell, "80h-photos");
    press(&mut shell, &[Enter], 1);
    wait(&mut shell, 1500);
    save(&mut shell, "80i-photo");
    press(&mut shell, &[Char('+'), Right], 1);
    wait(&mut shell, 1500);
    save(&mut shell, "80j-photo-zoomed");
    press(&mut shell, &[Back, Char('i')], 1);
    wait(&mut shell, 1000);
    save(&mut shell, "80k-photo-details");
    press(&mut shell, &[Char('i'), Char('d')], 1);
    wait(&mut shell, 600);
    save(&mut shell, "80l-photo-delete");
    // And the album with nothing in it.
    let kept = photos::dir().with_extension("kept");
    std::fs::rename(photos::dir(), &kept).ok();
    std::fs::create_dir_all(photos::dir()).ok();
    std::fs::write(photos::dir().join(".seeded"), b"").ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Photos")], 20);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "80m-photos-empty");
    std::fs::remove_dir_all(photos::dir()).ok();
    std::fs::rename(&kept, photos::dir()).ok();

    // Remote: the simulator's two remotes, the TV's buttons, one sent, then a
    // button learned from the simulated remote (which waits a moment in real
    // time before its button is pressed), named, and kept. Any remotes the
    // simulator had kept are put back after.
    let remotes_file = config_dir().join("remotes.json");
    let remotes_before = std::fs::read(&remotes_file).ok();
    std::fs::remove_file(&remotes_file).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Remote")], 20);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "90-remote");
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down], 30);
    save(&mut shell, "91-remote-buttons");
    shell.key(Enter);
    settle(&mut shell, 9);
    save(&mut shell, "92-remote-sending");
    press(&mut shell, &[Char('l')], 60);
    save(&mut shell, "93-remote-listening");
    std::thread::sleep(ir::SIMULATED_PRESS + Duration::from_millis(400));
    settle(&mut shell, 60);
    save(&mut shell, "94-remote-learned");
    press(&mut shell, &[Enter], 20);
    for c in "Netflix".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 20);
    save(&mut shell, "94b-remote-naming");
    press(&mut shell, &[Enter], 90);
    settle(&mut shell, 30);
    save(&mut shell, "94c-remote-kept");
    match remotes_before {
        Some(bytes) => std::fs::write(&remotes_file, bytes).ok(),
        None => std::fs::remove_file(&remotes_file).ok(),
    };

    // The tour, as it follows first-boot setup: its first steps.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.start_tour();
    settle(&mut shell, 60);
    save(&mut shell, "42-tour-welcome");
    for (name, step) in [("43-tour-apps", 1), ("44-tour-back", 2), ("45-tour-island", 3), ("46-tour-pages", 4), ("47-tour-end", 8)] {
        while shell.tour_step() != Some(step) {
            shell.key(Enter);
        }
        settle(&mut shell, 60);
        save(&mut shell, name);
    }

    // The recent apps: Robot and Logs used and left, Health in front.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    for key in [Enter, Back, Right, Right, Enter, Back, Left, Enter] {
        shell.key(key);
        settle(&mut shell, 70);
    }
    shell.key(Ctrl(' '));
    settle(&mut shell, 90);
    save(&mut shell, "70-control-centre");
    shell.key(Back);
    settle(&mut shell, 60);
    shell.key(Ctrl('\t'));
    settle(&mut shell, 90);
    assert!(shell.recents_open());
    save(&mut shell, "71-recent-apps");
    shell.key(Right);
    settle(&mut shell, 60);
    save(&mut shell, "72-recent-apps-chosen");
    // Enter: the app grows out of its card.
    shell.key(Enter);
    settle(&mut shell, 6);
    save(&mut shell, "73-recent-app-growing");
    settle(&mut shell, 90);
    // Up lets one go: it rises away as the others close up.
    shell.key(Ctrl('\t'));
    settle(&mut shell, 60);
    shell.key(Up);
    settle(&mut shell, 7);
    save(&mut shell, "74-recent-app-let-go");
    settle(&mut shell, 60);
    shell.key(Back);
    settle(&mut shell, 60);
    // A screenshot from the control centre: a flash, then the island.
    shell.key(Ctrl(' '));
    settle(&mut shell, 90);
    shell.key(Char('s'));
    settle(&mut shell, 4);
    save(&mut shell, "75-screenshot-flash");
    // It is saved on a thread of its own, in real time; then the island says so.
    std::thread::sleep(std::time::Duration::from_millis(400));
    settle(&mut shell, 75);
    save(&mut shell, "76-screenshot-saved");
    // Settings › Display: the screen off after a minute, the processor balanced.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let keys = std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, screens::settings_row("Display"))).chain([Enter, Down, Down, Down]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 40);
    }
    save(&mut shell, "77-display-standby");
    // Standby, dimmed (on a computer the picture dims; on the handheld, the backlight).
    shell.key(Back);
    settle(&mut shell, 60);
    shell.settings.screen_off = 30;
    settle(&mut shell, 60 * 24);
    assert_eq!(shell.standby_state(), standby::State::Dim);
    save(&mut shell, "78-standby-dimmed");

    // Settings › Battery: the charge and its day at the top, then the saver
    // and the rest of the choices (deep standby among them), and the last
    // deep standby beneath; the control centre's saver switch; and the card
    // as the battery runs out. A night of deep standby is kept for them to
    // show, and whatever was kept before is put back after.
    let record = deep::record_path();
    let kept = std::fs::read(&record).ok();
    let night = power::Reading { percent: 41, flow: power::Flow::Discharging, milliamps: Some(180.0), volts: Some(3.8), ..power::Reading::default() };
    let measured = deep::Slept { woke: chrono::Utc::now().timestamp() - 3 * 3600, seconds: 7 * 3600 + 12 * 60, watts: Some(0.684), charging: false };
    deep::keep(&measured);
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let at_battery = std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, screens::settings_row("Battery")));
    for key in at_battery.chain([Enter]) {
        shell.key(key);
        settle(&mut shell, 40);
    }
    settle(&mut shell, 40);
    save(&mut shell, "81-battery");
    // Down to the last deep standby's figures.
    for key in std::iter::once(Enter).chain(std::iter::repeat_n(Down, power::DEEP_ROW + 3)) {
        shell.key(key);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 40);
    save(&mut shell, "81b-battery-below");
    // Back up to Deep standby, its choice moved on to an hour and back.
    for key in std::iter::repeat_n(Up, 3).chain([Right]) {
        shell.key(key);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 40);
    save(&mut shell, "81c-battery-deep-standby");
    shell.key(Left);
    settle(&mut shell, 30);
    shell.key(Ctrl(' '));
    settle(&mut shell, 90);
    save(&mut shell, "82-control-centre-saver");
    shell.key(Back);
    settle(&mut shell, 60);
    shell.show_battery_empty();
    settle(&mut shell, 70);
    save(&mut shell, "83-battery-empty");
    // Battery › Deep standby: what it does and draws. Never measured, an
    // estimate; once it has slept, what it drew.
    std::fs::remove_file(&record).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(deep::DeepPage::new()));
    settle(&mut shell, 90);
    save(&mut shell, "81d-deep-standby-estimate");
    deep::keep(&measured);
    shell.key(Back);
    settle(&mut shell, 60);
    shell.show(Box::new(deep::DeepPage::new()));
    settle(&mut shell, 90);
    save(&mut shell, "81e-deep-standby-measured");
    // A night of it on Home, and a key: the island says how long and what it drew.
    shell.key(Back);
    settle(&mut shell, 90);
    shell.deep_for(7.0 * 3600.0 + 12.0 * 60.0, night);
    shell.key(Char(' '));
    settle(&mut shell, 40);
    save(&mut shell, "81f-deep-standby-woke");
    match &kept {
        Some(bytes) => std::fs::write(&record, bytes).ok(),
        None => std::fs::remove_file(&record).ok(),
    };
    // Set up from a computer with no network yet: Wi-Fi offered there.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(setup::Setup::without_network()));
    settle(&mut shell, 100);
    save(&mut shell, "24b-setup-computer-no-network");
    // And what a computer sent, arriving a row at a time, then all in place.
    let sent = remote::answers(&remote::parse_form(
        "team=5805&ssid1=Workshop&psk1=hunter22&ssid2=5805_Robot&psk2=robotpass&tba=abcdefgh12345678wxyz&nexus=nexus-key-9f2a&ssh_key=ssh-ed25519%20AAAAC3Nza%20me%40laptop&accent=rose&mode=dark&text=standard&home=grid&choices=1&open_robot=on&match_report=on",
    ));
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(setup::Setup::showing_received(&sent)));
    settle(&mut shell, 110);
    save(&mut shell, "24c-setup-receiving");
    settle(&mut shell, 150);
    save(&mut shell, "24d-setup-received");
    // On first start, the pages left for the handheld follow what the
    // computer sent: four steps, the hardware check first, then the lesson.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(setup::Setup::after_computer(&sent)));
    settle(&mut shell, 110);
    save(&mut shell, "24e-setup-after-computer-hardware");
    press(&mut shell, &[Tab], 110);
    save(&mut shell, "24f-setup-after-computer-lesson");
    // The start the tear-free panel is tried in: keep it? (Counting down,
    // a few seconds in.)
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(panel::KeepPanel::new(false)));
    settle(&mut shell, 60 * 7 + 30);
    save(&mut shell, "84d-keep-the-tear-free-panel");
    // Lower voltage, switched on: what it does, and a restart to try it.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(power::TryUndervolt::new()));
    settle(&mut shell, 90);
    save(&mut shell, "84e-try-lower-voltage");

    // Update logs. What an update brings, before it installs (its notes
    // are the newest section of the changelog, as its manifest carries them),
    // and scrolled down; what changed, once, after restarting into it; and
    // About › Update history, with a version's notes, in each look.
    let newest = changelog::history().into_iter().next().expect("the changelog has a version");
    let section = include_str!("../../../../os/CHANGELOG.md").split("\n## ").nth(1).and_then(|s| s.split_once('\n')).map_or(String::new(), |(_, body)| body.to_owned());
    let coming = sysupdate::Status { state: "available".into(), version: newest.version.clone(), size: 14_200_000, notes: section, ..sysupdate::Status::default() };
    // Settings › System update with that update found: its line opens them.
    let update_dir = config_dir().join("update");
    std::fs::create_dir_all(&update_dir).ok();
    let status = serde_json::json!({ "state": "available", "version": coming.version, "size": coming.size, "notes": coming.notes });
    std::fs::write(update_dir.join("status.json"), status.to_string()).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let keys = std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, screens::settings_row("System update"))).chain([Enter, Down, Down]);
    for key in keys {
        shell.key(key);
        settle(&mut shell, 40);
    }
    settle(&mut shell, 40);
    save(&mut shell, "105-settings-update-whats-new");
    shell.key(Enter);
    settle(&mut shell, 100);
    save(&mut shell, "105b-whats-new-before-installing");
    for _ in 0..3 {
        shell.key(Down);
        settle(&mut shell, 20);
    }
    settle(&mut shell, 40);
    save(&mut shell, "105c-whats-new-scrolled");
    std::fs::remove_dir_all(&update_dir).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(changelog::WhatsNew::after_update(&newest.version).expect("the newest has notes")));
    settle(&mut shell, 100);
    save(&mut shell, "106-whats-new-after-updating");
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(changelog::History::new()));
    settle(&mut shell, 100);
    save(&mut shell, "107-update-history");
    for key in [Down, Enter] {
        shell.key(key);
        settle(&mut shell, 60);
    }
    settle(&mut shell, 40);
    save(&mut shell, "107b-release-notes");
    shell.key(Right);
    settle(&mut shell, 8);
    save(&mut shell, "107c-release-notes-turning");
    settle(&mut shell, 60);
    // Bezel's look, light, at the large text size.
    let mut shell = Shell::new(Robot::demo());
    (shell.settings.theme, shell.settings.light, shell.settings.text) = (look::Theme::Bezel, true, gfx::TextSize::Large);
    shell.apply_settings();
    settle(&mut shell, 100);
    shell.show(Box::new(changelog::WhatsNew::history(changelog::history(), 0)));
    settle(&mut shell, 100);
    save(&mut shell, "107d-release-notes-bezel-light-large");

    // The developer's tools, each from a fresh Home. What they read comes
    // in real time (a reading a second, a ping a second), so real time is
    // let pass before each picture.
    let fresh = || {
        let mut shell = Shell::new(Robot::demo());
        shell.apply_settings();
        settle(&mut shell, 100);
        shell
    };
    let open = |shell: &mut Shell, name: &str| {
        for _ in 0..steps_to(name) {
            shell.key(Right);
            settle(shell, 12);
        }
        shell.key(Enter);
        settle(shell, 90);
    };
    let wait = |shell: &mut Shell, seconds: f32| {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            settle(shell, 6);
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let retype = |shell: &mut Shell, text: &str| {
        for _ in 0..40 {
            shell.key(Backspace);
        }
        for c in text.chars() {
            shell.key(Char(c));
        }
        shell.key(Enter);
        settle(shell, 30);
    };
    let mut shell = fresh();
    open(&mut shell, "Monitor");
    wait(&mut shell, 20.0);
    save(&mut shell, "60-monitor");
    press(&mut shell, &[Right], 60);
    save(&mut shell, "61-monitor-processes");
    let mut shell = fresh();
    open(&mut shell, "Net tools");
    save(&mut shell, "62-net-tools");
    press(&mut shell, &[Enter], 30);
    wait(&mut shell, 4.0);
    save(&mut shell, "63-find-the-robot");
    press(&mut shell, &[Back, Down, Enter, Enter], 40);
    retype(&mut shell, "127.0.0.1");
    wait(&mut shell, 6.0);
    save(&mut shell, "64-ping");
    press(&mut shell, &[Back, Down, Enter, Enter], 40);
    retype(&mut shell, "127.0.0.1");
    wait(&mut shell, 3.0);
    save(&mut shell, "65-ports");
    press(&mut shell, &[Back, Down, Enter], 40);
    retype(&mut shell, "localhost");
    wait(&mut shell, 1.5);
    save(&mut shell, "65b-name-lookup");
    press(&mut shell, &[Back, Down, Enter], 40);
    wait(&mut shell, 2.5);
    save(&mut shell, "66-wifi-survey");
    std::fs::write(config_dir().join("ssh-hosts"), "admin@10.58.5.2\npi@build.local:2222\n").ok();
    let mut shell = fresh();
    open(&mut shell, "Connect");
    save(&mut shell, "67-connect");
    press(&mut shell, &[Down, Down, Down, Down, Enter], 30);
    for c in "hello".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    settle(&mut shell, 60);
    save(&mut shell, "68-serial");
    let mut shell = fresh();
    open(&mut shell, "Actions");
    save(&mut shell, "69-actions");
    press(&mut shell, &[Down, Down, Enter], 90);
    save(&mut shell, "69b-action-running");
    press(&mut shell, &[Ctrl('q')], 60);
    press(&mut shell, &[Char('e')], 90);
    save(&mut shell, "69c-action-edit");

    // Live graphs: two of the battery's table graphed from Tables, two more,
    // a minute of them, then paused.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    shell.robot.graphs.clear();
    settle(&mut shell, 100);
    let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    press(&mut shell, &vec![Right; steps_to("Tables")], 20);
    press(&mut shell, &[Enter], 90);
    let systemcore = robot_tables(&shell.robot).iter().position(|t| t == "Systemcore").unwrap_or(0);
    press(&mut shell, &vec![Down; systemcore], 12);
    press(&mut shell, &[Enter, Enter, Down, Down, Down, Enter], 40);
    save(&mut shell, "50-tables-graphing");
    shell.robot.graphs.toggle("Elevator/PositionMeters", true);
    shell.robot.graphs.toggle("Elevator/TemperatureC", true);
    press(&mut shell, &[Char('g')], 90);
    settle(&mut shell, 60 * 62);
    press(&mut shell, &[Down, Down], 30);
    save(&mut shell, "51-graphs");
    press(&mut shell, &[Char(' ')], 120);
    save(&mut shell, "52-graphs-paused");

    // The pit: its two cards, the checklist part ticked, and the match timer.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Pit")], 20);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "53-pit");
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Char('r'), Enter, Down, Enter, Down, Down, Enter, Down, Enter, Down], 20);
    settle(&mut shell, 150);
    save(&mut shell, "54-pit-checklist");
    // A practice match, two shifts into TELEOP, running.
    press(&mut shell, &[Back, Right, Enter], 90);
    press(&mut shell, &[Char('r'), Right, Right, Right, Enter], 20);
    settle(&mut shell, 40);
    save(&mut shell, "55-match-timer");
    press(&mut shell, &[Right, Right, Right, Right], 20);
    settle(&mut shell, 40);
    save(&mut shell, "56-match-timer-end-game");
    press(&mut shell, &[Enter, Back], 90);
    save(&mut shell, "57-pit-timer-card");
    press(&mut shell, &[Enter, Char('r'), Back], 30);

    // Trends: Logs' row for them, then the robot going wrong over an event.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Logs")], 20);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Up], 40);
    save(&mut shell, "58-logs-trends");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "59-trends");
    press(&mut shell, &[Down, Down, Down, Left, Left], 30);
    save(&mut shell, "59b-trends-earlier-match");

    // The Level on the simulated bench, zeroed there, and stood on an edge;
    // the Compass, and a calibration turning; and where lift to wake is.
    // (The simulated sensor keeps real time, so some shots wait for it.)
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Level")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "99-home-level-tile");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "95-level");
    press(&mut shell, &[Char(' ')], 70);
    save(&mut shell, "95b-level-zeroed");
    press(&mut shell, &[Char(' ')], 10);
    imu::simulate_standing(true);
    settle(&mut shell, 120);
    save(&mut shell, "96-level-on-edge");
    imu::simulate_standing(false);
    press(&mut shell, &[Back, Right], 90);
    press(&mut shell, &[Enter], 90);
    std::thread::sleep(std::time::Duration::from_millis(800));
    settle(&mut shell, 30);
    save(&mut shell, "97-compass");
    press(&mut shell, &[Enter], 10);
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(400));
        settle(&mut shell, 6);
    }
    save(&mut shell, "97b-compass-calibrating");
    press(&mut shell, &[Enter, Back], 90);
    press(&mut shell, &[Right, Enter], 90);
    press(&mut shell, &[Down, Down, Down, Down, Enter], 90);
    press(&mut shell, &[Down, Down, Down, Down], 40);
    save(&mut shell, "98-display-lift-to-wake");

    // Weather, on the simulator's own forecast (Sydney, mid-afternoon): its
    // tile, now and the hours, along the hours, the week, a day, and a place
    // searched for.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Weather")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "100-home-weather-tile");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "101-weather");
    press(&mut shell, &[Down, Right, Right, Right, Right, Right, Right, Right], 20);
    settle(&mut shell, 60);
    save(&mut shell, "101b-weather-hours");
    press(&mut shell, &[Down], 90);
    save(&mut shell, "102-weather-week");
    press(&mut shell, &[Down, Down, Enter], 90);
    save(&mut shell, "103-weather-day");
    press(&mut shell, &[Back, Char('l')], 90);
    for c in "san".chars() {
        shell.key(Char(c));
        settle(&mut shell, 6);
    }
    press(&mut shell, &[Enter], 60);
    save(&mut shell, "104-weather-place");
    press(&mut shell, &[Back, Char('u')], 90);
    save(&mut shell, "104b-weather-fahrenheit");

    // The Graphs tile among its page's; Esc from there glides Home back to
    // its first app (caught on the way, and arrived).
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Graphs")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "105-home-graphs-tile");
    shell.key(Back);
    settle(&mut shell, 4);
    save(&mut shell, "105b-home-esc-gliding-back");
    settle(&mut shell, 60);
    save(&mut shell, "105c-home-esc-at-the-start");

    // The assistant, answered by the simulator's scripted service with the
    // demo robot: with no key, waiting for a question, thinking, an answer
    // that looked at the robot and ran an Action, a critical Action waiting
    // for a yes, its choices, and an Action's editor saying what it does.
    keys::Keys::default().save().ok();
    std::fs::remove_file(config_dir().join("assistant")).ok();
    std::fs::remove_dir_all(config_dir().join("assistant-chats")).ok();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Assistant")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "106-home-assistant-tile");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "106b-assistant-no-key");
    press(&mut shell, &[Back], 60);
    let mut held = keys::Keys::default();
    held.set("openai", "sk-proj-demo-7f3a");
    held.save().ok();
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "107-assistant");
    for c in "how is the robot doing?".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 10);
    save(&mut shell, "107b-assistant-typed");
    press(&mut shell, &[Enter], 20);
    save(&mut shell, "107c-assistant-thinking");
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "108-assistant-answer");
    press(&mut shell, &[Up, Up], 40);
    save(&mut shell, "108b-assistant-scrolled-up");
    // A question the rules answer: the web searched and a page read, the
    // answer with its sources under it, and those on a page of their own.
    for c in "what are the autonomous rules?".chars() {
        shell.key(Char(c));
    }
    press(&mut shell, &[Enter], 20);
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "108c-assistant-web-answer");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "108d-assistant-sources");
    press(&mut shell, &[Back], 90);
    let mut actions = commands::load();
    actions.push(commands::Saved { name: "Deploy robot code".into(), command: "cd /data/code/robot && ./gradlew deploy".into(), ask: commands::Ask::Auto });
    commands::save(&actions);
    for c in "deploy the code".chars() {
        shell.key(Char(c));
    }
    press(&mut shell, &[Enter], 20);
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "109-assistant-asks");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "109b-assistant-confirm");
    press(&mut shell, &[Up], 40);
    save(&mut shell, "109c-assistant-confirm-run-it");
    press(&mut shell, &[Back], 90);
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "109d-assistant-skipped");
    press(&mut shell, &[Tab], 90);
    save(&mut shell, "110-assistant-choices");
    press(&mut shell, &[Down, Down, Enter], 90);
    save(&mut shell, "110b-assistant-conversations");
    // Back to Home, on the Assistant's tile (a fourth Esc would take Home
    // to its first app).
    press(&mut shell, &[Back, Back, Back], 90);
    press(&mut shell, &vec![Left; steps_to("Assistant") - steps_to("Actions")], 20);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &vec![Down; actions.len() + 1], 20);
    save(&mut shell, "111-actions-built-in");
    press(&mut shell, &vec![Up; actions.len() + 1], 20);
    press(&mut shell, &vec![Down; actions.len() - 1], 10);
    press(&mut shell, &[Char('e')], 90);
    press(&mut shell, &[Down, Down], 40);
    save(&mut shell, "111b-action-asks-first");

    // Browse, with the same key: places to start from, an address being
    // read, the page it made with links to follow, one chosen, and a
    // question answered with its sources among the links.
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    press(&mut shell, &vec![Right; steps_to("Browse")], 20);
    settle(&mut shell, 60);
    save(&mut shell, "112-home-browse-tile");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "112b-browse-start");
    press(&mut shell, &[Down], 40);
    press(&mut shell, &[Enter], 20);
    save(&mut shell, "112c-browse-reading");
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "112d-browse-page");
    press(&mut shell, &[Down, Down, Down], 40);
    save(&mut shell, "112e-browse-scrolled");
    press(&mut shell, &[Right], 60);
    save(&mut shell, "112f-browse-link");
    press(&mut shell, &[Tab], 30);
    for c in "how long is autonomous this year?".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 10);
    save(&mut shell, "112g-browse-typed");
    press(&mut shell, &[Enter], 20);
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        settle(&mut shell, 8);
    }
    settle(&mut shell, 60);
    save(&mut shell, "112h-browse-answer");
    keys::Keys::default().save().ok();
    std::fs::remove_dir_all(config_dir().join("assistant-chats")).ok();
    std::fs::remove_file(config_dir().join("commands")).ok();
    std::fs::remove_file(config_dir().join("commands-ask")).ok();
    // The assistant again with no key: offline, from the match logs.
    offline_shots(dir);

    model_shots(dir, None);
    // The tools: Calculator (an answer, then programmer mode), Units (torque),
    // Notes (the list, a search), Clock (timers running, the stopwatch with
    // laps, the world), HTTP (a request to the demo robot's agent, its
    // answer and headers), Share (the Wi-Fi as a code), Monitor's process
    // detail, and a file in hex from Files with a match found.
    let notes_dir = config_dir().join("notes");
    std::fs::remove_dir_all(&notes_dir).ok();
    std::fs::create_dir_all(&notes_dir).ok();
    for (file, text) in [
        ("2026-11-20-1012.txt", "Battery 3\nsagged to 6.2 V in Q14: retire it\n\n— Q14 · nov 20 10:12\n"),
        ("2026-11-20-1130.txt", "Elevator belt\nfollower slips at the top · retension\n\n— Q19 · nov 20 11:30\n"),
        ("2026-11-21-0905.txt", "Intake off can_s0\nmove it to can_s1 before Q27\n\n— Q27 · nov 21 09:05\n"),
        ("2026-11-21-0940.txt", "Pit list for Saturday\nzip ties, 10 batteries charged, spare NEO\n\n— nov 21 09:40\n"),
    ] {
        std::fs::write(notes_dir.join(file), text).ok();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    notes::toggle_pin(&notes_dir, "2026-11-20-1012.txt");
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    let home = |shell: &mut Shell| {
        for _ in 0..4 {
            shell.key(Back);
            settle(shell, 30);
        }
    };
    let typed = |shell: &mut Shell, text: &str| {
        for c in text.chars() {
            shell.key(Char(c));
        }
        settle(shell, 12);
    };
    press(&mut shell, &vec![Right; steps_to("Calculator")], 12);
    settle(&mut shell, 60);
    save(&mut shell, "112-home-tools");
    press(&mut shell, &vec![Right; steps_to("Share") - steps_to("Calculator")], 12);
    settle(&mut shell, 60);
    save(&mut shell, "112b-home-tools-more");
    press(&mut shell, &vec![Left; steps_to("Share") - steps_to("Calculator")], 12);
    settle(&mut shell, 60);
    press(&mut shell, &[Enter], 90);
    typed(&mut shell, "2*(3+4)^2 - sqrt(16)");
    save(&mut shell, "113-calculator");
    press(&mut shell, &[Enter], 20);
    typed(&mut shell, "/7");
    save(&mut shell, "113b-calculator-answer");
    press(&mut shell, &[Enter, Tab], 20);
    typed(&mut shell, "0xf0 | 0b1010 << 1");
    press(&mut shell, &[Enter], 20);
    save(&mut shell, "113c-calculator-programmer");
    press(&mut shell, &[Ctrl('w')], 20);
    typed(&mut shell, "~0xff");
    press(&mut shell, &[Enter], 20);
    save(&mut shell, "113d-calculator-64-bit");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("Units")], 12);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "114-units");
    press(&mut shell, &[Down, Down, Down, Enter], 60);
    press(&mut shell, &[Backspace], 4);
    typed(&mut shell, "25");
    save(&mut shell, "114b-units-torque");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("Notes")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down], 30);
    save(&mut shell, "115-notes");
    press(&mut shell, &[Char('/')], 20);
    typed(&mut shell, "can_s");
    save(&mut shell, "115b-notes-search");
    press(&mut shell, &[Back, Back], 30);
    home(&mut shell);
    clock::add_timer(300.0);
    clock::add_timer(150.0);
    clock::add_timer(20.0);
    press(&mut shell, &vec![Right; steps_to("Clock")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down, Enter, Up], 20);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    settle(&mut shell, 30);
    save(&mut shell, "116-clock-timers");
    press(&mut shell, &[Left], 60);
    press(&mut shell, &[Enter], 10);
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(700));
        press(&mut shell, &[Char('l')], 4);
    }
    std::thread::sleep(std::time::Duration::from_millis(430));
    settle(&mut shell, 20);
    save(&mut shell, "116b-clock-stopwatch");
    press(&mut shell, &[Enter, Right, Right], 60);
    save(&mut shell, "116c-clock-world");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("HTTP")], 12);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "117-http");
    press(&mut shell, &[Down, Down, Down, Down, Enter], 10);
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(40));
        settle(&mut shell, 6);
    }
    settle(&mut shell, 90);
    save(&mut shell, "117b-http-response");
    press(&mut shell, &[Tab], 30);
    save(&mut shell, "117c-http-headers");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("Share")], 12);
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "118-share");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "118b-share-wifi-code");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("Monitor")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Right], 60);
    press(&mut shell, &[Down], 20);
    save(&mut shell, "119-monitor-processes");
    press(&mut shell, &[Enter], 90);
    save(&mut shell, "119b-monitor-process");
    home(&mut shell);
    press(&mut shell, &vec![Right; steps_to("Files")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down, Down], 20);
    press(&mut shell, &[Char('h')], 90);
    save(&mut shell, "120-files-hex");
    press(&mut shell, &[Char('/')], 20);
    typed(&mut shell, "elevator");
    press(&mut shell, &[Enter], 60);
    save(&mut shell, "120b-files-hex-found");
    reminder_shots(dir);
    std::fs::remove_dir_all(&notes_dir).ok();
    sound_shots(&mut shell, &press, &save, &settle);
    music_shots(&mut shell, &press, &save, &settle);
    voice::gallery(&mut shell, dir);
    let written =std::fs::read_dir(dir).map_or(0, |d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "png")).count());
    println!("{written} screens in {}, {frames} frames at {:.2} ms each", dir.display(), started.elapsed().as_secs_f64() * 1e3 / frames as f64);
    // An icon asked for and never built in draws as nothing: a blank where
    // a symbol should be. Any in the tour fails it.
    let missing = shell.gfx.missing_icons();
    if !missing.is_empty() {
        eprintln!("icons asked for that are not built in: {}", missing.iter().cloned().collect::<Vec<_>>().join(", "));
        std::process::exit(1);
    }
}

/// Reminders in the gallery, on the demo event: Today with one come due and
/// the pit's cues, the lists turning, Upcoming and Done; quick add reading
/// what is typed; the pit routine; a note's bell in Notes and its reminder;
/// Bezel's light look; and one coming due with the screen off, which wakes
/// it with the island and opens Reminders.
#[cfg(not(target_arch = "aarch64"))]
fn reminder_shots(dir: &std::path::Path) {
    use reminders::parse::Parsed;
    use reminders::{Book, Link, Repeat, When};
    use Key::*;
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    let press = |shell: &mut Shell, keys: &[Key], frames: usize| {
        for key in keys {
            shell.key(*key);
            settle(shell, frames);
        }
    };
    let typed = |shell: &mut Shell, text: &str| {
        for c in text.chars() {
            shell.key(Char(c));
        }
        settle(shell, 12);
    };
    // Two pit notes, one of them to be reminded of (the tour's own folder,
    // emptied first).
    let notes_dir = notes::dir();
    std::fs::remove_dir_all(&notes_dir).ok();
    std::fs::create_dir_all(&notes_dir).ok();
    for (file, text) in [
        ("2026-11-20-1130.txt", "Elevator belt\nfollower slips at the top · retension\n\n— Q19 · nov 20 11:30\n"),
        ("2026-11-21-0905.txt", "Intake off can_s0\nmove it to can_s1 before Q27\n\n— Q27 · nov 21 09:05\n"),
    ] {
        std::fs::write(notes_dir.join(file), text).ok();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(&mut shell, 100);
    // The demo event's reminders, a fresh book of them.
    let view = shell.robot.schedule.view();
    reminders::with(|s| {
        std::fs::remove_file(reminders::path()).ok();
        s.book = Book::open(reminders::path());
        let n = reminders::plan::Now { at: reminders::now(), zone: &reminders::plan::Here, view: view.as_ref() };
        let add = |book: &mut Book, typed: &str| {
            for p in reminders::parse::parse(typed, &n).expect("a reminder") {
                book.add(p, &n);
            }
        };
        s.book.add(Parsed { text: "swap battery".into(), when: When::In(n.at - 150), repeat: Repeat::Never, list: None, link: Link::None }, &n);
        add(&mut s.book, "run the pit checklist 20 min before each match");
        add(&mut s.book, "after each match: check bumpers, download logs");
        add(&mut s.book, "before Q40 charge the spare");
        add(&mut s.book, "buy zip ties #personal");
        add(&mut s.book, "tomorrow 8am robot inspection");
        add(&mut s.book, "every weekday at 07:30 bus to school #personal");
        s.book.add(Parsed { text: "Elevator belt".into(), when: When::Match(reminders::Cue::After, reminders::Of::Next), repeat: Repeat::Never, list: None, link: Link::Note("2026-11-20-1130.txt".into()) }, &n);
        for done in ["charge battery 3", "zip-tie the radio"] {
            let id = s.book.add(Parsed { text: done.into(), when: When::Whenever, repeat: Repeat::Never, list: None, link: Link::None }, &n);
            s.book.tick(id, &n, true);
        }
        s.book.fire(&n, true);
    });
    settle(&mut shell, 30);
    press(&mut shell, &vec![Right; steps_to("Reminders")], 12);
    settle(&mut shell, 40);
    save(&mut shell, "121-home-reminders");
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down], 30);
    save(&mut shell, "121b-reminders-today");
    press(&mut shell, &[Down, Down, Down], 20);
    settle(&mut shell, 30);
    save(&mut shell, "121c-reminders-today-cues");
    shell.key(Right);
    settle(&mut shell, 7);
    save(&mut shell, "121d-reminders-turning");
    settle(&mut shell, 60);
    press(&mut shell, &[Down], 30);
    save(&mut shell, "121e-reminders-upcoming");
    press(&mut shell, &[Right], 60);
    save(&mut shell, "121f-reminders-done");
    // Quick add: the examples, then a line read as it is typed, then several at once.
    press(&mut shell, &[Left, Left], 40);
    press(&mut shell, &[Char('n')], 60);
    save(&mut shell, "122-reminders-new");
    typed(&mut shell, "before Q40 bring the cart");
    settle(&mut shell, 20);
    save(&mut shell, "122b-reminders-new-read");
    for _ in 0.."before Q40 bring the cart".len() {
        shell.key(Backspace);
    }
    typed(&mut shell, "tomorrow 8am check bumpers every day");
    save(&mut shell, "122c-reminders-new-repeat");
    for _ in 0.."tomorrow 8am check bumpers every day".len() {
        shell.key(Backspace);
    }
    typed(&mut shell, "after each match: swap battery, check bumpers, download logs");
    save(&mut shell, "122d-reminders-new-several");
    press(&mut shell, &[Back], 40);
    // The pit routine.
    press(&mut shell, &[Char('p')], 60);
    save(&mut shell, "123-reminders-pit-routine");
    // Notes: the bell on the note with a reminder, and giving another one.
    // (Esc on Home goes back to its first app.)
    for _ in 0..4 {
        shell.key(Back);
        settle(&mut shell, 30);
    }
    // Each note's row in Notes, below its New note row.
    let row = |title: &str| 1 + notes::load(&notes_dir).iter().position(|n| n.title == title).expect("the tour's note");
    press(&mut shell, &vec![Right; steps_to("Notes")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &vec![Down; row("Elevator belt")], 30);
    save(&mut shell, "124-notes-bell");
    press(&mut shell, &[Home], 20);
    press(&mut shell, &vec![Down; row("Intake off can_s0")], 20);
    press(&mut shell, &[Char('r')], 60);
    typed(&mut shell, "before Q40");
    save(&mut shell, "124b-notes-remind");
    for _ in 0..4 {
        shell.key(Back);
        settle(&mut shell, 30);
    }
    // In Bezel's light look.
    shell.settings.theme = look::Theme::Bezel;
    shell.settings.light = true;
    shell.apply_settings();
    press(&mut shell, &vec![Right; steps_to("Reminders")], 12);
    press(&mut shell, &[Enter], 90);
    press(&mut shell, &[Down], 30);
    save(&mut shell, "125-reminders-bezel-light");
    for _ in 0..3 {
        shell.key(Back);
        settle(&mut shell, 30);
    }
    shell.settings.theme = look::Theme::Catalyst;
    shell.settings.light = false;
    shell.apply_settings();
    settle(&mut shell, 30);
    // With the screen off, one comes due: the screen wakes with the island
    // holding it, the keyboard's lights blink, and Reminders opens on it.
    shell.settings.screen_off = 30;
    reminders::with(|s| {
        let n = reminders::plan::Now { at: reminders::now(), zone: &reminders::plan::Here, view: None };
        s.book.add(Parsed { text: "tighten the intake chain".into(), when: When::In(n.at + 2), repeat: Repeat::Never, list: None, link: Link::None }, &n);
    });
    for _ in 0..40 {
        shell.tick(1.0);
    }
    std::thread::sleep(std::time::Duration::from_millis(2100));
    settle(&mut shell, 14);
    save(&mut shell, "126-reminder-due-waking");
    settle(&mut shell, 60);
    save(&mut shell, "126b-reminder-due");
    let missing = shell.gfx.missing_icons();
    if !missing.is_empty() {
        eprintln!("icons asked for that are not built in: {}", missing.iter().cloned().collect::<Vec<_>>().join(", "));
        std::process::exit(1);
    }
    std::fs::remove_file(reminders::path()).ok();
}

/// The tour's sound: the volume beside the brightness in the control centre,
/// Settings › Sound with the microphone listening, and Bluetooth from off,
/// through its one restart, to headphones found, paired and playing.
#[cfg(not(target_arch = "aarch64"))]
fn sound_shots(shell: &mut Shell, press: &dyn Fn(&mut Shell, &[Key], usize), save: &dyn Fn(&mut Shell, &str), settle: &dyn Fn(&mut Shell, usize)) {
    use Key::*;
    let real_time = |shell: &mut Shell, seconds: f32| {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            settle(shell, 6);
            std::thread::sleep(Duration::from_millis(40));
        }
    };
    sound::set_level(sound::Level { volume: 65, muted: false });
    bluetooth::demo_reset();
    *shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(shell, 100);
    // The control centre, the volume bar chosen and turned up a step.
    press(shell, &[Ctrl(' ')], 90);
    press(shell, &[Right, Down, Down], 30);
    press(shell, &[Right], 60);
    save(shell, "140-control-centre-volume");
    // The volume keys over Home: the island says it.
    press(shell, &[Back], 90);
    press(shell, &[Media(platform::Media::VolumeDown)], 20);
    save(shell, "140b-volume-key");
    settle(shell, 200);
    // Settings › Sound.
    press(shell, &vec![Right; steps_to("Settings")], 12);
    press(shell, &[Enter], 90);
    press(shell, &vec![Down; screens::settings_row("Sound")], 20);
    save(shell, "141-settings-sound-row");
    press(shell, &[Enter], 90);
    real_time(shell, 0.4);
    settle(shell, 40);
    save(shell, "141b-sound");
    // Down to the microphone: its level, live.
    press(shell, &[Down, Down, Down, Down], 20);
    real_time(shell, 0.6);
    save(shell, "141c-sound-microphone");
    // Bluetooth: off, then on (it asks for a restart), then the simulator's
    // "restart", the room found, and headphones paired and playing.
    press(shell, &[End], 20);
    press(shell, &[Enter], 90);
    real_time(shell, 0.4);
    settle(shell, 40);
    save(shell, "142-bluetooth-off");
    press(shell, &[Enter], 20);
    real_time(shell, 0.9);
    settle(shell, 40);
    save(shell, "142b-bluetooth-restart");
    press(shell, &[Down, Enter, Enter], 20);
    real_time(shell, 0.8);
    real_time(shell, 2.6);
    settle(shell, 60);
    save(shell, "142c-bluetooth-found");
    press(shell, &[Down, Down, Down], 20);
    press(shell, &[Enter], 20);
    real_time(shell, 0.3);
    save(shell, "142d-bluetooth-pairing");
    real_time(shell, 2.2);
    settle(shell, 60);
    save(shell, "142e-bluetooth-playing");
    // Back on Sound: playing on the speaker just paired.
    press(shell, &[Back], 90);
    real_time(shell, 0.4);
    settle(shell, 40);
    press(shell, &[Home], 20);
    save(shell, "141d-sound-bluetooth");
    // Light, large text: the same page must still fit.
    let mut light = Shell::new(Robot::demo());
    (light.settings.theme, light.settings.light, light.settings.text) = (look::Theme::Bezel, true, gfx::TextSize::Large);
    light.apply_settings();
    settle(&mut light, 100);
    light.show(Box::new(bluetooth::BluetoothScreen::new()));
    real_time(&mut light, 2.6);
    settle(&mut light, 60);
    save(&mut light, "142f-bluetooth-bezel-light-large");
    light.show(Box::new(sound::SoundScreen::new()));
    real_time(&mut light, 0.5);
    settle(&mut light, 60);
    save(&mut light, "141e-sound-bezel-light-large");
    bluetooth::demo_reset();
    let missing = light.gfx.missing_icons();
    if !missing.is_empty() {
        eprintln!("icons asked for that are not built in: {}", missing.iter().cloned().collect::<Vec<_>>().join(", "));
        std::process::exit(1);
    }
}

/// The tour's Music: the start page's quick picks, a search typed and found,
/// a song playing with its cover and the queue after it, paused from Home's
/// keys, and the account page.
#[cfg(not(target_arch = "aarch64"))]
fn music_shots(shell: &mut Shell, press: &dyn Fn(&mut Shell, &[Key], usize), save: &dyn Fn(&mut Shell, &str), settle: &dyn Fn(&mut Shell, usize)) {
    use Key::*;
    let real_time = |shell: &mut Shell, seconds: f32| {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            settle(shell, 6);
            std::thread::sleep(Duration::from_millis(40));
        }
    };
    *shell = Shell::new(Robot::demo());
    shell.apply_settings();
    settle(shell, 100);
    press(shell, &vec![Right; steps_to("Music")], 12);
    settle(shell, 60);
    save(shell, "143-home-music");
    press(shell, &[Enter], 30);
    real_time(shell, 0.9);
    settle(shell, 60);
    save(shell, "143b-music-start");
    // Typing searches.
    for c in "juniper".chars() {
        shell.key(Char(c));
    }
    settle(shell, 30);
    save(shell, "143c-music-search-typing");
    press(shell, &[Enter], 20);
    real_time(shell, 1.1);
    settle(shell, 60);
    save(shell, "143d-music-search-found");
    // Played: the song and its radio after it.
    press(shell, &[Enter], 30);
    real_time(shell, 0.2);
    save(shell, "143e-music-loading");
    real_time(shell, 3.2);
    settle(shell, 30);
    save(shell, "143f-music-playing");
    press(shell, &[Down], 60);
    save(shell, "143g-music-up-next");
    // Over Home: the tile says what plays, and Fn + Q pauses it.
    press(shell, &[Back, Back], 60);
    press(shell, &[Media(platform::Media::PlayPause)], 20);
    save(shell, "143h-music-paused-from-home");
    settle(shell, 200);
    press(shell, &[Enter], 60);
    press(shell, &[Up], 60);
    save(shell, "143i-music-start-playing");
    press(shell, &[End], 30);
    press(shell, &[Enter], 60);
    real_time(shell, 0.6);
    settle(shell, 30);
    save(shell, "143j-music-account");
    // Light, large text: the now-playing page must still fit.
    let mut light = Shell::new(Robot::demo());
    (light.settings.theme, light.settings.light, light.settings.text) = (look::Theme::Bezel, true, gfx::TextSize::Large);
    light.apply_settings();
    settle(&mut light, 100);
    light.show(Box::new(music::MusicScreen::new()));
    real_time(&mut light, 0.5);
    settle(&mut light, 60);
    save(&mut light, "143k-music-paused-bezel-light-large");
    // Its account page: the warning always whole.
    press(&mut light, &[Up, End, Enter], 40);
    real_time(&mut light, 0.4);
    settle(&mut light, 40);
    save(&mut light, "143l-music-account-bezel-light-large");
    // The library on the card: from the song playing, L.
    shell.show(Box::new(music::MusicScreen::new()));
    settle(shell, 60);
    press(shell, &[Char('s'), Char('r')], 20);
    save(shell, "143m-music-shuffle-repeat");
    press(shell, &[Char('s'), Char('r'), Char('r'), Char('l')], 30);
    real_time(shell, 0.4);
    settle(shell, 30);
    save(shell, "143n-music-library");
    press(shell, &[Enter], 40);
    save(shell, "143o-music-artists");
    press(shell, &[Back, Down, Enter], 40);
    real_time(shell, 0.4);
    settle(shell, 30);
    save(shell, "143p-music-albums");
    // Your own album (no cover of its own: one drawn from its name).
    press(shell, &[Down, Down], 20);
    real_time(shell, 0.4);
    settle(shell, 30);
    save(shell, "143q-music-albums-own");
    press(shell, &[Up, Up, Enter], 40);
    real_time(shell, 0.3);
    settle(shell, 30);
    save(shell, "143r-music-album");
    press(shell, &[Back, Back, Down, Down, Enter], 40);
    real_time(shell, 0.3);
    settle(shell, 30);
    save(shell, "143s-music-played");
    // An album found on YouTube Music: Download all, its size, then the downloads.
    press(shell, &[Back, Back, Char('/')], 30);
    for c in "orbit".chars() {
        shell.key(Char(c));
    }
    press(shell, &[Tab], 20);
    save(shell, "143t-music-search-albums-typing");
    press(shell, &[Enter], 20);
    real_time(shell, 0.8);
    settle(shell, 40);
    save(shell, "143u-music-search-albums");
    press(shell, &[Enter], 20);
    real_time(shell, 0.7);
    settle(shell, 40);
    save(shell, "143v-music-album-from-youtube");
    press(shell, &[Down, Down, Enter], 20);
    save(shell, "143w-music-download-confirm");
    press(shell, &[Enter], 40);
    real_time(shell, 0.5);
    settle(shell, 20);
    save(shell, "143x-music-downloading");
    real_time(shell, 4.5);
    settle(shell, 30);
    save(shell, "143y-music-downloads-done");
    // Settings › Music.
    let mut settings = Shell::new(Robot::demo());
    settings.apply_settings();
    settle(&mut settings, 100);
    press(&mut settings, &vec![Right; steps_to("Settings")], 12);
    press(&mut settings, &[Enter], 60);
    press(&mut settings, &vec![Down; screens::settings_row("Music")], 12);
    press(&mut settings, &[Enter], 60);
    save(&mut settings, "143z-settings-music");
    // Offline, Music opens on the library.
    music::polite::OFFLINE_FOR_TESTS.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut offline = Shell::new(Robot::demo());
    offline.apply_settings();
    settle(&mut offline, 100);
    music::stop_for_shots();
    offline.show(Box::new(music::MusicScreen::new()));
    settle(&mut offline, 60);
    save(&mut offline, "143za-music-offline-library");
    press(&mut offline, &[Back], 30);
    for c in "night".chars() {
        offline.key(Char(c));
    }
    press(&mut offline, &[Enter], 40);
    save(&mut offline, "143zb-music-offline-search");
    music::polite::OFFLINE_FOR_TESTS.store(false, std::sync::atomic::Ordering::Relaxed);
    for missing in [light.gfx.missing_icons(), shell.gfx.missing_icons(), settings.gfx.missing_icons(), offline.gfx.missing_icons()] {
        if !missing.is_empty() {
            eprintln!("icons asked for that are not built in: {}", missing.iter().cloned().collect::<Vec<_>>().join(", "));
            std::process::exit(1);
        }
    }
}

/// The robot's top-level tables, as Tables lists them.
#[cfg(not(target_arch = "aarch64"))]
fn robot_tables(robot: &Robot) -> Vec<String> {
    let mut tables: Vec<String> = robot.under("").into_iter().filter_map(|(k, _)| k.split_once('/').map(|(t, _)| t.to_owned())).collect();
    tables.dedup();
    tables
}

/// StandBy's faces (by day, at night, in light mode, turning), the lock
/// screen (waiting, typed into, wrong, shaking, waiting out, opening, each
/// face and colour, with notices), and their Settings pages, the passcode's
/// steps and the editor: all as the simulator draws them, at 9:41.
#[cfg(not(target_arch = "aarch64"))]
fn standby_shots(dir: &std::path::Path) {
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    tour_fixtures();
    photos::seed_demo();
    upnext::set_reminders(upnext::demo_reminder);
    let morning = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(9, 41, 0));
    faces::pretend_time(morning);
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
        println!("{name}");
    };
    let fresh = |light: bool| {
        let mut shell = Shell::new(Robot::demo());
        shell.settings.light = light;
        shell.apply_settings();
        shell
    };
    // Left on the charger: the screen dims, then StandBy comes.
    let to_standby = |shell: &mut Shell| {
        shell.pretend_charger(Some(true));
        shell.settings.screen_off = 30;
        for _ in 0..32 {
            shell.tick(1.0);
        }
        settle(shell, 120);
        assert!(shell.faces_showing(), "StandBy on the charger");
    };
    let mut shell = fresh(false);
    settle(&mut shell, 100);
    to_standby(&mut shell);
    save(&mut shell, "01-clock-digits");
    // The pager tucks away 2.5 s after the last key.
    shell.key(Right);
    settle(&mut shell, 8);
    save(&mut shell, "02-clock-style-turning");
    settle(&mut shell, 200);
    save(&mut shell, "03-clock-dial");
    shell.key(Right);
    settle(&mut shell, 200);
    save(&mut shell, "04-clock-dots");
    shell.key(Left);
    shell.key(Left);
    settle(&mut shell, 60);
    for name in ["04b-nothing-playing", "05-charge", "06-next-match", "07-weather", "08-photos", "09-everything"] {
        shell.key(Down);
        settle(&mut shell, if name == "08-photos" { 30 } else { 200 });
        if name == "08-photos" {
            // The photo is decoded on a thread of its own.
            std::thread::sleep(std::time::Duration::from_millis(600));
            settle(&mut shell, 200);
        }
        save(&mut shell, name);
    }
    // Past the last face: a give, and back.
    shell.key(Down);
    settle(&mut shell, 5);
    save(&mut shell, "10-rubber-band-at-the-end");
    settle(&mut shell, 60);
    shell.key(Up);
    settle(&mut shell, 9);
    save(&mut shell, "11-turning-between-faces");
    settle(&mut shell, 200);
    // Night: red, and the light down (the simulator dims the picture itself).
    for _ in 0..6 {
        shell.key(Up);
    }
    settle(&mut shell, 200);
    faces::pretend_time(chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(23, 18, 0)));
    for _ in 0..6 {
        shell.tick(1.0);
    }
    settle(&mut shell, 300);
    save(&mut shell, "12-night-clock");
    shell.key(Right);
    settle(&mut shell, 200);
    save(&mut shell, "13-night-dial-resting");
    shell.key(Left);
    for _ in 0..6 {
        shell.key(Down);
    }
    settle(&mut shell, 300);
    save(&mut shell, "14-night-everything");
    faces::pretend_time(morning);
    // Light mode is its own design: StandBy goes light too.
    let mut shell = fresh(true);
    shell.settings.lock.face = lock::Face::Clock;
    settle(&mut shell, 100);
    to_standby(&mut shell);
    save(&mut shell, "15-light-clock");
    shell.key(Down);
    shell.key(Down);
    settle(&mut shell, 200);
    save(&mut shell, "16-light-charge");

    // The lock screen: a passcode set, so it starts locked.
    let (phc, _, _) = passcode::make("2468").expect("hashes");
    passcode::keep(&phc).expect("kept");
    let mut shell = fresh(false);
    shell.settings.lock.face = lock::Face::Clock;
    settle(&mut shell, 120);
    assert!(shell.locked());
    save(&mut shell, "20-lock-screen");
    for c in "24".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 4);
    save(&mut shell, "21-lock-typing-dots-growing");
    settle(&mut shell, 60);
    save(&mut shell, "22-lock-typed");
    for c in "99".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    settle(&mut shell, 2);
    save(&mut shell, "23-lock-checking");
    while shell.checking_passcode() {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut shell, 1);
    }
    settle(&mut shell, 4);
    save(&mut shell, "24-lock-wrong-shaking");
    settle(&mut shell, 90);
    save(&mut shell, "25-lock-wrong");
    // Four more wrong: a wait.
    for wrong in ["1111", "2222", "3333", "4444"] {
        for c in wrong.chars() {
            shell.key(Char(c));
        }
        shell.key(Enter);
        while shell.checking_passcode() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            settle(&mut shell, 1);
        }
        settle(&mut shell, 90);
        if wrong == "3333" {
            save(&mut shell, "25b-lock-one-more-before-a-wait");
        }
    }
    save(&mut shell, "26-lock-wait");
    // Kept again: the wrong tries forgotten.
    passcode::keep(&phc).expect("kept");
    let mut shell = fresh(false);
    settle(&mut shell, 120);
    shell.say("time to queue for Q31");
    shell.say("timer done · 5:00");
    settle(&mut shell, 200);
    save(&mut shell, "27-lock-notices");
    shell.settings.lock.previews = true;
    shell.apply_settings();
    settle(&mut shell, 60);
    save(&mut shell, "28-lock-notice-words");
    shell.settings.lock.previews = false;
    for (name, face, colour) in [("29-lock-stacked-ice", lock::LockFace::Stacked, lock::LockColour::Ice), ("30-lock-dots-amber", lock::LockFace::Dots, lock::LockColour::Amber), ("31-lock-centre-accent", lock::LockFace::Centre, lock::LockColour::Accent)] {
        (shell.settings.lock.lock_face, shell.settings.lock.colour) = (face, colour);
        shell.apply_settings();
        settle(&mut shell, 30);
        save(&mut shell, name);
    }
    (shell.settings.lock.lock_face, shell.settings.lock.colour) = (lock::LockFace::Centre, lock::LockColour::White);
    shell.apply_settings();
    // The right one: it lifts away, over where it was.
    for c in "2468".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    while shell.checking_passcode() {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut shell, 1);
    }
    settle(&mut shell, 9);
    save(&mut shell, "32-lock-opening");
    settle(&mut shell, 120);
    save(&mut shell, "33-unlocked-notices-said");
    // The larger text, and the light mode.
    let mut shell = fresh(true);
    settle(&mut shell, 120);
    save(&mut shell, "34-lock-light");
    let mut shell = fresh(false);
    shell.settings.text = gfx::TextSize::Large;
    shell.apply_settings();
    settle(&mut shell, 120);
    save(&mut shell, "35-lock-large-text");
    // StandBy with a passcode: leaving it is the lock screen.
    let mut shell = fresh(false);
    settle(&mut shell, 60);
    for c in "2468".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    while shell.checking_passcode() {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut shell, 1);
    }
    settle(&mut shell, 120);
    to_standby(&mut shell);
    shell.key(Char(' '));
    settle(&mut shell, 12);
    save(&mut shell, "36-standby-giving-way-to-the-lock");
    settle(&mut shell, 120);

    // Settings: the two new sections, Lock screen, the passcode's steps,
    // the editor, StandBy.
    let mut shell = fresh(false);
    settle(&mut shell, 60);
    for c in "2468".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    while shell.checking_passcode() {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut shell, 1);
    }
    settle(&mut shell, 120);
    let to_settings = std::iter::repeat_n(Right, steps_to("Settings")).chain([Enter]).chain(std::iter::repeat_n(Down, screens::settings_row("Lock screen")));
    for key in to_settings {
        shell.key(key);
        settle(&mut shell, 40);
    }
    settle(&mut shell, 60);
    save(&mut shell, "40-settings-list");
    shell.key(Enter);
    settle(&mut shell, 90);
    save(&mut shell, "41-settings-lock-screen");
    for _ in 0..4 {
        shell.key(Down);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 60);
    save(&mut shell, "42-settings-lock-screen-note");
    for _ in 0..4 {
        shell.key(Up);
        settle(&mut shell, 20);
    }
    shell.key(Enter);
    settle(&mut shell, 90);
    save(&mut shell, "43-passcode-current");
    for c in "2468".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    while shell.checking_passcode() {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut shell, 1);
    }
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        settle(&mut shell, 2);
    }
    save(&mut shell, "44-passcode-new");
    for c in "hunt".chars() {
        shell.key(Char(c));
    }
    settle(&mut shell, 60);
    shell.key(Enter);
    settle(&mut shell, 60);
    save(&mut shell, "45-passcode-again");
    for c in "hung".chars() {
        shell.key(Char(c));
    }
    shell.key(Enter);
    settle(&mut shell, 5);
    save(&mut shell, "46-passcode-not-the-same-shaking");
    settle(&mut shell, 50);
    save(&mut shell, "46b-passcode-not-the-same");
    settle(&mut shell, 60);
    shell.key(Back);
    settle(&mut shell, 90);
    // Customise: the lock screen itself, live.
    shell.key(Down);
    settle(&mut shell, 20);
    shell.key(Down);
    settle(&mut shell, 20);
    shell.key(Enter);
    settle(&mut shell, 120);
    save(&mut shell, "47-editor-clock");
    shell.key(Right);
    settle(&mut shell, 60);
    save(&mut shell, "48-editor-clock-stacked");
    shell.key(Down);
    shell.key(Right);
    settle(&mut shell, 60);
    save(&mut shell, "49-editor-colour-ice");
    for _ in 0..3 {
        shell.key(Down);
    }
    shell.key(Left);
    settle(&mut shell, 60);
    save(&mut shell, "50-editor-match-hidden");
    shell.key(Right);
    shell.key(Up);
    shell.key(Up);
    shell.key(Up);
    shell.key(Left);
    shell.key(Up);
    shell.key(Left);
    settle(&mut shell, 30);
    shell.key(Back);
    settle(&mut shell, 90);
    shell.key(Back);
    settle(&mut shell, 90);
    shell.key(Down);
    settle(&mut shell, 40);
    shell.key(Enter);
    settle(&mut shell, 90);
    save(&mut shell, "51-settings-standby");
    for _ in 0..3 {
        shell.key(Down);
        settle(&mut shell, 20);
    }
    settle(&mut shell, 40);
    save(&mut shell, "52-settings-standby-note");
    shell.key(Up);
    settle(&mut shell, 20);
    shell.key(Enter);
    settle(&mut shell, 200);
    save(&mut shell, "53-standby-shown-now");
    passcode::remove().ok();
    faces::pretend_time(None);
    println!("StandBy and lock screen shots in {}", dir.display());
}

/// Live activities in the island (each kind, two at once, opened on one), on
/// the lock screen (a song, two, three; each clock style; the curtain shown
/// by hand; coming down; its parts rising), Tab held (the cue part way),
/// and StandBy (now playing, by night, a card coming down), in both looks
/// and at the largest text, at 9:41.
#[cfg(not(target_arch = "aarch64"))]
fn activity_shots(dir: &std::path::Path) {
    use liveactivity::{Activity, Kind};
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    tour_fixtures();
    photos::seed_demo();
    let morning = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(9, 41, 0));
    faces::pretend_time(morning);
    let cover = photos::list().first().map(|p| p.path.clone());
    let unix = chrono::Utc::now().timestamp();
    let at = std::time::Instant::now();
    let song = |playing: bool| nowplaying::NowPlaying {
        title: "Around the World".into(),
        artist: "Daft Punk".into(),
        album: "Homework".into(),
        cover: cover.clone(),
        position: 83.0,
        duration: 427.0,
        playing,
        at,
    };
    nowplaying::set(Some(song(true)));
    nowplaying::on_control(|_| {});
    // Each provider's activity, from the inputs it reads.
    let music = || liveactivity::music(Some(&song(true))).expect("music");
    let timer = || liveactivity::timers(Some((272.0, 300.0, 1)), None).remove(0);
    let last_minute = || liveactivity::timers(Some((42.0, 300.0, 2)), None).remove(0);
    let stopwatch = || liveactivity::timers(None, Some(754.0)).remove(0);
    let queue_view = |queue: i64, status: Option<&str>| schedule::View {
        team: 5805,
        matches: vec![schedule::Match { key: "q31".into(), label: "Q31".into(), red: vec![1507, 5254, 5805], blue: vec![3044, 6328, 2337], time: Some(unix + 23 * 60), score: None, status: status.map(str::to_owned), queue_at: Some(queue) }],
        ..schedule::View::default()
    };
    let queue = || liveactivity::next_match(Some(&queue_view(unix + 8 * 60 + 12, None)), unix).expect("queue");
    let queue_now = || liveactivity::next_match(Some(&queue_view(unix - 20, None)), unix).expect("queue now");
    let reminder = || liveactivity::reminder(Some(&upnext::Reminder { title: "charge the spares".into(), due: Some(unix + 12 * 60 + 3) }), unix).expect("reminder");
    let update = || liveactivity::update(&sysupdate::Status { state: "downloading".into(), version: "0.6.0".into(), progress: 0.46, ..Default::default() }, None).expect("update");
    let installed = || {
        let t = liveactivity::task(Kind::Download, "Installing Scouting", "from the catalogue");
        t.done("installed");
        let a = liveactivity::gather(&liveactivity::Sources { robot: &Robot::none(), update: &sysupdate::Status::default(), unix }, None).into_iter().find(|a| a.kind == Kind::Download).expect("the install");
        a
    };
    let assistant = || {
        let mut a = liveactivity::from_task(&liveactivity::Task::sample(Kind::Assistant, "Assistant", "thinking", 7.0));
        a.state = "under way".into();
        a
    };
    let robot = || liveactivity::running_robot("Atlas".into(), "teleop".into(), Some(12.4), Some(83.0));
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
        println!("{name}");
    };
    let fresh = |light: bool, text: gfx::TextSize| {
        let mut shell = Shell::new(Robot::demo());
        shell.settings.light = light;
        shell.settings.text = text;
        shell.apply_settings();
        shell
    };
    let island = |name: &str, list: Vec<Activity>, light: bool, text: gfx::TextSize| {
        let mut shell = fresh(light, text);
        settle(&mut shell, 100);
        // The first look opens nothing: as if each had been there a while.
        shell.pretend_activities(Vec::new());
        shell.pretend_activities(list.iter().map(|a| Activity { state: String::new(), ..a.clone() }).collect());
        shell.pretend_activities(list);
        settle(&mut shell, 10);
        let mut shell = shell;
        shell.close_island_for_gallery();
        settle(&mut shell, 90);
        save(&mut shell, name);
    };
    use gfx::TextSize::{Large, Standard};
    island("a01-island-music", vec![music()], false, Standard);
    island("a02-island-timer", vec![timer()], false, Standard);
    island("a03-island-timer-last-minute", vec![last_minute()], false, Standard);
    island("a04-island-queue-coming", vec![queue()], false, Standard);
    island("a05-island-queue-now", vec![queue_now()], false, Standard);
    island("a06-island-reminder", vec![reminder()], false, Standard);
    island("a07-island-update", vec![update()], false, Standard);
    island("a08-island-installed", vec![installed()], false, Standard);
    island("a09-island-assistant", vec![assistant()], false, Standard);
    island("a10-island-robot", vec![robot()], false, Standard);
    island("a11-island-stopwatch", vec![stopwatch()], false, Standard);
    island("a12-island-two-split", vec![timer(), music()], false, Standard);
    island("a13-island-two-light", vec![queue_now(), music()], true, Standard);
    island("a14-island-two-large-text", vec![timer(), music()], false, Large);
    // A change opens the island on its card for a moment, then it closes.
    for (name, list, light, text, frames) in [
        ("a15-island-opening", vec![music()], false, Standard, 6),
        ("a16-island-open-music", vec![music()], false, Standard, 60),
        ("a17-island-open-queue-now", vec![queue_now(), music()], false, Standard, 60),
        ("a18-island-open-light", vec![music()], true, Standard, 60),
        ("a19-island-open-large-text", vec![timer()], false, Large, 60),
    ] {
        let mut shell = fresh(light, text);
        settle(&mut shell, 100);
        shell.pretend_activities(Vec::new());
        shell.pretend_activities(list);
        settle(&mut shell, frames);
        save(&mut shell, name);
    }

    // Tab held: the cue part way round the island, on Home and in the Terminal.
    let mut shell = fresh(false, Standard);
    settle(&mut shell, 100);
    shell.pretend_hold(0.55);
    save(&mut shell, "c01-hold-cue-half");
    shell.pretend_hold(0.92);
    save(&mut shell, "c02-hold-cue-nearly");
    let mut shell = fresh(true, Standard);
    settle(&mut shell, 100);
    shell.pretend_activities(vec![music()]);
    settle(&mut shell, 10);
    shell.close_island_for_gallery();
    settle(&mut shell, 90);
    shell.pretend_hold(0.55);
    save(&mut shell, "c03-hold-cue-light-with-music");
    let mut shell = fresh(false, Standard);
    settle(&mut shell, 100);
    for key in std::iter::repeat_n(Right, steps_to("Terminal")).chain([Enter]) {
        shell.key(key);
        settle(&mut shell, 30);
    }
    settle(&mut shell, 360);
    shell.pretend_hold(0.6);
    save(&mut shell, "c04-hold-cue-terminal");
    shell.pretend_hold(0.0);
    settle(&mut shell, 240);
    save(&mut shell, "c05-terminal-with-activities");

    // The lock screen with live activities: without a passcode first, shown
    // by hand (the curtain), coming down and its parts rising.
    let mut shell = fresh(false, Standard);
    settle(&mut shell, 100);
    shell.pretend_activities(vec![music()]);
    settle(&mut shell, 10);
    shell.close_island_for_gallery();
    settle(&mut shell, 60);
    shell.lock_as_if_held();
    settle(&mut shell, 7);
    save(&mut shell, "b01-lock-coming-down");
    settle(&mut shell, 8);
    save(&mut shell, "b02-lock-parts-rising");
    settle(&mut shell, 90);
    save(&mut shell, "b03-lock-curtain-with-a-song");
    shell.key(Char(' '));
    settle(&mut shell, 12);
    save(&mut shell, "b04-lock-curtain-lifting");
    // A passcode: locked.
    let (phc, _, _) = passcode::make("2468").expect("hashes");
    passcode::keep(&phc).expect("kept");
    for (name, list, light, text, face) in [
        ("b05-lock-song", vec![music()], false, Standard, lock::LockFace::Centre),
        ("b06-lock-song-paused", vec![liveactivity::music(Some(&song(false))).expect("music")], false, Standard, lock::LockFace::Centre),
        ("b07-lock-song-dots", vec![music()], false, Standard, lock::LockFace::Dots),
        ("b08-lock-two", vec![queue_now(), music()], false, Standard, lock::LockFace::Stacked),
        ("b09-lock-three", vec![timer(), music(), update()], false, Standard, lock::LockFace::Centre),
        ("b10-lock-song-light", vec![music()], true, Standard, lock::LockFace::Centre),
        ("b11-lock-song-large-text", vec![music()], false, Large, lock::LockFace::Centre),
        ("b12-lock-two-large-text", vec![timer(), music()], false, Large, lock::LockFace::Centre),
    ] {
        let mut shell = fresh(light, text);
        shell.settings.lock.lock_face = face;
        shell.apply_settings();
        settle(&mut shell, 120);
        shell.pretend_activities(Vec::new());
        shell.pretend_activities(list);
        settle(&mut shell, 120);
        save(&mut shell, name);
    }
    passcode::remove().ok();

    // StandBy: music's face, picked by itself while it plays on the charger.
    for (name, light, night) in [("d01-standby-now-playing", false, false), ("d02-standby-now-playing-light", true, false), ("d03-standby-now-playing-night", false, true)] {
        faces::pretend_time(if night { chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(23, 18, 0)) } else { morning });
        let mut shell = fresh(light, Standard);
        settle(&mut shell, 100);
        shell.pretend_activities(vec![music()]);
        settle(&mut shell, 10);
        shell.close_island_for_gallery();
        shell.pretend_charger(Some(true));
        shell.settings.screen_off = 30;
        for _ in 0..32 {
            shell.tick(1.0);
        }
        settle(&mut shell, 300);
        save(&mut shell, name);
    }
    faces::pretend_time(morning);
    // A timer's last minute on StandBy: its card comes down for a moment.
    let mut shell = fresh(false, Standard);
    settle(&mut shell, 100);
    shell.pretend_charger(Some(true));
    shell.settings.screen_off = 30;
    for _ in 0..32 {
        shell.tick(1.0);
    }
    settle(&mut shell, 200);
    shell.pretend_activities(Vec::new());
    shell.pretend_activities(vec![last_minute()]);
    settle(&mut shell, 70);
    save(&mut shell, "d04-standby-card-coming-down");
    // Nothing playing: music's face says so.
    nowplaying::set(None);
    let mut shell = fresh(false, Standard);
    settle(&mut shell, 100);
    shell.settings.lock.face = lock::Face::Music;
    shell.pretend_charger(Some(true));
    shell.settings.screen_off = 30;
    for _ in 0..32 {
        shell.tick(1.0);
    }
    settle(&mut shell, 200);
    save(&mut shell, "d05-standby-nothing-playing");
    faces::pretend_time(None);
    println!("live activity shots in {}", dir.display());
}

/// Notifications, to PNG: the notification centre (both tones, the Bezel
/// look, every text size, empty, Quiet, scrolled, the page turning), the
/// control centre's Quiet and Notifications, the island's count, the lock
/// screen with notifications (words hidden and shown, one picked, under a
/// song), and the lock screen charging with a song at every text size (the
/// chip that used to clip into the card).
#[cfg(not(target_arch = "aarch64"))]
fn notify_shots(dir: &std::path::Path) {
    use gfx::TextSize::{Compact, Large, Standard};
    use notify::Notification;
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    tour_fixtures();
    photos::seed_demo();
    let morning = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(9, 41, 0));
    faces::pretend_time(morning);
    let cover = photos::list().first().map(|p| p.path.clone());
    let song = nowplaying::NowPlaying {
        title: "Around the World".into(),
        artist: "Daft Punk".into(),
        album: "Homework".into(),
        cover,
        position: 83.0,
        duration: 427.0,
        playing: true,
        at: std::time::Instant::now(),
    };
    let music = || liveactivity::music(Some(&song)).expect("music");
    let now = chrono::Utc::now().timestamp();
    // One from each source, as each would post it, over the last day.
    let samples = || {
        vec![
            Notification::new("Reminders", "checklist", "swap the battery").body("after Q31").urgent().action("reminder:1").at(now - 40),
            Notification::new("Schedule", "flag", "queue for Q31 in 5 min").tag("queue:q31").action("builtin:Schedule").at(now - 3 * 60),
            Notification::new("Assistant", "auto_awesome", "your answer is written").action("builtin:Assistant").at(now - 18 * 60),
            Notification::new("Logs", "insights", "how Q30 went").body("its summary is in Logs").at(now - 26 * 60),
            Notification::new("Music", "download_done", "12 songs saved").body("in the library, to play offline").action("builtin:Music").at(now - 52 * 60),
            Notification::new("Catalogue", "download_done", "Scouting installed").action("builtin:Catalogue").at(now - 2 * 3600),
            Notification::new("Robot", "bolt", "Brownout").body("Atlas's battery at 6.8 V").action("builtin:Robot").at(now - 3 * 3600),
            Notification::new("System update", "download", "update 0.6.2 ready").body("restart in settings").action("setting:System update").at(now - 5 * 3600),
            Notification::new("Clock", "timer", "timer 5 min done").urgent().action("builtin:Clock").at(now - 26 * 3600),
        ]
    };
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
        println!("{name}");
    };
    let fresh = |light: bool, bezel: bool, text: gfx::TextSize, notes: usize| {
        let mut shell = Shell::new(Robot::demo());
        shell.settings.light = light;
        if bezel {
            (shell.settings.theme, shell.settings.wallpaper, shell.settings.accent) = (look::Theme::Bezel, look::Wallpaper::Glow, bezel::Accent::Orange);
        }
        shell.settings.text = text;
        shell.apply_settings();
        shell.notes.clear();
        for n in samples().into_iter().take(notes) {
            shell.notes.add(n, false);
        }
        settle(&mut shell, 100);
        shell
    };

    // The notification centre: Ctrl-Backspace, over Home.
    for (name, light, bezel, text, notes) in [
        ("n01-centre", false, false, Standard, 9),
        ("n02-centre-light", true, false, Standard, 9),
        ("n03-centre-bezel", false, true, Standard, 9),
        ("n04-centre-bezel-light", true, true, Standard, 9),
        ("n05-centre-compact", false, false, Compact, 9),
        ("n06-centre-large", false, false, Large, 9),
        ("n07-centre-empty", false, false, Standard, 0),
    ] {
        let mut shell = fresh(light, bezel, text, notes);
        shell.key(Ctrl(platform::NOTIFICATIONS));
        settle(&mut shell, 90);
        save(&mut shell, name);
    }
    // Scrolled down the list, and coming down.
    let mut shell = fresh(false, false, Standard, 9);
    shell.key(Ctrl(platform::NOTIFICATIONS));
    settle(&mut shell, 7);
    save(&mut shell, "n08-centre-coming-down");
    settle(&mut shell, 80);
    for _ in 0..6 {
        shell.key(Down);
        settle(&mut shell, 12);
    }
    settle(&mut shell, 60);
    save(&mut shell, "n09-centre-scrolled");
    // Quiet on: the heading says so; empty, it says they wait.
    let mut shell = fresh(false, false, Standard, 4);
    shell.notes.set_quiet(true);
    shell.key(Ctrl(platform::NOTIFICATIONS));
    settle(&mut shell, 90);
    save(&mut shell, "n10-centre-quiet");
    let mut shell = fresh(true, false, Standard, 0);
    shell.notes.set_quiet(true);
    shell.key(Ctrl(platform::NOTIFICATIONS));
    settle(&mut shell, 90);
    save(&mut shell, "n11-centre-quiet-empty");
    // The control centre: Quiet, and Notifications beside it (a dot on the
    // bell while some are new); the page turning to them.
    for (name, light, text) in [("n12-control-centre", false, Standard), ("n13-control-centre-light", true, Standard), ("n14-control-centre-large", false, Large), ("n15-control-centre-compact", false, Compact)] {
        let mut shell = fresh(light, false, text, 3);
        shell.notes.set_quiet(true);
        shell.key(Ctrl(' '));
        settle(&mut shell, 60);
        for key in [Down, Down, Right] {
            shell.key(key);
            settle(&mut shell, 12);
        }
        settle(&mut shell, 60);
        save(&mut shell, name);
    }
    let mut shell = fresh(false, false, Standard, 3);
    shell.key(Ctrl(' '));
    settle(&mut shell, 60);
    shell.key(Char('n'));
    settle(&mut shell, 6);
    save(&mut shell, "n16-control-centre-turning");

    // The island: how many are new, at rest (no live activity carrying
    // it); one said as it comes; with Quiet, kept and counted, unsaid.
    for (name, light, text) in [("n17-island-count", false, Standard), ("n18-island-count-light", true, Standard), ("n19-island-count-large", false, Large)] {
        let mut shell = fresh(light, false, text, 3);
        shell.pretend_activities(Vec::new());
        settle(&mut shell, 300);
        save(&mut shell, name);
    }
    for (name, quiet) in [("n20-island-said", false), ("n21-island-quiet", true)] {
        let mut shell = fresh(false, false, Standard, 2);
        shell.pretend_activities(Vec::new());
        shell.notes.set_quiet(quiet);
        settle(&mut shell, 300);
        shell.notify(Notification::new("Music", "download_done", "12 songs saved").body("in the library").action("builtin:Music"));
        settle(&mut shell, 40);
        save(&mut shell, name);
    }

    // The lock screen with notifications (a passcode set: locked).
    let (phc, _, _) = passcode::make("2468").expect("hashes");
    passcode::keep(&phc).expect("kept");
    // (`acts`: the demo's next match, as it comes; or none; or a song.)
    for (name, light, bezel, text, notes, words, acts, pick) in [
        ("n30-lock-notifications", false, false, Standard, 3, false, "none", false),
        ("n31-lock-notifications-words", false, false, Standard, 3, true, "none", false),
        ("n32-lock-one-picked", false, false, Standard, 3, true, "none", true),
        ("n33-lock-song-and-a-notification", false, false, Standard, 3, true, "song", false),
        ("n34-lock-match-and-a-notification", false, false, Standard, 3, true, "match", false),
        ("n35-lock-notifications-light", true, false, Standard, 3, true, "none", false),
        ("n36-lock-notifications-bezel", false, true, Standard, 3, true, "none", false),
        ("n37-lock-notifications-compact", false, false, Compact, 4, true, "none", false),
        ("n38-lock-notifications-large", false, false, Large, 3, true, "none", false),
        ("n39-lock-song-and-a-notification-large", false, false, Large, 3, true, "song", false),
    ] {
        let mut shell = fresh(light, bezel, text, notes);
        shell.settings.lock.previews = words;
        shell.apply_settings();
        match acts {
            "none" => shell.pretend_activities(Vec::new()),
            "song" => {
                shell.pretend_activities(Vec::new());
                shell.pretend_activities(vec![music()]);
            }
            _ => {}
        }
        settle(&mut shell, 120);
        if pick {
            shell.key(Down);
        }
        settle(&mut shell, 60);
        save(&mut shell, name);
    }
    // Charging with a song: the chip under the date, the card below it
    // (it used to run into the card), at every text size and in each tone,
    // locked and as the curtain.
    for (name, light, text) in [
        ("n40-lock-charging-song", false, Standard),
        ("n41-lock-charging-song-compact", false, Compact),
        ("n42-lock-charging-song-large", false, Large),
        ("n43-lock-charging-song-light", true, Standard),
        ("n44-lock-charging-song-two", false, Standard),
    ] {
        let mut shell = fresh(light, false, text, 0);
        shell.pretend_charger(Some(true));
        shell.pretend_activities(Vec::new());
        let list = if name.ends_with("two") { vec![liveactivity::timers(Some((272.0, 300.0, 1)), None).remove(0), music()] } else { vec![music()] };
        shell.pretend_activities(list);
        settle(&mut shell, 180);
        save(&mut shell, name);
    }
    passcode::remove().ok();
    let mut shell = fresh(false, false, Standard, 0);
    shell.pretend_charger(Some(true));
    shell.pretend_activities(Vec::new());
    shell.pretend_activities(vec![music()]);
    settle(&mut shell, 10);
    shell.close_island_for_gallery();
    shell.lock_as_if_held();
    settle(&mut shell, 120);
    save(&mut shell, "n45-curtain-charging-song");
    let mut shell = fresh(false, false, Standard, 3);
    shell.lock_as_if_held();
    settle(&mut shell, 120);
    shell.key(Down);
    settle(&mut shell, 30);
    save(&mut shell, "n46-curtain-one-picked");
    faces::pretend_time(None);
    println!("notification shots in {}", dir.display());
}

/// Home's tiles come in the order of the built-in apps, and Left and Right go
/// through them in turn: from the first tile, this many Rights reach `name`.
fn steps_to(name: &str) -> usize {
    screens::builtin_apps().position(|(_, _, n)| n == name).expect("a built-in app")
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
    let each = |d: Duration| d.as_secs_f64() * 1e3 / frames as f64;
    println!("  of which the screens {:.2} ms, the glass over them {:.2} ms", each(shell.spent.0), each(shell.spent.1));
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

/// Once a second: frames drawn, what a frame costs, the longest wait
/// between two frames of the same motion (a stutter is one long wait), and
/// how busy the processor was. An idle screen should read 0 fps and about 0%.
#[derive(Default)]
struct Meter {
    since: Option<Instant>,
    frames: u32,
    busy: Duration,
    frame_cost: Duration,
    /// The last frame sent, carried across seconds, and the longest gap
    /// between two frames under half a second apart (longer is rest).
    last_drawn: Option<Instant>,
    worst_gap: Duration,
}

impl Meter {
    fn record(&mut self, cost: Duration, drew: bool) -> Option<String> {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.busy += cost;
        if drew {
            self.frames += 1;
            self.frame_cost += cost;
            let now = Instant::now();
            if let Some(gap) = self.last_drawn.map(|t| now - t).filter(|g| *g < Duration::from_millis(500)) {
                self.worst_gap = self.worst_gap.max(gap);
            }
            self.last_drawn = Some(now);
        }
        let elapsed = since.elapsed();
        if elapsed < Duration::from_secs(1) {
            return None;
        }
        let frame_ms = if self.frames > 0 { self.frame_cost.as_secs_f32() * 1e3 / self.frames as f32 } else { 0.0 };
        let line = format!(
            "{:.0} fps · {frame_ms:.1} ms a frame · worst gap {:.0} ms · processor {:.0}% busy",
            self.frames as f32 / elapsed.as_secs_f32(),
            self.worst_gap.as_secs_f32() * 1e3,
            (self.busy.as_secs_f32() / elapsed.as_secs_f32() * 100.0).min(100.0)
        );
        let last_drawn = self.last_drawn;
        *self = Meter { last_drawn, ..Meter::default() };
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
    fn settle(shell: &mut Shell) {
        for _ in 0..240 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    }
    /// Measure one kind of motion: press `key`, follow it frame by frame as
    /// the handheld's loop would, then `undo` it; several times over. With
    /// `leave`, the step is left done for the one after it.
    fn measure(shell: &mut Shell, beat: &mut Beat, speed: &Speed, label: &str, key: Key, undo: Key, leave: bool) {
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
                if !shell.moving() || frames > 400 {
                    break;
                }
            }
            counts.push(frames);
            beat.rest();
            settle(shell);
            // Undo, so every run starts from the same place.
            shell.key(undo);
            settle(shell);
        }
        if leave {
            shell.key(key);
            settle(shell);
        }
        if rest.is_empty() {
            println!("  {label:<20} nothing moved");
            return;
        }
        let median = |v: &mut Vec<f32>| {
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2] * 1e3
        };
        let worst = rest.iter().cloned().fold(0.0f32, f32::max) * 1e3;
        let typical = median(&mut rest);
        // How many frames came exactly a typical frame after the one before.
        let even = rest.iter().filter(|i| (**i * 1e3 - typical).abs() < 0.5).count() as f32 / rest.len().max(1) as f32;
        counts.sort();
        println!("  {label:<20} {:>6} {:>8.0}ms {:>8.0}ms {:>8.0}ms {:>8.0}%", counts[counts.len() / 2], median(&mut firsts), typical, worst, even * 100.0);
    }

    // Each kind of motion: the keys that get to where it starts, its name,
    // the key that sets it off, the key that undoes it, and whether to leave
    // it done. They begin on Home with the Settings tile focused (Settings is
    // an app with pages inside it) and run in order.
    let to_first = vec![Left; steps_to("Settings")];
    let steps: [(&[Key], &str, Key, Key, bool); 10] = [
        (&[], "app opens", Enter, Back, true),
        (&[], "app closes", Back, Enter, true),
        (&[], "app opens", Enter, Back, true),
        (&[], "page slides in", Enter, Back, true),
        (&[], "page slides out", Back, Enter, true),
        (&[], "control comes down", Ctrl(' '), Ctrl(' '), true),
        (&[], "control goes away", Ctrl(' '), Ctrl(' '), true),
        // About, ninth in Settings: a list longer than the page.
        (&[Down, Down, Down, Down, Down, Down, Down, Down, Enter], "list scrolls", PageDown, PageUp, false),
        // Home again, on the Settings tile: left is the page before.
        (&[Back, Back], "home page turns", Left, Right, false),
        // To the first tile of the first page: left again is the glance page.
        (&to_first, "glance slides in", Left, Right, false),
    ];
    for name in ["device", "worse"] {
        let speed = Speed::named(Some(name));
        let mut beat = Beat::new();
        // Home, the Settings tile focused: first on the third page.
        for _ in 0..steps_to("Settings") {
            shell.key(Right);
        }
        settle(&mut shell);
        println!("{}: processor x{}, panel {} ms a full frame", speed.name, speed.cpu, speed.panel.as_millis());
        println!("  {:<20} {:>6} {:>10} {:>10} {:>10} {:>9}", "", "frames", "first", "typical", "worst", "even");
        for (setup, label, key, undo, leave) in steps {
            for key in setup {
                shell.key(*key);
                settle(&mut shell);
            }
            measure(&mut shell, &mut beat, &speed, label, key, undo, leave);
        }
        // The other Home.
        shell.settings.home = shell::HomeStyle::Carousel;
        shell.apply_settings();
        settle(&mut shell);
        measure(&mut shell, &mut beat, &speed, "carousel turns", Right, Left, false);
        // And Flow: a card on, and one turned over.
        shell.settings.home = shell::HomeStyle::Flow;
        shell.apply_settings();
        settle(&mut shell);
        measure(&mut shell, &mut beat, &speed, "flow turns", Right, Left, false);
        measure(&mut shell, &mut beat, &speed, "flow turns over", Enter, Back, false);
        // Back to the grid, on the first tile, for the next speed.
        shell.settings.home = shell::HomeStyle::Grid;
        shell.apply_settings();
        settle(&mut shell);
        assert_eq!(shell.top_title(), "Catalyst", "each speed starts from Home");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gallery's Reminders pictures alone, into CATALYST_SHOTS (or the
    /// temporary folder): `cargo test --profile sim -p catalyst-shell
    /// reminders_gallery -- --ignored`.
    #[test]
    #[ignore]
    fn reminders_gallery() {
        shell::tests::sandbox();
        let dir = std::env::var_os("CATALYST_SHOTS").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("catalyst-reminder-shots"));
        std::fs::create_dir_all(&dir).unwrap();
        reminder_shots(&dir);
    }

    fn ms(v: f32) -> Duration {
        Duration::from_secs_f32(v / 1000.0)
    }

    #[test]
    fn every_setting_comes_back_as_it_was_saved() {
        let mut s = shell::Settings::default();
        (s.light, s.calm, s.accent, s.columns) = (true, true, bezel::Accent::Hue(43), 2);
        (s.theme, s.wallpaper, s.corners) = (look::Theme::Bezel, look::Wallpaper::Aurora, look::Corners::Square);
        (s.letters, s.width) = (look::Letters::Round, look::Width::Wide);
        s.order = vec!["Schedule".into(), "Logs".into()];
        s.hidden = vec!["Tables".into(), "Git".into()];
        s.favourites = vec!["Logs".into()];
        (s.clock24, s.seconds, s.island, s.greeting) = (false, true, shell::IslandRest::Battery, false);
        (s.robot_name, s.gpu) = (Some("atlas".into()), false);
        (s.developer, s.motion_scale, s.show_keys, s.toured) = (true, 5.0, true, true);
        (s.screen_off, s.power) = (300, standby::Power::Saving);
        (s.lift_wake, s.face_down) = (false, false);
        let saved = settings_text(&s);
        assert!(saved != settings_text(&shell::Settings::default()), "every one of these differs from the default");
        let mut back = shell::Settings::default();
        parse_settings(&saved, &mut back);
        assert_eq!(settings_text(&back), saved);
        // A file from an older shell, or a line garbled: the rest still reads.
        let mut partial = shell::Settings::default();
        parse_settings("theme=bezel\nnonsense\ncolumns=9\naccent=hue-99\n", &mut partial);
        assert_eq!(partial.theme, look::Theme::Bezel);
        assert_eq!(partial.columns, 3, "out of range: kept as it was");
        assert!(partial.gpu, "not in the file: the default (the gpu when it helps)");
        assert!(partial.lift_wake && partial.face_down, "not in the file: on, as they start");
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

    #[test]
    fn the_battery_saver_holds_the_beat_at_thirty() {
        // The handheld's 40 a second drops to 30 at once, and stays however
        // much room the frames leave.
        let mut beat = Beat::new();
        beat.set_floor(Some(power::SAVER_FRAME_MS));
        assert_eq!(beat.level, 1);
        assert!((beat.interval().as_secs_f32() * 1000.0 - 33.3).abs() < 0.1);
        for _ in 0..100 {
            beat.frame_took(ms(5.0));
        }
        beat.rest();
        assert_eq!(beat.level, 1, "never quicker than 30 a second");
        // The simulator's 60 a second: two of its steps down, to 33 ms.
        let mut sim = Beat::every(1000.0 / 60.0);
        sim.set_floor(Some(power::SAVER_FRAME_MS));
        assert_eq!(sim.level, 2);
        // Lifted: it quickens again at rest, after a run with room to spare.
        beat.set_floor(None);
        for _ in 0..40 {
            beat.frame_took(ms(5.0));
        }
        beat.rest();
        assert_eq!(beat.level, 0);
    }
}
