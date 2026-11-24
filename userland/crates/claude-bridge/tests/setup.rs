//! The setup against a made-up computer (nothing installed, signed in,
//! published or changed for real): a fresh laptop, one half done, one done
//! already, a sign-in that never comes, Windows' permission refused and
//! given, a code pasted for the sign-in, the power settings put back, and
//! the repairs a later start makes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use catalyst_claude_bridge::setup::{self, Options, Saved, Shared, State, Step};
use catalyst_claude_bridge::system::{Job, Output, System, Tool};
use serde_json::{json, Value};

const SCHEME: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const OURS: &str = "http://127.0.0.1:47318";

/// The made-up computer.
#[derive(Clone)]
struct World {
    t: Duration,
    claude: bool,
    signed: bool,
    /// How long after `claude auth login` starts the person signs in (None: never).
    sign_in_after: Option<Duration>,
    login_at: Option<Duration>,
    /// The sign-in wants a code pasted instead.
    wants_code: bool,
    tailscale: bool,
    backend: String,
    ts_sign_in_after: Option<Duration>,
    ts_login_at: Option<Duration>,
    magic: bool,
    allowed: bool,
    allow_after: Option<Duration>,
    allow_at: Option<Duration>,
    /// Funnel's web handlers: port, proxy, published.
    served: Vec<(u16, String, bool)>,
    /// Sleep, hibernate, lid (AC).
    power: (u32, u32, Option<u32>),
    needs_admin: bool,
    admin_ok: bool,
    elevations: u32,
    at_login: bool,
    winget: bool,
    /// Every command run, and those that change the computer marked `change:`.
    log: Vec<String>,
    opened: Vec<String>,
}

impl World {
    fn fresh() -> World {
        World {
            t: Duration::ZERO,
            claude: false,
            signed: false,
            sign_in_after: Some(Duration::from_secs(8)),
            login_at: None,
            wants_code: false,
            tailscale: false,
            backend: "NoState".into(),
            ts_sign_in_after: Some(Duration::from_secs(8)),
            ts_login_at: None,
            magic: true,
            allowed: false,
            allow_after: Some(Duration::from_secs(10)),
            allow_at: None,
            served: Vec::new(),
            power: (1800, 3600, Some(1)),
            needs_admin: false,
            admin_ok: true,
            elevations: 0,
            at_login: false,
            winget: true,
            log: Vec::new(),
            opened: Vec::new(),
        }
    }

    /// Everything done already.
    fn done() -> World {
        World { claude: true, signed: true, tailscale: true, backend: "Running".into(), allowed: true, served: vec![(10000, OURS.into(), true)], power: (0, 0, Some(0)), at_login: true, ..World::fresh() }
    }

    /// Time passing: sign-ins and approvals that come by themselves.
    fn tick(&mut self) {
        if let (Some(at), Some(after)) = (self.login_at, self.sign_in_after) {
            if self.t >= at + after && !self.wants_code {
                self.signed = true;
            }
        }
        if let (Some(at), Some(after)) = (self.ts_login_at, self.ts_sign_in_after) {
            if self.t >= at + after {
                self.backend = "Running".into();
            }
        }
        if let (Some(at), Some(after)) = (self.allow_at, self.allow_after) {
            if self.t >= at + after {
                self.allowed = true;
            }
        }
    }

    fn changes(&self) -> Vec<&String> {
        self.log.iter().filter(|l| l.starts_with("change:")).collect()
    }
}

#[derive(Clone)]
struct Fake(Arc<Mutex<World>>);

impl Fake {
    fn new(w: World) -> Fake {
        Fake(Arc::new(Mutex::new(w)))
    }

    fn w(&self) -> std::sync::MutexGuard<'_, World> {
        self.0.lock().unwrap()
    }
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

fn out(code: i32, stdout: &str) -> Output {
    Output { code: Some(code), stdout: stdout.into(), stderr: String::new() }
}

fn status_json(w: &World) -> String {
    let caps = if w.allowed { json!({"funnel": [], "https": []}) } else { json!({}) };
    let auth = if w.backend == "NeedsLogin" { "https://login.tailscale.com/a/abc123" } else { "" };
    json!({"BackendState": w.backend, "AuthURL": auth, "CurrentTailnet": {"MagicDNSEnabled": w.magic, "Name": "someone@example.com"}, "User": {"1": {"LoginName": "someone@example.com"}}, "Self": {"DNSName": "laptop.tail1234.ts.net.", "CapMap": caps}}).to_string()
}

