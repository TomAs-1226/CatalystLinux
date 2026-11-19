//! The cards the flasher may write: removable drives only (SD readers and
//! USB), never a disk the computer starts from, never one larger than an SD
//! card plausibly is. And writing one: the whole image, with its first
//! megabyte (the partition table) written last, so the computer does not
//! notice partitions and mount them while the rest is still being written.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Disk {
    /// What is opened to write it: `\\.\PhysicalDrive2`, `/dev/sdb`, or (for
    /// tests) a plain file.
    pub path: PathBuf,
    /// Windows' disk number, to clear and refresh it.
    pub number: Option<u32>,
    pub name: String,
    pub size: u64,
    /// Drive letters (Windows) or mount points it has now.
    pub mounted: String,
}

/// No SD card is bigger than this; anything bigger is not offered.
const LARGEST: u64 = 300 * 1_000_000_000;

#[cfg(windows)]
pub fn list() -> Vec<Disk> {
    let script = "Get-Disk | Where-Object { ($_.BusType -eq 'USB' -or $_.BusType -eq 'SD' -or $_.BusType -eq 'MMC') -and -not $_.IsSystem -and -not $_.IsBoot -and $_.Size -gt 0 } | \
        ForEach-Object { [pscustomobject]@{ Number = $_.Number; Name = $_.FriendlyName; Size = $_.Size; \
        Letters = ((@(Get-Partition -DiskNumber $_.Number -ErrorAction SilentlyContinue | Where-Object DriveLetter | ForEach-Object { \"$($_.DriveLetter):\" })) -join ' ') } } | ConvertTo-Json -Compress";
    let Ok(out) = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).output() else { return Vec::new() };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    // One disk comes back as an object, several as a list.
    let all = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![v],
        _ => Vec::new(),
    };
    all.iter()
        .filter_map(|d| {
            let number = d["Number"].as_u64()? as u32;
            Some(Disk {
                path: format!(r"\\.\PhysicalDrive{number}").into(),
                number: Some(number),
                name: d["Name"].as_str().unwrap_or("Removable drive").trim().to_owned(),
                size: d["Size"].as_u64()?,
                mounted: d["Letters"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .filter(|d| d.size <= LARGEST)
        .collect()
}

#[cfg(target_os = "linux")]
pub fn list() -> Vec<Disk> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let roots: Vec<&str> = mounts.lines().filter(|l| matches!(l.split(' ').nth(1), Some("/") | Some("/boot") | Some("/boot/efi"))).filter_map(|l| l.split(' ').next()).collect();
    let mut disks = Vec::new();
    for entry in std::fs::read_dir("/sys/block").into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let sys = entry.path();
        let read = |f: &str| std::fs::read_to_string(sys.join(f)).unwrap_or_default().trim().to_owned();
        let usb = std::fs::read_link(&sys).is_ok_and(|p| p.to_string_lossy().contains("/usb"));
        if !(read("removable") == "1" || usb || name.starts_with("mmcblk")) {
            continue;
        }
        let dev = format!("/dev/{name}");
        // Never the disk this computer runs from.
        if roots.iter().any(|r| r.starts_with(&dev)) {
            continue;
        }
        let size = read("size").parse::<u64>().unwrap_or(0) * 512;
        if size == 0 || size > LARGEST {
            continue;
        }
        let model = read("device/model");
        let mounted: Vec<&str> = mounts.lines().filter(|l| l.starts_with(&dev)).filter_map(|l| l.split(' ').nth(1)).collect();
        disks.push(Disk { path: dev.into(), number: None, name: if model.is_empty() { name } else { model }, size, mounted: mounted.join(" ") });
    }
    disks
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn list() -> Vec<Disk> {
    Vec::new()
}

/// Whether this program may write disks (Administrator, or root).
pub fn can_write() -> bool {
    #[cfg(windows)]
    {
        // Opening the first disk to read needs the same rights as writing one.
        std::fs::File::open(r"\\.\PhysicalDrive0").is_ok()
    }
    #[cfg(unix)]
    {
        std::fs::read_to_string("/proc/self/status").is_ok_and(|s| s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0")))
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

/// Start this program again with the rights to write disks, carrying on
/// from where it was (`args`). The current copy should then close.
pub fn elevate(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let quoted: Vec<String> = args.iter().map(|a| format!("'\"{}\"'", a.replace('\'', "''"))).collect();
        let list = if quoted.is_empty() { String::new() } else { format!(" -ArgumentList {}", quoted.join(",")) };
        let command = format!("Start-Process -FilePath '{}'{list} -Verb RunAs", exe.display());
        let ok = Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &command]).status().is_ok_and(|s| s.success());
        if ok {
            Ok(())
        } else {
            Err("writing a card needs Administrator: the request was declined".into())
        }
    }
    #[cfg(not(windows))]
    {
        let ok = Command::new("pkexec").arg(exe).args(args).spawn().is_ok();
        if ok {
            Ok(())
        } else {
            Err("writing a card needs root: run with sudo".into())
        }
    }
}

/// Get a disk ready to be written: nothing mounted from it, no partitions
/// left for the computer to hold on to.
pub fn prepare(disk: &Disk) -> Result<(), String> {
    if disk.path.to_string_lossy().starts_with(r"\\.\") {
        let n = disk.number.ok_or("no disk number")?;
        let script = format!("$ErrorActionPreference = 'Stop'; Set-Disk -Number {n} -IsReadOnly $false -ErrorAction SilentlyContinue; Clear-Disk -Number {n} -RemoveData -RemoveOEM -Confirm:$false");
        let out = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output().map_err(|e| e.to_string())?;
        // A blank card has nothing to clear, which is not a failure.
        let err = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !err.contains("RAW") && !err.contains("not initialized") {
            return Err(format!("could not clear the card: {}", err.lines().next().unwrap_or("").trim()));
        }
    } else if disk.path.starts_with("/dev") {
        for point in disk.mounted.split_whitespace() {
            Command::new("umount").arg(point).status().ok();
        }
    }
    Ok(())
}

/// After writing: the computer reads the card's new partitions.
pub fn refresh(disk: &Disk) {
    if let Some(n) = disk.number {
        Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &format!("Update-Disk -Number {n}")]).status().ok();
    }
}

/// Writes go in whole sectors.
const SECTOR: usize = 512;
/// The partition table and everything before the first partition.
pub const HEAD: usize = 1 << 20;

/// Write an image, read from `image`, to `disk`. Its first megabyte is held
/// back and written last. Returns the image's length and SHA-256.
pub fn write(disk: &Disk, image: &mut dyn Read, progress: &mut dyn FnMut(u64)) -> Result<(u64, String), String> {
    use sha2::{Digest, Sha256};
    let mut out = std::fs::OpenOptions::new().write(true).create(!disk.path.starts_with("/dev") && !disk.path.to_string_lossy().starts_with(r"\\.\")).truncate(false).open(&disk.path).map_err(|e| format!("opening the card: {e}"))?;
    let mut hasher = Sha256::new();
    let mut head = Vec::with_capacity(HEAD);
    let mut chunk = vec![0u8; 4 << 20];
    let mut written = 0u64;
    loop {
        let mut filled = 0;
        while filled < chunk.len() {
            let n = image.read(&mut chunk[filled..]).map_err(|e| format!("reading the image: {e}"))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        hasher.update(&chunk[..filled]);
        let mut data = &chunk[..filled];
        // The first megabyte waits.
        if head.len() < HEAD {
            let take = (HEAD - head.len()).min(data.len());
            head.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        if !data.is_empty() {
            let at = written + (filled - data.len()) as u64;
            let mut padded = data.to_vec();
            padded.resize(data.len().div_ceil(SECTOR) * SECTOR, 0);
            out.seek(SeekFrom::Start(at)).map_err(|e| format!("writing the card: {e}"))?;
            out.write_all(&padded).map_err(|e| format!("writing the card: {e}"))?;
        }
        written += filled as u64;
        progress(written);
        if filled < chunk.len() {
            break;
        }
    }
    // Now the partition table: the card becomes whole at once.
    let mut padded = head.clone();
    padded.resize(head.len().div_ceil(SECTOR) * SECTOR, 0);
    out.seek(SeekFrom::Start(0)).map_err(|e| format!("writing the card: {e}"))?;
    out.write_all(&padded).map_err(|e| format!("writing the card: {e}"))?;
    out.flush().map_err(|e| format!("writing the card: {e}"))?;
    out.sync_all().ok();
    Ok((written, sha2_hex(&hasher.finalize())))
}

/// Read the first `len` bytes of the card back and hash them.
pub fn read_back(disk: &Disk, len: u64, progress: &mut dyn FnMut(u64)) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(&disk.path).map_err(|e| format!("reading the card back: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    let mut done = 0u64;
    while done < len {
        // Whole sectors from the device; only the image's own bytes counted.
        let want = ((len - done) as usize).min(buf.len()).div_ceil(SECTOR) * SECTOR;
        let want = want.min(buf.len());
        let n = f.read(&mut buf[..want]).map_err(|e| format!("reading the card back: {e}"))?;
        if n == 0 {
            return Err("the card ended before the image did".into());
        }
        let used = (n as u64).min(len - done) as usize;
        hasher.update(&buf[..used]);
        done += used as u64;
        progress(done);
    }
    Ok(sha2_hex(&hasher.finalize()))
}

pub fn sha2_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_lands_whole_with_its_head_written_last() {
        let dir = std::env::temp_dir().join(format!("catalyst-flasher-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image: Vec<u8> = (0..(HEAD * 3 + 777)).map(|i| (i * 7 % 251) as u8).collect();
        let card = Disk { path: dir.join("card.img"), number: None, name: "test".into(), size: 8 << 20, mounted: String::new() };
        std::fs::write(&card.path, vec![0xAAu8; 8 << 20]).unwrap();
        let mut seen = Vec::new();
        let (len, hash) = write(&card, &mut image.as_slice(), &mut |n| seen.push(n)).unwrap();
        assert_eq!(len, image.len() as u64);
        let on_card = std::fs::read(&card.path).unwrap();
        assert_eq!(&on_card[..image.len()], &image[..]);
        // Padding to a whole sector, and the rest of the card left alone.
        assert!(on_card[image.len()..image.len().div_ceil(SECTOR) * SECTOR].iter().all(|&b| b == 0));
        assert_eq!(on_card[image.len().div_ceil(SECTOR) * SECTOR], 0xAA);
        assert_eq!(read_back(&card, len, &mut |_| {}).unwrap(), hash);
        assert_eq!(*seen.last().unwrap(), len);
        std::fs::remove_dir_all(&dir).ok();
    }
}
