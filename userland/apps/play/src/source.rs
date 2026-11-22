//! A song file that may still be downloading: reading catches up with what
//! is on the card and waits there (never taking the end of what has come
//! so far for the end of the song), as long as the size it will have is
//! known. Seeking works anywhere; reading past what has come waits for it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use symphonia::core::io::MediaSource;

/// How often to look while waiting: only while the download is behind,
/// which is rare (it runs a hundred times faster than the song plays).
const LOOK_EVERY: Duration = Duration::from_millis(100);
/// Said as `buffering` once waiting this long.
const BUFFERING_AFTER: Duration = Duration::from_millis(400);
/// Given up on after this long with nothing new.
const GIVE_UP_AFTER: Duration = Duration::from_secs(600);

pub struct Follow {
    file: File,
    pos: u64,
    /// The size it will be once downloaded; `None`: it is all here.
    total: Option<u64>,
    buffering: bool,
}

impl Follow {
    pub fn open(path: &Path, total: Option<u64>) -> std::io::Result<Follow> {
        Ok(Follow { file: File::open(path)?, pos: 0, total, buffering: false })
    }

    fn len_now(&self) -> u64 {
        self.file.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

impl Read for Follow {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let started = Instant::now();
        loop {
            let n = self.file.read(buf)?;
            if n > 0 {
                self.pos += n as u64;
                if self.buffering {
                    self.buffering = false;
                    crate::say("playing");
                }
                return Ok(n);
            }
            match self.total {
                Some(total) if self.pos < total => {
                    let waited = started.elapsed();
                    if waited >= GIVE_UP_AFTER {
                        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the download stopped"));
                    }
                    if waited >= BUFFERING_AFTER && !self.buffering {
                        self.buffering = true;
                        crate::say("buffering");
                    }
                    std::thread::sleep(LOOK_EVERY);
                    // The file may have been replaced meanwhile only by a
                    // rename of this same one: the handle still reads it.
                    let _ = self.len_now();
                }
                _ => return Ok(0),
            }
        }
    }
}

impl Seek for Follow {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let to = match (to, self.total) {
            // From the end it will have, not the end it has so far.
            (SeekFrom::End(back), Some(total)) => SeekFrom::Start((total as i64 + back).max(0) as u64),
            (other, _) => other,
        };
        self.pos = self.file.seek(to)?;
        Ok(self.pos)
    }
}

impl MediaSource for Follow {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.total.unwrap_or_else(|| self.len_now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reading_waits_for_a_download_and_ends_at_its_size() {
        let dir = std::env::temp_dir().join(format!("catalyst-play-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.webm.part");
        std::fs::write(&path, b"abc").unwrap();
        let mut f = Follow::open(&path, Some(6)).unwrap();
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"def").unwrap();
            })
        };
        let mut all = Vec::new();
        f.read_to_end(&mut all).unwrap();
        writer.join().unwrap();
        assert_eq!(all, b"abcdef", "the end so far is not the end");
        assert_eq!(f.byte_len(), Some(6));
        assert_eq!(f.seek(SeekFrom::End(-2)).unwrap(), 4, "from the end it will have");
        // All here: the end is the end.
        let mut whole = Follow::open(&path, None).unwrap();
        let mut all = Vec::new();
        whole.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 6);
        std::fs::remove_dir_all(&dir).ok();
    }
}
