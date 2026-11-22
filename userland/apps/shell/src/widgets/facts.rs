//! What widgets show, kept, and read again on a policy of each kind's own.
//! Nothing here reads the card or the kernel inside a frame.
//!
//! - **In memory, on events**: the robot's values, the schedule and the
//!   logs (each says when it changes, by a generation), music, the timers
//!   and the stopwatch are read as they are drawn. So is the battery: it is
//!   the shell's own reading ([`Facts::battery`]), the one the island and
//!   the lock screen show, never a reading of the widget's own.
//! - **In memory, worked out**: reminders and Spotlight's picks, every 30 s
//!   and every minute.
//! - **On a thread**: what is on the card or in the kernel (the system's
//!   figures every 5 s, the forecast every minute, the pinned note, the
//!   world's places and their offsets, the photos, the charge's history) is
//!   asked of a reading thread of the widgets' own, no sooner than its
//!   period, and taken in as it answers; a photo is decoded on a thread of
//!   its own. Until a kind's first answer its widget shows only its name.
//! - [`Facts::due`] says when what a widget shows is next due, for Home to
//!   draw again then. What an app may have changed is asked again when Home
//!   is back in front ([`Facts::invalidate`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use detent::{role, Animated};

use super::Kind;
use crate::photos::Picture;
use crate::robot::Robot;

const HISTORY_EVERY: Duration = Duration::from_secs(300);
const SYSTEM_EVERY: Duration = Duration::from_secs(5);
/// As often as Home's Weather tile reads it, so the two say the same.
const WEATHER_EVERY: Duration = Duration::from_secs(60);
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
    pub before: Option<(PathBuf, Arc<Picture>)>,
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

// ------------------------------------------------------- reading thread ---

/// What the reading thread is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Ask {
    System,
    Weather { h24: bool },
    Note,
    Places,
    Photos,
    History,
}

impl Ask {
    /// The widgets that wait on it.
    fn of(self) -> &'static [Kind] {
        match self {
            Ask::System => &[Kind::System],
            Ask::Weather { .. } => &[Kind::Weather],
            Ask::Note => &[Kind::Note],
            Ask::Places => &[Kind::World, Kind::Clock],
            Ask::Photos => &[Kind::Photos],
            Ask::History => &[],
        }
    }

    /// One in flight at a time of each sort (the forecast whatever its clock).
    fn sort(self) -> u8 {
        match self {
            Ask::System => 0,
            Ask::Weather { .. } => 1,
            Ask::Note => 2,
            Ask::Places => 3,
            Ask::Photos => 4,
            Ask::History => 5,
        }
    }
}

/// What it answers.
enum Answer {
    System(System),
    Weather(Option<crate::weather::Glance>, bool),
    Note(Option<NoteFact>),
    /// The world's places with their offsets, and the handheld's own zone
    /// with its offset.
    Places(Vec<(String, Option<i64>)>, (String, Option<i64>)),
    Photos(Vec<crate::photos::Photo>),
    History(Vec<crate::power::Point>),
}

/// The widgets' reading thread: started the first time one asks, it reads
/// one thing at a time and ends with Home.
struct Reader {
    asks: Sender<Ask>,
    answers: Receiver<Answer>,
}

