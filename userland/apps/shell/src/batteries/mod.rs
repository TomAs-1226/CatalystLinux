//! Batteries: the team's robot batteries, which went in for each match, how
//! each is holding up, and which to put in next. A team brings 6 to 12 to an
//! event, and a tired one browns the robot out.
//!
//! - **The fleet**: each battery by its name or number, and its year, kept
//!   in `/data/batteries/fleet.json`. A sheet of QR labels for them
//!   (`labels.png`, label.rs) is there for Catalyst Companion to download;
//!   a label read in the Camera's scan mode opens its battery.
//! - **Per match**: which battery went in, picked before queueing. When the
//!   next match's queue time nears (ten minutes; twenty before the match
//!   with no queue time) and none is picked, the island asks, and Batteries
//!   opens on the pick with the suggested battery first.
//! - **Health**: once a match's log is on the handheld, the battery that was
//!   in for it is read from it (health.rs): its start and low volts, its
//!   sag, its resistance where the log has the current, its brownouts. Its
//!   trend over its matches, and a flag to retire it soon.
//! - **Charging and rest**: on the charger and off it; resting a while once
//!   off; a reminder (Reminders) once it has been on long enough. The next
//!   battery is the charged, rested one in the best health.
//!
//! The times below are my own tuning: a match takes 3 to 4 Ah, which a 6 A
//! charger puts back in under an hour; a battery fresh off the charger is
//! warm and carries a surface charge for a quarter of an hour.

mod app;
pub mod health;
pub mod label;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

pub use app::open;
use health::{Health, Reading};

/// How long on the charger counts as charged (and the reminder comes).
pub const CHARGE_SECONDS: i64 = 60 * 60;
/// How long off the charger before it is rested.
pub const REST_SECONDS: i64 = 15 * 60;
/// The island asks this long before the queue time, or before the match
/// itself when there is no queue time.
pub const ASK_BEFORE_QUEUE: i64 = 10 * 60;
pub const ASK_BEFORE_MATCH: i64 = 20 * 60;
/// The longest name a battery takes.
pub const NAME_MOST: usize = 12;
/// What a battery's QR label says, before its id.
pub const QR_PREFIX: &str = "catalyst-battery:";
/// The label given a battery put in with no match coming.
pub const PRACTICE: &str = "practice";

// ------------------------------------------------------------- the model ---

/// Where a battery is in its round: used (in a match, or back from one),
/// on the charger, or off it (resting, and then charged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    Used { since: i64 },
    Charging { since: i64 },
    Off { since: i64 },
}

