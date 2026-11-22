//! The vault: passwords and secrets kept on the card, sealed, opened with
//! the lock screen's passcode, and never sent anywhere.
//!
//! - **What opens it: the lock screen's passcode**, not a master password of
//!   its own. One secret to remember on a handheld typed on a thumb
//!   keyboard, and it is the one you already type; a second would be
//!   written down, or made the same. The passcode itself is never kept (the
//!   lock screen keeps only its hash), so the vault derives its own key from
//!   it, with a salt of its own (`crypto.rs`). Changing the passcode in
//!   Settings › Lock screen wraps the vault's key again under the new one;
//!   turning the passcode off is refused while there is a vault. With no
//!   passcode set, the vault asks for one first. A forgotten passcode is a
//!   lost vault, and the vault says so plainly: taking the passcode off over
//!   SSH opens the handheld, never the vault.
//! - **What that costs**: someone with the card, a computer and time can try
//!   passcodes against the file as fast as Argon2id lets them: a passcode of
//!   four digits falls in minutes. The vault says so when it is made, and a
//!   longer passcode is the remedy. At the keys, wrong tries count with the
//!   lock screen's (`passcode-tries`) and wait as long.
//! - **The file**: one, `/data/catalyst/vault`, root's alone (0600), written
//!   whole or not at all (a new file renamed over the old, flushed to the
//!   card), sealed as `crypto.rs` says. Nothing of it is ever written in the
//!   clear: a body is sealed in the buffer it was built in. The shell's
//!   memory is locked in RAM from start (`mlockall` with `MCL_FUTURE`, in
//!   main.rs), so nothing the vault holds, nor Argon2's working memory, is
//!   ever swapped (and the handheld's swap is zram, in RAM, besides).
//!   Keys and entries are wiped from memory when they are let go.
//! - **It locks** after two minutes without a key in it, as the screen goes
//!   off or StandBy comes, as the lock screen comes down, and as soon as the
//!   app is left (back to Home, or another app from the recent apps): what
//!   was copied stays on the clipboard its 30 seconds, so nothing needs the
//!   vault open behind you. It is not kept among the recent apps, whose
//!   cards are pictures of what was on the screen, and no screenshot is
//!   taken while it is open.
//! - **Spotlight** finds entries by their titles while it is open, and
//!   nothing at all while it is locked; it is never given anything else.
//! - **A file that will not open** (changed, cut short, not a vault) is
//!   never written over: the vault says so, and can set it aside under
//!   another name to make a new one, into which a backup restores.

mod clip;
mod crypto;
mod entry;
mod gen;
mod import;
mod screen;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
mod totp;
mod words;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::passcode::{self, Cost};
use crypto::{Keyring, Role};
pub use entry::{Entry, Field, Kind};
pub use screen::Vault;

/// The vault's derivation: Argon2id at the lock screen's memory (19 MiB),
/// with three passes where the passcode takes two. Measured on a desktop
/// PC at about 15 ms; the A53 is taken as 20 times slower (the simulator's
/// own estimate), so about 300 ms an unlock, an estimate until a handheld
/// runs `catalyst-shell --vault-bench`. Sized when made, as the passcode
/// is: a handheld that takes longer than [`BUDGET_MS`] gets the largest
/// cost of the same shape that fits.
pub const COST: Cost = Cost { m_kib: 19 * 1024, t: 3, p: 1 };
pub const BUDGET_MS: f32 = 380.0;
/// A backup leaves the handheld: four times the memory and the passes, for
/// the one second it takes once.
pub const BACKUP_COST: Cost = Cost { m_kib: 64 * 1024, t: 3, p: 1 };
/// A backup's password: this long at least.
pub const BACKUP_MIN: usize = 10;
/// It locks after this long with no key in it.
pub const IDLE: Duration = Duration::from_secs(120);

/// Where the vault is kept: `/data/catalyst/vault`, beside the passcode.
pub fn path() -> PathBuf {
    passcode::file().with_file_name("vault")
}

/// There is a vault (open or not).
pub fn exists() -> bool {
    path().is_file()
}

