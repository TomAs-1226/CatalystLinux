//! Live activities: what is under way, shown wherever the handheld is
//! looked at, without opening anything. The island carries the most
//! important one, compact (a second sits beside it as a bubble, as on iOS);
//! the lock screen stacks the two most important as cards; StandBy gives
//! music a face of its own and shows the rest as they change.
//!
//! - **An activity** is data: its kind, who it is, how much it matters, a
//!   mark (an icon, a cover, a dot), a title and a line under it, the few
//!   characters the compact island holds, a value for its card, a meter (a
//!   fraction, or one that moves by itself from a time), and whether it has
//!   controls. Everything that shows one draws it from that alone
//!   ([`draw_compact`], [`draw_bubble`], [`draw_card`]).
//! - **Providers** turn what a module already keeps into activities, once a
//!   second while the screen is on, and never while it is off: music
//!   (`nowplaying`), Clock's timers and stopwatch, the next match and its
//!   queue (the schedule), the next reminder, downloads and installs (the
//!   system update, and [`task`] for the rest), the Assistant answering or
//!   its on-device model loading ([`task`]), and the robot while it runs.
//! - **Which shows**: the highest priority is the island's; when one appears
//!   or changes (a song, queue time, a download done) the island opens to its
//!   card for a few seconds and closes again. They never wake the screen nor
//!   keep it on: what already wakes it (a reminder due, queue time) still
//!   does, through the island's messages.
//! - **Music on the island** (livemusic.rs): compact, its cover and four
//!   equalizer bars dancing with the song while the island shows them (flat
//!   when paused), and the sleep timer's countdown while it runs; open, the
//!   song, its progress in its cover's colour, its keys, and the line being
//!   sung on a row of its own. A new song rises in as the last one goes, and
//!   the colour eases across, on Detent springs ([`Look`]).
//!
//! Priorities, highest first: queue now (90), a timer's last minute (85), a
//! timer (80), the queue coming (75), a reminder or a match soon (70), music
//! (60), the robot in a match (55), the Assistant (50), downloads (40), the
//! stopwatch (30).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::gfx::Gfx;
use crate::photos::Picture;

// ------------------------------------------------------------------ data ---

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Music,
    Timer,
    Stopwatch,
    Match,
    Reminder,
    Download,
    Assistant,
    Robot,
    /// AirPods connecting: their battery, for a moment (airpods.rs).
    Headphones,
    /// Claude working in a session on the laptop, or waiting on an answer
    /// (claude/).
    Claude,
}

/// What leads an activity: an icon in its tint, a cover, or a coloured dot.
#[derive(Clone, PartialEq, Debug)]
pub enum Mark {
    Icon(&'static str, Tint),
    Cover(PathBuf, &'static str),
}

/// An activity's colour, as a role (so it follows the palette and night).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tint {
    Signal,
    Ice,
    Amber,
    Fixed(Color),
}

impl Tint {
    pub fn color(self, g: &Gfx) -> Color {
        let t = g.tokens;
        match self {
            Tint::Signal => t.signal,
            Tint::Ice => g.good(),
            Tint::Amber => t.amber_ink,
            Tint::Fixed(c) => c,
        }
    }
}

/// How far along it is: still, or moving by itself from a moment.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Meter {
    Fraction(f32),
    /// `from` at `at`, going up by `rate` a second.
    Moving { from: f32, rate: f32, at: Instant },
}

impl Meter {
    pub fn now(&self) -> f32 {
        match *self {
            Meter::Fraction(f) => f.clamp(0.0, 1.0),
            Meter::Moving { from, rate, at } => (from + rate * at.elapsed().as_secs_f32()).clamp(0.0, 1.0),
        }
    }

