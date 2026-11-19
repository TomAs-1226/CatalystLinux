//! Where an update stands, kept in `/data/update/status.json` for the shell
//! to show, and the choices made in Settings › System update.

use serde_json::{json, Value};

use crate::device::Device;

/// The states an update moves through, as the shell shows them.
pub mod state {
    /// Nothing newer than what is running.
    pub const CURRENT: &str = "current";
    pub const CHECKING: &str = "checking";
    /// A newer version, not downloaded yet.
    pub const AVAILABLE: &str = "available";
    pub const DOWNLOADING: &str = "downloading";
    pub const INSTALLING: &str = "installing";
    /// Written into the spare system: a restart finishes it.
    pub const READY: &str = "ready";
    /// Started in the new system, which has not yet shown that it works.
    pub const TRYING: &str = "trying";
    /// The new system came up and is now the one that starts.
    pub const UPDATED: &str = "updated";
    pub const FAILED: &str = "failed";
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub state: String,
    /// The version it is about (the one available, ready, or just installed).
    pub version: String,
    pub progress: f32,
    pub detail: String,
    /// How much there is to download, in bytes.
    pub size: u64,
    /// When updates were last looked for: seconds since 1970.
    pub checked: u64,
    pub notes: String,
}

impl Status {
    pub fn to_json(&self) -> String {
        json!({
            "state": self.state, "version": self.version, "progress": self.progress, "detail": self.detail,
            "size": self.size, "checked": self.checked, "notes": self.notes,
        })
        .to_string()
    }

    pub fn parse(text: &str) -> Status {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
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
}

pub fn read(device: &Device) -> Status {
    Status::parse(&std::fs::read_to_string(device.state.join("status.json")).unwrap_or_default())
}

/// Written whole and renamed into place, so the shell never reads half of it.
pub fn write(device: &Device, status: &Status) {
    std::fs::create_dir_all(&device.state).ok();
    let tmp = device.state.join("status.json.new");
    if std::fs::write(&tmp, status.to_json()).is_ok() {
        std::fs::rename(&tmp, device.state.join("status.json")).ok();
    }
}

/// Change some of the status and write it.
pub fn update(device: &Device, f: impl FnOnce(&mut Status)) -> Status {
    let mut s = read(device);
    f(&mut s);
    write(device, &s);
    s
}

/// A choice from Settings › System update.
pub fn pref(device: &Device, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(device.state.join("prefs")).ok()?;
    text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')).map(|v| v.trim().to_owned()))
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
