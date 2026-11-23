//! The window: finding the handheld and pairing, then five pages on a rail
//! (Sign-ins, Music, Files, Settings, System), each drawn and handled in
//! one pass a frame. Everything that waits on the network runs on a thread
//! of its own; the window only ever draws.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect, TextStyle};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::jobs::{Job, Progress};
use crate::net::{self, Found, Paired, Send, Sink, Target};
use crate::store::{self, Saved};
use crate::ui::{size, Kind, Ui, CRIMSON, GOOD, H, K, W};
use crate::{backup, github, win, youtube};

const RAIL: f32 = 232.0;
const X0: f32 = 264.0;
const CW: f32 = W as f32 - X0 - 32.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    SignIns,
    Music,
    Files,
    Settings,
    System,
}

const PAGES: [(Page, &str, &str); 5] = [(Page::SignIns, "Sign-ins", "key"), (Page::Music, "Music", "library_music"), (Page::Files, "Files", "folder"), (Page::Settings, "Settings", "tune"), (Page::System, "System", "sync")];

/// The places Files shows, and what the handheld calls them.
pub const AREAS: [(&str, &str, &str); 7] = [("logs", "Match logs", "insights"), ("photos", "Photos", "photo"), ("screenshots", "Screenshots", "photo_library"), ("notes", "Notes", "description"), ("batteries", "Batteries", "battery_full"), ("strategy", "Strategy", "photo"), ("scans", "Scans", "description")];

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Looking for handhelds, none paired here (or the one paired chosen against).
    Find,
    /// Typing the code shown on this one.
    Pair(Found),
    Main,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub modified: i64,
}

/// A folder of the handheld's, as listed.
pub struct Browser {
    pub area: String,
    pub path: String,
    pub entries: Option<Result<Vec<Entry>, String>>,
    listing: Job<Result<Vec<Entry>, String>>,
    scroll: Animated,
}

impl Browser {
    fn new(area: &str) -> Self {
        Browser { area: area.into(), path: String::new(), entries: None, listing: Job::new(), scroll: Animated::new(0.0, role::settle()).with_rest(0.5) }
    }

