//! When a reminder comes due: wall-clock times through the time zone (and
//! its daylight saving), match cues through the event schedule.
//!
//! - A time typed as a clock time ("8am", "tomorrow 8:00") is kept as that
//!   wall-clock time, and found again through the zone each time it is
//!   asked: it stays at 8:00 when the zone or daylight saving changes. In
//!   the hour the clocks skip, a time is read with the offset before the
//!   change (RFC 5545), so 2:30 is 3:30; in the hour they repeat, the
//!   first of the two.
//! - A time typed as a wait ("in 10 min") is an instant.
//! - A match cue is worked out from the schedule each time, never kept: when
//!   the event runs late and the matches move, so do the reminders on them.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use super::{Cue, Of, Reminder, Repeat, When};
use crate::schedule::{Match, View};

/// "before Q42" with no time given: twenty minutes before it, in time to queue.
pub const LEAD: i64 = 20 * 60;
/// Without FRC Nexus's queue times, a team queues about this long before its match.
pub const QUEUE_LEAD: i64 = 20 * 60;
/// This long after a match starts, it is over and the robot is back in the pit.
pub const AFTER: i64 = 6 * 60;
/// An occurrence of a repeating reminder found later than this (the
/// handheld was off) passes unsaid: the next one is what matters now.
pub const STALE: i64 = 30 * 60;
/// What a day with no time given means: the morning.
pub const MORNING: u32 = 9;

/// How wall-clock times turn into instants and back.
pub trait Zone {
    /// Seconds east of UTC at an instant.
    fn offset(&self, at: i64) -> i64;

    /// The wall-clock time at an instant.
    fn local(&self, at: i64) -> NaiveDateTime {
        DateTime::from_timestamp(at + self.offset(at), 0).map(|t| t.naive_utc()).unwrap_or_default()
    }

    /// The instant a wall-clock time falls at.
    fn instant(&self, local: NaiveDateTime) -> i64 {
        let naive = local.and_utc().timestamp();
        // The offsets a day either side: a zone changes at most once in that time.
        let (before, after) = (self.offset(naive - 86_400), self.offset(naive + 86_400));
        let fits = |offset: i64| self.offset(naive - offset) == offset;
        match (fits(before), fits(after)) {
            // The clocks go back: the first of the two.
            (true, true) => (naive - before).min(naive - after),
            (true, false) => naive - before,
            (false, true) => naive - after,
            // The clocks skip it: read with the offset before the change.
            (false, false) => naive - before,
        }
    }
}

/// The handheld's own zone, as set in Settings (the TZ the shell follows).
pub struct Here;

impl Zone for Here {
    fn offset(&self, at: i64) -> i64 {
        use chrono::TimeZone;
        chrono::Local.timestamp_opt(at, 0).single().map_or(0, |t| t.offset().local_minus_utc() as i64)
    }
}

/// What the reminders are worked out against: the time, the zone, and the
/// event schedule when there is one.
pub struct Now<'a> {
    pub at: i64,
    pub zone: &'a dyn Zone,
    pub view: Option<&'a View>,
}

impl Now<'_> {
    pub fn today(&self) -> NaiveDate {
        self.zone.local(self.at).date()
    }

    /// The first instant of tomorrow, here.
    pub fn tomorrow_starts(&self) -> i64 {
        self.zone.instant(self.today().succ_opt().unwrap_or(self.today()).and_time(NaiveTime::MIN))
    }

    /// An event is on today: what is typed belongs on the pit's list.
    pub fn event_today(&self) -> bool {
        let today = self.today().format("%Y-%m-%d").to_string();
        self.view.and_then(|v| v.event.as_ref()).is_some_and(|e| e.start <= today && today <= e.end)
    }
}

/// One occurrence of a reminder: when, and which (so none is said twice).
#[derive(Clone, Debug, PartialEq)]
pub struct Occ {
    pub at: i64,
    pub key: String,
    /// The match it is tied to.
    pub label: Option<String>,
    /// Worked out without Nexus's queue time.
    pub estimated: bool,
}

impl Occ {
    fn instant(at: i64) -> Occ {
        Occ { at, key: format!("t{at}"), label: None, estimated: false }
    }

