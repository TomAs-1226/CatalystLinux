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
        // Online first: a disk another tool took offline (WSL's mount does) cannot be cleared.
        let script = format!("$ErrorActionPreference = 'Stop'; Set-Disk -Number {n} -IsOffline $false -ErrorAction SilentlyContinue; Set-Disk -Number {n} -IsReadOnly $false -ErrorAction SilentlyContinue; Clear-Disk -Number {n} -RemoveData -RemoveOEM -Confirm:$false");
        let out = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output().map_err(|e| e.to_string())?;
        // A blank card has nothing to clear, which is not a failure.
        let err = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !err.contains("RAW") && !err.contains("not initialized") {
            // Some USB card readers refuse Clear-Disk (error 40004) but take
            // diskpart's clean, which drops the partition table the same way.
            let script = std::env::temp_dir().join(format!("catalyst-flasher-clean-{n}.txt"));
            std::fs::write(&script, format!("select disk {n}\r\nattributes disk clear readonly noerr\r\nclean\r\n")).map_err(|e| e.to_string())?;
            let cleaned = Command::new("diskpart").arg("/s").arg(&script).output();
            std::fs::remove_file(&script).ok();
            let ok = cleaned.as_ref().is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).to_lowercase().contains("succeeded in cleaning"));
            if !ok {
                return Err(format!("could not clear the card: {}", err.lines().next().unwrap_or("").trim()));
            }
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

/// One MBR partition: its type, first sector and length in sectors.
pub type Part = (u8, u32, u32);

/// The four partitions of an MBR (sector 0), if it is one.
pub fn partitions(mbr: &[u8]) -> Option<[Part; 4]> {
    if mbr.len() < 512 || mbr[510] != 0x55 || mbr[511] != 0xAA {
        return None;
    }
    let mut out = [(0u8, 0u32, 0u32); 4];
    for (i, p) in out.iter_mut().enumerate() {
        let e = &mbr[446 + 16 * i..446 + 16 * (i + 1)];
        *p = (e[4], u32::from_le_bytes([e[8], e[9], e[10], e[11]]), u32::from_le_bytes([e[12], e[13], e[14], e[15]]));
    }
    Some(out)
}

/// Where an image may be written over a card while its data partition (the
/// fourth) is kept: the boot partition and both systems must lie exactly
/// where the image puts them, and the data partition start where it does
/// (it may have grown since). The bytes from the first partition's start up
/// to the data partition's, as (from, to).
pub fn keep_data_range(image: &[Part; 4], card: &[Part; 4]) -> Result<(u64, u64), String> {
    for i in 0..3 {
        if image[i] != card[i] {
            return Err(format!("the card's partition {} is not where this image has it ({:?} on the card, {:?} in the image)", i + 1, card[i], image[i]));
        }
    }
    if image[3].0 != card[3].0 || image[3].1 != card[3].1 {
        return Err("the card's data partition does not start where this image's does".into());
    }
    if image[0].1 == 0 || image[3].1 <= image[0].1 {
        return Err("the image's partitions are not in order".into());
    }
    Ok((u64::from(image[0].1) * SECTOR as u64, u64::from(image[3].1) * SECTOR as u64))
}

/// Windows lets nothing write into the sectors of a volume it holds, even
/// through the whole disk: each of these partitions' volumes is locked and
/// dismounted, and stays so while the files returned are kept (dropping
/// them lets go). Elsewhere, nothing to do.
pub fn lock_partitions(disk: &Disk, partitions: &[u32]) -> Result<Vec<std::fs::File>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        const FSCTL_LOCK_VOLUME: u32 = 0x0009_0018;
        const FSCTL_DISMOUNT_VOLUME: u32 = 0x0009_0020;
        #[link(name = "kernel32")]
        extern "system" {
            fn DeviceIoControl(device: *mut std::ffi::c_void, code: u32, input: *mut std::ffi::c_void, input_len: u32, output: *mut std::ffi::c_void, output_len: u32, returned: *mut u32, overlapped: *mut std::ffi::c_void) -> i32;
        }
        let Some(n) = disk.number else { return Ok(Vec::new()) };
        let mut held = Vec::new();
        for p in partitions {
            let path = format!(r"\\.\Harddisk{n}Partition{p}");
            let volume = std::fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|e| format!("opening partition {p}: {e}"))?;
            for (code, what) in [(FSCTL_LOCK_VOLUME, "lock"), (FSCTL_DISMOUNT_VOLUME, "dismount")] {
                let mut returned = 0u32;
                // SAFETY: a valid handle, no buffers.
                let ok = unsafe { DeviceIoControl(volume.as_raw_handle(), code, std::ptr::null_mut(), 0, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut()) };
                if ok == 0 {
                    return Err(format!("could not {what} partition {p}: {}", std::io::Error::last_os_error()));
                }
            }
            held.push(volume);
        }
        Ok(held)
    }
    #[cfg(not(windows))]
    {
        let _ = (disk, partitions);
        Ok(Vec::new())
    }
}

