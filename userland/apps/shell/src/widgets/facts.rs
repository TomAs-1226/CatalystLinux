//! What widgets show, read only for widgets in sight and kept: each kind on
//! a refresh policy of its own.
//!
//! - **Events**: the robot's values and the schedule are in memory and say
//!   when they change (the schedule's generation); music, the timers and the
//!   stopwatch are in memory too. These are read as they are drawn.
//! - **Timestamps**: what is on the card or in the kernel is read again no
//!   sooner than its period (the battery every 30 s, the system every 5 s,
//!   the forecast every 5 minutes…) and only while its widget is drawn;
//!   [`Facts::due`] says when, for Home to draw again then. What an app may
//!   have changed is read afresh when Home is back in front
//!   ([`Facts::invalidate`]).
//! - The slow work (a photo decoded, the world clock's offsets asked of the
//!   zone database) runs on a thread of its own, never in a frame.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use detent::{role, Animated};

use super::Kind;
use crate::photos::Picture;
use crate::robot::Robot;

const BATTERY_EVERY: Duration = Duration::from_secs(30);
const HISTORY_EVERY: Duration = Duration::from_secs(300);
const SYSTEM_EVERY: Duration = Duration::from_secs(5);
const WEATHER_EVERY: Duration = Duration::from_secs(300);
const REMINDERS_EVERY: Duration = Duration::from_secs(30);
const NOTE_EVERY: Duration = Duration::from_secs(120);
const ACTIONS_EVERY: Duration = Duration::from_secs(60);
const PLACES_EVERY: Duration = Duration::from_secs(600);
const PHOTOS_EVERY: Duration = Duration::from_secs(600);
/// How long a photo stays before the next one fades in.
pub const PHOTO_TURNS: Duration = Duration::from_secs(600);
/// While something is being read on a thread: how soon to look for it.
const LOOK_AGAIN: Duration = Duration::from_millis(120);

/// The handheld's vital signs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct System {
    /// Memory in use and in all, MB.
    pub memory: Option<(f32, f32)>,
    /// How busy the processor is, 0 to 1.
    pub cpu: Option<f32>,
    /// The processor's temperature, °C.
    pub celsius: Option<f32>,
    /// The data partition, used and in all, GB.
    pub storage: Option<(f32, f32)>,
}

/// The note pinned to the widget.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteFact {
    pub title: String,
    pub lines: Vec<String>,
    pub file: PathBuf,
    pub pinned: bool,
}

/// The photo widget's photo: the one shown, the one fading out, the next
/// being decoded.
pub struct PhotoState {
    pub list: Vec<crate::photos::Photo>,
    pub index: usize,
    pub shown: Option<(PathBuf, (usize, usize), Arc<Picture>)>,
    pub before: Option<Arc<Picture>>,
    /// The one shown fading in over the one before: 0 to 1.
    pub fade: Animated,
    turned: Option<Instant>,
    loading: Option<(PathBuf, (usize, usize), Receiver<Option<Picture>>)>,
}

impl Default for PhotoState {
    fn default() -> Self {
        PhotoState { list: Vec::new(), index: 0, shown: None, before: None, fade: Animated::new(1.0, role::smooth()).with_rest(0.004), turned: None, loading: None }
    }
}

#[derive(Default)]
pub struct Facts {
    pub battery: Option<crate::power::Reading>,
    pub history: Vec<crate::power::Point>,
    pub system: System,
    pub weather: Option<crate::weather::Glance>,
    pub view: Option<crate::schedule::View>,
    pub reminders: Vec<crate::reminders::Upcoming>,
    /// The world clock's places, and each one's offset from UTC once known.
    pub places: Vec<(String, Option<i64>)>,
    pub note: Option<NoteFact>,
    /// Spotlight's picks of late: icon and name.
    pub actions: Vec<(String, String)>,
    pub photo: PhotoState,
    /// The newest match log with its summary: its title, and the summary.
    pub last_match: Option<(String, crate::logs::Digest)>,
    logs_seen: Option<u64>,
    /// When each kind was last read.
    read: HashMap<Kind, Instant>,
    history_at: Option<Instant>,
    photos_at: Option<Instant>,
    cpu_before: Option<(u64, u64)>,
    schedule_seen: Option<u64>,
    places_reading: Option<Receiver<Vec<(String, Option<i64>)>>>,
    weather_h24: Option<bool>,
}

