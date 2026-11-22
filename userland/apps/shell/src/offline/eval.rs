//! How well the offline assistant answers: an event's worth of match logs
//! made up with faults put in at known times, and the questions a pit crew
//! asks of them, each with what its answer must say.
//!
//! `catalyst-shell --offline-eval <out.json>` asks every question of the
//! model at `CATALYST_LOCAL_URL` (named by `CATALYST_EVAL_MODEL`) and of the
//! facts alone, and writes what each answered and how the check found it;
//! `scripts/offline-eval.sh run` does so for each model in turn. The tests
//! hold the answers made from the facts alone to every question.

use std::time::Instant;

use serde_json::{json, Value};

use super::facts::analyse;
use super::model::{self, Model};
use super::route::{self, Intent, Ref};
use super::synth::{write, Auto, Cause, Dip, Gap, Miss, Spec};
use super::tools::{Library, Topic};
use super::{ask, Answer, Context};

/// The event: practice, then eight qualifying matches, each with something
/// wrong put in where the questions look for it. Newest first.
pub fn event() -> Vec<Spec> {
    let name = |stamp: &str, label: &str| format!("FRC_202611{stamp}_NYLI_{label}.wpilog");
    let mut out = Vec::new();

    // Q36: e-stopped at 0:47, and never enabled again.
    let mut q36 = Spec::quiet(&name("21_154000", "Q36"), 36);
    q36.gaps.push(Gap { at: 47.0, dur: None, cause: Cause::Estop });
    q36.code = "9ab33c0d11";
    out.push(q36);

    // Q31: two brownouts, the drive and the elevator pulling hard; lost.
    let mut q31 = Spec::quiet(&name("21_143000", "Q31"), 31);
    q31.rest_v = 12.4;
    q31.dips.push(Dip { at: 78.0, dur: 0.4, to: 6.4, load: vec![("FrontLeftDrive", 70.0), ("Elevator", 55.0)] });
    q31.dips.push(Dip { at: 122.0, dur: 0.2, to: 6.7, load: vec![("FrontLeftDrive", 66.0)] });
    q31.motor("Shooter").peak_c = 61.0;
    q31.motor("Shooter").peak_c_at = 150.0;
    q31.motor("Elevator").peak_c = 58.0;
    q31.code = "9ab33c0d11";
    out.push(q31);

    // Q28: a battery not charged, a deep sag, new code, an exception.
    let mut q28 = Spec::quiet(&name("21_131500", "Q28"), 28);
    q28.rest_v = 12.1;
    q28.sag = 1.5;
    q28.dips.push(Dip { at: 111.0, dur: 0.6, to: 7.6, load: vec![("FrontLeftDrive", 62.0)] });
    q28.code = "9ab33c0d11";
    q28.errors.push((33.0, "Unhandled exception: java.lang.NullPointerException: Cannot invoke \"Pose2d.getX()\" at frc.robot.Vision.update(Vision.java:88)"));
    out.push(q28);

    // Q22: comms lost at 1:12 for 4.1 s; auto ran out of time.
    let mut q22 = Spec::quiet(&name("21_115000", "Q22"), 22);
    q22.gaps.push(Gap { at: 72.0, dur: Some(4.1), cause: Cause::Comms });
    q22.auto = Some(Auto { routine: "FourPiece", done_at: None, start_off: None });
    out.push(q22);

    // Q17: the elevator: stalled at 0:52, short of its setpoint at 1:44, 84 °C at 2:10.
    let mut q17 = Spec::quiet(&name("21_104000", "Q17"), 17);
    {
        let e = q17.motor("Elevator");
        e.peak_c = 84.0;
        e.peak_c_at = 130.0;
        e.stall = Some((52.0, 1.2, 58.0));
    }
    q17.misses.push(Miss { mechanism: "Elevator", at: 104.0, dur: 3.6, setpoint: 1.1, reached: 0.86 });
    out.push(q17);

    // Q12: CAN bus can_s1 at 94%, then down at 1:05 with 3 bus-offs; the shooter on it dropped off.
    let mut q12 = Spec::quiet(&name("20_163000", "Q12"), 12);
    q12.can[1] = 0.32;
    q12.can_spikes.push((63.0, 1.6, 1, 0.94));
    q12.can_down.push((65.0, 2.4, 1, 3));
    q12.motor("Shooter").bus = 1;
    q12.lost.push(("Shooter", 65.0, 2.0));
    out.push(q12);

    // Q8: the loop overrunning (worst 63 ms at 0:41), the processor flat out.
    let mut q8 = Spec::quiet(&name("20_143000", "Q8"), 8);
    q8.overruns = vec![(41.0, 63.0), (43.0, 27.0), (47.5, 24.0), (52.0, 31.0), (58.0, 22.5), (63.5, 26.0), (71.0, 23.0), (77.0, 29.0), (84.0, 21.5), (90.0, 25.0), (97.5, 22.0), (104.0, 28.0), (113.0, 24.5), (121.0, 23.5)];
    q8.cpu_high.push((40.0, 12.0, 97.0));
    out.push(q8);

    // Q3: a good match: auto done at 0:14, won.
    let mut q3 = Spec::quiet(&name("20_110000", "Q3"), 3);
    q3.auto = Some(Auto { routine: "TwoPiece", done_at: Some(14.0), start_off: None });
    q3.code = "4f2c9e1a7b";
    out.push(q3);

    // P2: practice, older code.
    let mut p2 = Spec::quiet(&name("20_093000", "P2"), 2);
    p2.code = "a1b2c3d4e5";
    p2.fms = Some((1, 2));
    out.push(p2);
    // The matches not given code of their own (Q22 back to Q8) ran
    // Spec::quiet's, 4f2c9e1; new code came before Q28.
    out
}

