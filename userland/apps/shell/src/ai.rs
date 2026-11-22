//! The assistant: questions about the robot answered by OpenAI's GPT-6.2 Sol
//! or Anthropic's Claude Opus 5.5, with the team's own key, looking at the
//! robot and running the team's saved Actions while it answers.
//!
//! Each service is asked with curl over HTTPS, as the schedule and Weather
//! are: nothing of the model runs on the handheld, so the app costs nothing
//! while it is closed. The key reaches curl in a file only root can read,
//! never on its command line.
//!
//! - OpenAI: the Responses API (`POST /v1/responses`), not stored
//!   (`store: false`). Every turn sends the whole conversation back, its
//!   reasoning items included (they carry the reasoning encrypted), so the
//!   model keeps its train of thought between tool calls.
//! - Claude: the Messages API (`POST /v1/messages`, version 2026-10-21). Its
//!   thinking blocks go back unchanged; a request it declines on safety
//!   grounds is answered by another model on the server (`fallbacks:
//!   "default"`) rather than failing outright.
//!
//! Effort is "light" (low) or "medium", as chosen; medium by default.
//!
//! - On this handheld: the on-device model (`crate::local`, llama.cpp's
//!   server on 127.0.0.1) through OpenAI-style chat completions. No tools and
//!   no web: short answers, offline.
//!
//! Its tools: what the robot publishes (FRC Catalyst under `/Catalyst/`), the
//! Catalyst Agent's report from the Systemcore, the match logs pulled, the
//! event schedule, the values on the live graphs, text files under /data, the
//! handheld's own state, and the saved Actions. Looking is free. An Action
//! that could change something (`commands::Saved::critical`) waits for the
//! person to say yes, and a no goes back to the model as a no.
//!
//! The web: both services search and read pages on their own servers
//! (Claude's `web_search` and `web_fetch` server tools; OpenAI's hosted
//! `web_search`). What they come back with is kept in the conversation
//! exactly as it came (search results carry encrypted content the service
//! needs again), the steps are shown as rows, and the pages an answer rests
//! on are listed under it: Enter opens one in the Web app. Browse
//! (`crate::browse`) is the same engine as a browser.
//!
//! Conversations are kept (`/data/assistant/chats`, the last 20) in the
//! service's own form, so one reopened carries on where it stopped.
//!
//! It acts on the handheld too (`device`): reminders, timers, notes, music,
//! apps, settings, downloads, several in one turn, each a row as it is
//! done, with a live activity while it works; what could change something
//! asks first on the confirm page. Offline, or with no internet, plain
//! requests are read by rules into the same tools. It briefs (`briefing`):
//! the next match or the day at a glance, made by a template offline. And it
//! remembers what it is told to (`memory`), in a short plain file the person
//! can read and delete, never a secret, that leaves the handheld only with
//! a question to OpenAI or Claude.
//!
//! The simulator answers from a script with the demo robot, unless told to
//! ask the real services (`CATALYST_AI_LIVE`).

pub mod briefing;
pub mod device;
pub mod memory;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::commands::Saved;
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::keys::Keys;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const OPENAI_URL: &str = "https://api.openai.com/v1/responses";
const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_VERSION: &str = "2026-10-21";
/// Server-side fallback on a refusal (`fallbacks: "default"`).
const CLAUDE_BETA: &str = "server-side-fallback-2026-07-01";
/// Room for thinking and the answer: OpenAI suggests at least 25,000 for its
/// reasoning models; 16,000 keeps a Claude request well inside a minute.
const OPENAI_MAX_OUTPUT: u32 = 25_000;
const CLAUDE_MAX_TOKENS: u32 = 16_000;
/// An answer this long in the writing is a notification once written (one
/// quicker is read where it was asked).
const LONG_ANSWER: std::time::Duration = std::time::Duration::from_secs(10);
/// How long one request may take before it is given up.
const REQUEST_SECONDS: u32 = 180;
/// Rounds of tool calls for one question before the assistant stops to ask.
const ROUNDS: usize = 12;
/// How long an Action may run for the assistant, and how much of what it
/// printed goes back (its end, where the result usually is).
const ACTION_SECONDS: u64 = 60;
const OUTPUT_KEEP: usize = 6000;
/// Values one look at the robot returns at most.
const VALUES_AT_MOST: usize = 150;
/// The simulator's scripted service waits this long, as a real one would.
pub(crate) const DEMO_PAUSE: f32 = 0.9;
/// The on-device model answers in at most this many tokens: it is slow.
const LOCAL_MAX_TOKENS: u32 = 400;
/// How long the on-device model may take for one answer.
const LOCAL_SECONDS: u32 = 300;
/// Times a paused turn (the service's own web loop ran long) is carried on
/// before the assistant stops to ask.
const CONTINUATIONS: usize = 5;
/// Searches and page reads one request may make, and how much of a page read
/// comes into the conversation (tokens; pages are cut there).
const WEB_USES: u32 = 5;
const FETCH_TOKENS: u32 = 20_000;
/// Conversations kept.
const CHATS_KEPT: usize = 20;

const SYSTEM: &str = "You are the assistant on Catalyst Linux, a pocket Linux handheld (an M5Stack CardputerZero) that an FRC robotics team carries in the pit and the stands.

Its screen is 320 by 170 pixels. Answer in plain sentences and keep it short: usually two to five sentences, at most about 80 words unless asked for more. No Markdown: no headings, bullets, bold, tables or code blocks. If a list helps, put each item on its own short line.

The team's robot runs FRC Catalyst, which publishes its telemetry under /Catalyst/ in NetworkTables, and the Catalyst Agent on the robot's Systemcore reports the robot program, its recent log, CPU and memory. Look with the tools before you answer anything about the robot, and never guess a value: if something is not published, say so. Say plainly when the robot is not linked. The event schedule, the live graphs, text files under /data and the handheld's own state are there to look at too.

You can search the web and read pages. Use them for anything current or outside this handheld: FRC rules and the game manual, parts and their specifications, WPILib and vendor documentation, events and results. Say where an answer came from; the handheld lists the pages under it.

Actions are the team's saved shell commands on this handheld. Run one when it answers the question or the person asks for it. Some are critical because they could change something; the handheld asks the person before those run, and they may decline. Then say what it would have done and leave it. Do not run a critical Action that the person did not ask for and the question does not need.

You can act on the handheld when the person asks: set reminders (in their own words, match cues included), start or stop timers, write notes, play music or download a playlist, album or artist, open an app or a Settings page, change a setting, start a waiting download. Do everything they asked in one turn, calling the tools together when they do not depend on each other; each shows on the screen as it is done. Changes that could matter (Wi-Fi, Bluetooth, a system update, forgetting everything) are shown to the person first, and a no means leave it. Afterwards say briefly what was done and anything that is waiting, in a sentence or two.

When asked to brief them, call briefing: the handheld shows its data as a card, so say only the one to three things that matter most.

When the person tells you something to remember, or a lasting preference or fact about themselves or the team (\"remember our robot is called Numbers\", \"I like short answers\", \"my team is 5805\"), keep it with memory_save; \"forget that\" is memory_forget. Never keep a password, passcode, key or token, and say so if asked to.";

/// For the on-device model asked something not about the robot: small,
/// offline, with nothing to look at (questions about the robot go through
/// `crate::offline`, which gives it the facts).
pub(crate) const LOCAL_SYSTEM: &str = "You are a small assistant running offline on a pocket Linux handheld that an FRC robotics team uses. Answer in two or three short, plain sentences. You cannot see the robot, the web or any files: if the question needs them, say so in one sentence.";

/// The on-device model's prompt, with the tiny version of what it was told
/// to remember.
pub(crate) fn local_system() -> String {
    format!("{LOCAL_SYSTEM}{}", memory::tiny(&memory::load(&memory::path())))
}

/// For Browse: the page or the answer, in a form the handheld lays out.
pub(crate) const BROWSE_SYSTEM: &str = "You are the web browser on Catalyst Linux, a pocket handheld with a 320 by 170 pixel screen and a keyboard, used by an FRC robotics team.

When the person gives an address, read that page with web_fetch (or open it) and give it back as readable text: its main content in short paragraphs, with headings where the page has sections, leaving out navigation, adverts, cookie notices and other boilerplate. Keep the page's own words where you can; shorten long pages to what matters. When the person asks a question, search the web and answer it briefly from what you read.

Write in exactly this form and nothing else:
TITLE: the page's title, or the question answered in a few words
then the text: paragraphs separated by a blank line, a line starting with \"## \" for a heading, no other Markdown.
Last, a line \"LINKS:\" and then up to twelve links worth following (from the page, or the sources you used), one a line as: short description | full URL";

// -------------------------------------------------------------- services ---

/// Who answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    OpenAi,
    Claude,
    /// The on-device model (`crate::local`).
    Local,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::OpenAi, Service::Claude, Service::Local];

    /// Its name in the prefs file, and (for the two services) its key's
    /// name in the keys file.
    pub fn key(self) -> &'static str {
        match self {
            Service::OpenAi => "openai",
            Service::Claude => "claude",
            Service::Local => "local",
        }
    }

    /// The model as people know it; on this handheld with no model
    /// downloaded, the answers come from the logs alone.
    pub fn model_name(self) -> &'static str {
        match self {
            Service::OpenAi => "GPT-6.2 Sol",
            Service::Claude => "Claude Opus 5.5",
            Service::Local if crate::local::ready() => crate::local::model_name(),
            Service::Local => "On device",
        }
    }

    /// The same, lower case, for mono labels.
    pub fn label(self) -> &'static str {
        match self {
            Service::OpenAi => "gpt-6.2 sol",
            Service::Claude => "opus 5.5",
            Service::Local => "on device",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Service::OpenAi => "gpt-6.2-sol",
            Service::Claude => "claude-opus-5-5",
            Service::Local => "local",
        }
    }

    fn company(self) -> &'static str {
        match self {
            Service::OpenAi => "OpenAI",
            Service::Claude => "Anthropic",
            Service::Local => "The on-device model",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Service::OpenAi => OPENAI_URL,
            Service::Claude => CLAUDE_URL,
            Service::Local => "",
        }
    }

    /// A service on the internet, with tools and the web; the on-device
    /// model has neither.
    pub fn online(self) -> bool {
        self != Service::Local
    }
}

/// How hard the model thinks before it answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effort {
    Light,
    #[default]
    Medium,
}

impl Effort {
    fn wire(self) -> &'static str {
        match self {
            Effort::Light => "low",
            Effort::Medium => "medium",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Effort::Light => "light",
            Effort::Medium => "medium",
        }
    }

    fn next(self) -> Effort {
        match self {
            Effort::Light => Effort::Medium,
            Effort::Medium => Effort::Light,
        }
    }
}

/// What was chosen: the service (when both keys are set) and the effort.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub service: Option<Service>,
    pub effort: Effort,
}

fn prefs_path() -> PathBuf {
    crate::config_dir().join("assistant")
}

impl Prefs {
    pub fn load() -> Self {
        std::fs::read_to_string(prefs_path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let mut prefs = Prefs::default();
        for (name, value) in text.lines().filter_map(|l| l.split_once('=')) {
            match (name.trim(), value.trim()) {
                ("service", v) => prefs.service = Service::ALL.into_iter().find(|s| s.key() == v),
                ("effort", "light") => prefs.effort = Effort::Light,
                ("effort", "medium") => prefs.effort = Effort::Medium,
                _ => {}
            }
        }
        prefs
    }

    pub fn save(&self) -> bool {
        let text = format!("service={}\neffort={}\n", self.service.map_or("", Service::key), self.effort.name());
        std::fs::create_dir_all(crate::config_dir()).ok();
        let saved = crate::sys::write_atomic(&prefs_path(), text.as_bytes()).is_ok();
        TILE.with(|t| *t.borrow_mut() = None);
        saved
    }
}

/// The service that answers, and its key: the one chosen if it can answer
/// (its key set), else the first that can, the two services before this
/// handheld. This handheld always can: offline, from the match logs
/// (`crate::offline`), its model wording the answers once one is downloaded.
pub fn answering(prefs: &Prefs, keys: &Keys) -> Option<(Service, String)> {
    let can = |s: Service| match s {
        Service::Local => Some((s, String::new())),
        _ => keys.get(s.key()).map(|k| (s, k.to_owned())),
    };
    prefs.service.and_then(can).or_else(|| Service::ALL.into_iter().find_map(can))
}

/// The services that can answer now.
pub fn available(keys: &Keys) -> Vec<Service> {
    Service::ALL.into_iter().filter(|s| *s == Service::Local || keys.get(s.key()).is_some()).collect()
}

thread_local! {
    /// Home's line for the tile and when it was worked out: the files are
    /// read again at most every few seconds, not every frame.
    static TILE: RefCell<Option<(Instant, String)>> = const { RefCell::new(None) };
}

/// Home's tile: which model answers, or that a key is wanted.
pub fn tile() -> (String, Option<&'static str>) {
    TILE.with(|cell| {
        let mut cell = cell.borrow_mut();
        if !cell.as_ref().is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(5)) {
            let line = match answering(&Prefs::load(), &Keys::load()) {
                Some((service, _)) => service.label().to_owned(),
                None => "no key yet".to_owned(),
            };
            *cell = Some((Instant::now(), line));
        }
        (cell.as_ref().map(|(_, l)| l.clone()).unwrap_or_default(), None)
    })
}

/// The simulator answers from a script unless told otherwise, and so do the
/// tests, the handheld's own ARM build of them too (which has no key).
pub(crate) fn demo() -> bool {
    (cfg!(test) || !cfg!(target_arch = "aarch64")) && std::env::var_os("CATALYST_AI_LIVE").is_none()
}

// ----------------------------------------------------------------- tools ---

/// The tools, as (name, what it does, its input's JSON schema).
fn tool_specs() -> Vec<(&'static str, &'static str, Value)> {
    let nothing = || json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false});
    let mut specs = vec![
        (
            "robot_status",
            "How the robot stands right now: whether it is linked to this handheld and how, its address and team, the round trip, and every health check firing (FRC Catalyst's alerts, worst first). Also counts the values in each group the robot publishes, to read with robot_values.",
            nothing(),
        ),
        (
            "robot_values",
            "The values the robot publishes in one group of FRC Catalyst's telemetry (NetworkTables, under /Catalyst/), by key. Groups are like \"Loop/\", \"Devices/Motors/\", \"Health/\" or \"OpMode/\"; robot_status lists them. An empty prefix lists the groups.",
            json!({"type": "object", "properties": {"prefix": {"type": "string", "description": "A group under /Catalyst/, ending in a slash, such as \"Loop/\" or \"Devices/Motors/\"; empty for the list of groups."}}, "required": ["prefix"], "additionalProperties": false}),
        ),
        (
            "robot_agent",
            "The Catalyst Agent's report from the robot's Systemcore: the robot program's state, how long it has run, its restarts, memory and its recent log lines, plus the machine's CPU load, memory and uptime. Needs the robot reachable over the network.",
            nothing(),
        ),
        (
            "match_logs",
            "The match logs pulled from the robot onto this handheld, each with its match and time and what stood out: faults and warnings, the lowest battery voltage, the worst loop time and overruns, brownouts, the hottest mechanism and the busiest CAN bus.",
            nothing(),
        ),
        (
            "match_facts",
            "One match's facts, worked out from its log on this handheld: each fault and measure with its numbers and its time into the match (brownouts and the battery, e-stops and lost comms, CAN, the loop, each motor's heat and current, stalls, mechanisms short of their setpoints, auto, the code it ran), worst first, and how the match went if the schedule knows. The match by its label as match_logs gives it (\"Q14\"), or \"latest\".",
            json!({"type": "object", "properties": {"match": {"type": "string", "description": "A match's label, such as \"Q14\", or \"latest\"."}}, "required": ["match"], "additionalProperties": false}),
        ),
        (
            "event_schedule",
            "The team's event and its matches from The Blue Alliance (and FRC Nexus when its key is set): the next match and when to queue, each match's alliances and score, and what is queueing now.",
            nothing(),
        ),
        (
            "graph_values",
            "The values on the live graphs this handheld is recording from the robot: each key with its recent samples (seconds ago, value), newest last.",
            nothing(),
        ),
        (
            "read_file",
            "A text file on this handheld under /data (notes, logs, robot code, configuration), read only: its first 64 KB. Secrets (keys, Wi-Fi passwords, SSH and Tailscale state) are not given out.",
            json!({"type": "object", "properties": {"path": {"type": "string", "description": "The file's path, under /data, e.g. /data/notes/pit.txt; a folder lists what is in it."}}, "required": ["path"], "additionalProperties": false}),
        ),
        (
            "handheld_status",
            "This handheld itself: its battery, whether Wi-Fi and Ethernet are up and their addresses, storage left, memory, temperature, uptime and the system's version.",
            nothing(),
        ),
        (
            "list_actions",
            "The team's saved Actions: shell commands on this handheld, each with its name, its command, and whether it is critical (could change something, so the person is asked before it runs).",
            nothing(),
        ),
        (
            "run_action",
            "Run one saved Action by its exact name and get what it printed and its exit code (it may run for up to a minute). A critical one is shown to the person first, who may decline.",
            json!({"type": "object", "properties": {
                "name": {"type": "string", "description": "The Action's name exactly as list_actions gives it."},
                "reason": {"type": "string", "description": "Why you are running it, in a few words the person will read."}
            }, "required": ["name", "reason"], "additionalProperties": false}),
        ),
    ];
    // Those that act on the handheld (and brief, and remember).
    specs.extend(device::specs());
    specs
}

/// The robot's address without a port: the robot's own, else the team's, else
/// (for the demo robot) the address its team's would be.
fn host(robot: &Robot) -> Option<String> {
    let address = crate::tools::robot_address(robot).or_else(|| robot.is_demo().then(|| "10.58.5.2".to_owned()))?;
    Some(address.split(':').next().unwrap_or_default().to_owned())
}

fn robot_status(robot: &Robot) -> Value {
    let alerts: Vec<Value> = robot
        .firing()
        .into_iter()
        .map(|(severity, subsystem, check, detail)| json!({"severity": severity, "subsystem": subsystem, "check": check, "detail": detail}))
        .collect();
    json!({
        "linked": robot.linked(),
        "link": robot.status(),
        "address": host(robot),
        "team": crate::tools::team(robot),
        "roundTripMs": robot.rtt_ms(),
        "demoRobot": robot.is_demo(),
        "alertsFiring": alerts,
        "groups": groups(robot),
    })
}

/// How many values each top-level group holds.
fn groups(robot: &Robot) -> Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (key, _) in robot.under("") {
        let group = key.split_once('/').map_or_else(|| key.clone(), |(g, _)| format!("{g}/"));
        *counts.entry(group).or_default() += 1;
    }
    json!(counts)
}

fn robot_values(robot: &Robot, prefix: &str) -> Value {
    let prefix = prefix.trim().trim_start_matches("/Catalyst/").trim_start_matches('/');
    if prefix.is_empty() {
        return json!({"groups": groups(robot)});
    }
    let all = robot.under(prefix);
    let values: serde_json::Map<String, Value> = all.iter().take(VALUES_AT_MOST).map(|(k, v)| (k.clone(), value_json(v))).collect();
    json!({"prefix": prefix, "count": all.len(), "shown": values.len(), "values": values})
}

/// A NetworkTables value as JSON: numbers to four places, arrays to their
/// first 32, raw payloads by size.
fn value_json(v: &nt4::Value) -> Value {
    use nt4::Value as V;
    let number = |x: f64| if x.is_finite() { json!((x * 1e4).round() / 1e4) } else { Value::Null };
    match v {
        V::Bool(b) => json!(b),
        V::Double(x) => number(*x),
        V::Float(x) => number(f64::from(*x)),
        V::Int(i) => json!(i),
        V::String(s) => json!(s),
        V::Raw(bytes) => json!(format!("{} bytes", bytes.len())),
        V::BoolArray(a) => json!(a.iter().take(32).collect::<Vec<_>>()),
        V::DoubleArray(a) => Value::Array(a.iter().take(32).map(|x| number(*x)).collect()),
        V::IntArray(a) => json!(a.iter().take(32).collect::<Vec<_>>()),
        V::FloatArray(a) => Value::Array(a.iter().take(32).map(|x| number(f64::from(*x))).collect()),
        V::StringArray(a) => json!(a.iter().take(32).collect::<Vec<_>>()),
    }
}

fn match_logs(robot: &Robot) -> Value {
    let logs: Vec<Value> = robot
        .logs
        .list()
        .iter()
        .take(20)
        .map(|row| {
            let mut log = json!({"match": row.title(), "file": row.name, "bytes": row.bytes});
            if let Some(d) = &row.digest {
                log["seconds"] = json!(d.seconds.round());
                log["enabledSeconds"] = json!(d.enabled_seconds.map(f64::round));
                log["lowestBatteryVolts"] = json!(d.battery_low);
                log["worstLoopMs"] = json!(d.worst_loop_ms);
                log["loopOverruns"] = json!(d.loop_overruns);
                log["brownouts"] = json!(d.brownouts);
                log["hottest"] = json!(d.hottest.as_ref().map(|(name, c)| json!({"mechanism": name, "celsius": c})));
                log["busiestCanBus"] = json!(d.can_peak);
                log["stoodOut"] = json!(d.highlights.iter().map(|(level, text)| format!("{}: {text}", format!("{level:?}").to_lowercase())).collect::<Vec<_>>());
            }
            log
        })
        .collect();
    json!({"status": robot.logs.status(), "logs": logs})
}

/// A match's sheet of facts (`crate::offline::facts`), for the services.
fn match_facts(robot: &Robot, label: &str) -> Result<String, String> {
    let lib = crate::offline::load(&crate::offline::Source::of(robot), &mut |_| {});
    let label = label.trim();
    let i = if label.is_empty() || label.eq_ignore_ascii_case("latest") { Some(0) } else { lib.index_of(label) };
    let Some(m) = i.and_then(|i| lib.matches.get(i)) else {
        return Err(format!("no log of {label} on this handheld; match_logs lists those there are"));
    };
    Ok(json!({
        "match": m.label,
        "file": m.log,
        "seconds": m.seconds,
        "enabledSeconds": m.enabled_seconds,
        "code": m.code,
        "autoRoutine": m.routine,
        "outcome": lib.outcomes.get(&m.label),
        "facts": m.facts.iter().map(|f| json!({"level": format!("{:?}", f.level).to_lowercase(), "fact": f.text})).collect::<Vec<_>>(),
        "timesAre": "minutes:seconds into the match, from when the robot was first enabled",
    })
    .to_string())
}

fn list_actions(actions: &[Saved]) -> Value {
    json!(actions.iter().map(|s| json!({"name": s.name, "command": s.command, "critical": s.critical()})).collect::<Vec<_>>())
}

fn event_schedule(robot: &Robot) -> Value {
    let Some(view) = robot.schedule.view() else {
        return json!({"known": false, "why": "no schedule yet: it needs the team number, Wi-Fi with internet and a Blue Alliance key"});
    };
    let now = chrono::Utc::now().timestamp();
    let when = |t: Option<i64>| t.map(|t| chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&chrono::Local).format("%a %H:%M").to_string()).unwrap_or_default());
    let matches: Vec<Value> = view
        .matches
        .iter()
        .take(40)
        .map(|m| {
            json!({"match": m.label, "red": m.red, "blue": m.blue, "time": when(m.time), "queue": when(m.queue_at), "score": m.score.map(|(r, b)| json!({"red": r, "blue": b})), "status": m.status})
        })
        .collect();
    json!({
        "team": view.team,
        "event": view.event.as_ref().map(|e| json!({"name": e.name, "key": e.key, "start": e.start, "end": e.end})),
        "next": view.next(now).map(|m| m.label.clone()),
        "nowQueuing": view.now_queuing,
        "matches": matches,
        "problem": view.problem,
    })
}

fn graph_values(robot: &Robot) -> Value {
    let rec = &robot.graphs;
    let now = rec.now();
    let graphs: Vec<Value> = rec
        .keys()
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let points = rec.points(i).map(|p| p.iter().rev().take(30).rev().map(|(t, v)| json!([((now - t) * 10.0).round() / 10.0, (v * 1e4).round() / 1e4])).collect::<Vec<_>>()).unwrap_or_default();
            json!({"key": key, "samples": points})
        })
        .collect();
    json!({"graphs": graphs, "samplesAre": "[seconds ago, value]"})
}

/// Files under /data that are never given out: keys, passwords, private
/// keys and the like.
fn secret(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["/catalyst/keys", "/wifi/", "/ssh/", "/tailscale/", "/update/", ".assistant-request", "/.ssh/", ".git-credentials", "/assistant/chats"].iter().any(|s| text.contains(s))
        || name.starts_with("id_")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.contains("secret")
        || name.contains("token")
        || name == "keys"
}

/// The /data a path must be under: the handheld's, or the simulator's stand-in.
fn data_root() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data")
    } else {
        crate::config_dir()
    }
}

/// A text file under /data (its first 64 KB), or a folder's contents.
fn read_file(path: &str) -> Result<String, String> {
    const MOST: u64 = 64 * 1024;
    let root = data_root();
    let asked = std::path::Path::new(path.trim());
    let relative = asked.strip_prefix("/data").unwrap_or(asked);
    let full = root.join(relative.strip_prefix("/").unwrap_or(relative));
    let real = full.canonicalize().map_err(|_| format!("{path}: no such file"))?;
    let root = root.canonicalize().unwrap_or(root);
    if !real.starts_with(&root) {
        return Err("only files under /data can be read".into());
    }
    if secret(&real) {
        return Err(format!("{path} holds secrets: it is not given out"));
    }
    if real.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&real)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|e| !secret(&e.path()))
            .map(|e| format!("{}{}", e.file_name().to_string_lossy(), if e.path().is_dir() { "/" } else { "" }))
            .collect();
        names.sort();
        names.truncate(200);
        return Ok(json!({"folder": path, "entries": names}).to_string());
    }
    let size = real.metadata().map_err(|e| e.to_string())?.len();
    let mut bytes = Vec::new();
    std::fs::File::open(&real).and_then(|f| f.take(MOST).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.iter().take(4096).any(|b| *b == 0) {
        return Err(format!("{path} is not text ({size} bytes)"));
    }
    Ok(json!({"path": path, "bytes": size, "cut": size > MOST, "text": String::from_utf8_lossy(&bytes)}).to_string())
}

fn handheld_status() -> Value {
    json!({
        "version": crate::sysupdate::version(),
        "battery": crate::sys::battery(),
        "minutesLeft": crate::sys::battery_minutes_left(),
        "wifiOn": crate::wifi::switched_on(),
        "wifiAddress": crate::sys::address("wlan0"),
        "ethernetUp": crate::sys::link_up("eth0"),
        "ethernetAddress": crate::sys::address("eth0"),
        "storage": crate::sys::storage(),
        "memory": crate::sys::memory(),
        "processorTemperature": crate::sys::cpu_temperature(),
        "uptime": crate::sys::uptime(),
        "time": chrono::Local::now().format("%Y-%m-%d %H:%M %Z").to_string(),
    })
}

/// A call the model made.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// What a call came to, to send back.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub id: String,
    pub text: String,
    pub error: bool,
}

/// What answering a call takes.
enum Plan {
    /// Answered at once, from what the shell already holds.
    Now(Result<String, String>),
    /// The Catalyst Agent asked, on a thread.
    Agent(String),
    /// An Action run, on a thread, with the model's reason for it; a
    /// critical one only once the person says yes.
    Run { saved: Saved, line: String, reason: String, ask: bool },
    /// A tool that acts on the handheld (`device`).
    Device(device::Plan),
}

fn plan(call: &Call, robot: &Robot, actions: &[Saved], cx: &device::Ctx) -> Plan {
    if let Some(p) = device::plan(call, cx) {
        return Plan::Device(p);
    }
    match call.name.as_str() {
        "robot_status" => Plan::Now(Ok(robot_status(robot).to_string())),
        "robot_values" => Plan::Now(Ok(robot_values(robot, call.input["prefix"].as_str().unwrap_or_default()).to_string())),
        "robot_agent" => match host(robot) {
            Some(host) => Plan::Agent(host),
            None => Plan::Now(Err("no robot address yet: the team or robot is set in the Robot app".into())),
        },
        "match_logs" => Plan::Now(Ok(match_logs(robot).to_string())),
        "match_facts" => Plan::Now(match_facts(robot, call.input["match"].as_str().unwrap_or_default())),
        "event_schedule" => Plan::Now(Ok(event_schedule(robot).to_string())),
        "graph_values" => Plan::Now(Ok(graph_values(robot).to_string())),
        "read_file" => Plan::Now(read_file(call.input["path"].as_str().unwrap_or_default())),
        "handheld_status" => Plan::Now(Ok(handheld_status().to_string())),
        "list_actions" => Plan::Now(Ok(list_actions(actions).to_string())),
        "run_action" => {
            let name = call.input["name"].as_str().unwrap_or_default().trim();
            let found = actions.iter().find(|s| s.name == name).or_else(|| actions.iter().find(|s| s.name.eq_ignore_ascii_case(name)));
            let Some(saved) = found else {
                return Plan::Now(Err(format!("no Action is called \"{name}\"; list_actions gives their names")));
            };
            match crate::commands::expand(&saved.command, host(robot).as_deref(), crate::tools::team(robot)) {
                Ok(line) => Plan::Run { saved: saved.clone(), line, reason: call.input["reason"].as_str().unwrap_or_default().trim().to_owned(), ask: saved.critical() },
                Err(why) => Plan::Now(Err(why)),
            }
        }
        other => Plan::Now(Err(format!("there is no tool called {other}"))),
    }
}

/// The Catalyst Agent's report, its log cut to the last lines.
fn ask_agent(host: &str) -> Result<String, String> {
    let mut report = if demo() { crate::agent::Agent::demo().snapshot().ok_or("the agent did not answer")? } else { crate::agent::get(host, "/api/system")? };
    if let Some(log) = report.pointer_mut("/robotProgram/log").and_then(Value::as_array_mut) {
        let cut = log.len().saturating_sub(40);
        log.drain(..cut);
    }
    Ok(report.to_string())
}

/// The end of `text`, at most `keep` characters.
fn tail(text: &str, keep: usize) -> String {
    let count = text.chars().count();
    if count <= keep {
        return text.to_owned();
    }
    let rest: String = text.chars().skip(count - keep).collect();
    format!("…{rest}")
}

/// Where what the assistant ran is kept: in /data on the handheld, where
/// Files opens (a file a run, newest last by name).
fn runs_dir() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/assistant")
    } else {
        crate::config_dir().join("assistant-runs")
    }
}

/// Runs kept before the oldest go.
const RUNS_KEPT: usize = 200;

