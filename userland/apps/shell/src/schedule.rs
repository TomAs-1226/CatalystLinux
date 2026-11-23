//! The event schedule: the team's matches at the event under way (or the
//! next one), from The Blue Alliance, with live queueing from FRC Nexus when
//! its key is set. Fetched in the background with `curl` over HTTPS, kept on
//! the data partition so it still shows (and says how old it is) when the
//! handheld is offline, as it is in the stands.
//!
//! The Blue Alliance API v3: `X-TBA-Auth-Key`, `/team/frcN/events/YEAR/simple`
//! and `/team/frcN/event/KEY/matches/simple`; times in Unix seconds.
//! FRC Nexus API v1: `Nexus-Api-Key`, `/event/KEY`; times in Unix milliseconds.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bezel::{Align, Color, Rect};
use serde_json::Value;

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const TBA: &str = "https://www.thebluealliance.com/api/v3";
const NEXUS: &str = "https://frc.nexus/api/v1";
/// How often each is asked: the schedule changes slowly, the queue quickly.
const TBA_EVERY: u64 = 300;
const NEXUS_EVERY: u64 = 30;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub key: String,
    pub name: String,
    /// `YYYY-MM-DD`.
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Match {
    pub key: String,
    /// "Q14", "SF3", "F1".
    pub label: String,
    pub red: Vec<u32>,
    pub blue: Vec<u32>,
    /// When it is played or expected to be, Unix seconds.
    pub time: Option<i64>,
    /// The score, once played: (red, blue).
    pub score: Option<(i32, i32)>,
    /// Live, from Nexus: "Now queuing", "On deck", "On field"…
    pub status: Option<String>,
    /// From Nexus: when to queue, Unix seconds.
    pub queue_at: Option<i64>,
}

impl Match {
    pub fn alliance(&self, team: u32) -> Option<&'static str> {
        if self.red.contains(&team) {
            Some("red")
        } else if self.blue.contains(&team) {
            Some("blue")
        } else {
            None
        }
    }

    /// "W 87–60", "L", "T": how it went for `team`.
    pub fn result(&self, team: u32) -> Option<String> {
        let (red, blue) = self.score?;
        if red < 0 || blue < 0 {
            return None;
        }
        let (ours, theirs) = if self.alliance(team)? == "red" { (red, blue) } else { (blue, red) };
        let mark = match ours.cmp(&theirs) {
            std::cmp::Ordering::Greater => "W",
            std::cmp::Ordering::Less => "L",
            std::cmp::Ordering::Equal => "T",
        };
        Some(format!("{mark} {ours}–{theirs}"))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub team: u32,
    pub event: Option<Event>,
    pub matches: Vec<Match>,
    /// From Nexus: the match being queued now.
    pub now_queuing: Option<String>,
    /// When this was fetched, Unix seconds.
    pub fetched: i64,
    /// Why it is not fresher: no key, offline…
    pub problem: Option<String>,
}

impl View {
    /// The team's next match not yet played, as of `now`.
    pub fn next(&self, now: i64) -> Option<&Match> {
        self.matches.iter().filter(|m| m.score.is_none()).find(|m| m.time.is_none_or(|t| t + 600 > now))
    }
}

/// "Q14" from a match's level and numbers.
fn label(level: &str, set: i64, number: i64) -> String {
    match level {
        "qm" => format!("Q{number}"),
        "f" => format!("F{number}"),
        // Double elimination: each playoff match is its own set.
        "sf" if number == 1 => format!("M{set}"),
        other => format!("{}{set}-{number}", other.to_uppercase()),
    }
}

fn teams(keys: &Value) -> Vec<u32> {
    keys.as_array().map(|a| a.iter().filter_map(|k| k.as_str()?.trim_start_matches("frc").parse().ok()).collect()).unwrap_or_default()
}

/// The event to show from a team's events this year: the one under way,
/// else the next to start, else the last one played. Dates are `YYYY-MM-DD`,
/// which compare as text.
pub fn pick_event(events: &Value, today: &str) -> Option<Event> {
    let mut all: Vec<Event> = events
        .as_array()?
        .iter()
        .filter_map(|e| Some(Event { key: e["key"].as_str()?.into(), name: e["name"].as_str()?.into(), start: e["start_date"].as_str()?.into(), end: e["end_date"].as_str()?.into() }))
        .collect();
    all.sort_by(|a, b| a.start.cmp(&b.start));
    all.iter().find(|e| e.start.as_str() <= today && today <= e.end.as_str()).or_else(|| all.iter().find(|e| e.start.as_str() > today)).or_else(|| all.last()).cloned()
}