fn funnel_json(w: &World) -> String {
    let mut web = serde_json::Map::new();
    let mut allow = serde_json::Map::new();
    for (port, proxy, public) in &w.served {
        let host = format!("laptop.tail1234.ts.net:{port}");
        web.insert(host.clone(), json!({"Handlers": {"/": {"Proxy": proxy}}}));
        if *public {
            allow.insert(host, Value::Bool(true));
        }
    }
    json!({"Web": web, "AllowFunnel": allow}).to_string()
}

/// A funnel command: published if allowed, else the link to allow it.
fn funnel_cmd(w: &mut World, args: &[&str]) -> Output {
    let port: u16 = args.iter().find_map(|a| a.strip_prefix("--https=")).and_then(|p| p.parse().ok()).unwrap_or(443);
    if w.allowed {
        w.log.push(format!("change: funnel on {port}"));
        w.served.retain(|(p, ..)| *p != port);
        w.served.push((port, OURS.into(), true));
        out(0, &format!("Available on the internet:\n\nhttps://laptop.tail1234.ts.net:{port}/\n"))
    } else {
        w.log.push("funnel asked to be allowed".into());
        if w.allow_at.is_none() {
            w.allow_at = Some(w.t);
        }
        out(1, "Funnel is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/funnel?node=n1\n")
    }
}

fn power_line(v: Option<u32>) -> String {
    match v {
        Some(v) => format!("Power Setting GUID: x\n    Current AC Power Setting Index: 0x{v:08x}\n    Current DC Power Setting Index: 0x00000258\n"),
        None => format!("Power Scheme GUID: {SCHEME}  (Balanced)\n"),
    }
}

fn apply_powercfg(w: &mut World, args: &[&str]) {
    if args.first() == Some(&"/setacvalueindex") {
        let v: u32 = args[4].parse().unwrap();
        match args[3] {
            "STANDBYIDLE" => w.power.0 = v,
            "HIBERNATEIDLE" => w.power.1 = v,
            "LIDACTION" => w.power.2 = Some(v),
            _ => {}
        }
        w.log.push(format!("change: powercfg {} {v}", args[3]));
    }
}

enum Kind {
    ClaudeLogin,
    TsLogin,
    Funnel(Output),
}

struct FakeJob {
    world: Arc<Mutex<World>>,
    kind: Kind,
    said: String,
    stopped: bool,
}

impl Job for FakeJob {
    fn said(&self) -> String {
        self.said.clone()
    }

    fn ended(&mut self) -> Option<Option<i32>> {
        let mut w = self.world.lock().unwrap();
        w.tick();
        if self.stopped {
            return Some(None);
        }
        match &self.kind {
            Kind::ClaudeLogin => w.signed.then_some(Some(0)),
            Kind::TsLogin => (w.backend == "Running").then_some(Some(0)),
            Kind::Funnel(o) => Some(o.code),
        }
    }

    fn type_line(&mut self, line: &str) {
        let mut w = self.world.lock().unwrap();
        w.log.push(format!("typed {line}"));
        if matches!(self.kind, Kind::ClaudeLogin) && line == "CODE-42" {
            w.signed = true;
        }
    }

    fn stop(&mut self) {
        self.stopped = true;
        self.world.lock().unwrap().log.push("stopped a job".into());
    }
}

