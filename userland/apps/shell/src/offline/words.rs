//! Putting facts into words: the answer made from the facts alone (always
//! right, a little stiff), the request asking a small model to word it, and
//! the check every word of the model's goes through before it is shown.
//! An answer with a number, a time, a match or a name the facts do not
//! have, a fault the facts do not mention, or a fault denied that the facts
//! have, is not shown: the answer made from the facts is.

use super::facts::Kind;
use super::route::{Intent, Plan};
use super::tools::{capital, Line, MECHANISM_WORDS};

// ------------------------------------------------------------ templates ---

/// The answer made from the facts alone: the first match's label, then
/// each fact as a sentence.
pub fn template(plan: &Plan, lines: &[Line]) -> String {
    if lines.is_empty() {
        return "Nothing in the logs on this handheld answers that.".into();
    }
    let mut out: Vec<String> = Vec::new();
    match plan.intent {
        Intent::Compare | Intent::Changed => {
            let pair = lines.iter().find(|l| l.labels.len() == 2).map(|l| l.labels.clone());
            let mut parts: Vec<String> = Vec::new();
            let mut after: Vec<String> = Vec::new();
            for l in lines {
                if l.labels.len() == 2 {
                    parts.push(l.body.clone());
                } else {
                    after.push(sentence(&l.text()));
                }
            }
            if let Some(pair) = pair {
                out.push(format!("{} against {}: {}.", pair[0], pair[1], parts.join("; ")));
            }
            out.extend(after);
        }
        _ => {
            let mut said_label: Option<String> = None;
            for l in lines {
                let text = match (l.labels.as_slice(), &said_label) {
                    ([one], Some(said)) if one == said => sentence(&l.body),
                    ([one], _) => {
                        said_label = Some(one.clone());
                        format!("{one}: {}", end(&l.body))
                    }
                    _ => sentence(&l.text()),
                };
                out.push(text);
            }
        }
    }
    out.join(" ")
}

/// A fact as a sentence of its own: capitalised, with a full stop.
fn sentence(text: &str) -> String {
    end(&capital(text))
}

fn end(text: &str) -> String {
    let t = text.trim();
    if t.ends_with(['.', '!', '?']) {
        t.to_owned()
    } else {
        format!("{t}.")
    }
}

// --------------------------------------------------------------- the ask ---

/// The writer's instructions, the same every time (cached by llama-server).
pub const WRITER_SYSTEM: &str = "You answer a robotics team's question about their robot from FACTS read from its match logs. Write one or two short, plain sentences. Use only the facts. Copy every number, time, match and name exactly as the facts write them. Do not add causes, advice, numbers or names that the facts do not give.";

/// One worked example, shown before the question.
pub const WRITER_EXAMPLE: (&str, &str) = (
    "FACTS:\n- Q7: the robot browned out once: at 1:12 in teleop (6.5 V for 0.3 s)\n- Q7: at the 1:12 brownout the robot was drawing Drive 61 A and Arm 40 A\n- Q7: the battery started the match at only 12.1 V, not fully charged\nQUESTION: why did Q7 brown out?",
    "Q7 browned out at 1:12 in teleop, the battery falling to 6.5 V for 0.3 s while Drive drew 61 A and Arm 40 A. The battery had started the match at only 12.1 V, not fully charged.",
);

/// The question with its facts, as the model is given them.
pub fn writer_prompt(question: &str, lines: &[Line]) -> String {
    let facts: Vec<String> = lines.iter().map(|l| format!("- {}", l.text())).collect();
    format!("FACTS:\n{}\nQUESTION: {}", facts.join("\n"), question.trim())
}

/// A model's answer, cleaned: any thinking it wrote taken out, quotes and
/// a leading "Answer:" dropped.
pub fn clean(reply: &str) -> String {
    let mut text = reply.to_owned();
    while let (Some(a), Some(b)) = (text.find("<think>"), text.find("</think>")) {
        if b < a {
            break;
        }
        text.replace_range(a..b + "</think>".len(), "");
    }
    let text = text.trim().trim_start_matches("Answer:").trim_start_matches("ANSWER:").trim();
    let text = text.trim_matches('"').trim();
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

// -------------------------------------------------------------- checking ---

/// Why a model's answer was not shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub ok: bool,
    /// Each problem, in a few words: "number 6.8 not in the facts".
    pub problems: Vec<String>,
    /// The problems that are made-up facts, rather than an answer that
    /// missed the point or ran long.
    pub invented: bool,
}