    pub fn moving(&self) -> bool {
        matches!(self, Meter::Moving { rate, .. } if *rate != 0.0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Activity {
    pub kind: Kind,
    /// Who it is, among others of its kind ("music", "update", "task 3").
    pub id: String,
    pub priority: u8,
    pub mark: Mark,
    pub title: String,
    pub detail: String,
    /// The compact island's few characters: "4:59", "queue 8:12".
    pub short: String,
    /// A value large on its card: "4:59".
    pub value: Option<String>,
    pub meter: Option<Meter>,
    /// Music: playing (true) or paused, and controls on its card.
    pub playing: Option<bool>,
    /// It needs doing now (queue now, a timer's last minute): its value and
    /// meter in the signal colour.
    pub urgent: bool,
    /// What counts as it changing (the song, playing or paused, queue now,
    /// done): a new one opens the island to its card for a moment.
    pub state: String,
    /// Music: the palette its cover gives it (`bezel::hues`), if any.
    pub seed: Option<usize>,
    /// Music: the sleep timer's seconds left, while it runs.
    pub sleep: Option<f32>,
    /// Music: its synced lyrics and where the song is, for the line being
    /// sung on its card (Settings › Music › Lyrics on the island).
    pub sung: Option<Sung>,
}

/// A song's synced lyrics, and its place in them moving on by itself.
#[derive(Clone, Debug)]
pub struct Sung {
    pub lyrics: Arc<crate::music::lyrics::Lyrics>,
    /// Seconds into the song at `at`, moving on while `playing`.
    pub from: f32,
    pub at: Instant,
    pub playing: bool,
}

impl PartialEq for Sung {
    /// The same words, playing or not: where it is moves on by itself.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.lyrics, &other.lyrics) && self.playing == other.playing
    }
}

impl Sung {
    pub fn position(&self) -> f32 {
        self.from + if self.playing { self.at.elapsed().as_secs_f32() } else { 0.0 }
    }

    /// The line being sung (`true`), or before the singing and in its
    /// pauses, the one to come (`false`).
    pub fn line(&self) -> Option<(&str, bool)> {
        let lines = self.lyrics.lines()?;
        let at = crate::music::lyrics::line_at(lines, self.position());
        if let Some(l) = at.map(|i| &lines[i]).filter(|l| !l.text.is_empty()) {
            return Some((&l.text, true));
        }
        lines[at.map_or(0, |i| i + 1)..].iter().find(|l| !l.text.is_empty()).map(|l| (l.text.as_str(), false))
    }
}

impl Activity {
    fn new(kind: Kind, id: &str, priority: u8, mark: Mark, title: impl Into<String>, detail: impl Into<String>, short: impl Into<String>) -> Activity {
        Activity {
            kind,
            id: id.to_owned(),
            priority,
            mark,
            title: title.into(),
            detail: detail.into(),
            short: short.into(),
            value: None,
            meter: None,
            playing: None,
            urgent: false,
            state: String::new(),
            seed: None,
            sleep: None,
            sung: None,
        }
    }
}

/// "4:59", "1:02:03": a countdown or a time kept, in whole seconds.
pub fn clock_text(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

// ------------------------------------------------------------- providers ---

/// Music, from `nowplaying`: while something plays, or is paused but not
/// stopped.
pub fn music(np: Option<&crate::nowplaying::NowPlaying>) -> Option<Activity> {
    let np = np?;
    let mark = match &np.cover {
        Some(path) => Mark::Cover(path.clone(), "music_note"),
        None => Mark::Icon("music_note", Tint::Signal),
    };
    let detail = match (np.artist.is_empty(), np.album.is_empty()) {
        (false, _) => np.artist.clone(),
        (true, false) => np.album.clone(),
        (true, true) => String::new(),
    };
    // Compact, music is its cover and its level, no words (the song's name
    // is on the card the island opens to when the song changes, for a few
    // seconds): with the name in it the island was wide for as long as
    // anything played, and the time and the battery gave way to it.
    let mut a = Activity::new(Kind::Music, "music", 60, mark, np.title.clone(), detail, String::new());
    if np.duration > 0.0 {
        let rate = if np.playing { 1.0 / np.duration } else { 0.0 };
        a.meter = Some(Meter::Moving { from: np.position_now() / np.duration, rate, at: Instant::now() });
    }
    a.playing = Some(np.playing);
    a.state = format!("{}\u{1f}{}\u{1f}{}", np.title, np.artist, np.playing);
    Some(a)
}

/// Music with what else the island shows of it: its cover's colour, the
/// sleep timer counting down (the compact island's few characters while it
/// runs), and its lyrics for the card.
pub fn music_now(np: Option<&crate::nowplaying::NowPlaying>) -> Option<Activity> {
    let mut a = music(np)?;
    let np = np?;
    a.seed = np.cover.as_deref().and_then(crate::livemusic::cover_seed).flatten();
    with_sleep(&mut a, crate::music::sleep_left());
    a.sung = crate::music::sung_now().map(|lyrics| Sung { lyrics, from: np.position_now(), at: Instant::now(), playing: np.playing });
    Some(a)
}

/// The sleep timer's seconds left, on music's activity.
pub fn with_sleep(a: &mut Activity, left: Option<f32>) {
    a.sleep = left;
    a.short = left.map_or_else(String::new, |l| clock_text(l as f64));
}

/// Clock: the timer that runs out soonest (seconds left, its length, how
/// many run), and the stopwatch while it runs.
pub fn timers(soonest: Option<(f64, f64, usize)>, stopwatch: Option<f64>) -> Vec<Activity> {
    let mut out = Vec::new();
    if let Some((left, total, running)) = soonest {
        let last_minute = left < 60.0;
        let detail = match running {
            1 => format!("of {}", clock_text(total)),
            n => format!("of {} · {} more running", clock_text(total), n - 1),
        };
        let mut a = Activity::new(Kind::Timer, "timer", if last_minute { 85 } else { 80 }, Mark::Icon("timer", Tint::Signal), "Timer", detail, clock_text(left.ceil()));
        a.value = Some(clock_text(left.ceil()));
        if total > 0.0 {
            a.meter = Some(Meter::Moving { from: (1.0 - left / total) as f32, rate: 1.0 / total as f32, at: Instant::now() });
        }
        a.urgent = last_minute;
        a.state = format!("{}{}", total, last_minute);
        out.push(a);
    }
    if let Some(elapsed) = stopwatch {
        let mut a = Activity::new(Kind::Stopwatch, "stopwatch", 30, Mark::Icon("timer", Tint::Ice), "Stopwatch", "running", clock_text(elapsed.floor()));
        a.value = Some(clock_text(elapsed.floor()));
        out.push(a);
    }
    out
}

/// How long before queue time the match shows, and before the match itself
/// when there is no queue time (no FRC Nexus key).
const QUEUE_AHEAD: i64 = 20 * 60;
const MATCH_AHEAD: i64 = 15 * 60;

/// The team's next match, in the pit: counting down to queue time, then
/// "queue now", with the match and the alliance's colour. Nexus's own word
/// ("on deck", "on field") once it has one.
pub fn next_match(view: Option<&crate::schedule::View>, unix: i64) -> Option<Activity> {
    let view = view?;
    let m = view.next(unix)?;
    let side = m.alliance(view.team);
    let color = crate::upnext::side_color(side);
    let with: Vec<String> = (if side == Some("red") { &m.red } else { &m.blue }).iter().filter(|t| **t != view.team).map(u32::to_string).collect();
    let alliance = side.unwrap_or("match");
    let status = m.status.as_deref().map(str::to_lowercase);
    let live = status.as_deref().filter(|s| s.contains("queu") || s.contains("deck") || s.contains("field"));
    let partners = format!("{alliance} · with {}", with.join(", "));
    // (words for the island and its state, the card's value, urgent, the line under).
    let (words, value, urgent, detail) = if let Some(s) = live {
        let soon = s.contains("queu") && s.contains("soon");
        match m.queue_at.filter(|q| soon && *q > unix) {
            // Queuing soon, and when is known: a countdown to it.
            Some(q) => {
                let left = clock_text((q - unix) as f64);
                (format!("queue {left}"), left, false, format!("{alliance} · queue at {}", hm(q)))
            }
            None => {
                let words = match s {
                    _ if soon => "queue soon".to_owned(),
                    s if s.contains("queu") => "queue now".to_owned(),
                    s => s.to_owned(),
                };
                // When to queue, if it is known, says more than who with.
                let detail = m.queue_at.map_or(partners, |q| format!("{alliance} · queue at {}", hm(q)));
                (words.clone(), words, !soon, detail)
            }
        }
    } else if let Some(q) = m.queue_at {
        let match_gone = m.time.is_some_and(|t| t - unix <= -120);
        if unix >= q && !match_gone {
            ("queue now".to_owned(), "queue now".to_owned(), true, format!("{alliance} · queue at {}", hm(q)))
        } else if q > unix && q - unix <= QUEUE_AHEAD {
            let left = clock_text((q - unix) as f64);
            (format!("queue {left}"), left, false, format!("{alliance} · queue at {}", hm(q)))
        } else {
            return None;
        }
    } else if let Some(t) = m.time.filter(|t| *t - unix <= MATCH_AHEAD && *t > unix - 60) {
        let left = clock_text((t - unix).max(0) as f64);
        (left.clone(), left, false, partners)
    } else {
        return None;
    };
    let priority = match (urgent, m.queue_at.is_some() || live.is_some()) {
        (true, _) => 90,
        (false, true) => 75,
        (false, false) => 70,
    };
    let state = if urgent { words.clone() } else { "coming".to_owned() };
    let mut a = Activity::new(Kind::Match, "match", priority, Mark::Icon("flag", Tint::Fixed(color)), m.label.clone(), detail, format!("{} · {words}", m.label));
    a.value = Some(value);
    a.urgent = urgent;
    a.state = format!("{}\u{1f}{state}", m.key);
    Some(a)
}

/// "10:42", in the handheld's own time.
fn hm(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()
}

/// How long before a reminder is due it shows, counting down.
const REMINDER_AHEAD: i64 = 15 * 60;

/// The next reminder, within a quarter of an hour (once due, the island
/// says it, as it did).
pub fn reminder(r: Option<&crate::upnext::Reminder>, unix: i64) -> Option<Activity> {
    let r = r?;
    let due = r.due?;
    let left = due - unix;
    if !(0..=REMINDER_AHEAD).contains(&left) {
        return None;
    }
    let mut a = Activity::new(Kind::Reminder, "reminder", 70, Mark::Icon("checklist", Tint::Amber), r.title.clone(), format!("due at {}", hm(due)), clock_text(left as f64));
    a.value = Some(clock_text(left as f64));
    a.meter = Some(Meter::Moving { from: 1.0 - left as f32 / REMINDER_AHEAD as f32, rate: 1.0 / REMINDER_AHEAD as f32, at: Instant::now() });
    a.state = r.title.clone();
    Some(a)
}

/// The robot, while it runs (enabled, in a match or practising): its mode,
/// its battery, the match's time left. In the pit, disabled, it is the
/// Robot app's to show, not the island's.
pub fn robot(robot: &crate::robot::Robot) -> Option<Activity> {
    if !robot.linked() || robot.bool("OpMode/Running") != Some(true) {
        return None;
    }
    let name = robot.string("Robot/Identity/Name").map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Robot".into());
    let mode = robot.string("OpMode/Active").map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty()).unwrap_or_else(|| "enabled".into());
    Some(running_robot(name, mode, robot.f64("Systemcore/BatteryVolts"), robot.f64("Match/TimeLeft").filter(|t| *t >= 0.0)))
}

/// The robot running: its name, its mode, its battery, the match's time left.
pub fn running_robot(name: String, mode: String, volts: Option<f64>, left: Option<f64>) -> Activity {
    let detail = match volts {
        Some(v) => format!("{mode} · {v:.1} V"),
        None => mode.clone(),
    };
    let short = match (left, volts) {
        (Some(t), _) => clock_text(t),
        (None, Some(v)) => format!("{v:.1} V"),
        (None, None) => mode.clone(),
    };
    let tint = if volts.is_some_and(|v| v < 11.5) { Tint::Amber } else { Tint::Ice };
    let mut a = Activity::new(Kind::Robot, "robot", 55, Mark::Icon("smart_toy", tint), name, detail, short);
    a.value = left.map(clock_text);
    a.state = mode;
    a
}

/// How long a download or an answer shows once it is done.
pub const DONE_FOR: Duration = Duration::from_secs(5);

/// The system update downloading or installing (from its status), and when
/// it has just become ready, `ready_since`.
pub fn update(status: &crate::sysupdate::Status, ready_since: Option<Instant>) -> Option<Activity> {
    let (title, short, done) = match status.state.as_str() {
        "downloading" => ("Downloading", format!("{:.0}%", status.progress * 100.0), false),
        "installing" => ("Installing", format!("{:.0}%", status.progress * 100.0), false),
        "ready" if ready_since.is_some_and(|at| at.elapsed() < DONE_FOR) => ("Ready", "ready".to_owned(), true),
        _ => return None,
    };
    let mut a = Activity::new(Kind::Download, "update", 40, Mark::Icon(if done { "check_circle" } else { "download" }, Tint::Ice), format!("{title} {}", status.version), if done { "restart to install it".to_owned() } else { "system update".to_owned() }, short);
    a.meter = (!done).then_some(Meter::Fraction(status.progress));
    a.value = (!done).then(|| format!("{:.0}%", status.progress * 100.0));
    a.state = status.state.clone();
    Some(a)
}

// ----------------------------------------------------------------- tasks ---

/// Something under way that a module reports for itself: an app installing,
/// music downloading, the Assistant answering, a model loading. Begun with
/// [`task`]; gone when its handle is dropped, or shown done a moment first.
#[derive(Clone, Debug)]
pub struct Task {
    id: u64,
    kind: Kind,
    title: String,
    detail: String,
    progress: Option<f32>,
    began: Instant,
    done: Option<(Instant, String)>,
}

mod store {
    use super::Task;

    #[cfg(not(test))]
    static TASKS: std::sync::Mutex<Vec<Task>> = std::sync::Mutex::new(Vec::new());

    #[cfg(not(test))]
    pub fn with<T>(f: impl FnOnce(&mut Vec<Task>) -> T) -> T {
        f(&mut TASKS.lock().unwrap_or_else(|e| e.into_inner()))
    }

    // Each test thread its own, as the tests run side by side.
    #[cfg(test)]
    thread_local! {
        static TASKS: std::cell::RefCell<Vec<Task>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    #[cfg(test)]
    pub fn with<T>(f: impl FnOnce(&mut Vec<Task>) -> T) -> T {
        TASKS.with(|t| f(&mut t.borrow_mut()))
    }
}

static NEXT_TASK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A task's handle: it is under way while this is kept.
pub struct TaskHandle {
    id: u64,
}

/// Begin a task: `kind` Download or Assistant, `title` what it is
/// ("Installing Scouting", "Assistant").
pub fn task(kind: Kind, title: impl Into<String>, detail: impl Into<String>) -> TaskHandle {
    let id = NEXT_TASK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = Task { id, kind, title: title.into(), detail: detail.into(), progress: None, began: Instant::now(), done: None };
    store::with(|tasks| tasks.push(t));
    TaskHandle { id }
}

impl TaskHandle {
    /// How far it has come, 0 to 1.
    #[allow(dead_code)]
    pub fn progress(&self, fraction: f32) {
        store::with(|tasks| tasks.iter_mut().filter(|t| t.id == self.id).for_each(|t| t.progress = Some(fraction.clamp(0.0, 1.0))));
    }

    /// What it is doing now ("step 2 · setting a reminder"): the card's line;
    /// the compact island takes what comes before the first " · ".
    pub fn detail(&self, words: impl Into<String>) {
        let words = words.into();
        store::with(|tasks| tasks.iter_mut().filter(|t| t.id == self.id).for_each(|t| t.detail.clone_from(&words)));
    }

    /// Done: it shows so, in these words, for a moment.
    pub fn done(self, words: impl Into<String>) {
        let words = words.into();
        store::with(|tasks| tasks.iter_mut().filter(|t| t.id == self.id).for_each(|t| t.done = Some((Instant::now(), words.clone()))));
        std::mem::forget(self);
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        store::with(|tasks| tasks.retain(|t| t.id != self.id));
    }
}

/// The tasks under way, and those done in the last moment (the rest let go).
fn tasks() -> Vec<Task> {
    store::with(|tasks| {
        tasks.retain(|t| t.done.as_ref().is_none_or(|(at, _)| at.elapsed() < DONE_FOR));
        tasks.clone()
    })
}

impl Task {
    /// A task as if begun `seconds` ago (the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn sample(kind: Kind, title: &str, detail: &str, seconds: f32) -> Task {
        Task { id: 0, kind, title: title.into(), detail: detail.into(), progress: None, began: Instant::now() - Duration::from_secs_f32(seconds), done: None }
    }
}

/// A task as an activity.
pub fn from_task(t: &Task) -> Activity {
    let (icon, priority) = match t.kind {
        Kind::Assistant => ("auto_awesome", 50),
        // Above the Assistant: a permission request costs Claude's turn.
        Kind::Claude => (crate::claude::ICON, 52),
        _ => ("download", 40),
    };
    let id = format!("task {}", t.id);
    match &t.done {
        Some((_, words)) => {
            let mut a = Activity::new(t.kind, &id, priority, Mark::Icon("check_circle", Tint::Ice), t.title.clone(), words.clone(), words.clone());
            a.state = "done".into();
            a
        }
        None => {
            let elapsed = clock_text(t.began.elapsed().as_secs_f64().floor());
            let short = match t.progress {
                Some(p) => format!("{:.0}%", p * 100.0),
                None if t.kind == Kind::Assistant => format!("{} {elapsed}", if t.title.starts_with("Loading") { "loading" } else { t.detail.split(" · ").next().unwrap_or_default() }),
                None => t.detail.clone(),
            };
            // Claude waiting on an answer: in the signal colour, and it needs doing now.
            let asking = t.kind == Kind::Claude && t.detail.starts_with("approval");
            let mark = if asking { Mark::Icon("front_hand", Tint::Signal) } else { Mark::Icon(icon, Tint::Ice) };
            let mut a = Activity::new(t.kind, &id, priority, mark, t.title.clone(), t.detail.clone(), short);
            a.urgent = asking;
            a.meter = t.progress.map(Meter::Fraction);
            a.value = Some(match t.progress {
                Some(p) => format!("{:.0}%", p * 100.0),
                None => elapsed,
            });
            a.state = "under way".into();
            a
        }
    }
}

// ------------------------------------------------------------ the whole ---

/// What the providers read, gathered by the shell.
pub struct Sources<'a> {
    pub robot: &'a crate::robot::Robot,
    pub update: &'a crate::sysupdate::Status,
    pub unix: i64,
}

/// Every activity there is now, most important first.
pub fn gather(src: &Sources, update_ready: Option<Instant>) -> Vec<Activity> {
    let mut all: Vec<Activity> = Vec::new();
    all.extend(music_now(crate::nowplaying::get().as_ref()));
    all.extend(timers(crate::clock::soonest(), crate::clock::stopwatch_running()));
    all.extend(next_match(src.robot.schedule.view().as_ref(), src.unix));
    all.extend(reminder(crate::upnext::next_reminder(src.unix).as_ref(), src.unix));
    all.extend(update(src.update, update_ready));
    all.extend(tasks().iter().map(from_task));
    all.extend(robot(src.robot));
    all.extend(crate::airpods::activity());
    rank(&mut all);
    all
}

/// Most important first; of equal importance, as they were found.
pub fn rank(all: &mut [Activity]) {
    all.sort_by(|a, b| b.priority.cmp(&a.priority));
}

/// How long the island stays open on an activity that changed.
const OPEN_FOR: f32 = 3.5;

/// The activities, as the shell keeps them: gathered once a second while the
/// screen is on, and the island's state (open on one for a moment).
pub struct Activities {
    list: Vec<Activity>,
    age: f32,
    /// Each one's state when last seen, to tell a change.
    seen: Vec<(String, String)>,
    /// The island open on one: how far (0 closed, 1 open), and for how long more.
    pub open: Animated,
    open_left: f32,
    open_id: Option<String>,
    /// Music's level: 1 playing, 0 paused, springing between.
    pub level: Animated,
    update_was: String,
    update_ready: Option<Instant>,
    first: bool,
    /// Set by hand (the tests and the galleries): not read again.
    pinned: bool,
    /// What the screen in front already shows itself (Music, a song): a
    /// change to it does not open the island over that screen.
    pub quiet: Option<Kind>,
    /// Music's equalizer bars (livemusic.rs), and whether the island showed
    /// them at its last frame (the shell says, as it draws): they run only
    /// while it does.
    pub bars: crate::livemusic::Bars,
    /// The bars moved at the last tick: ambient, not motion (Shell::ambient).
    pub bars_live: bool,
    pub bars_seen: bool,
    /// A new song coming in: 0 as it changes, springing to 1; and the song
    /// before, going.
    pub song: Animated,
    previous: Option<Activity>,
    /// The cover's palette, easing from the last song's to this one's.
    tint: Animated,
    tint_from: Option<usize>,
    tint_to: Option<usize>,
    /// The open card's row for the line being sung, growing and going as a
    /// song with synced lyrics comes and goes.
    lyric: Animated,
}

/// How the island draws an activity beyond its data: music's bars and
/// level, its cover's colour as it eases, the song going as the next comes
/// in, and the lyric line (the island's card alone has room for it).
#[derive(Clone, Copy)]
pub struct Look<'a> {
    /// The bars' heights (`None`: standing still, risen by `level`).
    pub bars: Option<[f32; crate::livemusic::BARS]>,
    pub level: f32,
    pub accent: Option<Color>,
    pub previous: Option<&'a Activity>,
    /// The song's change, 0 to 1 (1: settled on the new one).
    pub change: f32,
    pub lyrics: bool,
}