/// The event's logs read, with how the matches went.
pub fn library() -> Library {
    let mut lib = Library::default();
    for spec in event() {
        let bytes = write(&spec);
        lib.matches.push(analyse(&bytes[..], &spec.name, bytes.len() as u64).expect("the evaluation's logs read"));
        lib.titles.push(crate::logs::Row { name: spec.name.clone(), bytes: 0, digest: None }.title());
    }
    lib.outcomes.insert("Q31".into(), "lost 87–112 on red".into());
    lib.outcomes.insert("Q3".into(), "won 98–75 on blue".into());
    lib.outcomes.insert("Q28".into(), "won 104–99 on red".into());
    lib
}

/// One question: what its answer must say (each group, any one of its
/// words), what it must not, how it should be routed, and a question asked
/// before it, for a follow-up.
pub struct Case {
    pub q: &'static str,
    pub before: Option<&'static str>,
    pub want: &'static [&'static [&'static str]],
    pub not: &'static [&'static str],
    pub intent: Intent,
    pub topic: Topic,
    pub matches: &'static [&'static str],
}

const fn case(q: &'static str, want: &'static [&'static [&'static str]], intent: Intent, topic: Topic, matches: &'static [&'static str]) -> Case {
    Case { q, before: None, want, not: &[], intent, topic, matches }
}

