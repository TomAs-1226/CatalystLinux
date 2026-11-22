//! The handhelds this computer is paired with: where each was last found,
//! its pinned certificate, and its token, kept encrypted to this Windows
//! account (DPAPI) in `%APPDATA%\Catalyst Companion\handhelds.json`.

use catalyst_companion_proto as proto;
use serde_json::{json, Value};

use crate::net::Target;
use crate::win;

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub id: String,
    pub name: String,
    pub version: String,
    pub target: Target,
}

fn file() -> std::path::PathBuf {
    win::app_dir().join("handhelds.json")
}

pub fn load() -> Vec<Saved> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file()).unwrap_or_default()).unwrap_or(Value::Null);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let pin: [u8; 32] = proto::unhex(h["pin"].as_str()?)?.try_into().ok()?;
            let token = String::from_utf8(win::unprotect(&proto::unhex(h["token"].as_str()?)?)?).ok()?;
            Some(Saved {
                id: h["id"].as_str()?.to_owned(),
                name: h["name"].as_str().unwrap_or("catalyst").to_owned(),
                version: h["version"].as_str().unwrap_or_default().to_owned(),
                target: Target { host: h["host"].as_str()?.to_owned(), port: h["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(proto::PORT), pin, token },
            })
        })
        .collect()
}

pub fn save(all: &[Saved]) -> Result<(), String> {
    let v: Vec<Value> = all
        .iter()
        .filter_map(|s| {
            let token = win::protect(s.target.token.as_bytes())?;
            Some(json!({"id": s.id, "name": s.name, "version": s.version, "host": s.target.host, "port": s.target.port, "pin": proto::hex(&s.target.pin), "token": proto::hex(&token)}))
        })
        .collect();
    std::fs::create_dir_all(win::app_dir()).map_err(|e| e.to_string())?;
    let path = file();
    let temp = path.with_extension("saving");
    std::fs::write(&temp, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| format!("could not keep the pairing: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("could not keep the pairing: {e}"))
}

/// Kept, replacing any earlier pairing with the same handheld.
pub fn remember(s: Saved) -> Result<(), String> {
    let mut all: Vec<Saved> = load().into_iter().filter(|x| x.id != s.id).collect();
    all.insert(0, s);
    save(&all)
}

pub fn forget(id: &str) -> Result<(), String> {
    let all: Vec<Saved> = load().into_iter().filter(|x| x.id != id).collect();
    save(&all)
}