impl Reader {
    fn start() -> Option<Reader> {
        let (asks, asked) = std::sync::mpsc::channel::<Ask>();
        let (answer, answers) = std::sync::mpsc::channel::<Answer>();
        std::thread::Builder::new()
            .name("widget-facts".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut cpu_before = None;
                for ask in asked {
                    let read = match ask {
                        Ask::System => Answer::System(read_system(&mut cpu_before)),
                        Ask::Weather { h24 } => Answer::Weather(crate::weather::glance(h24), h24),
                        Ask::Note => Answer::Note(read_note()),
                        Ask::Places => {
                            let places = crate::clock::load_places().into_iter().map(|z| {
                                let offset = crate::clock::offset_now(&z);
                                (z, offset)
                            });
                            let here = crate::device::zone();
                            let at = crate::clock::offset_now(&here);
                            Answer::Places(places.collect(), (here, at))
                        }
                        Ask::Photos => Answer::Photos(crate::photos::list()),
                        Ask::History => Answer::History(read_history()),
                    };
                    if answer.send(read).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Reader { asks, answers })
    }
}

#[derive(Default)]
pub struct Facts {
    /// The shell's battery reading, as Home was last given it.
    pub battery: Option<crate::power::Reading>,
    pub history: Vec<crate::power::Point>,
    pub system: System,
    pub weather: Option<crate::weather::Glance>,
    pub view: Option<crate::schedule::View>,
    pub reminders: Vec<crate::reminders::Upcoming>,
    /// The world clock's places, and each one's offset from UTC once known.
    pub places: Vec<(String, Option<i64>)>,
    /// The handheld's own zone, and its offset from UTC.
    pub here: Option<(String, Option<i64>)>,
    pub note: Option<NoteFact>,
    /// Spotlight's picks of late: icon and name.
    pub actions: Vec<(String, String)>,
    pub photo: PhotoState,
    /// The newest match log with its summary: its title, and the summary.
    pub last_match: Option<(String, crate::logs::Digest)>,
    logs_seen: Option<u64>,
    robot_seen: Option<u64>,
    /// When each kind was last read, or asked for.
    read: HashMap<Kind, Instant>,
    history_at: Option<Instant>,
    photos_at: Option<Instant>,
    schedule_seen: Option<u64>,
    weather_h24: Option<bool>,
    reader: Option<Reader>,
    /// Asked of the reading thread and not answered yet, by sort.
    waiting: Vec<Ask>,
    /// The kinds the reading thread has answered for at least once.
    answered: Vec<Kind>,
    /// Pictures made once and laid down each frame (pictures.rs).
    pub pictures: super::pictures::Pictures,
}

fn stale(at: Option<&Instant>, every: Duration) -> bool {
    at.is_none_or(|at| at.elapsed() >= every)
}

impl Facts {
    /// What `kind` shows, brought up to date where it is due: what is in
    /// memory read now, the rest asked of the reading thread (never waited
    /// for). `want` is the size a photo is drawn at, in pixels; `turn`
    /// whether photos may move on (not with the battery saver, nor on a
    /// battery under a fifth).
    pub fn prepare(&mut self, kind: Kind, robot: &Robot, h24: bool, want: (usize, usize), turn: bool) {
        self.take_answers();
        let every = match kind {
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
            // The logs' generation looks at their folder: only once the
            // robot has seen something change.
            Kind::LastMatch if self.robot_seen != Some(robot.seen()) => {
                self.robot_seen = Some(robot.seen());
                let generation = robot.logs.generation();
                if self.logs_seen != Some(generation) {
                    self.last_match = robot.logs.list().into_iter().find_map(|row| row.digest.clone().map(|d| (row.title(), d)));
                    self.logs_seen = Some(generation);
                }
            }
            Kind::Battery if stale(self.history_at.as_ref(), HISTORY_EVERY) => {
                self.history_at = Some(Instant::now());
                self.ask(Ask::History);
            }
            Kind::System if due => {
                self.read.insert(kind, Instant::now());
                self.ask(Ask::System);
            }
            Kind::Weather if due => {
                self.read.insert(kind, Instant::now());
                self.weather_h24 = Some(h24);
                self.ask(Ask::Weather { h24 });
            }
            Kind::Reminders if due => {
                self.reminders = crate::reminders::upcoming(6);
                self.read.insert(kind, Instant::now());
            }
            Kind::Note if due => {
                self.read.insert(kind, Instant::now());
                self.ask(Ask::Note);
            }
            Kind::Actions if due => {
                self.actions = crate::spotlight::index().recent(4).into_iter().map(|hit| (hit.item.icon.into_owned(), hit.item.title)).collect();
                self.read.insert(kind, Instant::now());
            }
            Kind::World | Kind::Clock if due => {
                self.read.insert(Kind::World, Instant::now());
                self.read.insert(Kind::Clock, Instant::now());
                self.ask(Ask::Places);
            }
            Kind::Photos => self.prepare_photo(want, turn),
            _ => {}
        }
    }

    /// Whether `kind` has anything to show yet: its first answer has come
    /// (what is read in memory always has).
    pub fn known(&self, kind: Kind) -> bool {
        match kind {
            Kind::System | Kind::Weather | Kind::Note | Kind::World | Kind::Photos => self.answered.contains(&kind),
            _ => true,
        }
    }

    /// When what `kind` shows is next due to be read, for Home to draw again
    /// then; none for what is read as it changes.
    pub fn due(&self, kind: Kind) -> Option<Instant> {
        let after = |every: Duration| self.read.get(&kind).map(|at| *at + every);
        if self.waiting.iter().any(|a| a.of().contains(&kind)) || (kind == Kind::Photos && self.photo.loading.is_some()) {
            return Some(Instant::now() + LOOK_AGAIN);
        }
        match kind {
            Kind::System => after(SYSTEM_EVERY),
            Kind::Weather => after(WEATHER_EVERY),
            Kind::Reminders => after(REMINDERS_EVERY),
            Kind::Note => after(NOTE_EVERY),
            Kind::Actions => after(ACTIONS_EVERY),
            Kind::World | Kind::Clock => after(PLACES_EVERY),
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
        self.robot_seen = None;
    }

    /// Something just changed what `kind` shows (Enter on it): read at once.
    pub fn changed(&mut self, kind: Kind) {
        self.read.remove(&kind);
    }

    /// The photo's fade. True while it moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.photo.fade.tick(dt)
    }

    /// Ask the reading thread, unless that is being read already.
    fn ask(&mut self, ask: Ask) {
        if self.waiting.iter().any(|a| a.sort() == ask.sort()) {
            return;
        }
        if self.reader.is_none() {
            self.reader = Reader::start();
        }
        if self.reader.as_ref().is_some_and(|r| r.asks.send(ask).is_ok()) {
            self.waiting.push(ask);
        } else {
            // It has gone (it never panics on purpose): another next time.
            self.reader = None;
        }
    }

