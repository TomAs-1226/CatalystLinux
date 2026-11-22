//! The assistant's hands: tools that act on the handheld, beside those that
//! look (`super::tool_specs`). The model may call several in one turn ("download
//! my Pit Mix playlist and remind me to swap the battery at queueing for
//! Q42"); each shows as a row in the conversation as it is done.
//!
//! **What runs at once, and what asks first.** A change that is harmless or
//! easily undone runs at once; anything critical keeps the full confirm page
//! (the one critical Actions have):
//!
//! | tool | | why |
//! |---|---|---|
//! | `reminder_add` | at once | it only adds; Reminders ticks it off or deletes it |
//! | `timer` | at once | a timer is not kept data, and starting or stopping one is the whole of the ask |
//! | `note_add` | at once | a new note, or lines added at a note's end: nothing written over |
//! | `music_play` | at once | Esc or the play key stops it |
//! | `music_download` | at once, up to `BIG_SONGS` songs or `BIG_BYTES` | Music › Downloads cancels it, and it waits politely (Wi-Fi, battery); more than that asks, for the card's room |
//! | `open_app` | at once | it only goes somewhere; Esc comes back |
//! | `setting_set` battery saver, brightness, light | at once | each is one switch back, in the control centre |
//! | `setting_set` Wi-Fi, Bluetooth | asks | network changes: Wi-Fi off cuts the internet the assistant answers over and the robot's link; Bluetooth's first switch-on restarts the handheld |
//! | `next_match`, `briefing` | at once | they only look |
//! | `start_download` music | at once | it goes on with downloads already asked for |
//! | `start_download` update | asks | it installs a new system, to start on the next restart |
//! | `memory_save` | at once | the person asked; a secret is refused whatever is asked |
//! | `memory_forget` one thing | at once | the person asked, and it deletes only the assistant's own note about them |
//! | `memory_forget` everything | asks | it deletes the whole memory |
//!
//! (There is no Do Not Disturb on the handheld; the setting tool says so.)

use std::path::Path;

use serde_json::{json, Value};

use super::briefing;
use super::memory;
use super::Call;

/// A download bigger than this asks first: songs, and bytes (about 1.5 GB).
pub const BIG_SONGS: usize = 150;
pub const BIG_BYTES: u64 = 1_500_000_000;

/// The tools that act, as (name, what it does, its input's JSON schema).
/// Every field is required (OpenAI's strict mode wants them all); one that
/// can be left out takes an empty string.
pub fn specs() -> Vec<(&'static str, &'static str, Value)> {
    let schema = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required, "additionalProperties": false});
    let text = |about: &str| json!({"type": "string", "description": about});
    let pick = |values: &[&str], about: &str| json!({"type": "string", "enum": values, "description": about});
    vec![
        (
            "reminder_add",
            "Set a reminder in Reminders, read as its quick add reads what is typed. Use it whenever the person asks to be reminded of something, at a time or tied to a match. It runs at once (a reminder is easily ticked off or deleted). A match not on the schedule yet is waited for.",
            schema(
                json!({"text": text("The reminder as the person would type it, its words and when together: \"swap the battery at queueing for Q42\", \"charge the spare in 20 min\", \"tomorrow 8am check bumpers\", \"10 min before our next match grab the cart\", \"after each match: check bumpers, download logs\". Match cues: before a match (20 min), N min before, at queueing, after a match, between matches; #pit or #personal for a list.")}),
                &["text"],
            ),
        ),
        (
            "timer",
            "Start a countdown timer in Clock, or stop timers running. Runs at once. The island says when one is done.",
            schema(json!({"action": pick(&["start", "stop"], "Start one, or stop."), "length": text("How long: \"10 min\", \"2:30\", \"90 s\", \"1 h 20 min\". To stop: the length of the one to stop, or empty for every timer.")}), &["action", "length"]),
        ),
        (
            "note_add",
            "Write a new note in Notes, or add lines to the end of one. Runs at once; nothing already written is changed.",
            schema(
                json!({
                    "title": text("A new note's title, a few words; empty to take its first words."),
                    "text": text("What to write."),
                    "append_to": text("The title of the note to add to; empty for a new note.")
                }),
                &["title", "text", "append_to"],
            ),
        ),
        (
            "music_play",
            "Search YouTube Music for songs and play them, the first now and the rest after it, behind whatever else is open. Runs at once.",
            schema(json!({"query": text("What to play: a song, an artist, a mood.")}), &["query"]),
        ),
        (
            "music_download",
            "Find a playlist, album or artist on YouTube Music and save its songs to the card, to play offline. Runs at once unless it is very large (then the person is asked); Music › Downloads shows it and can cancel it.",
            schema(json!({"query": text("Its name as the person said it: \"Pit Mix\", \"Random Access Memories\"."), "kind": pick(&["playlist", "album", "artist"], "What it is.")}), &["query", "kind"]),
        ),
        (
            "open_app",
            "Open one of the handheld's apps, or a page of Settings, once your answer is shown. Use it only when the person asks to go there.",
            schema(json!({"app": text("The app's name: Music, Reminders, Clock, Notes, Weather, Schedule, Robot, Logs, Graphs, Pit, Settings…"), "page": text("A Settings page: \"Battery\", \"Network\", \"Bluetooth\", \"Display\"; empty for the app itself.")}), &["app", "page"]),
        ),
        (
            "setting_set",
            "Change a setting. Battery saver, brightness and light mode change at once; Wi-Fi and Bluetooth are network changes, so the person is shown the change and asked first. There is no Do Not Disturb on this handheld.",
            schema(
                json!({
                    "setting": pick(&["wifi", "bluetooth", "battery_saver", "brightness", "light_mode"], "Which."),
                    "value": text("\"on\" or \"off\"; for brightness a percent (\"60\"), \"up\" or \"down\".")
                }),
                &["setting", "value"],
            ),
        ),
        (
            "next_match",
            "The team's next match from the event schedule: its label, alliance, partners and opponents, when it starts and when to queue.",
            schema(json!({}), &[]),
        ),
        (
            "start_download",
            "Start a download that is waiting: the system update the handheld has found (it installs a new system for the next restart, so the person is asked first), or Music's downloads, paused or failed (at once).",
            schema(json!({"what": pick(&["update", "music"], "Which.")}), &["what"]),
        ),
        (
            "briefing",
            "A briefing at a glance: the next match (queue time, alliance, partners) or the day's matches, the weather, the battery and its charge plan, reminders due today, what stood out in the robot's last match, and downloads or updates waiting. The handheld shows it as a card: call it when the person asks to be briefed, then say only the one to three things that matter most.",
            schema(json!({"kind": pick(&["auto", "morning", "pit"], "morning: the day ahead; pit: the next match; auto: whichever fits the hour and the schedule.")}), &["kind"]),
        ),
        (
            "memory_save",
            "Remember a preference or fact the person tells you to keep, or states as lasting (\"remember our robot is called Numbers\", \"I like short answers\", \"my team is 5805\"). Kept in a short plain list on the handheld that they can read and delete. Never a password, passcode, key or token: those are refused.",
            schema(
                json!({
                    "fact": text("One short line in the third person: \"The robot is called Numbers\", \"Likes short answers\", \"The team is 5805\"."),
                    "replaces": text("The remembered line this changes, word for word; empty if none.")
                }),
                &["fact", "replaces"],
            ),
        ),
        (
            "memory_forget",
            "Forget something remembered, when the person asks (\"forget that\": the one just saved). Forgetting everything asks first.",
            schema(json!({"fact": text("The remembered line, word for word, or its words; empty with everything set."), "everything": json!({"type": "boolean", "description": "Forget all of it."})}), &["fact", "everything"]),
        ),
    ]
}

