//! Power: the battery, and what it is spent on.
//!
//! - **Readings**: the fuel gauge (a BQ27220) through the kernel's power
//!   supply class: the charge, what is drawn, the time left, its health; and
//!   the charge over the last day, kept on /data for the Battery page.
//! - **Battery saver**: 30 frames a second instead of 40, the wallpaper held
//!   still, the processor capped at 800 MHz and slow to climb. Switched in
//!   the control centre or Settings › Battery, and on by itself at 20% (off
//!   again on the charger).
//! - **Low-battery care**: a warning at 15% and at 10% (the island says it,
//!   and the keyboard's lights blink for a few seconds, once), and at 5% a
//!   clean power-off, so the card is never cut off half-written.
//! - **Standby**, while the screen is off: the panel itself sleeps (see
//!   `Platform::panel_sleep`), the processor is at its slowest (standby.rs),
//!   and after ten minutes Wi-Fi goes off, unless an update is downloading
//!   or someone is logged in over SSH. It comes back as the screen wakes.
//! - **Deep standby**, after half an hour of that: everything a key need not
//!   have, off (deep.rs and `catalyst-power deep`).
//! - **Idle USB sleeps**: the Ethernet chip (an SR9900A, driven by r8152)
//!   and the hub suspend whenever they are idle, the largest draw after the
//!   screen itself. The driver wakes on a cable or a packet, so the robot
//!   link is not missed.
//! - **Lower voltage**, in developer options: tried, and kept only once the
//!   system has run on it for five minutes (catalyst-power).

use std::cell::Cell;
use std::path::{Path, PathBuf};

use bezel::{Color, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

pub(crate) const SCRIPT: &str = "/usr/sbin/catalyst-power";
const UNDERVOLT_STATE: &str = "/data/catalyst/undervolt";
/// The battery saver comes on by itself at this charge.
pub const SAVER_AT: u32 = 20;
/// Warnings, once each time on battery.
const WARN_AT: [u32; 2] = [15, 10];
/// At this charge the handheld turns itself off; held for this many readings.
pub const EMPTY_AT: u32 = 5;
const EMPTY_READS: u32 = 3;
/// Postponed once, it turns off anyway at this charge.
const EMPTY_FLOOR: u32 = 2;
/// How long the screen is off before Wi-Fi goes too.
pub const WIFI_SLEEPS_AFTER: f32 = 600.0;
/// The battery saver's processor ceiling.
const SAVER_MAX_KHZ: u64 = 800_000;
/// The battery saver's frame beat: 30 a second.
pub const SAVER_FRAME_MS: f32 = 1000.0 / 30.0;
/// The charge history: a point every five minutes, a week of them kept.
const HISTORY_EVERY: f32 = 300.0;
const HISTORY_KEEP: i64 = 7 * 24 * 3600;
/// Seconds the system must have run on a lower voltage before it is kept.
const CONFIRM_AFTER: f64 = 300.0;
/// How long the turning-off card counts down.
const EMPTY_SECONDS: f32 = 10.0;

/// The choices on Settings › Battery.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Choices {
    pub saver: bool,
    /// The saver comes on by itself at 20%.
    pub saver_auto: bool,
    /// Turn off at 5%, rather than run until the battery cuts out.
    pub empty_off: bool,
    /// Wi-Fi goes off after ten minutes with the screen off.
    pub wifi_sleeps: bool,
    /// The red light stays dark instead of flickering with the card.
    pub quiet_light: bool,
    /// Minutes with the screen off and no key before deep standby (0: never).
    pub deep_after: u32,
}

impl Default for Choices {
    fn default() -> Self {
        Choices { saver: false, saver_auto: true, empty_off: true, wifi_sleeps: true, quiet_light: true, deep_after: crate::deep::AFTER_DEFAULT }
    }
}

impl Choices {
    /// As the settings file keeps them.
    pub fn lines(&self) -> [(&'static str, String); 6] {
        [
            ("saver", self.saver.to_string()),
            ("saver_auto", self.saver_auto.to_string()),
            ("empty_off", self.empty_off.to_string()),
            ("wifi_sleeps", self.wifi_sleeps.to_string()),
            ("quiet_light", self.quiet_light.to_string()),
            ("deep_after", self.deep_after.to_string()),
        ]
    }

    /// One line of the settings file, if it is one of these.
    pub fn parse(&mut self, key: &str, value: &str) -> bool {
        let on = value == "true";
        match key {
            "saver" => self.saver = on,
            "saver_auto" => self.saver_auto = on,
            "empty_off" => self.empty_off = on,
            "wifi_sleeps" => self.wifi_sleeps = on,
            "quiet_light" => self.quiet_light = on,
            "deep_after" => self.deep_after = crate::deep::parse_after(value),
            _ => return false,
        }
        true
    }
}

// --------------------------------------------------------------- readings ---

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Flow {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Full,
    /// On the charger, not charging (held at a level).
    Holding,
}

impl Flow {
    fn of(status: &str) -> Flow {
        match status.trim().to_ascii_lowercase().as_str() {
            "charging" => Flow::Charging,
            "discharging" => Flow::Discharging,
            "full" => Flow::Full,
            "not charging" => Flow::Holding,
            _ => Flow::Unknown,
        }
    }

    pub fn on_battery(self) -> bool {
        self == Flow::Discharging
    }
}

/// The gauge's readings at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub percent: u32,
    pub flow: Flow,
    /// Drawn from the battery (into it while charging: negative), mA.
    pub milliamps: Option<f32>,
    pub volts: Option<f32>,
    /// To empty, on battery; to full, charging.
    pub minutes_left: Option<u64>,
    pub minutes_to_full: Option<u64>,
    pub celsius: Option<f32>,
    pub cycles: Option<u32>,
    /// What it holds now, as a share of what it held new.
    pub health: Option<u32>,
    /// The charge left, mAh: deep standby's draw, from what it lost.
    pub charge_mah: Option<f32>,
}

/// The power supply that is a battery: the first with a charge.
fn gauge_dir() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir("/sys/class/power_supply").ok()?.flatten().map(|e| e.path()).filter(|p| p.join("capacity").exists()).collect();
    dirs.sort();
    dirs.into_iter().next()
}

impl Reading {
    /// The gauge now, if there is one.
    pub fn read() -> Option<Reading> {
        Reading::from_dir(&gauge_dir()?)
    }

