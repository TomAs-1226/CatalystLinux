//! Quick add: a reminder typed the way it would be said. The words that say
//! when come out, and what is left is the reminder:
//!
//! - **a wait**: "swap battery in 10 min", "in half an hour", "1h30 from now"
//! - **a day and a time**: "tomorrow 8am check bumpers", "friday at 14:30",
//!   "tonight", "dec 2", "at 3" (whichever 3 o'clock comes next)
//! - **a repeat**: "every day at 7:30", "weekdays", "every monday", "every 30 min"
//! - **a match**: "before Q42 charge spare" (twenty minutes before),
//!   "10 min before our next match", "at queueing", "after each match",
//!   "between matches every time", "charge spare for Q42"
//! - **a list**: "#pit" or "#personal" (a match cue is the pit's; otherwise
//!   the pit's while an event is on)
//!
//! One cue for several: "after each match: swap battery, check bumpers,
//! download logs" makes three. A reminder that mentions the checklist opens
//! the pit checklist.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use super::plan::{Now, LEAD, MORNING};
use super::{Cue, Link, List, Of, Reminder, Repeat, When};

/// What a line typed says.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub text: String,
    pub when: When,
    pub repeat: Repeat,
    /// A list asked for (#pit, #personal), or the pit's for a match cue.
    pub list: Option<List>,
    pub link: Link,
}

/// What to say when nothing is typed to be reminded of.
pub const NO_TEXT: &str = "what should it say?";

/// Read a line: one reminder, or several sharing one cue.
pub fn parse(input: &str, n: &Now) -> Result<Vec<Parsed>, String> {
    let toks = tokens(input);
    if toks.is_empty() {
        return Err(NO_TEXT.into());
    }
    // "after each match: swap battery, check bumpers": the cue, then a list.
    if let Some(k) = toks.iter().position(|t| t.colon) {
        let head = one(&toks[..=k], n);
        if head.text.is_empty() && head.when != When::Whenever {
            let tail = toks[k + 1..].iter().map(|t| t.raw.as_str()).collect::<Vec<_>>().join(" ");
            let items: Vec<Parsed> = tail.split([',', ';']).map(clean).filter(|t| !t.is_empty()).map(|text| Parsed { link: link_for(&text), text, ..head.clone() }).collect();
            if !items.is_empty() {
                return Ok(items);
            }
        }
    }
    let p = one(&toks, n);
    if p.text.is_empty() {
        return Err(NO_TEXT.into());
    }
    Ok(vec![p])
}

/// Only when: a note's reminder, its words the note's own title.
pub fn when_only(input: &str, n: &Now) -> Option<Parsed> {
    let p = one(&tokens(input), n);
    (p.when != When::Whenever).then_some(p)
}

// ------------------------------------------------------------- words ---

struct Tok {
    /// As typed.
    raw: String,
    /// Lower case, without the punctuation around it.
    w: String,
    /// Ended with a colon: what follows is a list.
    colon: bool,
}

fn tokens(input: &str) -> Vec<Tok> {
    input
        .split_whitespace()
        .filter_map(|raw| {
            let trimmed = raw.trim_start_matches(['(', '[', '"', '\'', '“', '‘']);
            let core = trimmed.trim_end_matches([',', ';', '!', '?', ')', ']', '"', '\'', ':', '”', '’']);
            let colon = trimmed[core.len()..].contains(':');
            let mut w = core.to_lowercase();
            if let Some(stem) = w.strip_suffix("a.m.").or_else(|| w.strip_suffix("a.m")) {
                w = format!("{stem}am");
            } else if let Some(stem) = w.strip_suffix("p.m.").or_else(|| w.strip_suffix("p.m")) {
                w = format!("{stem}pm");
            } else {
                // A full stop, not a decimal point.
                while w.ends_with('.') {
                    w.pop();
                }
            }
            (!raw.is_empty()).then(|| Tok { raw: raw.to_owned(), w, colon })
        })
        .collect()
}

fn number(w: &str) -> Option<f64> {
    if let Ok(v) = w.parse::<f64>() {
        return (v.is_finite() && v >= 0.0).then_some(v);
    }
    Some(match w {
        "a" | "an" | "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "eleven" => 11.0,
        "twelve" => 12.0,
        "fifteen" => 15.0,
        "twenty" => 20.0,
        "thirty" => 30.0,
        "forty" => 40.0,
        "forty-five" | "fortyfive" => 45.0,
        "fifty" => 50.0,
        "sixty" => 60.0,
        "ninety" => 90.0,
        _ => return None,
    })
}

fn unit(w: &str) -> Option<i64> {
    Some(match w {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        "w" | "wk" | "wks" | "week" | "weeks" => 7 * 86_400,
        _ => return None,
    })
}

