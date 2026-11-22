//! The files an update is made of, and how each is written:
//!
//! - **the whole system**, gzip-compressed, streamed into the spare system's
//!   partition;
//! - **a patch** from one version's system to the next: only the blocks that
//!   differ, compressed; the rest is copied from the running system. A
//!   handful of megabytes instead of tens;
//! - **the boot files** (kernel, device trees, overlays, command line) in a
//!   small archive, unpacked into the spare system's folder on the boot
//!   partition.
//!
//! Every write is hashed as it goes and read back afterwards: what is on the
//! partition must be exactly what was built, or the update stops there.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::manifest::hex;

/// Patches compare systems in blocks this large.
pub const BLOCK: usize = 64 * 1024;
const DELTA_MAGIC: &[u8; 8] = b"CATDELTA";
const PACK_MAGIC: &[u8; 8] = b"CATBOOT1";

/// How far a long job has come, 0 to 1.
pub type Progress<'a> = &'a mut dyn FnMut(f32);

fn io(what: &str) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

/// The SHA-256 of the first `size` bytes of a file or partition.
pub fn hash_prefix(path: &Path, size: u64, progress: Progress) -> Result<String, String> {
    let mut f = File::open(path).map_err(io("reading the system"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut left = size;
    while left > 0 {
        let n = (left as usize).min(buf.len());
        f.read_exact(&mut buf[..n]).map_err(io("reading the system"))?;
        hasher.update(&buf[..n]);
        left -= n as u64;
        progress(1.0 - left as f32 / size.max(1) as f32);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(io("reading a download"))?.len();
    hash_prefix(path, size, &mut |_| {})
}

/// Written to the card in steps of this much, each flushed before the next:
/// a whole system held back in memory and written at once filled it and
/// left everything else waiting on the card (the handheld lagged while an
/// update installed).
const FLUSH_EVERY: usize = 4 << 20;

/// The spare system as it is written: each piece compared with what the
/// partition holds there already, and written only where it differs. Most
/// of a system is the same from one release to the next, and the rest of
/// its partition empty; the card reads quickly and writes slowly (most of
/// all at the idle priority an update has), and an update wrote the whole
/// 1 GB, about 15 minutes on the handheld.
struct Spare {
    file: File,
    at: u64,
    unflushed: usize,
    have: Vec<u8>,
    written: u64,
}

impl Spare {
    fn open(path: &Path) -> Result<Spare, String> {
        // A partition already exists; an image file (tests) may not.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(!path.starts_with("/dev"))
            .truncate(false)
            .open(path)
            .map_err(io("opening the spare system"))?;
        Ok(Spare { file, at: 0, unflushed: 0, have: Vec::new(), written: 0 })
    }

    /// The next piece of the system.
    fn put(&mut self, data: &[u8]) -> Result<(), String> {
        self.have.resize(data.len(), 0);
        let same = self.file.seek(SeekFrom::Start(self.at)).is_ok() && self.file.read_exact(&mut self.have).is_ok() && self.have == data;
        if !same {
            self.file.seek(SeekFrom::Start(self.at)).map_err(io("writing the spare system"))?;
            self.file.write_all(data).map_err(io("writing the spare system"))?;
            self.unflushed += data.len();
            self.written += data.len() as u64;
            if self.unflushed >= FLUSH_EVERY {
                self.file.sync_data().map_err(io("writing the spare system"))?;
                self.unflushed = 0;
            }
        }
        self.at += data.len() as u64;
        Ok(())
    }

    /// All of it on the card; how much had to be written.
    fn finish(self) -> Result<u64, String> {
        self.file.sync_all().map_err(io("writing the spare system"))?;
        Ok(self.written)
    }
}

/// Write a whole system: `gz` decompressed into `target`. Returns its hash.
pub fn write_full(gz: &Path, target: &Path, size: u64, progress: Progress) -> Result<String, String> {
    let mut input = GzDecoder::new(std::io::BufReader::new(File::open(gz).map_err(io("reading the download"))?));
    let mut out = Spare::open(target)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(io("unpacking the system"))?;
        if n == 0 {
            break;
        }
        out.put(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done as f32 / size.max(1) as f32);
    }
    let written = out.finish()?;
    eprintln!("catalyst-update: {} MB of {} MB written, the rest already there", written >> 20, done >> 20);
    if done != size {
        return Err(format!("the system came to {done} bytes, not {size}"));
    }
    Ok(hex(&hasher.finalize()))
}

/// Make a patch from `old` to `new` (image files of the same build's layout).
/// Returns how many blocks differ.
pub fn make_delta(old: &Path, new: &Path, out: &Path) -> Result<usize, String> {
    let (old_bytes, new_bytes) = (std::fs::read(old).map_err(io("reading the old system"))?, std::fs::read(new).map_err(io("reading the new system"))?);
    let blocks = new_bytes.len().div_ceil(BLOCK);
    let changed: Vec<u32> = (0..blocks)
        .filter(|&i| {
            let range = i * BLOCK..((i + 1) * BLOCK).min(new_bytes.len());
            old_bytes.get(range.clone()) != Some(&new_bytes[range])
        })
        .map(|i| i as u32)
        .collect();
    let mut f = File::create(out).map_err(io("writing the patch"))?;
    let mut header = Vec::new();
    header.extend_from_slice(DELTA_MAGIC);
    header.extend_from_slice(&(BLOCK as u32).to_le_bytes());
    header.extend_from_slice(&(new_bytes.len() as u64).to_le_bytes());
    header.extend_from_slice(&(old_bytes.len() as u64).to_le_bytes());
    header.extend_from_slice(&Sha256::digest(&old_bytes));
    header.extend_from_slice(&Sha256::digest(&new_bytes));
    header.extend_from_slice(&(changed.len() as u32).to_le_bytes());
    for i in &changed {
        header.extend_from_slice(&i.to_le_bytes());
    }
    f.write_all(&header).map_err(io("writing the patch"))?;
    let mut z = ZlibEncoder::new(f, Compression::best());
    for &i in &changed {
        let i = i as usize;
        z.write_all(&new_bytes[i * BLOCK..((i + 1) * BLOCK).min(new_bytes.len())]).map_err(io("writing the patch"))?;
    }
    z.finish().map_err(io("writing the patch"))?;
    Ok(changed.len())
}

fn take<const N: usize>(r: &mut impl Read) -> Result<[u8; N], String> {
    let mut b = [0u8; N];
    r.read_exact(&mut b).map_err(io("reading the patch"))?;
    Ok(b)
}

/// The system a patch was made from: its size and its hash.
pub fn delta_base(delta: &Path) -> Result<(u64, String), String> {
    let mut f = File::open(delta).map_err(io("reading the patch"))?;
    if &take::<8>(&mut f)? != DELTA_MAGIC {
        return Err("not a Catalyst patch".into());
    }
    let (_block, _size) = (take::<4>(&mut f)?, take::<8>(&mut f)?);
    let base_size = u64::from_le_bytes(take(&mut f)?);
    Ok((base_size, hex(&take::<32>(&mut f)?)))
}

/// Apply a patch: the running system `base` copied into `target`, with the
/// blocks that changed taken from the patch. Returns the result's hash.
pub fn write_delta(delta: &Path, base: &Path, target: &Path, progress: Progress) -> Result<String, String> {
    let mut f = std::io::BufReader::new(File::open(delta).map_err(io("reading the patch"))?);
    if &take::<8>(&mut f)? != DELTA_MAGIC {
        return Err("not a Catalyst patch".into());
    }
    let block = u32::from_le_bytes(take(&mut f)?) as usize;
    let size = u64::from_le_bytes(take(&mut f)?);
    let _base_size = u64::from_le_bytes(take(&mut f)?);
    let (_base_hash, _target_hash) = (take::<32>(&mut f)?, take::<32>(&mut f)?);
    let count = u32::from_le_bytes(take(&mut f)?) as usize;
    let mut changed = Vec::with_capacity(count);
    for _ in 0..count {
        changed.push(u32::from_le_bytes(take(&mut f)?) as usize);
    }
    let mut z = ZlibDecoder::new(f);
    let mut src = File::open(base).map_err(io("reading the running system"))?;
    let mut out = Spare::open(target)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; block];
    let blocks = (size as usize).div_ceil(block);
    let mut next = changed.iter().peekable();
    for i in 0..blocks {
        let n = (size as usize - i * block).min(block);
        if next.peek() == Some(&&i) {
            next.next();
            z.read_exact(&mut buf[..n]).map_err(io("unpacking the patch"))?;
        } else {
            src.seek(SeekFrom::Start((i * block) as u64)).map_err(io("reading the running system"))?;
            src.read_exact(&mut buf[..n]).map_err(io("reading the running system"))?;
        }
        out.put(&buf[..n])?;
        hasher.update(&buf[..n]);
        if i % 32 == 0 {
            progress(i as f32 / blocks as f32);
        }
    }
    let written = out.finish()?;
    eprintln!("catalyst-update: {} MB of {} MB written, the rest already there", written >> 20, size >> 20);
    progress(1.0);
    Ok(hex(&hasher.finalize()))
}

/// Pack a folder (the kernel, device trees, overlays, command line).
pub fn pack_dir(dir: &Path, out: &Path) -> Result<(), String> {
    let mut entries = Vec::new();
    fn walk(root: &Path, at: &Path, entries: &mut Vec<(String, std::path::PathBuf)>) -> std::io::Result<()> {
        let mut items: Vec<_> = std::fs::read_dir(at)?.collect::<Result<_, _>>()?;
        items.sort_by_key(|e| e.file_name());
        for e in items {
            let path = e.path();
            if e.file_type()?.is_dir() {
                walk(root, &path, entries)?;
            } else {
                let rel = path.strip_prefix(root).expect("under the root").to_string_lossy().replace('\\', "/");
                entries.push((rel, path));
            }
        }
        Ok(())
    }
    walk(dir, dir, &mut entries).map_err(io("reading the boot files"))?;
    let mut gz = GzEncoder::new(File::create(out).map_err(io("writing the boot pack"))?, Compression::best());
    gz.write_all(PACK_MAGIC).map_err(io("writing the boot pack"))?;
    for (name, path) in entries {
        let bytes = std::fs::read(&path).map_err(io("reading a boot file"))?;
        gz.write_all(&(name.len() as u16).to_le_bytes()).map_err(io("writing the boot pack"))?;
        gz.write_all(name.as_bytes()).map_err(io("writing the boot pack"))?;
        gz.write_all(&(bytes.len() as u64).to_le_bytes()).map_err(io("writing the boot pack"))?;
        gz.write_all(&bytes).map_err(io("writing the boot pack"))?;
    }
    gz.write_all(&0u16.to_le_bytes()).map_err(io("writing the boot pack"))?;
    gz.finish().map_err(io("writing the boot pack"))?;
    Ok(())
}

/// Unpack a boot pack into `dest` (emptied first). `edit` may change a
/// file's bytes on the way (the command line's root partition).
pub fn unpack(pack: &Path, dest: &Path, edit: &dyn Fn(&str, Vec<u8>) -> Vec<u8>) -> Result<usize, String> {
    let mut gz = GzDecoder::new(std::io::BufReader::new(File::open(pack).map_err(io("reading the boot pack"))?));
    if &take::<8>(&mut gz)? != PACK_MAGIC {
        return Err("not a Catalyst boot pack".into());
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(io("clearing the spare system's boot folder"))?;
    }
    std::fs::create_dir_all(dest).map_err(io("making the boot folder"))?;
    let mut files = 0;
    loop {
        let len = u16::from_le_bytes(take(&mut gz)?) as usize;
        if len == 0 {
            break;
        }
        let mut name = vec![0u8; len];
        gz.read_exact(&mut name).map_err(io("reading the boot pack"))?;
        let name = String::from_utf8(name).map_err(|_| "a boot file's name is not text")?;
        if name.starts_with('/') || name.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(format!("a boot file would land outside its folder: {name}"));
        }
        let size = u64::from_le_bytes(take(&mut gz)?) as usize;
        let mut bytes = vec![0u8; size];
        gz.read_exact(&mut bytes).map_err(io("reading the boot pack"))?;
        let path = dest.join(&name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io("making a boot folder"))?;
        }
        std::fs::write(&path, edit(&name, bytes)).map_err(io("writing a boot file"))?;
        files += 1;
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-update-pack-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A pretend system image: mostly zeros, some data, as an ext4 image is.
    fn system(seed: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        for (i, b) in v.iter_mut().enumerate().step_by(7) {
            *b = (i as u8).wrapping_mul(31).wrapping_add(seed);
        }
        v
    }

    #[test]
    fn a_patch_turns_the_old_system_into_the_new_one_exactly() {
        let dir = scratch("delta");
        let old = system(1, 20 * BLOCK + 123);
        let mut new = old.clone();
        // A few blocks change, one grows the image, the rest stay.
        new[5 * BLOCK + 9] ^= 0xff;
        new[17 * BLOCK..17 * BLOCK + 300].fill(7);
        new.extend(std::iter::repeat(3u8).take(BLOCK + 17));
        std::fs::write(dir.join("old.img"), &old).unwrap();
        std::fs::write(dir.join("new.img"), &new).unwrap();
        let changed = make_delta(&dir.join("old.img"), &dir.join("new.img"), &dir.join("p.cdelta")).unwrap();
        assert_eq!(changed, 4, "the two changed blocks, the old last block grown, and the new one after it");
        let patch_size = std::fs::metadata(dir.join("p.cdelta")).unwrap().len() as usize;
        assert!(patch_size < new.len() / 4, "a patch is small: {patch_size}");
        let hash = write_delta(&dir.join("p.cdelta"), &dir.join("old.img"), &dir.join("out.img"), &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("out.img")).unwrap(), new);
        assert_eq!(hash, hex(&Sha256::digest(&new)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_what_the_spare_system_does_not_hold_already_is_written() {
        let dir = scratch("spare");
        let old = system(1, 8 * BLOCK);
        let mut new = old.clone();
        new[3 * BLOCK + 5] ^= 0xff;
        // The spare partition holds the old system: one block to write.
        std::fs::write(dir.join("spare.img"), &old).unwrap();
        let mut spare = Spare::open(&dir.join("spare.img")).unwrap();
        for block in new.chunks(BLOCK) {
            spare.put(block).unwrap();
        }
        assert_eq!(spare.finish().unwrap(), BLOCK as u64);
        assert_eq!(std::fs::read(dir.join("spare.img")).unwrap(), new);
        // Again: nothing to write. And into a new, empty image: all of it.
        let mut spare = Spare::open(&dir.join("spare.img")).unwrap();
        new.chunks(BLOCK).for_each(|b| spare.put(b).unwrap());
        assert_eq!(spare.finish().unwrap(), 0);
        let mut fresh = Spare::open(&dir.join("fresh.img")).unwrap();
        new.chunks(BLOCK).for_each(|b| fresh.put(b).unwrap());
        assert_eq!(fresh.finish().unwrap(), new.len() as u64);
        assert_eq!(std::fs::read(dir.join("fresh.img")).unwrap(), new);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_whole_system_writes_and_hashes() {
        let dir = scratch("full");
        let img = system(9, 3 * BLOCK + 5);
        let mut gz = GzEncoder::new(File::create(dir.join("s.img.gz")).unwrap(), Compression::fast());
        gz.write_all(&img).unwrap();
        gz.finish().unwrap();
        let hash = write_full(&dir.join("s.img.gz"), &dir.join("out.img"), img.len() as u64, &mut |_| {}).unwrap();
        assert_eq!(hash, hex(&Sha256::digest(&img)));
        assert_eq!(hash_prefix(&dir.join("out.img"), img.len() as u64, &mut |_| {}).unwrap(), hash);
        // The wrong length is caught.
        assert!(write_full(&dir.join("s.img.gz"), &dir.join("out2.img"), img.len() as u64 + 1, &mut |_| {}).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn boot_files_pack_and_unpack_and_cannot_escape() {
        let dir = scratch("boot");
        std::fs::create_dir_all(dir.join("in/overlays")).unwrap();
        std::fs::write(dir.join("in/Image"), b"kernel").unwrap();
        std::fs::write(dir.join("in/cmdline.txt"), b"root=/dev/mmcblk0p2 ro").unwrap();
        std::fs::write(dir.join("in/overlays/dwc2.dtbo"), b"overlay").unwrap();
        pack_dir(&dir.join("in"), &dir.join("boot.pack")).unwrap();
        // Something left over from before is cleared away.
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("out/stale"), b"old").unwrap();
        let edit = |name: &str, bytes: Vec<u8>| if name == "cmdline.txt" { String::from_utf8(bytes).unwrap().replace("p2", "p3").into_bytes() } else { bytes };
        assert_eq!(unpack(&dir.join("boot.pack"), &dir.join("out"), &edit).unwrap(), 3);
        assert_eq!(std::fs::read(dir.join("out/overlays/dwc2.dtbo")).unwrap(), b"overlay");
        assert_eq!(std::fs::read_to_string(dir.join("out/cmdline.txt")).unwrap(), "root=/dev/mmcblk0p3 ro");
        assert!(!dir.join("out/stale").exists());
        // A pack naming ../ is refused.
        let mut gz = GzEncoder::new(File::create(dir.join("evil.pack")).unwrap(), Compression::fast());
        gz.write_all(PACK_MAGIC).unwrap();
        gz.write_all(&8u16.to_le_bytes()).unwrap();
        gz.write_all(b"../x.txt").unwrap();
        gz.write_all(&1u64.to_le_bytes()).unwrap();
        gz.write_all(b"x").unwrap();
        gz.write_all(&0u16.to_le_bytes()).unwrap();
        gz.finish().unwrap();
        assert!(unpack(&dir.join("evil.pack"), &dir.join("out2"), &|_, b| b).is_err());
        assert!(!dir.join("x.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