impl Look<'_> {
    /// Still, as the lock screen and StandBy draw a card.
    pub fn still(level: f32) -> Look<'static> {
        Look { bars: None, level, accent: None, previous: None, change: 1.0, lyrics: false }
    }
}

impl Default for Activities {
    fn default() -> Self {
        Self::new()
    }
}

impl Activities {
    pub fn new() -> Activities {
        Activities {
            list: Vec::new(),
            age: f32::MAX,
            seen: Vec::new(),
            open: Animated::new(0.0, role::release()).with_rest(0.004),
            open_left: 0.0,
            open_id: None,
            level: Animated::new(0.0, role::smooth()).with_rest(0.004),
            update_was: String::new(),
            update_ready: None,
            first: true,
            pinned: false,
            quiet: None,
            bars: crate::livemusic::Bars::new(),
            bars_live: false,
            bars_seen: false,
            song: Animated::new(1.0, role::smooth()).with_rest(0.004),
            previous: None,
            tint: Animated::new(1.0, role::effect()).with_rest(0.004),
            tint_from: None,
            tint_to: None,
            lyric: Animated::new(0.0, role::smooth()).with_rest(0.004),
        }
    }

    /// The island open: its height. Music with the line being sung has a
    /// row more for it, springing in and out.
    pub fn open_h(&self, g: &Gfx) -> f32 {
        OPEN_H + lyric_row(g) * self.lyric.value().clamp(0.0, 1.0)
    }

