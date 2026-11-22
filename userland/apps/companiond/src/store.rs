//! What the companion keeps: on /data, the computers paired (each by its
//! token's SHA-256, never the token), the handheld's id and its TLS
//! certificate; in /run, the pairing code the shell is showing and how the
//! pairing ended, for the shell to say so.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Where things are: `/data` and `/run/catalyst` on the handheld; a
/// scratch folder in the tests and the simulator.
#[derive(Clone, Debug)]
pub struct Paths {
    pub data: PathBuf,
    pub run: PathBuf,
}

impl Paths {
    pub fn own(&self) -> PathBuf {
        self.data.join("companion")
    }
    pub fn computers(&self) -> PathBuf {
        self.own().join("computers.json")
    }
    pub fn cert(&self) -> PathBuf {
        self.own().join("tls-cert.der")
    }
    pub fn key(&self) -> PathBuf {
        self.own().join("tls-key.der")
    }
    pub fn id(&self) -> PathBuf {
        self.own().join("id")
    }
    /// The code the shell shows: `<code>\n<expires, unix seconds>\n`.
    pub fn pairing(&self) -> PathBuf {
        self.run.join("companion-pair")
    }
    /// How the last pairing ended: `paired\t<name>` or `refused`.
    pub fn pair_result(&self) -> PathBuf {
        self.run.join("companion-paired")
    }
    /// The shell's socket for what only it can do (settings, sign-ins).
    pub fn shell_socket(&self) -> PathBuf {
        self.run.join("companion.sock")
    }
}

/// Written whole or not at all, readable by root alone.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".saving");
    let temp = path.with_file_name(name);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&temp, path)
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `n` random bytes from the kernel, as hex.
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the kernel's random numbers");
    catalyst_companion_proto::hex(&bytes)
}

/// A paired computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Computer {
    pub id: String,
    pub name: String,
    /// SHA-256 of its token, as hex.
    pub hash: String,
    /// When it was paired (unix seconds).
    pub paired: u64,
}

pub fn computers(paths: &Paths) -> Vec<Computer> {
    let text = std::fs::read_to_string(paths.computers()).unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Computer {
                id: c["id"].as_str()?.to_owned(),
                name: c["name"].as_str().unwrap_or("a computer").to_owned(),
                hash: c["hash"].as_str()?.to_owned(),
                paired: c["paired"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

pub fn save_computers(paths: &Paths, list: &[Computer]) -> std::io::Result<()> {
    let v: Vec<Value> = list.iter().map(|c| json!({"id": c.id, "name": c.name, "hash": c.hash, "paired": c.paired})).collect();
    write_private(&paths.computers(), serde_json::to_string_pretty(&v).unwrap_or_default().as_bytes())
}

/// The computer a token belongs to, if any.
pub fn computer_for(paths: &Paths, token: &str) -> Option<Computer> {
    let hash = catalyst_companion_proto::token_hash(token);
    computers(paths).into_iter().find(|c| catalyst_companion_proto::same(c.hash.as_bytes(), hash.as_bytes()))
}

/// The handheld's own id (made once).
pub fn id(paths: &Paths) -> String {
    if let Some(id) = std::fs::read_to_string(paths.id()).ok().map(|s| s.trim().to_owned()).filter(|s| s.len() == 16) {
        return id;
    }
    let id = random_hex(8);
    write_private(&paths.id(), id.as_bytes()).ok();
    id
}

/// The code the shell is showing, while it is good.
pub fn pairing_code(paths: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(paths.pairing()).ok()?;
    let mut lines = text.lines();
    let code = catalyst_companion_proto::clean_code(lines.next()?)?;
    let expires: u64 = lines.next()?.trim().parse().ok()?;
    (now() < expires).then_some(code)
}

/// The pairing is over (`paired`, the computer's name; or refused).
pub fn end_pairing(paths: &Paths, paired: Option<&str>) {
    std::fs::remove_file(paths.pairing()).ok();
    let result = match paired {
        Some(name) => format!("paired\t{name}\n"),
        None => "refused\n".to_owned(),
    };
    write_private(&paths.pair_result(), result.as_bytes()).ok();
}

/// The handheld's name, as its network knows it.
pub fn name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok().map(|n| n.to_lowercase()))
        .unwrap_or_else(|| "catalyst".into())
}

/// This system's version.
pub fn version() -> String {
    let text = std::fs::read_to_string("/etc/catalyst/version").unwrap_or_default();
    text.lines().find_map(|l| l.strip_prefix("version=")).map(|v| v.trim().to_owned()).unwrap_or_else(|| include_str!("../../../../os/VERSION").trim().to_owned())
}