/// A length written as one word: "10min", "90s", "1h30", "1.5h".
fn compact(w: &str) -> Option<i64> {
    let (mut total, mut rest, mut last) = (0.0, w, 0i64);
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let n: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let letters = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        let secs = match &rest[..letters] {
            // A bare number after hours is minutes; after minutes, seconds.
            "" if last == 3600 => 60,
            "" if last == 60 => 1,
            "" => return None,
            u => unit(u)?,
        };
        rest = &rest[letters..];
        total += n * secs as f64;
        last = secs;
    }
    (total >= 1.0).then_some(total.round() as i64)
}

fn weekday(w: &str, short: bool) -> Option<Weekday> {
    let full = match w {
        "monday" | "mondays" => Some(Weekday::Mon),
        "tuesday" | "tuesdays" => Some(Weekday::Tue),
        "wednesday" | "wednesdays" => Some(Weekday::Wed),
        "thursday" | "thursdays" => Some(Weekday::Thu),
        "friday" | "fridays" => Some(Weekday::Fri),
        "saturday" | "saturdays" => Some(Weekday::Sat),
        "sunday" | "sundays" => Some(Weekday::Sun),
        _ => None,
    };
    full.or_else(|| {
        short.then(|| match w {
            "mon" => Some(Weekday::Mon),
            "tue" | "tues" => Some(Weekday::Tue),
            "wed" | "weds" => Some(Weekday::Wed),
            "thu" | "thur" | "thurs" => Some(Weekday::Thu),
            "fri" => Some(Weekday::Fri),
            "sat" => Some(Weekday::Sat),
            "sun" => Some(Weekday::Sun),
            _ => None,
        })?
    })
}

fn month(w: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    if w.len() < 3 {
        return None;
    }
    let at = MONTHS.iter().position(|m| w.starts_with(m))?;
    // The whole name, or its first three or four letters ("sept").
    let names = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    (names[at].starts_with(w)).then_some(at as u32 + 1)
}

fn day_number(w: &str) -> Option<u32> {
    let digits = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &w[digits.len()..];
    let d: u32 = digits.parse().ok()?;
    (matches!(suffix, "" | "st" | "nd" | "rd" | "th") && (1..=31).contains(&d)).then_some(d)
}

/// A match as the schedule names it: Q14, M3, F1, SF2-1.
pub fn label(w: &str) -> Option<String> {
    let rest = ["sf", "qf", "ef", "q", "m", "f"].iter().find_map(|p| w.strip_prefix(p))?;
    let (number, set) = rest.split_once('-').map_or((rest, None), |(a, b)| (a, Some(b)));
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    (digits(number) && set.is_none_or(digits)).then(|| w.to_uppercase())
}

fn is_match(w: &str) -> bool {
    matches!(w, "match" | "matches" | "round" | "rounds")
}

// ----------------------------------------------------------- reading ---

#[derive(Clone, Copy, Debug, PartialEq)]
enum Day {
    Today,
    Tomorrow,
    On(Weekday),
    Date(u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Clock {
    h: u32,
    m: u32,
    /// "at 3", "3:30": which 3 o'clock is for the day to say.
    loose: bool,
}

#[derive(Default)]
struct Pieces {
    wait: Option<i64>,
    day: Option<Day>,
    clock: Option<Clock>,
    /// "morning", "tonight": an hour.
    part: Option<u32>,
    repeat: Option<Repeat>,
    cue: Option<(Cue, Of)>,
    /// "every time", "every match".
    each: bool,
    list: Option<List>,
}

struct Reader<'a> {
    t: &'a [Tok],
    used: Vec<bool>,
}

impl Reader<'_> {
    fn w(&self, i: usize) -> &str {
        match self.t.get(i) {
            Some(t) if !self.used[i] => &t.w,
            _ => "",
        }
    }

    fn take(&mut self, i: usize, len: usize) {
        for u in self.used.iter_mut().skip(i).take(len) {
            *u = true;
        }
    }

    /// A length: "10 min", "10min", "an hour", "half an hour", "1 hour 30 min".
    fn length(&self, i: usize) -> Option<(usize, i64)> {
        let w = self.w(i);
        let hour = |w: &str| matches!(w, "hour" | "hr" | "h");
        if w == "half" {
            if self.w(i + 1) == "an" && hour(self.w(i + 2)) {
                return Some((3, 1800));
            }
            if hour(self.w(i + 1)) {
                return Some((2, 1800));
            }
        }
        if let Some(secs) = compact(w) {
            return Some((1, secs));
        }
        let n = number(w)?;
        let u = unit(self.w(i + 1))?;
        let mut total = n * u as f64;
        let mut len = 2;
        if self.w(i + 2) == "and" && self.w(i + 3) == "a" && self.w(i + 4) == "half" {
            total += u as f64 * 0.5;
            len = 5;
        } else if let (Some(n2), Some(u2)) = (number(self.w(i + 2)), unit(self.w(i + 3))) {
            if u2 < u && self.w(i + 2) != "a" {
                total += n2 * u2 as f64;
                len = 4;
            }
        }
        (total >= 1.0).then_some((len, total.round() as i64))
    }

