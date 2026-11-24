//! Setting the laptop up, by itself: one *Set up* that does every step and
//! stops only where a person has to sign in or approve something in a
//! browser.
//!
//! 1. **Claude Code**: found, or installed with Anthropic's own installer
//!    (`irm https://claude.ai/install.ps1 | iex`, the native install the
//!    setup docs give for PowerShell; no administrator needed). Signed in,
//!    or `claude auth login --claudeai` started, which opens the browser;
//!    then it waits until `claude auth status` says signed in. A code the
//!    browser shows instead can be pasted in the window.
//! 2. **Tailscale**: found, or installed (`winget install --id
//!    tailscale.tailscale -e`, Windows asks to allow it). Signed in, or
//!    `tailscale login` started and its link opened. MagicDNS checked, and
//!    the admin page opened only if it is off. Funnel on the bridge's port,
//!    or `tailscale funnel --bg` run on the first of 443, 8443 and 10000
//!    that serves nothing else; if Funnel or HTTPS is not yet allowed, the
//!    link it prints is opened, and once allowed it is run again. The port
//!    is remembered.
//! 3. **The bridge**: running, and started with Windows.
//! 4. **Kept awake when plugged in** (unless unticked): sleep and hibernate
//!    Never and the lid Does nothing, on mains power only, never battery,
//!    through `powercfg`; the values before are kept, to be restored. If
//!    Windows wants an administrator for it, it asks once.
//! 5. **Pairing**: the QR code and the six-digit code (the window shows them).
//!
//! Every step looks first and acts only if it must, so running it again,
//! or after it stopped part way, does again only what is not done. Each
//! thing that changes the computer is done once a run (Funnel twice: before
//! and after its approval). On each later start the bridge looks again
//! ([`repair`]): what drifted and needs nobody is put right; what needs the
//! person is said.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::system::{Job, System, Tool};
use crate::tailscale;

/// Anthropic's installer for Windows PowerShell (code.claude.com/docs/en/setup).
pub const CLAUDE_INSTALL: &str = "irm https://claude.ai/install.ps1 | iex";
/// Where MagicDNS is switched on.
pub const DNS_PAGE: &str = "https://login.tailscale.com/admin/dns";
/// Where Tailscale for Windows is, when winget is not.
pub const TAILSCALE_PAGE: &str = "https://tailscale.com/download/windows";

// ---------------------------------------------------------------- steps ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Claude,
    Tailscale,
    Bridge,
    Power,
    Pair,
}

pub const STEPS: [Step; 5] = [Step::Claude, Step::Tailscale, Step::Bridge, Step::Power, Step::Pair];

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Claude => "Claude Code",
            Step::Tailscale => "Tailscale and Funnel",
            Step::Bridge => "The bridge",
            Step::Power => "Keep awake",
            Step::Pair => "Pairing",
        }
    }
}

/// How a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Working,
    /// Waiting for the person: to sign in, or approve, in the browser.
    Waiting,
    Done,
    Failed,
    /// Not wanted (keeping awake unticked).
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub step: Step,
    pub state: State,
    /// What it is doing, or what it found.
    pub words: String,
    /// For a failure: what to do about it.
    pub fix: String,
    /// A page the person may need (opened once already).
    pub url: Option<String>,
    /// The sign-in asks for a code the browser shows.
    pub wants_code: bool,
}

/// The whole setup, as the window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub rows: Vec<Row>,
    pub running: bool,
    /// The address Funnel publishes the bridge at, once known.
    pub address: Option<String>,
}

impl Default for Progress {
    fn default() -> Progress {
        Progress { rows: STEPS.iter().map(|&step| Row { step, state: State::Pending, words: String::new(), fix: String::new(), url: None, wants_code: false }).collect(), running: false, address: None }
    }
}

impl Progress {
    pub fn row(&self, step: Step) -> &Row {
        &self.rows[STEPS.iter().position(|s| *s == step).unwrap_or(0)]
    }

    fn row_mut(&mut self, step: Step) -> &mut Row {
        let i = STEPS.iter().position(|s| *s == step).unwrap_or(0);
        &mut self.rows[i]
    }

    /// Everything before pairing done (or not wanted).
    pub fn ready(&self) -> bool {
        self.rows.iter().filter(|r| r.step != Step::Pair).all(|r| matches!(r.state, State::Done | State::Skipped))
    }
}

