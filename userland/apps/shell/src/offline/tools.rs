//! The match sheets on this handheld, and the few questions they are asked:
//! one match's facts, two matches compared, which matches had something,
//! how something trends over the event, one value's range, and a whole
//! match's report. Each answers in `Line`s: a fact as the model reads it
//! and the post-check holds it to.

use std::collections::BTreeMap;

use super::facts::{clock, join_and, Fact, Kind, MatchFacts, Num, Unit};
use crate::wpilog::Level;

/// What a question is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Any,
    Brownout,
    Battery,
    Can,
    Loop,
    Heat,
    Current,
    Stall,
    Link,
    Comms,
    Estop,
    Auto,
    Mechanism,
    Code,
    Motors,
    Outcome,
}

impl Topic {
    pub const ALL: [Topic; 16] = [
        Topic::Any,
        Topic::Brownout,
        Topic::Battery,
        Topic::Can,
        Topic::Loop,
        Topic::Heat,
        Topic::Current,
        Topic::Stall,
        Topic::Link,
        Topic::Comms,
        Topic::Estop,
        Topic::Auto,
        Topic::Mechanism,
        Topic::Code,
        Topic::Motors,
        Topic::Outcome,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Topic::Any => "any",
            Topic::Brownout => "brownout",
            Topic::Battery => "battery",
            Topic::Can => "can",
            Topic::Loop => "loop",
            Topic::Heat => "heat",
            Topic::Current => "current",
            Topic::Stall => "stall",
            Topic::Link => "link",
            Topic::Comms => "comms",
            Topic::Estop => "estop",
            Topic::Auto => "auto",
            Topic::Mechanism => "mechanism",
            Topic::Code => "code",
            Topic::Motors => "motors",
            Topic::Outcome => "outcome",
        }
    }

    pub fn from_name(name: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The kinds of fact it takes in, most telling first.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Topic::Any => &[],
            Topic::Brownout | Topic::Battery => &[Kind::Brownout, Kind::Sag, Kind::BatteryStart, Kind::Battery],
            Topic::Can => &[Kind::CanDown, Kind::CanErrors, Kind::CanBusy, Kind::MotorLost, Kind::Can],
            Topic::Loop => &[Kind::Loop, Kind::Cpu],
            Topic::Heat => &[Kind::Heat],
            Topic::Current => &[Kind::Stall, Kind::Current],
            Topic::Stall => &[Kind::Stall, Kind::Current],
            Topic::Link => &[Kind::Estop, Kind::Comms, Kind::Disabled],
            Topic::Comms => &[Kind::Comms, Kind::Disabled],
            Topic::Estop => &[Kind::Estop, Kind::Disabled],
            Topic::Auto => &[Kind::Auto],
            Topic::Mechanism => &[Kind::Mechanism, Kind::Stall],
            Topic::Code => &[Kind::CodeError, Kind::Code],
            Topic::Motors => &[Kind::MotorLost, Kind::Heat, Kind::Stall, Kind::Current],
            Topic::Outcome => &[Kind::Outcome],
        }
    }

    /// The kinds that say it happened at all (a brownout, a hot motor), and
    /// how a match without one is put.
    fn happened(self) -> Option<(&'static [Kind], Kind, &'static str)> {
        Some(match self {
            Topic::Brownout => (&[Kind::Brownout], Kind::Brownout, "the robot did not brown out"),
            Topic::Heat => (&[Kind::Heat], Kind::Heat, "no motor reached 70 °C"),
            Topic::Link => (&[Kind::Estop, Kind::Comms, Kind::Disabled], Kind::Comms, "the robot was never e-stopped, disabled or cut off from the driver station mid-match"),
            Topic::Comms => (&[Kind::Comms], Kind::Comms, "the robot never lost comms with the driver station"),
            Topic::Estop => (&[Kind::Estop], Kind::Estop, "the robot was never e-stopped"),
            Topic::Can => (&[Kind::CanDown, Kind::CanErrors, Kind::CanBusy], Kind::CanErrors, "no CAN bus went down, logged errors or passed 90%"),
            Topic::Stall => (&[Kind::Stall], Kind::Stall, "no motor stalled"),
            Topic::Mechanism => (&[Kind::Mechanism], Kind::Mechanism, "no mechanism was short of its setpoint for 2 s or more"),
            Topic::Code => (&[Kind::CodeError], Kind::CodeError, "the robot code threw no exceptions"),
            Topic::Motors => (&[Kind::MotorLost], Kind::MotorLost, "no motor dropped off its bus"),
            _ => return None,
        })
    }
}

