//! The app catalogue: apps to get, update and remove over Wi-Fi, listed in
//! an index file in a GitHub repository (by default the one this project's
//! `catalyst-apps/` folder becomes). Each app is a gzipped tar of an app
//! folder, as `/data/apps` holds them, with its SHA-256 in the index.
//!
//! Getting one: download it (HTTPS only), check its hash, check every path
//! in it stays inside it, unpack it beside the others, check it has an `app`
//! file that names something to run, and only then put it in place. A
//! failure at any step leaves what was there as it was. Apps installed this
//! way carry a `.catalogue` file; only those are removed from here.

use std::path::{Path, PathBuf};
use std::process::Command;

use bezel::Rect;
use serde_json::Value;

use crate::git::Job;
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Where the catalogue's index is, unless `CATALYST_CATALOGUE` says otherwise.
pub const INDEX: &str = "https://raw.githubusercontent.com/TomAs-1226/catalyst-apps/main/index.json";
/// The largest app the catalogue will download.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub about: String,
    pub icon: String,
    pub version: String,
    /// The archive's address, made whole against the index's.
    pub archive: String,
    pub sha256: String,
    pub bytes: u64,
}

fn index_url() -> String {
    std::env::var("CATALYST_CATALOGUE").unwrap_or_else(|_| INDEX.to_owned())
}

/// The apps an index lists. An id names a folder, so it is kept to letters,
/// digits and dashes; an entry that is not complete is passed over.
pub fn parse_index(index: &Value, index_url: &str) -> Vec<Listing> {
    let base = index_url.rsplit_once('/').map_or(index_url, |(b, _)| b);
    index["apps"]
        .as_array()
        .map(|apps| {
            apps.iter()
                .filter_map(|a| {
                    let id = a["id"].as_str()?;
                    if id.is_empty() || id.len() > 40 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                        return None;
                    }
                    let archive = a["archive"].as_str()?;
                    let archive = if archive.starts_with("https://") { archive.to_owned() } else { format!("{base}/{}", archive.trim_start_matches('/')) };
                    let sha256 = a["sha256"].as_str()?.to_ascii_lowercase();
                    (sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit())).then_some(())?;
                    Some(Listing {
                        id: id.into(),
                        name: a["name"].as_str().unwrap_or(id).into(),
                        about: a["about"].as_str().unwrap_or("").into(),
                        icon: a["icon"].as_str().unwrap_or("apps").into(),
                        version: a["version"].as_str().unwrap_or("").into(),
                        archive,
                        sha256,
                        bytes: a["bytes"].as_u64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The folder apps of the user's own are kept in.
pub fn apps_dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        "/data/apps".into()
    } else {
        crate::apps::folders().pop().unwrap_or_else(|| crate::config_dir().join("apps"))
    }
}

/// The version of an app the catalogue installed, if it did.
pub fn installed(dir: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(id).join(".catalogue")).ok()?;
    Some(text.lines().find_map(|l| l.strip_prefix("version="))?.trim().to_owned())
}

/// SHA-256 of a byte string, as lower-case hex.
pub fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Whether every path in an archive's listing stays inside the folder it
/// is unpacked into: nothing absolute, nothing climbing out with `..`.
pub fn paths_stay_inside(listing: &str) -> bool {
    listing.lines().map(str::trim).filter(|l| !l.is_empty()).all(|p| !p.starts_with('/') && !p.starts_with('\\') && !p.contains(':') && p.split(['/', '\\']).all(|part| part != ".."))
}

/// Whether an archive's long listing (`tar -tv`) holds only plain files and
/// folders: a link could point outside the folder, and a later file written
/// through it would land there.
pub fn only_files_and_folders(long_listing: &str) -> bool {
    long_listing.lines().filter(|l| !l.trim().is_empty()).all(|l| matches!(l.as_bytes()[0], b'-' | b'd') && !l.contains(" link to ") && !l.contains(" -> "))
}

/// The apps being installed just now, so the same one is not installed
/// twice at once (both would use the same files beside it).
static INSTALLING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|_| format!("no {program} on this system"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("failed").trim().to_owned())
    }
}

fn fetch_index() -> Result<Vec<Listing>, String> {
    let url = index_url();
    let text = run("curl", &["-fsS", "--max-time", "20", "--proto", "=https", &url]).map_err(|_| "could not reach the catalogue".to_string())?;
    let index: Value = serde_json::from_str(&text).map_err(|_| "the catalogue's index is not readable".to_string())?;
    Ok(parse_index(&index, &url))
}