/// Write the bytes of an image that lie in [from, to) to the same place on
/// the disk, and nothing else. Returns their SHA-256.
pub fn write_range(disk: &Disk, image: &mut dyn Read, from: u64, to: u64, progress: &mut dyn FnMut(u64)) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut out = std::fs::OpenOptions::new().write(true).open(&disk.path).map_err(|e| format!("opening the card: {e}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 4 << 20];
    let mut at = 0u64;
    while at < to {
        let want = (chunk.len() as u64).min(to - at) as usize;
        let mut filled = 0;
        while filled < want {
            let n = image.read(&mut chunk[filled..want]).map_err(|e| format!("reading the image: {e}"))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            return Err("the image ended before the data partition".into());
        }
        let end = at + filled as u64;
        if end > from {
            let skip = from.saturating_sub(at) as usize;
            let data = &chunk[skip..filled];
            hasher.update(data);
            let mut padded = data.to_vec();
            padded.resize(data.len().div_ceil(SECTOR) * SECTOR, 0);
            out.seek(SeekFrom::Start(at + skip as u64)).map_err(|e| format!("writing the card: {e}"))?;
            out.write_all(&padded).map_err(|e| format!("writing the card: {e}"))?;
            progress(end - from);
        }
        at = end;
    }
    out.flush().map_err(|e| format!("writing the card: {e}"))?;
    out.sync_all().ok();
    Ok(sha2_hex(&hasher.finalize()))
}

/// Read [from, to) of the card back and hash it.
pub fn read_back_range(disk: &Disk, from: u64, to: u64, progress: &mut dyn FnMut(u64)) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(&disk.path).map_err(|e| format!("reading the card back: {e}"))?;
    f.seek(SeekFrom::Start(from)).map_err(|e| format!("reading the card back: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    let mut done = from;
    while done < to {
        let want = ((to - done) as usize).min(buf.len());
        let n = f.read(&mut buf[..want]).map_err(|e| format!("reading the card back: {e}"))?;
        if n == 0 {
            return Err("the card ended early".into());
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done - from);
    }
    Ok(sha2_hex(&hasher.finalize()))
}

/// The first sector of the card: its partition table.
pub fn read_mbr(disk: &Disk) -> Result<Vec<u8>, String> {
    let mut f = std::fs::File::open(&disk.path).map_err(|e| format!("reading the card: {e}"))?;
    let mut mbr = vec![0u8; SECTOR];
    f.read_exact(&mut mbr).map_err(|e| format!("reading the card: {e}"))?;
    Ok(mbr)
}

pub fn sha2_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An MBR with these partitions.
    fn mbr(parts: &[Part; 4]) -> Vec<u8> {
        let mut m = vec![0u8; 512];
        for (i, (kind, first, len)) in parts.iter().enumerate() {
            let e = &mut m[446 + 16 * i..446 + 16 * (i + 1)];
            e[4] = *kind;
            e[8..12].copy_from_slice(&first.to_le_bytes());
            e[12..16].copy_from_slice(&len.to_le_bytes());
        }
        m[510] = 0x55;
        m[511] = 0xAA;
        m
    }

    #[test]
    fn keeping_the_data_partition_writes_only_the_boot_and_systems() {
        let dir = std::env::temp_dir().join(format!("catalyst-flasher-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The image: boot at sector 8, systems after, data at 40 (small).
        let image_parts: [Part; 4] = [(0x0c, 8, 8), (0x83, 16, 12), (0x83, 28, 12), (0x83, 40, 8)];
        let mut image = mbr(&image_parts);
        image.resize(48 * 512, 0);
        for (i, b) in image.iter_mut().enumerate().skip(512) {
            *b = (i % 250) as u8 + 1;
        }
        // The card: the same, but its data partition grown and full of data.
        let card_parts: [Part; 4] = [(0x0c, 8, 8), (0x83, 16, 12), (0x83, 28, 12), (0x83, 40, 400)];
        let mut card = mbr(&card_parts);
        card.resize(440 * 512, 0xDD);
        let card_path = dir.join("card.img");
        std::fs::write(&card_path, &card).unwrap();
        let disk = Disk { path: card_path.clone(), number: None, name: "test".into(), size: card.len() as u64, mounted: String::new() };

        let (from, to) = keep_data_range(&image_parts, &partitions(&read_mbr(&disk).unwrap()).unwrap()).unwrap();
        assert_eq!((from, to), (8 * 512, 40 * 512));
        let hash = write_range(&disk, &mut &image[..], from, to, &mut |_| {}).unwrap();
        assert_eq!(read_back_range(&disk, from, to, &mut |_| {}).unwrap(), hash);
        let after = std::fs::read(&card_path).unwrap();
        assert_eq!(&after[..512], &card[..512], "the card's partition table (its grown data partition) stays");
        assert_eq!(&after[from as usize..to as usize], &image[from as usize..to as usize], "boot and systems are the image's");
        assert!(after[to as usize..].iter().all(|b| *b == 0xDD), "not a byte of the data partition touched");

        // A card laid out otherwise is refused before anything is written.
        let other: [Part; 4] = [(0x0c, 8, 8), (0x83, 16, 10), (0x83, 26, 12), (0x83, 38, 400)];
        assert!(keep_data_range(&image_parts, &other).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

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
