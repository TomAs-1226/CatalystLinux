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
//! Its tools: what the robot publishes (FRC Catalyst under `/Catalyst/`), the
//! Catalyst Agent's report from the Systemcore, the match logs pulled, and the
//! saved Actions. Looking is free. An Action that could change something
//! (`commands::Saved::critical`) waits for the person to say yes, and a no
//! goes back to the model as a no.
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
const DEMO_PAUSE: f32 = 0.9;

const SYSTEM: &str = "You are the assistant on Catalyst Linux, a pocket Linux handheld (an M5Stack CardputerZero) that an FRC robotics team carries in the pit and the stands.

Its screen is 320 by 170 pixels. Answer in plain sentences and keep it short: usually two to five sentences, at most about 80 words unless asked for more. No Markdown: no headings, bullets, bold, tables or code blocks. If a list helps, put each item on its own short line.

The team's robot runs FRC Catalyst, which publishes its telemetry under /Catalyst/ in NetworkTables, and the Catalyst Agent on the robot's Systemcore reports the robot program, its recent log, CPU and memory. Look with the tools before you answer anything about the robot, and never guess a value: if something is not published, say so. Say plainly when the robot is not linked.

Actions are the team's saved shell commands on this handheld. Run one when it answers the question or the person asks for it. Some are critical because they could change something; the handheld asks the person before those run, and they may decline. Then say what it would have done and leave it. Do not run a critical Action that the person did not ask for and the question does not need.";

// -------------------------------------------------------------- services ---

/// Who answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    OpenAi,
    Claude,
}

impl Service {
    pub const ALL: [Service; 2] = [Service::OpenAi, Service::Claude];

    /// Its key's name in the keys file.
    pub fn key(self) -> &'static str {
        match self {
            Service::OpenAi => "openai",
            Service::Claude => "claude",
        }
    }

    /// The model as people know it.
    pub fn model_name(self) -> &'static str {
        match self {
            Service::OpenAi => "GPT-6.2 Sol",
            Service::Claude => "Claude Opus 5.5",
        }
    }

    /// The same, lower case, for mono labels.
    pub fn label(self) -> &'static str {
        match self {
            Service::OpenAi => "gpt-6.2 sol",
            Service::Claude => "opus 5.5",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Service::OpenAi => "gpt-6.2-sol",
            Service::Claude => "claude-opus-5-5",
        }
    }

    fn company(self) -> &'static str {
        match self {
            Service::OpenAi => "OpenAI",
            Service::Claude => "Anthropic",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Service::OpenAi => OPENAI_URL,
            Service::Claude => CLAUDE_URL,
        }
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