/// What the window and the setup's thread share.
#[derive(Default)]
pub struct Shared {
    pub progress: Mutex<Progress>,
    /// A code typed in the window for the sign-in.
    pub pasted: Mutex<Option<String>>,
}

impl Shared {
    fn set(&self, step: Step, state: State, words: &str) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let r = p.row_mut(step);
        r.state = state;
        r.words = words.to_owned();
        if state != State::Failed {
            r.fix.clear();
        }
        if state != State::Waiting {
            r.wants_code = false;
        }
    }

    fn fail(&self, step: Step, words: &str, fix: &str) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let r = p.row_mut(step);
        r.state = State::Failed;
        r.words = words.to_owned();
        r.fix = fix.to_owned();
        r.wants_code = false;
    }

    fn with<T>(&self, f: impl FnOnce(&mut Progress) -> T) -> T {
        f(&mut self.progress.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn snapshot(&self) -> Progress {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

// ---------------------------------------------------------------- saved ---

/// The AC power values before the setup changed them.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerBefore {
    pub scheme: String,
    pub standby: u32,
    pub hibernate: u32,
    /// None where there is no lid.
    pub lid: Option<u32>,
}

/// What the setup remembers: `setup.json` in the bridge's folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// Set up all the way once: later starts look again and repair.
    pub completed: bool,
    /// The Funnel port chosen.
    pub port: Option<u16>,
    /// The person wants the laptop kept awake when plugged in.
    pub keep_awake: bool,
    pub power_before: Option<PowerBefore>,
    /// Start with Windows was switched on by the setup, and since switched
    /// off by the person (then it is left off).
    pub at_login_by_setup: bool,
    pub at_login_declined: bool,
}

impl Default for Saved {
    fn default() -> Saved {
        Saved { completed: false, port: None, keep_awake: true, power_before: None, at_login_by_setup: false, at_login_declined: false }
    }
}

impl Saved {
    fn file(dir: &Path) -> PathBuf {
        dir.join("setup.json")
    }

    pub fn load(dir: &Path) -> Saved {
        let Ok(v) = serde_json::from_str::<Value>(&std::fs::read_to_string(Saved::file(dir)).unwrap_or_default()) else { return Saved::default() };
        let power = &v["power_before"];
        let power_before = power["scheme"].as_str().map(|scheme| PowerBefore {
            scheme: scheme.to_owned(),
            standby: power["standby"].as_u64().unwrap_or(0) as u32,
            hibernate: power["hibernate"].as_u64().unwrap_or(0) as u32,
            lid: power["lid"].as_u64().map(|l| l as u32),
        });
        Saved {
            completed: v["completed"] == true,
            port: v["port"].as_u64().and_then(|p| u16::try_from(p).ok()),
            keep_awake: v["keep_awake"].as_bool().unwrap_or(true),
            power_before,
            at_login_by_setup: v["at_login_by_setup"] == true,
            at_login_declined: v["at_login_declined"] == true,
        }
    }

    pub fn save(&self, dir: &Path) {
        let power = self.power_before.as_ref().map(|p| json!({"scheme": p.scheme, "standby": p.standby, "hibernate": p.hibernate, "lid": p.lid}));
        let v = json!({"completed": self.completed, "port": self.port, "keep_awake": self.keep_awake, "power_before": power, "at_login_by_setup": self.at_login_by_setup, "at_login_declined": self.at_login_declined});
        std::fs::create_dir_all(dir).ok();
        let path = Saved::file(dir);
        let temp = path.with_extension("saving");
        if std::fs::write(&temp, v.to_string()).is_ok() {
            std::fs::rename(&temp, &path).ok();
        }
    }
}

/// How the setup runs.
#[derive(Clone)]
pub struct Options {
    /// The bridge's port, which Funnel publishes.
    pub bridge_port: u16,
    /// The bridge running, or why not.
    pub bridge: Result<(), String>,
    /// How long a sign-in or an approval is waited for.
    pub wait: Duration,
    /// How often it looks while it waits.
    pub poll: Duration,
}

impl Options {
    pub fn new(bridge: Result<(), String>) -> Options {
        Options { bridge_port: crate::PORT, bridge, wait: Duration::from_secs(10 * 60), poll: Duration::from_secs(2) }
    }
}

// -------------------------------------------------------------- looking ---

/// The first web address in some words.
pub fn first_url(text: &str) -> Option<String> {
    let at = text.find("https://")?;
    let url: String = text[at..].chars().take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != '<' && *c != '>').collect();
    Some(url.trim_end_matches(['.', ',', ')', ';']).to_owned())
}

