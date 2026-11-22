//! Offline answers about the robot: from the match logs on this handheld
//! (pulled from the robot, kept after it is unplugged), the schedule kept
//! from the last time there was internet, and the robot itself if linked.
//!
//! A small model cannot be trusted to read a log or do sums, so it does
//! neither. The work is done here, the same way every time:
//!
//! 1. `facts` reads each log once into a ranked sheet of facts with their
//!    numbers and times (kept beside the log).
//! 2. `route` works out what is asked: rules first, the model (held by a
//!    grammar to fixed lists) only when the rules cannot tell.
//! 3. `tools` picks the facts that answer it: one match's, two compared,
//!    which matches had something, a trend, one value's range.
//! 4. `words` makes the answer from those facts; then, if there is a model,
//!    asks it to word them, and checks every number, time, match, name and
//!    fault it wrote against them. Anything it made up, and its answer is
//!    not shown: the one made from the facts is.
//!
//! The report of a match needs no model at all.

pub mod facts;
pub mod model;
pub mod route;
pub mod synth;
pub mod tools;
pub mod words;

#[cfg(test)]
pub mod meter;

#[cfg(not(target_arch = "aarch64"))]
pub mod eval;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use facts::Kind;
use model::Model;
use route::{Intent, Plan, Ref};
use tools::{Library, Line, Live, Report, Topic};

/// The most logs read for an answer: an event's qualifying, near enough.
const MATCHES: usize = 16;

/// An answer, and what it rests on.
#[derive(Clone, Debug)]
pub struct Answer {
    pub text: String,
    /// A whole match's report, shown as a card.
    pub report: Option<Report>,
    /// "from the log of Q14", "from 7 match logs".
    pub basis: String,
    /// The model whose wording is shown (it passed the check), if any.
    pub worded_by: Option<String>,
    /// The facts it rests on.
    pub lines: Vec<Line>,
    pub plan: Plan,
    /// What the model wrote and how the check found it.
    pub draft: Option<Draft>,
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub routed_by_model: bool,
}

/// (Read by the evaluation; the handheld keeps only its verdict.)
#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub struct Draft {
    pub text: String,
    pub verdict: words::Verdict,
    pub prompt_tokens: u32,
    /// Of the prompt, what the server read afresh (the rest it had cached).
    pub prompt_read: u32,
    pub answer_tokens: u32,
    pub seconds: f32,
}

/// What the thread working on a question says as it goes.
pub enum Event {
    /// A step done: "read Q14's log".
    Step(String),
    /// The model is wording the answer.
    Wording(String),
    /// The answer made from the facts, before the model words it: shown at
    /// once if the person will not wait.
    Ready(Box<Answer>),
    Done(Box<Answer>),
}

/// Where the logs come from.
#[derive(Clone, Debug)]
pub enum Logs {
    /// A folder of logs: their names and titles, newest first.
    Dir(PathBuf, Vec<(String, String)>),
    /// The demo robot's event.
    Demo,
}

/// What a question is answered from, gathered on the screen's thread.
#[derive(Clone, Debug)]
pub struct Source {
    pub logs: Logs,
    pub schedule: Option<crate::schedule::View>,
    pub live: Option<Live>,
    /// Set when nobody is waiting for the answer any more: reading stops.
    pub stop: Option<Arc<AtomicBool>>,
}

/// Whether a stop flag, if there is one, is set.
fn stopped(stop: &Option<Arc<AtomicBool>>) -> bool {
    stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed))
}

impl Source {
    /// The robot's logs (or those kept on this handheld with no robot set),
    /// its schedule, and how it is just now.
    pub fn of(robot: &crate::robot::Robot) -> Self {
        let rows = robot.logs.list();
        let logs = if robot.logs.is_demo() {
            Logs::Demo
        } else {
            let dir = robot.logs.dir().map(PathBuf::from).unwrap_or_else(crate::logs::kept_dir);
            let rows = if robot.logs.dir().is_some() { rows } else { crate::logs::rows_in(&dir) };
            Logs::Dir(dir, rows.iter().map(|r| (r.name.clone(), r.title())).collect())
        };
        Source { logs, schedule: robot.schedule.view(), live: robot.linked().then(|| live(robot)), stop: None }
    }
}

