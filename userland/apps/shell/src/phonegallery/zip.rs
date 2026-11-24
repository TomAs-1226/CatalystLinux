//! Photos as one ZIP, streamed as it is sent and never built in memory.
//!
//! Stored, not compressed: a JPEG does not get smaller. Every size is known
//! before the first byte, so the archive's length is too, and a phone shows
//! how far its download has got. Each file is read twice, once for its
//! CRC-32 just before its header goes and once as it is sent, so every
//! header carries its real sizes and CRC: no data descriptors, which some
//! unzippers refuse on stored entries. ZIP64 records are added only where
//! an archive passes 4 GiB or 65,535 files.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDateTime, Timelike};

/// What the classic records can hold; past it, ZIP64's.
const MOST_32: u64 = 0xFFFF_FFFF;
const MOST_16: usize = 0xFFFF;
/// How much is read and sent at a time.
pub const CHUNK: usize = 64 * 1024;

/// A file to put in the archive.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Its name in the archive.
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// When it was taken: the archive's (local) time for it.
    pub when: NaiveDateTime,
}

/// An archive laid out before any of it is sent.
#[derive(Debug)]
pub struct Plan {
    entries: Vec<Entry>,
    /// Where each entry's local header starts.
    offsets: Vec<u64>,
    cd_offset: u64,
    cd_size: u64,
    zip64_end: bool,
    /// Every entry with a ZIP64 offset (the tests, on a small archive).
    all_64: bool,
}

impl Plan {
    /// The archive of `entries`, in their order. A file of 4 GiB or more is
    /// left out (a photo or a loop never is one).
    pub fn new(entries: Vec<Entry>) -> Plan {
        Self::laid_out(entries, false)
    }

    /// As `new`, with ZIP64 records throughout, as a very large archive has.
    #[cfg(test)]
    pub fn zip64(entries: Vec<Entry>) -> Plan {
        Self::laid_out(entries, true)
    }

    fn laid_out(entries: Vec<Entry>, all_64: bool) -> Plan {
        let entries: Vec<Entry> = entries.into_iter().filter(|e| e.size < MOST_32).collect();
        let mut offsets = Vec::with_capacity(entries.len());
        let mut at = 0u64;
        for e in &entries {
            offsets.push(at);
            at += 30 + e.name.len() as u64 + e.size;
        }
        let cd_offset = at;
        let cd_size: u64 = entries.iter().zip(&offsets).map(|(e, o)| 46 + e.name.len() as u64 + if all_64 || *o >= MOST_32 { 12 } else { 0 }).sum();
        let zip64_end = all_64 || entries.len() >= MOST_16 || cd_offset >= MOST_32 || cd_size >= MOST_32 || offsets.iter().any(|o| *o >= MOST_32);
        Plan { entries, offsets, cd_offset, cd_size, zip64_end, all_64 }
    }

    /// Its length in bytes, as it will be sent.
    pub fn len(&self) -> u64 {
        self.cd_offset + self.cd_size + if self.zip64_end { 56 + 20 } else { 0 } + 22
    }

    fn entry_64(&self, i: usize) -> bool {
        self.all_64 || self.offsets[i] >= MOST_32
    }

    /// Send it: every header, every file read as it goes, then the central
    /// directory. Ends early, with an error, when `stop` is set, or when a
    /// file is not as it was when the archive was laid out (its length
    /// was promised already).
    pub fn write(&self, out: &mut impl Write, stop: &AtomicBool) -> std::io::Result<u64> {
        let mut sent = 0u64;
        let mut crcs = Vec::with_capacity(self.entries.len());
        let mut buf = vec![0u8; CHUNK];
        for e in &self.entries {
            let mut file = std::fs::File::open(&e.path)?;
            if file.metadata()?.len() != e.size {
                return Err(changed());
            }
            // Its CRC, read through once.
            let mut crc = crc32fast::Hasher::new();
            let mut left = e.size;
            while left > 0 {
                halt_if(stop)?;
                let n = file.read(&mut buf[..(left.min(CHUNK as u64) as usize)])?;
                if n == 0 {
                    return Err(changed());
                }
                crc.update(&buf[..n]);
                left -= n as u64;
            }
            let crc = crc.finalize();
            crcs.push(crc);
            let head = local_header(e, crc);
            out.write_all(&head)?;
            sent += head.len() as u64;
            // And again as it is sent.
            file.seek(SeekFrom::Start(0))?;
            let mut left = e.size;
            while left > 0 {
                halt_if(stop)?;
                let n = file.read(&mut buf[..(left.min(CHUNK as u64) as usize)])?;
                if n == 0 {
                    return Err(changed());
                }
                out.write_all(&buf[..n])?;
                left -= n as u64;
                sent += n as u64;
            }
        }
        let mut tail = Vec::with_capacity(self.cd_size as usize + 100);
        for (i, e) in self.entries.iter().enumerate() {
            tail.extend_from_slice(&central_header(e, crcs[i], self.offsets[i], self.entry_64(i)));
        }
        let count = self.entries.len() as u64;
        if self.zip64_end {
            let end64_at = self.cd_offset + self.cd_size;
            // The ZIP64 end of central directory record…
            put32(&mut tail, 0x0606_4b50);
            put64(&mut tail, 44);
            put16(&mut tail, MADE_BY_64);
            put16(&mut tail, 45);
            put32(&mut tail, 0);
            put32(&mut tail, 0);
            put64(&mut tail, count);
            put64(&mut tail, count);
            put64(&mut tail, self.cd_size);
            put64(&mut tail, self.cd_offset);
            // …and where to find it.
            put32(&mut tail, 0x0706_4b50);
            put32(&mut tail, 0);
            put64(&mut tail, end64_at);
            put32(&mut tail, 1);
        }
        put32(&mut tail, 0x0605_4b50);
        put16(&mut tail, 0);
        put16(&mut tail, 0);
        let n16 = if self.zip64_end { 0xFFFF } else { count as u16 };
        put16(&mut tail, n16);
        put16(&mut tail, n16);
        put32(&mut tail, if self.zip64_end { 0xFFFF_FFFF } else { self.cd_size as u32 });
        put32(&mut tail, if self.zip64_end { 0xFFFF_FFFF } else { self.cd_offset as u32 });
        put16(&mut tail, 0);
        out.write_all(&tail)?;
        sent += tail.len() as u64;
        out.flush()?;
        Ok(sent)
    }
}