    fn list(&mut self, target: &Target) {
        let (t, area, path) = (target.clone(), self.area.clone(), self.path.clone());
        self.listing.start(move || {
            let v = t.get(&format!("/v1/files?area={}&path={}", net::encode(&area), net::encode(&path)))?;
            Ok(v["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| Entry { name: e["name"].as_str().unwrap_or_default().to_owned(), dir: e["dir"].as_bool().unwrap_or(false), size: e["size"].as_u64().unwrap_or(0), modified: e["modified"].as_i64().unwrap_or(0) })
                .collect())
        });
    }

    fn go(&mut self, target: &Target, path: String) {
        self.path = path;
        self.entries = None;
        self.scroll.jump(0.0);
        self.list(target);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sent {
    Waiting,
    Sending,
    Done(String),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Upload {
    pub path: PathBuf,
    pub size: u64,
    pub state: Sent,
    pub progress: Progress,
}

/// GitHub's card: what it is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum Gh {
    Idle,
    /// Asked: read this computer's sign-in?
    Consent,
    Paste(String),
    Code(github::Device),
}

/// Settings, as the page edits them (what is sent is what changed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub team: String,
    pub ssid: String,
    pub psk: String,
    pub keys: [String; 4],
    pub accent: usize,
    pub light: bool,
    pub text: usize,
    pub home: usize,
    pub choices: [bool; 4],
}

const KEYS: [(&str, &str); 4] = [("tba", "The Blue Alliance"), ("nexus", "FRC Nexus"), ("openai", "OpenAI"), ("claude", "Claude")];
const CHOICES: [(&str, &str); 4] = [("open_robot", "Open Robot on link"), ("match_report", "Match report after a match"), ("ssh", "SSH"), ("calm", "Calm: fewer animations")];
const TEXTS: [&str; 3] = ["compact", "standard", "large"];
const HOMES: [&str; 3] = ["grid", "carousel", "flow"];
const ACCENTS: [(&str, u32); 5] = [("orange", 0xff8a3d), ("blue", 0x4c8dff), ("green", 0x3ccf7e), ("violet", 0x9a7bff), ("rose", 0xe94560)];

impl Form {
    fn from_now(now: &Value) -> Form {
        let at = |list: &[&str], v: &Value| list.iter().position(|x| Some(*x) == v.as_str()).unwrap_or(0);
        Form {
            team: now["team"].as_str().unwrap_or_default().to_owned(),
            accent: ACCENTS.iter().position(|(n, _)| Some(*n) == now["accent"].as_str()).unwrap_or(4),
            light: now["mode"] == "light",
            text: at(&TEXTS, &now["text"]).max(usize::from(now["text"].is_null())),
            home: at(&HOMES, &now["home"]),
            choices: CHOICES.map(|(k, _)| now[k].as_bool().unwrap_or(false)),
            ..Form::default()
        }
    }

    /// The fields that changed from `was`, as the handheld's setup page names them.
    fn changes(&self, was: &Form) -> Value {
        let mut v = json!({});
        if self.team.trim() != was.team && !self.team.trim().is_empty() {
            v["team"] = json!(self.team.trim());
        }
        if !self.ssid.trim().is_empty() {
            v["ssid1"] = json!(self.ssid);
            if !self.psk.is_empty() {
                v["psk1"] = json!(self.psk);
            }
        }
        for (i, (name, _)) in KEYS.iter().enumerate() {
            if !self.keys[i].trim().is_empty() {
                v[*name] = json!(self.keys[i].trim());
            }
        }
        if self.accent != was.accent {
            v["accent"] = json!(ACCENTS[self.accent].0);
        }
        if self.light != was.light {
            v["mode"] = json!(if self.light { "light" } else { "dark" });
        }
        if self.text != was.text {
            v["text"] = json!(TEXTS[self.text]);
        }
        if self.home != was.home {
            v["home"] = json!(HOMES[self.home]);
        }
        if self.choices != was.choices {
            v["choices"] = json!("1");
            for (i, (name, _)) in CHOICES.iter().enumerate() {
                v[*name] = json!(self.choices[i]);
            }
        }
        v
    }
}

pub struct App {
    pub phase: Phase,
    pub page: Page,
    shown: Animated,
    nav: Animated,
    pub found: Vec<Found>,
    finding: Job<Vec<Found>>,
    found_at: Option<Instant>,
    pub current: Option<Saved>,
    /// Reached the last time it was asked; by the USB cable.
    pub online: Option<bool>,
    pub usb: bool,
    connecting: Job<Result<(Saved, bool), String>>,
    pub now: Option<Value>,
    pub update: Option<Value>,
    refreshing: Job<Result<(Value, Value), String>>,
    refreshed_at: Option<Instant>,
    pub code: String,
    pairing: Job<Result<Paired, String>>,
    pub pair_error: Option<String>,
    pub toast: Option<(String, bool, Instant)>,
    yt: Job<Result<String, String>>,
    pub gh: Gh,
    gh_job: Job<Result<String, String>>,
    gh_device: Job<Result<github::Device, String>>,
    gh_poll: Job<Result<Option<String>, String>>,
    gh_polled: Instant,
    pub uploads: Arc<Mutex<Vec<Upload>>>,
    uploading: Job<()>,
    /// When the last songs finished going.
    sent_at: Option<Instant>,
    pub lib: Browser,
    pub files: Browser,
    pub area: usize,
    download: Job<Result<PathBuf, String>>,
    download_progress: Progress,
    pub form: Form,
    form_was: Option<Form>,
    sending: Job<Result<Value, String>>,
    pub sent: Option<Result<String, String>>,
    upd: Job<Result<Value, String>>,
    backing: Job<Result<PathBuf, String>>,
    pub backup_progress: Progress,
    pub last_backup: Option<PathBuf>,
    pub restore: Option<(PathBuf, usize, u64)>,
    restoring: Job<Result<Value, String>>,
    pub restore_progress: Progress,
    pub forget_armed: bool,
    /// The handhelds paired here, and the one chosen among those found.
    known: Vec<Saved>,
    pick: usize,
    /// When each handheld found last answered.
    seen: std::collections::HashMap<String, Instant>,
    pub window: usize,
    born: Instant,
}

fn when(modified: i64) -> String {
    let days = modified.div_euclid(86_400);
    let (y, m, d) = civil(days);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {d}, {y}", MONTHS[(m - 1) as usize])
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn is_song(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    ["mp3", "m4a", "flac", "opus", "ogg", "aac", "wav"].contains(&ext.as_str())
}

impl App {
    pub fn new() -> App {
        let saved = store::load();
        let current = saved.first().cloned();
        let mut app = App {
            phase: if current.is_some() { Phase::Main } else { Phase::Find },
            page: Page::SignIns,
            shown: Animated::new(1.0, role::release()).with_rest(0.002),
            nav: Animated::new(0.0, role::detent()).with_rest(0.05),
            found: Vec::new(),
            finding: Job::new(),
            found_at: None,
            current,
            online: None,
            usb: false,
            connecting: Job::new(),
            now: None,
            update: None,
            refreshing: Job::new(),
            refreshed_at: None,
            code: String::new(),
            pairing: Job::new(),
            pair_error: None,
            toast: None,
            yt: Job::new(),
            gh: Gh::Idle,
            gh_job: Job::new(),
            gh_device: Job::new(),
            gh_poll: Job::new(),
            gh_polled: Instant::now(),
            uploads: Arc::new(Mutex::new(Vec::new())),
            uploading: Job::new(),
            sent_at: None,
            lib: Browser::new("music"),
            files: Browser::new(AREAS[0].0),
            area: 0,
            download: Job::new(),
            download_progress: Progress::default(),
            form: Form::default(),
            form_was: None,
            sending: Job::new(),
            sent: None,
            upd: Job::new(),
            backing: Job::new(),
            backup_progress: Progress::default(),
            last_backup: None,
            restore: None,
            restoring: Job::new(),
            restore_progress: Progress::default(),
            forget_armed: false,
            known: saved.clone(),
            pick: 0,
            seen: Default::default(),
            window: 0,
            born: Instant::now(),
        };
        app.find();
        app
    }

    /// The app as its shots show it: paired with a handheld that answered.
    pub fn for_shots(now: Value, update: Value) -> App {
        let mut app = App::new();
        app.phase = Phase::Main;
        app.current = Some(Saved { id: "c0ffee00c0ffee00".into(), name: "catalyst".into(), version: "0.7.0".into(), target: Target { host: "192.168.1.42".into(), port: 47315, pin: [0; 32], token: String::new() } });
        app.online = Some(true);
        app.form = Form::from_now(&now);
        app.form_was = Some(app.form.clone());
        app.now = Some(now);
        app.update = Some(update);
        app.found_at = Some(Instant::now());
        app
    }

    pub fn say(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some((text.into(), error, Instant::now()));
    }

    fn target(&self) -> Option<Target> {
        self.current.as_ref().map(|s| s.target.clone())
    }

    fn name(&self) -> String {
        self.current.as_ref().map_or_else(|| "the handheld".into(), |s| s.name.clone())
    }

    pub fn find(&mut self) {
        self.finding.start(|| net::discover(Duration::from_millis(1400)));
    }

    /// The saved handheld reached: where it answered on the network (its
    /// address may have changed), else where it was last.
    fn connect(&mut self) {
        let Some(saved) = self.current.clone() else { return };
        let found = self.found.iter().find(|f| f.id == saved.id).cloned();
        self.connecting.start(move || {
            let mut s = saved;
            let usb = found.as_ref().is_some_and(|f| f.usb);
            if let Some(f) = found {
                s.target.host = f.host;
                s.target.port = f.port;
                s.version = f.version;
            }
            let hello = s.target.get("/v1/hello")?;
            if hello["paired"] != true {
                return Err("the handheld no longer knows this computer: pair it again".into());
            }
            s.name = hello["name"].as_str().unwrap_or(&s.name).to_owned();
            s.version = hello["version"].as_str().unwrap_or(&s.version).to_owned();
            store::remember(s.clone()).ok();
            let usb = usb || s.target.host == catalyst_companion_proto::USB_ADDRESS;
            Ok((s, usb))
        });
    }

    fn refresh(&mut self) {
        let Some(t) = self.target() else { return };
        self.refreshed_at = Some(Instant::now());
        self.refreshing.start(move || Ok((t.get("/v1/settings")?, t.get("/v1/update")?)));
    }

    fn go(&mut self, page: Page) {
        if page == self.page {
            return;
        }
        self.page = page;
        self.shown.jump(0.0);
        self.shown.set(1.0);
        self.forget_armed = false;
        if let Some(t) = self.target() {
            match page {
                Page::Music if self.lib.entries.is_none() => self.lib.list(&t),
                Page::Files if self.files.entries.is_none() => self.files.list(&t),
                Page::Settings | Page::System | Page::SignIns => self.refresh(),
                _ => {}
            }
        }
    }

    /// Files dropped on the window: songs to the library, a zip to restore.
    pub fn dropped(&mut self, paths: Vec<PathBuf>) {
        let (songs, rest): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| is_song(p));
        if !songs.is_empty() {
            self.add_songs(songs);
            self.go(Page::Music);
        }
        if let Some(zip) = rest.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))) {
            self.choose_restore(zip.clone());
            self.go(Page::System);
        } else if !rest.is_empty() {
            self.say("Music takes MP3, M4A, FLAC and Opus; a backup is a .zip", true);
        }
    }

    pub fn add_songs(&mut self, paths: Vec<PathBuf>) {
        {
            let mut q = self.uploads.lock().unwrap_or_else(|e| e.into_inner());
            // What was sent a while ago makes way; songs dropped one after another stay listed together.
            if self.sent_at.is_some_and(|t| t.elapsed() > Duration::from_secs(20)) {
                q.retain(|u| !matches!(u.state, Sent::Done(_)));
            }
            for p in paths.into_iter().filter(|p| is_song(p)) {
                let size = p.metadata().map_or(0, |m| m.len());
                q.push(Upload { path: p, size, state: Sent::Waiting, progress: Progress::default() });
            }
        }
        let Some(t) = self.target() else { return };
        let q = self.uploads.clone();
        self.uploading.start(move || loop {
            let next = q.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|u| u.state == Sent::Waiting).map(|u| {
                u.state = Sent::Sending;
                u.clone()
            });
            let Some(u) = next else { break };
            let name = u.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let result = std::fs::File::open(&u.path).map_err(|e| format!("could not read it: {e}")).and_then(|mut f| {
                let total = u.size.max(1) as f32;
                let progress = u.progress.clone();
                let mut tell = |n: u64| progress.set(n as f32 / total);
                let answer = t.call("PUT", &format!("/v1/music?name={}", net::encode(&name)), Send::Stream { len: u.size, from: &mut f, progress: &mut tell }, Sink::Memory, Duration::from_secs(120))?;
                answer.ok()
            });
            let state = match result {
                Ok(v) => Sent::Done(v["saved"].as_str().unwrap_or_default().to_owned()),
                Err(why) => Sent::Failed(why),
            };
            if let Some(x) = q.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|x| x.path == u.path && x.state == Sent::Sending) {
                x.state = state;
            }
        });
    }

    fn choose_restore(&mut self, zip: PathBuf) {
        match backup::Restore::open(&zip) {
            Ok(r) => self.restore = Some((zip, r.files, r.len)),
            Err(why) => self.say(why, true),
        }
    }

    /// Results of work done away from the window, taken in.
    pub fn poll(&mut self) {
        if let Some(found) = self.finding.take() {
            // A handheld missed by one look (a packet lost on Wi-Fi) stays listed a while.
            let now = Instant::now();
            for f in found {
                self.seen.insert(f.id.clone(), now);
                match self.found.iter_mut().find(|x| x.id == f.id) {
                    Some(x) => *x = f,
                    None => self.found.push(f),
                }
            }
            self.found.retain(|f| self.seen.get(&f.id).is_some_and(|t| t.elapsed() < Duration::from_secs(12)));
            self.known = store::load();
            self.found_at = Some(Instant::now());
            if self.phase == Phase::Main && self.online != Some(true) {
                self.connect();
            }
        }
        if let Some(r) = self.connecting.take() {
            match r {
                Ok((s, usb)) => {
                    self.current = Some(s);
                    self.online = Some(true);
                    self.usb = usb;
                    self.refresh();
                    if let Some(t) = self.target() {
                        self.lib.list(&t);
                        self.files.list(&t);
                    }
                }
                Err(why) => {
                    self.online = Some(false);
                    if why.contains("pair") {
                        self.say(why, true);
                    }
                }
            }
        }
        if let Some(r) = self.refreshing.take() {
            match r {
                Ok((now, update)) => {
                    let fresh = Form::from_now(&now);
                    // What was being typed stays; the rest follows the handheld.
                    if self.form_was.as_ref() != Some(&fresh) {
                        let typed = (self.form.ssid.clone(), self.form.psk.clone(), self.form.keys.clone());
                        self.form = fresh.clone();
                        (self.form.ssid, self.form.psk, self.form.keys) = typed;
                        self.form_was = Some(fresh);
                    }
                    self.now = Some(now);
                    self.update = Some(update);
                    self.online = Some(true);
                }
                Err(why) => {
                    self.online = Some(false);
                    self.say(why, true);
                }
            }
        }
        if let Some(r) = self.pairing.take() {
            match r {
                Ok(p) => {
                    let saved = Saved { id: p.id, name: p.name, version: p.version, target: p.target };
                    if let Err(why) = store::remember(saved.clone()) {
                        self.say(why, true);
                    }
                    self.say(format!("Paired with {}", saved.name), false);
                    self.current = Some(saved);
                    self.phase = Phase::Main;
                    self.online = None;
                    self.code.clear();
                    self.connect();
                }
                Err(why) => self.pair_error = Some(why),
            }
        }
        if let Some(r) = self.yt.take() {
            match r {
                Ok(m) => self.say(m, false),
                Err(why) => self.say(why, true),
            }
            self.refresh();
        }
        if let Some(r) = self.gh_job.take() {
            match r {
                Ok(m) => {
                    self.gh = Gh::Idle;
                    self.say(m, false);
                }
                Err(why) => self.say(why, true),
            }
            self.refresh();
        }
        if let Some(r) = self.gh_device.take() {
            match r {
                Ok(d) => {
                    win::open_url(&d.verification_uri);
                    self.gh = Gh::Code(d);
                    self.gh_polled = Instant::now();
                }
                Err(why) => self.say(why, true),
            }
        }
        if let Some(r) = self.gh_poll.take() {
            match r {
                Ok(Some(token)) => self.send_github(token, "GitHub"),
                Ok(None) => {}
                Err(why) => {
                    self.gh = Gh::Idle;
                    self.say(why, true);
                }
            }
        }
        if let Gh::Code(d) = &self.gh {
            if self.gh_polled.elapsed() >= d.interval && !self.gh_poll.busy() && !self.gh_job.busy() {
                self.gh_polled = Instant::now();
                let (mut d, id) = (d.clone(), github::client_id().unwrap_or_default());
                self.gh_poll.start(move || github::device_poll(&id, &mut d));
            }
        }
        // Songs sent: the library shows them where they landed.
        if self.uploading.take().is_some() {
            self.sent_at = Some(Instant::now());
            if let Some(t) = self.target() {
                self.lib.list(&t);
            }
        }
        if let Some(r) = self.lib.listing.take() {
            self.lib.entries = Some(r);
        }
        if let Some(r) = self.files.listing.take() {
            self.files.entries = Some(r);
        }
        if let Some(r) = self.download.take() {
            match r {
                Ok(p) => {
                    let folder = p.parent().and_then(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    self.say(format!("Saved {} to Downloads › Catalyst › {folder}", p.file_name().unwrap_or_default().to_string_lossy()), false);
                }
                Err(why) => self.say(why, true),
            }
        }
        if let Some(r) = self.sending.take() {
            self.sent = Some(r.map(|v| {
                let rows: Vec<String> = v["received"].as_array().into_iter().flatten().map(|r| format!("{} {}", r[0].as_str().unwrap_or_default(), r[1].as_str().unwrap_or_default()).trim().to_owned()).collect();
                rows.join(" · ")
            }));
            if self.sent.as_ref().is_some_and(Result::is_ok) {
                self.form.ssid.clear();
                self.form.psk.clear();
                self.form.keys = Default::default();
            }
            self.refresh();
        }
        if let Some(r) = self.upd.take() {
            match r {
                Ok(v) => self.update = Some(v),
                Err(why) => self.say(why, true),
            }
        }
        if let Some(r) = self.backing.take() {
            match r {
                Ok(p) => {
                    self.say(format!("Backed up to {}", p.file_name().unwrap_or_default().to_string_lossy()), false);
                    self.last_backup = Some(p);
                }
                Err(why) => self.say(why, true),
            }
        }
        if let Some(r) = self.restoring.take() {
            match r {
                Ok(v) => {
                    self.restore = None;
                    let n = v["restored"].as_u64().unwrap_or(0);
                    self.say(format!("Restored {n} files{}", if v["restarting"] == true { ": the handheld is restarting" } else { "" }), false);
                    self.online = None;
                }
                Err(why) => self.say(why, true),
            }
        }
        // A handheld not reached is looked for again now and then.
        let stale = self.found_at.is_none_or(|t| t.elapsed() > Duration::from_secs(if self.phase == Phase::Main { 8 } else { 3 }));
        if stale && (self.phase == Phase::Find || self.online != Some(true)) && !self.connecting.busy() {
            self.find();
        }
        // While an update moves, how far it has got.
        let moving = self.update.as_ref().is_some_and(|u| ["checking", "downloading", "installing"].contains(&u["status"]["state"].as_str().unwrap_or_default()));
        if self.online == Some(true) && self.page == Page::System && moving && self.refreshed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.refresh();
        }
    }

    fn send_github(&mut self, token: String, from: &'static str) {
        let Some(t) = self.target() else { return };
        let name = self.name();
        self.gh_job.start(move || {
            let login = github::whose(&token)?;
            t.post("/v1/github", json!({ "token": token }))?;
            Ok(format!("{name} signed in to GitHub as @{login} ({from})"))
        });
    }

    // ------------------------------------------------------------ drawing ---

    pub fn frame(&mut self, ui: &mut Ui) {
        let dt = 1.0 / 60.0;
        self.shown.tick(dt);
        self.nav.tick(dt);
        self.lib.scroll.tick(dt);
        self.files.scroll.tick(dt);
        crate::ui::ground(ui.canvas, &ui.t);
        match self.phase.clone() {
            Phase::Find => self.draw_find(ui),
            Phase::Pair(f) => self.draw_pair(ui, &f),
            Phase::Main => {
                self.draw_rail(ui);
                // The page arrives: its title rises a little as it fades in.
                let a = self.shown.value().clamp(0.0, 1.0);
                match self.page {
                    Page::SignIns => self.page_signins(ui),
                    Page::Music => self.page_music(ui),
                    Page::Files => self.page_files(ui),
                    Page::Settings => self.page_settings(ui),
                    Page::System => self.page_system(ui),
                }
                if a < 1.0 {
                    // Fading in over the ground: the page's last frames before rest.
                    let veil = Rect::new(RAIL, 0.0, W as f32 - RAIL, H as f32);
                    ui.canvas.fill_rect(veil, ui.t.ground, (1.0 - a) * 0.85);
                }
            }
        }
        self.draw_toast(ui);
    }

    fn header(&self, ui: &mut Ui, title: &str, under: &str) {
        let t = ui.t;
        let rise = (1.0 - self.shown.value().clamp(0.0, 1.0)) * 10.0;
        ui.put(X0, 52.0 + rise, title, TextStyle { tracking: -0.01, ..TextStyle::sans(26.0, 680.0) }, t.ink, Align::Left);
        ui.put_fit(X0, 82.0 + rise, under, TextStyle::body(14.0), t.dim, CW);
    }

    fn draw_toast(&mut self, ui: &mut Ui) {
        let Some((text, error, at)) = self.toast.clone() else { return };
        let age = at.elapsed().as_secs_f32();
        let life = if error { 7.0 } else { 4.5 };
        if age > life {
            self.toast = None;
            return;
        }
        let alpha = (age * 6.0).min(1.0).min((life - age) * 3.0).clamp(0.0, 1.0);
        let style = TextStyle::sans(14.0, 550.0);
        let max = W as f32 - RAIL - 80.0;
        let text = ui.text.fit(&text, &style, max);
        let w = ui.measure(&text, style) + 48.0;
        let cx = if self.phase == Phase::Main { RAIL + (W as f32 - RAIL) * 0.5 } else { W as f32 * 0.5 };
        let r = Rect::new(cx - w * 0.5, H as f32 - 64.0 + (1.0 - alpha) * 8.0, w, 40.0);
        ui.canvas.rrect(r.offset(0.0, 3.0), 20.0, Color::BLACK, 0.3 * alpha);
        ui.canvas.rrect(r, 20.0, ui.t.surface3, alpha);
        ui.dot(r.x + 18.0, r.cy(), if error { CRIMSON } else { GOOD });
        ui.put_alpha(r.x + 30.0, r.cy(), &text, style, ui.t.ink, Align::Left, alpha);
    }

    fn draw_rail(&mut self, ui: &mut Ui) {
        let t = ui.t;
        ui.canvas.fill_rect(Rect::new(0.0, 0.0, RAIL, H as f32), t.surface1, 0.55);
        crate::ui::bolt(ui.canvas, 36.0, 44.0, 26.0);
        let w = ui.put(54.0, 44.0, "Catalyst", TextStyle::sans(18.0, 700.0), t.ink, Align::Left);
        ui.put(58.0 + w, 44.0, "Companion", TextStyle::sans(18.0, 300.0), t.dim, Align::Left);
        // The handheld: its name and how it is reached.
        let card = Rect::new(16.0, 78.0, RAIL - 32.0, 70.0);
        // The card itself leads back to finding a handheld (another one, or this one again).
        if ui.over(card) {
            ui.canvas.rrect(card.inset(-2.0), 16.0, t.ink, 0.04);
        }
        if ui.hit(card) {
            self.phase = Phase::Find;
            self.find();
        }
        ui.canvas.rrect(card, 14.0, t.surface2, 1.0);
        let name = self.name();
        ui.put_fit(card.x + 14.0, card.y + 24.0, &name, TextStyle::sans(15.0, 650.0), t.ink, card.w - 28.0);
        let (dot, line) = match (self.online, &self.current) {
            (Some(true), Some(s)) if self.usb => (GOOD, format!("USB cable · {}", s.version)),
            (Some(true), Some(s)) => (GOOD, format!("Wi-Fi · {}", s.target.host)),
            (Some(false), _) => (CRIMSON, "not found".to_owned()),
            _ => (t.faint, "looking…".to_owned()),
        };
        ui.dot(card.x + 18.0, card.y + 48.0, dot);
        ui.put_fit(card.x + 30.0, card.y + 48.0, &line, TextStyle::label(12.0), t.dim, card.w - 44.0);
        // The pages, with the pill gliding to the one open.
        let top = 172.0;
        let at = PAGES.iter().position(|(p, _, _)| *p == self.page).unwrap_or(0) as f32;
        if self.nav.target() != at {
            self.nav.set(at);
        }
        let pill = Rect::new(12.0, top + self.nav.value() * 46.0, RAIL - 24.0, 40.0);
        ui.canvas.rrect(pill, 20.0, t.surface3, 1.0);
        for (i, (page, label, icon)) in PAGES.iter().enumerate() {
            let r = Rect::new(12.0, top + i as f32 * 46.0, RAIL - 24.0, 40.0);
            let on = *page == self.page;
            if !on && ui.over(r) {
                ui.canvas.rrect(r, 20.0, t.ink, 0.05);
            }
            ui.icon(icon, r.x + 26.0, r.cy(), 20.0, if on { CRIMSON } else { t.dim });
            ui.put(r.x + 48.0, r.cy(), label, TextStyle::sans(15.0, if on { 650.0 } else { 500.0 }), if on { t.ink } else { t.dim }, Align::Left);
            if ui.hit(r) {
                self.go(*page);
            }
        }
        let version = format!("Catalyst Companion {}", include_str!("../../../../os/VERSION").trim());
        ui.put(24.0, H as f32 - 28.0, &version, TextStyle::label(11.0), t.faint, Align::Left);
        for k in ui.keys.clone() {
            if ui.focus.is_none() {
                let i = PAGES.iter().position(|(p, _, _)| *p == self.page).unwrap_or(0);
                match k {
                    K::Down if i + 1 < PAGES.len() => self.go(PAGES[i + 1].0),
                    K::Up if i > 0 => self.go(PAGES[i - 1].0),
                    _ => {}
                }
            }
        }
    }

    fn draw_find(&mut self, ui: &mut Ui) {
        let t = ui.t;
        let (x, w) = ((W as f32 - 560.0) * 0.5, 560.0);
        crate::ui::bolt(ui.canvas, x + 14.0, 70.0, 34.0);
        let nw = ui.put(x + 38.0, 70.0, "Catalyst", TextStyle::sans(26.0, 700.0), t.ink, Align::Left);
        ui.put(x + 44.0 + nw, 70.0, "Companion", TextStyle::sans(26.0, 300.0), t.dim, Align::Left);
        ui.put(x, 150.0, "Find your handheld", TextStyle { tracking: -0.01, ..TextStyle::sans(24.0, 680.0) }, t.ink, Align::Left);
        ui.para(x, 168.0, w, "On the handheld, open Settings › Computer companion › Pair a computer. Then choose it here: it is found on this computer's network, or over the USB cable.", TextStyle::body(14.0), t.dim, 21.0);
        let mut y = 236.0;
        let saved = self.known.clone();
        // Up and Down choose among those found; Enter pairs or opens.
        let mut chosen = false;
        for k in ui.keys.clone() {
            match k {
                K::Down => self.pick = (self.pick + 1).min(self.found.len().saturating_sub(1)),
                K::Up => self.pick = self.pick.saturating_sub(1),
                K::Enter => chosen = true,
                _ => {}
            }
        }
        self.pick = self.pick.min(self.found.len().saturating_sub(1));
        for (i, f) in self.found.clone().into_iter().enumerate() {
            let r = Rect::new(x, y, w, 68.0);
            ui.card(r);
            if i == self.pick && self.found.len() > 1 {
                ui.canvas.stroke_rrect(r.inset(-3.0), 19.0, 1.5, t.ink, 0.8);
            }
            ui.icon(if f.usb { "usb" } else { "wifi" }, r.x + 30.0, r.cy(), 22.0, t.dim);
            ui.put_fit(r.x + 56.0, r.y + 24.0, &f.name, TextStyle::sans(17.0, 650.0), t.ink, w - 220.0);
            let how = format!("{} · {} · Catalyst Linux {}", if f.usb { "USB cable" } else { "Wi-Fi" }, f.host, f.version);
            ui.put_fit(r.x + 56.0, r.y + 47.0, &how, TextStyle::label(12.0), t.dim, w - 220.0);
            let known = saved.iter().find(|s| s.id == f.id).cloned();
            let label = if known.is_some() { "Open" } else { "Pair" };
            let bw = ui.button_w(label).max(96.0);
            if ui.button(Rect::new(r.right() - bw - 16.0, r.cy() - 18.0, bw, 36.0), label, Kind::Primary, true) || (chosen && i == self.pick) {
                match known {
                    Some(s) => {
                        self.current = Some(s);
                        self.phase = Phase::Main;
                        self.online = None;
                        self.connect();
                    }
                    None => {
                        self.code.clear();
                        self.pair_error = None;
                        self.phase = Phase::Pair(f.clone());
                    }
                }
            }
            y += 80.0;
        }
        if self.found.is_empty() {
            let r = Rect::new(x, y, w, 68.0);
            ui.card(r);
            let spin = self.born.elapsed().as_secs_f32() * 4.0;
            ui.canvas.arc(r.x + 30.0, r.cy(), 10.0, 2.5, spin, 4.2, CRIMSON, 1.0);
            ui.put(r.x + 56.0, r.y + 24.0, "Looking…", TextStyle::sans(17.0, 650.0), t.ink, Align::Left);
            ui.put_fit(r.x + 56.0, r.y + 47.0, "on this computer's networks and the USB cable", TextStyle::label(12.0), t.dim, w - 80.0);
            y += 80.0;
        }
        let tip = "Over USB, set the handheld's Settings › USB to network first. Wi-Fi needs both on the same network.";
        ui.para(x, y + 8.0, w, tip, TextStyle::label(12.0), t.faint, 18.0);
        if let Some(s) = self.current.clone().filter(|_| !saved.is_empty()) {
            if ui.button(Rect::new(x, H as f32 - 70.0, 200.0, 36.0), &format!("Back to {}", s.name), Kind::Secondary, true) {
                self.phase = Phase::Main;
            }
        }
    }

    fn draw_pair(&mut self, ui: &mut Ui, f: &Found) {
        let t = ui.t;
        let (x, w) = ((W as f32 - 560.0) * 0.5, 560.0);
        crate::ui::bolt(ui.canvas, x + 14.0, 70.0, 34.0);
        let nw = ui.put(x + 38.0, 70.0, "Catalyst", TextStyle::sans(26.0, 700.0), t.ink, Align::Left);
        ui.put(x + 44.0 + nw, 70.0, "Companion", TextStyle::sans(26.0, 300.0), t.dim, Align::Left);
        ui.put_fit(x, 150.0, &format!("Pair with {}", f.name), TextStyle { tracking: -0.01, ..TextStyle::sans(24.0, 680.0) }, t.ink, w);
        ui.para(x, 168.0, w, "Type the six digits the handheld shows under Settings › Computer companion.", TextStyle::body(14.0), t.dim, 21.0);
        let busy = self.pairing.busy();
        if !busy {
            for c in std::mem::take(&mut ui.typed) {
                if c.is_ascii_digit() && self.code.len() < 6 {
                    self.code.push(c);
                    self.pair_error = None;
                }
            }
            if let Some(p) = ui.pasted.take() {
                self.code = p.chars().filter(char::is_ascii_digit).take(6).collect();
            }
        }
        let (bw, gap) = (64.0, 12.0);
        let left = x;
        for i in 0..6 {
            let r = Rect::new(left + i as f32 * (bw + gap) + if i >= 3 { 16.0 } else { 0.0 }, 214.0, bw, 76.0);
            ui.canvas.rrect(r, 14.0, t.surface2, 1.0);
            let next = i == self.code.len() && !busy;
            if next {
                ui.canvas.stroke_rrect(r.inset(-1.5), 15.5, 1.5, CRIMSON, 1.0);
            }
            if let Some(c) = self.code.chars().nth(i) {
                ui.put(r.cx(), r.cy(), &c.to_string(), TextStyle::display(38.0), t.ink, Align::Center);
            } else if next && (ui.time * 1.6).fract() < 0.6 {
                ui.canvas.fill_rect(Rect::new(r.cx() - 1.0, r.cy() - 16.0, 2.0, 32.0), CRIMSON, 1.0);
            }
        }
        let mut go = false;
        for k in ui.keys.clone() {
            match k {
                K::Backspace if !busy => {
                    self.code.pop();
                    self.pair_error = None;
                }
                K::Enter => go = true,
                K::Escape if !busy => self.phase = Phase::Find,
                _ => {}
            }
        }
        let ready = self.code.len() == 6 && !busy;
        match ui.buttons(x, 318.0, &[(if busy { "Pairing…" } else { "Pair" }, Kind::Primary, ready), ("Back", Kind::Secondary, !busy)]) {
            Some(0) => go = true,
            Some(1) => self.phase = Phase::Find,
            _ => {}
        }
        if go && ready {
            let (host, port, code, me) = (f.host.clone(), f.port, self.code.clone(), win::computer_name());
            self.pair_error = None;
            self.pairing.start(move || net::pair(&host, port, &code, &me));
        }
        if let Some(why) = &self.pair_error {
            let why = format!("{}{}", why[..1].to_uppercase(), &why[1..]);
            ui.para(x, 372.0, w, &why, TextStyle::sans(14.0, 550.0), CRIMSON, 21.0);
        }
        let note = "The code never crosses the network: this computer and the handheld each prove they know it, over the handheld's own certificate, which this computer then trusts and no other.";
        ui.para(x, H as f32 - 120.0, w, note, TextStyle::label(12.0), t.faint, 18.0);
    }

    // ----------------------------------------------------------- sign-ins ---

    fn page_signins(&mut self, ui: &mut Ui) {
        let t = ui.t;
        self.header(ui, "Sign-ins", "Sign the handheld in from here, with no typing on its keys.");
        let name = self.name();
        let online = self.online == Some(true);
        // YouTube Music.
        let r = Rect::new(X0, 112.0, CW, 188.0);
        ui.card(r);
        ui.icon("music_note", r.x + 32.0, r.y + 32.0, 22.0, CRIMSON);
        ui.put(r.x + 56.0, r.y + 32.0, "YouTube Music", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        let music = self.now.as_ref().is_some_and(|n| n["music"] == true);
        let (dot, state) = if music { (GOOD, format!("Signed in on {name}")) } else { (t.faint, "Not signed in".to_owned()) };
        let sw = ui.measure(&state, TextStyle::label(12.0));
        ui.dot(r.right() - 24.0 - sw - 12.0, r.y + 32.0, dot);
        ui.put(r.right() - 24.0, r.y + 32.0, &state, TextStyle::label(12.0), t.dim, Align::Right);
        let words = "Sign in with Google in a window here. The handheld checks the sign-in and keeps it; this computer keeps it only in that window's own profile, which Sign out clears.";
        ui.para(r.x + 24.0, r.y + 56.0, r.w - 48.0, words, TextStyle::body(14.0), t.dim, 20.0);
        let busy = self.yt.busy();
        let first = if busy { "Signing in…" } else if music { "Sign in again" } else { "Sign in with Google" };
        let mut buttons = vec![(first, Kind::Primary, online && !busy)];
        if music || youtube::held_here() {
            buttons.push(("Sign out", Kind::Secondary, online && !busy));
        }
        match ui.buttons(r.x + 24.0, r.y + 118.0, &buttons) {
            Some(0) => {
                let (t2, name) = (self.target(), name.clone());
                self.yt.start(move || {
                    let jar = youtube::sign_in()?;
                    let t2 = t2.ok_or("no handheld")?;
                    t2.call("POST", "/v1/music/sign-in", Send::Text(jar), Sink::Memory, Duration::from_secs(30))?.ok()?;
                    Ok(format!("{name} is signed in to YouTube Music"))
                });
            }
            Some(1) => {
                let t2 = self.target();
                self.yt.start(move || {
                    youtube::forget_here()?;
                    t2.ok_or("no handheld")?.delete("/v1/music/sign-in")?;
                    Ok("Signed out of YouTube Music, here and on the handheld".into())
                });
            }
            _ => {}
        }
        ui.put_fit(r.x + 24.0, r.bottom() - 22.0, "Third-party players can break YouTube's terms, which could put the account at risk.", TextStyle::body(12.5), t.faint, r.w - 48.0);

        // GitHub.
        let r = Rect::new(X0, 316.0, CW, 216.0);
        ui.card(r);
        ui.icon("key", r.x + 32.0, r.y + 32.0, 22.0, CRIMSON);
        ui.put(r.x + 56.0, r.y + 32.0, "GitHub", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        let key = self.now.as_ref().and_then(|n| n["keys"]["github"].as_str()).unwrap_or("not set").to_owned();
        let login = self.now.as_ref().and_then(|n| n["github"].as_str()).map(str::to_owned);
        let set = key != "not set";
        let (dot, state) = match (&login, set) {
            (Some(l), _) => (GOOD, format!("Signed in as @{l}")),
            (None, true) => (GOOD, format!("Token {}", key.trim_start_matches("set · "))),
            _ => (t.faint, "Not signed in".to_owned()),
        };
        let sw = ui.measure(&state, TextStyle::label(12.0));
        ui.dot(r.right() - 24.0 - sw - 12.0, r.y + 32.0, dot);
        ui.put(r.right() - 24.0, r.y + 32.0, &state, TextStyle::label(12.0), t.dim, Align::Right);
        let busy = self.gh_job.busy() || self.gh_device.busy();
        match self.gh.clone() {
            Gh::Idle => {
                let ways = if github::client_id().is_some() { "Use the sign-in this computer already has, sign in with a code, or paste a token." } else { "Use the sign-in this computer already has, or paste a token." };
                ui.para(r.x + 24.0, r.y + 56.0, r.w - 48.0, &format!("For Git on the handheld: your repositories, pulling and pushing. {ways}"), TextStyle::body(14.0), t.dim, 20.0);
                let mut buttons = vec![("Use this computer's GitHub sign-in", Kind::Primary, online && !busy)];
                let device = github::client_id().is_some();
                if device {
                    buttons.push(("Sign in with a code", Kind::Secondary, online && !busy));
                }
                buttons.push(("Paste a token", Kind::Quiet, online && !busy));
                if set {
                    buttons.push(("Sign out", Kind::Danger, online && !busy));
                }
                let labels: Vec<&str> = buttons.iter().map(|b| b.0).collect();
                if let Some(i) = ui.buttons(r.x + 24.0, r.y + 118.0, &buttons) {
                    match labels[i] {
                        "Use this computer's GitHub sign-in" => self.gh = Gh::Consent,
                        "Sign in with a code" => {
                            let id = github::client_id().unwrap_or_default();
                            self.gh_device.start(move || github::device_start(&id));
                        }
                        "Paste a token" => {
                            self.gh = Gh::Paste(String::new());
                            *ui.focus = Some("gh-token".into());
                        }
                        _ => {
                            let t2 = self.target();
                            self.gh_job.start(move || {
                                t2.ok_or("no handheld")?.delete("/v1/github")?;
                                Ok("The handheld is signed out of GitHub".into())
                            });
                        }
                    }
                }
                if !device {
                    ui.put_fit(r.x + 24.0, r.bottom() - 22.0, "To sign in with a code instead, register a GitHub OAuth app: the README says how.", TextStyle::body(12.5), t.faint, r.w - 48.0);
                }
            }
            Gh::Consent => {
                let words = format!("Catalyst Companion will read the GitHub sign-in this computer has (GitHub CLI, or Git Credential Manager, which may ask you to sign in) and send it to {name}. Git on the handheld can then reach your repositories as you do here.");
                ui.para(r.x + 24.0, r.y + 56.0, r.w - 48.0, &words, TextStyle::body(14.0), t.ink, 20.0);
                match ui.buttons(r.x + 24.0, r.y + 140.0, &[(if busy { "Sending…" } else { "Send it to the handheld" }, Kind::Primary, !busy), ("Cancel", Kind::Secondary, !busy)]) {
                    Some(0) => {
                        let (t2, name) = (self.target(), name.clone());
                        self.gh_job.start(move || {
                            let (token, from) = github::from_this_computer()?;
                            let login = github::whose(&token)?;
                            t2.ok_or("no handheld")?.post("/v1/github", json!({ "token": token }))?;
                            Ok(format!("{name} signed in to GitHub as @{login} ({from})"))
                        });
                    }
                    Some(1) => self.gh = Gh::Idle,
                    _ => {}
                }
            }
            Gh::Paste(mut token) => {
                ui.para(r.x + 24.0, r.y + 56.0, r.w - 48.0, "A token from github.com › Settings › Developer settings, with access to the repositories the handheld works on.", TextStyle::body(14.0), t.dim, 20.0);
                let entered = ui.field("gh-token", Rect::new(r.x + 24.0, r.y + 100.0, r.w - 48.0, 40.0), "ghp_… or github_pat_…", &mut token, true);
                let ready = token.trim().len() >= 20 && !busy;
                let clicked = ui.buttons(r.x + 24.0, r.y + 156.0, &[(if busy { "Checking…" } else { "Send" }, Kind::Primary, ready), ("Cancel", Kind::Secondary, !busy)]);
                if (entered || clicked == Some(0)) && ready {
                    self.send_github(token.trim().to_owned(), "pasted");
                }
                self.gh = if clicked == Some(1) { Gh::Idle } else { Gh::Paste(token) };
            }
            Gh::Code(d) => {
                ui.put(r.x + 24.0, r.y + 66.0, "Enter this code on github.com/login/device:", TextStyle::body(14.0), t.dim, Align::Left);
                ui.put(r.x + 24.0, r.y + 106.0, &d.user_code, TextStyle { tracking: 0.08, ..TextStyle::display(34.0) }, t.ink, Align::Left);
                let left = d.expires.saturating_duration_since(Instant::now()).as_secs();
                ui.put(r.x + 24.0, r.y + 140.0, &format!("waiting for GitHub · {}:{:02} left", left / 60, left % 60), TextStyle::label(12.0), t.faint, Align::Left);
                match ui.buttons(r.x + 24.0, r.y + 166.0, &[("Open github.com", Kind::Secondary, true), ("Cancel", Kind::Quiet, true)]) {
                    Some(0) => win::open_url(&d.verification_uri),
                    Some(1) => self.gh = Gh::Idle,
                    _ => {}
                }
            }
        }
    }

    // -------------------------------------------------------------- music ---

    fn page_music(&mut self, ui: &mut Ui) {
        let t = ui.t;
        self.header(ui, "Music", "Songs added here are the handheld's own: Music sorts them by their tags.");
        let online = self.online == Some(true);
        let r = Rect::new(X0, 112.0, CW, 96.0);
        ui.canvas.rrect(r, 16.0, t.surface1, 1.0);
        ui.canvas.stroke_rrect(r.inset(1.0), 15.0, 1.5, t.faint, 0.5);
        ui.icon("music_note", r.x + 40.0, r.cy(), 28.0, CRIMSON);
        ui.put(r.x + 72.0, r.y + 36.0, "Drop songs here", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        ui.put(r.x + 72.0, r.y + 62.0, "MP3, M4A, FLAC or Opus, from Explorer onto this window", TextStyle::body(14.0), t.dim, Align::Left);
        let bw = ui.button_w("Add songs…");
        if ui.button(Rect::new(r.right() - bw - 24.0, r.cy() - 18.0, bw, 36.0), "Add songs…", Kind::Primary, online) {
            let picked = win::choose_files(self.window as *mut _, "Add songs to the handheld", &[("Songs", "*.mp3;*.m4a;*.flac;*.opus;*.ogg;*.aac;*.wav")], true);
            if !picked.is_empty() {
                self.add_songs(picked);
            }
        }
        // What is being sent.
        let queue: Vec<Upload> = self.uploads.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut top = 224.0;
        if !queue.is_empty() {
            let done = queue.iter().filter(|u| matches!(u.state, Sent::Done(_))).count();
            let failed = queue.iter().filter(|u| matches!(u.state, Sent::Failed(_))).count();
            let head = if done + failed == queue.len() { format!("{done} of {} sent{}", queue.len(), if failed > 0 { format!(" · {failed} did not go") } else { String::new() }) } else { format!("Sending {} of {}", (done + failed + 1).min(queue.len()), queue.len()) };
            ui.put(X0, top + 10.0, &head, TextStyle::sans(14.0, 650.0), t.ink, Align::Left);
            top += 28.0;
            for u in queue.iter().rev().take(3) {
                let name = u.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                ui.put_fit(X0, top + 12.0, &name, TextStyle::body(13.5), t.ink, CW * 0.45);
                let bar = Rect::new(X0 + CW * 0.48, top + 8.0, CW * 0.3, 8.0);
                match &u.state {
                    Sent::Waiting => ui.put(bar.x, top + 12.0, "waiting", TextStyle::label(12.0), t.faint, Align::Left),
                    Sent::Sending => {
                        ui.progress(bar, u.progress.get());
                        0.0
                    }
                    Sent::Done(_) => ui.put(bar.x, top + 12.0, "in the library", TextStyle::label(12.0), GOOD, Align::Left),
                    Sent::Failed(why) => ui.put_fit(bar.x, top + 12.0, why, TextStyle::label(12.0), CRIMSON, CW * 0.5),
                };
                if !matches!(u.state, Sent::Failed(_)) {
                    ui.put(X0 + CW, top + 12.0, &size(u.size), TextStyle::label(12.0), t.faint, Align::Right);
                }
                top += 24.0;
            }
            top += 8.0;
        }
        let area = Rect::new(X0, top, CW, H as f32 - top - 20.0);
        let target = self.target();
        let mut lib = std::mem::replace(&mut self.lib, Browser::new("music"));
        let got = browse(ui, &mut lib, area, "Your library", target.as_ref());
        self.lib = lib;
        self.downloaded(got);
    }

    fn downloaded(&mut self, got: Option<(String, String, String)>) {
        let (Some((area, path, name)), Some(t)) = (got, self.target()) else { return };
        let folder = win::downloads_dir().join(match area.as_str() {
            "music" => "Music",
            other => AREAS.iter().find(|a| a.0 == other).map_or("Files", |a| a.1),
        });
        let progress = self.download_progress.clone();
        self.say(format!("Downloading {name}…"), false);
        self.download.start(move || {
            std::fs::create_dir_all(&folder).map_err(|e| format!("could not make {}: {e}", folder.display()))?;
            let to = folder.join(&name);
            let part = folder.join(format!("{name}.part"));
            let mut f = std::fs::File::create(&part).map_err(|e| format!("could not write {}: {e}", part.display()))?;
            let mut tell = |n: u64, total: Option<u64>| progress.set(n as f32 / total.unwrap_or(n.max(1)) as f32);
            let answer = t.call("GET", &format!("/v1/file?area={}&path={}", net::encode(&area), net::encode(&path)), Send::None, Sink::Writer { to: &mut f, progress: &mut tell }, Duration::from_secs(60));
            drop(f);
            match answer.and_then(|a| if a.status == 200 { Ok(()) } else { a.ok().map(|_| ()) }) {
                Ok(()) => {
                    std::fs::rename(&part, &to).map_err(|e| e.to_string())?;
                    Ok(to)
                }
                Err(why) => {
                    std::fs::remove_file(&part).ok();
                    Err(why)
                }
            }
        });
    }

    // -------------------------------------------------------------- files ---

    fn page_files(&mut self, ui: &mut Ui) {
        self.header(ui, "Files", "Match logs, photos, scans, screenshots, notes and battery labels from the handheld, to this computer.");
        let names: Vec<&str> = AREAS.iter().map(|a| a.1).collect();
        // As wide as the row leaves beside the button, so each area's name is whole.
        let room = CW - ui.button_w("Open downloads") - 16.0;
        if let Some(i) = ui.segmented(Rect::new(X0, 106.0, room.min(560.0), 36.0), &names, self.area) {
            self.area = i;
            self.files.area = AREAS[i].0.into();
            if let Some(t) = self.target() {
                self.files.go(&t, String::new());
            }
        }
        let bw = ui.button_w("Open downloads");
        if ui.button(Rect::new(X0 + CW - bw, 106.0, bw, 36.0), "Open downloads", Kind::Quiet, true) {
            std::fs::create_dir_all(win::downloads_dir()).ok();
            win::show_in_folder(&win::downloads_dir());
        }
        let target = self.target();
        let mut files = std::mem::replace(&mut self.files, Browser::new("logs"));
        let label = AREAS[self.area].1;
        let got = browse(ui, &mut files, Rect::new(X0, 156.0, CW, H as f32 - 176.0), label, target.as_ref());
        self.files = files;
        self.downloaded(got);
    }

    // ----------------------------------------------------------- settings ---

    fn page_settings(&mut self, ui: &mut Ui) {
        let t = ui.t;
        self.header(ui, "Settings", "The handheld's team, Wi-Fi, keys and look. Only what you change is sent.");
        let online = self.online == Some(true) && self.now.is_some();
        let label = |ui: &mut Ui, x: f32, y: f32, s: &str| {
            ui.put(x, y, s, TextStyle::sans(13.0, 650.0), t.dim, Align::Left);
        };
        let (lx, rx, colw) = (X0, X0 + CW * 0.5 + 16.0, CW * 0.5 - 16.0);
        // Left: team, Wi-Fi, keys.
        label(ui, lx, 116.0, "Team number");
        let mut team = self.form.team.clone();
        ui.field("team", Rect::new(lx, 128.0, 140.0, 38.0), "5805", &mut team, false);
        self.form.team = team.chars().filter(char::is_ascii_digit).take(5).collect();
        label(ui, lx, 188.0, "Add a Wi-Fi network");
        let (mut ssid, mut psk) = (self.form.ssid.clone(), self.form.psk.clone());
        ui.field("ssid", Rect::new(lx, 200.0, colw, 38.0), "Network name", &mut ssid, false);
        ui.field("psk", Rect::new(lx, 244.0, colw, 38.0), "Password (empty for an open one)", &mut psk, true);
        (self.form.ssid, self.form.psk) = (ssid, psk);
        label(ui, lx, 304.0, "Keys");
        for (i, (name, what)) in KEYS.iter().enumerate() {
            let y = 316.0 + i as f32 * 46.0;
            let shown = self.now.as_ref().and_then(|n| n["keys"][*name].as_str()).unwrap_or("not set").to_owned();
            let mut v = self.form.keys[i].clone();
            ui.field(&format!("key-{name}"), Rect::new(lx, y, colw, 38.0), &format!("{what} · {shown}"), &mut v, true);
            self.form.keys[i] = v;
        }
        // Right: the look and the choices.
        label(ui, rx, 116.0, "Accent");
        for (i, (_, hex)) in ACCENTS.iter().enumerate() {
            let (cx, cy) = (rx + 18.0 + i as f32 * 46.0, 146.0);
            if i == self.form.accent {
                ui.canvas.circle(cx, cy, 19.0, t.ink, 1.0);
                ui.canvas.circle(cx, cy, 16.5, t.ground, 1.0);
            }
            ui.canvas.circle(cx, cy, 14.0, Color::hex(*hex), 1.0);
            if ui.hit(Rect::new(cx - 18.0, cy - 18.0, 36.0, 36.0)) {
                self.form.accent = i;
            }
        }
        label(ui, rx, 188.0, "Mode");
        if let Some(i) = ui.segmented(Rect::new(rx, 200.0, colw, 36.0), &["Dark", "Light"], usize::from(self.form.light)) {
            self.form.light = i == 1;
        }
        label(ui, rx, 254.0, "Text size");
        if let Some(i) = ui.segmented(Rect::new(rx, 266.0, colw, 36.0), &["Compact", "Standard", "Large"], self.form.text) {
            self.form.text = i;
        }
        label(ui, rx, 320.0, "Home");
        if let Some(i) = ui.segmented(Rect::new(rx, 332.0, colw, 36.0), &["Grid", "Carousel", "Flow"], self.form.home) {
            self.form.home = i;
        }
        for (i, (_, what)) in CHOICES.iter().enumerate() {
            let cy = 398.0 + i as f32 * 36.0;
            ui.put(rx, cy, what, TextStyle::body(14.0), t.ink, Align::Left);
            if ui.toggle(rx + colw - 40.0, cy, self.form.choices[i]) {
                self.form.choices[i] = !self.form.choices[i];
            }
        }
        // Along the foot: send what changed.
        let was = self.form_was.clone().unwrap_or_default();
        let changes = self.form.changes(&was);
        let any = changes.as_object().is_some_and(|o| !o.is_empty());
        let busy = self.sending.busy();
        let y = H as f32 - 64.0;
        let send = if busy { "Sending…" } else { "Send to the handheld" };
        let sw = ui.button_w(send);
        let mut go = ui.button(Rect::new(lx, y, sw, 38.0), send, Kind::Primary, online && any && !busy);
        go |= ui.keys.contains(&K::Enter) && ui.focus.is_some() && online && any && !busy;
        let line = match (&self.sent, any) {
            (_, true) => (format!("Ready to send: {}", what_changes(&changes)), t.dim),
            (Some(Ok(rows)), false) if !rows.is_empty() => (format!("Sent: {rows}"), GOOD),
            (Some(Err(why)), false) => (why.clone(), CRIMSON),
            _ => ("Nothing changed".into(), t.faint),
        };
        ui.put_fit(lx + sw + 16.0, y + 19.0, &line.0, TextStyle::body(13.5), line.1, CW - sw - 16.0);
        if go {
            let t2 = self.target();
            self.sent = None;
            self.sending.start(move || t2.ok_or("no handheld")?.post("/v1/settings", changes));
        }
    }

    // ------------------------------------------------------------- system ---

    fn page_system(&mut self, ui: &mut Ui) {
        let t = ui.t;
        self.header(ui, "System", "Updates, backups, and this computer's pairing.");
        let online = self.online == Some(true);
        // Updates.
        let r = Rect::new(X0, 112.0, CW, 150.0);
        ui.card(r);
        let u = self.update.clone().unwrap_or(Value::Null);
        let version = u["version"].as_str().map_or_else(|| self.current.as_ref().map(|s| s.version.clone()).unwrap_or_default(), str::to_owned);
        ui.icon("sync", r.x + 32.0, r.y + 32.0, 22.0, CRIMSON);
        ui.put(r.x + 56.0, r.y + 32.0, &format!("Catalyst Linux {version}"), TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        let s = &u["status"];
        let state = s["state"].as_str().unwrap_or_default();
        let next = s["version"].as_str().unwrap_or_default();
        let pct = |p: &Value| format!("{}%", (p.as_f64().unwrap_or(0.0) * 100.0).round());
        let line = match state {
            "checking" => "Checking for an update…".to_owned(),
            "available" => format!("{next} is ready to install{}", s["size"].as_u64().map_or(String::new(), |b| format!(" · {}", size(b)))),
            "downloading" => format!("Downloading {next} · {}", pct(&s["progress"])),
            "installing" => format!("Installing {next} · {}", pct(&s["progress"])),
            "ready" => format!("{next} is installed: restart into it to finish"),
            "trying" => format!("Starting {next}"),
            "updated" => format!("Updated to {version}"),
            "failed" => format!("The update did not install{}", s["detail"].as_str().filter(|d| !d.is_empty()).map_or(String::new(), |d| format!(": {d}"))),
            _ if u["available"] == false => "This system is not updated over the air".to_owned(),
            _ => "Up to date".to_owned(),
        };
        ui.put_fit(r.x + 24.0, r.y + 66.0, &line, TextStyle::body(14.0), if state == "failed" { CRIMSON } else { t.dim }, r.w - 48.0);
        if state == "downloading" || state == "installing" {
            ui.progress(Rect::new(r.x + 24.0, r.y + 86.0, r.w - 48.0, 6.0), s["progress"].as_f64().unwrap_or(0.0) as f32);
        }
        let busy = self.upd.busy();
        let can = online && u["available"] == true && !busy;
        let (label, step) = match state {
            "available" => (format!("Install {next}"), "install"),
            "ready" => (format!("Restart into {next}"), "apply"),
            "downloading" | "installing" | "checking" | "trying" => ("Updating…".to_owned(), ""),
            _ => ((if busy { "Checking…" } else { "Check for updates" }).to_owned(), "check"),
        };
        let lw = ui.button_w(&label).max(170.0);
        if ui.button(Rect::new(r.x + 24.0, r.bottom() - 52.0, lw, 36.0), &label, Kind::Primary, can && !step.is_empty()) {
            let t2 = self.target();
            let step = step.to_owned();
            self.upd.start(move || t2.ok_or("no handheld")?.call("POST", &format!("/v1/update/{step}"), Send::None, Sink::Memory, Duration::from_secs(150))?.ok());
        }
        // Back up and restore, side by side.
        let half = (CW - 16.0) * 0.5;
        let b = Rect::new(X0, 278.0, half, 192.0);
        ui.card(b);
        ui.icon("cloud", b.x + 32.0, b.y + 32.0, 22.0, CRIMSON);
        ui.put(b.x + 56.0, b.y + 32.0, "Back up", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        ui.para(b.x + 24.0, b.y + 54.0, b.w - 48.0, "Settings, keys, Wi-Fi, music, logs, photos and notes, as a zip in Documents › Catalyst backups. Keep it private: it holds keys and Wi-Fi passwords.", TextStyle::body(13.5), t.dim, 19.0);
        let busy = self.backing.busy();
        if busy {
            let p = self.backup_progress.get();
            ui.progress(Rect::new(b.x + 24.0, b.bottom() - 70.0, b.w - 48.0, 6.0), p);
            ui.put(b.x + 24.0, b.bottom() - 34.0, &format!("Backing up · {}%", (p * 100.0) as u32), TextStyle::body(13.5), t.dim, Align::Left);
        } else {
            let mut items = vec![("Back up now", Kind::Primary, online)];
            if self.last_backup.is_some() {
                items.push(("Show it", Kind::Quiet, true));
            }
            match ui.buttons(b.x + 24.0, b.bottom() - 52.0, &items) {
                Some(0) => self.back_up(),
                Some(1) => {
                    if let Some(p) = &self.last_backup {
                        win::show_in_folder(p);
                    }
                }
                _ => {}
            }
        }
        let rs = Rect::new(X0 + half + 16.0, 278.0, half, 192.0);
        ui.card(rs);
        ui.icon("history", rs.x + 32.0, rs.y + 32.0, 22.0, CRIMSON);
        ui.put(rs.x + 56.0, rs.y + 32.0, "Restore", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        let restoring = self.restoring.busy();
        match self.restore.clone() {
            Some((path, files, _)) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                ui.put_fit(rs.x + 24.0, rs.y + 66.0, &name, TextStyle::sans(14.0, 600.0), t.ink, rs.w - 48.0);
                ui.para(rs.x + 24.0, rs.y + 80.0, rs.w - 48.0, &format!("{files} files. The handheld's files and settings become the backup's, then it restarts."), TextStyle::body(13.5), t.dim, 19.0);
                if restoring {
                    let p = self.restore_progress.get();
                    ui.progress(Rect::new(rs.x + 24.0, rs.bottom() - 70.0, rs.w - 48.0, 6.0), p);
                    ui.put(rs.x + 24.0, rs.bottom() - 34.0, &format!("Restoring · {}%", (p * 100.0) as u32), TextStyle::body(13.5), t.dim, Align::Left);
                } else {
                    match ui.buttons(rs.x + 24.0, rs.bottom() - 52.0, &[("Restore and restart", Kind::Danger, online), ("Cancel", Kind::Quiet, true)]) {
                        Some(0) => self.restore_now(path),
                        Some(1) => self.restore = None,
                        _ => {}
                    }
                }
            }
            None => {
                ui.para(rs.x + 24.0, rs.y + 54.0, rs.w - 48.0, "Puts a backup back on the handheld, which then restarts. Choose its zip, or drop it on this window. Sign in to YouTube Music again after.", TextStyle::body(13.5), t.dim, 19.0);
                let cw = ui.button_w("Choose a backup…");
                if ui.button(Rect::new(rs.x + 24.0, rs.bottom() - 52.0, cw, 36.0), "Choose a backup…", Kind::Secondary, online) {
                    if let Some(p) = win::choose_files(self.window as *mut _, "Restore a backup to the handheld", &[("Catalyst backups", "*.zip")], false).into_iter().next() {
                        self.choose_restore(p);
                    }
                }
            }
        }
        // This computer.
        let c = Rect::new(X0, 486.0, CW, 92.0);
        ui.card(c);
        ui.icon("lock", c.x + 32.0, c.y + 32.0, 22.0, t.dim);
        ui.put(c.x + 56.0, c.y + 32.0, &format!("Paired as {}", win::computer_name()), TextStyle::sans(16.0, 650.0), t.ink, Align::Left);
        ui.put_fit(c.x + 56.0, c.y + 60.0, "Pinned TLS · its token locked to this Windows account", TextStyle::body(12.5), t.dim, c.w - 300.0);
        let label = if self.forget_armed { "Forget it: sure?" } else { "Forget this handheld" };
        let fw = ui.button_w(label);
        if ui.button(Rect::new(c.right() - fw - 24.0, c.cy() - 18.0, fw, 36.0), label, Kind::Danger, true) {
            if self.forget_armed {
                self.forget();
            } else {
                self.forget_armed = true;
            }
        }
    }

    fn back_up(&mut self) {
        let Some(t) = self.target() else { return };
        let progress = self.backup_progress.clone();
        progress.set(0.0);
        let day = civil(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) / 86_400);
        let name = format!("{}-backup-{:04}-{:02}-{:02}.zip", self.name(), day.0, day.1, day.2);
        self.backing.start(move || {
            std::fs::create_dir_all(win::backups_dir()).map_err(|e| format!("could not make the backups folder: {e}"))?;
            let mut to = win::backups_dir().join(&name);
            let mut n = 2;
            while to.exists() {
                to = win::backups_dir().join(name.replace(".zip", &format!(" ({n}).zip")));
                n += 1;
            }
            let (mut writer, reader) = crate::jobs::pipe();
            let zip_to = to.clone();
            let zipping = std::thread::spawn(move || backup::tar_to_zip(reader, &zip_to));
            let mut tell = |n: u64, total: Option<u64>| progress.set(n as f32 / total.unwrap_or(n.max(1)) as f32);
            let fetched = t.call("GET", "/v1/backup", Send::None, Sink::Writer { to: &mut writer, progress: &mut tell }, Duration::from_secs(60));
            drop(writer);
            let zipped = zipping.join().map_err(|_| "the backup could not be written".to_owned())?;
            let answer = fetched?;
            if answer.status != 200 {
                std::fs::remove_file(&to).ok();
                return answer.ok().map(|_| to);
            }
            zipped?;
            Ok(to)
        });
    }

    fn restore_now(&mut self, zip: PathBuf) {
        let Some(t) = self.target() else { return };
        let progress = self.restore_progress.clone();
        progress.set(0.0);
        self.restoring.start(move || {
            let r = backup::Restore::open(&zip)?;
            let len = r.len;
            let mut stream = backup::Stream::new(r);
            let total = len.max(1) as f32;
            let mut tell = |n: u64| progress.set(n as f32 / total);
            t.call("POST", "/v1/restore", Send::Stream { len, from: &mut stream, progress: &mut tell }, Sink::Memory, Duration::from_secs(300))?.ok()
        });
    }

    fn forget(&mut self) {
        let Some(s) = self.current.clone() else { return };
        // The handheld lets go of this computer's token too, if it can be reached.
        s.target.delete("/v1/pair").ok();
        store::forget(&s.id).ok();
        self.current = store::load().into_iter().next();
        self.forget_armed = false;
        self.now = None;
        self.update = None;
        self.online = None;
        self.say(format!("{} forgotten", s.name), false);
        self.phase = if self.current.is_some() { Phase::Main } else { Phase::Find };
        if self.current.is_some() {
            self.connect();
        } else {
            self.find();
        }
    }
}

/// What a change sends, in a few words: "team · Wi-Fi · OpenAI key · look".
fn what_changes(v: &Value) -> String {
    let has = |k: &str| v.get(k).is_some();
    let mut parts: Vec<String> = Vec::new();
    if has("team") {
        parts.push("team".into());
    }
    if has("ssid1") {
        parts.push("Wi-Fi".into());
    }
    parts.extend(KEYS.iter().filter(|(k, _)| has(k)).map(|(_, what)| format!("{what} key")));
    if ["accent", "mode", "text", "home"].iter().any(|k| has(k)) {
        parts.push("look".into());
    }
    if has("choices") {
        parts.push("choices".into());
    }
    parts.join(" · ")
}

/// A folder of the handheld's: where it is (each step back clickable), its
/// folders and files, and Download on each file. A file asked for: (area,
/// path, name).
fn browse(ui: &mut Ui, b: &mut Browser, area: Rect, root: &str, target: Option<&Target>) -> Option<(String, String, String)> {
    let t = ui.t;
    // Where: root › folder › folder.
    let mut x = area.x;
    let mut crumbs = vec![(root.to_owned(), String::new())];
    let mut acc = String::new();
    for part in b.path.split('/').filter(|p| !p.is_empty()) {
        acc = if acc.is_empty() { part.to_owned() } else { format!("{acc}/{part}") };
        crumbs.push((part.to_owned(), acc.clone()));
    }
    let last = crumbs.len() - 1;
    for (i, (name, path)) in crumbs.iter().enumerate() {
        let style = TextStyle::sans(15.0, if i == last { 650.0 } else { 500.0 });
        let name = ui.text.fit(name, &style, 220.0);
        let w = ui.measure(&name, style);
        let r = Rect::new(x - 4.0, area.y, w + 8.0, 28.0);
        if i != last && ui.over(r) {
            ui.canvas.rrect(r, 8.0, t.ink, 0.06);
        }
        ui.put(x, area.y + 14.0, &name, style, if i == last { t.ink } else { t.dim }, Align::Left);
        if i != last && ui.hit(r) {
            if let Some(tg) = target {
                b.go(tg, path.clone());
            }
            return None;
        }
        x += w + 8.0;
        if i != last {
            ui.put(x, area.y + 14.0, "›", style, t.faint, Align::Left);
            x += 16.0;
        }
    }
    let list = Rect::new(area.x, area.y + 38.0, area.w, area.h - 38.0);
    ui.canvas.rrect(list, 16.0, t.surface1, 1.0);
    let entries = match &b.entries {
        None => {
            let msg = if target.is_some() { "Reading the handheld…" } else { "Not connected" };
            ui.put(list.cx(), list.cy(), msg, TextStyle::body(14.0), t.faint, Align::Center);
            return None;
        }
        Some(Err(why)) => {
            let why = why.clone();
            ui.put_fit(list.x + 24.0, list.cy(), &why, TextStyle::body(14.0), CRIMSON, list.w - 48.0);
            return None;
        }
        Some(Ok(e)) if e.is_empty() => {
            ui.icon("folder", list.cx(), list.cy() - 18.0, 28.0, t.faint);
            ui.put(list.cx(), list.cy() + 16.0, "Nothing here yet", TextStyle::body(14.0), t.faint, Align::Center);
            return None;
        }
        Some(Ok(e)) => e.clone(),
    };
    let row_h = 46.0;
    let inner = list.inset(6.0);
    let most = (entries.len() as f32 * row_h - inner.h).max(0.0);
    if ui.over(list) && ui.wheel != 0.0 {
        let to = (b.scroll.target() - ui.wheel * 60.0).clamp(0.0, most);
        b.scroll.set(to);
    }
    let scroll = b.scroll.value().clamp(0.0, most);
    ui.canvas.set_clip(inner);
    let mut asked = None;
    let mut open = None;
    for (i, e) in entries.iter().enumerate() {
        let y = inner.y + i as f32 * row_h - scroll;
        if y + row_h < inner.y || y > inner.bottom() {
            continue;
        }
        let r = Rect::new(inner.x, y, inner.w, row_h - 4.0);
        let over = ui.over(r) && ui.over(inner);
        if over {
            ui.canvas.rrect(r, 12.0, t.surface2, 1.0);
        }
        let icon = if e.dir {
            "folder"
        } else {
            match b.area.as_str() {
                "music" => "music_note",
                "photos" | "screenshots" | "strategy" => "photo",
                "logs" => "insights",
                "batteries" => "qr_code_2",
                _ => "description",
            }
        };
        ui.icon(icon, r.x + 24.0, r.cy(), 20.0, if e.dir { CRIMSON } else { t.dim });
        ui.put_fit(r.x + 48.0, r.cy(), &e.name, TextStyle::body(14.0), t.ink, r.w * 0.5);
        let meta = if e.dir { when(e.modified) } else { format!("{} · {}", size(e.size), when(e.modified)) };
        if e.dir {
            ui.put(r.right() - 16.0, r.cy(), &meta, TextStyle::label(12.0), t.faint, Align::Right);
            if over && ui.clicked && ui.over(inner) {
                open = Some(e.name.clone());
            }
        } else {
            let dw = 108.0;
            ui.put(r.right() - dw - 28.0, r.cy(), &meta, TextStyle::label(12.0), t.faint, Align::Right);
            if ui.button(Rect::new(r.right() - dw - 8.0, r.cy() - 15.0, dw, 30.0), "Download", if over { Kind::Secondary } else { Kind::Quiet }, target.is_some()) && ui.over(inner) {
                let path = if b.path.is_empty() { e.name.clone() } else { format!("{}/{}", b.path, e.name) };
                asked = Some((b.area.clone(), path, e.name.clone()));
            }
        }
        ui.pointing |= over;
    }
    ui.canvas.clear_clip();
    if let (Some(name), Some(tg)) = (open, target) {
        let path = if b.path.is_empty() { name } else { format!("{}/{name}", b.path) };
        b.go(tg, path);
    }
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_changed_is_sent_by_the_setup_pages_names() {
        let now = json!({"team": "5805", "accent": "rose", "mode": "dark", "text": "standard", "home": "grid", "open_robot": true, "match_report": false, "ssh": false, "calm": false});
        let was = Form::from_now(&now);
        assert_eq!((was.accent, was.text, was.home, was.light), (4, 1, 0, false));
        assert_eq!(was.changes(&was), json!({}));
        let mut f = was.clone();
        f.light = true;
        f.ssid = "Pit Wi-Fi".into();
        f.psk = "secret".into();
        f.keys[0] = "tba-key".into();
        f.choices[3] = true;
        let v = f.changes(&was);
        assert_eq!(v["mode"], "light");
        assert_eq!((v["ssid1"].as_str(), v["psk1"].as_str(), v["tba"].as_str()), (Some("Pit Wi-Fi"), Some("secret"), Some("tba-key")));
        assert_eq!((v["choices"].as_str(), v["calm"].as_bool(), v["open_robot"].as_bool()), (Some("1"), Some(true), Some(true)), "all four choices go together");
        assert!(v.get("team").is_none() && v.get("accent").is_none());
    }

    #[test]
    fn dates_read_as_a_person_reads_them() {
        assert_eq!(when(1_763_800_000), "Nov 22, 2025");
        assert!(is_song(Path::new("a/B.FLAC")) && !is_song(Path::new("notes.txt")));
    }
}