    /// A match: "Q42", "our next match", "each match", "qual 42".
    fn of(&self, i: usize) -> Option<(usize, Of)> {
        let mut j = i;
        while matches!(self.w(j), "our" | "the" | "a") {
            j += 1;
        }
        let w = self.w(j);
        if let Some(label) = label(w) {
            return Some((j + 1 - i, Of::One { label, key: None }));
        }
        if matches!(w, "qual" | "quals" | "qualification") {
            if let Ok(n) = self.w(j + 1).parse::<u32>() {
                return Some((j + 2 - i, Of::One { label: format!("Q{n}"), key: None }));
            }
        }
        if w == "next" && is_match(self.w(j + 1)) {
            return Some((j + 2 - i, Of::Next));
        }
        if matches!(w, "each" | "every" | "all") && is_match(self.w(j + 1)) {
            return Some((j + 2 - i, Of::Each));
        }
        if w == "each" && self.w(j + 1) == "of" && self.w(j + 2) == "our" && is_match(self.w(j + 3)) {
            return Some((j + 4 - i, Of::Each));
        }
        match w {
            "matches" | "rounds" => Some((j + 1 - i, Of::Each)),
            "match" | "round" => Some((j + 1 - i, Of::Next)),
            _ => None,
        }
    }

    /// "before", "ahead of", "prior to": how many words.
    fn before(&self, i: usize) -> Option<usize> {
        match (self.w(i), self.w(i + 1)) {
            ("before", _) => Some(1),
            ("ahead", "of") | ("prior", "to") => Some(2),
            _ => None,
        }
    }

    /// A clock time: "8am", "8 pm", "14:05", "at 3", "noon".
    fn clock(&self, i: usize) -> Option<(usize, Clock)> {
        let at = matches!(self.w(i), "at" | "@" | "by" | "around");
        let j = i + at as usize;
        let w = self.w(j);
        match w {
            "noon" | "midday" => return Some((j + 1 - i, Clock { h: 12, m: 0, loose: false })),
            "midnight" => return Some((j + 1 - i, Clock { h: 0, m: 0, loose: false })),
            _ => {}
        }
        let (body, meridiem, used) = if let Some(b) = w.strip_suffix("am").or_else(|| w.strip_suffix('a')) {
            (b, Some(false), 1)
        } else if let Some(b) = w.strip_suffix("pm").or_else(|| w.strip_suffix('p')) {
            (b, Some(true), 1)
        } else {
            match self.w(j + 1) {
                "am" => (w, Some(false), 2),
                "pm" => (w, Some(true), 2),
                "o'clock" | "oclock" => (w, None, 2),
                _ => (w, None, 1),
            }
        };
        let (h, m, colon) = match body.split_once(':') {
            Some((h, m)) if m.len() == 2 => (h, m.parse::<u32>().ok()?, true),
            Some(_) => return None,
            None => (body, 0, false),
        };
        if h.is_empty() || h.len() > 2 || !h.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let h: u32 = h.parse().ok()?;
        if m > 59 {
            return None;
        }
        let o_clock = used == 2 && meridiem.is_none();
        match meridiem {
            Some(pm) if (1..=12).contains(&h) => Some((j + used - i, Clock { h: h % 12 + if pm { 12 } else { 0 }, m, loose: false })),
            Some(_) => None,
            // A bare number is a time only after "at" (or with o'clock);
            // with minutes it is the 24-hour clock's.
            None if !colon && !at && !o_clock => None,
            None if h <= 23 => Some((j + used - i, Clock { h, m, loose: !colon && (1..=11).contains(&h) })),
            None => None,
        }
    }

    /// Read every piece that says when, leaving the rest.
    fn read(&mut self) -> Pieces {
        let mut p = Pieces::default();
        let mut i = 0;
        while i < self.t.len() {
            let len = self.piece(i, &mut p);
            if len == 0 {
                i += 1;
            } else {
                self.take(i, len);
                i += len;
            }
        }
        // "charge spare for Q42": with nothing else to say when, for a match is before it.
        if p.cue.is_none() && p.wait.is_none() && p.day.is_none() && p.clock.is_none() && p.repeat.is_none() {
            for i in 0..self.t.len() {
                if matches!(self.w(i), "for" | "by") {
                    if let Some(label) = label(self.w(i + 1)) {
                        p.cue = Some((Cue::Before(LEAD), Of::One { label, key: None }));
                        self.take(i, 2);
                        break;
                    }
                }
            }
        }
        // "check bumpers every match": before each.
        if p.each && p.cue.is_none() {
            p.cue = Some((Cue::Before(LEAD), Of::Each));
        }
        p
    }

