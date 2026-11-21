//! The lock screen's passcode, which is never kept: only a salted, slow hash
//! of it is.
//!
//! - **The hash**: Argon2id (RFC 9106), with a random 16-byte salt, at
//!   OWASP's first choice of cost: 19 MiB of memory, two passes, one lane.
//!   Both of the memory-hard hashes are fine on the CM0's Cortex-A53 without
//!   its crypto instructions (the RP3A0 has no AES or SHA): Argon2's work is
//!   BLAKE2b's 64-bit adds, rotations and multiplies, plain integer code the
//!   A53 does at full speed. Argon2id is OWASP's first choice, and its floor
//!   for scrypt (128 MiB) is several times this work: more than 300 ms here
//!   could hold. It takes 10 ms on a desktop PC; on the
//!   A53, `catalyst-shell --passcode-bench` measures it. It is checked on a
//!   thread of its own, so the frame never waits.
//! - **Sized for the handheld when set**: the hash is timed as it is made.
//!   On a handheld slow enough that it took longer than [`BUDGET_MS`] (a busy
//!   processor, a slower board), it is made again at the largest cost that
//!   fits, never under [`FLOOR`]. The cost is written into the file, so
//!   checking always takes what making took.
//! - **The file**: `/data/catalyst/passcode`, readable and writable by root
//!   alone (0600, from the moment it exists), written whole or not at all.
//!   One line, the standard PHC string: `$argon2id$v=19$m=19456,t=2,p=1$
//!   <salt>$<hash>`.
//! - **Wrong tries**: counted in `passcode-tries` beside it (root only too),
//!   so a restart does not start the count again. The first four cost
//!   nothing; from the fifth each waits longer: 30 s, 1 min, 5 min, 15 min,
//!   then an hour each. The handheld has no clock of its own across a power
//!   cut, so a wait read back is never longer than its own length: a clock
//!   set back cannot lock anyone out for years.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

/// A passcode's length: four digits at least; a short word at most.
pub const MIN_LEN: usize = 4;
pub const MAX_LEN: usize = 24;

/// What a passcode is made of: digits, letters and the keyboard's signs, no
/// spaces (a space is easy to type by mistake and impossible to see).
pub fn allowed(c: char) -> bool {
    c.is_ascii_graphic()
}

/// Argon2id's cost: memory in KiB, passes, lanes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Cost {
    fn work(self) -> f32 {
        self.m_kib as f32 * self.t as f32
    }

    pub fn name(self) -> String {
        format!("m={},t={},p={}", self.m_kib, self.t, self.p)
    }
}

/// OWASP's first choice for Argon2id (19 MiB, two passes, one lane).
pub const COST: Cost = Cost { m_kib: 19 * 1024, t: 2, p: 1 };
/// The least it steps down to on a slow handheld: 8 MiB, two passes.
pub const FLOOR: Cost = Cost { m_kib: 8 * 1024, t: 2, p: 1 };
/// What making (and so checking) it may take, in milliseconds.
pub const BUDGET_MS: f32 = 260.0;

/// The cost a file may name: within reason, so a damaged file cannot ask
/// for gigabytes or minutes. (No floor: whoever can write the file can as
/// well write a hash of their own.)
fn sane(cost: Cost) -> bool {
    (8..=64 * 1024).contains(&cost.m_kib) && (1..=8).contains(&cost.t) && cost.p == 1
}

fn argon(cost: Cost) -> Option<Argon2<'static>> {
    Some(Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(cost.m_kib, cost.t, cost.p, Some(32)).ok()?))
}

/// The PHC string for `passcode` with `salt` at `cost`.
pub fn hash_with(passcode: &str, salt: &[u8], cost: Cost) -> Option<String> {
    let salt = SaltString::encode_b64(salt).ok()?;
    Some(argon(cost)?.hash_password(passcode.as_bytes(), &salt).ok()?.to_string())
}

/// The cost a PHC string was made at, if it is one this keeps.
pub fn cost_of(phc: &str) -> Option<Cost> {
    let parsed = PasswordHash::new(phc).ok()?;
    if parsed.algorithm.as_str() != "argon2id" || parsed.version != Some(0x13) {
        return None;
    }
    let get = |name: &str| parsed.params.get_decimal(name);
    Some(Cost { m_kib: get("m")?, t: get("t")?, p: get("p")? }).filter(|c| sane(*c))
}