/// Get (or update) an app: download, check, unpack beside, check, put in place.
pub fn install(app: &Listing, dir: &Path) -> Result<(), String> {
    {
        let mut busy = INSTALLING.lock().unwrap();
        if busy.contains(&app.id) {
            return Err(format!("{} is already being installed", app.name));
        }
        busy.push(app.id.clone());
    }
    let result = install_now(app, dir);
    INSTALLING.lock().unwrap().retain(|id| *id != app.id);
    result
}

fn install_now(app: &Listing, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let archive = dir.join(format!(".{}.tar.gz", app.id));
    let unpacked = dir.join(format!(".{}.new", app.id));
    let result = (|| {
        let max = MAX_BYTES.to_string();
        run("curl", &["-fsS", "--max-time", "120", "--proto", "=https", "--max-filesize", &max, "-o", &archive.to_string_lossy(), &app.archive]).map_err(|_| "could not download it".to_string())?;
        let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
        if sha256(&bytes) != app.sha256 {
            return Err("it did not match the catalogue: not installed".into());
        }
        let listing = run("tar", &["-tzf", &archive.to_string_lossy()])?;
        let long_listing = run("tar", &["-tvzf", &archive.to_string_lossy()])?;
        if !paths_stay_inside(&listing) || !only_files_and_folders(&long_listing) {
            return Err("it would write outside its folder: not installed".into());
        }
        std::fs::remove_dir_all(&unpacked).ok();
        std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
        run("tar", &["-xzf", &archive.to_string_lossy(), "-C", &unpacked.to_string_lossy()])?;
        let manifest = std::fs::read_to_string(unpacked.join("app")).map_err(|_| "it has no app file: not installed".to_string())?;
        crate::apps::parse(&manifest, &unpacked).ok_or("its app file names nothing to run: not installed")?;
        std::fs::write(unpacked.join(".catalogue"), format!("id={}\nversion={}\n", app.id, app.version)).map_err(|e| e.to_string())?;
        // Into place: the old one aside, the new one in, the old one gone.
        let target = dir.join(&app.id);
        let old = dir.join(format!(".{}.old", app.id));
        std::fs::remove_dir_all(&old).ok();
        if target.exists() {
            std::fs::rename(&target, &old).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&unpacked, &target).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&old).ok();
        Ok(())
    })();
    std::fs::remove_file(&archive).ok();
    std::fs::remove_dir_all(&unpacked).ok();
    result
}

/// Remove an app the catalogue installed; one put there by hand is left alone.
pub fn remove(id: &str, dir: &Path) -> Result<(), String> {
    if installed(dir, id).is_none() {
        return Err("not one the catalogue installed".into());
    }
    std::fs::remove_dir_all(dir.join(id)).map_err(|e| e.to_string())
}

// -------------------------------------------------------------- screen ---

pub struct CatalogueScreen {
    list: ListState,
    index: Job<Result<Vec<Listing>, String>>,
    working: Option<(String, Job<Result<(), String>>)>,
    /// The app `d` was pressed on once; a second press removes it.
    removing: Option<String>,
}

impl CatalogueScreen {
    pub fn new() -> Self {
        CatalogueScreen { list: ListState::default(), index: Job::start(fetch_index), working: None, removing: None }
    }
}

