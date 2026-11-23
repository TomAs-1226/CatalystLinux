//! Gamepad: a driver's controller checked in the pit. A USB controller in
//! the USB-A port (Xbox, PlayStation, a Logitech F310 or any HID gamepad,
//! read through evdev) shows live: both sticks, the triggers, every button
//! and the D-pad, and every axis it has as a bar. A check measures each
//! stick's drift and shake at rest, whether it reaches its edge, and how
//! often reports come, suggests a deadzone, and keeps the result by the
//! controller's ID, so the team can see which one to bring.
//!
//! It draws only when what is shown changes (to the pixel), and no more
//! often than the frame beat; with the controller still, nothing is drawn.

// Read through the Linux backend; in the simulator only parts are used.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod evdev;
pub mod health;
mod pads;
mod screen;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

pub use screen::Gamepad;

use health::{Rate, Rest, Verdict};
use std::path::PathBuf;

/// Home's line under the tile.
pub fn tile() -> (String, Option<&'static str>) {
    ("sticks · drift".into(), None)
}

// --------------------------------------------------------------------- usb ---

/// Whether the USB-A port can take a controller: it needs the CM0's USB
/// running as the host of the internal hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usb {
    Host,
    /// A mode for a computer is chosen in Settings › USB.
    ComputerMode,
    /// Charging only is chosen, but the port is a device until a restart.
    Restart,
    /// The host runs but no hub answers: the switch is on USB-C2's side.
    NoHub,
}

static DEMO_USB: std::sync::Mutex<Option<Usb>> = std::sync::Mutex::new(None);

/// The gallery's choice of how USB stands.
#[cfg(not(target_arch = "aarch64"))]
pub fn demo_usb(usb: Option<Usb>) {
    *DEMO_USB.lock().unwrap_or_else(|e| e.into_inner()) = usb;
}

pub fn usb() -> Usb {
    if let Some(u) = *DEMO_USB.lock().unwrap_or_else(|e| e.into_inner()) {
        return u;
    }
    // Only the handheld has the switch and the modes.
    if !std::path::Path::new("/usr/sbin/catalyst-usb").exists() {
        return Usb::Host;
    }
    if crate::usb::mode() != 0 {
        Usb::ComputerMode
    } else if crate::usb::running_as_device() {
        Usb::Restart
    } else if !crate::sound::hub_present() {
        Usb::NoHub
    } else {
        Usb::Host
    }
}

impl Usb {
    /// What to do about it, in a sentence.
    pub fn advice(self) -> &'static str {
        match self {
            Usb::Host => "Plug a controller into the USB-A port.",
            Usb::ComputerMode => "USB is set for a computer. For a controller: Settings › USB on charging only, the USB switch on the hub's side, then restart.",
            Usb::Restart => "USB is on charging only now. Restart with the USB switch on the hub's side, and plug the controller into USB-A.",
            Usb::NoHub => "The USB switch is on USB-C2's side. Slide it to the hub's side for the USB-A port.",
        }
    }
}

// ------------------------------------------------------------------- check ---

/// A controller's check, as kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Vendor:product[:serial], as [`evdev::Pad::id`] gives it.
    pub id: String,
    /// What the team calls it ("driver", "spare 2").
    pub label: String,
    /// What it calls itself.
    pub model: String,
    /// Seconds since 1970.
    pub when: i64,
    pub rest: [Rest; 2],
    pub deadzone: [f32; 2],
    pub reach: [[f32; 4]; 2],
    pub rate: Option<Rate>,
    pub verdict: Verdict,
}

impl Record {
    pub fn new(pad: &evdev::Pad, rest: [Rest; 2], reach: [[f32; 4]; 2], rate: Option<Rate>) -> Record {
        let deadzone = [health::deadzone(&rest[0]), health::deadzone(&rest[1])];
        let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        Record { id: pad.id(), label: String::new(), model: pad.name.clone(), when, rest, deadzone, reach, rate, verdict: health::verdict(deadzone, reach, rate) }
    }

    /// What its row is called: its label, else its model.
    pub fn title(&self) -> &str {
        if self.label.is_empty() {
            &self.model
        } else {
            &self.label
        }
    }

    pub fn to_json(&self) -> String {
        let stick = |i: usize| {
            serde_json::json!({
                "drift": [self.rest[i].drift.0, self.rest[i].drift.1],
                "furthest": self.rest[i].furthest,
                "noise": self.rest[i].noise,
                "deadzone": self.deadzone[i],
                "reach": self.reach[i],
            })
        };
        let doc = serde_json::json!({
            "id": self.id,
            "label": self.label,
            "model": self.model,
            "when": self.when,
            "left": stick(0),
            "right": stick(1),
            "rate": self.rate.map(|r| serde_json::json!({"hz": r.hz, "median_ms": r.median_ms, "worst_ms": r.worst_ms})),
            "verdict": self.verdict.word(),
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<Record> {
        let d: serde_json::Value = serde_json::from_str(text).ok()?;
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
        let stick = |k: &str| -> (Rest, f32, [f32; 4]) {
            let s = &d[k];
            let reach = [f(&s["reach"][0]), f(&s["reach"][1]), f(&s["reach"][2]), f(&s["reach"][3])];
            (Rest { drift: (f(&s["drift"][0]), f(&s["drift"][1])), furthest: f(&s["furthest"]), noise: f(&s["noise"]) }, f(&s["deadzone"]), reach)
        };
        let (l, r) = (stick("left"), stick("right"));
        let rate = d["rate"].is_object().then(|| Rate { hz: f(&d["rate"]["hz"]), median_ms: f(&d["rate"]["median_ms"]), worst_ms: f(&d["rate"]["worst_ms"]) });
        Some(Record {
            id: d["id"].as_str()?.to_owned(),
            label: d["label"].as_str().unwrap_or_default().to_owned(),
            model: d["model"].as_str().unwrap_or_default().to_owned(),
            when: d["when"].as_i64().unwrap_or(0),
            rest: [l.0, r.0],
            deadzone: [l.1, r.1],
            reach: [l.2, r.2],
            rate,
            verdict: Verdict::from_word(d["verdict"].as_str().unwrap_or("")),
        })
    }

    /// Its file: the controller's ID, and its label so two of one model
    /// (with no serial to tell them apart) are kept apart.
    pub fn file_name(&self) -> String {
        let id: String = self.id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        if self.label.is_empty() {
            format!("{id}.json")
        } else {
            format!("{id}-{}.json", crate::mech::slug(&self.label))
        }
    }
}

/// Where checks are kept: `/data/gamepad` on the handheld.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/gamepad")
    } else {
        crate::config_dir().join("gamepad")
    }
}

pub fn keep(r: &Record) -> std::io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(r.file_name());
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, r.to_json())?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every check kept, the one to bring first: good before fair before
/// worn, then the smallest deadzone, then the newest.
pub fn kept() -> Vec<(PathBuf, Record)> {
    let mut all: Vec<(PathBuf, Record)> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read_to_string(&p).ok().and_then(|t| Record::from_json(&t)).map(|r| (p, r)))
        .collect();
    all.sort_by(|(_, a), (_, b)| a.verdict.cmp(&b.verdict).then(a.deadzone[0].max(a.deadzone[1]).total_cmp(&b.deadzone[0].max(b.deadzone[1]))).then(b.when.cmp(&a.when)));
    all
}