/// The robot just now, for a question about it.
fn live(robot: &crate::robot::Robot) -> Live {
    let can = robot.get("Systemcore/CanUtilization").and_then(|v| match v {
        nt4::Value::DoubleArray(a) => a.iter().copied().enumerate().max_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, u)| (format!("can_s{i}"), u)),
        nt4::Value::Double(u) => Some(("can_s0".to_owned(), u)),
        _ => None,
    });
    Live {
        linked: robot.linked(),
        battery: robot.f64("Systemcore/BatteryVolts"),
        browned_out: robot.bool("Systemcore/BrownedOut") == Some(true),
        loop_ms: robot.f64("Loop/Robot/LastMs"),
        can,
        motors: robot.i64("Devices/Motors/Connected").zip(robot.i64("Devices/Motors/Expected")),
        firing: robot.firing(),
    }
}

/// The demo robot's event as Logs lists it: each match's name, size and
/// summary (a few hundred bytes each, made once). Its sheets of facts are
/// not kept: each question reads the event again, as one does a real
/// robot's sheets from the card, and lets them go when answered.
pub fn demo_logs() -> &'static [(String, u64, crate::wpilog::Summary)] {
    static DEMO: OnceLock<Vec<(String, u64, crate::wpilog::Summary)>> = OnceLock::new();
    DEMO.get_or_init(|| {
        synth::demo_event()
            .iter()
            .map(|spec| {
                let bytes = synth::write(spec);
                let summary = crate::wpilog::summarize(&bytes[..]).unwrap_or_default();
                (spec.name.clone(), bytes.len() as u64, summary)
            })
            .collect()
    })
}

/// Read what a question is answered from: each log's sheet (worked out now,
/// once, for a log no question has needed before), how each match went, the
/// next match. Nothing of it is kept once the question is answered: the
/// sheets on the card are the index, and reading them again is cheap.
pub fn load(source: &Source, on: &mut dyn FnMut(Event)) -> Library {
    let mut lib = Library::default();
    match &source.logs {
        Logs::Demo => {
            for spec in synth::demo_event().iter().take(MATCHES) {
                if stopped(&source.stop) {
                    break;
                }
                let bytes = synth::write(spec);
                if let Ok(sheet) = facts::analyse(&bytes[..], &spec.name, bytes.len() as u64) {
                    lib.matches.push(sheet);
                    lib.titles.push(crate::logs::Row { name: spec.name.clone(), bytes: 0, digest: None }.title());
                }
            }
        }
        Logs::Dir(dir, rows) => {
            let started = Instant::now();
            let mut fresh = 0;
            for (name, title) in rows.iter().take(MATCHES) {
                if stopped(&source.stop) {
                    break;
                }
                if let Some((sheet, new)) = facts::for_log(dir, name) {
                    fresh += usize::from(new);
                    lib.matches.push(sheet);
                    lib.titles.push(title.clone());
                }
            }
            if fresh > 0 {
                let s = started.elapsed().as_secs_f32();
                on(Event::Step(if fresh == 1 { format!("read a new match log · {s:.1} s") } else { format!("read {fresh} new match logs · {s:.1} s") }));
            }
        }
    }
    // Labels are each match's own: a replayed one is told apart.
    let mut seen: Vec<String> = Vec::new();
    for m in &mut lib.matches {
        let base = m.label.clone();
        let mut n = 1;
        while seen.contains(&m.label) {
            n += 1;
            m.label = format!("{base}#{n}");
        }
        seen.push(m.label.clone());
    }
    if let Some(view) = &source.schedule {
        for m in &view.matches {
            if let (Some(result), Some(side)) = (m.result(view.team), m.alliance(view.team)) {
                let mut parts = result.splitn(2, ' ');
                let word = match parts.next() {
                    Some("W") => "won",
                    Some("L") => "lost",
                    _ => "tied",
                };
                lib.outcomes.insert(m.label.clone(), format!("{word} {} on {side}", parts.next().unwrap_or_default()));
            }
        }
        let now = chrono::Utc::now().timestamp();
        lib.next = view.next(now).map(|m| {
            let at = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
            let mut line = format!("the next match is {}", m.label);
            if let Some(t) = m.time {
                line.push_str(&format!(" at {}", at(t)));
            }
            if let Some(q) = m.queue_at {
                line.push_str(&format!(" (queue at {})", at(q)));
            }
            if let Some(side) = m.alliance(view.team) {
                let (ours, theirs) = if side == "red" { (&m.red, &m.blue) } else { (&m.blue, &m.red) };
                let partners: Vec<String> = ours.iter().filter(|t| **t != view.team).map(u32::to_string).collect();
                let against: Vec<String> = theirs.iter().map(u32::to_string).collect();
                line.push_str(&format!(", on {side} with {} against {}", facts::join_and(&partners), facts::join_and(&against)));
            }
            if let Some(q) = &view.now_queuing {
                line.push_str(&format!("; {q} is queuing now"));
            }
            line
        });
    }
    lib.live = source.live.clone();
    lib
}

