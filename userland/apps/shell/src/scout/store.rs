//! Records: one per team per match, each a small JSON file under
//! `records/<event>/`, with the history of who changed it and when. Records
//! from another handheld merge in: one already here is not added twice, and
//! of two versions the one edited last wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The six places on the field a scout watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Station(pub u8);

impl Station {
    pub const ALL: [Station; 6] = [Station(0), Station(1), Station(2), Station(3), Station(4), Station(5)];

    pub fn red(self) -> bool {
        self.0 < 3
    }

    /// 1, 2 or 3 along its alliance's wall.
    pub fn number(self) -> u8 {
        self.0 % 3 + 1
    }

    /// "red 2".
    pub fn label(self) -> String {
        format!("{} {}", if self.red() { "red" } else { "blue" }, self.number())
    }

    /// "r2", as records and codes carry it.
    pub fn short(self) -> String {
        format!("{}{}", if self.red() { 'r' } else { 'b' }, self.number())
    }

    pub fn parse(s: &str) -> Option<Station> {
        let s = s.trim().to_ascii_lowercase();
        let (side, n) = s.split_at(s.len().checked_sub(1)?);
        let n: u8 = n.parse().ok().filter(|n| (1..=3).contains(n))?;
        match side.trim() {
            "r" | "red" => Some(Station(n - 1)),
            "b" | "blue" => Some(Station(n + 2)),
            _ => None,
        }
    }

    /// The team at this station in a match.
    pub fn team_in(self, m: &crate::schedule::Match) -> Option<u32> {
        let side = if self.red() { &m.red } else { &m.blue };
        side.get(self.number() as usize - 1).copied()
    }

    pub fn next(self, by: i32) -> Station {
        Station((self.0 as i32 + by).rem_euclid(6) as u8)
    }
}

/// One change to a record: when, by whom, and what changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub at: i64,
    pub by: String,
    pub change: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Record {
    pub event: String,
    /// The Blue Alliance's key: `2026nyro_qm31`.
    pub match_key: String,
    /// "Q31".
    pub label: String,
    pub team: u32,
    pub station: Station,
    pub form: String,
    pub values: BTreeMap<String, f32>,
    pub texts: BTreeMap<String, String>,
    /// Who scouted it.
    pub scout: String,
    /// When it was last changed, Unix seconds, and by whom.
    pub updated: i64,
    pub by: String,
    pub history: Vec<Version>,
}

/// How long a history is kept, newest last.
const HISTORY_MOST: usize = 40;

impl Record {
    pub fn id(&self) -> String {
        format!("{}-{}", self.match_key, self.team)
    }

    pub fn value(&self, column: &str) -> f32 {
        self.values.get(column).copied().unwrap_or(0.0)
    }

    /// Whether two records say the same about the match.
    pub fn same_content(&self, other: &Record) -> bool {
        let trimmed = |m: &BTreeMap<String, f32>| m.iter().filter(|(_, v)| **v != 0.0).map(|(k, v)| (k.clone(), v.to_bits())).collect::<Vec<_>>();
        let words = |m: &BTreeMap<String, String>| m.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>();
        trimmed(&self.values) == trimmed(&other.values) && words(&self.texts) == words(&other.texts) && self.station == other.station
    }

    /// What changed from `before`, in a line: "teleop fuel in 12 → 14 · notes".
    pub fn change_from(&self, before: &Record, form: &super::form::Form) -> String {
        let mut parts = Vec::new();
        for f in &form.fields {
            let c = f.column(form);
            match f.kind {
                super::form::Kind::Text(_) => {
                    if self.texts.get(&c).map_or("", String::as_str) != before.texts.get(&c).map_or("", String::as_str) {
                        parts.push(f.label.clone());
                    }
                }
                _ => {
                    let (a, b) = (before.value(&c), self.value(&c));
                    if a != b {
                        parts.push(format!("{} {} → {}", form.metric_label(&super::form::Metric::Field(form.fields.iter().position(|x| x == f).unwrap_or(0))), f.shown(a), f.shown(b)));
                    }
                }
            }
        }
        parts.join(" · ")
    }