/// A team's matches at an event, in the order they are played.
pub fn parse_matches(matches: &Value) -> Vec<Match> {
    let order = |level: &str| match level {
        "qm" => 0,
        "ef" => 1,
        "qf" => 2,
        "sf" => 3,
        _ => 4,
    };
    let mut out: Vec<(i32, i64, i64, Match)> = matches
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let level = m["comp_level"].as_str()?;
                    let (set, number) = (m["set_number"].as_i64().unwrap_or(1), m["match_number"].as_i64().unwrap_or(0));
                    let a = &m["alliances"];
                    let (rs, bs) = (a["red"]["score"].as_i64(), a["blue"]["score"].as_i64());
                    let played = m["actual_time"].as_i64().is_some() || rs.is_some_and(|s| s >= 0);
                    Some((
                        order(level),
                        set,
                        number,
                        Match {
                            key: m["key"].as_str()?.into(),
                            label: label(level, set, number),
                            red: teams(&a["red"]["team_keys"]),
                            blue: teams(&a["blue"]["team_keys"]),
                            time: m["actual_time"].as_i64().or(m["predicted_time"].as_i64()).or(m["time"].as_i64()),
                            score: played.then(|| (rs.unwrap_or(-1) as i32, bs.unwrap_or(-1) as i32)),
                            status: None,
                            queue_at: None,
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    out.into_iter().map(|(.., m)| m).collect()
}

/// Nexus names matches "Qualification 14", "Final 1"…: the TBA label they are.
fn nexus_label(label: &str) -> Option<String> {
    let (kind, number) = label.rsplit_once(' ')?;
    let number: u32 = number.parse().ok()?;
    Some(match kind {
        "Qualification" => format!("Q{number}"),
        "Playoff" | "Match" => format!("M{number}"),
        "Final" => format!("F{number}"),
        _ => return None,
    })
}

/// Fold Nexus's live view of the event into the schedule.
pub fn apply_nexus(view: &mut View, nexus: &Value) {
    view.now_queuing = nexus["nowQueuing"].as_str().map(|l| nexus_label(l).unwrap_or_else(|| l.to_owned()));
    for m in nexus["matches"].as_array().into_iter().flatten() {
        let Some(label) = m["label"].as_str().and_then(nexus_label) else { continue };
        if let Some(ours) = view.matches.iter_mut().find(|x| x.label == label) {
            ours.status = m["status"].as_str().map(str::to_owned);
            ours.queue_at = m["times"]["estimatedQueueTime"].as_i64().map(|ms| ms / 1000);
            if let Some(start) = m["times"]["estimatedStartTime"].as_i64() {
                ours.time = Some(start / 1000);
            }
        }
    }
}

/// The team a robot address belongs to: a team number as given, or the
/// TE.AM of a `10.TE.AM.x` address.
pub fn team_of(address: &str) -> Option<u32> {
    let address = address.trim();
    if let Ok(team) = address.parse::<u32>() {
        return (team > 0 && team < 100_000).then_some(team);
    }
    let parts: Vec<u32> = address.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [10, te, am, _] if am < 100 => Some(te * 100 + am),
        _ => None,
    }
}

// ------------------------------------------------------------- service ---

#[derive(Default)]
struct Shared {
    view: Mutex<Option<View>>,
    generation: AtomicU64,
    stop: AtomicBool,
    /// The wait between looks, which a stop ends.
    nap: crate::deep::Nap,
}

pub struct Schedule {
    shared: Arc<Shared>,
}

impl Drop for Schedule {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.nap.stop();
    }
}

fn now() -> i64 {
    // The reminders' clock: the same as the system's, but a test can move it.
    crate::reminders::now()
}

fn cache_file() -> std::path::PathBuf {
    crate::config_dir().join("schedule.json")
}

impl Schedule {
    pub fn none() -> Self {
        Schedule { shared: Arc::new(Shared::default()) }
    }

    /// A made-up event for the demo robot: some matches played, the next
    /// one being queued.
    pub fn demo() -> Self {
        let t = now();
        let m = |label: &str, red: [u32; 3], blue: [u32; 3], at: i64, score: Option<(i32, i32)>, status: Option<&str>| Match {
            key: format!("2026nyro_{}", label.to_lowercase()),
            label: label.into(),
            red: red.to_vec(),
            blue: blue.to_vec(),
            time: Some(t + at),
            score,
            status: status.map(str::to_owned),
            queue_at: status.map(|_| t + at - 900),
        };
        let view = View {
            team: 5805,
            event: Some(Event { key: "2026nyro".into(), name: "Finger Lakes Regional".into(), start: "2026-03-12".into(), end: "2026-03-15".into() }),
            matches: vec![
                m("Q9", [5805, 254, 1678], [118, 2056, 971], -9400, Some((112, 87)), None),
                m("Q14", [340, 3015, 191], [5805, 1114, 4613], -5200, Some((95, 101)), None),
                m("Q22", [5805, 610, 2168], [1241, 1325, 4039], -1300, Some((78, 78)), None),
                m("Q31", [1507, 5254, 5805], [3044, 6328, 2337], 1380, None, Some("Queuing soon")),
                m("Q40", [174, 5805, 1559], [1585, 3173, 4508], 5100, None, None),
            ],
            now_queuing: Some("Q27".into()),
            fetched: t - 40,
            problem: None,
        };
        let shared = Arc::new(Shared::default());
        *shared.view.lock().unwrap() = Some(view);
        Schedule { shared }
    }

    /// The schedule of the team at `address`, kept fresh in the background
    /// while there is a key to ask with.
    pub fn for_robot(address: &str) -> Self {
        let Some(team) = team_of(address) else { return Self::none() };
        let shared = Arc::new(Shared::default());
        // What was fetched before, for straight away and for offline.
        if let Some(view) = std::fs::read_to_string(cache_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| from_cache(&v)) {
            if view.team == team {
                *shared.view.lock().unwrap() = Some(view);
            }
        }
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("schedule".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                // Its looks every ten seconds folded into other wakeups (sched.rs).
                crate::sched::enter(crate::sched::Class::Poller);
                run(&thread_shared, team)
            })
            .ok();
        Schedule { shared }
    }

    pub fn view(&self) -> Option<View> {
        self.shared.view.lock().unwrap().clone()
    }

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }
}

