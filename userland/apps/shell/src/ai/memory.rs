//! What the assistant remembers: the preferences and facts the person told
//! it to keep ("remember our robot is called Numbers", "I like short
//! answers", "my team is 5805"), and nothing else.
//!
//! - **Kept plainly**, where the person can read, change and delete it: a
//!   short Markdown list, one thing a line, at `/data/assistant/memory.md`
//!   (Files opens it too). Assistant › choices › Memory lists it, and `d`
//!   twice forgets one.
//! - **Small:** at most `MOST` things of `LONGEST` characters each. Into
//!   OpenAI's and Claude's system prompt whole, under `PROMPT_CAP`; a
//!   conversation keeps the copy it began with, so its prompt stays cached,
//!   and a change made during it reaches the model as its tool's answer.
//!   The on-device model is given a tiny version (`TINY_CAP`).
//! - **Leaves the handheld only with a question to OpenAI or Claude,** in
//!   that question's system prompt, while one of them answers. Offline
//!   nothing of it goes anywhere: the on-device model runs here. The
//!   Memory page says which is the case.
//! - **Never a secret.** A password, passcode, PIN, key, token or card
//!   number is refused when it is asked to be kept (by its words, and by
//!   anything shaped like a key), and a line like one typed into the file by
//!   hand is left out of every prompt.
//! - "forget that", "forget …" and "what do you remember" work: tools for
//!   the services, rules here offline ([`command`]).

use std::path::{Path, PathBuf};

/// Things kept at most, and the longest one.
pub const MOST: usize = 24;
pub const LONGEST: usize = 140;
/// What the services' system prompt holds of it at most, in characters;
/// and the on-device model's.
const PROMPT_CAP: usize = 1600;
const TINY_CAP: usize = 200;

const HEAD: &str = "# What the assistant remembers\n\nOne thing a line. Delete a line and it is forgotten. Passwords and keys are never kept here.\n\n";

/// Where it is kept: /data/assistant/memory.md on the handheld.
pub fn path() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/assistant/memory.md")
    } else {
        crate::config_dir().join("assistant-memory.md")
    }
}

/// Everything remembered, oldest first.
pub fn load(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file).map(|t| parse(&t)).unwrap_or_default()
}

/// The things in the file: its list's lines ("- …").
pub fn parse(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("- ").or_else(|| l.trim().strip_prefix("* ")))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect()
}

fn write(file: &Path, all: &[String]) -> Result<(), String> {
    let mut text = HEAD.to_owned();
    for fact in all {
        text.push_str("- ");
        text.push_str(fact);
        text.push('\n');
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    crate::sys::write_private(file, text.as_bytes()).map_err(|_| "the memory file could not be written".to_owned())
}

/// Words that say a secret follows, lower case.
const SECRET_WORDS: [&str; 31] = [
    "password", "passwd", "passcode", "passphrase", "pin code", "pin number", "my pin", "the pin", "pin is", "api key", "apikey", "api-key", "access key",
    "secret", "token", "private key", "ssh key", "credential", "one-time code", "recovery code", "2fa", "credit card", "card number", "cvv", "security code",
    "social security", "lock code", "door code", "combination is", "wi-fi key", "wifi key",
];

/// Why `text` is not kept, if it looks like a secret: a password or
/// passcode, a key or token, a card number, or something shaped like one.
pub fn secret(text: &str) -> Option<&'static str> {
    const WHY: &str = "that looks like a password or a key, and those are never kept · keys go in settings › keys";
    // Each word from its start ("passwords" is "password"; "spin is" is not "pin is").
    let flat: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { ' ' }).collect();
    let flat = format!(" {} ", flat.split_whitespace().collect::<Vec<_>>().join(" "));
    if SECRET_WORDS.iter().any(|w| flat.contains(&format!(" {w}"))) {
        return Some(WHY);
    }
    for word in text.split(|c: char| c.is_whitespace() || ",;:()\"'`".contains(c)) {
        let w = word.trim_matches(|c: char| c == '.' || c == '!' || c == '?');
        let lw = w.to_lowercase();
        let keyish = ["sk-", "sk_", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "akia", "aiza", "eyj", "-----begin"].iter().any(|p| lw.starts_with(p)) && w.len() >= 12;
        let letters = w.chars().filter(|c| c.is_ascii_alphabetic()).count();
        let digits = w.chars().filter(|c| c.is_ascii_digit()).count();
        // A long run of letters and digits mixed: what keys and tokens look like.
        let random = w.len() >= 16 && letters >= 4 && digits >= 3 && !w.contains('/');
        if keyish || random || digits >= 12 {
            return Some(WHY);
        }
    }
    // A card's number, in groups.
    if text.chars().filter(char::is_ascii_digit).count() >= 13 && !text.contains(':') {
        return Some(WHY);
    }
    None
}

