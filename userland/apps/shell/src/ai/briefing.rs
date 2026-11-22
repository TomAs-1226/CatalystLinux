
//! Briefings: the day, or the next match, at a glance. "Brief me" asks for
//! one, and Assistant › choices can have one come by itself each morning or
//! before each match (a reminder that opens it, so Reminders' own timing,
//! wake-ups and schedule-following carry it; see [`schedule`]).
//!
//! A briefing is one card of short rows, the same however it is made:
//!
//! - **the next match:** when to queue, the alliance and partners (a pit
//!   briefing); or the day's matches and the first of them (a morning one);
//! - **the robot now:** linked or not, its battery, the worst health check
//!   firing (under the match before one; later on a morning);
//! - **the weather** where the handheld is, from the forecast kept;
//! - **the battery** and the plan for it: whether it lasts the day's
//!   matches, or the match to charge before;
//! - **reminders** due today;
//! - **the robot's last match:** what stood out in its log (the offline
//!   facts, `crate::offline`);
//! - **anything waiting:** a system update, songs to download.
//!
//! With OpenAI or Claude the `briefing` tool returns these as data and the
//! model says the one to three sentences that matter most. Offline (or with
//! no internet) a template says them, with no model: the match, then what
//! wants doing soon, three things at most. Either is said aloud when spoken
//! answers are on; the card is shown from its top.

use serde_json::{json, Value};

use crate::offline::Source;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The day ahead.
    Morning,
    /// The next match.
    Pit,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Morning => "morning",
            Kind::Pit => "pit",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        match name {
            "morning" => Some(Kind::Morning),
            "pit" => Some(Kind::Pit),
            _ => None,
        }
    }
}

/// A next match this close (or closer) makes "brief me" a pit briefing.
const PIT_WITHIN: i64 = 90 * 60;
/// When a team queues if the schedule does not say: this long before.
const QUEUE_LEAD: i64 = crate::reminders::plan::QUEUE_LEAD;

/// Whether `question` asks for a briefing, and which (`None`: whichever
/// fits the hour and the schedule).
pub fn asked(question: &str) -> Option<Option<Kind>> {
    let q = question.trim().to_lowercase();
    if q.contains("debrief") {
        return None;
    }
    let asks = q.contains("briefing") || ["brief me", "brief us", "brief", "give me a brief", "morning brief", "pit brief"].iter().any(|p| q.starts_with(p) || q.trim_end_matches(['.', '!', '?']) == *p);
    if !asks {
        return None;
    }
    Some(if q.contains("pit") || q.contains("match") || q.contains("queue") {
        Some(Kind::Pit)
    } else if q.contains("morning") || q.contains("day") {
        Some(Kind::Morning)
    } else {
        None
    })
}

/// One row of the card.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub icon: String,
    pub text: String,
    /// It wants doing soon: queue now, a battery to charge, a reminder due.
    pub urgent: bool,
}

/// A briefing: its card, what is said, and the data the services word.
#[derive(Clone, Debug, PartialEq)]
pub struct Brief {
    pub kind: Kind,
    /// "Pit briefing · Q31", "Morning briefing".
    pub title: String,
    /// When it was made: "10:19".
    pub when: String,
    pub rows: Vec<Row>,
    /// The template's words, offline.
    pub said: String,
    pub data: Value,
}

/// The team's next match, as a briefing says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Next {
    pub label: String,
    pub side: Option<&'static str>,
    pub partners: Vec<u32>,
    pub opponents: Vec<u32>,
    pub starts: Option<i64>,
    /// When to queue, and whether the schedule said so (FRC Nexus) or it is
    /// worked out (20 minutes before).
    pub queue: Option<(i64, bool)>,
    /// Live, from Nexus: "Queuing soon", "On deck".
    pub status: Option<String>,
}

