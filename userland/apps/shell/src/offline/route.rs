//! Working out what a question asks: which tool, which matches, what topic.
//! Rules first; a pit question is short and its words say most of it. When
//! the rules cannot tell, a model big enough to be trusted with it is asked,
//! held by a grammar to four words from fixed lists (the matches among them
//! only those logged here), so it cannot name a match or a tool that is not.

use super::tools::{Library, Topic, MECHANISM_WORDS};

/// What is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// A whole match, no model needed.
    Report,
    /// Why something happened in a match.
    Why,
    /// What went wrong in a match.
    Wrong,
    Compare,
    /// What changed since the match before.
    Changed,
    Trend,
    /// Which matches had something.
    Find,
    /// One value's numbers.
    Stats,
    List,
    Schedule,
    /// The robot just now.
    Now,
    /// Anything else: general knowledge, for the model if there is one.
    Chat,
}

impl Intent {
    pub const ALL: [Intent; 12] = [Intent::Report, Intent::Why, Intent::Wrong, Intent::Compare, Intent::Changed, Intent::Trend, Intent::Find, Intent::Stats, Intent::List, Intent::Schedule, Intent::Now, Intent::Chat];

    /// Its word for the model's grammar.
    pub fn word(self) -> &'static str {
        match self {
            Intent::Report => "report",
            Intent::Why => "why",
            Intent::Wrong => "facts",
            Intent::Compare => "compare",
            Intent::Changed => "changed",
            Intent::Trend => "trend",
            Intent::Find => "find",
            Intent::Stats => "stats",
            Intent::List => "list",
            Intent::Schedule => "schedule",
            Intent::Now => "now",
            Intent::Chat => "chat",
        }
    }

    pub fn from_word(word: &str) -> Option<Intent> {
        Intent::ALL.into_iter().find(|i| i.word() == word)
    }
}

/// A match, as a question names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    Label(String),
    Latest,
    Previous,
    Oldest,
}

/// What to do for a question.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub intent: Intent,
    pub matches: Vec<Ref>,
    pub topic: Topic,
    /// A mechanism or motor named: "Elevator".
    pub subject: Option<String>,
    /// The question's own words, for finding a value by name.
    pub words: Vec<String>,
    /// The rules were sure of it.
    pub sure: bool,
}