pub const CASES: [Case; 40] = [
    case("why did we brown out in Q31?", &[&["Q31"], &["1:18"], &["6.4"]], Intent::Why, Topic::Brownout, &["Q31"]),
    case("what went wrong last match?", &[&["Q36"], &["e-stop", "estop", "emergency"], &["0:47"]], Intent::Wrong, Topic::Any, &["latest"]),
    case("compare Q31 and Q28", &[&["Q31"], &["Q28"], &["brownout"]], Intent::Compare, Topic::Any, &["Q31", "Q28"]),
    case("any motor running hot?", &[&["Elevator"], &["84"]], Intent::Find, Topic::Heat, &[]),
    case("what happened in Q22?", &[&["Q22"], &["comms", "connection", "driver station", "radio"], &["1:12"]], Intent::Wrong, Topic::Any, &["Q22"]),
    case("did the auto finish in Q22?", &[&["FourPiece"], &["still running", "not finish", "didn't finish", "did not"]], Intent::Why, Topic::Auto, &["Q22"]),
    case("why was the loop slow in Q8?", &[&["14"], &["63"]], Intent::Why, Topic::Loop, &["Q8"]),
    case("what happened to the CAN bus in Q12?", &[&["can_s1"], &["1:05", "1:04"]], Intent::Why, Topic::Can, &["Q12"]),
    Case { q: "which matches had brownouts?", before: None, want: &[&["Q31"]], not: &["Q28", "Q22", "Q36"], intent: Intent::Find, topic: Topic::Brownout, matches: &[] },
    case("how low did the battery get in Q28?", &[&["7.6"], &["1:51"]], Intent::Stats, Topic::Battery, &["Q28"]),
    case("did we brown out in Q28?", &[&["not", "no ", "didn't", "never"]], Intent::Why, Topic::Brownout, &["Q28"]),
    case("what changed since last match?", &[&["Q36"], &["Q31"]], Intent::Changed, Topic::Any, &["latest"]),
    case("max elevator temperature in Q17", &[&["84"]], Intent::Stats, Topic::Heat, &["Q17"]),
    case("was the elevator stalling in Q17?", &[&["0:52"], &["58"]], Intent::Why, Topic::Stall, &["Q17"]),
    case("did the elevator reach its setpoint in Q17?", &[&["1.10", "1.1 "], &["0.86"]], Intent::Why, Topic::Mechanism, &["Q17"]),
    case("is the battery getting worse over the event?", &[&["Q36"]], Intent::Trend, Topic::Battery, &[]),
    case("what code was on the robot in Q28?", &[&["9ab33c0"]], Intent::Why, Topic::Code, &["Q28"]),
    case("did the code change before Q28?", &[&["9ab33c0"], &["4f2c9e1"]], Intent::Changed, Topic::Code, &["Q28"]),
    case("any errors in the robot code?", &[&["NullPointerException"], &["Q28"]], Intent::Find, Topic::Code, &[]),
    case("how did auto go in Q3?", &[&["TwoPiece"], &["0:14"]], Intent::Why, Topic::Auto, &["Q3"]),
    case("report Q17", &[&["84"], &["0:52"]], Intent::Report, Topic::Any, &["Q17"]),
    case("what was the CAN utilisation in Q12?", &[&["94"]], Intent::Stats, Topic::Can, &["Q12"]),
    case("did we lose any motors?", &[&["Shooter"], &["Q12"]], Intent::Find, Topic::Motors, &[]),
    case("what went wrong in Q31?", &[&["brown"], &["1:18"]], Intent::Wrong, Topic::Any, &["Q31"]),
    case("why did the robot stop in Q36?", &[&["e-stop", "estop", "emergency"], &["0:47"]], Intent::Why, Topic::Link, &["Q36"]),
    case("was there a radio problem?", &[&["Q22"], &["1:12"]], Intent::Why, Topic::Comms, &[]),
    Case { q: "how was Q3?", before: None, want: &[&["Q3"]], not: &["brownout", "browned"], intent: Intent::Wrong, topic: Topic::Any, matches: &["Q3"] },
    case("compare Q17 and Q12", &[&["Q17"], &["Q12"]], Intent::Compare, Topic::Any, &["Q17", "Q12"]),
    case("how many brownouts in Q31?", &[&["2", "two", "twice"]], Intent::Why, Topic::Brownout, &["Q31"]),
    case("what's the worst loop time in Q8?", &[&["63"]], Intent::Why, Topic::Loop, &["Q8"]),
    case("did the battery start charged in Q28?", &[&["12.1"]], Intent::Why, Topic::Battery, &["Q28"]),
    case("tell me about the elevator in Q17", &[&["84", "58 A", "0:52", "1:44"]], Intent::Wrong, Topic::Any, &["Q17"]),
    case("how hot did the shooter get in Q31?", &[&["61"]], Intent::Stats, Topic::Heat, &["Q31"]),
    case("q12 problems", &[&["can_s1"]], Intent::Wrong, Topic::Any, &["Q12"]),
    case("anything wrong with Q99?", &[&["Q99"], &["no log"]], Intent::Wrong, Topic::Any, &["Q99"]),
    Case { q: "and Q28?", before: Some("why did we brown out in Q31?"), want: &[&["Q28"], &["not", "no ", "didn't", "never"]], not: &[], intent: Intent::Why, topic: Topic::Brownout, matches: &["Q28"] },
    case("what's the peak current on the elevator in Q31?", &[&["55"]], Intent::Stats, Topic::Current, &["Q31"]),
    case("any CAN problems this event?", &[&["Q12"], &["can_s1"]], Intent::Find, Topic::Can, &[]),
    case("did we win Q31?", &[&["lost", "87"]], Intent::Why, Topic::Outcome, &["Q31"]),
    case("how many loop overruns in Q8?", &[&["14"]], Intent::Why, Topic::Loop, &["Q8"]),
];