    /// A gauge's files in `dir`, as the kernel lays them out.
    pub fn from_dir(dir: &Path) -> Option<Reading> {
        let text = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned());
        let num = |name: &str| text(name)?.parse::<f64>().ok();
        let percent = num("capacity")?.clamp(0.0, 100.0) as u32;
        let flow = Flow::of(&text("status").unwrap_or_default());
        // The sign of current_now varies from gauge to gauge: the flow says
        // which way it goes.
        let milliamps = num("current_now").map(|ua| {
            let ma = (ua.abs() / 1000.0) as f32;
            if flow == Flow::Charging { -ma } else { ma }
        });
        let minutes = |name: &str| num(name).filter(|s| *s > 0.0 && *s < 7.0 * 86400.0).map(|s| (s / 60.0).round() as u64);
        let health = match (num("charge_full"), num("charge_full_design")) {
            (Some(full), Some(design)) if design > 0.0 && full > 0.0 => Some(((full / design) * 100.0).round().min(100.0) as u32),
            _ => None,
        };
        Some(Reading {
            percent,
            flow,
            milliamps,
            volts: num("voltage_now").map(|uv| (uv / 1e6) as f32).filter(|v| *v > 0.5),
            minutes_left: minutes("time_to_empty_now").filter(|_| flow == Flow::Discharging),
            minutes_to_full: minutes("time_to_full_now").filter(|_| flow == Flow::Charging),
            celsius: num("temp").map(|t| (t / 10.0) as f32),
            cycles: num("cycle_count").map(|c| c as u32),
            health,
            charge_mah: num("charge_now").map(|uah| (uah / 1000.0) as f32).filter(|mah| *mah >= 0.0),
        })
    }

    /// A battery to show in the simulator.
    pub fn demo() -> Reading {
        Reading {
            percent: 76,
            flow: Flow::Discharging,
            milliamps: Some(287.0),
            volts: Some(3.86),
            minutes_left: Some(4 * 60 + 38),
            minutes_to_full: None,
            celsius: Some(27.5),
            cycles: Some(12),
            health: Some(98),
            charge_mah: Some(1330.0),
        }
    }

    /// What it draws, in watts, on battery.
    pub fn watts(&self) -> Option<f32> {
        Some(self.milliamps? * self.volts? / 1000.0)
    }

    /// "on battery · 4 h 38 m left", "charging · full in 40 m", "full".
    pub fn status(&self) -> String {
        match self.flow {
            Flow::Discharging => match self.minutes_left {
                Some(m) => format!("on battery · {} left", crate::sys::minutes_text(m)),
                None => "on battery".into(),
            },
            Flow::Charging => match self.minutes_to_full {
                Some(m) => format!("charging · full in {}", crate::sys::minutes_text(m)),
                None => "charging".into(),
            },
            Flow::Full => "full".into(),
            Flow::Holding => "plugged in".into(),
            Flow::Unknown => "–".into(),
        }
    }

    /// "287 mA · 1.11 W" on battery, "charging at 820 mA".
    pub fn draw_line(&self) -> Option<String> {
        let ma = self.milliamps?;
        Some(match self.flow {
            Flow::Charging => format!("charging at {:.0} mA", -ma),
            _ => match self.watts() {
                Some(w) => format!("{ma:.0} mA · {w:.2} W"),
                None => format!("{ma:.0} mA"),
            },
        })
    }
}

/// Where the gauge's readings come from.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Source {
    Gauge,
    /// A computer: the demo battery.
    Demo,
    /// A handheld with no gauge (the emulator).
    Nothing,
}

impl Source {
    fn find() -> Source {
        if gauge_dir().is_some() {
            Source::Gauge
        } else if Path::new(SCRIPT).exists() {
            Source::Nothing
        } else {
            Source::Demo
        }
    }

    fn read(self) -> Option<Reading> {
        match self {
            Source::Gauge => Reading::read(),
            Source::Demo => Some(Reading::demo()),
            Source::Nothing => None,
        }
    }
}

// ---------------------------------------------------------------- history ---

/// The charge over time: (Unix seconds, percent, charging).
pub type Point = (i64, u32, bool);

fn history_path() -> PathBuf {
    crate::config_dir().join("battery-history")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The points in a history file's text, oldest first.
pub fn parse_history(text: &str) -> Vec<Point> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse::<u32>().ok()?.min(100), f.next()? == "c"))
        })
        .collect()
}

fn history_line(p: Point) -> String {
    format!("{} {} {}\n", p.0, p.1, if p.2 { "c" } else { "d" })
}

/// The kept history, oldest first.
pub fn history() -> Vec<Point> {
    std::fs::read_to_string(history_path()).map(|t| parse_history(&t)).unwrap_or_default()
}

/// A day of charge for the simulator: down through a school day, up on the
/// charger at lunch.
pub fn demo_history(now: i64) -> Vec<Point> {
    (0..=288)
        .map(|i| {
            let t = now - (288 - i) * 300;
            let h = i as f32 / 12.0;
            let (p, c) = if h < 9.0 {
                (100.0 - h * 7.5, false)
            } else if h < 11.0 {
                (32.5 + (h - 9.0) * 30.0, true)
            } else {
                (92.5 - (h - 11.0) * 1.27, false)
            };
            (t, p.round().clamp(0.0, 100.0) as u32, c)
        })
        .collect()
}

/// Add a point; now and then, let go of those older than a week.
fn record(point: Point) {
    use std::io::Write;
    let path = history_path();
    let line = history_line(point);
    let appended = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(line.as_bytes()));
    if appended.is_err() {
        return;
    }
    // A week is about 2000 points: trim once it is half again as long.
    let points = history();
    if points.len() > 3000 {
        let keep: String = points.into_iter().filter(|p| p.0 >= point.0 - HISTORY_KEEP).map(history_line).collect();
        crate::sys::write_atomic(&path, keep.as_bytes()).ok();
    }
}

// ------------------------------------------------------------ the machine ---

/// The processor's ceiling, for the battery saver: the fastest speed the
/// kernel offers at or under 800 MHz (if it offers one above its slowest),
/// or back to the fastest it can go.
fn set_ceiling(saving: bool) {
    for policy in std::fs::read_dir("/sys/devices/system/cpu/cpufreq").into_iter().flatten().flatten() {
        let dir = policy.path();
        let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok();
        let Some(max) = read("cpuinfo_max_freq").and_then(|v| v.trim().parse::<u64>().ok()) else { continue };
        let min = read("cpuinfo_min_freq").and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0);
        let offered: Vec<u64> = read("scaling_available_frequencies").unwrap_or_default().split_whitespace().filter_map(|f| f.parse().ok()).collect();
        let ceiling = if saving { offered.iter().copied().filter(|f| *f <= SAVER_MAX_KHZ && *f > min).max().unwrap_or(max) } else { max };
        std::fs::write(dir.join("scaling_max_freq"), ceiling.to_string()).ok();
    }
}

/// Let the Ethernet chip sleep when idle (its driver wakes it on a cable or
/// a packet). The hub follows by itself once nothing on it is awake.
fn usb_sleeps() {
    for dev in std::fs::read_dir("/sys/bus/usb/devices").into_iter().flatten().flatten() {
        let p = dev.path();
        let id = |n: &str| std::fs::read_to_string(p.join(n)).map(|s| s.trim().to_owned()).unwrap_or_default();
        if id("idVendor") == "0fe6" && id("idProduct") == "9900" {
            let control = p.join("power/control");
            if std::fs::read_to_string(&control).is_ok_and(|c| c.trim() == "on") {
                std::fs::write(p.join("power/autosuspend_delay_ms"), "2000").ok();
                std::fs::write(&control, "auto").ok();
            }
        }
    }
}

/// Someone is logged in over SSH: an established connection to port 22.
pub(crate) fn ssh_session() -> bool {
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|file| {
        std::fs::read_to_string(file).is_ok_and(|text| {
            text.lines().skip(1).any(|line| {
                let f: Vec<&str> = line.split_whitespace().collect();
                f.len() > 3 && f[1].ends_with(":0016") && f[3] == "01"
            })
        })
    })
}

/// catalyst-power, away from the frame: it starts and stops services.
fn run_script(args: &'static [&'static str]) {
    if Path::new(SCRIPT).exists() {
        std::thread::spawn(move || std::process::Command::new(SCRIPT).args(args).status().ok());
    }
}

fn uptime() -> f64 {
    std::fs::read_to_string("/proc/uptime").ok().and_then(|t| t.split_whitespace().next()?.parse().ok()).unwrap_or(0.0)
}

// ------------------------------------------------------------ lower voltage ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Undervolt {
    /// Not on the handheld.
    Unavailable,
    Off,
    /// Set, and not yet started with (a restart that did not happen).
    Pending,
    /// This start is the one it is tried in; kept once it has held.
    Trying,
    On,
    /// Tried, and the system did not stay up: taken out again.
    Failed,
}

pub fn undervolt() -> Undervolt {
    if !Path::new(SCRIPT).exists() {
        return Undervolt::Unavailable;
    }
    state_of(&std::fs::read_to_string(UNDERVOLT_STATE).unwrap_or_default())
}

