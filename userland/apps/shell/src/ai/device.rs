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
//! | `timer` start, or stop one | at once | a timer is not kept data, and starting or stopping one is the whole of the ask |
//! | `timer` stop several | asks | each one's time left is lost, and "stop the timer" may have meant one |
//! | `note_add` | at once | a new note, or lines added at a note's end: nothing written over |
//! | `music_play` | at once | Esc or the play key stops it |
//! | `music_control` | at once | pause, carry on, next, back: each is one key back |
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
//!
//! **Offline**, with no model to choose the tools, [`rules`] reads plain
//! requests ("start a 10 min timer and remind me to check the bumpers at
//! queueing") into the same calls, so they are done, shown and asked about
//! just as they are online.

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
            "Start a countdown timer in Clock, or stop timers running. Runs at once, except stopping several, which asks first. The island says when one is done.",
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
            "music_control",
            "Pause the music, carry on playing, skip to the next song or go back one. Runs at once.",
            schema(json!({"action": pick(&["pause", "resume", "next", "previous"], "Which.")}), &["action"]),
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
    /// The timers to stop: each one's id and length.
    StopTimers(Vec<(u32, f64)>),
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
            ActKind::StopTimers(timers) => {
                // Those that ran out meanwhile are gone already: the rest.
                let stopped = timers.iter().filter(|(id, _)| crate::clock::remove_timer(*id)).count();
                if stopped == 0 {
                    return Plan::Now(Err("they had run out already".into()));
                }
                let lengths = timers.iter().map(|(_, total)| crate::clock::length(*total)).collect::<Vec<_>>().join(", ");
                Plan::Now(Ok(Done::new(format!("{} stopped", count(stopped, "timer")), lengths, "timer", json!({"done": format!("Stopped {}.", count(stopped, "timer"))}))))
            }
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
        "timer" => timer(&s("action"), &s("length")),
        "note_add" => Plan::Now(note_add(&s("title"), &s("text"), &s("append_to"), cx)),
        "music_control" => Plan::Now(crate::music::control(&s("action")).map(|(words, song)| {
            let icon = match s("action").as_str() {
                "pause" => "pause",
                "next" => "skip_next",
                "previous" => "skip_previous",
                _ => "play_arrow",
            };
            Done::new(words.clone(), song.clone(), icon, json!({"done": format!("Music: {words}."), "song": song}))
        })),
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

/// A timer started, or those running stopped: one at once, several only
/// after a yes.
fn timer(action: &str, length: &str) -> Plan {
    if action != "stop" {
        return Plan::Now(start_timer(length));
    }
    let running = crate::clock::running_timers();
    if running.is_empty() {
        return Plan::Now(Err("no timer is running".into()));
    }
    let wanted = length_of(length);
    let stop: Vec<(u32, f64, f64)> = running.into_iter().filter(|(_, total, _)| wanted.is_none_or(|w| (w - total).abs() < 1.0)).collect();
    match stop.as_slice() {
        [] => Plan::Now(Err(format!("no {} timer is running", crate::clock::length(wanted.unwrap_or(0.0))))),
        [(id, total, left)] => {
            crate::clock::remove_timer(*id);
            let shown = crate::clock::length(*total);
            Plan::Now(Ok(Done::new(format!("timer stopped · {shown}"), format!("{} was left", crate::clock::length(left.ceil())), "timer", json!({"done": format!("Stopped the {shown} timer.")}))))
        }
        several => {
            let each: Vec<String> = several.iter().map(|(_, total, left)| format!("{} ({} left)", crate::clock::length(*total), crate::clock::length(left.ceil()))).collect();
            let (rest, last) = each.split_at(each.len() - 1);
            let which = if several.len() == 2 { "Both timers".to_owned() } else { format!("All {} timers", several.len()) };
            Plan::Ask(Act {
                title: format!("Stop {} timers", several.len()),
                what: format!("{which} in Clock stop and are taken away: {} and {}.", rest.join(", "), last[0]),
                yes: "Stop them".into(),
                no: "Keep them".into(),
                kind: ActKind::StopTimers(several.iter().map(|(id, total, _)| (*id, *total)).collect()),
            })
        }
    }
}