/// Whether a text says every group (one word of each) and none of `not`.
pub fn says(text: &str, want: &[&[&str]], not: &[&str]) -> bool {
    let lower = format!("{} ", text.to_lowercase());
    want.iter().all(|group| group.iter().any(|w| lower.contains(&w.to_lowercase()))) && !not.iter().any(|w| lower.contains(&w.to_lowercase()))
}

/// What an answer shows: its text, or its report's rows.
pub fn shown(a: &Answer) -> String {
    match &a.report {
        Some(r) => r.rows.iter().map(|(_, row)| row.as_str()).collect::<Vec<_>>().join(" "),
        None => a.text.clone(),
    }
}

fn routed_right(plan: &route::Plan, c: &Case, lib: &Library) -> bool {
    let want: Vec<Ref> = c.matches.iter().map(|m| if *m == "latest" { Ref::Latest } else { Ref::Label((*m).to_owned()) }).collect();
    // A match named either way it resolves to.
    let same = |a: &Ref, b: &Ref| a == b || matches!((a, b), (Ref::Latest, Ref::Label(l)) | (Ref::Label(l), Ref::Latest) if lib.index_of(l) == Some(0));
    let matches_ok = plan.matches.len() == want.len() && plan.matches.iter().zip(&want).all(|(a, b)| same(a, b));
    let intent_ok = plan.intent == c.intent || matches!((plan.intent, c.intent), (Intent::Why | Intent::Wrong | Intent::Stats, Intent::Why | Intent::Wrong | Intent::Stats));
    let topic_ok = c.topic == Topic::Any || plan.topic == c.topic || matches!((plan.topic, c.topic), (Topic::Link | Topic::Comms | Topic::Estop, Topic::Link | Topic::Comms | Topic::Estop) | (Topic::Brownout | Topic::Battery, Topic::Brownout | Topic::Battery));
    matches_ok && intent_ok && topic_ok
}