fn state_of(text: &str) -> Undervolt {
    match text.trim() {
        "trying 0" => Undervolt::Pending,
        s if s.starts_with("trying") => Undervolt::Trying,
        "on" => Undervolt::On,
        "failed" => Undervolt::Failed,
        _ => Undervolt::Off,
    }
}

impl Undervolt {
    /// Set, or on its way to being.
    pub fn on(self) -> bool {
        matches!(self, Undervolt::Pending | Undervolt::Trying | Undervolt::On)
    }

    /// Enter on it asks to restart and try it (rather than switching it off).
    pub fn asks_to_try(self) -> bool {
        matches!(self, Undervolt::Off | Undervolt::Failed | Undervolt::Pending)
    }
}

/// The processor core's voltage as the firmware has it, in volts: its
/// ceiling at full speed (what over_voltage sets), its floor when idle
/// (over_voltage_min), and now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CoreVolts {
    pub max: f32,
    pub min: f32,
    pub now: f32,
}

/// The ceiling the firmware gives the core without the lower voltage, as
/// read on a start without it; kept to tell whether the lower one took.
const STOCK_VOLTS: &str = "/data/catalyst/voltage-stock";
/// The firmware's own ceiling where none was read: 1.2 V, over_voltage 0.
const NOMINAL_VOLTS: f32 = 1.2;

/// A voltage from the firmware's mailbox: microvolts from current firmware;
/// older firmware gave steps of 25 mV from 1.2 V.
#[cfg_attr(not(all(target_os = "linux", target_arch = "aarch64")), allow(dead_code))]
fn mailbox_volts(raw: u32) -> Option<f32> {
    match raw {
        500_000..=2_000_000 => Some(raw as f32 / 1e6),
        _ => {
            let steps = raw as i32;
            (-16..=16).contains(&steps).then(|| NOMINAL_VOLTS + 0.025 * steps as f32)
        }
    }
}

/// Ask the firmware, through its mailbox (`/dev/vcio`, as vcgencmd does).
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn ask_firmware() -> Option<CoreVolts> {
    use std::os::fd::AsRawFd;
    const GET_VOLTAGE: u32 = 0x0003_0003;
    const GET_MAX_VOLTAGE: u32 = 0x0003_0005;
    const GET_MIN_VOLTAGE: u32 = 0x0003_0008;
    const CORE: u32 = 1;
    const REPLIED: u32 = 0x8000_0000;
    // _IOWR(100, 0, char *) on a 64-bit kernel.
    const IOCTL_MBOX_PROPERTY: libc::c_ulong = 0xC008_6400;
    let vcio = std::fs::OpenOptions::new().read(true).write(true).open("/dev/vcio").ok()?;
    // Size, request; three tags of (tag, value size, request, id, value); end.
    let mut message = [0u32; 18];
    message[0] = (message.len() * 4) as u32;
    for (i, tag) in [GET_MAX_VOLTAGE, GET_MIN_VOLTAGE, GET_VOLTAGE].into_iter().enumerate() {
        message[2 + i * 5..][..5].copy_from_slice(&[tag, 8, 0, CORE, 0]);
    }
    if unsafe { libc::ioctl(vcio.as_raw_fd(), IOCTL_MBOX_PROPERTY as _, message.as_mut_ptr()) } < 0 || message[1] != REPLIED {
        return None;
    }
    Some(CoreVolts { max: mailbox_volts(message[6])?, min: mailbox_volts(message[11])?, now: mailbox_volts(message[16])? })
}

#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
fn ask_firmware() -> Option<CoreVolts> {
    None
}

thread_local! {
    /// The firmware's answer, asked again at most every five seconds.
    static VOLTS: Cell<Option<(f64, Option<CoreVolts>)>> = const { Cell::new(None) };
}

/// The core's voltages now, if the firmware says (on the handheld).
pub fn core_volts() -> Option<CoreVolts> {
    let now = uptime();
    VOLTS.with(|v| match v.get() {
        Some((at, read)) if now - at < 5.0 => read,
        _ => {
            let read = ask_firmware();
            v.set(Some((now, read)));
            read
        }
    })
}

fn stock_ceiling() -> Option<f32> {
    std::fs::read_to_string(STOCK_VOLTS).ok()?.trim().parse().ok()
}

/// Whether the lower voltage took: the firmware's ceiling at least 20 mV
/// under its own (as read on a start without it, or else 1.2 V).
pub fn lowered(read: CoreVolts, stock: Option<f32>) -> bool {
    read.max <= stock.unwrap_or(NOMINAL_VOLTS) - 0.020
}

/// Developer options › Lower voltage, as the row says it: where it stands
/// and, once started with, what the firmware actually gave the core.
pub fn undervolt_line() -> String {
    line_for(undervolt(), core_volts(), stock_ceiling())
}

fn line_for(state: Undervolt, read: Option<CoreVolts>, stock: Option<f32>) -> String {
    let now = match state {
        Undervolt::Unavailable => return "on the handheld".into(),
        Undervolt::Off => return "off".into(),
        Undervolt::Failed => return "did not hold".into(),
        Undervolt::Pending => return "set · restart to try".into(),
        Undervolt::Trying => "trying",
        Undervolt::On => "on",
    };
    match read {
        Some(v) if lowered(v, stock) => format!("{now} · {:.3} V", v.max),
        Some(_) => format!("{now} · not taken"),
        None => format!("{now} · −25 mV"),
    }
}

/// Switch the lower voltage off (from the next start). Switching it on goes
/// through [`TryUndervolt`], which restarts into it.
pub fn set_undervolt(on: bool) -> Result<String, String> {
    if !Path::new(SCRIPT).exists() {
        return Err("the voltage is set on the handheld".into());
    }
    let ok = std::process::Command::new(SCRIPT).args(["undervolt", if on { "on" } else { "off" }]).status().is_ok_and(|s| s.success());
    match (ok, on) {
        (true, true) => Ok("lower voltage · restart to try it".into()),
        (true, false) => Ok("lower voltage off · from the next start".into()),
        (false, _) => Err("could not change config.txt".into()),
    }
}

/// Developer options › Lower voltage, switched on: what it does, and a
/// restart into it at once (Esc: not now). That start tries it; it is kept
/// once the system has run on it for five minutes, and otherwise goes by
/// itself (a hang is caught by the watchdog, catalyst-power).
pub struct TryUndervolt {
    restarting: bool,
}

const TRY_WORDS: &str = "The core 25 mV lower at full speed, 50 mV when idle: a little less heat and power. It restarts to try it, and goes back by itself if it does not hold.";

impl TryUndervolt {
    pub fn new() -> Self {
        TryUndervolt { restarting: false }
    }
}

impl Default for TryUndervolt {
    fn default() -> Self {
        Self::new()
    }
}

/// Its one row, at the foot above the keys.
fn try_row(g: &Gfx) -> Rect {
    let row_h = g.row_h();
    Rect::new(PAD, H - PAD - g.hint_h() - row_h, W - 2.0 * PAD, row_h)
}

