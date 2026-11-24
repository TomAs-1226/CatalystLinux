//! Photo documentation (1.4): the album kept as a build log, the way a
//! robotics team keeps an engineering notebook. Each photo knows, by
//! itself, what it shows and where it was taken:
//!
//! - **Captions by Claude** (caption.rs), opt-in: new photos shrunk and
//!   sent in the background, in batches at half price, within a monthly
//!   limit; each caption kept in the photo's sidecar (meta.rs), shown under
//!   the photo and editable.
//! - **Named places** (places.rs), offline and private: the Wi-Fi around
//!   each photo, noted as it is taken; "Name this place" names it, and
//!   every photo there, before and after, takes the name.
//! - **The event and the match** (event.rs), from the event schedule:
//!   "Finger Lakes Regional · during Q31".
//! - **The town** (locate.rs, gazetteer.rs), opt-in: the Wi-Fi to BeaconDB
//!   for a position, made a town on the handheld.
//! - **The build log** (log.rs, ui.rs): days, then places; exported as
//!   Markdown with the photos, or a day sent to Team.
//!
//! The work runs on one thread, at the background's priority, only while
//! there is some: a photo taken starts it, and it ends once nothing waits
//! (or what waits cannot go on: captions turned off, the month's limit, no
//! key). While it waits to be online or for power it looks again every five
//! minutes, and in deep standby it parks. The frame beat never waits on it.

pub mod caption;
pub mod event;
pub mod gazetteer;
pub mod locate;
pub mod log;
pub mod meta;
pub mod net;
pub mod places;
pub mod store;
mod ui;
mod view;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

pub use store::Ledger;
pub use ui::{BuildLog, PhotoSettings};
pub use view::Under;

/// The handheld's own folder for it: `/data/catalyst/photolog`.
pub fn dir() -> PathBuf {
    crate::config_dir().join("photolog")
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// The records, opened on first use: one for the shell's life, behind its
/// own lock (the worker's functions take a lock as the tests take theirs).
pub fn ledger() -> &'static Mutex<Ledger> {
    static ONE: std::sync::OnceLock<Mutex<Ledger>> = std::sync::OnceLock::new();
    ONE.get_or_init(|| Mutex::new(Ledger::open(&dir(), &crate::photos::dir())))
}

/// The records read again from where they are kept now (the pictures,
/// after moving the folders).
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn reload() {
    *lock(ledger()) = Ledger::open(&dir(), &crate::photos::dir());
    changed();
}

/// Look at (or change) the records.
pub fn with<R>(f: impl FnOnce(&mut Ledger) -> R) -> R {
    let mut l = lock(ledger());
    f(&mut l)
}

fn lock(m: &Mutex<Ledger>) -> MutexGuard<'_, Ledger> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Something about the photos changed: the screens that show it look again.
pub fn changed() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Whether Claude is asked to caption a photo of this name: not a pixel
/// picture (a few dozen dithered pixels say little), not an edit (it takes
/// its photo's caption), not a PNG of any kind.
pub fn captionable(name: &str) -> Result<(), &'static str> {
    let lower = name.to_ascii_lowercase();
    if crate::photos::is_pixel(name) {
        return Err("a pixel picture");
    }
    if store::edit_of(name).is_some() {
        return Err("an edit: it takes its photo's caption");
    }
    if !(lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        return Err("not a photo the camera took");
    }
    Ok(())
}

fn unix(at: chrono::NaiveDateTime) -> i64 {
    use chrono::TimeZone;
    chrono::Local.from_local_datetime(&at).earliest().map_or_else(|| at.and_utc().timestamp(), |d| d.timestamp())
}

// ------------------------------------------------------------------ hooks ---

/// A photo is being taken (photos::new_path): its moment noted, on a thread
/// of its own (the Wi-Fi's last scan, the schedule as it stands), and the
/// work woken.
pub fn taken(path: &Path, at: chrono::NaiveDateTime) {
    // The tests note moments themselves (`note`), with fingerprints of their own.
    if cfg!(test) {
        return;
    }
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("photo-place".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            crate::sched::enter(crate::sched::Class::Background);
            let wifi = with(|l| l.prefs.wifi);
            let fp = if wifi { places::here() } else { Vec::new() };
            note(&path, at, fp);
            nudge();
        })
        .ok();
}

/// A photo's moment noted with its fingerprint: the records, the sidecar.
pub fn note(path: &Path, at: chrono::NaiveDateTime, fp: Vec<places::Ap>) {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
    let when = unix(at);
    let tag = crate::schedule::latest().and_then(|v| event::tag(&v, when, &at.format("%Y-%m-%d").to_string()));
    with(|l| {
        l.note_taken(&name, when, fp, tag.as_ref());
        l.save();
    });
}

/// A photo deleted: its sidecar and its records go too.
pub fn forget(path: &Path) {
    std::fs::remove_file(meta::sidecar(path)).ok();
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) {
        with(|l| {
            l.forget(&name);
            l.save();
        });
    }
}

/// At the shell's start: the work, if any waits.
pub fn start() {
    let waiting = with(|l| !l.batches.is_empty() || l.records.values().any(|r| r.caption.waiting() || r.town.waiting()));
    if waiting {
        nudge();
    }
}

