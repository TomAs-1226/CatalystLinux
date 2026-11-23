//! Scout: match scouting for FRC team 5805, made for the handheld's keyboard.
//!
//! A scout is given a place on the field (red 1 to blue 3) and scouts the
//! robot there, match after match: the next match comes first. The form
//! (form.rs) is the season's game as data; during a match its keys count
//! as fast as the robot scores, the match clock turning the form from AUTO
//! to TELEOP to END GAME as the field does. Records (store.rs) are kept in
//! `/data/scout/`, one a team a match, with who changed what. They leave
//! the handheld as CSV for Catalyst Companion, or as QR codes another
//! handheld's camera reads (share.rs). Teams' pages, a picklist (stats.rs)
//! and a cycle timer for drive practice (practice.rs) are built on them.

pub mod event;
pub mod form;
pub mod practice;
pub mod share;
pub mod stats;
pub mod store;

mod home;
mod scouting;
mod sharing;
mod teams;

#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use home::Scout;

use form::Form;
use store::{Station, Store};

/// Where scouting is kept: `/data/scout` on the handheld.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CATALYST_SCOUT") {
        return dir.into();
    }
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/scout")
    } else {
        crate::config_dir().join("scout")
    }
}

/// Unix seconds (the reminders' clock, which a test can move).
pub fn now() -> i64 {
    crate::reminders::now()
}

// ---------------------------------------------------------------- the scout ---

/// Who scouts on this handheld, and where on the field they watch.
#[derive(Clone, Debug, PartialEq)]
pub struct Me {
    pub name: String,
    pub station: Station,
}

impl Me {
    pub fn load() -> Me {
        let v = std::fs::read_to_string(dir().join("me.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let host = std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_owned()).filter(|h| !h.is_empty()).unwrap_or_else(|| "catalyst".into());
        Me {
            name: v.as_ref().and_then(|v| v["name"].as_str()).filter(|n| !n.is_empty()).map_or(host, str::to_owned),
            station: v.as_ref().and_then(|v| v["station"].as_str()).and_then(Station::parse).unwrap_or_default(),
        }
    }

    pub fn save(&self) {
        let d = dir();
        std::fs::create_dir_all(&d).ok();
        crate::sys::write_atomic(&d.join("me.json"), serde_json::json!({"name": self.name, "station": self.station.short()}).to_string().as_bytes()).ok();
    }
}

// ------------------------------------------------------------ form, records ---

/// The form in use (`/data/scout/form.json`, else the one shipped), and
/// why one put there could not be used.
static FORM: Mutex<Option<(PathBuf, Arc<Form>, Option<String>)>> = Mutex::new(None);

pub fn form() -> Arc<Form> {
    form_and_problem().0
}

pub fn form_and_problem() -> (Arc<Form>, Option<String>) {
    let root = dir();
    let mut held = FORM.lock().unwrap_or_else(|e| e.into_inner());
    if held.as_ref().is_none_or(|(r, ..)| *r != root) {
        let (form, problem) = match std::fs::read_to_string(root.join("form.json")) {
            Err(_) => (Form::shipped(), None),
            Ok(text) => match Form::parse(&text) {
                Ok(f) => (f, None),
                Err(why) => (Form::shipped(), Some(format!("form.json: {why}"))),
            },
        };
        *held = Some((root, Arc::new(form), problem));
    }
    let (_, form, problem) = held.as_ref().expect("just loaded");
    (form.clone(), problem.clone())
}

/// Every record, read once from the card and kept.
static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// Work with the records.
pub fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let root = dir();
    let mut held = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if held.as_ref().is_none_or(|s| s.root() != root) {
        *held = Some(Store::load(&root));
    }
    f(held.as_mut().expect("just loaded"))
}

/// Read the records from the card again (another program wrote them).
#[cfg(not(target_arch = "aarch64"))]
pub fn reload() {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

// ------------------------------------------------------------- the camera ---

/// The codes being read with the camera.
static INBOX: Mutex<share::Inbox> = Mutex::new(share::Inbox { set: String::new(), total: 0, have: Vec::new(), added: 0, updated: 0, problem: None });

/// A code the camera read: when it is one of Scout's, its records merge in
/// (each code once). What to say of it, for the camera's card.
pub fn scanned(text: &str) -> Option<(String, &'static str)> {
    if !share::is_code(text) {
        return None;
    }
    let form = form();
    let mut inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
    match share::parse(&form, text) {
        Ok(chunk) => {
            with_store(|store| inbox.take(chunk, store));
        }
        Err(why) => inbox.problem = Some(why),
    }
    Some(scan_words_of(&inbox))
}

/// What the camera's card says of a Scout code: how far the set is read,
/// and what came in.
pub fn scan_words(text: &str) -> Option<(String, &'static str)> {
    share::is_code(text).then(|| scan_words_of(&INBOX.lock().unwrap_or_else(|e| e.into_inner())))
}

fn scan_words_of(inbox: &share::Inbox) -> (String, &'static str) {
    if let Some(why) = &inbox.problem {
        return (format!("Scout · {why}"), "this code cannot be read here");
    }
    let done = inbox.total > 0 && inbox.have.len() >= inbox.total;
    (format!("Scout · {} · {}", inbox.progress(), inbox.outcome()), if done { "all in · esc: back" } else { "hold each code in view" })
}

/// What the inbox says, for Scout's own page: progress and outcome.
pub fn inbox_words() -> Option<(String, String)> {
    let inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
    (inbox.total > 0).then(|| (inbox.progress(), inbox.outcome()))
}

// ------------------------------------------------------------------- team ---

/// A code's text sent over Team (team/) rather than shown: its records
/// merged in as the camera's are, each change noted as from Team. How many
/// came in new, and how many were newer; none if it would not read here.
pub fn from_team(text: &str, by: &str) -> Option<(usize, usize)> {
    let chunk = share::parse(&form(), text).ok()?;
    let records: Vec<store::Record> = chunk
        .records
        .into_iter()
        .map(|mut r| {
            r.history.push(store::Version { at: r.updated, by: r.by.clone(), change: format!("sent over Team by {by}") });
            r
        })
        .collect();
    Some(with_store(|s| s.merge_in(&records)))
}