/// A number as written: its value, the unit after it, and whether it is a
/// time ("1:18").
#[derive(Clone, Debug, PartialEq)]
struct Said {
    value: f64,
    unit: Option<&'static str>,
    time: bool,
    raw: String,
}

fn unit_word(word: &str) -> Option<&'static str> {
    Some(match word.trim_matches(|c: char| c == '.' || c == ',' || c == ')' || c == '(').to_lowercase().as_str() {
        "v" | "volt" | "volts" => "V",
        "a" | "amp" | "amps" => "A",
        "°c" | "c" | "°" | "degrees" | "celsius" | "deg" => "C",
        "%" | "percent" => "%",
        "ms" | "milliseconds" => "ms",
        "s" | "sec" | "secs" | "second" | "seconds" => "s",
        "m" | "metres" | "meters" | "metre" | "meter" => "m",
        "rps" => "rps",
        _ => return None,
    })
}

/// Words a count is of: "14 times", "2 brownouts", "3 bus-offs".
fn counted(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-').to_lowercase();
    ["time", "times", "overrun", "overruns", "brownout", "brownouts", "bus-off", "bus-offs", "stall", "stalls", "match", "matches", "motor", "motors", "exception", "exceptions", "error", "errors", "check", "checks", "loops", "sags", "disables"].contains(&w.as_str())
}

/// Numbers written out. "One" and "once" are counted too: an answer that
/// says once where the facts say twice is wrong.
const SPELLED: [(&str, f64); 26] = [
    ("one", 1.0), ("once", 1.0), ("two", 2.0), ("three", 3.0), ("four", 4.0), ("five", 5.0), ("six", 6.0), ("seven", 7.0), ("eight", 8.0),
    ("nine", 9.0), ("ten", 10.0), ("eleven", 11.0), ("twelve", 12.0), ("thirteen", 13.0), ("fourteen", 14.0), ("fifteen", 15.0),
    ("sixteen", 16.0), ("seventeen", 17.0), ("eighteen", 18.0), ("nineteen", 19.0), ("twenty", 20.0), ("thirty", 30.0), ("dozen", 12.0),
    ("twice", 2.0), ("thrice", 3.0), ("half", 0.5),
];

/// The words of a text, keeping what numbers and names are written with.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '"' | '“' | '”' | '!' | '?'))
        .map(|w| w.trim_end_matches(['.', ':']).trim_start_matches('-').to_owned())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A match label: Q14, QM14, P2, E3, SF2, QF1, F1, M5.
fn is_label(word: &str) -> Option<String> {
    let upper = word.to_ascii_uppercase();
    let at = upper.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = upper.split_at(at);
    let letter = match letters {
        "Q" | "QM" => "Q",
        "P" | "E" | "SF" | "QF" | "F" | "M" => letters,
        _ => return None,
    };
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())).then(|| format!("{letter}{}", digits.trim_start_matches('0')))
}