    fn local(zone: &dyn Zone, local: NaiveDateTime) -> Occ {
        Occ { at: zone.instant(local), key: format!("l{}", local.format(super::LOCAL)), label: None, estimated: false }
    }
}

/// When a team queues for a match: Nexus's time, else an estimate.
fn queue_time(m: &Match) -> Option<(i64, bool)> {
    m.queue_at.map(|q| (q, false)).or_else(|| m.time.map(|t| (t - QUEUE_LEAD, true)))
}

/// When a cue falls for a match (and the one after it, for Between).
pub fn cue_at(cue: Cue, m: &Match, next: Option<&Match>) -> Option<(i64, bool)> {
    match cue {
        Cue::Before(lead) => m.time.map(|t| (t - lead, false)),
        Cue::Queue => queue_time(m),
        Cue::After => m.time.map(|t| (t + AFTER, false)),
        // Halfway between the robot back in the pit and queueing for the next.
        Cue::Between => {
            let back = m.time? + AFTER;
            let (queue, estimated) = queue_time(next?)?;
            Some((back + ((queue - back) / 2).max(0), estimated))
        }
    }
}

/// Every occurrence of a cue over the team's matches, in the order they are played.
pub fn cues(view: &View, cue: Cue) -> Vec<Occ> {
    view.matches
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let (at, estimated) = cue_at(cue, m, view.matches.get(i + 1))?;
            Some(Occ { at, key: format!("m:{}", m.key), label: Some(m.label.clone()), estimated })
        })
        .collect()
}

/// The next day a repeat comes round on, after `day`.
fn next_day(day: NaiveDate, repeat: Repeat) -> Option<NaiveDate> {
    match repeat {
        Repeat::Daily => day.succ_opt(),
        Repeat::Weekly => day.checked_add_signed(Duration::days(7)),
        Repeat::Weekdays => {
            let mut d = day.succ_opt()?;
            while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                d = d.succ_opt()?;
            }
            Some(d)
        }
        Repeat::Never | Repeat::Every(_) => None,
    }
}

impl Reminder {
    /// It comes round again: a repeat, or every match.
    pub fn repeats(&self) -> bool {
        self.repeat != Repeat::Never || matches!(self.when, When::Match(_, Of::Each))
    }

    /// The occurrence it is on: none for a to-do with no time, or one that
    /// waits on the schedule.
    pub fn current(&self, n: &Now) -> Option<Occ> {
        match &self.when {
            When::Whenever => None,
            When::In(at) => Some(Occ::instant(*at)),
            When::At(local) => Some(Occ::local(n.zone, *local)),
            When::Match(cue, of) => {
                let all = cues(n.view?, *cue);
                match of {
                    // Tied to that match: at another event, it is not on the schedule.
                    Of::One { key: Some(key), .. } => {
                        let key = format!("m:{key}");
                        all.into_iter().find(|o| o.key == key)
                    }
                    Of::One { label, key: None } => all.into_iter().find(|o| o.label.as_deref() == Some(label.as_str())),
                    // Not tied to a match yet (no schedule when it was made): the next to come.
                    Of::Next => all.into_iter().find(|o| o.at > n.at),
                    // The first match not yet past, of those after it was made.
                    Of::Each => all.into_iter().find(|o| !self.past.contains(&o.key) && o.at >= self.created),
                }
            }
        }
    }

    /// The occurrence after the current one, for one that repeats.
    pub fn following(&self, n: &Now) -> Option<Occ> {
        match (&self.when, self.repeat) {
            (When::At(local), Repeat::Every(every)) => Some(Occ::instant(n.zone.instant(*local) + every)),
            (When::At(local), repeat) => next_day(local.date(), repeat).map(|d| Occ::local(n.zone, d.and_time(local.time()))),
            (When::In(at), Repeat::Every(every)) => Some(Occ::instant(at + every)),
            (When::Match(cue, Of::Each), _) => {
                let current = self.current(n)?;
                cues(n.view?, *cue).into_iter().skip_while(|o| o.key != current.key).skip(1).find(|o| !self.past.contains(&o.key))
            }
            _ => None,
        }
    }

