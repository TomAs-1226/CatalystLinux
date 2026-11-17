//! What the handheld knows about itself, read from the kernel. Every value
//! is an `Option`: on a development PC there is nothing to read, and the
//! screens show a dash rather than a made-up number.

use std::fs;

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

/// Battery charge and whether it is charging, from the fuel gauge.
pub fn battery() -> Option<String> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let Some(percent) = read(&dir.join("capacity").to_string_lossy()) else { continue };
        let status = read(&dir.join("status").to_string_lossy()).unwrap_or_default().to_lowercase();
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
    let out = std::process::Command::new("ip").args(["-4", "-o", "addr", "show", "dev", interface]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.split_whitespace().skip_while(|w| *w != "inet").nth(1) {
        Some(cidr) => Some(cidr.split('/').next().unwrap_or(cidr).to_owned()),
        None if state == "down" => Some("not connected".into()),
        None => Some("no address".into()),
    }
}

fn backlight() -> Option<std::path::PathBuf> {
    fs::read_dir("/sys/class/backlight").ok()?.flatten().next().map(|e| e.path())
}

/// Backlight brightness, 0–100.
pub fn brightness() -> Option<u32> {
    let dir = backlight()?;
    let now: u32 = read(&dir.join("brightness").to_string_lossy())?.parse().ok()?;
    let max: u32 = read(&dir.join("max_brightness").to_string_lossy())?.parse().ok()?;
    (max > 0).then(|| (now * 100 + max / 2) / max)
}

/// Set the backlight; never fully off, so the screen cannot be lost.
pub fn set_brightness(percent: u32) -> bool {
    let Some(dir) = backlight() else { return false };
    let Some(max) = read(&dir.join("max_brightness").to_string_lossy()).and_then(|m| m.parse::<u32>().ok()) else { return false };
    let value = (percent.clamp(5, 100) * max / 100).max(1);
    fs::write(dir.join("brightness"), value.to_string()).is_ok()
}