    /// How the island draws music now: its bars (still, calm), its cover's
    /// colour, the song going as the next comes in.
    pub fn look(&self, g: &Gfx, calm: bool) -> Look<'_> {
        let level = self.level.value();
        let mix = self.tint.value().clamp(0.0, 1.0);
        let accent = crate::livemusic::signal_of(g, self.tint_from).mix(crate::livemusic::signal_of(g, self.tint_to), mix);
        let change = self.song.value().clamp(0.0, 1.0);
        Look { bars: (!calm).then(|| self.bars.heights()), level, accent: Some(accent), previous: self.previous.as_ref().filter(|_| change < 1.0), change, lyrics: true }
    }

    pub fn list(&self) -> &[Activity] {
        &self.list
    }

    /// The one the island is open on, while it is.
    pub fn opened(&self) -> Option<&Activity> {
        let id = self.open_id.as_ref()?;
        self.list.iter().find(|a| &a.id == id)
    }

    /// Open the island on this kind for a moment (a music key pressed).
    pub fn show(&mut self, kind: Kind, calm: bool) {
        if self.quiet == Some(kind) {
            return;
        }
        if let Some(a) = self.list.iter().find(|a| a.kind == kind) {
            self.open_id = Some(a.id.clone());
            self.open_left = OPEN_FOR;
            if calm {
                self.open.jump(1.0);
            } else {
                self.open.set_with(1.0, role::release());
            }
        }
    }

    /// The island closed at once (for the galleries).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn close_now(&mut self) {
        self.open_left = 0.0;
        self.open.jump(0.0);
    }

    /// Read again at the next tick (the screen woke; a key asked).
    pub fn refresh(&mut self) {
        self.age = f32::MAX;
    }

    /// Time passing: `awake` while the screen is on (off, nothing is read or
    /// drawn). Returns (moving, a new frame wanted).
    pub fn tick(&mut self, dt: f32, md: f32, awake: bool, calm: bool, src: &Sources) -> (bool, bool) {
        if !awake {
            // Read afresh on waking; and no island left open meanwhile.
            self.age = f32::MAX;
            self.open_left = 0.0;
            self.open.jump(0.0);
            return (false, false);
        }
        let mut changed = false;
        self.age += dt;
        if self.age >= 1.0 && !self.pinned {
            self.age = 0.0;
            if src.update.state != self.update_was {
                if src.update.state == "ready" && matches!(self.update_was.as_str(), "downloading" | "installing") {
                    self.update_ready = Some(Instant::now());
                }
                self.update_was = src.update.state.clone();
            }
            let list = gather(src, self.update_ready);
            changed |= list != self.list;
            self.take(list, calm);
        }
        if self.open_left > 0.0 {
            self.open_left -= dt;
            if self.open_left <= 0.0 || self.opened().is_none() {
                self.open_left = 0.0;
                if calm {
                    self.open.jump(0.0);
                } else {
                    self.open.set_with(0.0, role::smooth());
                }
            }
        }
        let playing = self.list.iter().any(|a| a.playing == Some(true));
        let level = if playing { 1.0 } else { 0.0 };
        if self.level.target() != level {
            if calm {
                self.level.jump(level);
            } else {
                self.level.set(level);
            }
        }
        // Music's bars: dancing while it plays and the island shows them
        // (never calm), settling flat otherwise and then costing nothing.
        let music = self.list.iter().find(|a| a.kind == Kind::Music);
        let seed = music.map_or(0, |a| crate::livemusic::song_seed(&a.title));
        // Their own time, not motion's (which stands still at rest): ambient,
        // they run between the frames of anything moving.
        let bars = self.bars.tick(dt, dt.min(0.1), playing && self.bars_seen && !calm, seed);
        let row = if self.opened().is_some_and(|a| a.sung.is_some()) { 1.0 } else { 0.0 };
        if self.lyric.target() != row {
            if calm {
                self.lyric.jump(row);
            } else {
                self.lyric.set(row);
            }
        }
        let moving = self.open.tick(md) | self.level.tick(md) | self.song.tick(md) | self.tint.tick(md) | self.lyric.tick(md);
        // The bars are ambient, not motion: drawn a few times a second on
        // their own (Shell::ambient), so the frame beat still rests and the
        // processor is not held up for them. As motion they kept every page
        // at 60 frames a second while music played, and the beat, never
        // resting, never came back from a heavy moment.
        self.bars_live = bars;
        // A meter moving by itself, or a clock counting: a frame now and then
        // (the main loop looks in four times a second at rest).
        let live = self.list.iter().any(|a| a.meter.is_some_and(|m| m.moving()));
        (moving, changed || moving || live || bars)
    }

    /// Music's song changed (another one, not paused or played): the one
    /// going is kept to fade as the new one comes in; its cover's colour
    /// eases to the new one's on the effects spring.
    fn music_changed(&mut self, list: &[Activity], calm: bool) {
        let was = self.list.iter().find(|a| a.kind == Kind::Music);
        let now = list.iter().find(|a| a.kind == Kind::Music);
        if let (Some(was), Some(now)) = (was, now) {
            if (was.title.as_str(), was.detail.as_str()) != (now.title.as_str(), now.detail.as_str()) {
                self.previous = Some(was.clone());
                self.song.jump(0.0);
                if calm {
                    self.song.jump(1.0);
                } else {
                    self.song.set(1.0);
                }
            }
        }
        let seed = now.and_then(|a| a.seed);
        if seed != self.tint_to {
            self.tint_from = if self.tint.value() >= 0.5 { self.tint_to } else { self.tint_from };
            self.tint_to = seed;
            self.tint.jump(0.0);
            if calm {
                self.tint.jump(1.0);
            } else {
                self.tint.set(1.0);
            }
        }
    }

    /// A new list: any that appeared or changed opens the island on it.
    fn take(&mut self, list: Vec<Activity>, calm: bool) {
        let mut opened = None;
        for a in &list {
            let was = self.seen.iter().find(|(id, _)| *id == a.id).map(|(_, s)| s);
            if was != Some(&a.state) && opened.is_none() && !self.first && self.quiet != Some(a.kind) {
                opened = Some(a.id.clone());
            }
        }
        self.seen = list.iter().map(|a| (a.id.clone(), a.state.clone())).collect();
        self.music_changed(&list, calm);
        self.list = list;
        self.first = false;
        if let Some(id) = opened {
            self.open_id = Some(id);
            self.open_left = OPEN_FOR;
            if calm {
                self.open.jump(1.0);
            } else {
                self.open.set_with(1.0, role::release());
            }
        }
    }

    /// For the tests and the galleries: these, as if just gathered.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub fn set(&mut self, mut list: Vec<Activity>, calm: bool) {
        rank(&mut list);
        self.take(list, calm);
        self.age = 0.0;
        self.pinned = true;
    }
}