    /// The piece at `i`, if one starts there: how many words it took.
    fn piece(&self, i: usize, p: &mut Pieces) -> usize {
        let w = self.w(i);
        if w.is_empty() {
            return 0;
        }
        // A list.
        match w {
            "#pit" => {
                p.list = Some(List::Pit);
                return 1;
            }
            "#personal" | "#me" | "#home" => {
                p.list = Some(List::Personal);
                return 1;
            }
            _ => {}
        }
        // A wait: "in 10 min", "after 10 min", "10 min from now".
        if matches!(w, "in" | "after" | "within") {
            if let Some((len, secs)) = self.length(i + 1) {
                p.wait = Some(secs);
                return 1 + len;
            }
        }
        if let Some((len, secs)) = self.length(i) {
            if self.w(i + len) == "from" && self.w(i + len + 1) == "now" {
                p.wait = Some(secs);
                return len + 2;
            }
            // "10 min before Q42".
            if let Some(b) = self.before(i + len) {
                if let Some((of_len, of)) = self.of(i + len + b) {
                    p.cue = Some((Cue::Before(secs), of));
                    return len + b + of_len;
                }
            }
        }
        // Before a match.
        if let Some(b) = self.before(i) {
            if let Some((len, of)) = self.of(i + b) {
                p.cue = Some((Cue::Before(LEAD), of));
                return b + len;
            }
        }
        // Queueing: "at queueing", "when we queue", "at queue time for Q42".
        if let Some(len) = self.queue(i) {
            let (of_len, of) = match (self.w(i + len), self.of(i + len + 1)) {
                ("for", Some((l, of))) => (l + 1, of),
                _ => (0, Of::Next),
            };
            p.cue = Some((Cue::Queue, of));
            return len + of_len;
        }
        // After a match.
        if w == "after" {
            if let Some((len, of)) = self.of(i + 1) {
                p.cue = Some((Cue::After, of));
                return 1 + len;
            }
        }
        // Between matches: each gap, halfway.
        if w == "between" {
            let mut j = i + 1;
            while matches!(self.w(j), "each" | "every" | "our" | "the" | "all") {
                j += 1;
            }
            if is_match(self.w(j)) {
                p.cue = Some((Cue::Between, Of::Each));
                return j + 1 - i;
            }
        }
        // "every time", "each time", "every match".
        if matches!(w, "every" | "each") && matches!(self.w(i + 1), "time" | "match" | "round") {
            p.each = true;
            return 2;
        }
        if let Some(len) = self.repeat(i, p) {
            return len;
        }
        if let Some(len) = self.day(i, p) {
            return len;
        }
        if let Some((len, clock)) = self.clock(i) {
            p.clock = Some(clock);
            return len;
        }
        0
    }

    fn queue(&self, i: usize) -> Option<usize> {
        let lead = matches!(self.w(i), "at" | "when" | "on");
        let mut j = i + lead as usize;
        if lead && self.w(j) == "we" {
            j += 1;
        }
        let queue = matches!(self.w(j), "queue" | "queueing" | "queuing" | "queued");
        if !queue {
            return None;
        }
        let time = self.w(j + 1) == "time";
        (lead || time).then_some(j + 1 + time as usize - i)
    }

    fn repeat(&self, i: usize, p: &mut Pieces) -> Option<usize> {
        let set = |p: &mut Pieces, r: Repeat| p.repeat = Some(r);
        match self.w(i) {
            "daily" | "everyday" => set(p, Repeat::Daily),
            "weekly" => set(p, Repeat::Weekly),
            "hourly" => set(p, Repeat::Every(3600)),
            "weekdays" => set(p, Repeat::Weekdays),
            "on" if self.w(i + 1) == "weekdays" => {
                set(p, Repeat::Weekdays);
                return Some(2);
            }
            "every" | "each" => {
                let next = self.w(i + 1);
                let part = match next {
                    "morning" => Some(MORNING),
                    "afternoon" => Some(14),
                    "evening" => Some(18),
                    "night" => Some(20),
                    _ => None,
                };
                if let Some(h) = part {
                    set(p, Repeat::Daily);
                    p.part = Some(h);
                } else if matches!(next, "day" | "days") {
                    set(p, Repeat::Daily);
                } else if matches!(next, "weekday" | "weekdays") {
                    set(p, Repeat::Weekdays);
                } else if next == "week" {
                    set(p, Repeat::Weekly);
                } else if next == "hour" {
                    set(p, Repeat::Every(3600));
                } else if let Some(day) = weekday(next, true) {
                    set(p, Repeat::Weekly);
                    p.day = Some(Day::On(day));
                } else if let Some((len, secs)) = self.length(i + 1) {
                    // "every 30 min": no shorter than a minute, no longer than a week.
                    set(p, Repeat::Every(secs.clamp(60, 7 * 86_400)));
                    return Some(1 + len);
                } else {
                    return None;
                }
                return Some(2);
            }
            _ => return None,
        }
        Some(1)
    }

