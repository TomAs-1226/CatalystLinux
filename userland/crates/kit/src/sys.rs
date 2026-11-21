//! The few facts about the handheld an app's chrome needs, read as the
//! shell reads them: the battery, the backlight, and this app's memory.

use std::fs;

fn read(path: &std::path::Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// The battery's charge, 0–100, from the fuel gauge.
pub fn battery_percent() -> Option<u32> {
    fs::read_dir("/sys/class/power_supply").ok()?.flatten().find_map(|e| read(&e.path().join("capacity"))?.parse().ok())
}

fn backlight() -> Option<std::path::PathBuf> {
    fs::read_dir("/sys/class/backlight").ok()?.flatten().next().map(|e| e.path())
}

/// The backlight, 0–100.
pub fn brightness() -> Option<u32> {
    let dir = backlight()?;
    let now: u32 = read(&dir.join("brightness"))?.parse().ok()?;
    let max: u32 = read(&dir.join("max_brightness"))?.parse().ok()?;
    (max > 0).then(|| (now * 100 + max / 2) / max)
}

/// Set the backlight; never fully off, so the screen cannot be lost. The
/// shell keeps its own setting and puts it back when the app ends.
pub fn set_brightness(percent: u32) -> bool {
    let Some(dir) = backlight() else { return false };
    let Some(max) = read(&dir.join("max_brightness")).and_then(|m| m.parse::<u32>().ok()) else { return false };
    let value = (percent.clamp(5, 100) * max / 100).max(1);
    fs::write(dir.join("brightness"), value.to_string()).is_ok()
}

/// This program's memory as the kernel counts it: resident now, at most,
/// and its proportional share (what it adds to the system), in KB.
pub fn memory() -> Option<(u64, u64, Option<u64>)> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| status.lines().find_map(|l| l.strip_prefix(name)?.split_whitespace().next()?.parse::<u64>().ok());
    let pss = fs::read_to_string("/proc/self/smaps_rollup").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("Pss:")?.split_whitespace().next()?.parse().ok()));
    Some((field("VmRSS:")?, field("VmHWM:")?, pss))
}

/// [`memory`] as one line, for `CATALYST_KIT_STATS`.
pub fn memory_line() -> Option<String> {
    let (rss, peak, pss) = memory()?;
    Some(format!("catalyst-kit: memory {rss} KB resident, {peak} KB at most, {} KB its share", pss.map_or("?".into(), |p| p.to_string())))
}