/// The numbers a text says, with their units: every run of digits in a
/// word, whatever is before it ("~6.8", "≈6.8", "+3") and both ends of a
/// range ("5.1-5.3", "1:18-1:20"); not the digits of a name ("can_s1",
/// "Q14", "4f2c9e1") or an order ("2nd").
fn numbers(text: &str) -> Vec<Said> {
    let words = tokens(text);
    let mut out = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if let Some((_, v)) = SPELLED.iter().find(|(s, _)| word.eq_ignore_ascii_case(s)) {
            out.push(Said { value: *v, unit: None, time: false, raw: word.clone() });
            continue;
        }
        let chars: Vec<char> = word.chars().collect();
        let mut k = 0;
        while k < chars.len() {
            if !chars[k].is_ascii_digit() {
                k += 1;
                continue;
            }
            // A run of digits inside a name is part of the name.
            let inside_name = k > 0 && (chars[k - 1].is_alphabetic() || chars[k - 1] == '_');
            let start = k;
            while k < chars.len() && (chars[k].is_ascii_digit() || (chars[k] == '.' && chars.get(k + 1).is_some_and(char::is_ascii_digit))) {
                k += 1;
            }
            // A time: "1:18".
            let mut time = false;
            if chars.get(k) == Some(&':') && chars.get(k + 1).is_some_and(char::is_ascii_digit) && chars.get(k + 2).is_some_and(char::is_ascii_digit) && !chars.get(k + 3).is_some_and(char::is_ascii_digit) {
                time = true;
                k += 3;
            }
            let raw: String = chars[start..k].iter().collect();
            let after: String = chars[k..].iter().take_while(|c| c.is_alphabetic() || **c == '°' || **c == '%').collect();
            let lower_after = after.to_lowercase();
            if inside_name || matches!(lower_after.as_str(), "st" | "nd" | "rd" | "th") {
                k += after.chars().count();
                continue;
            }
            let value = if time {
                let (m, sec) = raw.split_once(':').unwrap_or(("0", "0"));
                m.parse::<f64>().unwrap_or(0.0) * 60.0 + sec.parse::<f64>().unwrap_or(0.0)
            } else {
                match raw.parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => continue,
                }
            };
            let unit = if time {
                Some("s")
            } else if after.starts_with('%') {
                Some("%")
            } else if !after.is_empty() {
                match unit_word(&after) {
                    Some(u) => Some(u),
                    // Letters run on: a name (4f2c9e1), not a number.
                    None => {
                        k += after.chars().count();
                        continue;
                    }
                }
            } else if k < chars.len() {
                // A range or a pair ("5.1-5.3 V"): the unit after it all.
                let tail: String = chars[k..].iter().collect();
                let unit_at_end: String = tail.chars().rev().take_while(|c| c.is_alphabetic() || *c == '°' || *c == '%').collect::<String>().chars().rev().collect();
                unit_word(&unit_at_end).or_else(|| words.get(i + 1).and_then(|next| unit_word(next)))
            } else {
                // A unit after it; or a count of something ("14 times", "20
                // loop overruns"), which is not the 20 of "20 ms".
                words.get(i + 1).and_then(|next| unit_word(next)).or_else(|| words.iter().skip(i + 1).take(2).any(|w| counted(w)).then_some("n"))
            };
            k += after.chars().count();
            out.push(Said { value, unit, time, raw });
        }
    }
    out
}

/// Words that say a kind of fault, and the kinds that allow them.
const CLAIMS: [(&[&str], &[Kind]); 9] = [
    (&["brownout", "brownouts", "browned", "brown-out"], &[Kind::Brownout]),
    (&["e-stop", "e-stopped", "estop", "estopped", "emergency"], &[Kind::Estop]),
    (&["comms", "radio", "communication", "communications"], &[Kind::Comms]),
    (&["stall", "stalled", "stalling", "stalls", "jammed", "stuck"], &[Kind::Stall]),
    (&["overheat", "overheated", "overheating", "hot", "hotter"], &[Kind::Heat]),
    (&["overran", "overrun", "overruns", "overrunning"], &[Kind::Loop]),
    (&["exception", "exceptions", "crashed", "crash", "threw"], &[Kind::CodeError]),
    (&["bus-off", "bus-offs"], &[Kind::CanErrors, Kind::CanDown]),
    (&["dropped", "disconnected", "lose", "lost"], &[Kind::MotorLost, Kind::Comms, Kind::CanDown]),
];

/// Things a model says that a fault of a kind contradicts: a mechanism
/// short of its setpoint did not reach it; an auto still running when auto
/// ended did not finish.
const CONTRADICTS: [(&[&str], Kind); 2] = [
    (&["reached its setpoint", "reached the setpoint", "reached setpoint", "reached its target", "hit its setpoint", "made its setpoint", "at its setpoint", "got to its setpoint"], Kind::Mechanism),
    (&["finished", "completed", "complete", "on time"], Kind::Auto),
];

/// Causes and conclusions: the facts never draw one, so neither may an answer.
const CAUSES: [&str; 19] = ["because", "due", "caused", "causing", "causes", "cause", "led", "leading", "result", "resulted", "resulting", "therefore", "hence", "indicating", "indicates", "indicate", "suggesting", "suggests", "means"];