    /// Note an edit: the time and who, and the line saying what changed.
    pub fn stamp(&mut self, at: i64, by: &str, change: String) {
        self.updated = at;
        self.by = by.to_owned();
        self.history.push(Version { at, by: by.to_owned(), change });
        if self.history.len() > HISTORY_MOST {
            self.history.drain(..self.history.len() - HISTORY_MOST);
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "event": self.event, "match": self.match_key, "label": self.label, "team": self.team,
            "station": self.station.short(), "form": self.form, "values": self.values, "texts": self.texts,
            "scout": self.scout, "updated": self.updated, "by": self.by,
            "history": self.history.iter().map(|v| json!({"at": v.at, "by": v.by, "change": v.change})).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(v: &Value) -> Option<Record> {
        Some(Record {
            event: v["event"].as_str()?.to_owned(),
            match_key: v["match"].as_str()?.to_owned(),
            label: v["label"].as_str().unwrap_or("").to_owned(),
            team: v["team"].as_u64()? as u32,
            station: v["station"].as_str().and_then(Station::parse).unwrap_or_default(),
            form: v["form"].as_str().unwrap_or("").to_owned(),
            values: v["values"].as_object().map(|o| o.iter().filter_map(|(k, n)| Some((k.clone(), n.as_f64()? as f32))).collect()).unwrap_or_default(),
            texts: v["texts"].as_object().map(|o| o.iter().filter_map(|(k, s)| Some((k.clone(), s.as_str()?.to_owned()))).collect()).unwrap_or_default(),
            scout: v["scout"].as_str().unwrap_or("").to_owned(),
            updated: v["updated"].as_i64().unwrap_or(0),
            by: v["by"].as_str().unwrap_or("").to_owned(),
            history: v["history"]
                .as_array()
                .map(|a| a.iter().filter_map(|h| Some(Version { at: h["at"].as_i64()?, by: h["by"].as_str()?.to_owned(), change: h["change"].as_str().unwrap_or("").to_owned() })).collect())
                .unwrap_or_default(),
        })
    }
}

/// What merging a record in did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merged {
    /// New here.
    Added,
    /// Newer than the one here, which it replaced.
    Updated,
    /// The one here was as new or newer: kept.
    Kept,
}

/// Merge `incoming` into `here` (if there is one): the version edited last
/// wins (on the same moment, the later name, so every handheld agrees), and
/// the histories join, each change once.
pub fn merge(here: Option<&Record>, incoming: &Record) -> (Merged, Record) {
    let Some(here) = here else { return (Merged::Added, incoming.clone()) };
    // The same version come back (this handheld's own, read from a code):
    // nothing to add, not even to the history.
    if incoming.updated == here.updated && incoming.same_content(here) {
        return (Merged::Kept, here.clone());
    }
    let newer = (incoming.updated, &incoming.by) > (here.updated, &here.by);
    let mut out = if newer { incoming.clone() } else { here.clone() };
    let mut history: Vec<Version> = here.history.iter().chain(incoming.history.iter()).cloned().collect();
    history.sort_by(|a, b| (a.at, &a.by, &a.change).cmp(&(b.at, &b.by, &b.change)));
    history.dedup();
    if history.len() > HISTORY_MOST {
        history.drain(..history.len() - HISTORY_MOST);
    }
    out.history = history;
    (if newer { Merged::Updated } else { Merged::Kept }, out)
}

/// The order matches are played in, from a TBA key's tail: `qm31`, `sf2m1`, `f1m2`.
pub fn match_order(key: &str) -> (u8, u32, u32) {
    let tail = key.rsplit('_').next().unwrap_or(key);
    let level_len = tail.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    let (level, rest) = tail.split_at(level_len);
    let rank = match level {
        "qm" => 0,
        "ef" => 1,
        "qf" => 2,
        "sf" => 3,
        "f" => 4,
        _ => 5,
    };
    let (set, number) = match rest.split_once('m') {
        Some((s, n)) => (s.parse().unwrap_or(0), n.parse().unwrap_or(0)),
        None => (0, rest.parse().unwrap_or(0)),
    };
    (rank, set, number)
}

/// Every record kept, in memory, with the folder they are kept in.
pub struct Store {
    root: PathBuf,
    pub records: Vec<Record>,
}

/// A name safe as a file's or a folder's: letters, digits, `_` and `-`.
fn safe(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if s.is_empty() { "event".into() } else { s }
}

impl Store {
    /// The records under `root`, every event's.
    pub fn load(root: &Path) -> Store {
        let mut records = Vec::new();
        if let Ok(events) = std::fs::read_dir(root.join("records")) {
            for event in events.flatten() {
                for file in std::fs::read_dir(event.path()).into_iter().flatten().flatten() {
                    if file.path().extension().is_some_and(|e| e == "json") {
                        if let Some(r) = std::fs::read_to_string(file.path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| Record::from_json(&v)) {
                            records.push(r);
                        }
                    }
                }
            }
        }
        records.sort_by(|a, b| (match_order(&a.match_key), a.station).cmp(&(match_order(&b.match_key), b.station)));
        Store { root: root.to_path_buf(), records }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn file(&self, r: &Record) -> PathBuf {
        self.root.join("records").join(safe(&r.event)).join(format!("{}.json", safe(&r.id())))
    }

    pub fn get(&self, match_key: &str, team: u32) -> Option<&Record> {
        self.records.iter().find(|r| r.match_key == match_key && r.team == team)
    }

    /// Keep a record (a new one, or a new version of one here).
    pub fn put(&mut self, record: Record) -> std::io::Result<()> {
        let file = self.file(&record);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::sys::write_atomic(&file, serde_json::to_string(&record.to_json()).unwrap_or_default().as_bytes())?;
        match self.records.iter_mut().find(|r| r.match_key == record.match_key && r.team == record.team) {
            Some(here) => *here = record,
            None => {
                let at = self.records.partition_point(|r| (match_order(&r.match_key), r.station) <= (match_order(&record.match_key), record.station));
                self.records.insert(at, record);
            }
        }
        Ok(())
    }

    /// Merge records from elsewhere: how many were new, and how many newer.
    pub fn merge_in(&mut self, incoming: &[Record]) -> (usize, usize) {
        let (mut added, mut updated) = (0, 0);
        for r in incoming {
            let here = self.get(&r.match_key, r.team).cloned();
            let (what, merged) = merge(here.as_ref(), r);
            if here.as_ref() == Some(&merged) {
                continue;
            }
            if self.put(merged).is_ok() {
                match what {
                    Merged::Added => added += 1,
                    Merged::Updated => updated += 1,
                    Merged::Kept => {}
                }
            }
        }
        (added, updated)
    }

    /// An event's records, in match order.
    pub fn of_event<'a>(&'a self, event: &'a str) -> impl Iterator<Item = &'a Record> + 'a {
        self.records.iter().filter(move |r| r.event == event)
    }
}
