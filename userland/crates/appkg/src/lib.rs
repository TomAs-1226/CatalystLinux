//! Catalyst's app packages.
//!
//! - **The app file** ([`Manifest`]): `key = value` lines in a file named
//!   `app` at the top of the app's folder. What it is called, its icon and
//!   version, what it runs, whether it takes the screen, and what it uses
//!   ([`Permission`]).
//! - **The package**: the folder as a gzipped tar ([`pack`]), the same
//!   bytes every time it is packed from the same files.
//! - **The signature**: Ed25519 over a short statement of the app's id,
//!   version and the package's SHA-256 ([`statement`]), made with the
//!   catalogue's key. The handheld installs only what a key it trusts has
//!   signed.
//! - **The index** ([`Listing`]): every app the catalogue offers, with its
//!   package's address, size, hash and signature, as `index.json`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The longest a name may be on a tile, and the line under it.
pub const NAME_MAX: usize = 24;
pub const ABOUT_MAX: usize = 40;
/// The longest description the catalogue's page for an app shows.
pub const DESCRIPTION_MAX: usize = 600;
/// The largest package the catalogue will fetch.
pub const PACKAGE_MAX: u64 = 32 * 1024 * 1024;

/// What an app may use, said before it is installed. Catalyst does not
/// yet fence an app in: these are what the catalogue was told and checked,
/// shown so nothing comes as a surprise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Network,
    Files,
    Camera,
    Microphone,
    Speaker,
    Motion,
    Infrared,
    Usb,
    System,
}

impl Permission {
    pub const ALL: [Permission; 9] =
        [Permission::Network, Permission::Files, Permission::Camera, Permission::Microphone, Permission::Speaker, Permission::Motion, Permission::Infrared, Permission::Usb, Permission::System];

    pub fn name(self) -> &'static str {
        match self {
            Permission::Network => "network",
            Permission::Files => "files",
            Permission::Camera => "camera",
            Permission::Microphone => "microphone",
            Permission::Speaker => "speaker",
            Permission::Motion => "motion",
            Permission::Infrared => "infrared",
            Permission::Usb => "usb",
            Permission::System => "system",
        }
    }

    /// What it means, in a few plain words.
    pub fn means(self) -> &'static str {
        match self {
            Permission::Network => "reaches the internet and the local network",
            Permission::Files => "reads and writes your files in /data",
            Permission::Camera => "takes pictures with the camera",
            Permission::Microphone => "listens through the microphone",
            Permission::Speaker => "plays sound",
            Permission::Motion => "reads the motion sensor and compass",
            Permission::Infrared => "sends and reads infrared",
            Permission::Usb => "uses what is plugged into USB",
            Permission::System => "changes how the handheld itself is set up",
        }
    }

    /// The symbol shown beside it.
    pub fn icon(self) -> &'static str {
        match self {
            Permission::Network => "language",
            Permission::Files => "folder",
            Permission::Camera => "photo_camera",
            Permission::Microphone => "forum",
            Permission::Speaker => "notes",
            Permission::Motion => "explore",
            Permission::Infrared => "settings_remote",
            Permission::Usb => "usb",
            Permission::System => "settings",
        }
    }

    pub fn from_name(name: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.name() == name.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Runs in the handheld's terminal.
    Terminal,
    /// Takes the panel and keyboard for itself.
    Own,
}

/// An app file, read.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Letters, digits and dashes: the app's folder and its name in the
    /// catalogue. Empty in a folder of your own that does not say.
    pub id: String,
    pub name: String,
    /// A symbol from the shell's set (`timer`), or `icon.svg`: a Material
    /// Symbol drawn in its 960-unit box, carried in the package.
    pub icon: String,
    pub about: String,
    /// The longer words on the catalogue's page for it.
    pub description: String,
    /// Dotted numbers, `1.2.0`: what the catalogue compares to offer updates.
    pub version: String,
    /// A shell command line, run from the app's folder.
    pub run: String,
    pub screen: Screen,
    pub permissions: Vec<Permission>,
    /// The program built from the source tree that the package carries
    /// (`catalyst-sudoku`), copied in when it is packed.
    pub binary: String,
    /// Where it came from and whose work it is ("after Sudoku by …").
    pub credit: String,
    /// Its licence, as an SPDX name (`MIT`).
    pub licence: String,
    /// Its source code.
    pub source: String,
    /// The oldest Catalyst Linux it runs on.
    pub needs: String,
    /// Words the app file holds that this version does not know: kept, not
    /// an error, so an older handheld reads a newer app file.
    pub unknown: Vec<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            id: String::new(),
            name: String::new(),
            icon: String::new(),
            about: String::new(),
            description: String::new(),
            version: String::new(),
            run: String::new(),
            screen: Screen::Terminal,
            permissions: Vec::new(),
            binary: String::new(),
            credit: String::new(),
            licence: String::new(),
            source: String::new(),
            needs: String::new(),
            unknown: Vec::new(),
        }
    }
}

