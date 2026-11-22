//! Backups as a zip on this computer: the handheld sends /data (caches and
//! downloaded updates left out) as a tar stream, kept here as a zip any
//! computer opens; a restore sends a zip back as a tar stream.

use std::io::{self, Read, Seek, Write};
use std::path::Path;

use catalyst_companion_proto::tar;
use zip::write::SimpleFileOptions;

/// Files already compressed are stored as they are.
fn stored(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    ["mp3", "m4a", "aac", "flac", "opus", "ogg", "jpg", "jpeg", "png", "gz", "zip", "webm", "mka", "gguf"].contains(&ext.as_str())
}

/// Days since 1970 to a date (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

fn zip_time(unix: u64) -> zip::DateTime {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil(days);
    zip::DateTime::from_date_and_time(y.clamp(1980, 2107) as u16, m, d, (secs / 3600) as u8, (secs / 60 % 60) as u8, (secs % 60) as u8).unwrap_or_default()
}

/// A backup's tar stream into a zip at `to`; how many files it holds.
pub fn tar_to_zip(from: impl Read, to: &Path) -> Result<usize, String> {
    let part = to.with_extension("zip.part");
    let made = (|| -> Result<usize, String> {
        let file = std::fs::File::create(&part).map_err(|e| format!("could not write the backup: {e}"))?;
        let mut zip = zip::ZipWriter::new(io::BufWriter::new(file));
        let mut reader = tar::Reader::new(from);
        let mut files = 0;
        while let Some(e) = reader.next().map_err(|e| format!("the backup stopped coming: {e}"))? {
            let options = SimpleFileOptions::default().unix_permissions(e.mode).last_modified_time(zip_time(e.mtime)).large_file(e.size >= 1 << 32);
            if e.dir {
                zip.add_directory(format!("{}/", e.path), options).map_err(|e| e.to_string())?;
                continue;
            }
            let method = if stored(&e.path) { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
            zip.start_file(e.path.clone(), options.compression_method(method)).map_err(|e| e.to_string())?;
            io::copy(&mut reader.body(), &mut zip).map_err(|e| format!("the backup stopped coming: {e}"))?;
            files += 1;
        }
        zip.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
        Ok(files)
    })();
    match made {
        Ok(n) => {
            std::fs::rename(&part, to).map_err(|e| format!("could not keep the backup: {e}"))?;
            Ok(n)
        }
        Err(why) => {
            std::fs::remove_file(&part).ok();
            Err(why)
        }
    }
}

/// What a zip holds, as the tar stream will carry it (only safe paths).
fn entries<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Vec<(usize, tar::Entry)> {
    (0..zip.len())
        .filter_map(|i| {
            let f = zip.by_index(i).ok()?;
            let path = f.name().trim_end_matches('/').to_owned();
            tar::safe_path(&path)?;
            let dir = f.is_dir();
            let mode = f.unix_mode().map_or(if dir { 0o755 } else { 0o644 }, |m| m & 0o777);
            Some((i, tar::Entry { path, mode, size: if dir { 0 } else { f.size() }, mtime: 0, dir }))
        })
        .collect()
}

/// A zip's backup: its tar stream's length, and the stream itself written.
pub struct Restore {
    zip: zip::ZipArchive<io::BufReader<std::fs::File>>,
    list: Vec<(usize, tar::Entry)>,
    pub len: u64,
    pub files: usize,
}

impl Restore {
    pub fn open(path: &Path) -> Result<Restore, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|_| "that is not a zip".to_owned())?;
        let list = entries(&mut zip);
        let looks_like_one = list.iter().any(|(_, e)| e.path.starts_with("catalyst/") || e.path == "catalyst" || e.path.starts_with("music/"));
        if !looks_like_one {
            return Err("that zip is not a Catalyst backup".into());
        }
        let len = list.iter().map(|(_, e)| tar::entry_len(&e.path, e.size, e.dir)).sum::<u64>() + tar::END.len() as u64;
        let files = list.iter().filter(|(_, e)| !e.dir).count();
        Ok(Restore { zip, list, len, files })
    }
}

/// A backup's tar stream, made from its zip an entry at a time as the
/// connection takes it: exactly `Restore::len` bytes.
pub struct Stream {
    restore: Restore,
    at: usize,
    buf: Vec<u8>,
    pos: usize,
    ended: bool,
}

impl Stream {
    pub fn new(restore: Restore) -> Stream {
        Stream { restore, at: 0, buf: Vec::new(), pos: 0, ended: false }
    }
}

impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            if self.ended {
                return Ok(0);
            }
            self.buf.clear();
            self.pos = 0;
            match self.restore.list.get(self.at) {
                Some((i, e)) => {
                    let e = e.clone();
                    let mut f = self.restore.zip.by_index(*i).map_err(io::Error::other)?;
                    tar::write_entry(&mut self.buf, &e, &mut f)?;
                    self.at += 1;
                }
                None => {
                    self.buf.extend_from_slice(&tar::END);
                    self.ended = true;
                }
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tar_stream_becomes_a_zip_and_goes_back_as_the_same_stream() {
        let dir = std::env::temp_dir().join(format!("companion-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut stream = Vec::new();
        let files = [("catalyst", None), ("catalyst/settings", Some(&b"accent=rose\n"[..])), ("keys", Some(&b"tba=x\n"[..])), ("music/library/A/01 Song.mp3", Some(&[7u8; 3000][..]))];
        for (path, body) in files {
            let e = tar::Entry { path: path.into(), mode: if path == "keys" { 0o600 } else { 0o644 }, size: body.map_or(0, |b| b.len() as u64), mtime: 1_763_800_000, dir: body.is_none() };
            tar::write_entry(&mut stream, &e, &mut body.unwrap_or_default()).unwrap();
        }
        stream.extend_from_slice(&tar::END);
        let zip_path = dir.join("backup.zip");
        assert_eq!(tar_to_zip(&stream[..], &zip_path).unwrap(), 3);
        let restore = Restore::open(&zip_path).unwrap();
        let len = restore.len;
        let mut again = Vec::new();
        Stream::new(restore).read_to_end(&mut again).unwrap();
        assert_eq!(again.len() as u64, len, "the length said beforehand is the stream's");
        let mut r = tar::Reader::new(&again[..]);
        let mut seen = Vec::new();
        while let Some(e) = r.next().unwrap() {
            let mut body = Vec::new();
            r.body().read_to_end(&mut body).unwrap();
            seen.push((e.path, e.mode, body.len()));
        }
        assert!(seen.contains(&("keys".into(), 0o600, 6)), "a secret stays readable by root alone: {seen:?}");
        assert!(seen.contains(&("music/library/A/01 Song.mp3".into(), 0o644, 3000)));
        assert_eq!(zip_time(1_763_800_000), zip::DateTime::from_date_and_time(2025, 11, 22, 8, 26, 40).unwrap());
        std::fs::write(dir.join("other.zip"), b"PK no").unwrap();
        assert!(Restore::open(&dir.join("other.zip")).is_err());
    }
}