/// Whether `passcode` is the one `phc` was made from (in constant time, at
/// the cost the string names).
pub fn check(passcode: &str, phc: &str) -> bool {
    let (Some(cost), Ok(parsed)) = (cost_of(phc), PasswordHash::new(phc)) else { return false };
    argon(cost).is_some_and(|a| a.verify_password(passcode.as_bytes(), &parsed).is_ok())
}

/// Sixteen bytes from the kernel's random source (on a computer without
/// one, the simulator, from the system's hashing keys).
fn salt() -> [u8; 16] {
    let mut out = [0u8; 16];
    #[cfg(unix)]
    {
        use std::io::Read;
        if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut out)).is_ok() {
            return out;
        }
    }
    use std::hash::{BuildHasher, RandomState};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        chunk.copy_from_slice(&RandomState::new().hash_one((nanos, i)).to_le_bytes()[..chunk.len()]);
    }
    out
}

/// The cost to make a new hash at, given that `COST` took `took` here: the
/// largest of the same shape within the budget, never under the floor.
pub fn fitted(took: Duration) -> Cost {
    let ms = took.as_secs_f32() * 1000.0;
    if ms <= BUDGET_MS {
        return COST;
    }
    let m = (COST.work() * BUDGET_MS / ms / COST.t as f32) as u32;
    Cost { m_kib: (m / 1024 * 1024).clamp(FLOOR.m_kib, COST.m_kib), ..COST }
}

/// A new passcode's hash, sized for this handheld: (the PHC string, the
/// cost, what checking it will take).
pub fn make(passcode: &str) -> Option<(String, Cost, Duration)> {
    let cost = test_cost().unwrap_or(COST);
    let salt = salt();
    let began = Instant::now();
    let phc = hash_with(passcode, &salt, cost)?;
    let took = began.elapsed();
    let fit = if cost == COST { fitted(took) } else { cost };
    if fit == cost {
        return Some((phc, cost, took));
    }
    let began = Instant::now();
    let phc = hash_with(passcode, &salt, fit)?;
    Some((phc, fit, began.elapsed()))
}

/// Overwrite what a passcode was held in before it is let go (best effort:
/// the allocator may have copied it while it grew).
pub fn wipe(text: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { text.as_bytes_mut().fill(0) };
    text.clear();
}

// ---------------------------------------------------------------- the file ---