impl Screen for TryUndervolt {
    fn title(&self) -> &str {
        // "Lower voltage" does not fit beside the island at the title's size.
        "Voltage"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.restarting {
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                if !Path::new(SCRIPT).exists() {
                    *cx.say = Some("the voltage is set on the handheld".into());
                    return Nav::Pop;
                }
                // It sets the lines, then restarts cleanly (the services stopped, the card synced).
                run_script(&["undervolt", "try"]);
                self.restarting = true;
                *cx.say = Some("restarting to try the lower voltage".into());
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let row = try_row(g);
        // What it does, above the one row at the foot.
        let line_h = ty.caption.size + 4.0;
        let lines = crate::help::wrap(g, TRY_WORDS, ty.caption, W - 2.0 * PAD);
        let fit = (((row.y - 6.0 - top) / line_h).floor().max(0.0) as usize).min(lines.len());
        for (i, line) in lines.iter().take(fit).enumerate() {
            g.text_alpha(PAD, top + 2.0 + line_h * (i as f32 + 0.5), line, ty.caption, t.dim, 1.0);
        }
        g.row(row, if self.restarting { "Restarting…" } else { "Restart and try it" }, "›", t.ink, true);
        g.hint(if self.restarting { "the handheld restarts" } else { "enter: restart · esc: not now" });
    }
}

// ------------------------------------------------------------------ care ---

/// What the shell is to do about the battery.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    /// Say it in the island.
    Say(String),
    /// Blink the keyboard's lights: a warning.
    Warn,
    /// The battery is empty: wake the screen and count down to off.
    Empty,
    /// The saver was switched by the rules: apply and save the settings.
    Saver,
}

thread_local! {
    /// The turning-off card was put off with Esc.
    static POSTPONED: Cell<bool> = const { Cell::new(false) };
}

/// Readings, the history, the saver's rules, low-battery care and
/// standby's Wi-Fi, ticked by the shell (once a minute in deep standby).
pub struct Care {
    pub reading: Option<Reading>,
    source: Source,
    read_age: f32,
    history_age: f32,
    /// The lowest warning given this time on battery (101: none yet).
    warned: u32,
    /// The saver's 20% rule has had its turn this time on battery, and
    /// whether it was the rule that switched the saver on.
    auto_spent: bool,
    by_auto: bool,
    empty_reads: u32,
    emptying: bool,
    postponed: bool,
    asleep_for: f32,
    wifi_slept: bool,
    usb_age: f32,
    ceiling: Option<bool>,
    light: Option<bool>,
    undervolt_checked: bool,
}

impl Care {
    pub fn new() -> Care {
        Care::from(Source::find())
    }

    /// The charge as the gauge last gave it, on a handheld that has one
    /// (the island shows this, as Home's battery widget does).
    pub fn gauge_percent(&self) -> Option<u32> {
        self.reading.as_ref().filter(|_| self.source == Source::Gauge).map(|r| r.percent)
    }

    fn from(source: Source) -> Care {
        Care {
            reading: None,
            source,
            read_age: f32::MAX,
            history_age: HISTORY_EVERY,
            warned: 101,
            auto_spent: false,
            by_auto: false,
            empty_reads: 0,
            emptying: false,
            postponed: false,
            asleep_for: 0.0,
            wifi_slept: false,
            usb_age: f32::MAX,
            ceiling: None,
            light: None,
            undervolt_checked: false,
        }
    }

    /// Time passing. `asleep`: the screen is off and dark; `busy_update`:
    /// an update is checking, downloading or installing.
    pub fn tick(&mut self, dt: f32, asleep: bool, choices: &mut Choices, busy_update: bool) -> Vec<Event> {
        let mut events = Vec::new();
        let handheld = self.source != Source::Demo;
        if handheld {
            if self.light != Some(choices.quiet_light) {
                crate::lights::red_light(choices.quiet_light);
                self.light = Some(choices.quiet_light);
            }
            if self.ceiling != Some(choices.saver) {
                set_ceiling(choices.saver);
                self.ceiling = Some(choices.saver);
            }
            self.usb_age += dt;
            if self.usb_age >= 30.0 {
                self.usb_age = 0.0;
                usb_sleeps();
            }
            self.standby_wifi(dt, asleep, choices, busy_update);
            self.check_undervolt(&mut events);
        }
        self.read_age += dt;
        if self.read_age >= if asleep { 20.0 } else { 5.0 } {
            self.read_age = 0.0;
            self.reading = self.source.read();
            if let Some(r) = self.reading.clone() {
                events.extend(self.observe(&r, choices));
            }
        }
        if self.source == Source::Gauge {
            self.history_age += dt;
            if self.history_age >= HISTORY_EVERY {
                if let Some(r) = &self.reading {
                    self.history_age = 0.0;
                    record((now(), r.percent, !r.flow.on_battery()));
                }
            }
        }
        events
    }

    /// Ten minutes into the screen being off, Wi-Fi goes, unless something
    /// needs it; it comes back with the screen. Woken meanwhile for an update
    /// (which wakes it when it needs the network), it goes again once the
    /// update is done with it.
    fn standby_wifi(&mut self, dt: f32, asleep: bool, choices: &Choices, busy_update: bool) {
        if asleep {
            self.asleep_for += dt;
            if choices.wifi_sleeps && self.asleep_for >= WIFI_SLEEPS_AFTER && !busy_update && crate::wifi::switched_on() && !ssh_session() {
                if !self.wifi_slept {
                    run_script(&["wifi-sleep"]);
                    self.wifi_slept = true;
                } else if crate::wifi::woken() {
                    run_script(&["wifi-sleep"]);
                }
            }
        } else {
            self.asleep_for = 0.0;
            if self.wifi_slept {
                run_script(&["wifi-wake"]);
                self.wifi_slept = false;
            }
        }
    }

    /// A lower voltage being tried is kept once the system has run on it
    /// for five minutes (and the island says what the firmware gave the
    /// core); one that did not hold is said once. On a start without it, the
    /// firmware's own ceiling is noted, to tell later whether it took.
    fn check_undervolt(&mut self, events: &mut Vec<Event>) {
        if self.undervolt_checked {
            return;
        }
        let state = std::fs::read_to_string(UNDERVOLT_STATE).unwrap_or_default();
        match state.trim() {
            "trying 1" if uptime() >= CONFIRM_AFTER => {
                run_script(&["undervolt", "confirm"]);
                events.push(Event::Say(match core_volts() {
                    Some(v) if lowered(v, stock_ceiling()) => format!("lower voltage held · core {:.3} V", v.max),
                    Some(v) => format!("lower voltage not taken · core {:.3} V", v.max),
                    None => "lower voltage held · kept".into(),
                }));
                self.undervolt_checked = true;
            }
            "trying 1" => {}
            "failed" => {
                events.push(Event::Say("lower voltage did not hold · it is off".into()));
                std::fs::write(UNDERVOLT_STATE, "off\n").ok();
                self.undervolt_checked = true;
            }
            other => {
                if state_of(other) == Undervolt::Off {
                    if let Some(v) = core_volts() {
                        std::fs::write(STOCK_VOLTS, format!("{:.4}\n", v.max)).ok();
                    }
                }
                self.undervolt_checked = true;
            }
        }
    }

    /// The rules, on a new reading.
    pub fn observe(&mut self, r: &Reading, choices: &mut Choices) -> Vec<Event> {
        let mut events = Vec::new();
        if POSTPONED.with(|p| p.replace(false)) {
            self.postponed = true;
            self.emptying = false;
            self.empty_reads = 0;
        }
        if !r.flow.on_battery() {
            // On the charger: the warnings may come again next time, and a
            // saver the rule switched on goes off.
            self.warned = 101;
            self.auto_spent = false;
            self.empty_reads = 0;
            self.postponed = false;
            if self.by_auto && choices.saver {
                choices.saver = false;
                events.push(Event::Saver);
                events.push(Event::Say("battery saver off · charging".into()));
            }
            self.by_auto = false;
            return events;
        }
        if choices.saver_auto && !self.auto_spent && r.percent <= SAVER_AT {
            self.auto_spent = true;
            if !choices.saver {
                choices.saver = true;
                self.by_auto = true;
                events.push(Event::Saver);
                events.push(Event::Say(format!("battery at {}% · battery saver on", r.percent)));
            }
        }
        if let Some(level) = WARN_AT.iter().copied().filter(|l| r.percent <= *l && *l < self.warned).min() {
            self.warned = level;
            events.push(Event::Warn);
            events.push(Event::Say(if level <= 10 { format!("battery at {}% · plug in soon", r.percent) } else { format!("battery at {}%", r.percent) }));
        }
        let floor = if self.postponed { EMPTY_FLOOR } else { EMPTY_AT };
        if choices.empty_off && r.percent <= floor {
            self.empty_reads += 1;
            if self.empty_reads >= EMPTY_READS && !self.emptying {
                self.emptying = true;
                events.push(Event::Warn);
                events.push(Event::Empty);
            }
        } else {
            self.empty_reads = 0;
        }
        events
    }
}