/// Settings › Lock screen asks: the passcode may not come off while a
/// vault depends on it.
pub fn needs_passcode() -> bool {
    exists()
}

// ------------------------------------------------------------ the session ---

/// The open vault: its key, its entries, and when it was last used.
struct Session {
    ring: Keyring,
    entries: Vec<Entry>,
    used: Instant,
    path: PathBuf,
}

#[cfg(not(test))]
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static OPEN: AtomicBool = AtomicBool::new(false);
/// Bumped whenever it opens, locks or changes: screens and Spotlight look.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(not(test))]
fn session() -> MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

// The tests run many shells at once, each locking the vault as its lock
// screen comes down: each test thread has a vault session of its own.
#[cfg(test)]
fn session() -> MutexGuard<'static, Option<Session>> {
    thread_local! {
        static CELL: &'static Mutex<Option<Session>> = Box::leak(Box::new(Mutex::new(None)));
    }
    CELL.with(|c| *c).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_open() -> bool {
    if cfg!(test) {
        return session().is_some();
    }
    OPEN.load(Ordering::Relaxed)
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn changed() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Lock it now: its key and entries wiped from memory, and a password made
/// for an entry not yet taken up.
pub fn lock() {
    screen::forget();
    let gone = session().take();
    if gone.is_some() {
        OPEN.store(false, Ordering::Relaxed);
        changed();
    }
    drop(gone);
}

/// Every frame, from the shell: lock it if `now` (the screen off, StandBy,
/// the lock screen) or after two idle minutes, and wipe a key held for a
/// passcode change likewise; and let a copied secret go once its 30 seconds
/// are up. Costs a load or two when it is locked.
pub fn watch(now: bool) {
    clip::clear_after();
    rewrap_watch(now);
    if !is_open() {
        return;
    }
    let idle = session().as_ref().is_some_and(|s| s.used.elapsed() >= IDLE);
    if now || idle {
        lock();
    }
}

/// A key in the vault: the idle wait starts again.
pub fn touch() {
    if let Some(s) = session().as_mut() {
        s.used = Instant::now();
    }
}

/// Seconds before it locks by itself.
pub fn locks_in() -> Option<u64> {
    session().as_ref().map(|s| IDLE.saturating_sub(s.used.elapsed()).as_secs())
}

fn install(s: Session) {
    *session() = Some(s);
    OPEN.store(true, Ordering::Relaxed);
    changed();
}

/// Look at the entries, while it is open.
pub fn with<R>(f: impl FnOnce(&[Entry]) -> R) -> Option<R> {
    session().as_ref().map(|s| f(&s.entries))
}

/// One entry, by its id.
pub fn get(id: u64) -> Option<Entry> {
    with(|entries| entries.iter().find(|e| e.id == id).cloned()).flatten()
}

/// Change the entries and keep them: sealed here, written on a thread.
pub fn change(f: impl FnOnce(&mut Vec<Entry>)) -> Result<(), String> {
    let mut guard = session();
    let s = guard.as_mut().ok_or("the vault is locked")?;
    f(&mut s.entries);
    entry::sort(&mut s.entries);
    let file = s.ring.seal(&entry::encode(&s.entries)).map_err(|e| e.words().to_owned())?;
    write_later(s.path.clone(), file);
    s.used = Instant::now();
    drop(guard);
    changed();
    Ok(())
}

// ------------------------------------------------------------- writing ---

static WRITES_QUEUED: AtomicU64 = AtomicU64::new(0);
static WRITES_PENDING: AtomicUsize = AtomicUsize::new(0);
/// Each file written, and the newest write made to it.
static WRITER: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());
static WRITE_FAILED: Mutex<Option<String>> = Mutex::new(None);