/// A thing to keep, tidied: one line, its spaces single, no full stop,
/// a capital first.
fn tidy(fact: &str) -> String {
    let one: String = fact.split_whitespace().collect::<Vec<_>>().join(" ");
    let one = one.trim_end_matches(['.', '!', ';', ',']).trim().to_owned();
    let mut chars = one.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What saving came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Kept {
    Added(String),
    /// Kept in place of an older one.
    Replaced { old: String, new: String },
    /// It was remembered already.
    Already(String),
}

impl Kept {
    pub fn fact(&self) -> &str {
        match self {
            Kept::Added(f) | Kept::Already(f) => f,
            Kept::Replaced { new, .. } => new,
        }
    }
}

/// Remember `fact`, in place of `replaces` when that names one kept (a
/// preference changed: "our robot is called Atlas now").
pub fn save(file: &Path, fact: &str, replaces: &str) -> Result<Kept, String> {
    let fact = tidy(fact);
    if fact.is_empty() {
        return Err("nothing to remember was given".into());
    }
    if let Some(why) = secret(&fact) {
        return Err(why.into());
    }
    if fact.chars().count() > LONGEST {
        return Err(format!("too long to keep: at most {LONGEST} characters · say it in fewer words"));
    }
    let mut all = load(file);
    if let Some(same) = all.iter().find(|f| f.eq_ignore_ascii_case(&fact)) {
        return Ok(Kept::Already(same.clone()));
    }
    let kept = match find(&all, replaces).filter(|_| !replaces.trim().is_empty()) {
        Some(i) => {
            let old = std::mem::replace(&mut all[i], fact.clone());
            Kept::Replaced { old, new: fact }
        }
        None if all.len() >= MOST => return Err(format!("memory is full ({MOST} things) · forget one first, in assistant › memory")),
        None => {
            all.push(fact.clone());
            Kept::Added(fact)
        }
    };
    write(file, &all)?;
    Ok(kept)
}

/// Forget the thing `which` names: "that" (or nothing) is the newest.
pub fn forget(file: &Path, which: &str) -> Result<String, String> {
    let mut all = load(file);
    if all.is_empty() {
        return Err("nothing is remembered".into());
    }
    let w = which.trim().trim_end_matches(['.', '!']).to_lowercase();
    let at = if matches!(w.as_str(), "" | "that" | "it" | "this" | "the last one" | "the last thing") { Some(all.len() - 1) } else { find(&all, which) };
    let Some(at) = at else {
        return Err(format!("nothing remembered matches “{}”", crate::ai::short(which, 40)));
    };
    let gone = all.remove(at);
    write(file, &all)?;
    Ok(gone)
}

/// Forget everything: how many things were.
pub fn forget_all(file: &Path) -> usize {
    let n = load(file).len();
    if n > 0 {
        write(file, &[]).ok();
    }
    n
}

/// Forget one by its place in the list.
pub fn forget_at(file: &Path, at: usize) -> Option<String> {
    let mut all = load(file);
    (at < all.len()).then(|| all.remove(at)).filter(|_| write(file, &all).is_ok())
}

/// Words worth matching on: three letters or more, not the little ones.
fn words(text: &str) -> Vec<String> {
    const LITTLE: [&str; 14] = ["the", "and", "our", "that", "this", "is", "are", "was", "my", "his", "her", "about", "called", "with"];
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3 && !LITTLE.contains(w)).map(str::to_owned).collect()
}