// ------------------------------------------------------------ turning off ---

/// The card shown as the battery runs out: it counts down and turns the
/// handheld off cleanly. Plugged in meanwhile, it stays on; Esc puts it off
/// until 2%.
pub struct Empty {
    left: f32,
    /// Seconds since the charger was seen, and since it was put off.
    charging: Option<f32>,
    postponed: Option<f32>,
    off: bool,
    read_age: f32,
    floor: bool,
    shown: Animated,
}

impl Empty {
    pub fn new(calm: bool) -> Empty {
        let mut shown = Animated::new(0.0, role::smooth()).with_rest(0.004);
        if calm {
            shown.jump(1.0);
        } else {
            shown.set(1.0);
        }
        Empty { left: EMPTY_SECONDS, charging: None, postponed: None, off: false, read_age: 0.0, floor: false, shown }
    }

    /// The second time, at 2%: no putting it off.
    pub fn at_floor(mut self) -> Empty {
        self.floor = true;
        self
    }
}

impl Screen for Empty {
    fn title(&self) -> &str {
        "Battery"
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if matches!(key, Key::Back | Key::Leave) && !self.floor && !self.off && self.charging.is_none() {
            POSTPONED.with(|p| p.set(true));
            self.postponed = Some(0.0);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.shown.tick(dt);
        self.read_age += dt;
        if self.read_age >= 1.0 {
            self.read_age = 0.0;
            if Reading::read().is_some_and(|r| !r.flow.on_battery()) && self.charging.is_none() && !self.off {
                self.charging = Some(0.0);
            }
        }
        if let Some(t) = &mut self.charging {
            *t += dt;
            return true;
        }
        if let Some(t) = &mut self.postponed {
            *t += dt;
            return true;
        }
        if !self.off {
            self.left -= dt;
            if self.left <= 0.0 {
                self.off = true;
                // Cleanly: the services stop, /data is put away, then off.
                if Path::new(SCRIPT).exists() {
                    std::process::Command::new("poweroff").spawn().ok();
                }
            }
        }
        // It counts down on screen: a frame for each step.
        true
    }

    fn finished(&self) -> bool {
        self.charging.is_some_and(|t| t > 2.0) || self.postponed.is_some_and(|t| t > 2.5)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let a = self.shown.value().clamp(0.0, 1.0);
        g.canvas.rrect(Rect::new(0.0, 0.0, W, H), 0.0, t.ground, 1.0);
        let (title, line, sub) = if self.charging.is_some() {
            ("Charging", "Staying on.".to_owned(), String::new())
        } else if self.postponed.is_some() {
            ("Staying on", "It turns off at 2% instead.".to_owned(), "Plug it in soon.".to_owned())
        } else if self.off {
            ("Turning off", "Charge it, then slide the switch".to_owned(), "off and on again.".to_owned())
        } else {
            let secs = self.left.ceil().max(1.0) as u32;
            ("Battery empty", format!("Turning off in {secs} s to keep"), "your files safe. Plug in to stay on.".to_owned())
        };
        // The battery, nearly empty: a capsule with a sliver in the fault colour.
        let body = Rect::new(W * 0.5 - 22.0, 30.0, 40.0, 20.0);
        g.canvas.stroke_rrect(body, 6.0, 2.0, t.ink, a);
        g.canvas.rrect(Rect::new(body.right() + 1.5, body.cy() - 4.0, 3.0, 8.0), 1.5, t.ink, a);
        let fill = if self.charging.is_some() { t.ice } else { g.fault() };
        g.canvas.rrect(Rect::new(body.x + 4.0, body.y + 4.0, if self.charging.is_some() { 20.0 } else { 5.0 }, body.h - 8.0), 2.0, fill, a);
        let big = ty.display(24.0);
        let title_w = g.measure(title, big);
        g.text_alpha(W * 0.5 - title_w * 0.5, 78.0, title, big, t.ink, a);
        for (i, text) in [line, sub].iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let fitted = g.text.fit(text, &ty.body, W - 2.0 * PAD);
            let w = g.measure(&fitted, ty.body);
            g.text_alpha(W * 0.5 - w * 0.5, 106.0 + i as f32 * (ty.body.size + 6.0), &fitted, ty.body, t.dim, a);
        }
        if !self.floor && !self.off && self.charging.is_none() && self.postponed.is_none() {
            g.hint("esc · stay on until 2%");
        }
    }

    fn full_screen(&self) -> bool {
        true
    }
}

/// Put off by Esc, it is shown again at 2%; whether that is now.
pub fn postponed_floor(care: &Care) -> bool {
    care.postponed
}

// --------------------------------------------------------- Battery page ---

/// The header's height at rest.
const HERO_H: f32 = 62.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Saver,
    SaverAuto,
    EmptyOff,
    WifiSleeps,
    Deep,
    RedLight,
    Drawing,
    LastDeep,
    Health,
    Temperature,
    Voltage,
}

const ROWS: [Row; 11] =
    [Row::Saver, Row::SaverAuto, Row::EmptyOff, Row::WifiSleeps, Row::Deep, Row::RedLight, Row::Drawing, Row::LastDeep, Row::Health, Row::Temperature, Row::Voltage];

/// Where the deep standby row is, for search and the gallery.
pub const DEEP_ROW: usize = 4;

/// Settings › Battery: the charge and the last day of it at the top, then
/// the battery saver and the rest of the choices, then the battery itself.
pub struct BatteryPage {
    list: ListState,
    reading: Option<Reading>,
    points: Vec<Point>,
    read_at: std::time::Instant,
    hero: Animated,
    demo: bool,
    /// The last deep standby: how long, and what it drew.
    last: Option<crate::deep::Slept>,
}

impl BatteryPage {
    pub fn new() -> BatteryPage {
        let demo = Source::find() == Source::Demo;
        let mut page = BatteryPage {
            list: ListState::default(),
            reading: None,
            points: Vec::new(),
            read_at: std::time::Instant::now(),
            hero: Animated::new(1.0, role::page()).with_rest(0.004),
            demo,
            last: None,
        };
        page.read();
        page
    }

    /// Opened at a row (from search).
    pub fn at(mut self, row: usize) -> BatteryPage {
        self.list.focus = row.min(ROWS.len() - 1);
        if self.list.focus >= 2 {
            self.hero.jump(0.0);
        }
        self
    }

    fn read(&mut self) {
        self.read_at = std::time::Instant::now();
        let source = if self.demo { Source::Demo } else { Source::find() };
        self.reading = source.read();
        let now = now();
        self.points = if self.demo { demo_history(now) } else { history() };
        self.points.retain(|p| p.0 >= now - 24 * 3600);
        self.last = crate::deep::last();
    }