    /// On to the next occurrence: this one is done with (ticked, or passed).
    pub fn advance(&mut self, n: &Now) {
        match (&self.when, self.repeat) {
            (When::At(local), Repeat::Every(every)) => self.when = When::In(n.zone.instant(*local) + every),
            (When::At(local), repeat) => {
                if let Some(d) = next_day(local.date(), repeat) {
                    self.when = When::At(d.and_time(local.time()));
                }
            }
            (When::In(at), Repeat::Every(every)) => self.when = When::In(at + every),
            (When::Match(_, Of::Each), _) => {
                if let Some(current) = self.current(n) {
                    self.past.push(current.key);
                    // Only what can still matter is kept: an event's matches.
                    if self.past.len() > 200 {
                        self.past.remove(0);
                    }
                }
            }
            _ => {}
        }
    }

    /// A repeating one left behind (the handheld off a while): on to the
    /// latest occurrence that has come, without passing through each.
    pub fn catch_up(&mut self, n: &Now) {
        if let (When::In(at), Repeat::Every(every)) = (&self.when, self.repeat) {
            if every > 0 && n.at - at > every {
                self.when = When::In(at + (n.at - at) / every * every);
            }
            return;
        }
        if let (When::At(local), Repeat::Daily | Repeat::Weekly | Repeat::Weekdays) = (&self.when, self.repeat) {
            // Most of the way in one step: to a week or so before today.
            let today = n.today();
            let behind = (today - local.date()).num_days();
            if behind > 14 {
                let step = if self.repeat == Repeat::Daily { behind - 7 } else { (behind - 7) / 7 * 7 };
                // Whole weeks but for every day, so a weekday stays one.
                if let Some(d) = local.date().checked_add_signed(Duration::days(step)) {
                    self.when = When::At(d.and_time(local.time()));
                }
            }
        }
        for _ in 0..400 {
            match self.following(n) {
                Some(next) if next.at <= n.at => self.advance(n),
                _ => break,
            }
        }
    }

    /// When it next needs saying: its time, or (once said) the time the
    /// next occurrence comes round.
    pub fn next_due(&self, n: &Now) -> Option<i64> {
        if self.done.is_some() {
            return None;
        }
        if let Some(until) = self.snooze {
            return Some(until);
        }
        let current = self.current(n)?;
        if self.said.as_deref() != Some(current.key.as_str()) {
            Some(current.at)
        } else {
            self.following(n).map(|o| o.at)
        }
    }

    /// Said, and waiting to be ticked off (or snoozed).
    pub fn ringing(&self, n: &Now) -> bool {
        self.done.is_none() && self.snooze.is_none() && self.current(n).is_some_and(|o| self.said.as_deref() == Some(o.key.as_str()) || o.at <= n.at)
    }

    /// The time the list goes by: the snooze, else the occurrence's.
    pub fn shown_at(&self, n: &Now) -> Option<i64> {
        self.snooze.or_else(|| self.current(n).map(|o| o.at))
    }
}

/// "in 9 min", "14:05", "tmrw 08:00", "sat 08:00", "nov 28": a time as a
/// list says it, from here.
pub fn when_text(at: i64, n: &Now, h24: bool) -> String {
    let d = at - n.at;
    if d.abs() < 60 {
        return "now".into();
    }
    if d > 0 && d < 3600 {
        return format!("in {} min", (d + 59) / 60);
    }
    if d < 0 && d > -3600 {
        return format!("{} min ago", -d / 60);
    }
    let local = n.zone.local(at);
    let clock = clock_text(local.time(), h24);
    match (local.date() - n.today()).num_days() {
        0 => clock,
        1 => format!("tmrw {clock}"),
        -1 => format!("yday {clock}"),
        2..=6 => format!("{} {clock}", local.format("%a").to_string().to_lowercase()),
        _ => local.format("%b %-d").to_string().to_lowercase(),
    }
}

