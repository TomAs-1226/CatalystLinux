//! The backup stream: plain tar (ustar headers, a GNU long-name entry for a
//! path past 100 bytes), so any tar tool reads it too. Files and folders
//! only: links and devices are neither written nor taken.

use std::io::{self, Read, Write};

const BLOCK: usize = 512;
const LONG_NAME: &str = "././@LongLink";

/// One file or folder in the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its place, `/` between folders, never starting with `/`.
    pub path: String,
    /// Permission bits (0o600 for a secret).
    pub mode: u32,
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: u64,
    pub dir: bool,
}

fn padding(size: u64) -> usize {
    (BLOCK - (size % BLOCK as u64) as usize) % BLOCK
}

fn octal(field: &mut [u8], value: u64) {
    let text = format!("{value:0width$o}", width = field.len() - 1);
    let text = &text.as_bytes()[text.len() - (field.len() - 1)..];
    field[..text.len()].copy_from_slice(text);
    field[field.len() - 1] = 0;
}

fn header(name: &str, mode: u32, size: u64, mtime: u64, kind: u8) -> [u8; BLOCK] {
    let mut h = [0u8; BLOCK];
    let bytes = name.as_bytes();
    let n = bytes.len().min(100);
    h[..n].copy_from_slice(&bytes[..n]);
    octal(&mut h[100..108], u64::from(mode & 0o7777));
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], mtime);
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    octal(&mut h[148..155], u64::from(sum));
    h[155] = b' ';
    h
}

/// The bytes an entry takes in the stream, its contents and padding with it.
pub fn entry_len(path: &str, size: u64, dir: bool) -> u64 {
    let name_len = path.len() + usize::from(dir);
    let long = if name_len > 100 { (BLOCK + name_len + 1 + padding(name_len as u64 + 1)) as u64 } else { 0 };
    let body = if dir { 0 } else { size + padding(size) as u64 };
    long + BLOCK as u64 + body
}

/// The end of the stream: two empty blocks.
pub const END: [u8; 2 * BLOCK] = [0; 2 * BLOCK];

/// An entry's header (and long name, if it needs one), then exactly
/// `entry.size` bytes from `body` and the padding after them.
pub fn write_entry(out: &mut dyn Write, entry: &Entry, body: &mut dyn Read) -> io::Result<()> {
    let name = if entry.dir { format!("{}/", entry.path.trim_end_matches('/')) } else { entry.path.clone() };
    if name.len() > 100 {
        let long = name.len() as u64 + 1;
        out.write_all(&header(LONG_NAME, 0o644, long, 0, b'L'))?;
        out.write_all(name.as_bytes())?;
        out.write_all(&[0])?;
        out.write_all(&[0u8; BLOCK][..padding(long)])?;
    }
    let size = if entry.dir { 0 } else { entry.size };
    if size >= 1 << 33 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is 8 GB or more: too large for a backup", entry.path)));
    }
    out.write_all(&header(&name, entry.mode, size, entry.mtime, if entry.dir { b'5' } else { b'0' }))?;
    if size > 0 {
        let copied = io::copy(&mut body.take(size), out)?;
        if copied != size {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("{} changed while it was read", entry.path)));
        }
        out.write_all(&[0u8; BLOCK][..padding(size)])?;
    }
    Ok(())
}

fn parse_octal(field: &[u8]) -> Option<u64> {
    let text: String = field.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

/// Reads a tar stream an entry at a time: [`Reader::next`], then that
/// entry's contents from [`Reader::body`] (or not: the next `next` passes
/// over whatever was left).
pub struct Reader<R: Read> {
    inner: R,
    /// Of the current entry: contents not yet read, and the padding after.
    left: u64,
    pad: usize,
}

impl<R: Read> Reader<R> {
    pub fn new(inner: R) -> Self {
        Reader { inner, left: 0, pad: 0 }
    }

    fn skip(&mut self, n: u64) -> io::Result<()> {
        let got = io::copy(&mut (&mut self.inner).take(n), &mut io::sink())?;
        if got != n {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the stream ended inside a file"));
        }
        Ok(())
    }

    /// The next file or folder (links, devices and the like are passed
    /// over), or `None` at the end.
    pub fn next(&mut self) -> io::Result<Option<Entry>> {
        let rest = self.left + self.pad as u64;
        self.skip(rest)?;
        self.left = 0;
        self.pad = 0;
        let mut long: Option<String> = None;
        loop {
            let mut h = [0u8; BLOCK];
            match self.inner.read_exact(&mut h) {
                Ok(()) => {}
                // A stream that just stops where an entry would start has ended.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e),
            }
            if h.iter().all(|&b| b == 0) {
                return Ok(None);
            }
            let stored = parse_octal(&h[148..156]).ok_or_else(|| bad("a header's checksum is not a number"))?;
            let mut copy = h;
            copy[148..156].copy_from_slice(b"        ");
            if copy.iter().map(|&b| u64::from(b)).sum::<u64>() != stored {
                return Err(bad("a header's checksum is wrong: not a backup, or a damaged one"));
            }
            let size = parse_octal(&h[124..136]).ok_or_else(|| bad("a size is not a number"))?;
            let kind = h[156];
            if kind == b'L' {
                if size > 4096 {
                    return Err(bad("a name longer than any file's"));
                }
                let mut name = vec![0u8; size as usize];
                self.inner.read_exact(&mut name)?;
                self.skip(padding(size) as u64)?;
                long = Some(String::from_utf8_lossy(&name).trim_end_matches('\0').to_owned());
                continue;
            }
            let field = |r: std::ops::Range<usize>| String::from_utf8_lossy(&h[r]).trim_end_matches('\0').to_owned();
            let name = match long.take() {
                Some(name) => name,
                None => {
                    let (prefix, name) = (field(345..500), field(0..100));
                    if prefix.is_empty() {
                        name
                    } else {
                        format!("{prefix}/{name}")
                    }
                }
            };
            let mode = parse_octal(&h[100..108]).unwrap_or(0o644) as u32;
            let mtime = parse_octal(&h[136..148]).unwrap_or(0);
            let dir = kind == b'5' || name.ends_with('/');
            let file = kind == b'0' || kind == 0 || kind == b'7';
            let body = if dir { 0 } else { size };
            self.left = body;
            self.pad = padding(body);
            if dir || file {
                return Ok(Some(Entry { path: name.trim_start_matches("./").trim_end_matches('/').to_owned(), mode, size: body, mtime, dir }));
            }
            // A link or the like: its contents (if any) passed over.
            self.skip(self.left + self.pad as u64)?;
            self.left = 0;
            self.pad = 0;
        }
    }

    /// What is left of the current entry's contents.
    pub fn body(&mut self) -> Body<'_, R> {
        Body { reader: self }
    }
}