/// What a briefing is made from, gathered on the screen's thread (the
/// schedule, the reminders, the robot), finished on another (the logs).
pub struct Inputs {
    pub wanted: Option<Kind>,
    pub now: i64,
    pub h24: bool,
    pub next: Option<Next>,
    /// Why there is no next match, when there is none.
    pub no_next: String,
    /// Matches still to play today: label and when.
    pub today: Vec<(String, i64)>,
    pub event: Option<String>,
    /// Today's reminders not done: words, when, and whether due.
    pub reminders: Vec<(String, String, bool)>,
    pub source: Source,
    pub robot: RobotNow,
}

/// The robot as it is now: linked or not, its battery, and the health
/// checks firing, worst first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RobotNow {
    pub linked: bool,
    pub volts: Option<f64>,
    /// Each check firing: its severity ("ERROR", "WARN") and what it says
    /// ("Hood: 78 °C, limit 70").
    pub alerts: Vec<(String, String)>,
}

impl RobotNow {
    pub fn of(robot: &crate::robot::Robot) -> RobotNow {
        let linked = robot.linked();
        let alerts = if linked {
            robot
                .firing()
                .into_iter()
                .map(|(severity, subsystem, check, detail)| {
                    let what = if detail.is_empty() { check.replace(['/', '_'], " ") } else { detail };
                    (severity, format!("{subsystem}: {what}"))
                })
                .collect()
        } else {
            Vec::new()
        };
        RobotNow { linked, volts: linked.then(|| robot.f64("Systemcore/BatteryVolts")).flatten(), alerts }
    }
}

impl Inputs {
    pub fn of(robot: &crate::robot::Robot, h24: bool, wanted: Option<Kind>) -> Inputs {
        let now = crate::reminders::now();
        let view = robot.schedule.view();
        let (mut next, mut no_next, mut today, mut event) = (None, String::new(), Vec::new(), None);
        match &view {
            Some(v) => {
                event = v.event.as_ref().map(|e| e.name.clone());
                match v.next(now) {
                    Some(m) => {
                        let side = m.alliance(v.team);
                        let (ours, theirs) = if side == Some("red") { (&m.red, &m.blue) } else { (&m.blue, &m.red) };
                        let queue = m.queue_at.map(|q| (q, true)).or_else(|| m.time.map(|t| (t - QUEUE_LEAD, false)));
                        next = Some(Next { label: m.label.clone(), side, partners: ours.iter().copied().filter(|t| *t != v.team).collect(), opponents: theirs.clone(), starts: m.time, queue, status: m.status.clone() });
                    }
                    None => no_next = v.problem.clone().filter(|p| !p.starts_with("saved")).unwrap_or_else(|| "no match coming".into()),
                }
                let day = local_date(now);
                today = v.matches.iter().filter(|m| m.score.is_none()).filter_map(|m| Some((m.label.clone(), m.time?))).filter(|(_, t)| *t + 600 > now && local_date(*t) == day).collect();
            }
            None => no_next = "no team set: the robot app sets it".into(),
        }
        let reminders = crate::reminders::with(|s| {
            let n = crate::reminders::plan::Now { at: now, zone: &crate::reminders::plan::Here, view: view.as_ref() };
            s.book.rows(crate::reminders::Page::Today, None, &n, h24).into_iter().filter(|r| !r.done).map(|r| (r.text, r.when, r.due)).collect()
        });
        Inputs { wanted, now, h24, next, no_next, today, event, reminders, source: Source::of(robot), robot: RobotNow::of(robot) }
    }
}

fn local_date(t: i64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&chrono::Local).date_naive()).unwrap_or_default()
}

/// "10:42", or "10:42 am" on a 12-hour clock.
fn clock(t: i64, h24: bool) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| crate::reminders::plan::clock_text(d.with_timezone(&chrono::Local).time(), h24)).unwrap_or_default()
}

/// "in 8 min", "now", or the clock.
fn soon(t: i64, now: i64, h24: bool) -> String {
    match t - now {
        d if d.abs() < 60 => "now".into(),
        d if d > 0 && d < 3600 => format!("in {} min", (d + 59) / 60),
        d if d < 0 && d > -3600 => format!("{} min ago", -d / 60),
        _ => format!("at {}", clock(t, h24)),
    }
}

