//! The handheld's own records of its photos, kept where only it reads them
//! (`/data/catalyst/photolog`, readable by root alone): each photo's Wi-Fi
//! fingerprint and when it was taken, where its caption and its town stand
//! (waiting, sent, done, given up), what its caption cost; the batches out
//! at Anthropic; what was spent each month; the named places; the choices.
//!
//! What is said about a photo is in its sidecar beside it (meta.rs); this is
//! what the work needs and what should not travel with a copy of the photo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::meta;
use super::places::{self, Ap, Places};

/// Settings › Photos.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// Caption photos with Claude: off until turned on.
    pub captions: bool,
    /// Find the town online (BeaconDB): off until turned on.
    pub town: bool,
    /// Note the Wi-Fi with each photo (named places): on, and kept here.
    pub wifi: bool,
    /// Captions stop for the month at this, US cents.
    pub cap_cents: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { captions: false, town: false, wifi: true, cap_cents: 300 }
    }
}

/// The monthly limits offered, US cents.
pub const CAPS: [u32; 7] = [100, 200, 300, 500, 1000, 2000, 5000];

/// Where one piece of work for a photo stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Job {
    /// Not wanted.
    #[default]
    None,
    /// To do, from `next` (Unix seconds); `tries` failed so far.
    Waiting { tries: u32, next: i64 },
    /// Sent in this batch, its answer awaited.
    Sent(String),
    Done,
    /// Given up, and why.
    Failed(String),
    /// Not done, and why (a pixel picture; declined; not known).
    Skipped(String),
}

impl Job {
    pub fn waiting(&self) -> bool {
        matches!(self, Job::Waiting { .. } | Job::Sent(_))
    }

    fn to_json(&self) -> Value {
        match self {
            Job::None => Value::Null,
            Job::Waiting { tries, next } => json!({"s": "waiting", "tries": tries, "next": next}),
            Job::Sent(batch) => json!({"s": "sent", "batch": batch}),
            Job::Done => json!({"s": "done"}),
            Job::Failed(why) => json!({"s": "failed", "why": why}),
            Job::Skipped(why) => json!({"s": "skipped", "why": why}),
        }
    }

    fn from_json(v: &Value) -> Job {
        let why = || v["why"].as_str().unwrap_or("").to_owned();
        match v["s"].as_str() {
            Some("waiting") => Job::Waiting { tries: v["tries"].as_u64().unwrap_or(0) as u32, next: v["next"].as_i64().unwrap_or(0) },
            Some("sent") => Job::Sent(v["batch"].as_str().unwrap_or("").to_owned()),
            Some("done") => Job::Done,
            Some("failed") => Job::Failed(why()),
            Some("skipped") => Job::Skipped(why()),
            _ => Job::None,
        }
    }
}

/// A position from the town lookup: latitude, longitude, how far it may be
/// out (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: f64,
}

/// One photo, as the handheld keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    /// When it was taken, Unix seconds.
    pub at: i64,
    pub fp: Vec<Ap>,
    pub caption: Job,
    pub town: Job,
    /// What its caption cost, millionths of a US dollar.
    pub cost: u64,
    pub spot: Option<Spot>,
}

/// A batch of captions out at Anthropic.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub id: String,
    pub sent: i64,
    pub next_poll: i64,
    pub polls: u32,
    /// (its `custom_id`, the photo's name)
    pub items: Vec<(String, String)>,
    /// The most it can cost, held against the month's limit until it is in.
    pub reserved: u64,
}

/// Everything kept, and where.
#[derive(Debug)]
pub struct Ledger {
    /// The handheld's own folder for it.
    pub dir: PathBuf,
    /// The album's.
    pub photos: PathBuf,
    pub prefs: Prefs,
    pub records: BTreeMap<String, Record>,
    pub batches: Vec<Batch>,
    /// Spent on captions each month ("2026-11"), millionths of a dollar.
    pub spend: BTreeMap<String, u64>,
    pub places: Places,
    /// Requests to Anthropic wait until then (it was busy, or unreachable),
    /// and how many times running that has been so.
    pub pause_until: i64,
    pub pauses: u32,
    /// The key was refused: captions wait for the keys to change (the keys
    /// file's time when it was refused).
    pub refused: Option<i64>,
}