/// The same questions asked other ways, written after the rules and never
/// tuned against: how the routing does on words it was not written for.
pub const HELD_OUT: [Case; 26] = [
    case("what made the robot brown out during Q31", &[&["1:18"], &["6.4"]], Intent::Why, Topic::Brownout, &["Q31"]),
    case("anything bad happen in our most recent match?", &[&["Q36"], &["e-stop", "estop", "emergency"]], Intent::Wrong, Topic::Any, &["latest"]),
    case("Q31 versus Q28 please", &[&["Q31"], &["Q28"]], Intent::Compare, Topic::Any, &["Q31", "Q28"]),
    case("are any of the motors overheating", &[&["Elevator"], &["84"]], Intent::Find, Topic::Heat, &[]),
    case("tell me what happened during qualification 22", &[&["1:12"]], Intent::Wrong, Topic::Any, &["Q22"]),
    case("was autonomous successful in Q22", &[&["FourPiece"], &["still running", "not finish", "didn't finish", "did not"]], Intent::Why, Topic::Auto, &["Q22"]),
    case("how bad was the lag in Q8", &[&["63"]], Intent::Why, Topic::Loop, &["Q8"]),
    case("can bus issues in match 12?", &[&["can_s1"]], Intent::Why, Topic::Can, &["Q12"]),
    case("have we had any brownouts at all", &[&["Q31"]], Intent::Find, Topic::Brownout, &[]),
    case("lowest voltage in Q28", &[&["7.6"]], Intent::Stats, Topic::Battery, &["Q28"]),
    case("Q28 brownout?", &[&["not", "no ", "didn't", "never"]], Intent::Why, Topic::Brownout, &["Q28"]),
    case("differences between the last two matches", &[&["Q36"], &["Q31"]], Intent::Compare, Topic::Any, &["latest", "previous"]),
    case("how warm did the elevator get in qm17", &[&["84"]], Intent::Stats, Topic::Heat, &["Q17"]),
    case("did anything stall in Q17", &[&["0:52"]], Intent::Why, Topic::Stall, &["Q17"]),
    case("was the elevator able to get to its target in Q17", &[&["1.10", "1.1 "], &["0.86"]], Intent::Why, Topic::Mechanism, &["Q17"]),
    case("what software version was running in Q28", &[&["9ab33c0"]], Intent::Why, Topic::Code, &["Q28"]),
    case("did we deploy new code between Q22 and Q28", &[&["9ab33c0"], &["4f2c9e1"]], Intent::Compare, Topic::Code, &["Q28", "Q22"]),
    case("any exceptions thrown?", &[&["NullPointerException"]], Intent::Find, Topic::Code, &[]),
    case("how did the autonomous routine do in Q3", &[&["TwoPiece"]], Intent::Why, Topic::Auto, &["Q3"]),
    case("give me a full rundown of Q17", &[&["84"]], Intent::Report, Topic::Any, &["Q17"]),
    case("did any motors disconnect", &[&["Shooter"]], Intent::Find, Topic::Motors, &[]),
    case("did the radio drop out in any match", &[&["Q22"]], Intent::Find, Topic::Comms, &[]),
    case("were we e-stopped?", &[&["Q36"]], Intent::Find, Topic::Estop, &[]),
    case("how many times did the loop overrun in Q8", &[&["14"]], Intent::Why, Topic::Loop, &["Q8"]),
    case("what was the elevator's peak amperage in Q31", &[&["55"]], Intent::Stats, Topic::Current, &["Q31"]),
    case("Q12 summary", &[&["can_s1"]], Intent::Report, Topic::Any, &["Q12"]),
];

/// Ask every question of both sets; the results as JSON.
pub fn run(model: Option<&dyn Model>) -> Value {
    let lib = library();
    let name = model.map_or("facts alone".to_owned(), |m| m.name());
    json!({
        "model": name,
        "sizeM": model.map(|m| model::size_m(&m.name())),
        "main": run_set(&lib, &CASES, model),
        "heldOut": run_set(&lib, &HELD_OUT, model),
    })
}