/// What a tool's row shows, and what goes back to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    /// To the model: JSON, with a `done` sentence in it.
    pub text: String,
    /// The row: what was done ("reminder set · swap the battery"), and
    /// under it the detail ("at queueing for Q42 · waits for the schedule").
    pub words: String,
    pub detail: String,
    pub icon: &'static str,
    /// What the screen does with it: a setting, an app to open.
    pub effect: Option<Effect>,
    /// A briefing's card.
    pub brief: Option<Box<briefing::Brief>>,
}

impl Done {
    fn new(words: impl Into<String>, detail: impl Into<String>, icon: &'static str, data: Value) -> Done {
        Done { text: data.to_string(), words: words.into(), detail: detail.into(), icon, effect: None, brief: None }
    }

    fn with(mut self, effect: Effect) -> Done {
        self.effect = Some(effect);
        self
    }
}

/// What the screen does once a tool is done.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Saver(bool),
    Light(bool),
    Brightness(u32),
    /// An app (by its place among the built-in ones) or a Settings page (by
    /// its place among them), opened once the answer is in.
    Open { name: String, app: Option<usize>, setting: Option<usize> },
}

/// What a thread's work came to: done, or a change to ask about first.
pub enum Step {
    Done(Done),
    Ask(Act),
}

/// What answering a call takes.
pub enum Plan {
    Now(Result<Done, String>),
    /// On a thread, its row showing `working` (with `icon`) meanwhile.
    Work { working: String, icon: &'static str, job: Box<dyn FnOnce() -> Result<Step, String> + Send> },
    /// A change shown in full on the confirm page before it is made.
    Ask(Act),
}

/// A change asked about first: what it is, what it does, and the words of
/// the two answers.
pub struct Act {
    pub title: String,
    pub what: String,
    pub yes: String,
    pub no: String,
    pub kind: ActKind,
}

pub enum ActKind {
    Wifi(bool),
    Bluetooth(bool),
    Update(String),
    ForgetAll,
    Download { title: String, opened: crate::music::Opened },
}

impl Act {
    /// Done, on a yes.
    pub fn run(self, memory_file: &Path) -> Plan {
        match self.kind {
            ActKind::Wifi(on) => {
                crate::wifi::switch(on);
                let words = if on { "wi-fi on" } else { "wi-fi off" };
                Plan::Now(Ok(Done::new(words, if on { "joining a saved network" } else { "the handheld left its network" }, if on { "wifi" } else { "wifi_off" }, json!({"done": format!("Wi-Fi switched {}.", if on { "on" } else { "off" })}))))
            }
            ActKind::Bluetooth(on) => Plan::Work {
                working: format!("switching bluetooth {}", if on { "on" } else { "off" }),
                icon: "bluetooth",
                job: Box::new(move || {
                    let out = crate::bluetooth::switch(on)?;
                    let restart = out.contains("restart");
                    let detail = if restart { "on after a restart · settings › bluetooth" } else { "settings › bluetooth pairs devices" };
                    Ok(Step::Done(Done::new(format!("bluetooth {}", if on { "on" } else { "off" }), detail, "bluetooth", json!({"done": if restart { "Bluetooth is switched on; it starts working after the handheld restarts once." } else if on { "Bluetooth is on." } else { "Bluetooth is off." }}))))
                }),
            },
            ActKind::Update(version) => Plan::Now(crate::sysupdate::run("install").map(|()| Done::new(format!("downloading update {version}"), "settings › system update shows it · restart to finish", "download", json!({"done": format!("Update {version} is downloading; it installs beside this system and starts on the next restart.")})))),
            ActKind::ForgetAll => {
                let n = memory::forget_all(memory_file);
                Plan::Now(Ok(Done::new("forgot everything", format!("{} gone", count(n, "thing")), "delete", json!({"done": format!("Forgot all {n} remembered things.")}))))
            }
            ActKind::Download { title, opened } => Plan::Now(Ok(save(&title, &opened))),
        }
    }
}

/// What a tool needs from the screen.
pub struct Ctx<'a> {
    pub robot: &'a crate::robot::Robot,
    pub h24: bool,
    pub memory: &'a Path,
    /// The battery saver and light mode, as they are now.
    pub saver: bool,
    pub light: bool,
}

