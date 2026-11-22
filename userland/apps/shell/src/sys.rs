//! What the handheld knows about itself, read from the kernel. Every value
//! is an `Option`: on a development PC there is nothing to read, and the
//! screens show a dash rather than a made-up number.

use std::fs;

/// Replace a file's contents so that a power cut at any moment leaves either
/// the old contents or the new, never an empty or half-written file: write a
/// temporary file beside it, put it on the card, then rename it over. A file
/// that was there keeps its permissions.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    write_replacing(path, bytes, false)
}

/// As `write_atomic`, for secrets (keys, passwords): readable by root alone
/// from the moment the file exists, not made so afterwards.
pub fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    write_replacing(path, bytes, true)
}

fn write_replacing(path: &std::path::Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write;
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".catalyst-saving");
    let temp = path.with_file_name(name);
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        if let (false, Ok(meta)) = (private, fs::metadata(path)) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        fs::rename(&temp, path)?;
        // The rename itself, on the card.
        #[cfg(unix)]
        if let Some(dir) = path.parent().and_then(|d| fs::File::open(if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d }).ok()) {
            dir.sync_all().ok();
        }
        Ok(())
    })();
    if result.is_err() {
        fs::remove_file(&temp).ok();
    }
    result
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_owned())
}

fn meminfo(name: &str) -> Option<u64> {
    read("/proc/meminfo")?.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse().ok()
}

pub fn kernel() -> Option<String> {
    read("/proc/sys/kernel/osrelease")
}

pub fn model() -> Option<String> {
    read("/proc/device-tree/model")
}

pub fn hostname() -> Option<String> {
    read("/proc/sys/kernel/hostname")
}

/// Running on the handheld itself, in Catalyst Linux (not the simulator,
/// and not a test on a Linux computer): only then is the system's own
/// state (its name, its clock) changed.
pub fn on_handheld() -> bool {
    cfg!(target_os = "linux") && std::path::Path::new("/etc/catalyst/version").is_file()
}

pub fn uptime() -> Option<String> {
    let seconds = read("/proc/uptime")?.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    Some(match seconds {
        s if s >= 86_400 => format!("{} d {} h", s / 86_400, s % 86_400 / 3600),
        s if s >= 3600 => format!("{} h {} m", s / 3600, s % 3600 / 60),
        s => format!("{} m", s / 60),
    })
}

/// Memory in use across the system: "46 of 427 mb".
pub fn memory() -> Option<String> {
    let (total, available) = (meminfo("MemTotal:")?, meminfo("MemAvailable:")?);
    Some(format!("{:.0} of {:.0} mb", (total - available) as f64 / 1024.0, total as f64 / 1024.0))
}

/// This program's own resident memory.
pub fn shell_memory() -> Option<String> {
    let kb: u64 = read("/proc/self/status")?.lines().find(|l| l.starts_with("VmRSS:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(format!("{:.1} mb", kb as f64 / 1024.0))
}

pub fn cpu_temperature() -> Option<String> {
    let milli: f64 = read("/sys/class/thermal/thermal_zone0/temp")?.parse().ok()?;
    Some(format!("{:.0} °c", milli / 1000.0))
}

/// Battery charge and whether it is charging, from the fuel gauge; on
/// battery, how long it has left when the gauge can say.
pub fn battery() -> Option<String> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let Some(percent) = read(&dir.join("capacity").to_string_lossy()) else { continue };
        let status = read(&dir.join("status").to_string_lossy()).unwrap_or_default().to_lowercase();
        if let Some(left) = battery_minutes_left() {
            return Some(format!("{percent}% · {} left", minutes_text(left)));
        }
        return Some(if status.is_empty() || status == "unknown" { format!("{percent}%") } else { format!("{percent}% · {status}") });
    }
    None
}