/// What a battery is, as shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// On the charger (`full` once it has been on long enough).
    Charging { full: bool },
    /// Off the charger, and rested.
    Charged,
    /// Off the charger, not yet rested.
    Resting,
    Used,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Charging { full: false } => "charging",
            State::Charging { full: true } => "charged",
            State::Charged => "charged",
            State::Resting => "resting",
            State::Used => "used",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            State::Charging { .. } => "bolt",
            State::Charged => "battery_full",
            State::Resting => "bedtime",
            State::Used => "history",
        }
    }

    /// How ready, for the next pick: lower is readier.
    fn tier(self) -> u8 {
        match self {
            State::Charged => 0,
            State::Resting | State::Charging { full: true } => 1,
            State::Charging { full: false } => 2,
            State::Used => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    /// Its name made into a key: what its QR label carries.
    pub id: String,
    pub name: String,
    pub year: Option<u16>,
    pub charge: Charge,
    /// The reminder to take it off the charger, while it is on.
    pub reminder: Option<u64>,
    pub added: i64,
}

impl Battery {
    pub fn state(&self, now: i64) -> State {
        match self.charge {
            Charge::Used { .. } => State::Used,
            Charge::Charging { since } => State::Charging { full: now - since >= CHARGE_SECONDS },
            Charge::Off { since } if now - since < REST_SECONDS => State::Resting,
            Charge::Off { .. } => State::Charged,
        }
    }

    /// Seconds of rest still to go, while resting.
    pub fn rest_left(&self, now: i64) -> Option<i64> {
        match self.charge {
            Charge::Off { since } if now - since < REST_SECONDS => Some(REST_SECONDS - (now - since)),
            _ => None,
        }
    }

    /// Seconds on the charger, while on it.
    pub fn charging_for(&self, now: i64) -> Option<i64> {
        match self.charge {
            Charge::Charging { since } => Some((now - since).max(0)),
            _ => None,
        }
    }

    /// "7 · 2024", or its name alone.
    pub fn title(&self) -> String {
        match self.year {
            Some(y) => format!("{} · {y}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A battery put in for a match, and what the match's log said of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub battery: String,
    /// "Q31", or [`PRACTICE`].
    pub label: String,
    pub at: i64,
    /// Where it was before it went in, to put it back if another is picked instead.
    pub before: Charge,
    /// The log read for it, once there is one.
    pub log: Option<String>,
    pub reading: Option<Reading>,
}

/// How the fleet is ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Health,
    Ready,
    Name,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Health => Sort::Ready,
            Sort::Ready => Sort::Name,
            Sort::Name => Sort::Health,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Sort::Health => "by health",
            Sort::Ready => "ready first",
            Sort::Name => "by name",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fleet {
    pub batteries: Vec<Battery>,
    /// Oldest first.
    pub uses: Vec<Use>,
    /// The matches the island has asked about.
    pub asked: Vec<String>,
    pub sort: Option<Sort>,
}

/// A name made into a key: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

impl Fleet {
    pub fn get(&self, id: &str) -> Option<&Battery> {
        self.batteries.iter().find(|b| b.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Battery> {
        self.batteries.iter_mut().find(|b| b.id == id)
    }

    /// Add a battery; its id, or why not. A new one is taken as charged.
    pub fn add(&mut self, name: &str, year: Option<u16>, now: i64) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("give it a name or number".into());
        }
        if name.chars().count() > NAME_MOST {
            return Err(format!("at most {NAME_MOST} letters"));
        }
        let base = slug(name);
        if base.is_empty() {
            return Err("letters or digits, please".into());
        }
        if self.batteries.iter().any(|b| b.name.eq_ignore_ascii_case(name) && b.year == year) {
            return Err(format!("{name} is here already"));
        }
        let mut id = base.clone();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        self.batteries.push(Battery { id: id.clone(), name: name.to_owned(), year, charge: Charge::Off { since: now - REST_SECONDS }, reminder: None, added: now });
        Ok(id)
    }

    /// Take a battery out of the fleet, with its matches.
    pub fn remove(&mut self, id: &str) -> Option<Battery> {
        let at = self.batteries.iter().position(|b| b.id == id)?;
        self.uses.retain(|u| u.battery != id);
        Some(self.batteries.remove(at))
    }

    /// On the charger, from wherever it was.
    pub fn put_on(&mut self, id: &str, now: i64) -> Result<(), String> {
        let b = self.get_mut(id).ok_or("that battery is gone")?;
        if matches!(b.charge, Charge::Charging { .. }) {
            return Err(format!("{} is on the charger already", b.name));
        }
        b.charge = Charge::Charging { since: now };
        Ok(())
    }

    /// Off the charger: it rests from now.
    pub fn take_off(&mut self, id: &str, now: i64) -> Result<(), String> {
        let b = self.get_mut(id).ok_or("that battery is gone")?;
        if !matches!(b.charge, Charge::Charging { .. }) {
            return Err(format!("{} is not on the charger", b.name));
        }
        b.charge = Charge::Off { since: now };
        Ok(())
    }

    /// The battery picked for a match. Picking another for the same match
    /// puts the first back where it was. Says what to know about the one
    /// picked (still charging, still resting), if anything.
    pub fn assign(&mut self, id: &str, label: &str, now: i64) -> Result<Option<String>, String> {
        let b = self.get(id).ok_or("that battery is gone")?;
        let caution = match b.state(now) {
            State::Charging { full: false } => Some(format!("{} was still charging", b.name)),
            State::Resting => Some(format!("{} had {} of rest left", b.name, minutes(b.rest_left(now).unwrap_or(0)))),
            State::Used => Some(format!("{} was used and not charged", b.name)),
            _ => None,
        };
        let before = b.charge;
        if label != PRACTICE {
            if let Some(at) = self.uses.iter().position(|u| u.label == label && u.reading.is_none()) {
                if self.uses[at].battery == id {
                    return Ok(None);
                }
                let old = self.uses.remove(at);
                if let Some(o) = self.get_mut(&old.battery) {
                    o.charge = old.before;
                }
            }
        }
        // Out of the charger's hands: its reminder is no longer needed.
        let b = self.get_mut(id).ok_or("that battery is gone")?;
        b.charge = Charge::Used { since: now };
        self.uses.push(Use { battery: id.to_owned(), label: label.to_owned(), at: now, before, log: None, reading: None });
        Ok(caution)
    }

    /// The battery picked for a match not yet read from its log.
    pub fn battery_for(&self, label: &str) -> Option<&str> {
        self.uses.iter().rev().find(|u| u.label.eq_ignore_ascii_case(label)).map(|u| u.battery.as_str())
    }

    /// The match a battery went in for last.
    pub fn last_use(&self, id: &str) -> Option<&Use> {
        self.uses.iter().rev().find(|u| u.battery == id)
    }

    /// Its matches read from their logs, oldest first.
    pub fn readings(&self, id: &str) -> Vec<(&str, &Reading)> {
        self.uses.iter().filter(|u| u.battery == id).filter_map(|u| Some((u.label.as_str(), u.reading.as_ref()?))).collect()
    }

    pub fn health(&self, id: &str) -> Health {
        health::health(&self.readings(id))
    }

    /// The batteries in pick order: the readiest first; among equals, a
    /// battery not flagged to retire, then the healthiest, then the one
    /// used least lately. The battery in the robot is last.
    pub fn ranked(&self, now: i64) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.batteries.len()).collect();
        let keys: Vec<(u8, bool, i64, i64, i64)> = self
            .batteries
            .iter()
            .map(|b| {
                let h = self.health(&b.id);
                let score = h.score.map_or(-1, |s| -(s.round() as i64));
                let last = self.last_use(&b.id).map_or(i64::MIN, |u| u.at);
                // Among those resting or charging, the one nearest ready.
                let nearer = match b.charge {
                    Charge::Off { since } | Charge::Charging { since } => since,
                    Charge::Used { since } => since,
                };
                (b.state(now).tier(), h.retire.is_some(), score, nearer, last)
            })
            .collect();
        order.sort_by(|&a, &b| {
            let (ka, kb) = (&keys[a], &keys[b]);
            ka.0.cmp(&kb.0).then(ka.1.cmp(&kb.1)).then_with(|| if ka.0 == 0 { ka.2.cmp(&kb.2).then(ka.4.cmp(&kb.4)) } else { ka.3.cmp(&kb.3).then(ka.2.cmp(&kb.2)) })
        });
        order
    }

    /// The next battery to put in: charged and rested in the best health,
    /// else the one nearest ready. None with no battery that is not used.
    pub fn next(&self, now: i64) -> Option<&Battery> {
        self.ranked(now).into_iter().map(|i| &self.batteries[i]).find(|b| b.state(now) != State::Used)
    }

    /// The fleet in the order asked for.
    pub fn sorted(&self, sort: Sort, now: i64) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.batteries.len()).collect();
        match sort {
            Sort::Ready => order = self.ranked(now),
            Sort::Name => order.sort_by(|&a, &b| natural(&self.batteries[a].name, &self.batteries[b].name)),
            Sort::Health => {
                let scores: Vec<f64> = self.batteries.iter().map(|b| self.health(&b.id).score.unwrap_or(-1.0)).collect();
                order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(natural(&self.batteries[a].name, &self.batteries[b].name)));
            }
        }
        order
    }

    /// Logs to read: (use's place, the log's name) for matches whose
    /// battery is known and whose log has arrived, each log once.
    pub fn logs_wanted(&self, logs: &[String]) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, u) in self.uses.iter().enumerate() {
            if u.reading.is_some() || u.log.is_some() || u.label == PRACTICE {
                continue;
            }
            let found = logs.iter().find(|name| {
                crate::offline::facts::label_from_name(name).is_some_and(|l| l.eq_ignore_ascii_case(&u.label)) && !self.uses.iter().any(|o| o.log.as_deref() == Some(name.as_str())) && !out.iter().any(|(_, n): &(usize, String)| n == *name)
            });
            if let Some(name) = found {
                out.push((i, name.clone()));
            }
        }
        out
    }

    // ---- kept ----

    pub fn to_json(&self) -> Value {
        let charge = |c: &Charge| match *c {
            Charge::Used { since } => json!({ "state": "used", "since": since }),
            Charge::Charging { since } => json!({ "state": "charging", "since": since }),
            Charge::Off { since } => json!({ "state": "off", "since": since }),
        };
        json!({
            "batteries": self.batteries.iter().map(|b| json!({ "id": b.id, "name": b.name, "year": b.year, "charge": charge(&b.charge), "reminder": b.reminder, "added": b.added })).collect::<Vec<_>>(),
            "uses": self.uses.iter().map(|u| json!({ "battery": u.battery, "match": u.label, "at": u.at, "before": charge(&u.before), "log": u.log, "reading": u.reading.as_ref().map(Reading::to_json) })).collect::<Vec<_>>(),
            "asked": self.asked,
            "sort": self.sort.map(|s| s.word()),
        })
    }

    pub fn from_json(v: &Value) -> Fleet {
        let charge = |c: &Value| -> Option<Charge> {
            let since = c["since"].as_i64()?;
            Some(match c["state"].as_str()? {
                "used" => Charge::Used { since },
                "charging" => Charge::Charging { since },
                _ => Charge::Off { since },
            })
        };
        let text = |v: &Value| v.as_str().map(str::to_owned);
        let batteries = v["batteries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| {
                        Some(Battery { id: text(&b["id"])?, name: text(&b["name"])?, year: b["year"].as_u64().map(|y| y as u16), charge: charge(&b["charge"])?, reminder: b["reminder"].as_u64(), added: b["added"].as_i64().unwrap_or(0) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let uses = v["uses"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|u| {
                        Some(Use { battery: text(&u["battery"])?, label: text(&u["match"])?, at: u["at"].as_i64()?, before: charge(&u["before"])?, log: text(&u["log"]), reading: Reading::from_json(&u["reading"]) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let asked = v["asked"].as_array().map(|a| a.iter().filter_map(|s| text(s)).collect()).unwrap_or_default();
        let sort = [Sort::Health, Sort::Ready, Sort::Name].into_iter().find(|s| v["sort"].as_str() == Some(s.word()));
        Fleet { batteries, uses, asked, sort }
    }
}

/// Names in the order people count: "2" before "10".
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let num = |s: &str| s.trim().parse::<u64>().ok();
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// "6 min", "1 h 05".
pub fn minutes(seconds: i64) -> String {
    let m = (seconds.max(0) + 59) / 60;
    if m >= 60 {
        format!("{} h {:02}", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// "6m", "1h05": for a card's few characters.
pub fn short(seconds: i64) -> String {
    let m = (seconds.max(0) + 59) / 60;
    if m >= 60 {
        format!("{}h{:02}", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

// ------------------------------------------------------------- the store ---

/// Where the fleet is kept: `/data/batteries` on the handheld.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/batteries")
    } else {
        crate::config_dir().join("batteries")
    }
}

#[cfg(not(test))]
fn load(path: &Path) -> Fleet {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).map(|v| Fleet::from_json(&v)).unwrap_or_default()
}

/// Changes since start: screens redraw when it moves.
static GENERATION: AtomicU64 = AtomicU64::new(1);
/// The newest write asked for: an older one finding a newer leaves it be.
static WRITES: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Kept on a thread of its own, never the frame's time, through a file
/// beside it so a write cut short loses nothing.
fn save(fleet: &Fleet) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    if cfg!(test) {
        return;
    }
    let text = serde_json::to_string_pretty(&fleet.to_json()).unwrap_or_default();
    let seq = WRITES.fetch_add(1, Ordering::Relaxed) + 1;
    let dir = dir();
    std::thread::Builder::new()
        .name("batteries".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            static ONE: Mutex<()> = Mutex::new(());
            let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
            if WRITES.load(Ordering::Relaxed) != seq || std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            let (part, path) = (dir.join("fleet.json.part"), dir.join("fleet.json"));
            if std::fs::write(&part, text).is_ok() {
                std::fs::rename(&part, &path).ok();
            }
        })
        .ok();
}

#[cfg(not(test))]
static STORE: Mutex<Option<Fleet>> = Mutex::new(None);

/// The fleet, read once from the card, shared by the app, the island's
/// question and the logs' reading. Changes made in `f` are kept.
#[cfg(not(test))]
pub fn with<T>(f: impl FnOnce(&mut Fleet) -> T) -> T {
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let fleet = store.get_or_insert_with(|| load(&dir().join("fleet.json")));
    let before = fleet.clone();
    let out = f(fleet);
    if *fleet != before {
        save(fleet);
    }
    out
}

// The tests run many at once, one to a thread: each has its own, in memory.
#[cfg(test)]
thread_local! {
    static STORE: std::cell::RefCell<Fleet> = std::cell::RefCell::new(Fleet::default());
}

#[cfg(test)]
pub fn with<T>(f: impl FnOnce(&mut Fleet) -> T) -> T {
    STORE.with(|s| {
        let mut fleet = s.borrow_mut();
        let before = fleet.clone();
        let out = f(&mut fleet);
        if *fleet != before {
            save(&fleet);
        }
        out
    })
}

/// Read without the copy `with` makes to see what changed.
#[cfg(not(test))]
pub fn read<T>(f: impl FnOnce(&Fleet) -> T) -> T {
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(store.get_or_insert_with(|| load(&dir().join("fleet.json"))))
}

#[cfg(test)]
pub fn read<T>(f: impl FnOnce(&Fleet) -> T) -> T {
    STORE.with(|s| f(&s.borrow()))
}

/// For the gallery: the fleet in use replaced.
#[cfg(not(target_arch = "aarch64"))]
pub fn pretend(fleet: Fleet) {
    with(|f| *f = fleet);
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ------------------------------------------------------------- actions ---

/// The charger, for one battery: on if it is off, off if it is on. A
/// reminder to take it off comes once it has been on long enough. What
/// the island says.
pub fn charger(id: &str) -> String {
    let now = now();
    let outcome = with(|f| {
        let b = f.get(id)?.clone();
        Some(if matches!(b.charge, Charge::Charging { .. }) {
            f.take_off(id, now).map(|_| (false, b))
        } else {
            f.put_on(id, now).map(|_| (true, b))
        })
    });
    match outcome {
        None => "that battery is gone".into(),
        Some(Err(why)) => why,
        Some(Ok((true, b))) => {
            let reminder = remind_charged(&b.name, now);
            with(|f| f.get_mut(id).map(|b| b.reminder = Some(reminder)));
            format!("{} on the charger", b.name)
        }
        Some(Ok((false, b))) => {
            if let Some(r) = b.reminder {
                forget_reminder(r);
            }
            with(|f| f.get_mut(id).map(|b| b.reminder = None));
            format!("{} off · rested in {}", b.name, minutes(REST_SECONDS))
        }
    }
}

/// A battery put in for a match: what the island says.
pub fn pick(id: &str, label: &str) -> String {
    let now = now();
    let reminder = read(|f| f.get(id).and_then(|b| b.reminder));
    match with(|f| f.assign(id, label, now).map(|caution| (caution, f.get(id).map(|b| b.name.clone()).unwrap_or_default()))) {
        Ok((caution, name)) => {
            if let Some(r) = reminder {
                forget_reminder(r);
                with(|f| f.get_mut(id).map(|b| b.reminder = None));
            }
            let what = if label == PRACTICE { format!("{name} in for practice") } else { format!("{name} in for {label}") };
            caution.map_or(what.clone(), |c| format!("{what} · {c}"))
        }
        Err(why) => why,
    }
}

fn remind_charged(name: &str, now: i64) -> u64 {
    use crate::reminders::{parse::Parsed, plan, Link, List, Repeat, When};
    crate::reminders::with(|s| {
        let n = plan::Now { at: now, zone: &plan::Here, view: None };
        s.book.add(Parsed { text: format!("battery {name} charged: take it off"), when: When::In(now + CHARGE_SECONDS), repeat: Repeat::Never, list: Some(List::Pit), link: Link::None }, &n)
    })
}

fn forget_reminder(id: u64) {
    crate::reminders::with(|s| s.book.delete(id));
}

/// A battery's page from what its QR label says, read by the Camera.
pub fn scanned(text: &str) -> Option<Box<dyn crate::shell::Screen>> {
    let id = text.trim().strip_prefix(QR_PREFIX)?;
    read(|f| f.get(id).is_some()).then(|| Box::new(app::Detail::new(id)) as Box<dyn crate::shell::Screen>)
}

/// Home's line under the tile.
pub fn tile() -> (String, Option<&'static str>) {
    let now = now();
    read(|f| {
        if f.batteries.is_empty() {
            return ("add the fleet".into(), None);
        }
        let ready = f.batteries.iter().filter(|b| b.state(now) == State::Charged).count();
        let retire = f.batteries.iter().any(|b| f.health(&b.id).retire.is_some());
        match f.next(now) {
            Some(b) if ready > 0 => (format!("{} next · {ready} ready", b.name), retire.then_some("WARN")),
            _ => (format!("none ready · {}", f.batteries.len()), retire.then_some("WARN")),
        }
    })
}

// ------------------------------------------------------------- watching ---

/// What the shell looks at a few times a minute: the queue nearing with no
/// battery picked (the island asks), and logs come in for matches a battery
/// was in (read on a thread of their own).
pub struct Watch {
    looked: Option<std::time::Instant>,
    logs_seen: u64,
}

/// How often the shell's watch looks.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// While a log is being read, no other reading starts.
static READING: AtomicBool = AtomicBool::new(false);

impl Watch {
    pub fn new() -> Watch {
        Watch { looked: None, logs_seen: u64::MAX }
    }

    /// A question for the island, when one is due.
    pub fn tick(&mut self, robot: &crate::robot::Robot) -> Option<crate::notify::Notification> {
        // The tests' shells keep to themselves: only a battery test asks.
        if cfg!(test) && !testing::watching() {
            return None;
        }
        if self.looked.is_some_and(|at| at.elapsed() < WATCH_EVERY) {
            return None;
        }
        self.looked = Some(std::time::Instant::now());
        if read(|f| f.batteries.is_empty()) {
            return None;
        }
        let generation = robot.logs.generation();
        if generation != self.logs_seen && !READING.load(Ordering::Relaxed) {
            self.logs_seen = generation;
            read_logs(&robot.logs);
        }
        let view = robot.schedule.view()?;
        ask(&view, now())
    }
}

#[cfg(test)]
pub mod testing {
    thread_local! {
        static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    pub fn watching() -> bool {
        WATCHING.with(std::cell::Cell::get)
    }
    pub fn watch(on: bool) {
        WATCHING.with(|w| w.set(on));
    }
}

#[cfg(not(test))]
mod testing {
    pub fn watching() -> bool {
        false
    }
}

/// The match coming that the island should ask about, if any: the next
/// one, its queue (or the match) near, no battery picked for it, not
/// asked about already.
pub fn due(view: &crate::schedule::View, now: i64) -> Option<String> {
    let m = view.next(now)?;
    let near = match m.queue_at {
        Some(q) => q - now <= ASK_BEFORE_QUEUE,
        None => m.time.is_some_and(|t| t - now <= ASK_BEFORE_MATCH),
    };
    near.then(|| m.label.clone())
}

/// The island's question, once a match: which battery, the next one named.
pub fn ask(view: &crate::schedule::View, now: i64) -> Option<crate::notify::Notification> {
    let label = due(view, now)?;
    let next = with(|f| {
        if f.batteries.is_empty() || f.battery_for(&label).is_some() || f.asked.iter().any(|a| a.eq_ignore_ascii_case(&label)) {
            return None;
        }
        f.asked.push(label.clone());
        let keep = f.asked.len().saturating_sub(24);
        f.asked.drain(..keep);
        Some(f.next(now).map(|b| (b.name.clone(), b.state(now))))
    })?;
    // The island says the title alone (the body, longer than it says, is
    // for the notification centre): whole at every text size.
    let (title, body) = match next {
        Some((name, State::Charged)) => (format!("put {name} in for {label}?"), "charged and rested, the healthiest ready".to_owned()),
        Some((name, state)) => (format!("put {name} in for {label}?"), format!("none is ready: it is nearest, {}", state.word())),
        None => (format!("no battery charged for {label}"), "every battery is used: pick one in Batteries".to_owned()),
    };
    Some(crate::notify::Notification::new("Batteries", "battery_charging_full", title).body(body).tag(format!("batteries:{}", label.to_lowercase())).action("builtin:Batteries"))
}

/// The match asked about that is still waiting for its battery, for the app
/// to open on the pick: the next match, near enough, none picked.
pub fn waiting(view: Option<&crate::schedule::View>, now: i64) -> Option<String> {
    let label = due(view?, now)?;
    read(|f| (!f.batteries.is_empty() && f.battery_for(&label).is_none()).then_some(label))
}

/// The logs of matches a battery was in, read on a thread of their own,
/// each once; what they say kept with the battery. A battery newly flagged
/// to retire is a notification.
fn read_logs(logs: &crate::logs::Logs) {
    let rows: Vec<String> = logs.list().into_iter().map(|r| r.name).collect();
    let wanted = read(|f| f.logs_wanted(&rows));
    if wanted.is_empty() {
        return;
    }
    let (dir, demo) = (logs.dir().map(Path::to_path_buf), logs.is_demo());
    READING.store(true, Ordering::Relaxed);
    let spawned = std::thread::Builder::new()
        .name("battery-logs".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            crate::sched::enter(crate::sched::Class::Background);
            for (at, name) in wanted {
                let reading = match (&dir, demo) {
                    (Some(dir), _) => std::fs::File::open(dir.join(&name)).ok().and_then(|f| health::analyse(std::io::BufReader::with_capacity(64 * 1024, f)).ok().flatten()),
                    (None, true) => crate::offline::synth::demo_event().into_iter().find(|s| s.name == name).and_then(|s| health::analyse(&crate::offline::synth::write(&s)[..]).ok().flatten()),
                    _ => None,
                };
                if let Some(n) = record(at, &name, reading) {
                    crate::notify::post(n);
                }
            }
            READING.store(false, Ordering::Relaxed);
        });
    if spawned.is_err() {
        READING.store(false, Ordering::Relaxed);
    }
}

/// What a match's log said, kept with the battery that was in for it.
/// A notification if it newly flags the battery to retire.
pub fn record(at: usize, log: &str, reading: Option<Reading>) -> Option<crate::notify::Notification> {
    with(|f| {
        let u = f.uses.get(at)?;
        let id = u.battery.clone();
        let flagged = f.health(&id).retire.is_some();
        let u = &mut f.uses[at];
        u.log = Some(log.to_owned());
        u.reading = reading;
        let label = u.label.clone();
        let why = f.health(&id).retire.filter(|_| !flagged)?;
        let name = f.get(&id)?.name.clone();
        Some(crate::notify::Notification::new("Batteries", "battery_alert", format!("retire {name} soon")).body(format!("{why} · after {label}")).tag(format!("batteries:retire:{id}")).action("builtin:Batteries"))
    })
}