/// The service that answers, and its key: the one chosen if its key is set,
/// else the first that has one.
pub fn answering(prefs: &Prefs, keys: &Keys) -> Option<(Service, String)> {
    let with_key = |s: Service| keys.get(s.key()).map(|k| (s, k.to_owned()));
    prefs.service.and_then(with_key).or_else(|| Service::ALL.into_iter().find_map(with_key))
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
fn demo() -> bool {
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
    /// It would not answer, and why.
    Refused(String),
    /// It ran out of room.
    Cut,
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<Call>,
    pub stop: Stop,
}

/// The conversation as the service keeps it: its own items, sent back whole
/// each turn and never edited, so reasoning carried in them stays valid.
pub struct Chat {
    service: Service,
    items: Vec<Value>,
}

impl Chat {
    pub fn new(service: Service) -> Self {
        Chat { service, items: Vec::new() }
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

    /// The request for the next answer.
    pub fn body(&self, effort: Effort) -> Value {
        let specs = tool_specs();
        match self.service {
            Service::OpenAi => json!({
                "model": self.service.model(),
                "instructions": SYSTEM,
                "input": self.items,
                "tools": specs.iter().map(|(name, about, schema)| json!({"type": "function", "name": name, "description": about, "parameters": schema, "strict": true})).collect::<Vec<_>>(),
                "reasoning": {"effort": effort.wire()},
                "store": false,
                "include": ["reasoning.encrypted_content"],
                "max_output_tokens": OPENAI_MAX_OUTPUT,
            }),
            Service::Claude => json!({
                "model": self.service.model(),
                "max_tokens": CLAUDE_MAX_TOKENS,
                "system": SYSTEM,
                "tools": specs.iter().map(|(name, about, schema)| json!({"name": name, "description": about, "input_schema": schema, "strict": true})).collect::<Vec<_>>(),
                "output_config": {"effort": effort.wire()},
                "fallbacks": "default",
                "cache_control": {"type": "ephemeral"},
                "messages": self.items,
            }),
        }
    }

    /// An answer taken into the conversation: its words and the calls it
    /// makes. A refusal leaves the conversation as it was.
    pub fn take(&mut self, v: &Value) -> Result<Reply, String> {
        if let Some(message) = v["error"]["message"].as_str() {
            return Err(short(message, 140));
        }
        let mut text = String::new();
        let mut calls = Vec::new();
        match self.service {
            Service::Claude => {
                let content = v["content"].as_array().cloned().unwrap_or_default();
                let stop = v["stop_reason"].as_str().unwrap_or_default();
                if stop == "refusal" {
                    let why = v["stop_details"]["explanation"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("it would not answer that");
                    return Ok(Reply { text, calls, stop: Stop::Refused(short(why, 140)) });
                }
                for block in &content {
                    match block["type"].as_str() {
                        Some("text") => push_text(&mut text, block["text"].as_str().unwrap_or_default()),
                        Some("tool_use") => calls.push(Call { id: str_of(&block["id"]), name: str_of(&block["name"]), input: block["input"].clone() }),
                        _ => {}
                    }
                }
                if content.is_empty() {
                    return if stop == "end_turn" { Ok(Reply { text, calls, stop: Stop::Answered }) } else { Err("an empty answer came back".into()) };
                }
                self.items.push(json!({"role": "assistant", "content": content}));
                let stop = match stop {
                    "tool_use" if !calls.is_empty() => Stop::Tools,
                    "max_tokens" => Stop::Cut,
                    _ => Stop::Answered,
                };
                Ok(Reply { text, calls, stop })
            }
            Service::OpenAi => {
                let output = v["output"].as_array().cloned().unwrap_or_default();
                let mut refusal = None;
                for item in &output {
                    match item["type"].as_str() {
                        Some("message") => {
                            for part in item["content"].as_array().into_iter().flatten() {
                                match part["type"].as_str() {
                                    Some("output_text") => push_text(&mut text, part["text"].as_str().unwrap_or_default()),
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
                        _ => {}
                    }
                }
                let incomplete = v["status"].as_str() == Some("incomplete");
                if incomplete && v["incomplete_details"]["reason"].as_str() == Some("content_filter") {
                    refusal.get_or_insert_with(|| "it would not answer that".into());
                }
                if let Some(why) = refusal {
                    return Ok(Reply { text: String::new(), calls: Vec::new(), stop: Stop::Refused(short(&why, 140)) });
                }
                if output.is_empty() {
                    return Err("an empty answer came back".into());
                }
                self.items.extend(output);
                let stop = if incomplete {
                    Stop::Cut
                } else if calls.is_empty() {
                    Stop::Answered
                } else {
                    Stop::Tools
                };
                Ok(Reply { text, calls, stop })
            }
        }
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
        }
    }
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
fn short(text: &str, n: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= n {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(n - 1).collect::<String>().trim_end())
    }
}

/// An answer as the small screen shows it: Markdown's marks taken out, its
/// bullets as dots, no runs of blank lines.
fn plain(text: &str) -> String {
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
fn request(service: Service, key: &str, body: &Value) -> Result<Value, String> {
    if demo() {
        return Ok(demo_reply(service, body));
    }
    let headers = match service {
        Service::OpenAi => format!("Authorization: Bearer {key}\nContent-Type: application/json\n"),
        Service::Claude => format!("x-api-key: {key}\nanthropic-version: {CLAUDE_VERSION}\nanthropic-beta: {CLAUDE_BETA}\nContent-Type: application/json\n"),
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
    let (question, rounds, last) = match service {
        Service::Claude => {
            let messages = body["messages"].as_array().cloned().unwrap_or_default();
            let at = messages.iter().rposition(|m| m["role"] == "user" && m["content"].is_string()).unwrap_or(0);
            let question = messages.get(at).and_then(|m| m["content"].as_str()).unwrap_or_default().to_lowercase();
            let results: Vec<&Value> = messages.iter().skip(at + 1).filter(|m| m["role"] == "user").collect();
            let last = results.last().and_then(|m| m["content"].as_array()).and_then(|c| c.last()).map(|b| str_of(&b["content"])).unwrap_or_default();
            (question, results.len(), last)
        }
        Service::OpenAi => {
            let input = body["input"].as_array().cloned().unwrap_or_default();
            let at = input.iter().rposition(|i| i["role"] == "user").unwrap_or(0);
            let question = input.get(at).and_then(|i| i["content"].as_str()).unwrap_or_default().to_lowercase();
            let outputs = |i: &Value| i["type"] == "function_call_output";
            let rounds = (at + 1..input.len()).filter(|&k| outputs(&input[k]) && !outputs(&input[k - 1])).count();
            let last = input.iter().skip(at + 1).rev().find(|i| outputs(i)).map(|i| str_of(&i["output"])).unwrap_or_default();
            (question, rounds, last)
        }
    };
    let deploy = question.contains("deploy");
    let (text, calls): (&str, Vec<(&str, Value)>) = match (deploy, rounds) {
        (true, 0) => ("", vec![("run_action", json!({"name": "Deploy robot code", "reason": "you asked for the code to be deployed"}))]),
        (true, _) if last.contains("declined") => ("Left it: nothing was deployed. Say when the robot is on its cart and disabled, and I'll run it then.", vec![]),
        (true, _) => ("Deployed: the build passed in 41 seconds and the robot program restarted.", vec![]),
        (false, 0) => ("Looking at the robot first.", vec![("robot_status", json!({})), ("robot_agent", json!({}))]),
        (false, 1) => ("", vec![("run_action", json!({"name": "Ping the robot", "reason": "to check the link is steady"}))]),
        (false, _) => (DEMO_ANSWER, vec![]),
    };
    match service {
        Service::Claude => {
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
}

const GAP: f32 = 6.0;
const INPUT_LIMIT: usize = 600;
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
        self.revision += 1;
        self.scroll.jump(0.0);
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
        self.question = text.clone();
        self.rounds = 0;
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
        if !reply.text.trim().is_empty() {
            self.push(Entry::Said(plain(&reply.text)));
        }
        match reply.stop {
            Stop::Answered => {}
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
            return;
        }
        self.ask();
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
        let cy = bar.cy();
        let send_w = 22.0;
        if self.input.is_empty() {
            let words = if self.idle() || matches!(self.busy, Busy::Approving { .. }) { "type a question" } else { "esc: stop" };
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
        "run_action" => {
            let name = call.input["name"].as_str().unwrap_or_default().to_lowercase();
            StepWords { working: format!("running {name}"), done: format!("ran {name}") }
        }
        other => same(&other.replace('_', " ")),
    }
}

/// The conversation's text: the sans at the caption's size (which follows the
/// text size chosen), so more of it fits on the panel.
fn chat_style(g: &Gfx) -> bezel::TextStyle {
    bezel::TextStyle::body(g.ty.caption.size)
}

fn chat_line(g: &Gfx) -> f32 {
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

    fn resume(&mut self) {
        // A key added or the service changed: a fresh conversation, as the
        // two services cannot carry on each other's.
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
            Key::Enter => self.send(),
            Key::Backspace => {
                self.input.pop();
            }
            Key::Char(c) if !c.is_control() && self.input.chars().count() < INPUT_LIMIT => self.input.push(c),
            _ => {}
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
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
        moving || rising || !self.idle()
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

/// Which service answers, how hard it thinks, and a fresh conversation.
pub struct Choices {
    prefs: Prefs,
    keys: Keys,
    list: ListState,
}

const CHOICE_ROWS: usize = 4;

impl Choices {
    fn new() -> Self {
        Choices { prefs: Prefs::load(), keys: Keys::load(), list: ListState::default() }
    }

    fn with_keys(&self) -> Vec<Service> {
        Service::ALL.into_iter().filter(|s| self.keys.get(s.key()).is_some()).collect()
    }
}

impl Screen for Choices {
    fn title(&self) -> &str {
        "Assistant"
    }

    fn resume(&mut self) {
        self.keys = Keys::load();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match (key, self.list.focus) {
            (Key::Back | Key::Leave, _) => return Nav::Pop,
            (Key::Enter | Key::Right, 0) => {
                let with_keys = self.with_keys();
                let current = answering(&self.prefs, &self.keys).map(|(s, _)| s);
                match with_keys.len() {
                    0 | 1 => *cx.say = Some("the other service needs its key in keys".into()),
                    _ => {
                        let at = with_keys.iter().position(|s| Some(*s) == current).unwrap_or(0);
                        self.prefs.service = Some(with_keys[(at + 1) % with_keys.len()]);
                        self.prefs.save();
                        *cx.say = Some("a new conversation with it".into());
                    }
                }
            }
            (Key::Enter | Key::Right, 1) => {
                self.prefs.effort = self.prefs.effort.next();
                self.prefs.save();
            }
            (Key::Enter | Key::Right, 2) => {
                NEW_CHAT.store(true, Ordering::Relaxed);
                return Nav::Pop;
            }
            (Key::Enter | Key::Right, 3) => return Nav::Push(Box::new(crate::screens::SettingsApp::keys_page())),
            (other, _) => crate::screens::list_key(&mut self.list, other, CHOICE_ROWS),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let answering = answering(&self.prefs, &self.keys).map(|(s, _)| s);
        for (i, y) in self.list.layout(g, CHOICE_ROWS, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value, color) = match i {
                0 => ("Answers with", answering.map_or("no key yet".to_owned(), |s| s.model_name().to_owned()), t.ink),
                1 => ("Effort", self.prefs.effort.name().to_owned(), t.ink),
                2 => ("New conversation", String::new(), t.dim),
                _ => ("Keys", format!("{} of 2 set", self.with_keys().len()), t.dim),
            };
            g.row(r, name, &value, color, i == self.list.focus);
        }
        self.list.scrollbar(g, CHOICE_ROWS, top, bottom);
        g.hint(match self.list.focus {
            0 => "enter: the other service · a new conversation",
            1 => "light answers sooner · medium thinks it through",
            2 => "enter: clear this one",
            _ => "openai and claude keys",
        });
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
        assert!(body["tools"].as_array().unwrap().iter().all(|t| t["parameters"]["additionalProperties"] == false && t["strict"] == true));
        let mut chat = Chat::new(Service::Claude);
        chat.ask("how is the robot?");
        let body = chat.body(Effort::Medium);
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["fallbacks"], "default");
        assert!(body.get("thinking").is_none(), "Opus 5.5 thinks as it sees fit; nothing to set");
        assert!(body.get("temperature").is_none());
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body["tools"].as_array().unwrap().iter().all(|t| t["input_schema"]["type"] == "object"));
        let names: Vec<&str> = tool_specs().iter().map(|(n, ..)| *n).collect();
        assert_eq!(names, ["robot_status", "robot_values", "robot_agent", "match_logs", "list_actions", "run_action"]);
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
        page
    }

    #[test]
    fn a_question_looks_at_the_robot_runs_an_action_and_is_answered_by_either_service() {
        crate::shell::tests::sandbox();
        for service in Service::ALL {
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
