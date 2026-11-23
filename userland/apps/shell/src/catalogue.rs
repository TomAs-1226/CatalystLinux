//! The app catalogue: apps to get, update and remove over the air, listed
//! in an index served with the catalogue's GitHub release (by default the
//! one this project's `catalyst-apps/` folder is published to). Each app is
//! a gzipped tar of an app folder, as `/data/apps` holds them, with its
//! SHA-256 and an Ed25519 signature in the index (see `appkg`).
//!
//! Getting one: check the signature against the keys this handheld trusts
//! (the image's `/etc/catalyst/apps.pub`, and any added under
//! `/data/catalyst/app-keys`), download it (HTTPS only), check its hash,
//! check every path in it stays inside it and nothing in it is a link,
//! unpack it beside the others, check its app file names this app at this
//! version and something to run, and only then put it in place. A failure
//! at any step leaves what was there as it was. Apps installed this way
//! carry a `.catalogue` file; only those are removed from here. Nothing of
//! the system is touched: apps live on `/data`.
//!
//! An app's page says what it uses (its permissions) before it is got, and
//! an update that would use more says so and waits for a second Enter.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use appkg::{Listing, Permission};
use bezel::{Align, Rect};
use serde_json::Value;

use crate::git::Job;
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Where the catalogue's index is, unless `CATALYST_CATALOGUE` (or the file
/// `/data/catalyst/catalogue`) says otherwise: the newest `catalogue`
/// release of TomAs-1226/catalyst-apps, where the packages sit beside it.
pub const INDEX: &str = "https://github.com/TomAs-1226/catalyst-apps/releases/download/catalogue/index.json";

/// Where the handheld keeps the catalogue's own state: `/data/catalyst`.
fn state_dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        "/data/catalyst".into()
    } else {
        crate::config_dir()
    }
}

/// The index's address: `CATALYST_CATALOGUE`, else the first line of
/// `/data/catalyst/catalogue` (to try a catalogue of your own), else [`INDEX`].
pub fn index_url() -> String {
    if let Ok(url) = std::env::var("CATALYST_CATALOGUE") {
        return url;
    }
    std::fs::read_to_string(state_dir().join("catalogue"))
        .ok()
        .and_then(|t| t.lines().next().map(|l| l.trim().to_owned()))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| INDEX.to_owned())
}

/// The files and folders holding the keys this handheld trusts to sign apps.
pub fn key_files() -> Vec<PathBuf> {
    if let Some(list) = std::env::var_os("CATALYST_APP_KEYS") {
        return std::env::split_paths(&list).collect();
    }
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        vec!["/etc/catalyst/apps.pub".into(), "/data/catalyst/app-keys".into()]
    } else {
        vec![crate::config_dir().join("app-keys")]
    }
}

/// The folder apps of the user's own are kept in.
pub fn apps_dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        "/data/apps".into()
    } else {
        crate::apps::folders().pop().unwrap_or_else(|| crate::config_dir().join("apps"))
    }
}

/// The version of an app the catalogue installed, if it did.
pub fn installed(dir: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(id).join(".catalogue")).ok()?;
    Some(text.lines().find_map(|l| l.strip_prefix("version="))?.trim().to_owned())
}

/// What the app in `dir/id` says it uses, as installed (any app, the
/// catalogue's or not).
fn installed_permissions(dir: &Path, id: &str) -> Vec<Permission> {
    std::fs::read_to_string(dir.join(id).join("app")).map(|t| appkg::Manifest::parse(&t).permissions).unwrap_or_default()
}

/// Whether every path in an archive's listing stays inside the folder it
/// is unpacked into: nothing absolute, nothing climbing out with `..`.
pub fn paths_stay_inside(listing: &str) -> bool {
    listing.lines().map(str::trim).filter(|l| !l.is_empty()).all(|p| !p.starts_with('/') && !p.starts_with('\\') && !p.contains(':') && p.split(['/', '\\']).all(|part| part != ".."))
}

/// Whether an archive's long listing (`tar -tv`) holds only plain files and
/// folders: a link could point outside the folder, and a later file written
/// through it would land there.
pub fn only_files_and_folders(long_listing: &str) -> bool {
    long_listing.lines().filter(|l| !l.trim().is_empty()).all(|l| matches!(l.as_bytes()[0], b'-' | b'd') && !l.contains(" link to ") && !l.contains(" -> "))
}

/// The apps being installed just now, so the same one is not installed
/// twice at once (both would use the same files beside it).
static INSTALLING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|_| format!("no {program} on this system"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("failed").trim().to_owned())
    }
}

/// The last index read, kept for Spotlight to find apps in without asking
/// the network.
fn cache_path() -> PathBuf {
    state_dir().join("catalogue-index.json")
}

/// Give every app that carries its own icon that icon, by `app:<id>`.
fn register_icons(apps: &[Listing]) {
    for app in apps {
        if app.icon.starts_with("app:") && !app.icon_path.is_empty() {
            bezel::icons::register(&app.icon, app.icon_path.clone());
        }
    }
}

fn fetch_index() -> Result<Vec<Listing>, String> {
    let url = index_url();
    if url == "demo" {
        return Ok(demo());
    }
    // Releases' downloads redirect to GitHub's file servers: followed, HTTPS only.
    let text = run("curl", &["-fsSL", "--max-time", "20", "--proto", "=https", "--proto-redir", "=https", &url]).map_err(|_| "could not reach the catalogue".to_string())?;
    let index: Value = serde_json::from_str(&text).map_err(|_| "the catalogue's index is not readable".to_string())?;
    let apps = appkg::parse_index(&index, &url);
    register_icons(&apps);
    // Kept, with the address it came from, for Spotlight.
    let kept = serde_json::json!({ "url": url, "index": index });
    let path = cache_path();
    let tmp = path.with_extension("new");
    if std::fs::write(&tmp, kept.to_string()).is_ok() {
        std::fs::rename(&tmp, &path).ok();
    }
    Ok(apps)
}