/// "14:05", or "2:05 pm" on a 12-hour clock.
pub fn clock_text(t: NaiveTime, h24: bool) -> String {
    if h24 {
        t.format("%H:%M").to_string()
    } else {
        t.format("%-I:%M %P").to_string()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::reminders::{List, Link};
    use crate::schedule::{Match, View};

    /// New York's rules for 2026, as the tests' zone: daylight saving from
    /// 8 March 02:00 to 1 November 02:00.
    pub struct NewYork;

    pub fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap().and_utc().timestamp()
    }

    impl Zone for NewYork {
        fn offset(&self, at: i64) -> i64 {
            // 2:00 EST is 7:00 UTC; 2:00 EDT is 6:00 UTC.
            if (utc(2026, 3, 8, 7, 0)..utc(2026, 11, 1, 6, 0)).contains(&at) {
                -4 * 3600
            } else {
                -5 * 3600
            }
        }
    }

    pub fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    pub fn reminder(when: When, repeat: Repeat, created: i64) -> Reminder {
        Reminder { id: 1, text: "swap battery".into(), list: List::Pit, when, repeat, link: Link::None, created, said: None, snooze: None, done: None, past: Vec::new() }
    }

    /// Team 5805's matches on a Saturday in New York: Q31 at 14:00, Q40 at
    /// 14:50, Q52 at 15:40 (UTC is five hours on).
    pub fn saturday() -> View {
        let m = |label: &str, at: i64| Match { key: format!("2026nyro_{}", label.to_lowercase()), label: label.into(), red: vec![5805], blue: vec![], time: Some(at), score: None, status: None, queue_at: None };
        View { team: 5805, matches: vec![m("Q31", utc(2026, 11, 21, 19, 0)), m("Q40", utc(2026, 11, 21, 19, 50)), m("Q52", utc(2026, 11, 21, 20, 40))], ..View::default() }
    }

    #[test]
    fn wall_clock_times_go_through_daylight_saving() {
        let z = NewYork;
        // An ordinary winter and summer morning.
        assert_eq!(z.instant(local(2026, 1, 15, 8, 0)), utc(2026, 1, 15, 13, 0));
        assert_eq!(z.instant(local(2026, 7, 15, 8, 0)), utc(2026, 7, 15, 12, 0));
        assert_eq!(z.local(utc(2026, 7, 15, 12, 0)), local(2026, 7, 15, 8, 0));
        // Spring forward: 2:30 does not happen, and is read as 3:30.
        assert_eq!(z.instant(local(2026, 3, 8, 2, 30)), utc(2026, 3, 8, 7, 30));
        assert_eq!(z.local(utc(2026, 3, 8, 7, 30)), local(2026, 3, 8, 3, 30));
        // Fall back: 1:30 happens twice; the first.
        assert_eq!(z.instant(local(2026, 11, 1, 1, 30)), utc(2026, 11, 1, 5, 30));
    }

    #[test]
    fn a_daily_reminder_keeps_its_clock_time_across_the_change() {
        let z = NewYork;
        // Every day at 8:00, from the Saturday before the clocks go back.
        let mut r = reminder(When::At(local(2026, 10, 31, 8, 0)), Repeat::Daily, 0);
        let n = Now { at: utc(2026, 10, 31, 11, 0), zone: &z, view: None };
        assert_eq!(r.current(&n).unwrap().at, utc(2026, 10, 31, 12, 0), "8:00 EDT");
        assert_eq!(r.following(&n).unwrap().at, utc(2026, 11, 1, 13, 0), "8:00 EST: 25 hours later");
        r.advance(&n);
        assert_eq!(r.when, When::At(local(2026, 11, 1, 8, 0)));
        // Weekdays skip the weekend.
        let r = reminder(When::At(local(2026, 11, 20, 7, 30)), Repeat::Weekdays, 0);
        assert_eq!(z.local(r.following(&n).unwrap().at), local(2026, 11, 23, 7, 30), "friday, then monday");
    }

    #[test]
    fn match_cues_move_with_the_schedule() {
        let z = NewYork;
        let mut view = saturday();
        let n = |at: i64, view: &View| -> Option<i64> {
            let n = Now { at, zone: &z, view: Some(view) };
            reminder(When::Match(Cue::Before(600), Of::One { label: "Q40".into(), key: Some("2026nyro_q40".into()) }), Repeat::Never, 0).next_due(&n)
        };
        let early = utc(2026, 11, 21, 18, 0);
        assert_eq!(n(early, &view), Some(utc(2026, 11, 21, 19, 40)), "10 min before Q40");
        // The event runs 25 minutes late: so does the reminder.
        view.matches[1].time = Some(utc(2026, 11, 21, 20, 15));
        assert_eq!(n(early, &view), Some(utc(2026, 11, 21, 20, 5)));
        // Nexus's queue time, else 20 minutes before; after, six minutes in.
        let m = &view.matches[0];
        assert_eq!(cue_at(Cue::Queue, m, None), Some((utc(2026, 11, 21, 18, 40), true)));
        let mut queued = m.clone();
        queued.queue_at = Some(utc(2026, 11, 21, 18, 35));
        assert_eq!(cue_at(Cue::Queue, &queued, None), Some((utc(2026, 11, 21, 18, 35), false)));
        assert_eq!(cue_at(Cue::After, m, None), Some((utc(2026, 11, 21, 19, 6), false)));
        // Between: halfway from back in the pit (19:06) to queueing for Q40
        // (19:55, now that it is late).
        assert_eq!(cue_at(Cue::Between, m, view.matches.get(1)), Some((utc(2026, 11, 21, 19, 30) + 30, true)));
        assert_eq!(cue_at(Cue::Between, &view.matches[2], None), None, "none after the last match");
    }

    #[test]
    fn every_match_goes_on_from_the_one_it_was_made_before() {
        let z = NewYork;
        let view = saturday();
        // Made at 14:03, while Q31 is on: after Q31 is its first.
        let made = utc(2026, 11, 21, 19, 3);
        let mut r = reminder(When::Match(Cue::After, Of::Each), Repeat::Never, made);
        let n = Now { at: made, zone: &z, view: Some(&view) };
        assert_eq!(r.current(&n).and_then(|o| o.label), Some("Q31".into()));
        assert_eq!(r.following(&n).and_then(|o| o.label), Some("Q40".into()));
        r.advance(&n);
        assert_eq!(r.current(&n).and_then(|o| o.label), Some("Q40".into()));
        // Made after Q31 was over: Q31 is not one of its matches.
        let late = reminder(When::Match(Cue::After, Of::Each), Repeat::Never, utc(2026, 11, 21, 19, 10));
        assert_eq!(late.current(&n).and_then(|o| o.label), Some("Q40".into()));
        // Left a whole afternoon: caught up to the latest come.
        let mut r = reminder(When::Match(Cue::After, Of::Each), Repeat::Never, made);
        r.catch_up(&Now { at: utc(2026, 11, 21, 21, 0), zone: &z, view: Some(&view) });
        assert_eq!(r.current(&n).and_then(|o| o.label), Some("Q52".into()));
        assert_eq!(r.past.len(), 2);
    }

    #[test]
    fn a_long_gap_is_caught_up_in_a_few_steps() {
        let z = NewYork;
        let now = utc(2026, 11, 21, 15, 0);
        let n = Now { at: now, zone: &z, view: None };
        let mut r = reminder(When::In(now - 400 * 86_400), Repeat::Every(60), 0);
        r.catch_up(&n);
        let When::In(at) = r.when else { panic!() };
        assert!(at <= now && now - at < 60, "{}", now - at);
        let mut r = reminder(When::At(local(2025, 1, 6, 8, 0)), Repeat::Weekdays, 0);
        r.catch_up(&n);
        assert_eq!(r.when, When::At(local(2026, 11, 20, 8, 0)), "friday's, the last before saturday");
        let mut r = reminder(When::At(local(2025, 1, 6, 8, 0)), Repeat::Weekly, 0);
        r.catch_up(&n);
        assert_eq!(r.when, When::At(local(2026, 11, 16, 8, 0)), "mondays");
    }

    #[test]
    fn times_read_as_a_list_says_them() {
        let z = NewYork;
        let now = utc(2026, 11, 21, 15, 0); // sat 10:00
        let n = Now { at: now, zone: &z, view: None };
        assert_eq!(when_text(now + 30, &n, true), "now");
        assert_eq!(when_text(now + 9 * 60 - 20, &n, true), "in 9 min");
        assert_eq!(when_text(now - 12 * 60, &n, true), "12 min ago");
        assert_eq!(when_text(now + 4 * 3600 + 5 * 60, &n, true), "14:05");
        assert_eq!(when_text(now + 4 * 3600 + 5 * 60, &n, false), "2:05 pm");
        assert_eq!(when_text(utc(2026, 11, 22, 13, 0), &n, true), "tmrw 08:00");
        assert_eq!(when_text(utc(2026, 11, 25, 13, 0), &n, true), "wed 08:00");
        assert_eq!(when_text(utc(2026, 12, 2, 13, 0), &n, true), "dec 2");
    }
}