/// Kinds of fact that are events: an answer about one says when.
const EVENTS: [Kind; 10] = [Kind::Brownout, Kind::Sag, Kind::Estop, Kind::Comms, Kind::Disabled, Kind::Stall, Kind::CanDown, Kind::MotorLost, Kind::CodeError, Kind::Mechanism];

/// What a stretch of text is about, for holding a time to its event.
fn topics(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let words = tokens(&lower);
    let has = |list: &[&str]| words.iter().any(|w| list.iter().any(|l| w.starts_with(l)));
    let mut out = Vec::new();
    if has(&["brown", "battery", "volt", "sag"]) {
        out.push("power");
    }
    if has(&["hot", "temperature", "heat", "overheat"]) || lower.contains("°c") {
        out.push("heat");
    }
    if has(&["stall", "stuck", "jam"]) {
        out.push("stall");
    }
    if has(&["setpoint", "short"]) {
        out.push("setpoint");
    }
    if has(&["comms", "radio", "communication"]) || lower.contains("driver station") {
        out.push("comms");
    }
    if has(&["e-stop", "estop", "emergency"]) {
        out.push("estop");
    }
    if has(&["disabled"]) {
        out.push("disabled");
    }
    if has(&["loop", "overran", "overrun"]) {
        out.push("loop");
    }
    if has(&["exception", "threw", "crash"]) {
        out.push("code");
    }
    if text.contains("CAN") || has(&["can_s", "bus-off", "bus"]) {
        out.push("can");
    }
    if has(&["dropped", "disconnect"]) {
        out.push("lost");
    }
    if has(&["auto"]) {
        out.push("auto");
    }
    if has(&["drew", "drawing", "draw", "amps", "current"]) {
        out.push("current");
    }
    out
}

/// A text in its stretches: at stops, commas, brackets, and the words that
/// join one thing said to the next.
fn clauses(text: &str) -> Vec<String> {
    let mut t = format!(" {text} ");
    for joint in [", ", "; ", ". ", ": ", "(", ")", " and ", " while ", " when ", " but ", " after ", " before ", " then "] {
        t = t.replace(joint, "|");
    }
    t.split('|').map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect()
}

const NEGATORS: [&str; 11] = ["no", "not", "never", "didn't", "wasn't", "without", "none", "nothing", "neither", "nor", "zero"];

/// Whether a word is denied by one of the few before it.
fn denied(words: &[String], i: usize) -> bool {
    let from = i.saturating_sub(4);
    words[from..i].iter().any(|w| NEGATORS.contains(&w.as_str()) || w.ends_with("n't"))
}