/// The apps of the last index read, from what was kept.
fn cached() -> Vec<Listing> {
    let Ok(text) = std::fs::read_to_string(cache_path()) else { return Vec::new() };
    let Ok(kept) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    let apps = appkg::parse_index(&kept["index"], kept["url"].as_str().unwrap_or(INDEX));
    register_icons(&apps);
    apps
}

/// Whether the system is new enough for an app that needs `needs`.
fn system_ok(needs: &str) -> bool {
    needs.is_empty() || !appkg::newer(needs, &crate::sysupdate::version())
}

/// Get (or update) an app: check, download, check, unpack beside, check, put in place.
pub fn install(app: &Listing, dir: &Path, keys: &[String]) -> Result<(), String> {
    {
        let mut busy = INSTALLING.lock().unwrap();
        if busy.contains(&app.id) {
            return Err(format!("{} is already being installed", app.name));
        }
        busy.push(app.id.clone());
    }
    // A live activity while it installs, and a moment of done after.
    let task = crate::liveactivity::task(crate::liveactivity::Kind::Download, format!("Installing {}", app.name), "from the catalogue");
    let result = install_now(app, dir, keys);
    INSTALLING.lock().unwrap().retain(|id| *id != app.id);
    if result.is_ok() {
        task.done("installed");
    }
    // A notification either way (unsaid if the catalogue is in front).
    let n = match &result {
        Ok(()) => crate::notify::Notification::new("Catalogue", "download_done", format!("{} installed", app.name)),
        Err(why) => crate::notify::Notification::new("Catalogue", "error", format!("{} not installed", app.name)).body(why.clone()),
    };
    crate::notify::post(n.action("builtin:Catalogue"));
    result
}

fn install_now(app: &Listing, dir: &Path, keys: &[String]) -> Result<(), String> {
    if !system_ok(&app.needs) {
        return Err(format!("{} needs Catalyst {}: update the system first", app.name, app.needs));
    }
    if app.archive == "demo" {
        return Err("the demo catalogue only shows its apps".into());
    }
    // Signed by a key this handheld trusts, before a byte is fetched.
    let key = appkg::signed_by(&app.id, &app.version, &app.sha256, &app.signature, keys)
        .ok_or_else(|| format!("{} is not signed by a key this handheld trusts: not installed", app.name))?
        .to_owned();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let archive = dir.join(format!(".{}.tar.gz", app.id));
    let unpacked = dir.join(format!(".{}.new", app.id));
    let result = (|| {
        let max = appkg::PACKAGE_MAX.to_string();
        run("curl", &["-fsSL", "--max-time", "120", "--proto", "=https", "--proto-redir", "=https", "--max-filesize", &max, "-o", &archive.to_string_lossy(), &app.archive])
            .map_err(|_| "could not download it".to_string())?;
        let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
        if appkg::sha256_hex(&bytes) != app.sha256 {
            return Err("it did not match the catalogue: not installed".into());
        }
        let listing = run("tar", &["-tzf", &archive.to_string_lossy()])?;
        let long_listing = run("tar", &["-tvzf", &archive.to_string_lossy()])?;
        if !paths_stay_inside(&listing) || !only_files_and_folders(&long_listing) {
            return Err("it would write outside its folder: not installed".into());
        }
        std::fs::remove_dir_all(&unpacked).ok();
        std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
        run("tar", &["-xzf", &archive.to_string_lossy(), "-C", &unpacked.to_string_lossy()])?;
        let text = std::fs::read_to_string(unpacked.join("app")).map_err(|_| "it has no app file: not installed".to_string())?;
        let manifest = appkg::Manifest::parse(&text);
        // The package is the app the index (and its signature) named.
        if (!manifest.id.is_empty() && manifest.id != app.id) || (!manifest.version.is_empty() && manifest.version != app.version) {
            return Err("it is not the app the catalogue named: not installed".into());
        }
        crate::apps::parse(&text, &unpacked).ok_or("its app file names nothing to run: not installed")?;
        std::fs::write(unpacked.join(".catalogue"), format!("id={}\nversion={}\nsha256={}\nkey={}\n", app.id, app.version, app.sha256, appkg::key_id(&key))).map_err(|e| e.to_string())?;
        // Into place: the old one aside, the new one in, the old one gone.
        let target = dir.join(&app.id);
        let old = dir.join(format!(".{}.old", app.id));
        std::fs::remove_dir_all(&old).ok();
        if target.exists() {
            std::fs::rename(&target, &old).map_err(|e| e.to_string())?;
        }
        if let Err(e) = std::fs::rename(&unpacked, &target) {
            // Put the old one back rather than leave neither.
            std::fs::rename(&old, &target).ok();
            return Err(e.to_string());
        }
        std::fs::remove_dir_all(&old).ok();
        Ok(())
    })();
    std::fs::remove_file(&archive).ok();
    std::fs::remove_dir_all(&unpacked).ok();
    result
}

/// Remove an app the catalogue installed; one put there by hand is left
/// alone. What it kept in its own folder on /data stays, for a reinstall.
pub fn remove(id: &str, dir: &Path) -> Result<(), String> {
    if installed(dir, id).is_none() {
        return Err("not one the catalogue installed".into());
    }
    std::fs::remove_dir_all(dir.join(id)).map_err(|e| e.to_string())
}