/// The current entry's contents, read through.
pub struct Body<'a, R: Read> {
    reader: &'a mut Reader<R>,
}

impl<R: Read> Read for Body<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.reader.left;
        if left == 0 {
            return Ok(0);
        }
        let want = buf.len().min(left.min(usize::MAX as u64) as usize);
        let n = self.reader.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the stream ended inside a file"));
        }
        self.reader.left -= n as u64;
        Ok(n)
    }
}

/// A path from a stream that is safe to put under a folder: relative, with
/// no `..`, no empty or `.` parts, no backslashes or NULs.
pub fn safe_path(path: &str) -> Option<Vec<&str>> {
    if path.is_empty() || path.starts_with('/') || path.contains(['\\', '\0']) {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts.iter().all(|p| !p.is_empty() && *p != "." && *p != "..").then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, body: &[u8], mode: u32) -> Entry {
        Entry { path: path.into(), mode, size: body.len() as u64, mtime: 1_700_000_000, dir: false }
    }

    #[test]
    fn files_and_folders_go_there_and_back_long_names_too() {
        let long = format!("music/library/{}/{}/01 {}.flac", "A".repeat(60), "B".repeat(60), "C".repeat(40));
        let files: Vec<(Entry, Vec<u8>)> = vec![
            (Entry { path: "catalyst".into(), mode: 0o755, size: 0, mtime: 5, dir: true }, vec![]),
            (entry("catalyst/settings", b"accent=rose\n", 0o644), b"accent=rose\n".to_vec()),
            (entry("keys", b"tba=x\n", 0o600), b"tba=x\n".to_vec()),
            (entry(&long, &[9u8; 1500], 0o644), vec![9u8; 1500]),
            (entry("empty", b"", 0o644), vec![]),
        ];
        let mut out = Vec::new();
        let mut expected = 0;
        for (e, body) in &files {
            write_entry(&mut out, e, &mut &body[..]).unwrap();
            expected += entry_len(&e.path, e.size, e.dir);
        }
        out.extend_from_slice(&END);
        assert_eq!(out.len() as u64, expected + END.len() as u64, "entry_len says what the stream takes");
        let mut r = Reader::new(&out[..]);
        for (e, body) in &files {
            let got = r.next().unwrap().expect("an entry");
            assert_eq!(&got, e);
            let mut read = Vec::new();
            r.body().read_to_end(&mut read).unwrap();
            assert_eq!(&read, body);
        }
        assert_eq!(r.next().unwrap(), None);
    }

    #[test]
    fn contents_left_unread_are_passed_over() {
        let mut out = Vec::new();
        write_entry(&mut out, &entry("a", &[1u8; 700], 0o644), &mut &[1u8; 700][..]).unwrap();
        write_entry(&mut out, &entry("b", b"bee", 0o644), &mut &b"bee"[..]).unwrap();
        out.extend_from_slice(&END);
        let mut r = Reader::new(&out[..]);
        assert_eq!(r.next().unwrap().unwrap().path, "a");
        assert_eq!(r.next().unwrap().unwrap().path, "b");
    }

    #[test]
    fn a_damaged_header_or_a_path_out_of_the_folder_is_refused() {
        let mut out = Vec::new();
        write_entry(&mut out, &entry("a", b"x", 0o644), &mut &b"x"[..]).unwrap();
        out[3] ^= 0x55;
        assert!(Reader::new(&out[..]).next().is_err());
        assert!(safe_path("../etc/passwd").is_none());
        assert!(safe_path("/etc/passwd").is_none());
        assert!(safe_path("a/./b").is_none());
        assert!(safe_path("a\\b").is_none());
        assert_eq!(safe_path("music/library/x.mp3"), Some(vec!["music", "library", "x.mp3"]));
    }
}