/// One fact as an answer is made from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The matches it is about: one, or two compared.
    pub labels: Vec<String>,
    /// The fact, without the match.
    pub body: String,
    pub kind: Kind,
    pub level: Level,
    /// It says something did not happen.
    pub negative: bool,
    pub names: Vec<String>,
    /// What an answer resting on it must carry: its number, its time, or its name.
    pub key: Option<String>,
}

impl Line {
    fn of(fact: &Fact, label: &str) -> Line {
        // What an answer resting on it must carry: when, for an event; the
        // measure, for a measure; the routine, exception or commit by name.
        let time = fact.at.map(clock);
        let measure = fact.nums.iter().find(|n| !matches!(n.unit, Unit::Count | Unit::Seconds)).or_else(|| fact.nums.iter().find(|n| n.unit != Unit::Count)).or(fact.nums.first()).map(Num::shown);
        let key = match fact.kind {
            Kind::Estop | Kind::Comms | Kind::Disabled | Kind::MotorLost | Kind::CanDown | Kind::Stall => time.clone(),
            Kind::Mechanism => fact.nums.iter().find(|n| matches!(n.unit, Unit::Meters | Unit::Degrees | Unit::Rps)).map(Num::shown).or(measure),
            Kind::CodeError | Kind::Auto | Kind::Code => fact.names.first().cloned(),
            _ => measure,
        };
        let key = key.or(time).or_else(|| Some(fact.subject.clone()).filter(|s| !s.is_empty()));
        let mut names = vec![fact.subject.clone()];
        names.extend(fact.names.iter().cloned());
        Line { labels: vec![label.to_owned()], body: fact.text.clone(), kind: fact.kind, level: fact.level, negative: false, names, key }
    }

    fn none(labels: Vec<String>, kind: Kind, body: String) -> Line {
        Line { labels, body, kind, level: Level::Fine, negative: true, names: Vec::new(), key: None }
    }

    /// As the model reads it: "Q14: the robot browned out once…".
    pub fn text(&self) -> String {
        if self.labels.is_empty() {
            self.body.clone()
        } else {
            format!("{}: {}", self.labels.join(" vs "), self.body)
        }
    }
}

/// A whole match at a glance, for the report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub label: String,
    /// "nyro q14 · nov 16 · 10:15"
    pub title: String,
    pub rows: Vec<(Level, String)>,
}

/// The robot as it is just now, for a question about it offline.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub linked: bool,
    pub battery: Option<f64>,
    pub browned_out: bool,
    pub loop_ms: Option<f64>,
    pub can: Option<(String, f64)>,
    pub motors: Option<(i64, i64)>,
    /// Checks firing: (severity, subsystem, id, detail).
    pub firing: Vec<(String, String, String, String)>,
}

/// Everything offline answers come from.
#[derive(Default)]
pub struct Library {
    /// Newest first.
    pub matches: Vec<MatchFacts>,
    /// Each match's title as Logs shows it.
    pub titles: Vec<String>,
    /// How each match went, from the schedule: label → "won 101–95 on blue".
    pub outcomes: BTreeMap<String, String>,
    /// The schedule's next match and its alliances, in a line.
    pub next: Option<String>,
    pub live: Option<Live>,
}

/// Mechanism words people use, for holding an answer to the names in its facts.
pub const MECHANISM_WORDS: [&str; 19] = [
    "elevator", "shooter", "intake", "arm", "wrist", "climber", "drive", "drivetrain", "swerve", "turret", "hood", "indexer", "feeder", "pivot", "conveyor", "flywheel", "claw", "gripper", "steer",
];

impl Library {
    pub fn labels(&self) -> Vec<String> {
        self.matches.iter().map(|m| m.label.clone()).collect()
    }

    /// The match a label names (Q14, qm14, q 14), if its log is here.
    pub fn index_of(&self, label: &str) -> Option<usize> {
        let want = label.to_ascii_uppercase().replace(' ', "").replace("QM", "Q");
        self.matches.iter().position(|m| m.label.eq_ignore_ascii_case(&want))
    }