// --------------------------------------------------------------- covers ---

/// Covers, decoded on a thread of their own and kept small (the last few).
struct Covers {
    kept: Vec<(PathBuf, Arc<Picture>)>,
    loading: Option<(PathBuf, std::sync::mpsc::Receiver<Option<Picture>>)>,
    failed: Vec<PathBuf>,
}

static COVERS: std::sync::Mutex<Covers> = std::sync::Mutex::new(Covers { kept: Vec::new(), loading: None, failed: Vec::new() });

/// A cover's size as kept: enough for StandBy's face, small for the rest.
pub const COVER_PX: usize = 128;

/// A cover, if it has been decoded; asked for, it is decoded meanwhile.
pub fn cover(path: &Path) -> Option<Arc<Picture>> {
    let mut c = COVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, pic)) = c.kept.iter().find(|(p, _)| p == path) {
        return Some(pic.clone());
    }
    if let Some((p, rx)) = &c.loading {
        match rx.try_recv() {
            Ok(pic) => {
                let p = p.clone();
                c.loading = None;
                match pic {
                    Some(pic) => {
                        c.kept.insert(0, (p.clone(), Arc::new(pic)));
                        c.kept.truncate(3);
                    }
                    None => c.failed.push(p.clone()),
                }
                if p == path {
                    return c.kept.first().filter(|(k, _)| k == path).map(|(_, pic)| pic.clone());
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => c.loading = None,
        }
    }
    if c.loading.is_none() && !c.failed.iter().any(|p| p == path) {
        let (tx, rx) = std::sync::mpsc::channel();
        let file = path.to_owned();
        let spawned = std::thread::Builder::new().name("cover".into()).stack_size(256 * 1024).spawn(move || {
            tx.send(decode_cover(&file)).ok();
        });
        if spawned.is_ok() {
            c.loading = Some((path.to_owned(), rx));
        }
    }
    None
}

/// Whether a cover is still being decoded (so whoever shows it looks again
/// soon), rather than kept, or found not to read.
pub fn cover_pending(path: &Path) -> bool {
    let c = COVERS.lock().unwrap_or_else(|e| e.into_inner());
    c.loading.as_ref().is_some_and(|(p, _)| p == path)
}

/// A cover file (JPEG or PNG), squared and shrunk to [`COVER_PX`].
pub fn decode_cover(path: &Path) -> Option<Picture> {
    let bytes = std::fs::read(path).ok()?;
    let pic = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        let (w, h) = (info.width as usize, info.height as usize);
        if w * h > 4096 * 4096 {
            return None;
        }
        let channels = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            _ => return None,
        };
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            let row = &buf[y * info.line_size..][..w * channels];
            for px in row.chunks_exact(channels) {
                match channels {
                    1 | 2 => rgb.extend_from_slice(&[px[0], px[0], px[0]]),
                    _ => rgb.extend_from_slice(&px[..3]),
                }
            }
        }
        Picture { w, h, rgb }
    } else {
        crate::photos::decode(path, COVER_PX, COVER_PX).ok()?
    };
    Some(pic.cover(COVER_PX, COVER_PX))
}

// ------------------------------------------------------------- drawing ---

/// The island open on an activity: its height, and its corners (Bezel's
/// card corners, scaled; concentric with the card drawn inset 4 within).
pub const OPEN_H: f32 = 60.0;
pub const OPEN_RADIUS: f32 = 18.0;

/// How wide the compact island is for `a`: its mark, its few words, its
/// trailing meter.
pub fn compact_w(g: &mut Gfx, a: &Activity) -> f32 {
    let words = g.text.fit(&a.short, &g.ty.caption, COMPACT_WORDS);
    (g.measure(&words, g.ty.caption) + COMPACT_PAD + lead_w(g, a) + trailing_w(a)).max(70.0).ceil()
}

/// Before the words: the sleep timer's moon, while it counts down.
fn lead_w(g: &Gfx, a: &Activity) -> f32 {
    if a.sleep.is_some() {
        g.ty.caption.size + 4.0
    } else {
        0.0
    }
}

/// The lyric line's row on music's card.
fn lyric_row(g: &Gfx) -> f32 {
    (g.ty.body.size + 7.0).round()
}

/// The compact island's words in `room`: all of them where they fit; else
/// what comes after the last " · " ("Q31 · queue 8:12": "queue 8:12"); else
/// a time or a share at the end ("8:12", "46%"); else cut short.
pub fn compact_words(g: &mut Gfx, a: &Activity, room: f32) -> String {
    let style = g.ty.caption;
    let mut tries = vec![a.short.clone()];
    if let Some((_, tail)) = a.short.rsplit_once(" · ") {
        tries.push(tail.to_owned());
    }
    if let Some((_, last)) = tries.last().and_then(|t| t.rsplit_once(' ')) {
        if last.contains(':') || last.contains('%') {
            tries.push(last.to_owned());
        }
    }
    match tries.iter().find(|t| g.measure(t, style) <= room) {
        Some(fits) => fits.clone(),
        None => g.text.fit(tries.last().map_or("", String::as_str), &style, room),
    }
}

/// The compact island's words at most, and what it adds round them (the
/// mark, the edges).
const COMPACT_WORDS: f32 = 150.0;
const COMPACT_PAD: f32 = 40.0;

fn trailing_w(a: &Activity) -> f32 {
    if a.playing.is_some() || a.meter.is_some() {
        16.0
    } else {
        0.0
    }
}

/// The mark: a cover (rounded), or an icon in a tonal circle of its tint.
pub fn draw_mark(g: &mut Gfx, cx: f32, cy: f32, size: f32, mark: &Mark, alpha: f32) {
    let t = g.tokens;
    match mark {
        Mark::Cover(path, fallback) => match cover(path) {
            Some(pic) => {
                let r = Rect::new(cx - size * 0.5, cy - size * 0.5, size, size);
                crate::photos::draw_picture(g, r, &pic, crate::photos::whole(&pic), (size * 0.18).max(2.0), alpha);
            }
            None => {
                g.canvas.circle(cx, cy, size * 0.5, t.surface3, alpha);
                g.icon_alpha(fallback, true, cx, cy, size * 0.62, t.signal, alpha);
            }
        },
        Mark::Icon(name, tint) => {
            let c = tint.color(g);
            if size >= 20.0 {
                g.canvas.circle(cx, cy, size * 0.5, t.surface2.mix(c, 0.16), alpha);
                g.icon_alpha(name, true, cx, cy, size * 0.6, c, alpha);
            } else {
                g.icon_alpha(name, true, cx, cy, size, c, alpha);
            }
        }
    }
}