/// Whether an id can name an app's folder: letters, digits and dashes, not
/// starting with a dash, at most 40.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Whether a version is dotted numbers (`1`, `1.2`, `1.2.10`).
pub fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 20 && v.split('.').all(|part| !part.is_empty() && part.len() <= 6 && part.chars().all(|c| c.is_ascii_digit()))
}

/// Whether `a` is a later version than `b` (`1.10` is later than `1.9`).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (mut a, mut b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

impl Manifest {
    /// Read an app file. Lenient, as the shell has always been: only `run`
    /// is needed for an app of your own. [`Manifest::check`] says whether it
    /// is fit for the catalogue.
    pub fn parse(text: &str) -> Manifest {
        let mut m = Manifest::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "id" => m.id = value.to_owned(),
                "name" => m.name = value.chars().take(NAME_MAX).collect(),
                "icon" => m.icon = value.to_owned(),
                "about" => m.about = value.chars().take(ABOUT_MAX).collect(),
                // Long words may run over several `description =` lines.
                "description" => {
                    if !m.description.is_empty() {
                        m.description.push(' ');
                    }
                    m.description.push_str(value);
                    m.description = m.description.chars().take(DESCRIPTION_MAX).collect();
                }
                "version" => m.version = value.to_owned(),
                "run" => m.run = value.to_owned(),
                "screen" => m.screen = if value == "own" { Screen::Own } else { Screen::Terminal },
                "permissions" => {
                    for word in value.split([',', ' ']).filter(|w| !w.trim().is_empty()) {
                        match Permission::from_name(word) {
                            Some(p) if !m.permissions.contains(&p) => m.permissions.push(p),
                            Some(_) => {}
                            None => m.unknown.push(format!("permission {word}")),
                        }
                    }
                    m.permissions.sort();
                }
                "binary" => m.binary = value.to_owned(),
                "credit" => m.credit = value.to_owned(),
                "licence" | "license" => m.licence = value.to_owned(),
                "source" => m.source = value.to_owned(),
                "needs" => m.needs = value.to_owned(),
                other => m.unknown.push(other.to_owned()),
            }
        }
        m
    }

    /// Whether the app may go in the catalogue: everything the catalogue
    /// shows is there and well formed. Each problem, in plain words.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if !valid_id(&self.id) {
            problems.push("id: letters, digits and dashes, at most 40".to_owned());
        }
        if self.name.trim().is_empty() {
            problems.push("name: missing".to_owned());
        }
        if !valid_version(&self.version) {
            problems.push("version: dotted numbers, as 1.0.0".to_owned());
        }
        if self.run.trim().is_empty() {
            problems.push("run: nothing to run".to_owned());
        }
        if self.icon.is_empty() {
            problems.push("icon: a symbol's name, or icon.svg".to_owned());
        }
        if self.about.trim().is_empty() {
            problems.push("about: the line under the name is missing".to_owned());
        }
        if self.licence.trim().is_empty() {
            problems.push("licence: missing (an SPDX name, as MIT)".to_owned());
        }
        if !self.needs.is_empty() && !valid_version(&self.needs) {
            problems.push("needs: a version, as 0.6.0".to_owned());
        }
        problems.extend(self.unknown.iter().filter(|u| u.starts_with("permission ")).map(|u| format!("{u}: not one Catalyst knows")));
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// The app file written out again, in a fixed order.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut line = |k: &str, v: &str| {
            if !v.is_empty() {
                out.push_str(&format!("{k} = {v}\n"));
            }
        };
        line("id", &self.id);
        line("name", &self.name);
        line("icon", &self.icon);
        line("about", &self.about);
        line("description", &self.description);
        line("version", &self.version);
        line("run", &self.run);
        line("screen", if self.screen == Screen::Own { "own" } else { "terminal" });
        let permissions: Vec<&str> = self.permissions.iter().map(|p| p.name()).collect();
        line("permissions", &permissions.join(", "));
        line("binary", &self.binary);
        line("credit", &self.credit);
        line("licence", &self.licence);
        line("source", &self.source);
        line("needs", &self.needs);
        out
    }
}