/// The plan for `call`, if it is one of these tools.
pub fn plan(call: &Call, cx: &Ctx) -> Option<Plan> {
    let s = |key: &str| call.input[key].as_str().unwrap_or_default().trim().to_owned();
    Some(match call.name.as_str() {
        "reminder_add" => Plan::Now(reminder_add(&s("text"), cx)),
        "timer" => Plan::Now(timer(&s("action"), &s("length"))),
        "note_add" => Plan::Now(note_add(&s("title"), &s("text"), &s("append_to"), cx)),
        "music_play" => {
            let query = s("query");
            Plan::Work { working: format!("finding {}", super::short(&query, 40)), icon: "music_note", job: Box::new(move || play(&query).map(Step::Done)) }
        }
        "music_download" => {
            let (query, kind) = (s("query"), s("kind"));
            Plan::Work { working: format!("finding {}", super::short(&query, 40)), icon: "downloading", job: Box::new(move || download(&query, &kind)) }
        }
        "open_app" => Plan::Now(open_app(&s("app"), &s("page"))),
        "setting_set" => setting(&s("setting"), &s("value"), cx),
        "next_match" => Plan::Now(next_match(cx)),
        "start_download" => start_download(&s("what")),
        "briefing" => {
            let inputs = briefing::Inputs::of(cx.robot, cx.h24, briefing::Kind::from_name(&s("kind")));
            Plan::Work {
                working: "making a briefing".into(),
                icon: "auto_awesome",
                job: Box::new(move || {
                    let b = briefing::build(&inputs);
                    let mut done = Done::new("made a briefing", b.title.to_lowercase(), "auto_awesome", b.data.clone());
                    done.brief = Some(Box::new(b));
                    Ok(Step::Done(done))
                }),
            }
        }
        "memory_save" => Plan::Now(memory::save(cx.memory, &s("fact"), &s("replaces")).map(|kept| {
            let (words, said) = match &kept {
                memory::Kept::Added(f) => ("remembered", format!("Remembered: {f}.")),
                memory::Kept::Replaced { old, new } => ("remembered", format!("Remembered: {new} (in place of: {old}).")),
                memory::Kept::Already(f) => ("remembered already", format!("Already remembered: {f}.")),
            };
            Done::new(words, kept.fact(), "push_pin", json!({"done": said, "file": "/data/assistant/memory.md"}))
        })),
        "memory_forget" if call.input["everything"].as_bool() == Some(true) => {
            let n = memory::load(cx.memory).len();
            if n == 0 {
                Plan::Now(Err("nothing is remembered".into()))
            } else {
                Plan::Ask(Act { title: "Forget everything".into(), what: format!("All {} it remembers about you and the team are deleted from memory.md.", count(n, "thing")), yes: "Forget them".into(), no: "Keep them".into(), kind: ActKind::ForgetAll })
            }
        }
        "memory_forget" => Plan::Now(memory::forget(cx.memory, &s("fact")).map(|gone| Done::new("forgotten", &gone, "delete", json!({"done": format!("Forgotten: {gone}.")})))),
        _ => return None,
    })
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// "35 MB", "1.8 GB".
fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 10_000_000 => format!("{} MB", b / 1_000_000),
        b => format!("{:.1} MB", b as f64 / 1e6),
    }
}

// ------------------------------------------------------------ reminders ---

fn reminder_add(text: &str, cx: &Ctx) -> Result<Done, String> {
    use crate::reminders::{parse, plan, Page};
    let view = cx.robot.schedule.view();
    let n = plan::Now { at: crate::reminders::now(), zone: &plan::Here, view: view.as_ref() };
    let parsed = parse::parse(text, &n)?;
    let mut set = Vec::new();
    crate::reminders::with(|s| {
        s.sync(&cx.robot.schedule);
        for p in parsed {
            let already = s.book.has_like(&p);
            let id = if already { s.book.reminders.iter().rev().find(|r| r.done.is_none() && r.text.eq_ignore_ascii_case(&p.text)).map(|r| r.id) } else { Some(s.book.add(p, &n)) };
            let Some(r) = id.and_then(|id| s.book.get(id)).cloned() else { continue };
            let row = s.book.rows(Page::Today, None, &n, cx.h24).into_iter().chain(s.book.rows(Page::Upcoming, None, &n, cx.h24)).find(|row| row.id == r.id);
            set.push((r.text.clone(), parse::when_text(&r, &n), row.as_ref().map(|r| r.when.clone()).unwrap_or_default(), row.as_ref().is_some_and(|r| r.waiting), r.list.name(), already));
        }
    });
    let Some(first) = set.first().cloned() else { return Err("no reminder was set".into()) };
    let (text, when, at, waiting, _, already) = &first;
    let words = match set.len() {
        1 if *already => format!("reminder set already · {text}"),
        1 => format!("reminder set · {text}"),
        k => format!("{k} reminders set · {}", set.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", ")),
    };
    // When, as typed ("at queueing for Q42"), then the time it comes to, or
    // that it waits for the schedule.
    let mut detail = if when.is_empty() { "no time · on today's list".to_owned() } else { when.clone() };
    if *waiting {
        detail.push_str(" · waits for the schedule");
    } else if !at.is_empty() && !when.contains(at.as_str()) {
        detail.push_str(&format!(" · {at}"));
    }
    let said: Vec<String> = set
        .iter()
        .map(|(text, when, at, waiting, _, already)| {
            let when = if when.is_empty() { "with no time, on today's list".to_owned() } else { when.clone() };
            let label = when.split_whitespace().find_map(|w| crate::reminders::parse::label(&w.to_lowercase()));
            let wait = match (waiting, label) {
                (true, Some(l)) => format!(" ({l} is not on the schedule yet: its time comes once it is)"),
                (true, None) => " (its time comes from the schedule)".to_owned(),
                _ if !at.is_empty() && !when.contains(at.as_str()) => format!(" ({at})"),
                _ => String::new(),
            };
            format!("{} {text}, {when}{wait}.", if *already { "Already set:" } else { "Reminder set:" })
        })
        .collect();
    let data = json!({
        "set": set.iter().map(|(text, when, at, waiting, list, already)| json!({"text": text, "when": when, "at": at, "waitingForSchedule": waiting, "list": list, "alreadySet": already})).collect::<Vec<_>>(),
        "done": said.join(" "),
    });
    Ok(Done::new(words, detail, "notification_add", data))
}