/// Ask a set of questions.
fn run_set(lib: &Library, set: &[Case], model: Option<&dyn Model>) -> Value {

    let mut cases = Vec::new();
    let (mut finals, mut rules_ok, mut model_route_ok, mut model_routed) = (0, 0, 0, 0);
    let (mut drafts, mut used, mut correct_drafts, mut invented, mut missed, mut passed_but_short) = (0, 0, 0, 0, 0, 0);
    let (mut prompt_tokens, mut prompt_max, mut answer_tokens, mut seconds) = (0u64, 0u32, 0u64, 0.0f32);
    let mut prompt_read = 0u64;
    let mut route_prompt = 0u32;
    // The model asked to route every question first, held by the grammar:
    // how it would do if it routed everything. (All of them first, so the
    // answers after find the writer's instructions cached, as they would be.)
    let mut model_plans: Vec<Value> = vec![Value::Null; set.len()];
    if let Some(m) = model.filter(|m| model::tier(model::size_m(&m.name())).route || std::env::var_os("CATALYST_EVAL_ROUTE").is_some()) {
        let grammar = route::router_grammar(&lib.labels());
        for (k, c) in set.iter().enumerate() {
            let mut turns: Vec<(&str, &str)> = Vec::new();
            for (q, a) in route::ROUTER_EXAMPLES {
                turns.push(("user", q));
                turns.push(("assistant", a));
            }
            turns.push(("user", c.q));
            let body = model::body(route::ROUTER_SYSTEM, &turns, 16, Some(&grammar));
            if let Ok((text, prompt, _)) = m.chat(&body, 60).and_then(|v| model::reply(&v)) {
                route_prompt = route_prompt.max(prompt);
                let plan = route::from_model(text.trim(), lib, c.q);
                let right = plan.as_ref().is_some_and(|p| routed_right(p, c, lib));
                model_routed += 1;
                model_route_ok += usize::from(right);
                model_plans[k] = json!({"said": text.trim(), "right": right});
            }
        }
    }
    for (k, c) in set.iter().enumerate() {
        let mut ctx = Context::default();
        if let Some(before) = c.before {
            ctx.prior = Some(ask(before, &Context::default(), lib, None, &mut |_| {}).plan);
        }
        let rules = route::rules(c.q, lib, ctx.prior.as_ref());
        let rules_right = routed_right(&rules, c, lib);
        rules_ok += usize::from(rules_right);
        let model_plan = model_plans[k].clone();
        let started = Instant::now();
        let answer = ask(c.q, &ctx, &lib, model, &mut |_| {});
        let took = started.elapsed().as_secs_f32();
        let text = shown(&answer);
        let ok = says(&text, c.want, c.not);
        finals += usize::from(ok);
        // The answer made from the facts passes the check a model's must.
        let checks_itself = answer.report.is_some() || answer.lines.is_empty() || super::words::check(&super::words::template(&answer.plan, &answer.lines), &answer.lines, c.q, &lib.proper_names()).ok;
        let mut draft_json = Value::Null;
        if let Some(d) = &answer.draft {
            drafts += 1;
            let right = says(&d.text, c.want, c.not);
            correct_drafts += usize::from(right && d.verdict.ok);
            used += usize::from(d.verdict.ok);
            invented += usize::from(d.verdict.invented);
            missed += usize::from(!d.verdict.ok && !d.verdict.invented);
            passed_but_short += usize::from(d.verdict.ok && !right);
            prompt_tokens += u64::from(d.prompt_tokens);
            prompt_max = prompt_max.max(d.prompt_tokens);
            answer_tokens += u64::from(d.answer_tokens);
            seconds += d.seconds;
            prompt_read += u64::from(d.prompt_read);
            draft_json = json!({"text": d.text, "ok": d.verdict.ok, "invented": d.verdict.invented, "problems": d.verdict.problems, "says_what_it_must": right, "promptTokens": d.prompt_tokens, "promptRead": d.prompt_read, "answerTokens": d.answer_tokens, "seconds": d.seconds});
        }
        cases.push(json!({
            "q": c.q, "rulesRouted": rules_right, "modelRouted": model_plan,
            "intent": answer.plan.intent.word(), "topic": answer.plan.topic.name(),
            "facts": answer.lines.iter().map(|l| l.text()).collect::<Vec<_>>(),
            "answer": text, "basis": answer.basis, "wordedBy": answer.worded_by, "right": ok, "templateChecks": checks_itself, "draft": draft_json, "seconds": took,
        }));
    }
    let n = set.len();
    let pct = |a: usize, b: usize| if b == 0 { Value::Null } else { json!(((a as f64 / b as f64) * 1000.0).round() / 10.0) };
    json!({
        "questions": n,
        "answersRight": pct(finals, n),
        "rulesRouting": pct(rules_ok, n),
        "modelRouting": pct(model_route_ok, model_routed),
        "drafts": drafts,
        "draftsShown": pct(used, drafts),
        "draftsRightAndShown": pct(correct_drafts, drafts),
        "caughtMadeUp": pct(invented, drafts),
        "caughtMissingThePoint": pct(missed, drafts),
        "shownButIncomplete": passed_but_short,
        "promptTokensMean": if drafts == 0 { Value::Null } else { json!(prompt_tokens / drafts as u64) },
        "promptTokensMax": prompt_max,
        "promptReadMean": if drafts == 0 { Value::Null } else { json!(prompt_read / drafts as u64) },
        "routerPromptTokens": route_prompt,
        "answerTokensMean": if drafts == 0 { Value::Null } else { json!(answer_tokens / drafts as u64) },
        "secondsPerDraftOnThisPc": if drafts == 0 { Value::Null } else { json!(((seconds / drafts as f32) * 100.0).round() / 100.0) },
        "cases": cases,
    })
}

