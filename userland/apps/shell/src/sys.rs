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

/// The time of day in hours and minutes, on the 24-hour clock or the 12-hour.
pub fn clock(h24: bool) -> String {
    chrono::Local::now().format(if h24 { "%H:%M" } else { "%-I:%M" }).to_string()
}

/// Battery charge alone, 0–100.
pub fn battery_percent() -> Option<u32> {
    fs::read_dir("/sys/class/power_supply").ok()?.flatten().find_map(|e| read(&e.path().join("capacity").to_string_lossy())?.parse().ok())
}

/// Whether a network interface has a link, without asking another program.
pub fn link_up(interface: &str) -> bool {
    read(&format!("/sys/class/net/{interface}/operstate")).is_some_and(|state| state == "up")
}