/// The one kept that `query` means: the same words, one inside the
/// other, or most of the query's words.
pub fn find(all: &[String], query: &str) -> Option<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    if let Some(i) = all.iter().position(|f| f.to_lowercase() == q) {
        return Some(i);
    }
    if let Some(i) = all.iter().position(|f| f.to_lowercase().contains(&q) || q.contains(&f.to_lowercase())) {
        return Some(i);
    }
    let asked = words(query);
    if asked.is_empty() {
        return None;
    }
    let (best, score) = all.iter().enumerate().map(|(i, f)| (i, words(f).iter().filter(|w| asked.contains(w)).count())).max_by_key(|(i, s)| (*s, usize::MAX - i))?;
    (score * 2 >= asked.len() && score > 0).then_some(best)
}

/// The services' system prompt's part: what to follow, whole up to the cap;
/// anything like a secret left out.
pub fn for_prompt(all: &[String]) -> String {
    let shown: Vec<&String> = all.iter().filter(|f| secret(f).is_none()).collect();
    if shown.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nWhat the person asked you to remember (kept on this handheld in /data/assistant/memory.md; follow it, and use memory_save or memory_forget to change it):");
    let mut left = 0;
    for (i, fact) in shown.iter().enumerate() {
        if out.len() + fact.len() + 3 > PROMPT_CAP {
            left = shown.len() - i;
            break;
        }
        out.push_str("\n- ");
        out.push_str(fact);
    }
    if left > 0 {
        out.push_str(&format!("\n(and {left} more not shown)"));
    }
    out
}

/// The on-device model's: a line of the first few.
pub fn tiny(all: &[String]) -> String {
    let mut out = String::new();
    for fact in all.iter().filter(|f| secret(f).is_none()) {
        let piece = if out.is_empty() { format!(" Remember: {fact}") } else { format!("; {fact}") };
        if out.len() + piece.len() > TINY_CAP {
            break;
        }
        out.push_str(&piece);
    }
    if !out.is_empty() {
        out.push('.');
    }
    out
}

/// Asked of memory, read by rules (offline, with no tools).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Remember(String),
    /// What to forget ("" or "that": the newest; "everything").
    Forget(String),
    List,
}

/// What `question` asks of memory, if it asks something of it.
pub fn command(question: &str) -> Option<Command> {
    let mut q = question.trim().trim_end_matches(['?', '.', '!']).trim();
    for lead in ["please ", "hey "] {
        if q.get(..lead.len()).is_some_and(|h| h.eq_ignore_ascii_case(lead)) {
            q = q[lead.len()..].trim_start();
        }
    }
    let lower = q.to_lowercase();
    let lower = lower.as_str();
    // What follows a prefix, as typed (the prefixes are plain letters).
    let rest = |prefix: &str| q.get(prefix.len()..).map(|r| r.trim().trim_start_matches("that ").trim().to_owned());
    if ["what do you remember", "what do you know about me", "what have you remembered", "what are you remembering", "show your memory", "show memory", "list memory", "what's in your memory", "what is in your memory"].iter().any(|p| lower.starts_with(p)) {
        return Some(Command::List);
    }
    if lower.starts_with("forget ") || lower == "forget" {
        let what = rest("forget").unwrap_or_default();
        let what = match what.to_lowercase().as_str() {
            "everything" | "all of it" | "it all" | "all" => "everything".to_owned(),
            // The newest.
            "that" | "it" | "this" | "the last one" => String::new(),
            _ => what,
        };
        return Some(Command::Forget(what));
    }
    for prefix in ["remember ", "don't forget ", "do not forget "] {
        // "remember to swap the battery" is a reminder, not a fact.
        if lower.starts_with(prefix) && !lower.starts_with("remember when") && !lower[prefix.len()..].starts_with("to ") {
            return rest(prefix.trim_end()).filter(|r| !r.is_empty()).map(Command::Remember);
        }
    }
    for prefix in ["i like ", "i prefer ", "i'd like ", "call me ", "my team is ", "our team is ", "our robot is called ", "our robot is named ", "my name is "] {
        if lower.starts_with(prefix) && lower.split_whitespace().count() <= 12 {
            return Some(Command::Remember(q.to_owned()));
        }
    }
    None
}