    fn day(&self, i: usize, p: &mut Pieces) -> Option<usize> {
        let w = self.w(i);
        match w {
            "today" => p.day = Some(Day::Today),
            "tonight" => {
                p.day = Some(Day::Today);
                p.part.get_or_insert(20);
            }
            "tomorrow" | "tmrw" | "tmr" | "tomorow" | "tommorow" | "tommorrow" => p.day = Some(Day::Tomorrow),
            _ => {
                // A part of the day: "morning", "this evening", "in the afternoon".
                let (skip, part) = match (w, self.w(i + 1), self.w(i + 2)) {
                    ("this", x, _) => (1, x),
                    ("in", "the", x) => (2, x),
                    (x, _, _) => (0, x),
                };
                let hour = match part {
                    "morning" => Some(MORNING),
                    "afternoon" => Some(14),
                    "evening" => Some(18),
                    "night" if skip > 0 => Some(20),
                    _ => None,
                };
                if let Some(h) = hour {
                    p.part = Some(h);
                    return Some(skip + 1);
                }
                // A weekday: "friday", "on fri", "next friday"; a short name
                // only with a word or a time that makes it one.
                let lead = matches!(w, "on" | "next" | "this");
                let j = i + lead as usize;
                let short = lead || self.clock(j + 1).is_some();
                if let Some(day) = weekday(self.w(j), short) {
                    p.day = Some(Day::On(day));
                    return Some(j + 1 - i);
                }
                // A date: "dec 2", "2 dec", "on the 2nd of december".
                let mut j = i;
                for filler in ["on", "the"] {
                    if self.w(j) == filler {
                        j += 1;
                    }
                }
                if let (Some(m), Some(d)) = (month(self.w(j)), day_number(self.w(j + 1))) {
                    p.day = Some(Day::Date(m, d));
                    return Some(j + 2 - i);
                }
                let of = (self.w(j + 1) == "of") as usize;
                if let (Some(d), Some(m)) = (day_number(self.w(j)), month(self.w(j + 1 + of))) {
                    p.day = Some(Day::Date(m, d));
                    return Some(j + 2 + of - i);
                }
                return None;
            }
        }
        Some(1)
    }
}

/// What is left once the words that say when are out: without "remind me
/// to" and the punctuation at its ends.
fn clean(text: &str) -> String {
    let mut t = text.trim().to_owned();
    loop {
        let before = t.clone();
        t = t.trim_matches(|c: char| c.is_whitespace() || ",;:-–—".contains(c)).to_owned();
        let low = t.to_lowercase();
        for lead in ["remind me to ", "remind me ", "to ", "that ", "and ", "then ", "please "] {
            if low.starts_with(lead) {
                t = t[lead.len()..].to_owned();
                break;
            }
        }
        let low = t.to_lowercase();
        for tail in [" and", " then"] {
            if low.ends_with(tail) {
                t.truncate(t.len() - tail.len());
                break;
            }
        }
        if t == before {
            return t;
        }
    }
}

/// A reminder about the checklist opens it; one for a briefing, that.
fn link_for(text: &str) -> Link {
    let lower = text.to_lowercase();
    if lower.contains("checklist") {
        Link::Checklist
    } else if lower.contains("briefing") {
        Link::Briefing
    } else {
        Link::None
    }
}

fn one(toks: &[Tok], n: &Now) -> Parsed {
    let mut reader = Reader { t: toks, used: vec![false; toks.len()] };
    let p = reader.read();
    let left: Vec<&str> = toks.iter().zip(&reader.used).filter(|(_, used)| !**used).map(|(t, _)| t.raw.as_str()).collect();
    let text = clean(&left.join(" "));
    let (when, repeat) = build(&p, n);
    let list = p.list.or_else(|| matches!(when, When::Match(..)).then_some(List::Pit));
    Parsed { link: link_for(&text), text, when, repeat, list }
}

fn build(p: &Pieces, n: &Now) -> (When, Repeat) {
    if let Some((cue, of)) = &p.cue {
        return (When::Match(*cue, if p.each { Of::Each } else { of.clone() }), Repeat::Never);
    }
    let timed = p.day.is_some() || p.clock.is_some() || p.part.is_some();
    match p.repeat {
        Some(Repeat::Every(every)) if !timed => (When::In(n.at + every), Repeat::Every(every)),
        Some(repeat) => (When::At(resolve(p, n)), repeat),
        None if timed => (When::At(resolve(p, n)), Repeat::Never),
        None => match p.wait {
            Some(secs) => (When::In(n.at + secs.clamp(1, 366 * 86_400)), Repeat::Never),
            None => (When::Whenever, Repeat::Never),
        },
    }
}

