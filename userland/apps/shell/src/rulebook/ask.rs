//! Ask: a question in plain words answered from the manual. The search
//! finds the rules (retrieval); a model reads only those and answers in a
//! few sentences, citing them by ID. Claude answers when its key is set and
//! the handheld is online, through the assistant's own request code; the
//! on-device model otherwise, given as much of the rules as its context
//! holds. With neither, the rules found are the answer. The rules are always
//! shown under the answer, to check it against.

use serde_json::Value;

use super::parse;
use super::store::{self, Book};

/// Rules (or sections) found for a question, for Claude and on this handheld.
pub const PASSAGES_ONLINE: usize = 6;
/// Characters of one rule Claude is given at most.
const PASSAGE_CHARS: usize = 3000;
/// The longest answer from Claude, in tokens (thinking included).
const CLAUDE_TOKENS: u32 = 2000;
/// The on-device model's answer, in tokens.
pub const LOCAL_ANSWER: u32 = 120;
/// Room in the context for its chat template's own tokens.
const TEMPLATE: usize = 48;

const SYSTEM: &str = "You answer questions about the 2026 FIRST Robotics Competition game manual (REBUILT) for a team at an event. Answer only from the rules given below, which a search of the manual found; do not use anything else you know about FRC. Cite every rule you rely on by its ID in square brackets, like [G101] (a section by its number, like [6.4]). Keep it short: at most four sentences, plain text, no Markdown. If the rules given do not settle the question, say so and name the rule that comes closest. Never invent a rule ID.";

const LOCAL_SYSTEM: &str = "Answer from these FRC rules only, in one or two short sentences. Name the rule you used in brackets, like [G101]. If they do not answer it, say so.";

/// A rule or section given to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct Passage {
    pub entry: usize,
    /// "[G101]", or "[6.4]" for a section.
    pub cite: String,
    pub title: String,
    pub text: String,
}

/// The entries a question is answered from: the rules it names, then what
/// the search ranks best, at most `n`.
pub fn retrieve(book: &Book, question: &str, n: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for (a, b) in store::spans(question) {
        if let Some(i) = book.find_id(&question[a..b]) {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    // A question is whole: its last word is not matched as a prefix.
    for hit in book.search(&format!("{} ", question.trim()), n * 2).hits {
        if out.len() >= n {
            break;
        }
        // A section that is only a heading (its rules follow it) says nothing to answer from.
        let empty = !book.items[hit.entry].is_rule() && book.body(hit.entry).trim().is_empty();
        if !empty && !out.contains(&hit.entry) {
            out.push(hit.entry);
        }
    }
    out.truncate(n);
    out
}

pub fn passages(book: &Book, entries: &[usize]) -> Vec<Passage> {
    entries
        .iter()
        .map(|&i| {
            let item = &book.items[i];
            Passage { entry: i, cite: format!("[{}]", item.id), title: item.title.clone(), text: parse::plain(&book.body(i)) }
        })
        .collect()
}

/// Tokens a text takes, estimated on the generous side (about three
/// characters a token; the manual's capitals take more than prose).
pub fn tokens(text: &str) -> usize {
    (text.chars().count() * 10).div_ceil(28) + 1
}

/// At most `chars` characters of a text, ended at a sentence (or a word)
/// where it is cut.
fn cut(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let end = text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i);
    let head = &text[..end];
    let at = head.rfind(". ").map(|i| i + 1).filter(|&i| i > end / 2).or_else(|| head.rfind(' ')).unwrap_or(end);
    format!("{}…", head[..at].trim_end())
}

fn rules_block(passages: &[Passage], chars: &[usize]) -> String {
    let mut out = String::new();
    for (p, &room) in passages.iter().zip(chars) {
        let head = format!("{} {}", p.cite, p.title);
        out.push_str(&head);
        if room > 0 && !p.text.is_empty() {
            out.push('\n');
            out.push_str(&cut(&p.text, room));
        }
        out.push_str("\n\n");
    }
    out
}

/// Claude's request: the rules found, whole (up to a few pages), and the question.
pub fn claude_prompt(passages: &[Passage], question: &str) -> (String, String) {
    let chars: Vec<usize> = passages.iter().map(|_| PASSAGE_CHARS).collect();
    (SYSTEM.to_owned(), format!("Rules found in the manual:\n\n{}Question: {}", rules_block(passages, &chars), question.trim()))
}

/// The on-device model's request, within its context (`context` tokens,
/// `answer` of them kept for the answer): as many of the rules found as fit,
/// best first, each cut short to share the room; a rule that does not fit
/// even by its headline is left out.
pub fn local_prompt(passages: &[Passage], question: &str, context: u32, answer: u32) -> (String, String) {
    let ask = format!("Question: {}", question.trim());
    let fixed = TEMPLATE + tokens(LOCAL_SYSTEM) + tokens("Rules:\n\n") + tokens(&ask) + answer as usize;
    let mut room = (context as usize).saturating_sub(fixed);
    let mut kept: Vec<Passage> = Vec::new();
    let mut chars: Vec<usize> = Vec::new();
    for (k, p) in passages.iter().enumerate() {
        let head = tokens(&format!("{} {}\n\n", p.cite, p.title));
        if head > room {
            break;
        }
        room -= head;
        // An even share of what is left among this rule and those after it.
        let share = room / (passages.len() - k);
        let whole = p.text.chars().count();
        let mut c = if tokens(&p.text) + 1 <= share {
            whole
        } else {
            // Cut to its share; a share too small to say anything is not worth its line.
            let c = share.saturating_sub(3) * 28 / 10;
            if c < 60 {
                0
            } else {
                c
            }
        };
        let mut used = if c == 0 { 0 } else { tokens(&cut(&p.text, c)) + 1 };
        if used > room {
            (c, used) = (0, 0);
        }
        room -= used;
        kept.push(p.clone());
        chars.push(c);
    }
    (LOCAL_SYSTEM.to_owned(), format!("Rules:\n\n{}{ask}", rules_block(&kept, &chars)))
}

/// The rules an answer cites, by entry, in the order it names them: IDs
/// in brackets or out, and sections by their number in brackets. Only
/// those in the manual: an ID it made up is not shown.
pub fn cited(book: &Book, answer: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut push = |i: usize| {
        if !out.contains(&i) {
            out.push(i);
        }
    };
    let bytes = answer.as_bytes();
    for (a, b) in store::spans(answer) {
        if let Some(i) = book.find_id(&answer[a..b]) {
            push(i);
        }
        // "[6.4]": a section's number, between brackets.
        if a > 0 && bytes[a - 1] == b'[' {
            let rest = &answer[a..];
            let number: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if rest[number.len()..].starts_with(']') {
                if let Some(i) = book.items.iter().position(|it| !it.is_rule() && it.id == number) {
                    push(i);
                }
            }
        }
    }
    out
}

/// Who answers here.
#[derive(Clone, Debug, PartialEq)]
pub enum Who {
    /// Claude, with the key set in Settings › Keys.
    Claude(String),
    /// The on-device model (or one named by CATALYST_LOCAL_URL).
    Local,
    /// No model: the rules found are the answer.
    Nobody,
}

/// Claude when its key is set; the on-device model when it is downloaded.
pub fn who() -> Who {
    if let Some(key) = crate::keys::Keys::load().get(crate::ai::Service::Claude.key()) {
        return Who::Claude(key.to_owned());
    }
    if crate::offline::model::find().is_some() || (crate::ai::demo() && std::env::var_os("CATALYST_RULEBOOK_LOCAL").is_some()) {
        return Who::Local;
    }
    Who::Nobody
}

impl Who {
    /// Its name under the answer.
    pub fn name(&self) -> String {
        match self {
            Who::Claude(_) => crate::ai::Service::Claude.model_name().to_owned(),
            Who::Local => crate::offline::model::find().map(|m| m.name()).unwrap_or_else(|| localmodel::MODELS[localmodel::DEFAULT].name.to_owned()),
            Who::Nobody => String::new(),
        }
    }
}

/// An answer: its words, and who gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub text: String,
    pub by: String,
    /// Online was tried and could not be reached: the on-device model answered.
    pub offline: bool,
}