/// Music's level at the compact island's end: its bars dancing with the
/// song (livemusic.rs), or standing still (calm), risen by `level` while it
/// plays and laid flat as it pauses.
fn draw_level(g: &mut Gfx, cx: f32, cy: f32, h: f32, look: &Look, color: Color, alpha: f32) {
    let heights = look.bars.unwrap_or_else(|| crate::livemusic::STILL.map(|k| k * look.level.clamp(0.0, 1.2)));
    crate::livemusic::draw_bars(g, cx, cy, h, heights, color, alpha);
}

/// A ring round a point: a meter at its size.
fn draw_ring(g: &mut Gfx, cx: f32, cy: f32, r: f32, fraction: f32, color: Color, alpha: f32) {
    let t = g.tokens;
    g.canvas.arc(cx, cy, r, 2.0, 0.0, std::f32::consts::TAU, t.surface3, alpha);
    if fraction > 0.005 {
        g.canvas.arc(cx, cy, r, 2.0, 0.0, std::f32::consts::TAU * fraction, color, alpha);
    }
}

/// The compact island (`r`, the island itself, glass already drawn): the
/// mark, the few words, a meter or music's level at the end.
pub fn draw_compact(g: &mut Gfx, r: Rect, a: &Activity, look: &Look, alpha: f32) {
    let t = g.tokens;
    let ty = g.ty;
    let mark = r.h - 8.0;
    let mark_x = r.x + 6.0 + mark * 0.5;
    // A new song: its cover comes up into place as the last one's goes up
    // and out.
    let change = look.change.clamp(0.0, 1.0);
    if let Some(prev) = look.previous.filter(|_| change < 1.0) {
        draw_mark(g, mark_x, r.cy() - change * 5.0, mark, &prev.mark, alpha * (1.0 - change));
    }
    draw_mark(g, mark_x, r.cy() + (1.0 - change) * 5.0, mark, &a.mark, alpha * change);
    let ink = if a.urgent { t.signal } else { t.ink };
    let mut x = r.x + 12.0 + mark;
    if a.sleep.is_some() {
        let size = ty.caption.size;
        g.icon_alpha("bedtime", true, x + size * 0.5, r.cy(), size, t.dim, alpha);
        x += lead_w(g, a);
    }
    let words = compact_words(g, a, (r.w - COMPACT_PAD - lead_w(g, a) - trailing_w(a) + 2.0).clamp(0.0, COMPACT_WORDS));
    g.text.draw(&mut g.canvas, x, (r.cy() + ty.caption.size * 0.36).round(), &words, &ty.caption, ink, alpha);
    let end = r.right() - 11.0;
    if a.playing.is_some() {
        let color = look.accent.unwrap_or(t.signal);
        draw_level(g, end, r.cy(), r.h - 8.0, look, color, alpha);
    } else if let Some(m) = a.meter {
        draw_ring(g, end, r.cy(), (r.h - 9.0) * 0.5, m.now(), if a.urgent { t.signal } else { g.good() }, alpha);
    }
}

/// The bubble beside the island: the second activity, its mark alone (and
/// a meter round it, if it has one).
pub fn draw_bubble(g: &mut Gfx, r: Rect, a: &Activity, alpha: f32) {
    let size = r.h - 8.0;
    draw_mark(g, r.cx(), r.cy(), size.min(14.0), &a.mark, alpha);
    if let Some(m) = a.meter {
        let color = if a.urgent { g.tokens.signal } else { g.good() };
        draw_ring(g, r.cx(), r.cy(), r.h * 0.5 - 2.5, m.now(), color, alpha);
    }
}

/// The keys music answers to (Fn + W, Q, E), under their marks.
pub const MUSIC_KEYS: [(&str, &str); 3] = [("skip_previous", "w"), ("play_arrow", "q"), ("skip_next", "e")];

/// Music's three keys: their marks centred on `xs`, at `cy`, the letters
/// beneath and "fn" once before them.
pub fn draw_music_keys(g: &mut Gfx, xs: [f32; 3], cy: f32, playing: bool, size: f32, alpha: f32) {
    let t = g.tokens;
    let ty = g.ty;
    let quiet = t.faint.mix(t.dim, 0.5);
    let below = (cy + size * 0.5 + 5.0 + ty.caption.size * 0.72).round();
    for ((icon, key), x) in MUSIC_KEYS.iter().zip(xs) {
        let middle = *icon == "play_arrow";
        let icon = if middle && playing { "pause" } else { icon };
        g.icon_alpha(icon, true, x, cy, if middle { size * 1.25 } else { size }, t.ink, alpha);
        g.text.draw_aligned(&mut g.canvas, x, below, Align::Center, key, &ty.caption, quiet, alpha);
    }
    let fw = g.measure("w", ty.caption);
    g.text.draw_aligned(&mut g.canvas, xs[0] - fw * 0.5 - 5.0, below, Align::Right, "fn", &ty.caption, quiet, alpha);
}

/// The line under a card's title, as much of it as fits whole: "red · queue
/// at 19:22" with too little room gives up its first part ("queue at
/// 19:22": the mark's colour says red) before anything is cut.
fn fitting_detail<'a>(g: &mut Gfx, detail: &'a str, style: bezel::TextStyle, room: f32) -> &'a str {
    if g.measure(detail, style) <= room {
        return detail;
    }
    detail.split_once(" · ").map_or(detail, |(_, rest)| rest)
}

/// A card: the activity whole, on `r` (a tonal panel, or the open island's
/// glass, drawn by whoever places it). Large (56 px and up): the mark big,
/// the title and the line under it, the value or music's keys at the right,
/// the meter along the foot. Small: one band.
pub fn draw_card(g: &mut Gfx, r: Rect, a: &Activity, level: f32, alpha: f32) {
    draw_card_look(g, r, a, &Look::still(level), alpha);
}