// ----------------------------------------------------------- signatures ---

/// SHA-256, as lower-case hex.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    hex(&sha2::Sha256::digest(data))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// What is signed for a package: which app, which version, which bytes.
pub fn statement(id: &str, version: &str, sha256: &str) -> String {
    format!("catalyst-app 1\nid {id}\nversion {version}\nsha256 {}\n", sha256.to_ascii_lowercase())
}

/// A key's short name: the first 16 hex digits of its public half.
pub fn key_id(public_hex: &str) -> String {
    public_hex.trim().chars().take(16).collect()
}

/// Sign `message` with a secret key's 32-byte seed (hex). The signature, as hex.
pub fn sign(message: &[u8], seed_hex: &str) -> Result<String, String> {
    Ok(hex(key_pair(seed_hex)?.sk.sign(message, None).as_ref()))
}

/// The public half (hex) of a secret key's seed.
pub fn public_of(seed_hex: &str) -> Result<String, String> {
    Ok(hex(key_pair(seed_hex)?.pk.as_ref()))
}

fn key_pair(seed_hex: &str) -> Result<ed25519_compact::KeyPair, String> {
    let seed = unhex(seed_hex).filter(|s| s.len() == 32).ok_or("the signing key should be 32 bytes of hex")?;
    let seed = ed25519_compact::Seed::from_slice(&seed).map_err(|_| "bad signing key")?;
    Ok(ed25519_compact::KeyPair::from_seed(seed))
}

/// Whether `signature_hex` is `message` signed by the key `public_hex`.
pub fn verify(message: &[u8], signature_hex: &str, public_hex: &str) -> bool {
    let (Some(key), Some(sig)) = (unhex(public_hex), unhex(signature_hex)) else { return false };
    let (Ok(key), Ok(sig)) = (ed25519_compact::PublicKey::from_slice(&key), ed25519_compact::Signature::from_slice(&sig)) else { return false };
    key.verify(message, &sig).is_ok()
}

/// The keys this handheld trusts to sign apps: the image's
/// (`/etc/catalyst/apps.pub`), and any you added (`/data/catalyst/app-keys/*.pub`).
/// One key of 64 hex digits a line; `#` starts a comment.
pub fn trusted_keys(files: &[PathBuf]) -> Vec<String> {
    let mut keys = Vec::new();
    for file in files {
        let paths: Vec<PathBuf> = if file.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(file).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "pub")).collect();
            found.sort();
            found
        } else {
            vec![file.clone()]
        };
        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for line in text.lines() {
                let key = line.split('#').next().unwrap_or("").trim().to_ascii_lowercase();
                if key.len() == 64 && unhex(&key).is_some() && !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    }
    keys
}

/// Which trusted key signed a package, if one did.
pub fn signed_by<'k>(id: &str, version: &str, sha256: &str, signature: &str, keys: &'k [String]) -> Option<&'k str> {
    let message = statement(id, version, sha256);
    keys.iter().find(|k| verify(message.as_bytes(), signature, k)).map(String::as_str)
}

// ---------------------------------------------------------------- index ---

/// One app as the catalogue's index lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub about: String,
    pub description: String,
    pub icon: String,
    /// An icon carried as path data (`icon = icon.svg`), to show before it is got.
    pub icon_path: String,
    pub version: String,
    /// The package's address, made whole against the index's.
    pub archive: String,
    pub sha256: String,
    pub bytes: u64,
    pub signature: String,
    pub permissions: Vec<Permission>,
    pub screen: Screen,
    pub credit: String,
    pub licence: String,
    pub source: String,
    pub needs: String,
}