/// Write a sealed file on a thread of its own (never the frame's), newest
/// last: a write overtaken by a newer one to the same file is not made.
fn write_later(path: PathBuf, file: Vec<u8>) {
    let seq = WRITES_QUEUED.fetch_add(1, Ordering::SeqCst) + 1;
    WRITES_PENDING.fetch_add(1, Ordering::SeqCst);
    let write = move || {
        let mut written = WRITER.lock().unwrap_or_else(|e| e.into_inner());
        let newer = written.iter().any(|(p, s)| *p == path && *s > seq);
        if !newer {
            let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| crate::sys::write_private(&path, &file));
            if let Err(e) = result {
                *WRITE_FAILED.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("could not keep the vault: {e}"));
            }
            written.retain(|(p, _)| *p != path);
            written.push((path, seq));
        }
        WRITES_PENDING.fetch_sub(1, Ordering::SeqCst);
    };
    if let Err(e) = std::thread::Builder::new().name("vault-write".into()).stack_size(128 * 1024).spawn(write) {
        WRITES_PENDING.fetch_sub(1, Ordering::SeqCst);
        *WRITE_FAILED.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("could not keep the vault: {e}"));
    }
}

/// Wait for writes under way (before the file is read again).
pub fn flush() {
    let began = Instant::now();
    while WRITES_PENDING.load(Ordering::SeqCst) > 0 && began.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A write that failed since last asked, in words.
pub fn take_write_error() -> Option<String> {
    WRITE_FAILED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

// ------------------------------------------------------ opening, making ---

/// What a try at opening (or making) the vault came to.
pub enum Outcome {
    /// Open, to be taken up by the screen that asked ([`Unlocked::install`]):
    /// if that screen has gone meanwhile, the key is wiped unused.
    Opened(Unlocked),
    /// Not the passcode.
    Wrong,
    /// The file will not open with any passcode: changed, cut short, or not
    /// a vault at all. It is left as it is; [`set_aside_damaged`] makes room
    /// for a new one without losing it.
    Damaged(String),
    Failed(String),
}

/// An opened vault on its way to the screen that asked for it.
pub struct Unlocked(Session);

impl Unlocked {
    /// The vault is open from now on.
    pub fn install(self) {
        install(self.0);
    }
}

/// The lock screen's passcode now, as its hash (to make or move the vault).
fn current_phc() -> Option<String> {
    passcode::stored()
}

/// Whether the vault was wrapped under the passcode set now (false: under
/// an earlier one, the passcode having come off and gone on again).
pub fn under_current_passcode() -> bool {
    let Ok(file) = std::fs::read(path()) else { return true };
    match crypto::Head::read(&file) {
        Ok(head) => head.mark == crypto::mark_of(current_phc().as_deref()),
        Err(_) => true,
    }
}

/// The cost to derive at: the tests' quick one on a test's thread,
/// otherwise `base`.
fn cost_for(base: Cost) -> Cost {
    passcode::test_cost().unwrap_or(base)
}

/// Work on a thread of its own, its answer to be picked up.
fn work<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    // With no thread, the receiver says it is disconnected, which every
    // caller takes as a failure.
    std::thread::Builder::new().name(name.into()).stack_size(256 * 1024).spawn(move || {
        tx.send(f()).ok();
    }).ok();
    rx
}

/// Open the vault with `passcode`, on a thread. Once it is opened, a vault
/// still wrapped under an earlier passcode moves to the one set now, if
/// that is what was typed.
pub fn begin_unlock(passcode: Zeroizing<String>) -> Receiver<Outcome> {
    let path = path();
    let phc = current_phc();
    work("vault-open", move || {
        flush();
        let file = match std::fs::read(&path) {
            Ok(f) => f,
            Err(e) => return Outcome::Failed(format!("could not read the vault: {e}")),
        };
        let (ring, plain) = match Keyring::open(&file, passcode.as_bytes(), Role::Vault) {
            Ok(opened) => opened,
            Err(crypto::Error::Wrong) => return Outcome::Wrong,
            Err(e @ (crypto::Error::Damaged | crypto::Error::Tampered | crypto::Error::NotOurs)) => return Outcome::Damaged(e.words().to_owned()),
            Err(e) => return Outcome::Failed(e.words().to_owned()),
        };
        let Some(entries) = entry::decode(plain.bytes()) else { return Outcome::Damaged(crypto::Error::Damaged.words().to_owned()) };
        drop(plain);
        let mut ring = ring;
        let mark = crypto::mark_of(phc.as_deref());
        if ring.head().mark != mark && phc.as_deref().is_some_and(|phc| passcode::check(&passcode, phc)) {
            if let Ok(moved) = ring.rewrap(passcode.as_bytes(), ring.head().cost, mark) {
                if let Ok(sealed) = moved.seal(&entry::encode(&entries)) {
                    if crate::sys::write_private(&path, &sealed).is_ok() {
                        ring = moved;
                    }
                }
            }
        }
        Outcome::Opened(Unlocked(Session { ring, entries, used: Instant::now(), path }))
    })
}

/// Make the vault, empty, under `passcode` (which must be the lock
/// screen's): sized for this handheld as it is made, and opened.
pub fn begin_create(passcode: Zeroizing<String>) -> Receiver<Outcome> {
    let path = path();
    let phc = current_phc();
    let quick = passcode::test_cost();
    work("vault-make", move || {
        let Some(phc) = phc else { return Outcome::Failed("set a passcode first".into()) };
        if !passcode::check(&passcode, &phc) {
            return Outcome::Wrong;
        }
        let mark = crypto::mark_of(Some(&phc));
        let base = quick.unwrap_or(COST);
        let began = Instant::now();
        let made = Keyring::make(passcode.as_bytes(), base, Role::Vault, mark);
        let fit = if quick.is_some() { base } else { passcode::fitted_to(base, BUDGET_MS, began.elapsed()) };
        let ring = match made {
            Ok(r) if fit == base => r,
            Ok(_) => match Keyring::make(passcode.as_bytes(), fit, Role::Vault, mark) {
                Ok(r) => r,
                Err(e) => return Outcome::Failed(e.words().to_owned()),
            },
            Err(e) => return Outcome::Failed(e.words().to_owned()),
        };
        let file = match ring.seal(&entry::encode(&[])) {
            Ok(f) => f,
            Err(e) => return Outcome::Failed(e.words().to_owned()),
        };
        if let Err(e) = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| crate::sys::write_private(&path, &file)) {
            return Outcome::Failed(format!("could not keep the vault: {e}"));
        }
        Outcome::Opened(Unlocked(Session { ring, entries: Vec::new(), used: Instant::now(), path }))
    })
}