/// What was asked before, for a follow-up.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub prior: Option<Plan>,
    /// Set when nobody is waiting for the answer any more: the model is not asked.
    pub stop: Option<Arc<AtomicBool>>,
}

/// The offline answer to a question.
pub fn ask(question: &str, ctx: &Context, lib: &Library, model: Option<&dyn Model>, on: &mut dyn FnMut(Event)) -> Answer {
    let tier = model.map(|m| model::tier(model::size_m(&m.name())));
    let mut plan = route::rules(question, lib, ctx.prior.as_ref());
    let mut routed_by_model = false;
    if !plan.sure && !stopped(&ctx.stop) {
        if let (Some(m), Some(t)) = (model, tier) {
            if t.route {
                if let Some(p) = model_route(m, question, lib) {
                    plan = p;
                    routed_by_model = true;
                }
            }
        }
    }
    if plan.intent == Intent::Chat || !plan.sure && !routed_by_model {
        return chat(question, plan, model, on);
    }
    let most = tier.map_or(3, |t| t.lines);
    let (lines, report, basis) = match execute(&plan, lib, most, on) {
        Ok(found) => found,
        Err(said) => return Answer { text: said, report: None, basis: "from the logs on this handheld".into(), worded_by: None, lines: Vec::new(), plan, draft: None, routed_by_model },
    };
    let text = if report.is_some() { String::new() } else { words::template(&plan, &lines) };
    let mut answer = Answer { text, report, basis, worded_by: None, lines, plan, draft: None, routed_by_model };
    if answer.report.is_none() && !answer.lines.is_empty() && !stopped(&ctx.stop) {
        if let (Some(m), Some(t)) = (model, tier) {
            on(Event::Ready(Box::new(answer.clone())));
            word(m, t, question, lib, &mut answer, on);
        }
    }
    answer
}

/// A match's report, with no model: the newest, or one by label.
pub fn report(lib: &Library, label: Option<&str>) -> Answer {
    let plan = Plan { intent: Intent::Report, matches: label.map(|l| vec![Ref::Label(l.to_owned())]).unwrap_or_default(), topic: Topic::Any, subject: None, words: Vec::new(), sure: true };
    match execute(&plan, lib, 4, &mut |_| {}) {
        Ok((lines, report, basis)) => Answer { text: String::new(), report, basis, worded_by: None, lines, plan, draft: None, routed_by_model: false },
        Err(said) => Answer { text: said, report: None, basis: "from the logs on this handheld".into(), worded_by: None, lines: Vec::new(), plan, draft: None, routed_by_model: false },
    }
}

/// The model's reading of a question it was asked to sort.
fn model_route(m: &dyn Model, question: &str, lib: &Library) -> Option<Plan> {
    let grammar = route::router_grammar(&lib.labels());
    let mut turns: Vec<(&str, &str)> = Vec::new();
    for (q, a) in route::ROUTER_EXAMPLES {
        turns.push(("user", q));
        turns.push(("assistant", a));
    }
    turns.push(("user", question));
    let body = model::body(route::ROUTER_SYSTEM, &turns, 16, Some(&grammar));
    let v = m.chat(&body, 30).ok()?;
    let (text, ..) = model::reply(&v).ok()?;
    route::from_model(text.trim(), lib, question)
}

