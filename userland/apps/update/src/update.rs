//! An update on the handheld, step by step, as Android's A/B updates go:
//!
//! 1. **check**: find the newest release on the chosen channel, check its
//!    manifest's signature, and see whether it is newer than this system;
//! 2. **install**: in the background, download a patch from this version
//!    (or the whole system), write it into the spare system's partition,
//!    read it back to check it, and put its kernel in the spare system's
//!    folder on the boot partition, with config.txt set to try it;
//! 3. **apply**: restart with `reboot "0 tryboot"`, so the firmware starts
//!    the new system once;
//! 4. **verify**, at every start: in a tried start, wait for the shell to
//!    show it is up, then make the new system the one that starts. If it
//!    never does (a panic, a hang, a shell that will not stay up), the next
//!    start is the old system again, which then reports that the update did
//!    not take.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::device::{self, Device};
use crate::manifest::{self, Manifest};
use crate::net;
use crate::pack;
use crate::status::{self, now, state, Status};

/// Below this charge, an install waits for the charger.
const LOW_BATTERY: u32 = 25;

fn saved(dev: &Device, name: &str) -> PathBuf {
    dev.state.join(name)
}

/// The manifest saved by the last check, its signature checked again.
fn saved_manifest(dev: &Device) -> Result<Manifest, String> {
    let bytes = std::fs::read(saved(dev, "manifest.json")).map_err(|_| "no update has been found yet: check first")?;
    let sig = std::fs::read_to_string(saved(dev, "manifest.sig")).map_err(|_| "the update's signature is missing")?;
    manifest::verify(&bytes, &sig, &dev.public_key().ok_or("this system has no update key")?)?;
    Manifest::parse(&bytes)
}

fn saved_release(dev: &Device) -> Result<net::Release, String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(saved(dev, "release.json")).map_err(|_| "check first")?).map_err(|e| e.to_string())?;
    let files = v["files"].as_array().into_iter().flatten().filter_map(|f| Some((f[0].as_str()?.to_owned(), f[1].as_str()?.to_owned()))).collect();
    Ok(net::Release { tag: v["tag"].as_str().unwrap_or_default().to_owned(), files })
}

/// How much an install would download from here: the patch from this
/// version when there is one, else the whole system; and the boot files.
fn download_size(m: &Manifest, version: &str) -> u64 {
    m.delta_from(version).map_or(m.full.size, |d| d.file.size) + m.boot.size
}

pub fn check(dev: &Device) -> Result<(), String> {
    let before = status::read(dev);
    if [state::DOWNLOADING, state::INSTALLING, state::TRYING].contains(&before.state.as_str()) {
        return Ok(());
    }
    let (version, build) = dev.version();
    let channel = status::pref(dev, "channel").unwrap_or_else(|| "stable".into());
    status::update(dev, |s| s.state = state::CHECKING.into());
    // The clock first: with it wrong (a clock chip that lost its time, a
    // network that blocks the time servers), no certificate checks out and
    // GitHub cannot be asked at all. Waits for a setting under way; quick
    // when the clock was set in the last few hours.
    if !dev.test {
        std::process::Command::new("/usr/sbin/catalyst-time").arg("sync").status().ok();
    }
    let found = (|| {
        let key = dev.public_key().ok_or("this system has no update key")?;
        let release = net::latest(&dev.source(), &channel)?;
        let url = |name: &str| net::file_url(&release, name).ok_or(format!("the release has no {name}"));
        let bytes = net::get(&url("manifest.json")?)?;
        let sig = String::from_utf8_lossy(&net::get(&url("manifest.sig")?)?).trim().to_owned();
        manifest::verify(&bytes, &sig, &key)?;
        let m = Manifest::parse(&bytes)?;
        std::fs::create_dir_all(&dev.state).map_err(|e| e.to_string())?;
        std::fs::write(saved(dev, "manifest.json"), &bytes).map_err(|e| e.to_string())?;
        std::fs::write(saved(dev, "manifest.sig"), &sig).map_err(|e| e.to_string())?;
        let files: Vec<Value> = release.files.iter().map(|(n, u)| json!([n, u])).collect();
        std::fs::write(saved(dev, "release.json"), json!({ "tag": release.tag, "files": files }).to_string()).map_err(|e| e.to_string())?;
        Ok::<_, String>(m)
    })();
    match found {
        Ok(m) if m.build > build => {
            // Already written and waiting for a restart: still so.
            let ready = before.state == state::READY && before.version == m.version;
            status::write(dev, &Status {
                state: (if ready { state::READY } else { state::AVAILABLE }).into(),
                version: m.version.clone(),
                size: download_size(&m, &version),
                checked: now(),
                notes: m.notes.clone(),
                ..Status::default()
            });
        }
        Ok(_) => status::write(dev, &Status { state: state::CURRENT.into(), version, checked: now(), ..Status::default() }),
        Err(why) => {
            // Not being able to look is not an update failing: say so, and
            // keep whatever was known.
            let mut s = before;
            if s.state.is_empty() || s.state == state::CHECKING {
                s.state = state::CURRENT.into();
            }
            s.detail = why_not(&why, now(), build);
            status::write(dev, &s);
            return Err(why);
        }
    }
    Ok(())
}