fn run(shared: &Shared, team: u32) {
    let (mut tba_at, mut nexus_at) = (0i64, 0i64);
    while !shared.stop.load(Ordering::Relaxed) {
        let keys = crate::keys::Keys::load();
        let t = now();
        let publish = |view: View| {
            *shared.view.lock().unwrap() = Some(view);
            shared.generation.fetch_add(1, Ordering::Relaxed);
        };
        match keys.get("tba") {
            None => {
                let mut view = shared.view.lock().unwrap().clone().unwrap_or(View { team, ..View::default() });
                if view.problem.as_deref() != Some("no Blue Alliance key: Settings › Keys") {
                    view.problem = Some("no Blue Alliance key: Settings › Keys".into());
                    publish(view);
                }
            }
            Some(key) if t - tba_at >= TBA_EVERY as i64 => {
                tba_at = t;
                let mut view = match fetch_tba(team, key) {
                    Ok(view) => view,
                    Err(why) => {
                        let mut old = shared.view.lock().unwrap().clone().unwrap_or(View { team, ..View::default() });
                        old.problem = Some(why);
                        old
                    }
                };
                // Nexus's live view folded in straight away.
                if let (Some(event), Some(nexus_key)) = (view.event.clone(), keys.get("nexus")) {
                    if let Ok(nexus) = get(&format!("{NEXUS}/event/{}", event.key), &[("Nexus-Api-Key", nexus_key)]) {
                        apply_nexus(&mut view, &nexus);
                        nexus_at = t;
                    }
                }
                if view.problem.is_none() {
                    crate::sys::write_atomic(&cache_file(), to_cache(&view).to_string().as_bytes()).ok();
                }
                publish(view);
            }
            Some(_) => {
                if let (Some(nexus_key), Some(mut view)) = (keys.get("nexus"), shared.view.lock().unwrap().clone()) {
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    let live = view.event.as_ref().is_some_and(|e| e.start <= today && today <= e.end);
                    if live && t - nexus_at >= NEXUS_EVERY as i64 {
                        nexus_at = t;
                        if let Ok(nexus) = get(&format!("{NEXUS}/event/{}", view.event.as_ref().map_or("", |e| e.key.as_str())), &[("Nexus-Api-Key", nexus_key)]) {
                            apply_nexus(&mut view, &nexus);
                            publish(view);
                        }
                    }
                }
            }
        }
        // Ten seconds in one wait (a stop ends it; in deep standby it parks
        // after): one wakeup, where steps of 200 ms were fifty.
        if shared.nap.nap(Duration::from_secs(10)) {
            return;
        }
    }
}