fn teams(list: &[u32]) -> String {
    list.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
}

/// "1507 and 5254", for saying.
fn teams_said(list: &[u32]) -> String {
    match list {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", teams(rest)),
    }
}

/// The handheld's battery, now (the simulator's made-up one on a computer,
/// and in the tests).
fn battery() -> Option<crate::power::Reading> {
    crate::power::Reading::read().or_else(|| (cfg!(test) || !cfg!(target_arch = "aarch64")).then(crate::power::Reading::demo))
}

/// The battery's plan: whether it lasts the day's matches, or which to charge before.
pub fn charge_plan(r: &crate::power::Reading, today: &[(String, i64)], now: i64) -> (String, bool) {
    use crate::power::Flow;
    match r.flow {
        Flow::Charging => return (r.minutes_to_full.map_or("charging".into(), |m| format!("charging · full in {}", crate::sys::minutes_text(m))), false),
        Flow::Full | Flow::Holding => return ("on the charger".into(), false),
        _ => {}
    }
    if r.percent < 20 {
        return ("low · charge it now".into(), true);
    }
    let Some(left) = r.minutes_left else { return ("on battery".into(), false) };
    let empty = now + left as i64 * 60;
    let left_words = format!("{} left", crate::sys::minutes_text(left));
    match today.last() {
        None => (left_words, false),
        Some((last, at)) if empty > at + 15 * 60 => (format!("{left_words} · lasts past {last}"), false),
        Some(_) => {
            let before = today.iter().find(|(_, at)| *at + 10 * 60 > empty).map_or_else(|| today[0].0.clone(), |(l, _)| l.clone());
            (format!("{left_words} · charge before {before}"), true)
        }
    }
}

/// Things said of a briefing at most (a morning's greeting aside): the
/// match, then what wants doing soon, then the rest. The card shows it all.
const SAID_AT_MOST: usize = 3;