/// Space used on the data partition: "1.2 of 29 gb".
#[cfg(target_os = "linux")]
pub fn storage() -> Option<String> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c"/data".as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let gb = |blocks: u64| blocks as f64 * stat.f_frsize as f64 / 1e9;
    let (total, free) = (gb(stat.f_blocks as u64), gb(stat.f_bavail as u64));
    Some(if total < 1.0 { format!("{:.0} of {:.0} mb", (total - free) * 1000.0, total * 1000.0) } else { format!("{:.1} of {:.0} gb", total - free, total) })
}

#[cfg(not(target_os = "linux"))]
pub fn storage() -> Option<String> {
    None
}

/// An interface's IPv4 address, "no cable" for a wired link that is down,
/// or nothing if the interface does not exist.
pub fn address(interface: &str) -> Option<String> {
    let state = read(&format!("/sys/class/net/{interface}/operstate"))?;
    match ipv4(interface) {
        Some(address) => Some(address),
        None if state == "down" => Some("not connected".into()),
        None => Some("no address".into()),
    }
}

/// An interface's IPv4 address, asked of the kernel directly: this is read
/// every few seconds while a page shows it, so no program is started for it.
#[cfg(target_os = "linux")]
fn ipv4(interface: &str) -> Option<String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut at = list;
    while !at.is_null() && found.is_none() {
        // Safe: the list is the kernel's, valid until freed below.
        let entry = unsafe { &*at };
        let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) };
        if !entry.ifa_addr.is_null() && name.to_bytes() == interface.as_bytes() && unsafe { (*entry.ifa_addr).sa_family } as i32 == libc::AF_INET {
            let v4 = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
            found = Some(std::net::Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr)).to_string());
        }
        at = entry.ifa_next;
    }
    unsafe { libc::freeifaddrs(list) };
    found
}

#[cfg(not(target_os = "linux"))]
fn ipv4(_interface: &str) -> Option<String> {
    None
}

fn backlight() -> Option<std::path::PathBuf> {
    fs::read_dir("/sys/class/backlight").ok()?.flatten().next().map(|e| e.path())
}

/// Backlight brightness, 0–100, as the eye sees it (see `level_for`).
pub fn brightness() -> Option<u32> {
    let dir = backlight()?;
    let now: u32 = read(&dir.join("brightness").to_string_lossy())?.parse().ok()?;
    let max: u32 = read(&dir.join("max_brightness").to_string_lossy())?.parse().ok()?;
    (max > 0).then(|| percent_for(now, max))
}

/// Set the backlight; never fully off, so the screen cannot be lost.
pub fn set_brightness(percent: u32) -> bool {
    let Some(dir) = backlight() else { return false };
    let Some(max) = read(&dir.join("max_brightness").to_string_lossy()).and_then(|m| m.parse::<u32>().ok()) else { return false };
    fs::write(dir.join("brightness"), level_for(percent, max).to_string()).is_ok()
}

/// The lowest brightness offered, in percent, and the least of the
/// backlight's levels it may come to (so it stays readable indoors).
const LEAST_PERCENT: u32 = 5;
const LEAST_SHARE: f32 = 0.012;

/// The backlight's level for a brightness in percent: on a perceptual
/// curve (the square, close to how the eye sees light), as phones do. Even
/// steps on the slider look even, and the light it takes falls away fast:
/// 70% is half the light the straight line gave, so about 40% less of the
/// backlight's power at the usual setting.
pub fn level_for(percent: u32, max: u32) -> u32 {
    let p = percent.clamp(LEAST_PERCENT, 100) as f32 / 100.0;
    let share = (p * p).max(LEAST_SHARE);
    ((share * max as f32).round() as u32).clamp(1, max.max(1))
}

/// The brightness in percent a backlight level is, the curve undone: what
/// `level_for` gives back the same level for, so a brightness read and
/// written again (standby dimming and coming back) never drifts.
pub fn percent_for(level: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let guess = ((level as f32 / max as f32).sqrt() * 100.0).round() as u32;
    // The nearest percent whose level is this level, else the nearest level.
    (LEAST_PERCENT..=100)
        .min_by_key(|&p| (level_for(p, max).abs_diff(level), p.abs_diff(guess)))
        .unwrap_or(guess)
        .clamp(LEAST_PERCENT, 100)
}

