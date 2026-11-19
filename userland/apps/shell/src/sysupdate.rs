//! System updates as the shell shows them. The work is `catalyst-update`'s
//! (a program of its own, so an install carries on whatever the shell does);
//! the shell starts each step, reads where it stands from the status file,
//! and says so: in Settings › System update, and in the island.

use std::path::PathBuf;

/// Where an update stands, as `catalyst-update` writes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: String,
    pub version: String,
    pub progress: f32,
    pub detail: String,
    pub size: u64,
    pub checked: u64,
    pub notes: String,
}

const PROGRAM: &str = "/usr/bin/catalyst-update";

fn state_dir() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        "/data/update".into()
    } else {
        crate::config_dir().join("update")
    }
}

/// Whether this system can update itself (the updater is installed).
pub fn available() -> bool {
    std::path::Path::new(PROGRAM).exists()
}

pub fn read() -> Status {
    let text = std::fs::read_to_string(state_dir().join("status.json")).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
    Status {
        state: s("state"),
        version: s("version"),
        progress: v["progress"].as_f64().unwrap_or(0.0) as f32,
        detail: s("detail"),
        size: v["size"].as_u64().unwrap_or(0),
        checked: v["checked"].as_u64().unwrap_or(0),
        notes: s("notes"),
    }
}

/// This system's version.
pub fn version() -> String {
    let text = std::fs::read_to_string("/etc/catalyst/version").unwrap_or_default();
    text.lines().find_map(|l| l.strip_prefix("version=")).map(|v| v.trim().to_owned()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

/// A choice in Settings › System update, with its default.
pub fn pref(key: &str, default: &str) -> String {
    std::fs::read_to_string(state_dir().join("prefs"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')).map(|v| v.trim().to_owned())))
        .unwrap_or_else(|| default.to_owned())
}

pub fn set_pref(key: &str, value: &str) {
    let dir = state_dir();
    std::fs::create_dir_all(&dir).ok();
    let text = std::fs::read_to_string(dir.join("prefs")).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{key}="))).map(str::to_owned).collect();
    lines.push(format!("{key}={value}"));
    crate::sys::write_atomic(&dir.join("prefs"), (lines.join("\n") + "\n").as_bytes()).ok();
}

/// Start a step (check, install, apply) in the background. It is started
/// through `sh -c '… &'`, so it belongs to no one once the shell lets go
/// of it: nothing is left behind as a zombie, and an install carries on if
/// the shell restarts.
pub fn run(step: &str) -> Result<(), String> {
    if !available() {
        return Err("updates are made on the handheld".into());
    }
    std::process::Command::new("sh")
        .args(["-c", &format!("{PROGRAM} {step} >/dev/null 2>&1 &")])
        .status()
        .map_err(|e| format!("could not start the update: {e}"))?;
    Ok(())
}

/// Megabytes, for a download's size.
pub fn megabytes(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 10_000_000 => format!("{:.1} mb", b as f64 / 1e6),
        b => format!("{} mb", b / 1_000_000),
    }
}

/// One line for where things stand: what Settings shows beside "System update".
pub fn summary(s: &Status) -> String {
    let pct = |p: f32| format!("{}%", (p * 100.0).round() as u32);
    match s.state.as_str() {
        "checking" => "checking…".into(),
        "available" => format!("{} available · {}", s.version, megabytes(s.size)).trim_end_matches(" · ").to_owned(),
        "downloading" => format!("downloading {}", pct(s.progress)),
        "installing" => format!("installing {}", pct(s.progress)),
        "ready" => format!("{} ready · restart", s.version),
        "trying" => format!("starting {}", s.version),
        "updated" => format!("updated to {}", s.version),
        "failed" => "did not install".into(),
        _ => format!("{} · up to date", version()),
    }
}

/// What the island says when an update reaches a new state, once.
pub fn notice(s: &Status) -> Option<String> {
    match s.state.as_str() {
        "available" => Some(format!("update {} available", s.version)),
        "ready" => Some(format!("update {} ready · restart in settings", s.version)),
        "updated" => Some(format!("updated to {}", s.version)),
        "failed" if !s.detail.is_empty() => Some(s.detail.clone()),
        _ => None,
    }
}

/// The shell has been up and drawing for a while: a system tried once after
/// an update is kept on the strength of this (see catalyst-update verify).
pub fn say_well() {
    if cfg!(target_os = "linux") && std::path::Path::new("/run").is_dir() {
        std::fs::write("/run/catalyst-shell-ok", b"").ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_an_update_stands_reads_in_a_line() {
        let s = |state: &str| Status { state: state.into(), version: "0.3.0".into(), progress: 0.456, size: 12_400_000, ..Status::default() };
        assert_eq!(summary(&s("available")), "0.3.0 available · 12 mb");
        assert_eq!(summary(&s("downloading")), "downloading 46%");
        assert_eq!(summary(&s("ready")), "0.3.0 ready · restart");
        assert!(summary(&s("")).ends_with("up to date"));
        assert_eq!(notice(&s("updated")).as_deref(), Some("updated to 0.3.0"));
        assert_eq!(notice(&s("downloading")), None);
        assert_eq!(megabytes(2_345_678), "2.3 mb");
    }
}