fn claude_signed_in(sys: &dyn System, exe: &Path) -> Option<bool> {
    let out = sys.run(exe, &["auth", "status", "--json"], Duration::from_secs(30));
    serde_json::from_str::<Value>(&out.stdout).ok()?["loggedIn"].as_bool()
}

fn claude_version(sys: &dyn System, exe: &Path) -> String {
    let out = sys.run(exe, &["--version"], Duration::from_secs(30));
    out.stdout.split_whitespace().next().unwrap_or("").to_owned()
}

/// `tailscale status --json`, with only what the setup reads: never the
/// account it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TsStatus {
    pub backend: String,
    pub auth_url: Option<String>,
    pub magic_dns: Option<bool>,
    /// The node may use Funnel and HTTPS.
    pub funnel_allowed: bool,
    pub status: Value,
}

fn ts_status(sys: &dyn System, exe: &Path) -> Option<TsStatus> {
    let out = sys.run(exe, &["status", "--json"], Duration::from_secs(30));
    let v: Value = serde_json::from_str(&out.stdout).ok()?;
    let caps = &v["Self"]["CapMap"];
    let has = |k: &str| caps.get(k).is_some() || v["Self"]["Capabilities"].as_array().is_some_and(|a| a.iter().any(|c| c == k));
    Some(TsStatus {
        backend: v["BackendState"].as_str().unwrap_or_default().to_owned(),
        auth_url: v["AuthURL"].as_str().filter(|u| u.starts_with("https://")).map(str::to_owned),
        magic_dns: v["CurrentTailnet"]["MagicDNSEnabled"].as_bool(),
        funnel_allowed: has("funnel") && has("https"),
        status: json!({"BackendState": v["BackendState"], "Self": {"DNSName": v["Self"]["DNSName"]}}),
    })
}

fn funnel(sys: &dyn System, exe: &Path, status: &TsStatus, port: u16) -> tailscale::Tailscale {
    let out = sys.run(exe, &["funnel", "status", "--json"], Duration::from_secs(30));
    let f: Option<Value> = serde_json::from_str(&out.stdout).ok();
    tailscale::read(Some(&status.status), f.as_ref(), port)
}

/// The AC values as they are: the scheme, sleep, hibernate, lid.
fn power_now(sys: &dyn System, powercfg: &Path) -> Option<PowerBefore> {
    let scheme_out = sys.run(powercfg, &["/getactivescheme"], Duration::from_secs(20));
    let scheme = scheme_out.stdout.split_whitespace().find(|w| w.len() == 36 && w.chars().filter(|c| *c == '-').count() == 4)?.to_owned();
    let read = |sub: &str, setting: &str| -> Option<u32> {
        let out = sys.run(powercfg, &["/qh", &scheme, sub, setting], Duration::from_secs(20));
        let line = out.stdout.lines().find(|l| l.contains("Current AC Power Setting Index"))?;
        u32::from_str_radix(line.rsplit("0x").next()?.trim(), 16).ok()
    };
    Some(PowerBefore { standby: read("SUB_SLEEP", "STANDBYIDLE")?, hibernate: read("SUB_SLEEP", "HIBERNATEIDLE")?, lid: read("SUB_BUTTONS", "LIDACTION"), scheme })
}

/// What sets the AC values to these: each `powercfg` argument list.
fn power_commands(to: &PowerBefore) -> Vec<Vec<String>> {
    let s = |x: &str| x.to_owned();
    let mut c = vec![
        vec![s("/setacvalueindex"), to.scheme.clone(), s("SUB_SLEEP"), s("STANDBYIDLE"), to.standby.to_string()],
        vec![s("/setacvalueindex"), to.scheme.clone(), s("SUB_SLEEP"), s("HIBERNATEIDLE"), to.hibernate.to_string()],
    ];
    if let Some(lid) = to.lid {
        c.push(vec![s("/setacvalueindex"), to.scheme.clone(), s("SUB_BUTTONS"), s("LIDACTION"), lid.to_string()]);
    }
    c.push(vec![s("/setactive"), to.scheme.clone()]);
    c
}

