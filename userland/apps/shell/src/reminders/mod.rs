//! Reminders: what to do and when, for the pit and for the rest of the day.
//!
//! - **Lists**: Today (what is due, and the rest of today), Upcoming, and
//!   Done; each reminder is the pit's or personal, and Tab shows one list.
//! - **Quick add**, typed as it would be said (parse.rs): "swap battery in
//!   10 min", "tomorrow 8am check bumpers", "before Q42 charge spare".
//! - **Pit cues** tied to the event schedule (plan.rs): N minutes before a
//!   match, at queueing, after each match, halfway between matches. The
//!   schedule is read each time, so when the event runs late, they move too.
//! - **The pit routine**: a few lines applied in one go ("after each match:
//!   swap battery, check bumpers, download logs").
//! - **Notes**: `r` on a note gives it a reminder, which opens the note.
//!
//! When one comes due the island says it (longer than a message), the
//! keyboard's lights blink, and the chime plays once there is a speaker
//! (`set_chime`). With the screen off it wakes, and Reminders opens on what
//! is due. In deep standby the shell looks in once a minute and compares
//! the clock with one time worked out before it went deep (`Watch::due`);
//! the minute's wait ends early instead when that comes sooner.
//!
//! Kept in `/data/catalyst/reminders.json` (beside the settings), so they
//! outlast a restart and an update. A wall-clock time is kept as one, so it
//! follows the time zone and daylight saving; a wait ("in 10 min") is an
//! instant, and moves with the clock when the network corrects it.
//!
//! For other screens: `upcoming(limit)` (the lock screen, StandBy) and
//! `search(query, limit)` with `open(id)` (Spotlight).

mod app;
pub mod parse;
pub mod plan;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use chrono::NaiveDateTime;
use serde_json::{json, Value};

pub use app::{capture_add, capture_reading, Compose, Reminders};
use parse::Parsed;
use plan::{when_text, Here, Now, Occ};

/// How a wall-clock time is kept.
pub const LOCAL: &str = "%Y-%m-%dT%H:%M";

// ------------------------------------------------------------- the model ---

/// Which list a reminder is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Pit,
    Personal,
}

impl List {
    pub fn name(self) -> &'static str {
        match self {
            List::Pit => "pit",
            List::Personal => "personal",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            List::Pit => "#pit",
            List::Personal => "#personal",
        }
    }

    fn from_name(name: &str) -> Option<List> {
        [List::Pit, List::Personal].into_iter().find(|l| l.name() == name)
    }

    pub fn other(self) -> List {
        match self {
            List::Pit => List::Personal,
            List::Personal => List::Pit,
        }
    }
}

/// Where in a match's run a pit reminder falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// This many seconds before it starts.
    Before(i64),
    /// When the team queues (Nexus's time, else 20 minutes before).
    Queue,
    /// Once it is over and the robot is back.
    After,
    /// Halfway from one match being over to queueing for the next.
    Between,
}

/// Which match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Of {
    /// The next one, tied to it once the schedule says which that is.
    Next,
    /// One match: by its label until the schedule has it, then by its key.
    One { label: String, key: Option<String> },
    /// Every match, one after another.
    Each,
}

#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// No time: a to-do on Today's list.
    Whenever,
    /// An instant, from a wait ("in 10 min").
    In(i64),
    /// A wall-clock time here.
    At(NaiveDateTime),
    /// Tied to the team's matches.
    Match(Cue, Of),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Never,
    Daily,
    Weekdays,
    Weekly,
    /// Every so many seconds.
    Every(i64),
}

/// What a reminder opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    None,
    /// A note, by its file in the notes folder.
    Note(String),
    /// The pit checklist.
    Checklist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reminder {
    pub id: u64,
    pub text: String,
    pub list: List,
    pub when: When,
    pub repeat: Repeat,
    pub link: Link,
    /// When it was made, Unix seconds.
    pub created: i64,
    /// The occurrence last said, so none is said twice.
    pub said: Option<String>,
    /// Put off until then.
    pub snooze: Option<i64>,
    /// Ticked off then (one that does not repeat).
    pub done: Option<i64>,
    /// The matches an every-match reminder is past.
    pub past: Vec<String>,
}

impl Reminder {
    /// A to-do with nothing else to it.
    pub fn blank(id: u64, text: &str, created: i64) -> Reminder {
        Reminder { id, text: text.into(), list: List::Personal, when: When::Whenever, repeat: Repeat::Never, link: Link::None, created, said: None, snooze: None, done: None, past: Vec::new() }
    }
}

/// A reminder come due, as the shell announces it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    pub id: u64,
    /// What the island says: "swap battery · after Q31".
    pub said: String,
    pub text: String,
    /// The pit's (the chime may choose by it).
    pub pit: bool,
    /// It came due a while ago, the handheld off meanwhile.
    pub late: bool,
    pub link: Link,
}

/// The three lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Today,
    Upcoming,
    Done,
}

/// One reminder as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub text: String,
    /// "in 9 min", "Q42 · 14:05", "no time".
    pub when: String,
    /// Said and not yet ticked off.
    pub due: bool,
    pub done: bool,
    pub repeats: bool,
    pub link: Link,
    pub list: List,
    /// Waiting for the schedule to give it a time.
    pub waiting: bool,
    pub at: Option<i64>,
}

// ---------------------------------------------------------------- saying ---

/// What the island adds to a reminder's words as it comes due.
fn context(r: &Reminder, occ: &Occ, late: i64, n: &Now, h24: bool) -> Option<String> {
    let label = occ.label.clone().unwrap_or_default();
    match &r.when {
        When::Match(Cue::Before(lead), _) if *lead >= 3600 && lead % 3600 == 0 => Some(format!("{label} in {} h", lead / 3600)),
        When::Match(Cue::Before(lead), _) => Some(format!("{label} in {} min", (lead + 59) / 60)),
        When::Match(Cue::Queue, _) => Some(format!("queue for {label}")),
        When::Match(Cue::After, _) => Some(format!("after {label}")),
        When::Match(Cue::Between, _) => Some("between matches".into()),
        _ if late > 120 => Some(format!("was due {}", when_text(occ.at, n, h24))),
        _ => None,
    }
}

fn ring(r: &Reminder, occ: Option<&Occ>, late: i64, n: &Now, h24: bool) -> Ring {
    let context = occ.and_then(|o| context(r, o, late, n, h24));
    let said = match context {
        Some(c) => format!("{} · {c}", r.text),
        None => r.text.clone(),
    };
    Ring { id: r.id, said, text: r.text.clone(), pit: r.list == List::Pit, late: late > 120, link: r.link.clone() }
}