impl Default for CatalogueScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for CatalogueScreen {
    fn title(&self) -> &str {
        "Catalogue"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let apps = self.index.get().and_then(Result::ok).unwrap_or_default();
        let armed = self.removing.take();
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('r') => self.index = Job::start(fetch_index),
            Key::Enter if self.working.is_none() => {
                if let Some(app) = apps.get(self.list.focus).cloned() {
                    let dir = apps_dir();
                    let verb = if installed(&dir, &app.id).is_some() { "updating" } else { "getting" };
                    *cx.say = Some(format!("{verb} {}…", app.name));
                    self.working = Some((app.name.clone(), Job::start(move || install(&app, &dir))));
                }
            }
            Key::Char('d') => {
                if let Some(app) = apps.get(self.list.focus) {
                    if armed.as_deref() == Some(app.id.as_str()) {
                        *cx.say = Some(match remove(&app.id, &apps_dir()) {
                            Ok(()) => format!("removed {}", app.name),
                            Err(why) => why,
                        });
                    } else if installed(&apps_dir(), &app.id).is_some() {
                        *cx.say = Some(format!("d again removes {}", app.name));
                        self.removing = Some(app.id.clone());
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, apps.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some((name, job)) = &self.working {
            if let Some(done) = job.get() {
                *cx.say = Some(match done {
                    Ok(()) => format!("{name} is on Home"),
                    Err(why) => why,
                });
                self.working = None;
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.index.get().is_none() || self.working.is_some()
    }

    fn busy(&self) -> bool {
        self.index.get().is_none() || self.working.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let apps = match self.index.get() {
            None => {
                g.empty("storefront", "looking at the catalogue…");
                return;
            }
            Some(Err(why)) => {
                g.empty("storefront", &why);
                g.hint("r: try again · needs Wi-Fi");
                return;
            }
            Some(Ok(apps)) => apps,
        };
        if apps.is_empty() {
            g.empty("storefront", "the catalogue is empty");
            return;
        }
        let dir = apps_dir();
        let bottom = H - PAD - g.hint_h();
        let row_h = g.row_h() + 6.0;
        let ty = g.ty;
        for (i, y) in self.list.layout(g, apps.len(), top, row_h, 3.0, bottom + 2.0) {
            let app = &apps[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, 12.0, if focused { t.surface2 } else { t.surface1 });
            g.icon(&app.icon, focused, r.x + 15.0, r.cy(), 16.0, if focused { t.ink } else { t.dim });
            let state = match (installed(&dir, &app.id), &self.working) {
                (_, Some((name, _))) if *name == app.name => "working…".to_owned(),
                (Some(v), _) if v == app.version => "installed".to_owned(),
                (Some(_), _) => format!("update {}", app.version),
                (None, _) => "get".to_owned(),
            };
            let sw = g.text_at(r.right() - 10.0, r.cy(), bezel::Align::Right, &state, ty.label, if state == "get" || state.starts_with("update") { t.signal } else { t.dim });
            g.text_fit(r.x + 30.0, r.y + 6.0 + ty.name.size * 0.5, &app.name, ty.name, t.ink, r.w - 44.0 - sw);
            g.text_fit(r.x + 30.0, r.bottom() - 5.0 - ty.caption.size * 0.5, &app.about, ty.caption, t.dim, r.w - 44.0 - sw);
            if focused {
                g.focus_ring(r, 12.0);
            }
        }
        self.list.scrollbar(g, apps.len(), top, bottom);
        g.hint("enter: get or update · d d: remove · r: refresh");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(sha256(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256(&[b'a'; 1000]), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    }

    #[test]
    fn an_index_lists_whole_entries_with_safe_names() {
        let index = serde_json::json!({"catalogue": 1, "apps": [
            {"id": "ping-robot", "name": "Ping the robot", "about": "is it there?", "icon": "link", "version": "1.0.0",
             "archive": "apps/ping-robot-1.0.0.tar.gz", "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "bytes": 400},
            {"id": "../escape", "archive": "x.tar.gz", "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},
            {"id": "no-hash", "archive": "x.tar.gz"}
        ]});
        let apps = parse_index(&index, "https://raw.githubusercontent.com/TomAs-1226/catalyst-apps/main/index.json");
        assert_eq!(apps.len(), 1, "an unsafe id and an entry without a hash are passed over");
        assert_eq!(apps[0].archive, "https://raw.githubusercontent.com/TomAs-1226/catalyst-apps/main/apps/ping-robot-1.0.0.tar.gz");
    }

    #[test]
    fn an_archive_that_would_write_outside_its_folder_is_refused() {
        assert!(paths_stay_inside("app\nping.sh\nicons/a.svg\n"));
        assert!(!paths_stay_inside("app\n../../etc/profile\n"));
        assert!(!paths_stay_inside("/etc/passwd\n"));
        assert!(!paths_stay_inside("app\nsub/../../x\n"));
        // Links, which a later file could be written through.
        assert!(only_files_and_folders("drwxr-xr-x 0/0 0 2026-01-01 00:00 ./\n-rw-r--r-- 0/0 98 2026-01-01 00:00 ./app\n"));
        assert!(!only_files_and_folders("lrwxrwxrwx 0/0 0 2026-01-01 00:00 ./etc -> /etc\n"));
        assert!(!only_files_and_folders("hrw-r--r-- 0/0 0 2026-01-01 00:00 ./x link to ./app\n"));
    }

    /// The whole of getting an app, with a stand-in `curl` that hands over a
    /// local archive: a good one goes in, a tampered one and one that climbs
    /// out of its folder are refused, and the installed one stays.
    #[cfg(target_os = "linux")]
    #[test]
    fn getting_an_app_checks_it_before_it_goes_in() {
        let base = std::env::temp_dir().join(format!("catalyst-install-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let (src, bin, apps) = (base.join("src"), base.join("bin"), base.join("apps"));
        for d in [&src, &bin, &apps] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(src.join("app"), "name = Hello\nversion = 1.0.0\nrun = echo hi\n").unwrap();
        let pack = |out: &Path| {
            let ok = Command::new("tar").arg("-czf").arg(out).arg("-C").arg(&src).arg(".").status().unwrap().success();
            assert!(ok);
            std::fs::read(out).unwrap()
        };
        let good = base.join("good.tar.gz");
        let hash = sha256(&pack(&good));
        // curl, as the catalogue calls it: copy $CATALYST_FAKE_ARCHIVE to the -o file.
        let curl = bin.join("curl");
        std::fs::write(&curl, "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=$2; shift; done\ncp \"$CATALYST_FAKE_ARCHIVE\" \"$out\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        let listing = |sha: &str, version: &str| Listing {
            id: "hello".into(),
            name: "Hello".into(),
            about: String::new(),
            icon: "apps".into(),
            version: version.into(),
            archive: "https://example.invalid/hello.tar.gz".into(),
            sha256: sha.into(),
            bytes: 0,
        };
        std::env::set_var("CATALYST_FAKE_ARCHIVE", &good);
        install(&listing(&hash, "1.0.0"), &apps).unwrap();
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.0.0"));
        assert!(apps.join("hello/app").exists());
        // A hash that does not match: refused, and 1.0.0 is still there.
        assert!(install(&listing(&"0".repeat(64), "1.1.0"), &apps).unwrap_err().contains("did not match"));
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.0.0"));
        // An archive holding a link out of its folder: refused before unpacking.
        let linked = base.join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("app"), "name = Linked\nrun = true\n").unwrap();
        std::os::unix::fs::symlink("/etc", linked.join("etc")).unwrap();
        let link_archive = base.join("link.tar.gz");
        Command::new("tar").arg("-czf").arg(&link_archive).arg("-C").arg(&linked).arg(".").status().unwrap();
        std::env::set_var("CATALYST_FAKE_ARCHIVE", &link_archive);
        let link_hash = sha256(&std::fs::read(&link_archive).unwrap());
        assert!(install(&listing(&link_hash, "6.6.5"), &apps).unwrap_err().contains("outside"));
        // An archive with a path that climbs out of its folder: refused before unpacking.
        std::fs::create_dir_all(base.join("deep/inner")).unwrap();
        std::fs::write(base.join("deep/evil"), "x").unwrap();
        let evil = base.join("evil.tar.gz");
        Command::new("tar").current_dir(base.join("deep/inner")).args(["-P", "-czf"]).arg(&evil).arg("../evil").status().unwrap();
        std::env::set_var("CATALYST_FAKE_ARCHIVE", &evil);
        let evil_hash = sha256(&std::fs::read(&evil).unwrap());
        assert!(install(&listing(&evil_hash, "6.6.6"), &apps).unwrap_err().contains("outside"));
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.0.0"));
        // An update: the new version takes the old one's place.
        std::fs::write(src.join("app"), "name = Hello\nversion = 1.1.0\nrun = echo hi again\n").unwrap();
        let newer = base.join("newer.tar.gz");
        let newer_hash = sha256(&pack(&newer));
        std::env::set_var("CATALYST_FAKE_ARCHIVE", &newer);
        install(&listing(&newer_hash, "1.1.0"), &apps).unwrap();
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.1.0"));
        assert!(std::fs::read_to_string(apps.join("hello/app")).unwrap().contains("again"));
        let leftovers: Vec<_> = std::fs::read_dir(&apps).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with('.')).collect();
        assert!(leftovers.is_empty(), "nothing left beside it: {leftovers:?}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn only_what_the_catalogue_installed_is_removed() {
        let dir = std::env::temp_dir().join(format!("catalyst-catalogue-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::create_dir_all(dir.join("fetched")).unwrap();
        std::fs::write(dir.join("fetched/.catalogue"), "id=fetched\nversion=1.2.0\n").unwrap();
        assert_eq!(installed(&dir, "fetched").as_deref(), Some("1.2.0"));
        assert!(remove("mine", &dir).is_err(), "an app put there by hand stays");
        assert!(remove("fetched", &dir).is_ok());
        assert!(dir.join("mine").exists() && !dir.join("fetched").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