/// How an app stands on this handheld.
#[derive(Clone, Debug, PartialEq)]
enum Standing {
    Get,
    Installed(String),
    Update(String),
    Working(&'static str),
    NeedsSystem(String),
}

// ---------------------------------------------------------------- state ---

struct Working {
    id: String,
    name: String,
    verb: &'static str,
    job: Job<Result<(), String>>,
}

/// What the catalogue's pages share: the index, the install under way, and
/// those queued behind it (update all).
struct Shared {
    index: Job<Result<Vec<Listing>, String>>,
    working: Option<Working>,
    queue: Vec<String>,
    keys: Vec<String>,
}

type State = Rc<RefCell<Shared>>;

impl Shared {
    fn new(index: Job<Result<Vec<Listing>, String>>) -> State {
        Rc::new(RefCell::new(Shared { index, working: None, queue: Vec::new(), keys: appkg::trusted_keys(&key_files()) }))
    }

    fn apps(&self) -> Vec<Listing> {
        self.index.get().and_then(Result::ok).unwrap_or_default()
    }

    fn standing(&self, app: &Listing, dir: &Path) -> Standing {
        if let Some(w) = self.working.as_ref().filter(|w| w.id == app.id) {
            return Standing::Working(w.verb);
        }
        match installed(dir, &app.id) {
            Some(v) if v == app.version || !appkg::newer(&app.version, &v) => Standing::Installed(v),
            _ if !system_ok(&app.needs) => Standing::NeedsSystem(app.needs.clone()),
            Some(_) => Standing::Update(app.version.clone()),
            None => Standing::Get,
        }
    }

    /// Start getting `app`, or queue it behind the one under way.
    fn get(&mut self, app: &Listing, say: &mut Option<String>) {
        if self.working.is_some() {
            if !self.queue.contains(&app.id) {
                self.queue.push(app.id.clone());
            }
            *say = Some(format!("{} next", app.name));
            return;
        }
        let dir = apps_dir();
        let verb = if installed(&dir, &app.id).is_some() { "updating" } else { "getting" };
        *say = Some(format!("{verb} {}…", app.name));
        let (listing, keys) = (app.clone(), self.keys.clone());
        // Behind the frames, and tar and the rest it starts with it (sched.rs).
        let job = Job::start(move || {
            crate::sched::enter(crate::sched::Class::Background);
            install(&listing, &dir, &keys)
        });
        self.working = Some(Working { id: app.id.clone(), name: app.name.clone(), verb, job });
    }

    /// Every frame: has the install under way finished? Then the next.
    fn poll(&mut self, say: &mut Option<String>) {
        let Some(done) = self.working.as_ref().and_then(|w| w.job.get()) else { return };
        let w = self.working.take().expect("checked above");
        *say = Some(match done {
            Ok(()) if w.verb == "updating" => format!("{} is up to date", w.name),
            Ok(()) => format!("{} is on Home", w.name),
            Err(why) => why,
        });
        let apps = self.apps();
        while let Some(id) = (!self.queue.is_empty()).then(|| self.queue.remove(0)) {
            if let Some(next) = apps.iter().find(|a| a.id == id) {
                let mut said = None;
                self.get(next, &mut said);
                break;
            }
        }
    }

    fn busy(&self) -> bool {
        self.index.get().is_none() || self.working.is_some()
    }
}

/// The icon to draw for a listing: its own, a built-in one, or the store's.
fn icon_of(app: &Listing) -> &str {
    if bezel::Icons::has(&app.icon) {
        &app.icon
    } else {
        "apps"
    }
}

/// A size as people say it.
fn size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// A sliver of light running along the bottom of something being got, for
/// as long as the download takes (curl says nothing of how far it is).
fn working_bar(g: &mut Gfx, r: Rect, t: f32) {
    let track = Rect::new(r.x + 12.0, r.bottom() - 4.0, r.w - 24.0, 2.0);
    g.surface(track, 1.0, g.tokens.track);
    let w = track.w * 0.28;
    // Out and back on a smooth ease, a little over a second each way.
    let phase = (t / 1.2).fract();
    let ease = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    let signal = g.tokens.signal;
    g.surface(Rect::new(track.x + (track.w - w) * ease, track.y, w, track.h), 1.0, signal);
}

// ---------------------------------------------------------------- list ---

pub struct CatalogueScreen {
    state: State,
    list: ListState,
    t: f32,
}

impl CatalogueScreen {
    pub fn new() -> Self {
        CatalogueScreen { state: Shared::new(Job::start(fetch_index)), list: ListState::default(), t: 0.0 }
    }