fn fetch_tba(team: u32, key: &str) -> Result<View, String> {
    let auth = [("X-TBA-Auth-Key", key)];
    let year = chrono::Local::now().format("%Y").to_string();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let events = get(&format!("{TBA}/team/frc{team}/events/{year}/simple"), &auth)?;
    let Some(event) = pick_event(&events, &today) else {
        return Ok(View { team, fetched: now(), problem: Some(format!("no events for {team} in {year}")), ..View::default() });
    };
    let matches = get(&format!("{TBA}/team/frc{team}/event/{}/matches/simple", event.key), &auth)?;
    Ok(View { team, event: Some(event), matches: parse_matches(&matches), now_queuing: None, fetched: now(), problem: None })
}

/// One HTTPS GET with `curl`, as JSON. curl checks the server's certificate.
pub(crate) fn get(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut command = std::process::Command::new("curl");
    command.args(["-fsS", "--max-time", "15", "--proto", "=https"]);
    for (name, value) in headers {
        command.args(["-H", &format!("{name}: {value}")]);
    }
    let out = command.arg(url).output().map_err(|_| "no curl on this system".to_string())?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(if why.contains("401") || why.contains("403") {
            "the key was refused".into()
        } else if why.contains("resolve") || why.contains("connect") {
            "offline".into()
        } else {
            "could not fetch the schedule".into()
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "the reply was not readable".to_string())
}

fn to_cache(view: &View) -> Value {
    let matches: Vec<Value> = view
        .matches
        .iter()
        .map(|m| serde_json::json!({"key": m.key, "label": m.label, "red": m.red, "blue": m.blue, "time": m.time, "score": m.score.map(|(r, b)| [r, b])}))
        .collect();
    serde_json::json!({
        "team": view.team,
        "event": view.event.as_ref().map(|e| serde_json::json!({"key": e.key, "name": e.name, "start": e.start, "end": e.end})),
        "matches": matches,
        "fetched": view.fetched,
    })
}

fn from_cache(v: &Value) -> Option<View> {
    let nums = |a: &Value| a.as_array().map(|a| a.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
    let e = &v["event"];
    Some(View {
        team: v["team"].as_u64()? as u32,
        event: e["key"].as_str().map(|key| Event { key: key.into(), name: e["name"].as_str().unwrap_or("").into(), start: e["start"].as_str().unwrap_or("").into(), end: e["end"].as_str().unwrap_or("").into() }),
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
        now_queuing: None,
        fetched: v["fetched"].as_i64().unwrap_or(0),
        problem: Some("saved earlier".into()),
    })
}

// -------------------------------------------------------------- screens ---

/// The alliances' own colours, quieter on the panel than on a field display.
pub const RED: Color = Color::hex(0xe5484d);
pub const BLUE: Color = Color::hex(0x3e8ed0);

/// "in 23 min", "12 min ago", "10:42" for a time in Unix seconds.
pub fn when(at: i64, now: i64) -> String {
    let d = at - now;
    match d {
        d if d.abs() < 60 => "now".into(),
        d if d > 0 && d < 3600 => format!("in {} min", d / 60),
        d if d < 0 && d > -3600 => format!("{} min ago", -d / 60),
        _ => chrono::DateTime::from_timestamp(at, 0).map(|t| t.with_timezone(&chrono::Local).format("%a %H:%M").to_string().to_lowercase()).unwrap_or_default(),
    }
}

/// How old what is shown is, when it is not fresh: "saved 3 h ago".
fn age(view: &View, now: i64) -> Option<String> {
    let old = now - view.fetched;
    (old > 600 && view.fetched > 0).then(|| if old > 7200 { format!("saved {} h ago", old / 3600) } else { format!("saved {} min ago", old / 60) })
}

#[derive(Default)]
pub struct ScheduleScreen {
    list: ListState,
    /// The focus has been put on the next match, once, when the list first
    /// showed; after that it is wherever the keys put it.
    placed: bool,
}

impl ScheduleScreen {
    /// The schedule open on one match (by its place in the list), as search
    /// finds a match or a team.
    pub fn at(index: usize) -> Self {
        let mut list = ListState::default();
        list.focus = index;
        ScheduleScreen { list, placed: true }
    }
}

impl Screen for ScheduleScreen {
    fn title(&self) -> &str {
        "Schedule"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let view = cx.robot.schedule.view();
        let count = view.as_ref().map_or(0, |v| v.matches.len());
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                if let Some(m) = view.as_ref().and_then(|v| v.matches.get(self.list.focus)) {
                    return Nav::Push(Box::new(MatchScreen { team: view.as_ref().map_or(0, |v| v.team), m: m.clone() }));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, count),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let now = now();
        let Some(view) = robot.schedule.view() else {
            g.empty("event", "set the team first: Settings › Robot");
            return;
        };
        if view.matches.is_empty() {
            g.empty("event", view.problem.as_deref().unwrap_or("no matches yet"));
            return;
        }
        // The event, and what is on the field's doorstep.
        let top = g.top();
        let event = view.event.as_ref().map_or("", |e| e.name.as_str());
        let head = match &view.now_queuing {
            Some(q) => format!("{event} · queuing {q}"),
            None => event.to_owned(),
        };
        g.text_fit(PAD + 2.0, top + ty.caption.size * 0.5, &head, ty.caption, t.dim, W - 2.0 * PAD);
        let list_top = top + ty.caption.size + 6.0;
        let bottom = g.above_hint();
        let row_h = g.row_h();
        // Start at the next match, the first time the list is shown.
        if !self.placed {
            self.placed = true;
            if let Some(next) = view.next(now) {
                self.list.focus = view.matches.iter().position(|m| m.key == next.key).unwrap_or(0);
            }
        }
        let next_key = view.next(now).map(|m| m.key.clone());
        for (i, y) in self.list.layout(g, view.matches.len(), list_top, row_h, 3.0, bottom) {
            let m = &view.matches[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, row_h * 0.5, if focused { t.surface2 } else { t.surface1 });
            let side = m.alliance(view.team);
            g.dot(r.x + 12.0, r.cy(), 3.5, match side { Some("red") => RED, Some("blue") => BLUE, _ => t.faint });
            let name_w = g.text_at(r.x + 22.0, r.cy(), Align::Left, &m.label, ty.name, t.ink);
            let (right, color) = match (m.result(view.team), &m.status, m.time) {
                (Some(result), ..) => (result.clone(), if result.starts_with('W') { t.ink } else { t.dim }),
                (None, Some(status), _) => (status.to_lowercase(), t.signal),
                (None, None, Some(at)) => (when(at, now), if Some(&m.key) == next_key.as_ref() { t.ink } else { t.dim }),
                _ => ("–".into(), t.faint),
            };
            g.text_at(r.right() - 10.0, r.cy(), Align::Right, &right, ty.label, color);
            if let Some(q) = m.queue_at.filter(|_| m.score.is_none()) {
                g.text_at(r.x + 30.0 + name_w, r.cy(), Align::Left, &format!("queue {}", when(q, now)), ty.label, t.dim);
            }
            if focused {
                g.focus_ring(r, row_h * 0.5);
            }
        }
        self.list.scrollbar(g, view.matches.len(), list_top, bottom);
        let footer = match (age(&view, now), &view.problem) {
            (Some(age), _) => age,
            (None, Some(problem)) if problem != "saved earlier" => problem.clone(),
            _ => format!("team {} · enter shows the alliances", view.team),
        };
        g.hint(&footer);
    }

    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
}

/// One match: both alliances, the time, how it went.
pub struct MatchScreen {
    team: u32,
    m: Match,
}

impl Screen for MatchScreen {
    fn title(&self) -> &str {
        &self.m.label
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let w = (W - 2.0 * PAD - 8.0) * 0.5;
        for (i, (name, teams, color)) in [("red", &self.m.red, RED), ("blue", &self.m.blue, BLUE)].into_iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (w + 8.0), top, w, g.above_hint() - top);
            g.surface(r, 14.0, t.surface1);
            g.canvas.rrect(Rect::new(r.x, r.y, r.w, 3.0), 1.5, color, 1.0);
            let score = self.m.score.map(|(red, blue)| if i == 0 { red } else { blue }).filter(|s| *s >= 0);
            g.text_at(r.x + 10.0, r.y + 14.0, Align::Left, name, ty.caption, t.dim);
            if let Some(score) = score {
                g.text_at(r.right() - 10.0, r.y + 14.0, Align::Right, &score.to_string(), ty.name, t.ink);
            }
            for (k, team) in teams.iter().enumerate() {
                let ours = *team == self.team;
                g.text_at(r.x + 10.0, r.y + 36.0 + k as f32 * (ty.name.size + 7.0), Align::Left, &team.to_string(), if ours { ty.title } else { ty.name }, if ours { t.ink } else { t.dim });
            }
        }
        let now = now();
        let line = match (self.m.result(self.team), self.m.time) {
            (Some(result), _) => result,
            (None, Some(at)) => format!("{}{}", when(at, now), self.m.queue_at.map(|q| format!(" · queue {}", when(q, now))).unwrap_or_default()),
            _ => "time not set yet".into(),
        };
        g.hint(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_event_under_way_is_picked_then_the_next() {
        let events = json!([
            {"key": "2026nyro", "name": "Finger Lakes", "start_date": "2026-03-12", "end_date": "2026-03-15"},
            {"key": "2026nyny", "name": "New York City", "start_date": "2026-04-02", "end_date": "2026-04-05"},
            {"key": "2026week0", "name": "Week Zero", "start_date": "2026-02-14", "end_date": "2026-02-14"}
        ]);
        assert_eq!(pick_event(&events, "2026-03-13").unwrap().key, "2026nyro");
        assert_eq!(pick_event(&events, "2026-03-20").unwrap().key, "2026nyny");
        assert_eq!(pick_event(&events, "2026-05-01").unwrap().key, "2026nyny", "after the season: the last one");
    }

    #[test]
    fn matches_read_in_play_order_with_results() {
        let matches = json!([
            {"key": "2026nyro_f1m1", "comp_level": "f", "set_number": 1, "match_number": 1,
             "alliances": {"red": {"score": -1, "team_keys": ["frc5805", "frc254", "frc1678"]}, "blue": {"score": -1, "team_keys": ["frc118", "frc2056", "frc971"]}},
             "time": 1773500000, "predicted_time": null, "actual_time": null},
            {"key": "2026nyro_qm14", "comp_level": "qm", "set_number": 1, "match_number": 14,
             "alliances": {"red": {"score": 95, "team_keys": ["frc340", "frc3015", "frc191"]}, "blue": {"score": 101, "team_keys": ["frc5805", "frc1114", "frc4613"]}},
             "time": 1773300000, "predicted_time": 1773300100, "actual_time": 1773300200},
            {"key": "2026nyro_sf3m1", "comp_level": "sf", "set_number": 3, "match_number": 1,
             "alliances": {"red": {"score": -1, "team_keys": []}, "blue": {"score": -1, "team_keys": []}}, "time": 1773400000}
        ]);
        let m = parse_matches(&matches);
        assert_eq!(m.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["Q14", "M3", "F1"]);
        assert_eq!(m[0].alliance(5805), Some("blue"));
        assert_eq!(m[0].result(5805).as_deref(), Some("W 101–95"));
        assert_eq!(m[0].time, Some(1773300200), "the actual time once played");
        assert_eq!(m[2].score, None, "not played: no score");
        let view = View { team: 5805, matches: m, ..View::default() };
        assert_eq!(view.next(1773350000).map(|m| m.label.as_str()), Some("M3"));
    }

    #[test]
    fn nexus_folds_its_queue_into_the_schedule() {
        let mut view = View { team: 5805, matches: vec![Match { label: "Q31".into(), ..Match::default() }], ..View::default() };
        let nexus = json!({"eventKey": "2026nyro", "nowQueuing": "Qualification 27", "matches": [
            {"label": "Qualification 31", "status": "Queuing soon", "redTeams": ["5805"], "blueTeams": [],
             "times": {"estimatedQueueTime": 1773400000000i64, "estimatedStartTime": 1773400900000i64}}
        ]});
        apply_nexus(&mut view, &nexus);
        assert_eq!(view.now_queuing.as_deref(), Some("Q27"));
        assert_eq!(view.matches[0].status.as_deref(), Some("Queuing soon"));
        assert_eq!((view.matches[0].queue_at, view.matches[0].time), (Some(1773400000), Some(1773400900)));
    }

    #[test]
    fn a_team_is_known_from_its_robots_address() {
        assert_eq!(team_of("5805"), Some(5805));
        assert_eq!(team_of("10.58.5.2"), Some(5805));
        assert_eq!(team_of("10.2.54.2"), Some(254));
        assert_eq!(team_of("robot.local"), None);
        assert_eq!(team_of("192.168.1.9"), None);
    }

    #[test]
    fn what_was_fetched_is_kept_and_read_back() {
        let view = Schedule::demo().view().unwrap();
        let back = from_cache(&to_cache(&view)).unwrap();
        assert_eq!(back.matches.len(), view.matches.len());
        assert_eq!(back.matches[1].score, view.matches[1].score);
        assert_eq!(back.event, view.event);
    }
}