// ---------------------------------------------------------------- clock ---

/// A length as said ("10 min", "1 h 20 min", "90 seconds") in Clock's own form.
fn length_of(said: &str) -> Option<f64> {
    let mut t = format!(" {} ", said.to_lowercase().replace(" and ", " ")).replace("half an hour", "30m").replace("an hour", "1h").replace("a minute", "1m");
    for (long, unit) in [("hours", "h"), ("hour", "h"), ("hrs", "h"), ("hr", "h"), ("minutes", "m"), ("minute", "m"), ("mins", "m"), ("min", "m"), ("seconds", "s"), ("second", "s"), ("secs", "s"), ("sec", "s")] {
        t = t.replace(&format!(" {long} "), &format!("{unit} ")).replace(&format!("{long} "), &format!("{unit} "));
    }
    crate::clock::parse_length(t.trim())
}

fn timer(action: &str, length: &str) -> Result<Done, String> {
    if action == "stop" {
        let running = crate::clock::running_timers();
        if running.is_empty() {
            return Err("no timer is running".into());
        }
        let wanted = length_of(length);
        let stop: Vec<(u32, f64, f64)> = running.into_iter().filter(|(_, total, _)| wanted.is_none_or(|w| (w - total).abs() < 1.0)).collect();
        if stop.is_empty() {
            return Err(format!("no {} timer is running", crate::clock::length(wanted.unwrap_or(0.0))));
        }
        for (id, ..) in &stop {
            crate::clock::remove_timer(*id);
        }
        let (_, total, left) = stop[0];
        let words = if stop.len() == 1 { format!("timer stopped · {}", crate::clock::length(total)) } else { format!("{} timers stopped", stop.len()) };
        return Ok(Done::new(words, format!("{} was left", crate::clock::length(left.ceil())), "timer", json!({"done": format!("Stopped {}.", count(stop.len(), "timer"))})));
    }
    let seconds = length_of(length).ok_or_else(|| format!("“{length}” is not a length Clock reads: say “10 min” or “2:30”"))?;
    crate::clock::add_timer(seconds);
    let shown = crate::clock::length(seconds);
    Ok(Done::new(format!("timer started · {shown}"), "clock counts it down · the island says when it is done", "timer", json!({"done": format!("A {shown} timer is running in Clock."), "seconds": seconds})))
}

// ---------------------------------------------------------------- notes ---

fn note_add(title: &str, text: &str, append_to: &str, cx: &Ctx) -> Result<Done, String> {
    if text.is_empty() {
        return Err("nothing to write was given".into());
    }
    let dir = crate::notes::dir();
    std::fs::create_dir_all(&dir).map_err(|_| "the notes folder could not be made".to_owned())?;
    if !append_to.is_empty() {
        let notes = crate::notes::load(&dir);
        let want = append_to.to_lowercase();
        let note = notes.iter().find(|n| n.title.to_lowercase() == want).or_else(|| notes.iter().find(|n| n.title.to_lowercase().contains(&want))).ok_or_else(|| format!("no note is called “{append_to}”"))?;
        // Before the note's foot ("— Q27 · nov 21 09:05"), if it has one.
        let mut lines: Vec<&str> = note.text.trim_end().lines().collect();
        let foot = lines.last().is_some_and(|l| l.starts_with("— ")).then(|| lines.pop()).flatten();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        let mut body = lines.join("\n");
        body.push('\n');
        body.push_str(text);
        body.push('\n');
        if let Some(foot) = foot {
            body.push('\n');
            body.push_str(foot);
            body.push('\n');
        }
        crate::sys::write_atomic(&dir.join(&note.file), body.as_bytes()).map_err(|_| "the note could not be written".to_owned())?;
        return Ok(Done::new(format!("added to {}", note.title), super::short(text, 90), "sticky_note_2", json!({"done": format!("Added to the note “{}”.", note.title), "note": note.title})));
    }
    let title = if title.is_empty() { text.split_whitespace().take(6).collect::<Vec<_>>().join(" ") } else { title.to_owned() };
    let now = chrono::Local::now();
    let label = cx.robot.schedule.view().and_then(|v| v.next(chrono::Utc::now().timestamp()).map(|m| m.label.clone()));
    let body = format!("{title}\n{text}\n\n{}\n", crate::notes::stamp(label.as_deref(), now));
    let mut name = now.format("%Y-%m-%d-%H%M%S").to_string();
    while dir.join(format!("{name}.txt")).exists() {
        name.push('b');
    }
    crate::sys::write_atomic(&dir.join(format!("{name}.txt")), body.as_bytes()).map_err(|_| "the note could not be written".to_owned())?;
    Ok(Done::new(format!("note made · {title}"), super::short(text, 90), "sticky_note_2", json!({"done": format!("A note “{title}” is in Notes."), "note": title})))
}

// ---------------------------------------------------------------- music ---