/// The wall-clock time a day and a time typed mean, from now.
fn resolve(p: &Pieces, n: &Now) -> NaiveDateTime {
    let today = n.today();
    let later = |d: NaiveDate, h: u32, m: u32| n.zone.instant(d.and_hms_opt(h, m, 0).unwrap_or_default()) > n.at;
    let (h, m, loose) = match (p.clock, p.part) {
        (Some(c), _) => (c.h, c.m, c.loose),
        (None, Some(h)) => (h, 0, false),
        (None, None) => (MORNING, 0, false),
    };
    // A bare hour, "at 3": with a day, 1 to 6 are the afternoon's; without
    // one, whichever of its two comes next.
    let h = match (loose, p.day.is_some()) {
        (false, _) => h,
        (true, true) => h + if (1..=6).contains(&h) { 12 } else { 0 },
        (true, false) if !later(today, h, m) && later(today, h + 12, m) => h + 12,
        (true, false) => h,
    };
    let date = match p.day {
        Some(Day::Today) => today,
        Some(Day::Tomorrow) => today + Duration::days(1),
        Some(Day::On(day)) => {
            let ahead = (day.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
            let d = today + Duration::days(ahead as i64);
            if ahead == 0 && !later(d, h, m) {
                d + Duration::days(7)
            } else {
                d
            }
        }
        Some(Day::Date(month, day)) => {
            let this = NaiveDate::from_ymd_opt(today.year(), month, day);
            match this {
                Some(d) if d >= today => d,
                _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day).unwrap_or(today),
            }
        }
        // No day: today's, or tomorrow's once today's has gone.
        None if later(today, h, m) => today,
        None => today + Duration::days(1),
    };
    let mut at = date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN));
    if p.repeat == Some(Repeat::Weekdays) {
        while matches!(at.weekday(), Weekday::Sat | Weekday::Sun) {
            at += Duration::days(1);
        }
    }
    at
}

// ----------------------------------------------------- saying it back ---

/// A length as typed back: "10 min", "1 hour", "3 hours", "90 min".
pub fn length_text(secs: i64) -> String {
    match secs {
        3600 => "1 hour".into(),
        s if s % 3600 == 0 && s >= 7200 => format!("{} hours", s / 3600),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{s} seconds"),
    }
}

/// A match as typed back.
fn of_text(of: &Of) -> String {
    match of {
        Of::One { label, .. } => label.clone(),
        Of::Next => "the next match".into(),
        Of::Each => "each match".into(),
    }
}

/// A cue as typed back: "20 min before Q42", "at queueing", "after each match".
pub fn cue_text(cue: Cue, of: &Of) -> String {
    match cue {
        Cue::Before(LEAD) => format!("before {}", of_text(of)),
        Cue::Before(lead) => format!("{} before {}", length_text(lead), of_text(of)),
        Cue::Queue => match of {
            Of::Next => "at queueing".into(),
            Of::Each => "at queueing every time".into(),
            one => format!("at queueing for {}", of_text(one)),
        },
        Cue::After => format!("after {}", of_text(of)),
        Cue::Between => "between matches".into(),
    }
}

/// "today at 14:05", "tomorrow at 08:00", "saturday at 08:00", "dec 2 at 08:00".
fn local_text(at: NaiveDateTime, n: &Now) -> String {
    let clock = at.format("%H:%M");
    let day = match (at.date() - n.today()).num_days() {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        2..=6 => at.format("%A").to_string().to_lowercase(),
        _ => at.format("%b %-d").to_string().to_lowercase(),
    };
    format!("{day} at {clock}")
}

/// How a reminder's time is typed: what quick add reads back to the same.
pub fn when_text(r: &Reminder, n: &Now) -> String {
    let weekday = |at: NaiveDateTime| at.format("%A").to_string().to_lowercase();
    match (&r.when, r.repeat) {
        (When::Whenever, _) => String::new(),
        (When::In(_) | When::At(_), Repeat::Every(every)) => format!("every {}", length_text(every)),
        (When::In(at), _) => local_text(n.zone.local(*at), n),
        (When::At(at), Repeat::Never) => local_text(*at, n),
        (When::At(at), Repeat::Daily) => format!("every day at {}", at.format("%H:%M")),
        (When::At(at), Repeat::Weekdays) => format!("every weekday at {}", at.format("%H:%M")),
        (When::At(at), Repeat::Weekly) => format!("every {} at {}", weekday(*at), at.format("%H:%M")),
        (When::Match(cue, of), _) => cue_text(*cue, of),
    }
}

