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
//! The simulator answers from a script with the demo robot, unless told to
//! ask the real services (`CATALYST_AI_LIVE`).

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

Actions are the team's saved shell commands on this handheld. Run one when it answers the question or the person asks for it. Some are critical because they could change something; the handheld asks the person before those run, and they may decline. Then say what it would have done and leave it. Do not run a critical Action that the person did not ask for and the question does not need.";

/// For the on-device model: small, offline, with nothing to look at.
const LOCAL_SYSTEM: &str = "You are a small assistant running offline on a pocket Linux handheld that an FRC robotics team uses. Answer in two or three short, plain sentences. You cannot see the robot, the web or any files: if the question needs them, say so in one sentence.";

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

    /// The model as people know it.
    pub fn model_name(self) -> &'static str {
        match self {
            Service::OpenAi => "GPT-6.2 Sol",
            Service::Claude => "Claude Opus 5.5",
            Service::Local => crate::local::model_name(),
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
/// (its key set; the on-device model downloaded), else the first that can,
/// the two services before the on-device model.
pub fn answering(prefs: &Prefs, keys: &Keys) -> Option<(Service, String)> {
    let can = |s: Service| match s {
        Service::Local => crate::local::ready().then(|| (s, String::new())),
        _ => keys.get(s.key()).map(|k| (s, k.to_owned())),
    };
    prefs.service.and_then(can).or_else(|| Service::ALL.into_iter().find_map(can))
}

/// The services that can answer now.
pub fn available(keys: &Keys) -> Vec<Service> {
    Service::ALL.into_iter().filter(|s| if *s == Service::Local { crate::local::ready() } else { keys.get(s.key()).is_some() }).collect()
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

/// The simulator answers from a script unless told otherwise.
pub(crate) fn demo() -> bool {
    !cfg!(target_arch = "aarch64") && std::env::var_os("CATALYST_AI_LIVE").is_none()
}

// ----------------------------------------------------------------- tools ---

/// The tools, as (name, what it does, its input's JSON schema).
fn tool_specs() -> Vec<(&'static str, &'static str, Value)> {
    let nothing = || json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false});
    vec![
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
    ]
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
}

fn plan(call: &Call, robot: &Robot, actions: &[Saved]) -> Plan {
    match call.name.as_str() {
        "robot_status" => Plan::Now(Ok(robot_status(robot).to_string())),
        "robot_values" => Plan::Now(Ok(robot_values(robot, call.input["prefix"].as_str().unwrap_or_default()).to_string())),
        "robot_agent" => match host(robot) {
            Some(host) => Plan::Agent(host),
            None => Plan::Now(Err("no robot address yet: the team or robot is set in the Robot app".into())),
        },
        "match_logs" => Plan::Now(Ok(match_logs(robot).to_string())),
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
    pub fn new(service: Service) -> Self {
        Self::for_mode(service, Mode::Assistant)
    }

    pub fn for_mode(service: Service, mode: Mode) -> Self {
        Chat { service, mode, items: Vec::new() }
    }

    pub fn service(&self) -> Service {
        self.service
    }

    pub fn ask(&mut self, text: &str) {
        self.items.push(json!({"role": "user", "content": text}));
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
    fn with_items(service: Service, mode: Mode, items: Vec<Value>) -> Self {
        Chat { service, mode, items }
    }

    fn system(&self) -> &'static str {
        match (self.service, self.mode) {
            (Service::Local, _) => LOCAL_SYSTEM,
            (_, Mode::Browse) => BROWSE_SYSTEM,
            (_, Mode::Assistant) => SYSTEM,
        }
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
    let base = crate::local::start()?;
    if !(base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:")) {
        return Err("the on-device model must be on this handheld".into());
    }
    crate::local::touch();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let max_time = LOCAL_SECONDS.to_string();
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", &max_time, "--proto", "=http", "-H", "Content-Type: application/json", "--data-binary", "@-", "-w", "\n%{http_code}", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "curl could not start".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the question could not be sent".to_owned())?;
    }
    let output = child.wait_with_output().map_err(|_| "the on-device model stopped".to_owned())?;
    crate::local::touch();
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
    let deploy = lower.contains("deploy");
    let (text, calls): (&str, Vec<(&str, Value)>) = match (deploy, rounds) {
        (true, 0) => ("", vec![("run_action", json!({"name": "Deploy robot code", "reason": "you asked for the code to be deployed"}))]),
        (true, _) if last.contains("declined") => ("Left it: nothing was deployed. Say when the robot is on its cart and disabled, and I'll run it then.", vec![]),
        (true, _) => ("Deployed: the build passed in 41 seconds and the robot program restarted.", vec![]),
        (false, 0) => ("Looking at the robot first.", vec![("robot_status", json!({})), ("robot_agent", json!({}))]),
        (false, 1) => ("", vec![("run_action", json!({"name": "Ping the robot", "reason": "to check the link is steady"}))]),
        (false, _) => (DEMO_ANSWER, vec![]),
    };
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
    Tool { rx: Receiver<Result<String, String>>, id: String, row: usize, done: String },
    /// A critical Action shown, waiting for yes or no.
    Approving { id: String, saved: Saved, line: String, reason: String, row: usize },
}