/// The album and the records made to agree (new names, edits, photos gone).
pub fn reconcile() {
    let photos = crate::photos::list();
    let names: Vec<(String, i64)> = photos.iter().map(|p| (p.name.clone(), unix(p.taken))).collect();
    with(|l| {
        if l.reconcile(&names) {
            l.save();
        }
    });
}

// ----------------------------------------------------------------- worker ---

static RUNNING: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Wake the work (starting it if it is not running).
pub fn nudge() {
    if cfg!(test) {
        // The tests drive each step themselves.
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        let (m, c) = &WAKE;
        *m.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        c.notify_all();
        return;
    }
    let spawned = std::thread::Builder::new().name("photolog".into()).stack_size(512 * 1024).spawn(work);
    if spawned.is_err() {
        RUNNING.store(false, Ordering::Release);
    }
}

/// What the work is doing, for Settings › Photos.
static STATUS: Mutex<String> = Mutex::new(String::new());

pub fn status() -> String {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_status(words: &str) {
    if let Ok(mut s) = STATUS.lock() {
        if *s != words {
            *s = words.to_owned();
            changed();
        }
    }
}

fn work() {
    crate::sched::enter(crate::sched::Class::Background);
    loop {
        crate::deep::rest();
        reconcile();
        let now = chrono::Utc::now().timestamp();
        let cond = caption::Conditions::now();
        let api = caption::Api::from_keys();
        let c = caption::step(ledger(), api.as_ref(), now, cond);
        let t = locate::step(ledger(), locate::BEACONDB, crate::ai::demo(), now, cond);
        set_status(&match &c {
            caption::Did::Waits(why) => (*why).to_owned(),
            caption::Did::Sent(n) => format!("{n} sent to claude"),
            caption::Did::Polled => "waiting for claude's captions".into(),
            caption::Did::Took(n) => format!("{n} captioned"),
            caption::Did::Failed(why) => format!("trying again later · {why}"),
            caption::Did::Nothing(_) => String::new(),
        });
        let busy = |d: &caption::Did| matches!(d, caption::Did::Sent(_) | caption::Did::Took(_));
        if busy(&c) || busy(&t) {
            continue;
        }
        // Nothing done this round: until the next thing is due, or a nudge.
        let (pending, next) = with(|l| {
            let pending = !l.batches.is_empty() || l.records.values().any(|r| r.caption.waiting() || r.town.waiting());
            let next = [caption::next_due(l), if let caption::Did::Nothing(n) = t { n } else { None }].into_iter().flatten().min();
            (pending, next)
        });
        let stuck = |d: &caption::Did| matches!(d, caption::Did::Waits(w) if !w.starts_with("waiting"));
        let blocked_for_good = stuck(&c) && (stuck(&t) || !with(|l| l.records.values().any(|r| r.town.waiting())));
        if !pending || blocked_for_good {
            RUNNING.store(false, Ordering::Release);
            return;
        }
        let wait = match (&c, next) {
            (caption::Did::Waits(_), _) | (caption::Did::Failed(_), None) => Duration::from_secs(300),
            (_, Some(at)) => Duration::from_secs((at - now).clamp(20, 1800) as u64),
            _ => Duration::from_secs(300),
        };
        let (m, cv) = &WAKE;
        let guard = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut woken, _) = cv.wait_timeout_while(guard, wait, |w| !*w).unwrap_or_else(std::sync::PoisonError::into_inner);
        *woken = false;
    }
}

// --------------------------------------------------------------- settings ---

/// Settings' one line for Photos.
pub fn summary() -> String {
    with(|l| match (l.prefs.captions, l.prefs.town) {
        (true, true) => "captions · towns".into(),
        (true, false) => "captions on".into(),
        (false, true) => "towns on".into(),
        _ => format!("{} places", l.places.list.len()),
    })
}

/// The simulator's album as the build log would have it: captions, places
/// and the event on the demo photos, once.
#[cfg(not(target_arch = "aarch64"))]
pub fn seed_demo() {
    if crate::sys::on_handheld() {
        return;
    }
    let photos = crate::photos::list();
    let seeded = dir().join(".seeded");
    if seeded.exists() || photos.is_empty() {
        return;
    }
    with(|l| {
        let shop = places::demo_place("shop");
        if l.places.list.is_empty() {
            l.places.name("Shop", &shop);
        }
        for (k, p) in photos.iter().enumerate() {
            let fp = if k % 3 == 2 { places::demo_place("pit") } else { shop.clone() };
            let when = unix(p.taken);
            l.records.insert(p.name.clone(), store::Record { at: when, fp: fp.clone(), caption: store::Job::Done, cost: 4_100, ..Default::default() });
            let mut about = meta::read(&p.path);
            about.caption = caption::DEMO_CAPTIONS[k % caption::DEMO_CAPTIONS.len()].into();
            about.by = caption::MODEL.into();
            about.place = l.places.best(&fp).map(|x| x.name.clone()).unwrap_or_default();
            if about.place.is_empty() {
                about.event = "Finger Lakes Regional".into();
                about.venue = "Rochester, NY".into();
                about.context = "before Q31".into();
            }
            meta::write(&p.path, &about).ok();
        }
        l.save();
    });
    std::fs::create_dir_all(dir()).ok();
    std::fs::write(seeded, b"").ok();
}