/// Why a check could not be made, in the few words Settings has room for.
/// A clock before this system was built is certainly wrong; a secure
/// connection that failed is most often the clock too.
fn why_not(why: &str, now: u64, build: u64) -> String {
    if now < build {
        return format!("clock wrong · {}", day_of(now));
    }
    if ["certificate", "SSL", "(60)", "(35)"].iter().any(|w| why.contains(w)) {
        return "secure link failed · clock?".into();
    }
    format!("could not check: {why}")
}

/// A moment as "2026-11-20" (UTC), from seconds since 1970: the civil date
/// from a day count (Howard Hinnant's method).
fn day_of(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}-{m:02}-{d:02}")
}

/// Whether the release's system fits the spare system's partition. Cards
/// flashed before 0.4.0 have 160 MB slots and later systems take 1 GB: such
/// a card has to be flashed afresh, which is said before anything is fetched.
fn fits(m: &Manifest, spare: &Path) -> Result<(), String> {
    // Only a partition has a fixed size; the tests' files grow as written.
    if !spare.starts_with("/dev") {
        return Ok(());
    }
    let room = room_of(spare).ok_or_else(|| format!("could not read the size of {}", spare.display()))?;
    if m.system_size > room {
        return Err(format!(
            "{} needs a {} MB system slot and this card's are {} MB: flash it fresh",
            m.version,
            m.system_size.div_ceil(1 << 20),
            room >> 20
        ));
    }
    Ok(())
}

/// A block device's size: where its end is.
fn room_of(path: &Path) -> Option<u64> {
    use std::io::{Seek, SeekFrom};
    std::fs::File::open(path).ok()?.seek(SeekFrom::End(0)).ok()
}

/// One install at a time: a lock holding the installer's process id.
struct Lock(PathBuf);

impl Lock {
    fn take(dev: &Device) -> Result<Lock, String> {
        std::fs::create_dir_all(&dev.state).map_err(|e| e.to_string())?;
        let path = saved(dev, "lock");
        if let Ok(pid) = std::fs::read_to_string(&path) {
            let alive = Path::new(&format!("/proc/{}", pid.trim())).exists() && pid.trim() != std::process::id().to_string();
            if alive && !dev.test {
                return Err("an update is already being installed".into());
            }
        }
        std::fs::write(&path, std::process::id().to_string()).map_err(|e| e.to_string())?;
        Ok(Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

/// The battery's charge, if there is a battery to ask.
fn battery() -> Option<(u32, bool)> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        if let Some(cap) = std::fs::read_to_string(p.join("capacity")).ok().and_then(|c| c.trim().parse().ok()) {
            let charging = std::fs::read_to_string(p.join("status")).is_ok_and(|s| s.trim() == "Charging" || s.trim() == "Full");
            return Some((cap, charging));
        }
    }
    None
}

/// Progress written for the shell, at most every 1%.
fn reporter<'a>(dev: &'a Device, stage: &'a str, version: &'a str) -> impl FnMut(f32) + 'a {
    let mut last = -1.0f32;
    move |p: f32| {
        if p - last >= 0.01 || p >= 1.0 {
            last = p;
            status::update(dev, |s| {
                s.state = stage.into();
                s.version = version.into();
                s.progress = p.clamp(0.0, 1.0);
            });
        }
    }
}

