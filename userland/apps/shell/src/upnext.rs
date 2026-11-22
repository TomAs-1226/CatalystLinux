//! What is coming up, as StandBy and the lock screen show it: the team's
//! next match (from the schedule, schedule.rs) and the next reminder.
//!
//! **The next reminder** comes from whatever registers itself here: the
//! Reminders app, once it lands, calls [`set_reminders`] once at start (in
//! `main`, beside the other start-up calls) with a function that answers
//! "what is next, as of now". Until something does, there is no reminder
//! slot at all: StandBy's "everything" face and the lock screen leave the
//! row out, and the lock screen's editor does not offer it.

use std::sync::RwLock;

use bezel::Color;

use crate::robot::Robot;

/// One reminder, as the slot shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reminder {
    /// What to do, in the user's words ("charge the spare batteries").
    pub title: String,
    /// When it is due, in Unix seconds, if it has a time.
    pub due: Option<i64>,
}

/// Answers "the next reminder, as of `now` (Unix seconds)".
pub type Source = fn(now: i64) -> Option<Reminder>;

static SOURCE: RwLock<Option<Source>> = RwLock::new(None);

/// Where the next reminder comes from: set once, by the Reminders app (on
/// the handheld nothing calls it until that app lands).
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn set_reminders(source: Source) {
    *SOURCE.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(source);
}

/// Whether anything gives reminders, so the slot is offered at all.
pub fn reminders_available() -> bool {
    SOURCE.read().map(|s| s.is_some()).unwrap_or(false)
}

/// The next reminder, if something gives them and one is coming.
pub fn next_reminder(now: i64) -> Option<Reminder> {
    let source = (*SOURCE.read().ok()?)?;
    source(now)
}

/// A reminder's line: "charge the spare batteries · in 25 min".
pub fn reminder_line(r: &Reminder, now: i64) -> String {
    match r.due {
        Some(at) => format!("{} · {}", r.title, crate::schedule::when(at, now)),
        None => r.title.clone(),
    }
}

/// The gallery's reminders: one, at a time that reads well.
#[cfg(not(target_arch = "aarch64"))]
pub fn demo_reminder(now: i64) -> Option<Reminder> {
    Some(Reminder { title: "charge the spares".into(), due: Some(now + 25 * 60) })
}

/// The team's next match, as one glance: its label, the alliance's colour,
/// when, and with and against whom.
#[derive(Clone, Debug, PartialEq)]
pub struct NextMatch {
    pub label: String,
    /// "red", "blue", or none when the team is not in it.
    pub side: Option<&'static str>,
    pub color: Color,
    /// "in 23 min", "sat 10:42".
    pub when: String,
    /// Where the queue stands, if it is known: ("now", "queuing soon") from
    /// FRC Nexus's status, or ("queue", "in 8 min").
    pub queue: Option<(&'static str, String)>,
    pub with: Vec<u32>,
    pub against: Vec<u32>,
    pub team: u32,
    /// The event's name.
    pub event: Option<String>,
    /// The matches after it: label, time, side.
    pub after: Vec<(String, String, Option<&'static str>)>,
}

pub fn side_color(side: Option<&str>) -> Color {
    match side {
        Some("red") => crate::schedule::RED,
        Some("blue") => crate::schedule::BLUE,
        _ => Color::hex(0x5d616a),
    }
}

/// The next match, when a team and its schedule are known; otherwise why not.
pub fn next_match(robot: &Robot, now: i64) -> Result<NextMatch, String> {
    next_match_in(robot.schedule.view().as_ref(), now)
}

/// The same, from a schedule already read (the widgets keep theirs).
pub fn next_match_in(view: Option<&crate::schedule::View>, now: i64) -> Result<NextMatch, String> {
    let view = view.ok_or("no team set")?;
    let Some(m) = view.next(now) else {
        return Err(view.problem.clone().filter(|p| !p.starts_with("saved")).unwrap_or_else(|| "no match coming".into()));
    };
    let side = m.alliance(view.team);
    let others = |teams: &[u32]| teams.iter().copied().filter(|t| *t != view.team).collect::<Vec<_>>();
    let (ours, theirs) = if side == Some("red") { (&m.red, &m.blue) } else { (&m.blue, &m.red) };
    let queue = match (&m.status, m.queue_at) {
        (Some(status), _) => Some(("now", status.to_lowercase())),
        (None, Some(q)) if q > now => Some(("queue", crate::schedule::when(q, now))),
        _ => None,
    };
    let at = view.matches.iter().position(|x| x.key == m.key).unwrap_or(0);
    let after = view.matches[at + 1..]
        .iter()
        .filter(|x| x.score.is_none())
        .take(3)
        .map(|x| (x.label.clone(), x.time.map(|t| crate::schedule::when(t, now)).unwrap_or_else(|| "–".into()), x.alliance(view.team)))
        .collect();
    Ok(NextMatch {
        label: m.label.clone(),
        side,
        color: side_color(side),
        when: m.time.map(|t| crate::schedule::when(t, now)).unwrap_or_else(|| "time not set".into()),
        queue,
        with: others(ours),
        against: theirs.clone(),
        team: view.team,
        event: view.event.as_ref().map(|e| e.name.clone()),
        after,
    })
}

/// Teams as a line: "1507 5254".
pub fn teams(list: &[u32]) -> String {
    list.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_schedule_gives_the_next_match_and_those_after() {
        let robot = Robot::demo();
        let now = chrono::Utc::now().timestamp();
        let m = next_match(&robot, now).expect("the demo robot's event");
        assert_eq!(m.label, "Q31");
        assert_eq!((m.side, m.team), (Some("red"), 5805));
        assert_eq!(m.with, [1507, 5254]);
        assert_eq!(m.against, [3044, 6328, 2337]);
        assert_eq!(m.queue, Some(("now", "queuing soon".to_owned())));
        assert_eq!(m.after.len(), 1);
        assert_eq!(m.after[0].0, "Q40");
        assert!(next_match(&Robot::none(), now).is_err());
    }

    #[test]
    fn a_reminder_reads_with_its_time() {
        let now = 1_800_000_000;
        let r = Reminder { title: "charge the spare batteries".into(), due: Some(now + 25 * 60) };
        assert_eq!(reminder_line(&r, now), "charge the spare batteries · in 25 min");
        assert_eq!(reminder_line(&Reminder { due: None, ..r }, now), "charge the spare batteries");
    }
}