    /// The names an answer can be held to: motors, mechanisms, CAN buses,
    /// routines, exceptions and commits (not "battery" or "robot").
    pub fn proper_names(&self) -> Vec<String> {
        let mut out = self.mechanisms();
        for m in &self.matches {
            for f in &m.facts {
                let named: Vec<&String> = match f.kind {
                    Kind::CanDown | Kind::CanErrors | Kind::CanBusy | Kind::Can if f.subject.starts_with("can_") => vec![&f.subject],
                    Kind::MotorLost | Kind::Auto | Kind::CodeError | Kind::Code => f.names.iter().collect(),
                    _ => Vec::new(),
                };
                for n in named {
                    if !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                        out.push(n.clone());
                    }
                }
            }
        }
        out
    }

    /// The motors and mechanisms the sheets name, for a question naming one.
    pub fn mechanisms(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for m in &self.matches {
            for f in &m.facts {
                let named = match f.kind {
                    Kind::Heat | Kind::Current | Kind::Stall | Kind::Mechanism | Kind::MotorLost => vec![&f.subject],
                    Kind::Brownout | Kind::Sag => f.names.iter().collect(),
                    _ => Vec::new(),
                };
                for n in named {
                    if !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                        out.push(n.clone());
                    }
                }
            }
        }
        out
    }

    fn label(&self, i: usize) -> &str {
        &self.matches[i].label
    }

    /// A match's facts on a topic, about a subject if one is named, worst
    /// first. A topic whose fault is not there is said not to be.
    pub fn match_facts(&self, i: usize, topic: Topic, subject: Option<&str>, most: usize) -> Vec<Line> {
        let m = &self.matches[i];
        let label = m.label.clone();
        if topic == Topic::Outcome {
            return vec![self.outcome(i).unwrap_or_else(|| Line::none(vec![label], Kind::Outcome, "its result is not known on this handheld (the schedule has no score for it)".into()))];
        }
        let about = |f: &Fact| subject.is_none_or(|s| f.subject.eq_ignore_ascii_case(s) || f.names.iter().any(|n| n.eq_ignore_ascii_case(s)));
        let kinds = topic.kinds();
        let mut picked: Vec<&Fact> = m.facts.iter().filter(|f| (kinds.is_empty() || kinds.contains(&f.kind)) && about(f)).collect();
        // Most telling first: the topic's own order of kinds, then the score.
        picked.sort_by_key(|f| (kinds.iter().position(|k| *k == f.kind).unwrap_or(0), std::cmp::Reverse(f.score)));
        if kinds.is_empty() {
            picked.sort_by_key(|f| std::cmp::Reverse(f.score));
        }
        let mut lines: Vec<Line> = Vec::new();
        if let Some((happened, kind, none)) = topic.happened() {
            let did = picked.iter().any(|f| happened.contains(&f.kind) && f.level != Level::Fine);
            if !did {
                let body = match subject {
                    Some(s) if topic == Topic::Heat => format!("{s} did not reach 70 °C"),
                    Some(s) if topic == Topic::Stall => format!("{s} did not stall"),
                    _ => none.to_owned(),
                };
                lines.push(Line::none(vec![label.clone()], kind, body));
                // What is there instead: the fine facts of the topic.
                picked.retain(|f| f.level == Level::Fine || !happened.contains(&f.kind));
            }
        }
        lines.extend(picked.into_iter().map(|f| Line::of(f, &label)));
        if lines.is_empty() {
            let what = subject.map_or_else(|| format!("about {}", topic.name()), |s| format!("about {s}"));
            lines.push(Line::none(vec![label], Kind::Health, format!("nothing {what} is in its log")));
        }
        lines.truncate(most.max(1));
        lines
    }

    /// What went wrong in a match: its faults and warnings, worst first, or
    /// that nothing did; and how it went, if known.
    pub fn wrong(&self, i: usize, most: usize) -> Vec<Line> {
        let m = &self.matches[i];
        let mut lines: Vec<Line> = m.facts.iter().filter(|f| f.level != Level::Fine).take(most).map(|f| Line::of(f, &m.label)).collect();
        if lines.is_empty() {
            lines.push(Line::none(vec![m.label.clone()], Kind::Health, "nothing stood out: no fault or warning in its log".into()));
            lines.extend(m.facts.iter().filter(|f| matches!(f.kind, Kind::Battery | Kind::Heat | Kind::Loop)).take(most.saturating_sub(1).min(2)).map(|f| Line::of(f, &m.label)));
        }
        if lines.len() < most {
            if let Some(o) = self.outcome(i) {
                lines.push(o);
            }
        }
        lines
    }

    pub fn outcome(&self, i: usize) -> Option<Line> {
        let label = self.label(i).to_owned();
        let result = self.outcomes.get(&label)?;
        Some(Line { labels: vec![label], body: format!("the team {result}"), kind: Kind::Outcome, level: Level::Fine, negative: false, names: Vec::new(), key: result.split_whitespace().nth(1).map(str::to_owned) })
    }

    /// Two matches side by side, `newer` against `older`: what is worse,
    /// what is new, what is better.
    pub fn compare(&self, newer: usize, older: usize, most: usize) -> Vec<Line> {
        let (a, b) = (&self.matches[newer], &self.matches[older]);
        let labels = vec![a.label.clone(), b.label.clone()];
        let mut scored: Vec<(i32, Line)> = Vec::new();
        let line = |body: String, kind: Kind, level: Level, key: Option<String>, names: Vec<String>| Line { labels: labels.clone(), body, kind, level, negative: false, names, key };
        // (metric, words, unit, worse when higher, how much is a change)
        let mut measures: Vec<(String, String, Unit, bool, f64, Kind, Vec<String>)> = vec![
            ("brownouts".into(), "brownouts".into(), Unit::Count, true, 0.5, Kind::Brownout, vec![]),
            ("battery_low".into(), "battery low point".into(), Unit::Volts, false, 0.3, Kind::Battery, vec![]),
            ("battery_start".into(), "battery at the start".into(), Unit::Volts, false, 0.2, Kind::BatteryStart, vec![]),
            ("link_faults".into(), "e-stops, lost links and disables".into(), Unit::Count, true, 0.5, Kind::Comms, vec![]),
            ("motors_lost".into(), "motors that dropped off".into(), Unit::Count, true, 0.5, Kind::MotorLost, vec![]),
            ("can_down".into(), "CAN buses down".into(), Unit::Count, true, 0.5, Kind::CanDown, vec![]),
            ("can_errors".into(), "CAN errors".into(), Unit::Count, true, 0.5, Kind::CanErrors, vec![]),
            ("can_peak".into(), "CAN peak".into(), Unit::Percent, true, 8.0, Kind::Can, vec![]),
            ("overruns".into(), "loop overruns".into(), Unit::Count, true, 3.0, Kind::Loop, vec![]),
            ("worst_loop_ms".into(), "worst loop".into(), Unit::Ms, true, 10.0, Kind::Loop, vec![]),
            ("code_errors".into(), "code exceptions".into(), Unit::Count, true, 0.5, Kind::CodeError, vec![]),
        ];
        // The motors that ran hottest in either.
        let mut motors: Vec<(String, f64)> = a.metrics.iter().chain(&b.metrics).filter_map(|(k, v)| k.strip_prefix("temp:").map(|n| (n.to_owned(), *v))).collect();
        motors.sort_by(|x, y| y.1.total_cmp(&x.1));
        let mut seen: Vec<String> = Vec::new();
        for (name, _) in motors {
            if seen.contains(&name) || seen.len() >= 2 {
                continue;
            }
            seen.push(name.clone());
            measures.push((format!("temp:{name}"), format!("{name} peak temperature"), Unit::Celsius, true, 5.0, Kind::Heat, vec![name]));
        }
        // Worse first, in the order the measures matter (a brownout before
        // the battery's low point), then better, then the same.
        let count = measures.len() as i32;
        for (k, (metric, words, unit, higher_worse, step, kind, names)) in measures.into_iter().enumerate() {
            let (Some(x), Some(y)) = (a.metric(&metric), b.metric(&metric)) else { continue };
            let delta = if higher_worse { x - y } else { y - x };
            let order = (count - k as i32) * 10;
            let (verdict, rank) = if delta >= step {
                ("worse", 300 + order)
            } else if delta <= -step {
                ("better", 100 + order)
            } else {
                ("about the same", 0)
            };
            if rank == 0 && unit == Unit::Count && x == 0.0 {
                continue;
            }
            let shown = |v: f64| if unit == Unit::Count && v == 0.0 { "none".to_owned() } else { Num::new(v, unit).shown() };
            let level = if verdict == "worse" { Level::Warn } else { Level::Fine };
            let body = format!("{words} {} vs {}, {verdict}", shown(x), shown(y));
            scored.push((rank, line(body, kind, level, Some(shown(x)), names)));
        }
        // The code each ran.
        match (&a.code, &b.code) {
            (Some(x), Some(y)) if x != y => scored.push((250, line(format!("the code changed: {x} vs {y}"), Kind::Code, Level::Warn, Some(x.clone()), vec![x.clone(), y.clone()]))),
            (Some(x), Some(_)) => scored.push((1, line(format!("the same code, {x}"), Kind::Code, Level::Fine, Some(x.clone()), vec![x.clone()]))),
            _ => {}
        }
        // Faults the newer has that the older had none of.
        for f in a.facts.iter().filter(|f| f.level != Level::Fine) {
            if !b.facts.iter().any(|g| g.kind == f.kind && g.level != Level::Fine) && !scored.iter().any(|(_, l)| l.kind == f.kind && l.level != Level::Fine && l.labels.len() == 1) {
                let mut l = Line::of(f, &a.label);
                l.body = format!("new since {}: {}", b.label, f.text);
                scored.push((280 + f.score as i32 / 10, l));
            }
        }
        scored.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
        let mut lines: Vec<Line> = scored.into_iter().map(|(_, l)| l).collect();
        // New code is what changed most often: never past the third line.
        if let Some(at) = lines.iter().position(|l| l.kind == Kind::Code && l.level == Level::Warn) {
            let code = lines.remove(at);
            lines.insert(at.min(2), code);
        }
        lines.truncate(most.max(2));
        lines
    }

    /// Which matches had something, newest first; or that none did.
    pub fn find(&self, topic: Topic, subject: Option<&str>, most: usize) -> Vec<Line> {
        let mut lines = Vec::new();
        let kinds: Vec<Kind> = match topic.happened() {
            Some((happened, ..)) => happened.to_vec(),
            None => topic.kinds().to_vec(),
        };
        let about = |f: &Fact| subject.is_none_or(|s| f.subject.eq_ignore_ascii_case(s) || f.names.iter().any(|n| n.eq_ignore_ascii_case(s)));
        // Each match's worst two (a second hot motor is worth saying), newest
        // match first, as many as there is room for.
        let mut matched = 0;
        let mut shown = 0;
        for m in &self.matches {
            let mut hits: Vec<&Fact> = m.facts.iter().filter(|f| (kinds.is_empty() || kinds.contains(&f.kind)) && f.level != Level::Fine && about(f)).collect();
            hits.sort_by_key(|f| std::cmp::Reverse(f.score));
            if hits.is_empty() {
                continue;
            }
            matched += 1;
            if lines.len() < most {
                shown += 1;
            }
            for f in hits.into_iter().take(2) {
                if lines.len() < most {
                    lines.push(Line::of(f, &m.label));
                }
            }
        }
        let span = match (self.matches.last(), self.matches.first()) {
            (Some(old), Some(new)) if self.matches.len() > 1 => format!("the {} matches logged ({} to {})", self.matches.len(), old.label, new.label),
            (Some(only), _) => format!("the one match logged ({})", only.label),
            _ => "any match logged".to_owned(),
        };
        if matched == 0 {
            let body = match topic.happened() {
                Some((_, _, none)) => format!("in {span}, {none}"),
                None => format!("nothing about {} stood out in {span}", subject.unwrap_or(topic.name())),
            };
            lines.push(Line::none(Vec::new(), topic.happened().map_or(Kind::Health, |h| h.1), body));
            // The nearest there is: the hottest motor, the lowest battery.
            if let Some(l) = self.extreme(topic, subject) {
                lines.push(l);
            }
        } else if matched > shown {
            lines.push(Line { labels: Vec::new(), body: format!("{matched} of {span} had it"), kind: kinds.first().copied().unwrap_or(Kind::Health), level: Level::Fine, negative: false, names: Vec::new(), key: Some(matched.to_string()) });
        }
        lines
    }

    /// The worst of a measure across the matches: the hottest motor seen, the
    /// lowest battery.
    fn extreme(&self, topic: Topic, subject: Option<&str>) -> Option<Line> {
        let (metric, words, unit, highest, kind) = match topic {
            Topic::Heat | Topic::Motors => (subject.map_or("temp_peak".to_owned(), |s| format!("temp:{s}")), "the hottest a motor got", Unit::Celsius, true, Kind::Heat),
            Topic::Brownout | Topic::Battery => ("battery_low".to_owned(), "the battery's lowest was", Unit::Volts, false, Kind::Battery),
            Topic::Can => ("can_peak".to_owned(), "the busiest CAN bus peaked at", Unit::Percent, true, Kind::Can),
            Topic::Loop => ("worst_loop_ms".to_owned(), "the worst loop was", Unit::Ms, true, Kind::Loop),
            _ => return None,
        };
        let (m, v) = self.matches.iter().filter_map(|m| Some((m, m.metric(&metric)?))).reduce(|x, y| if (y.1 > x.1) == highest { y } else { x })?;
        let who = match topic {
            Topic::Heat | Topic::Motors => m.facts.iter().find(|f| f.kind == Kind::Heat).map(|f| f.subject.clone()),
            _ => None,
        };
        let body = match &who {
            Some(name) => format!("{words} was {name} at {}", Num::new(v, unit).shown()),
            None => format!("{words} {}", Num::new(v, unit).shown()),
        };
        Some(Line { labels: vec![m.label.clone()], body, kind, level: Level::Fine, negative: false, names: who.into_iter().collect(), key: Some(Num::new(v, unit).shown()) })
    }

    /// A measure match by match, oldest first, and whether the newest is
    /// worse than those before it (as Logs › Trends judges it).
    pub fn trend(&self, topic: Topic, subject: Option<&str>) -> Vec<Line> {
        use crate::trends::Metric;
        let (metric, words, unit, judge, kind) = match topic {
            Topic::Heat | Topic::Motors => (subject.map_or("temp_peak".to_owned(), |s| format!("temp:{s}")), subject.map_or("the hottest motor's peak".to_owned(), |s| format!("{s}'s peak temperature")), Unit::Celsius, Some(Metric::Hottest), Kind::Heat),
            Topic::Loop => ("overruns".to_owned(), "loop overruns".to_owned(), Unit::Count, Some(Metric::Overruns), Kind::Loop),
            Topic::Can => ("can_peak".to_owned(), "the CAN peak".to_owned(), Unit::Percent, Some(Metric::Can), Kind::Can),
            Topic::Brownout => ("brownouts".to_owned(), "brownouts".to_owned(), Unit::Count, Some(Metric::Brownouts), Kind::Brownout),
            Topic::Current | Topic::Stall => (subject.map_or("current:Elevator".to_owned(), |s| format!("current:{s}")), format!("{}'s peak current", subject.unwrap_or("Elevator")), Unit::Amps, None, Kind::Current),
            _ => ("battery_low".to_owned(), "the battery's low point".to_owned(), Unit::Volts, Some(Metric::Battery), Kind::Battery),
        };
        let series: Vec<(&str, f64)> = self.matches.iter().rev().filter_map(|m| Some((m.label.as_str(), m.metric(&metric)?))).collect();
        if series.is_empty() {
            return vec![Line::none(Vec::new(), Kind::Health, format!("no match logged has {words}"))];
        }
        let shown: Vec<String> = series.iter().rev().take(8).rev().map(|(l, v)| format!("{l} {}", Num::new(*v, unit).shown())).collect();
        let mut lines = vec![Line { labels: Vec::new(), body: format!("{words} by match, oldest first: {}", shown.join(", ")), kind, level: Level::Fine, negative: false, names: subject.map(str::to_owned).into_iter().collect(), key: series.last().map(|(_, v)| Num::new(*v, unit).shown()) }];
        if let (Some(judge), Some(((latest, now), before))) = (judge, series.split_last()) {
            let scale = if judge == Metric::Can { 0.01 } else { 1.0 };
            let earlier: Vec<f64> = before.iter().rev().take(5).map(|(_, v)| v * scale).collect();
            let worse = judge.worse(now * scale, &earlier);
            let value = Num::new(*now, unit).shown();
            let before_it = if earlier.len() == 1 { "the match before it".to_owned() } else { format!("the {} matches before it", earlier.len()) };
            let body = if earlier.is_empty() {
                format!("only {latest} is logged to compare, at {value}")
            } else if worse {
                format!("{words} is worse in {latest}, at {value}, than in {before_it}")
            } else {
                format!("{words} in {latest}, {value}, is no worse than in {before_it}")
            };
            // The verdict first: it is what was asked.
            lines.insert(0, Line { labels: vec![(*latest).to_owned()], body, kind, level: if worse { Level::Warn } else { Level::Fine }, negative: !worse && !earlier.is_empty(), names: Vec::new(), key: Some(value) });
        }
        // And the worst of it, wherever it was.
        if series.len() > 1 {
            lines.extend(self.extreme(topic, subject));
        }
        lines
    }

    /// One value's range over a match, found by the words asked: "elevator
    /// temperature", "shooter velocity".
    pub fn signal(&self, i: usize, words: &[String]) -> Option<Line> {
        let m = &self.matches[i];
        let split = |key: &str| -> Vec<String> {
            let mut parts = Vec::new();
            let mut word = String::new();
            for c in key.chars() {
                if c == '/' || c == '_' || (c.is_ascii_uppercase() && !word.is_empty() && !word.ends_with(|p: char| p.is_ascii_uppercase())) {
                    if !word.is_empty() {
                        parts.push(word.to_lowercase());
                    }
                    word.clear();
                }
                if c != '/' && c != '_' {
                    word.push(c);
                }
            }
            if !word.is_empty() {
                parts.push(word.to_lowercase());
            }
            parts
        };
        fn alias(w: &str) -> &str {
            match w {
                "temp" | "temperature" | "hot" | "heat" => "temperature",
                "amps" | "amp" | "current" | "draw" => "current",
                "volts" | "voltage" | "battery" => "volts",
                "speed" | "velocity" | "rpm" | "rps" => "velocity",
                "position" | "height" => "position",
                "cpu" | "processor" => "cpu",
                other => other,
            }
        }
        let wanted: Vec<&str> = words.iter().map(|w| alias(w)).collect();
        let best = m
            .signals
            .iter()
            .map(|s| {
                let parts = split(&s.key);
                let score = wanted.iter().filter(|w| w.len() > 2 && parts.iter().any(|p| p.starts_with(*w) || (p.len() > 3 && w.starts_with(p.as_str())))).count();
                (score, s)
            })
            .filter(|(score, _)| *score > 0)
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.key.len().cmp(&a.1.key.len())))?
            .1;
        let unit = match best.key.as_str() {
            k if k.ends_with("TemperatureC") || k.ends_with("Temperature") || k.ends_with("TempCelsius") => Unit::Celsius,
            k if k.ends_with("Amps") || k.ends_with("Current") => Unit::Amps,
            k if k.ends_with("Volts") => Unit::Volts,
            k if k.contains("CanUtilization") || k.ends_with("Percent") => Unit::Percent,
            k if k.ends_with("Ms") => Unit::Ms,
            k if k.ends_with("Meters") => Unit::Meters,
            k if k.ends_with("RPS") => Unit::Rps,
            k if k.ends_with("Deg") || k.ends_with("Degrees") => Unit::Degrees,
            _ => Unit::Plain,
        };
        let kind = match unit {
            Unit::Celsius => Kind::Heat,
            Unit::Amps => Kind::Current,
            Unit::Volts => Kind::Battery,
            Unit::Percent if best.key.contains("Can") => Kind::Can,
            Unit::Ms => Kind::Loop,
            _ => Kind::Health,
        };
        let n = |v: f64| Num::new(v, unit).shown();
        let body = if best.boolean {
            format!("{} was true for {}, turning true {} times", best.key, super::facts::seconds(best.true_s), best.rises)
        } else {
            format!("{} peaked at {} at {} and was lowest at {}, averaging {}", best.key, n(best.max), clock(best.at_max), n(best.min), n(best.mean))
        };
        let key = if best.boolean { best.rises.to_string() } else { n(best.max) };
        Some(Line { labels: vec![m.label.clone()], body, kind, level: Level::Fine, negative: false, names: vec![best.key.clone()], key: Some(key) })
    }

    /// The matches logged, newest first, each with what stood out.
    pub fn list(&self, most: usize) -> Vec<Line> {
        if self.matches.is_empty() {
            return vec![Line::none(Vec::new(), Kind::Health, "no match logs are on this handheld yet".into())];
        }
        let mut lines: Vec<Line> = self
            .matches
            .iter()
            .take(most)
            .map(|m| match m.facts.iter().find(|f| f.level != Level::Fine) {
                Some(f) => Line::of(f, &m.label),
                None => Line::none(vec![m.label.clone()], Kind::Health, "nothing stood out".into()),
            })
            .collect();
        if self.matches.len() > most {
            lines.push(Line { labels: Vec::new(), body: format!("{} match logs in all", self.matches.len()), kind: Kind::Health, level: Level::Fine, negative: false, names: Vec::new(), key: None });
        }
        lines
    }

    /// A whole match: how it went, its faults and warnings, then the measures
    /// worth knowing, each a row.
    pub fn report(&self, i: usize) -> Report {
        let m = &self.matches[i];
        let mut rows: Vec<(Level, String)> = Vec::new();
        if let Some(result) = self.outcomes.get(&m.label) {
            rows.push((Level::Fine, capital(&format!("the team {result}"))));
        }
        for f in m.facts.iter().filter(|f| f.level != Level::Fine) {
            rows.push((f.level, capital(&f.text)));
        }
        let fine = [Kind::BatteryStart, Kind::Battery, Kind::Heat, Kind::Can, Kind::Loop, Kind::Auto, Kind::Code];
        for kind in fine {
            if let Some(f) = m.facts.iter().find(|f| f.kind == kind && f.level == Level::Fine) {
                rows.push((Level::Fine, capital(&f.text)));
            }
        }
        if rows.iter().all(|(l, _)| *l == Level::Fine) {
            rows.insert(0, (Level::Fine, "Nothing stood out: no fault or warning in its log".into()));
        }
        let title = self.titles.get(i).cloned().unwrap_or_default();
        Report { label: m.label.clone(), title, rows }
    }
}