/// Set by the choices page for a fresh conversation when the assistant is
/// back on top.
static NEW_CHAT: AtomicBool = AtomicBool::new(false);

/// The confirm page's answer, taken up when the assistant is on top again.
static DECISION: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

/// A kept conversation to reopen, chosen in Conversations; taken up when the
/// assistant is on top again.
static OPEN_CHAT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

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

    fn with(prefs: Prefs, service: Option<(Service, String)>) -> Self {
        Assistant {
            chat: service.as_ref().map(|(s, _)| Chat::new(*s)),
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
        }
    }

    fn idle(&self) -> bool {
        matches!(self.busy, Busy::Idle)
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
        self.chat = self.service.as_ref().map(|(s, _)| Chat::new(*s));
        self.entries.clear();
        self.calls.clear();
        self.outcomes.clear();
        self.busy = Busy::Idle;
        self.chat_id = None;
        self.countdown = None;
        self.revision += 1;
        self.scroll.jump(0.0);
    }

    /// A kept conversation, back as it was left, to carry on.
    fn reopen(&mut self, id: &str) -> Result<(), String> {
        let kept = load_chat(&self.chats, id).ok_or("that conversation is no longer kept")?;
        let keys = Keys::load();
        let key = match kept.service {
            Service::Local if crate::local::ready() => String::new(),
            Service::Local => return Err("it was with the on-device model, which is not downloaded".into()),
            s => keys.get(s.key()).ok_or_else(|| format!("it was with {}, whose key is gone", s.model_name()))?.to_owned(),
        };
        self.service = Some((kept.service, key));
        self.chat = Some(Chat::with_items(kept.service, Mode::Assistant, kept.items));
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

    fn send(&mut self) {
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
        self.actions = self.actions_given.clone().unwrap_or_else(crate::commands::all);
        self.push(Entry::You(text));
        self.ask();
    }

    /// The conversation sent for the next answer, on a thread.
    fn ask(&mut self) {
        let (Some(chat), Some((service, key))) = (&self.chat, &self.service) else { return };
        let body = chat.body(self.prefs.effort);
        let (service, key) = (*service, key.clone());
        let (tx, rx) = mpsc::channel();
        let started = std::thread::Builder::new().name("assistant".into()).stack_size(512 * 1024).spawn(move || {
            tx.send(request(service, &key, &body)).ok();
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
                    self.push(Entry::Note("it kept on searching · ask again to carry on".into()));
                    self.keep();
                } else {
                    self.ask();
                }
            }
            Stop::Tools => {
                self.calls = reply.calls.into();
                self.next_call(robot);
            }
            Stop::Refused(why) => {
                if let Some(chat) = &mut self.chat {
                    chat.rewind(self.mark);
                }
                self.push(Entry::Note(format!("{} would not answer: {why}", service.model_name())));
            }
            Stop::Cut => {
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
            match plan(&call, robot, &self.actions) {
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
                        tx.send(ask_agent(&host)).ok();
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
                    self.busy = Busy::Approving { id: call.id, saved, line, reason, row };
                    return;
                }
            }
        }
        let outcomes = std::mem::take(&mut self.outcomes);
        if let Some(chat) = &mut self.chat {
            chat.answer(&outcomes);
        }
        self.rounds += 1;
        if self.rounds >= ROUNDS {
            self.busy = Busy::Idle;
            self.push(Entry::Note(format!("stopped after {ROUNDS} rounds of looking · ask again to carry on")));
            self.keep();
            return;
        }
        self.ask();
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
            tx.send(result).ok();
        });
        self.busy = Busy::Tool { rx, id, row, done: format!("ran {lower}") };
    }

    /// The person's yes or no to a critical Action.
    fn approve(&mut self, yes: bool, robot: &Robot) {
        let Busy::Approving { id, saved, line, reason, row } = std::mem::replace(&mut self.busy, Busy::Idle) else { return };
        if let Some((Entry::Asks { state, .. }, _)) = self.entries.get_mut(row) {
            *state = if yes { AskState::Yes } else { AskState::No };
            self.revision += 1;
        }
        if yes {
            self.run(id, &saved, line, reason, true);
        } else {
            self.outcomes.push(Outcome { id, text: "The person declined: the Action was not run.".into(), error: true });
            self.next_call(robot);
        }
    }

    /// Stop what is under way: the answer, a tool, or a question waiting.
    fn stop(&mut self) {
        let running = matches!(self.busy, Busy::Tool { .. });
        self.busy = Busy::Idle;
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
                    Laid { lines, h }
                }
                Entry::Said(text) => {
                    let lines = wrap_paragraphs(g, text, chat, W - 2.0 * PAD - 8.0);
                    let h = lines.iter().map(|l| if l.is_empty() { body_lh * 0.5 } else { body_lh }).sum();
                    Laid { lines, h }
                }
                Entry::Step { .. } => Laid { lines: Vec::new(), h: cap_lh },
                Entry::Asks { reason, .. } => {
                    let mut lines = crate::help::wrap(g, reason, ty.caption, W - 2.0 * PAD - 24.0);
                    lines.truncate(3);
                    let h = 9.0 + ty.body.size + 5.0 + lines.len() as f32 * cap_lh + cap_lh + 3.0 + cap_lh + 7.0;
                    Laid { lines, h }
                }
                Entry::Note(text) => {
                    let lines = crate::help::wrap(g, text, ty.caption, W - 2.0 * PAD - 22.0);
                    let h = lines.len() as f32 * cap_lh + 2.0;
                    Laid { lines, h }
                }
                Entry::Sources(_) => Laid { lines: Vec::new(), h: cap_lh + 2.0 },
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
        g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 4.0 + ty.caption.size * 0.5, Align::Center, "it reads the robot, logs and actions", ty.caption, t.dim);
        let model = self.service.as_ref().map_or("", |(s, _)| s.label());
        let line = format!("{model} · {} · tab: choices", self.prefs.effort.name());
        g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 8.0 + ty.caption.size * 1.5, Align::Center, &line, ty.caption, t.faint);
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
            let words = if self.idle() || matches!(self.busy, Busy::Approving { .. }) { "type, or hold alt to talk" } else { "esc: stop" };
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
        "list_actions" => same("looked at the actions"),
        "event_schedule" => same("looked at the schedule"),
        "graph_values" => same("looked at the live graphs"),
        "handheld_status" => same("looked at this handheld"),
        "read_file" => same(&format!("opened {}", call.input["path"].as_str().unwrap_or("a file").trim())),
        "run_action" => {
            let name = call.input["name"].as_str().unwrap_or_default().to_lowercase();
            StepWords { working: format!("running {name}"), done: format!("ran {name}") }
        }
        other => same(&other.replace('_', " ")),
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