#[cfg(test)]
mod brightness_tests {
    use super::*;

    #[test]
    fn the_curve_saves_light_and_reads_back_what_it_set() {
        for max in [100, 255, 1023, 4095] {
            // Half the light at 70%, and full at 100%.
            assert!((level_for(70, max) as f32 / max as f32 - 0.49).abs() < 0.02, "max {max}");
            assert_eq!(level_for(100, max), max);
            // Never dark, never less than the least share.
            assert!(level_for(0, max) >= 1);
            // Read and written again: the same level, every one reachable.
            for p in 0..=100 {
                let level = level_for(p, max);
                assert_eq!(level_for(percent_for(level, max), max), level, "max {max}, {p}%");
            }
            // Brighter is never dimmer.
            assert!((LEAST_PERCENT..100).all(|p| level_for(p + 1, max) >= level_for(p, max)));
        }
    }
}

/// The backlight switched on or off (`bl_power`), its brightness kept for
/// when it comes back: standby's "off". False if there is none to switch.
pub fn backlight_power(on: bool) -> bool {
    let Some(dir) = backlight() else { return false };
    // 0 is on; 4 is FB_BLANK_POWERDOWN.
    fs::write(dir.join("bl_power"), if on { "0" } else { "4" }).is_ok()
}

/// The ways the kernel offers of choosing the processor's speed.
pub fn governors() -> Vec<String> {
    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors").map(|s| s.split_whitespace().map(str::to_owned).collect()).unwrap_or_default()
}

/// Set every processor's governor to the first of `wanted` the kernel has.
/// False if it has none of them, or there is no cpufreq (a computer).
pub fn set_governor(wanted: &[&str]) -> bool {
    let have = governors();
    let Some(pick) = wanted.iter().find(|w| have.iter().any(|h| h == *w)) else { return false };
    let mut set = false;
    for policy in fs::read_dir("/sys/devices/system/cpu/cpufreq").into_iter().flatten().flatten() {
        let file = policy.path().join("scaling_governor");
        if file.exists() {
            set |= fs::write(file, pick).is_ok();
        }
    }
    set || fs::write("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor", pick).is_ok()
}

/// Minutes of battery left, while it runs on battery: the fuel gauge's own
/// reckoning, or its charge (or energy) over the current (or power) drawn.
/// Nothing while charging, or if the gauge does not say.
pub fn battery_minutes_left() -> Option<u64> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let value = |name: &str| read(&dir.join(name).to_string_lossy()).and_then(|v| v.parse::<f64>().ok()).map(f64::abs);
        if value("capacity").is_none() {
            continue;
        }
        let status = read(&dir.join("status").to_string_lossy()).unwrap_or_default().to_lowercase();
        if status != "discharging" {
            return None;
        }
        let hours = value("time_to_empty_now")
            .or_else(|| value("time_to_empty_avg"))
            .map(|seconds| seconds / 3600.0)
            .or_else(|| Some(value("energy_now")? / value("power_now").filter(|p| *p > 0.0)?))
            .or_else(|| Some(value("charge_now")? / value("current_now").filter(|c| *c > 0.0)?))?;
        let minutes = (hours * 60.0).round();
        return (1.0..=7.0 * 24.0 * 60.0).contains(&minutes).then_some(minutes as u64);
    }
    None
}

/// Minutes as a short time: "3 h 10 m", "45 m".
pub fn minutes_text(minutes: u64) -> String {
    if minutes >= 60 {
        format!("{} h {} m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} m")
    }
}

/// Battery charge alone, 0–100.
pub fn battery_percent() -> Option<u32> {
    fs::read_dir("/sys/class/power_supply").ok()?.flatten().find_map(|e| read(&e.path().join("capacity").to_string_lossy())?.parse().ok())
}

/// Whether a network interface has a link, without asking another program.
pub fn link_up(interface: &str) -> bool {
    read(&format!("/sys/class/net/{interface}/operstate")).is_some_and(|state| state == "up")
}