/// Unix, ZIP 2.0 (stored); with ZIP64 records, 4.5.
const MADE_BY: u16 = 0x0314;
const MADE_BY_64: u16 = 0x032D;

fn changed() -> std::io::Error {
    std::io::Error::other("a photo changed while it was being sent")
}

fn halt_if(stop: &AtomicBool) -> std::io::Result<()> {
    if stop.load(Ordering::Relaxed) {
        Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "sharing stopped"))
    } else {
        Ok(())
    }
}

fn put16(v: &mut Vec<u8>, n: u16) {
    v.extend_from_slice(&n.to_le_bytes());
}
fn put32(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_le_bytes());
}
fn put64(v: &mut Vec<u8>, n: u64) {
    v.extend_from_slice(&n.to_le_bytes());
}

/// MS-DOS time and date, as ZIP keeps them: two-second steps, 1980 to 2107.
pub fn dos_time(at: NaiveDateTime) -> (u16, u16) {
    let year = at.year().clamp(1980, 2107) as u16;
    let time = ((at.hour() as u16) << 11) | ((at.minute() as u16) << 5) | (at.second() as u16 / 2);
    let date = ((year - 1980) << 9) | ((at.month() as u16) << 5) | at.day() as u16;
    (time, date)
}

/// Bit 11: the name is UTF-8 (set only when it needs to be).
fn flags(name: &str) -> u16 {
    if name.is_ascii() {
        0
    } else {
        1 << 11
    }
}

fn local_header(e: &Entry, crc: u32) -> Vec<u8> {
    let (time, date) = dos_time(e.when);
    let mut v = Vec::with_capacity(30 + e.name.len());
    put32(&mut v, 0x0403_4b50);
    put16(&mut v, 10);
    put16(&mut v, flags(&e.name));
    put16(&mut v, 0);
    put16(&mut v, time);
    put16(&mut v, date);
    put32(&mut v, crc);
    put32(&mut v, e.size as u32);
    put32(&mut v, e.size as u32);
    put16(&mut v, e.name.len() as u16);
    put16(&mut v, 0);
    v.extend_from_slice(e.name.as_bytes());
    v
}

fn central_header(e: &Entry, crc: u32, offset: u64, zip64: bool) -> Vec<u8> {
    let (time, date) = dos_time(e.when);
    let mut v = Vec::with_capacity(46 + e.name.len() + 12);
    put32(&mut v, 0x0201_4b50);
    put16(&mut v, if zip64 { MADE_BY_64 } else { MADE_BY });
    put16(&mut v, if zip64 { 45 } else { 10 });
    put16(&mut v, flags(&e.name));
    put16(&mut v, 0);
    put16(&mut v, time);
    put16(&mut v, date);
    put32(&mut v, crc);
    put32(&mut v, e.size as u32);
    put32(&mut v, e.size as u32);
    put16(&mut v, e.name.len() as u16);
    put16(&mut v, if zip64 { 12 } else { 0 });
    put16(&mut v, 0);
    put16(&mut v, 0);
    put16(&mut v, 0);
    // A regular file, rw-r--r--.
    put32(&mut v, 0o100644 << 16);
    put32(&mut v, if zip64 { 0xFFFF_FFFF } else { offset as u32 });
    v.extend_from_slice(e.name.as_bytes());
    if zip64 {
        // The ZIP64 extra field: only the offset, the one field marked so.
        put16(&mut v, 0x0001);
        put16(&mut v, 8);
        put64(&mut v, offset);
    }
    v
}