pub fn install(dev: &Device) -> Result<(), String> {
    let _lock = Lock::take(dev)?;
    let result = install_locked(dev);
    if let Err(why) = &result {
        status::update(dev, |s| {
            s.state = state::FAILED.into();
            s.detail = why.clone();
            s.progress = 0.0;
        });
    }
    result
}

fn install_locked(dev: &Device) -> Result<(), String> {
    let m = saved_manifest(dev)?;
    let release = saved_release(dev)?;
    if !dev.test {
        if let Some((charge, charging)) = battery() {
            if charge < LOW_BATTERY && !charging {
                return Err(format!("the battery is at {charge}%: plug in to install"));
            }
        }
    }
    let running = dev.running()?;
    let spare = 1 - running;
    fits(&m, &dev.systems[spare])?;
    let (version, _) = dev.version();
    let dl = dev.state.join("dl");
    std::fs::create_dir_all(&dl).map_err(|e| e.to_string())?;

    // A patch from this version, if there is one and this system is exactly
    // the one it was made from; otherwise the whole system.
    let mut patch = m.delta_from(&version).cloned();
    let total = download_size(&m, &version);
    let mut fetched = 0u64;
    let mut fetch = |file: &manifest::File, dev: &Device| -> Result<PathBuf, String> {
        let url = net::file_url(&release, &file.name).ok_or(format!("the release has no {}", file.name))?;
        let to = dl.join(&file.name);
        let mut report = reporter(dev, state::DOWNLOADING, &m.version);
        net::download(&url, &to, file.size, &mut |bytes| report((fetched + bytes) as f32 / total.max(1) as f32))?;
        fetched += file.size;
        let hash = pack::hash_file(&to)?;
        if hash != file.sha256 {
            std::fs::remove_file(&to).ok();
            return Err(format!("{} did not download whole (its hash is wrong); it will be fetched again", file.name));
        }
        Ok(to)
    };
    let boot_pack = fetch(&m.boot, dev)?;
    let payload = match patch.as_ref() {
        Some(d) => {
            let p = fetch(&d.file, dev)?;
            let (base_size, base_hash) = pack::delta_base(&p)?;
            let mut report = reporter(dev, state::INSTALLING, &m.version);
            let here = pack::hash_prefix(&dev.systems[running], base_size, &mut |p| report(p * 0.2))?;
            if here != base_hash || here != d.base_sha256 {
                // This system is not quite the one the patch was made from.
                patch = None;
                fetch(&m.full, dev)?
            } else {
                p
            }
        }
        None => fetch(&m.full, dev)?,
    };

    // Into the spare system.
    let written = {
        let mut report = reporter(dev, state::INSTALLING, &m.version);
        match &patch {
            Some(_) => pack::write_delta(&payload, &dev.systems[running], &dev.systems[spare], &mut |p| report(0.2 + p * 0.5))?,
            None => pack::write_full(&payload, &dev.systems[spare], m.system_size, &mut |p| report(p * 0.7))?,
        }
    };
    if written != m.system_sha256 {
        return Err("the new system did not come out as built (its hash is wrong)".into());
    }
    // Read it back: what is on the card must be what was built.
    let mut report = reporter(dev, state::INSTALLING, &m.version);
    if pack::hash_prefix(&dev.systems[spare], m.system_size, &mut |p| report(0.7 + p * 0.25))? != m.system_sha256 {
        return Err("the new system did not read back as it was written".into());
    }

    // Its kernel into its folder on the boot partition, with its own root,
    // and config.txt set to try it once.
    dev.with_boot(|boot| {
        let root = device::root_of(spare);
        let edit = |name: &str, bytes: Vec<u8>| -> Vec<u8> {
            if name != "cmdline.txt" {
                return bytes;
            }
            let line = String::from_utf8_lossy(&bytes);
            line.split_whitespace().map(|a| if a.starts_with("root=") { format!("root={root}") } else { a.to_owned() }).collect::<Vec<_>>().join(" ").into_bytes()
        };
        pack::unpack(&boot_pack, &boot.join(device::slot_dir(spare)), &edit)?;
        let config_path = boot.join("config.txt");
        let config = std::fs::read_to_string(&config_path).map_err(|e| format!("reading config.txt: {e}"))?;
        let tmp = boot.join("config.new");
        std::fs::write(&tmp, device::set_slots(&config, running, spare)).map_err(|e| format!("writing config.txt: {e}"))?;
        std::fs::rename(&tmp, &config_path).map_err(|e| format!("writing config.txt: {e}"))?;
        Ok(())
    })?;
    device::sync();
    std::fs::remove_dir_all(&dl).ok();
    status::write(dev, &Status { state: state::READY.into(), version: m.version.clone(), progress: 1.0, notes: m.notes.clone(), checked: status::read(dev).checked, ..Status::default() });
    Ok(())
}