/// The whole reminder as typed: its words, its time, and its list when it
/// is not the one it would be on anyway.
pub fn phrase(r: &Reminder, n: &Now) -> String {
    let usual = if matches!(r.when, When::Match(..)) || n.event_today() { List::Pit } else { List::Personal };
    let list = if r.list == usual { "" } else { r.list.tag() };
    [r.text.as_str(), &when_text(r, n), list].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminders::plan::tests::{local, saturday, utc, NewYork};
    use crate::reminders::plan::Zone;

    /// Saturday 21 November 2026, 10:00 in New York.
    fn sat() -> i64 {
        utc(2026, 11, 21, 15, 0)
    }

    fn read(text: &str) -> Parsed {
        read_at(text, sat())
    }

    fn read_at(text: &str, at: i64) -> Parsed {
        let view = saturday();
        let n = Now { at, zone: &NewYork, view: Some(&view) };
        let mut all = parse(text, &n).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert_eq!(all.len(), 1, "{text:?}");
        all.remove(0)
    }

    fn at(text: &str) -> NaiveDateTime {
        match read(text).when {
            When::At(at) => at,
            other => panic!("{text:?}: {other:?}"),
        }
    }

    #[test]
    fn a_wait_is_taken_out_and_the_rest_is_the_reminder() {
        for (typed, text, secs) in [
            ("swap battery in 10 min", "swap battery", 600),
            ("in 10 minutes swap battery", "swap battery", 600),
            ("Swap battery in 10min", "Swap battery", 600),
            ("remind me to charge the spare in an hour", "charge the spare", 3600),
            ("check chain in half an hour", "check chain", 1800),
            ("in 1h30 re-zero the arm", "re-zero the arm", 5400),
            ("in 1 hour 30 min lunch", "lunch", 5400),
            ("in an hour and a half stretch", "stretch", 5400),
            ("tighten bolts 90s from now", "tighten bolts", 90),
            ("in 2.5 hours pack up", "pack up", 9000),
            ("after 15 minutes: check the battery", "check the battery", 900),
        ] {
            let p = read(typed);
            assert_eq!((p.text.as_str(), p.when.clone()), (text, When::In(sat() + secs)), "{typed:?}");
            assert_eq!(p.repeat, Repeat::Never);
        }
    }

    #[test]
    fn a_day_and_a_time_are_a_wall_clock_time() {
        let p = read("tomorrow 8am check bumpers");
        assert_eq!((p.text.as_str(), p.when), ("check bumpers", When::At(local(2026, 11, 22, 8, 0))));
        assert_eq!(at("call home at 8:30pm"), local(2026, 11, 21, 20, 30));
        assert_eq!(at("call home at 8:30 p.m."), local(2026, 11, 21, 20, 30));
        assert_eq!(at("judges at 14:05"), local(2026, 11, 21, 14, 5));
        assert_eq!(at("judges at noon"), local(2026, 11, 21, 12, 0));
        // 9:00 has gone today: tomorrow's.
        assert_eq!(at("standup 09:00"), local(2026, 11, 22, 9, 0));
        // "at 3": the next 3 o'clock is this afternoon's; "at 11", this morning's.
        assert_eq!(at("pizza at 3"), local(2026, 11, 21, 15, 0));
        assert_eq!(at("pizza at 11"), local(2026, 11, 21, 11, 0));
        // With a day, 1 to 6 are the afternoon's.
        assert_eq!(at("tomorrow at 4 alliance meeting"), local(2026, 11, 22, 16, 0));
        assert_eq!(at("monday at 7 bus"), local(2026, 11, 23, 7, 0));
        // A day alone is its morning; tonight its evening.
        assert_eq!(at("friday order parts"), local(2026, 11, 27, 9, 0));
        assert_eq!(at("on sat charge everything"), local(2026, 11, 28, 9, 0), "today is saturday and 9:00 has gone");
        assert_eq!(at("tonight wash the bumpers"), local(2026, 11, 21, 20, 0));
        assert_eq!(at("this evening pack the cart"), local(2026, 11, 21, 18, 0));
        assert_eq!(at("dec 2 inspection"), local(2026, 12, 2, 9, 0));
        assert_eq!(at("on the 2nd dec at 15:30 inspection"), local(2026, 12, 2, 15, 30));
        assert_eq!(at("jan 5 kickoff"), local(2027, 1, 5, 9, 0), "past this year: next year's");
        assert_eq!(read("dec 2 inspection").text, "inspection");
    }

    #[test]
    fn short_day_names_need_a_word_or_a_time_to_be_days() {
        assert_eq!(read("wipe the sun visor").when, When::Whenever, "sun is just a word here");
        assert_eq!(read("wipe the sun visor").text, "wipe the sun visor");
        assert_eq!(at("sun 10:30 brunch"), local(2026, 11, 22, 10, 30));
        assert_eq!(at("brunch on sun"), local(2026, 11, 22, 9, 0));
    }

    #[test]
    fn repeats_are_read() {
        let p = read("stretch every day at 7:30");
        assert_eq!((p.when, p.repeat), (When::At(local(2026, 11, 22, 7, 30)), Repeat::Daily));
        let p = read("weekdays 7:00 bus");
        assert_eq!((p.when, p.repeat), (When::At(local(2026, 11, 23, 7, 0)), Repeat::Weekdays), "the next weekday's");
        let p = read("every monday team meeting");
        assert_eq!((p.text.as_str(), p.when, p.repeat), ("team meeting", When::At(local(2026, 11, 23, 9, 0)), Repeat::Weekly));
        let p = read("check battery voltage every 30 min");
        assert_eq!((p.text.as_str(), p.when, p.repeat), ("check battery voltage", When::In(sat() + 1800), Repeat::Every(1800)));
        let p = read("water hourly");
        assert_eq!(p.repeat, Repeat::Every(3600));
        let p = read("every morning vitamins");
        assert_eq!((p.when, p.repeat), (When::At(local(2026, 11, 22, 9, 0)), Repeat::Daily));
    }

    #[test]
    fn matches_are_cues() {
        let one = |label: &str| Of::One { label: label.into(), key: None };
        for (typed, text, cue, of) in [
            ("before Q42 charge spare", "charge spare", Cue::Before(LEAD), one("Q42")),
            ("charge spare before q42", "charge spare", Cue::Before(LEAD), one("Q42")),
            ("10 min before our next match grab the cart", "grab the cart", Cue::Before(600), Of::Next),
            ("swap battery 5 minutes before each match", "swap battery", Cue::Before(300), Of::Each),
            ("at queueing bring the driver station", "bring the driver station", Cue::Queue, Of::Next),
            ("when we queue for Q40 bring the cart", "bring the cart", Cue::Queue, one("Q40")),
            ("after each match swap battery", "swap battery", Cue::After, Of::Each),
            ("download logs after the match", "download logs", Cue::After, Of::Next),
            ("after Q31 download logs", "download logs", Cue::After, one("Q31")),
            ("check chain tension between matches every time", "check chain tension", Cue::Between, Of::Each),
            ("at queueing every time grab the battery", "grab the battery", Cue::Queue, Of::Each),
            ("charge spare for Q42", "charge spare", Cue::Before(LEAD), one("Q42")),
            ("check bumpers every match", "check bumpers", Cue::Before(LEAD), Of::Each),
            ("before qual 42 charge spare", "charge spare", Cue::Before(LEAD), one("Q42")),
            ("before SF2-1 swap battery", "swap battery", Cue::Before(LEAD), one("SF2-1")),
        ] {
            let p = read(typed);
            assert_eq!((p.text.as_str(), p.when.clone()), (text, When::Match(cue, of.clone())), "{typed:?}");
            assert_eq!(p.list, Some(List::Pit), "{typed:?}: a match cue is the pit's");
        }
    }

    #[test]
    fn one_cue_can_carry_a_list() {
        let view = saturday();
        let n = Now { at: sat(), zone: &NewYork, view: Some(&view) };
        let all = parse("after each match: swap battery, check bumpers, download logs", &n).unwrap();
        assert_eq!(all.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["swap battery", "check bumpers", "download logs"]);
        assert!(all.iter().all(|p| p.when == When::Match(Cue::After, Of::Each)));
        // A colon with no cue before it is just a colon.
        let all = parse("Note: zip ties in the blue bin", &n).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "Note: zip ties in the blue bin");
        // Nor is the colon in a time one.
        let p = read("at 14:30 judges");
        assert_eq!((p.text.as_str(), p.when), ("judges", When::At(local(2026, 11, 21, 14, 30))));
    }

    #[test]
    fn what_has_no_time_is_a_to_do_and_nothing_at_all_is_asked_about() {
        let p = read("buy zip ties #personal");
        assert_eq!((p.text.as_str(), p.when, p.list), ("buy zip ties", When::Whenever, Some(List::Personal)));
        let view = saturday();
        let n = Now { at: sat(), zone: &NewYork, view: Some(&view) };
        assert_eq!(parse("", &n), Err(NO_TEXT.into()));
        assert_eq!(parse("in 10 min", &n), Err(NO_TEXT.into()));
        assert_eq!(parse("after each match:", &n), Err(NO_TEXT.into()));
        // A note's reminder: when, and nothing else.
        assert_eq!(when_only("in 30 min", &n).map(|p| p.when), Some(When::In(sat() + 1800)));
        assert_eq!(when_only("whenever", &n), None);
    }

    #[test]
    fn the_checklist_is_linked() {
        assert_eq!(read("20 min before each match run the pit checklist").link, Link::Checklist);
        assert_eq!(read("swap battery in 5 min").link, Link::None);
    }

    #[test]
    fn what_is_said_back_reads_back_the_same() {
        let view = saturday();
        let z = NewYork;
        let n = Now { at: sat(), zone: &z, view: Some(&view) };
        for typed in [
            "swap battery tomorrow 8am",
            "judges at 14:05",
            "standup every day at 7:30",
            "bus weekdays 7:00",
            "team meeting every monday",
            "dec 2 inspection",
            "check voltage every 30 min",
            "charge spare before Q42",
            "grab the cart 10 min before our next match",
            "swap battery after each match",
            "bring the cart when we queue for Q40",
            "check chain between matches",
            "laundry friday 18:00 #pit",
        ] {
            let p = read(typed);
            let r = Reminder { id: 1, text: p.text.clone(), list: p.list.unwrap_or(List::Personal), when: p.when.clone(), repeat: p.repeat, link: Link::None, created: sat(), said: None, snooze: None, done: None, past: Vec::new() };
            let again = parse(&phrase(&r, &n), &n).unwrap().remove(0);
            assert_eq!((again.text.as_str(), &again.when, again.repeat), (p.text.as_str(), &p.when, p.repeat), "{typed:?} said back as {:?}", phrase(&r, &n));
            assert_eq!(again.list.unwrap_or(List::Personal), r.list, "{typed:?}");
        }
        // A wait is said back as the time it comes to.
        let r = Reminder { when: When::In(sat() + 3600 * 4 + 300), ..Reminder::blank(1, "pack", sat()) };
        assert_eq!(phrase(&r, &n), "pack today at 14:05");
        assert_eq!(z.instant(at("pack today at 14:05")), sat() + 3600 * 4 + 300);
    }
}