fn play(query: &str) -> Result<Done, String> {
    let found = crate::music::find_songs(query)?;
    let Some(first) = found.first().cloned() else { return Err(format!("nothing found for “{query}”")) };
    let more = found.len() - 1;
    crate::music::play_list(found);
    Ok(Done::new(
        format!("playing · {}", first.title),
        format!("{}{}", first.artist, if more > 0 { format!(" · {more} more after it") } else { String::new() }),
        "play_circle",
        json!({"done": format!("Playing {} by {}, with {more} more after it.", first.title, first.artist), "song": first.title, "artist": first.artist}),
    ))
}

/// The words of a name worth matching on.
fn name_words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !matches!(*w, "my" | "the" | "our" | "a" | "playlist" | "album" | "songs")).map(str::to_owned).collect()
}

/// The best of what a search found for `query`: the same name, else the one
/// with most of its words.
pub fn best<'a>(found: &'a [crate::music::Collection], query: &str) -> Option<&'a crate::music::Collection> {
    let q = name_words(query);
    if let Some(same) = found.iter().find(|c| name_words(&c.title) == q) {
        return Some(same);
    }
    found.iter().enumerate().max_by_key(|(i, c)| (name_words(&c.title).iter().filter(|w| q.contains(w)).count(), usize::MAX - i)).map(|(_, c)| c)
}

fn download(query: &str, kind: &str) -> Result<Step, String> {
    let kind = match kind {
        "album" => crate::music::Kind::Album,
        "artist" => crate::music::Kind::Artist,
        _ => crate::music::Kind::Playlist,
    };
    eprintln!("DBG search {:?}", std::time::Instant::now());
    let found = crate::music::search_collections(query, kind)?;
    eprintln!("DBG found {:?}", std::time::Instant::now());
    let c = best(&found, query).cloned().ok_or_else(|| format!("nothing on YouTube Music is called “{query}”"))?;
    let opened = crate::music::open_collection(&c)?;
    eprintln!("DBG opened {:?}", std::time::Instant::now());
    if opened.tracks.is_empty() {
        return Err(format!("{} has no songs to save", c.title));
    }
    let bytes = crate::music::estimate(&opened.tracks);
    if opened.tracks.len() > BIG_SONGS || bytes > BIG_BYTES {
        return Ok(Step::Ask(Act {
            title: format!("Download {} songs", opened.tracks.len()),
            what: format!("{}, about {}, saved to the card over a while. Music › Downloads can cancel it.", c.title, size(bytes)),
            yes: "Download it".into(),
            no: "Don't".into(),
            kind: ActKind::Download { title: c.title.clone(), opened },
        }));
    }
    Ok(Step::Done(save(&c.title, &opened)))
}

/// An album's, playlist's or artist's songs put in the download queue.
fn save(title: &str, opened: &crate::music::Opened) -> Done {
    let added = crate::music::save_opened(opened, title);
    eprintln!("DBG added {added} {:?}", std::time::Instant::now());
    let bytes = crate::music::estimate(&opened.tracks);
    let shown = size(bytes);
    let wait = crate::music::download_wait();
    eprintln!("DBG wait {wait:?}");
    if added == 0 {
        let (on_card, waiting) = crate::music::saved_or_waiting(&opened.tracks);
        if waiting == 0 {
            return Done::new(format!("{title} is on the card"), format!("{} saved already", count(on_card.max(opened.tracks.len()), "song")), "download_done", json!({"done": format!("All of {title} is on the card already.")}));
        }
        // Asked for before, and still on its way.
        let detail = format!("{} downloading already{}", count(waiting, "song"), wait.map(|w| format!(" · {}", w.to_lowercase())).unwrap_or_default());
        return Done::new(format!("downloading {title}"), detail, "downloading", json!({"done": format!("{title} is downloading already: {} still to come.", count(waiting, "song")), "found": title, "waiting": wait}));
    }
    let mut detail = format!("{} · about {shown}", count(added, "song"));
    if let Some(w) = wait {
        detail.push_str(&format!(" · {}", w.to_lowercase()));
    }
    let said = format!("Downloading {title}: {}, about {shown}.{}", count(added, "song"), wait.map(|w| format!(" It is {}.", w.to_lowercase())).unwrap_or_default());
    Done::new(format!("downloading {title}"), detail, "downloading", json!({"done": said, "found": title, "songs": added, "bytes": bytes, "waiting": wait}))
}

// ----------------------------------------------------------------- apps ---

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// A page of Settings, by its name as search finds it.
fn open_setting(page: &str) -> Result<Done, String> {
    let want = lower(page);
    let places: Vec<(usize, &str)> = crate::screens::setting_names().collect();
    let found = places.iter().find(|(_, n)| lower(n) == want).or_else(|| places.iter().find(|(_, n)| lower(n).starts_with(&want))).or_else(|| places.iter().find(|(_, n)| lower(n).contains(&want)));
    let (at, name) = found.copied().ok_or_else(|| format!("Settings has no page called “{page}”"))?;
    Ok(Done::new(format!("opens settings › {}", name.to_lowercase()), "once the answer is in", "settings", json!({"done": format!("Settings › {name} opens once you have answered.")}))
        .with(Effect::Open { name: format!("Settings › {name}"), app: None, setting: Some(at) }))
}