    fn updates(&self) -> Vec<Listing> {
        let s = self.state.borrow();
        let dir = apps_dir();
        s.apps().into_iter().filter(|a| matches!(s.standing(a, &dir), Standing::Update(_))).collect()
    }
}

impl Default for CatalogueScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for CatalogueScreen {
    fn title(&self) -> &str {
        "Catalogue"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![
            keys("catalogue", &[("enter", "Open the app"), ("u", "Update them all"), ("r", "Look again")]),
            keys("moving", crate::screens::LIST_KEYS),
        ]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let apps = self.state.borrow().apps();
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('r') => {
                let mut s = self.state.borrow_mut();
                s.index = Job::start(fetch_index);
                s.keys = appkg::trusted_keys(&key_files());
            }
            Key::Char('u') => {
                let updates = self.updates();
                if updates.is_empty() {
                    *cx.say = Some("everything is up to date".into());
                } else {
                    let mut said = None;
                    for app in &updates {
                        self.state.borrow_mut().get(app, &mut said);
                    }
                    *cx.say = Some(format!("updating {} app{}", updates.len(), if updates.len() == 1 { "" } else { "s" }));
                }
            }
            Key::Enter => {
                if let Some(app) = apps.get(self.list.focus) {
                    return Nav::Push(Box::new(AppPage::new(self.state.clone(), app.id.clone())));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, apps.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        self.state.borrow_mut().poll(cx.say);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.t += dt;
        self.state.borrow().busy()
    }

    fn busy(&self) -> bool {
        self.state.borrow().busy()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let state = self.state.borrow();
        let apps = match state.index.get() {
            None => {
                g.empty("storefront", "looking at the catalogue…");
                return;
            }
            Some(Err(why)) => {
                g.empty("storefront", &why);
                g.hint("r: try again · needs Wi-Fi");
                return;
            }
            Some(Ok(apps)) => apps,
        };
        if apps.is_empty() {
            g.empty("storefront", "the catalogue is empty");
            return;
        }
        let dir = apps_dir();
        let bottom = g.above_hint();
        // Two lines a row: the name, and the line under it.
        let row_h = (ty.name.size + ty.caption.size + 11.0).round();
        let mut updates = 0;
        for (i, y) in self.list.layout(g, apps.len(), top, row_h, 3.0, bottom) {
            let app = &apps[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, 12.0, if focused { t.surface2 } else { t.surface1 });
            g.icon(icon_of(app), focused, r.x + 15.0, r.cy(), 16.0, if focused { t.ink } else { t.dim });
            let standing = state.standing(app, &dir);
            let (words, loud) = match &standing {
                Standing::Get => ("get".to_owned(), true),
                Standing::Installed(_) => ("installed".to_owned(), false),
                Standing::Update(v) => (format!("update {v}"), true),
                Standing::Working(verb) => (format!("{verb}…"), false),
                Standing::NeedsSystem(_) => ("needs update".to_owned(), false),
            };
            // On the name's line, so the line under it can run the width.
            let sw = g.text_at(r.right() - 10.0, r.y + 5.0 + ty.name.size * 0.5, Align::Right, &words, ty.label, if loud { t.signal } else { t.dim });
            g.text_fit(r.x + 30.0, r.y + 5.0 + ty.name.size * 0.5, &app.name, ty.name, t.ink, r.w - 44.0 - sw);
            g.text_fit(r.x + 30.0, r.bottom() - 5.0 - ty.caption.size * 0.5, &app.about, ty.caption, t.dim, r.w - 40.0);
            if matches!(standing, Standing::Working(_)) {
                working_bar(g, r, self.t);
            }
            if focused {
                g.focus_ring(r, 12.0);
            }
        }
        self.list.scrollbar(g, apps.len(), top, bottom);
        for app in apps.iter() {
            if matches!(state.standing(app, &dir), Standing::Update(_)) {
                updates += 1;
            }
        }
        drop(state);
        if updates > 0 {
            let n = if updates == 1 { "u: 1 update".to_owned() } else { format!("u: {updates} updates") };
            let (all, short) = (format!("enter: open · {n} · r: refresh"), format!("enter · {n}"));
            g.hint_any(&[&all, &short]);
        } else {
            g.hint_any(&["enter: open · r: refresh", "enter · r"]);
        }
    }
}

// --------------------------------------------------------- an app's page ---

/// What a row of an app's page does.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Action,
    Uses(Option<Permission>),
    About,
    Remove,
}

struct AppPage {
    state: State,
    id: String,
    title: String,
    list: ListState,
    t: f32,
    /// Enter pressed once on Remove, or on an update that uses more.
    armed: Option<Row>,
}

impl AppPage {
    fn new(state: State, id: String) -> Self {
        let title = state.borrow().apps().into_iter().find(|a| a.id == id).map_or_else(|| id.clone(), |a| a.name);
        AppPage { state, id, title, list: ListState::default(), t: 0.0, armed: None }
    }

    fn app(&self) -> Option<Listing> {
        self.state.borrow().apps().into_iter().find(|a| a.id == self.id)
    }

    fn rows(&self, app: &Listing) -> Vec<Row> {
        let mut rows = vec![Row::Action];
        if app.permissions.is_empty() {
            rows.push(Row::Uses(None));
        } else {
            rows.extend(app.permissions.iter().map(|p| Row::Uses(Some(*p))));
        }
        rows.push(Row::About);
        if installed(&apps_dir(), &app.id).is_some() {
            rows.push(Row::Remove);
        }
        rows
    }