/// A question's words: lower case, split at anything but letters, digits,
/// apostrophes and the hyphen of "e-stop".
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-' || c == '_'))
        .map(|w| w.trim_matches(|c| c == '\'' || c == '-').to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A match label from one word or two: "q31", "qm31", "match 31", "quals 31",
/// "p2", "practice 2", "sf2", "f1".
fn label_at(w: &[String], i: usize) -> Option<(String, usize)> {
    let word = w[i].as_str();
    let split = word.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = word.split_at(split);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && !letters.is_empty() {
        let letter = match letters {
            "q" | "qm" | "qual" | "quals" => "Q",
            "p" | "pm" => "P",
            "e" => "E",
            "sf" => "SF",
            "qf" => "QF",
            "f" => "F",
            "m" => "M",
            _ => return None,
        };
        return Some((format!("{letter}{}", digits.trim_start_matches('0')), 1));
    }
    None
}

fn label_pair(w: &[String], i: usize) -> Option<(String, usize)> {
    if let Some(found) = label_at(w, i) {
        return Some(found);
    }
    let next = w.get(i + 1)?;
    if !next.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let letter = match w[i].as_str() {
        "q" | "qm" | "qual" | "quals" | "qualification" | "match" | "matches" => "Q",
        "practice" | "p" => "P",
        "elim" | "playoff" | "e" => "E",
        "final" | "finals" | "f" => "F",
        "m" => "M",
        _ => return None,
    };
    Some((format!("{letter}{}", next.trim_start_matches('0')), 2))
}

fn has(w: &[String], word: &str) -> bool {
    w.iter().any(|x| x == word)
}

fn has_any(w: &[String], list: &[&str]) -> bool {
    list.iter().any(|x| has(w, x))
}

/// Whether the words run in this order anywhere: "last match".
fn phrase(w: &[String], p: &str) -> bool {
    let p: Vec<&str> = p.split(' ').collect();
    w.windows(p.len()).any(|win| win.iter().zip(&p).all(|(a, b)| a == b))
}

fn topic_of(w: &[String], text: &str) -> Topic {
    let any = |list: &[&str]| has_any(w, list);
    if any(&["brownout", "brownouts", "browned", "brown-out", "brown-outs"]) || phrase(w, "brown out") {
        Topic::Brownout
    } else if any(&["e-stop", "estop", "e-stopped", "estopped", "emergency"]) {
        Topic::Estop
    } else if any(&["comms", "communication", "communications", "radio", "connection", "ds", "fms"]) || phrase(w, "driver station") {
        Topic::Comms
    } else if any(&["disabled", "disable", "died", "dead", "stop", "stopped", "stops"]) || phrase(w, "cut out") {
        Topic::Link
    } else if any(&["stall", "stalled", "stalling", "stalls", "stuck", "jam", "jammed"]) {
        Topic::Stall
    } else if any(&["hot", "hotter", "hottest", "heat", "temperature", "temperatures", "temp", "temps", "overheat", "overheating", "overheated", "thermal", "warm"]) {
        Topic::Heat
    } else if any(&["battery", "batteries", "voltage", "volts", "volt", "sag", "sagged", "sagging", "power", "charged", "charge"]) {
        Topic::Battery
    } else if text.contains("CAN") || any(&["canbus", "can_s0", "can_s1", "can_s2", "can_s3", "can_s4", "bus-off", "busoff", "utilization", "utilisation"]) || phrase(w, "can bus") || phrase(w, "the can") || phrase(w, "can errors") || phrase(w, "can problems") || phrase(w, "bus off") {
        Topic::Can
    } else if any(&["loop", "loops", "overrun", "overruns", "overran", "lag", "laggy", "lagging", "cpu", "processor"]) || phrase(w, "loop time") {
        Topic::Loop
    } else if any(&["auto", "autonomous", "routine", "path"]) {
        Topic::Auto
    } else if any(&["exception", "exceptions", "crash", "crashed", "crashes", "code", "deploy", "deployed", "version", "commit", "errors", "error"]) {
        Topic::Code
    } else if any(&["current", "amps", "amp", "draw", "drawing", "drew"]) {
        Topic::Current
    } else if any(&["setpoint", "setpoints", "reach", "reached", "reaching", "position"]) {
        Topic::Mechanism
    } else if any(&["motor", "motors", "disconnect", "disconnected", "dropped", "dropout"]) {
        Topic::Motors
    } else if any(&["win", "won", "score", "scored", "result", "results", "outcome", "lose", "lost"]) {
        Topic::Outcome
    } else {
        Topic::Any
    }
}

/// The mechanism a question names, as the logs name it.
fn subject_of(w: &[String], names: &[String]) -> Option<String> {
    for name in names {
        let lower = name.to_lowercase();
        if w.iter().any(|x| *x == lower || (x.len() > 4 && lower.starts_with(x.as_str())) || lower.replace(' ', "") == *x) {
            return Some(name.clone());
        }
    }
    // A mechanism word the logs do not have (asked about all the same).
    MECHANISM_WORDS.iter().find(|m| has(w, m)).map(|m| super::tools::capital(m))
}

/// The rules' reading of a question; `prior` is the last question's plan,
/// for "and Q12?" and "what about the shooter?".
pub fn rules(question: &str, library: &Library, prior: Option<&Plan>) -> Plan {
    let w = words(question);
    let mut matches: Vec<Ref> = Vec::new();
    let mut i = 0;
    while i < w.len() {
        if let Some((label, used)) = label_pair(&w, i) {
            let r = Ref::Label(label);
            if !matches.contains(&r) {
                matches.push(r);
            }
            i += used;
        } else {
            i += 1;
        }
    }
    let last_match = ["last match", "latest match", "the latest", "newest", "most recent", "this match", "that match", "last game", "just now", "last one", "latest log", "last log"].iter().any(|p| phrase(&w, p)) || has(&w, "latest");
    let previous = ["previous match", "match before", "one before", "before that", "previous one", "the previous"].iter().any(|p| phrase(&w, p));
    let since_last = phrase(&w, "since last") || phrase(&w, "since the last") || phrase(&w, "from last") || phrase(&w, "since our last");
    if phrase(&w, "first match") {
        matches.push(Ref::Oldest);
    }
    if (last_match || since_last) && !matches.contains(&Ref::Latest) {
        matches.insert(0, Ref::Latest);
    }
    if previous && !matches.contains(&Ref::Previous) {
        matches.push(Ref::Previous);
    }
    if phrase(&w, "last two matches") || phrase(&w, "last 2 matches") {
        matches = vec![Ref::Latest, Ref::Previous];
    }
    let topic = topic_of(&w, question);
    let subject = subject_of(&w, &library.mechanisms());
    let any = |list: &[&str]| has_any(&w, list);
    let starts = |p: &str| question.trim().to_lowercase().starts_with(p);

    let general = (starts("what is") || starts("what's a") || starts("what are") || starts("what does") || starts("how does") || starts("how do i") || starts("define") || starts("explain what") || starts("tell me what"))
        && matches.is_empty()
        && !any(&["we", "our", "us", "did", "robot's", "match", "matches", "last", "latest", "now", "right"]);
    let intent = if any(&["report", "summary", "summarize", "summarise", "rundown", "recap", "debrief", "overview"]) {
        Intent::Report
    } else if any(&["compare", "comparison", "vs", "versus", "against", "differ", "difference", "between"]) || (matches.len() >= 2 && any(&["better", "worse"])) {
        Intent::Compare
    } else if any(&["changed", "change", "changes", "different", "new"]) && (since_last || has(&w, "since") || has(&w, "different") || matches.len() == 1) {
        Intent::Changed
    } else if any(&["next", "queue", "queuing", "queueing", "schedule", "alliance", "partners", "upcoming"]) && !any(&["log", "logs"]) {
        Intent::Schedule
    } else if general {
        Intent::Chat
    } else if any(&["trend", "trends", "trending", "lately", "worsening", "improving", "over"]) && (any(&["trend", "trends", "trending", "lately", "worsening", "improving"]) || phrase(&w, "over time") || phrase(&w, "over the event") || phrase(&w, "over the day"))
        || phrase(&w, "getting worse")
        || phrase(&w, "getting better")
        || phrase(&w, "each match")
        || phrase(&w, "every match")
    {
        Intent::Trend
    } else if (phrase(&w, "right now") || has(&w, "currently") || phrase(&w, "at the moment") || (has(&w, "now") && matches.is_empty()) || phrase(&w, "robot doing") || phrase(&w, "robot ok") || phrase(&w, "is the robot")) && matches.is_empty() {
        Intent::Now
    } else if phrase(&w, "which match") || phrase(&w, "which matches") || phrase(&w, "what matches") || phrase(&w, "when did we") || phrase(&w, "did we ever") || phrase(&w, "how many matches") || phrase(&w, "in which") || phrase(&w, "any match") || phrase(&w, "any matches") {
        Intent::Find
    } else if (any(&["list", "logs", "logged"]) && !any(&["why", "wrong"])) && topic == Topic::Any && matches.is_empty() {
        Intent::List
    } else if any(&["max", "maximum", "min", "minimum", "average", "mean", "peak", "highest", "lowest", "range"]) || phrase(&w, "how high") || phrase(&w, "how low") || phrase(&w, "how hot") || phrase(&w, "how much") || phrase(&w, "how fast") || phrase(&w, "what was the") {
        Intent::Stats
    } else if any(&["why", "cause", "caused", "causes", "reason", "explain"]) || phrase(&w, "how come") {
        Intent::Why
    } else if matches.is_empty() && topic != Topic::Any && (any(&["any", "anything", "ever", "were", "are", "have"]) || w.len() <= 3) {
        // "any motor running hot?", "brownouts?": across the matches.
        Intent::Find
    } else if topic != Topic::Any {
        Intent::Why
    } else {
        Intent::Wrong
    };
    // A follow-up: a match or a subject alone, the last question's plan kept.
    let short = w.len() <= 5 || starts("and ") || starts("what about") || starts("how about") || starts("same for");
    if let Some(prior) = prior {
        let only_ref = topic == Topic::Any && intent == Intent::Wrong && !any(&["wrong", "happened", "problems", "problem", "issues", "issue", "bad"]);
        if short && only_ref && (!matches.is_empty() || subject.is_some()) && prior.intent != Intent::Chat {
            let mut plan = prior.clone();
            if !matches.is_empty() {
                plan.matches = matches;
            }
            if subject.is_some() {
                plan.subject = subject;
            }
            plan.words = w;
            return plan;
        }
        // "it", "that match": the match last asked about.
        if matches.is_empty() && (has(&w, "it") || phrase(&w, "that match") || has(&w, "then")) && intent != Intent::Find && intent != Intent::Trend {
            matches = prior.matches.clone();
        }
    }
    let sure = intent != Intent::Wrong || !matches.is_empty() || topic != Topic::Any || any(&["wrong", "happened", "problems", "problem", "issues", "issue"]);
    Plan { intent, matches, topic, subject, words: w, sure }
}

// ------------------------------------------------------------- the model ---

/// The router's instructions: fixed, so llama-server keeps them cached
/// from one question to the next.
pub const ROUTER_SYSTEM: &str = "You sort questions about a robot's match logs. Reply with four words: tool, match, second match, topic.
tools: report (all of one match), facts (what happened in a match), why (the cause of something), compare (two matches), changed (since the match before), find (which matches had it), trend (over the event), stats (a value's numbers), list (the logs), schedule (next match), now (the robot right now), chat (not about the robot)
matches: latest, previous, none, or a match such as Q14
topics: any, brownout, battery, can, loop, heat, current, stall, link, comms, estop, auto, mechanism, code, motors, outcome";

/// Examples the router is shown, as turns before the question.
pub const ROUTER_EXAMPLES: [(&str, &str); 4] = [
    ("why did we brown out in Q7?", "why Q7 none brownout"),
    ("compare Q5 and Q3", "compare Q5 Q3 any"),
    ("any motor running hot?", "find none none heat"),
    ("what went wrong last match?", "facts latest none any"),
];

/// The grammar the router answers in: four words, from these lists only.
pub fn router_grammar(labels: &[String]) -> String {
    let quote = |s: &str| format!("\"{s}\"");
    let tools: Vec<String> = Intent::ALL.iter().map(|i| quote(i.word())).collect();
    let mut matches: Vec<String> = ["latest", "previous", "none"].iter().map(|s| quote(s)).collect();
    // The examples' matches too, so the examples are in the grammar's language.
    let mut all: Vec<String> = labels.to_vec();
    for l in ["Q7", "Q5", "Q3"] {
        if !all.iter().any(|x| x == l) {
            all.push(l.to_owned());
        }
    }
    matches.extend(all.iter().map(|l| quote(l)));
    let topics: Vec<String> = Topic::ALL.iter().map(|t| quote(t.name())).collect();
    format!("root ::= tool \" \" match \" \" match \" \" topic\ntool ::= {}\nmatch ::= {}\ntopic ::= {}\n", tools.join(" | "), matches.join(" | "), topics.join(" | "))
}

/// The model's four words as a plan, if they are four words of the lists
/// (a match only if it is logged here, or one of the plan's relative ones).
pub fn from_model(reply: &str, library: &Library, question: &str) -> Option<Plan> {
    let parts: Vec<&str> = reply.split_whitespace().collect();
    let [tool, a, b, topic] = parts[..] else { return None };
    let intent = Intent::from_word(tool)?;
    let topic = Topic::from_name(topic)?;
    let r = |word: &str| -> Option<Option<Ref>> {
        Some(match word {
            "none" => None,
            "latest" => Some(Ref::Latest),
            "previous" => Some(Ref::Previous),
            label => {
                library.index_of(label)?;
                Some(Ref::Label(label.to_owned()))
            }
        })
    };
    let matches: Vec<Ref> = [r(a)?, r(b)?].into_iter().flatten().collect();
    let w = words(question);
    let subject = subject_of(&w, &library.mechanisms());
    Some(Plan { intent, matches, topic, subject, words: w, sure: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        let mut lib = Library::default();
        let specs = [("FRC_20261116_101500_NYRO_Q31.wpilog", 31), ("FRC_20261116_093000_NYRO_Q28.wpilog", 28)];
        for (name, seed) in specs {
            let spec = crate::offline::synth::Spec::quiet(name, seed);
            let bytes = crate::offline::synth::write(&spec);
            lib.matches.push(crate::offline::facts::analyse(&bytes[..], name, bytes.len() as u64).unwrap());
        }
        lib
    }

    fn plan(q: &str) -> (Intent, Vec<Ref>, Topic) {
        let p = rules(q, &lib(), None);
        (p.intent, p.matches, p.topic)
    }

    #[test]
    fn pit_questions_route_by_their_words() {
        let q = |l: &str| Ref::Label(l.to_owned());
        assert_eq!(plan("why did we brown out in Q31?"), (Intent::Why, vec![q("Q31")], Topic::Brownout));
        assert_eq!(plan("what went wrong last match?"), (Intent::Wrong, vec![Ref::Latest], Topic::Any));
        assert_eq!(plan("compare Q31 and Q28"), (Intent::Compare, vec![q("Q31"), q("Q28")], Topic::Any));
        assert_eq!(plan("any motor running hot?"), (Intent::Find, vec![], Topic::Heat));
        assert_eq!(plan("match report qm 31").0, Intent::Report);
        assert_eq!(plan("what changed since last match?"), (Intent::Changed, vec![Ref::Latest], Topic::Any));
        assert_eq!(plan("which matches had brownouts?").0, Intent::Find);
        assert_eq!(plan("is the battery getting worse?").0, Intent::Trend);
        assert_eq!(plan("how low did the battery get in Q28?"), (Intent::Stats, vec![q("Q28")], Topic::Battery));
        assert_eq!(plan("did we lose comms in q31").2, Topic::Comms);
        assert_eq!(plan("why did the robot stop in Q31?").2, Topic::Link);
        assert_eq!(plan("what is a brownout?").0, Intent::Chat);
        assert_eq!(plan("when is our next match?").0, Intent::Schedule);
        assert_eq!(plan("how is the robot doing right now?").0, Intent::Now);
        assert_eq!(plan("we can't drive").2, Topic::Any, "\"can\" the word is not CAN");
        assert_eq!(plan("CAN problems in Q31?").2, Topic::Can);
        let p = rules("tell me about the elevator in Q31", &lib(), None);
        assert_eq!(p.subject.as_deref(), Some("Elevator"));
    }

    #[test]
    fn a_follow_up_keeps_the_question_before() {
        let library = lib();
        let first = rules("why did we brown out in Q31?", &library, None);
        let next = rules("and Q28?", &library, Some(&first));
        assert_eq!((next.intent, next.topic, next.matches.clone()), (Intent::Why, Topic::Brownout, vec![Ref::Label("Q28".into())]));
        let it = rules("was the elevator hot then?", &library, Some(&first));
        assert_eq!(it.matches, vec![Ref::Label("Q31".into())]);
    }

    #[test]
    fn the_model_router_is_held_to_its_lists() {
        let library = lib();
        let grammar = router_grammar(&library.labels());
        assert!(grammar.contains("\"Q31\"") && grammar.contains("\"heat\"") && grammar.contains("\"compare\""));
        let p = from_model("compare Q31 Q28 any", &library, "q").unwrap();
        assert_eq!(p.matches, vec![Ref::Label("Q31".into()), Ref::Label("Q28".into())]);
        assert!(from_model("compare Q99 none any", &library, "q").is_none(), "a match not logged here is refused");
        assert!(from_model("dance latest none any", &library, "q").is_none());
    }
}