fn start_timer(length: &str) -> Result<Done, String> {
    if length.trim().is_empty() {
        return Err("say how long: “10 min” or “2:30”".into());
    }
    let seconds = length_of(length).ok_or_else(|| format!("“{length}” is not a length Clock reads: say “10 min” or “2:30”"))?;
    crate::clock::add_timer(seconds);
    let shown = crate::clock::length(seconds);
    Ok(Done::new(format!("timer started · {shown}"), "clock counts it down", "timer", json!({"done": format!("A {shown} timer is running in Clock."), "seconds": seconds})))
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
    // Named by the time, as Notes names one; the name is claimed before it
    // is written, so two notes made in the same second never share it.
    let mut name = now.format("%Y-%m-%d-%H%M%S").to_string();
    let path = loop {
        let path = dir.join(format!("{name}.txt"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => name.push('b'),
            Err(_) => return Err("the note could not be written".into()),
        }
    };
    crate::sys::write_atomic(&path, body.as_bytes()).map_err(|_| "the note could not be written".to_owned())?;
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
    let found = crate::music::search_collections(query, kind)?;
    let c = best(&found, query).cloned().ok_or_else(|| format!("nothing on YouTube Music is called “{query}”"))?;
    let opened = crate::music::open_collection(&c)?;
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
    let bytes = crate::music::estimate(&opened.tracks);
    let shown = size(bytes);
    let wait = crate::music::download_wait();
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

// ----------------------------------------------------------------- rules ---

/// How a request begins: the words a clause after "and" or "then" must
/// start with to be a request of its own (else it belongs to the one before:
/// "remind me to grab the cart and the spare").
const STARTS: [&str; 39] = [
    "remind ", "remember to ", "don't forget to ", "do not forget to ", "set ", "add ", "start ", "stop ", "cancel ", "pause", "resume", "unpause", "skip", "next song",
    "previous song", "go back a song", "keep playing", "carry on playing", "play ", "open ", "go to ", "turn ", "switch ", "enable ", "disable ", "change ", "use ", "dim ",
    "brighten ", "brightness", "download ", "install ", "retry ", "note", "make ", "take a note", "write ", "jot down", "when",
];

/// Words before a request that are only polite.
const POLITE: [&str; 16] = [
    "please ", "can you ", "could you ", "would you ", "will you ", "hey ", "ok ", "okay ", "also ", "now ", "then ", "and ", "i want you to ", "i'd like you to ", "go ahead and ", "just ",
];

/// The tools a request calls, read by rules rather than a model: offline (or
/// with no internet), so what can be done on the handheld still is. Plain
/// requests only, one or several joined by "and", "then" or a comma ("start
/// a 10 min timer and remind me to check the bumpers at queueing"); `None`
/// if it is not one, so the question goes on to the logs or the model.
/// "What do you remember" and briefings are the caller's.
pub fn rules(question: &str) -> Option<Vec<(&'static str, Value)>> {
    let q = question.trim().trim_end_matches(['.', '!', '?']).trim();
    let q = polite(q);
    // Memory, whole: "remember our robot is called Numbers", "forget that".
    match memory::command(&q) {
        Some(memory::Command::Remember(fact)) => return Some(vec![("memory_save", json!({"fact": memory::as_kept(&fact), "replaces": ""}))]),
        Some(memory::Command::Forget(what)) => {
            let all = what == "everything";
            return Some(vec![("memory_forget", json!({"fact": if all { String::new() } else { what }, "everything": all}))]);
        }
        Some(memory::Command::List) | None => {}
    }
    // Each part that reads as a request of its own; one that does not is
    // given back to the part before it, as words of that one.
    let mut parts: Vec<String> = Vec::new();
    for (joint, part) in clauses(&q) {
        match parts.last_mut() {
            Some(last) if rule(&polite(&part)).is_none() => {
                last.push_str(&joint);
                last.push_str(&part);
            }
            _ => parts.push(part),
        }
    }
    let calls: Option<Vec<(&'static str, Value)>> = parts.iter().map(|p| rule(&polite(p))).collect();
    calls.filter(|c| !c.is_empty())
}

/// `text` without the polite words before it, or a "please" after.
fn polite(text: &str) -> String {
    let mut t = text.trim();
    loop {
        let before = t;
        for lead in POLITE {
            if t.get(..lead.len()).is_some_and(|h| h.eq_ignore_ascii_case(lead)) {
                t = t[lead.len()..].trim_start();
            }
        }
        for tail in [" please", ", please"] {
            if t.len() > tail.len() && t.get(t.len() - tail.len()..).is_some_and(|h| h.eq_ignore_ascii_case(tail)) {
                t = t[..t.len() - tail.len()].trim_end();
            }
        }
        if t == before {
            return t.to_owned();
        }
    }
}

/// A request cut where another begins: at "and", "then", a comma or a
/// semicolon followed by words that start one. Each part with the joint
/// before it (to give it back whole if it is not one after all).
fn clauses(q: &str) -> Vec<(String, String)> {
    const JOINTS: [&str; 8] = [", and then ", ", and ", ", then ", " and then ", " and ", " then ", "; ", ", "];
    let lower = q.to_lowercase();
    // The words as typed, unless lower case changed their lengths.
    let text = if lower.len() == q.len() { q } else { lower.as_str() };
    let mut out = Vec::new();
    let (mut start, mut joint, mut at) = (0, String::new(), 0);
    while at < lower.len() {
        let next = JOINTS.iter().find(|j| lower[at..].starts_with(*j)).filter(|j| {
            let after = polite(&lower[at + j.len()..]);
            STARTS.iter().any(|s| after.starts_with(s))
        });
        if let Some(j) = next {
            out.push((std::mem::take(&mut joint), text[start..at].to_owned()));
            joint = text[at..at + j.len()].to_owned();
            start = at + j.len();
            at = start;
            continue;
        }
        at += lower[at..].chars().next().map_or(1, char::len_utf8);
    }
    out.push((joint, text[start..].to_owned()));
    out
}

/// The call one request makes, if it is one the rules read.
fn rule(part: &str) -> Option<(&'static str, Value)> {
    let part = part.trim().trim_end_matches(['.', '!', '?', ',']).trim();
    let lower = part.to_lowercase();
    let same = lower.len() == part.len();
    // What follows a beginning, as typed.
    let after = |lead: &str| -> Option<String> {
        lower.starts_with(lead).then(|| if same { &part[lead.len()..] } else { &lower[lead.len()..] }).map(|r| r.trim().to_owned()).filter(|r| !r.is_empty())
    };
    let first = |leads: &[&str]| leads.iter().find_map(|l| after(l));
    // Reminders: "remind me to …", "remember to …", "set a reminder: …".
    let remind = [
        "remind me to ", "remind us to ", "remind me ", "remind us ", "set a reminder to ", "set a reminder for ", "set a reminder: ", "set a reminder ", "add a reminder to ", "add a reminder: ",
        "add a reminder ", "remember to ", "don't forget to ", "do not forget to ",
    ];
    if let Some(text) = first(&remind) {
        // "in 10 min to swap the battery": the words first, as Reminders reads them.
        let text = match text.split_once(" to ") {
            Some((when, what)) if ["in ", "at ", "tomorrow", "tonight", "before ", "after ", "on "].iter().any(|w| when.to_lowercase().starts_with(w)) => format!("{what} {when}"),
            _ => text,
        };
        return Some(("reminder_add", json!({"text": text})));
    }
    // Apps and Settings pages, by name ("open music", "open battery
    // settings"); only what is there ("open the toolbox" is no app).
    if let Some(rest) = first(&["open the ", "open ", "go to the ", "go to "]) {
        let rest = rest.trim_end_matches(" app").to_owned();
        let (app, page) = match rest.to_lowercase().strip_suffix(" settings") {
            Some(page) => ("Settings".to_owned(), page.trim().to_owned()),
            None => (rest, String::new()),
        };
        if open_app(&app, &page).is_ok() {
            return Some(("open_app", json!({"app": app, "page": page})));
        }
    }
    // Downloads: the update, music waiting, or a playlist, album or artist.
    let downloads = ["download ", "install ", "retry ", "resume download", "restart download", "start the download", "start download"];
    if downloads.iter().any(|d| lower.starts_with(d)) {
        if lower.contains("update") {
            return Some(("start_download", json!({"what": "update"})));
        }
        if !lower.starts_with("download ") && !lower.starts_with("install ") {
            return Some(("start_download", json!({"what": "music"})));
        }
        let (kind, name) = match first(&["download songs by ", "download music by ", "download the songs by "]) {
            Some(artist) => ("artist", artist),
            None => {
                let rest = after("download ")?;
                let l = rest.to_lowercase();
                (if l.ends_with(" album") { "album" } else if l.ends_with(" artist") { "artist" } else { "playlist" }, rest)
            }
        };
        let mut query = name;
        for cut in ["my ", "our ", "the "] {
            if query.to_lowercase().starts_with(cut) {
                query = query[cut.len()..].to_owned();
            }
        }
        for cut in [" playlist", " album", " artist"] {
            if query.to_lowercase().ends_with(cut) {
                query.truncate(query.len() - cut.len());
            }
        }
        return Some(("music_download", json!({"query": query.trim(), "kind": kind})));
    }
    // Timers.
    if lower.contains("timer") {
        let stop = ["stop", "cancel", "end ", "clear", "turn off", "kill"].iter().any(|w| lower.starts_with(w));
        return Some(("timer", json!({"action": if stop { "stop" } else { "start" }, "length": length_said(&lower)})));
    }
    // Music: pause, carry on, skip, back; or something to play.
    let control = match lower.as_str() {
        l if l.starts_with("pause") || ["stop the music", "stop music", "stop playing", "stop the song"].contains(&l) => Some("pause"),
        l if ["resume", "unpause", "keep playing", "carry on playing", "continue playing"].iter().any(|w| l.starts_with(w)) => Some("resume"),
        l if l.starts_with("skip") || ["next song", "next track", "play the next song"].iter().any(|w| l.starts_with(w)) => Some("next"),
        l if ["previous song", "previous track", "go back a song", "play the previous song", "last song"].iter().any(|w| l.starts_with(w)) => Some("previous"),
        _ => None,
    };
    if let Some(action) = control {
        return Some(("music_control", json!({"action": action})));
    }
    if let Some(query) = first(&["play me some ", "play some ", "play me ", "play "]) {
        return Some(("music_play", json!({"query": query})));
    }
    // Settings: the screen's brightness, then a switch named and turned.
    if lower.contains("brightness") || lower.starts_with("dim ") || lower.starts_with("brighten ") || lower.contains("screen brighter") || lower.contains("screen dimmer") {
        let number = lower.split(|c: char| !c.is_ascii_digit()).find(|w| !w.is_empty());
        let value = match number {
            Some(n) => Some(n.to_owned()),
            None if lower.contains("max") || lower.contains("full") => Some("max".into()),
            None if lower.contains("up") || lower.contains("bright") => Some("up".into()),
            None if lower.contains("down") || lower.contains("dim") || lower.contains("low") => Some("down".into()),
            None => None,
        };
        if let Some(value) = value {
            return Some(("setting_set", json!({"setting": "brightness", "value": value})));
        }
    }
    let switch = [
        ("battery saver", "battery_saver", false), ("low power", "battery_saver", false), ("wi-fi", "wifi", false), ("wifi", "wifi", false), ("bluetooth", "bluetooth", false),
        ("light mode", "light_mode", false), ("dark mode", "light_mode", true), ("do not disturb", "do_not_disturb", false),
    ];
    if let Some((_, setting, inverted)) = switch.iter().find(|(words, ..)| lower.contains(words)) {
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        // "switch to dark mode", "use light mode": on.
        let to = ["switch to ", "change to ", "use ", "go "].iter().any(|w| lower.starts_with(w));
        let on = to || words.iter().any(|w| matches!(*w, "on" | "enable"));
        let off = words.iter().any(|w| matches!(*w, "off" | "disable"));
        if on != off {
            return Some(("setting_set", json!({"setting": setting, "value": if on != *inverted { "on" } else { "off" }})));
        }
    }
    // Notes: a new one, or lines added to one by name ("add to the pit
    // note: chain retensioned").
    if let Some(rest) = first(&["add to my ", "add to the ", "add to "]) {
        let (title, text) = rest.split_once(':')?;
        let title = title.trim().trim_end_matches(" note").trim();
        return Some(("note_add", json!({"title": "", "text": text.trim(), "append_to": title})));
    }
    let note = [
        "note that ", "note: ", "make a note that ", "make a note: ", "make a note ", "take a note that ", "take a note: ", "take a note ", "write a note that ", "write a note: ", "write a note ",
        "write down that ", "write down ", "jot down ",
    ];
    if let Some(text) = first(&note) {
        return Some(("note_add", json!({"title": "", "text": text, "append_to": ""})));
    }
    // The schedule, asked plainly ("when is our next match").
    let schedule = ["when", "what", "next match", "our next match"].iter().any(|w| lower.starts_with(w)) && lower.contains("next match");
    if schedule && lower.split_whitespace().count() <= 7 || lower.starts_with("when do we queue") {
        return Some(("next_match", json!({})));
    }
    None
}

/// The length said for a timer, as Clock reads it: "10 min", "1 hour 30
/// minutes", "2:30"; empty if none was said.
fn length_said(lower: &str) -> String {
    let text = lower.replace('-', " ");
    if text.contains("half an hour") {
        return "30 min".into();
    }
    const UNITS: [&str; 15] = ["h", "hr", "hrs", "hour", "hours", "m", "min", "mins", "minute", "minutes", "s", "sec", "secs", "second", "seconds"];
    let words: Vec<&str> = text.split_whitespace().map(|w| w.trim_matches([',', '.'])).collect();
    let mut said: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let number = !w.is_empty() && w.chars().all(|c| c.is_ascii_digit() || c == '.') || w == "a" || w == "an";
        if number && words.get(i + 1).is_some_and(|u| UNITS.contains(u)) {
            said.push(format!("{} {}", if w == "a" || w == "an" { "1" } else { w }, words[i + 1]));
            i += 2;
            continue;
        }
        match () {
            // "10min", "2:30", standing alone.
            _ if said.is_empty() && w.starts_with(|c: char| c.is_ascii_digit()) && length_of(w).is_some() => return w.to_owned(),
            _ if !said.is_empty() && w != "and" => break,
            _ => i += 1,
        }
    }
    said.join(" ")
}

/// The Pit Mix download's detail as a test may find it: asked for now ("10
/// songs · about 35 MB"), or by another test first, still on its way ("9
/// songs downloading already": the queue is shared, and saves as it goes).
#[cfg(test)]
pub fn queued_words(detail: &str) -> bool {
    let waiting = detail.split_once(" downloading already").and_then(|(songs, _)| songs.split_whitespace().next()?.parse::<u32>().ok());
    detail.starts_with("10 songs · about 35 MB") || waiting.is_some_and(|n| (1..=10).contains(&n))
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
        let started = now_done(Some(timer("start", "7 min"))).unwrap();
        assert_eq!(started.words, "timer started · 7:00");
        let stopped = now_done(Some(timer("stop", "7 min"))).unwrap();
        assert_eq!(stopped.words, "timer stopped · 7:00");
        assert!(now_done(Some(timer("stop", ""))).is_err(), "none left");
        assert!(now_done(Some(timer("start", "soonish"))).is_err());
        assert_eq!(now_done(Some(timer("start", ""))).unwrap_err(), "say how long: “10 min” or “2:30”");
        // Several running: stopping them all asks first, and a yes stops them.
        now_done(Some(timer("start", "10 min"))).unwrap();
        now_done(Some(timer("start", "5 min"))).unwrap();
        let Plan::Ask(act) = timer("stop", "") else { panic!("several ask first") };
        assert_eq!((act.title.as_str(), act.yes.as_str(), act.no.as_str()), ("Stop 2 timers", "Stop them", "Keep them"));
        assert!(act.what.starts_with("Both timers in Clock stop and are taken away: 10:00 (") && act.what.contains(" and 5:00 ("), "{}", act.what);
        assert_eq!(crate::clock::running_timers().len(), 2, "nothing stopped before the yes");
        let Plan::Now(Ok(done)) = act.run(Path::new("unused")) else { panic!("stopped") };
        assert_eq!((done.words.as_str(), done.detail.as_str()), ("2 timers stopped", "10:00, 5:00"));
        assert!(crate::clock::running_timers().is_empty());
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

    // Found in the simulator's own music catalogue: the handheld's build
    // searches YouTube Music itself.
    #[cfg(not(target_arch = "aarch64"))]
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
                assert!(queued_words(&d.detail), "{}", d.detail);
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
    fn plain_requests_are_read_into_the_same_calls_offline() {
        let one = |q: &str| rules(q).map(|c| c.into_iter().map(|(n, v)| format!("{n} {v}")).collect::<Vec<_>>().join(" | "));
        let cases = [
            ("download my Pit Mix playlist and remind me to swap the battery at queueing for Q42", r#"music_download {"kind":"playlist","query":"Pit Mix"} | reminder_add {"text":"swap the battery at queueing for Q42"}"#),
            ("Start a 10 minute timer", r#"timer {"action":"start","length":"10 minute"}"#),
            ("set a timer for 1 hour and 20 minutes please", r#"timer {"action":"start","length":"1 hour 20 minutes"}"#),
            ("stop the timer", r#"timer {"action":"stop","length":""}"#),
            ("remind me in 10 min to swap the battery", r#"reminder_add {"text":"swap the battery in 10 min"}"#),
            ("remember to charge the spare before Q40", r#"reminder_add {"text":"charge the spare before Q40"}"#),
            // "and" inside a reminder stays in it; "open the toolbox" is no app.
            ("remind me to check the bumpers and the chain", r#"reminder_add {"text":"check the bumpers and the chain"}"#),
            ("remind me to stop by the pit and open the toolbox", r#"reminder_add {"text":"stop by the pit and open the toolbox"}"#),
            ("can you pause the music", r#"music_control {"action":"pause"}"#),
            ("skip this song", r#"music_control {"action":"next"}"#),
            ("play some Daft Punk", r#"music_play {"query":"Daft Punk"}"#),
            ("turn wi-fi off", r#"setting_set {"setting":"wifi","value":"off"}"#),
            ("switch to dark mode", r#"setting_set {"setting":"light_mode","value":"off"}"#),
            ("turn on battery saver, then set brightness to 40", r#"setting_set {"setting":"battery_saver","value":"on"} | setting_set {"setting":"brightness","value":"40"}"#),
            ("open battery settings", r#"open_app {"app":"Settings","page":"battery"}"#),
            ("open music and play lofi", r#"open_app {"app":"music","page":""} | music_play {"query":"lofi"}"#),
            ("note that the chain slipped in Q14", r#"note_add {"append_to":"","text":"the chain slipped in Q14","title":""}"#),
            ("when is our next match?", "next_match {}"),
            ("download the update", r#"start_download {"what":"update"}"#),
            ("resume downloads", r#"start_download {"what":"music"}"#),
            ("remember our robot is called Numbers", r#"memory_save {"fact":"The robot is called Numbers","replaces":""}"#),
            ("forget everything", r#"memory_forget {"everything":true,"fact":""}"#),
        ];
        for (asked, calls) in cases {
            assert_eq!(one(asked).as_deref(), Some(calls), "{asked}");
        }
        for not in ["why did we brown out in Q14?", "what went wrong last match", "open the toolbox", "how is the robot doing?", "what do you remember?", "turn the robot", "play"] {
            assert_eq!(one(not), None, "{not}");
        }
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