fn no_internet(why: &str) -> bool {
    why.starts_with("no internet") || why.starts_with("could not reach") || why.starts_with("no answer in time")
}

/// Ask. Call it on a thread of its own: an answer takes seconds.
pub fn answer(who: &Who, passages: &[Passage], question: &str) -> Result<Answer, String> {
    if let Who::Claude(key) = who {
        let (system, user) = claude_prompt(passages, question);
        let reply = if crate::ai::demo() { Ok(scripted(passages)) } else { crate::ai::grounded_request(key, &system, &user, CLAUDE_TOKENS) };
        match reply {
            Ok(text) => return Ok(Answer { text: crate::ai::plain(&text), by: who.name(), offline: false }),
            // Offline at the event: the model on this handheld, if it has one.
            Err(why) if no_internet(&why) && crate::offline::model::find().is_some() => {
                return local(passages, question).map(|mut a| {
                    a.offline = true;
                    a
                })
            }
            Err(why) => return Err(why),
        }
    }
    local(passages, question)
}

fn local(passages: &[Passage], question: &str) -> Result<Answer, String> {
    let Some(model) = crate::offline::model::find() else {
        if crate::ai::demo() {
            return Ok(Answer { text: scripted(passages), by: Who::Local.name(), offline: false });
        }
        return Err("no model on this handheld: Settings › On-device model".into());
    };
    let name = model.name();
    let tier = crate::offline::model::tier(crate::offline::model::size_m(&name));
    let context = if std::env::var_os("CATALYST_LOCAL_URL").is_some() { 2048 } else { crate::local::model().context };
    let few = &passages[..passages.len().min(tier.lines)];
    let (system, user) = local_prompt(few, question, context, LOCAL_ANSWER);
    let body = crate::offline::model::body(&system, &[("user", &user)], LOCAL_ANSWER, None);
    let reply: Value = model.chat(&body, tier.seconds.max(45))?;
    let (text, _, _) = crate::offline::model::reply(&reply)?;
    let text = crate::ai::plain(text.trim());
    if text.is_empty() {
        return Err("the on-device model gave no answer".into());
    }
    Ok(Answer { text, by: name, offline: false })
}

/// The simulator's answer, made from the rules found: the first rule's
/// first sentence (the others it found are listed under it).
pub fn scripted(passages: &[Passage]) -> String {
    let rules: Vec<&Passage> = passages.iter().filter(|p| parse::rule_id_at_start(&format!("{} ", p.cite.trim_matches(['[', ']']))).is_some() && !p.text.is_empty()).collect();
    let mut out = String::new();
    if let Some(p) = rules.first() {
        let line = p.text.lines().next().unwrap_or(&p.title);
        let first = line.split(". ").next().unwrap_or(line).trim().trim_end_matches('.');
        out.push_str(&format!("{first} {}.", p.cite));
    }
    if out.is_empty() {
        out = "The rules below are the closest the manual has to that.".into();
    }
    out
}