/// Check a model's answer against the facts it was given. `question` may
/// name matches and numbers of its own; `names` is every name the logs on
/// this handheld have (a model naming one its facts do not is caught).
pub fn check(answer: &str, lines: &[Line], question: &str, names: &[String]) -> Verdict {
    let mut problems: Vec<String> = Vec::new();
    let mut invented = false;
    let facts_text: String = lines.iter().map(Line::text).collect::<Vec<_>>().join("\n");
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Verdict { ok: false, problems: vec!["no answer".into()], invented: false };
    }
    if trimmed.contains("FACTS") || trimmed.contains("QUESTION") || trimmed.contains(['{', '}', '<', '>', '#', '*']) {
        problems.push("not plain sentences".into());
    }
    // Words, numbers and their punctuation; much else is not an answer.
    let odd = trimmed.chars().filter(|c| !(c.is_alphanumeric() || c.is_whitespace() || ".,:;()'’%°-–—/\"".contains(*c))).count();
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if odd * 10 > trimmed.chars().count() || letters * 4 < trimmed.chars().count() {
        problems.push("not plain sentences".into());
    }
    // Whole sentences, not the facts pasted in as they are written.
    if !trimmed.trim_end_matches(['"', '\'', '”', ')']).ends_with(['.', '!', '?']) {
        problems.push("does not end a sentence".into());
    }
    for l in lines.iter().filter(|l| l.labels.len() == 2) {
        if trimmed.contains(&format!("{} vs {}:", l.labels[0], l.labels[1])) {
            problems.push("copies the facts as they are written".into());
            break;
        }
    }
    // Sentences end at a stop followed by a space or the end (not "6.4").
    let sentences = trimmed.char_indices().filter(|(i, c)| matches!(c, '.' | '!' | '?') && trimmed[i + 1..].chars().next().is_none_or(char::is_whitespace)).count();
    if trimmed.chars().count() > 420 || sentences > 5 {
        problems.push("too long for the screen".into());
    }

    // Numbers and times: each in the facts (or the question), in its unit.
    let allowed: Vec<Said> = numbers(&facts_text).into_iter().chain(numbers(question)).collect();
    for said in numbers(trimmed) {
        let same = |a: &Said| (a.value - said.value).abs() < 1e-6 && a.time == said.time;
        let found: Vec<&Said> = allowed.iter().filter(|a| same(a)).collect();
        let spelled = SPELLED.iter().any(|(s, _)| said.raw.eq_ignore_ascii_case(s));
        if found.is_empty() {
            // A count spelled out may also be written so in the facts ("once").
            if !(spelled && tokens(&facts_text.to_lowercase()).iter().any(|w| *w == said.raw.to_lowercase())) {
                problems.push(format!("{} is not in the facts", said.raw));
                invented = true;
            }
            continue;
        }
        if let Some(unit) = said.unit {
            let units: Vec<Option<&str>> = found.iter().map(|a| a.unit).collect();
            if !units.iter().any(|u| u.is_none_or(|u| u == unit)) {
                problems.push(format!("{} has the wrong unit", said.raw));
                invented = true;
            }
        }
    }

    // Matches: only those the facts or the question name.
    let fact_labels: Vec<String> = lines.iter().flat_map(|l| l.labels.iter().cloned()).chain(tokens(&facts_text).iter().chain(&tokens(question)).filter_map(|w| is_label(w))).collect();
    let words = tokens(trimmed);
    for w in &words {
        if let Some(label) = is_label(w.trim_end_matches("'s").trim_end_matches("’s")) {
            if !fact_labels.iter().any(|l| l.eq_ignore_ascii_case(&label)) {
                problems.push(format!("{label} is not in the facts"));
                invented = true;
            }
        }
    }

    // Names: a motor, mechanism, bus or code the logs have, said only if
    // its facts say it; and identifiers (can_s3, 4f2c9e1) only as written.
    let facts_lower = facts_text.to_lowercase();
    let fact_words: Vec<String> = tokens(&facts_lower);
    let lower_words: Vec<String> = words.iter().map(|w| w.to_lowercase().trim_end_matches("'s").to_owned()).collect();
    for name in names.iter().map(|n| n.to_lowercase()).chain(MECHANISM_WORDS.iter().map(|m| m.to_string())) {
        if name.len() < 3 {
            continue;
        }
        let said = if name.contains(' ') { trimmed.to_lowercase().contains(&name) } else { lower_words.iter().any(|w| *w == name) };
        if said && !facts_lower.contains(&name) && !question.to_lowercase().contains(&name) {
            problems.push(format!("{name} is not in the facts"));
            invented = true;
        }
    }
    for w in &lower_words {
        let ordinal = ["st", "nd", "rd", "th"].iter().any(|s| w.ends_with(s)) && w.len() > 2 && w[..w.len() - 2].chars().all(|c| c.is_ascii_digit());
        let mixed = w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_alphabetic()) && is_label(w).is_none() && !ordinal;
        let numberish = unit_word(w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')).is_some() && w.starts_with(|c: char| c.is_ascii_digit());
        if mixed && !numberish && !fact_words.iter().any(|f| f == w) && !facts_lower.contains(w.as_str()) {
            problems.push(format!("{w} is not in the facts"));
            invented = true;
        }
    }

    // Faults: one said only if a fact is of its kind or says it; one the
    // facts have is not denied; one the facts deny is not claimed.
    for (said_words, kinds) in CLAIMS {
        let real = lines.iter().any(|l| !l.negative && kinds.contains(&l.kind) && l.level != crate::wpilog::Level::Fine);
        // Said by a fact in so many words, and not denied there ("never overran").
        let worded = lines.iter().any(|l| {
            let body: Vec<String> = tokens(&l.body.to_lowercase());
            body.iter().enumerate().any(|(j, x)| said_words.contains(&x.as_str()) && !denied(&body, j))
        });
        let positive = real || worded;
        let negative = lines.iter().any(|l| l.negative && kinds.contains(&l.kind));
        for (i, w) in lower_words.iter().enumerate() {
            if !said_words.contains(&w.as_str()) && !(w == "brown" && lower_words.get(i + 1).is_some_and(|n| n == "out")) {
                continue;
            }
            let no = denied(&lower_words, i);
            if !no && !positive {
                problems.push(format!("says {w}, which the facts do not"));
                invented = true;
            } else if !no && negative && !positive {
                problems.push(format!("says {w}, which the facts deny"));
                invented = true;
            } else if no && real {
                problems.push(format!("denies {w}, which the facts have"));
                invented = true;
            }
        }
    }

    let answer_lower = trimmed.to_lowercase();
    for (phrases, kind) in CONTRADICTS {
        if !lines.iter().any(|l| l.kind == kind && l.level != crate::wpilog::Level::Fine && !l.negative) {
            continue;
        }
        for phrase in phrases {
            if let Some(at) = answer_lower.find(phrase) {
                let before = tokens(&answer_lower[..at]);
                if !denied(&before, before.len()) && !facts_lower.contains(phrase) {
                    problems.push(format!("says '{phrase}', which the facts contradict"));
                    invented = true;
                }
            }
        }
    }
    for cause in CAUSES {
        if lower_words.iter().any(|w| w == cause) && !fact_words.iter().any(|w| w == cause) {
            problems.push(format!("gives a cause ('{cause}') the facts do not"));
            invented = true;
        }
    }

    // A time is said with its own event: "short of its setpoint at 0:52"
    // when 0:52 was a stall is not what the facts say; and a measurement
    // with its own motor: the Shooter's 61 °C is not the Elevator's.
    let line_texts: Vec<String> = lines.iter().map(Line::text).collect();
    for clause in clauses(trimmed) {
        let about = topics(&clause);
        if !about.is_empty() {
            for said in numbers(&clause).into_iter().filter(|n| n.time) {
                let with: Vec<&String> = line_texts.iter().filter(|t| tokens(t).iter().any(|w| *w == said.raw)).collect();
                if !with.is_empty() && !with.iter().any(|t| topics(t).iter().any(|k| about.contains(k))) {
                    problems.push(format!("puts {} with the wrong event", said.raw));
                    invented = true;
                }
            }
        }
        let clause_words: Vec<String> = tokens(&clause.to_lowercase()).iter().map(|w| w.trim_end_matches("'s").to_owned()).collect();
        let named: Vec<String> = names.iter().map(|n| n.to_lowercase()).filter(|n| n.len() > 2 && clause_words.iter().any(|w| w == n)).collect();
        if named.is_empty() {
            continue;
        }
        for said in numbers(&clause).into_iter().filter(|n| !n.time && matches!(n.unit, Some("C" | "A" | "V" | "%"))) {
            let with: Vec<String> = line_texts.iter().filter(|t| numbers(t).iter().any(|x| (x.value - said.value).abs() < 1e-6 && !x.time)).map(|t| t.to_lowercase()).collect();
            let any_named = with.iter().any(|t| names.iter().any(|n| n.len() > 2 && t.contains(&n.to_lowercase())));
            if any_named && !with.iter().any(|t| named.iter().any(|n| t.contains(n.as_str()))) {
                problems.push(format!("gives {} to the wrong motor", said.raw));
                invented = true;
            }
        }
    }

    // The point of it: the first fact's match, its number (and for an
    // event, its time), its motor, and what happened; or (for a fact that
    // something did not happen) a no.
    if let Some(first) = lines.first() {
        for label in &first.labels {
            if !words.iter().any(|w| is_label(w.trim_end_matches("'s")).is_some_and(|l| l.eq_ignore_ascii_case(label))) {
                problems.push(format!("leaves out {label}"));
            }
        }
        if first.negative {
            if !lower_words.iter().enumerate().any(|(i, w)| NEGATORS.contains(&w.as_str()) || w.ends_with("n't") || denied(&lower_words, i)) {
                problems.push("does not say it did not happen".into());
            }
        } else {
            let said = numbers(trimmed);
            let mut needed: Vec<String> = first.key.iter().cloned().collect();
            if EVENTS.contains(&first.kind) {
                if let Some(time) = numbers(&first.body).into_iter().find(|n| n.time) {
                    if !needed.contains(&time.raw) {
                        needed.push(time.raw);
                    }
                }
            }
            // Asked how many, or a loop overrunning (how often is what
            // matters as much as the worst): the count.
            let asked = question.to_lowercase();
            let loop_fault = first.kind == Kind::Loop && first.level != crate::wpilog::Level::Fine;
            if asked.contains("how many") || asked.contains("how often") || loop_fault {
                if let Some(count) = numbers(&first.body).into_iter().find(|n| n.unit == Some("n")) {
                    let spelled = SPELLED.iter().filter(|(_, v)| (*v - count.value).abs() < 1e-6).any(|(s, _)| lower_words.iter().any(|w| w == s));
                    if !spelled && !needed.contains(&count.raw) {
                        needed.push(count.raw);
                    }
                }
            }
            for key in needed {
                let key_numbers = numbers(&key);
                let carried = if key_numbers.is_empty() {
                    answer_lower.contains(&key.to_lowercase())
                } else {
                    key_numbers.iter().all(|k| said.iter().any(|s| (s.value - k.value).abs() < 1e-6 && s.time == k.time))
                };
                if !carried {
                    problems.push(format!("leaves out {key}"));
                }
            }
            // Its motor or bus, by name.
            if let Some(subject) = first.names.first().filter(|n| names.iter().any(|x| x.eq_ignore_ascii_case(n))) {
                if !answer_lower.contains(&subject.to_lowercase()) {
                    problems.push(format!("leaves out {subject}"));
                }
            }
            // And what happened, in a word for it: a brownout said as one.
            if let Some((said_words, _)) = CLAIMS.iter().find(|(_, kinds)| kinds.first() == Some(&first.kind)) {
                let says_it = lower_words.iter().any(|w| w.starts_with("brown") || said_words.iter().any(|s| w == s || w.ends_with(s)));
                if first.level != crate::wpilog::Level::Fine && EVENTS.contains(&first.kind) && !says_it {
                    problems.push(format!("does not say what happened ({})", said_words[0]));
                }
            }
        }
    }
    problems.dedup();
    Verdict { ok: problems.is_empty(), problems, invented }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wpilog::Level;

    fn line(label: &str, body: &str, kind: Kind, key: &str) -> Line {
        Line { labels: vec![label.into()], body: body.into(), kind, level: Level::Fault, negative: false, names: vec!["FrontLeftDrive".into()], key: Some(key.into()) }
    }

    fn brownout() -> Vec<Line> {
        vec![
            line("Q31", "the robot browned out 2 times: at 1:18 in teleop (6.4 V for 0.4 s) and at 2:02 (6.7 V for 0.2 s)", Kind::Brownout, "6.4 V"),
            line("Q31", "at the 1:18 brownout the robot was drawing FrontLeftDrive 70 A and Elevator 55 A", Kind::Brownout, "70 A"),
        ]
    }

    fn names() -> Vec<String> {
        vec!["FrontLeftDrive".into(), "Elevator".into(), "Shooter".into(), "can_s1".into()]
    }

    #[test]
    fn a_faithful_answer_passes() {
        let v = check("Q31 browned out twice, first at 1:18 in teleop when the battery fell to 6.4 V for 0.4 s, while FrontLeftDrive drew 70 A.", &brownout(), "why did we brown out in Q31?", &names());
        assert!(v.ok, "{:?}", v.problems);
    }

    #[test]
    fn made_up_numbers_matches_names_and_faults_are_caught() {
        let q = "why did we brown out in Q31?";
        let cases = [
            ("Q31 browned out at 1:18 when the battery fell to 6.8 V.", "6.8 is not in the facts"),
            ("Q31 browned out at 1:20 when the battery fell to 6.4 V.", "1:20 is not in the facts"),
            ("Q31 browned out at 1:18 when the battery fell to 6.4 A.", "6.4 has the wrong unit"),
            ("Q28 browned out at 1:18 at 6.4 V.", "Q28 is not in the facts"),
            ("Q31 browned out at 1:18 at 6.4 V while the Shooter drew 70 A.", "shooter is not in the facts"),
            ("Q31 browned out at 1:18 at 6.4 V after can_s1 went down.", "can_s1 is not in the facts"),
            ("Q31 browned out at 1:18 at 6.4 V; the elevator stalled.", "says stalled, which the facts do not"),
            ("Q31 browned out at 1:18 when the battery fell to ~6.8 V.", "6.8 is not in the facts"),
            ("Q31 browned out at 1:18 between 6.4-6.9 V.", "6.9 is not in the facts"),
            ("Q31 browned out twelve times, first at 1:18 at 6.4 V.", "twelve is not in the facts"),
            ("Q31 did not brown out; the battery held at 6.4 V.", "denies brown"),
            ("The robot was fine.", "leaves out 6.4 V"),
        ];
        for (answer, problem) in cases {
            let v = check(answer, &brownout(), q, &names());
            assert!(!v.ok, "{answer}");
            assert!(v.problems.iter().any(|p| p.starts_with(problem)), "{answer}: {:?}", v.problems);
        }
        // Cut off, or the facts pasted in as written.
        let pair = vec![Line { labels: vec!["Q14".into(), "Q12".into()], body: "brownouts 1 vs none, worse".into(), kind: Kind::Brownout, level: Level::Warn, negative: false, names: vec![], key: Some("1".into()) }];
        let v = check("Q14 vs Q12: brownouts 1 vs none, worse Q14 vs Q12: the difference between Q14 and Q", &pair, "compare Q14 and Q12", &names());
        assert!(v.problems.contains(&"does not end a sentence".to_owned()) && v.problems.contains(&"copies the facts as they are written".to_owned()), "{:?}", v.problems);
        assert!(check("Q14 browned out 1 time and Q12 did not, so Q14 was worse.", &pair, "compare Q14 and Q12", &names()).problems.iter().all(|p| !p.starts_with("copies")));
        // A count is a count: not the 63 of "63 ms".
        let lines = vec![Line { names: vec!["loop".into()], ..line("Q8", "the loop overran its budget 14 times, worst 63 ms at 0:41 in teleop", Kind::Loop, "63 ms") }];
        let v = check("Q8 had 63 loop overruns, the worst 63 ms at 0:41 in teleop.", &lines, "how many overruns in Q8?", &names());
        assert!(v.problems.iter().any(|p| p.starts_with("63 has the wrong unit")), "{:?}", v.problems);
        assert!(check("Q8's loop overran 14 times, the worst 63 ms at 0:41 in teleop.", &lines, "how many overruns in Q8?", &names()).ok);
        // A fact that it did not happen: the answer must say no, and not claim it.
        let none = vec![Line { labels: vec!["Q28".into()], body: "the robot did not brown out".into(), kind: Kind::Brownout, level: Level::Fine, negative: true, names: vec![], key: None }];
        assert!(check("No, Q28 did not brown out.", &none, "did Q28 brown out?", &names()).ok);
        let v = check("Q28 browned out.", &none, "did Q28 brown out?", &names());
        assert!(!v.ok && v.invented, "{:?}", v.problems);
    }

    #[test]
    fn the_template_reads_as_sentences() {
        let plan = Plan { intent: Intent::Why, matches: vec![], topic: super::super::tools::Topic::Brownout, subject: None, words: vec![], sure: true };
        assert_eq!(
            template(&plan, &brownout()),
            "Q31: the robot browned out 2 times: at 1:18 in teleop (6.4 V for 0.4 s) and at 2:02 (6.7 V for 0.2 s). At the 1:18 brownout the robot was drawing FrontLeftDrive 70 A and Elevator 55 A."
        );
        // And passes its own check.
        assert!(check(&template(&plan, &brownout()), &brownout(), "why?", &names()).ok);
        assert_eq!(clean("<think>\nhmm\n</think>\n\nAnswer: \"Q31 browned out.\""), "Q31 browned out.");
    }
}