    /// Whatever the reading thread has answered, taken in.
    fn take_answers(&mut self) {
        let Some(reader) = &self.reader else { return };
        loop {
            let answer = match reader.answers.try_recv() {
                Ok(answer) => answer,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.reader = None;
                    self.waiting.clear();
                    return;
                }
            };
            let ask = match answer {
                Answer::System(system) => {
                    self.system = system;
                    Ask::System
                }
                Answer::Weather(weather, h24) => {
                    self.weather = weather;
                    Ask::Weather { h24 }
                }
                Answer::Note(note) => {
                    self.note = note;
                    Ask::Note
                }
                Answer::Places(places, here) => {
                    self.places = places;
                    self.here = Some(here);
                    Ask::Places
                }
                Answer::Photos(list) => {
                    let p = &mut self.photo;
                    let shown = p.list.get(p.index).map(|ph| ph.path.clone());
                    p.list = list;
                    p.index = shown.and_then(|s| p.list.iter().position(|ph| ph.path == s)).unwrap_or(0);
                    Ask::Photos
                }
                Answer::History(history) => {
                    self.history = history;
                    Ask::History
                }
            };
            self.waiting.retain(|a| a.sort() != ask.sort());
            for kind in ask.of() {
                if !self.answered.contains(kind) {
                    self.answered.push(*kind);
                }
            }
        }
    }

    fn prepare_photo(&mut self, want: (usize, usize), turn: bool) {
        if stale(self.photos_at.as_ref(), PHOTOS_EVERY) {
            self.photos_at = Some(Instant::now());
            self.ask(Ask::Photos);
        }
        let low = self.battery.as_ref().is_some_and(|b| b.flow.on_battery() && b.percent < 20);
        let p = &mut self.photo;
        if p.list.is_empty() {
            p.shown = None;
            p.before = None;
            return;
        }
        // A decode finished: it fades in over the one before.
        if let Some((path, size, rx)) = &p.loading {
            match rx.try_recv() {
                Ok(Some(pic)) => {
                    p.before = p.shown.take().map(|(path, _, pic)| (path, pic));
                    p.shown = Some((path.clone(), *size, Arc::new(pic)));
                    p.loading = None;
                    if p.before.is_some() {
                        p.fade.jump(0.0);
                        p.fade.set(1.0);
                    }
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    // That one does not read: the next, another time.
                    p.loading = None;
                    p.list.remove(p.index.min(p.list.len() - 1));
                    p.index = 0;
                }
                Err(TryRecvError::Empty) => return,
            }
        }
        if p.list.is_empty() {
            return;
        }
        let turn_due = p.turned.is_some_and(|at| at.elapsed() >= PHOTO_TURNS);
        if turn_due && turn && !low && p.list.len() > 1 && p.loading.is_none() {
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

    /// What `kind` shows, read and waited for (a photo decoded too): for the
    /// galleries and the tests, which draw it once and must see it whole.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn ready(&mut self, kind: Kind, robot: &Robot, h24: bool, want: (usize, usize)) {
        if kind == Kind::Battery && self.battery.is_none() {
            self.battery = crate::power::reading_now();
        }
        // A song's cover, decoded.
        if let Some(cover) = crate::nowplaying::get().and_then(|np| np.cover).filter(|_| kind == Kind::Music) {
            for _ in 0..300 {
                if crate::liveactivity::cover(&cover).is_some() || !crate::liveactivity::cover_pending(&cover) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        for _ in 0..300 {
            self.prepare(kind, robot, h24, want, false);
            let pending = self.waiting.iter().any(|a| a.of().contains(&kind) || (kind == Kind::Battery && *a == Ask::History))
                || (kind == Kind::Photos && (self.photo.loading.is_some() || (self.photo.shown.is_none() && !self.photo.list.is_empty())));
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

    /// What `kind` shows, set by hand (the tests): kept
    /// as it is until its period is up.
    #[cfg(test)]
    pub fn hold(&mut self, kind: Kind) {
        self.read.insert(kind, Instant::now());
        if !self.answered.contains(&kind) {
            self.answered.push(kind);
        }
    }
}

/// The handheld's vital signs, read from `/proc`, `/sys` and the data
/// partition; the processor against `before`, its last reading.
fn read_system(before: &mut Option<(u64, u64)>) -> System {
    let file = |path: &str| std::fs::read_to_string(path).ok();
    let memory = file("/proc/meminfo").and_then(|t| crate::monitor::meminfo(&t)).map(|(used, total)| (used as f32 / 1024.0, total as f32 / 1024.0));
    let cpu = file("/proc/stat").and_then(|t| crate::monitor::cpu_times(&t).first().copied()).map(|now| {
        // Against the last reading; the first, since the handheld started.
        let (before_busy, before_total) = before.unwrap_or((0, 0));
        *before = Some(now);
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

/// The charge over the last day, as the Battery page draws it.
fn read_history() -> Vec<crate::power::Point> {
    let now = chrono::Utc::now().timestamp();
    let mut history = if cfg!(target_os = "linux") { crate::power::history() } else { crate::power::demo_history(now) };
    history.retain(|p| p.0 >= now - 24 * 3600);
    history
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