fn open_app(app: &str, page: &str) -> Result<Done, String> {
    if !page.is_empty() {
        return open_setting(page);
    }
    let want = lower(app);
    // "Schedule" is the Robot app's schedule in all but name: the event's own page.
    let want = match want.as_str() {
        "schedule" | "event" | "matches" => "robot".to_owned(),
        "timers" | "timer" | "stopwatch" => "clock".to_owned(),
        "youtube music" => "music".to_owned(),
        other => other.to_owned(),
    };
    let apps: Vec<(usize, &str, &str)> = crate::screens::builtin_apps().collect();
    let found = apps.iter().find(|(_, _, n)| lower(n) == want).or_else(|| apps.iter().find(|(_, _, n)| lower(n).starts_with(&want)));
    // Not an app: perhaps a page of Settings ("open battery").
    let Some((at, icon, name)) = found.copied() else { return open_setting(app).map_err(|_| format!("there is no app called “{app}”")) };
    if name == "Assistant" {
        return Err("this is the Assistant".into());
    }
    let icon: &'static str = icon;
    Ok(Done::new(format!("opens {name}"), "once the answer is in", icon, json!({"done": format!("{name} opens once you have answered.")})).with(Effect::Open { name: name.to_owned(), app: Some(at), setting: None }))
}

// ------------------------------------------------------------- settings ---

