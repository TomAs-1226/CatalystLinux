//! Claude: the Claude Code sessions on Thomas's laptop, worked from the
//! handheld when he is out, over a phone's hotspot.
//!
//! The handheld never talks to claude.ai and holds no Claude sign-in. It
//! talks only to the bridge Catalyst Companion runs on the laptop
//! (`catalyst-companion --claude-bridge`), which drives the `claude` CLI
//! there and sends back text. Tailscale Funnel publishes the bridge at the
//! laptop's fixed `*.ts.net` address, so all the handheld needs is HTTPS.
//!
//! - **Pairing, once** ([`Paired`]): the laptop's QR code, read with the
//!   Camera in scan mode, or its address and six-digit code typed. The key
//!   is kept in `/data/claude/pairing.json`, readable by root alone.
//! - **Sealed end to end**: every request and every event, with keys from
//!   the pairing (companion-proto's `claude`, Team's seal), so the tunnel
//!   reads nothing ([`link`]).
//! - **The pages** ([`ui`]): the sessions, with their project and when they
//!   were last worked in; a session's conversation, its tool calls one line
//!   each (Enter opens one), the composer (Alt dictates), and a permission
//!   request as a sheet that says what and where, to allow or deny.
//! - **Away from it**: a reply finishing or a request waiting is said in the
//!   island, and a live activity shows while Claude works.
//! - **Power**: the connection is held only while a Claude page is open or
//!   a reply or a request is still to come; nothing polls.

pub mod link;
pub mod ui;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use catalyst_companion_proto::claude::{self as proto, Key};
use serde_json::json;

pub use ui::ClaudeApp;

/// The app's symbol.
pub const ICON: &str = "code_blocks";

/// Where it keeps the pairing: `/data/claude` on the handheld.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CATALYST_CLAUDE") {
        return dir.into();
    }
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/claude")
    } else {
        crate::config_dir().join("claude")
    }
}

/// The laptop this handheld is paired with.
#[derive(Clone, Debug, PartialEq)]
pub struct Paired {
    /// `https://laptop.tail1234.ts.net[:port]`.
    pub address: String,
    pub key: Key,
    /// The laptop's name, as it gave it.
    pub computer: String,
    /// When, by the laptop's clock.
    pub since: i64,
}

pub fn load() -> Option<Paired> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir().join("pairing.json")).ok()?).ok()?;
    let key: [u8; 32] = catalyst_companion_proto::unhex(v["key"].as_str()?)?.try_into().ok()?;
    Some(Paired { address: proto::clean_address(v["address"].as_str()?)?, key: Key::from_bytes(key), computer: v["computer"].as_str().unwrap_or("laptop").to_owned(), since: v["since"].as_i64().unwrap_or(0) })
}

pub fn save(p: &Paired) -> Result<(), String> {
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|_| "the card would not take it".to_owned())?;
    let v = json!({"address": p.address, "key": catalyst_companion_proto::hex(p.key.bytes()), "computer": p.computer, "since": p.since});
    crate::sys::write_private(&d.join("pairing.json"), v.to_string().as_bytes()).map_err(|_| "the card would not take it".to_owned())
}

pub fn forget() {
    std::fs::remove_file(dir().join("pairing.json")).ok();
}

// --------------------------------------------------------------- camera ---

/// Whether a code the camera read is a laptop's pairing code.
pub fn is_pair_code(text: &str) -> bool {
    proto::read_qr(text).is_some()
}

/// The camera's card for it: its symbol, its line, what Enter does.
pub fn card(text: &str) -> (&'static str, String, &'static str) {
    let name = proto::read_qr(text).map(|i| i.computer).unwrap_or_default();
    (ICON, format!("Claude · {name}"), "enter: pair with this laptop")
}

/// Enter on it in the camera: the page to pair.
pub fn scanned(text: &str) -> Box<dyn crate::shell::Screen> {
    match proto::read_qr(text) {
        Some(invite) => Box::new(ui::PairPage::scanned(invite)),
        None => Box::new(ClaudeApp::new()),
    }
}

/// Home's tile: how it stands, without asking the laptop.
pub fn tile() -> (String, Option<&'static str>) {
    link::with(|s| match (&s.paired, s.approvals.len(), s.working.len()) {
        (None, ..) => ("laptop sessions".into(), None),
        (Some(_), a, _) if a > 0 => (format!("{a} to answer"), Some("WARN")),
        (Some(_), _, w) if w > 0 => ("working".into(), None),
        (Some(p), ..) => (p.computer.to_lowercase(), None),
    })
}