/// Set the AC values: as this user, and if Windows refuses, once more with
/// an administrator's rights (one prompt for all of them).
fn set_power(sys: &dyn System, powercfg: &Path, to: &PowerBefore) -> Result<(), String> {
    let commands = power_commands(to);
    let mut refused = false;
    for c in &commands {
        let args: Vec<&str> = c.iter().map(String::as_str).collect();
        if !sys.run(powercfg, &args, Duration::from_secs(20)).ok() {
            refused = true;
            break;
        }
    }
    if !refused {
        return Ok(());
    }
    let cmd = sys.find(Tool::Cmd).ok_or("cmd.exe is not there")?;
    let line = commands.iter().map(|c| format!("\"{}\" {}", powercfg.display(), c.join(" "))).collect::<Vec<_>>().join(" && ");
    match sys.run_elevated(&cmd, &format!("/d /s /c \"{line}\""))? {
        0 => Ok(()),
        code => Err(format!("powercfg stopped with {code}")),
    }
}

/// The AC values that keep it awake: the same scheme, all Never / Do nothing.
fn awake(now: &PowerBefore) -> PowerBefore {
    PowerBefore { scheme: now.scheme.clone(), standby: 0, hibernate: 0, lid: now.lid.map(|_| 0) }
}

/// How things stand, looked at only (nothing changed): for the window when
/// it opens, and before each step.
pub fn look(sys: &dyn System, saved: &Saved, opts: &Options) -> Progress {
    let mut p = Progress::default();
    {
        let r = p.row_mut(Step::Claude);
        match sys.find(Tool::Claude) {
            None => (r.words, r.state) = ("not installed".into(), State::Pending),
            Some(exe) => match claude_signed_in(sys, &exe) {
                Some(true) => (r.words, r.state) = (format!("Claude Code {} · signed in", claude_version(sys, &exe)).replace("  ", " "), State::Done),
                _ => (r.words, r.state) = ("installed · not signed in".into(), State::Pending),
            },
        }
    }
    match sys.find(Tool::Tailscale).and_then(|exe| ts_status(sys, &exe).map(|s| (exe, s))) {
        None => {
            let r = p.row_mut(Step::Tailscale);
            r.words = if sys.find(Tool::Tailscale).is_some() { "installed · not running".into() } else { "not installed".into() };
        }
        Some((exe, s)) => {
            let t = funnel(sys, &exe, &s, opts.bridge_port);
            let r = p.row_mut(Step::Tailscale);
            match (s.backend.as_str(), &t.address) {
                ("Running", Some(a)) => {
                    (r.words, r.state) = (format!("Funnel publishes {}", a.trim_start_matches("https://")), State::Done);
                    p.address = Some(a.clone());
                }
                ("Running", None) => r.words = "signed in · Funnel not set up".into(),
                _ => r.words = "not signed in".into(),
            }
        }
    }
    {
        let at_login = sys.at_login();
        let r = p.row_mut(Step::Bridge);
        match &opts.bridge {
            Ok(()) => (r.words, r.state) = (if at_login { "running · starts with Windows".into() } else { "running".into() }, if at_login || saved.at_login_declined { State::Done } else { State::Pending }),
            Err(why) => (r.words, r.fix, r.state) = (why.clone(), "Quit what holds the port, then start the bridge again".into(), State::Failed),
        }
    }
    {
        let now = sys.find(Tool::Powercfg).and_then(|pc| power_now(sys, &pc));
        let r = p.row_mut(Step::Power);
        match (saved.keep_awake, now) {
            (false, _) => (r.words, r.state) = ("left as it is".into(), State::Skipped),
            (true, Some(n)) if n == awake(&n) => (r.words, r.state) = ("on mains power: never sleeps, lid does nothing".into(), State::Done),
            (true, Some(n)) => {
                // Only what would change, named.
                let mut what = Vec::new();
                if n.standby != 0 {
                    what.push("sleep");
                }
                if n.hibernate != 0 {
                    what.push("hibernate");
                }
                if n.lid.is_some_and(|l| l != 0) {
                    what.push("the lid");
                }
                r.words = format!("would change on mains power: {}", what.join(", "));
            }
            (true, None) => r.words = "its settings could not be read".into(),
        }
    }
    p
}