/// Restart into the new system, once.
pub fn apply(dev: &Device) -> Result<(), String> {
    let s = status::read(dev);
    if s.state != state::READY {
        return Err("no update is waiting for a restart".into());
    }
    let running = dev.running()?;
    let (from, _) = dev.version();
    std::fs::write(saved(dev, "trying.json"), json!({ "version": s.version, "from": from, "slot": 1 - running, "at": now() }).to_string()).map_err(|e| e.to_string())?;
    status::update(dev, |s| {
        s.state = state::TRYING.into();
        s.detail.clear();
    });
    restart(dev, true)
}

/// Restart, as the one-time try of the other system or as usual.
fn restart(dev: &Device, tryboot: bool) -> Result<(), String> {
    device::sync();
    if dev.test {
        std::fs::write(saved(dev, "restart"), if tryboot { "0 tryboot" } else { "normal" }).map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        // /data put away first: nothing is left half-written.
        std::process::Command::new("mount").args(["-o", "remount,ro", "/data"]).status().ok();
        device::sync();
        if tryboot {
            let arg = std::ffi::CString::new("0 tryboot").expect("no NUL");
            unsafe {
                libc::syscall(libc::SYS_reboot, libc::LINUX_REBOOT_MAGIC1, libc::LINUX_REBOOT_MAGIC2, libc::LINUX_REBOOT_CMD_RESTART2, arg.as_ptr());
            }
            return Err("the restart did not happen".into());
        }
        std::process::Command::new("reboot").status().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The watchdog, fed while a tried system proves itself: if it hangs
/// outright, the watchdog restarts it, which means the old system.
struct Dog(Option<std::fs::File>);

impl Dog {
    fn open(dev: &Device) -> Dog {
        Dog(if dev.test { None } else { std::fs::OpenOptions::new().write(true).open(&dev.watchdog).ok() })
    }
    fn feed(&mut self) {
        use std::io::Write;
        if let Some(f) = &mut self.0 {
            f.write_all(b"1").ok();
        }
    }
}

impl Drop for Dog {
    fn drop(&mut self) {
        use std::io::Write;
        // The magic close: stand the watchdog down.
        if let Some(f) = &mut self.0 {
            f.write_all(b"V").ok();
        }
    }
}

pub fn verify(dev: &Device) -> Result<(), String> {
    let trying: Option<Value> = std::fs::read_to_string(saved(dev, "trying.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let running = dev.running()?;
    let (version, _) = dev.version();
    match (dev.tried(), trying) {
        (true, Some(t)) if t["slot"].as_u64() == Some(running as u64) => {
            let new_version = t["version"].as_str().unwrap_or(&version).to_owned();
            status::update(dev, |s| {
                s.state = state::TRYING.into();
                s.version = new_version.clone();
            });
            let wait: u64 = std::env::var("CATALYST_UPDATE_VERIFY_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(180);
            let mut dog = Dog::open(dev);
            let started = std::time::Instant::now();
            let up = loop {
                dog.feed();
                if dev.shell_ok.exists() {
                    break true;
                }
                if started.elapsed().as_secs() >= wait {
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(if dev.test { 20 } else { 1000 }));
            };
            if up {
                // It works: from now on it is the system that starts.
                dev.with_boot(|boot| {
                    let path = boot.join("config.txt");
                    let config = std::fs::read_to_string(&path).map_err(|e| format!("reading config.txt: {e}"))?;
                    let tmp = boot.join("config.new");
                    std::fs::write(&tmp, device::set_slots(&config, running, 1 - running)).map_err(|e| e.to_string())?;
                    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
                })?;
                std::fs::remove_file(saved(dev, "trying.json")).ok();
                let from = t["from"].as_str().unwrap_or_default().to_owned();
                status::write(dev, &Status { state: state::UPDATED.into(), version: new_version, detail: format!("from {from}"), checked: status::read(dev).checked, ..Status::default() });
                Ok(())
            } else {
                // Not up in time: back to the system that worked.
                drop(dog);
                restart(dev, false)
            }
        }
        // A try nobody asked for: leave it; the next start is the usual system.
        (true, _) => Ok(()),
        (false, Some(t)) => {
            // The try did not take, and the firmware started this system again.
            std::fs::remove_file(saved(dev, "trying.json")).ok();
            let tried = t["version"].as_str().unwrap_or("the update").to_owned();
            status::write(dev, &Status {
                state: state::FAILED.into(),
                version: tried.clone(),
                detail: format!("{tried} did not start, so this is still {version}"),
                checked: status::read(dev).checked,
                ..Status::default()
            });
            Ok(())
        }
        (false, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release;

    fn curl_works() -> bool {
        std::process::Command::new("curl").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn a_check_that_could_not_be_made_says_why_in_a_few_words() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(951_782_400), "2000-02-29");
        assert_eq!(day_of(951_868_800), "2000-03-01");
        assert_eq!(day_of(1_795_132_800), "2026-11-20");
        let build = 1_795_132_800;
        // A clock chip that lost its time: before this system was built.
        assert_eq!(why_not("could not fetch …: curl: (60) SSL certificate problem", 946_684_800, build), "clock wrong · 2000-01-01");
        // After it, but a certificate would not check out: most often the clock.
        assert_eq!(why_not("curl: (60) SSL certificate problem: certificate is not yet valid", build + 10, build), "secure link failed · clock?");
        assert_eq!(why_not("no stable release yet", build + 10, build), "could not check: no stable release yet");
        // Short enough for the row it is shown in.
        assert!(why_not("x (60)", build + 10, build).chars().count() <= 28);
        assert!(why_not("x", 0, build).chars().count() <= 28);
    }

    fn file_url(path: &Path) -> String {
        let p = path.to_string_lossy().replace('\\', "/");
        if p.starts_with('/') {
            format!("file://{p}")
        } else {
            format!("file:///{p}")
        }
    }

    /// A pretend build's outputs: a system image, the boot folder, a card.
    fn build(dir: &Path, seed: u8) -> PathBuf {
        let images = dir.join(format!("images-{seed}"));
        std::fs::create_dir_all(images.join("bootfs/slot-a/overlays")).unwrap();
        let mut system = vec![0u8; 40 * pack::BLOCK];
        for (i, b) in system.iter_mut().enumerate().step_by(97) {
            *b = (i as u8).wrapping_add(if i > 30 * pack::BLOCK { seed } else { 0 });
        }
        std::fs::write(images.join("rootfs.ext2"), &system).unwrap();
        std::fs::write(images.join("bootfs/slot-a/Image"), format!("kernel {seed}")).unwrap();
        std::fs::write(images.join("bootfs/slot-a/cmdline.txt"), "root=/dev/mmcblk0p2 rootfstype=ext4 ro panic=10").unwrap();
        std::fs::write(images.join("bootfs/slot-a/overlays/dwc2.dtbo"), "overlay").unwrap();
        std::fs::write(images.join("sdcard.img"), &system[..1000]).unwrap();
        images
    }

    #[test]
    fn an_update_is_found_installed_tried_kept_or_rolled_back() {
        if !curl_works() {
            eprintln!("no curl here: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("catalyst-update-e2e-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let key = "42".repeat(32);

        // Two releases, the second with a patch from the first.
        let first = dir.join("release-0.1.0");
        release::make(&release::Make { images: build(&dir, 1), out: first.clone(), version: "0.1.0".into(), channel: "stable".into(), build: 100, notes: String::new(), key: key.clone(), previous: None, repo: "x/y".into(), tag: "v0.1.0".into() }).unwrap();
        let second = dir.join("release-0.2.0");
        release::make(&release::Make { images: build(&dir, 2), out: second.clone(), version: "0.2.0".into(), channel: "stable".into(), build: 200, notes: "new glass".into(), key: key.clone(), previous: Some(first.clone()), repo: "x/y".into(), tag: "v0.2.0".into() }).unwrap();

        // A handheld running 0.1.0 from system A.
        let root = dir.join("device");
        let dev = Device::under(&root);
        for d in ["boot/slot-a", "proc", "run", "etc/catalyst"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::copy(first.join("system.img"), &dev.systems[0]).unwrap();
        std::fs::write(root.join("boot/config.txt"), device::set_slots("kernel=Image\n", 0, 1)).unwrap();
        std::fs::write(&dev.cmdline, "root=/dev/mmcblk0p2 ro").unwrap();
        std::fs::write(dev.etc.join("version"), "version=0.1.0\nbuild=100\n").unwrap();
        std::fs::write(dev.etc.join("update.pub"), manifest::public_of(&key).unwrap()).unwrap();
        std::fs::write(dev.etc.join("update.conf"), format!("source=url:{}\n", file_url(&second))).unwrap();

        // Found, and smaller than the whole system: it is a patch.
        check(&dev).unwrap();
        let s = status::read(&dev);
        assert_eq!((s.state.as_str(), s.version.as_str(), s.notes.as_str()), (state::AVAILABLE, "0.2.0", "new glass"));
        assert!(s.size < std::fs::metadata(second.join("system.img.gz")).unwrap().len(), "a patch: {} bytes", s.size);

        // Installed into system B, its kernel into slot-b, config.txt set to try it.
        install(&dev).unwrap();
        assert_eq!(status::read(&dev).state, state::READY);
        assert_eq!(std::fs::read(&dev.systems[1]).unwrap(), std::fs::read(second.join("system.img")).unwrap());
        assert_eq!(std::fs::read_to_string(root.join("boot/slot-b/Image")).unwrap(), "kernel 2");
        assert!(std::fs::read_to_string(root.join("boot/slot-b/cmdline.txt")).unwrap().contains("root=/dev/mmcblk0p3"));
        let config = std::fs::read_to_string(root.join("boot/config.txt")).unwrap();
        assert_eq!(device::slots_of(&config), Some((0, 1)));
        assert!(config.starts_with("kernel=Image"));
        // Checking again does not lose it.
        check(&dev).unwrap();
        assert_eq!(status::read(&dev).state, state::READY);

        // Restart: a try of system B.
        apply(&dev).unwrap();
        assert_eq!(std::fs::read_to_string(dev.state.join("restart")).unwrap(), "0 tryboot");

        // It comes up and the shell shows it is well: kept.
        std::fs::write(&dev.cmdline, "root=/dev/mmcblk0p3 ro").unwrap();
        std::fs::write(&dev.tryboot, [0, 0, 0, 1]).unwrap();
        std::fs::write(dev.etc.join("version"), "version=0.2.0\nbuild=200\n").unwrap();
        std::fs::write(&dev.shell_ok, "").unwrap();
        verify(&dev).unwrap();
        let s = status::read(&dev);
        assert_eq!((s.state.as_str(), s.version.as_str(), s.detail.as_str()), (state::UPDATED, "0.2.0", "from 0.1.0"));
        assert_eq!(device::slots_of(&std::fs::read_to_string(root.join("boot/config.txt")).unwrap()), Some((1, 0)), "B starts from now on");

        // Another time: a try whose shell never comes up is restarted, and
        // the old system then says the update did not take.
        std::fs::write(dev.state.join("trying.json"), json!({ "version": "0.3.0", "from": "0.2.0", "slot": 0 }).to_string()).unwrap();
        std::fs::write(&dev.cmdline, "root=/dev/mmcblk0p2 ro").unwrap();
        std::fs::remove_file(&dev.shell_ok).unwrap();
        std::env::set_var("CATALYST_UPDATE_VERIFY_SECONDS", "0");
        verify(&dev).unwrap();
        assert_eq!(std::fs::read_to_string(dev.state.join("restart")).unwrap(), "normal");
        std::fs::write(&dev.cmdline, "root=/dev/mmcblk0p3 ro").unwrap();
        std::fs::write(&dev.tryboot, [0, 0, 0, 0]).unwrap();
        verify(&dev).unwrap();
        let s = status::read(&dev);
        assert_eq!(s.state, state::FAILED);
        assert_eq!(s.detail, "0.3.0 did not start, so this is still 0.2.0");
        assert_eq!(device::slots_of(&std::fs::read_to_string(root.join("boot/config.txt")).unwrap()), Some((1, 0)), "still B");

        // A manifest not signed by the handheld's key is refused.
        std::fs::write(dev.etc.join("update.pub"), manifest::public_of(&"77".repeat(32)).unwrap()).unwrap();
        assert!(check(&dev).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
