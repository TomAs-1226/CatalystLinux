//! The event and the match a photo was taken at, from the event schedule
//! the handheld already keeps (schedule.rs: The Blue Alliance's matches for
//! the team, Nexus's live times folded in).
//!
//! A photo is at the event when it was taken on one of the event's days, or
//! within twelve hours of one of the team's matches there (a schedule whose
//! dates and match times disagree, as the simulator's does, still counts).
//! Then it is tagged with the event's name and where it is held, and with
//! where it fell among the team's matches:
//!
//! - **during Q31**: from a minute before the match's time to five after;
//! - **before Q32**: the team's next match within an hour and a half;
//! - **after Q31**: else within an hour of the last one;
//! - nothing more otherwise (a quiet morning in the pit).

use crate::schedule::View;

/// What a photo is tagged with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tag {
    pub event: String,
    pub key: String,
    /// Where it is held: "Rochester, NY".
    pub venue: String,
    /// "during Q31", "before Q32", "after Q31", or empty.
    pub context: String,
}

const DURING_BEFORE: i64 = 60;
const DURING_AFTER: i64 = 5 * 60;
const BEFORE: i64 = 90 * 60;
const AFTER: i64 = 60 * 60;
const NEAR_A_MATCH: i64 = 12 * 3600;

/// The tag for a photo taken at `at` (Unix seconds), on `day` (its local
/// date, `YYYY-MM-DD`), as `view` knows the event.
pub fn tag(view: &View, at: i64, day: &str) -> Option<Tag> {
    let event = view.event.as_ref()?;
    let times: Vec<(i64, &str)> = {
        let mut t: Vec<(i64, &str)> = view.matches.iter().filter_map(|m| Some((m.time?, m.label.as_str()))).collect();
        t.sort_by_key(|(at, _)| *at);
        t
    };
    let on_its_days = !event.start.is_empty() && event.start.as_str() <= day && day <= event.end.as_str();
    let near_a_match = times.iter().any(|(t, _)| (at - t).abs() <= NEAR_A_MATCH);
    if !on_its_days && !near_a_match {
        return None;
    }
    Some(Tag { event: event.name.clone(), key: event.key.clone(), venue: event.city.clone(), context: context(&times, at) })
}

fn context(times: &[(i64, &str)], at: i64) -> String {
    if let Some((_, label)) = times.iter().find(|(t, _)| at >= t - DURING_BEFORE && at <= t + DURING_AFTER) {
        return format!("during {label}");
    }
    if let Some((t, label)) = times.iter().find(|(t, _)| *t > at) {
        if t - at <= BEFORE {
            return format!("before {label}");
        }
    }
    if let Some((t, label)) = times.iter().rev().find(|(t, _)| *t <= at) {
        if at - t <= AFTER {
            return format!("after {label}");
        }
    }
    String::new()
}