/// A card, as the island draws it (`look`): music's cover colouring its
/// progress, the song going up and out as the next comes in, the sleep
/// timer at the end of its progress, and the line being sung along its foot.
pub fn draw_card_look(g: &mut Gfx, r: Rect, a: &Activity, look: &Look, alpha: f32) {
    let t = g.tokens;
    let ty = g.ty;
    // The lyric line's row comes in as the island opens past its usual height.
    let full = r;
    let row = lyric_row(g);
    let lyric_h = if a.sung.is_some() && look.lyrics { (r.h - (OPEN_H - 8.0)).clamp(0.0, row) } else { 0.0 };
    let r = Rect::new(r.x, r.y, r.w, r.h - lyric_h);
    let large = r.h >= 50.0;
    let pad = if large { 8.0 } else { 6.0 };
    let mark = if large { (r.h - 2.0 * pad).min(56.0) } else { r.h - 2.0 * pad };
    let change = look.change.clamp(0.0, 1.0);
    let previous = look.previous.filter(|p| change < 1.0 && p.kind == a.kind);
    if let Some(p) = previous {
        draw_mark(g, r.x + pad + mark * 0.5, r.cy(), mark, &p.mark, alpha * (1.0 - change));
    }
    draw_mark(g, r.x + pad + mark * 0.5, r.cy(), mark, &a.mark, alpha * if previous.is_some() { change } else { 1.0 });
    let x = r.x + pad + mark + 9.0;
    let accent = if a.urgent { t.signal } else { t.ink };
    // At the right: music's keys, or the value.
    let mut right = r.right() - pad - 2.0;
    if large && a.playing.is_some() {
        let step = 22.0;
        let xs = [right - 8.0 - 2.0 * step, right - 8.0 - step, right - 8.0];
        draw_music_keys(g, xs, r.y + r.h * 0.4, a.playing == Some(true), 15.0, alpha);
        // The keys, and "fn" before them.
        right -= 2.0 * step + 8.0 + 24.0;
    } else if a.playing.is_some() {
        let icon = if a.playing == Some(true) { "pause" } else { "play_arrow" };
        g.icon_alpha(icon, true, right - 8.0, r.cy(), 16.0, t.ink, alpha);
        right -= 22.0;
    } else if let Some(v) = &a.value {
        // Figures large; words ("queue now") in the name's voice.
        let figures = v.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '%' || c == '.');
        let style = if !large { ty.label } else if figures { ty.display(22.0) } else { ty.name };
        let w = g.measure(v, style);
        let baseline = if large { r.y + r.h * 0.5 + g.text.cap_height(&style) * 0.5 - 3.0 } else { r.cy() + style.size * 0.36 };
        g.text.draw_aligned(&mut g.canvas, right, baseline.round(), Align::Right, v, &style, accent, alpha);
        right -= w + 10.0;
    }
    let room = (right - x).max(0.0);
    // Room for the meter: under the words on a large card; along the foot
    // of a small one, the words kept clear of it.
    let meter_h = match (a.meter.is_some(), large) {
        (true, true) => 10.0,
        (true, false) => 5.0,
        (false, _) => 0.0,
    };
    let (title_style, line) = if large { (ty.name, ty.caption) } else { (ty.body, ty.caption) };
    let block = title_style.size + 4.0 + line.size;
    let top = r.cy() - (block + meter_h) * 0.5;
    // The title and the line under it; a new song's rise into place as the
    // last one's rise up and out (Bezel's smooth spring, `look.change`).
    // The old words are gone before the new are much in, so the two never
    // read over each other.
    if let Some(p) = previous {
        card_words(g, p, x, top - change * 10.0, room, (title_style, line), false, alpha * (1.0 - ease(change / 0.5)));
    }
    // The sleep timer: at the end of the lyric row if there is one, else
    // after the artist if both fit whole, else at the end of the progress.
    let sleep = a.sleep.filter(|_| a.kind == Kind::Music && large).map(|l| clock_text(l as f64));
    let sleep_w = sleep.as_ref().map_or(0.0, |s| line.size + 3.0 + g.measure(s, line));
    let sleep_on_lyrics = sleep.is_some() && lyric_h > 0.5;
    let sleep_on_line = sleep.is_some() && !sleep_on_lyrics && g.measure(&a.detail, line) + sleep_w + 18.0 <= room;
    let (rise, into) = if previous.is_some() { ((1.0 - change) * 10.0, ease((change - 0.35) / 0.65)) } else { (0.0, 1.0) };
    card_words(g, a, x, top + rise, room, (title_style, line), sleep_on_line, alpha * into);
    if let Some(m) = a.meter {
        let music = a.kind == Kind::Music;
        let fill = if a.urgent { t.signal } else if music { look.accent.unwrap_or(t.signal) } else { g.good() };
        if large {
            let mut bar = Rect::new(x, (top + block + 6.0).round(), room.max(20.0), 3.0);
            if let Some(words) = sleep.as_ref().filter(|_| !sleep_on_line && !sleep_on_lyrics) {
                let size = line.size;
                let end = x + room - 10.0;
                bar.w = (bar.w - sleep_w - 18.0).max(20.0);
                g.icon_alpha("bedtime", true, end - sleep_w + size * 0.5, bar.cy(), size, t.dim, alpha);
                g.text.draw_aligned(&mut g.canvas, end, (bar.cy() + size * 0.36).round(), Align::Right, words, &line, t.dim, alpha);
            }
            g.canvas.rrect(bar, 1.5, t.meter, alpha);
            let w = (bar.w * m.now()).max(3.0);
            g.canvas.rrect_clipped(Rect::new(bar.x, bar.y, w, bar.h), 1.5, bar, 1.5, fill, alpha);
        } else {
            // Along the foot, inside the card's corners.
            let bar = Rect::new(r.x + r.h * 0.4, r.bottom() - 3.0, r.w - r.h * 0.8, 2.0);
            g.canvas.rrect(bar, 1.0, t.meter, alpha * 0.8);
            g.canvas.rrect_clipped(Rect::new(bar.x, bar.y, (bar.w * m.now()).max(2.0), bar.h), 1.0, bar, 1.0, fill, alpha);
        }
    }
    // The line being sung, in ink; before the singing and in its pauses,
    // the one to come, quieter.
    if lyric_h > 0.5 {
        let style = ty.body;
        let cy = r.bottom() + lyric_h * 0.5 - 2.0;
        let shown = alpha * (lyric_h / row).clamp(0.0, 1.0);
        let mut width = full.w - 2.0 * pad;
        if let Some(words) = sleep.as_ref().filter(|_| sleep_on_lyrics) {
            let end = full.right() - pad - 2.0;
            g.icon_alpha("bedtime", true, end - sleep_w + line.size * 0.5, cy, line.size, t.dim, shown);
            g.text.draw_aligned(&mut g.canvas, end, (cy + line.size * 0.36).round(), Align::Right, words, &line, t.dim, shown);
            width -= sleep_w + 14.0;
        }
        if let Some((words, sung)) = a.sung.as_ref().and_then(Sung::line) {
            let words = g.text.fit(words, &style, width);
            g.text.draw(&mut g.canvas, full.x + pad, (cy + style.size * 0.36).round(), &words, &style, if sung { t.ink } else { t.dim }, shown);
        }
    }
}