    fn describe(&self, row: Row, c: &Choices) -> (&'static str, String, bool) {
        let onoff = |on: bool| (if on { "on" } else { "off" }).to_owned();
        let r = self.reading.as_ref();
        match row {
            Row::Saver => ("Battery saver", onoff(c.saver), c.saver),
            Row::SaverAuto => ("On by itself at 20%", onoff(c.saver_auto), c.saver_auto),
            Row::EmptyOff => ("Turn off at 5%", onoff(c.empty_off), c.empty_off),
            Row::WifiSleeps => ("Wi-Fi in standby", (if c.wifi_sleeps { "off after 10 min" } else { "stays on" }).into(), c.wifi_sleeps),
            Row::Deep => ("Deep standby", crate::deep::after_value(c.deep_after), c.deep_after > 0),
            Row::RedLight => ("Red light", (if c.quiet_light { "quiet" } else { "card activity" }).into(), true),
            Row::Drawing => ("Drawing now", r.and_then(Reading::draw_line).unwrap_or_else(|| "–".into()), false),
            Row::LastDeep => ("Last sleep", self.last.map_or_else(|| "–".into(), |s| s.summary()), false),
            Row::Health => (
                "Health",
                match (r.and_then(|r| r.health), r.and_then(|r| r.cycles)) {
                    (Some(h), Some(n)) => format!("{h}% · {n} cycles"),
                    (Some(h), None) => format!("{h}%"),
                    (None, Some(n)) => format!("{n} cycles"),
                    (None, None) => "–".into(),
                },
                false,
            ),
            Row::Temperature => ("Temperature", r.and_then(|r| r.celsius).map_or("–".into(), |c| format!("{c:.1} °c")), false),
            Row::Voltage => ("Voltage", r.and_then(|r| r.volts).map_or("–".into(), |v| format!("{v:.2} v")), false),
        }
    }

    fn draw_hero(&mut self, g: &mut Gfx, top: f32, shown: f32) {
        if shown < 0.02 {
            return;
        }
        let t = g.tokens;
        let ty = g.ty;
        let h = HERO_H * shown;
        let y = top - (1.0 - shown) * HERO_H * 0.5;
        g.canvas.set_clip(Rect::new(0.0, top, W, h));
        let big = ty.display(28.0);
        let (percent, status, line) = match &self.reading {
            Some(r) => (format!("{}%", r.percent), r.status(), r.draw_line().unwrap_or_default()),
            None => ("–".to_owned(), "no battery gauge".to_owned(), String::new()),
        };
        let x = PAD + 2.0;
        let w = g.measure(&percent, big).max(g.measure("88%", big));
        g.text_alpha(x, y + 18.0, &percent, big, t.ink, shown);
        // Beneath: how it stands, and what it draws where both fit.
        let room = W - 2.0 * PAD - 4.0;
        let both = g.measure(&status, ty.caption) + 18.0 + g.measure(&line, ty.caption) <= room;
        let status = g.text.fit(&status, &ty.caption, room);
        g.text_alpha(x + 1.0, y + 48.0, &status, ty.caption, t.dim, shown);
        if both && !line.is_empty() {
            let lw = g.measure(&line, ty.caption);
            g.text_alpha(W - PAD - 2.0 - lw, y + 48.0, &line, ty.caption, t.dim, shown);
        }
        // The last day, beside the charge.
        let cx0 = x + w + 14.0;
        let chart = Rect::new(cx0, y + 2.0, W - PAD - 2.0 - cx0, 34.0);
        g.canvas.rrect(chart.offset(g.ox, 0.0), 8.0, t.surface1, shown);
        let inner = chart.inset(5.0);
        let low = self.reading.as_ref().is_some_and(|r| r.percent <= WARN_AT[0] && r.flow.on_battery());
        let color: Color = if low { g.fault() } else { t.signal };
        if self.points.len() >= 2 {
            // One value per five minutes across the day; gaps keep the last.
            let start = now() - 24 * 3600;
            let mut values = vec![f32::NAN; 289];
            for p in &self.points {
                let i = ((p.0 - start) / 300).clamp(0, 288) as usize;
                values[i] = p.1 as f32;
            }
            let first = values.iter().position(|v| !v.is_nan()).unwrap_or(0);
            let mut last = values[first];
            let filled: Vec<f32> = values[first..]
                .iter()
                .map(|v| {
                    if !v.is_nan() {
                        last = *v;
                    }
                    last
                })
                .collect();
            crate::tools::sparkline(g, inner, &filled, 100.0, color, 289);
            // What the chart spans, low in its first corner: the day starts
            // there, so the line is seldom low enough to cross it.
            let label = Rect::new(inner.x - 1.0, inner.bottom() - ty.caption.size - 1.0, g.measure("24 h", ty.caption) + 6.0, ty.caption.size + 2.0);
            g.canvas.rrect(label.offset(g.ox, 0.0), 4.0, t.surface1, 0.85 * shown);
            g.text_alpha(label.x + 3.0, label.cy(), "24 h", ty.caption, t.dim, shown);
        } else {
            let note = "the day's charge, from now";
            let fitted = g.text.fit(note, &ty.caption, inner.w);
            g.text_alpha(inner.x + 2.0, inner.cy(), &fitted, ty.caption, t.faint, shown);
        }
        g.canvas.clear_clip();
    }
}