/// The briefing, from what was gathered: the logs read here (so on a thread).
pub fn build(i: &Inputs) -> Brief {
    let now = i.now;
    let kind = i.wanted.unwrap_or(match &i.next {
        Some(n) if n.starts.is_some_and(|t| t - now < PIT_WITHIN) => Kind::Pit,
        _ => Kind::Morning,
    });
    let hour: u32 = chrono::DateTime::from_timestamp(now, 0).map(|d| chrono::Timelike::hour(&d.with_timezone(&chrono::Local))).unwrap_or(9);
    let title = match (kind, &i.next) {
        (Kind::Pit, Some(n)) => format!("Pit briefing · {}", n.label),
        (Kind::Pit, None) => "Pit briefing".to_owned(),
        (Kind::Morning, _) if hour < 12 => "Morning briefing".to_owned(),
        (Kind::Morning, _) => "Briefing".to_owned(),
    };
    let mut rows = Vec::new();
    // What is said: the match first, then each other thing, and whether it
    // wants doing soon (those go before the rest).
    let mut first: Option<String> = None;
    let mut said: Vec<(bool, String)> = Vec::new();
    // The match.
    let mut data = json!({"kind": kind.name(), "time": clock(now, i.h24), "event": i.event});
    match &i.next {
        Some(n) => {
            let side = n.side.unwrap_or("not on an alliance");
            let queue = n.queue.map(|(q, known)| (soon(q, now, i.h24), known));
            let queue_now = n.queue.is_some_and(|(q, _)| q - now < 10 * 60) || n.status.as_deref().is_some_and(|s| s.to_lowercase().contains("now"));
            let with = if n.partners.is_empty() { String::new() } else { format!(" with {}", teams(&n.partners)) };
            let starts = n.starts.map(|t| soon(t, now, i.h24));
            let text = match (kind, &queue) {
                (Kind::Pit, Some((q, _))) => format!("{} on {side}{with} · queue {q}{}", n.label, starts.as_ref().map(|s| format!(" · starts {s}")).unwrap_or_default()),
                (Kind::Pit, None) => format!("{} on {side}{with}", n.label),
                (Kind::Morning, _) if i.today.len() > 1 => format!("{} matches today · first {} {} on {side}", i.today.len(), n.label, n.starts.map(|t| format!("at {}", clock(t, i.h24))).unwrap_or_default()),
                (Kind::Morning, _) => format!("{} on {side}{with} · {}", n.label, starts.clone().unwrap_or_else(|| "time not set".into())),
            };
            rows.push(Row { icon: "event".into(), text: tidy(&text), urgent: kind == Kind::Pit && queue_now });
            first = Some(match kind {
                Kind::Pit => {
                    let with = if n.partners.is_empty() { String::new() } else { format!(" with {}", teams_said(&n.partners)) };
                    let q = queue.as_ref().map(|(q, _)| format!(" Queue {q}.")).unwrap_or_default();
                    format!("{} is next, on {side}{with}.{q}", n.label)
                }
                Kind::Morning if i.today.len() > 1 => format!("{} matches today; the first is {}{} on {side}.", words_number(i.today.len()), n.label, n.starts.map(|t| format!(" at {}", clock(t, i.h24))).unwrap_or_default()),
                Kind::Morning => format!("Next is {}{} on {side}.", n.label, starts.map(|s| format!(" {s}")).unwrap_or_default()),
            });
            data["nextMatch"] = json!({
                "match": n.label, "alliance": n.side, "partners": n.partners, "opponents": n.opponents,
                "starts": n.starts.map(|t| clock(t, i.h24)), "startsIn": starts_in(n.starts, now),
                "queue": n.queue.map(|(q, _)| clock(q, i.h24)), "queueIn": starts_in(n.queue.map(|q| q.0), now),
                "queueTimeFrom": n.queue.map(|(_, known)| if known { "FRC Nexus" } else { "worked out: 20 min before the match" }),
                "status": n.status,
            });
        }
        None => {
            rows.push(Row { icon: "event".into(), text: tidy(&i.no_next), urgent: false });
            data["nextMatch"] = json!({"none": i.no_next});
        }
    }
    data["matchesToday"] = json!(i.today.iter().map(|(l, t)| json!({"match": l, "at": clock(*t, i.h24)})).collect::<Vec<_>>());
    // The robot as it is now: before a match, straight under it.
    let robot_row = robot_row(&i.robot);
    if kind == Kind::Pit {
        let (row, words) = robot_row.clone();
        rows.push(row);
        said.extend(words);
    }
    data["robot"] = json!({"linked": i.robot.linked, "batteryVolts": i.robot.volts, "alertsFiring": i.robot.alerts.iter().map(|(s, w)| json!({"severity": s, "what": w})).collect::<Vec<_>>()});
    // The weather (where it is worth a row: a morning, or rain on the way).
    if let Some(w) = crate::weather::glance(i.h24) {
        let mut text = format!("{} · {}", w.temp, w.condition.to_lowercase());
        if let Some(range) = &w.range {
            text.push_str(&format!(" · {range}"));
        }
        if let Some(rain) = w.rain {
            text.push_str(&format!(" · rain {rain}%"));
        }
        if kind == Kind::Morning || w.rain.is_some() {
            rows.push(Row { icon: w.icon.into(), text: tidy(&text), urgent: false });
            said.push((false, format!("{} and {}.", w.temp, w.condition.to_lowercase())));
        }
        data["weather"] = json!({"place": w.place, "now": w.temp, "condition": w.condition, "range": w.range, "rainChance": w.rain});
    }
    // The battery, and its plan.
    if let Some(b) = battery() {
        let (plan, urgent) = charge_plan(&b, &i.today, now);
        let charging = matches!(b.flow, crate::power::Flow::Charging);
        let icon = if charging { "battery_charging_full" } else if urgent || b.percent < 25 { "battery_alert" } else { "battery_full" };
        rows.push(Row { icon: icon.into(), text: tidy(&format!("{}% · {plan}", b.percent)), urgent });
        said.push((urgent, format!("Battery {}%, {}.", b.percent, plan.split(" · ").last().unwrap_or(&plan))));
        data["battery"] = json!({"percent": b.percent, "status": b.status(), "plan": plan});
    }
    // Today's reminders.
    let due = i.reminders.iter().filter(|r| r.2).count();
    let named: Vec<String> = i.reminders.iter().take(2).map(|(text, when, _)| if when.is_empty() { text.clone() } else { format!("{text} ({when})") }).collect();
    let text = match i.reminders.len() {
        0 => "no reminders today".to_owned(),
        n => {
            let more = if n > 2 { format!(" · +{} more", n - 2) } else { String::new() };
            format!("{} today · {}{more}", count(n, "reminder"), named.join(" · "))
        }
    };
    rows.push(Row { icon: "notifications".into(), text: tidy(&text), urgent: due > 0 });
    if !i.reminders.is_empty() {
        let names = i.reminders.iter().take(2).map(|r| r.0.clone()).collect::<Vec<_>>().join(", and ");
        said.push((due > 0, format!("{} today: {names}.", capital(&count(i.reminders.len(), "reminder")))));
    }
    data["remindersToday"] = json!(i.reminders.iter().map(|(t, w, d)| json!({"text": t, "when": w, "due": d})).collect::<Vec<_>>());
    // The robot, on a morning: after the day's own things.
    if kind == Kind::Morning {
        let (row, words) = robot_row;
        rows.push(row);
        said.extend(words);
    }
    // The robot's last match, from its log.
    let lib = crate::offline::load(&i.source, &mut |_| {});
    if !lib.matches.is_empty() {
        let label = lib.matches[0].label.clone();
        let lines = lib.wrong(0, 2);
        let fault = lib.matches[0].facts.iter().any(|f| f.level == crate::wpilog::Level::Fault);
        let worst = lines.first().map(|l| l.body.clone()).unwrap_or_else(|| "nothing stood out".into());
        rows.push(Row { icon: if fault { "warning".into() } else { "insights".into() }, text: tidy(&format!("{label} · {worst}")), urgent: fault });
        said.push((fault, format!("Last match, {label}: {}.", worst.trim_end_matches('.'))));
        data["lastMatch"] = json!({"match": label, "stoodOut": lines.iter().map(|l| l.body.clone()).collect::<Vec<_>>(), "outcome": lib.outcomes.get(&label)});
    }
    // Anything waiting.
    let mut waiting = Vec::new();
    let update = crate::sysupdate::read();
    match update.state.as_str() {
        "available" => waiting.push(format!("update {} to download", update.version)),
        "downloading" | "installing" => waiting.push(format!("update {}", crate::sysupdate::summary(&update))),
        "ready" => waiting.push(format!("update {} ready · restart in settings", update.version)),
        _ => {}
    }
    let (songs, failed) = crate::music::downloads_pending();
    if songs > 0 {
        waiting.push(format!("{} to download", count(songs, "song")));
    }
    if failed > 0 {
        waiting.push(format!("{} failed", count(failed, "download")));
    }
    if !waiting.is_empty() {
        rows.push(Row { icon: "download".into(), text: tidy(&waiting.join(" · ")), urgent: false });
        said.push((false, format!("Waiting: {}.", waiting.join(", "))));
    }
    data["waiting"] = json!(waiting);
    // Said: the match, what wants doing soon, then the rest, a few at most.
    said.sort_by_key(|(urgent, _)| !urgent);
    let greeting = (kind == Kind::Morning && hour < 12).then(|| "Good morning.".to_owned());
    let things: Vec<String> = first.into_iter().chain(said.into_iter().map(|(_, s)| s)).take(SAID_AT_MOST).collect();
    let said = greeting.into_iter().chain(things).collect::<Vec<_>>().join(" ");
    data["summary"] = json!(said);
    data["shown"] = json!("the handheld shows all of this as a card above your answer");
    Brief { kind, title, when: clock(now, i.h24), rows, said, data }
}