fn on_off(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enable" | "enabled" => Some(true),
        "off" | "false" | "no" | "0" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

fn setting(name: &str, value: &str, cx: &Ctx) -> Plan {
    let flag = on_off(value);
    let need = || Plan::Now(Err(format!("{name} takes \"on\" or \"off\"")));
    match name {
        "battery_saver" => match flag {
            Some(on) if on == cx.saver => Plan::Now(Ok(Done::new(format!("battery saver is {} already", if on { "on" } else { "off" }), "nothing changed", "bolt", json!({"done": "Nothing to change."})))),
            Some(on) => Plan::Now(Ok(Done::new(format!("battery saver {}", if on { "on" } else { "off" }), "the control centre switches it back", "bolt", json!({"done": format!("Battery saver is {}.", if on { "on" } else { "off" })})).with(Effect::Saver(on)))),
            None => need(),
        },
        "light_mode" => match flag {
            Some(on) if on == cx.light => Plan::Now(Ok(Done::new(format!("{} mode already", if on { "light" } else { "dark" }), "nothing changed", if on { "sunny" } else { "bedtime" }, json!({"done": "Nothing to change."})))),
            Some(on) => Plan::Now(Ok(Done::new(if on { "light mode" } else { "dark mode" }, "the control centre switches it back", if on { "sunny" } else { "bedtime" }, json!({"done": format!("The screen is {} now.", if on { "light" } else { "dark" })})).with(Effect::Light(on)))),
            None => need(),
        },
        "brightness" => {
            let now = crate::sys::brightness().unwrap_or(70);
            let v = value.trim().trim_end_matches('%').to_lowercase();
            let level = match v.as_str() {
                "up" | "brighter" => Some(now + 20),
                "down" | "dimmer" => Some(now.saturating_sub(20)),
                "max" | "full" | "on" => Some(100),
                "min" | "low" | "off" => Some(10),
                n => n.trim().parse::<u32>().ok(),
            };
            match level {
                Some(level) => {
                    let level = level.clamp(10, 100);
                    crate::sys::set_brightness(level);
                    Plan::Now(Ok(Done::new(format!("brightness {level}%"), "the control centre sets it again", "lightbulb", json!({"done": format!("Brightness is {level}%.")})).with(Effect::Brightness(level))))
                }
                None => Plan::Now(Err("brightness takes a percent, \"up\" or \"down\"".into())),
            }
        }
        "wifi" => match flag {
            Some(on) if on == crate::wifi::switched_on() => Plan::Now(Ok(Done::new(format!("wi-fi is {} already", if on { "on" } else { "off" }), "nothing changed", if on { "wifi" } else { "wifi_off" }, json!({"done": "Nothing to change."})))),
            Some(true) => Plan::Ask(Act { title: "Switch Wi-Fi on".into(), what: "The handheld joins the saved networks it finds, for the internet and the robot.".into(), yes: "Switch it on".into(), no: "Leave it off".into(), kind: ActKind::Wifi(true) }),
            Some(false) => Plan::Ask(Act {
                title: "Switch Wi-Fi off".into(),
                what: "The handheld leaves its network: no internet for the assistant and no robot over Wi-Fi, until it is back on.".into(),
                yes: "Switch it off".into(),
                no: "Leave it on".into(),
                kind: ActKind::Wifi(false),
            }),
            None => need(),
        },
        "bluetooth" => match flag {
            Some(on) if on == crate::bluetooth::enabled() => Plan::Now(Ok(Done::new(format!("bluetooth is {} already", if on { "on" } else { "off" }), "nothing changed", "bluetooth", json!({"done": "Nothing to change."})))),
            Some(true) => Plan::Ask(Act { title: "Switch Bluetooth on".into(), what: "The radio comes on for headphones and speakers. The first time, the handheld restarts once.".into(), yes: "Switch it on".into(), no: "Leave it off".into(), kind: ActKind::Bluetooth(true) }),
            Some(false) => Plan::Ask(Act { title: "Switch Bluetooth off".into(), what: "Headphones and speakers let go: sound goes back to the handheld's own speaker.".into(), yes: "Switch it off".into(), no: "Leave it on".into(), kind: ActKind::Bluetooth(false) }),
            None => need(),
        },
        "do_not_disturb" | "dnd" => Plan::Now(Err("there is no Do Not Disturb on this handheld".into())),
        other => Plan::Now(Err(format!("there is no setting called {other}"))),
    }
}

// --------------------------------------------------------------- schedule ---

fn next_match(cx: &Ctx) -> Result<Done, String> {
    let now = crate::reminders::now();
    let inputs = briefing::Inputs::of(cx.robot, cx.h24, Some(briefing::Kind::Pit));
    let Some(n) = inputs.next else { return Err(inputs.no_next) };
    let soon = |t: i64| crate::schedule::when(t, now);
    let side = n.side.unwrap_or("not on an alliance");
    let queue = n.queue.map(|(q, _)| soon(q));
    let detail = format!("{side}{} · queue {}", if n.partners.is_empty() { String::new() } else { format!(" with {}", n.partners.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")) }, queue.clone().unwrap_or_else(|| "time not known".into()));
    Ok(Done::new(
        format!("next is {}{}", n.label, n.starts.map(|t| format!(" · {}", soon(t))).unwrap_or_default()),
        detail,
        "event",
        json!({
            "match": n.label, "alliance": n.side, "partners": n.partners, "opponents": n.opponents,
            "starts": n.starts.map(soon), "queue": queue,
            "queueTimeFrom": n.queue.map(|(_, known)| if known { "FRC Nexus" } else { "worked out: 20 min before the match" }),
            "status": n.status, "event": inputs.event,
        }),
    ))
}

fn start_download(what: &str) -> Plan {
    if what == "music" {
        let n = crate::music::retry_downloads();
        let (waiting, _) = crate::music::downloads_pending();
        if waiting == 0 {
            return Plan::Now(Err("no music downloads are waiting".into()));
        }
        let wait = crate::music::download_wait();
        return Plan::Now(Ok(Done::new(
            format!("music downloads going · {}", count(waiting, "song")),
            wait.map_or_else(|| "music › downloads shows them".to_owned(), |w| w.to_lowercase()),
            "downloading",
            json!({"done": format!("{} waiting to download{}.", count(waiting, "song"), if n > 0 { format!(", {n} of them tried again") } else { String::new() }), "waiting": wait}),
        )));
    }
    let u = crate::sysupdate::read();
    match u.state.as_str() {
        "available" => Plan::Ask(Act {
            title: format!("Download update {}", u.version),
            what: format!("{}It installs beside this system and starts on the next restart; this one is kept to go back to.", if u.size > 0 { format!("About {}. ", size(u.size)) } else { String::new() }),
            yes: "Download it".into(),
            no: "Not now".into(),
            kind: ActKind::Update(u.version.clone()),
        }),
        "downloading" | "installing" => Plan::Now(Ok(Done::new(format!("update {}", crate::sysupdate::summary(&u)), "settings › system update shows it", "download", json!({"done": "The update is on its way already."})))),
        "ready" => Plan::Now(Ok(Done::new(format!("update {} is ready", u.version), "settings › system update restarts into it", "download_done", json!({"done": format!("Update {} is downloaded; restart from Settings › System update to finish.", u.version)})))),
        _ => Plan::Now(Err(format!("no update is waiting: {}", crate::sysupdate::summary(&u)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(robot: &'a crate::robot::Robot, memory: &'a Path) -> Ctx<'a> {
        Ctx { robot, h24: true, memory, saver: false, light: false }
    }

    fn call(name: &str, input: Value) -> Call {
        Call { id: "c1".into(), name: name.into(), input }
    }

    fn now_done(plan: Option<Plan>) -> Result<Done, String> {
        match plan {
            Some(Plan::Now(r)) => r,
            Some(Plan::Work { job, .. }) => match job()? {
                Step::Done(d) => Ok(d),
                Step::Ask(a) => Err(format!("asks: {}", a.title)),
            },
            Some(Plan::Ask(a)) => Err(format!("asks: {}", a.title)),
            None => Err("not a device tool".into()),
        }
    }

    #[test]
    fn every_tool_is_strict_with_every_field_required() {
        for (name, about, schema) in specs() {
            assert_eq!(schema["additionalProperties"], false, "{name}");
            let props: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
            let required: Vec<&str> = schema["required"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
            assert_eq!(props.len(), required.len(), "{name}: OpenAI's strict mode wants every field required");
            assert!(props.iter().all(|p| required.contains(&p.as_str())), "{name}");
            assert!(about.len() > 40 && about.len() < 520, "{name}: {}", about.len());
        }
    }

    #[test]
    fn a_reminder_is_set_from_words_and_a_pit_cue_waits_for_its_match() {
        crate::shell::tests::sandbox();
        let robot = crate::robot::Robot::demo();
        let mem = std::env::temp_dir().join("catalyst-device-mem-unused.md");
        let done = now_done(plan(&call("reminder_add", json!({"text": "swap the battery at queueing for Q42"})), &ctx(&robot, &mem))).unwrap();
        assert_eq!(done.words, "reminder set · swap the battery");
        assert_eq!(done.detail, "at queueing for Q42 · waits for the schedule", "Q42 is not on the demo schedule");
        let v: Value = serde_json::from_str(&done.text).unwrap();
        assert_eq!(v["set"][0]["waitingForSchedule"], true);
        assert!(v["done"].as_str().unwrap().contains("Q42 is not on the schedule yet"), "{v}");
        assert!(crate::reminders::with(|s| s.book.reminders.iter().any(|r| r.text == "swap the battery")));
        // One on the schedule comes to a time.
        let done = now_done(plan(&call("reminder_add", json!({"text": "charge the spare before Q40"})), &ctx(&robot, &mem))).unwrap();
        assert!(done.detail.starts_with("before Q40 · "), "{}", done.detail);
        // Asked twice: set once.
        let again = now_done(plan(&call("reminder_add", json!({"text": "charge the spare before Q40"})), &ctx(&robot, &mem))).unwrap();
        assert_eq!(again.words, "reminder set already · charge the spare");
        assert!(now_done(plan(&call("reminder_add", json!({"text": "in 10 min"})), &ctx(&robot, &mem))).is_err(), "nothing to be reminded of");
    }

    #[test]
    fn timers_start_from_lengths_as_said_and_stop() {
        assert_eq!(length_of("10 min"), Some(600.0));
        assert_eq!(length_of("1 hour 20 minutes"), Some(4800.0));
        assert_eq!(length_of("90 seconds"), Some(90.0));
        assert_eq!(length_of("2:30"), Some(150.0));
        assert_eq!(length_of("half an hour"), Some(1800.0));
        let started = timer("start", "7 min").unwrap();
        assert_eq!(started.words, "timer started · 7:00");
        let stopped = timer("stop", "7 min").unwrap();
        assert_eq!(stopped.words, "timer stopped · 7:00");
        assert!(timer("stop", "").is_err(), "none left");
        assert!(timer("start", "soonish").is_err());
    }

    #[test]
    fn settings_that_are_one_switch_back_change_and_network_ones_ask() {
        crate::shell::tests::sandbox();
        let robot = crate::robot::Robot::demo();
        let mem = std::env::temp_dir().join("catalyst-device-mem-unused.md");
        let c = ctx(&robot, &mem);
        let saver = now_done(plan(&call("setting_set", json!({"setting": "battery_saver", "value": "on"})), &c)).unwrap();
        assert_eq!((saver.words.as_str(), saver.effect), ("battery saver on", Some(Effect::Saver(true))));
        let bright = now_done(plan(&call("setting_set", json!({"setting": "brightness", "value": "60%"})), &c)).unwrap();
        assert_eq!(bright.effect, Some(Effect::Brightness(60)));
        let wifi_on = crate::wifi::switched_on();
        let wifi = plan(&call("setting_set", json!({"setting": "wifi", "value": if wifi_on { "off" } else { "on" }})), &c);
        assert!(matches!(wifi, Some(Plan::Ask(Act { kind: ActKind::Wifi(_), .. }))), "a network change asks first");
        assert!(matches!(plan(&call("setting_set", json!({"setting": "bluetooth", "value": "on"})), &c), Some(Plan::Ask(_))));
        assert!(now_done(plan(&call("setting_set", json!({"setting": "do_not_disturb", "value": "on"})), &c)).unwrap_err().contains("no Do Not Disturb"));
    }

    #[test]
    fn apps_and_settings_pages_open_by_name_once_the_answer_is_in() {
        let music = open_app("Music", "").unwrap();
        assert!(matches!(music.effect, Some(Effect::Open { ref name, app: Some(_), setting: None }) if name == "Music"));
        assert_eq!(music.words, "opens Music");
        let battery = open_app("Settings", "battery").unwrap();
        assert!(matches!(battery.effect, Some(Effect::Open { setting: Some(_), app: None, .. })), "{battery:?}");
        assert!(open_app("Schedule", "").unwrap().words.starts_with("opens "), "the schedule is an app's page");
        assert!(open_app("Nonsense", "").is_err());
        assert!(open_app("Assistant", "").is_err());
    }

    #[test]
    fn a_playlist_is_found_by_most_of_its_words_and_downloaded() {
        let c = |t: &str| crate::music::Collection { kind: crate::music::Kind::Playlist, id: t.into(), title: t.into(), byline: String::new(), art: String::new(), year: 0 };
        let found = vec![c("Late Night Drive"), c("Pit Crew Mix"), c("Mix")];
        assert_eq!(best(&found, "my Pit Mix playlist").map(|c| c.title.as_str()), Some("Pit Crew Mix"));
        assert_eq!(best(&found, "mix").map(|c| c.title.as_str()), Some("Mix"), "the same name first");
        // The demo catalogue: found, opened, and queued to download.
        match download("Pit Mix", "playlist").unwrap() {
            Step::Done(d) => {
                assert_eq!(d.words, "downloading Pit Crew Mix");
                // (Asked for already, by another test sharing the queue: "10 songs downloading already".)
                assert!(d.detail.starts_with("10 songs · about 35 MB") || d.detail.starts_with("10 songs downloading already"), "{}", d.detail);
            }
            Step::Ask(_) => panic!("ten songs do not ask"),
        }
    }

    #[test]
    fn notes_are_made_and_added_to_without_writing_over_them() {
        crate::shell::tests::sandbox();
        let robot = crate::robot::Robot::demo();
        let mem = std::env::temp_dir().join("catalyst-device-mem-unused.md");
        let c = ctx(&robot, &mem);
        let title = format!("Intake test {}", std::process::id());
        let made = note_add(&title, "chain slips at the top", "", &c).unwrap();
        assert_eq!(made.words, format!("note made · {title}"));
        note_add("", "retension before Q40", &title, &c).unwrap();
        let note = crate::notes::load(&crate::notes::dir()).into_iter().find(|n| n.title == title).unwrap();
        let lines: Vec<&str> = note.text.lines().collect();
        assert_eq!(&lines[..3], [title.as_str(), "chain slips at the top", "retension before Q40"], "added before its foot: {lines:?}");
        assert!(lines.last().unwrap().starts_with("— "), "the foot stays last");
        std::fs::remove_file(crate::notes::dir().join(&note.file)).ok();
        assert!(note_add("", "x", "no such note at all", &c).is_err());
    }

    #[test]
    fn memory_is_kept_and_forgotten_and_all_of_it_only_after_a_yes() {
        let robot = crate::robot::Robot::demo();
        let mem = std::env::temp_dir().join(format!("catalyst-device-mem-{}.md", std::process::id()));
        std::fs::remove_file(&mem).ok();
        let c = ctx(&robot, &mem);
        let saved = now_done(plan(&call("memory_save", json!({"fact": "The robot is called Numbers", "replaces": ""})), &c)).unwrap();
        assert_eq!((saved.words.as_str(), saved.detail.as_str()), ("remembered", "The robot is called Numbers"));
        assert!(now_done(plan(&call("memory_save", json!({"fact": "Our Wi-Fi password is hunter22", "replaces": ""})), &c)).unwrap_err().contains("never kept"));
        assert!(matches!(plan(&call("memory_forget", json!({"fact": "", "everything": true})), &c), Some(Plan::Ask(Act { kind: ActKind::ForgetAll, .. }))));
        let gone = now_done(plan(&call("memory_forget", json!({"fact": "that", "everything": false})), &c)).unwrap();
        assert_eq!(gone.detail, "The robot is called Numbers");
        std::fs::remove_file(&mem).ok();
    }
}