// -------------------------------------------------------------- running ---

/// Wait for `done` while watching `job` (a sign-in): its link opened once,
/// a code pasted in the window typed to it. True when done in time.
fn wait_for(sys: &dyn System, shared: &Shared, step: Step, job: &mut Option<Box<dyn Job>>, opts: &Options, open_link: bool, done: &mut dyn FnMut() -> bool) -> bool {
    let began = sys.now();
    let mut opened = false;
    loop {
        if done() {
            if let Some(j) = job.as_mut() {
                if j.ended().is_none() {
                    j.stop();
                }
            }
            return true;
        }
        if let Some(j) = job.as_mut() {
            let said = j.said();
            if let Some(url) = first_url(&said) {
                if open_link && !opened {
                    sys.open_url(&url);
                    opened = true;
                }
                shared.with(|p| p.row_mut(step).url = Some(url));
            }
            if said.to_lowercase().contains("paste code") {
                shared.with(|p| p.row_mut(step).wants_code = true);
            }
            if let Some(code) = shared.pasted.lock().unwrap_or_else(|e| e.into_inner()).take() {
                j.type_line(code.trim());
            }
        }
        if sys.now().saturating_sub(began) >= opts.wait {
            if let Some(j) = job.as_mut() {
                j.stop();
            }
            return false;
        }
        sys.sleep(opts.poll);
    }
}