/// The model asked to word the facts, its answer kept only if the check
/// finds nothing in it the facts do not say.
fn word(m: &dyn Model, tier: model::Tier, question: &str, lib: &Library, answer: &mut Answer, on: &mut dyn FnMut(Event)) {
    on(Event::Wording(m.name()));
    let prompt = words::writer_prompt(question, &answer.lines);
    let body = model::body(words::WRITER_SYSTEM, &[("user", words::WRITER_EXAMPLE.0), ("assistant", words::WRITER_EXAMPLE.1), ("user", &prompt)], tier.max_tokens, None);
    let started = Instant::now();
    // What the server had to read afresh: the instructions and example are
    // the same every time, and it keeps them from the last question.
    let mut processed = 0;
    let mut cut = false;
    let result = m.chat(&body, tier.seconds).and_then(|v| {
        processed = v["timings"]["prompt_n"].as_u64().unwrap_or(0) as u32;
        cut = v["choices"][0]["finish_reason"] == "length";
        model::reply(&v)
    });
    let seconds = started.elapsed().as_secs_f32();
    let draft = match result {
        Ok((text, prompt_tokens, answer_tokens)) => {
            let text = words::clean(&text);
            let mut verdict = words::check(&text, &answer.lines, question, &lib.proper_names());
            // Cut off by its room to answer: not a whole answer.
            if cut {
                verdict.ok = false;
                verdict.problems.insert(0, "ran out of room".into());
            }
            Draft { text, verdict, prompt_tokens, prompt_read: processed, answer_tokens, seconds }
        }
        Err(why) => Draft { text: String::new(), verdict: words::Verdict { ok: false, problems: vec![why], invented: false }, prompt_tokens: 0, prompt_read: 0, answer_tokens: 0, seconds },
    };
    if draft.verdict.ok {
        answer.text = draft.text.clone();
        answer.worded_by = Some(m.name());
    }
    answer.draft = Some(draft);
}

/// Not a question the match logs answer (a chat, or one the rules could not
/// place and no model sorted): the model's own answer, said to be that; with
/// no model, what offline answers are; and when the model could not answer,
/// why (not enough memory free, say), never the same few words as if there
/// were no model at all.
fn chat(question: &str, plan: Plan, model: Option<&dyn Model>, on: &mut dyn FnMut(Event)) -> Answer {
    let help = "Offline, I answer from the match logs on this handheld: ask what went wrong in a match, why it browned out, whether a motor ran hot, what CAN did, or to compare two matches. Ctrl R gives the newest match's report, and “brief me” a briefing.";
    let blank = |text: String, basis: &str| Answer { text, report: None, basis: basis.to_owned(), worded_by: None, lines: Vec::new(), plan: plan.clone(), draft: None, routed_by_model: false };
    let Some(m) = model else { return blank(help.into(), "offline") };
    on(Event::Wording(m.name()));
    // With the tiny version of what the person asked it to remember.
    let body = model::body(&crate::ai::local_system(), &[("user", question)], 160, None);
    match m.chat(&body, 90).and_then(|v| model::reply(&v)) {
        Ok((text, ..)) if !words::clean(&text).is_empty() => {
            let mut a = blank(words::clean(&text), &format!("{}'s own knowledge, not the logs", m.name().to_lowercase()));
            a.worded_by = Some(m.name());
            a
        }
        Ok(_) => blank(format!("{} gave no answer to that. Ask again, or ask about a match.", m.name()), "offline"),
        Err(why) => blank(format!("The on-device model could not answer: {why}."), "offline"),
    }
}