/// 0 below 0, 1 above 1, smoothly between.
fn ease(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A card's title and the line under it, from `top`; music's sleep timer
/// at that line's end, the moon before it.
#[allow(clippy::too_many_arguments)]
fn card_words(g: &mut Gfx, a: &Activity, x: f32, top: f32, room: f32, (title_style, line): (bezel::TextStyle, bezel::TextStyle), sleep: bool, alpha: f32) {
    let t = g.tokens;
    let title = g.text.fit(&a.title, &title_style, room);
    let baseline = (top + title_style.size + 4.0 + line.size * 0.8).round();
    let mut detail_room = room;
    if let Some(left) = a.sleep.filter(|_| sleep) {
        let words = clock_text(left as f64);
        let size = line.size;
        let w = size + 3.0 + g.measure(&words, line);
        // Clear of the keys' letters to the right.
        let end = x + room - 8.0;
        g.icon_alpha("bedtime", true, end - w + size * 0.5, baseline - line.size * 0.36, size, t.dim, alpha);
        g.text.draw_aligned(&mut g.canvas, end, baseline, Align::Right, &words, &line, t.dim, alpha);
        detail_room = (room - w - 18.0).max(0.0);
    }
    let detail = fitting_detail(g, &a.detail, line, detail_room);
    let detail = g.text.fit(detail, &line, detail_room);
    g.text.draw(&mut g.canvas, x, (top + title_style.size * 0.8).round(), &title, &title_style, t.ink, alpha);
    if !detail.is_empty() {
        g.text.draw(&mut g.canvas, x, baseline, &detail, &line, t.dim, alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nowplaying::NowPlaying;
    use crate::schedule::{Match, View};

    fn song(playing: bool) -> NowPlaying {
        NowPlaying { title: "Around the World".into(), artist: "Daft Punk".into(), album: "Homework".into(), cover: None, position: 30.0, duration: 120.0, playing, at: Instant::now() }
    }

    #[test]
    fn the_compact_island_keeps_what_matters_of_its_words_in_the_room_it_has() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut a = Activity::new(Kind::Match, "queue", 75, Mark::Icon("flag", Tint::Signal), "Q31", "on red", "Q31 · queue 8:12");
        let room = |g: &mut Gfx, s: &str| g.measure(s, g.ty.caption) + 0.5;
        let all = room(&mut g, "Q31 · queue 8:12");
        assert_eq!(compact_words(&mut g, &a, all), "Q31 · queue 8:12", "room for all of it");
        let tail = room(&mut g, "queue 8:12");
        assert_eq!(compact_words(&mut g, &a, tail), "queue 8:12", "what follows the dot");
        let time = room(&mut g, "8:12");
        assert_eq!(compact_words(&mut g, &a, time), "8:12", "the time it says");
        // A last word that is not a time or a share is not taken alone.
        a.short = "3 of 12".into();
        let twelve = room(&mut g, "12");
        assert_ne!(compact_words(&mut g, &a, twelve), "12");
        // Music has no words: its cover and its level.
        let music = music(Some(&song(true))).unwrap();
        assert!(music.short.is_empty());
        assert!(compact_w(&mut g, &music) <= 70.0);
    }

    #[test]
    fn music_plays_paused_or_not_at_all() {
        assert!(music(None).is_none());
        let a = music(Some(&song(true))).unwrap();
        assert_eq!((a.title.as_str(), a.detail.as_str(), a.playing, a.priority), ("Around the World", "Daft Punk", Some(true), 60));
        let m = a.meter.unwrap();
        assert!((m.now() - 0.25).abs() < 0.01 && m.moving(), "a quarter in, moving");
        let paused = music(Some(&song(false))).unwrap();
        assert!(!paused.meter.unwrap().moving() && paused.playing == Some(false));
        assert_ne!(a.state, paused.state, "playing and paused are a change");
    }

    #[test]
    fn a_timer_counts_down_and_its_last_minute_matters_more() {
        let list = timers(Some((299.4, 300.0, 1)), None);
        assert_eq!((list[0].short.as_str(), list[0].priority, list[0].urgent), ("5:00", 80, false));
        let list = timers(Some((42.0, 300.0, 3)), Some(65.0));
        assert_eq!((list[0].short.as_str(), list[0].priority, list[0].urgent), ("0:42", 85, true));
        assert_eq!(list[0].detail, "of 5:00 · 2 more running");
        assert_eq!((list[1].kind, list[1].short.as_str(), list[1].priority), (Kind::Stopwatch, "1:05", 30));
        assert!(timers(None, None).is_empty());
        assert_eq!(clock_text(3725.0), "1:02:05");
    }

    fn view(queue_at: Option<i64>, time: i64, status: Option<&str>) -> View {
        let m = Match { key: "2026x_q31".into(), label: "Q31".into(), red: vec![1507, 5254, 5805], blue: vec![3044, 6328, 2337], time: Some(time), score: None, status: status.map(str::to_owned), queue_at };
        View { team: 5805, matches: vec![m], ..View::default() }
    }

    #[test]
    fn the_next_match_counts_down_to_queue_time_then_says_queue_now() {
        let now = 1_800_000_000;
        assert!(next_match(Some(&view(Some(now + 30 * 60), now + 45 * 60, None)), now).is_none(), "half an hour off: not yet");
        let a = next_match(Some(&view(Some(now + 8 * 60 + 12), now + 23 * 60, None)), now).unwrap();
        assert_eq!((a.short.as_str(), a.priority, a.urgent), ("Q31 · queue 8:12", 75, false));
        assert_eq!(a.mark, Mark::Icon("flag", Tint::Fixed(crate::schedule::RED)), "the alliance's colour");
        assert!(a.detail.starts_with("red · queue at"));
        let now_q = next_match(Some(&view(Some(now - 30), now + 10 * 60, None)), now).unwrap();
        assert_eq!((now_q.short.as_str(), now_q.priority, now_q.urgent), ("Q31 · queue now", 90, true));
        assert_ne!(a.state, now_q.state, "queue time is a change");
        let nexus = next_match(Some(&view(None, now + 600, Some("On deck"))), now).unwrap();
        assert_eq!((nexus.short.as_str(), nexus.urgent), ("Q31 · on deck", true));
        let soon = next_match(Some(&view(None, now + 600, Some("Queuing soon"))), now).unwrap();
        assert!(!soon.urgent && soon.priority == 75);
        assert_eq!(soon.value.as_deref(), Some("queue soon"));
        // Nexus says soon, and the queue time is known: a countdown to it.
        let timed = next_match(Some(&view(Some(now + 5 * 60), now + 20 * 60, Some("Queuing soon"))), now).unwrap();
        assert_eq!((timed.short.as_str(), timed.value.as_deref(), timed.urgent), ("Q31 · queue 5:00", Some("5:00"), false));
        assert!(timed.detail.starts_with("red · queue at"));
        let no_queue = next_match(Some(&view(None, now + 10 * 60, None)), now).unwrap();
        assert_eq!((no_queue.short.as_str(), no_queue.priority), ("Q31 · 10:00", 70));
        assert!(next_match(None, now).is_none());
    }

    #[test]
    fn a_reminder_shows_in_its_last_quarter_hour() {
        let now = 1_800_000_000;
        let r = |due: i64| crate::upnext::Reminder { title: "charge the spares".into(), due: Some(due) };
        assert!(reminder(Some(&r(now + 16 * 60)), now).is_none());
        let a = reminder(Some(&r(now + 12 * 60 + 3)), now).unwrap();
        assert_eq!((a.short.as_str(), a.priority), ("12:03", 70));
        assert!(reminder(Some(&r(now - 5)), now).is_none(), "due: the island says it, as before");
        assert!(reminder(Some(&crate::upnext::Reminder { title: "x".into(), due: None }), now).is_none());
    }

    #[test]
    fn the_robot_shows_only_while_it_runs() {
        let demo = crate::robot::Robot::demo();
        assert_eq!(robot(&demo).is_some(), demo.bool("OpMode/Running") == Some(true));
        assert!(robot(&crate::robot::Robot::none()).is_none());
    }

    #[test]
    fn an_update_shows_its_progress_and_a_moment_of_ready() {
        let mut s = crate::sysupdate::Status { state: "downloading".into(), version: "0.6.0".into(), progress: 0.46, ..Default::default() };
        let a = update(&s, None).unwrap();
        assert_eq!((a.short.as_str(), a.title.as_str()), ("46%", "Downloading 0.6.0"));
        assert_eq!(a.meter, Some(Meter::Fraction(0.46)));
        s.state = "ready".into();
        assert!(update(&s, None).is_none(), "ready long since: Settings says it");
        assert_eq!(update(&s, Some(Instant::now())).unwrap().short, "ready");
        s.state = "idle".into();
        assert!(update(&s, None).is_none());
    }

    #[test]
    fn tasks_show_under_way_and_a_moment_done_then_go() {
        let install = task(Kind::Download, "Installing Scouting", "from the catalogue");
        install.progress(0.3);
        let thinking = task(Kind::Assistant, "Assistant", "thinking");
        let list: Vec<Activity> = tasks().iter().map(from_task).collect();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].short.as_str(), list[0].priority), ("30%", 40));
        assert_eq!((list[1].short.as_str(), list[1].priority), ("thinking 0:00", 50), "the Assistant counts its time");
        drop(thinking);
        install.done("installed");
        let list: Vec<Activity> = tasks().iter().map(from_task).collect();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].short.as_str(), list[0].state.as_str()), ("installed", "done"));
    }

    #[test]
    fn the_most_important_leads_and_a_change_opens_the_island() {
        let mut acts = Activities::new();
        let mut list = timers(Some((200.0, 300.0, 1)), Some(10.0));
        list.extend(music(Some(&song(true))));
        acts.set(list.clone(), false);
        let kinds: Vec<Kind> = acts.list().iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [Kind::Timer, Kind::Music, Kind::Stopwatch]);
        assert!(acts.opened().is_none(), "the first look opens nothing");
        // The song changes: the island opens on music, though the timer leads.
        let mut next = list.clone();
        next.retain(|a| a.kind != Kind::Music);
        next.extend(music(Some(&NowPlaying { title: "Da Funk".into(), ..song(true) })));
        acts.set(next, false);
        assert_eq!(acts.opened().map(|a| a.kind), Some(Kind::Music));
        assert_eq!(acts.list().first().map(|a| a.kind), Some(Kind::Timer));
        // It closes by itself.
        let src_robot = crate::robot::Robot::none();
        let status = crate::sysupdate::Status::default();
        let src = Sources { robot: &src_robot, update: &status, unix: 0 };
        acts.open_left = 0.1;
        acts.tick(0.2, 0.2, true, false, &src);
        assert_eq!(acts.open.target(), 0.0);
        // With the screen off nothing is read or drawn.
        assert_eq!(acts.tick(5.0, 0.0, false, false, &src), (false, false));
    }
}