/// `--offline-eval <out.json>`: the evaluation against the model at
/// `CATALYST_LOCAL_URL`, or the facts alone.
pub fn main(out: &std::path::Path) {
    let model = model::find();
    let result = run(model.as_deref());
    std::fs::write(out, serde_json::to_string_pretty(&result).unwrap_or_default()).ok();
    for set in ["main", "heldOut"] {
        let r = &result[set];
        let get = |k: &str| r[k].to_string();
        println!(
            "{} [{set}]: answers right {}% · rules routing {}% · model routing {}% · drafts shown {}% · right and shown {}% · made up (caught) {}% · missed the point {}% · shown but short {} · prompt {} tokens (max {}, {} read afresh) · {} s a draft here",
            result["model"].as_str().unwrap_or_default(),
            get("answersRight"),
            get("rulesRouting"),
            get("modelRouting"),
            get("draftsShown"),
            get("draftsRightAndShown"),
            get("caughtMadeUp"),
            get("caughtMissingThePoint"),
            get("shownButIncomplete"),
            get("promptTokensMean"),
            get("promptTokensMax"),
            get("promptReadMean"),
            get("secondsPerDraftOnThisPc"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_question_is_answered_right_from_the_facts_alone() {
        let all = run(None);
        // The questions asked other ways: what they come to, said not hidden.
        let held = &all["heldOut"];
        let missed: Vec<String> = held["cases"].as_array().unwrap().iter().filter(|c| c["right"] != true).map(|c| format!("{} -> {}", c["q"], c["answer"])).collect();
        println!("held out: {}% right, rules routing {}%\n{}", held["answersRight"], held["rulesRouting"], missed.join("\n"));
        let result = &all["main"];
        let wrong: Vec<String> = result["cases"].as_array().unwrap().iter().filter(|c| c["right"] != true).map(|c| format!("{}\n  -> {}\n  facts: {}", c["q"], c["answer"], c["facts"])).collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        let lib = library();
        let unchecked: Vec<String> = result["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["templateChecks"] != true)
            .map(|c| {
                let q = c["q"].as_str().unwrap();
                let a = ask(q, &Context::default(), &lib, None, &mut |_| {});
                let v = super::super::words::check(&a.text, &a.lines, q, &lib.proper_names());
                format!("{q} -> {} {:?}", a.text, v.problems)
            })
            .collect();
        assert!(unchecked.is_empty(), "answers from the facts that fail their own check: {unchecked:#?}");
        let misrouted: Vec<String> = result["cases"].as_array().unwrap().iter().filter(|c| c["rulesRouted"] != true).map(|c| format!("{} ({} {})", c["q"], c["intent"], c["topic"])).collect();
        assert!(misrouted.is_empty(), "{misrouted:#?}");
    }
}