/// What the island says for what just came due: the first, and how many more.
pub fn island(rang: &[Ring]) -> String {
    match rang {
        [] => String::new(),
        [one] => one.said.clone(),
        [first, rest @ ..] => format!("{} · +{} more", first.text, rest.len()),
    }
}

// ------------------------------------------------------------------ book ---

/// The most ticked-off reminders kept, and for how long.
const DONE_KEPT: usize = 60;
const DONE_DAYS: i64 = 30;

/// Every reminder, kept on the data partition.
pub struct Book {
    pub reminders: Vec<Reminder>,
    next_id: u64,
    path: Option<PathBuf>,
    /// Reminders whose time was set this run from a wait or a snooze: a
    /// clock correction moves them with it.
    relative: Vec<u64>,
    /// Counts every change.
    pub edits: u64,
}

impl Book {
    /// A book kept only in memory (the tests).
    pub fn memory() -> Book {
        Book { reminders: Vec::new(), next_id: 1, path: None, relative: Vec::new(), edits: 0 }
    }

    /// The book kept at `path`. One that cannot be read is put aside (as
    /// `.bad`), never written over.
    pub fn open(path: PathBuf) -> Book {
        let mut book = Book { path: Some(path.clone()), ..Book::memory() };
        let Ok(text) = std::fs::read_to_string(&path) else { return book };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                book.reminders = v["reminders"].as_array().map(|a| a.iter().filter_map(from_json).collect()).unwrap_or_default();
                let highest = book.reminders.iter().map(|r| r.id).max().unwrap_or(0);
                book.next_id = v["next"].as_u64().unwrap_or(1).max(highest + 1);
            }
            Err(_) => {
                std::fs::rename(&path, path.with_extension("json.bad")).ok();
            }
        }
        book
    }

    fn changed(&mut self) {
        self.edits += 1;
        if let Some(path) = &self.path {
            let all: Vec<Value> = self.reminders.iter().map(to_json).collect();
            let text = json!({ "version": 1, "next": self.next_id, "reminders": all }).to_string();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            crate::sys::write_atomic(path, text.as_bytes()).ok();
        }
    }

    pub fn get(&self, id: u64) -> Option<&Reminder> {
        self.reminders.iter().find(|r| r.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Reminder> {
        self.reminders.iter_mut().find(|r| r.id == id)
    }

    /// The list a new reminder goes on when none is asked for: the pit's
    /// for a match cue or while an event is on, else personal.
    fn usual(p: &Parsed, n: &Now) -> List {
        p.list.unwrap_or(if n.event_today() { List::Pit } else { List::Personal })
    }

    /// Add what was typed.
    pub fn add(&mut self, p: Parsed, n: &Now) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut r = Reminder { list: Self::usual(&p, n), when: p.when, repeat: p.repeat, link: p.link, ..Reminder::blank(id, &p.text, n.at) };
        bind(&mut r, n);
        if matches!(r.when, When::In(_)) {
            self.relative.push(id);
        }
        self.reminders.push(r);
        self.changed();
        id
    }

    /// The same reminder, typed again: its words, time and list.
    pub fn replace(&mut self, id: u64, p: Parsed, n: &Now) -> bool {
        let list = Self::usual(&p, n);
        let Some(r) = self.get_mut(id) else { return false };
        r.text = p.text;
        r.when = p.when;
        r.repeat = p.repeat;
        r.list = list;
        if p.link != Link::None {
            r.link = p.link;
        }
        (r.said, r.snooze, r.done, r.created) = (None, None, None, n.at);
        r.past.clear();
        bind(r, n);
        let relative = matches!(r.when, When::In(_));
        if relative {
            self.relative.push(id);
        }
        self.changed();
        true
    }

    /// Enter: tick it off (one that repeats goes on to its next), or put a
    /// ticked one back. What the island says.
    pub fn tick(&mut self, id: u64, n: &Now, h24: bool) -> Option<String> {
        let r = self.get_mut(id)?;
        let said = if r.done.take().is_some() {
            "back on the list".to_owned()
        } else if r.repeats() {
            r.catch_up(n);
            r.advance(n);
            (r.said, r.snooze) = (None, None);
            match r.current(n) {
                Some(next) => format!("next {}", next.label.map_or_else(|| when_text(next.at, n, h24), |l| format!("{l} · {}", when_text(next.at, n, h24)))),
                None => "done for now".to_owned(),
            }
        } else {
            r.done = Some(n.at);
            r.snooze = None;
            "done".to_owned()
        };
        self.changed();
        Some(said)
    }

    /// Put it off until `until`.
    pub fn snooze(&mut self, id: u64, until: i64) {
        if let Some(r) = self.get_mut(id) {
            r.snooze = Some(until);
            r.done = None;
            self.relative.push(id);
            self.changed();
        }
    }

    pub fn delete(&mut self, id: u64) -> Option<Reminder> {
        let at = self.reminders.iter().position(|r| r.id == id)?;
        let gone = self.reminders.remove(at);
        self.changed();
        Some(gone)
    }

    /// The next repeat along: never, every day, weekdays, every week (for
    /// a time); one match or every match (for a cue). What the island says.
    pub fn cycle_repeat(&mut self, id: u64, n: &Now) -> Option<String> {
        let r = self.get_mut(id)?;
        let said = match &r.when {
            When::Whenever => return Some("give it a time first: e".into()),
            When::Match(cue, Of::Each) => {
                r.when = When::Match(*cue, Of::Next);
                bind(r, n);
                "just the next match"
            }
            When::Match(cue, _) => {
                (r.when, r.created) = (When::Match(*cue, Of::Each), n.at);
                r.past.clear();
                "every match"
            }
            When::In(at) => {
                // A wait made a clock time, so it can come round each day.
                let at = n.zone.local(*at);
                r.when = When::At(at);
                r.repeat = Repeat::Daily;
                "every day"
            }
            When::At(_) => {
                r.repeat = match r.repeat {
                    Repeat::Never => Repeat::Daily,
                    Repeat::Daily => Repeat::Weekdays,
                    Repeat::Weekdays => Repeat::Weekly,
                    Repeat::Weekly | Repeat::Every(_) => Repeat::Never,
                };
                match r.repeat {
                    Repeat::Daily => "every day",
                    Repeat::Weekdays => "weekdays",
                    Repeat::Weekly => "every week",
                    _ => "once",
                }
            }
        };
        r.said = None;
        self.changed();
        Some(said.into())
    }

    /// To the other list.
    pub fn switch_list(&mut self, id: u64) -> Option<List> {
        let r = self.get_mut(id)?;
        r.list = r.list.other();
        let list = r.list;
        self.changed();
        Some(list)
    }

    /// Tie what can now be tied to the schedule, and let old ticked-off ones go.
    pub fn settle(&mut self, n: &Now) {
        let mut changed = false;
        for r in &mut self.reminders {
            changed |= bind(r, n);
        }
        let mut done: Vec<(i64, u64)> = self.reminders.iter().filter_map(|r| r.done.map(|d| (d, r.id))).collect();
        done.sort_unstable_by(|a, b| b.cmp(a));
        let gone: Vec<u64> = done.iter().enumerate().filter(|(i, (at, _))| *i >= DONE_KEPT || n.at - at > DONE_DAYS * 86_400).map(|(_, (_, id))| *id).collect();
        if !gone.is_empty() {
            self.reminders.retain(|r| !gone.contains(&r.id));
            changed = true;
        }
        if changed {
            self.changed();
        }
    }

    /// The soonest any reminder needs saying, still to come.
    pub fn next_due(&self, n: &Now) -> Option<i64> {
        self.reminders.iter().filter_map(|r| r.next_due(n)).filter(|at| *at > n.at).min()
    }

    /// Everything come due by now, each once.
    pub fn fire(&mut self, n: &Now, h24: bool) -> Vec<Ring> {
        let mut rang = Vec::new();
        let mut changed = false;
        for r in &mut self.reminders {
            if r.done.is_some() {
                continue;
            }
            if let Some(until) = r.snooze {
                if n.at < until {
                    continue;
                }
                r.snooze = None;
                let current = r.current(n);
                r.said = current.as_ref().map(|o| o.key.clone()).or(r.said.take());
                rang.push(ring(r, None, 0, n, h24));
                changed = true;
                continue;
            }
            if r.repeats() {
                let before = (r.when.clone(), r.past.len());
                r.catch_up(n);
                changed |= before != (r.when.clone(), r.past.len());
            }
            let Some(current) = r.current(n) else { continue };
            if n.at < current.at || r.said.as_deref() == Some(current.key.as_str()) {
                continue;
            }
            r.said = Some(current.key.clone());
            changed = true;
            let late = n.at - current.at;
            // One that repeats, found long after: the next is what matters.
            if r.repeats() && late > plan::STALE {
                continue;
            }
            rang.push(ring(r, Some(&current), late, n, h24));
        }
        if changed {
            self.changed();
        }
        rang
    }

    /// The clock was set or corrected by `delta` seconds: waits set this
    /// run still come as long after they were set as they said.
    pub fn clock_moved(&mut self, delta: i64) {
        let mut changed = false;
        for r in self.reminders.iter_mut().filter(|r| self.relative.contains(&r.id)) {
            if let When::In(at) = &mut r.when {
                *at += delta;
                changed = true;
            }
            if let Some(until) = &mut r.snooze {
                *until += delta;
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
    }

    /// The reminders on a list, in its order, as rows.
    pub fn rows(&self, page: Page, filter: Option<List>, n: &Now, h24: bool) -> Vec<Row> {
        let mut rows: Vec<(u8, i64, Row)> = self
            .reminders
            .iter()
            .filter(|r| filter.is_none_or(|l| r.list == l) && page_of(r, n) == page)
            .map(|r| {
                let row = row(r, n, h24);
                let order = match page {
                    Page::Done => (0, -r.done.unwrap_or(0)),
                    _ if row.due => (0, row.at.unwrap_or(0)),
                    _ => match row.at {
                        Some(at) => (1, at),
                        None => (2, r.created),
                    },
                };
                (order.0, order.1, row)
            })
            .collect();
        rows.sort_by(|a, b| (a.0, a.1, a.2.id).cmp(&(b.0, b.1, b.2.id)));
        rows.into_iter().map(|(.., row)| row).collect()
    }

    /// The reminder a note has, if any (not yet ticked off).
    pub fn for_note(&self, file: &str) -> Option<&Reminder> {
        self.reminders.iter().find(|r| r.done.is_none() && r.link == Link::Note(file.to_owned()))
    }

    /// One like it is already on a list (the pit routine applied twice).
    pub fn has_like(&self, p: &Parsed) -> bool {
        self.reminders.iter().any(|r| r.done.is_none() && r.text.eq_ignore_ascii_case(&p.text) && same_kind(&r.when, &p.when))
    }
}

/// The same cue, or the same time: enough to call two reminders the same.
fn same_kind(a: &When, b: &When) -> bool {
    match (a, b) {
        (When::Match(ca, oa), When::Match(cb, ob)) => ca == cb && std::mem::discriminant(oa) == std::mem::discriminant(ob),
        _ => a == b,
    }
}

/// Tie a cue to its match, once the schedule says which: "the next match"
/// to the one coming, "Q42" to its key. True if it changed.
fn bind(r: &mut Reminder, n: &Now) -> bool {
    let Some(view) = n.view else { return false };
    match &r.when {
        When::Match(cue, Of::Next) => {
            let cue = *cue;
            let Some(next) = plan::cues(view, cue).into_iter().find(|o| o.at > n.at) else { return false };
            let key = next.key.trim_start_matches("m:").to_owned();
            r.when = When::Match(cue, Of::One { label: next.label.unwrap_or_default(), key: Some(key) });
            true
        }
        When::Match(cue, Of::One { label, key: None }) => {
            let (cue, label) = (*cue, label.clone());
            let Some(m) = view.matches.iter().find(|m| m.label.eq_ignore_ascii_case(&label)) else { return false };
            r.when = When::Match(cue, Of::One { label: m.label.clone(), key: Some(m.key.clone()) });
            true
        }
        _ => false,
    }
}

/// Which list a reminder shows on now.
pub fn page_of(r: &Reminder, n: &Now) -> Page {
    if r.done.is_some() {
        return Page::Done;
    }
    if r.ringing(n) {
        return Page::Today;
    }
    match r.shown_at(n) {
        Some(at) if at < n.tomorrow_starts() => Page::Today,
        Some(_) => Page::Upcoming,
        // Waiting for the schedule: it is to come.
        None if matches!(r.when, When::Match(..)) => Page::Upcoming,
        None => Page::Today,
    }
}

fn row(r: &Reminder, n: &Now, h24: bool) -> Row {
    let current = r.current(n);
    let due = r.ringing(n);
    let (when, at, waiting) = if let Some(done) = r.done {
        let when = if (n.at - done).abs() < 60 { "just now".to_owned() } else { when_text(done, n, h24) };
        (when, Some(done), false)
    } else if let Some(until) = r.snooze {
        (when_text(until, n, h24), Some(until), false)
    } else if let Some(o) = &current {
        // "Q42 14:05": the match, then when (~ when Nexus has no queue time for it).
        let time = format!("{}{}", if o.estimated { "~" } else { "" }, when_text(o.at, n, h24));
        (o.label.as_ref().map_or(time.clone(), |l| format!("{l} {time}")), Some(o.at), false)
    } else {
        let why = match (&r.when, n.view) {
            (When::Match(..), None) => "no schedule yet".to_owned(),
            // Its match is not on the team's schedule (yet).
            (When::Match(_, Of::One { label, .. }), Some(_)) => format!("{label} not yet"),
            (When::Match(..), Some(_)) => "no match coming".to_owned(),
            _ => "no time".to_owned(),
        };
        (why, None, matches!(r.when, When::Match(..)))
    };
    Row { id: r.id, text: r.text.clone(), when, due, done: r.done.is_some(), repeats: r.repeats(), link: r.link.clone(), list: r.list, waiting, at }
}

// ------------------------------------------------------------- the file ---

fn to_json(r: &Reminder) -> Value {
    let when = match &r.when {
        When::Whenever => Value::Null,
        When::In(at) => json!({ "in": at }),
        When::At(at) => json!({ "at": at.format(LOCAL).to_string() }),
        When::Match(cue, of) => {
            let (name, lead) = match cue {
                Cue::Before(lead) => ("before", Some(*lead)),
                Cue::Queue => ("queue", None),
                Cue::After => ("after", None),
                Cue::Between => ("between", None),
            };
            let of = match of {
                Of::Next => json!("next"),
                Of::Each => json!("each"),
                Of::One { label, key } => json!({ "label": label, "key": key }),
            };
            json!({ "cue": name, "lead": lead, "of": of })
        }
    };
    let repeat = match r.repeat {
        Repeat::Never => json!("never"),
        Repeat::Daily => json!("daily"),
        Repeat::Weekdays => json!("weekdays"),
        Repeat::Weekly => json!("weekly"),
        Repeat::Every(every) => json!({ "every": every }),
    };
    let link = match &r.link {
        Link::None => Value::Null,
        Link::Note(file) => json!({ "note": file }),
        Link::Checklist => json!("checklist"),
    };
    json!({
        "id": r.id, "text": r.text, "list": r.list.name(), "when": when, "repeat": repeat, "link": link,
        "created": r.created, "said": r.said, "snooze": r.snooze, "done": r.done, "past": r.past,
    })
}

fn from_json(v: &Value) -> Option<Reminder> {
    let w = &v["when"];
    let when = if w.is_null() {
        When::Whenever
    } else if let Some(at) = w["in"].as_i64() {
        When::In(at)
    } else if let Some(at) = w["at"].as_str() {
        When::At(NaiveDateTime::parse_from_str(at, LOCAL).ok()?)
    } else {
        let cue = match w["cue"].as_str()? {
            "before" => Cue::Before(w["lead"].as_i64().unwrap_or(plan::LEAD)),
            "queue" => Cue::Queue,
            "after" => Cue::After,
            "between" => Cue::Between,
            _ => return None,
        };
        let of = match &w["of"] {
            Value::String(s) if s == "next" => Of::Next,
            Value::String(s) if s == "each" => Of::Each,
            o => Of::One { label: o["label"].as_str()?.to_owned(), key: o["key"].as_str().map(str::to_owned) },
        };
        When::Match(cue, of)
    };
    let repeat = match &v["repeat"] {
        Value::String(s) => match s.as_str() {
            "daily" => Repeat::Daily,
            "weekdays" => Repeat::Weekdays,
            "weekly" => Repeat::Weekly,
            _ => Repeat::Never,
        },
        o => o["every"].as_i64().filter(|e| *e >= 60).map_or(Repeat::Never, Repeat::Every),
    };
    let link = match &v["link"] {
        Value::String(s) if s == "checklist" => Link::Checklist,
        o => o["note"].as_str().map_or(Link::None, |f| Link::Note(f.to_owned())),
    };
    Some(Reminder {
        id: v["id"].as_u64()?,
        text: v["text"].as_str()?.to_owned(),
        list: v["list"].as_str().and_then(List::from_name).unwrap_or(List::Personal),
        when,
        repeat,
        link,
        created: v["created"].as_i64().unwrap_or(0),
        said: v["said"].as_str().map(str::to_owned),
        snooze: v["snooze"].as_i64(),
        done: v["done"].as_i64(),
        past: v["past"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
    })
}

// ----------------------------------------------------------- the store ---

/// The book, with what it is worked out against, shared by the shell, the
/// app and the screens that show reminders elsewhere.
pub struct Store {
    pub book: Book,
    /// The event schedule as last seen, and its generation then.
    view: Option<crate::schedule::View>,
    seen: Option<u64>,
    /// The clock is a 24-hour one.
    h24: bool,
    /// Said, and not yet looked at in Reminders.
    ringing: Vec<u64>,
    /// Just added or opened from elsewhere: Reminders shows it.
    show: Option<u64>,
    /// Home's line for the tile, and what it was worked out at.
    #[allow(clippy::type_complexity)]
    tile: Option<((u64, Option<u64>, i64), (String, Option<&'static str>))>,
}

impl Store {
    fn with_book(book: Book) -> Store {
        Store { book, view: None, seen: None, h24: true, ringing: Vec::new(), show: None, tile: None }
    }

    /// The schedule's latest, if it has changed since it was last taken.
    pub fn sync(&mut self, schedule: &crate::schedule::Schedule) {
        let generation = schedule.generation();
        if self.seen != Some(generation) {
            self.view = schedule.view();
            self.seen = Some(generation);
        }
    }
}

/// Where the reminders are kept: beside the settings (`/data/catalyst`).
#[cfg_attr(test, allow(dead_code))]
pub fn path() -> PathBuf {
    crate::config_dir().join("reminders.json")
}

/// The time, Unix seconds (in the tests, as far on as `later` has put it).
pub fn now() -> i64 {
    chrono::Utc::now().timestamp() + skew()
}

/// The time passing, beside the clock: a clock set shows as the two
/// disagreeing.
fn mono() -> Instant {
    Instant::now() + std::time::Duration::from_secs(skew() as u64)
}

#[cfg(not(test))]
fn skew() -> i64 {
    0
}

#[cfg(test)]
thread_local! {
    static SKEW: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn skew() -> i64 {
    SKEW.with(std::cell::Cell::get)
}

/// For the tests: as if `seconds` had passed, by the clock and the time alike.
#[cfg(test)]
pub fn later(seconds: i64) {
    SKEW.with(|s| s.set(s.get() + seconds.max(0)));
}

/// For the tests: a reminder typed, on this thread's book.
#[cfg(test)]
pub fn remind(typed: &str) {
    with(|s| {
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        for p in parse::parse(typed, &n).expect("a reminder") {
            s.book.add(p, &n);
        }
    });
}

/// For the galleries: reminders typed ("charge the spares in 25 min"), on
/// the book in use.
#[cfg(not(target_arch = "aarch64"))]
pub fn pretend(typed: &[&str]) {
    with(|s| {
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        for text in typed {
            for p in parse::parse(text, &n).unwrap_or_default() {
                s.book.add(p, &n);
            }
        }
    });
}

// The shell, its screens and the other screens that show reminders share
// one book. The tests run many shells at once, one to a thread: each has
// its own, in memory.
#[cfg(not(test))]
static STORE: Mutex<Option<Store>> = Mutex::new(None);

#[cfg(not(test))]
pub fn with<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(store.get_or_insert_with(|| Store::with_book(Book::open(path()))))
}

#[cfg(test)]
thread_local! {
    static STORE: std::cell::RefCell<Option<Store>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn with<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    STORE.with(|s| f(s.borrow_mut().get_or_insert_with(|| Store::with_book(Book::memory()))))
}

// ------------------------------------------------- for other screens ---
//
// The lock screen, StandBy and Spotlight are being built beside this; until
// they call these, nothing in the shell does.

/// A reminder coming up, as the lock screen and StandBy show it.
#[derive(Clone, Debug, PartialEq)]
pub struct Upcoming {
    pub id: u64,
    pub text: String,
    /// "in 9 min", "Q42 · 14:05", "tmrw 08:00": as Reminders says it.
    pub when: String,
    /// When, Unix seconds (none while it waits on the schedule).
    pub at: Option<i64>,
    /// Said, and not ticked off yet.
    pub due: bool,
    pub pit: bool,
    /// The note it opens, by its file in the notes folder.
    pub note: Option<String>,
}

/// StandBy's and the lock screen's slot (`upnext::set_reminders`): the
/// reminder due, else the next with a time.
pub fn next_for_upnext(_now: i64) -> Option<crate::upnext::Reminder> {
    upcoming(1).into_iter().next().map(|u| crate::upnext::Reminder { title: u.text, due: u.at })
}

/// For the lock screen and StandBy: the next `limit` reminders not done
/// that have a time, what is due first, then the soonest.
#[allow(dead_code)]
pub fn upcoming(limit: usize) -> Vec<Upcoming> {
    with(|s| {
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        let mut rows: Vec<Row> = s.book.rows(Page::Today, None, &n, s.h24).into_iter().chain(s.book.rows(Page::Upcoming, None, &n, s.h24)).filter(|r| r.at.is_some()).collect();
        rows.sort_by_key(|r| (!r.due, r.at));
        rows.into_iter()
            .take(limit)
            .map(|r| Upcoming { id: r.id, text: r.text, when: r.when, at: r.at, due: r.due, pit: r.list == List::Pit, note: if let Link::Note(f) = r.link { Some(f) } else { None } })
            .collect()
    })
}

/// Tick a reminder off (Enter on the reminders widget), as Enter in
/// Reminders does: what the island says, if it is still there.
pub fn done(id: u64) -> Option<String> {
    with(|s| {
        let h24 = s.h24;
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        s.book.tick(id, &n, h24)
    })
}

/// A reminder found by Spotlight.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: u64,
    pub icon: &'static str,
    /// Its words.
    pub name: String,
    /// When, or "done".
    pub value: String,
    /// How well it matched: 0 its start, 1 a word's, 2 anywhere (as `search::rank`).
    pub rank: u8,
}

/// For Spotlight: the reminders whose words (or the match they are tied
/// to) match `query`, best first; `open(found.id)` shows one.
#[allow(dead_code)]
pub fn search(query: &str, limit: usize) -> Vec<Found> {
    with(|s| {
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        let mut found: Vec<Found> = s
            .book
            .reminders
            .iter()
            .filter_map(|r| {
                let row = row(r, &n, s.h24);
                let rank = rank(&r.text, query).or_else(|| rank(&row.when, query).map(|k| k.max(1)))?;
                let value = if row.done { "done".to_owned() } else { row.when };
                Some(Found { id: r.id, icon: "notifications", name: r.text.clone(), value, rank })
            })
            .collect();
        found.sort_by(|a, b| (a.rank, &a.name).cmp(&(b.rank, &b.name)));
        found.truncate(limit);
        found
    })
}

/// How well `name` matches `query` (both compared in lower case): 0 when it
/// starts with it, 1 when a word in it does, 2 anywhere; `None` if not at all.
fn rank(name: &str, query: &str) -> Option<u8> {
    let (name, query) = (name.to_lowercase(), query.trim().to_lowercase());
    if query.is_empty() {
        return None;
    }
    if name.starts_with(&query) {
        return Some(0);
    }
    if name.split(|c: char| !c.is_alphanumeric()).any(|w| !w.is_empty() && w.starts_with(&query)) {
        return Some(1);
    }
    name.contains(&query).then_some(2)
}

/// Reminders, open on one reminder (from Spotlight, the lock screen).
#[allow(dead_code)]
pub fn open(id: u64) -> Box<dyn crate::shell::Screen> {
    with(|s| s.show = Some(id));
    Box::new(Reminders::new())
}

/// What Home's tile says: what is due, else today's count, else what is
/// coming. Twelve characters at most.
pub fn tile() -> (String, Option<&'static str>) {
    with(|s| {
        // Home asks every frame it draws: worked out again only as the book,
        // the schedule or the quarter minute changes.
        let at = now();
        let key = (s.book.edits ^ ((s.book.reminders.len() as u64) << 48), s.seen, at / 15);
        if let Some((k, line)) = &s.tile {
            if *k == key {
                return line.clone();
            }
        }
        let n = Now { at, zone: &Here, view: s.view.as_ref() };
        let today = s.book.rows(Page::Today, None, &n, s.h24);
        let due = today.iter().filter(|r| r.due).count();
        let line = if due > 0 {
            (format!("{due} due now"), Some("WARN"))
        } else if !today.is_empty() {
            (format!("{} today", today.len()), None)
        } else {
            match s.book.rows(Page::Upcoming, None, &n, s.h24).len() {
                0 => ("pit · notes".into(), None),
                k => (format!("{k} coming"), None),
            }
        };
        s.tile = Some((key, line.clone()));
        line
    })
}

/// What a reminder opens: its note in the editor, or the pit checklist.
pub fn opens(link: &Link) -> Result<Box<dyn crate::shell::Screen>, &'static str> {
    match link {
        Link::Note(file) => match crate::files::Editor::open(&crate::notes::dir().join(file)) {
            Ok(editor) => Ok(Box::new(editor)),
            Err(_) => Err("that note is gone"),
        },
        Link::Checklist => Ok(Box::new(crate::pit::Checklist::new())),
        Link::None => Err("nothing to open"),
    }
}

/// The notes (by file) that have a reminder: Notes shows a bell on them.
pub fn noted() -> Vec<String> {
    with(|s| s.book.reminders.iter().filter(|r| r.done.is_none()).filter_map(|r| if let Link::Note(f) = &r.link { Some(f.clone()) } else { None }).collect())
}

/// A note was deleted: its reminders go with it.
pub fn note_deleted(file: &str) {
    with(|s| {
        let ids: Vec<u64> = s.book.reminders.iter().filter(|r| r.link == Link::Note(file.to_owned())).map(|r| r.id).collect();
        for id in ids {
            s.book.delete(id);
        }
    });
}

// ------------------------------------------------------------- the chime ---

/// What plays as reminders come due. The sound side sets it once the
/// speaker works; until then nothing plays (the island and the lights say it).
pub type Chime = fn(&[Ring]);

static CHIME: Mutex<Option<Chime>> = Mutex::new(None);

/// Set what plays as reminders come due (for the audio side, which is
/// being built beside this).
#[allow(dead_code)]
pub fn set_chime(chime: Chime) {
    *CHIME.lock().unwrap_or_else(|e| e.into_inner()) = Some(chime);
}

/// Play it, if there is one.
pub fn chime(rang: &[Ring]) {
    let chime = *CHIME.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(chime) = chime {
        chime(rang);
    }
}

// ------------------------------------------------------- the shell's side ---

/// How often the shell works the next due time out afresh while awake (it
/// does at once whenever a reminder or the schedule changes).
const LOOK: f32 = 30.0;
/// More than this between the clock and the time that passed is the clock
/// being set, not the time passing.
const JUMP: i64 = 90;
/// Before 2025 the clock has not been set (the clock chip lost its time):
/// nothing is said until it is.
const FLOOR: i64 = 1_735_689_600;

/// The shell's watch over the reminders: when the next is due, worked out
/// ahead, so a tick (and deep standby's minute) only compares two numbers.
pub struct Watch {
    due: Option<i64>,
    look: f32,
    /// The book's edits and the schedule's generation it was worked out at.
    seen: Option<(u64, u64)>,
    /// The clock against the time passing, to tell when it is set.
    wall: Option<(Instant, i64)>,
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

impl Watch {
    pub fn new() -> Watch {
        Watch { due: None, look: 0.0, seen: None, wall: None }
    }

    /// When the next reminder is due, Unix seconds: what deep standby's
    /// minute tick compares the clock with.
    pub fn due(&self) -> Option<i64> {
        self.due
    }

    /// Each tick: what has come due, said once each. The next due time is
    /// worked out again when a reminder or the schedule changed, the clock
    /// was set, it came, or half a minute went by.
    pub fn tick(&mut self, dt: f32, schedule: &crate::schedule::Schedule, h24: bool) -> Vec<Ring> {
        self.tick_at(dt, schedule, h24, now(), mono())
    }

    pub fn tick_at(&mut self, dt: f32, schedule: &crate::schedule::Schedule, h24: bool, now: i64, mono: Instant) -> Vec<Ring> {
        let mut jumped = false;
        if let Some((then, wall)) = self.wall {
            let delta = now - (wall + mono.saturating_duration_since(then).as_secs_f64().round() as i64);
            if delta.abs() > JUMP {
                with(|s| s.book.clock_moved(delta));
                jumped = true;
            }
        }
        self.wall = Some((mono, now));
        self.look -= dt;
        let seen = (with(|s| s.book.edits), schedule.generation());
        let came = self.due.is_some_and(|d| now >= d);
        if !(jumped || came || self.look <= 0.0 || self.seen != Some(seen)) {
            return Vec::new();
        }
        self.look = LOOK;
        let (rang, due, edits) = with(|s| {
            // Taken afresh each time: a robot changed brings another schedule.
            s.view = schedule.view();
            s.seen = Some(schedule.generation());
            s.h24 = h24;
            let n = Now { at: now, zone: &Here, view: s.view.as_ref() };
            s.book.settle(&n);
            let set = now >= FLOOR;
            let rang = if set { s.book.fire(&n, h24) } else { Vec::new() };
            for r in &rang {
                if !s.ringing.contains(&r.id) {
                    s.ringing.push(r.id);
                }
            }
            (rang, if set { s.book.next_due(&n) } else { None }, s.book.edits)
        });
        self.due = due;
        self.seen = Some((edits, schedule.generation()));
        rang
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plan::tests::{local, saturday, utc, NewYork};
    use plan::Zone;

    fn sat() -> i64 {
        utc(2026, 11, 21, 15, 0)
    }

    fn add(book: &mut Book, typed: &str, n: &Now) -> u64 {
        let mut last = 0;
        for p in parse::parse(typed, n).unwrap() {
            last = book.add(p, n);
        }
        last
    }

    #[test]
    fn a_reminder_is_said_once_when_it_comes_and_stays_until_ticked() {
        let view = saturday();
        let z = NewYork;
        let at = |t: i64| Now { at: t, zone: &z, view: Some(&view) };
        let mut book = Book::memory();
        let id = add(&mut book, "swap battery in 10 min", &at(sat()));
        assert_eq!(book.next_due(&at(sat())), Some(sat() + 600));
        assert!(book.fire(&at(sat() + 599), true).is_empty());
        let rang = book.fire(&at(sat() + 600), true);
        assert_eq!(rang.iter().map(|r| r.said.as_str()).collect::<Vec<_>>(), ["swap battery"]);
        assert!(book.fire(&at(sat() + 660), true).is_empty(), "said once");
        assert_eq!(book.next_due(&at(sat() + 660)), None);
        let rows = book.rows(Page::Today, None, &at(sat() + 660), true);
        assert!(rows[0].due && rows[0].when == "1 min ago", "{rows:?}");
        // Ticked: on the done list.
        assert_eq!(book.tick(id, &at(sat() + 700), true).as_deref(), Some("done"));
        assert!(book.rows(Page::Today, None, &at(sat() + 700), true).is_empty());
        assert_eq!(book.rows(Page::Done, None, &at(sat() + 700), true).len(), 1);
        // And back.
        assert_eq!(book.tick(id, &at(sat() + 710), true).as_deref(), Some("back on the list"));
    }

    #[test]
    fn snoozed_it_comes_again_later() {
        let z = NewYork;
        let at = |t: i64| Now { at: t, zone: &z, view: None };
        let mut book = Book::memory();
        let id = add(&mut book, "stretch in 1 min", &at(sat()));
        assert_eq!(book.fire(&at(sat() + 60), true).len(), 1);
        book.snooze(id, sat() + 660);
        assert_eq!(book.next_due(&at(sat() + 61)), Some(sat() + 660));
        assert!(!book.rows(Page::Today, None, &at(sat() + 61), true)[0].due, "snoozed: not due");
        assert!(book.fire(&at(sat() + 600), true).is_empty());
        assert_eq!(book.fire(&at(sat() + 660), true).len(), 1);
        assert!(book.fire(&at(sat() + 700), true).is_empty());
        assert_eq!(book.next_due(&at(sat() + 700)), None);
    }

    #[test]
    fn a_daily_one_comes_round_and_a_missed_day_passes_unsaid() {
        let z = NewYork;
        let at = |t: i64| Now { at: t, zone: &z, view: None };
        let mut book = Book::memory();
        let id = add(&mut book, "every day at 08:00 check bumpers", &at(sat()));
        let sun8 = utc(2026, 11, 22, 13, 0);
        assert_eq!(book.next_due(&at(sat())), Some(sun8));
        assert_eq!(book.fire(&at(sun8 + 5), true).len(), 1);
        assert_eq!(book.next_due(&at(sun8 + 5)), Some(sun8 + 86_400), "monday's next");
        // Off from Sunday to Wednesday noon: Monday's and Tuesday's are gone,
        // and Wednesday's 8:00 was four hours ago: it passes unsaid.
        let wed = utc(2026, 11, 25, 17, 0);
        assert!(book.fire(&at(wed), true).is_empty());
        assert_eq!(book.next_due(&at(wed)), Some(utc(2026, 11, 26, 13, 0)), "thursday's");
        // Ticked on Wednesday: on to Thursday's.
        assert_eq!(book.tick(id, &at(wed), true).as_deref(), Some("next tmrw 08:00"));
    }

    #[test]
    fn pit_cues_ring_for_each_match_and_follow_the_schedule_when_it_slips() {
        let z = NewYork;
        let mut view = saturday();
        let mut book = Book::memory();
        let made = utc(2026, 11, 21, 18, 0);
        {
            let n = Now { at: made, zone: &z, view: Some(&view) };
            add(&mut book, "after each match: swap battery, check bumpers", &n);
            add(&mut book, "10 min before our next match grab the cart", &n);
        }
        // "The next match" is tied to Q31 straight away.
        assert!(book.reminders.iter().any(|r| r.when == When::Match(Cue::Before(600), Of::One { label: "Q31".into(), key: Some("2026nyro_q31".into()) })));
        // The event runs 20 minutes late before Q31.
        for m in &mut view.matches {
            m.time = m.time.map(|t| t + 1200);
        }
        // Q31 was 14:00, now 14:20: the cart at 14:10, the battery at 14:26.
        let at = |t: i64| Now { at: t, zone: &z, view: Some(&view) };
        assert!(book.fire(&at(utc(2026, 11, 21, 19, 0)), true).is_empty(), "14:00 is not 10 min before any more");
        assert_eq!(book.fire(&at(utc(2026, 11, 21, 19, 10)), true).iter().map(|r| r.said.clone()).collect::<Vec<_>>(), ["grab the cart · Q31 in 10 min"]);
        let after = book.fire(&at(utc(2026, 11, 21, 19, 26)), true);
        assert_eq!(after.iter().map(|r| r.said.as_str()).collect::<Vec<_>>(), ["swap battery · after Q31", "check bumpers · after Q31"]);
        assert_eq!(island(&after), "swap battery · +1 more");
        // And again after Q40 (15:10 now), without being ticked.
        let after = book.fire(&at(utc(2026, 11, 21, 20, 16)), true);
        assert_eq!(after.iter().map(|r| r.said.as_str()).collect::<Vec<_>>(), ["swap battery · after Q40", "check bumpers · after Q40"]);
    }

    #[test]
    fn a_wait_moves_with_the_clock_when_it_is_set_but_a_clock_time_does_not() {
        let z = NewYork;
        let at = |t: i64| Now { at: t, zone: &z, view: None };
        let mut book = Book::memory();
        // Set while the clock was an hour slow.
        let slow = sat() - 3600;
        add(&mut book, "swap battery in 10 min", &at(slow));
        add(&mut book, "judges at 14:00", &at(slow));
        book.clock_moved(3600);
        assert_eq!(book.reminders[0].when, When::In(sat() + 600), "still ten minutes after it was set");
        assert_eq!(book.reminders[1].when, When::At(local(2026, 11, 21, 14, 0)), "still 14:00");
    }

    #[test]
    fn the_book_is_kept_and_read_back_and_a_bad_one_is_put_aside() {
        let dir = std::env::temp_dir().join(format!("catalyst-reminders-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let file = dir.join("reminders.json");
        let view = saturday();
        let z = NewYork;
        let n = Now { at: sat(), zone: &z, view: Some(&view) };
        let mut book = Book::open(file.clone());
        for typed in ["swap battery in 10 min", "every weekday at 07:30 bus #personal", "before Q42 charge spare", "after each match download logs", "between matches check chain", "buy zip ties", "check voltage every 30 min", "at queueing for Q40 bring the cart"] {
            add(&mut book, typed, &n);
        }
        let note = book.add(Parsed { text: "Elevator belt".into(), when: When::At(local(2026, 11, 21, 16, 0)), repeat: Repeat::Never, list: None, link: Link::Note("2026-11-20-1130.txt".into()) }, &n);
        book.fire(&Now { at: sat() + 700, ..n }, true);
        book.snooze(1, sat() + 1300);
        book.tick(6, &n, true);
        let back = Book::open(file.clone());
        assert_eq!(back.reminders, book.reminders);
        assert_eq!(back.next_id, book.next_id);
        assert_eq!(back.for_note("2026-11-20-1130.txt").map(|r| r.id), Some(note));
        // Unreadable: put aside, not written over.
        std::fs::write(&file, "{ not json").unwrap();
        let fresh = Book::open(file.clone());
        assert!(fresh.reminders.is_empty());
        assert!(dir.join("reminders.json.bad").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_lists_hold_what_they_should_in_order() {
        let view = saturday();
        let z = NewYork;
        let n = Now { at: sat(), zone: &z, view: Some(&view) };
        let mut book = Book::memory();
        add(&mut book, "judges at 14:05", &n);
        add(&mut book, "swap battery in 10 min", &n);
        add(&mut book, "buy zip ties", &n);
        add(&mut book, "tomorrow 8am check bumpers", &n);
        add(&mut book, "before Q99 charge spare", &n);
        let texts = |book: &Book, page| book.rows(page, None, &n, true).into_iter().map(|r| format!("{} | {}", r.text, r.when)).collect::<Vec<_>>();
        assert_eq!(texts(&book, Page::Today), ["swap battery | in 10 min", "judges | 14:05", "buy zip ties | no time"]);
        assert_eq!(texts(&book, Page::Upcoming), ["check bumpers | tmrw 08:00", "charge spare | Q99 not yet"]);
        // The pit's list alone: only the match cue is the pit's (no event on today).
        assert_eq!(book.rows(Page::Upcoming, Some(List::Pit), &n, true).len(), 1);
        // A match cue reads with its match.
        add(&mut book, "after Q31 download logs", &n);
        assert!(texts(&book, Page::Today).contains(&"download logs | Q31 14:06".to_owned()));
    }

    #[test]
    fn repeats_go_round_with_r() {
        let view = saturday();
        let z = NewYork;
        let n = Now { at: sat(), zone: &z, view: Some(&view) };
        let mut book = Book::memory();
        let id = add(&mut book, "tomorrow 8am check bumpers", &n);
        let said: Vec<String> = (0..4).filter_map(|_| book.cycle_repeat(id, &n)).collect();
        assert_eq!(said, ["every day", "weekdays", "every week", "once"]);
        let cue = add(&mut book, "after Q31 download logs", &n);
        assert_eq!(book.cycle_repeat(cue, &n).as_deref(), Some("every match"));
        assert_eq!(book.get(cue).unwrap().when, When::Match(Cue::After, Of::Each));
        assert_eq!(book.cycle_repeat(cue, &n).as_deref(), Some("just the next match"));
        assert!(matches!(book.get(cue).unwrap().when, When::Match(Cue::After, Of::One { .. })));
    }

    #[test]
    fn the_watch_rings_and_works_its_next_time_out_ahead() {
        let schedule = crate::schedule::Schedule::none();
        let mut watch = Watch::new();
        let start = Instant::now();
        let t0 = now();
        with(|s| {
            s.book = Book::memory();
            let n = Now { at: t0, zone: &Here, view: None };
            add(&mut s.book, "swap battery in 2 min", &n);
        });
        assert!(watch.tick_at(0.1, &schedule, true, t0, start).is_empty());
        assert_eq!(watch.due(), Some(t0 + 120), "worked out ahead");
        // Nothing else is worked out until it comes.
        assert!(watch.tick_at(1.0, &schedule, true, t0 + 1, start + std::time::Duration::from_secs(1)).is_empty());
        let rang = watch.tick_at(1.0, &schedule, true, t0 + 120, start + std::time::Duration::from_secs(120));
        assert_eq!(rang.len(), 1);
        assert_eq!(watch.due(), None);
        assert_eq!(with(|s| s.ringing.clone()), [rang[0].id]);
    }

    #[test]
    fn the_watch_sees_the_clock_set_and_moves_waits_with_it() {
        let schedule = crate::schedule::Schedule::none();
        let mut watch = Watch::new();
        let start = Instant::now();
        let t0 = now();
        with(|s| {
            s.book = Book::memory();
            let n = Now { at: t0, zone: &Here, view: None };
            add(&mut s.book, "swap battery in 10 min", &n);
        });
        watch.tick_at(0.1, &schedule, true, t0, start);
        // Five seconds later the network sets the clock an hour on.
        let rang = watch.tick_at(5.0, &schedule, true, t0 + 3605, start + std::time::Duration::from_secs(5));
        assert!(rang.is_empty(), "not an hour overdue: still ten minutes from when it was set");
        assert_eq!(watch.due(), Some(t0 + 3600 + 600));
    }

    #[test]
    fn before_the_clock_is_set_nothing_is_said() {
        let schedule = crate::schedule::Schedule::none();
        let mut watch = Watch::new();
        with(|s| {
            s.book = Book::memory();
            s.book.add(Parsed { text: "x".into(), when: When::In(100), repeat: Repeat::Never, list: None, link: Link::None }, &Now { at: 0, zone: &Here, view: None });
        });
        assert!(watch.tick_at(0.1, &schedule, true, 200, Instant::now()).is_empty());
        assert_eq!(watch.due(), None);
    }

    #[test]
    fn other_screens_get_what_is_coming_and_what_matches() {
        let t0 = now();
        with(|s| {
            s.book = Book::memory();
            s.view = None;
            let n = Now { at: t0, zone: &Here, view: None };
            add(&mut s.book, "swap battery in 10 min", &n);
            add(&mut s.book, "check bumpers in 3 min", &n);
            add(&mut s.book, "buy zip ties", &n);
            let id = add(&mut s.book, "charge spare in 1 min", &n);
            s.book.tick(id, &n, true);
        });
        let up = upcoming(5);
        assert_eq!(up.iter().map(|u| u.text.as_str()).collect::<Vec<_>>(), ["check bumpers", "swap battery"], "timed, not done, soonest first");
        assert_eq!(up[0].when, "in 3 min");
        assert_eq!(upcoming(1).len(), 1);
        let found = search("batt", 5);
        assert_eq!(found.iter().map(|f| (f.name.as_str(), f.rank)).collect::<Vec<_>>(), [("swap battery", 1)]);
        assert_eq!(search("charge", 5)[0].value, "done");
        // (Unless ten minutes from now is tomorrow already.)
        if Here.local(t0 + 600).date() == Here.local(t0).date() {
            assert_eq!(tile(), ("3 today".to_owned(), None));
        }
    }

    #[test]
    fn a_deleted_note_takes_its_reminder() {
        with(|s| {
            s.book = Book::memory();
            let n = Now { at: now(), zone: &Here, view: None };
            s.book.add(Parsed { text: "Belt".into(), when: When::In(now() + 600), repeat: Repeat::Never, list: None, link: Link::Note("a.txt".into()) }, &n);
        });
        assert_eq!(noted(), ["a.txt"]);
        note_deleted("a.txt");
        assert!(noted().is_empty());
    }
}