fn stale(at: Option<&Instant>, every: Duration) -> bool {
    at.is_none_or(|at| at.elapsed() >= every)
}

impl Facts {
    /// Read what `kind` shows, if it is due. `want` is the size a photo is
    /// drawn at, in pixels; `turn` whether photos may move on (not with the
    /// battery saver; a low battery is looked at as a turn comes due).
    pub fn prepare(&mut self, kind: Kind, robot: &Robot, h24: bool, want: (usize, usize), turn: bool) {
        let every = match kind {
            Kind::Battery => BATTERY_EVERY,
            Kind::System => SYSTEM_EVERY,
            Kind::Weather => WEATHER_EVERY,
            Kind::Reminders => REMINDERS_EVERY,
            Kind::Note => NOTE_EVERY,
            Kind::Actions => ACTIONS_EVERY,
            Kind::World | Kind::Clock => PLACES_EVERY,
            _ => Duration::MAX,
        };
        let due = stale(self.read.get(&kind), every) || (kind == Kind::Weather && self.weather_h24 != Some(h24));
        match kind {
            Kind::NextMatch => {
                let generation = robot.schedule.generation();
                if self.schedule_seen != Some(generation) {
                    self.view = robot.schedule.view();
                    self.schedule_seen = Some(generation);
                }
            }
            Kind::LastMatch => {
                let generation = robot.logs.generation();
                if self.logs_seen != Some(generation) {
                    self.last_match = robot.logs.list().into_iter().find_map(|row| row.digest.clone().map(|d| (row.title(), d)));
                    self.logs_seen = Some(generation);
                }
            }
            Kind::Battery => {
                if due {
                    self.battery = crate::power::reading_now();
                    self.read.insert(kind, Instant::now());
                }
                if stale(self.history_at.as_ref(), HISTORY_EVERY) {
                    let now = chrono::Utc::now().timestamp();
                    self.history = if cfg!(target_os = "linux") { crate::power::history() } else { crate::power::demo_history(now) };
                    self.history.retain(|p| p.0 >= now - 24 * 3600);
                    self.history_at = Some(Instant::now());
                }
            }
            Kind::System if due => {
                self.system = self.read_system();
                self.read.insert(kind, Instant::now());
            }
            Kind::Weather if due => {
                self.weather = crate::weather::glance(h24);
                self.weather_h24 = Some(h24);
                self.read.insert(kind, Instant::now());
            }
            Kind::Reminders if due => {
                self.reminders = crate::reminders::upcoming(6);
                self.read.insert(kind, Instant::now());
            }
            Kind::Note if due => {
                self.note = read_note();
                self.read.insert(kind, Instant::now());
            }
            Kind::Actions if due => {
                self.actions = crate::spotlight::index().recent(4).into_iter().map(|hit| (hit.item.icon.into_owned(), hit.item.title)).collect();
                self.read.insert(kind, Instant::now());
            }
            Kind::World | Kind::Clock => self.prepare_places(due),
            Kind::Photos => self.prepare_photo(want, turn),
            _ => {}
        }
    }

    /// When what `kind` shows is next due to be read, for Home to draw again
    /// then; none for what is read as it changes.
    pub fn due(&self, kind: Kind) -> Option<Instant> {
        let after = |every: Duration| self.read.get(&kind).map(|at| *at + every);
        match kind {
            Kind::Battery => after(BATTERY_EVERY),
            Kind::System => after(SYSTEM_EVERY),
            Kind::Weather => after(WEATHER_EVERY),
            Kind::Reminders => after(REMINDERS_EVERY),
            Kind::Note => after(NOTE_EVERY),
            Kind::Actions => after(ACTIONS_EVERY),
            Kind::World | Kind::Clock if self.places_reading.is_some() => Some(Instant::now() + LOOK_AGAIN),
            Kind::World | Kind::Clock => after(PLACES_EVERY),
            Kind::Photos if self.photo.loading.is_some() => Some(Instant::now() + LOOK_AGAIN),
            Kind::Photos => self.photo.turned.map(|at| at + PHOTO_TURNS),
            _ => None,
        }
    }

