//! Making a release, on the computer that builds Catalyst Linux: the files
//! a handheld updates from, the card image the flasher and Raspberry Pi
//! Imager write, and the manifest that lists them, signed.

use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::json;

use crate::manifest::{self, Delta, File, Manifest};
use crate::pack;

pub struct Make {
    /// Buildroot's images folder: rootfs.ext2, bootfs/slot-a, sdcard.img.
    pub images: PathBuf,
    pub out: PathBuf,
    pub version: String,
    pub channel: String,
    pub build: u64,
    pub notes: String,
    /// The signing key's seed, hex.
    pub key: String,
    /// The release before this one (its folder): a patch from it is made.
    pub previous: Option<PathBuf>,
    pub repo: String,
    pub tag: String,
}

fn gzip(from: &Path, to: &Path) -> Result<(), String> {
    let mut input = std::fs::File::open(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let mut gz = GzEncoder::new(std::fs::File::create(to).map_err(|e| e.to_string())?, Compression::best());
    std::io::copy(&mut input, &mut gz).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn file(out: &Path, name: &str) -> Result<File, String> {
    let path = out.join(name);
    Ok(File { name: name.into(), size: std::fs::metadata(&path).map_err(|e| e.to_string())?.len(), sha256: pack::hash_file(&path)? })
}

pub fn make(m: &Make) -> Result<Manifest, String> {
    std::fs::create_dir_all(&m.out).map_err(|e| e.to_string())?;
    let system = m.images.join("rootfs.ext2");
    // Kept whole (not uploaded), for the next release's patch from this one.
    std::fs::copy(&system, m.out.join("system.img")).map_err(|e| format!("copying the system: {e}"))?;
    let system_size = std::fs::metadata(&system).map_err(|e| e.to_string())?.len();
    let system_sha256 = pack::hash_file(&system)?;
    gzip(&system, &m.out.join("system.img.gz"))?;
    pack::pack_dir(&m.images.join("bootfs/slot-a"), &m.out.join("boot.pack"))?;

    let mut deltas = Vec::new();
    if let Some(prev) = &m.previous {
        let from = std::fs::read_to_string(prev.join("version")).map_err(|_| format!("{} has no version file", prev.display()))?.trim().to_owned();
        let name = format!("system-from-{from}.cdelta");
        let blocks = pack::make_delta(&prev.join("system.img"), &system, &m.out.join(&name))?;
        eprintln!("patch from {from}: {blocks} blocks of {} KB differ", pack::BLOCK / 1024);
        deltas.push(Delta { from, base_sha256: pack::hash_file(&prev.join("system.img"))?, file: file(&m.out, &name)? });
    }

    // The whole card, for the flasher and Raspberry Pi Imager.
    let card = m.images.join("sdcard.img");
    let card_name = format!("catalyst-linux-{}.img.gz", m.version);
    gzip(&card, &m.out.join(&card_name))?;
    let card_file = file(&m.out, &card_name)?;
    let imager = json!({
        "os_list": [{
            "name": format!("Catalyst Linux {}", m.version),
            "description": "For the M5Stack CardputerZero: the robot's companion and a pocket developer machine.",
            "url": format!("https://github.com/{}/releases/download/{}/{card_name}", m.repo, m.tag),
            "icon": format!("https://raw.githubusercontent.com/{}/main/assets/catalyst-icon.png", m.repo),
            "website": format!("https://github.com/{}", m.repo),
            "release_date": chrono_date(m.build),
            "extract_size": std::fs::metadata(&card).map_err(|e| e.to_string())?.len(),
            "extract_sha256": pack::hash_file(&card)?,
            "image_download_size": card_file.size,
            "image_download_sha256": card_file.sha256,
            "devices": ["pi3-64bit"],
        }]
    });
    std::fs::write(m.out.join("os_list_catalyst.json"), serde_json::to_string_pretty(&imager).expect("json")).map_err(|e| e.to_string())?;

    let manifest = Manifest {
        version: m.version.clone(),
        channel: m.channel.clone(),
        build: m.build,
        notes: m.notes.clone(),
        system_size,
        system_sha256,
        full: file(&m.out, "system.img.gz")?,
        deltas,
        boot: file(&m.out, "boot.pack")?,
    };
    let text = manifest.to_json();
    std::fs::write(m.out.join("manifest.json"), &text).map_err(|e| e.to_string())?;
    std::fs::write(m.out.join("manifest.sig"), manifest::sign(text.as_bytes(), &m.key)?).map_err(|e| e.to_string())?;
    std::fs::write(m.out.join("version"), &m.version).map_err(|e| e.to_string())?;
    Ok(manifest)
}

/// A date as Raspberry Pi Imager wants it: YYYY-MM-DD.
fn chrono_date(epoch: u64) -> String {
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = (epoch / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A new signing key: its seed into `key` (kept secret, never published)
/// and its public half into `public` (built into the image).
pub fn keygen(key: &Path, public: &Path) -> Result<(), String> {
    if key.exists() {
        return Err(format!("{} exists already: a new key would orphan every handheld that has the old one", key.display()));
    }
    let mut seed = [0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut seed)).map_err(|e| format!("no randomness: {e}"))?;
    let seed = manifest::hex(&seed);
    if let Some(dir) = key.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::File::create(key).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600)).ok();
    }
    writeln!(f, "{seed}").map_err(|e| e.to_string())?;
    std::fs::write(public, format!("{}\n", manifest::public_of(&seed)?)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_are_civil() {
        assert_eq!(super::chrono_date(0), "1970-01-01");
        assert_eq!(super::chrono_date(1_763_500_000), "2025-11-18");
        assert_eq!(super::chrono_date(951_782_400), "2000-02-29");
    }
}