impl System for Fake {
    fn find(&self, tool: Tool) -> Option<PathBuf> {
        let w = self.w();
        let p = |s: &str| Some(PathBuf::from(s));
        match tool {
            Tool::Claude => if w.claude { p("C:\\Users\\U\\.local\\bin\\claude.exe") } else { None },
            Tool::Tailscale => if w.tailscale { p("C:\\Program Files\\Tailscale\\tailscale.exe") } else { None },
            Tool::Winget => if w.winget { p("C:\\Users\\U\\AppData\\Local\\Microsoft\\WindowsApps\\winget.exe") } else { None },
            Tool::PowerShell => p("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            Tool::Powercfg => p("C:\\Windows\\System32\\powercfg.exe"),
            Tool::Cmd => p("C:\\Windows\\System32\\cmd.exe"),
        }
    }

    fn run(&self, program: &Path, args: &[&str], _wait: Duration) -> Output {
        let mut w = self.w();
        w.tick();
        w.log.push(format!("{} {}", name(program), args.join(" ")));
        match (name(program).as_str(), args) {
            ("claude.exe", ["--version"]) => out(0, "2.1.325 (Claude Code)\n"),
            ("claude.exe", ["auth", "status", "--json"]) => out(if w.signed { 0 } else { 1 }, &json!({"loggedIn": w.signed, "authMethod": if w.signed { "claude.ai" } else { "none" }, "email": "someone@example.com"}).to_string()),
            ("powershell.exe", [.., script]) if *script == setup::CLAUDE_INSTALL => {
                w.log.push("change: installed claude".into());
                w.claude = true;
                out(0, "Claude Code successfully installed!\n")
            }
            ("winget.exe", ["install", "--id", "tailscale.tailscale", "-e", ..]) => {
                w.log.push("change: installed tailscale".into());
                w.tailscale = true;
                w.backend = "NeedsLogin".into();
                out(0, "Successfully installed\n")
            }
            ("tailscale.exe", ["status", "--json"]) => out(0, &status_json(&w)),
            ("tailscale.exe", ["funnel", "status", "--json"]) => out(0, &funnel_json(&w)),
            ("tailscale.exe", ["up", ..]) => {
                w.log.push("change: tailscale up".into());
                w.backend = "Running".into();
                out(0, "")
            }
            ("tailscale.exe", ["funnel", ..]) => funnel_cmd(&mut w, args),
            ("powercfg.exe", ["/getactivescheme"]) => out(0, &format!("Power Scheme GUID: {SCHEME}  (Balanced)\n")),
            ("powercfg.exe", ["/qh", _, _, setting]) => {
                let v = match *setting {
                    "STANDBYIDLE" => Some(w.power.0),
                    "HIBERNATEIDLE" => Some(w.power.1),
                    _ => w.power.2,
                };
                out(0, &power_line(v))
            }
            ("powercfg.exe", [first, ..]) if *first == "/setacvalueindex" || *first == "/setactive" => {
                if w.needs_admin {
                    return Output { code: Some(1), stdout: String::new(), stderr: "Access is denied.\n".into() };
                }
                apply_powercfg(&mut w, args);
                out(0, "")
            }
            _ => out(1, "unknown"),
        }
    }

    fn start(&self, program: &Path, args: &[&str], console: bool) -> Option<Box<dyn Job>> {
        let mut w = self.w();
        w.log.push(format!("start {} {}{}", name(program), args.join(" "), if console { " (console)" } else { "" }));
        let (kind, said) = match (name(program).as_str(), args) {
            ("claude.exe", ["auth", "login", ..]) => {
                w.login_at = Some(w.t);
                let paste = if w.wants_code { "\nPaste code here if prompted > " } else { "" };
                (Kind::ClaudeLogin, format!("Opening browser to sign in…\nIf the browser didn't open, visit: https://claude.ai/oauth/authorize?client=x{paste}"))
            }
            ("tailscale.exe", ["login", ..]) => {
                w.ts_login_at = Some(w.t);
                (Kind::TsLogin, "\nTo authenticate, visit:\n\n\thttps://login.tailscale.com/a/abc123\n\n".to_owned())
            }
            ("tailscale.exe", ["funnel", ..]) => {
                let o = funnel_cmd(&mut w, args);
                let said = o.stdout.clone();
                (Kind::Funnel(o), said)
            }
            _ => return None,
        };
        drop(w);
        Some(Box::new(FakeJob { world: self.0.clone(), kind, said, stopped: false }))
    }

    fn run_elevated(&self, program: &Path, args: &str) -> Result<i32, String> {
        let mut w = self.w();
        w.elevations += 1;
        w.log.push(format!("elevated {} {args}", name(program)));
        if !w.admin_ok {
            return Err("the permission was not given".into());
        }
        // Each powercfg command in the line, as cmd would run them.
        let line = args.trim_start_matches("/d /s /c \"").trim_end_matches('"').to_owned();
        for part in line.split(" && ") {
            let words: Vec<&str> = part.split_whitespace().skip(1).collect();
            apply_powercfg(&mut w, &words);
        }
        Ok(0)
    }

    fn open_url(&self, url: &str) {
        self.w().opened.push(url.to_owned());
    }

    fn at_login(&self) -> bool {
        self.w().at_login
    }

    fn set_at_login(&self, on: bool) -> Result<(), String> {
        let mut w = self.w();
        w.log.push(format!("change: start with windows {on}"));
        w.at_login = on;
        Ok(())
    }

    fn now(&self) -> Duration {
        self.w().t
    }

    fn sleep(&self, d: Duration) {
        let mut w = self.w();
        w.t += d;
        w.tick();
    }
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("catalyst-setup-test-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn opts() -> Options {
    Options { bridge_port: 47318, bridge: Ok(()), wait: Duration::from_secs(600), poll: Duration::from_secs(2) }
}

fn states(shared: &Shared) -> Vec<State> {
    shared.snapshot().rows.iter().map(|r| r.state).collect()
}

#[test]
fn a_fresh_laptop_is_set_up_with_only_the_sign_ins_left_to_the_person() {
    let d = dir("fresh");
    let fake = Fake::new(World::fresh());
    let shared = Shared::default();
    let address = setup::run(&fake, &d, &opts(), &shared);
    assert_eq!(address.as_deref(), Some("https://laptop.tail1234.ts.net"), "{:#?}", shared.snapshot());
    assert_eq!(states(&shared), [State::Done, State::Done, State::Done, State::Done, State::Working], "pairing shows last");
    let w = fake.w();
    let changes: Vec<&str> = w.changes().iter().map(|s| s.as_str()).collect();
    assert_eq!(
        changes,
        [
            "change: installed claude",
            "change: installed tailscale",
            "change: funnel on 443",
            "change: start with windows true",
            "change: powercfg STANDBYIDLE 0",
            "change: powercfg HIBERNATEIDLE 0",
            "change: powercfg LIDACTION 0",
        ]
    );
    // The sign-ins: Claude's started (it opens the browser itself), Tailscale's
    // link and Funnel's approval opened for the person.
    assert!(w.log.iter().any(|l| l == "start claude.exe auth login --claudeai"));
    assert_eq!(w.opened, ["https://login.tailscale.com/a/abc123", "https://login.tailscale.com/f/funnel?node=n1"]);
    assert_eq!(w.log.iter().filter(|l| l.starts_with("start tailscale.exe funnel")).count(), 2, "once before its approval and once after");
    assert!(!w.log.iter().any(|l| l.contains("SCHEME_") || l.contains("/change")), "AC values only, by the scheme's id");
    assert!(w.log.iter().all(|l| !l.contains("setdcvalueindex")), "battery never touched");
    let saved = Saved::load(&d);
    assert!(saved.completed && saved.at_login_by_setup);
    assert_eq!(saved.port, Some(443));
    assert_eq!(saved.power_before.as_ref().map(|p| (p.standby, p.hibernate, p.lid)), Some((1800, 3600, Some(1))));
    // Nothing of the account was kept or shown.
    assert!(!std::fs::read_to_string(d.join("setup.json")).unwrap().contains("someone@"));
    assert!(!format!("{:?}", shared.snapshot()).contains("someone@"));
}

#[test]
fn a_half_done_laptop_only_gets_what_is_missing() {
    let d = dir("half");
    let w = World { claude: true, signed: true, tailscale: true, backend: "Running".into(), allowed: true, served: vec![(443, "http://127.0.0.1:8080".into(), true)], power: (0, 0, Some(0)), at_login: true, ..World::fresh() };
    let fake = Fake::new(w);
    let shared = Shared::default();
    let address = setup::run(&fake, &d, &opts(), &shared);
    assert_eq!(address.as_deref(), Some("https://laptop.tail1234.ts.net:8443"), "443 serves something else");
    let w = fake.w();
    assert_eq!(w.changes(), [&"change: funnel on 8443".to_owned()]);
    assert!(w.opened.is_empty() && w.elevations == 0);
    assert!(!w.log.iter().any(|l| l.contains("auth login") || l.contains("install")));
}

#[test]
fn a_laptop_already_set_up_is_left_alone_however_often_it_runs() {
    let d = dir("done");
    let fake = Fake::new(World::done());
    for _ in 0..3 {
        let shared = Shared::default();
        assert_eq!(setup::run(&fake, &d, &opts(), &shared).as_deref(), Some("https://laptop.tail1234.ts.net:10000"));
        assert_eq!(states(&shared)[..4], [State::Done; 4]);
    }
    assert!(fake.w().changes().is_empty(), "{:?}", fake.w().log);
    assert_eq!(Saved::load(&d).port, Some(10000), "the port in use remembered");
    assert!(Saved::load(&d).power_before.is_none(), "nothing changed, nothing to restore");
}

#[test]
fn a_sign_in_that_never_comes_stops_in_time_with_a_fix() {
    let d = dir("timeout");
    let w = World { claude: true, sign_in_after: None, ..World::done() };
    let w = World { signed: false, ..w };
    let fake = Fake::new(w);
    let shared = Shared::default();
    let o = opts();
    let began = fake.now();
    assert_eq!(setup::run(&fake, &d, &o, &shared), None);
    let p = shared.snapshot();
    let claude = p.row(Step::Claude);
    assert_eq!(claude.state, State::Failed);
    assert!(claude.words.contains("ten minutes") && claude.fix.contains("Set up again"), "{claude:?}");
    assert_eq!(p.row(Step::Pair).state, State::Pending, "no code shown");
    let w = fake.w();
    assert!(w.log.iter().any(|l| l == "stopped a job"), "the sign-in let go");
    assert!(w.t - began >= o.wait && w.t - began < o.wait + Duration::from_secs(30));
    assert_eq!(w.log.iter().filter(|l| l.contains("auth login")).count(), 1);
}

#[test]
fn a_code_pasted_in_the_window_finishes_the_sign_in() {
    let d = dir("code");
    let w = World { signed: false, wants_code: true, ..World::done() };
    let fake = Fake::new(w);
    let shared = Shared::default();
    *shared.pasted.lock().unwrap() = Some(" CODE-42 ".into());
    setup::run(&fake, &d, &opts(), &shared);
    assert_eq!(shared.snapshot().row(Step::Claude).state, State::Done);
    assert!(fake.w().log.iter().any(|l| l == "typed CODE-42"));
}

#[test]
fn windows_refusing_permission_fails_the_power_step_only_and_asks_once() {
    let d = dir("refused");
    let w = World { power: (1800, 3600, Some(1)), needs_admin: true, admin_ok: false, ..World::done() };
    let fake = Fake::new(w);
    let shared = Shared::default();
    assert_eq!(setup::run(&fake, &d, &opts(), &shared), None);
    let p = shared.snapshot();
    assert_eq!(p.row(Step::Power).state, State::Failed);
    assert!(p.row(Step::Power).fix.contains("choose Yes") && p.row(Step::Power).fix.contains("untick"), "{:?}", p.row(Step::Power));
    assert_eq!(p.row(Step::Claude).state, State::Done);
    let w = fake.w();
    assert_eq!(w.elevations, 1, "asked once");
    assert_eq!(w.power, (1800, 3600, Some(1)), "unchanged");
    // The elevated line runs every command, quoted for cmd.
    let line = w.log.iter().find(|l| l.starts_with("elevated")).unwrap();
    assert!(line.starts_with("elevated cmd.exe /d /s /c \"\"C:\\Windows\\System32\\powercfg.exe\" /setacvalueindex"), "{line}");
    assert!(line.ends_with(&format!("/setactive {SCHEME}\"")), "{line}");
}

#[test]
fn windows_asked_once_for_permission_sets_them_all() {
    let d = dir("allowed");
    let w = World { power: (1800, 3600, None), needs_admin: true, admin_ok: true, ..World::done() };
    let fake = Fake::new(w);
    let shared = Shared::default();
    assert!(setup::run(&fake, &d, &opts(), &shared).is_some());
    let w = fake.w();
    assert_eq!((w.elevations, w.power), (1, (0, 0, None)), "no lid: none set");
}

#[test]
fn keeping_awake_unticked_leaves_the_power_settings_alone() {
    let d = dir("untick");
    Saved { keep_awake: false, ..Saved::default() }.save(&d);
    let w = World { power: (1800, 3600, Some(1)), ..World::done() };
    let fake = Fake::new(w);
    let shared = Shared::default();
    assert!(setup::run(&fake, &d, &opts(), &shared).is_some());
    assert_eq!(shared.snapshot().row(Step::Power).state, State::Skipped);
    assert!(fake.w().changes().is_empty());
}

#[test]
fn the_power_settings_go_back_as_they_were() {
    let d = dir("restore");
    let fake = Fake::new(World { power: (1800, 3600, Some(1)), ..World::done() });
    setup::run(&fake, &d, &opts(), &Shared::default());
    assert_eq!(fake.w().power, (0, 0, Some(0)));
    // Run again: the values kept are still the first ones, not its own.
    setup::run(&fake, &d, &opts(), &Shared::default());
    assert_eq!(Saved::load(&d).power_before.map(|p| (p.standby, p.hibernate, p.lid)), Some((1800, 3600, Some(1))));
    setup::restore_power(&fake, &d).unwrap();
    assert_eq!(fake.w().power, (1800, 3600, Some(1)));
    let saved = Saved::load(&d);
    assert!(saved.power_before.is_none() && !saved.keep_awake, "and not kept awake again by itself");
    assert!(setup::restore_power(&fake, &d).is_err(), "nothing more to restore");
}

#[test]
fn a_later_start_repairs_what_drifted_and_says_what_needs_the_person() {
    let d = dir("repair");
    let fake = Fake::new(World::done());
    setup::run(&fake, &d, &opts(), &Shared::default());
    // A Tailscale update turned Funnel off; Claude Code signed out; Start
    // with Windows switched off by something else.
    {
        let mut w = fake.w();
        w.served.clear();
        w.signed = false;
        w.at_login = false;
        w.log.clear();
    }
    let said = setup::repair(&fake, &d, &opts());
    assert!(said.iter().any(|s| s.starts_with("Claude Code is signed out")), "{said:?}");
    assert!(said.contains(&"Funnel was off: turned it back on".to_owned()), "{said:?}");
    assert!(said.contains(&"Start with Windows was off: switched it back on".to_owned()), "{said:?}");
    let w = fake.w();
    assert!(!w.log.iter().any(|l| l.contains("auth login") || l.starts_with("elevated")), "nothing that asks the person");
    assert_eq!(w.served, [(10000, OURS.to_owned(), true)], "on the remembered port");
    drop(w);
    // Switched off by the person in the window: left off.
    let mut saved = Saved::load(&d);
    saved.at_login_declined = true;
    saved.save(&d);
    fake.w().at_login = false;
    assert!(!setup::repair(&fake, &d, &opts()).iter().any(|s| s.contains("Start with Windows")));
    // Not set up yet: nothing looked at.
    let fresh = Fake::new(World::fresh());
    assert!(setup::repair(&fresh, &dir("repair-none"), &opts()).is_empty());
    assert!(fresh.w().log.is_empty());
}

#[test]
fn looking_changes_nothing() {
    let fake = Fake::new(World { power: (1800, 3600, Some(1)), served: vec![], ..World::done() });
    let p = setup::look(&fake, &Saved::default(), &opts());
    assert_eq!(p.row(Step::Claude).state, State::Done);
    assert_eq!(p.row(Step::Tailscale).state, State::Pending);
    assert_eq!(p.row(Step::Power).state, State::Pending);
    assert!(fake.w().changes().is_empty() && fake.w().opened.is_empty());
    assert_eq!(setup::first_url("visit:\n\n\thttps://login.tailscale.com/a/abc.\n"), Some("https://login.tailscale.com/a/abc".into()));
}

/// How this computer stands, looked at only: `claude --version`, `claude
/// auth status`, `tailscale status`, `tailscale funnel status`, `powercfg
/// /getactivescheme` and `/qh`, and the Run key read. Nothing is changed.
/// `cargo test -p catalyst-claude-bridge --test setup -- --ignored --nocapture`
#[test]
#[ignore = "looks at this computer's own Claude Code, Tailscale and power settings (read only)"]
fn looking_at_this_computer_reads_it() {
    let sys = catalyst_claude_bridge::system::Real::new("unused".into());
    let p = setup::look(&sys, &Saved::default(), &opts());
    for r in &p.rows {
        println!("{:<22} {:?} · {}", r.step.name(), r.state, r.words);
    }
    println!("address known: {}", p.address.is_some());
}