/// The robot's row: linked or not, its battery, and the worst check firing;
/// and what is said of it, if anything wants saying (an alert, or no link).
fn robot_row(r: &RobotNow) -> (Row, Option<(bool, String)>) {
    if !r.linked {
        return (Row { icon: "link_off".into(), text: "robot not linked".into(), urgent: false }, Some((false, "The robot is not linked.".into())));
    }
    let volts = r.volts.map(|v| format!(" {v:.1} V")).unwrap_or_default();
    let error = r.alerts.iter().any(|(s, _)| s == "ERROR");
    let (text, said) = match r.alerts.as_slice() {
        [] => (format!("robot linked{} · no alerts", volts.replacen(' ', " · ", 1)), None),
        [(_, what)] => (format!("robot{volts} · {what}"), Some((error, format!("The robot has an alert: {}.", what.replace(": ", ", "))))),
        [(_, what), rest @ ..] => (format!("robot{volts} · {what} · +{} more", rest.len()), Some((error, format!("The robot has {} alerts; the worst is {}.", rest.len() + 1, what.replace(": ", ", "))))),
    };
    let icon = if r.alerts.is_empty() { "smart_toy" } else { "warning" };
    (Row { icon: icon.into(), text: tidy(&text), urgent: error }, said)
}

fn starts_in(t: Option<i64>, now: i64) -> Option<String> {
    t.map(|t| match (t - now) / 60 {
        m if m < 0 => format!("{} min ago", -m),
        m => format!("{m} min"),
    })
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

fn words_number(n: usize) -> String {
    ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"].get(n).map_or_else(|| n.to_string(), |w| (*w).to_owned())
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A row's words: a number kept with its unit, the first letter as it is.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------- schedule ---

/// The morning briefing's times to choose from, and the pit briefing's lead.
pub const MORNINGS: [&str; 5] = ["06:30", "07:00", "07:30", "08:00", "08:30"];
pub const PIT_LEAD_MIN: i64 = 30;

/// The briefing reminders kept: the morning one's time, and whether there
/// is one before each match.
pub fn schedule() -> (Option<String>, bool) {
    crate::reminders::with(|s| {
        let mine: Vec<&crate::reminders::Reminder> = s.book.reminders.iter().filter(|r| r.link == crate::reminders::Link::Briefing && r.done.is_none()).collect();
        let morning = mine.iter().find_map(|r| if let crate::reminders::When::At(at) = &r.when { Some(at.format("%H:%M").to_string()) } else { None });
        let pit = mine.iter().any(|r| matches!(r.when, crate::reminders::When::Match(..)));
        (morning, pit)
    })
}

/// The morning briefing at `at` ("07:30") each day, or none.
pub fn set_morning(at: Option<&str>, robot: &crate::robot::Robot) {
    use crate::reminders::{List, Repeat, When};
    let when = at.and_then(|t| chrono::NaiveTime::parse_from_str(t, "%H:%M").ok()).map(|t| {
        // Today's, if it is still to come; else tomorrow's.
        let now = chrono::DateTime::from_timestamp(crate::reminders::now(), 0).map(|d| d.with_timezone(&chrono::Local).naive_local()).unwrap_or_default();
        let today = now.date().and_time(t);
        When::At(if today > now { today } else { today + chrono::Duration::days(1) })
    });
    replace(|w| matches!(w, When::At(_)), when.map(|w| ("Morning briefing", w, Repeat::Daily, List::Personal)), robot);
}

/// A pit briefing before each match (`PIT_LEAD_MIN` before it, so about
/// ten minutes before queueing), or none.
pub fn set_pit(on: bool, robot: &crate::robot::Robot) {
    use crate::reminders::{Cue, List, Of, Repeat, When};
    replace(|w| matches!(w, When::Match(..)), on.then(|| ("Pit briefing", When::Match(Cue::Before(PIT_LEAD_MIN * 60), Of::Each), Repeat::Never, List::Pit)), robot);
}

/// The briefing reminders of one kind taken away, and the new one (if any)
/// added in their place.
fn replace(kind: impl Fn(&crate::reminders::When) -> bool, new: Option<(&str, crate::reminders::When, crate::reminders::Repeat, crate::reminders::List)>, robot: &crate::robot::Robot) {
    use crate::reminders::{parse::Parsed, Link};
    let view = robot.schedule.view();
    crate::reminders::with(|s| {
        let old: Vec<u64> = s.book.reminders.iter().filter(|r| r.link == Link::Briefing && kind(&r.when)).map(|r| r.id).collect();
        for id in old {
            s.book.delete(id);
        }
        if let Some((text, when, repeat, list)) = new {
            let n = crate::reminders::plan::Now { at: crate::reminders::now(), zone: &crate::reminders::plan::Here, view: view.as_ref() };
            s.book.add(Parsed { text: text.into(), when, repeat, list: Some(list), link: Link::Briefing }, &n);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::{Flow, Reading};

    #[test]
    fn brief_me_and_its_kinds_are_read_from_the_question() {
        assert_eq!(asked("brief me"), Some(None));
        assert_eq!(asked("Brief me!"), Some(None));
        assert_eq!(asked("give me the morning briefing"), Some(Some(Kind::Morning)));
        assert_eq!(asked("pit briefing please"), Some(Some(Kind::Pit)));
        assert_eq!(asked("brief me on the next match"), Some(Some(Kind::Pit)));
        assert_eq!(asked("debrief Q14"), None);
        assert_eq!(asked("why did we brown out?"), None);
        assert_eq!(asked("keep it brief: how is the robot?"), None);
    }

    #[test]
    fn the_robot_row_says_its_link_its_battery_and_its_worst_alert() {
        let r = |linked, volts, alerts: &[(&str, &str)]| robot_row(&RobotNow { linked, volts, alerts: alerts.iter().map(|(s, w)| (s.to_string(), w.to_string())).collect() });
        assert_eq!(r(false, None, &[]).0.text, "robot not linked");
        let (row, said) = r(true, Some(12.43), &[]);
        assert_eq!((row.text.as_str(), row.urgent, said), ("robot linked · 12.4 V · no alerts", false, None));
        assert_eq!(r(true, None, &[]).0.text, "robot linked · no alerts");
        let (row, said) = r(true, Some(11.9), &[("ERROR", "Drive: left front stalled"), ("WARN", "CAN: 71% for 4 s")]);
        assert_eq!(row.text, "robot 11.9 V · Drive: left front stalled · +1 more");
        assert!(row.urgent, "an error wants seeing to");
        assert_eq!(said, Some((true, "The robot has 2 alerts; the worst is Drive, left front stalled.".into())));
    }

    #[test]
    fn the_battery_plan_says_whether_it_lasts_the_matches_left() {
        let now = 1_000_000;
        let today = vec![("Q31".to_owned(), now + 1380), ("Q40".to_owned(), now + 5100), ("Q52".to_owned(), now + 4 * 3600)];
        let r = |percent, left: Option<u64>| Reading { percent, flow: Flow::Discharging, minutes_left: left, ..Reading::demo() };
        assert_eq!(charge_plan(&r(76, Some(278)), &today, now), ("4 h 38 m left · lasts past Q52".into(), false));
        assert_eq!(charge_plan(&r(40, Some(120)), &today, now), ("2 h 0 m left · charge before Q52".into(), true));
        assert_eq!(charge_plan(&r(15, Some(40)), &today, now), ("low · charge it now".into(), true));
        assert_eq!(charge_plan(&r(60, Some(90)), &[], now), ("1 h 30 m left".into(), false));
        let charging = Reading { flow: Flow::Charging, minutes_to_full: Some(40), ..Reading::demo() };
        assert_eq!(charge_plan(&charging, &today, now).0, "charging · full in 40 m");
    }

    #[test]
    fn a_briefing_is_made_from_the_schedule_the_reminders_the_battery_and_the_logs() {
        crate::shell::tests::sandbox();
        let robot = crate::robot::Robot::demo();
        crate::reminders::remind("swap the battery at queueing for Q31");
        let b = build(&Inputs::of(&robot, true, None));
        // The demo event: Q31 next in 23 minutes, queueing in 8, on red.
        assert_eq!(b.kind, Kind::Pit, "a match this soon: the pit's");
        assert_eq!(b.title, "Pit briefing · Q31");
        assert!(b.rows[0].text.starts_with("Q31 on red with 1507 5254 · queue in 8 min · starts in 23 min"), "{:?}", b.rows[0]);
        assert!(b.rows[0].urgent, "queueing in under ten minutes");
        assert!(b.rows.iter().any(|r| r.icon.starts_with("battery") && r.text.starts_with("76% · 4 h 38 m left")), "{:?}", b.rows);
        // The reminder is for Q31's queueing, 8 minutes on: today's, unless
        // that is past midnight (then tomorrow's, and not in today's list).
        let now = crate::reminders::now();
        if local_date(now + 8 * 60) == local_date(now) {
            assert!(b.rows.iter().any(|r| r.icon == "notifications" && r.text.contains("swap the battery")), "{:?}", b.rows);
        } else {
            assert!(b.rows.iter().any(|r| r.icon == "notifications" && r.text == "no reminders today"), "{:?}", b.rows);
        }
        assert!(b.rows.iter().any(|r| r.text.starts_with("Q14 · the robot browned out")), "the newest log's worst: {:?}", b.rows);
        assert!(b.said.starts_with("Q31 is next, on red with 1507 and 5254. Queue in 8 min."), "{}", b.said);
        assert_eq!(b.data["nextMatch"]["partners"], json!([1507, 5254]));
        assert_eq!(b.data["nextMatch"]["queueTimeFrom"], "FRC Nexus");
        assert!(b.data["lastMatch"]["stoodOut"][0].as_str().unwrap().contains("browned out"));
        // Asked for the morning's: the day, and the weather.
        let inputs = Inputs::of(&robot, true, Some(Kind::Morning));
        let m = build(&inputs);
        if inputs.today.len() > 1 {
            assert!(m.rows[0].text.starts_with(&format!("{} matches today · first Q31 at ", inputs.today.len())), "{:?}", m.rows[0]);
        }
        // (The simulator's forecast: the handheld's build has only what it fetched.)
        if cfg!(not(target_arch = "aarch64")) {
            assert!(m.data["weather"]["now"].is_string());
            assert!(m.rows.iter().any(|r| r.text.contains('°')), "the weather has a row in the morning: {:?}", m.rows);
        }
        // Kept as reminders that open a briefing: each morning, and before each match.
        set_morning(Some("07:30"), &robot);
        set_pit(true, &robot);
        assert_eq!(schedule(), (Some("07:30".into()), true));
        set_morning(Some("08:00"), &robot);
        assert_eq!(schedule(), (Some("08:00".into()), true), "the old time replaced");
        let kept: Vec<(String, crate::reminders::Link)> = crate::reminders::with(|s| s.book.reminders.iter().filter(|r| r.text.ends_with("briefing")).map(|r| (r.text.clone(), r.link.clone())).collect());
        assert_eq!(kept, [("Pit briefing".to_owned(), crate::reminders::Link::Briefing), ("Morning briefing".to_owned(), crate::reminders::Link::Briefing)]);
        set_morning(None, &robot);
        set_pit(false, &robot);
        assert_eq!(schedule(), (None, false));
    }
}