impl Ledger {
    pub fn open(dir: &Path, photos: &Path) -> Ledger {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Null);
        let p = read("prefs.json");
        let d = Prefs::default();
        let prefs = Prefs {
            captions: p["captions"].as_bool().unwrap_or(d.captions),
            town: p["town"].as_bool().unwrap_or(d.town),
            wifi: p["wifi"].as_bool().unwrap_or(d.wifi),
            cap_cents: p["cap_cents"].as_u64().map_or(d.cap_cents, |c| c as u32),
        };
        let s = read("state.json");
        let records = s["photos"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(name, r)| {
                        let spot = r["spot"].as_array().and_then(|a| Some(Spot { lat: a.first()?.as_f64()?, lon: a.get(1)?.as_f64()?, accuracy: a.get(2)?.as_f64()? }));
                        (
                            name.clone(),
                            Record { at: r["at"].as_i64().unwrap_or(0), fp: places::fp_from(&r["fp"]), caption: Job::from_json(&r["caption"]), town: Job::from_json(&r["town"]), cost: r["cost"].as_u64().unwrap_or(0), spot },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let batches = s["batches"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| {
                        Some(Batch {
                            id: b["id"].as_str()?.to_owned(),
                            sent: b["sent"].as_i64().unwrap_or(0),
                            next_poll: b["next_poll"].as_i64().unwrap_or(0),
                            polls: b["polls"].as_u64().unwrap_or(0) as u32,
                            items: b["items"].as_array().map(|i| i.iter().filter_map(|x| Some((x[0].as_str()?.to_owned(), x[1].as_str()?.to_owned()))).collect()).unwrap_or_default(),
                            reserved: b["reserved"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let spend = s["spend"].as_object().map(|o| o.iter().filter_map(|(m, v)| Some((m.clone(), v.as_u64()?))).collect()).unwrap_or_default();
        Ledger {
            dir: dir.to_path_buf(),
            photos: photos.to_path_buf(),
            prefs,
            records,
            batches,
            spend,
            places: Places::from_json(&read("places.json")),
            pause_until: s["pause_until"].as_i64().unwrap_or(0),
            pauses: s["pauses"].as_u64().unwrap_or(0) as u32,
            refused: s["refused"].as_i64(),
        }
    }

    /// Everything written back, each file whole or not at all, readable by
    /// root alone.
    pub fn save(&self) {
        std::fs::create_dir_all(&self.dir).ok();
        let p = &self.prefs;
        let prefs = json!({"captions": p.captions, "town": p.town, "wifi": p.wifi, "cap_cents": p.cap_cents});
        let photos: serde_json::Map<String, Value> = self
            .records
            .iter()
            .map(|(name, r)| {
                let mut v = json!({"at": r.at, "fp": places::fp_json(&r.fp), "caption": r.caption.to_json(), "town": r.town.to_json(), "cost": r.cost});
                if let Some(s) = r.spot {
                    v["spot"] = json!([s.lat, s.lon, s.accuracy]);
                }
                (name.clone(), v)
            })
            .collect();
        let batches: Vec<Value> = self
            .batches
            .iter()
            .map(|b| json!({"id": b.id, "sent": b.sent, "next_poll": b.next_poll, "polls": b.polls, "items": b.items.iter().map(|(c, n)| json!([c, n])).collect::<Vec<_>>(), "reserved": b.reserved}))
            .collect();
        let state = json!({"photos": photos, "batches": batches, "spend": self.spend, "pause_until": self.pause_until, "pauses": self.pauses, "refused": self.refused});
        for (name, v) in [("prefs.json", prefs), ("state.json", state), ("places.json", self.places.to_json())] {
            crate::sys::write_private(&self.dir.join(name), v.to_string().as_bytes()).ok();
        }
        super::changed();
    }

    pub fn photo_path(&self, name: &str) -> PathBuf {
        self.photos.join(name)
    }

    /// What was spent on captions in `month`, with what is out in batches.
    pub fn spent(&self, month: &str) -> u64 {
        self.spend.get(month).copied().unwrap_or(0)
    }

    pub fn reserved(&self) -> u64 {
        self.batches.iter().map(|b| b.reserved).sum()
    }

    pub fn cap(&self) -> u64 {
        u64::from(self.prefs.cap_cents) * 10_000
    }

    /// Room left under the month's limit, with what is out reserved.
    pub fn room(&self, month: &str) -> u64 {
        self.cap().saturating_sub(self.spent(month) + self.reserved())
    }

    /// A photo just taken: its record, and its sidecar with the place, the
    /// event and the match. `tag` is what the schedule says.
    pub fn note_taken(&mut self, name: &str, at: i64, fp: Vec<Ap>, tag: Option<&super::event::Tag>) {
        let path = self.photo_path(name);
        let place = self.places.best(&fp).map(|p| p.name.clone()).unwrap_or_default();
        let mut about = meta::read(&path);
        about.place = place;
        if let Some(t) = tag {
            about.event = t.event.clone();
            about.venue = t.venue.clone();
            about.context = t.context.clone();
        }
        let caption = match super::captionable(name) {
            Err(why) => Job::Skipped(why.into()),
            Ok(()) if self.prefs.captions => Job::Waiting { tries: 0, next: at },
            Ok(()) => Job::None,
        };
        let shareable = fp.iter().filter(|a| a.share).count() >= 2;
        let town = if self.prefs.town && shareable && about.place.is_empty() && about.event.is_empty() { Job::Waiting { tries: 0, next: at } } else { Job::None };
        meta::write(&path, &about).ok();
        self.records.insert(name.to_owned(), Record { at, fp, caption, town, cost: 0, spot: None });
    }

    /// Every photo's named place made right again (after a place is named,
    /// renamed or deleted): the photos taken before it was named take it
    /// too. How many sidecars changed.
    pub fn apply_places(&mut self) -> usize {
        let mut changed = 0;
        for (name, r) in &self.records {
            if r.fp.is_empty() {
                continue;
            }
            let path = self.photos.join(name);
            if !path.exists() {
                continue;
            }
            let want = self.places.best(&r.fp).map(|p| p.name.clone()).unwrap_or_default();
            let mut about = meta::read(&path);
            if about.place != want {
                about.place = want;
                if meta::write(&path, &about).is_ok() {
                    changed += 1;
                }
            }
        }
        changed
    }

    /// Name the place a photo was taken at (a new name, or one already
    /// given: then this is another look at it). Every photo of the place
    /// takes the name. The place's id.
    pub fn name_place(&mut self, photo: &str, name: &str) -> Result<u32, String> {
        let fp = self.records.get(photo).map(|r| r.fp.clone()).unwrap_or_default();
        if fp.len() < 2 {
            return Err("no wi-fi was noted with this photo".into());
        }
        let id = self.places.name(name, &fp).ok_or("give the place a name")?;
        self.apply_places();
        Ok(id)
    }

    /// The photos of a place (by its fingerprints), for its page.
    pub fn photos_of(&self, id: u32) -> usize {
        self.records.values().filter(|r| !r.fp.is_empty() && self.places.best(&r.fp).is_some_and(|p| p.id == id)).count()
    }

    /// Town lookups for the photos that have none and could: when the
    /// lookup is turned on, for those taken before.
    pub fn queue_towns(&mut self, now: i64) -> usize {
        let mut n = 0;
        for (name, r) in self.records.iter_mut() {
            if r.town != Job::None || r.fp.iter().filter(|a| a.share).count() < 2 {
                continue;
            }
            let about = meta::read(&self.photos.join(name));
            if about.place.is_empty() && about.event.is_empty() && about.town.is_empty() {
                r.town = Job::Waiting { tries: 0, next: now };
                n += 1;
            }
        }
        n
    }

    /// A photo's records gone with it.
    pub fn forget(&mut self, name: &str) {
        self.records.remove(name);
    }

    /// Photos in the album the records do not match: one the camera named
    /// `_2` after its moment was noted takes that moment's record; an edit
    /// (`…_edit.jpg`) takes what its photo's sidecar says; others are
    /// simply known from now on. True if anything changed.
    pub fn reconcile(&mut self, names: &[(String, i64)]) -> bool {
        let mut changed = false;
        for (name, at) in names {
            if self.records.contains_key(name) {
                continue;
            }
            // A moment noted under a name no photo took, a few seconds off.
            let orphan = self.records.iter().find(|(n, r)| !self.photos.join(n).exists() && (r.at - at).abs() <= 5 && names.iter().all(|(m, _)| m != *n)).map(|(n, _)| n.clone());
            if let Some(old) = orphan {
                let r = self.records.remove(&old).unwrap_or_default();
                let about = meta::read(&self.photos.join(&old));
                meta::write(&self.photo_path(name), &about).ok();
                std::fs::remove_file(meta::sidecar(&self.photos.join(&old))).ok();
                self.records.insert(name.clone(), r);
                changed = true;
                continue;
            }
            if let Some(source) = edit_of(name) {
                let from = self.photos.join(&source);
                if from.exists() {
                    let mut about = meta::read(&from);
                    if !about.is_empty() {
                        if !about.caption.is_empty() && about.by.is_empty() {
                            about.by = "you".into();
                        }
                        meta::write(&self.photo_path(name), &about).ok();
                    }
                    let mut r = self.records.get(&source).cloned().unwrap_or_default();
                    r.caption = if about.caption.is_empty() { Job::None } else { Job::Done };
                    r.town = Job::None;
                    r.cost = 0;
                    self.records.insert(name.clone(), r);
                    changed = true;
                    continue;
                }
            }
            let caption = match super::captionable(name) {
                Err(why) => Job::Skipped(why.into()),
                Ok(()) => Job::None,
            };
            self.records.insert(name.clone(), Record { at: *at, caption, ..Record::default() });
            changed = true;
        }
        // Records whose photo has gone (deleted outside Photos), after a while.
        let gone: Vec<String> = self.records.keys().filter(|n| !names.iter().any(|(m, _)| m == *n) && !self.photos.join(n).exists()).cloned().collect();
        for n in gone {
            let old = self.records.get(&n).is_some_and(|r| r.at < chrono::Utc::now().timestamp() - 600);
            if old {
                self.records.remove(&n);
                changed = true;
            }
        }
        changed
    }
}

/// The photo an edit was made from: `IMG_x_edit.jpg`, `IMG_x_edit_2.jpg`
/// are `IMG_x.jpg`'s.
pub fn edit_of(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".jpg")?;
    let at = stem.rfind("_edit")?;
    let rest = &stem[at + 5..];
    (rest.is_empty() || (rest.starts_with('_') && rest[1..].chars().all(|c| c.is_ascii_digit()))).then(|| format!("{}.jpg", &stem[..at]))
}

/// "2026-11": the month a time falls in, here.
pub fn month_of(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m").to_string()).unwrap_or_default()
}