    /// What an update would use that the installed version does not.
    fn more_used(&self, app: &Listing) -> Vec<Permission> {
        let dir = apps_dir();
        if installed(&dir, &app.id).is_none() {
            return Vec::new();
        }
        let now = installed_permissions(&dir, &app.id);
        app.permissions.iter().copied().filter(|p| !now.contains(p)).collect()
    }
}

impl Screen for AppPage {
    fn title(&self) -> &str {
        &self.title
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let row = self.app().and_then(|app| self.rows(&app).get(self.list.focus).copied());
        let here = match row {
            Some(Row::Remove) => keys("remove", &[("enter twice", "Remove it")]),
            Some(Row::About) => keys("about it", &[("enter", "Read it")]),
            _ => keys("the app", &[("enter", "Get or update it")]),
        };
        vec![here, keys("moving", crate::screens::LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let Some(app) = self.app() else { return Nav::Pop };
        let rows = self.rows(&app);
        let armed = self.armed.take();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => match rows.get(self.list.focus).copied() {
                Some(Row::Action) => {
                    let standing = self.state.borrow().standing(&app, &apps_dir());
                    match standing {
                        Standing::Get => self.state.borrow_mut().get(&app, cx.say),
                        Standing::Update(_) => {
                            let more = self.more_used(&app);
                            if more.is_empty() || armed == Some(Row::Action) {
                                self.state.borrow_mut().get(&app, cx.say);
                            } else {
                                let names: Vec<&str> = more.iter().map(|p| p.name()).collect();
                                *cx.say = Some(format!("now uses {}: enter again", names.join(", ")));
                                self.armed = Some(Row::Action);
                            }
                        }
                        Standing::Installed(_) => *cx.say = Some(format!("{} is on Home", app.name)),
                        Standing::NeedsSystem(v) => *cx.say = Some(format!("needs Catalyst {v}: update the system first")),
                        Standing::Working(_) => {}
                    }
                }
                Some(Row::Uses(_)) => {}
                Some(Row::About) => return Nav::Push(Box::new(AboutApp::new(&app))),
                Some(Row::Remove) => {
                    if armed == Some(Row::Remove) {
                        *cx.say = Some(match remove(&app.id, &apps_dir()) {
                            Ok(()) => format!("removed {}", app.name),
                            Err(why) => why,
                        });
                        self.list.focus = 0;
                    } else {
                        *cx.say = Some(format!("enter again removes {}", app.name));
                        self.armed = Some(Row::Remove);
                    }
                }
                None => {}
            },
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        self.state.borrow_mut().poll(cx.say);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.t += dt;
        self.state.borrow().working.is_some()
    }

    fn busy(&self) -> bool {
        self.state.borrow().working.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let Some(app) = self.app() else { return };
        let (t, ty) = (g.tokens, g.ty);
        // Its icon on a tile, the line under its name, and the facts.
        let top = g.top() + 1.0;
        let tile = Rect::new(PAD, top, 30.0, 30.0);
        g.surface(tile, g.radius(9.0), t.surface2);
        g.icon(icon_of(&app), true, tile.cx(), tile.cy(), 20.0, t.ink);
        let text_x = tile.right() + 8.0;
        let room = W - PAD - text_x;
        g.text_fit(text_x, tile.y + ty.body.size * 0.55, &app.about, ty.body, t.ink, room);
        let facts: Vec<String> = [app.version.clone(), size(app.bytes), app.licence.clone()].into_iter().filter(|s| !s.is_empty()).collect();
        g.text_fit(text_x, tile.bottom() - ty.caption.size * 0.5, &facts.join(" · "), ty.caption, t.dim, room);
        // The rows.
        let rows = self.rows(&app);
        let dir = apps_dir();
        let standing = self.state.borrow().standing(&app, &dir);
        let more = if matches!(standing, Standing::Update(_)) { self.more_used(&app) } else { Vec::new() };
        let list_top = tile.bottom() + 6.0;
        let bottom = g.above_hint();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            let radius = r.h * 0.5;
            g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
            let (icon, words, value, loud): (&str, String, String, bool) = match rows[i] {
                Row::Action => match &standing {
                    Standing::Get => ("download", format!("Get {}", app.name), size(app.bytes), true),
                    Standing::Update(v) => ("sync", format!("Update to {v}"), size(app.bytes), true),
                    Standing::Installed(v) => ("check_circle", "On Home".into(), v.clone(), false),
                    Standing::Working(verb) => ("download", format!("{}{}…", verb[..1].to_uppercase(), &verb[1..]), String::new(), false),
                    Standing::NeedsSystem(v) => ("info", "Needs a system update".into(), format!("catalyst {v}"), false),
                },
                // What an update would use that the version on Home does not.
                Row::Uses(Some(p)) => (p.icon(), p.means().into(), if more.contains(&p) { "new".into() } else { String::new() }, false),
                Row::Uses(None) => ("check_circle", "Uses only its screen and keys".into(), String::new(), false),
                Row::About => ("info", "About it".into(), "credit · licence".into(), false),
                Row::Remove => ("delete", "Remove".into(), "keeps what it saved".into(), false),
            };
            let ink = if loud { t.signal } else if focused { t.ink } else { t.dim };
            g.icon(icon, focused || loud, r.x + 15.0, r.cy(), 15.0, ink);
            let vw = if value.is_empty() { 0.0 } else { g.text_at(r.right() - 10.0, r.cy(), Align::Right, &value, ty.label, t.dim) + 8.0 };
            let word_ink = if loud { t.signal } else { t.ink };
            g.text_fit(r.x + 30.0, r.cy(), &words, ty.body, word_ink, r.w - 40.0 - vw);
            if rows[i] == Row::Action && matches!(standing, Standing::Working(_)) {
                working_bar(g, r, self.t);
            }
            if focused {
                g.focus_ring(r, radius);
            }
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom);
        let hint = match rows.get(self.list.focus) {
            Some(Row::Remove) => "enter twice: remove · esc: back",
            Some(Row::About) => "enter: read · esc: back",
            _ => "enter: get or update · esc: back",
        };
        g.hint_any(&[hint, "enter · esc"]);
    }
}

/// An app's longer words: what it does, whose work it is, its licence and
/// where its source is. Up and Down read on.
struct AboutApp {
    title: String,
    text: String,
    first: usize,
    lines: usize,
    shown: usize,
}

impl AboutApp {
    fn new(app: &Listing) -> Self {
        let mut parts = Vec::new();
        if !app.description.is_empty() {
            parts.push(app.description.clone());
        }
        if !app.credit.is_empty() {
            parts.push(app.credit.clone());
        }
        if !app.licence.is_empty() {
            parts.push(format!("Licence: {}.", app.licence));
        }
        if !app.source.is_empty() {
            parts.push(format!("Source: {}", app.source));
        }
        AboutApp { title: app.name.clone(), text: parts.join("\n\n"), first: 0, lines: 0, shown: 0 }
    }
}

/// Words broken into lines no wider than `width`, at spaces (and between
/// Chinese and Japanese characters), or inside a word too long for a line
/// (an address). Each paragraph starts a line; an empty one is a blank line.
fn wrap(g: &mut Gfx, text: &str, style: bezel::TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let wrapped = g.text.wrap(para, &style, width, true);
        if wrapped.is_empty() {
            lines.push(String::new());
        }
        lines.extend(wrapped);
    }
    lines
}

impl Screen for AboutApp {
    fn title(&self) -> &str {
        &self.title
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("reading", &[("↑ ↓", "Read on"), ("pg up · pg dn", "Page by page"), ("enter", "Done")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let last = self.lines.saturating_sub(self.shown);
        match key {
            Key::Back | Key::Enter => return Nav::Pop,
            Key::Up => self.first = self.first.saturating_sub(1),
            Key::Down => self.first = (self.first + 1).min(last),
            Key::PageUp => self.first = self.first.saturating_sub(self.shown.max(1)),
            Key::PageDown => self.first = (self.first + self.shown.max(1)).min(last),
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        let (top, bottom) = (g.top() + 2.0, g.above_hint());
        let lines = wrap(g, &self.text, ty.body, W - 2.0 * PAD - 12.0);
        let pitch = (ty.body.size * 1.32).round();
        self.lines = lines.len();
        self.shown = (((bottom - top) / pitch).floor() as usize).max(1);
        self.first = self.first.min(self.lines.saturating_sub(self.shown));
        for (i, line) in lines.iter().skip(self.first).take(self.shown).enumerate() {
            g.text_at(PAD + 2.0, top + pitch * (i as f32 + 0.5), Align::Left, line, ty.body, t.ink);
        }
        if self.lines > self.shown {
            let track = bottom - top;
            let thumb = (track * self.shown as f32 / self.lines as f32).max(10.0);
            let y = top + (track - thumb) * self.first as f32 / (self.lines - self.shown) as f32;
            g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, t.faint);
            g.hint_any(&["f x: read on · esc: back", "esc"]);
        } else {
            g.hint("esc: back");
        }
    }
}

// ------------------------------------------------------------ Spotlight ---

/// Spotlight finds what the catalogue offers and is not on this handheld
/// yet, from the index last read (no network): Enter opens its page here.
/// Installed apps are found as apps already.
pub struct Finder;

impl crate::spotlight::Provider for Finder {
    fn name(&self) -> &'static str {
        "catalogue"
    }

    fn items(&self) -> Vec<crate::spotlight::Item> {
        let dir = apps_dir();
        cached()
            .into_iter()
            .filter(|a| installed(&dir, &a.id).is_none() && !dir.join(&a.id).exists())
            .map(|a| {
                let icon = std::borrow::Cow::Owned(icon_of(&a).to_owned());
                crate::spotlight::Item::new(format!("get:{}", a.id), a.name.clone(), icon, crate::spotlight::weight::OTHER).detail("in the catalogue").words(format!("{} {}", a.about, a.id))
            })
            .collect()
    }

    fn stamp(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for path in [cache_path(), apps_dir()] {
            std::fs::metadata(&path).ok().map(|m| (m.modified().ok(), m.len())).hash(&mut h);
        }
        Some(h.finish())
    }

    fn open(&self, item: &crate::spotlight::Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let id = item.id.strip_prefix("get:").unwrap_or_default().to_owned();
        let apps = cached();
        if !apps.iter().any(|a| a.id == id) {
            return Err("it is no longer in the catalogue".into());
        }
        Ok(Box::new(AppPage::new(Shared::new(Job::start(move || Ok(apps))), id)))
    }
}

// ----------------------------------------------------------------- demo ---

/// The catalogue as the simulator's gallery shows it, with no network:
/// this project's own apps, at the sizes they pack to for the handheld.
fn demo() -> Vec<Listing> {
    let apps: [(&str, &str, Option<&str>, u64); 8] = [
        (include_str!("../../../../catalyst-apps/src/sudoku/app"), "sudoku", None, 1_382 * 1024),
        (include_str!("../../../../catalyst-apps/src/battleship/app"), "battleship", Some(include_str!("../../../../catalyst-apps/src/battleship/icon.svg")), 1_393 * 1024),
        (include_str!("../../../../catalyst-apps/src/typing/app"), "typing", Some(include_str!("../../../../catalyst-apps/src/typing/icon.svg")), 1_375 * 1024),
        (include_str!("../../../../catalyst-apps/src/orbits/app"), "orbits", Some(include_str!("../../../../catalyst-apps/src/orbits/icon.svg")), 1_387 * 1024),
        (include_str!("../../../../catalyst-apps/src/hello/app"), "hello", None, 1_363 * 1024),
        (include_str!("../../../../catalyst-apps/src/ping-robot/app"), "ping-robot", None, 2 * 1024),
        (include_str!("../../../../catalyst-apps/src/network/app"), "network", None, 2 * 1024),
        (include_str!("../../../../catalyst-apps/src/storage/app"), "storage", None, 2 * 1024),
    ];
    let listings: Vec<Listing> = apps
        .iter()
        .map(|(text, id, svg, bytes)| {
            let mut m = appkg::Manifest::parse(text);
            if m.id.is_empty() {
                m.id = (*id).to_owned();
            }
            let path = svg.and_then(bezel::icons::svg_path).unwrap_or_default();
            let mut l = Listing::of(&m, &path, "demo", &[], String::new());
            l.bytes = *bytes;
            l
        })
        .collect();
    register_icons(&listings);
    listings
}

/// The catalogue's screens for the gallery (`--catalogue-shots DIR`): the
/// list with one app on Home and one with an update, an app's page, one that
/// uses the network, an update that would use more, its longer words, and
/// a download under way.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &Path) {
    use Key::*;
    std::fs::create_dir_all(dir).expect("make the gallery's folder");
    let apps = std::env::temp_dir().join("catalyst-catalogue-gallery");
    std::fs::remove_dir_all(&apps).ok();
    // Hello on Home at its version; Ping the robot an older one, before it used the network.
    for (id, version, app) in [("hello", "1.0.0", include_str!("../../../../catalyst-apps/src/hello/app")), ("ping-robot", "0.9.0", "name = Ping the robot\nrun = sh ./run.sh\n")] {
        std::fs::create_dir_all(apps.join(id)).ok();
        std::fs::write(apps.join(id).join("app"), app).ok();
        std::fs::write(apps.join(id).join(".catalogue"), format!("id={id}\nversion={version}\n")).ok();
    }
    std::env::set_var("CATALYST_APPS", &apps);
    std::env::set_var("CATALYST_CATALOGUE", "demo");
    let mut shell = crate::shell::Shell::new(Robot::demo());
    shell.apply_settings();
    let settle = |shell: &mut crate::shell::Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let save = |shell: &mut crate::shell::Shell, name: &str| {
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    settle(&mut shell, 100);
    shell.show(Box::new(CatalogueScreen::new()));
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(15));
        settle(&mut shell, 6);
    }
    settle(&mut shell, 60);
    save(&mut shell, "91-catalogue");
    let steps: &[(&str, &[Key])] = &[
        ("91b-catalogue-app", &[Enter]),
        ("91c-catalogue-about", &[Down, Down, Enter]),
        ("91d-catalogue-network", &[Back, Back, Down, Down, Down, Down, Down, Enter]),
        ("91e-catalogue-update-uses-more", &[Enter]),
        ("91f-catalogue-installed", &[Back, Up, Up, Up, Up, Up, Up, Enter]),
    ];
    for (name, keys) in steps {
        for key in keys.iter() {
            shell.key(*key);
            settle(&mut shell, 40);
        }
        settle(&mut shell, 50);
        save(&mut shell, name);
    }
    // A download under way, as it shows on the list (the demo catalogue
    // fetches nothing: the job is one that has not finished).
    shell.key(Back);
    settle(&mut shell, 60);
    let list = CatalogueScreen::new();
    for _ in 0..50 {
        if list.state.borrow().index.get().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let sudoku = list.state.borrow().apps().into_iter().find(|a| a.id == "sudoku").expect("Sudoku is in the demo");
    list.state.borrow_mut().working = Some(Working {
        id: sudoku.id.clone(),
        name: sudoku.name.clone(),
        verb: "getting",
        job: Job::start(|| {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            Ok(())
        }),
    });
    shell.show(Box::new(list));
    settle(&mut shell, 110);
    save(&mut shell, "91g-catalogue-getting");
    println!("the catalogue drawn to {}", dir.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_that_would_write_outside_its_folder_is_refused() {
        assert!(paths_stay_inside("app\nping.sh\nicons/a.svg\n"));
        assert!(!paths_stay_inside("app\n../../etc/profile\n"));
        assert!(!paths_stay_inside("/etc/passwd\n"));
        assert!(!paths_stay_inside("app\nsub/../../x\n"));
        assert!(only_files_and_folders("drwxr-xr-x 0/0 0 2026-01-01 00:00 ./\n-rw-r--r-- 0/0 98 2026-01-01 00:00 ./app\n"));
        assert!(!only_files_and_folders("lrwxrwxrwx 0/0 0 2026-01-01 00:00 ./etc -> /etc\n"));
        assert!(!only_files_and_folders("hrw-r--r-- 0/0 0 2026-01-01 00:00 ./x link to ./app\n"));
    }

    #[test]
    fn the_demo_catalogue_lists_this_projects_apps_with_their_icons() {
        let apps = demo();
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"sudoku") && ids.contains(&"orbits") && ids.contains(&"ping-robot"));
        for app in &apps {
            assert!(bezel::Icons::has(icon_of(app)), "{}: {}", app.id, app.icon);
            assert!(app.about.chars().count() <= appkg::ABOUT_MAX && !app.licence.is_empty(), "{}", app.id);
        }
        assert!(apps.iter().find(|a| a.id == "ping-robot").unwrap().permissions.contains(&Permission::Network));
    }