/// The matches a plan names, as indexes; or why one cannot be answered.
fn resolve(plan: &Plan, lib: &Library) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for r in &plan.matches {
        let i = match r {
            Ref::Latest => 0,
            Ref::Previous => 1,
            Ref::Oldest => lib.matches.len().saturating_sub(1),
            Ref::Label(label) => match lib.index_of(label) {
                Some(i) => i,
                None => {
                    let newest = lib.matches.first().map(|m| format!(" The newest log here is {}{}.", m.label, lib.titles.first().map(|t| format!(", {t}")).unwrap_or_default())).unwrap_or_default();
                    return Err(format!("There is no log of {label} on this handheld.{newest}"));
                }
            },
        };
        if i >= lib.matches.len() {
            return Err(match lib.matches.len() {
                0 => no_logs(),
                1 => format!("Only one match is logged here, {}: there is no match before it.", lib.matches[0].label),
                _ => "That match is not logged here.".into(),
            });
        }
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

fn no_logs() -> String {
    "No match logs are on this handheld yet. They come from the robot's Catalyst Agent over the cable after a match, and stay here unplugged.".into()
}

/// The facts a plan wants, from the tools.
fn execute(plan: &Plan, lib: &Library, most: usize, on: &mut dyn FnMut(Event)) -> Result<(Vec<Line>, Option<Report>, String), String> {
    let needs_logs = !matches!(plan.intent, Intent::Schedule | Intent::Now | Intent::Chat);
    if needs_logs && lib.matches.is_empty() {
        return Err(no_logs());
    }
    let picked = resolve(plan, lib)?;
    let label = |i: usize| lib.matches[i].label.clone();
    let one = |i: usize| format!("from the log of {}", label(i));
    let all = || format!("from {} match logs", lib.matches.len());
    let robot_topic = matches!(plan.topic, Topic::Brownout | Topic::Battery | Topic::Link | Topic::Comms | Topic::Estop | Topic::Loop | Topic::Can | Topic::Auto | Topic::Code | Topic::Outcome);
    let subject = if robot_topic { None } else { plan.subject.as_deref() };
    Ok(match plan.intent {
        Intent::Report => {
            let i = picked.first().copied().unwrap_or(0);
            on(Event::Step(format!("read {}'s log", label(i))));
            let report = lib.report(i);
            (lib.wrong(i, most), Some(report), one(i))
        }
        Intent::Why | Intent::Wrong | Intent::Stats => {
            let i = match picked.first() {
                Some(i) => *i,
                // "why did we brown out?": the newest match it happened in.
                None if plan.intent == Intent::Why && plan.topic != Topic::Any => {
                    let kinds = plan.topic.kinds();
                    match lib.matches.iter().position(|m| m.facts.iter().any(|f| kinds.contains(&f.kind) && f.level != crate::wpilog::Level::Fine)) {
                        Some(i) => i,
                        None => {
                            on(Event::Step(format!("looked through {} match logs", lib.matches.len())));
                            return Ok((lib.find(plan.topic, subject, most), None, all()));
                        }
                    }
                }
                None => 0,
            };
            on(Event::Step(format!("read {}'s log", label(i))));
            let mut lines = if plan.intent == Intent::Wrong && plan.topic == Topic::Any && subject.is_none() {
                lib.wrong(i, most)
            } else {
                let mut lines = lib.match_facts(i, plan.topic, subject, most + 2);
                prefer(&mut lines, &plan.words);
                lines.truncate(most);
                lines
            };
            if plan.intent == Intent::Stats || lines.iter().all(|l| l.negative) {
                let mut words = plan.words.clone();
                if let Some(s) = &plan.subject {
                    words.push(s.to_lowercase());
                }
                if let Some(signal) = lib.signal(i, &words).filter(|s| !lines.iter().any(|l| l.key == s.key)) {
                    if lines.len() >= most {
                        lines.pop();
                    }
                    lines.push(signal);
                }
            }
            // A second match named: its facts too.
            if let Some(&j) = picked.get(1) {
                lines.extend(lib.match_facts(j, plan.topic, subject, 2));
                (lines, None, format!("from the logs of {} and {}", label(i), label(j)))
            } else {
                (lines, None, one(i))
            }
        }
        Intent::Compare | Intent::Changed => {
            // "since Q12": the newest against Q12; "what changed in Q12": Q12
            // against the match before it.
            let since = plan.words.iter().any(|w| w == "since");
            let (newer, older) = match (picked.first(), picked.get(1)) {
                (Some(&a), Some(&b)) => (a.min(b), a.max(b)),
                (Some(&a), None) if since && a > 0 => (0, a),
                (Some(&a), None) => (a, a + 1),
                _ => (0, 1),
            };
            if older >= lib.matches.len() {
                return Err(format!("{} is the first match logged here: there is nothing before it to compare it with.", label(newer)));
            }
            on(Event::Step(format!("compared {} and {}", label(newer), label(older))));
            let mut lines = lib.compare(newer, older, most + 1);
            if plan.topic != Topic::Any {
                let kinds = plan.topic.kinds();
                lines.sort_by_key(|l| !kinds.contains(&l.kind));
            }
            (lines, None, format!("from the logs of {} and {}", label(newer), label(older)))
        }
        Intent::Find => {
            on(Event::Step(format!("looked through {} match logs", lib.matches.len())));
            (lib.find(plan.topic, subject, most), None, all())
        }
        Intent::Trend => {
            on(Event::Step(format!("looked through {} match logs", lib.matches.len())));
            (lib.trend(plan.topic, plan.subject.as_deref()), None, all())
        }
        Intent::List => (lib.list(most + 2), None, all()),
        Intent::Schedule => {
            on(Event::Step("looked at the schedule".into()));
            let line = match &lib.next {
                Some(next) => Line { labels: Vec::new(), body: next.clone(), kind: Kind::Outcome, level: crate::wpilog::Level::Fine, negative: false, names: Vec::new(), key: None },
                None => return Err("No schedule is kept on this handheld yet: it needs the team, a Blue Alliance key and internet once.".into()),
            };
            (vec![line], None, "from the schedule kept on this handheld".into())
        }
        Intent::Now => match &lib.live {
            Some(live) => {
                on(Event::Step("looked at the robot".into()));
                (tools::live_lines(live), None, "from the robot just now".into())
            }
            None if !lib.matches.is_empty() => {
                on(Event::Step(format!("read {}'s log", label(0))));
                let mut lines = vec![Line { labels: Vec::new(), body: "the robot is not linked just now; this is from its newest log".into(), kind: Kind::Comms, level: crate::wpilog::Level::Fine, negative: true, names: Vec::new(), key: None }];
                lines.extend(lib.wrong(0, most.saturating_sub(1).max(2)));
                (lines, None, one(0))
            }
            None => return Err("The robot is not linked, and no match logs are on this handheld yet.".into()),
        },
        Intent::Chat => (Vec::new(), None, "offline".into()),
    })
}

/// The facts a question's own words point at first: "started" the battery
/// at the start, "version" the code it ran.
fn prefer(lines: &mut [Line], words: &[String]) {
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let first: Option<&[Kind]> = if has(&["start", "started", "charged", "charge", "charging"]) {
        Some(&[Kind::BatteryStart])
    } else if has(&["low", "lowest", "min", "minimum"]) {
        Some(&[Kind::Battery, Kind::Sag])
    } else if has(&["version", "commit", "deployed", "deploy", "running", "ran", "on"]) && !has(&["exception", "exceptions", "crash", "errors", "error", "threw"]) {
        Some(&[Kind::Code])
    } else if has(&["peak", "max", "maximum", "most", "highest"]) && has(&["current", "amps", "amp", "amperage", "draw"]) {
        Some(&[Kind::Current])
    } else if has(&["utilisation", "utilization", "busy", "busiest", "percent", "load"]) {
        Some(&[Kind::CanBusy, Kind::Can])
    } else {
        None
    };
    if let Some(kinds) = first {
        lines.sort_by_key(|l| !kinds.contains(&l.kind));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Library {
        load(&Source { logs: Logs::Demo, schedule: Some(crate::schedule::Schedule::demo().view().unwrap()), live: None, stop: None }, &mut |_| {})
    }

    #[test]
    fn the_demo_event_answers_offline_from_its_logs() {
        let lib = demo();
        assert_eq!(lib.labels(), ["Q14", "Q12", "Q9", "Q7", "Q5", "Q3", "Q1"]);
        assert_eq!(lib.outcomes.get("Q14").map(String::as_str), Some("won 101–95 on blue"));
        let a = ask("why did we brown out in Q14?", &Context::default(), &lib, None, &mut |_| {});
        assert!(a.text.starts_with("Q14: the robot browned out once: at 1:18 in teleop (6.3 V for 0.4 s)."), "{}", a.text);
        assert!(a.text.contains("FrontLeftDrive 71 A"), "{}", a.text);
        assert_eq!(a.basis, "from the log of Q14");
        let a = ask("what changed since Q12?", &Context::default(), &lib, None, &mut |_| {});
        assert!(a.text.starts_with("Q14 against Q12: brownouts 1 vs none, worse"), "{}", a.text);
        assert!(a.text.contains("the code changed: 4f2c9e1 vs b71d2a0"), "{}", a.text);
        let a = ask("any motor running hot?", &Context::default(), &lib, None, &mut |_| {});
        assert!(a.text.starts_with("Q14: Elevator reached 81 °C"), "{}", a.text);
        let a = ask("anything wrong with Q99?", &Context::default(), &lib, None, &mut |_| {});
        assert_eq!(a.text, "There is no log of Q99 on this handheld. The newest log here is Q14, nyro q14 · nov 16 · 10:15.");
        let r = report(&lib, None);
        let card = r.report.unwrap();
        assert_eq!(card.label, "Q14");
        assert_eq!(card.rows[0].1, "The team won 101–95 on blue");
    }

    #[test]
    fn every_template_answer_passes_its_own_check() {
        let lib = demo();
        for q in ["why did we brown out in Q14?", "what went wrong last match?", "compare Q14 and Q12", "any motor running hot?", "how did auto go in Q9?", "is the battery getting worse?", "max elevator temperature in Q12", "which matches had brownouts?", "did Q9 brown out?"] {
            let a = ask(q, &Context::default(), &lib, None, &mut |_| {});
            let v = words::check(&a.text, &a.lines, q, &lib.proper_names());
            assert!(v.ok, "{q}: {}\n{:?}", a.text, v.problems);
        }
    }
}
