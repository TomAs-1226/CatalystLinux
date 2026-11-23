//! The event's matches, every team's, for picking who to scout: from The
//! Blue Alliance (`/event/KEY/matches/simple`) with the key the Schedule
//! app asks with, kept on the card for the stands, where there is no
//! internet. Until it comes (or with no key), the team's own matches from
//! the Schedule app stand in: they have all six teams too.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::robot::Robot;
use crate::schedule::Match;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub key: String,
    pub name: String,
    pub matches: Vec<Match>,
    /// Every match of the event, not only the team's own.
    pub whole: bool,
}

impl Event {
    /// The next match to scout, as of `now`: the first not yet played.
    pub fn next(&self, now: i64) -> Option<usize> {
        self.matches.iter().position(|m| m.score.is_none() && m.time.is_none_or(|t| t + 300 > now))
    }
}

/// The event fetched for the whole field, and a count of its changes.
static FETCHED: Mutex<Option<Event>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static FETCHING: AtomicBool = AtomicBool::new(false);

/// Changes whenever a fetch brings the event in.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn cache() -> std::path::PathBuf {
    super::dir().join("event.json")
}

fn to_json(e: &Event) -> Value {
    let matches: Vec<Value> = e.matches.iter().map(|m| json!({"key": m.key, "label": m.label, "red": m.red, "blue": m.blue, "time": m.time, "score": m.score.map(|(r, b)| [r, b])})).collect();
    json!({"key": e.key, "name": e.name, "whole": e.whole, "matches": matches})
}

fn from_json(v: &Value) -> Option<Event> {
    let nums = |a: &Value| a.as_array().map(|a| a.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
    Some(Event {
        key: v["key"].as_str()?.to_owned(),
        name: v["name"].as_str().unwrap_or("").to_owned(),
        whole: v["whole"].as_bool().unwrap_or(false),
        matches: v["matches"]
            .as_array()?
            .iter()
            .filter_map(|m| {
                Some(Match {
                    key: m["key"].as_str()?.into(),
                    label: m["label"].as_str()?.into(),
                    red: nums(&m["red"]),
                    blue: nums(&m["blue"]),
                    time: m["time"].as_i64(),
                    score: m["score"].as_array().and_then(|s| Some((s.first()?.as_i64()? as i32, s.get(1)?.as_i64()? as i32))),
                    status: None,
                    queue_at: None,
                })
            })
            .collect(),
    })
}

/// The event to scout: the demo's for the demo robot; else the whole event
/// as fetched or kept, else the Schedule app's matches for the team; with
/// neither, the event kept from before.
pub fn current(robot: &Robot) -> Option<Event> {
    if robot.is_demo() {
        return Some(demo());
    }
    let kept = || FETCHED.lock().ok().and_then(|f| f.clone()).or_else(|| std::fs::read_to_string(cache()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| from_json(&v)));
    let view = robot.schedule.view();
    match view.as_ref().and_then(|v| v.event.clone()) {
        Some(e) => {
            if let Some(whole) = kept().filter(|k| k.key == e.key && k.whole) {
                return Some(whole);
            }
            fetch(&e.key, &e.name);
            Some(Event { key: e.key, name: e.name, matches: view.map(|v| v.matches).unwrap_or_default(), whole: false })
        }
        None => kept(),
    }
}

/// Ask The Blue Alliance for every match of the event, once at a time, in
/// the background; kept on the card when it comes.
fn fetch(key: &str, name: &str) {
    if FETCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let (key, name) = (key.to_owned(), name.to_owned());
    let spawned = std::thread::Builder::new()
        .name("scout-event".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Some(auth) = crate::keys::Keys::load().get("tba").map(str::to_owned) {
                let url = format!("https://www.thebluealliance.com/api/v3/event/{key}/matches/simple");
                if let Ok(v) = crate::schedule::get(&url, &[("X-TBA-Auth-Key", &auth)]) {
                    let matches = crate::schedule::parse_matches(&v);
                    if !matches.is_empty() {
                        let e = Event { key, name, matches, whole: true };
                        std::fs::create_dir_all(super::dir()).ok();
                        crate::sys::write_atomic(&cache(), to_json(&e).to_string().as_bytes()).ok();
                        if let Ok(mut f) = FETCHED.lock() {
                            *f = Some(e);
                        }
                        GENERATION.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            // Asked again only after a while (the next time Scout opens
            // five minutes on).
            std::thread::sleep(std::time::Duration::from_secs(300));
            FETCHING.store(false, Ordering::Relaxed);
        });
    if spawned.is_err() {
        FETCHING.store(false, Ordering::Relaxed);
    }
}

/// The demo robot's event: forty qualification matches around the demo
/// schedule's own (5805 in Q9, Q14, Q22, Q31 and Q40), about seven minutes
/// apart, Q27 next.
pub fn demo() -> Event {
    const TEAMS: [u32; 30] = [254, 1678, 118, 2056, 971, 340, 3015, 191, 1114, 4613, 610, 2168, 1241, 1325, 4039, 1507, 5254, 3044, 6328, 2337, 174, 1559, 1585, 3173, 4508, 2791, 3990, 5813, 6911, 1511];
    let t = super::now();
    let own: [(u32, [u32; 3], [u32; 3]); 5] = [
        (9, [5805, 254, 1678], [118, 2056, 971]),
        (14, [340, 3015, 191], [5805, 1114, 4613]),
        (22, [5805, 610, 2168], [1241, 1325, 4039]),
        (31, [1507, 5254, 5805], [3044, 6328, 2337]),
        (40, [174, 5805, 1559], [1585, 3173, 4508]),
    ];
    let matches = (1..=40u32)
        .map(|n| {
            let (red, blue) = match own.iter().find(|o| o.0 == n) {
                Some(o) => (o.1.to_vec(), o.2.to_vec()),
                None => {
                    // Six different teams, spread by a stride through the list.
                    let pick = |k: u32| TEAMS[((n * 7 + k * 11) % TEAMS.len() as u32) as usize];
                    ((0..3).map(pick).collect(), (3..6).map(pick).collect())
                }
            };
            let time = t + 1380 + (n as i64 - 31) * 413;
            let played = time + 300 <= t;
            let score = played.then(|| (60 + ((n * 37) % 70) as i32, 55 + ((n * 53) % 75) as i32));
            Match { key: format!("2026nyro_qm{n}"), label: format!("Q{n}"), red, blue, time: Some(time), score, status: None, queue_at: None }
        })
        .collect();
    Event { key: "2026nyro".into(), name: "Finger Lakes Regional".into(), matches, whole: true }
}