impl Screen for BatteryPage {
    fn title(&self) -> &str {
        "Battery"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        match key {
            Key::Back => return Nav::Pop,
            // Deep standby: Enter opens what it does and draws; ←→ choose when.
            Key::Enter if row == Row::Deep => return Nav::Push(Box::new(crate::deep::DeepPage::new())),
            Key::Left | Key::Right if row == Row::Deep => {
                let c = &mut cx.settings.battery;
                c.deep_after = crate::deep::step_after(c.deep_after, if key == Key::Left { -1 } else { 1 });
                crate::save_settings(cx.settings);
            }
            Key::Enter | Key::Left | Key::Right => {
                let c = &mut cx.settings.battery;
                match row {
                    Row::Saver => c.saver = !c.saver,
                    Row::SaverAuto => c.saver_auto = !c.saver_auto,
                    Row::EmptyOff => c.empty_off = !c.empty_off,
                    Row::WifiSleeps => c.wifi_sleeps = !c.wifi_sleeps,
                    Row::RedLight => c.quiet_light = !c.quiet_light,
                    _ => return Nav::Stay,
                }
                if row == Row::Saver {
                    *cx.say = Some(if c.saver { "battery saver on".into() } else { "battery saver off".into() });
                }
                crate::save_settings(cx.settings);
            }
            other => crate::screens::list_key(&mut self.list, other, ROWS.len()),
        }
        let want = if self.list.focus >= 2 { 0.0 } else { 1.0 };
        if self.hero.target() != want {
            self.hero.set(want);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.hero.tick(dt);
        if self.read_at.elapsed().as_secs_f32() < 5.0 {
            return moving;
        }
        let before = self.reading.clone();
        self.read();
        moving || before != self.reading
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if g.calm {
            self.hero.jump(self.hero.target());
        }
        let top = g.top();
        let shown = self.hero.value().clamp(0.0, 1.0);
        self.draw_hero(g, top, shown);
        let list_top = top + HERO_H * shown;
        let (row_h, bottom) = (g.row_h(), H);
        let choices = g.battery;
        for (i, y) in self.list.layout(g, ROWS.len(), list_top, row_h, 3.0, bottom) {
            let (name, value, on) = self.describe(ROWS[i], &choices);
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            g.row(r, name, &value, if on { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, ROWS.len(), list_top, bottom - 2.0);
    }
}

/// The gauge now (the demo battery on a computer), for a page that shows
/// the charge without the shell's own readings (StandBy's preview).
pub fn reading_now() -> Option<Reading> {
    Source::find().read()
}

/// "76% · 4 h 38 m left": Settings' summary of the battery.
pub fn summary() -> String {
    match Source::find().read() {
        Some(r) => match (r.flow, r.minutes_left) {
            (Flow::Discharging, Some(m)) => format!("{}% · {} left", r.percent, crate::sys::minutes_text(m)),
            (Flow::Charging, _) => format!("{}% · charging", r.percent),
            _ => format!("{}%", r.percent),
        },
        None => "–".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(percent: u32) -> Reading {
        Reading { percent, flow: Flow::Discharging, ..Reading::default() }
    }

    fn charging(percent: u32) -> Reading {
        Reading { percent, flow: Flow::Charging, ..Reading::default() }
    }

    fn says(events: &[Event]) -> Vec<String> {
        events.iter().filter_map(|e| if let Event::Say(s) = e { Some(s.clone()) } else { None }).collect()
    }

    #[test]
    fn choices_round_trip_and_default_to_saving() {
        let c = Choices::default();
        assert!(!c.saver && c.saver_auto && c.empty_off && c.wifi_sleeps && c.quiet_light && c.deep_after == 30);
        let mut back = Choices { saver: true, saver_auto: false, empty_off: false, wifi_sleeps: false, quiet_light: false, deep_after: 0 };
        for (k, v) in c.lines() {
            assert!(back.parse(k, &v));
        }
        assert_eq!(back, c);
        assert!(!back.parse("light", "true"));
        // Deep standby: each choice read back; anything else, half an hour.
        for minutes in crate::deep::AFTER_CHOICES {
            let c = Choices { deep_after: minutes, ..Choices::default() };
            let mut back = Choices::default();
            for (k, v) in c.lines() {
                back.parse(k, &v);
            }
            assert_eq!(back.deep_after, minutes);
        }
        for odd in ["45", "-1", "half an hour", ""] {
            back.parse("deep_after", odd);
            assert_eq!(back.deep_after, 30, "{odd:?}");
        }
        assert_eq!(ROWS[DEEP_ROW], Row::Deep);
    }

    #[test]
    fn a_gauges_files_read_as_a_reading() {
        let dir = std::env::temp_dir().join(format!("catalyst-gauge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, value) in [
            ("capacity", "42"),
            ("status", "Discharging"),
            ("current_now", "-310000"),
            ("voltage_now", "3790000"),
            ("time_to_empty_now", "16200"),
            ("temp", "291"),
            ("cycle_count", "7"),
            ("charge_full", "1680000"),
            ("charge_full_design", "1750000"),
            ("charge_now", "705000"),
        ] {
            std::fs::write(dir.join(name), format!("{value}\n")).unwrap();
        }
        let r = Reading::from_dir(&dir).unwrap();
        assert_eq!((r.percent, r.flow), (42, Flow::Discharging));
        assert_eq!(r.charge_mah, Some(705.0), "the charge left, for deep standby's draw");
        assert_eq!(r.milliamps, Some(310.0), "drawn, whichever sign the gauge gives");
        assert_eq!(r.minutes_left, Some(270));
        assert_eq!((r.health, r.cycles), (Some(96), Some(7)));
        assert_eq!(r.status(), "on battery · 4 h 30 m left");
        assert_eq!(r.draw_line().unwrap(), "310 mA · 1.17 W");
        // A gauge that reports the current unsigned, charging.
        std::fs::write(dir.join("status"), "Charging\n").unwrap();
        std::fs::write(dir.join("current_now"), "820000\n").unwrap();
        let r = Reading::from_dir(&dir).unwrap();
        assert_eq!(r.milliamps, Some(-820.0));
        assert_eq!(r.minutes_left, None);
        assert_eq!(r.draw_line().unwrap(), "charging at 820 mA");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_saver_comes_on_at_twenty_and_goes_on_the_charger() {
        let mut care = Care::from(Source::Nothing);
        let mut c = Choices::default();
        assert!(care.observe(&on_battery(21), &mut c).is_empty());
        let events = care.observe(&on_battery(20), &mut c);
        assert!(c.saver && events.contains(&Event::Saver));
        assert_eq!(says(&events), ["battery at 20% · battery saver on"]);
        // Switched off by hand, it stays off this time on battery.
        c.saver = false;
        assert!(care.observe(&on_battery(19), &mut c).is_empty());
        // Switched on by the rule, the charger switches it off.
        let mut care = Care::from(Source::Nothing);
        let mut c = Choices::default();
        care.observe(&on_battery(18), &mut c);
        let events = care.observe(&charging(18), &mut c);
        assert!(!c.saver && events.contains(&Event::Saver));
        // Switched on by hand, it stays on.
        let mut c = Choices { saver: true, ..Choices::default() };
        let mut care = Care::from(Source::Nothing);
        care.observe(&on_battery(18), &mut c);
        care.observe(&charging(18), &mut c);
        assert!(c.saver);
    }

    #[test]
    fn warnings_come_once_each_and_blink() {
        let mut care = Care::from(Source::Nothing);
        let mut c = Choices { saver_auto: false, ..Choices::default() };
        let first = care.observe(&on_battery(15), &mut c);
        assert!(first.contains(&Event::Warn));
        assert_eq!(says(&first), ["battery at 15%"]);
        assert!(care.observe(&on_battery(14), &mut c).is_empty(), "not again at 14");
        assert_eq!(says(&care.observe(&on_battery(10), &mut c)), ["battery at 10% · plug in soon"]);
        assert!(care.observe(&on_battery(9), &mut c).is_empty());
        // From 16 straight to 9: one warning, the lower.
        let mut care = Care::from(Source::Nothing);
        care.observe(&on_battery(16), &mut c);
        assert_eq!(says(&care.observe(&on_battery(9), &mut c)), ["battery at 9% · plug in soon"]);
        // Charged and on battery again, they come again.
        care.observe(&charging(40), &mut c);
        assert!(care.observe(&on_battery(15), &mut c).contains(&Event::Warn));
    }

    #[test]
    fn it_turns_off_after_three_readings_at_five_and_not_when_asked_not_to() {
        let mut care = Care::from(Source::Nothing);
        let mut c = Choices { saver_auto: false, ..Choices::default() };
        care.observe(&on_battery(6), &mut c);
        assert!(!care.observe(&on_battery(5), &mut c).contains(&Event::Empty));
        assert!(!care.observe(&on_battery(5), &mut c).contains(&Event::Empty));
        assert!(care.observe(&on_battery(5), &mut c).contains(&Event::Empty));
        assert!(!care.observe(&on_battery(5), &mut c).contains(&Event::Empty), "once");
        // A reading back above five (the gauge settling) starts the count again.
        let mut care = Care::from(Source::Nothing);
        care.observe(&on_battery(5), &mut c);
        care.observe(&on_battery(5), &mut c);
        care.observe(&on_battery(6), &mut c);
        assert!(!care.observe(&on_battery(5), &mut c).contains(&Event::Empty));
        // Switched off: it runs on.
        let mut c = Choices { empty_off: false, saver_auto: false, ..Choices::default() };
        let mut care = Care::from(Source::Nothing);
        for _ in 0..5 {
            assert!(!care.observe(&on_battery(3), &mut c).contains(&Event::Empty));
        }
    }

    #[test]
    fn put_off_with_esc_it_waits_for_two_percent() {
        let mut care = Care::from(Source::Nothing);
        let mut c = Choices { saver_auto: false, ..Choices::default() };
        for _ in 0..3 {
            care.observe(&on_battery(5), &mut c);
        }
        let mut card = Empty::new(true);
        let mut say = None;
        let (mut robot, mut settings) = (Robot::none(), crate::shell::Settings::default());
        card.key(Key::Back, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        for _ in 0..4 {
            assert!(!care.observe(&on_battery(4), &mut c).contains(&Event::Empty));
        }
        assert!(postponed_floor(&care));
        care.observe(&on_battery(2), &mut c);
        care.observe(&on_battery(2), &mut c);
        assert!(care.observe(&on_battery(2), &mut c).contains(&Event::Empty));
        // The card counts down from ten seconds and cannot be put off at the floor.
        let mut card = Empty::new(true).at_floor();
        card.key(Key::Back, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        assert!(card.postponed.is_none());
        card.tick(4.0);
        assert!((card.left - 6.0).abs() < 0.01);
    }

    #[test]
    fn the_history_reads_back_and_the_demo_day_is_a_day() {
        let text = "1773400000 80 d\n1773400300 79 d\nnot a line\n1773400600 81 c\n";
        assert_eq!(parse_history(text), [(1773400000, 80, false), (1773400300, 79, false), (1773400600, 81, true)]);
        assert_eq!(history_line((5, 50, true)), "5 50 c\n");
        let day = demo_history(1_000_000);
        assert_eq!(day.len(), 289);
        assert_eq!(day.first().unwrap().0, 1_000_000 - 86_400);
        assert!(day.iter().all(|p| p.1 <= 100));
    }

    #[test]
    fn the_battery_page_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        // The longest a last deep standby reads.
        let last = crate::deep::Slept { woke: 0, seconds: 23 * 3600 + 59 * 60, watts: Some(0.888), charging: false };
        let mut page = BatteryPage { list: ListState::default(), reading: Some(Reading::demo()), points: demo_history(now()), read_at: std::time::Instant::now(), hero: Animated::new(1.0, role::page()), demo: true, last: Some(last) };
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let width = W - 2.0 * PAD - 5.0;
            for last in [Some(last), Some(crate::deep::Slept { charging: true, ..last }), None] {
                page.last = last;
                for row in ROWS {
                    for c in [Choices::default(), Choices { saver: true, wifi_sleeps: false, quiet_light: false, deep_after: 60, ..Choices::default() }, Choices { deep_after: 0, ..Choices::default() }] {
                        let (name, value, _) = page.describe(row, &c);
                        let room = width - 30.0 - g.measure(&value, ty.label);
                        assert!(g.measure(name, ty.body) <= room, "{name} beside {value} at {size:?}");
                    }
                }
            }
            // The header: the charge with the chart beside it, and its line beneath.
            let r = Reading::demo();
            let percent_w = g.measure("100%", ty.display(28.0));
            assert!(W - PAD - 2.0 - (PAD + 2.0 + percent_w + 14.0) >= 120.0, "the chart keeps room at {size:?}");
            for line in [r.status(), Reading { flow: Flow::Charging, minutes_to_full: Some(85), ..r.clone() }.status(), "no battery gauge".to_owned()] {
                assert!(g.measure(&line, ty.caption) <= W - 2.0 * PAD - 4.0, "{line} at {size:?}");
            }
            assert!(48.0 + ty.caption.size * 0.5 <= HERO_H, "the line beneath sits in the header at {size:?}");
        }
    }

    #[test]
    fn the_turning_off_card_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for line in ["Turning off in 10 s to keep", "your files safe. Plug in to stay on.", "Charge it, then slide the switch", "off and on again.", "It turns off at 2% instead."] {
                assert!(g.measure(line, ty.body) <= W - 2.0 * PAD, "{line} at {size:?}");
            }
            for title in ["Battery empty", "Turning off", "Staying on", "Charging"] {
                assert!(g.measure(title, ty.display(24.0)) <= W - 2.0 * PAD, "{title}");
            }
        }
    }

    #[test]
    fn the_firmwares_voltages_are_read_in_either_unit() {
        let near = |raw: u32, want: f32| mailbox_volts(raw).is_some_and(|v| (v - want).abs() < 1e-4);
        assert!(near(1_200_000, 1.2), "microvolts, as current firmware gives");
        assert!(near(1_175_000, 1.175));
        assert!(near((-1i32) as u32, 1.175), "a step of 25 mV below 1.2 V, as older firmware gave");
        assert!(near(0, 1.2));
        assert_eq!(mailbox_volts(77), None, "neither: not taken for a voltage");
        assert_eq!(core_volts(), None, "off the handheld there is no firmware to ask");
    }

    #[test]
    fn lower_voltage_says_whether_the_firmware_took_it() {
        let read = |max: f32| Some(CoreVolts { max, min: max - 0.025, now: max - 0.025 });
        assert!(lowered(read(1.175).unwrap(), None), "25 mV under the firmware's own 1.2 V");
        assert!(!lowered(read(1.2).unwrap(), None));
        assert!(lowered(read(1.2625).unwrap(), Some(1.2875)), "a board whose own ceiling is higher");
        assert!(!lowered(read(1.2875).unwrap(), Some(1.2875)));
        assert_eq!(line_for(Undervolt::On, read(1.175), None), "on · 1.175 V");
        assert_eq!(line_for(Undervolt::Trying, read(1.175), Some(1.2)), "trying · 1.175 V");
        assert_eq!(line_for(Undervolt::On, read(1.2), None), "on · not taken");
        assert_eq!(line_for(Undervolt::On, None, None), "on · −25 mV", "no firmware to ask");
        assert_eq!(line_for(Undervolt::Pending, None, None), "set · restart to try");
        assert_eq!(line_for(Undervolt::Off, read(1.2), None), "off");
        assert_eq!(line_for(Undervolt::Failed, None, None), "did not hold");
        assert_eq!(undervolt_line(), "on the handheld");
        crate::panel::row_fits(
            "Lower voltage",
            &[line_for(Undervolt::Trying, read(1.175), None), line_for(Undervolt::Trying, read(1.2), None), line_for(Undervolt::Pending, None, None), line_for(Undervolt::Failed, None, None)],
        );
    }

    #[test]
    fn lower_voltage_states_from_catalyst_power() {
        assert_eq!(state_of("trying 0\n"), Undervolt::Pending);
        assert_eq!(state_of("trying 1\n"), Undervolt::Trying);
        assert_eq!(state_of("on\n"), Undervolt::On);
        assert_eq!(state_of("failed\n"), Undervolt::Failed);
        assert_eq!(state_of(""), Undervolt::Off);
        // Enter asks to restart and try it unless it is already in.
        assert!(Undervolt::Off.asks_to_try() && Undervolt::Failed.asks_to_try() && Undervolt::Pending.asks_to_try());
        assert!(!Undervolt::On.asks_to_try() && !Undervolt::Trying.asks_to_try());
        assert!(Undervolt::Pending.on() && Undervolt::Trying.on() && Undervolt::On.on() && !Undervolt::Off.on());
    }

    #[test]
    fn trying_the_lower_voltage_asks_before_restarting() {
        let mut ask = TryUndervolt::new();
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        let mut press = |ask: &mut TryUndervolt, key: Key, say: &mut Option<String>| ask.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say, at_rest: true });
        // Esc: not now, nothing changed; other keys do nothing.
        assert!(matches!(press(&mut ask, Key::Down, &mut say), Nav::Stay));
        assert!(matches!(press(&mut ask, Key::Back, &mut say), Nav::Pop));
        assert_eq!(say, None);
        // Enter restarts into it: off the handheld, it says where it is set.
        assert!(matches!(press(&mut TryUndervolt::new(), Key::Enter, &mut say), Nav::Pop));
        assert_eq!(say.as_deref(), Some("the voltage is set on the handheld"));
    }

    #[test]
    fn what_trying_the_lower_voltage_says_fits_above_its_row() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let lines = crate::help::wrap(&mut g, TRY_WORDS, ty.caption, W - 2.0 * PAD).len() as f32;
            assert!(g.top() + 2.0 + lines * (ty.caption.size + 4.0) <= try_row(&g).y - 6.0, "{lines} lines at {size:?}");
        }
    }
}