    #[test]
    fn only_what_the_catalogue_installed_is_removed() {
        let dir = std::env::temp_dir().join(format!("catalyst-catalogue-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::create_dir_all(dir.join("fetched")).unwrap();
        std::fs::write(dir.join("fetched/.catalogue"), "id=fetched\nversion=1.2.0\n").unwrap();
        assert_eq!(installed(&dir, "fetched").as_deref(), Some("1.2.0"));
        assert!(remove("mine", &dir).is_err(), "an app put there by hand stays");
        assert!(remove("fetched", &dir).is_ok());
        assert!(dir.join("mine").exists() && !dir.join("fetched").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_update_says_what_it_would_use_that_the_installed_one_does_not() {
        let mut listing = demo().into_iter().find(|a| a.id == "ping-robot").unwrap();
        let shared = Shared::new(Job::start(|| Ok(Vec::new())));
        let dir = std::env::temp_dir().join(format!("catalyst-standing-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ping-robot")).unwrap();
        std::fs::write(dir.join("ping-robot/app"), "name = Ping\nrun = x\n").unwrap();
        std::fs::write(dir.join("ping-robot/.catalogue"), "version=0.9.0\n").unwrap();
        assert_eq!(shared.borrow().standing(&listing, &dir), Standing::Update(listing.version.clone()));
        assert_eq!(installed_permissions(&dir, "ping-robot"), []);
        listing.version = "0.9.0".into();
        assert_eq!(shared.borrow().standing(&listing, &dir), Standing::Installed("0.9.0".into()));
        listing.needs = "99.0".into();
        std::fs::remove_dir_all(dir.join("ping-robot")).unwrap();
        assert_eq!(shared.borrow().standing(&listing, &dir), Standing::NeedsSystem("99.0".into()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The whole of getting an app, with a stand-in `curl` that hands over a
    /// local package: a signed one goes in; one signed by an untrusted key,
    /// a tampered one, one holding a link, one climbing out of its folder
    /// and one that is another app are refused, and the installed one stays.
    #[cfg(target_os = "linux")]
    #[test]
    fn getting_an_app_checks_it_before_it_goes_in() {
        const SEED: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        const OTHER: &str = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";
        let keys = vec![appkg::public_of(SEED).unwrap()];
        let base = std::env::temp_dir().join(format!("catalyst-install-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let (bin, apps) = (base.join("bin"), base.join("apps"));
        for d in [&bin, &apps] {
            std::fs::create_dir_all(d).unwrap();
        }
        let package = |files: &[(&str, &str)]| {
            let entries: Vec<appkg::Entry> = files.iter().map(|(p, d)| appkg::Entry { path: (*p).into(), data: d.as_bytes().to_vec(), executable: p.ends_with(".sh") }).collect();
            appkg::pack(&entries).unwrap()
        };
        let curl = bin.join("curl");
        std::fs::write(&curl, "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=$2; shift; done\ncp \"$CATALYST_FAKE_ARCHIVE\" \"$out\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        let offer = |bytes: &[u8], version: &str, seed: &str| {
            let file = base.join(format!("offer-{version}.tar.gz"));
            std::fs::write(&file, bytes).unwrap();
            std::env::set_var("CATALYST_FAKE_ARCHIVE", &file);
            let sha = appkg::sha256_hex(bytes);
            let signature = appkg::sign(appkg::statement("hello", version, &sha).as_bytes(), seed).unwrap();
            Listing { id: "hello".into(), name: "Hello".into(), version: version.into(), archive: "https://example.invalid/hello.tar.gz".into(), sha256: sha, bytes: bytes.len() as u64, signature, ..demo()[0].clone() }
        };
        let good = package(&[("app", "id = hello\nname = Hello\nversion = 1.0.0\nrun = echo hi\n"), ("run.sh", "echo hi\n")]);
        install(&offer(&good, "1.0.0", SEED), &apps, &keys).unwrap();
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.0.0"));
        assert!(std::fs::read_to_string(apps.join("hello/.catalogue")).unwrap().contains(&format!("key={}", appkg::key_id(&keys[0]))));
        // Signed by a key the handheld does not trust: refused before downloading.
        let newer = package(&[("app", "id = hello\nname = Hello\nversion = 1.1.0\nrun = echo hi again\n")]);
        assert!(install(&offer(&newer, "1.1.0", OTHER), &apps, &keys).unwrap_err().contains("not signed"));
        // Tampered after signing: the hash no longer matches.
        let mut tampered = offer(&newer, "1.1.0", SEED);
        std::fs::write(base.join("offer-1.1.0.tar.gz"), package(&[("app", "id = hello\nversion = 1.1.0\nrun = rm -rf /data\n")])).unwrap();
        assert!(install(&tampered, &apps, &keys).unwrap_err().contains("did not match"));
        // Signed, but another app inside.
        let other_app = package(&[("app", "id = sudoku\nname = Sudoku\nversion = 1.1.0\nrun = x\n")]);
        assert!(install(&offer(&other_app, "1.1.0", SEED), &apps, &keys).unwrap_err().contains("not the app"));
        // A package holding a link out of its folder (made by tar, as appkg never would).
        let linked = base.join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("app"), "id = hello\nversion = 6.6.5\nrun = true\n").unwrap();
        std::os::unix::fs::symlink("/etc", linked.join("etc")).unwrap();
        let link_tar = base.join("link.tar.gz");
        Command::new("tar").arg("-czf").arg(&link_tar).arg("-C").arg(&linked).arg(".").status().unwrap();
        assert!(install(&offer(&std::fs::read(&link_tar).unwrap(), "6.6.5", SEED), &apps, &keys).unwrap_err().contains("outside"));
        // One whose path climbs out.
        std::fs::create_dir_all(base.join("deep/inner")).unwrap();
        std::fs::write(base.join("deep/evil"), "x").unwrap();
        let evil = base.join("evil.tar.gz");
        Command::new("tar").current_dir(base.join("deep/inner")).args(["-P", "-czf"]).arg(&evil).arg("../evil").status().unwrap();
        assert!(install(&offer(&std::fs::read(&evil).unwrap(), "6.6.6", SEED), &apps, &keys).unwrap_err().contains("outside"));
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.0.0"), "through all of that, 1.0.0 stays");
        // The update, signed: it takes the old one's place.
        tampered = offer(&newer, "1.1.0", SEED);
        install(&tampered, &apps, &keys).unwrap();
        assert_eq!(installed(&apps, "hello").as_deref(), Some("1.1.0"));
        assert!(std::fs::read_to_string(apps.join("hello/app")).unwrap().contains("again"));
        let leftovers: Vec<_> = std::fs::read_dir(&apps).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with('.')).collect();
        assert!(leftovers.is_empty(), "nothing left beside it: {leftovers:?}");
        std::fs::remove_dir_all(&base).ok();
    }
}
