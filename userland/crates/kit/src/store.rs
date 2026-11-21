//! What an app keeps: its own folder on `/data`, outside the app's folder,
//! so an update (which replaces the app's folder whole) or a reinstall
//! keeps it; and a small settings file of `key = value` lines, written so
//! a power cut never leaves half of it.

use std::path::{Path, PathBuf};

/// The app's own folder: `CATALYST_APP_DATA` (the shell sets it to
/// `/data/catalyst/app-data/<id>`), else one named after the program, in
/// `/data` on the handheld or the user's folder on a computer. Made here.
pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("CATALYST_APP_DATA") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let id = std::env::var("CATALYST_APP_ID").ok().filter(|id| !id.is_empty()).unwrap_or_else(|| {
                std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "app".into())
            });
            let base = if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
                PathBuf::from("/data/catalyst/app-data")
            } else if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                PathBuf::from(local).join("CatalystLinux").join("app-data")
            } else {
                std::env::temp_dir().join("catalyst-app-data")
            };
            base.join(id)
        }
    };
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Write a file whole or not at all: to a temporary file beside it, synced,
/// then renamed over it.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("new");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// A few values kept between runs, as `key = value` lines (the form of the
/// shell's own settings and of an app's manifest).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    path: PathBuf,
    pairs: Vec<(String, String)>,
}

impl Saved {
    /// `name` in the app's folder (`settings`, `scores`), read now; a file
    /// that is not there yet is empty.
    pub fn open(name: &str) -> Saved {
        Saved::at(data_dir().join(name))
    }

    pub fn at(path: PathBuf) -> Saved {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let pairs = text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).filter(|(k, _)| !k.is_empty()).collect();
        Saved { path, pairs }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn number<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key)?.parse().ok()
    }

    /// Set a value (one line, so line breaks become spaces).
    pub fn set(&mut self, key: &str, value: impl ToString) {
        let value = value.to_string().replace(['\n', '\r'], " ");
        match self.pairs.iter_mut().find(|(k, _)| k == key) {
            Some(pair) => pair.1 = value,
            None => self.pairs.push((key.to_owned(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.pairs.retain(|(k, _)| k != key);
    }

    /// Write it out, whole or not at all.
    pub fn save(&self) -> std::io::Result<()> {
        let text: String = self.pairs.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        write_atomic(&self.path, text.as_bytes())
    }
}

/// A small, fast random number source (xorshift64*), for games. Seeded
/// from the clock, or from `CATALYST_SEED` when it is set (the gallery sets
/// it, so every gallery deals the same game).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // Never zero, and the first draws well mixed.
        let mut rng = Rng(seed ^ 0x9e37_79b9_7f4a_7c15 | 1);
        for _ in 0..4 {
            rng.next();
        }
        rng
    }

    /// From `CATALYST_SEED` if set, else from the clock.
    pub fn seeded() -> Rng {
        let seed = std::env::var("CATALYST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) ^ std::process::id() as u64
        });
        Rng::new(seed)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A whole number in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((self.next() >> 11) as u128 * n as u128 >> 53) as usize
    }

    /// A number in `0.0..1.0`.
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_kept_and_read_back() {
        let path = std::env::temp_dir().join(format!("catalyst-kit-saved-{}", std::process::id()));
        let mut saved = Saved::at(path.clone());
        assert_eq!(saved.get("best"), None);
        saved.set("best", 314);
        saved.set("name", "two\nlines");
        saved.save().unwrap();
        let back = Saved::at(path.clone());
        assert_eq!(back.number::<u32>("best"), Some(314));
        assert_eq!(back.get("name"), Some("two lines"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_same_seed_deals_the_same() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        let draws: Vec<usize> = (0..20).map(|_| a.below(10)).collect();
        assert_eq!(draws, (0..20).map(|_| b.below(10)).collect::<Vec<_>>());
        assert!(draws.iter().all(|d| *d < 10));
        let mut seen = [false; 10];
        let mut c = Rng::new(1);
        for _ in 0..500 {
            seen[c.below(10)] = true;
        }
        assert!(seen.iter().all(|s| *s), "every value comes up");
        let u = c.unit();
        assert!((0.0..1.0).contains(&u));
    }
}