/// What it remembers, said offline ("what do you remember"). Keeping and
/// forgetting offline go through the same tools as online
/// (`super::device::rules`), so forgetting everything still asks first.
pub fn listed(file: &Path) -> String {
    let all = load(file);
    match all.len() {
        0 => "Nothing yet. Tell me to remember something, like “remember our robot is called Numbers”.".into(),
        n => format!("I remember {}:\n{}", if n == 1 { "one thing".to_owned() } else { format!("{n} things") }, all.iter().map(|f| format!("· {f}")).collect::<Vec<_>>().join("\n")),
    }
}

/// Said in the first person, kept in the third: "I like short answers"
/// becomes "Likes short answers"; "my team is 5805", "The team is 5805".
pub fn as_kept(said: &str) -> String {
    let s = said.trim();
    let lower = s.to_lowercase();
    for (from, to) in [("i like ", "Likes "), ("i prefer ", "Prefers "), ("i'd like ", "Would like "), ("call me ", "Wants to be called "), ("my name is ", "Their name is "), ("my team is ", "The team is "), ("our team is ", "The team is "), ("our robot ", "The robot "), ("my robot ", "The robot ")] {
        if lower.starts_with(from) {
            return format!("{to}{}", &s[from.len()..]);
        }
    }
    s.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let f = std::env::temp_dir().join(format!("catalyst-memory-{name}-{}.md", std::process::id()));
        std::fs::remove_file(&f).ok();
        f
    }

    #[test]
    fn it_keeps_what_it_is_told_in_a_plain_list_and_forgets_on_request() {
        let f = file("keep");
        assert_eq!(save(&f, "the robot is called Numbers.", ""), Ok(Kept::Added("The robot is called Numbers".into())));
        assert_eq!(save(&f, "Likes short answers", ""), Ok(Kept::Added("Likes short answers".into())));
        assert_eq!(save(&f, "likes short answers", ""), Ok(Kept::Already("Likes short answers".into())), "the same, once");
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("# What the assistant remembers"), "{text}");
        assert!(text.contains("\n- The robot is called Numbers\n- Likes short answers\n"), "{text}");
        // Changed: kept in place of the old one.
        assert_eq!(save(&f, "The robot is called Atlas", "robot is called Numbers"), Ok(Kept::Replaced { old: "The robot is called Numbers".into(), new: "The robot is called Atlas".into() }));
        assert_eq!(load(&f), ["The robot is called Atlas", "Likes short answers"]);
        // "forget that": the newest; or by its words.
        assert_eq!(forget(&f, "that"), Ok("Likes short answers".into()));
        assert_eq!(forget(&f, "the robot's name"), Ok("The robot is called Atlas".into()), "by most of its words");
        assert!(forget(&f, "anything").is_err());
        // A file edited by hand: its lines are read as they are.
        std::fs::write(&f, "# mine\n\n- Team 5805\n* Drives on the left\n\nnot a line\n").unwrap();
        assert_eq!(load(&f), ["Team 5805", "Drives on the left"]);
        assert_eq!(forget_all(&f), 2);
        assert!(load(&f).is_empty());
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn secrets_are_refused_and_never_reach_a_prompt() {
        for said in [
            "The Wi-Fi password is hunter22",
            "My OpenAI key is sk-proj-AbCdEf123456789xyz",
            "github token ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "The lock code is 4821",
            "Card 4111 1111 1111 1111",
            "the api key is 9f8e7d6c5b4a39281706f5e4",
            "passcode 1234",
        ] {
            assert!(secret(said).is_some(), "{said}");
        }
        for fine in ["The robot is called Numbers", "Our team is 5805", "Queue at 10:42 for Q31", "Spare batteries are in the blue bin", "The robot code is in /data/code/robot", "Likes short answers"] {
            assert_eq!(secret(fine), None, "{fine}");
        }
        let f = file("secret");
        assert!(save(&f, "the wifi password is hunter22", "").unwrap_err().contains("never kept"));
        assert!(load(&f).is_empty(), "nothing written");
        // Typed into the file by hand: still never given to a model.
        let all = vec!["The robot is called Numbers".to_owned(), "Password is hunter22".to_owned()];
        assert!(!for_prompt(&all).contains("hunter22"));
        assert!(!tiny(&all).contains("hunter22"));
        assert!(for_prompt(&all).contains("- The robot is called Numbers"));
    }

    #[test]
    fn what_goes_to_a_model_is_capped() {
        let all: Vec<String> = (0..40).map(|i| format!("Thing number {i} that is worth keeping for a while, written out at some length to fill it")).collect();
        let prompt = for_prompt(&all);
        assert!(prompt.len() <= PROMPT_CAP + 40, "{}", prompt.len());
        assert!(prompt.ends_with("more not shown)"));
        let t = tiny(&all);
        assert!(t.len() <= TINY_CAP + 1 && t.starts_with(" Remember: Thing number 0"), "{t}");
        assert_eq!(tiny(&[]), "");
        assert_eq!(for_prompt(&[]), "");
        // Full: the rest refused.
        let f = file("full");
        for i in 0..MOST {
            save(&f, &format!("Fact {i}"), "").unwrap();
        }
        assert!(save(&f, "One more", "").unwrap_err().starts_with("memory is full"));
        assert!(save(&f, &"x".repeat(LONGEST + 1), "").unwrap_err().starts_with("too long"));
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn rules_read_what_is_asked_of_memory_offline() {
        assert_eq!(command("remember our robot is called Numbers"), Some(Command::Remember("our robot is called Numbers".into())));
        assert_eq!(command("Please remember that we queue early."), Some(Command::Remember("we queue early".into())));
        assert_eq!(command("I like short answers"), Some(Command::Remember("I like short answers".into())));
        assert_eq!(command("my team is 5805"), Some(Command::Remember("my team is 5805".into())));
        assert_eq!(command("forget that"), Some(Command::Forget("".into())));
        assert_eq!(command("forget everything"), Some(Command::Forget("everything".into())));
        assert_eq!(command("Forget the robot's name"), Some(Command::Forget("the robot's name".into())));
        assert_eq!(command("What do you remember?"), Some(Command::List));
        assert_eq!(command("do you remember Q14?"), None);
        assert_eq!(command("remember to swap the battery at 3pm"), None, "a reminder");
        assert_eq!(command("don't forget to charge the spare"), None);
        assert_eq!(command("why did we brown out in Q14?"), None);
        assert_eq!(as_kept("I like short answers"), "Likes short answers");
        assert_eq!(as_kept("my team is 5805"), "The team is 5805");
        assert_eq!(as_kept("our robot is called Numbers"), "The robot is called Numbers");
        let f = file("rules");
        assert!(listed(&f).starts_with("Nothing yet."));
        save(&f, &as_kept("our robot is called Numbers"), "").unwrap();
        assert_eq!(listed(&f), "I remember one thing:\n· The robot is called Numbers");
        std::fs::remove_file(&f).ok();
    }
}