impl Listing {
    /// The entry for a package made from `m`, at `archive`, of these bytes.
    pub fn of(m: &Manifest, icon_path: &str, archive: &str, package: &[u8], signature: String) -> Listing {
        Listing {
            id: m.id.clone(),
            name: m.name.clone(),
            about: m.about.clone(),
            description: m.description.clone(),
            icon: if m.icon == "icon.svg" { format!("app:{}", m.id) } else { m.icon.clone() },
            icon_path: icon_path.to_owned(),
            version: m.version.clone(),
            archive: archive.to_owned(),
            sha256: sha256_hex(package),
            bytes: package.len() as u64,
            signature,
            permissions: m.permissions.clone(),
            screen: m.screen,
            credit: m.credit.clone(),
            licence: m.licence.clone(),
            source: m.source.clone(),
            needs: m.needs.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id, "name": self.name, "about": self.about, "icon": self.icon, "version": self.version,
            "archive": self.archive, "sha256": self.sha256, "bytes": self.bytes, "signature": self.signature,
            "permissions": self.permissions.iter().map(|p| p.name()).collect::<Vec<_>>(),
            "screen": if self.screen == Screen::Own { "own" } else { "terminal" },
        });
        for (key, value) in [("description", &self.description), ("icon_path", &self.icon_path), ("credit", &self.credit), ("licence", &self.licence), ("source", &self.source), ("needs", &self.needs)] {
            if !value.is_empty() {
                v[key] = json!(value);
            }
        }
        v
    }
}