    /// Back in front after an app: what it may have changed is read again.
    pub fn invalidate(&mut self) {
        for kind in [Kind::Weather, Kind::Reminders, Kind::Note, Kind::Actions, Kind::World, Kind::Clock] {
            self.read.remove(&kind);
        }
        self.photos_at = None;
    }

    /// Something just changed what `kind` shows (Enter on it): read at once.
    pub fn changed(&mut self, kind: Kind) {
        self.read.remove(&kind);
    }

    /// The photo's fade. True while it moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.photo.fade.tick(dt)
    }

    fn read_system(&mut self) -> System {
        let file = |path: &str| std::fs::read_to_string(path).ok();
        let memory = file("/proc/meminfo").and_then(|t| crate::monitor::meminfo(&t)).map(|(used, total)| (used as f32 / 1024.0, total as f32 / 1024.0));
        let cpu = file("/proc/stat").and_then(|t| crate::monitor::cpu_times(&t).first().copied()).map(|now| {
            // Against the last reading; the first, since the handheld started.
            let (before_busy, before_total) = self.cpu_before.unwrap_or((0, 0));
            self.cpu_before = Some(now);
            let total = now.1.saturating_sub(before_total);
            if total == 0 {
                0.0
            } else {
                (now.0.saturating_sub(before_busy) as f32 / total as f32).clamp(0.0, 1.0)
            }
        });
        let celsius = file("/sys/class/thermal/thermal_zone0/temp").and_then(|t| t.trim().parse::<f32>().ok()).map(|milli| milli / 1000.0);
        let system = System { memory, cpu, celsius, storage: storage() };
        // A computer has none of these to read: show what the handheld would.
        if !cfg!(target_os = "linux") && system == System::default() {
            return System { memory: Some((46.0, 427.0)), cpu: Some(0.12), celsius: Some(48.0), storage: Some((1.2, 29.0)) };
        }
        system
    }

    fn prepare_places(&mut self, due: bool) {
        if let Some(rx) = &self.places_reading {
            match rx.try_recv() {
                Ok(places) => {
                    self.places = places;
                    self.places_reading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.places_reading = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !due || self.places_reading.is_some() {
            return;
        }
        let zones = crate::clock::load_places();
        // The places at once; their times once the zone database has said.
        if self.places.iter().map(|(z, _)| z).ne(zones.iter()) {
            self.places = zones.iter().map(|z| (z.clone(), None)).collect();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::thread::Builder::new().name("world-clock".into()).stack_size(128 * 1024).spawn(move || {
            tx.send(zones.iter().map(|z| (z.clone(), crate::clock::offset_now(z))).collect()).ok();
        });
        if started.is_ok() {
            self.places_reading = Some(rx);
        }
        self.read.insert(Kind::World, Instant::now());
        self.read.insert(Kind::Clock, Instant::now());
    }

    fn prepare_photo(&mut self, want: (usize, usize), turn: bool) {
        let p = &mut self.photo;
        if stale(self.photos_at.as_ref(), PHOTOS_EVERY) {
            let shown = p.list.get(p.index).map(|ph| ph.path.clone());
            p.list = crate::photos::list();
            p.index = shown.and_then(|s| p.list.iter().position(|ph| ph.path == s)).unwrap_or(0);
            self.photos_at = Some(Instant::now());
        }
        if p.list.is_empty() {
            p.shown = None;
            p.before = None;
            return;
        }
        // A decode finished: it fades in over the one before.
        if let Some((path, size, rx)) = &p.loading {
            match rx.try_recv() {
                Ok(Some(pic)) => {
                    p.before = p.shown.take().map(|(_, _, pic)| pic);
                    p.shown = Some((path.clone(), *size, Arc::new(pic)));
                    p.loading = None;
                    if p.before.is_some() {
                        p.fade.jump(0.0);
                        p.fade.set(1.0);
                    }
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // That one does not read: the next, another time.
                    p.loading = None;
                    p.list.remove(p.index.min(p.list.len() - 1));
                    p.index = 0;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if p.list.is_empty() {
            return;
        }
        let turn_due = p.turned.is_some_and(|at| at.elapsed() >= PHOTO_TURNS);
        // Power-aware: never with the battery saver on, nor on a battery
        // under a fifth (read only now, when a turn is due).
        let low = || crate::power::reading_now().is_some_and(|b| b.flow.on_battery() && b.percent < 20);
        if turn_due && turn && p.list.len() > 1 && p.loading.is_none() && !low() {
            p.index = (p.index + 1) % p.list.len();
            p.turned = Some(Instant::now());
        }
        p.turned.get_or_insert_with(Instant::now);
        let path = p.list[p.index.min(p.list.len() - 1)].path.clone();
        let fits = p.shown.as_ref().is_some_and(|(shown, size, _)| *shown == path && size.0 >= want.0 && size.1 >= want.1);
        if fits || p.loading.is_some() || want.0 == 0 || want.1 == 0 {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let file = path.clone();
        let started = std::thread::Builder::new().name("widget-photo".into()).stack_size(512 * 1024).spawn(move || {
            tx.send(crate::photos::decode(&file, want.0, want.1).ok().map(|pic| pic.cover(want.0, want.1))).ok();
        });
        if started.is_ok() {
            p.loading = Some((path, want, rx));
        }
    }

    /// Read what `kind` shows and wait for what is read on a thread (a
    /// photo, the world's offsets): the galleries and the tests, which draw
    /// it once and must see it whole.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn ready(&mut self, kind: Kind, robot: &Robot, h24: bool, want: (usize, usize)) {
        for _ in 0..300 {
            self.prepare(kind, robot, h24, want, false);
            let pending = match kind {
                Kind::Photos => self.photo.loading.is_some() || (self.photo.shown.is_none() && !self.photo.list.is_empty()),
                Kind::World | Kind::Clock => self.places_reading.is_some(),
                _ => false,
            };
            if !pending {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if kind == Kind::Photos {
            self.photo.fade.jump(1.0);
            self.photo.before = None;
        }
    }

    /// What `kind` shows, set by hand (the galleries and the tests): kept
    /// as it is until its period is up.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn hold(&mut self, kind: Kind) {
        self.read.insert(kind, Instant::now());
    }
}

/// The data partition, used and in all, in GB.
#[cfg(target_os = "linux")]
fn storage() -> Option<(f32, f32)> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a plain statvfs into a zeroed struct of the right type.
    if unsafe { libc::statvfs(c"/data".as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let gb = |blocks: u64| (blocks as f64 * stat.f_frsize as f64 / 1e9) as f32;
    let (total, free) = (gb(stat.f_blocks as u64), gb(stat.f_bavail as u64));
    (total > 0.0).then_some((total - free, total))
}

#[cfg(not(target_os = "linux"))]
fn storage() -> Option<(f32, f32)> {
    None
}

/// The note to show: the first pinned in Notes, else the glance page's own
/// note (`note.txt`, 0.6 and before), if either has words.
fn read_note() -> Option<NoteFact> {
    let lines_of = |text: &str| -> Vec<String> { text.lines().map(str::trim).filter(|l| !l.is_empty()).skip(1).take(8).map(str::to_owned).collect() };
    let dir = crate::notes::dir();
    if let Some(note) = crate::notes::load(&dir).into_iter().find(|n| n.pinned) {
        return Some(NoteFact { title: note.title.clone(), lines: lines_of(&note.text), file: dir.join(&note.file), pinned: true });
    }
    let file = note_file();
    let text = std::fs::read_to_string(&file).ok()?;
    text.lines().any(|l| !l.trim().is_empty()).then(|| NoteFact { title: crate::notes::title_of(&text), lines: lines_of(&text), file, pinned: false })
}

/// The glance page's own note: a plain text file on the data partition.
pub fn note_file() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        "/data/note.txt".into()
    } else {
        crate::config_dir().join("note.txt")
    }
}