/// The first letter in capitals.
pub fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What the robot is doing just now, in lines (it may be unplugged).
pub fn live_lines(live: &Live) -> Vec<Line> {
    let line = |body: String, kind: Kind, level: Level, key: Option<String>| Line { labels: Vec::new(), body, kind, level, negative: false, names: Vec::new(), key };
    if !live.linked {
        return vec![Line::none(Vec::new(), Kind::Comms, "the robot is not linked to this handheld just now".into())];
    }
    let mut lines = Vec::new();
    if live.browned_out {
        lines.push(line("the robot is browned out right now".into(), Kind::Brownout, Level::Fault, None));
    }
    for (severity, subsystem, id, detail) in live.firing.iter().take(3) {
        let what = if subsystem == "CAN" { format!("CAN {id}") } else { format!("{subsystem} {id}") };
        let detail = if detail.is_empty() { String::new() } else { format!(" ({detail})") };
        let level = if severity == "ERROR" { Level::Fault } else { Level::Warn };
        lines.push(Line { names: vec![subsystem.clone()], ..line(format!("{what} is firing now{detail}"), Kind::Health, level, Some(id.clone())) });
    }
    let mut now = Vec::new();
    if let Some(v) = live.battery {
        now.push(format!("battery {}", Num::new(v, Unit::Volts).shown()));
    }
    if let Some(ms) = live.loop_ms {
        now.push(format!("loop {}", Num::new(ms, Unit::Ms).shown()));
    }
    if let Some((bus, u)) = &live.can {
        now.push(format!("busiest CAN bus {bus} at {}", Num::new(u * 100.0, Unit::Percent).shown()));
    }
    if let Some((connected, expected)) = live.motors {
        now.push(format!("{connected} of {expected} motors connected"));
    }
    if !now.is_empty() {
        let key = live.battery.map(|v| Num::new(v, Unit::Volts).shown());
        lines.push(line(format!("the robot is linked: {}", join_and(&now)), Kind::Battery, Level::Fine, key));
    }
    if live.firing.is_empty() && !live.browned_out {
        lines.insert(0, Line::none(Vec::new(), Kind::Health, "no health check is firing".into()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::facts::analyse;
    use crate::offline::synth::{write, Dip, Spec};

    fn library() -> Library {
        let mut newer = Spec::quiet("FRC_20261116_101500_NYRO_Q31.wpilog", 31);
        newer.dips.push(Dip { at: 78.0, dur: 0.4, to: 6.4, load: vec![("FrontLeftDrive", 70.0)] });
        newer.motor("Elevator").peak_c = 82.0;
        newer.code = "9ab33c0d11";
        let older = Spec::quiet("FRC_20261116_093000_NYRO_Q28.wpilog", 28);
        let mut lib = Library::default();
        for spec in [newer, older] {
            let bytes = write(&spec);
            lib.matches.push(analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap());
        }
        lib.outcomes.insert("Q31".into(), "lost 87–112 on red".into());
        lib
    }

    #[test]
    fn a_topic_asked_of_a_match_without_it_is_said_not_to_be() {
        let lib = library();
        let q31 = lib.index_of("q31").unwrap();
        let q28 = lib.index_of("QM28").unwrap();
        let lines = lib.match_facts(q31, Topic::Brownout, None, 4);
        assert_eq!(lines[0].kind, Kind::Brownout);
        assert!(!lines[0].negative && lines[0].text().starts_with("Q31: the robot browned out once: at 1:18 in teleop (6.4 V for 0.4 s)"), "{}", lines[0].text());
        let lines = lib.match_facts(q28, Topic::Brownout, None, 4);
        assert!(lines[0].negative && lines[0].body == "the robot did not brown out", "{lines:?}");
        assert!(lines.iter().any(|l| l.kind == Kind::Battery), "with the battery's low point instead");
        assert_eq!(lib.outcome(q31).unwrap().text(), "Q31: the team lost 87–112 on red");
    }

    #[test]
    fn two_matches_compare_worst_first_with_the_code_that_changed() {
        let lib = library();
        let lines = lib.compare(0, 1, 5);
        let texts: Vec<String> = lines.iter().map(Line::text).collect();
        assert!(texts[0].starts_with("Q31 vs Q28: brownouts 1 vs none, worse"), "{texts:#?}");
        assert!(texts.iter().any(|t| t.contains("the code changed: 9ab33c0 vs 4f2c9e1")), "{texts:#?}");
        assert!(texts.iter().any(|t| t.contains("Elevator peak temperature 82 °C vs 52 °C, worse")), "{texts:#?}");
    }

    #[test]
    fn find_says_which_matches_had_it_or_that_none_did() {
        let lib = library();
        let hot = lib.find(Topic::Heat, None, 5);
        assert_eq!(hot[0].labels, ["Q31"]);
        let stalls = lib.find(Topic::Stall, None, 5);
        assert!(stalls[0].negative && stalls[0].body == "in the 2 matches logged (Q28 to Q31), no motor stalled", "{stalls:?}");
        let report = lib.report(0);
        assert_eq!(report.rows[0].1, "The team lost 87–112 on red");
        assert!(report.rows.iter().any(|(l, r)| *l == Level::Fault && r.starts_with("The robot browned out")));
        let signal = lib.signal(0, &["elevator".into(), "temperature".into()]).unwrap();
        assert!(signal.body.starts_with("Elevator/TemperatureC peaked at 82 °C"), "{}", signal.body);
        let trend = lib.trend(Topic::Battery, None);
        assert!(trend[0].body.starts_with("the battery's low point is worse in Q31, at 6.4 V, than in the match before it"), "the verdict first: {}", trend[0].body);
        assert!(trend[1].body.starts_with("the battery's low point by match, oldest first: Q28 "), "{}", trend[1].body);
    }
}