/// Delete the vault: locked, and its file gone (no one could open it
/// without the passcode anyway; this is for starting again).
pub fn delete() -> std::io::Result<()> {
    lock();
    flush();
    match std::fs::remove_file(path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => {
            changed();
            Ok(())
        }
    }
}

/// A vault that will not open, set aside to make room for a new one: renamed
/// beside it (`vault.damaged-20261121-214107`, still root's alone), never
/// deleted, so nothing in it is lost should it be mended. The name it was
/// given.
pub fn set_aside_damaged() -> std::io::Result<PathBuf> {
    lock();
    flush();
    let path = path();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut to = path.with_file_name(format!("vault.damaged-{stamp}"));
    let mut n = 2;
    while to.exists() {
        to = path.with_file_name(format!("vault.damaged-{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(&path, &to)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
        dir.sync_all().ok();
    }
    changed();
    Ok(to)
}

// ------------------------------------------------ the passcode changing ---

/// The vault's key, unwrapped with the passcode there is, while Settings
/// asks for the new one twice: the key, the file it came from, and when.
struct Held {
    ring: Keyring,
    file: Vec<u8>,
    path: PathBuf,
    since: Instant,
}

/// A passcode change under way.
struct Change {
    held: Option<Held>,
    /// The held key was wiped (the screen went off, or two minutes passed)
    /// before the new passcode was made: the change must start again.
    voided: bool,
}

#[cfg(not(test))]
fn pending() -> MutexGuard<'static, Change> {
    static PENDING: Mutex<Change> = Mutex::new(Change { held: None, voided: false });
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

// Each test thread changes a passcode of its own, out of reach of the other
// tests' shells, whose screens go off as they please.
#[cfg(test)]
fn pending() -> MutexGuard<'static, Change> {
    thread_local! {
        static CELL: &'static Mutex<Change> = Box::leak(Box::new(Mutex::new(Change { held: None, voided: false })));
    }
    CELL.with(|c| *c).lock().unwrap_or_else(|e| e.into_inner())
}

/// Settings › Lock screen › Change passcode: the passcode there is was
/// right. If it opens the vault, its key is held (wiped when let go) until
/// the new passcode is made, the change is given up, the screen goes off or
/// StandBy or the lock screen comes, or two minutes pass.
pub fn rewrap_begin(passcode: &str) {
    rewrap_begin_at(path(), passcode);
}

fn rewrap_begin_at(path: PathBuf, passcode: &str) {
    flush();
    let held = std::fs::read(&path).ok().and_then(|file| {
        let (ring, _) = Keyring::open(&file, passcode.as_bytes(), Role::Vault).ok()?;
        Some(Held { ring, file, path, since: Instant::now() })
    });
    *pending() = Change { held, voided: false };
}

/// From `watch`: a key held for a change is wiped as the vault would lock.
fn rewrap_watch(now: bool) {
    let mut change = pending();
    if change.held.as_ref().is_some_and(|h| now || h.since.elapsed() >= IDLE) {
        *change = Change { held: None, voided: true };
    }
}

/// The key held for a change was wiped before the change was made: Settings
/// asks for the passcode there is again.
pub fn rewrap_voided() -> bool {
    pending().voided
}

/// The new passcode is made: the vault's key wrapped under it and written,
/// then `keep` keeps the passcode's hash. The two never part: should the
/// vault not move, the passcode is not changed; should keeping fail, the
/// vault is put back as it was; and if the held key was wiped meanwhile,
/// nothing is changed.
pub fn rewrap_with(new: &str, new_phc: &str, keep: impl FnOnce() -> std::io::Result<()>) -> Result<(), String> {
    let keep = || keep().map_err(|e| format!("could not keep it: {e}"));
    let change = std::mem::replace(&mut *pending(), Change { held: None, voided: false });
    let Some(Held { ring, file: old, path, .. }) = change.held else {
        if change.voided {
            return Err("locked meanwhile · change it again".into());
        }
        // No vault, or one under an earlier passcode: nothing to move.
        return keep();
    };
    // An open vault would hold the old header.
    lock();
    let moved = ring.unseal(&old).and_then(|plain| ring.rewrap(new.as_bytes(), ring.head().cost, crypto::mark_of(Some(new_phc)))?.seal(plain.bytes()));
    drop(ring);
    let file = moved.map_err(|e| format!("could not move the vault: {}", e.words()))?;
    crate::sys::write_private(&path, &file).map_err(|e| format!("could not move the vault: {e}"))?;
    keep().inspect_err(|_| {
        crate::sys::write_private(&path, &old).ok();
    })
}

/// The change was given up: the held key is wiped.
pub fn rewrap_cancel() {
    *pending() = Change { held: None, voided: false };
}

// ------------------------------------------------------- backup, restore ---

/// A backup of the open vault at `dest`, sealed under `password`, on a thread.
pub fn begin_backup(password: Zeroizing<String>, dest: PathBuf) -> Option<Receiver<Result<PathBuf, String>>> {
    let payload = with(entry::encode)?;
    let cost = cost_for(BACKUP_COST);
    Some(work("vault-backup", move || {
        let ring = Keyring::make(password.as_bytes(), cost, Role::Backup, [0; 8]).map_err(|e| e.words().to_owned())?;
        let file = ring.seal(&payload).map_err(|e| e.words().to_owned())?;
        crate::sys::write_private(&dest, &file).map_err(|e| format!("could not write it: {e}"))?;
        Ok(dest)
    }))
}

/// The entries in a backup at `src`, opened with `password`, on a thread.
pub fn begin_restore(password: Zeroizing<String>, src: PathBuf) -> Receiver<Result<Vec<Entry>, crypto::Error>> {
    work("vault-restore", move || {
        let file = std::fs::read(&src).map_err(|_| crypto::Error::Damaged)?;
        let (_, plain) = Keyring::open(&file, password.as_bytes(), Role::Backup)?;
        let mut entries = entry::decode(plain.bytes()).ok_or(crypto::Error::Damaged)?;
        // Each its own new id: a backup restored twice, or into another
        // vault, never shares ids.
        for e in &mut entries {
            e.id = entry::new_id();
        }
        Ok(entries)
    })
}

// ------------------------------------------------------------ spotlight ---

/// Spotlight's way into the vault: entry titles, matched as the query is
/// typed, only while it is open. Nothing is indexed, so nothing is held by
/// the index or kept on the card; a pick is remembered by the entry's random
/// id alone.
pub struct Finder;

/// Spotlight finds the vault's titles from now on (while it is open).
pub fn register_spotlight() {
    crate::spotlight::register(Arc::new(Finder));
}

impl crate::spotlight::Provider for Finder {
    fn name(&self) -> &'static str {
        "vault"
    }

    fn answer(&self, query: &crate::spotlight::Query, _robot: &crate::robot::Robot, out: &mut Vec<crate::spotlight::Item>) {
        use crate::spotlight::{weight, Item};
        if query.is_empty() || !is_open() {
            return;
        }
        with(|entries| {
            for e in entries {
                if let Some(rank) = query.score(e.name(), "", weight::OTHER, false) {
                    out.push(Item::new(format!("{:016x}", e.id), e.name().to_owned(), e.kind.icon(), weight::OTHER).detail(format!("vault · {}", e.kind.word())).ranked(rank));
                }
            }
        });
    }

    fn open(&self, item: &crate::spotlight::Item, _cx: &mut crate::shell::Cx) -> Result<Box<dyn crate::shell::Screen>, String> {
        if !is_open() {
            return Err("the vault is locked".into());
        }
        let id = u64::from_str_radix(&item.id, 16).map_err(|_| "that is gone".to_owned())?;
        get(id).ok_or_else(|| "that is gone from the vault".to_owned())?;
        Ok(Box::new(screen::EntryPage::new(id)))
    }
}