/// Keep a record of a run: what, when, why, who asked, and what it printed.
fn keep_run(dir: &std::path::Path, name: &str, line: &str, reason: &str, by: &str, approved: bool, result: &Result<String, String>) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let now = chrono::Local::now();
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let path = dir.join(format!("{}-{}.txt", now.format("%Y%m%d-%H%M%S"), slug.trim_matches('-')));
    let (ending, output) = match result {
        Ok(json) => {
            let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
            let ending = match (v["exitCode"].as_i64(), v["timedOutAfterSeconds"].as_u64()) {
                (Some(code), _) => format!("exit {code}"),
                (None, Some(s)) => format!("stopped after {s} s"),
                _ => "ended".to_owned(),
            };
            (ending, v["output"].as_str().unwrap_or_default().to_owned())
        }
        Err(why) => ("did not run".to_owned(), why.clone()),
    };
    let asked = if approved { " · you said yes" } else { "" };
    let text = format!("{name}\n{} · run by the assistant ({by}){asked}\nwhy: {reason}\n$ {line}\n{ending}\n\n{output}\n", now.format("%Y-%m-%d %H:%M:%S"));
    crate::sys::write_atomic(&path, text.as_bytes()).ok()?;
    // The oldest go once there are too many.
    let mut kept: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    kept.sort();
    for old in kept.iter().take(kept.len().saturating_sub(RUNS_KEPT)) {
        std::fs::remove_file(old).ok();
    }
    Some(path)
}

/// Run an Action's command, what it prints and its errors together, for at
/// most a minute.
fn run_command(line: &str) -> Result<String, String> {
    if demo() {
        return Ok(json!({"exitCode": 0, "output": demo_output(line)}).to_string());
    }
    let mut command = Command::new("sh");
    command.arg("-c").arg(format!("exec 2>&1; {line}")).stdin(Stdio::null()).stdout(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a minute's limit stops all it started.
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| "the command could not start".to_owned())?;
    let mut stdout = child.stdout.take().ok_or("the command could not start")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).ok();
        tx.send(out).ok();
    });
    let started = Instant::now();
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(ACTION_SECONDS) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            child.kill().ok();
            child.wait().ok();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let out = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    let output = tail(String::from_utf8_lossy(&out).trim_end(), OUTPUT_KEEP);
    Ok(match status {
        Some(status) => json!({"exitCode": status.code(), "output": output}),
        None => json!({"timedOutAfterSeconds": ACTION_SECONDS, "output": output}),
    }
    .to_string())
}

// ------------------------------------------------------------------ chat ---

/// What came back from one request.
#[derive(Debug, PartialEq)]
pub enum Stop {
    /// The answer is complete.
    Answered,
    /// The model wants its tools first.
    Tools,
    /// The service's own loop (searching, reading) ran long and paused: the
    /// conversation goes back as it is and it carries on.
    Paused,
    /// It would not answer, and why.
    Refused(String),
    /// It ran out of room.
    Cut,
}

/// A page an answer rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub title: String,
    pub url: String,
}

/// Something the service did on its own servers while answering: searched
/// the web, read a page. Its words, and whether it worked.
#[derive(Clone, Debug, PartialEq)]
pub struct WebStep {
    pub words: String,
    pub ok: bool,
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<Call>,
    pub stop: Stop,
    pub steps: Vec<WebStep>,
    pub sources: Vec<Source>,
}

impl Reply {
    fn stopped(stop: Stop) -> Self {
        Reply { text: String::new(), calls: Vec::new(), stop, steps: Vec::new(), sources: Vec::new() }
    }
}

/// What a conversation is for: the assistant (robot tools, Actions, the
/// web) or Browse (the web only, pages given back in Browse's own form).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Assistant,
    Browse,
}

/// The conversation as the service keeps it: its own items, sent back whole
/// each turn and never edited, so reasoning (and search results' encrypted
/// content) carried in them stays valid.
pub struct Chat {
    service: Service,
    mode: Mode,
    items: Vec<Value>,
    /// Its system prompt, with what was remembered when it began: the same
    /// for the whole conversation, so the service's cache of it holds.
    system: String,
}

/// A page's host, for a step's words: "thebluealliance.com".
pub(crate) fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.trim_start_matches("www.").to_owned()
}

/// Sources in order, each page once, at most `n`.
fn dedupe(sources: Vec<Source>, n: usize) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for s in sources {
        if !s.url.is_empty() && !out.iter().any(|o| o.url == s.url) {
            out.push(Source { title: if s.title.trim().is_empty() { host_of(&s.url) } else { s.title.trim().to_owned() }, url: s.url });
        }
    }
    out.truncate(n);
    out
}

/// Sources shown under an answer at most.
const SOURCES_SHOWN: usize = 6;

impl Chat {
    /// A conversation that remembers nothing (the tests).
    #[cfg(test)]
    pub fn new(service: Service) -> Self {
        Self::for_mode(service, Mode::Assistant)
    }

    pub fn for_mode(service: Service, mode: Mode) -> Self {
        Self::with_memory(service, mode, &[])
    }

    /// A conversation that knows what the person asked it to remember.
    pub fn with_memory(service: Service, mode: Mode, remembered: &[String]) -> Self {
        Chat { service, mode, items: Vec::new(), system: Self::system_for(service, mode, remembered) }
    }

    fn system_for(service: Service, mode: Mode, remembered: &[String]) -> String {
        match (service, mode) {
            (Service::Local, _) => format!("{LOCAL_SYSTEM}{}", memory::tiny(remembered)),
            (_, Mode::Browse) => BROWSE_SYSTEM.to_owned(),
            (_, Mode::Assistant) => format!("{SYSTEM}{}", memory::for_prompt(remembered)),
        }
    }

    pub fn service(&self) -> Service {
        self.service
    }

    pub fn ask(&mut self, text: &str) {
        self.items.push(json!({"role": "user", "content": text}));
    }

    /// An answer made on this handheld, kept in the conversation as said.
    fn said(&mut self, text: &str) {
        self.items.push(json!({"role": "assistant", "content": text}));
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Back to how it was, `len` items long: a question that failed is gone.
    pub fn rewind(&mut self, len: usize) {
        self.items.truncate(len);
    }

    /// The items, as kept on disk.
    fn items(&self) -> &[Value] {
        &self.items
    }

    /// A conversation again from its kept items.
    fn with_items(service: Service, mode: Mode, items: Vec<Value>, remembered: &[String]) -> Self {
        Chat { service, mode, items, system: Self::system_for(service, mode, remembered) }
    }

    fn system(&self) -> &str {
        &self.system
    }

    /// The request for the next answer.
    pub fn body(&self, effort: Effort) -> Value {
        // Browse has the web alone; the assistant its own tools too.
        let specs = if self.mode == Mode::Assistant { tool_specs() } else { Vec::new() };
        match self.service {
            Service::OpenAi => {
                let mut tools: Vec<Value> = specs.iter().map(|(name, about, schema)| json!({"type": "function", "name": name, "description": about, "parameters": schema, "strict": true})).collect();
                tools.push(json!({"type": "web_search"}));
                json!({
                    "model": self.service.model(),
                    "instructions": self.system(),
                    "input": self.items,
                    "tools": tools,
                    "reasoning": {"effort": effort.wire()},
                    "store": false,
                    "include": ["reasoning.encrypted_content", "web_search_call.action.sources"],
                    "max_output_tokens": OPENAI_MAX_OUTPUT,
                })
            }
            Service::Claude => {
                let mut tools: Vec<Value> = specs.iter().map(|(name, about, schema)| json!({"name": name, "description": about, "input_schema": schema, "strict": true})).collect();
                tools.push(json!({"type": "web_search_20260209", "name": "web_search", "max_uses": WEB_USES}));
                tools.push(json!({"type": "web_fetch_20260209", "name": "web_fetch", "max_uses": WEB_USES, "citations": {"enabled": true}, "max_content_tokens": FETCH_TOKENS}));
                json!({
                    "model": self.service.model(),
                    "max_tokens": CLAUDE_MAX_TOKENS,
                    "system": self.system(),
                    "tools": tools,
                    "output_config": {"effort": effort.wire()},
                    "fallbacks": "default",
                    "cache_control": {"type": "ephemeral"},
                    "messages": self.items,
                })
            }
            Service::Local => {
                let mut messages = vec![json!({"role": "system", "content": self.system()})];
                messages.extend(self.items.iter().cloned());
                json!({"model": self.service.model(), "messages": messages, "max_tokens": LOCAL_MAX_TOKENS, "stream": false})
            }
        }
    }

    /// An answer taken into the conversation: its words, the calls it makes,
    /// what it did on the web and the pages it rests on. A refusal leaves the
    /// conversation as it was.
    pub fn take(&mut self, v: &Value) -> Result<Reply, String> {
        if let Some(message) = v["error"]["message"].as_str() {
            return Err(short(message, 140));
        }
        match self.service {
            Service::Claude => self.take_claude(v),
            Service::OpenAi => self.take_openai(v),
            Service::Local => self.take_local(v),
        }
    }

    fn take_claude(&mut self, v: &Value) -> Result<Reply, String> {
        let content = v["content"].as_array().cloned().unwrap_or_default();
        let stop = v["stop_reason"].as_str().unwrap_or_default();
        if stop == "refusal" {
            let why = v["stop_details"]["explanation"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("it would not answer that");
            return Ok(Reply::stopped(Stop::Refused(short(why, 140))));
        }
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut steps: Vec<WebStep> = Vec::new();
        // Each server tool call's step, by its id, to mark it failed if its
        // result is an error.
        let mut step_of: Vec<(String, usize)> = Vec::new();
        let (mut cited, mut read, mut found) = (Vec::new(), Vec::new(), Vec::new());
        for block in &content {
            match block["type"].as_str() {
                Some("text") => {
                    push_text(&mut text, block["text"].as_str().unwrap_or_default());
                    for c in block["citations"].as_array().into_iter().flatten() {
                        if c["type"] == "web_search_result_location" {
                            cited.push(Source { title: str_of(&c["title"]), url: str_of(&c["url"]) });
                        }
                    }
                }
                Some("tool_use") => calls.push(Call { id: str_of(&block["id"]), name: str_of(&block["name"]), input: block["input"].clone() }),
                Some("server_tool_use") => {
                    let words = match block["name"].as_str() {
                        Some("web_search") => format!("searched the web: {}", short(block["input"]["query"].as_str().unwrap_or_default(), 60)),
                        Some("web_fetch") => format!("read {}", host_of(block["input"]["url"].as_str().unwrap_or_default())),
                        _ => continue,
                    };
                    step_of.push((str_of(&block["id"]), steps.len()));
                    steps.push(WebStep { words, ok: true });
                }
                Some(kind @ ("web_search_tool_result" | "web_fetch_tool_result")) => {
                    let result = &block["content"];
                    // An error is one object, not a list of results.
                    let failed = result.is_object() && result["error_code"].is_string();
                    if failed {
                        if let Some((_, at)) = step_of.iter().find(|(id, _)| *id == str_of(&block["tool_use_id"])) {
                            steps[*at].ok = false;
                            steps[*at].words = format!("{} · {}", steps[*at].words, result["error_code"].as_str().unwrap_or_default().replace('_', " "));
                        }
                    } else if kind == "web_search_tool_result" {
                        for r in result.as_array().into_iter().flatten() {
                            found.push(Source { title: str_of(&r["title"]), url: str_of(&r["url"]) });
                        }
                    } else {
                        read.push(Source { title: str_of(&result["content"]["title"]), url: str_of(&result["url"]) });
                    }
                }
                _ => {}
            }
        }
        if content.is_empty() {
            return if stop == "end_turn" { Ok(Reply::stopped(Stop::Answered)) } else { Err("an empty answer came back".into()) };
        }
        // The whole message goes back as it came: server tool blocks, their
        // results' encrypted content and the citations included.
        self.items.push(json!({"role": "assistant", "content": content}));
        let stop = match stop {
            "tool_use" if !calls.is_empty() => Stop::Tools,
            "pause_turn" => Stop::Paused,
            "max_tokens" => Stop::Cut,
            _ => Stop::Answered,
        };
        let sources = sources_from(cited, read, found);
        Ok(Reply { text, calls, stop, steps, sources })
    }

    fn take_openai(&mut self, v: &Value) -> Result<Reply, String> {
        let output = v["output"].as_array().cloned().unwrap_or_default();
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut refusal = None;
        let mut steps = Vec::new();
        let (mut cited, mut read, mut found) = (Vec::new(), Vec::new(), Vec::new());
        for item in &output {
            match item["type"].as_str() {
                Some("message") => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        match part["type"].as_str() {
                            Some("output_text") => {
                                push_text(&mut text, part["text"].as_str().unwrap_or_default());
                                for a in part["annotations"].as_array().into_iter().flatten() {
                                    if a["type"] == "url_citation" {
                                        cited.push(Source { title: str_of(&a["title"]), url: str_of(&a["url"]) });
                                    }
                                }
                            }
                            Some("refusal") => refusal = Some(part["refusal"].as_str().unwrap_or("it would not answer that").to_owned()),
                            _ => {}
                        }
                    }
                }
                Some("function_call") => calls.push(Call {
                    id: str_of(&item["call_id"]),
                    name: str_of(&item["name"]),
                    input: item["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null),
                }),
                Some("web_search_call") => {
                    let action = &item["action"];
                    let ok = item["status"].as_str() != Some("failed");
                    let words = match action["type"].as_str() {
                        Some("open_page") => {
                            let url = str_of(&action["url"]);
                            read.push(Source { title: String::new(), url: url.clone() });
                            format!("read {}", host_of(&url))
                        }
                        Some("find_in_page") => format!("looked in {} for “{}”", host_of(action["url"].as_str().unwrap_or_default()), short(action["pattern"].as_str().unwrap_or_default(), 30)),
                        _ => {
                            let query = action["query"].as_str().or_else(|| action["queries"][0].as_str()).unwrap_or_default();
                            if query.is_empty() {
                                "searched the web".to_owned()
                            } else {
                                format!("searched the web: {}", short(query, 60))
                            }
                        }
                    };
                    for s in action["sources"].as_array().into_iter().flatten() {
                        if let Some(url) = s["url"].as_str() {
                            found.push(Source { title: str_of(&s["title"]), url: url.to_owned() });
                        }
                    }
                    steps.push(WebStep { words, ok });
                }
                _ => {}
            }
        }
        let incomplete = v["status"].as_str() == Some("incomplete");
        if incomplete && v["incomplete_details"]["reason"].as_str() == Some("content_filter") {
            refusal.get_or_insert_with(|| "it would not answer that".into());
        }
        if let Some(why) = refusal {
            return Ok(Reply::stopped(Stop::Refused(short(&why, 140))));
        }
        if output.is_empty() {
            return Err("an empty answer came back".into());
        }
        // Every item goes back as it came: reasoning, searches and all.
        self.items.extend(output);
        let stop = if incomplete {
            Stop::Cut
        } else if calls.is_empty() {
            Stop::Answered
        } else {
            Stop::Tools
        };
        let sources = sources_from(cited, read, found);
        Ok(Reply { text, calls, stop, steps, sources })
    }

    fn take_local(&mut self, v: &Value) -> Result<Reply, String> {
        let choice = &v["choices"][0];
        let text = choice["message"]["content"].as_str().ok_or("the on-device model gave no answer")?.trim().to_owned();
        if text.is_empty() {
            return Err("the on-device model gave an empty answer".into());
        }
        self.items.push(json!({"role": "assistant", "content": text}));
        let stop = if choice["finish_reason"].as_str() == Some("length") { Stop::Cut } else { Stop::Answered };
        Ok(Reply { text, calls: Vec::new(), stop, steps: Vec::new(), sources: Vec::new() })
    }

    /// What the calls came to, sent back together.
    pub fn answer(&mut self, outcomes: &[Outcome]) {
        match self.service {
            Service::OpenAi => {
                for o in outcomes {
                    self.items.push(json!({"type": "function_call_output", "call_id": o.id, "output": o.text}));
                }
            }
            Service::Claude => {
                let results: Vec<Value> = outcomes.iter().map(|o| json!({"type": "tool_result", "tool_use_id": o.id, "content": o.text, "is_error": o.error})).collect();
                self.items.push(json!({"role": "user", "content": results}));
            }
            // The on-device model has no tools to answer for.
            Service::Local => {}
        }
    }
}

/// The pages an answer rests on: those it cites, then those it read; with
/// neither, the first few it found.
fn sources_from(cited: Vec<Source>, read: Vec<Source>, found: Vec<Source>) -> Vec<Source> {
    let mut sources = dedupe(cited.into_iter().chain(read).collect(), SOURCES_SHOWN);
    if sources.is_empty() {
        sources = dedupe(found, 3);
    }
    sources
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn push_text(text: &mut String, more: &str) {
    if !text.is_empty() && !more.is_empty() {
        text.push('\n');
    }
    text.push_str(more);
}

/// At most `n` characters, cut with an ellipsis.
pub(crate) fn short(text: &str, n: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= n {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(n - 1).collect::<String>().trim_end())
    }
}

/// An answer as the small screen shows it: Markdown's marks taken out, its
/// bullets as dots, no runs of blank lines.
pub(crate) fn plain(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        let line = match line.trim_start_matches('#') {
            heading if line.starts_with('#') && heading.starts_with(' ') => heading.trim_start(),
            _ => line,
        };
        let line = match line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")) {
            Some(item) => format!("· {item}"),
            None => line.to_owned(),
        };
        let line = line.replace("**", "").replace("__", "").replace('`', "");
        if line.trim().is_empty() && lines.last().is_none_or(|l: &String| l.is_empty()) {
            continue;
        }
        lines.push(if line.trim().is_empty() { String::new() } else { line });
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

// ------------------------------------------------------------- transport ---

/// One request to the service, as JSON back.
pub(crate) fn request(service: Service, key: &str, body: &Value) -> Result<Value, String> {
    if demo() {
        return Ok(demo_reply(service, body));
    }
    if service == Service::Local {
        return request_local(body);
    }
    let headers = match service {
        Service::OpenAi => format!("Authorization: Bearer {key}\nContent-Type: application/json\n"),
        Service::Claude | Service::Local => format!("x-api-key: {key}\nanthropic-version: {CLAUDE_VERSION}\nanthropic-beta: {CLAUDE_BETA}\nContent-Type: application/json\n"),
    };
    // The key goes to curl in a file only root reads, not on its command
    // line where any process could see it.
    std::fs::create_dir_all(crate::config_dir()).ok();
    let file = crate::config_dir().join(".assistant-request");
    crate::sys::write_private(&file, headers.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    let max_time = REQUEST_SECONDS.to_string();
    let header_arg = format!("@{}", file.display());
    let output = (|| {
        let mut child = Command::new("curl")
            .args(["-sS", "--max-time", &max_time, "--proto", "=https", "--data-binary", "@-", "-w", "\n%{http_code}", "-H", &header_arg, service.url()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "curl could not start".to_owned())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the request could not be sent".to_owned())?;
        }
        child.wait_with_output().map_err(|_| "the request stopped".to_owned())
    })();
    std::fs::remove_file(&file).ok();
    let output = output?;
    if !output.status.success() {
        return Err(curl_failure(output.status.code(), &String::from_utf8_lossy(&output.stderr)));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if code != 200 {
        return Err(http_failure(service, code, body));
    }
    serde_json::from_str(body).map_err(|_| "the answer could not be read".to_owned())
}

/// The on-device model: its server started if it is not running, then asked
/// OpenAI-style chat completions over plain HTTP, on this handheld only.
fn request_local(body: &Value) -> Result<Value, String> {
    request_local_within(body, LOCAL_SECONDS)
}

/// The same, given at most `seconds` to answer (offline answers have a
/// deadline, past which the answer made from the facts is shown).
pub(crate) fn request_local_within(body: &Value, seconds: u32) -> Result<Value, String> {
    let base = crate::local::start()?;
    if !(base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:")) {
        return Err("the on-device model must be on this handheld".into());
    }
    crate::local::touch();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let max_time = seconds.to_string();
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", &max_time, "--proto", "=http", "-H", "Content-Type: application/json", "--data-binary", "@-", "-w", "\n%{http_code}", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "curl could not start".to_owned())?;
    // Under way: `local::cancel` cuts it (and the answer) short.
    crate::local::answering(Some(child.id()));
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the question could not be sent".to_owned())?;
    }
    let output = child.wait_with_output().map_err(|_| "the on-device model stopped".to_owned());
    crate::local::answering(None);
    let output = output?;
    if !output.status.success() {
        return Err(match output.status.code() {
            Some(7) => "the on-device model is not running".into(),
            Some(28) => "the on-device model took too long".into(),
            _ => "the on-device model did not answer".into(),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    if code.trim() != "200" {
        return Err(format!("the on-device model answered {}", code.trim()));
    }
    serde_json::from_str(body).map_err(|_| "the answer could not be read".to_owned())
}

/// Why curl could not get an answer, in a few words.
fn curl_failure(code: Option<i32>, stderr: &str) -> String {
    match code {
        Some(6) => "no internet: the service's name did not resolve".into(),
        Some(7) => "could not reach the service".into(),
        Some(28) => "no answer in time".into(),
        Some(35 | 60) => "the secure connection failed · is the clock right?".into(),
        _ => short(&format!("the request failed: {}", stderr.trim().trim_start_matches("curl: ")), 120),
    }
}

/// Why the service said no, in a few words.
fn http_failure(service: Service, code: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_owned)).unwrap_or_default();
    match code {
        401 | 403 => format!("{} refused the key · check it in Settings › Keys", service.company()),
        402 | 429 => "too many requests, or the account is out of credit".into(),
        404 => format!("this key cannot use {}", service.model_name()),
        500..=599 => format!("{} is busy · try again shortly", service.company()),
        _ if !message.is_empty() => short(&message, 140),
        _ => format!("{} answered {code}", service.company()),
    }
}

// ------------------------------------------------------------------ demo ---

const DEMO_ANSWER: &str = "The robot is linked and running well. Two things stand out: the intake reached 38 A of its 40 A limit at 10:09 and then cleared, and CAN bus can_s0 is busy with 14 devices on it. The ping came back 4 of 4 at about 2 ms.\n\nWorth moving the intake off can_s0 before the next match.";

const DEMO_ANSWER_JA: &str = "ロボットはつながっていて、調子は良好です。インテークが10時09分に上限40 Aのうち38 Aに達しましたが、その後は戻りました。「can_s0」には14台の機器がつながっていて混んでいます。\n\n次の試合の前に、インテークを別のバスへ移すことをおすすめします。";

/// The scripted service: from the conversation so far, the next reply, in
/// the service's own shape (so the simulator goes through the real parsing).
fn demo_reply(service: Service, body: &Value) -> Value {
    if service == Service::Local {
        return json!({"object": "chat.completion", "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": DEMO_LOCAL}}]});
    }
    let browsing = body["system"].as_str().or_else(|| body["instructions"].as_str()).is_some_and(|s| s == BROWSE_SYSTEM);
    let (question, rounds, last) = match service {
        Service::Claude | Service::Local => {
            let messages = body["messages"].as_array().cloned().unwrap_or_default();
            let at = messages.iter().rposition(|m| m["role"] == "user" && m["content"].is_string()).unwrap_or(0);
            let question = messages.get(at).and_then(|m| m["content"].as_str()).unwrap_or_default().to_owned();
            let results: Vec<&Value> = messages.iter().skip(at + 1).filter(|m| m["role"] == "user").collect();
            let last = results.last().and_then(|m| m["content"].as_array()).and_then(|c| c.last()).map(|b| str_of(&b["content"])).unwrap_or_default();
            (question, results.len(), last)
        }
        Service::OpenAi => {
            let input = body["input"].as_array().cloned().unwrap_or_default();
            let at = input.iter().rposition(|i| i["role"] == "user").unwrap_or(0);
            let question = input.get(at).and_then(|i| i["content"].as_str()).unwrap_or_default().to_owned();
            let outputs = |i: &Value| i["type"] == "function_call_output";
            let rounds = (at + 1..input.len()).filter(|&k| outputs(&input[k]) && !outputs(&input[k - 1])).count();
            let last = input.iter().skip(at + 1).rev().find(|i| outputs(i)).map(|i| str_of(&i["output"])).unwrap_or_default();
            (question, rounds, last)
        }
    };
    if browsing {
        let opening = question.strip_prefix("Open ").map(str::trim);
        return match opening {
            Some(url) => demo_web(service, None, Some((url, DEMO_PAGE_TITLE)), DEMO_PAGE, &[]),
            None => demo_web(service, Some(&question), None, DEMO_BROWSE_ANSWER, &[(DEMO_MANUAL_TITLE, DEMO_MANUAL_URL)]),
        };
    }
    let lower = question.to_lowercase();
    if ["rule", "manual", "autonomous"].iter().any(|w| lower.contains(w)) {
        return demo_web(service, Some("2026 FRC REBUILT match timing autonomous"), None, DEMO_WEB_ANSWER, &[(DEMO_MANUAL_TITLE, DEMO_MANUAL_URL)]);
    }
    // Asked to do something on the handheld, to brief, to remember: its
    // tools (all of them in one turn), then what they said they did.
    let system = body["system"].as_str().or_else(|| body["instructions"].as_str()).unwrap_or_default();
    if let Some(calls) = demo_device(&question, system) {
        let (text, calls) = match (rounds, calls) {
            (0, Err(answer)) => (answer, Vec::new()),
            (0, Ok(calls)) => (String::new(), calls),
            _ => (said_from(&demo_results(service, body)), Vec::new()),
        };
        return demo_shape(service, rounds, &text, &calls);
    }
    let deploy = lower.contains("deploy");
    // Asked in Japanese (or Chinese): answered in Japanese, at once.
    let japanese = question.chars().any(bezel::lines::breaks_anywhere);
    let (text, calls): (&str, Vec<(&str, Value)>) = match (deploy, rounds) {
        _ if japanese => (DEMO_ANSWER_JA, vec![]),
        (true, 0) => ("", vec![("run_action", json!({"name": "Deploy robot code", "reason": "you asked for the code to be deployed"}))]),
        (true, _) if last.contains("declined") => ("Left it: nothing was deployed. Say when the robot is on its cart and disabled, and I'll run it then.", vec![]),
        (true, _) => ("Deployed: the build passed in 41 seconds and the robot program restarted.", vec![]),
        (false, 0) => ("Looking at the robot first.", vec![("robot_status", json!({})), ("robot_agent", json!({}))]),
        (false, 1) => ("", vec![("run_action", json!({"name": "Ping the robot", "reason": "to check the link is steady"}))]),
        (false, _) => (DEMO_ANSWER, vec![]),
    };
    demo_shape(service, rounds, text, &calls)
}

/// A scripted reply in the service's own shape: its words, then its calls.
fn demo_shape(service: Service, rounds: usize, text: &str, calls: &[(&str, Value)]) -> Value {
    match service {
        Service::Claude | Service::Local => {
            let mut content = Vec::new();
            if !text.is_empty() {
                content.push(json!({"type": "text", "text": text}));
            }
            for (i, (name, input)) in calls.iter().enumerate() {
                content.push(json!({"type": "tool_use", "id": format!("toolu_demo_{rounds}_{i}"), "name": name, "input": input}));
            }
            json!({"type": "message", "role": "assistant", "model": service.model(), "content": content, "stop_reason": if calls.is_empty() { "end_turn" } else { "tool_use" }})
        }
        Service::OpenAi => {
            let mut output = vec![json!({"type": "reasoning", "id": format!("rs_demo_{rounds}"), "summary": [], "encrypted_content": "demo"})];
            if !text.is_empty() {
                output.push(json!({"type": "message", "id": format!("msg_demo_{rounds}"), "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": text, "annotations": []}]}));
            }
            for (i, (name, input)) in calls.iter().enumerate() {
                output.push(json!({"type": "function_call", "id": format!("fc_demo_{rounds}_{i}"), "call_id": format!("call_demo_{rounds}_{i}"), "name": name, "arguments": input.to_string(), "status": "completed"}));
            }
            json!({"object": "response", "status": "completed", "model": service.model(), "output": output})
        }
    }
}

/// What the scripted service does for a question that asks the handheld to
/// do something (to brief, to remember): the calls, all in its first turn,
/// read by the same rules that serve offline (`device::rules`); or, for
/// "what do you remember", the answer at once, from its prompt. `None` for
/// any other question.
#[allow(clippy::type_complexity)]
fn demo_device(question: &str, system: &str) -> Option<Result<Vec<(&'static str, Value)>, String>> {
    if memory::command(question) == Some(memory::Command::List) {
        let kept: Vec<&str> = system.lines().filter_map(|l| l.strip_prefix("- ")).collect();
        return Some(Err(match kept.len() {
            0 => "Nothing yet: tell me to remember something and I'll keep it on the handheld.".to_owned(),
            _ => format!("I remember:\n{}", kept.iter().map(|k| format!("· {k}")).collect::<Vec<_>>().join("\n")),
        }));
    }
    if let Some(kind) = briefing::asked(question) {
        return Some(Ok(vec![("briefing", json!({"kind": kind.map_or("auto", briefing::Kind::name)}))]));
    }
    device::rules(question).map(Ok)
}

/// What tools' results say was done, a sentence each: what each said it did
/// (a briefing's few words), what was left as it was, and what did not work.
/// The scripted service's answer, and what is said aloud for requests read
/// by rules.
fn said_from(results: &[String]) -> String {
    let mut said = Vec::new();
    for r in results {
        match serde_json::from_str::<Value>(r) {
            Ok(v) if v["summary"].is_string() => said.push(str_of(&v["summary"])),
            Ok(v) if v["done"].is_string() => said.push(str_of(&v["done"])),
            _ if r.contains("declined") => said.push("Left it as it is.".to_owned()),
            _ => said.push(format!("That didn't work: {}.", r.trim_end_matches('.'))),
        }
    }
    said.join(" ")
}

/// What the tools came back with, in the last round of the conversation.
fn demo_results(service: Service, body: &Value) -> Vec<String> {
    match service {
        Service::OpenAi => {
            let input = body["input"].as_array().cloned().unwrap_or_default();
            let mut out: Vec<String> = input.iter().rev().take_while(|i| i["type"] == "function_call_output").map(|i| str_of(&i["output"])).collect();
            out.reverse();
            out
        }
        _ => body["messages"].as_array().and_then(|m| m.last()).and_then(|m| m["content"].as_array()).map(|c| c.iter().map(|b| str_of(&b["content"])).collect()).unwrap_or_default(),
    }
}