// The tests run many shells at once: each test thread keeps its passcode in
// a folder of its own, so one test's passcode never locks another's shell.
#[cfg(test)]
thread_local! {
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static TEST_COST: std::cell::Cell<Option<Cost>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn test_store() -> PathBuf {
    use std::sync::atomic::AtomicUsize;
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("catalyst-passcode-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
    // Quick hashes for the shell's tests; the real cost is tested on its own.
    TEST_COST.with(|c| c.set(Some(Cost { m_kib: 256, t: 1, p: 1 })));
    dir
}

#[cfg(test)]
fn test_cost() -> Option<Cost> {
    TEST_COST.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn test_cost() -> Option<Cost> {
    None
}

fn dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return dir;
    }
    crate::config_dir()
}

/// Where the hash is kept: `/data/catalyst/passcode` on the handheld.
pub fn file() -> PathBuf {
    dir().join("passcode")
}

fn tries_file() -> PathBuf {
    dir().join("passcode-tries")
}

/// Bumped each time a passcode is set or taken off, so the lock can tell
/// without reading the file every frame.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The hash kept, if there is one this reads.
pub fn stored() -> Option<String> {
    let text = std::fs::read_to_string(file()).ok()?;
    let line = text.trim().to_owned();
    cost_of(&line).map(|_| line)
}

/// A passcode is set.
pub fn is_set() -> bool {
    stored().is_some()
}

/// Whether a passcode is set: `None` when the file could not be read just
/// now (the card busy), so a lock already on stays on. A file that is not
/// there, or is not a hash this reads, is no passcode: never a lock with no
/// way in.
pub fn state() -> Option<bool> {
    match std::fs::read_to_string(file()) {
        Ok(text) => Some(cost_of(text.trim()).is_some()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// Keep a new passcode's hash (and forget any wrong tries).
pub fn keep(phc: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir())?;
    crate::sys::write_private(&file(), format!("{phc}\n").as_bytes())?;
    std::fs::remove_file(tries_file()).ok();
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Take the passcode off: no lock screen from now on.
pub fn remove() -> std::io::Result<()> {
    match std::fs::remove_file(file()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::remove_file(tries_file()).ok();
    GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

// --------------------------------------------------------------- the tries ---

/// How long the next try waits after `fails` wrong ones in a row, seconds.
pub fn delay(fails: u32) -> u64 {
    match fails {
        0..=4 => 0,
        5 => 30,
        6 => 60,
        7 => 5 * 60,
        8 => 15 * 60,
        _ => 60 * 60,
    }
}

/// Wrong tries in a row, and the time before which no try is taken.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Tries {
    pub fails: u32,
    /// Unix seconds.
    pub until: i64,
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Tries {
    /// As kept, at `now`: a wait is never longer than its own length, so a
    /// clock that went back (no clock across a power cut) cannot stretch it.
    pub fn read(text: &str, now: i64) -> Tries {
        let mut f = text.split_whitespace().map(|v| v.parse::<i64>().ok());
        let (Some(Some(fails)), Some(Some(until))) = (f.next(), f.next()) else { return Tries::default() };
        let fails = fails.clamp(0, 1000) as u32;
        Tries { fails, until: until.min(now + delay(fails) as i64) }
    }

    pub fn load(now: i64) -> Tries {
        std::fs::read_to_string(tries_file()).map_or_else(|_| Tries::default(), |t| Tries::read(&t, now))
    }

    fn save(&self) {
        crate::sys::write_private(&tries_file(), format!("{} {}\n", self.fails, self.until).as_bytes()).ok();
    }

    /// Seconds still to wait at `now` before a try is taken.
    pub fn wait(&self, now: i64) -> u64 {
        (self.until - now).max(0) as u64
    }

    /// A wrong one: counted, kept, and the next waits as long as it must.
    pub fn failed(&mut self, now: i64) {
        self.fails = self.fails.saturating_add(1);
        self.until = now + delay(self.fails) as i64;
        self.save();
    }

    /// The right one: the count starts again.
    pub fn passed(&mut self) {
        *self = Tries::default();
        std::fs::remove_file(tries_file()).ok();
    }

    /// Tries left before one has to wait, when that is near (two or fewer).
    pub fn before_wait(&self) -> Option<u32> {
        let free = 5u32.saturating_sub(self.fails);
        (self.fails > 0 && free <= 2 && free > 0).then_some(free)
    }
}

/// "0:42", "4:59", "59:12": a wait as the lock screen counts it down.
pub fn wait_text(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

// ------------------------------------------------------------------ bench ---

/// `catalyst-shell --passcode-bench`: what making and checking a passcode
/// takes on this processor, at the cost it would be made at.
pub fn bench() -> String {
    let salt = [7u8; 16];
    let mut times: Vec<Duration> = (0..5)
        .map(|_| {
            let began = Instant::now();
            hash_with("12345678", &salt, COST).expect("hashes");
            began.elapsed()
        })
        .collect();
    times.sort();
    let median = times[2];
    let fit = fitted(median);
    let phc = hash_with("12345678", &salt, fit).expect("hashes");
    let began = Instant::now();
    let ok = check("12345678", &phc);
    let checked = began.elapsed();
    let wrong = !check("12345679", &phc);
    format!(
        "argon2id {}: {} ms (median of 5; fastest {} ms, slowest {} ms)\nmade at {}: a check takes {} ms ({})\n{}",
        COST.name(),
        median.as_millis(),
        times[0].as_millis(),
        times[4].as_millis(),
        fit.name(),
        checked.as_millis(),
        if ok && wrong { "right one passes, wrong one fails" } else { "CHECK FAILED" },
        if checked.as_secs_f32() * 1000.0 <= 300.0 { "under 300 ms: ok" } else { "OVER 300 ms" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passcode_is_kept_only_as_its_hash_and_checks() {
        let salt = [1u8; 16];
        let phc = hash_with("2468", &salt, Cost { m_kib: 1024, t: 2, p: 1 }).unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=1024,t=2,p=1$"), "{phc}");
        assert!(!phc.contains("2468"));
        assert!(check("2468", &phc));
        for wrong in ["2469", "246", "24680", "", " 2468"] {
            assert!(!check(wrong, &phc), "{wrong:?}");
        }
        // Another salt, another hash.
        assert_ne!(hash_with("2468", &[2u8; 16], Cost { m_kib: 1024, t: 2, p: 1 }).unwrap(), phc);
    }

    #[test]
    fn the_real_cost_is_owasps_first_choice_and_checks() {
        assert_eq!(COST, Cost { m_kib: 19456, t: 2, p: 1 });
        let phc = hash_with("correct horse", &salt(), COST).unwrap();
        assert_eq!(cost_of(&phc), Some(COST));
        assert!(check("correct horse", &phc) && !check("correct hors", &phc));
    }

    #[test]
    fn only_a_sound_argon2id_string_is_read() {
        let good = hash_with("abcd", &[3u8; 16], FLOOR).unwrap();
        assert_eq!(cost_of(&good), Some(FLOOR));
        // Another algorithm, an absurd cost, damage: none is a passcode.
        let argon2i = good.replace("$argon2id$", "$argon2i$");
        let huge = good.replace("m=8192", "m=4194304");
        for bad in [argon2i.as_str(), huge.as_str(), "", "2468", "$argon2id$v=19$m=19456,t=2,p=1$", &good[..good.len() - 10]] {
            assert!(cost_of(bad).is_none() || !check("abcd", bad), "{bad:?}");
        }
        assert_eq!(cost_of(&huge), None);
        assert!(!check("abcd", &argon2i));
    }

    #[test]
    fn a_slow_handheld_gets_the_largest_cost_that_fits() {
        assert_eq!(fitted(Duration::from_millis(180)), COST);
        assert_eq!(fitted(Duration::from_millis(260)), COST);
        let half = fitted(Duration::from_millis(520));
        assert_eq!((half.t, half.p), (2, 1));
        assert!((9 * 1024..=10 * 1024).contains(&half.m_kib), "{half:?}");
        assert_eq!(fitted(Duration::from_secs(5)), FLOOR, "never under the floor");
    }

    #[test]
    fn the_file_is_set_kept_root_only_and_taken_off() {
        test_store();
        assert!(!is_set());
        let before = generation();
        let (phc, cost, _) = make("7391").unwrap();
        assert_eq!(cost, Cost { m_kib: 256, t: 1, p: 1 }, "the tests' quick cost");
        keep(&phc).unwrap();
        assert!(is_set() && generation() > before);
        assert_eq!(stored().as_deref(), Some(phc.as_str()));
        assert!(!std::fs::read_to_string(file()).unwrap().contains("7391"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(file()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A damaged file is no passcode (and never a lock with no way in).
        std::fs::write(file(), "garbage\n").unwrap();
        assert!(!is_set());
        keep(&phc).unwrap();
        remove().unwrap();
        assert!(!is_set());
        remove().unwrap();
    }

    #[test]
    fn wrong_tries_wait_longer_and_longer_and_survive_a_restart() {
        test_store();
        let now = 1_800_000_000;
        let mut tries = Tries::load(now);
        for n in 1..=4 {
            tries.failed(now);
            assert_eq!(tries.wait(now), 0, "try {n} costs nothing");
        }
        assert_eq!(tries.before_wait(), Some(1));
        tries.failed(now);
        assert_eq!(tries.wait(now), 30);
        // A restart keeps the count and the wait.
        assert_eq!(Tries::load(now + 10), Tries { fails: 5, until: now + 30 });
        assert_eq!(Tries::load(now + 10).wait(now + 10), 20);
        let waits: Vec<u64> = (6..=11).map(delay).collect();
        assert_eq!(waits, [60, 300, 900, 3600, 3600, 3600]);
        // The clock set back by a year: the wait is still only its own length.
        let mut t = Tries::load(now);
        t.failed(now);
        assert_eq!(Tries::load(now - 365 * 86400).wait(now - 365 * 86400), 60);
        // The right one: all forgotten.
        t.passed();
        assert_eq!(Tries::load(now), Tries::default());
        assert_eq!(wait_text(59), "0:59");
        assert_eq!(wait_text(3600), "60:00");
        for bad in ["", "x y", "3", "-4 9"] {
            let t = Tries::read(bad, now);
            assert!(t.wait(now) <= delay(t.fails), "{bad:?}");
        }
    }

    #[test]
    fn passcodes_are_keyboard_signs_without_spaces() {
        assert!("2468".chars().all(allowed) && "Hunter2!".chars().all(allowed));
        assert!(!allowed(' ') && !allowed('é') && !allowed('\n'));
        let mut s = String::from("secret");
        wipe(&mut s);
        assert!(s.is_empty());
    }
}