// ---------------------------------------------------------------- bench ---

/// `catalyst-shell --vault-bench`: what opening the vault takes on this
/// processor, at the cost it would be made at.
pub fn bench() -> String {
    let salt = [7u8; 16];
    let mut times: Vec<Duration> = (0..5)
        .map(|_| {
            let began = Instant::now();
            crypto::derive(b"12345678", &salt, COST).expect("derives");
            began.elapsed()
        })
        .collect();
    times.sort();
    let median = times[2];
    let fit = passcode::fitted_to(COST, BUDGET_MS, median);
    let ring = Keyring::make(b"12345678", fit, Role::Vault, [0; 8]).expect("makes");
    let entries: Vec<Entry> = (0..200).map(|i| demo_entry(i, Kind::Login)).collect();
    let file = ring.seal(&entry::encode(&entries)).expect("seals");
    let began = Instant::now();
    let (_, plain) = Keyring::open(&file, b"12345678", Role::Vault).expect("opens");
    let n = entry::decode(plain.bytes()).map_or(0, |e| e.len());
    let opened = began.elapsed();
    let wrong = Keyring::open(&file, b"12345679", Role::Vault).is_err();
    let began = Instant::now();
    ring.seal(&entry::encode(&entries)).expect("seals");
    let sealed = began.elapsed();
    format!(
        "argon2id {}: {} ms (median of 5; fastest {} ms, slowest {} ms)\nmade at {}: opening {} entries takes {} ms ({}), sealing them {} ms\n{}",
        COST.name(),
        median.as_millis(),
        times[0].as_millis(),
        times[4].as_millis(),
        fit.name(),
        n,
        opened.as_millis(),
        if wrong { "a wrong passcode fails" } else { "WRONG PASSCODE OPENED IT" },
        sealed.as_millis(),
        if opened.as_secs_f32() * 1000.0 <= 400.0 { "under 400 ms: ok" } else { "OVER 400 ms" }
    )
}

/// An entry of demo data (never a real secret), for the bench and the shots.
pub fn demo_entry(i: usize, kind: Kind) -> Entry {
    let mut e = Entry::new(kind, 1000 + i as u64, 1_763_700_000 + i as i64);
    e.set(Field::Title, &format!("Demo site {i}"));
    e.set(Field::Username, &format!("demo{i}@example.org"));
    e.set(Field::Password, "demo-not-a-real-password");
    e
}

#[cfg(test)]
mod tests;