fn step_claude(sys: &dyn System, shared: &Shared, opts: &Options) -> bool {
    let step = Step::Claude;
    let exe = match sys.find(Tool::Claude) {
        Some(exe) => exe,
        None => {
            shared.set(step, State::Working, "installing Claude Code with Anthropic's installer");
            let Some(ps) = sys.find(Tool::PowerShell) else {
                shared.fail(step, "Windows PowerShell is not there", "Install Claude Code from code.claude.com/docs/en/setup, then Set up again");
                return false;
            };
            let out = sys.run(&ps, &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", CLAUDE_INSTALL], Duration::from_secs(20 * 60));
            match sys.find(Tool::Claude) {
                Some(exe) => exe,
                None => {
                    shared.fail(step, &format!("the installer did not finish: {}", out.last_line()), "Check the internet connection and Set up again");
                    return false;
                }
            }
        }
    };
    let version = claude_version(sys, &exe);
    if claude_signed_in(sys, &exe) == Some(true) {
        shared.set(step, State::Done, &format!("Claude Code {version} · signed in"));
        return true;
    }
    shared.set(step, State::Waiting, "sign in to Claude in your browser");
    // Claude Code opens the browser itself; its link is kept for the window.
    let mut job = sys.start(&exe, &["auth", "login", "--claudeai"], false);
    // A sign-in that wants a terminal of its own gets one.
    sys.sleep(opts.poll);
    if let Some(j) = job.as_mut() {
        let said = j.said().to_lowercase();
        if matches!(j.ended(), Some(c) if c != Some(0)) && (said.contains("not a terminal") || said.contains("raw mode")) {
            job = sys.start(&exe, &["auth", "login", "--claudeai"], true);
        }
    }
    if job.is_none() {
        shared.fail(step, "claude auth login would not start", "Run  claude auth login  in a terminal, then Set up again");
        return false;
    }
    let mut signed = || claude_signed_in(sys, &exe) == Some(true);
    if wait_for(sys, shared, step, &mut job, opts, false, &mut signed) {
        shared.set(step, State::Done, &format!("Claude Code {version} · signed in"));
        true
    } else {
        shared.fail(step, "no sign-in came within ten minutes", "Set up again when you can sign in");
        false
    }
}

fn step_tailscale(sys: &dyn System, shared: &Shared, saved: &mut Saved, opts: &Options) -> Option<String> {
    let step = Step::Tailscale;
    let exe = match sys.find(Tool::Tailscale) {
        Some(exe) => exe,
        None => {
            let Some(winget) = sys.find(Tool::Winget) else {
                sys.open_url(TAILSCALE_PAGE);
                shared.fail(step, "winget is not on this computer", "Install Tailscale from the page that opened, then Set up again");
                return None;
            };
            shared.set(step, State::Working, "installing Tailscale · Windows will ask to allow it");
            let out = sys.run(&winget, &["install", "--id", "tailscale.tailscale", "-e", "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"], Duration::from_secs(20 * 60));
            match sys.find(Tool::Tailscale) {
                Some(exe) => exe,
                None => {
                    shared.fail(step, &format!("the install did not finish: {}", out.last_line()), "Set up again and allow it when Windows asks");
                    return None;
                }
            }
        }
    };
    // Its service, just installed, may take a moment to answer.
    let mut status = None;
    for _ in 0..15 {
        status = ts_status(sys, &exe);
        if status.as_ref().is_some_and(|s| !s.backend.is_empty() && s.backend != "NoState") {
            break;
        }
        sys.sleep(opts.poll);
    }
    let Some(mut s) = status else {
        shared.fail(step, "Tailscale is not answering", "Start Tailscale from the Start menu, then Set up again");
        return None;
    };
    // Signed in and connected.
    if s.backend == "Stopped" {
        shared.set(step, State::Working, "connecting Tailscale");
        sys.run(&exe, &["up", "--timeout", "60s"], Duration::from_secs(90));
        s = ts_status(sys, &exe).unwrap_or(s);
    }
    if s.backend != "Running" {
        shared.set(step, State::Waiting, "sign in to Tailscale in your browser");
        let mut job = sys.start(&exe, &["login", "--timeout", "10m"], false);
        if let Some(url) = s.auth_url.clone() {
            sys.open_url(&url);
            shared.with(|p| p.row_mut(step).url = Some(url));
        }
        let opened_already = s.auth_url.is_some();
        let mut running = || ts_status(sys, &exe).is_some_and(|x| x.backend == "Running");
        if !wait_for(sys, shared, step, &mut job, opts, !opened_already, &mut running) {
            shared.fail(step, "no Tailscale sign-in came within ten minutes", "Set up again when you can sign in");
            return None;
        }
        s = ts_status(sys, &exe).unwrap_or(s);
    }
    // MagicDNS: only if it is off.
    if s.magic_dns == Some(false) {
        shared.set(step, State::Waiting, "turn on MagicDNS on the page that opened");
        sys.open_url(DNS_PAGE);
        shared.with(|p| p.row_mut(step).url = Some(DNS_PAGE.into()));
        let mut on = || ts_status(sys, &exe).is_some_and(|x| x.magic_dns != Some(false));
        if !wait_for(sys, shared, step, &mut None, opts, false, &mut on) {
            shared.fail(step, "MagicDNS is still off", "Turn it on in the Tailscale admin console, then Set up again");
            return None;
        }
        s = ts_status(sys, &exe).unwrap_or(s);
    }
    // Funnel on the bridge's port.
    let mut t = funnel(sys, &exe, &s, opts.bridge_port);
    if let Some(a) = t.address.clone() {
        saved.port = port_of(&a);
        shared.set(step, State::Done, &format!("Funnel publishes {}", a.trim_start_matches("https://")));
        return Some(a);
    }
    let port = saved.port.filter(|p| !t.taken.contains(p)).or_else(|| tailscale::FUNNEL_PORTS.iter().copied().find(|p| !t.taken.contains(p)));
    let Some(port) = port else {
        shared.fail(step, "Funnel's three ports (443, 8443, 10000) all serve something else", "Free one with  tailscale funnel --https=<port> off, then Set up again");
        return None;
    };
    saved.port = Some(port);
    let https = format!("--https={port}");
    let target = opts.bridge_port.to_string();
    let args = ["funnel", "--bg", "--yes", https.as_str(), target.as_str()];
    for attempt in 0..2 {
        shared.set(step, State::Working, &format!("publishing the bridge with Funnel on port {port}"));
        let mut job = sys.start(&exe, &args, false);
        // It may print a link to allow Funnel or HTTPS, and wait for it.
        let mut opened = false;
        let began = sys.now();
        loop {
            let Some(j) = job.as_mut() else { break };
            // Only a page to allow it: the address it prints once it works is not one.
            if let Some(url) = first_url(&j.said()).filter(|u| u.starts_with("https://login.tailscale.com/")) {
                if !opened {
                    sys.open_url(&url);
                    opened = true;
                    shared.set(step, State::Waiting, "allow Funnel for this laptop on the page that opened");
                    shared.with(|p| p.row_mut(step).url = Some(url));
                }
            }
            if j.ended().is_some() || sys.now().saturating_sub(began) >= opts.wait {
                if j.ended().is_none() {
                    j.stop();
                }
                break;
            }
            sys.sleep(opts.poll);
        }
        s = ts_status(sys, &exe).unwrap_or(s);
        t = funnel(sys, &exe, &s, opts.bridge_port);
        if let Some(a) = t.address.clone() {
            shared.set(step, State::Done, &format!("Funnel publishes {}", a.trim_start_matches("https://")));
            return Some(a);
        }
        if attempt == 1 {
            break;
        }
        // Not allowed yet: wait for the approval, then once more.
        if !s.funnel_allowed {
            if !opened {
                shared.set(step, State::Waiting, "allow Funnel for this laptop in the Tailscale admin console");
            }
            let mut allowed = || ts_status(sys, &exe).is_some_and(|x| x.funnel_allowed);
            if !wait_for(sys, shared, step, &mut None, opts, false, &mut allowed) {
                shared.fail(step, "Funnel was not allowed within ten minutes", "Allow it on the page Tailscale opened, then Set up again");
                return None;
            }
        }
    }
    shared.fail(step, "Funnel did not start", &format!("In a terminal:  tailscale funnel --bg --https={port} {}  then Set up again", opts.bridge_port));
    None
}

fn port_of(address: &str) -> Option<u16> {
    match address.trim_start_matches("https://").rsplit_once(':') {
        Some((_, p)) => p.parse().ok(),
        None => Some(443),
    }
}

fn step_bridge(sys: &dyn System, shared: &Shared, saved: &mut Saved, opts: &Options) -> bool {
    let step = Step::Bridge;
    if let Err(why) = &opts.bridge {
        shared.fail(step, why, "Quit what holds the port, then start the bridge again");
        return false;
    }
    if !sys.at_login() && !saved.at_login_declined {
        if let Err(why) = sys.set_at_login(true) {
            shared.fail(step, &format!("could not start with Windows: {why}"), "Switch on Start with Windows below");
            return false;
        }
        saved.at_login_by_setup = true;
    }
    let words = if sys.at_login() { "running · starts with Windows" } else { "running" };
    shared.set(step, State::Done, words);
    true
}

fn step_power(sys: &dyn System, shared: &Shared, saved: &mut Saved) -> bool {
    let step = Step::Power;
    if !saved.keep_awake {
        shared.set(step, State::Skipped, "left as it is");
        return true;
    }
    let Some(pc) = sys.find(Tool::Powercfg) else {
        shared.fail(step, "powercfg is not there", "Set Never and Do nothing yourself in Power Options");
        return false;
    };
    let Some(now) = power_now(sys, &pc) else {
        shared.fail(step, "its power settings could not be read", "Set Never and Do nothing yourself in Power Options");
        return false;
    };
    let want = awake(&now);
    if now == want {
        shared.set(step, State::Done, "on mains power: never sleeps, lid does nothing");
        return true;
    }
    // The values before, kept once: never this setup's own.
    if saved.power_before.is_none() {
        saved.power_before = Some(now.clone());
    }
    shared.set(step, State::Working, "keeping it awake on mains power");
    match set_power(sys, &pc, &want) {
        Ok(()) if power_now(sys, &pc).is_some_and(|n| n == want) => {
            shared.set(step, State::Done, "on mains power: never sleeps, lid does nothing");
            true
        }
        Ok(()) => {
            shared.fail(step, "Windows kept its own values", "Set Never and Do nothing yourself in Power Options");
            false
        }
        Err(why) => {
            shared.fail(step, &format!("not changed: {why}"), "Set up again and choose Yes, or untick Keep awake");
            false
        }
    }
}

/// The whole setup, from where it stands. The address, if it got that far.
pub fn run(sys: &dyn System, dir: &Path, opts: &Options, shared: &Shared) -> Option<String> {
    let mut saved = Saved::load(dir);
    shared.with(|p| {
        *p = look(sys, &saved, opts);
        p.running = true;
    });
    let claude = step_claude(sys, shared, opts);
    let address = step_tailscale(sys, shared, &mut saved, opts);
    saved.save(dir);
    let bridge = step_bridge(sys, shared, &mut saved, opts);
    saved.save(dir);
    let power = step_power(sys, shared, &mut saved);
    saved.save(dir);
    let ready = claude && address.is_some() && bridge && power;
    if ready {
        saved.completed = true;
        saved.save(dir);
        shared.set(Step::Pair, State::Working, "scan the code with the handheld's Camera");
    } else {
        shared.set(Step::Pair, State::Pending, "after the steps above");
    }
    shared.with(|p| {
        p.running = false;
        p.address = address.clone();
    });
    if ready {
        address
    } else {
        None
    }
}

/// The power values put back as they were before the setup changed them.
pub fn restore_power(sys: &dyn System, dir: &Path) -> Result<String, String> {
    let mut saved = Saved::load(dir);
    let Some(before) = saved.power_before.clone() else { return Err("nothing to restore".into()) };
    let pc = sys.find(Tool::Powercfg).ok_or("powercfg is not there")?;
    set_power(sys, &pc, &before)?;
    saved.power_before = None;
    saved.keep_awake = false;
    saved.save(dir);
    Ok("your power settings are as they were".into())
}

/// On each start of the bridge, once set up: what drifted is put right
/// where that needs nobody, and the rest said (for the tray). Nothing that
/// asks the person (a sign-in, Windows' permission) is started here.
pub fn repair(sys: &dyn System, dir: &Path, opts: &Options) -> Vec<String> {
    let mut saved = Saved::load(dir);
    if !saved.completed {
        return Vec::new();
    }
    let mut said = Vec::new();
    match sys.find(Tool::Claude) {
        None => said.push("Claude Code is not installed any more: open the window and Set up".into()),
        Some(exe) => {
            if claude_signed_in(sys, &exe) == Some(false) {
                said.push("Claude Code is signed out: open the window and Set up to sign in".into());
            }
        }
    }
    match sys.find(Tool::Tailscale) {
        None => said.push("Tailscale is not installed any more: open the window and Set up".into()),
        Some(exe) => {
            let mut s = ts_status(sys, &exe);
            if s.as_ref().is_some_and(|x| x.backend == "Stopped") {
                sys.run(&exe, &["up", "--timeout", "60s"], Duration::from_secs(90));
                s = ts_status(sys, &exe);
                if s.as_ref().is_some_and(|x| x.backend == "Running") {
                    said.push("Tailscale was disconnected: connected it again".into());
                }
            }
            match s {
                Some(s) if s.backend == "Running" => {
                    let t = funnel(sys, &exe, &s, opts.bridge_port);
                    if t.address.is_none() {
                        let port = saved.port.filter(|p| !t.taken.contains(p)).or_else(|| tailscale::FUNNEL_PORTS.iter().copied().find(|p| !t.taken.contains(p)));
                        match (s.funnel_allowed, port) {
                            (true, Some(port)) => {
                                let https = format!("--https={port}");
                                let target = opts.bridge_port.to_string();
                                let out = sys.run(&exe, &["funnel", "--bg", "--yes", &https, &target], Duration::from_secs(60));
                                let back = funnel(sys, &exe, &s, opts.bridge_port).address.is_some();
                                said.push(if out.ok() && back { "Funnel was off: turned it back on".into() } else { "Funnel is off and would not start: open the window and Set up".into() });
                                if back {
                                    saved.port = Some(port);
                                }
                            }
                            _ => said.push("Funnel needs allowing again: open the window and Set up".into()),
                        }
                    }
                }
                _ => said.push("Tailscale is signed out: open the window and Set up to sign in".into()),
            }
        }
    }
    if !saved.at_login_declined && !sys.at_login() && sys.set_at_login(true).is_ok() {
        said.push("Start with Windows was off: switched it back on".into());
    }
    if saved.keep_awake {
        if let Some(pc) = sys.find(Tool::Powercfg) {
            if let Some(now) = power_now(sys, &pc) {
                if now != awake(&now) {
                    // Only as this user: Windows' permission prompt is not
                    // raised by itself at sign-in.
                    let want = awake(&now);
                    let quiet_ok = power_commands(&want).iter().all(|c| sys.run(&pc, &c.iter().map(String::as_str).collect::<Vec<_>>(), Duration::from_secs(20)).ok());
                    said.push(if quiet_ok { "the power settings had changed: kept it awake on mains power again".into() } else { "the power settings changed: open the window and Set up to keep it awake".into() });
                }
            }
        }
    }
    saved.save(dir);
    said
}