/// The apps an index lists. An entry that is not whole (no id fit for a
/// folder, no hash) is passed over; an archive named relative to the index
/// is made whole against the index's address.
pub fn parse_index(index: &Value, index_url: &str) -> Vec<Listing> {
    let base = index_url.rsplit_once('/').map_or(index_url, |(b, _)| b);
    let text = |a: &Value, key: &str| a[key].as_str().unwrap_or("").to_owned();
    index["apps"]
        .as_array()
        .map(|apps| {
            apps.iter()
                .filter_map(|a| {
                    let id = a["id"].as_str()?;
                    if !valid_id(id) {
                        return None;
                    }
                    let archive = a["archive"].as_str()?;
                    let archive = if archive.starts_with("https://") { archive.to_owned() } else { format!("{base}/{}", archive.trim_start_matches('/')) };
                    let sha256 = a["sha256"].as_str()?.to_ascii_lowercase();
                    (sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit())).then_some(())?;
                    let mut permissions: Vec<Permission> = a["permissions"].as_array().into_iter().flatten().filter_map(|p| Permission::from_name(p.as_str()?)).collect();
                    permissions.sort();
                    permissions.dedup();
                    Some(Listing {
                        id: id.into(),
                        name: a["name"].as_str().unwrap_or(id).chars().take(NAME_MAX).collect(),
                        about: text(a, "about").chars().take(ABOUT_MAX).collect(),
                        description: text(a, "description").chars().take(DESCRIPTION_MAX).collect(),
                        icon: a["icon"].as_str().unwrap_or("apps").into(),
                        icon_path: text(a, "icon_path"),
                        version: text(a, "version"),
                        archive,
                        sha256,
                        bytes: a["bytes"].as_u64().unwrap_or(0),
                        signature: text(a, "signature"),
                        permissions,
                        screen: if a["screen"] == "own" { Screen::Own } else { Screen::Terminal },
                        credit: text(a, "credit"),
                        licence: text(a, "licence"),
                        source: text(a, "source"),
                        needs: text(a, "needs"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// An index of these apps, as the catalogue publishes it.
pub fn index_json(name: &str, apps: &[Listing]) -> String {
    let v = json!({ "catalogue": 2, "name": name, "apps": apps.iter().map(Listing::to_json).collect::<Vec<_>>() });
    serde_json::to_string_pretty(&v).expect("an index is always JSON") + "\n"
}

// -------------------------------------------------------------- packing ---

/// The files of an app's folder, as (path inside the package, file),
/// sorted, leaving out what is never packed (hidden files, a `target`
/// folder, the README the source keeps for people).
pub fn files_of(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                out.push((rel, path));
            }
            // Links are never packed: the handheld refuses packages holding them.
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The moment every packed file is dated, so packing is repeatable.
const PACKED_AT: u64 = 1_767_225_600; // 2026-01-01 00:00 UTC

/// One file for a package: its path inside it, its bytes, and whether it runs.
pub struct Entry {
    pub path: String,
    pub data: Vec<u8>,
    pub executable: bool,
}

/// A gzipped tar (POSIX ustar) of `entries`, with their folders, owned by
/// root, dated alike: the same files make the same bytes.
pub fn pack(entries: &[Entry]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut tar = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    for e in entries {
        if e.path.starts_with('/') || e.path.split('/').any(|p| p == ".." || p.is_empty()) {
            return Err(format!("{}: not a path inside the package", e.path));
        }
        let mut parts: Vec<&str> = e.path.split('/').collect();
        parts.pop();
        for i in 1..=parts.len() {
            let folder = parts[..i].join("/") + "/";
            if !folders.contains(&folder) {
                header(&mut tar, &folder, 0, 0o755, b'5')?;
                folders.push(folder);
            }
        }
        header(&mut tar, &e.path, e.data.len() as u64, if e.executable { 0o755 } else { 0o644 }, b'0')?;
        tar.extend_from_slice(&e.data);
        tar.resize(tar.len().div_ceil(512) * 512, 0);
    }
    tar.resize(tar.len() + 1024, 0);
    let mut gz = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::best());
    gz.write_all(&tar).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())
}

fn header(tar: &mut Vec<u8>, path: &str, size: u64, mode: u32, kind: u8) -> Result<(), String> {
    let mut h = [0u8; 512];
    let name = path.as_bytes();
    // ustar: up to 100 bytes of name, or a prefix of up to 155 before a slash.
    let (prefix, name) = if name.len() <= 100 {
        (&b""[..], name)
    } else {
        let split = path[..path.len().min(155)].rfind('/').ok_or_else(|| format!("{path}: too long a path"))?;
        let (p, n) = (&name[..split], &name[split + 1..]);
        if n.len() > 100 {
            return Err(format!("{path}: too long a name"));
        }
        (p, n)
    };
    h[..name.len()].copy_from_slice(name);
    let octal = |field: &mut [u8], value: u64| {
        let s = format!("{:0width$o}\0", value, width = field.len() - 1);
        field.copy_from_slice(s.as_bytes());
    };
    octal(&mut h[100..108], mode as u64);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], PACKED_AT);
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[265..269].copy_from_slice(b"root");
    h[297..301].copy_from_slice(b"root");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    tar.extend_from_slice(&h);
    Ok(())
}

/// The files in a package, as (path, bytes, executable): for checking one
/// was packed as meant.
pub fn unpack(package: &[u8]) -> Result<Vec<(String, Vec<u8>, bool)>, String> {
    use std::io::Read;
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(package).take(PACKAGE_MAX * 4).read_to_end(&mut tar).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| String::from_utf8_lossy(&h[r]).trim_end_matches('\0').trim().to_owned();
        let size = u64::from_str_radix(&field(124..136), 8).map_err(|_| "a broken header")? as usize;
        let mode = u32::from_str_radix(&field(100..108), 8).unwrap_or(0);
        let prefix = field(345..500);
        let name = field(0..100);
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let start = at + 512;
        if h[156] == b'0' || h[156] == 0 {
            let data = tar.get(start..start + size).ok_or("a file runs past the end")?.to_vec();
            out.push((path, data, mode & 0o111 != 0));
        }
        at = start + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// Whether a file packed for an app runs: the program `run` starts (when it
/// is a file of the package), the built binary, scripts, and what is in `bin/`.
pub fn runs(m: &Manifest, path: &str) -> bool {
    let first = m.run.split_whitespace().next().unwrap_or("");
    let program = first.trim_start_matches("./");
    path == program || (!m.binary.is_empty() && path == m.binary) || path.ends_with(".sh") || path.starts_with("bin/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";

    fn sudoku() -> Manifest {
        Manifest::parse(
            "id = sudoku\nname = Sudoku\nicon = icon.svg\nabout = nine by nine, one answer\nversion = 1.0.0\nrun = ./catalyst-sudoku\nscreen = own\npermissions = speaker\nbinary = catalyst-sudoku\nlicence = MIT\nnew_thing = later\n",
        )
    }

    #[test]
    fn an_app_file_reads_checks_and_writes_back() {
        let m = sudoku();
        assert_eq!((m.id.as_str(), m.screen, m.permissions.as_slice()), ("sudoku", Screen::Own, &[Permission::Speaker][..]));
        assert_eq!(m.unknown, ["new_thing"], "a word from a later version is kept, not an error");
        assert!(m.check().is_ok(), "{:?}", m.check());
        assert_eq!(Manifest::parse(&m.render()).render(), m.render());
        // What the catalogue cannot list.
        let bare = Manifest::parse("run = top\n");
        let problems = bare.check().unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("id")) && problems.iter().any(|p| p.starts_with("version")));
        let odd = Manifest::parse("id = x\nname = X\nicon = apps\nabout = a\nversion = 1\nrun = x\nlicence = MIT\npermissions = network, telepathy\n");
        assert_eq!(odd.check().unwrap_err(), ["permission telepathy: not one Catalyst knows"]);
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("1.10.0", "1.9.3") && newer("2", "1.99") && newer("1.0.1", "1.0"));
        assert!(!newer("1.0", "1.0.0") && !newer("0.9", "1.0"));
        assert!(valid_version("1.2.10") && !valid_version("1.2-beta") && !valid_version("") && !valid_version("1..2"));
        assert!(valid_id("ping-robot") && !valid_id("../x") && !valid_id("-x") && !valid_id(""));
    }

    #[test]
    fn a_package_is_signed_and_checked_against_trusted_keys() {
        let public = public_of(SEED).unwrap();
        let sha = sha256_hex(b"package bytes");
        let signature = sign(statement("sudoku", "1.0.0", &sha).as_bytes(), SEED).unwrap();
        let keys = vec![public.clone()];
        assert_eq!(signed_by("sudoku", "1.0.0", &sha, &signature, &keys), Some(public.as_str()));
        // The same signature for another version, app or package: refused.
        assert_eq!(signed_by("sudoku", "1.0.1", &sha, &signature, &keys), None);
        assert_eq!(signed_by("orbits", "1.0.0", &sha, &signature, &keys), None);
        assert_eq!(signed_by("sudoku", "1.0.0", &sha256_hex(b"other"), &signature, &keys), None);
        assert_eq!(signed_by("sudoku", "1.0.0", &sha, &signature, &[]), None, "no trusted key, nothing installs");
        assert_eq!(signed_by("sudoku", "1.0.0", &sha, "zz", &keys), None);
        // Keys are read from files and folders of them.
        let dir = std::env::temp_dir().join(format!("appkg-keys-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("added")).unwrap();
        std::fs::write(dir.join("apps.pub"), format!("# the catalogue\n{public}\n")).unwrap();
        std::fs::write(dir.join("added/mine.pub"), format!("{}\nnot a key\n", "ab".repeat(32))).unwrap();
        std::fs::write(dir.join("added/notes.txt"), "cd".repeat(32)).unwrap();
        let found = trusted_keys(&[dir.join("apps.pub"), dir.join("added"), dir.join("missing")]);
        assert_eq!(found, [public, "ab".repeat(32)]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn packing_is_repeatable_and_reads_back() {
        let m = sudoku();
        let entries = |text: &str| {
            vec![
                Entry { path: "app".into(), data: m.render().into_bytes(), executable: false },
                Entry { path: "catalyst-sudoku".into(), data: text.as_bytes().to_vec(), executable: true },
                Entry { path: "share/licences/LICENSE".into(), data: b"MIT".to_vec(), executable: false },
            ]
        };
        let a = pack(&entries("binary")).unwrap();
        assert_eq!(a, pack(&entries("binary")).unwrap(), "the same files, the same bytes");
        assert_ne!(a, pack(&entries("other")).unwrap());
        let files = unpack(&a).unwrap();
        let names: Vec<&str> = files.iter().map(|(p, ..)| p.as_str()).collect();
        assert_eq!(names, ["app", "catalyst-sudoku", "share/licences/LICENSE"]);
        assert!(files[1].2 && !files[0].2, "the program runs, the app file does not");
        assert!(pack(&[Entry { path: "../escape".into(), data: vec![], executable: false }]).is_err());
        assert!(runs(&m, "catalyst-sudoku") && runs(&m, "bin/helper") && !runs(&m, "icon.svg"));
    }

    #[test]
    fn an_index_round_trips_and_passes_over_what_is_not_whole() {
        let m = sudoku();
        let package = b"pretend package";
        let listing = Listing::of(&m, "M0 0h10v10z", "sudoku-1.0.0.tar.gz", package, "ab".into());
        assert_eq!(listing.icon, "app:sudoku", "a carried icon is named after the app");
        let text = index_json("Catalyst apps", std::slice::from_ref(&listing));
        let v: Value = serde_json::from_str(&text).unwrap();
        let back = parse_index(&v, "https://github.com/TomAs-1226/catalyst-apps/releases/download/catalogue/index.json");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].archive, "https://github.com/TomAs-1226/catalyst-apps/releases/download/catalogue/sudoku-1.0.0.tar.gz");
        assert_eq!(Listing { archive: listing.archive.clone(), ..back[0].clone() }, listing);
        let bad = json!({"apps": [{"id": "../x", "archive": "a", "sha256": "0".repeat(64)}, {"id": "no-hash", "archive": "a"}]});
        assert!(parse_index(&bad, "https://x/index.json").is_empty());
    }
}