/// A scripted answer that searched the web (`search`) or read a page
/// (`fetch`: its URL and title) first, citing `cited`, in the service's shape
/// as its documentation gives it.
fn demo_web(service: Service, search: Option<&str>, fetch: Option<(&str, &str)>, text: &str, cited: &[(&str, &str)]) -> Value {
    match service {
        Service::Claude | Service::Local => {
            let mut content = Vec::new();
            if let Some(query) = search {
                content.push(json!({"type": "server_tool_use", "id": "srvtoolu_demo_search", "name": "web_search", "input": {"query": query}}));
                let results: Vec<Value> = cited
                    .iter()
                    .chain([("FRC Game Q&A", "https://frc-qa.firstinspires.org/")].iter())
                    .map(|(title, url)| json!({"type": "web_search_result", "url": url, "title": title, "encrypted_content": "demo-encrypted", "page_age": "January 10, 2026"}))
                    .collect();
                content.push(json!({"type": "web_search_tool_result", "tool_use_id": "srvtoolu_demo_search", "content": results}));
            }
            if let Some((url, title)) = fetch {
                content.push(json!({"type": "server_tool_use", "id": "srvtoolu_demo_fetch", "name": "web_fetch", "input": {"url": url}}));
                content.push(json!({"type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_demo_fetch", "content": {
                    "type": "web_fetch_result", "url": url,
                    "content": {"type": "document", "source": {"type": "text", "media_type": "text/plain", "data": "(the page)"}, "title": title, "citations": {"enabled": true}},
                    "retrieved_at": "2026-11-21T10:30:00Z"
                }}));
            }
            let citations: Vec<Value> = cited
                .iter()
                .map(|(title, url)| json!({"type": "web_search_result_location", "url": url, "title": title, "encrypted_index": "demo-index", "cited_text": "Autonomous lasts 20 seconds."}))
                .collect();
            let mut block = json!({"type": "text", "text": text});
            if !citations.is_empty() {
                block["citations"] = json!(citations);
            }
            content.push(block);
            json!({"type": "message", "role": "assistant", "model": service.model(), "content": content, "stop_reason": "end_turn"})
        }
        Service::OpenAi => {
            let mut output = vec![json!({"type": "reasoning", "id": "rs_demo_web", "summary": [], "encrypted_content": "demo"})];
            if let Some(query) = search {
                let sources: Vec<Value> = cited.iter().map(|(_, url)| json!({"type": "url", "url": url})).collect();
                output.push(json!({"type": "web_search_call", "id": "ws_demo_search", "status": "completed", "action": {"type": "search", "query": query, "sources": sources}}));
            }
            if let Some((url, _)) = fetch {
                output.push(json!({"type": "web_search_call", "id": "ws_demo_open", "status": "completed", "action": {"type": "open_page", "url": url}}));
            }
            let annotations: Vec<Value> = cited.iter().map(|(title, url)| json!({"type": "url_citation", "start_index": 0, "end_index": 10, "url": url, "title": title})).collect();
            output.push(json!({"type": "message", "id": "msg_demo_web", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": text, "annotations": annotations}]}));
            json!({"object": "response", "status": "completed", "model": service.model(), "output": output})
        }
    }
}

const DEMO_LOCAL: &str = "A brownout is when the battery voltage sags so low under load that the robot controller cuts power to the motors to keep itself running. Check the battery, its leads and the main breaker first.";

const DEMO_MANUAL_TITLE: &str = "2026 FRC Game Manual";
const DEMO_MANUAL_URL: &str = "https://www.firstinspires.org/robotics/frc/game-and-season";

const DEMO_WEB_ANSWER: &str = "In 2026 REBUILT a match runs 2 minutes 40 seconds: autonomous for the first 20 seconds, then 2 minutes 20 seconds of teleoperated play.";

const DEMO_BROWSE_ANSWER: &str = "TITLE: How long autonomous is in 2026
A match in 2026 REBUILT runs 2 minutes 40 seconds. Autonomous takes the first 20 seconds; teleoperated play takes the remaining 2 minutes 20 seconds.

## What happens in autonomous
Robots move on their own, from code written before the match. Drivers keep their hands off the controls until teleop begins.

LINKS:
2026 FRC Game Manual | https://www.firstinspires.org/robotics/frc/game-and-season
FRC Game Q&A | https://frc-qa.firstinspires.org/";

const DEMO_PAGE_TITLE: &str = "WPILib Documentation";

const DEMO_PAGE: &str = "TITLE: WPILib Documentation
WPILib is the library FRC teams write their robot programs with, in Java, C++ or Python. These pages cover installing it, the command-based framework, hardware APIs, and the tools that come with it.

## Getting started
Install the WPILib suite for your computer, create a project from a template, and deploy it to the robot from Visual Studio Code.

## Command-based programming
Subsystems hold the robot's mechanisms; commands describe what they do, and triggers tie commands to buttons and sensors.

## Hardware
Motor controllers, sensors and pneumatics, and how each is wired and used from code.

LINKS:
Zero to Robot | https://docs.wpilib.org/en/stable/docs/zero-to-robot/introduction.html
Command-based programming | https://docs.wpilib.org/en/stable/docs/software/commandbased/index.html
Hardware APIs | https://docs.wpilib.org/en/stable/docs/software/hardware-apis/index.html
Vision processing | https://docs.wpilib.org/en/stable/docs/software/vision-processing/index.html";

/// What an Action prints in the simulator.
fn demo_output(line: &str) -> String {
    if line.contains("ping") {
        "PING 10.58.5.2 (10.58.5.2): 56 data bytes\n64 bytes from 10.58.5.2: seq=0 ttl=64 time=2.412 ms\n64 bytes from 10.58.5.2: seq=1 ttl=64 time=1.873 ms\n64 bytes from 10.58.5.2: seq=2 ttl=64 time=1.655 ms\n64 bytes from 10.58.5.2: seq=3 ttl=64 time=1.702 ms\n\n--- 10.58.5.2 ping statistics ---\n4 packets transmitted, 4 packets received, 0% packet loss\nround-trip min/avg/max = 1.655/1.910/2.412 ms".into()
    } else if line.contains("gradlew") {
        "> Task :deploy\nDeploying to 10.58.5.2 ... done\nBUILD SUCCESSFUL in 41s".into()
    } else {
        "done".into()
    }
}

// ---------------------------------------------------------------- screen ---

/// What the page shows, oldest first.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    /// A question asked.
    You(String),
    /// What the model said.
    Said(String),
    /// A tool used: what it did, and how it went.
    Step { words: String, state: StepState },
    /// A critical Action waiting for a yes.
    Asks { name: String, command: String, reason: String, state: AskState },
    /// Something that went wrong, or a stop.
    Note(String),
    /// The pages the answer above rests on.
    Sources(Vec<Source>),
    /// A match's report, made offline with no model: each fact a row.
    Report { label: String, title: String, rows: Vec<(crate::wpilog::Level, String)> },
    /// What an offline answer rests on ("from the log of Q14"), and whether
    /// it is a model's wording that passed the check.
    Basis { words: String, checked: bool },
    /// Something done on the handheld: its icon, what ("reminder set · swap
    /// the battery") and under it the detail ("at queueing for Q42").
    Did { icon: String, words: String, detail: String, state: StepState },
    /// A briefing's card.
    Briefing { title: String, when: String, rows: Vec<(String, String, bool)> },
}

impl Entry {
    fn briefing(b: &briefing::Brief) -> Entry {
        Entry::Briefing { title: b.title.clone(), when: b.when.clone(), rows: b.rows.iter().map(|r| (r.icon.clone(), r.text.clone(), r.urgent)).collect() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Working,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AskState {
    Waiting,
    Yes,
    No,
}

/// What the assistant is doing between questions.
enum Busy {
    Idle,
    /// Waiting for the service's answer.
    Asking(Receiver<Result<Value, String>>),
    /// A tool at work on a thread: the call it answers, its row, and the
    /// row's words once it is done.
    Tool { rx: Receiver<Result<Worked, String>>, id: String, row: usize, done: String },
    /// A critical Action (or a change to the handheld) shown, waiting for
    /// yes or no.
    Approving { id: String, pending: Pending, row: usize },
    /// Between one thing done on the handheld and the next, a beat, so each
    /// row lands before the next rises (until the clock reads this).
    Beat(f32),
    /// A briefing made offline from the template, on a thread.
    Briefing(Receiver<briefing::Brief>),
    /// Answering offline from the match logs, on a thread: the model wording
    /// it (once it is), and the answer made from the facts (once there is
    /// one), which Esc shows at once rather than waiting for the model.
    Offline { rx: Receiver<crate::offline::Event>, wording: Option<String>, ready: Option<Box<crate::offline::Answer>>, _stop: StopWhenDropped },
}

/// What a tool on a thread came to: words for the model (the Agent, an
/// Action), or something done on the handheld.
enum Worked {
    Text(String),
    Device(device::Step),
}

/// What waits for a yes: a critical Action, or a change to the handheld.
enum Pending {
    Action { saved: Saved, line: String, reason: String },
    Device(device::Act),
}

/// How long a row done on the handheld has to land before the next rises.
/// My own tuning: about the rise's own time, a little under.
const BEAT: f32 = 0.22;
/// An app asked for opens this long after the answer appears, so it is read.
const OPEN_AFTER: f32 = 0.9;

/// A worker's stop flag, set once nobody waits for it (the answer came, or
/// Esc, a new conversation, leaving): it reads no more logs and asks the
/// model nothing more, and its memory is given back.
struct StopWhenDropped(std::sync::Arc<AtomicBool>);

impl Drop for StopWhenDropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Set by the choices page for a fresh conversation when the assistant is
/// back on top.
static NEW_CHAT: AtomicBool = AtomicBool::new(false);

/// The confirm page's answer, taken up when the assistant is on top again.
static DECISION: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

/// A kept conversation to reopen, chosen in Conversations; taken up when the
/// assistant is on top again.
static OPEN_CHAT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// A question sent from search, taken up by the assistant once it is on top:
/// a new one, or one kept among the recent apps and brought back.
static ASK_NEXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn take_question() -> Option<String> {
    ASK_NEXT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn take_open_chat() -> Option<String> {
    OPEN_CHAT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn open_chat_waiting() -> bool {
    OPEN_CHAT.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// An entry laid out: its lines at the width it was wrapped to, and height.
#[derive(Clone)]
struct Laid {
    lines: Vec<String>,
    h: f32,
    /// A report's lines: the level of the row each begins (`None` for a
    /// row's second line).
    levels: Vec<Option<crate::wpilog::Level>>,
}

pub struct Assistant {
    prefs: Prefs,
    service: Option<(Service, String)>,
    chat: Option<Chat>,
    entries: Vec<(Entry, f32)>,
    input: String,
    busy: Busy,
    calls: VecDeque<Call>,
    outcomes: Vec<Outcome>,
    actions: Vec<Saved>,
    rounds: usize,
    /// The conversation's length before the question under way, and the
    /// question, put back to send again if it fails.
    mark: usize,
    question: String,
    /// Up from the newest, in pixels: 0 keeps the newest in view.
    scroll: Animated,
    clock: f32,
    /// Time spent waiting on the current request.
    waited: f32,
    laid: Option<(u64, f32, Vec<Laid>)>,
    revision: u64,
    /// The Actions to use instead of the saved ones (tests, which share
    /// one settings folder and run at once).
    actions_given: Option<Vec<Saved>>,
    /// Paused turns carried on for the question under way.
    continuations: usize,
    /// This conversation's name among those kept (when it started), once it
    /// has an answer worth keeping; and where they are kept (tests: apart).
    chat_id: Option<String>,
    chats: PathBuf,
    /// A question just heard (push-to-talk) sends itself when this runs out,
    /// unless a key stops it to edit.
    countdown: Option<f32>,
    /// The line holds what was said aloud; and the question under way was.
    asked_aloud: bool,
    aloud: bool,
    /// Offline: the question before, for a follow-up ("and Q12?").
    offline: crate::offline::Context,
    /// Offline, before anything is asked: the questions offered, and the one chosen.
    suggestions: Vec<Suggestion>,
    suggest: ListState,
    /// What the questions offered were worked out from: the logs'
    /// generation, and whether the robot was linked.
    suggested: Option<(u64, bool)>,
    /// The model that words offline answers, if one is downloaded.
    offline_model: Option<String>,
    /// Where what it remembers is kept (tests: apart).
    memory_file: PathBuf,
    /// The live activity while a question is under way, and its steps:
    /// those done so far, and the round's calls (where it began, how many).
    activity: Option<crate::liveactivity::TaskHandle>,
    steps: usize,
    batch: (usize, usize),
    /// What tools changed, for the next update to apply to the settings.
    effects: Vec<device::Effect>,
    /// An app or Settings page asked for: opened once the answer is in (at
    /// this time on the page's clock).
    open: Option<(device::Effect, Option<f32>)>,
    /// The settings as last seen, for the tools: the clock, the saver, light.
    h24: bool,
    saver: bool,
    light: bool,
    /// The calls under way were read by rules, not made by a model
    /// (offline, or with no internet): no model is asked once they are done.
    by_rules: bool,
    /// When the question under way was asked: a long one is a notification
    /// once it is answered.
    began: Option<std::time::Instant>,
    /// An entry to show from its top once it is laid out (a briefing's
    /// card, taller than what is left of the screen under the question).
    land: Option<usize>,
}

/// A question offered offline, before anything is asked.
#[derive(Clone, Debug, PartialEq)]
struct Suggestion {
    /// As the row shows it, and a mono value beside it (the match).
    name: String,
    value: String,
    /// What it asks; `None` is the newest match's report.
    question: Option<String>,
}

/// The questions offered offline, from the logs there are: the newest
/// match's report, why it browned out (or what went wrong in it), what
/// changed since the match before, a hot motor, the two compared.
fn suggestions_for(robot: &Robot) -> Vec<Suggestion> {
    let rows = robot.logs.list();
    let read: Vec<&crate::logs::Row> = rows.iter().filter(|r| r.digest.is_some()).collect();
    let label = |r: &crate::logs::Row| crate::offline::facts::label_from_name(&r.name).unwrap_or_else(|| r.title());
    let mut out = Vec::new();
    if let Some(newest) = read.first() {
        let l = label(newest);
        out.push(Suggestion { name: "Match report".into(), value: l.clone(), question: None });
        let browned = newest.digest.as_ref().is_some_and(|d| d.brownouts.unwrap_or(0) > 0);
        let q = if browned { format!("Why did we brown out in {l}?") } else { format!("What went wrong in {l}?") };
        out.push(Suggestion { name: q.clone(), value: String::new(), question: Some(q) });
        // A briefing, made from the template: no model needed.
        out.push(Suggestion { name: "Brief me".into(), value: String::new(), question: Some("Brief me".into()) });
        if let Some(before) = read.get(1) {
            let q = format!("What changed since {}?", label(before));
            out.push(Suggestion { name: q.clone(), value: String::new(), question: Some(q) });
        }
        out.push(Suggestion { name: "Any motor running hot?".into(), value: String::new(), question: Some("Any motor running hot?".into()) });
        if let Some(before) = read.get(1) {
            let q = format!("Compare {l} and {}", label(before));
            out.push(Suggestion { name: q.clone(), value: String::new(), question: Some(q) });
        }
    }
    if robot.linked() {
        out.push(Suggestion { name: "How is the robot doing?".into(), value: String::new(), question: Some("How is the robot doing right now?".into()) });
    }
    out
}

/// What offline answers come from, longest first: the logs and the model
/// that words them, or the logs alone.
fn offline_from(logs: usize, model: Option<&str>) -> Vec<String> {
    let count = match logs {
        0 => return vec!["no match logs yet".into(), "no logs yet".into()],
        1 => "1 match log".to_owned(),
        n => format!("{n} match logs"),
    };
    let mut out = Vec::new();
    if let Some(model) = model {
        out.push(format!("{logs} logs · {}", model.to_lowercase()));
        out.push(model.to_lowercase());
    }
    out.push(count);
    out.push(format!("{logs} logs"));
    out
}

/// Words a failed request ends in when there is no internet to ask over.
fn no_internet(why: &str) -> bool {
    ["no internet", "could not reach", "no answer in time", "secure connection failed"].iter().any(|w| why.contains(w))
}

const GAP: f32 = 6.0;
const INPUT_LIMIT: usize = 600;
/// How long a question heard stays in the line, to read (and stop and
/// edit), before it sends itself. My own tuning: long enough to read a
/// short question, short enough not to feel like waiting.
const AUTO_SEND: f32 = 1.5;
/// How far from the title (and the line typed into) a line fades out.
const EDGE: f32 = 14.0;

impl Assistant {
    pub fn new() -> Self {
        let prefs = Prefs::load();
        let service = answering(&prefs, &Keys::load());
        Self::with(prefs, service)
    }

    /// The assistant, asked `question` (from search) once it is on top.
    pub fn asking(question: &str) -> Self {
        *ASK_NEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(question.chars().take(INPUT_LIMIT).collect());
        Self::new()
    }

    /// The assistant, opened by a briefing's reminder: it briefs at once.
    pub fn briefing() -> Self {
        Self::asking("Brief me")
    }

    fn with(prefs: Prefs, service: Option<(Service, String)>) -> Self {
        let memory_file = memory::path();
        Assistant {
            chat: service.as_ref().map(|(s, _)| Chat::with_memory(*s, Mode::Assistant, &memory::load(&memory_file))),
            memory_file,
            activity: None,
            steps: 0,
            batch: (0, 0),
            effects: Vec::new(),
            open: None,
            h24: true,
            saver: false,
            light: false,
            by_rules: false,
            began: None,
            land: None,
            prefs,
            service,
            entries: Vec::new(),
            input: String::new(),
            busy: Busy::Idle,
            calls: VecDeque::new(),
            outcomes: Vec::new(),
            actions: Vec::new(),
            rounds: 0,
            mark: 0,
            question: String::new(),
            scroll: Animated::new(0.0, role::settle()).with_rest(0.3),
            clock: 0.0,
            waited: 0.0,
            laid: None,
            revision: 0,
            actions_given: None,
            continuations: 0,
            chat_id: None,
            chats: chats_dir(),
            countdown: None,
            asked_aloud: false,
            aloud: false,
            offline: crate::offline::Context::default(),
            suggestions: Vec::new(),
            suggest: ListState::default(),
            suggested: None,
            offline_model: crate::offline::model::find().map(|m| m.name()),
        }
    }

    /// The question under way is one the match logs answer (not a chat).
    fn about_logs(&self) -> bool {
        let plan = crate::offline::route::rules(&self.question, &crate::offline::tools::Library::default(), None);
        plan.sure && plan.intent != crate::offline::route::Intent::Chat
    }

    /// Answering on this handheld, from the match logs.
    fn offline_mode(&self) -> bool {
        self.service.as_ref().is_some_and(|(s, _)| *s == Service::Local)
    }

    /// A question answered offline, on a thread: the logs read (each new
    /// one worked out once), the facts picked, the model (if there is one)
    /// wording them, its wording checked. `report` asks for the newest
    /// match's report instead, which needs no model.
    fn ask_offline(&mut self, question: Option<String>, robot: &Robot) {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let mut source = crate::offline::Source::of(robot);
        source.stop = Some(stop.clone());
        let mut ctx = self.offline.clone();
        ctx.stop = Some(stop.clone());
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new().name("offline".into()).stack_size(1024 * 1024).spawn(move || {
            // Reading the match logs, for someone waiting: SCHED_BATCH at
            // nice 10, so the frames still come first (sched.rs).
            crate::sched::enter(crate::sched::Class::Batch(10));
            let mut send = |e: crate::offline::Event| {
                tx.send(e).ok();
            };
            let lib = crate::offline::load(&source, &mut send);
            let answer = match &question {
                Some(q) => {
                    let model = crate::offline::model::find();
                    crate::offline::ask(q, &ctx, &lib, model.as_deref(), &mut send)
                }
                None => crate::offline::report(&lib, None),
            };
            send(crate::offline::Event::Done(Box::new(answer)));
        });
        if started.is_err() {
            return self.failed("could not start answering".into());
        }
        self.busy = Busy::Offline { rx, wording: None, ready: None, _stop: StopWhenDropped(stop) };
        self.waited = 0.0;
        self.scroll.set(0.0);
    }

    /// The newest match's report, asked for with Ctrl R or from the questions offered.
    fn report(&mut self, robot: &Robot) {
        if !self.idle() {
            return;
        }
        self.mark = self.chat.as_ref().map_or(0, Chat::len);
        self.push(Entry::You("Match report".into()));
        self.question = "Match report".into();
        self.ask_offline(None, robot);
    }

    /// An offline answer, shown: a report as its card, an answer as said,
    /// then what it rests on. The conversation keeps it, to carry on.
    fn offline_done(&mut self, answer: crate::offline::Answer) {
        self.busy = Busy::Idle;
        match &answer.report {
            Some(r) => self.push(Entry::Report { label: r.label.clone(), title: r.title.clone(), rows: r.rows.clone() }),
            None => self.push(Entry::Said(answer.text.clone())),
        }
        let basis = match &answer.worded_by {
            // "lfm2.5 230m's own knowledge, not the logs" names it already.
            Some(model) if answer.lines.is_empty() && answer.basis.contains(&model.to_lowercase()) => answer.basis.clone(),
            Some(model) if answer.lines.is_empty() => format!("{} · {}", answer.basis, model.to_lowercase()),
            Some(model) => format!("{} · worded by {}, checked", answer.basis, model.to_lowercase()),
            None => answer.basis.clone(),
        };
        self.push(Entry::Basis { words: basis, checked: answer.worded_by.is_some() && !answer.lines.is_empty() });
        if answer.report.is_none() {
            self.offline.prior = Some(answer.plan.clone());
        }
        if let Some(chat) = &mut self.chat {
            if chat.service() == Service::Local {
                let said = if answer.report.is_some() { format!("(the report of {})", answer.report.as_ref().map_or("", |r| r.label.as_str())) } else { answer.text.clone() };
                if chat.len() == self.mark {
                    chat.ask(&self.question);
                }
                chat.said(&said);
            }
        }
        self.keep();
    }

    fn idle(&self) -> bool {
        matches!(self.busy, Busy::Idle)
    }

    /// The settings the tools read, as they are now.
    fn see_settings(&mut self, s: &crate::shell::Settings) {
        (self.h24, self.saver, self.light) = (s.clock24, s.battery.saver, s.light);
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push((entry, self.clock));
        self.revision += 1;
        self.scroll.set(0.0);
    }

    fn set_step(&mut self, row: usize, ok: bool, done: String) {
        if let Some((Entry::Step { state, words }, _)) = self.entries.get_mut(row) {
            *state = if ok { StepState::Done } else { StepState::Failed };
            *words = done;
            self.revision += 1;
        }
    }

    /// A fresh conversation with whichever service answers now.
    fn start_over(&mut self) {
        self.prefs = Prefs::load();
        self.service = answering(&self.prefs, &Keys::load());
        let remembered = memory::load(&self.memory_file);
        self.chat = self.service.as_ref().map(|(s, _)| Chat::with_memory(*s, Mode::Assistant, &remembered));
        self.entries.clear();
        self.calls.clear();
        self.outcomes.clear();
        self.busy = Busy::Idle;
        self.activity = None;
        self.open = None;
        self.by_rules = false;
        self.land = None;
        self.chat_id = None;
        self.countdown = None;
        self.offline = crate::offline::Context::default();
        self.suggest = ListState::default();
        self.suggested = None;
        self.offline_model = crate::offline::model::find().map(|m| m.name());
        self.revision += 1;
        self.scroll.jump(0.0);
    }

    /// A kept conversation, back as it was left, to carry on.
    fn reopen(&mut self, id: &str) -> Result<(), String> {
        let kept = load_chat(&self.chats, id).ok_or("that conversation is no longer kept")?;
        let keys = Keys::load();
        let key = match kept.service {
            // Offline answers need no model: it carries on with or without one.
            Service::Local => String::new(),
            s => keys.get(s.key()).ok_or_else(|| format!("it was with {}, whose key is gone", s.model_name()))?.to_owned(),
        };
        self.service = Some((kept.service, key));
        self.chat = Some(Chat::with_items(kept.service, Mode::Assistant, kept.items, &memory::load(&self.memory_file)));
        self.entries = kept.entries.into_iter().map(|e| (e, -10.0)).collect();
        self.calls.clear();
        self.outcomes.clear();
        self.busy = Busy::Idle;
        self.chat_id = Some(id.to_owned());
        self.revision += 1;
        self.scroll.jump(0.0);
        Ok(())
    }

    /// The conversation kept (once it has a whole answer), newest of the
    /// kept ones, so it can be reopened later.
    fn keep(&mut self) {
        let (Some(chat), true) = (&self.chat, self.idle()) else { return };
        if chat.len() == 0 {
            return;
        }
        let id = self.chat_id.get_or_insert_with(|| chrono::Utc::now().timestamp_millis().to_string()).clone();
        let title = self.entries.iter().find_map(|(e, _)| if let Entry::You(q) = e { Some(q.clone()) } else { None }).unwrap_or_default();
        let entries: Vec<Entry> = self.entries.iter().map(|(e, _)| e.clone()).collect();
        save_chat(&self.chats, &id, chat.service(), &title, chat.items(), &entries);
    }

    fn send(&mut self, robot: &Robot) {
        let text = self.input.trim().to_owned();
        if text.is_empty() || !self.idle() {
            return;
        }
        let Some(chat) = &mut self.chat else { return };
        self.mark = chat.len();
        chat.ask(&text);
        self.input.clear();
        self.countdown = None;
        self.aloud = std::mem::take(&mut self.asked_aloud);
        self.question = text.clone();
        self.rounds = 0;
        self.continuations = 0;
        self.steps = 0;
        self.batch = (0, 0);
        self.open = None;
        self.by_rules = false;
        self.began = Some(std::time::Instant::now());
        self.actions = self.actions_given.clone().unwrap_or_else(crate::commands::all);
        self.push(Entry::You(text.clone()));
        if self.offline_mode() {
            // Offline, with no model to call the tools: what it remembers,
            // briefings and requests to the handheld by rules; the rest
            // from the logs.
            if memory::command(&text) == Some(memory::Command::List) {
                return self.list_memory(true);
            }
            if let Some(kind) = briefing::asked(&text) {
                return self.brief_offline(kind, robot);
            }
            if let Some(calls) = device::rules(&text) {
                return self.run_rules(calls, robot);
            }
            return self.ask_offline(Some(text), robot);
        }
        // A live activity while it works, wherever one is shown: what it is
        // doing, step by step, until the answer is in.
        self.activity = Some(crate::liveactivity::task(crate::liveactivity::Kind::Assistant, "Assistant", "thinking"));
        self.ask();
    }

    /// What it remembers, said offline (kept in the on-device conversation,
    /// `into_chat`; a service's stays as it was).
    fn list_memory(&mut self, into_chat: bool) {
        let said = memory::listed(&self.memory_file);
        self.push(Entry::Said(said.clone()));
        self.push(Entry::Basis { words: "from memory.md on this handheld".into(), checked: false });
        if let Some(chat) = self.chat.as_mut().filter(|_| into_chat) {
            chat.said(&said);
        }
        self.keep();
        crate::voice::answer(&said, self.aloud);
    }

    /// Requests to the handheld read by rules (offline, or with no
    /// internet): the tools a model would call, in turn, each a row as it
    /// is done and a change asked about first, as online; no model is
    /// asked to word what was done.
    fn run_rules(&mut self, calls: Vec<(&'static str, Value)>, robot: &Robot) {
        self.by_rules = true;
        self.calls = calls.into_iter().enumerate().map(|(k, (name, input))| Call { id: format!("rule-{k}"), name: name.to_owned(), input }).collect();
        self.batch = (self.steps, self.calls.len());
        self.activity = Some(crate::liveactivity::task(crate::liveactivity::Kind::Assistant, "Assistant", "working"));
        self.scroll.set(0.0);
        self.next_call(robot);
    }

    /// The requests read by rules, done: what each did is in its row; under
    /// them, that no model was needed. Said aloud when answers are.
    fn rules_done(&mut self, outcomes: &[Outcome]) {
        self.by_rules = false;
        self.busy = Busy::Idle;
        self.push(Entry::Basis { words: "done on this handheld · no model needed".into(), checked: false });
        let said = said_from(&outcomes.iter().map(|o| o.text.clone()).collect::<Vec<_>>());
        // Kept in the on-device conversation (a service's stays as it was).
        if let Some(chat) = self.chat.as_mut().filter(|c| c.service() == Service::Local) {
            if chat.len() == self.mark {
                chat.ask(&self.question);
            }
            chat.said(&said);
        }
        self.finished();
        self.keep();
        crate::voice::answer(&said, self.aloud);
    }

    /// A question's work done: its live activity says so, an app asked for
    /// opens a moment later, and a briefing in the answer shows from its top.
    fn finished(&mut self) {
        if let Some(a) = self.activity.take() {
            a.done(if self.steps == 0 { "answer ready".to_owned() } else { format!("{} done", count_words(self.steps, "step")) });
        }
        if let Some((_, at @ None)) = &mut self.open {
            *at = Some(self.clock + OPEN_AFTER);
        }
        let asked = self.entries.iter().rposition(|(e, _)| matches!(e, Entry::You(_))).unwrap_or(0);
        self.land = self.entries.iter().skip(asked).position(|(e, _)| matches!(e, Entry::Briefing { .. })).map(|k| asked + k);
    }

    /// A briefing made here, with no model: the template, on a thread (it
    /// reads the newest match's log).
    fn brief_offline(&mut self, kind: Option<briefing::Kind>, robot: &Robot) {
        let inputs = briefing::Inputs::of(robot, self.h24, kind);
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new().name("briefing".into()).stack_size(1024 * 1024).spawn(move || {
            tx.send(briefing::build(&inputs)).ok();
        });
        if started.is_err() {
            return self.failed("could not start the briefing".into());
        }
        self.busy = Busy::Briefing(rx);
        self.waited = 0.0;
        self.scroll.set(0.0);
    }

    /// The briefing made offline, shown: its card, what it rests on, and
    /// said aloud when answers are.
    fn brief_done(&mut self, b: briefing::Brief) {
        self.busy = Busy::Idle;
        self.push(Entry::briefing(&b));
        self.push(Entry::Basis { words: "from the schedule, robot, forecast, battery and logs · no model".into(), checked: false });
        // Kept in the on-device conversation (a service's stays as it was).
        if let Some(chat) = self.chat.as_mut().filter(|c| c.service() == Service::Local) {
            if chat.len() == self.mark {
                chat.ask(&self.question);
            }
            chat.said(&b.said);
        }
        self.finished();
        self.keep();
        crate::voice::answer(&b.said, self.aloud);
    }

    /// The live activity's line: the step under way.
    fn note_step(&self, words: &str) {
        if let Some(a) = &self.activity {
            a.detail(format!("step {} · {words}", self.steps));
        }
    }

    /// The conversation sent for the next answer, on a thread.
    fn ask(&mut self) {
        let (Some(chat), Some((service, key))) = (&self.chat, &self.service) else { return };
        let body = chat.body(self.prefs.effort);
        let (service, key) = (*service, key.clone());
        if let Some(a) = &self.activity {
            a.detail(if self.steps == 0 { "thinking".to_owned() } else { format!("thinking · {} done", count_words(self.steps, "step")) });
        }
        let (began, steps, question) = (self.began, self.steps, self.question.clone());
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new().name("assistant".into()).stack_size(512 * 1024).spawn(move || {
            let answer = request(service, &key, &body);
            // A long one, a notification once it is the answer, posted from
            // here: the Assistant may have been left while it thought, and
            // it is only brought up to date in front (unsaid if it is,
            // `Shell::notify`).
            if let (Ok(reply), Some(began)) = (&answer, began) {
                if let Some(n) = answered_notice(service, reply, began.elapsed(), steps, &question) {
                    crate::notify::post(n);
                }
            }
            tx.send(answer).ok();
        });
        if started.is_err() {
            return self.failed("could not start the request".into());
        }
        self.busy = Busy::Asking(rx);
        self.waited = 0.0;
        self.scroll.set(0.0);
    }

    /// The question failed: the conversation goes back to before it, and the
    /// question back into the line to send again.
    fn failed(&mut self, why: String) {
        if let Some(chat) = &mut self.chat {
            chat.rewind(self.mark);
        }
        self.calls.clear();
        self.outcomes.clear();
        self.busy = Busy::Idle;
        self.activity = None;
        self.by_rules = false;
        if self.input.is_empty() {
            self.input = std::mem::take(&mut self.question);
        }
        self.push(Entry::Note(why));
    }

    /// The service's answer, taken in.
    fn took(&mut self, result: Result<Value, String>, robot: &Robot) {
        self.busy = Busy::Idle;
        let Some((service, _)) = self.service.clone() else { return };
        let reply = match (result, &mut self.chat) {
            (Ok(v), Some(chat)) => chat.take(&v),
            (Ok(_), None) => return,
            (Err(why), _) => Err(why),
        };
        let reply = match reply {
            Ok(reply) => reply,
            // No internet, and a briefing, what it remembers or a request to
            // the handheld: made or done here, by the template or the
            // rules, saying so (the service's conversation stays as it was).
            Err(why) if no_internet(&why) && self.rounds == 0 && (briefing::asked(&self.question).is_some() || memory::command(&self.question).is_some() || device::rules(&self.question).is_some()) => {
                if let Some(chat) = &mut self.chat {
                    chat.rewind(self.mark);
                }
                self.calls.clear();
                self.outcomes.clear();
                self.activity = None;
                let question = self.question.clone();
                if memory::command(&question) == Some(memory::Command::List) {
                    self.push(Entry::Note(format!("{why} · read on this handheld")));
                    return self.list_memory(false);
                }
                if let Some(kind) = briefing::asked(&question) {
                    self.push(Entry::Note(format!("{why} · made offline, with no model")));
                    return self.brief_offline(kind, robot);
                }
                self.push(Entry::Note(format!("{why} · done on this handheld")));
                return self.run_rules(device::rules(&question).unwrap_or_default(), robot);
            }
            // No internet: a question the logs can answer, or any question
            // when a model is on this handheld, is answered offline, on this
            // handheld, saying so.
            Err(why) if no_internet(&why) && self.rounds == 0 && (self.about_logs() || crate::local::ready()) => {
                if let Some(chat) = &mut self.chat {
                    chat.rewind(self.mark);
                }
                self.calls.clear();
                self.outcomes.clear();
                self.activity = None;
                let from = if self.about_logs() { "answered offline, from the match logs" } else { "answered on this handheld" };
                self.push(Entry::Note(format!("{why} · {from}")));
                let question = self.question.clone();
                return self.ask_offline(Some(question), robot);
            }
            Err(why) => return self.failed(why),
        };
        // What it did on the web, then what it said, then the pages it rests on.
        for step in &reply.steps {
            self.push(Entry::Step { words: step.words.clone(), state: if step.ok { StepState::Done } else { StepState::Failed } });
        }
        if !reply.text.trim().is_empty() {
            self.push(Entry::Said(plain(&reply.text)));
        }
        if !reply.sources.is_empty() && reply.stop != Stop::Paused {
            self.push(Entry::Sources(reply.sources.clone()));
        }
        match reply.stop {
            Stop::Answered => {
                self.keep();
                self.finished();
                // Said aloud too, when answers are (Assistant › choices).
                if !reply.text.trim().is_empty() {
                    crate::voice::answer(&plain(&reply.text), self.aloud);
                }
            }
            Stop::Paused => {
                // The service's own loop ran long: the conversation goes back
                // as it is, and it carries on where it stopped.
                self.continuations += 1;
                if self.continuations > CONTINUATIONS {
                    self.activity = None;
                    self.push(Entry::Note("it kept on searching · ask again to carry on".into()));
                    self.keep();
                } else {
                    self.ask();
                }
            }
            Stop::Tools => {
                self.batch = (self.steps, reply.calls.len());
                self.calls = reply.calls.into();
                self.next_call(robot);
            }
            Stop::Refused(why) => {
                self.activity = None;
                if let Some(chat) = &mut self.chat {
                    chat.rewind(self.mark);
                }
                self.push(Entry::Note(format!("{} would not answer: {why}", service.model_name())));
            }
            Stop::Cut => {
                self.activity = None;
                if let Some(chat) = &mut self.chat {
                    chat.rewind(self.mark);
                }
                self.push(Entry::Note("the answer ran too long and was cut short · ask again more narrowly".into()));
            }
        }
    }

    /// Answer the model's calls in turn; once all are, back to the model.
    fn next_call(&mut self, robot: &Robot) {
        while let Some(call) = self.calls.pop_front() {
            let words = step_words(&call);
            self.steps += 1;
            self.note_step(&words.working);
            let planned = {
                let cx = device::Ctx { robot, h24: self.h24, memory: &self.memory_file, saver: self.saver, light: self.light };
                plan(&call, robot, &self.actions, &cx)
            };
            match planned {
                Plan::Now(result) => {
                    let error = result.is_err();
                    self.push(Entry::Step { words: words.done, state: if error { StepState::Failed } else { StepState::Done } });
                    self.outcomes.push(Outcome { id: call.id, text: result.unwrap_or_else(|why| why), error });
                }
                Plan::Agent(host) => {
                    self.push(Entry::Step { words: words.working, state: StepState::Working });
                    let row = self.entries.len() - 1;
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        tx.send(ask_agent(&host).map(Worked::Text)).ok();
                    });
                    self.busy = Busy::Tool { rx, id: call.id, row, done: words.done };
                    return;
                }
                Plan::Run { saved, line, reason, ask: false } => {
                    self.run(call.id, &saved, line, reason, false);
                    return;
                }
                Plan::Run { saved, line, reason, ask: true } => {
                    self.push(Entry::Asks { name: saved.name.clone(), command: line.clone(), reason: reason.clone(), state: AskState::Waiting });
                    let row = self.entries.len() - 1;
                    self.busy = Busy::Approving { id: call.id, pending: Pending::Action { saved, line, reason }, row };
                    return;
                }
                Plan::Device(p) => {
                    if self.device(call.id, p, &words.done) {
                        return;
                    }
                }
            }
        }
        let outcomes = std::mem::take(&mut self.outcomes);
        if self.by_rules {
            return self.rules_done(&outcomes);
        }
        if let Some(chat) = &mut self.chat {
            chat.answer(&outcomes);
        }
        self.rounds += 1;
        if self.rounds >= ROUNDS {
            self.busy = Busy::Idle;
            self.activity = None;
            self.push(Entry::Note(format!("stopped after {ROUNDS} rounds of looking · ask again to carry on")));
            self.keep();
            return;
        }
        self.ask();
    }

    /// Something to do on the handheld: done now (a beat before the next
    /// thing, so each row lands as it happens), on a thread, or asked about
    /// first. True while it is under way, false once it is done.
    fn device(&mut self, id: String, p: device::Plan, failed_words: &str) -> bool {
        match p {
            device::Plan::Now(Ok(done)) => {
                self.did(id, None, done);
                self.beat()
            }
            device::Plan::Now(Err(why)) => {
                self.push(Entry::Did { icon: "error".into(), words: failed_words.to_owned(), detail: why.clone(), state: StepState::Failed });
                self.outcomes.push(Outcome { id, text: why, error: true });
                self.beat()
            }
            device::Plan::Work { working, icon, job } => {
                self.push(Entry::Did { icon: icon.into(), words: working, detail: String::new(), state: StepState::Working });
                let row = self.entries.len() - 1;
                let (tx, rx) = mpsc::channel();
                let started = std::thread::Builder::new().name("assistant-tool".into()).stack_size(1024 * 1024).spawn(move || {
                    tx.send(job().map(Worked::Device)).ok();
                });
                if started.is_err() {
                    self.set_did_failed(row, "could not start".into());
                    self.outcomes.push(Outcome { id, text: "it could not start".into(), error: true });
                    return self.beat();
                }
                self.busy = Busy::Tool { rx, id, row, done: failed_words.to_owned() };
                true
            }
            device::Plan::Ask(act) => {
                self.ask_first(id, act, None);
                true
            }
        }
    }

    /// A beat before the next call, if there is one: true while it waits.
    fn beat(&mut self) -> bool {
        if self.calls.is_empty() {
            return false;
        }
        self.busy = Busy::Beat(self.clock + BEAT);
        true
    }

    /// Something done on the handheld, in its row (`row`, where it was under
    /// way; else a new one): a briefing as its card. What it changed is
    /// applied at the next update; an app asked for opens after the answer.
    fn did(&mut self, id: String, row: Option<usize>, done: device::Done) {
        let entry = match &done.brief {
            Some(b) => Entry::briefing(b),
            None => Entry::Did { icon: done.icon.into(), words: done.words.clone(), detail: done.detail.clone(), state: StepState::Done },
        };
        match row.filter(|r| *r < self.entries.len()) {
            Some(r) => {
                self.entries[r].0 = entry;
                self.revision += 1;
            }
            None => self.push(entry),
        }
        match done.effect {
            Some(open @ device::Effect::Open { .. }) => self.open = Some((open, None)),
            Some(other) => self.effects.push(other),
            None => {}
        }
        self.outcomes.push(Outcome { id, text: done.text, error: false });
    }

    fn set_did_failed(&mut self, row: usize, why: String) {
        if let Some((Entry::Did { icon, detail, state, .. }, _)) = self.entries.get_mut(row) {
            *icon = "error".into();
            *detail = why;
            *state = StepState::Failed;
            self.revision += 1;
        }
    }

    /// A change shown in full before it is made: its card (in its row, when
    /// it was found on a thread), waiting for Enter (the confirm page) or Esc.
    fn ask_first(&mut self, id: String, act: device::Act, row: Option<usize>) {
        let card = Entry::Asks { name: act.title.clone(), command: String::new(), reason: act.what.clone(), state: AskState::Waiting };
        let row = match row.filter(|r| *r < self.entries.len()) {
            Some(r) => {
                self.entries[r].0 = card;
                self.revision += 1;
                r
            }
            None => {
                self.push(card);
                self.entries.len() - 1
            }
        };
        self.note_step(&format!("asks: {}", act.title.to_lowercase()));
        self.busy = Busy::Approving { id, pending: Pending::Device(act), row };
    }

    /// What the tools changed, applied to the settings (and drawn so at once).
    fn apply_effects(&mut self, settings: &mut crate::shell::Settings) {
        if self.effects.is_empty() {
            return;
        }
        for effect in std::mem::take(&mut self.effects) {
            match effect {
                device::Effect::Saver(on) => settings.battery.saver = on,
                device::Effect::Light(on) => settings.light = on,
                device::Effect::Brightness(level) => settings.brightness = level,
                device::Effect::Open { .. } => {}
            }
        }
        crate::save_settings(settings);
        crate::shell::settings_changed();
    }

    /// The pages of the newest answer that has any.
    fn last_sources(&self) -> Option<Vec<Source>> {
        self.entries.iter().rev().find_map(|(e, _)| if let Entry::Sources(s) = e { Some(s.clone()) } else { None })
    }

    /// Run an Action on a thread, and keep a record of it on disk.
    fn run(&mut self, id: String, saved: &Saved, line: String, reason: String, approved: bool) {
        let lower = saved.name.to_lowercase();
        self.push(Entry::Step { words: format!("running {lower}"), state: StepState::Working });
        let row = self.entries.len() - 1;
        let (tx, rx) = mpsc::channel();
        let name = saved.name.clone();
        let by = self.service.as_ref().map_or("", |(s, _)| s.label()).to_owned();
        let dir = runs_dir();
        std::thread::spawn(move || {
            let result = run_command(&line);
            keep_run(&dir, &name, &line, &reason, &by, approved, &result);
            tx.send(result.map(Worked::Text)).ok();
        });
        self.busy = Busy::Tool { rx, id, row, done: format!("ran {lower}") };
    }

    /// The person's yes or no to a critical Action, or to a change.
    fn approve(&mut self, yes: bool, robot: &Robot) {
        let Busy::Approving { id, pending, row } = std::mem::replace(&mut self.busy, Busy::Idle) else { return };
        if let Some((Entry::Asks { state, .. }, _)) = self.entries.get_mut(row) {
            *state = if yes { AskState::Yes } else { AskState::No };
            self.revision += 1;
        }
        match (yes, pending) {
            (true, Pending::Action { saved, line, reason }) => self.run(id, &saved, line, reason, true),
            (true, Pending::Device(act)) => {
                let words = act.title.to_lowercase();
                let p = act.run(&self.memory_file);
                if !self.device(id, p, &words) {
                    self.next_call(robot);
                }
            }
            (false, pending) => {
                let text = match pending {
                    Pending::Action { .. } => "The person declined: the Action was not run.",
                    Pending::Device(_) => "The person declined: nothing was changed.",
                };
                self.outcomes.push(Outcome { id, text: text.into(), error: true });
                self.next_call(robot);
            }
        }
    }

    /// Stop what is under way: the answer, a tool, or a question waiting.
    /// The on-device model stops writing at once.
    fn stop(&mut self) {
        crate::local::cancel();
        // An Action (a command) carries on by itself; a tool's thread is let go.
        let running = matches!(&self.busy, Busy::Tool { done, .. } if done.starts_with("ran "));
        self.busy = Busy::Idle;
        self.activity = None;
        self.open = None;
        self.by_rules = false;
        if let Some(chat) = &mut self.chat {
            chat.rewind(self.mark);
        }
        self.calls.clear();
        self.outcomes.clear();
        self.push(Entry::Note(if running { "stopped · the Action may still finish on its own".into() } else { "stopped".into() }));
    }

    fn scroll_by(&mut self, by: f32) {
        let max = self.laid.as_ref().map_or(0.0, |(_, _, laid)| laid.iter().map(|l| l.h + GAP).sum::<f32>());
        self.scroll.set((self.scroll.target() + by).clamp(0.0, max));
    }

    /// Every entry laid out at the type size in use (wrapping is only done
    /// again when something changed).
    fn lay_out(&mut self, g: &mut Gfx) {
        let chat = chat_style(g);
        let size = chat.size;
        if self.laid.as_ref().is_some_and(|(rev, s, _)| *rev == self.revision && *s == size) {
            return;
        }
        let ty = g.ty;
        let body_lh = chat_line(g);
        let cap_lh = ty.caption.size + 4.0;
        let mut laid = Vec::with_capacity(self.entries.len());
        for (entry, _) in &self.entries {
            laid.push(match entry {
                Entry::You(text) => {
                    let lines = wrap_paragraphs(g, text, chat, W * 0.72 - 20.0);
                    let h = lines.len() as f32 * body_lh + 8.0;
                    Laid { lines, h, levels: Vec::new() }
                }
                Entry::Said(text) => {
                    let lines = wrap_paragraphs(g, &units_together(text), chat, W - 2.0 * PAD - 8.0);
                    let h = lines.iter().map(|l| if l.is_empty() { body_lh * 0.5 } else { body_lh }).sum();
                    Laid { lines, h, levels: Vec::new() }
                }
                Entry::Step { .. } => Laid { lines: Vec::new(), h: cap_lh, levels: Vec::new() },
                Entry::Asks { reason, command, .. } => {
                    let mut lines = crate::help::wrap(g, &units_together(reason), ty.caption, W - 2.0 * PAD - 24.0);
                    // An Action's reason, its command under it; a change's
                    // words in full (they say what it does).
                    lines.truncate(if command.is_empty() { 6 } else { 3 });
                    let command_h = if command.is_empty() { 0.0 } else { cap_lh };
                    let h = 9.0 + ty.body.size + 5.0 + lines.len() as f32 * cap_lh + command_h + 3.0 + cap_lh + 7.0;
                    Laid { lines, h, levels: Vec::new() }
                }
                Entry::Did { words, detail, .. } => {
                    let width = W - 2.0 * PAD - 22.0;
                    let mut lines = crate::help::wrap(g, &units_together(words), ty.caption, width);
                    let first = lines.len();
                    if !detail.is_empty() {
                        lines.extend(crate::help::wrap(g, &units_together(detail), ty.caption, width));
                    }
                    // Its words' lines (`Some`), then the detail's.
                    let levels = (0..lines.len()).map(|k| (k < first).then_some(crate::wpilog::Level::Fine)).collect();
                    let h = lines.len() as f32 * cap_lh + 2.0;
                    Laid { lines, h, levels }
                }
                Entry::Briefing { rows, .. } => {
                    let (mut lines, mut levels) = (Vec::new(), Vec::new());
                    for (_, text, urgent) in rows {
                        for (k, line) in crate::help::wrap(g, &units_together(text), chat, W - 2.0 * PAD - 32.0).into_iter().enumerate() {
                            let level = if *urgent { crate::wpilog::Level::Warn } else { crate::wpilog::Level::Fine };
                            levels.push((k == 0).then_some(level));
                            lines.push(line);
                        }
                    }
                    let h = 8.0 + ty.body.size + 6.0 + lines.len() as f32 * body_lh + 6.0;
                    Laid { lines, h, levels }
                }
                Entry::Note(text) => {
                    let lines = crate::help::wrap(g, text, ty.caption, W - 2.0 * PAD - 22.0);
                    let h = lines.len() as f32 * cap_lh + 2.0;
                    Laid { lines, h, levels: Vec::new() }
                }
                Entry::Sources(_) => Laid { lines: Vec::new(), h: cap_lh + 2.0, levels: Vec::new() },
                Entry::Report { rows, .. } => {
                    let (mut lines, mut levels) = (Vec::new(), Vec::new());
                    for (level, row) in rows {
                        for (k, line) in crate::help::wrap(g, &units_together(row), chat, W - 2.0 * PAD - 30.0).into_iter().enumerate() {
                            levels.push((k == 0).then_some(*level));
                            lines.push(line);
                        }
                    }
                    let h = 8.0 + ty.body.size + 6.0 + lines.len() as f32 * body_lh + 6.0;
                    Laid { lines, h, levels }
                }
                Entry::Basis { words, .. } => {
                    let lines = crate::help::wrap(g, words, ty.caption, W - 2.0 * PAD - 22.0);
                    let h = lines.len() as f32 * cap_lh + 2.0;
                    Laid { lines, h, levels: Vec::new() }
                }
            });
        }
        self.laid = Some((self.revision, size, laid));
    }

    fn draw_empty(&self, g: &mut Gfx, area: Rect) {
        let t = g.tokens;
        let ty = g.ty;
        let cy = area.cy() - 4.0;
        g.icon("auto_awesome", false, W * 0.5, cy - 26.0, 22.0, t.dim);
        g.text_at(W * 0.5, cy, Align::Center, "Ask about the robot", ty.name, t.ink);
        // What else it does, the longest way of saying it that fits whole.
        let more = ["or have it set reminders, play music, brief you", "or set reminders, play music, brief you", "or set reminders · “brief me”"];
        let more = more.iter().copied().find(|m| g.measure(m, ty.caption) <= W - 2.0 * PAD).unwrap_or(more[2]);
        g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 4.0 + ty.caption.size * 0.5, Align::Center, more, ty.caption, t.dim);
        let model = self.service.as_ref().map_or("", |(s, _)| s.label());
        let line = format!("{model} · {} · tab: choices", self.prefs.effort.name());
        g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 8.0 + ty.caption.size * 1.5, Align::Center, &line, ty.caption, t.faint);
    }

    /// Offline, with nothing asked yet: the questions offered, to choose.
    fn offering(&self) -> bool {
        self.offline_mode() && self.entries.is_empty() && self.input.is_empty() && self.idle() && !self.suggestions.is_empty()
    }

    /// Offline, before anything is asked: that it is working offline and
    /// from what, then the questions it answers best, Up and Down to choose
    /// and Enter to ask (the first, the newest match's report, needs no model).
    fn draw_offline_empty(&mut self, g: &mut Gfx, area: Rect, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let head = area.y + 2.0 + ty.body.size * 0.5;
        g.icon("cloud_off", false, PAD + 9.0, head, 15.0, t.ink);
        let name = "Working offline";
        let name_w = g.measure(name, ty.body);
        g.text_at(PAD + 22.0, head, Align::Left, name, ty.body, t.ink);
        // From what: the logs, worded by the model when there is one.
        // The longest of these that fits whole beside the heading.
        let logs = robot.logs.list().iter().filter(|r| r.digest.is_some()).count();
        let choices = offline_from(logs, self.offline_model.as_deref());
        let room = W - 2.0 * PAD - 22.0 - name_w - 12.0;
        let from = choices.iter().find(|c| g.measure(c, ty.caption) <= room).or(choices.last()).cloned().unwrap_or_default();
        g.text_at(W - PAD - 4.0, head, Align::Right, &from, ty.caption, t.dim);
        let rows_top = head + ty.body.size * 0.5 + 5.0;
        if self.suggestions.is_empty() {
            let words = "No match logs are on this handheld yet. They come over from the robot after a match, and stay here unplugged.";
            let cap_lh = ty.caption.size + 4.0;
            for (k, line) in crate::help::wrap(g, words, ty.caption, W - 2.0 * PAD - 8.0).iter().enumerate() {
                g.text_at(PAD + 4.0, rows_top + 6.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, t.dim);
            }
            return;
        }
        let row_h = g.row_h();
        let n = self.suggestions.len();
        // The question line is drawn below the area: the list runs down to it.
        g.take_foot(area.bottom());
        for (i, y) in self.suggest.layout(g, n, rows_top, row_h, 2.0, area.bottom()) {
            let s = &self.suggestions[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            g.row(r, &s.name, &s.value, t.dim, i == self.suggest.focus);
        }
        self.suggest.scrollbar(g, n, rows_top, area.bottom());
    }

    /// The line typed into, along the bottom.
    fn draw_input(&self, g: &mut Gfx, bar: Rect, focused: bool) {
        let t = g.tokens;
        let chat = chat_style(g);
        let radius = bar.h * 0.5;
        g.surface(bar, radius, t.surface2);
        // A question heard, about to send itself: a thin track along the
        // line's foot fills, and when it is full the question goes.
        if let Some(left) = self.countdown {
            let done = (1.0 - left / AUTO_SEND).clamp(0.0, 1.0);
            let track = Rect::new(bar.x + radius, bar.bottom() - 4.0, bar.w - 2.0 * radius, 2.0).offset(g.ox, 0.0);
            g.canvas.rrect(track, 1.0, t.faint, 0.45);
            g.canvas.rrect(Rect::new(track.x, track.y, (track.w * done).max(2.0), track.h), 1.0, t.signal, 1.0);
        }
        let cy = bar.cy();
        let send_w = 22.0;
        if self.input.is_empty() {
            let words = match &self.busy {
                Busy::Idle if self.offering() => "or type a question",
                // With a model downloaded it answers anything, the match
                // logs from their facts and the rest from what it knows.
                Busy::Idle if self.offline_mode() && crate::local::ready() => "ask anything, or about a match",
                Busy::Idle if self.offline_mode() => "ask about a match",
                Busy::Idle | Busy::Approving { .. } => "type, or hold alt to talk",
                Busy::Offline { ready: Some(_), .. } => "esc: the answer from the facts now",
                _ => "esc: stop",
            };
            if focused {
                g.surface(Rect::new(bar.x + 12.0, cy - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
            }
            g.text_at(bar.x + 17.0, cy, Align::Left, words, chat, t.faint);
        } else {
            // A long line shows its end, where the typing is.
            let room = bar.w - 24.0 - send_w;
            let mut shown: &str = &self.input;
            while g.measure(shown, chat) > room {
                let mut chars = shown.chars();
                chars.next();
                shown = chars.as_str();
            }
            let w = g.text_at(bar.x + 12.0, cy, Align::Left, shown, chat, t.ink);
            if focused {
                g.surface(Rect::new(bar.x + 13.0 + w, cy - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
            }
            let ready = self.idle();
            g.icon("send", ready, bar.right() - 8.0 - send_w * 0.5, cy, 16.0, if ready { t.signal } else { t.faint });
        }
        if focused {
            g.focus_ring(bar, radius);
        }
    }
}

/// The words for a tool's row: while it works, and once it has.
struct StepWords {
    working: String,
    done: String,
}

/// A step done on the web: "searched the web: …", "read example.com",
/// "looked in example.com for …" (not "read Loop/", a group of values).
fn web_words(words: &str) -> bool {
    words.starts_with("searched the web") || words.starts_with("looked in ") || words.strip_prefix("read ").is_some_and(|host| host.contains('.') && !host.contains('/'))
}

fn step_words(call: &Call) -> StepWords {
    let same = |s: &str| StepWords { working: s.to_owned(), done: s.to_owned() };
    match call.name.as_str() {
        "robot_status" => same("looked at the robot"),
        "robot_values" => {
            let prefix = call.input["prefix"].as_str().unwrap_or_default().trim();
            same(&if prefix.is_empty() { "read the robot's groups".to_owned() } else { format!("read {prefix}") })
        }
        "robot_agent" => StepWords { working: "asking the robot's agent".into(), done: "asked the robot's agent".into() },
        "match_logs" => same("looked at the match logs"),
        "match_facts" => {
            let label = call.input["match"].as_str().unwrap_or_default().trim();
            same(&if label.is_empty() || label.eq_ignore_ascii_case("latest") { "read the newest match's log".to_owned() } else { format!("read {label}'s log") })
        }
        "list_actions" => same("looked at the actions"),
        "event_schedule" => same("looked at the schedule"),
        "graph_values" => same("looked at the live graphs"),
        "handheld_status" => same("looked at this handheld"),
        "read_file" => same(&format!("opened {}", call.input["path"].as_str().unwrap_or("a file").trim())),
        "run_action" => {
            let name = call.input["name"].as_str().unwrap_or_default().to_lowercase();
            StepWords { working: format!("running {name}"), done: format!("ran {name}") }
        }
        // Those that act: what it is doing, and what did not get done.
        "reminder_add" => StepWords { working: "setting a reminder".into(), done: "a reminder not set".into() },
        "timer" if call.input["action"] == "stop" => StepWords { working: "stopping a timer".into(), done: "no timer stopped".into() },
        "timer" => StepWords { working: "starting a timer".into(), done: "no timer started".into() },
        "note_add" => StepWords { working: "writing a note".into(), done: "the note not written".into() },
        "music_play" => StepWords { working: "finding music".into(), done: "nothing played".into() },
        "music_control" => {
            let working = match call.input["action"].as_str().unwrap_or_default() {
                "pause" => "pausing the music",
                "next" => "skipping a song",
                "previous" => "going back a song",
                _ => "playing again",
            };
            StepWords { working: working.into(), done: "the music not changed".into() }
        }
        "music_download" => StepWords { working: "finding the music".into(), done: "nothing downloaded".into() },
        "open_app" => {
            let app = call.input["app"].as_str().unwrap_or("the app").trim();
            StepWords { working: format!("opening {app}"), done: format!("{app} not opened") }
        }
        "setting_set" => {
            let what = call.input["setting"].as_str().unwrap_or("a setting").replace('_', " ").replace("wifi", "wi-fi");
            StepWords { working: format!("changing {what}"), done: format!("{what} not changed") }
        }
        "next_match" => StepWords { working: "looking at the schedule".into(), done: "the schedule not read".into() },
        "start_download" => StepWords { working: "starting a download".into(), done: "no download started".into() },
        "briefing" => StepWords { working: "making a briefing".into(), done: "no briefing made".into() },
        "memory_save" => StepWords { working: "remembering".into(), done: "not remembered".into() },
        "memory_forget" => StepWords { working: "forgetting".into(), done: "nothing forgotten".into() },
        other => same(&other.replace('_', " ")),
    }
}

/// The notification for a question answered, if it took `LONG_ANSWER` or
/// more and `reply` is the answer (a reply that calls tools is a step on the
/// way): that it is written, or, after steps, that they are done.
fn answered_notice(service: Service, reply: &Value, took: std::time::Duration, steps: usize, question: &str) -> Option<crate::notify::Notification> {
    let answer = match service {
        Service::Claude => reply["stop_reason"] == "end_turn",
        Service::OpenAi => reply["status"] == "completed" && !reply["output"].as_array().is_some_and(|o| o.iter().any(|i| i["type"] == "function_call")),
        Service::Local => true,
    };
    if took < LONG_ANSWER || !answer {
        return None;
    }
    let n = match steps {
        0 => crate::notify::Notification::new("Assistant", "auto_awesome", "your answer is written"),
        k => crate::notify::Notification::new("Assistant", "auto_awesome", format!("{} done", count_words(k, "step"))).body(short(question, 60)),
    };
    Some(n.tag("assistant:answer").action("builtin:Assistant"))
}

/// "1 step", "3 steps".
fn count_words(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// The conversation's text: the sans at the caption's size (which follows the
/// text size chosen), so more of it fits on the panel.
pub(crate) fn chat_style(g: &Gfx) -> bezel::TextStyle {
    bezel::TextStyle::body(g.ty.caption.size)
}

pub(crate) fn chat_line(g: &Gfx) -> f32 {
    g.ty.caption.size + 3.5
}

/// A number kept on the line with its unit ("6.3 V", "4 s", "8 min", "+1
/// more"): the space between them one that does not break.
fn units_together(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let mut out = String::with_capacity(text.len());
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            let unit = word.trim_end_matches([',', '.', ';', ')', ':']);
            let number = words[i - 1].chars().last().is_some_and(|c| c.is_ascii_digit());
            let held = number && matches!(unit, "V" | "A" | "s" | "ms" | "°C" | "m" | "h" | "min" | "%" | "rps" | "°" | "times" | "vs" | "MB" | "GB" | "more" | "left");
            out.push(if held { '\u{a0}' } else { ' ' });
        }
        out.push_str(word);
    }
    out
}

/// Text wrapped paragraph by paragraph; a blank line between paragraphs.
fn wrap_paragraphs(g: &mut Gfx, text: &str, style: bezel::TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(crate::help::wrap(g, paragraph, style, width));
        }
    }
    lines
}

impl Drop for Assistant {
    /// Closed for good while the on-device model answers: it stops writing.
    fn drop(&mut self) {
        if !self.idle() && matches!(self.service, Some((Service::Local, _))) {
            crate::local::cancel();
        }
    }
}

impl Screen for Assistant {
    fn title(&self) -> &str {
        "Assistant"
    }

    fn wants_text(&self) -> bool {
        self.service.is_some()
    }

    /// Copy: what is typed, else the newest answer (clipboard.rs).
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        if !self.input.trim().is_empty() {
            return Some(self.input.clone());
        }
        self.entries.iter().rev().find_map(|(e, _)| if let Entry::Said(text) = e { Some(text.clone()) } else { None })
    }

    fn keeps_awake(&self) -> bool {
        !self.idle()
    }

    fn busy(&self) -> bool {
        !self.idle()
    }

    /// Left while the on-device model answers: it stops, rather than writing
    /// on where nobody reads it (a service on the internet carries on).
    fn set_aside(&mut self) {
        if !self.idle() && matches!(self.service, Some((Service::Local, _))) {
            self.stop();
        }
    }

    /// Push-to-talk here asks it: what was said is the question.
    fn dictation(&self) -> Option<crate::voice::Dictation> {
        self.service.is_some().then_some(crate::voice::Dictation::Question)
    }

    /// The question heard lands in the line, shown for a moment before it
    /// sends itself; any key stops that, to edit it first. Words already
    /// typed stay in front of it.
    fn dictated(&mut self, text: &str, cx: &mut Cx) -> Nav {
        let line = if self.input.trim().is_empty() { text.trim().to_owned() } else { format!("{} {}", self.input.trim_end(), text.trim()) };
        self.input = line.chars().filter(|c| !c.is_control()).take(INPUT_LIMIT).collect();
        self.asked_aloud = true;
        if self.idle() {
            self.countdown = Some(AUTO_SEND);
            *cx.say = Some("sending · any key edits".into());
        } else {
            // It waits in the line; Enter sends it once this answer is in.
            *cx.say = Some("still answering".into());
        }
        Nav::Stay
    }

    fn resume(&mut self) {
        // A kept conversation chosen: back to it, with its own service.
        if let Some(id) = take_open_chat() {
            self.prefs = Prefs::load();
            if let Err(why) = self.reopen(&id) {
                self.start_over();
                self.push(Entry::Note(why));
            }
            return;
        }
        // A key added or the service changed: a fresh conversation, as the
        // services cannot carry on each other's.
        let prefs = Prefs::load();
        let now = answering(&prefs, &Keys::load());
        let changed = now.as_ref().map(|(s, _)| *s) != self.service.as_ref().map(|(s, _)| *s);
        if NEW_CHAT.swap(false, Ordering::Relaxed) || changed {
            self.start_over();
        } else {
            self.prefs = prefs;
            self.service = now;
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.service.is_none() {
            return match key {
                Key::Enter | Key::Right => Nav::Push(Box::new(crate::screens::SettingsApp::keys_page())),
                Key::Back | Key::Leave => Nav::Pop,
                _ => Nav::Stay,
            };
        }
        // A question heard and about to send itself: any key stops that and
        // does what it does (Enter sends it now, Esc clears it).
        self.countdown = None;
        self.see_settings(cx.settings);
        let line = g_line_height();
        // Offline, before anything is asked: the questions offered.
        let offering = self.offline_mode() && self.entries.is_empty() && self.input.is_empty() && self.idle() && !self.suggestions.is_empty();
        match key {
            Key::Up | Key::Down if offering => crate::screens::list_key(&mut self.suggest, key, self.suggestions.len()),
            Key::Enter if offering => {
                let chosen = self.suggestions.get(self.suggest.focus).cloned();
                match chosen.and_then(|s| s.question) {
                    Some(q) => {
                        self.input = q;
                        self.send(cx.robot);
                    }
                    None => self.report(cx.robot),
                }
            }
            Key::Up => self.scroll_by(line * 3.0),
            Key::Down => self.scroll_by(-line * 3.0),
            Key::PageUp => self.scroll_by(line * 8.0),
            Key::PageDown => self.scroll_by(-line * 8.0),
            Key::Tab => return Nav::Push(Box::new(Choices::new())),
            Key::Ctrl('n') => {
                if self.idle() {
                    self.start_over();
                    *cx.say = Some("a new conversation".into());
                }
            }
            // A briefing, from any service (offline, from the template).
            Key::Ctrl('b') => {
                if self.idle() {
                    self.input = "Brief me".into();
                    self.send(cx.robot);
                } else {
                    *cx.say = Some("still answering · esc stops it".into());
                }
            }
            // The newest match's report, from its log alone: any service, no model.
            Key::Ctrl('r') => {
                if self.idle() {
                    self.report(cx.robot);
                } else {
                    *cx.say = Some("still answering · esc stops it".into());
                }
            }
            // Offline, the model still wording it: the answer from the facts, now.
            Key::Back if matches!(self.busy, Busy::Offline { ready: Some(_), .. }) => {
                if let Busy::Offline { ready: Some(answer), .. } = std::mem::replace(&mut self.busy, Busy::Idle) {
                    self.offline_done(*answer);
                    *cx.say = Some("the answer from the facts, not waiting for the model".into());
                }
            }
            Key::Leave => return Nav::Pop,
            // A critical Action: Enter opens the page that asks for certain;
            // Esc skips it here.
            _ if matches!(self.busy, Busy::Approving { .. }) => match key {
                Key::Enter => {
                    if let Busy::Approving { pending, .. } = &self.busy {
                        *DECISION.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        return Nav::Push(Box::new(match pending {
                            Pending::Action { saved, line, reason } => Confirm::new(&saved.name, line, reason),
                            Pending::Device(act) => Confirm::change(act),
                        }));
                    }
                }
                Key::Back => self.approve(false, cx.robot),
                _ => {}
            },
            Key::Back if !self.input.is_empty() => self.input.clear(),
            Key::Back if !self.idle() => self.stop(),
            Key::Back => return Nav::Pop,
            Key::Enter if !self.idle() => *cx.say = Some("still answering · esc stops it".into()),
            // Nothing typed: the newest answer's pages, to open one.
            Key::Enter if self.input.trim().is_empty() => {
                if let Some(sources) = self.last_sources() {
                    return Nav::Push(Box::new(SourcesPage::new(sources)));
                }
            }
            Key::Enter => self.send(cx.robot),
            Key::Backspace => {
                self.input.pop();
            }
            Key::Char(c) if !c.is_control() && self.input.chars().count() < INPUT_LIMIT => self.input.push(c),
            _ => {}
        }
        // What was said, cleared away: what is typed next was not.
        if self.input.is_empty() {
            self.asked_aloud = false;
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        self.see_settings(cx.settings);
        // A question from search: sent as soon as nothing else is under way
        // (with no service yet, it waits typed in the line).
        if let Some(question) = take_question() {
            self.input = question;
            if self.idle() && self.service.is_some() {
                self.send(cx.robot);
            }
        }
        // Offline, the questions offered: from the logs there are, worked
        // out again only when they change (or the robot comes or goes).
        if self.offline_mode() && self.entries.is_empty() && self.idle() {
            let now = (cx.robot.logs.generation(), cx.robot.linked());
            if self.suggested != Some(now) {
                self.suggested = Some(now);
                self.suggestions = suggestions_for(cx.robot);
                self.suggest.focus = self.suggest.focus.min(self.suggestions.len().saturating_sub(1));
            }
        }
        // A question heard, left alone long enough: it goes.
        if self.countdown.is_some_and(|left| left <= 0.0) {
            self.countdown = None;
            self.send(cx.robot);
        }
        // Back from the confirm page with an answer.
        if matches!(self.busy, Busy::Approving { .. }) {
            if let Some(yes) = DECISION.lock().unwrap_or_else(|e| e.into_inner()).take() {
                self.approve(yes, cx.robot);
            }
        }
        match &self.busy {
            Busy::Asking(rx) => {
                // The simulator's script answers at once; it is held as long
                // as a real answer would take, so the page reads the same.
                if demo() && self.waited < DEMO_PAUSE {
                    return Nav::Stay;
                }
                match rx.try_recv() {
                    Ok(result) => self.took(result, cx.robot),
                    Err(TryRecvError::Disconnected) => self.failed("the request stopped".into()),
                    Err(TryRecvError::Empty) => {}
                }
            }
            Busy::Tool { rx, .. } => match rx.try_recv() {
                Ok(result) => {
                    let Busy::Tool { id, row, done, .. } = std::mem::replace(&mut self.busy, Busy::Idle) else { return Nav::Stay };
                    let doing = matches!(self.entries.get(row), Some((Entry::Did { .. }, _)));
                    match result {
                        Ok(Worked::Text(text)) => {
                            self.set_step(row, true, done);
                            self.outcomes.push(Outcome { id, text, error: false });
                        }
                        Ok(Worked::Device(device::Step::Done(d))) => self.did(id, Some(row), d),
                        Ok(Worked::Device(device::Step::Ask(act))) => {
                            self.ask_first(id, act, Some(row));
                            return Nav::Stay;
                        }
                        Err(why) if doing => {
                            self.set_did_failed(row, why.clone());
                            if let Some((Entry::Did { words, .. }, _)) = self.entries.get_mut(row) {
                                *words = done;
                            }
                            self.outcomes.push(Outcome { id, text: why, error: true });
                        }
                        Err(why) => {
                            self.set_step(row, false, done);
                            self.outcomes.push(Outcome { id, text: why, error: true });
                        }
                    }
                    self.next_call(cx.robot);
                }
                Err(TryRecvError::Disconnected) => {
                    let Busy::Tool { id, row, done, .. } = std::mem::replace(&mut self.busy, Busy::Idle) else { return Nav::Stay };
                    self.set_step(row, false, done);
                    self.set_did_failed(row, "it stopped without an answer".into());
                    self.outcomes.push(Outcome { id, text: "it stopped without an answer".into(), error: true });
                    self.next_call(cx.robot);
                }
                Err(TryRecvError::Empty) => {}
            },
            Busy::Beat(until) => {
                if self.clock >= *until {
                    self.busy = Busy::Idle;
                    self.next_call(cx.robot);
                }
            }
            Busy::Briefing(rx) => match rx.try_recv() {
                Ok(b) => self.brief_done(b),
                Err(TryRecvError::Disconnected) => self.failed("the briefing stopped".into()),
                Err(TryRecvError::Empty) => {}
            },
            Busy::Offline { .. } => loop {
                let got = match &self.busy {
                    Busy::Offline { rx, .. } => rx.try_recv(),
                    _ => break,
                };
                match got {
                    Ok(crate::offline::Event::Step(words)) => self.push(Entry::Step { words, state: StepState::Done }),
                    Ok(crate::offline::Event::Wording(name)) => {
                        if let Busy::Offline { wording, .. } = &mut self.busy {
                            *wording = Some(name);
                        }
                    }
                    Ok(crate::offline::Event::Ready(answer)) => {
                        if let Busy::Offline { ready, .. } = &mut self.busy {
                            *ready = Some(answer);
                        }
                    }
                    Ok(crate::offline::Event::Done(answer)) => {
                        self.offline_done(*answer);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.failed("the answer stopped".into());
                        break;
                    }
                }
            },
            Busy::Idle | Busy::Approving { .. } => {}
        }
        // What the tools changed: the settings, drawn so at once.
        self.apply_effects(cx.settings);
        // An app asked for, once the answer has been there a moment.
        if self.idle() {
            if let Some((_, Some(at))) = &self.open {
                if self.clock >= *at {
                    if let Some((device::Effect::Open { app, setting, .. }, _)) = self.open.take() {
                        let screen = match (app, setting) {
                            (Some(i), _) => crate::screens::open_builtin(i, cx.robot),
                            (None, Some(i)) => crate::screens::open_setting(i),
                            (None, None) => return Nav::Stay,
                        };
                        return Nav::Push(screen);
                    }
                }
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.waited += dt;
        let moving = self.scroll.tick(dt);
        let rising = self.entries.last().is_some_and(|(_, born)| self.clock - born < 0.8);
        let counting = match &mut self.countdown {
            Some(left) => {
                *left -= dt;
                true
            }
            None => false,
        };
        // An app to open in a moment: frames keep coming until it does.
        let opening = self.open.as_ref().is_some_and(|(_, at)| at.is_some());
        moving || rising || counting || opening || !self.idle()
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        LINE.with(|l| l.set(chat_line(g)));
        let chat = chat_style(g);
        let lh = chat_line(g);
        if self.service.is_none() {
            g.empty("key", "add an OpenAI or Claude key to ask it anything");
            g.hint("enter: settings › keys");
            return;
        }
        if g.calm {
            self.scroll.jump(self.scroll.target());
        }
        let bar_h = chat.size + 13.0;
        let bar = Rect::new(PAD, H - PAD - bar_h, W - 2.0 * PAD, bar_h);
        let area = Rect::new(0.0, g.top(), W, bar.y - 4.0 - g.top());
        let approving = matches!(self.busy, Busy::Approving { .. });
        if self.entries.is_empty() {
            if self.offline_mode() {
                self.draw_offline_empty(g, area, robot);
                let offering = self.offering();
                self.draw_input(g, bar, !offering);
            } else {
                self.draw_empty(g, area);
                self.draw_input(g, bar, true);
            }
            return;
        }
        self.lay_out(g);
        let Some((_, _, laid)) = self.laid.clone() else { return };
        let total: f32 = laid.iter().map(|l| l.h + GAP).sum::<f32>() + if self.idle() || approving { 0.0 } else { ty.caption.size + 10.0 };
        let max = (total - area.h).max(0.0);
        // A card taller than the room left (a briefing's): shown from its
        // top, the rest a scroll down, rather than from its foot.
        if let Some(i) = self.land.take().filter(|i| *i < laid.len()) {
            let from_top: f32 = laid[i..].iter().map(|l| l.h + GAP).sum::<f32>() - GAP;
            self.scroll.set((from_top - area.h + 2.0).clamp(0.0, max));
        }
        if self.scroll.target() > max {
            self.scroll.set(max);
        }
        let scroll = self.scroll.value().clamp(0.0, max.max(self.scroll.value()));
        g.canvas.set_clip(area);
        let mut y = area.bottom() + scroll;
        // Thinking, under everything said so far.
        if !self.idle() && !approving {
            let h = ty.caption.size + 10.0;
            y -= h;
            let cy = y + h * 0.5;
            let calm = g.calm;
            for i in 0..3 {
                let phase = self.clock * 4.5 - i as f32 * 0.8;
                let a = if calm { 0.7 } else { 0.35 + 0.65 * (0.5 + 0.5 * phase.sin()) };
                g.canvas.rrect(Rect::new(PAD + 6.0 + i as f32 * 8.0 - 2.5 + g.ox, cy - 2.5, 5.0, 5.0), 2.5, t.ink, a);
            }
            let words = match &self.busy {
                Busy::Asking(_) => format!("thinking · {}", self.service.as_ref().map_or("", |(s, _)| s.label())),
                Busy::Offline { wording: Some(model), .. } => format!("wording it · {}", model.to_lowercase()),
                Busy::Offline { .. } => "working offline".into(),
                Busy::Briefing(_) => "making a briefing".into(),
                // Doing what was asked, one step after another: which, of the round's.
                Busy::Tool { .. } | Busy::Beat(_) if self.batch.1 > 1 => {
                    // In the beat between two, the next is the one coming.
                    let next = matches!(self.busy, Busy::Beat(_)) as usize;
                    let k = (self.steps.saturating_sub(self.batch.0) + next).clamp(1, self.batch.1);
                    format!("working · step {k} of {}", self.batch.1)
                }
                _ => "working".into(),
            };
            let words = g.text.fit(&words, &ty.caption, W - PAD - 34.0 - PAD);
            g.text_at(PAD + 34.0, cy, Align::Left, &words, ty.caption, t.dim);
        }
        let mut ring: Option<(Rect, f32)> = None;
        let newest_sources = self.entries.iter().rposition(|(e, _)| matches!(e, Entry::Sources(_))).unwrap_or(usize::MAX);
        for (i, ((entry, born), l)) in self.entries.iter().zip(&laid).enumerate().rev() {
            y -= l.h;
            if y + l.h < area.y {
                break;
            }
            if y > area.bottom() {
                y -= GAP;
                continue;
            }
            // The newest rise in from just below where they land.
            let since = self.clock - born;
            let left = if g.calm || since > 1.0 { 0.0 } else { role::release().solve(1.0, 0.0, since.max(0.0)).0 };
            let a = ((1.0 - left) * 1.6).clamp(0.0, 1.0);
            let dy = left * 10.0;
            let top = y + dy;
            // Lines fade out under the title rather than being cut there, and
            // likewise above the line typed into once scrolled up from it.
            let fade = |cy: f32| {
                let under_title = ((cy - area.y) / EDGE).clamp(0.0, 1.0);
                let over_bar = ((area.bottom() - cy) / EDGE).clamp(0.0, 1.0).max(1.0 - scroll / EDGE);
                a * under_title * over_bar
            };
            match entry {
                Entry::You(_) => {
                    let widest = l.lines.iter().map(|s| g.measure(s, chat)).fold(0.0, f32::max);
                    let r = Rect::new(W - PAD - widest - 18.0, top, widest + 18.0, l.h);
                    g.canvas.rrect(r.offset(g.ox, 0.0), 10.0_f32.min(l.h * 0.5), t.surface2, fade(r.y + 7.0).min(fade(r.bottom() - 7.0)));
                    for (k, line) in l.lines.iter().enumerate() {
                        let cy = top + 4.0 + (k as f32 + 0.5) * lh;
                        g.text_alpha(r.x + 9.0, cy, line, chat, t.ink, fade(cy));
                    }
                }
                Entry::Said(_) => {
                    let mut ly = top;
                    for line in &l.lines {
                        let this = if line.is_empty() { lh * 0.5 } else { lh };
                        if !line.is_empty() {
                            g.text_alpha(PAD + 2.0, ly + this * 0.5, line, chat, t.ink, fade(ly + this * 0.5));
                        }
                        ly += this;
                    }
                }
                Entry::Step { words, state } => {
                    let cy = top + l.h * 0.5;
                    let (icon, color) = match state {
                        StepState::Working => ("schedule", t.dim),
                        StepState::Done if words.starts_with("ran") || words.starts_with("running") => ("play_circle", t.dim),
                        StepState::Done if web_words(words) => ("language", t.dim),
                        StepState::Done => ("visibility", t.dim),
                        StepState::Failed => ("error", g.fault()),
                    };
                    g.icon_alpha(icon, false, PAD + 8.0, cy, 13.0, color, fade(cy));
                    let shown = if *state == StepState::Failed { format!("{words} · failed") } else { words.clone() };
                    let shown = g.text.fit(&shown, &ty.caption, W - 2.0 * PAD - 22.0);
                    g.text_alpha(PAD + 20.0, cy, &shown, ty.caption, t.dim, fade(cy));
                }
                Entry::Asks { name, command, state, .. } => {
                    let r = Rect::new(PAD, top, W - 2.0 * PAD, l.h);
                    let radius = g.tile_radius().min(14.0);
                    let card = fade(r.y + 10.0).min(fade(r.bottom() - 10.0));
                    g.canvas.rrect(r.offset(g.ox, 0.0), radius, t.surface1, card);
                    let mut ly = r.y + 9.0 + ty.body.size * 0.5;
                    g.icon_alpha("front_hand", *state == AskState::Waiting, r.x + 16.0, ly, 14.0, if *state == AskState::Waiting { t.signal } else { t.dim }, fade(ly));
                    // An Action, with its command; or a change, in words.
                    let change = command.is_empty();
                    let title = if change { format!("{name}?") } else { format!("Run {name}?") };
                    let title = g.text.fit(&title, &ty.body, r.w - 40.0);
                    g.text_alpha(r.x + 29.0, ly, &title, ty.body, t.ink, fade(ly));
                    ly += ty.body.size * 0.5 + 5.0;
                    let cap_lh = ty.caption.size + 4.0;
                    for line in &l.lines {
                        g.text_alpha(r.x + 12.0, ly + cap_lh * 0.5, line, ty.caption, t.dim, fade(ly + cap_lh * 0.5));
                        ly += cap_lh;
                    }
                    if !change {
                        let command = g.text.fit(command, &ty.caption, r.w - 24.0);
                        g.text_alpha(r.x + 12.0, ly + cap_lh * 0.5, &command, ty.caption, t.faint, fade(ly + cap_lh * 0.5));
                        ly += cap_lh;
                    }
                    ly += 3.0;
                    let (words, color) = match state {
                        AskState::Waiting => ("enter: confirm · esc: skip", t.ink),
                        AskState::Yes if change => ("you said yes", t.dim),
                        AskState::Yes => ("you said run it", t.dim),
                        AskState::No => ("you skipped it", t.dim),
                    };
                    let words = g.text.fit(words, &ty.caption, r.w - 24.0);
                    g.text_alpha(r.x + 12.0, ly + cap_lh * 0.5, &words, ty.caption, color, fade(ly + cap_lh * 0.5));
                    if *state == AskState::Waiting && approving && i + 1 == self.entries.len() {
                        ring = Some((r, radius));
                    }
                }
                Entry::Note(_) => {
                    let cap_lh = ty.caption.size + 4.0;
                    g.icon_alpha("warning", false, PAD + 8.0, top + cap_lh * 0.5, 13.0, t.dim, fade(top + cap_lh * 0.5));
                    for (k, line) in l.lines.iter().enumerate() {
                        let cy = top + (k as f32 + 0.5) * cap_lh;
                        g.text_alpha(PAD + 20.0, cy, line, ty.caption, t.dim, fade(cy));
                    }
                }
                Entry::Report { label, title, .. } => {
                    // The report: a card of rows, each a fact with its level.
                    let r = Rect::new(PAD, top, W - 2.0 * PAD, l.h);
                    let radius = g.tile_radius().min(14.0);
                    // A card taller than what shows stays, cut by the edges
                    // rather than faded by its own (scrolled away) ends.
                    let card = if r.y < area.y || r.bottom() > area.bottom() { a } else { fade(r.y + 10.0).min(fade(r.bottom() - 10.0)) };
                    g.canvas.rrect(r.offset(g.ox, 0.0), radius, t.surface1, card);
                    let ly = r.y + 8.0 + ty.body.size * 0.5;
                    g.icon_alpha("insights", false, r.x + 16.0, ly, 14.0, t.ink, fade(ly));
                    let head = format!("{label} report");
                    let head_w = g.measure(&head, ty.body);
                    g.text_alpha(r.x + 29.0, ly, &head, ty.body, t.ink, fade(ly));
                    // When it was: the title's date and time ("nov 16 · 10:15").
                    let parts: Vec<&str> = title.split(" · ").collect();
                    let when = if parts.len() >= 2 { parts[parts.len() - 2..].join(" · ") } else { title.clone() };
                    let when = g.text.fit(&when, &ty.caption, r.w - 29.0 - head_w - 20.0);
                    let when_w = g.measure(&when, ty.caption);
                    g.text_alpha(r.right() - 12.0 - when_w, ly, &when, ty.caption, t.faint, fade(ly));
                    let mut ly = ly + ty.body.size * 0.5 + 6.0;
                    let mut row_level = crate::wpilog::Level::Fine;
                    for (line, level) in l.lines.iter().zip(&l.levels) {
                        let cy = ly + lh * 0.5;
                        if let Some(level) = level {
                            row_level = *level;
                            let color = match level {
                                crate::wpilog::Level::Fault => g.fault(),
                                crate::wpilog::Level::Warn => t.amber_ink,
                                crate::wpilog::Level::Fine => t.dim,
                            };
                            g.canvas.circle(r.x + 15.0 + g.ox, cy, 2.5, color, fade(cy));
                        }
                        let ink = if row_level == crate::wpilog::Level::Fine { t.dim } else { t.ink };
                        g.text_alpha(r.x + 24.0, cy, line, chat, ink, fade(cy));
                        ly += lh;
                    }
                }
                Entry::Did { icon, state, .. } => {
                    // Something done on the handheld: its icon (the signal's
                    // colour while under way, ice once done), what, and how.
                    let cap_lh = ty.caption.size + 4.0;
                    let icon_y = top + cap_lh * 0.5;
                    let (icon, color, filled, alpha) = match state {
                        StepState::Working => {
                            let pulse = if g.calm { 1.0 } else { 0.55 + 0.45 * (0.5 + 0.5 * (self.clock * 5.0).sin()) };
                            (icon.as_str(), t.signal, false, pulse)
                        }
                        StepState::Done => (icon.as_str(), g.good(), true, 1.0),
                        StepState::Failed => ("error", g.fault(), false, 1.0),
                    };
                    g.icon_alpha(icon, filled, PAD + 8.0, icon_y, 13.0, color, fade(icon_y) * alpha);
                    for (k, (line, level)) in l.lines.iter().zip(&l.levels).enumerate() {
                        let cy = top + (k as f32 + 0.5) * cap_lh;
                        let ink = if level.is_some() { t.ink } else { t.dim };
                        g.text_alpha(PAD + 20.0, cy, line, ty.caption, ink, fade(cy));
                    }
                }
                Entry::Briefing { title, when, rows } => {
                    // The briefing: a card of short rows, each with its icon;
                    // what wants doing soon marked in the signal's colour.
                    let r = Rect::new(PAD, top, W - 2.0 * PAD, l.h);
                    let radius = g.tile_radius().min(14.0);
                    let card = if r.y < area.y || r.bottom() > area.bottom() { a } else { fade(r.y + 10.0).min(fade(r.bottom() - 10.0)) };
                    g.canvas.rrect(r.offset(g.ox, 0.0), radius, t.surface1, card);
                    let ly = r.y + 8.0 + ty.body.size * 0.5;
                    g.icon_alpha("auto_awesome", true, r.x + 16.0, ly, 14.0, t.signal, fade(ly));
                    let when_w = g.measure(when, ty.caption);
                    let head = g.text.fit(title, &ty.body, r.w - 29.0 - when_w - 22.0);
                    g.text_alpha(r.x + 29.0, ly, &head, ty.body, t.ink, fade(ly));
                    g.text_alpha(r.right() - 12.0 - when_w, ly, when, ty.caption, t.faint, fade(ly));
                    let mut ly = ly + ty.body.size * 0.5 + 6.0;
                    let mut row = 0;
                    for (line, level) in l.lines.iter().zip(&l.levels) {
                        let cy = ly + lh * 0.5;
                        if let Some(level) = level {
                            let urgent = *level != crate::wpilog::Level::Fine;
                            if let Some((icon, _, _)) = rows.get(row) {
                                g.icon_alpha(icon, urgent, r.x + 16.0, cy, 12.0, if urgent { t.signal } else { t.dim }, fade(cy));
                            }
                            row += 1;
                        }
                        g.text_alpha(r.x + 28.0, cy, line, chat, t.ink, fade(cy));
                        ly += lh;
                    }
                }
                Entry::Basis { checked, .. } => {
                    // What an offline answer rests on, under it.
                    let cap_lh = ty.caption.size + 4.0;
                    let icon = if *checked { "check_circle" } else { "description" };
                    g.icon_alpha(icon, false, PAD + 8.0, top + cap_lh * 0.5, 13.0, t.dim, fade(top + cap_lh * 0.5));
                    for (k, line) in l.lines.iter().enumerate() {
                        let cy = top + (k as f32 + 0.5) * cap_lh;
                        g.text_alpha(PAD + 20.0, cy, line, ty.caption, t.dim, fade(cy));
                    }
                }
                Entry::Sources(list) => {
                    // The pages an answer rests on, by site; Enter opens them
                    // while this is the newest and nothing is typed.
                    let cy = top + l.h * 0.5;
                    g.icon_alpha("link", false, PAD + 8.0, cy, 13.0, t.dim, fade(cy));
                    let opens = i == newest_sources && self.input.is_empty() && self.idle();
                    let right = if opens { "enter ›" } else { "" };
                    let right_w = if opens { g.measure(right, ty.caption) + 8.0 } else { 0.0 };
                    let hosts: Vec<String> = list.iter().map(|s| host_of(&s.url)).collect();
                    let words = g.text.fit(&format!("sources · {}", hosts.join(" · ")), &ty.caption, W - 2.0 * PAD - 22.0 - right_w);
                    g.text_alpha(PAD + 20.0, cy, &words, ty.caption, t.dim, fade(cy));
                    if opens {
                        g.text_alpha(W - PAD - right_w + 6.0, cy, right, ty.caption, t.faint, fade(cy));
                    }
                }
            }
            y -= GAP;
        }
        g.canvas.clear_clip();
        if let Some((r, radius)) = ring {
            g.focus_ring(r, radius);
        }
        self.draw_input(g, bar, ring.is_none());
    }
}

thread_local! {
    /// The line height at the text size last drawn, for scrolling by lines.
    static LINE: std::cell::Cell<f32> = const { std::cell::Cell::new(17.0) };
}

fn g_line_height() -> f32 {
    LINE.with(std::cell::Cell::get)
}

// --------------------------------------------------------------- confirm ---

/// A critical Action asked about in full before it runs: what it is, why the
/// assistant wants it, the command exactly as it will run, and run it or not
/// (not, unless chosen).
pub struct Confirm {
    name: String,
    command: String,
    reason: String,
    list: ListState,
    /// Asked about from search: on a yes the command runs here, in a
    /// terminal that takes the page's place, rather than for the assistant.
    run_here: bool,
    critical: bool,
    session: Option<crate::terminal::Terminal>,
    /// The terminal has just taken the page's place: its rows rise in.
    swapped: bool,
    /// A change to the handheld rather than a command: what it does, in
    /// words, in the box; and the two answers' words.
    change: bool,
    yes: String,
    no: String,
}

impl Confirm {
    fn new(name: &str, command: &str, reason: &str) -> Self {
        let mut list = ListState::default();
        list.focus = 1;
        Confirm {
            name: name.to_owned(),
            command: command.to_owned(),
            reason: reason.to_owned(),
            list,
            run_here: false,
            critical: true,
            session: None,
            swapped: false,
            change: false,
            yes: "Run it".into(),
            no: "Don't run it".into(),
        }
    }

    /// A change the assistant would make to the handheld (Wi-Fi off, an
    /// update, memory forgotten), asked about in full: what, what it does,
    /// and yes or no (no, unless chosen).
    fn change(act: &device::Act) -> Self {
        let mut page = Self::new(&act.title, &act.what, "");
        page.change = true;
        (page.yes, page.no) = (act.yes.clone(), act.no.clone());
        page
    }

    /// An Action picked in search, asked about in full as the assistant's
    /// critical ones are, and run here on a yes. One that only looks is
    /// offered "Run it" first; one that could change something, not.
    pub fn to_run(name: &str, command: &str, critical: bool) -> Self {
        let mut page = Self::new(name, command, if critical { "it could change something" } else { "it only looks" });
        (page.run_here, page.critical) = (true, critical);
        if !critical {
            page.list.focus = 0;
        }
        page
    }

    fn answer(&mut self, yes: bool) -> Nav {
        if self.run_here {
            if !yes {
                return Nav::Pop;
            }
            let banner = format!("$ {}", self.command);
            self.session = Some(crate::terminal::Terminal::running("Command", &self.command, crate::connect::session_dir()).with_banner(&banner));
            self.swapped = true;
            return Nav::Stay;
        }
        *DECISION.lock().unwrap_or_else(|e| e.into_inner()) = Some(yes);
        Nav::Pop
    }
}

/// A command broken into lines by width alone: it may have no spaces to
/// break at.
fn wrap_command(g: &mut Gfx, text: &str, style: bezel::TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for c in text.chars() {
        line.push(c);
        if g.measure(&line, style) > width && line.chars().count() > 1 {
            line.pop();
            lines.push(std::mem::take(&mut line));
            line.push(c);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl Screen for Confirm {
    fn title(&self) -> &str {
        self.session.as_ref().map_or("Confirm", |s| s.title())
    }

    // Once the command runs here, the page is its terminal.
    fn wants_text(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.wants_text())
    }

    fn busy(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.busy())
    }

    fn keeps_awake(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.keeps_awake())
    }

    /// A passing page; and a command in a terminal of its own ends when left.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.session.as_mut().is_some_and(|s| s.tick(dt))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(session) = &mut self.session {
            return session.key(key, cx);
        }
        match key {
            Key::Back | Key::Leave => return self.answer(false),
            Key::Enter => {
                let yes = self.list.focus == 0;
                let nav = self.answer(yes);
                // Run here: the terminal says how to leave it, as one opened does.
                if let Some(words) = self.session.as_ref().and_then(|s| s.greeting()) {
                    *cx.say = Some(words.to_owned());
                }
                return nav;
            }
            // A change's two answers sit side by side.
            Key::Left if self.change => self.list.focus = 0,
            Key::Right if self.change => self.list.focus = 1,
            other => crate::screens::list_key(&mut self.list, other, 2),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        if let Some(session) = &mut self.session {
            if std::mem::take(&mut self.swapped) {
                g.reveal_start();
                g.started_moving();
            }
            return session.draw(g, robot);
        }
        if self.change {
            return self.draw_change(g);
        }
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        // Its two rows rest on the hint, with room under them for the ring.
        let rows_top = bottom - 2.0 * row_h - 3.0 - 4.0;
        let cap_lh = ty.caption.size + 4.0;
        // What it is, and why.
        let mut y = top + 1.0;
        let cy = y + ty.name.size * 0.5;
        g.icon("front_hand", true, PAD + 8.0, cy, 16.0, t.signal);
        let title = g.text.fit(&self.name, &ty.name, W - 2.0 * PAD - 24.0);
        g.text_at(PAD + 22.0, cy, Align::Left, &title, ty.name, t.ink);
        y += ty.name.size + 4.0;
        let why = if self.reason.is_empty() { "the assistant wants to run it" } else { self.reason.as_str() };
        for line in crate::help::wrap(g, why, ty.caption, W - 2.0 * PAD).into_iter().take(2) {
            g.text_at(PAD, y + cap_lh * 0.5, Align::Left, &line, ty.caption, t.dim);
            y += cap_lh;
        }
        // The command itself, as it will run.
        y += 3.0;
        let fits = (((rows_top - 5.0 - y) - 8.0) / cap_lh).floor().max(1.0) as usize;
        let mut lines = wrap_command(g, &self.command, ty.caption, W - 2.0 * PAD - 16.0);
        if lines.len() > fits {
            lines.truncate(fits);
            if let Some(last) = lines.last_mut() {
                last.pop();
                last.push('…');
            }
        }
        let box_h = lines.len() as f32 * cap_lh + 8.0;
        g.surface(Rect::new(PAD, y, W - 2.0 * PAD, box_h), 8.0, t.surface2);
        for (k, line) in lines.iter().enumerate() {
            g.text_at(PAD + 8.0, y + 4.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, t.ink);
        }
        // Run it, or not.
        for (i, ry) in self.list.layout(g, 2, rows_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, ry, W - 2.0 * PAD, row_h);
            let (name, value) = if i == 0 { ("Run it", if self.critical { "it could change something" } else { "" }) } else { ("Don't run it", "") };
            g.row(r, name, value, t.dim, i == self.list.focus);
        }
        // Ends the list's clipping, which left the hint and the status unseen.
        self.list.scrollbar(g, 2, rows_top, bottom);
        g.hint("enter: confirm · esc: don't run");
    }
}

/// Where a change's page puts things: its title's centre, the box of what
/// it does (and its lines), and the two answers' row.
struct ChangeLayout {
    title_y: f32,
    words: Rect,
    lines: Vec<String>,
    answers: Rect,
}

impl Confirm {
    fn change_layout(&self, g: &mut Gfx) -> ChangeLayout {
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let title_y = top + 1.0 + ty.name.size * 0.5;
        let y = top + 1.0 + ty.name.size + 6.0;
        let lines = crate::help::wrap(g, &units_together(&self.command), chat_style(g), W - 2.0 * PAD - 16.0);
        let words = Rect::new(PAD, y, W - 2.0 * PAD, lines.len() as f32 * chat_line(g) + 8.0);
        // The answers side by side, resting on the hint with room for the ring.
        let answers = Rect::new(PAD, bottom - row_h - 3.0, W - 2.0 * PAD, row_h);
        ChangeLayout { title_y, words, lines, answers }
    }

    /// A change asked about: what, what it does (whole), and the two answers
    /// side by side, on "no" unless the other is chosen.
    fn draw_change(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let at = self.change_layout(g);
        g.icon("front_hand", true, PAD + 8.0, at.title_y, 16.0, t.signal);
        let title = g.text.fit(&self.name, &ty.name, W - 2.0 * PAD - 24.0);
        g.text_at(PAD + 22.0, at.title_y, Align::Left, &title, ty.name, t.ink);
        g.surface(at.words, 8.0, t.surface2);
        let (chat, lh) = (chat_style(g), chat_line(g));
        for (k, line) in at.lines.iter().enumerate() {
            g.text_at(PAD + 8.0, at.words.y + 4.0 + (k as f32 + 0.5) * lh, Align::Left, line, chat, t.ink);
        }
        let half = (at.answers.w - 6.0) * 0.5;
        for i in 0..2 {
            let r = Rect::new(at.answers.x + i as f32 * (half + 6.0), at.answers.y, half, at.answers.h);
            let name = if i == 0 { self.yes.clone() } else { self.no.clone() };
            g.row(r, &name, "", t.dim, i == self.list.focus);
        }
        g.take_foot(at.answers.y);
        let no = format!("enter: confirm · esc: {}", self.no.to_lowercase());
        g.hint_any(&[&no, "enter: confirm · esc: no", "enter: confirm"]);
    }
}

// --------------------------------------------------------------- choices ---

/// Which service answers, how hard it thinks, the conversations kept, and a
/// fresh one.
pub struct Choices {
    prefs: Prefs,
    keys: Keys,
    list: ListState,
    /// Whether answers are said aloud, and by whom (`crate::voice`).
    voice: crate::voice::Prefs,
    /// What it remembers, and the briefings that come by themselves (the
    /// morning's time, and one before each match).
    memory_file: PathBuf,
    remembered: usize,
    briefings: (Option<String>, bool),
}

const CHOICE_ROWS: usize = 10;

impl Choices {
    fn new() -> Self {
        let memory_file = memory::path();
        Choices {
            prefs: Prefs::load(),
            keys: Keys::load(),
            list: ListState::default(),
            voice: crate::voice::Prefs::load(),
            remembered: memory::load(&memory_file).len(),
            memory_file,
            briefings: briefing::schedule(),
        }
    }

    /// The morning briefing's next time: off, then each of the times offered, then off.
    fn next_morning(now: Option<&str>) -> Option<&'static str> {
        match now.and_then(|t| briefing::MORNINGS.iter().position(|m| *m == t)) {
            None if now.is_none() => Some(briefing::MORNINGS[2]),
            None => None,
            Some(i) => briefing::MORNINGS.get(i + 1).copied(),
        }
    }
}

impl Screen for Choices {
    fn title(&self) -> &str {
        "Assistant"
    }

    fn resume(&mut self) {
        self.keys = Keys::load();
        self.remembered = memory::load(&self.memory_file).len();
        self.briefings = briefing::schedule();
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        // A conversation chosen in the list: on back to the assistant.
        if open_chat_waiting() {
            return Nav::Pop;
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match (key, self.list.focus) {
            (Key::Back | Key::Leave, _) => return Nav::Pop,
            (Key::Enter | Key::Right, 0) => {
                let can = available(&self.keys);
                let current = answering(&self.prefs, &self.keys).map(|(s, _)| s);
                match can.len() {
                    0 | 1 => *cx.say = Some("an openai or claude key in keys adds another".into()),
                    _ => {
                        let at = can.iter().position(|s| Some(*s) == current).unwrap_or(0);
                        self.prefs.service = Some(can[(at + 1) % can.len()]);
                        self.prefs.save();
                        // Away from the on-device model: its memory back now,
                        // not when it would have idled out.
                        if current == Some(Service::Local) {
                            crate::local::stop();
                        }
                        *cx.say = Some("a new conversation with it".into());
                    }
                }
            }
            (Key::Enter | Key::Right, 1) => {
                self.prefs.effort = self.prefs.effort.next();
                self.prefs.save();
            }
            (Key::Enter | Key::Right, 2) => return Nav::Push(Box::new(Conversations::new(chats_dir()))),
            (Key::Enter | Key::Right, 3) => {
                NEW_CHAT.store(true, Ordering::Relaxed);
                return Nav::Pop;
            }
            (Key::Enter | Key::Right, 4) => return Nav::Push(Box::new(crate::screens::SettingsApp::keys_page())),
            (Key::Enter | Key::Right, 5) => {
                self.voice.speak = self.voice.speak.next();
                self.voice.save();
            }
            (Key::Enter | Key::Right, 6) => {
                self.voice.voice = self.voice.voice.next();
                self.voice.save();
                // Heard once, so the choice is made by ear.
                if self.voice.speak != crate::voice::Speak::Off {
                    if let Err(why) = crate::voice::say("This is how answers will sound.", self.voice.voice) {
                        *cx.say = Some(why);
                    }
                }
            }
            (Key::Enter | Key::Right, 7) => return Nav::Push(Box::new(MemoryPage::new(self.memory_file.clone()))),
            // The briefings that come by themselves: reminders that open one,
            // so Reminders lists them, and they wake the handheld to be said.
            (Key::Enter | Key::Right, 8) => {
                let next = Self::next_morning(self.briefings.0.as_deref());
                briefing::set_morning(next, cx.robot);
                self.briefings = briefing::schedule();
                *cx.say = Some(match next {
                    Some(t) => format!("a briefing each morning at {t}"),
                    None => "no morning briefing".into(),
                });
            }
            (Key::Enter | Key::Right, 9) => {
                let on = !self.briefings.1;
                briefing::set_pit(on, cx.robot);
                self.briefings = briefing::schedule();
                *cx.say = Some(if on { "a pit briefing before each match".into() } else { "no pit briefings".into() });
            }
            (other, _) => crate::screens::list_key(&mut self.list, other, CHOICE_ROWS),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let answering = answering(&self.prefs, &self.keys).map(|(s, _)| s);
        let kept = list_chats(&chats_dir()).len();
        let keys = Service::ALL.iter().filter(|s| s.online() && self.keys.get(s.key()).is_some()).count();
        for (i, y) in self.list.layout(g, CHOICE_ROWS, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value, color) = match i {
                0 => ("Answers with", answering.map_or("no key yet".to_owned(), |s| s.model_name().to_owned()), t.ink),
                1 => ("Effort", if answering == Some(Service::Local) { "–".to_owned() } else { self.prefs.effort.name().to_owned() }, t.ink),
                2 => ("Conversations", if kept == 0 { "none yet".to_owned() } else { format!("{kept} kept ›") }, t.dim),
                3 => ("New conversation", String::new(), t.dim),
                4 => ("Keys", format!("{keys} of 2 set"), t.dim),
                5 => ("Spoken answers", self.voice.speak.words().to_owned(), t.ink),
                6 => ("Voice", self.voice.voice.words(), if self.voice.speak == crate::voice::Speak::Off { t.dim } else { t.ink }),
                7 => ("Memory", if self.remembered == 0 { "nothing yet ›".to_owned() } else { format!("{} kept ›", self.remembered) }, t.dim),
                8 => ("Morning briefing", self.briefings.0.clone().unwrap_or_else(|| "off".into()), t.ink),
                _ => ("Pit briefing", if self.briefings.1 { "before matches".to_owned() } else { "off".to_owned() }, t.ink),
            };
            g.row(r, name, &value, color, i == self.list.focus);
        }
        self.list.scrollbar(g, CHOICE_ROWS, top, bottom);
        // On this handheld: answers from the logs (a downloaded model
        // words them); a key adds the services. Longest first, whole.
        if self.list.focus == 0 && answering == Some(Service::Local) {
            let more = available(&self.keys).len() > 1;
            match (crate::local::ready(), more) {
                (_, true) => g.hint_any(&["offline, from the logs · enter: the next one", "enter: the next one"]),
                (true, false) => g.hint_any(&["offline, from the logs · a key adds gpt, claude", "offline, from the logs"]),
                (false, false) => g.hint_any(&["offline, from the logs · a model words them", "offline, from the logs"]),
            }
            return;
        }
        g.hint(match self.list.focus {
            0 => "enter: the next one · a new conversation",
            1 => "light answers sooner · medium thinks it through",
            2 => "enter: reopen one and carry on",
            3 => "enter: clear this one",
            4 => "openai and claude keys",
            5 => "hold alt anywhere to ask aloud",
            6 => self.voice.voice.about(),
            // Longest first, whole: the first that fits.
            7 => return g.hint_any(&["kept on this handheld · never keys or passwords", "kept on this handheld · never keys", "kept on this handheld"]),
            8 => return g.hint_any(&["enter: a time, or off · reminders brings it", "enter: a time, or off"]),
            _ => return g.hint_any(&["30 min before each match, 10 before queueing", "30 min before each match"]),
        });
    }
}

// --------------------------------------------------------- conversations ---

/// The conversations kept, newest first: Enter reopens one, d (twice)
/// forgets one.
pub struct Conversations {
    dir: PathBuf,
    kept: Vec<KeptSummary>,
    list: ListState,
    deleting: Option<usize>,
}

impl Conversations {
    fn new(dir: PathBuf) -> Self {
        Conversations { kept: list_chats(&dir), dir, list: ListState::default(), deleting: None }
    }
}

/// "just now", "12 min ago", "3 h ago", "yesterday", "4 days ago".
fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 90 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86_400 => "yesterday".into(),
        s => format!("{} days ago", s / 86_400),
    }
}

impl Screen for Conversations {
    fn title(&self) -> &str {
        // "Conversations" would not fit beside the robot's name.
        "Chats"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let at = self.list.focus;
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter | Key::Right => {
                if let Some(k) = self.kept.get(at) {
                    *OPEN_CHAT.lock().unwrap_or_else(|e| e.into_inner()) = Some(k.id.clone());
                    return Nav::Pop;
                }
            }
            Key::Char('d') if at < self.kept.len() => {
                if armed == Some(at) {
                    forget_chat(&self.dir, &self.kept[at].id);
                    self.kept = list_chats(&self.dir);
                    self.list.focus = self.list.focus.min(self.kept.len().saturating_sub(1));
                    *cx.say = Some("forgotten".into());
                } else {
                    *cx.say = Some("d again forgets it".into());
                    self.deleting = Some(at);
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.kept.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.kept.is_empty() {
            g.empty("forum", "no conversations kept yet");
            g.hint("each one is kept once it has an answer");
            return;
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let now = chrono::Utc::now().timestamp();
        let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
        for (i, y) in self.list.layout(g, self.kept.len(), top, row_h, 3.0, bottom) {
            let k = &self.kept[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let value = ago(now - k.updated);
            let value_w = g.measure(&value, style);
            let title = g.text.fit(&k.title, &g.ty.body, (r.w - value_w - 34.0).max(40.0));
            g.row(r, &title, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, self.kept.len(), top, bottom);
        // Which service it was with, for the one chosen: it carries on there.
        let with = self.kept.get(self.list.focus).map_or("", |k| k.service.label());
        g.hint(&format!("enter: carry on with {with} · d: forget"));
    }
}

// ---------------------------------------------------------------- memory ---

/// What the assistant remembers, a thing to a row, each whole however long,
/// under where it is kept and where it goes: `d` twice forgets one, Enter
/// opens the file itself to change, and the last row forgets all of it
/// (Enter twice).
pub struct MemoryPage {
    file: PathBuf,
    facts: Vec<String>,
    /// The company it goes to with each question, while OpenAI or Claude
    /// answers; `None` offline, where it never leaves the handheld.
    sent_to: Option<&'static str>,
    focus: usize,
    /// Down from the top, in pixels: it glides to keep the focused row whole.
    scroll: Animated,
    /// Asked once: this row is forgotten on a second press.
    deleting: Option<usize>,
}

impl MemoryPage {
    pub fn new(file: PathBuf) -> Self {
        MemoryPage { facts: memory::load(&file), file, sent_to: Self::sent_to(), focus: 0, scroll: Animated::new(0.0, role::settle()).with_rest(0.3), deleting: None }
    }

    fn sent_to() -> Option<&'static str> {
        answering(&Prefs::load(), &Keys::load()).map(|(s, _)| s).filter(|s| s.online()).map(Service::company)
    }

    /// Where it is kept, and whether anything of it leaves the handheld:
    /// a line each.
    fn where_kept(&self) -> [String; 2] {
        let goes = match self.sent_to {
            Some(company) => format!("sent to {} only with your questions", company.to_lowercase()),
            None => "offline: it never leaves the handheld".into(),
        };
        ["kept on this handheld, in memory.md".into(), goes]
    }

    /// Rows: each thing remembered, then forgetting everything.
    fn rows(&self) -> usize {
        self.facts.len() + 1
    }

    /// Where it is kept, as its lines at the top of the page, and their height.
    fn kept_lines(&self, g: &mut Gfx) -> (Vec<String>, f32) {
        let cap = g.ty.caption;
        let lines: Vec<String> = self.where_kept().iter().flat_map(|said| crate::help::wrap(g, said, cap, W - 2.0 * PAD - 22.0)).collect();
        let h = lines.len() as f32 * (cap.size + 4.0);
        (lines, h)
    }

    /// Where it is kept, drawn from `y` (faded by `fade`).
    fn draw_kept(&self, g: &mut Gfx, lines: &[String], y: f32, fade: &dyn Fn(f32) -> f32) {
        let t = g.tokens;
        let cap = g.ty.caption;
        let lh = cap.size + 4.0;
        let icon = if self.sent_to.is_some() { "cloud" } else { "lock" };
        g.icon_alpha(icon, false, PAD + 8.0, y + lh * 0.5, 13.0, t.dim, fade(y + lh * 0.5));
        for (k, line) in lines.iter().enumerate() {
            let cy = y + (k as f32 + 0.5) * lh;
            g.text_alpha(PAD + 20.0, cy, line, cap, t.dim, fade(cy));
        }
    }
}

impl Screen for MemoryPage {
    fn title(&self) -> &str {
        "Memory"
    }

    fn resume(&mut self) {
        // Back from the file, changed by hand perhaps.
        self.facts = memory::load(&self.file);
        self.sent_to = Self::sent_to();
        self.focus = self.focus.min(self.rows() - 1);
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let at = self.focus;
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Down if !self.facts.is_empty() => self.focus = (self.focus + 1).min(self.rows() - 1),
            Key::Home => self.focus = 0,
            Key::End if !self.facts.is_empty() => self.focus = self.rows() - 1,
            Key::Char('d') if at < self.facts.len() => {
                if armed == Some(at) {
                    if let Some(gone) = memory::forget_at(&self.file, at) {
                        *cx.say = Some(format!("forgotten: {}", short(&gone.to_lowercase(), 40)));
                    }
                    self.facts = memory::load(&self.file);
                    self.focus = self.focus.min(self.rows() - 1);
                } else {
                    *cx.say = Some("d again forgets it".into());
                    self.deleting = Some(at);
                }
            }
            Key::Enter if at == self.facts.len() && !self.facts.is_empty() => {
                if armed == Some(at) {
                    let n = memory::forget_all(&self.file);
                    self.facts.clear();
                    self.focus = 0;
                    *cx.say = Some(format!("forgot all {n}"));
                } else {
                    *cx.say = Some("enter again forgets all of it".into());
                    self.deleting = Some(at);
                }
            }
            // The file itself, to change by hand.
            Key::Enter | Key::Right if !self.facts.is_empty() => match crate::files::Editor::open(&self.file) {
                Ok(editor) => return Nav::Push(Box::new(editor)),
                Err(why) => *cx.say = Some(why),
            },
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, bottom) = (g.top(), g.above_hint());
        let (kept, kept_h) = self.kept_lines(g);
        if self.facts.is_empty() {
            // Where it would be kept, then that nothing is, in the room left.
            self.draw_kept(g, &kept, top, &|_| 1.0);
            let cy = (top + kept_h + bottom) * 0.5;
            g.icon("push_pin", false, W * 0.5, cy - 12.0, 24.0, t.faint);
            let lines = crate::help::wrap(g, "nothing remembered yet · tell the assistant “remember …”", ty.caption, W - 2.0 * PAD);
            for (k, words) in lines.iter().enumerate() {
                g.text_at(W * 0.5, cy + 12.0 + k as f32 * (ty.caption.size + 4.0), Align::Center, words, ty.caption, t.dim);
            }
            g.hint_any(&["it never keeps passwords or keys", "never passwords or keys"]);
            return;
        }
        let (chat, lh) = (chat_style(g), chat_line(g));
        let bezel = g.theme == crate::look::Theme::Bezel;
        let width = W - 2.0 * PAD;
        // Each row's place, from the top of the list (under where it is
        // kept): a thing whole, or the last row.
        let mut rows: Vec<(f32, f32, Vec<String>)> = Vec::with_capacity(self.rows());
        let mut y = kept_h + 6.0;
        for fact in &self.facts {
            let lines = crate::help::wrap(g, &units_together(fact), chat, width - 34.0);
            let h = lines.len() as f32 * lh + 10.0;
            rows.push((y, h, lines));
            y += h + 4.0;
        }
        rows.push((y, g.row_h(), Vec::new()));
        // The focused row whole in view, with room for its ring; the first
        // with where it is kept above it.
        let view = bottom - top - crate::gfx::RING_ROOM;
        let (fy, fh, _) = &rows[self.focus.min(rows.len() - 1)];
        let (fy, fh) = if self.focus == 0 { (0.0, fy + fh) } else { (*fy, *fh) };
        let mut target = self.scroll.target();
        if fy - target < 0.0 {
            target = fy;
        } else if fy + fh - target > view {
            target = fy + fh - view;
        }
        if target != self.scroll.target() {
            self.scroll.set(target);
            if g.calm {
                self.scroll.jump(target);
            }
        }
        let scroll = self.scroll.value();
        g.canvas.set_clip(Rect::new(0.0, top - 3.0, W, bottom - top + 3.0 + crate::gfx::FOOT_AIR));
        let fade = |cy: f32| ((bottom - cy) / EDGE).clamp(0.0, 1.0).min(((cy - top + 6.0) / EDGE).clamp(0.0, 1.0));
        self.draw_kept(g, &kept, top - scroll, &fade);
        let mut ring = None;
        for (i, (ry, h, lines)) in rows.iter().enumerate() {
            let r = Rect::new(PAD, top + ry - scroll, width, *h);
            if r.bottom() < top - 4.0 || r.y > bottom + 4.0 {
                continue;
            }
            let focused = i == self.focus;
            let armed = self.deleting == Some(i);
            let radius = if bezel { g.radius(7.0) } else { 12.0_f32.min(r.h * 0.5) };
            let fill = if focused { t.surface2 } else { t.surface1 };
            g.canvas.rrect(r.offset(g.ox, 0.0), radius, fill, fade(r.y + 8.0).min(fade(r.bottom() - 8.0)).max(if focused { 1.0 } else { 0.0 }));
            if i < self.facts.len() {
                // A thing remembered: its pin, or, asked once to forget it,
                // the bin in the fault's colour.
                let first = r.y + 5.0 + lh * 0.5;
                let (icon, color) = if armed { ("delete", g.fault()) } else if focused { ("push_pin", t.signal) } else { ("push_pin", t.dim) };
                g.icon_alpha(icon, focused && !armed, r.x + 13.0, first, 12.0, color, fade(first));
                for (k, line) in lines.iter().enumerate() {
                    let cy = first + k as f32 * lh;
                    g.text_alpha(r.x + 26.0, cy, line, chat, t.ink, fade(cy));
                }
            } else {
                // Forgetting everything: a row of its own, at the end.
                let (name, value) = if armed { ("Enter again to forget", String::new()) } else { ("Forget everything", count_words(self.facts.len(), "thing")) };
                let style = if bezel { ty.label } else { ty.body };
                let name = if bezel { name.to_lowercase() } else { name.to_owned() };
                g.icon_alpha("delete", false, r.x + 13.0, r.cy(), 13.0, g.fault(), fade(r.cy()));
                g.text_alpha(r.x + 26.0, r.cy(), &name, style, t.ink, fade(r.cy()));
                let vw = g.measure(&value, ty.caption);
                g.text_alpha(r.right() - 10.0 - vw, r.cy(), &value, ty.caption, t.dim, fade(r.cy()));
            }
            if focused {
                ring = Some((r, radius));
            }
        }
        g.canvas.clear_clip();
        if let Some((r, radius)) = ring {
            g.focus_ring(r, radius);
        }
        let on_fact = self.focus < self.facts.len();
        match (self.deleting, on_fact) {
            (Some(_), true) => g.hint("d again forgets it"),
            (_, true) => g.hint_any(&["d: forget · enter: edit memory.md", "d: forget · enter: edit"]),
            (_, false) => g.hint_any(&["enter twice forgets all of it", "enter twice: all of it"]),
        }
    }
}

// --------------------------------------------------------------- sources ---

/// The pages an answer rests on: Enter opens one in the Web app (lynx).
pub struct SourcesPage {
    sources: Vec<Source>,
    list: ListState,
}

impl SourcesPage {
    pub(crate) fn new(sources: Vec<Source>) -> Self {
        SourcesPage { sources, list: ListState::default() }
    }
}

/// A URL as one word for the shell: in single quotes, its own quotes
/// escaped, so `&` and `?` reach lynx as they are.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The Web app (lynx in a terminal) on a page.
pub(crate) fn open_in_web(url: &str) -> Nav {
    let command = format!("lynx -accept_all_cookies -nopause {}", shell_word(url));
    Nav::Push(Box::new(crate::terminal::Terminal::running("Web", &command, crate::connect::session_dir())))
}

impl Screen for SourcesPage {
    fn title(&self) -> &str {
        "Sources"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave => Nav::Pop,
            Key::Enter | Key::Right => self.sources.get(self.list.focus).map_or(Nav::Stay, |s| open_in_web(&s.url)),
            other => {
                crate::screens::list_key(&mut self.list, other, self.sources.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        // The title has the row; where it is from is said below, for the
        // one chosen.
        for (i, y) in self.list.layout(g, self.sources.len(), top, row_h, 3.0, bottom) {
            let s = &self.sources[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let title = g.text.fit(&s.title, &g.ty.body, r.w - 24.0);
            g.row(r, &title, "", t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, self.sources.len(), top, bottom);
        let host = self.sources.get(self.list.focus).map(|s| host_of(&s.url)).unwrap_or_default();
        let host = g.text.fit(&host, &g.ty.caption, W * 0.55);
        g.hint(&format!("enter: open {host} in web"));
    }
}

// ----------------------------------------------------------- kept chats ---

/// Where conversations are kept: /data/assistant/chats on the handheld.
fn chats_dir() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/assistant/chats")
    } else {
        crate::config_dir().join("assistant-chats")
    }
}

/// A kept conversation, as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptSummary {
    pub id: String,
    pub title: String,
    pub service: Service,
    /// When it last had an answer (seconds since 1970).
    pub updated: i64,
}

/// A kept conversation, whole.
struct Kept {
    service: Service,
    items: Vec<Value>,
    entries: Vec<Entry>,
}

fn entry_json(entry: &Entry) -> Value {
    match entry {
        Entry::You(text) => json!({"you": text}),
        Entry::Said(text) => json!({"said": text}),
        Entry::Step { words, state } => json!({"step": words, "ok": *state != StepState::Failed}),
        Entry::Asks { name, command, reason, state } => json!({"asks": name, "command": command, "reason": reason, "answer": match state { AskState::Yes => "yes", AskState::No => "no", AskState::Waiting => "no" }}),
        Entry::Note(text) => json!({"note": text}),
        Entry::Sources(list) => json!({"sources": list.iter().map(|s| json!({"title": s.title, "url": s.url})).collect::<Vec<_>>()}),
        Entry::Report { label, title, rows } => json!({"report": label, "title": title, "rows": rows.iter().map(|(level, row)| json!([level_word(*level), row])).collect::<Vec<_>>()}),
        Entry::Basis { words, checked } => json!({"basis": words, "checked": checked}),
        Entry::Did { icon, words, detail, state } => json!({"did": words, "detail": detail, "icon": icon, "ok": *state != StepState::Failed}),
        Entry::Briefing { title, when, rows } => json!({"briefing": title, "when": when, "rows": rows.iter().map(|(icon, text, urgent)| json!([icon, text, urgent])).collect::<Vec<_>>()}),
    }
}

fn level_word(level: crate::wpilog::Level) -> &'static str {
    match level {
        crate::wpilog::Level::Fault => "fault",
        crate::wpilog::Level::Warn => "warn",
        crate::wpilog::Level::Fine => "fine",
    }
}

fn entry_from(v: &Value) -> Option<Entry> {
    let text = |k: &str| v[k].as_str().map(str::to_owned);
    if let Some(t) = text("you") {
        return Some(Entry::You(t));
    }
    if let Some(t) = text("said") {
        return Some(Entry::Said(t));
    }
    if let Some(words) = text("step") {
        return Some(Entry::Step { words, state: if v["ok"].as_bool() == Some(false) { StepState::Failed } else { StepState::Done } });
    }
    if let Some(name) = text("asks") {
        let state = if v["answer"] == "yes" { AskState::Yes } else { AskState::No };
        return Some(Entry::Asks { name, command: text("command").unwrap_or_default(), reason: text("reason").unwrap_or_default(), state });
    }
    if let Some(t) = text("note") {
        return Some(Entry::Note(t));
    }
    if let Some(label) = text("report") {
        let rows = v["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let level = match r[0].as_str()? {
                    "fault" => crate::wpilog::Level::Fault,
                    "warn" => crate::wpilog::Level::Warn,
                    _ => crate::wpilog::Level::Fine,
                };
                Some((level, r[1].as_str()?.to_owned()))
            })
            .collect();
        return Some(Entry::Report { label, title: text("title").unwrap_or_default(), rows });
    }
    if let Some(words) = text("basis") {
        return Some(Entry::Basis { words, checked: v["checked"].as_bool().unwrap_or(false) });
    }
    if let Some(words) = text("did") {
        let state = if v["ok"].as_bool() == Some(false) { StepState::Failed } else { StepState::Done };
        return Some(Entry::Did { icon: text("icon").unwrap_or_else(|| "check_circle".into()), words, detail: text("detail").unwrap_or_default(), state });
    }
    if let Some(title) = text("briefing") {
        let rows = v["rows"].as_array().into_iter().flatten().filter_map(|r| Some((r[0].as_str()?.to_owned(), r[1].as_str()?.to_owned(), r[2].as_bool().unwrap_or(false)))).collect();
        return Some(Entry::Briefing { title, when: text("when").unwrap_or_default(), rows });
    }
    let list = v["sources"].as_array()?;
    Some(Entry::Sources(list.iter().map(|s| Source { title: str_of(&s["title"]), url: str_of(&s["url"]) }).collect()))
}

/// Keep a conversation (replacing its earlier copy); the oldest beyond
/// `CHATS_KEPT` go.
fn save_chat(dir: &std::path::Path, id: &str, service: Service, title: &str, items: &[Value], entries: &[Entry]) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let kept = json!({
        "service": service.key(),
        "title": short(title, 80),
        "updated": chrono::Utc::now().timestamp(),
        "items": items,
        "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
    });
    let saved = crate::sys::write_private(&dir.join(format!("{id}.json")), kept.to_string().as_bytes()).is_ok();
    let all = list_chats(dir);
    for old in all.iter().skip(CHATS_KEPT) {
        forget_chat(dir, &old.id);
    }
    saved
}

/// The kept conversations, newest first.
pub fn list_chats(dir: &std::path::Path) -> Vec<KeptSummary> {
    let mut all: Vec<KeptSummary> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_string_lossy().into_owned();
            if path.extension()? != "json" {
                return None;
            }
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let service = Service::ALL.into_iter().find(|s| v["service"] == s.key())?;
            Some(KeptSummary { id, title: str_of(&v["title"]), service, updated: v["updated"].as_i64().unwrap_or(0) })
        })
        .collect();
    all.sort_by_key(|k| std::cmp::Reverse((k.updated, k.id.clone())));
    all
}

fn load_chat(dir: &std::path::Path, id: &str) -> Option<Kept> {
    if id.contains(['/', '\\', '.']) {
        return None;
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{id}.json"))).ok()?).ok()?;
    let service = Service::ALL.into_iter().find(|s| v["service"] == s.key())?;
    let items = v["items"].as_array()?.clone();
    let entries = v["entries"].as_array().into_iter().flatten().filter_map(entry_from).collect();
    Some(Kept { service, items, entries })
}

fn forget_chat(dir: &std::path::Path, id: &str) {
    if !id.contains(['/', '\\', '.']) {
        std::fs::remove_file(dir.join(format!("{id}.json"))).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx_parts() -> (Robot, crate::shell::Settings, Option<String>) {
        (Robot::demo(), crate::shell::Settings::default(), None)
    }

    #[test]
    fn choices_are_kept_and_the_service_with_a_key_answers() {
        crate::shell::tests::sandbox();
        assert_eq!(Prefs::parse("service=claude\neffort=light\n"), Prefs { service: Some(Service::Claude), effort: Effort::Light });
        assert_eq!(Prefs::parse("nonsense"), Prefs::default());
        assert_eq!(Prefs::default().effort, Effort::Medium, "medium unless chosen");
        let prefs = Prefs { service: Some(Service::Claude), effort: Effort::Light };
        assert!(prefs.save());
        assert_eq!(Prefs::load(), prefs);
        let mut keys = Keys::default();
        assert_eq!(answering(&prefs, &keys), Some((Service::Local, String::new())), "with no key, offline on this handheld");
        keys.set("openai", "sk-o");
        assert_eq!(answering(&prefs, &keys), Some((Service::OpenAi, "sk-o".into())), "the chosen one has no key: the one that does");
        keys.set("claude", "sk-ant-c");
        assert_eq!(answering(&prefs, &keys), Some((Service::Claude, "sk-ant-c".into())));
        assert_eq!(answering(&Prefs::default(), &keys).map(|a| a.0), Some(Service::OpenAi), "OpenAI first when nothing is chosen");
        std::fs::remove_file(prefs_path()).ok();
    }

    #[test]
    fn each_service_is_asked_in_its_own_shape() {
        let mut chat = Chat::new(Service::OpenAi);
        chat.ask("how is the robot?");
        let body = chat.body(Effort::Light);
        assert_eq!(body["model"], "gpt-6.2-sol");
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["store"], false, "nothing kept on OpenAI's side");
        assert_eq!(body["input"][0]["content"], "how is the robot?");
        assert_eq!(body["tools"][0]["type"], "function");
        let tools = body["tools"].as_array().unwrap();
        assert!(tools.iter().filter(|t| t["type"] == "function").all(|t| t["parameters"]["additionalProperties"] == false && t["strict"] == true));
        assert_eq!(tools.last(), Some(&json!({"type": "web_search"})), "the hosted web search, as OpenAI's guide gives it");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content", "web_search_call.action.sources"]));
        let mut chat = Chat::new(Service::Claude);
        chat.ask("how is the robot?");
        let body = chat.body(Effort::Medium);
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["fallbacks"], "default");
        assert!(body.get("thinking").is_none(), "Opus 5.5 thinks as it sees fit; nothing to set");
        assert!(body.get("temperature").is_none());
        assert_eq!(body["messages"][0]["role"], "user");
        let tools = body["tools"].as_array().unwrap();
        assert!(tools.iter().filter(|t| t.get("type").is_none()).all(|t| t["input_schema"]["type"] == "object"));
        // The server tools, as the docs give them: no code_execution beside them.
        let server: Vec<&Value> = tools.iter().filter(|t| t.get("type").is_some()).collect();
        assert_eq!(server[0], &json!({"type": "web_search_20260209", "name": "web_search", "max_uses": WEB_USES}));
        assert_eq!(server[1]["type"], "web_fetch_20260209");
        assert_eq!(server[1]["name"], "web_fetch");
        assert_eq!(server[1]["citations"], json!({"enabled": true}));
        assert!(!tools.iter().any(|t| t["name"] == "code_execution"));
        let names: Vec<&str> = tool_specs().iter().map(|(n, ..)| *n).collect();
        assert_eq!(
            names,
            [
                "robot_status", "robot_values", "robot_agent", "match_logs", "match_facts", "event_schedule", "graph_values", "read_file", "handheld_status", "list_actions", "run_action",
                // Those that act on the handheld, brief and remember.
                "reminder_add", "timer", "note_add", "music_play", "music_control", "music_download", "open_app", "setting_set", "next_match", "start_download", "briefing", "memory_save", "memory_forget",
            ]
        );
        // The on-device model: OpenAI-style chat completions, its own short
        // prompt first, no tools.
        let mut chat = Chat::new(Service::Local);
        chat.ask("what is a brownout?");
        let body = chat.body(Effort::Medium);
        assert_eq!(body["messages"][0], json!({"role": "system", "content": LOCAL_SYSTEM}));
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "what is a brownout?"}));
        assert_eq!(body["max_tokens"], LOCAL_MAX_TOKENS);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn claude_web_steps_and_sources_come_out_and_its_blocks_go_back_verbatim() {
        let mut chat = Chat::new(Service::Claude);
        chat.ask("how long is auto?");
        // The documentation's own response shape.
        let reply = json!({"content": [
            {"type": "text", "text": "I'll search."},
            {"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {"query": "frc 2026 auto length"}},
            {"type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
                {"type": "web_search_result", "url": "https://www.firstinspires.org/manual", "title": "Game Manual", "encrypted_content": "Eqgf…", "page_age": "January 10, 2026"},
                {"type": "web_search_result", "url": "https://frc-qa.firstinspires.org/", "title": "Q&A", "encrypted_content": "Eqgg…"}
            ]},
            {"type": "server_tool_use", "id": "srvtoolu_2", "name": "web_fetch", "input": {"url": "https://www.chiefdelphi.com/t/1"}},
            {"type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_2", "content": {"type": "web_fetch_tool_result_error", "error_code": "url_not_accessible"}},
            {"type": "server_tool_use", "id": "srvtoolu_3", "name": "web_fetch", "input": {"url": "https://docs.wpilib.org/a"}},
            {"type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_3", "content": {"type": "web_fetch_result", "url": "https://docs.wpilib.org/a", "content": {"type": "document", "source": {"type": "text", "media_type": "text/plain", "data": "…"}, "title": "WPILib A"}, "retrieved_at": "2025-08-25T10:30:00Z"}},
            {"type": "text", "text": "Autonomous lasts 20 seconds.", "citations": [{"type": "web_search_result_location", "url": "https://www.firstinspires.org/manual", "title": "Game Manual", "encrypted_index": "Eo8B…", "cited_text": "Autonomous…"}]}
        ], "stop_reason": "end_turn"});
        let r = chat.take(&reply).unwrap();
        assert_eq!(r.stop, Stop::Answered);
        assert_eq!(r.text, "I'll search.\nAutonomous lasts 20 seconds.");
        assert_eq!(
            r.steps,
            [
                WebStep { words: "searched the web: frc 2026 auto length".into(), ok: true },
                WebStep { words: "read chiefdelphi.com · url not accessible".into(), ok: false },
                WebStep { words: "read docs.wpilib.org".into(), ok: true },
            ]
        );
        // Cited first, then what it read; the merely found are left out.
        assert_eq!(r.sources, [Source { title: "Game Manual".into(), url: "https://www.firstinspires.org/manual".into() }, Source { title: "WPILib A".into(), url: "https://docs.wpilib.org/a".into() }]);
        // Every block goes back exactly as it came, encrypted content and all.
        assert_eq!(chat.items[1]["content"], reply["content"]);
        // A paused turn: kept, and to be carried on with no new question.
        let before = chat.len();
        let paused = chat.take(&json!({"content": [{"type": "server_tool_use", "id": "srvtoolu_9", "name": "web_search", "input": {"query": "more"}}], "stop_reason": "pause_turn"})).unwrap();
        assert_eq!(paused.stop, Stop::Paused);
        assert_eq!(chat.len(), before + 1);
        assert_eq!(chat.body(Effort::Medium)["messages"].as_array().unwrap().last().unwrap()["role"], "assistant", "sent back ending on the paused message itself");
        // A search that failed as a whole: its error object, not a list.
        let failed = chat.take(&json!({"content": [
            {"type": "server_tool_use", "id": "s", "name": "web_search", "input": {"query": "x"}},
            {"type": "web_search_tool_result", "tool_use_id": "s", "content": {"type": "web_search_tool_result_error", "error_code": "max_uses_exceeded"}},
            {"type": "text", "text": "Could not look."}
        ], "stop_reason": "end_turn"})).unwrap();
        assert_eq!(failed.steps, [WebStep { words: "searched the web: x · max uses exceeded".into(), ok: false }]);
        assert!(failed.sources.is_empty());
    }

    #[test]
    fn openai_web_searches_show_as_steps_and_their_citations_as_sources() {
        let mut chat = Chat::new(Service::OpenAi);
        chat.ask("latest news?");
        // The documentation's own output shape.
        let reply = json!({"status": "completed", "output": [
            {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc"},
            {"type": "web_search_call", "id": "ws_1", "status": "completed", "action": {"type": "search", "query": "latest frc news", "sources": [{"type": "url", "url": "https://a.example/x"}, {"type": "url", "url": "https://b.example/y"}]}},
            {"type": "web_search_call", "id": "ws_2", "status": "completed", "action": {"type": "open_page", "url": "https://b.example/y"}},
            {"type": "web_search_call", "id": "ws_3", "status": "completed", "action": {"type": "find_in_page", "url": "https://b.example/y", "pattern": "kickoff"}},
            {"id": "msg_1", "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Kickoff is in January.", "annotations": [{"type": "url_citation", "start_index": 0, "end_index": 7, "url": "https://b.example/y", "title": "B news"}]}]}
        ]});
        let r = chat.take(&reply).unwrap();
        assert_eq!(r.text, "Kickoff is in January.");
        assert_eq!(r.steps.iter().map(|s| s.words.as_str()).collect::<Vec<_>>(), ["searched the web: latest frc news", "read b.example", "looked in b.example for “kickoff”"]);
        assert_eq!(r.sources, [Source { title: "B news".into(), url: "https://b.example/y".into() }], "the cited page once, though it was also opened");
        assert_eq!(&chat.items[1..], reply["output"].as_array().unwrap().as_slice(), "every item goes back as it came");
        // With nothing cited or opened, the first few found stand in.
        let mut chat = Chat::new(Service::OpenAi);
        chat.ask("q");
        let r = chat.take(&json!({"status": "completed", "output": [
            {"type": "web_search_call", "id": "ws", "status": "completed", "action": {"type": "search", "queries": ["one"], "sources": [{"type": "url", "url": "https://c.example/"}]}},
            {"type": "message", "content": [{"type": "output_text", "text": "An answer.", "annotations": []}]}
        ]})).unwrap();
        assert_eq!(r.steps[0].words, "searched the web: one");
        assert_eq!(r.sources, [Source { title: "c.example".into(), url: "https://c.example/".into() }]);
    }

    #[test]
    fn the_on_device_model_answers_in_chat_completions_shape() {
        let mut chat = Chat::new(Service::Local);
        chat.ask("hi");
        let r = chat.take(&json!({"choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": " Hello. "}}]})).unwrap();
        assert_eq!((r.text.as_str(), r.stop), ("Hello.", Stop::Answered));
        assert_eq!(chat.items[1], json!({"role": "assistant", "content": "Hello."}));
        assert_eq!(chat.take(&json!({"choices": [{"finish_reason": "length", "message": {"content": "a long"}}]})).unwrap().stop, Stop::Cut);
        assert!(chat.take(&json!({"choices": []})).is_err());
    }

    #[test]
    fn secrets_under_data_are_never_given_out() {
        crate::shell::tests::sandbox();
        let root = data_root();
        // A file of its own in the notes folder, which other tests write in
        // at the same time (so only it is taken away after).
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/ai-test-pit.txt"), "swap battery 3").unwrap();
        let read = read_file("/data/notes/ai-test-pit.txt").unwrap();
        assert!(read.contains("swap battery 3"), "{read}");
        assert!(read_file("/data/notes").unwrap().contains("ai-test-pit.txt"), "a folder lists its files");
        for secret_path in ["/data/catalyst/keys", "/data/wifi/wpa_supplicant.conf", "/data/ssh/id_ed25519", "/data/tailscale/state", "/data/x/api.token"] {
            assert!(secret(std::path::Path::new(secret_path)), "{secret_path}");
        }
        assert!(read_file("/data/../../etc/passwd").is_err(), "nothing above /data");
        std::fs::remove_file(root.join("notes/ai-test-pit.txt")).ok();
    }

    #[test]
    fn conversations_are_kept_newest_first_and_reopened_whole() {
        let dir = std::env::temp_dir().join(format!("catalyst-chats-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let items = vec![json!({"role": "user", "content": "q"}), json!({"role": "assistant", "content": [{"type": "text", "text": "a"}]})];
        let entries = vec![
            Entry::You("q".into()),
            Entry::Step { words: "searched the web: q".into(), state: StepState::Done },
            Entry::Said("a".into()),
            Entry::Sources(vec![Source { title: "T".into(), url: "https://t.example/".into() }]),
            Entry::Asks { name: "Deploy".into(), command: "./gradlew deploy".into(), reason: "asked".into(), state: AskState::No },
            Entry::Note("stopped".into()),
        ];
        assert!(save_chat(&dir, "100", Service::Claude, "q", &items, &entries));
        let kept = load_chat(&dir, "100").unwrap();
        assert_eq!((kept.service, kept.items.clone(), kept.entries.clone()), (Service::Claude, items.clone(), entries.clone()));
        // The oldest go beyond the most kept; the list is newest first.
        for i in 0..CHATS_KEPT + 2 {
            save_chat(&dir, &format!("{}", 200 + i), Service::OpenAi, &format!("chat {i}"), &items, &[]);
        }
        let all = list_chats(&dir);
        assert_eq!(all.len(), CHATS_KEPT);
        assert!(all.windows(2).all(|w| (w[0].updated, &w[0].id) >= (w[1].updated, &w[1].id)));
        assert!(load_chat(&dir, "../keys").is_none(), "only a kept conversation's own name");
        forget_chat(&dir, &all[0].id);
        assert_eq!(list_chats(&dir).len(), CHATS_KEPT - 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_web_question_shows_its_steps_and_sources_and_is_kept() {
        crate::shell::tests::sandbox();
        for service in [Service::OpenAi, Service::Claude] {
            let (mut robot, mut settings, mut say) = cx_parts();
            let mut page = page_for(service);
            page.chats = std::env::temp_dir().join(format!("catalyst-webchat-{}-{service:?}", std::process::id()));
            std::fs::remove_dir_all(&page.chats).ok();
            type_and_send(&mut page, "how long is autonomous this year?", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            let entries: Vec<Entry> = page.entries.iter().map(|(e, _)| e.clone()).collect();
            assert!(entries.iter().any(|e| matches!(e, Entry::Step { words, state: StepState::Done } if words.starts_with("searched the web"))), "{service:?}: {entries:?}");
            assert_eq!(entries.last(), Some(&Entry::Sources(vec![Source { title: DEMO_MANUAL_TITLE.into(), url: DEMO_MANUAL_URL.into() }])));
            // Enter with nothing typed opens them.
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            assert!(matches!(page.key(Key::Enter, &mut cx), Nav::Push(_)));
            // Kept, and reopened whole on a fresh page.
            let kept = list_chats(&page.chats);
            assert_eq!(kept.len(), 1);
            assert_eq!(kept[0].title, "how long is autonomous this year?");
            let mut again = page_for(service);
            again.chats = page.chats.clone();
            let mut keys = Keys::default();
            keys.set(service.key(), "sk-test");
            keys.save().unwrap();
            again.reopen(&kept[0].id).unwrap();
            assert_eq!(again.entries.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>(), entries);
            assert_eq!(again.chat.as_ref().unwrap().len(), page.chat.as_ref().unwrap().len());
            std::fs::remove_dir_all(&page.chats).ok();
        }
        Keys::default().save().ok();
    }

    #[test]
    fn a_paused_turn_is_carried_on_without_a_new_question() {
        crate::shell::tests::sandbox();
        let (mut robot, _, _) = cx_parts();
        let mut page = page_for(Service::Claude);
        page.input = "search for something".into();
        page.send(&robot);
        page.busy = Busy::Idle;
        let before = page.chat.as_ref().unwrap().len();
        page.took(Ok(json!({"content": [{"type": "server_tool_use", "id": "s1", "name": "web_search", "input": {"query": "x"}}], "stop_reason": "pause_turn"})), &robot);
        assert!(matches!(page.busy, Busy::Asking(_)), "asked again at once");
        assert_eq!(page.continuations, 1);
        let items = page.chat.as_ref().unwrap();
        assert_eq!(items.len(), before + 1, "the paused message kept; no new question");
        // Too many: it stops and says so.
        page.continuations = CONTINUATIONS;
        page.busy = Busy::Idle;
        page.took(Ok(json!({"content": [{"type": "server_tool_use", "id": "s2", "name": "web_search", "input": {"query": "y"}}], "stop_reason": "pause_turn"})), &robot);
        assert!(page.idle());
        assert!(matches!(page.entries.last(), Some((Entry::Note(n), _)) if n.contains("kept on searching")));
        let _ = &mut robot;
    }

    #[test]
    fn a_claude_answer_with_calls_is_kept_whole_and_results_go_back() {
        let mut chat = Chat::new(Service::Claude);
        chat.ask("ping it");
        let reply = json!({"content": [
            {"type": "thinking", "thinking": "", "signature": "sig"},
            {"type": "text", "text": "Checking."},
            {"type": "tool_use", "id": "toolu_1", "name": "run_action", "input": {"name": "Ping the robot", "reason": "link"}}
        ], "stop_reason": "tool_use"});
        let r = chat.take(&reply).unwrap();
        assert_eq!(r.stop, Stop::Tools);
        assert_eq!(r.text, "Checking.");
        assert_eq!(r.calls, [Call { id: "toolu_1".into(), name: "run_action".into(), input: json!({"name": "Ping the robot", "reason": "link"}) }]);
        assert_eq!(chat.items[1]["content"][0]["signature"], "sig", "thinking goes back unchanged");
        chat.answer(&[Outcome { id: "toolu_1".into(), text: "ok".into(), error: false }]);
        assert_eq!(chat.items[2]["content"][0], json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok", "is_error": false}));
        // A refusal leaves the conversation as it was; running out of room says so.
        let before = chat.len();
        let refused = chat.take(&json!({"content": [], "stop_reason": "refusal", "stop_details": {"type": "refusal", "category": null, "explanation": "not this"}})).unwrap();
        assert_eq!((refused.stop, chat.len()), (Stop::Refused("not this".into()), before));
        assert_eq!(chat.take(&json!({"content": [{"type": "text", "text": "a"}], "stop_reason": "max_tokens"})).unwrap().stop, Stop::Cut);
        assert!(chat.take(&json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})).is_err());
    }

    #[test]
    fn an_openai_answer_keeps_its_reasoning_and_results_go_back_by_call() {
        let mut chat = Chat::new(Service::OpenAi);
        chat.ask("status?");
        let reply = json!({"status": "completed", "output": [
            {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc"},
            {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "robot_values", "arguments": "{\"prefix\":\"Loop/\"}"}
        ]});
        let r = chat.take(&reply).unwrap();
        assert_eq!(r.stop, Stop::Tools);
        assert_eq!(r.calls[0].input["prefix"], "Loop/");
        assert_eq!(chat.items[1]["encrypted_content"], "enc", "reasoning items go back as they came");
        chat.answer(&[Outcome { id: "call_1".into(), text: "{}".into(), error: false }]);
        assert_eq!(chat.items[3], json!({"type": "function_call_output", "call_id": "call_1", "output": "{}"}));
        let said = chat.take(&json!({"status": "completed", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Fine."}]}]})).unwrap();
        assert_eq!((said.text.as_str(), said.stop), ("Fine.", Stop::Answered));
        let refused = chat.take(&json!({"status": "completed", "output": [{"type": "message", "content": [{"type": "refusal", "refusal": "no"}]}]})).unwrap();
        assert_eq!(refused.stop, Stop::Refused("no".into()));
        let cut = chat.take(&json!({"status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}, "output": [{"type": "reasoning", "id": "rs_2", "summary": []}]})).unwrap();
        assert_eq!(cut.stop, Stop::Cut);
    }

    #[test]
    fn answers_lose_their_markdown_for_the_small_screen() {
        assert_eq!(plain("## Status\n\n**Battery** is `12.4 V`.\n\n\n- intake hot\n* can busy\n"), "Status\n\nBattery is 12.4 V.\n\n· intake hot\n· can busy");
        assert_eq!(plain("#5805 is fine"), "#5805 is fine", "a team number is not a heading");
    }

    #[test]
    fn failures_are_said_in_a_few_words() {
        assert_eq!(http_failure(Service::Claude, 401, "{}"), "Anthropic refused the key · check it in Settings › Keys");
        assert_eq!(http_failure(Service::OpenAi, 429, "{}"), "too many requests, or the account is out of credit");
        assert_eq!(http_failure(Service::OpenAi, 400, r#"{"error":{"message":"Bad tool"}}"#), "Bad tool");
        assert_eq!(curl_failure(Some(6), ""), "no internet: the service's name did not resolve");
    }

    #[test]
    fn the_robot_is_described_from_what_it_publishes() {
        let robot = Robot::demo();
        let status = robot_status(&robot);
        assert_eq!(status["linked"], true);
        assert!(status["groups"].as_object().is_some_and(|g| !g.is_empty()));
        let loop_values = robot_values(&robot, "/Catalyst/Loop/");
        assert_eq!(loop_values["prefix"], "Loop/");
        assert!(loop_values["count"].as_u64().unwrap() > 0);
        assert!(robot_values(&robot, "")["groups"].is_object());
        assert_eq!(value_json(&nt4::Value::Double(1.234567)), json!(1.2346));
    }

    /// Drive the page until it has nothing under way (or `limit` frames; and
    /// past them while a thread is still at work, for up to half a minute:
    /// reading the logs takes longer in the handheld's build under emulation).
    fn run(page: &mut Assistant, robot: &mut Robot, settings: &mut crate::shell::Settings, say: &mut Option<String>, limit: usize) {
        let began = std::time::Instant::now();
        for frame in 0.. {
            page.tick(0.1);
            let mut cx = Cx { robot, settings, say, at_rest: true };
            page.update(&mut cx);
            let waiting = page.idle() || matches!(page.busy, Busy::Approving { .. });
            if waiting && (page.waited > DEMO_PAUSE + 0.2 || page.idle() && !page.entries.is_empty()) {
                break;
            }
            if frame >= limit && (waiting || began.elapsed() > Duration::from_secs(30)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn type_and_send(page: &mut Assistant, text: &str, robot: &mut Robot, settings: &mut crate::shell::Settings, say: &mut Option<String>) {
        let mut cx = Cx { robot, settings, say, at_rest: true };
        for c in text.chars() {
            page.key(Key::Char(c), &mut cx);
        }
        page.key(Key::Enter, &mut cx);
    }

    /// A page answered by `service`, with a ping and a deploy among its Actions.
    fn page_for(service: Service) -> Assistant {
        let mut page = Assistant::with(Prefs::default(), Some((service, "sk-test".into())));
        let saved = |name: &str, command: &str| Saved { name: name.into(), command: command.into(), ask: crate::commands::Ask::Auto };
        page.actions_given = Some(vec![saved("Ping the robot", "ping -c 4 {robot}"), saved("Deploy robot code", "cd /data/code/robot && ./gradlew deploy")]);
        // Each page keeps its conversations and memory apart from every
        // other test's, in a folder of this run's emptied when the run
        // begins: Windows gives a new process the id of an old one, and a
        // folder named by it alone held that run's conversations too (one
        // more than a test expected, now and then).
        static PAGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static FRESH: std::sync::Once = std::sync::Once::new();
        let run = std::env::temp_dir().join(format!("catalyst-pages-{}", std::process::id()));
        FRESH.call_once(|| {
            std::fs::remove_dir_all(&run).ok();
        });
        let n = PAGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        page.chats = run.join(format!("chats-{n}"));
        page.memory_file = run.join(format!("memory-{n}.md"));
        page.chat = Some(Chat::with_memory(service, Mode::Assistant, &[]));
        page
    }

    /// Until the page has a thread's tool under way (or `limit` frames).
    fn run_until_working(page: &mut Assistant, robot: &mut Robot, settings: &mut crate::shell::Settings, say: &mut Option<String>, limit: usize) -> bool {
        for _ in 0..limit {
            page.tick(0.1);
            page.update(&mut Cx { robot, settings, say, at_rest: true });
            if matches!(page.busy, Busy::Tool { .. }) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        false
    }

    fn entries(page: &Assistant) -> Vec<Entry> {
        page.entries.iter().map(|(e, _)| e.clone()).collect()
    }

    /// The Assistant's live activity now, if there is one (this thread's).
    fn activity(robot: &Robot) -> Option<crate::liveactivity::Activity> {
        let update = crate::sysupdate::Status::default();
        let src = crate::liveactivity::Sources { robot, update: &update, unix: chrono::Utc::now().timestamp() };
        // The newest (one done a moment ago may still show beside it).
        crate::liveactivity::gather(&src, None).into_iter().filter(|a| a.kind == crate::liveactivity::Kind::Assistant).last()
    }

    // Found in the simulator's own music catalogue: the handheld's build
    // searches YouTube Music itself.
    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn a_task_of_several_steps_is_done_in_one_turn_each_step_shown_as_it_is_done() {
        crate::shell::tests::sandbox();
        for service in [Service::Claude, Service::OpenAi] {
            let (mut robot, mut settings, mut say) = cx_parts();
            let mut page = page_for(service);
            type_and_send(&mut page, "download my Pit Mix playlist and remind me to swap the battery at queueing for Q42", &mut robot, &mut settings, &mut say);
            assert_eq!(activity(&robot).map(|a| a.detail), Some("thinking".into()), "{service:?}: a live activity from the start");
            // The first step under way on its thread: its row, and the island's words.
            assert!(run_until_working(&mut page, &mut robot, &mut settings, &mut say, 400), "{service:?}");
            assert!(matches!(page.entries.last(), Some((Entry::Did { words, state: StepState::Working, .. }, _)) if words == "finding Pit Mix"), "{:?}", entries(&page));
            assert_eq!(activity(&robot).map(|a| a.detail), Some("step 1 · finding the music".into()));
            assert_eq!(page.batch, (0, 2), "two calls in the one turn");
            run(&mut page, &mut robot, &mut settings, &mut say, 1500);
            let all = entries(&page);
            assert_eq!(all[0], Entry::You("download my Pit Mix playlist and remind me to swap the battery at queueing for Q42".into()));
            // (The second service finds the songs asked for by the first still
            // queued, some of them saved by now perhaps.)
            assert!(matches!(&all[1], Entry::Did { words, detail, state: StepState::Done, icon } if words == "downloading Pit Crew Mix" && device::queued_words(detail) && icon == "downloading"), "{service:?}: {all:?}");
            assert_eq!(all[2], Entry::Did { icon: "notification_add".into(), words: "reminder set · swap the battery".into(), detail: "at queueing for Q42 · waits for the schedule".into(), state: StepState::Done });
            assert!(matches!(&all[3], Entry::Said(s) if s.contains("Pit Crew Mix") && s.contains("Reminder set: swap the battery, at queueing for Q42 (Q42 is not on the schedule yet")), "{all:?}");
            assert_eq!(all.len(), 4);
            assert!(page.idle());
            // The reminder, tied to Q42's queueing until the schedule has it.
            let kept = crate::reminders::with(|s| s.book.reminders.iter().find(|r| r.text == "swap the battery").map(|r| r.when.clone()));
            assert_eq!(kept, Some(crate::reminders::When::Match(crate::reminders::Cue::Queue, crate::reminders::Of::One { label: "Q42".into(), key: None })));
            // Both calls were made in one message and both answered together.
            let items = page.chat.as_ref().unwrap().items();
            match service {
                Service::Claude => {
                    let calls: Vec<&Value> = items[1]["content"].as_array().unwrap().iter().filter(|b| b["type"] == "tool_use").collect();
                    assert_eq!(calls.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["music_download", "reminder_add"]);
                    let results = items[2]["content"].as_array().unwrap();
                    assert_eq!(items[2]["role"], "user");
                    assert_eq!(results.len(), 2, "every result in a single user message");
                    assert_eq!((results[0]["tool_use_id"].clone(), results[1]["tool_use_id"].clone()), (calls[0]["id"].clone(), calls[1]["id"].clone()));
                    assert!(results.iter().all(|r| r["type"] == "tool_result" && r["is_error"] == false));
                }
                _ => {
                    let calls: Vec<&Value> = items.iter().filter(|i| i["type"] == "function_call").collect();
                    let outputs: Vec<&Value> = items.iter().filter(|i| i["type"] == "function_call_output").collect();
                    assert_eq!(calls.len(), 2);
                    assert_eq!(outputs.iter().map(|o| o["call_id"].clone()).collect::<Vec<_>>(), calls.iter().map(|c| c["call_id"].clone()).collect::<Vec<_>>());
                }
            }
            // The live activity says it is done, in so many steps.
            assert_eq!(activity(&robot).map(|a| (a.detail, a.state)), Some(("2 steps done".into(), "done".into())));
            crate::reminders::with(|s| s.book.reminders.retain(|r| r.text != "swap the battery"));
        }
    }

    #[test]
    fn a_change_that_matters_waits_for_a_yes_on_its_own_page() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        // Wi-Fi: asked about, and a no left it as it was.
        let mut page = page_for(Service::Claude);
        let ask = if crate::wifi::switched_on() { "turn wi-fi off" } else { "turn wi-fi on" };
        let was = crate::wifi::switched_on();
        type_and_send(&mut page, ask, &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(matches!(page.busy, Busy::Approving { pending: Pending::Device(_), .. }), "it waits: {:?}", entries(&page));
        assert!(matches!(page.entries.last(), Some((Entry::Asks { name, command, state: AskState::Waiting, .. }, _)) if name.starts_with("Switch Wi-Fi") && command.is_empty()));
        {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            assert!(matches!(page.key(Key::Enter, &mut cx), Nav::Push(_)), "Enter: the page that asks in full");
            page.key(Key::Back, &mut cx);
        }
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert_eq!(crate::wifi::switched_on(), was, "a no changes nothing");
        assert!(matches!(page.entries.last(), Some((Entry::Said(s), _)) if s == "Left it as it is."), "{:?}", entries(&page));
        // Forgetting everything: asked, and on a yes, done.
        let mut page = page_for(Service::OpenAi);
        memory::save(&page.memory_file, "The robot is called Numbers", "").unwrap();
        memory::save(&page.memory_file, "The team is 5805", "").unwrap();
        type_and_send(&mut page, "forget everything", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(matches!(page.busy, Busy::Approving { .. }));
        {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            let Busy::Approving { pending: Pending::Device(act), .. } = &page.busy else { panic!("a change") };
            let mut confirm = Confirm::change(act);
            assert_eq!(confirm.list.focus, 1, "on keeping them, unless chosen");
            assert_eq!((confirm.yes.as_str(), confirm.no.as_str()), ("Forget them", "Keep them"));
            confirm.key(Key::Left, &mut cx);
            assert!(matches!(confirm.key(Key::Enter, &mut cx), Nav::Pop));
            page.update(&mut cx);
        }
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(memory::load(&page.memory_file).is_empty());
        assert!(page.entries.iter().any(|(e, _)| matches!(e, Entry::Did { words, .. } if words == "forgot everything")));
        assert!(matches!(page.entries.last(), Some((Entry::Said(s), _)) if s == "Forgot all 2 remembered things."));
    }

    #[test]
    fn what_it_is_told_is_remembered_used_and_forgotten_and_a_secret_never_kept() {
        crate::shell::tests::sandbox();
        for service in [Service::Claude, Service::OpenAi] {
            let (mut robot, mut settings, mut say) = cx_parts();
            let mut page = page_for(service);
            type_and_send(&mut page, "remember our robot is called Numbers", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            assert!(entries(&page).contains(&Entry::Did { icon: "push_pin".into(), words: "remembered".into(), detail: "The robot is called Numbers".into(), state: StepState::Done }), "{service:?}: {:?}", entries(&page));
            assert_eq!(memory::load(&page.memory_file), ["The robot is called Numbers"]);
            // A secret: refused, said so, and not written.
            type_and_send(&mut page, "remember the wifi password is hunter22", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            assert!(page.entries.iter().any(|(e, _)| matches!(e, Entry::Did { state: StepState::Failed, detail, .. } if detail.contains("never kept"))));
            assert!(!std::fs::read_to_string(&page.memory_file).unwrap().contains("hunter22"));
            // A new conversation's prompt carries it, compactly.
            page.entries.clear();
            page.chat = Some(Chat::with_memory(service, Mode::Assistant, &memory::load(&page.memory_file)));
            let body = page.chat.as_ref().unwrap().body(Effort::Light);
            let system = body["system"].as_str().or_else(|| body["instructions"].as_str()).unwrap();
            assert!(system.ends_with("follow it, and use memory_save or memory_forget to change it):\n- The robot is called Numbers"), "{system}");
            type_and_send(&mut page, "what do you remember?", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            assert!(matches!(page.entries.last(), Some((Entry::Said(s), _)) if s.contains("· The robot is called Numbers")), "{:?}", entries(&page));
            // "forget that".
            type_and_send(&mut page, "forget that", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            assert!(entries(&page).contains(&Entry::Did { icon: "delete".into(), words: "forgotten".into(), detail: "The robot is called Numbers".into(), state: StepState::Done }));
            assert!(memory::load(&page.memory_file).is_empty());
        }
        // Offline: the same tools, by rules; the on-device model sees a tiny
        // version; forgetting everything still asks first.
        let (mut robot, mut settings, mut say) = cx_parts();
        let mut page = page_for(Service::Local);
        type_and_send(&mut page, "I like short answers", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert_eq!(entries(&page)[1..], [Entry::Did { icon: "push_pin".into(), words: "remembered".into(), detail: "Likes short answers".into(), state: StepState::Done }, Entry::Basis { words: "done on this handheld · no model needed".into(), checked: false }], "{:?}", entries(&page));
        assert_eq!(Chat::with_memory(Service::Local, Mode::Assistant, &memory::load(&page.memory_file)).body(Effort::Medium)["messages"][0]["content"], format!("{LOCAL_SYSTEM} Remember: Likes short answers."));
        type_and_send(&mut page, "what do you remember", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(page.entries.iter().any(|(e, _)| matches!(e, Entry::Said(s) if s == "I remember one thing:\n· Likes short answers")));
        type_and_send(&mut page, "forget everything", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(matches!(page.busy, Busy::Approving { pending: Pending::Device(_), .. }), "it asks first, offline too");
        {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            page.key(Key::Back, &mut cx);
        }
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert_eq!(memory::load(&page.memory_file), ["Likes short answers"], "a no keeps it");
        assert!(page.idle());
    }

    #[test]
    fn a_briefing_is_a_card_made_from_the_tools_online_and_from_the_template_offline() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        // Online: the briefing tool's card, then the model's few words.
        let mut page = page_for(Service::OpenAi);
        type_and_send(&mut page, "brief me", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 800);
        let all = entries(&page);
        let Entry::Briefing { title, rows, .. } = &all[1] else { panic!("a card: {all:?}") };
        assert_eq!(title, "Pit briefing · Q31");
        assert!(rows[0].1.starts_with("Q31 on red with 1507 5254 · queue in 8 min") && rows[0].2, "{rows:?}");
        assert!(rows.iter().any(|r| r.0.starts_with("battery")));
        assert!(matches!(&all[2], Entry::Said(s) if s.starts_with("Q31 is next, on red with 1507 and 5254.")), "{all:?}");
        // Offline: the same card, worded by the template, with no model.
        let mut page = page_for(Service::Local);
        type_and_send(&mut page, "brief me", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 800);
        let all = entries(&page);
        assert!(matches!(&all[1], Entry::Briefing { title, .. } if title == "Pit briefing · Q31"), "{all:?}");
        assert_eq!(all[2], Entry::Basis { words: "from the schedule, robot, forecast, battery and logs · no model".into(), checked: false });
        assert_eq!(page.chat.as_ref().unwrap().len(), 2, "kept in the on-device conversation, to carry on");
        // Taller than the room under the question: shown from its top, the
        // rest a scroll down.
        let mut g = Gfx::new(bezel::Tokens::dark());
        assert!(page.land.is_some());
        page.draw(&mut g, &robot);
        assert!(page.land.is_none() && page.scroll.target() > 0.0, "scrolled to the card's top: {}", page.scroll.target());
        // Online with no internet: made offline, saying so.
        let mut page = page_for(Service::Claude);
        page.input = "morning briefing".into();
        page.send(&robot);
        page.busy = Busy::Idle;
        page.took(Err("no internet: the service's name did not resolve".into()), &robot);
        assert!(matches!(page.busy, Busy::Briefing(_)));
        run(&mut page, &mut robot, &mut settings, &mut say, 800);
        assert!(entries(&page).iter().any(|e| matches!(e, Entry::Briefing { title, .. } if title.ends_with("riefing"))));
        assert_eq!(page.chat.as_ref().unwrap().len(), 0, "the service's conversation as it was");
    }

    #[test]
    fn offline_and_with_no_internet_requests_are_still_done_by_rules() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        // Offline: a timer and a reminder in one, each a row, then that no
        // model was needed; the live activity counts the steps.
        let mut page = page_for(Service::Local);
        type_and_send(&mut page, "start a 10 minute timer and remind me to check the bumpers at queueing for Q42", &mut robot, &mut settings, &mut say);
        assert_eq!(page.batch, (0, 2));
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let all = entries(&page);
        assert_eq!(all[1], Entry::Did { icon: "timer".into(), words: "timer started · 10:00".into(), detail: "clock counts it down".into(), state: StepState::Done }, "{all:?}");
        assert!(matches!(&all[2], Entry::Did { words, state: StepState::Done, .. } if words == "reminder set · check the bumpers"), "{all:?}");
        assert_eq!(all[3], Entry::Basis { words: "done on this handheld · no model needed".into(), checked: false });
        assert_eq!(all.len(), 4);
        assert_eq!(crate::clock::running_timers().len(), 1);
        assert_eq!(activity(&robot).map(|a| (a.detail, a.state)), Some(("2 steps done".into(), "done".into())));
        assert_eq!(page.chat.as_ref().unwrap().len(), 2, "kept in the on-device conversation");
        crate::reminders::with(|s| s.book.reminders.retain(|r| r.text != "check the bumpers"));
        // Stopping several timers asks first; a yes stops them.
        type_and_send(&mut page, "start a 5 min timer", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        type_and_send(&mut page, "stop the timers", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(matches!(page.entries.last(), Some((Entry::Asks { name, state: AskState::Waiting, .. }, _)) if name == "Stop 2 timers"));
        page.approve(true, &robot);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(crate::clock::running_timers().is_empty());
        assert!(entries(&page).iter().any(|e| matches!(e, Entry::Did { words, .. } if words == "2 timers stopped")));
        // Not a request: on to the logs, as before.
        assert!(device::rules("why did we brown out in Q14?").is_none());
        // With no internet, a service's request is done the same way, saying
        // so, and its conversation is left as it was.
        let mut page = page_for(Service::Claude);
        settings.battery.saver = false;
        page.input = "turn battery saver on".into();
        page.send(&robot);
        page.busy = Busy::Idle;
        page.took(Err("no internet: the service's name did not resolve".into()), &robot);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let all = entries(&page);
        assert_eq!(all[1], Entry::Note("no internet: the service's name did not resolve · done on this handheld".into()));
        assert!(matches!(&all[2], Entry::Did { words, .. } if words == "battery saver on"), "{all:?}");
        assert!(settings.battery.saver);
        assert_eq!(page.chat.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn a_long_answer_or_task_is_a_notification_once_it_is_the_answer() {
        let long = LONG_ANSWER + Duration::from_secs(1);
        let claude = |stop: &str| json!({"content": [{"type": "text", "text": "Done."}], "stop_reason": stop});
        let openai = |kind: &str| json!({"status": "completed", "output": [{"type": kind, "id": "x"}]});
        // Quick, or a step on the way (tools still to call): none.
        assert!(answered_notice(Service::Claude, &claude("end_turn"), Duration::from_secs(3), 0, "q").is_none());
        assert!(answered_notice(Service::Claude, &claude("tool_use"), long, 0, "q").is_none());
        assert!(answered_notice(Service::OpenAi, &openai("function_call"), long, 1, "q").is_none());
        // The answer, after a while: written, or the steps done.
        let n = answered_notice(Service::Claude, &claude("end_turn"), long, 0, "how is the robot?").unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str(), n.leads_to().as_deref()), ("your answer is written", "", Some("builtin:Assistant")));
        let n = answered_notice(Service::OpenAi, &openai("message"), long, 2, "turn light mode off and start a 3 min timer").unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str()), ("2 steps done", "turn light mode off and start a 3 min timer"));
        assert_eq!(n.tag.as_deref(), Some("assistant:answer"), "a newer one takes its place");
    }

    #[test]
    fn a_setting_changed_reaches_the_shell_and_an_app_opens_after_the_answer() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        let mut page = page_for(Service::Claude);
        settings.battery.saver = false;
        type_and_send(&mut page, "turn battery saver on", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(settings.battery.saver, "applied to the settings");
        assert!(entries(&page).contains(&Entry::Did { icon: "bolt".into(), words: "battery saver on".into(), detail: "the control centre switches it back".into(), state: StepState::Done }));
        type_and_send(&mut page, "open music", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(page.idle());
        assert!(matches!(page.entries.last(), Some((Entry::Said(s), _)) if s == "Music opens once you have answered."));
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        assert!(matches!(page.update(&mut cx), Nav::Stay), "not at once: the answer is read first");
        page.tick(OPEN_AFTER + 0.05);
        assert!(matches!(page.update(&mut cx), Nav::Push(_)), "then Music");
        assert!(page.open.is_none());
    }

    #[test]
    fn requests_carry_every_tool_strict_and_the_memory_in_a_stable_prompt() {
        let remembered = vec!["The robot is called Numbers".to_owned()];
        for service in [Service::OpenAi, Service::Claude] {
            let mut chat = Chat::with_memory(service, Mode::Assistant, &remembered);
            chat.ask("brief me");
            let first = chat.body(Effort::Medium);
            let tools = first["tools"].as_array().unwrap();
            let named: Vec<&Value> = tools.iter().filter(|t| t["type"] == "function" || t.get("type").is_none()).collect();
            assert_eq!(named.len(), tool_specs().len());
            for t in &named {
                let schema = if service == Service::OpenAi { &t["parameters"] } else { &t["input_schema"] };
                assert_eq!(t["strict"], true);
                assert_eq!(schema["additionalProperties"], false);
                assert_eq!(schema["required"].as_array().unwrap().len(), schema["properties"].as_object().unwrap().len(), "{}", t["name"]);
            }
            // A recorded reply with two calls: both taken, both answered.
            let reply = match service {
                Service::OpenAi => json!({"status": "completed", "output": [
                    {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc"},
                    {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "timer", "arguments": "{\"action\":\"start\",\"length\":\"5 min\"}"},
                    {"type": "function_call", "id": "fc_2", "call_id": "call_2", "name": "note_add", "arguments": "{\"title\":\"\",\"text\":\"chain\",\"append_to\":\"\"}"}
                ]}),
                _ => json!({"content": [
                    {"type": "thinking", "thinking": "", "signature": "sig"},
                    {"type": "tool_use", "id": "toolu_1", "name": "timer", "input": {"action": "start", "length": "5 min"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "note_add", "input": {"title": "", "text": "chain", "append_to": ""}}
                ], "stop_reason": "tool_use"}),
            };
            let r = chat.take(&reply).unwrap();
            assert_eq!((r.stop, r.calls.len()), (Stop::Tools, 2));
            chat.answer(&[Outcome { id: r.calls[0].id.clone(), text: "{}".into(), error: false }, Outcome { id: r.calls[1].id.clone(), text: "no".into(), error: true }]);
            let again = chat.body(Effort::Medium);
            // The prompt, tools and history the same as before: the cache holds.
            assert_eq!(again["tools"], first["tools"]);
            assert_eq!(again["system"], first["system"]);
            assert_eq!(again["instructions"], first["instructions"]);
            match service {
                Service::OpenAi => {
                    let input = again["input"].as_array().unwrap();
                    assert_eq!(input[input.len() - 2], json!({"type": "function_call_output", "call_id": "call_1", "output": "{}"}));
                    assert_eq!(input[input.len() - 1], json!({"type": "function_call_output", "call_id": "call_2", "output": "no"}));
                    assert!(again["instructions"].as_str().unwrap().contains("- The robot is called Numbers"));
                }
                _ => {
                    let last = again["messages"].as_array().unwrap().last().unwrap().clone();
                    assert_eq!(last["content"].as_array().unwrap().len(), 2, "both results in one message");
                    assert_eq!(last["content"][1], json!({"type": "tool_result", "tool_use_id": "toolu_2", "content": "no", "is_error": true}));
                    assert!(again["system"].as_str().unwrap().contains("- The robot is called Numbers"));
                }
            }
        }
    }

    #[test]
    fn a_change_and_its_rows_fit_whole_at_every_size() {
        let acts = [
            ("Switch Wi-Fi off", "The handheld leaves its network: no internet for the assistant and no robot over Wi-Fi, until it is back on.", "Switch it off", "Leave it on"),
            ("Switch Bluetooth on", "The radio comes on for headphones and speakers. The first time, the handheld restarts once.", "Switch it on", "Leave it off"),
            ("Download update 0.6.2", "About 120 MB. It installs beside this system and starts on the next restart; this one is kept to go back to.", "Download it", "Not now"),
            ("Forget everything", "All 24 things it remembers about you and the team are deleted from memory.md.", "Forget them", "Keep them"),
            ("Download 160 songs", "Random Access Memories (Deluxe Edition), about 1.8 GB, saved to the card over a while. Music › Downloads can cancel it.", "Download it", "Don't"),
        ];
        let mut g = Gfx::new(bezel::Tokens::dark());
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = theme;
            for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                for (title, what, yes, no) in acts {
                    let act = device::Act { title: title.into(), what: what.into(), yes: yes.into(), no: no.into(), kind: device::ActKind::ForgetAll };
                    let page = Confirm::change(&act);
                    let at = page.change_layout(&mut g);
                    assert!(at.words.bottom() + 4.0 <= at.answers.y, "{theme:?} {size:?} {title}: {} lines", at.lines.len());
                    assert!(g.measure(title, g.ty.name) <= W - 2.0 * PAD - 24.0, "{theme:?} {size:?}: {title}");
                    let half = (W - 2.0 * PAD - 6.0) * 0.5;
                    let style = if theme == crate::look::Theme::Bezel { g.ty.label } else { g.ty.body };
                    for answer in [yes, no] {
                        assert!(g.measure(&answer.to_lowercase(), style).max(g.measure(answer, style)) <= half - 20.0, "{theme:?} {size:?}: {answer}");
                    }
                }
            }
        }
    }

    #[test]
    fn on_this_handheld_it_answers_offline_from_the_logs_and_the_robot() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        let mut page = page_for(Service::Local);
        // Before anything is asked: the questions offered, the report first.
        {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            page.update(&mut cx);
        }
        assert_eq!(page.suggestions[0], Suggestion { name: "Match report".into(), value: "Q14".into(), question: None });
        assert_eq!(page.suggestions[1].question.as_deref(), Some("Why did we brown out in Q14?"), "Q14 browned out");
        // The robot just now, offline.
        type_and_send(&mut page, "how is the robot doing?", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let entries: Vec<Entry> = page.entries.iter().map(|(e, _)| e.clone()).collect();
        assert!(entries.contains(&Entry::Step { words: "looked at the robot".into(), state: StepState::Done }), "{entries:?}");
        assert!(matches!(&entries[2], Entry::Said(s) if s.contains("The robot is linked: battery")), "{entries:?}");
        assert_eq!(entries[3], Entry::Basis { words: "from the robot just now".into(), checked: false });
        // A match, from its log, and a follow-up about another.
        type_and_send(&mut page, "why did we brown out in Q14?", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        type_and_send(&mut page, "and Q12?", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let said: Vec<String> = page.entries.iter().filter_map(|(e, _)| if let Entry::Said(s) = e { Some(s.clone()) } else { None }).collect();
        assert!(said[1].starts_with("Q14: the robot browned out once: at 1:18 in teleop"), "{said:?}");
        assert!(said[2].starts_with("Q12: the robot did not brown out"), "{said:?}");
        assert!(page.entries.iter().any(|(e, _)| *e == Entry::Basis { words: "from the log of Q14".into(), checked: false }));
        // Ctrl R: the newest match's report, no model.
        {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            page.key(Key::Ctrl('r'), &mut cx);
        }
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let report = page.entries.iter().find_map(|(e, _)| if let Entry::Report { label, rows, .. } = e { Some((label.clone(), rows.clone())) } else { None }).expect("a report");
        assert_eq!(report.0, "Q14");
        assert_eq!(report.1[0].1, "The team won 101–95 on blue");
        assert!(report.1.iter().any(|(level, row)| *level == crate::wpilog::Level::Fault && row.starts_with("The robot browned out")));
        // Kept, and reopened whole, the report and what answers rest on too.
        let kept = list_chats(&page.chats);
        assert_eq!(kept.len(), 1);
        let mut again = page_for(Service::Local);
        again.chats = page.chats.clone();
        again.reopen(&kept[0].id).unwrap();
        assert_eq!(again.entries.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>(), page.entries.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>());
        // Not about the robot, and no model downloaded: what offline answers are.
        type_and_send(&mut page, "what is a brownout?", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        assert!(matches!(page.entries.iter().rev().nth(1), Some((Entry::Said(s), _)) if s.starts_with("Offline, I answer from the match logs")));
        std::fs::remove_dir_all(&page.chats).ok();
    }

    #[test]
    fn the_offline_page_fits_its_words_whole() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = theme;
            for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
                g.set_text_size(size);
                let ty = g.ty;
                let chat = chat_style(&g);
                // The heading and what it answers from, side by side: the
                // shortest way of saying it always fits; at the usual size,
                // the logs and the model.
                let room = W - 2.0 * PAD - 22.0 - g.measure("Working offline", ty.body) - 12.0;
                let from = offline_from(16, Some("SmolLM2 360M"));
                assert!(g.measure(from.last().unwrap(), ty.caption) <= room, "{theme:?} {size:?}");
                if size == crate::gfx::TextSize::Standard {
                    assert!(g.measure(&from[0], ty.caption) <= room, "{theme:?}: {}", from[0]);
                }
                // The questions offered, in their rows beside a match.
                let row_w = W - 2.0 * PAD - 5.0;
                for name in ["Why did we brown out in Q114?", "What changed since Q112?", "Compare Q114 and Q112", "How is the robot doing?"] {
                    let (name, style) = if theme == crate::look::Theme::Bezel { (name.to_lowercase(), ty.label) } else { (name.to_owned(), ty.body) };
                    assert!(g.measure(&name, style) <= row_w - 30.0, "{theme:?} {size:?}: {name}");
                }
                // The line typed into, while the model words an answer.
                let bar_w = W - 2.0 * PAD;
                assert!(g.measure("esc: the answer from the facts now", chat) <= bar_w - 24.0, "{size:?}");
            }
        }
    }

    #[test]
    fn with_no_internet_a_question_about_the_logs_is_answered_offline() {
        crate::shell::tests::sandbox();
        let robot = Robot::demo();
        let (mut r, mut settings, mut say) = cx_parts();
        let mut page = page_for(Service::OpenAi);
        page.input = "what went wrong last match?".into();
        page.send(&robot);
        page.busy = Busy::Idle;
        page.took(Err("no internet: the service's name did not resolve".into()), &robot);
        assert!(matches!(page.busy, Busy::Offline { .. }), "answering offline");
        run(&mut page, &mut r, &mut settings, &mut say, 400);
        let entries: Vec<Entry> = page.entries.iter().map(|(e, _)| e.clone()).collect();
        assert_eq!(entries[1], Entry::Note("no internet: the service's name did not resolve · answered offline, from the match logs".into()));
        assert!(entries.iter().any(|e| matches!(e, Entry::Said(s) if s.starts_with("Q14: the robot browned out"))), "{entries:?}");
        assert_eq!(page.chat.as_ref().unwrap().len(), 0, "the service's conversation as it was");
        // A question the logs cannot answer fails as before.
        page.input = "what's the weather?".into();
        page.send(&robot);
        page.busy = Busy::Idle;
        page.took(Err("no internet: the service's name did not resolve".into()), &robot);
        assert!(page.idle());
        assert_eq!(page.input, "what's the weather?", "back in the line to send again");
    }

    #[test]
    fn a_question_looks_at_the_robot_runs_an_action_and_is_answered_by_either_service() {
        crate::shell::tests::sandbox();
        for service in [Service::OpenAi, Service::Claude] {
            let (mut robot, mut settings, mut say) = cx_parts();
            let mut page = page_for(service);
            type_and_send(&mut page, "how is the robot doing?", &mut robot, &mut settings, &mut say);
            assert!(matches!(page.busy, Busy::Asking(_)));
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            let entries: Vec<Entry> = page.entries.iter().map(|(e, _)| e.clone()).collect();
            assert_eq!(entries[0], Entry::You("how is the robot doing?".into()));
            assert!(entries.contains(&Entry::Step { words: "looked at the robot".into(), state: StepState::Done }), "{service:?}: {entries:?}");
            assert!(entries.contains(&Entry::Step { words: "asked the robot's agent".into(), state: StepState::Done }), "{entries:?}");
            assert!(entries.contains(&Entry::Step { words: "ran ping the robot".into(), state: StepState::Done }), "pinging only reads: it runs without asking: {entries:?}");
            assert_eq!(entries.last(), Some(&Entry::Said(plain(DEMO_ANSWER))));
            assert!(page.idle());
        }
    }

    #[test]
    fn a_critical_action_waits_for_a_yes_and_a_no_goes_back_to_the_model() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        for yes in [false, true] {
            let mut page = page_for(Service::Claude);
            type_and_send(&mut page, "deploy the code", &mut robot, &mut settings, &mut say);
            run(&mut page, &mut robot, &mut settings, &mut say, 200);
            assert!(matches!(page.busy, Busy::Approving { .. }), "it waits");
            assert!(matches!(page.entries.last(), Some((Entry::Asks { state: AskState::Waiting, .. }, _))));
            {
                let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
                if yes {
                    // Enter asks for certain on a page of its own, which
                    // starts on not running it.
                    assert!(matches!(page.key(Key::Enter, &mut cx), Nav::Push(_)));
                    let mut confirm = Confirm::new("Deploy robot code", "cd /data/code/robot && ./gradlew deploy", "asked");
                    assert_eq!(confirm.list.focus, 1, "not run unless chosen");
                    confirm.key(Key::Up, &mut cx);
                    assert!(matches!(confirm.key(Key::Enter, &mut cx), Nav::Pop));
                    assert!(matches!(page.busy, Busy::Approving { .. }), "still waiting until it is on top again");
                    page.update(&mut cx);
                } else {
                    // Esc on the card skips it there and then.
                    page.key(Key::Back, &mut cx);
                }
            }
            run(&mut page, &mut robot, &mut settings, &mut say, 400);
            let last = page.entries.last().map(|(e, _)| e.clone());
            if yes {
                assert!(page.entries.iter().any(|(e, _)| *e == Entry::Step { words: "ran deploy robot code".into(), state: StepState::Done }));
                assert_eq!(last, Some(Entry::Said("Deployed: the build passed in 41 seconds and the robot program restarted.".into())));
            } else {
                assert!(page.entries.iter().all(|(e, _)| !matches!(e, Entry::Step { words, .. } if words.contains("deploy"))), "not run");
                assert!(matches!(last, Some(Entry::Said(s)) if s.starts_with("Left it")));
            }
        }
    }

    #[test]
    fn a_question_heard_shows_for_a_moment_then_sends_itself_unless_a_key_stops_it() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        assert_eq!(Assistant::with(Prefs::default(), None).dictation(), None, "no key: nothing to ask");
        let mut page = page_for(Service::Claude);
        assert_eq!(page.dictation(), Some(crate::voice::Dictation::Question));
        let step = |page: &mut Assistant, seconds: f32, robot: &mut Robot, settings: &mut crate::shell::Settings, say: &mut Option<String>| {
            for _ in 0..(seconds * 10.0).round() as usize {
                page.tick(0.1);
                page.update(&mut Cx { robot, settings, say, at_rest: true });
            }
        };
        page.dictated(" How is the robot doing? ", &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        assert_eq!(page.input, "How is the robot doing?");
        assert_eq!(say.take().as_deref(), Some("sending · any key edits"));
        step(&mut page, 1.2, &mut robot, &mut settings, &mut say);
        assert!(page.idle() && page.entries.is_empty(), "shown first, to read");
        step(&mut page, 0.5, &mut robot, &mut settings, &mut say);
        assert_eq!(page.entries[0].0, Entry::You("How is the robot doing?".into()), "then asked");
        assert!(page.aloud, "a question asked aloud: its answer may be said aloud");
        // A key stops it, and the words stay to edit.
        let mut page = page_for(Service::Claude);
        page.dictated("How is the robot doing?", &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        page.key(Key::Backspace, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        step(&mut page, 3.0, &mut robot, &mut settings, &mut say);
        assert_eq!((page.input.as_str(), page.entries.len()), ("How is the robot doing", 0));
        // Words typed already stay in front of what is said.
        let mut page = page_for(Service::Claude);
        page.input = "at Q31,".into();
        page.dictated("what was the battery?", &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        assert_eq!(page.input, "at Q31, what was the battery?");
        page.key(Key::Enter, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        assert_eq!(page.entries[0].0, Entry::You("at Q31, what was the battery?".into()), "enter sends it at once");
    }

    #[test]
    fn a_failed_question_comes_back_to_send_again() {
        crate::shell::tests::sandbox();
        let mut page = page_for(Service::OpenAi);
        page.input = "why so slow?".into();
        page.send(&Robot::demo());
        let marked = page.mark;
        page.failed("no internet: the service's name did not resolve".into());
        assert_eq!(page.input, "why so slow?");
        assert_eq!(page.chat.as_ref().unwrap().len(), marked, "the conversation is as it was");
        assert!(matches!(page.entries.last(), Some((Entry::Note(_), _))));
    }

    #[test]
    fn every_run_is_kept_on_disk_and_the_oldest_go() {
        let dir = std::env::temp_dir().join(format!("catalyst-runs-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let ok = Ok(json!({"exitCode": 0, "output": "4 packets received"}).to_string());
        let path = keep_run(&dir, "Ping the robot", "ping -c 4 10.58.5.2", "to check the link", "gpt-6.2 sol", false, &ok).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(path.file_name().unwrap().to_string_lossy().ends_with("-ping-the-robot.txt"));
        assert!(text.starts_with("Ping the robot\n"));
        assert!(text.contains("run by the assistant (gpt-6.2 sol)\nwhy: to check the link\n$ ping -c 4 10.58.5.2\nexit 0\n\n4 packets received"), "{text}");
        let deploy = keep_run(&dir, "Deploy robot code", "./gradlew deploy", "asked", "opus 5.5", true, &Err("the command could not start".into())).unwrap();
        assert!(std::fs::read_to_string(deploy).unwrap().contains("you said yes"));
        // Too many: the oldest names (by time) go first.
        for i in 0..RUNS_KEPT + 3 {
            std::fs::write(dir.join(format!("20000101-0000{i:03}-old.txt")), "old").unwrap();
        }
        keep_run(&dir, "Clock", "date", "", "opus 5.5", false, &ok).unwrap();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(left.len(), RUNS_KEPT);
        assert!(!dir.join("20000101-0000000-old.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Asks the real services with keys from the environment
    /// (OPENAI_API_KEY, ANTHROPIC_API_KEY); costs a little.
    #[test]
    #[ignore]
    fn live_services_answer() {
        std::env::set_var("CATALYST_AI_LIVE", "1");
        for (service, var) in [(Service::OpenAi, "OPENAI_API_KEY"), (Service::Claude, "ANTHROPIC_API_KEY")] {
            let Ok(key) = std::env::var(var) else { continue };
            let mut chat = Chat::new(service);
            chat.ask("Say hello in three words. Do not use any tools.");
            let v = request(service, &key, &chat.body(Effort::Light)).unwrap_or_else(|e| panic!("{service:?}: {e}"));
            let reply = chat.take(&v).unwrap();
            assert_eq!(reply.stop, Stop::Answered, "{v}");
            assert!(!reply.text.is_empty());
        }
    }
}