impl Screen for Assistant {
    fn title(&self) -> &str {
        "Assistant"
    }

    fn wants_text(&self) -> bool {
        self.service.is_some()
    }

    fn keeps_awake(&self) -> bool {
        !self.idle()
    }

    fn busy(&self) -> bool {
        !self.idle()
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
            *cx.say = Some("still answering · enter sends it after".into());
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
        let line = g_line_height();
        match key {
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
            Key::Leave => return Nav::Pop,
            // A critical Action: Enter opens the page that asks for certain;
            // Esc skips it here.
            _ if matches!(self.busy, Busy::Approving { .. }) => match key {
                Key::Enter => {
                    if let Busy::Approving { saved, line, reason, .. } = &self.busy {
                        *DECISION.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        return Nav::Push(Box::new(Confirm::new(&saved.name, line, reason)));
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
            Key::Enter => self.send(),
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
        // A question heard, left alone long enough: it goes.
        if self.countdown.is_some_and(|left| left <= 0.0) {
            self.countdown = None;
            self.send();
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
                    self.set_step(row, result.is_ok(), done);
                    let error = result.is_err();
                    self.outcomes.push(Outcome { id, text: result.unwrap_or_else(|why| why), error });
                    self.next_call(cx.robot);
                }
                Err(TryRecvError::Disconnected) => {
                    let Busy::Tool { id, row, done, .. } = std::mem::replace(&mut self.busy, Busy::Idle) else { return Nav::Stay };
                    self.set_step(row, false, done);
                    self.outcomes.push(Outcome { id, text: "it stopped without an answer".into(), error: true });
                    self.next_call(cx.robot);
                }
                Err(TryRecvError::Empty) => {}
            },
            Busy::Idle | Busy::Approving { .. } => {}
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
        moving || rising || counting || !self.idle()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
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
            self.draw_empty(g, area);
            self.draw_input(g, bar, true);
            return;
        }
        self.lay_out(g);
        let Some((_, _, laid)) = self.laid.clone() else { return };
        let total: f32 = laid.iter().map(|l| l.h + GAP).sum::<f32>() + if self.idle() || approving { 0.0 } else { ty.caption.size + 10.0 };
        let max = (total - area.h).max(0.0);
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
                _ => "working".into(),
            };
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
                    let title = g.text.fit(&format!("Run {name}?"), &ty.body, r.w - 40.0);
                    g.text_alpha(r.x + 29.0, ly, &title, ty.body, t.ink, fade(ly));
                    ly += ty.body.size * 0.5 + 5.0;
                    let cap_lh = ty.caption.size + 4.0;
                    for line in &l.lines {
                        g.text_alpha(r.x + 12.0, ly + cap_lh * 0.5, line, ty.caption, t.dim, fade(ly + cap_lh * 0.5));
                        ly += cap_lh;
                    }
                    let command = g.text.fit(command, &ty.caption, r.w - 24.0);
                    g.text_alpha(r.x + 12.0, ly + cap_lh * 0.5, &command, ty.caption, t.faint, fade(ly + cap_lh * 0.5));
                    ly += cap_lh + 3.0;
                    let (words, color) = match state {
                        AskState::Waiting => ("enter: confirm · esc: skip", t.ink),
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
}

impl Confirm {
    fn new(name: &str, command: &str, reason: &str) -> Self {
        let mut list = ListState::default();
        list.focus = 1;
        Confirm { name: name.to_owned(), command: command.to_owned(), reason: reason.to_owned(), list }
    }

    fn answer(&self, yes: bool) -> Nav {
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
        "Confirm"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave => return self.answer(false),
            Key::Enter => return self.answer(self.list.focus == 0),
            other => crate::screens::list_key(&mut self.list, other, 2),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let rows_top = bottom - 2.0 * row_h - 3.0;
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
            let (name, value) = if i == 0 { ("Run it", "it could change something") } else { ("Don't run it", "") };
            g.row(r, name, value, t.dim, i == self.list.focus);
        }
        g.hint("enter: confirm · esc: don't run");
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
}

const CHOICE_ROWS: usize = 7;

impl Choices {
    fn new() -> Self {
        Choices { prefs: Prefs::load(), keys: Keys::load(), list: ListState::default(), voice: crate::voice::Prefs::load() }
    }
}

impl Screen for Choices {
    fn title(&self) -> &str {
        "Assistant"
    }

    fn resume(&mut self) {
        self.keys = Keys::load();
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
                    0 => *cx.say = Some("add a key in keys, or download the on-device model in settings".into()),
                    1 => {
                        *cx.say = Some(if crate::local::ready() { "the other services need their keys".into() } else { "a key in keys, or the on-device model from settings, adds another".into() })
                    }
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
            (other, _) => crate::screens::list_key(&mut self.list, other, CHOICE_ROWS),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
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
                _ => ("Voice", self.voice.voice.words(), if self.voice.speak == crate::voice::Speak::Off { t.dim } else { t.ink }),
            };
            g.row(r, name, &value, color, i == self.list.focus);
        }
        self.list.scrollbar(g, CHOICE_ROWS, top, bottom);
        g.hint(match self.list.focus {
            0 if crate::local::ready() => "enter: the next one · a new conversation",
            0 => "on-device model: download it in settings",
            1 => "light answers sooner · medium thinks it through",
            2 => "enter: reopen one and carry on",
            3 => "enter: clear this one",
            4 => "openai and claude keys",
            5 => "hold alt anywhere to ask aloud",
            _ => self.voice.voice.about(),
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
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
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
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
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
        assert_eq!(answering(&prefs, &keys), None);
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
        assert_eq!(names, ["robot_status", "robot_values", "robot_agent", "match_logs", "event_schedule", "graph_values", "read_file", "handheld_status", "list_actions", "run_action"]);
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
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/pit.txt"), "swap battery 3").unwrap();
        let read = read_file("/data/notes/pit.txt").unwrap();
        assert!(read.contains("swap battery 3"), "{read}");
        assert!(read_file("/data/notes").unwrap().contains("pit.txt"), "a folder lists its files");
        for secret_path in ["/data/catalyst/keys", "/data/wifi/wpa_supplicant.conf", "/data/ssh/id_ed25519", "/data/tailscale/state", "/data/x/api.token"] {
            assert!(secret(std::path::Path::new(secret_path)), "{secret_path}");
        }
        assert!(read_file("/data/../../etc/passwd").is_err(), "nothing above /data");
        std::fs::remove_dir_all(root.join("notes")).ok();
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
        page.send();
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

    /// Drive the page until it has nothing under way (or `limit` frames).
    fn run(page: &mut Assistant, robot: &mut Robot, settings: &mut crate::shell::Settings, say: &mut Option<String>, limit: usize) {
        for _ in 0..limit {
            page.tick(0.1);
            let mut cx = Cx { robot, settings, say, at_rest: true };
            page.update(&mut cx);
            if page.idle() || matches!(page.busy, Busy::Approving { .. }) {
                if page.waited > DEMO_PAUSE + 0.2 || page.idle() && !page.entries.is_empty() {
                    break;
                }
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
        // Each page keeps its conversations apart from every other test's.
        static PAGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = PAGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        page.chats = std::env::temp_dir().join(format!("catalyst-page-chats-{}-{n}", std::process::id()));
        page
    }

    #[test]
    fn the_on_device_model_answers_plainly_with_no_tools() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = cx_parts();
        let mut page = page_for(Service::Local);
        type_and_send(&mut page, "how is the robot doing?", &mut robot, &mut settings, &mut say);
        run(&mut page, &mut robot, &mut settings, &mut say, 400);
        let entries: Vec<Entry> = page.entries.iter().map(|(e, _)| e.clone()).collect();
        assert_eq!(entries.len(), 2, "a question and its answer, nothing looked at: {entries:?}");
        assert!(matches!(&entries[1], Entry::Said(s) if s.contains("brownout")));
        assert!(page.idle());
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
        page.send();
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
