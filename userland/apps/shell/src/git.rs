//! Git on the handheld: the repositories under the data partition, each with
//! its branch, how far it is from its upstream, what has changed (as diffs),
//! its history drawn with its branches, a commit's details, branches to
//! switch between, and pulling, committing and pushing; cloning by address,
//! or from the GitHub account signed in (github.rs).
//!
//! One repository can be marked as the robot's code. FrcCatalyst publishes
//! the commit the robot was built from (`Robot/Software/RobotCodeVersion`,
//! from the team's build stamp), and the Robot page says how that stands
//! against the repository here: the same, so many commits behind, or built
//! from changes that were never committed. A repository with a `gradlew`
//! offers a Deploy Action.
//!
//! Git runs in a background thread: a status on a slow card never holds up
//! a frame. Its credentials for GitHub come from the credential helper named
//! in /etc/gitconfig; it never asks on a terminal that is not there.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::github::RemoteRepo;
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// Where repositories are kept and looked for.
pub fn code_dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        "/data/code".into()
    } else {
        crate::config_dir().join("code")
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        // Never ask for a password on a terminal that is not there.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "git is not installed".to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(last_line(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// Git's last word on what went wrong.
fn last_line(stderr: &str) -> String {
    stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("git failed").trim().trim_start_matches("fatal: ").trim_start_matches("error: ").to_owned()
}

/// A command run off the drawing thread; its answer arrives later.
#[derive(Clone, Default)]
pub struct Job<T: Clone + Send + 'static>(Arc<Mutex<Option<T>>>);

impl<T: Clone + Send + 'static> Job<T> {
    pub fn start(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        std::thread::Builder::new()
            .name("git".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let answer = work();
                *out.lock().unwrap() = Some(answer);
            })
            .ok();
        Job(slot)
    }

    pub fn get(&self) -> Option<T> {
        self.0.lock().unwrap().clone()
    }
}

/// Repositories under `root`, two folders deep at most.
pub fn find_repos(root: &Path) -> Vec<PathBuf> {
    fn look(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for path in dirs {
            let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if path.join(".git").exists() {
                found.push(path);
            } else if depth > 0 && !hidden {
                look(&path, depth - 1, found);
            }
        }
    }
    let mut found = Vec::new();
    look(root, 1, &mut found);
    found
}

/// The state of a repository as its screen shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Changed files: two-letter status and path.
    pub changes: Vec<(String, String)>,
}

/// `git status --porcelain=v1 --branch`.
pub fn parse_status(text: &str) -> Status {
    let mut s = Status::default();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            // "main...origin/main [ahead 1, behind 2]", or "No commits yet on main".
            let (names, counts) = head.split_once(" [").unwrap_or((head, ""));
            let (branch, upstream) = names.split_once("...").map_or((names, None), |(b, u)| (b, Some(u.to_owned())));
            s.branch = branch.trim_start_matches("No commits yet on ").to_owned();
            s.upstream = upstream;
            for part in counts.trim_end_matches(']').split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.len() > 3 {
            s.changes.push((line[..2].to_owned(), line[3..].to_owned()));
        }
    }
    s
}

/// One commit in the history: short hash, when, who, and its subject (the
/// plain log, without the graph; the tests read history this way).
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub when: String,
    pub author: String,
    pub subject: String,
}

/// `git log --format=%h%x1f%cr%x1f%an%x1f%s`.
#[cfg(test)]
pub fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\u{1f}');
            Some(Commit { hash: f.next()?.into(), when: f.next()?.into(), author: f.next()?.into(), subject: f.next()?.into() })
        })
        .collect()
}

// --------------------------------------------------------------- history ---

/// The format the history is read in: after git's graph, the full and short
/// hash, when (seconds), who, the subject, and the branches pointing at it.
const GRAPH_FORMAT: &str = "--format=%x1f%H%x1f%h%x1f%ct%x1f%an%x1f%s%x1f%D";

/// One line of `git log --graph`: its lanes (git's `*`, `|`, `/`, `\`, `_`
/// as drawn), and the commit on it, if it is one (else it only joins lanes).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphRow {
    pub lanes: String,
    pub commit: Option<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub hash: String,
    pub short: String,
    pub time: i64,
    pub author: String,
    pub subject: String,
    /// The branches and tags here, `HEAD -> ` taken off.
    pub refs: Vec<String>,
}

/// `git log --graph` in GRAPH_FORMAT.
pub fn parse_graph(text: &str) -> Vec<GraphRow> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| match line.split_once('\u{1f}') {
            None => GraphRow { lanes: line.trim_end().to_owned(), commit: None },
            Some((lanes, rest)) => {
                let f: Vec<&str> = rest.split('\u{1f}').collect();
                let field = |i: usize| f.get(i).copied().unwrap_or_default();
                let refs = field(5)
                    .split(", ")
                    .map(|r| r.trim().trim_start_matches("HEAD -> ").to_owned())
                    .filter(|r| !r.is_empty() && r != "HEAD" && !r.ends_with("/HEAD"))
                    .collect();
                GraphRow {
                    lanes: lanes.trim_end().to_owned(),
                    commit: Some(Entry { hash: field(0).into(), short: field(1).into(), time: field(2).parse().unwrap_or(0), author: field(3).into(), subject: field(4).into(), refs }),
                }
            }
        })
        .collect()
}

/// How long ago, in a few letters: "now", "5 min", "3 h", "2 d", "nov 20".
pub fn ago(time: i64, now: i64) -> String {
    match now - time {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{} min", s / 60),
        s if s < 86_400 => format!("{} h", s / 3600),
        s if s < 14 * 86_400 => format!("{} d", s / 86_400),
        _ => chrono::DateTime::from_timestamp(time, 0).map_or_else(String::new, |d| d.format("%b %-d").to_string().to_lowercase()),
    }
}

// --------------------------------------------------------------- branches ---

/// A branch: its name, whether it is the one checked out, whether it is only
/// on the remote (`origin/x` with no local `x`), and its upstream.
#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub remote_only: bool,
    pub upstream: Option<String>,
}

/// `git branch -a --format=%(HEAD)%1f%(refname)%1f%(upstream:short)`: local
/// branches first, then those on a remote that have no local branch.
pub fn parse_branches(text: &str) -> Vec<Branch> {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        let (head, refname, upstream) = (f.first().copied().unwrap_or(""), f.get(1).copied().unwrap_or(""), f.get(2).copied().unwrap_or(""));
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            local.push(Branch { name: name.into(), current: head.trim() == "*", remote_only: false, upstream: (!upstream.is_empty()).then(|| upstream.to_owned()) });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            if !name.ends_with("/HEAD") && name.contains('/') {
                remote.push(name.to_owned());
            }
        }
    }
    // The one checked out first; the others as git lists them.
    local.sort_by_key(|b| !b.current);
    for r in remote {
        let short = r.split_once('/').map_or(r.as_str(), |(_, b)| b).to_owned();
        if !local.iter().any(|b| b.name == short) {
            local.push(Branch { name: r, current: false, remote_only: true, upstream: None });
        }
    }
    local
}

// ----------------------------------------------------------------- clone ---

/// How a clone is going: git's stage ("receiving objects") and percent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub stage: String,
    pub percent: u32,
}

/// One line of `git clone --progress`'s output ("Receiving objects:  45% (450/1000)").
pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.trim().trim_start_matches("remote: ");
    let (stage, rest) = line.split_once(':')?;
    let percent = rest.trim().split('%').next()?.trim().parse().ok()?;
    Some(Progress { stage: stage.trim().to_lowercase(), percent })
}

/// Clone `url` into `dest`, telling `progress` as git does. On the simulator,
/// one of the made-up account's repositories is made here instead.
fn clone(url: &str, dest: &Path, progress: &Arc<Mutex<Progress>>) -> Result<PathBuf, String> {
    // An address, never something git would take as an option.
    if url.starts_with('-') {
        return Err("that is not an address".into());
    }
    if dest.exists() {
        return Err(format!("{} is here already", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    std::fs::create_dir_all(dest.parent().unwrap_or(dest)).map_err(|e| e.to_string())?;
    if !crate::sys::on_handheld() && url.starts_with("https://github.com/team5805/") {
        for percent in [10, 35, 70, 100] {
            *progress.lock().unwrap() = Progress { stage: "receiving objects".into(), percent };
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        crate::github::make_demo_repo(dest)?;
        return Ok(dest.to_owned());
    }
    let mut child = Command::new("git")
        .args(["clone", "--progress", "--", url])
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "git is not installed".to_owned())?;
    let mut stderr = child.stderr.take().ok_or("git did not start")?;
    // Git rewrites its progress line with \r; each piece is read as it comes.
    let (mut seen, mut buf, mut chunk) = (String::new(), Vec::new(), [0u8; 512]);
    while let Ok(n) = stderr.read(&mut chunk) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        while let Some(at) = buf.iter().position(|b| *b == b'\r' || *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=at).collect();
            let line = String::from_utf8_lossy(&line).trim().to_owned();
            if let Some(p) = parse_progress(&line) {
                *progress.lock().unwrap() = p;
            } else if !line.is_empty() {
                seen = line;
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(dest.to_owned())
    } else {
        std::fs::remove_dir_all(dest).ok();
        Err(last_line(&seen))
    }
}

/// A clone under way: its progress, and its answer once done.
struct Cloning {
    name: String,
    progress: Arc<Mutex<Progress>>,
    job: Job<Result<PathBuf, String>>,
}

impl Cloning {
    fn start(url: &str) -> Cloning {
        let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or("repo").trim_end_matches(".git").to_owned();
        let name = if name.is_empty() { "repo".to_owned() } else { name };
        let progress = Arc::new(Mutex::new(Progress::default()));
        let (url, dest, p) = (url.to_owned(), code_dir().join(&name), progress.clone());
        Cloning { name, progress, job: Job::start(move || clone(&url, &dest, &p)) }
    }

    /// "cloning 45%", for the row it is on.
    fn words(&self) -> String {
        let p = self.progress.lock().unwrap().clone();
        if p.stage.is_empty() {
            "cloning…".into()
        } else {
            format!("{} {}%", if p.stage.starts_with("resolving") { "resolving" } else { "cloning" }, p.percent)
        }
    }
}

// --------------------------------------------------------- robot's code ---

/// How the robot's code stands against the repository marked as its code.
/// `version` is what the robot publishes: a commit hash, perhaps with
/// `-dirty` after it.
pub fn compare(repo: &Path, version: &str) -> String {
    let dirty = version.ends_with("-dirty");
    let hash = version.trim_end_matches("-dirty").trim();
    // Only a commit hash is compared; anything else is shown as it is. (And
    // only once it is known to be ASCII is it cut by bytes.)
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return version.to_owned();
    }
    let short = &hash[..hash.len().min(7)];
    let note = if dirty { " + uncommitted" } else { "" };
    if git(repo, &["cat-file", "-e", &format!("{hash}^{{commit}}")]).is_err() {
        return format!("{short}{note} · not in this repo");
    }
    let head = git(repo, &["rev-parse", "HEAD"]).unwrap_or_default();
    if head.trim().starts_with(hash) {
        return format!("{short}{note} · the latest");
    }
    match git(repo, &["rev-list", "--count", &format!("{hash}..HEAD")]).ok().and_then(|n| n.trim().parse::<u32>().ok()) {
        Some(0) => format!("{short}{note} · ahead of here"),
        Some(n) => format!("{short}{note} · {n} behind"),
        None => format!("{short}{note}"),
    }
}

/// The repository marked as the robot's code, if any.
pub fn robot_repo() -> Option<PathBuf> {
    let text = std::fs::read_to_string(crate::config_dir().join("robot-repo")).ok()?;
    Some(PathBuf::from(text.trim())).filter(|p| p.join(".git").exists())
}

fn set_robot_repo(repo: Option<&Path>) {
    let file = crate::config_dir().join("robot-repo");
    match repo {
        Some(repo) => {
            crate::sys::write_atomic(&file, repo.to_string_lossy().as_bytes()).ok();
        }
        None => {
            std::fs::remove_file(file).ok();
        }
    }
    forget_comparison();
}

/// The comparison for the Robot page, worked out once for each repository
/// and version and kept until the repository changes (a pull, or another
/// marked as the robot's code).
static KEPT: Mutex<Option<(PathBuf, String, Job<String>)>> = Mutex::new(None);

pub fn robot_code_line(version: &str) -> Option<String> {
    let repo = robot_repo()?;
    let mut kept = KEPT.lock().unwrap();
    if kept.as_ref().map(|(r, v, _)| (r, v.as_str())) != Some((&repo, version)) {
        let (at, v) = (repo.clone(), version.to_owned());
        *kept = Some((repo, version.to_owned(), Job::start(move || compare(&at, &v))));
    }
    kept.as_ref().and_then(|(.., job)| job.get())
}

/// The repository changed: the comparison is worked out again.
fn forget_comparison() {
    *KEPT.lock().unwrap() = None;
}

/// The Action that deploys a repository's robot code with its gradlew.
fn deploy_action(repo: &Path) -> crate::commands::Saved {
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    crate::commands::Saved::new(&format!("Deploy {name}"), &format!("cd {} && ./gradlew deploy", repo.display()))
}

/// Whether that Action is among the team's own already.
fn has_deploy_action(repo: &Path) -> bool {
    let wanted = deploy_action(repo);
    crate::commands::load().iter().any(|s| s.command == wanted.command)
}

// -------------------------------------------------------------- screens ---

/// The repositories on the handheld, the account's, and cloning by address.
pub struct GitScreen {
    list: ListState,
    repos: Vec<PathBuf>,
    /// The address being typed for a clone, and a clone under way.
    typing: Option<Field>,
    cloning: Option<Cloning>,
}

/// The rows before the repositories: the account, and its repositories.
const HEAD_ROWS: usize = 2;

impl GitScreen {
    pub fn new() -> Self {
        GitScreen { list: ListState::default(), repos: find_repos(&code_dir()), typing: None, cloning: None }
    }
}

impl Default for GitScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for GitScreen {
    fn title(&self) -> &str {
        "Git"
    }

    fn wants_text(&self) -> bool {
        self.typing.is_some()
    }

    fn resume(&mut self) {
        self.repos = find_repos(&code_dir());
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.typing {
            match field.key(key, |c| !c.is_whitespace()) {
                Typed::Done(url) => {
                    self.typing = None;
                    if url.starts_with('-') || !url.contains("://") && !url.contains('@') {
                        *cx.say = Some("that is not an address".into());
                    } else if self.cloning.is_none() {
                        self.cloning = Some(Cloning::start(&url));
                    }
                }
                Typed::Cancelled => self.typing = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let count = HEAD_ROWS + self.repos.len() + 1;
        let at = self.list.focus;
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right if at == 0 => return Nav::Push(Box::new(AccountScreen::new())),
            Key::Enter | Key::Right if at == 1 => return Nav::Push(Box::new(RemoteReposScreen::new())),
            Key::Enter | Key::Right if at == count - 1 => self.typing = Some(Field::new("address of the repository", "https://github.com/", 200)),
            Key::Enter | Key::Right => {
                if let Some(repo) = self.repos.get(at - HEAD_ROWS) {
                    return Nav::Push(Box::new(RepoScreen::new(repo.clone())));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, count),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(done) = self.cloning.as_ref().and_then(|c| c.job.get()) else { return Nav::Stay };
        self.cloning = None;
        self.repos = find_repos(&code_dir());
        *cx.say = Some(match done {
            Ok(path) => format!("cloned {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Err(why) => why,
        });
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.cloning.is_some()
    }

    fn busy(&self) -> bool {
        self.cloning.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some(field) = &self.typing {
            return field.draw(g, "enter: clone · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let rows = HEAD_ROWS + self.repos.len() + 1;
        let robots = robot_repo();
        let account = crate::github::account();
        for (i, y) in self.list.layout(g, rows, top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match i {
                0 => {
                    let value = match (&account, crate::github::token().is_some()) {
                        (Some(a), _) => a.login.clone(),
                        (None, true) => "sign in".into(),
                        (None, false) => "add a key".into(),
                    };
                    g.row(r, "Account", &value, if account.is_some() { t.ink } else { t.dim }, focused);
                }
                1 => g.row(r, "Your repositories", "›", t.dim, focused),
                i if i == rows - 1 => {
                    let words = self.cloning.as_ref().map_or_else(|| "›".to_owned(), Cloning::words);
                    g.row(r, "Clone by address", &words, t.dim, focused);
                }
                i => {
                    let repo = &self.repos[i - HEAD_ROWS];
                    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mark = if robots.as_deref() == Some(repo.as_path()) { "robot code" } else { "" };
                    g.row(r, &name, mark, t.dim, focused);
                }
            }
        }
        self.list.scrollbar(g, rows, top, bottom);
        let line = match self.list.focus {
            0 => "github: your account, its repositories, and pushing".to_owned(),
            1 => "clone one of your account's repositories".to_owned(),
            _ if self.repos.is_empty() => "no repositories here yet".to_owned(),
            _ => {
                let place = code_dir();
                if place.starts_with("/data") { format!("repositories live in {}", place.display()) } else { "repositories live beside the settings".to_owned() }
            }
        };
        let fitted = g.text.fit(&line, &g.ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// The GitHub account: who the key belongs to, and signing in and out.
pub struct AccountScreen {
    list: ListState,
    signing_in: Option<Job<Result<crate::github::Account, String>>>,
}

impl AccountScreen {
    fn new() -> Self {
        AccountScreen { list: ListState::default(), signing_in: None }
    }

    /// The rows as things stand: (name, value), and what Enter on each does.
    fn rows(&self) -> Vec<(&'static str, String, Do)> {
        let busy = self.signing_in.is_some();
        match (crate::github::account(), crate::github::token().is_some()) {
            (Some(a), _) => vec![
                ("Signed in as", a.login, Do::Nothing),
                ("Name", a.name, Do::Nothing),
                ("Commits as", a.email, Do::Nothing),
                ("Sign in again", (if busy { "asking GitHub…" } else { "" }).into(), Do::SignIn),
                ("Sign out", String::new(), Do::SignOut),
            ],
            (None, true) => vec![("GitHub key", crate::keys::Keys::load().shown("github"), Do::Keys), ("Sign in", (if busy { "asking GitHub…" } else { "›" }).into(), Do::SignIn)],
            (None, false) => vec![("GitHub key", "not set".into(), Do::Keys)],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Do {
    Nothing,
    SignIn,
    SignOut,
    Keys,
}

impl Screen for AccountScreen {
    fn title(&self) -> &str {
        "Account"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = self.rows();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right => match rows.get(self.list.focus).map(|r| r.2) {
                Some(Do::SignIn) if self.signing_in.is_none() => self.signing_in = Some(Job::start(crate::github::sign_in)),
                Some(Do::SignOut) => {
                    crate::github::sign_out();
                    self.list.focus = 0;
                    *cx.say = Some("signed out · the key stays in settings › keys".into());
                }
                Some(Do::Keys) => return Nav::Push(Box::new(crate::screens::SettingsApp::keys_page())),
                _ => {}
            },
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.signing_in.as_ref().and_then(Job::get) {
            self.signing_in = None;
            *cx.say = Some(match done {
                Ok(a) => format!("signed in as {}", a.login),
                Err(why) => why,
            });
            self.list.focus = 0;
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.signing_in.is_some()
    }

    fn busy(&self) -> bool {
        self.signing_in.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let rows = self.rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value, what) = &rows[i];
            let room = r.w - g.measure(name, g.ty.body) - 34.0;
            let value = g.text.fit(value, &style, room.max(30.0));
            g.row(r, name, &value, if *what == Do::Nothing { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let hint = match rows.get(self.list.focus).map(|r| r.2) {
            Some(Do::Keys) => "enter: settings › keys · or set up from a computer",
            Some(Do::SignIn) => "asks github whose key it is",
            Some(Do::SignOut) => "the key stays in settings › keys",
            _ => "a token with repository access",
        };
        g.hint(hint);
    }
}

/// The account's repositories on GitHub: cloning one, or opening it when
/// it is here already.
pub struct RemoteReposScreen {
    list: ListState,
    repos: Job<Result<Vec<RemoteRepo>, String>>,
    cloning: Option<(usize, Cloning)>,
}

impl RemoteReposScreen {
    fn new() -> Self {
        RemoteReposScreen { list: ListState::default(), repos: Job::start(crate::github::repos), cloning: None }
    }

    fn here(repo: &RemoteRepo) -> PathBuf {
        code_dir().join(&repo.name)
    }
}

impl Screen for RemoteReposScreen {
    fn title(&self) -> &str {
        "Your repositories"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let repos = self.repos.get().and_then(Result::ok).unwrap_or_default();
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('r') => self.repos = Job::start(crate::github::repos),
            Key::Enter | Key::Right => {
                let Some(repo) = repos.get(self.list.focus) else {
                    if crate::github::token().is_none() {
                        return Nav::Push(Box::new(crate::screens::SettingsApp::keys_page()));
                    }
                    return Nav::Stay;
                };
                let here = Self::here(repo);
                if here.join(".git").exists() {
                    return Nav::Push(Box::new(RepoScreen::new(here)));
                }
                if self.cloning.is_some() {
                    *cx.say = Some("one clone at a time".into());
                } else {
                    self.cloning = Some((self.list.focus, Cloning::start(&repo.clone_url)));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, repos.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.cloning.as_ref().and_then(|(_, c)| c.job.get()) {
            let name = self.cloning.take().map(|(_, c)| c.name).unwrap_or_default();
            *cx.say = Some(match done {
                Ok(_) => format!("cloned {name}"),
                Err(why) => why,
            });
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.cloning.is_some() || self.repos.get().is_none()
    }

    fn busy(&self) -> bool {
        self.cloning.is_some() || self.repos.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let repos = match self.repos.get() {
            None => return g.empty("sync", "asking github…"),
            Some(Err(why)) => {
                let line = g.text.fit(&why, &g.ty.caption, W - 2.0 * PAD);
                g.empty("cloud_off", &line);
                return g.hint(if crate::github::token().is_none() { "enter: settings › keys" } else { "r: ask again" });
            }
            Some(Ok(repos)) if repos.is_empty() => return g.empty("folder", "no repositories on this account yet"),
            Some(Ok(repos)) => repos,
        };
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        for (i, y) in self.list.layout(g, repos.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let repo = &repos[i];
            let value = match &self.cloning {
                Some((at, c)) if *at == i => c.words(),
                _ if Self::here(repo).join(".git").exists() => "here".to_owned(),
                _ => {
                    let day = crate::github::day(&repo.updated);
                    [repo.private.then_some("private".to_owned()), (!day.is_empty()).then_some(day)].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                }
            };
            let value_w = g.measure(&value, g.ty.label);
            let name = g.text.fit(&repo.name, &g.ty.body, r.w - value_w - 40.0);
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, repos.len(), top, bottom);
        let hint = repos.get(self.list.focus).map_or_else(String::new, |r| {
            let about = if r.description.is_empty() { r.full_name.clone() } else { r.description.clone() };
            let verb = if Self::here(r).join(".git").exists() { "enter: open" } else { "enter: clone" };
            format!("{verb} · {about}")
        });
        let fitted = g.text.fit(&hint, &g.ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// One repository: pulling, pushing, committing, its changes, history and
/// branches, whether it is the robot's code, and a Deploy Action.
pub struct RepoScreen {
    repo: PathBuf,
    name: String,
    list: ListState,
    status: Job<Result<Status, String>>,
    /// Pull, push or commit under way: what it is, and its answer.
    working: Option<(Work, Job<Result<String, String>>)>,
    /// The message being typed for a commit.
    message: Option<Field>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Work {
    Pull,
    Push,
    Commit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Pull,
    Push,
    Commit,
    Changes,
    History,
    Branches,
    RobotCode,
    Deploy,
}

impl RepoScreen {
    pub fn new(repo: PathBuf) -> Self {
        let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repository".into());
        let status = Self::read(&repo);
        RepoScreen { repo, name, list: ListState::default(), status, working: None, message: None }
    }

    fn read(repo: &Path) -> Job<Result<Status, String>> {
        let repo = repo.to_owned();
        Job::start(move || git(&repo, &["status", "--porcelain=v1", "--branch"]).map(|t| parse_status(&t)))
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Pull, Row::Push, Row::Commit, Row::Changes, Row::History, Row::Branches, Row::RobotCode];
        if self.repo.join("gradlew").exists() {
            rows.push(Row::Deploy);
        }
        rows
    }

    fn start(&mut self, work: Work, cx: &mut Cx) {
        if self.working.is_some() {
            return;
        }
        let repo = self.repo.clone();
        let status = self.status.get().and_then(Result::ok).unwrap_or_default();
        let job = match work {
            Work::Pull => Job::start(move || git(&repo, &["pull", "--ff-only"])),
            // A branch with nowhere to push to yet gets one on origin.
            Work::Push if status.upstream.is_none() => Job::start(move || git(&repo, &["push", "-u", "origin", "HEAD"])),
            Work::Push => Job::start(move || git(&repo, &["push"])),
            Work::Commit => return,
        };
        *cx.say = Some(if work == Work::Pull { "pulling…" } else { "pushing…" }.into());
        self.working = Some((work, job));
    }

    fn commit(&mut self, message: String) {
        let repo = self.repo.clone();
        let (name, email) = crate::github::identity();
        self.working = Some((
            Work::Commit,
            Job::start(move || {
                git(&repo, &["add", "-A"])?;
                git(&repo, &["-c", &format!("user.name={name}"), "-c", &format!("user.email={email}"), "commit", "-q", "-m", &message])
            }),
        ));
    }
}

impl Screen for RepoScreen {
    fn title(&self) -> &str {
        &self.name
    }

    fn wants_text(&self) -> bool {
        self.message.is_some()
    }

    fn resume(&mut self) {
        self.status = Self::read(&self.repo);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.message {
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) if text.is_empty() => *cx.say = Some("a commit needs a message".into()),
                Typed::Done(text) => {
                    self.message = None;
                    self.commit(text);
                }
                Typed::Cancelled => self.message = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let rows = self.rows();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right => match rows.get(self.list.focus) {
                Some(Row::Pull) => self.start(Work::Pull, cx),
                Some(Row::Push) => self.start(Work::Push, cx),
                Some(Row::Commit) => {
                    let changes = self.status.get().and_then(Result::ok).map_or(0, |s| s.changes.len());
                    if changes == 0 {
                        *cx.say = Some("nothing to commit".into());
                    } else if self.working.is_none() {
                        self.message = Some(Field::new("what this commit does", "", 200));
                    }
                }
                Some(Row::Changes) => {
                    let changes = self.status.get().and_then(Result::ok).map(|s| s.changes).unwrap_or_default();
                    return Nav::Push(Box::new(TextScreen::changes(&self.repo, &changes)));
                }
                Some(Row::History) => return Nav::Push(Box::new(HistoryScreen::new(&self.repo))),
                Some(Row::Branches) => return Nav::Push(Box::new(BranchesScreen::new(&self.repo))),
                Some(Row::RobotCode) => {
                    let mine = robot_repo().as_deref() == Some(self.repo.as_path());
                    set_robot_repo((!mine).then_some(self.repo.as_path()));
                    *cx.say = Some(if mine { "no longer the robot's code".into() } else { "marked as the robot's code".into() });
                }
                Some(Row::Deploy) => {
                    if has_deploy_action(&self.repo) {
                        *cx.say = Some("it is in actions already".into());
                    } else {
                        let mut saved = crate::commands::load();
                        let action = deploy_action(&self.repo);
                        let name = action.name.to_lowercase();
                        saved.push(action);
                        *cx.say = Some(if crate::commands::save(&saved) { format!("added {name} to actions") } else { "could not keep it".into() });
                    }
                }
                None => {}
            },
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.working.as_ref().and_then(|(_, job)| job.get()) {
            let work = self.working.take().map(|(w, _)| w);
            forget_comparison();
            *cx.say = Some(match (work, done) {
                (Some(Work::Pull), Ok(out)) if out.contains("Already up to date") => "already up to date".into(),
                (Some(Work::Pull), Ok(_)) => "pulled".into(),
                (Some(Work::Push), Ok(_)) => "pushed".into(),
                (Some(Work::Commit), Ok(_)) => "committed".into(),
                (_, Ok(_)) => "done".into(),
                (_, Err(why)) => why,
            });
            self.status = Self::read(&self.repo);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.working.is_some() || self.status.get().is_none()
    }

    fn busy(&self) -> bool {
        self.working.is_some() || self.status.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.message {
            return field.draw(g, "enter: commit every change · esc: cancel");
        }
        let top = g.top();
        let status = self.status.get();
        let head = match &status {
            None => "reading…".to_owned(),
            Some(Err(why)) => why.clone(),
            Some(Ok(s)) => {
                let tracking = match (&s.upstream, s.ahead, s.behind) {
                    (None, ..) => "no upstream".to_owned(),
                    (Some(_), 0, 0) => "up to date".to_owned(),
                    (Some(_), a, b) => [(a > 0).then(|| format!("{a} to push")), (b > 0).then(|| format!("{b} to pull"))].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                };
                format!("{} · {tracking}", s.branch)
            }
        };
        g.text_fit(PAD + 2.0, top + ty.caption.size * 0.5, &head, ty.caption, t.dim, W - 2.0 * PAD);
        let list_top = top + ty.caption.size + 6.0;
        let bottom = H - PAD - g.hint_h();
        let row_h = g.row_h();
        let rows = self.rows();
        let mine = robot_repo().as_deref() == Some(self.repo.as_path());
        let s = status.as_ref().and_then(|s| s.as_ref().ok()).cloned().unwrap_or_default();
        let doing = self.working.as_ref().map(|(w, _)| *w);
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match rows[i] {
                Row::Pull if doing == Some(Work::Pull) => ("Pull", "pulling…".to_owned()),
                Row::Pull if s.behind > 0 => ("Pull", format!("{} new", s.behind)),
                Row::Pull => ("Pull", "fast-forward".to_owned()),
                Row::Push if doing == Some(Work::Push) => ("Push", "pushing…".to_owned()),
                Row::Push if s.ahead > 0 => ("Push", format!("{} to send", s.ahead)),
                Row::Push => ("Push", if s.upstream.is_none() { "to origin" } else { "nothing new" }.to_owned()),
                Row::Commit if doing == Some(Work::Commit) => ("Commit", "committing…".to_owned()),
                Row::Commit => ("Commit", if s.changes.is_empty() { "nothing to".into() } else { "every change ›".into() }),
                Row::Changes => ("Changes", if s.changes.is_empty() { "none".into() } else { s.changes.len().to_string() }),
                Row::History => ("History", "›".to_owned()),
                Row::Branches => ("Branches", if s.branch.is_empty() { "›".into() } else { s.branch.clone() }),
                Row::RobotCode => ("The robot's code", (if mine { "yes" } else { "no" }).to_owned()),
                Row::Deploy => ("Deploy as an Action", (if has_deploy_action(&self.repo) { "in actions" } else { "add" }).to_owned()),
            };
            let value = g.text.fit(&value, &ty.label, r.w * 0.5);
            g.row(r, name, &value, if rows[i] == Row::RobotCode && mine { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom);
        // The robot's build against this repository, when it is the robot's code.
        let line = match (rows.get(self.list.focus), mine, robot.string("Robot/Software/RobotCodeVersion")) {
            (Some(Row::Deploy), ..) => "an action: cd here, ./gradlew deploy · asks before it runs".to_owned(),
            (_, true, Some(version)) => robot_code_line(&version).map_or_else(|| "on the robot: comparing…".into(), |l| format!("on the robot: {l}")),
            (_, true, None) => "the robot does not say its version".into(),
            _ => self.repo.display().to_string(),
        };
        let fitted = g.text.fit(&line, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// The history, drawn with its branches: newest first, each commit with its
/// lane, the branches at it, its subject and when; Enter opens one.
pub struct HistoryScreen {
    repo: PathBuf,
    rows: Job<Result<Vec<GraphRow>, String>>,
    /// The row in focus (always a commit's), and the page scrolled, gliding.
    focus: usize,
    scroll: Animated,
    now: i64,
}

/// Lanes are this far apart, half a lane per character of git's graph.
const LANE: f32 = 9.0;
/// Rows that only join lanes are this much of a commit's row.
const JOIN: f32 = 0.5;
/// How far from the page's edges a row fades out.
const EDGE: f32 = 12.0;

impl HistoryScreen {
    fn new(repo: &Path) -> Self {
        let at = repo.to_owned();
        let rows = Job::start(move || git(&at, &["log", "--graph", "--date-order", "-n", "300", GRAPH_FORMAT, "--no-color"]).map(|t| parse_graph(&t)));
        HistoryScreen { repo: repo.to_owned(), rows, focus: 0, scroll: Animated::new(0.0, role::settle()).with_rest(0.3), now: chrono::Utc::now().timestamp() }
    }

    /// The next commit row from `at` in `step`'s direction, if any.
    fn step(rows: &[GraphRow], at: usize, step: isize) -> Option<usize> {
        let mut i = at as isize + step;
        while i >= 0 && (i as usize) < rows.len() {
            if rows[i as usize].commit.is_some() {
                return Some(i as usize);
            }
            i += step;
        }
        None
    }
}

impl Screen for HistoryScreen {
    fn title(&self) -> &str {
        "History"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let Some(Ok(rows)) = self.rows.get() else {
            return if key == Key::Back { Nav::Pop } else { Nav::Stay };
        };
        // So many commits on, or as far as there are.
        let on = |from: usize, n: isize| (0..n.unsigned_abs()).fold(from, |at, _| Self::step(&rows, at, n.signum()).unwrap_or(at));
        match key {
            Key::Back => return Nav::Pop,
            Key::Up => self.focus = on(self.focus, -1),
            Key::Down => self.focus = on(self.focus, 1),
            Key::PageUp => self.focus = on(self.focus, -6),
            Key::PageDown => self.focus = on(self.focus, 6),
            Key::Enter | Key::Right => {
                if let Some(c) = rows.get(self.focus).and_then(|r| r.commit.as_ref()) {
                    return Nav::Push(Box::new(TextScreen::commit(&self.repo, &c.hash, &c.short)));
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt) || self.rows.get().is_none()
    }

    fn busy(&self) -> bool {
        self.rows.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = match self.rows.get() {
            None => return g.empty("history", "reading the history…"),
            Some(Err(why)) => {
                let line = g.text.fit(&why, &ty.caption, W - 2.0 * PAD);
                return g.empty("history", &line);
            }
            Some(Ok(rows)) if rows.is_empty() => return g.empty("history", "no commits yet"),
            Some(Ok(rows)) => rows,
        };
        if rows.get(self.focus).is_none_or(|r| r.commit.is_none()) {
            self.focus = Self::step(&rows, 0, 1).filter(|_| rows[0].commit.is_none()).unwrap_or(0);
        }
        let top = g.top();
        let bottom = H - PAD - g.hint_h();
        let row_h = ty.body.size + 9.0;
        let height = |r: &GraphRow| if r.commit.is_some() { row_h } else { row_h * JOIN };
        // Where each row starts, from the top of the history.
        let mut starts = Vec::with_capacity(rows.len());
        let mut y = 0.0;
        for r in &rows {
            starts.push(y);
            y += height(r);
        }
        let total = y;
        let view = bottom - top;
        // The focused commit kept in view, the page gliding to it.
        let (fy, fh) = (starts[self.focus], height(&rows[self.focus]));
        let mut target = self.scroll.target();
        if fy < target + row_h {
            target = (fy - row_h).max(0.0);
        } else if fy + fh > target + view - row_h {
            target = (fy + fh + row_h - view).min((total - view).max(0.0));
        }
        if (target - self.scroll.target()).abs() > 0.5 {
            self.scroll.set(target);
        }
        if g.calm {
            self.scroll.jump(self.scroll.target());
        }
        let scroll = self.scroll.value();
        // The graph's width: its widest row's lanes, at most six lanes.
        let lanes_w = rows.iter().map(|r| r.lanes.chars().count()).max().unwrap_or(1).min(12) as f32 * (LANE * 0.5) + 6.0;
        let x0 = PAD + 6.0;
        let text_x = x0 + lanes_w + 4.0;
        let palette = [t.signal, t.ice, t.amber, t.dim];
        g.canvas.set_clip(Rect::new(0.0, top, W, view));
        for (i, r) in rows.iter().enumerate() {
            let y0 = top + starts[i] - scroll;
            let h = height(r);
            if y0 + h < top || y0 > bottom {
                continue;
            }
            let ox = g.ox;
            // On the canvas, which needs the page's slide added.
            let lane_x = |c: usize| x0 + ox + c as f32 * LANE * 0.5;
            let color = |c: usize| palette[(c / 2) % palette.len()];
            // Rows fade out at the page's edges rather than being cut there.
            let cy = y0 + h * 0.5;
            let a = ((cy - top) / EDGE).clamp(0.0, 1.0) * ((bottom - cy) / EDGE).clamp(0.0, 1.0);
            let last = i + 1 == rows.len();
            for (c, ch) in r.lanes.chars().enumerate() {
                let pts: Option<[(f32, f32); 2]> = match ch {
                    '|' | '*' => Some([(lane_x(c), y0), (lane_x(c), y0 + h)]),
                    '/' => Some([(lane_x(c + 1), y0), (lane_x(c.saturating_sub(1)), y0 + h)]),
                    '\\' => Some([(lane_x(c.saturating_sub(1)), y0), (lane_x(c + 1), y0 + h)]),
                    '_' => Some([(lane_x(c.saturating_sub(1)), y0 + h * 0.5), (lane_x(c + 1), y0 + h * 0.5)]),
                    _ => None,
                };
                if let Some(pts) = pts {
                    // The first commit has nothing above it, the last nothing below.
                    let pts = match ch {
                        '*' if i == 0 => [(pts[0].0, cy), pts[1]],
                        '*' if last => [pts[0], (pts[1].0, cy)],
                        _ => pts,
                    };
                    g.canvas.polyline(&pts, 1.5, if ch == '*' { color(c) } else { color(c + 1) }, 0.9 * a.max(0.25));
                }
                if ch == '*' {
                    g.canvas.circle(lane_x(c), cy, 4.0, t.ground, a);
                    g.canvas.circle(lane_x(c), cy, 3.0, color(c), a);
                }
            }
            let Some(c) = &r.commit else { continue };
            let when = ago(c.time, self.now);
            let when_w = g.measure(&when, ty.caption);
            g.text_alpha(W - PAD - 4.0 - when_w, cy, &when, ty.caption, t.dim, a);
            let mut x = text_x;
            // The branches here, as small tags before the subject.
            for name in c.refs.iter().take(2) {
                let label = g.text.fit(name, &ty.caption, 70.0);
                let w = g.measure(&label, ty.caption) + 10.0;
                let tag = Rect::new(x + ox, cy - ty.caption.size * 0.5 - 2.0, w, ty.caption.size + 4.0);
                g.canvas.rrect(tag, tag.h * 0.5, t.surface2, a);
                g.text_alpha(x + 5.0, cy, &label, ty.caption, t.ice, a);
                x += w + 4.0;
            }
            let room = W - PAD - 10.0 - when_w - x;
            let subject = g.text.fit(&c.subject, &ty.body, room.max(20.0));
            g.text_alpha(x, cy, &subject, ty.body, if i == self.focus { t.ink } else { t.dim }, a);
        }
        g.canvas.clear_clip();
        // The ring around the commit in focus.
        let fy0 = top + fy - scroll;
        let ring = Rect::new(text_x - 4.0, fy0 + 1.0, W - PAD - (text_x - 4.0), fh - 2.0);
        if ring.y >= top - 1.0 && ring.bottom() <= bottom + 1.0 {
            g.focus_ring(ring, ring.h * 0.5);
        }
        let hint = rows[self.focus].commit.as_ref().map_or_else(String::new, |c| format!("{} · {} · enter: the commit", c.short, c.author));
        let fitted = g.text.fit(&hint, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// The branches: the one checked out, the others here, and those only on
/// the remote; Enter switches (a remote one gets a local branch tracking it).
pub struct BranchesScreen {
    repo: PathBuf,
    list: ListState,
    branches: Job<Result<Vec<Branch>, String>>,
    switching: Option<Job<Result<String, String>>>,
}

impl BranchesScreen {
    fn new(repo: &Path) -> Self {
        BranchesScreen { repo: repo.to_owned(), list: ListState::default(), branches: Self::read(repo), switching: None }
    }

    fn read(repo: &Path) -> Job<Result<Vec<Branch>, String>> {
        let repo = repo.to_owned();
        Job::start(move || git(&repo, &["branch", "-a", "--format=%(HEAD)%1f%(refname)%1f%(upstream:short)"]).map(|t| parse_branches(&t)))
    }
}

impl Screen for BranchesScreen {
    fn title(&self) -> &str {
        "Branches"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let branches = self.branches.get().and_then(Result::ok).unwrap_or_default();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right => {
                let Some(b) = branches.get(self.list.focus).cloned() else { return Nav::Stay };
                if b.current {
                    *cx.say = Some(format!("on {} already", b.name));
                } else if self.switching.is_none() {
                    let repo = self.repo.clone();
                    self.switching = Some(Job::start(move || {
                        if b.remote_only {
                            git(&repo, &["switch", "--track", &b.name])
                        } else {
                            git(&repo, &["switch", &b.name])
                        }
                    }));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, branches.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.switching.as_ref().and_then(Job::get) {
            self.switching = None;
            forget_comparison();
            *cx.say = Some(match done {
                Ok(_) => "switched".into(),
                Err(why) => why,
            });
            self.branches = Self::read(&self.repo);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.switching.is_some() || self.branches.get().is_none()
    }

    fn busy(&self) -> bool {
        self.switching.is_some() || self.branches.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let branches = match self.branches.get() {
            None => return g.empty("account_tree", "reading…"),
            Some(Err(why)) => {
                let line = g.text.fit(&why, &g.ty.caption, W - 2.0 * PAD);
                return g.empty("account_tree", &line);
            }
            Some(Ok(b)) => b,
        };
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        for (i, y) in self.list.layout(g, branches.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let b = &branches[i];
            let value = match (b.current, b.remote_only, &b.upstream) {
                (true, ..) => "checked out".to_owned(),
                (_, true, _) => "on the remote".to_owned(),
                (_, _, Some(u)) => u.clone(),
                _ => "here only".to_owned(),
            };
            let name = g.text.fit(&b.name, &g.ty.body, r.w * 0.55);
            g.row(r, &name, &value, if b.current { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, branches.len(), top, bottom);
        g.hint(if self.switching.is_some() { "switching…" } else { "enter: switch to it · changes stay with you" });
    }
}

/// Lines of git's output, scrolled through: a diff, or a commit.
pub struct TextScreen {
    title: String,
    lines: Job<Vec<String>>,
    first: usize,
    left: usize,
    rows: usize,
}

impl TextScreen {
    fn changes(repo: &Path, changes: &[(String, String)]) -> Self {
        let repo = repo.to_owned();
        let untracked: Vec<String> = changes.iter().filter(|(s, _)| s == "??").map(|(_, p)| p.clone()).collect();
        let lines = Job::start(move || {
            let mut out: Vec<String> = git(&repo, &["diff", "HEAD", "--no-color"]).unwrap_or_default().lines().map(str::to_owned).collect();
            out.extend(untracked.iter().map(|p| format!("new, not added: {p}")));
            if out.is_empty() {
                out.push("nothing has changed".into());
            }
            out
        });
        TextScreen { title: "Changes".into(), lines, first: 0, left: 0, rows: 8 }
    }

    /// One commit: who and when, its whole message, the files it touched,
    /// and its diff.
    fn commit(repo: &Path, hash: &str, short: &str) -> Self {
        let (repo, hash) = (repo.to_owned(), hash.to_owned());
        let lines = Job::start(move || {
            match git(&repo, &["show", "--stat", "--patch", "--no-color", "--date=format:%Y-%m-%d %H:%M", "--format=commit %h%nby %an · %ad%n%n%B", &hash]) {
                // Git's bare "---" between the message and the files says nothing here.
                Ok(text) => text.lines().filter(|l| *l != "---").map(str::to_owned).collect(),
                Err(why) => vec![why],
            }
        });
        TextScreen { title: short.to_owned(), lines, first: 0, left: 0, rows: 8 }
    }
}

impl Screen for TextScreen {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let count = self.lines.get().map_or(0, |l| l.len());
        let last = count.saturating_sub(self.rows);
        match key {
            Key::Back => return Nav::Pop,
            Key::Down => self.first = (self.first + 1).min(last),
            Key::Up => self.first = self.first.saturating_sub(1),
            Key::PageDown | Key::Enter => self.first = (self.first + self.rows).min(last),
            Key::PageUp => self.first = self.first.saturating_sub(self.rows),
            Key::Right => self.left += 8,
            Key::Left => self.left = self.left.saturating_sub(8),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.lines.get().is_none()
    }

    fn busy(&self) -> bool {
        self.lines.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let style = g.ty.caption;
        let cell_w = g.measure("0", style);
        let cell_h = style.size + 2.0;
        let panel = Rect::new(4.0, g.top() - 3.0, W - 8.0, H - g.top() - g.hint_h() - 1.0);
        g.surface(panel, 10.0, t.surface1);
        let cols = ((panel.w - 8.0) / cell_w).floor().max(2.0) as usize;
        self.rows = ((panel.h - 6.0) / cell_h).floor().max(1.0) as usize;
        let Some(lines) = self.lines.get() else {
            g.hint("reading…");
            return;
        };
        for (row, line) in lines.iter().skip(self.first).take(self.rows).enumerate() {
            // Added lines in the cool colour, removed in red, headers quiet.
            let color = match line.as_bytes().first() {
                Some(b'+') if !line.starts_with("+++") => t.ice,
                Some(b'-') if !line.starts_with("---") => g.fault(),
                Some(b'@') | Some(b'd') | Some(b'i') => t.dim,
                _ if line.starts_with("commit ") || line.starts_with("by ") => t.dim,
                _ => t.ink,
            };
            let shown: String = line.chars().skip(self.left).take(cols).map(|c| if c == '\t' { ' ' } else { c }).collect();
            g.text_at(panel.x + 4.0, panel.y + 3.0 + (row as f32 + 0.5) * cell_h, Align::Left, &shown, style, color);
        }
        g.hint(&format!("{}–{} of {} · ← → scroll across", self.first + 1, (self.first + self.rows).min(lines.len()), lines.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_the_branch_its_distance_and_the_changes() {
        let s = parse_status("## main...origin/main [ahead 1, behind 2]\n M src/Robot.java\n?? notes.txt\n");
        assert_eq!((s.branch.as_str(), s.upstream.as_deref(), s.ahead, s.behind), ("main", Some("origin/main"), 1, 2));
        assert_eq!(s.changes, [(" M".to_owned(), "src/Robot.java".to_owned()), ("??".to_owned(), "notes.txt".to_owned())]);
        let fresh = parse_status("## No commits yet on main\n");
        assert_eq!((fresh.branch.as_str(), fresh.upstream), ("main", None));
    }

    #[test]
    fn the_robots_code_is_compared_with_the_repository() {
        let dir = std::env::temp_dir().join(format!("catalyst-git-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| git(&dir, args);
        if run(&["init", "-q"]).is_err() {
            return; // no git here
        }
        let commit = |message: &str| {
            std::fs::write(dir.join("Robot.java"), message).unwrap();
            run(&["add", "."]).unwrap();
            run(&["-c", "user.name=test", "-c", "user.email=test@example.invalid", "commit", "-q", "-m", message]).unwrap();
            run(&["rev-parse", "HEAD"]).unwrap().trim().to_owned()
        };
        let first = commit("first");
        commit("second");
        let latest = commit("third");
        assert!(compare(&dir, &latest[..10]).ends_with("· the latest"), "{}", compare(&dir, &latest));
        assert!(compare(&dir, &first).ends_with("· 2 behind"), "{}", compare(&dir, &first));
        assert!(compare(&dir, &format!("{first}-dirty")).contains("+ uncommitted · 2 behind"));
        assert!(compare(&dir, "deadbeefdeadbeef").ends_with("not in this repo"));
        let log = parse_log(&run(&["log", "--format=%h%x1f%cr%x1f%an%x1f%s"]).unwrap());
        assert_eq!(log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["third", "second", "first"]);
        assert_eq!(find_repos(dir.parent().unwrap()).iter().filter(|p| **p == dir).count(), 1, "found where it is");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_history_is_read_with_its_lanes_and_branches() {
        let text = "* \u{1f}aaaa1111\u{1f}aaaa111\u{1f}1795200000\u{1f}Team\u{1f}Lower the limit\u{1f}HEAD -> main, origin/main\n\
                    *   \u{1f}bbbb2222\u{1f}bbbb222\u{1f}1795190000\u{1f}Team\u{1f}Merge the intake\u{1f}\n\
                    |\\  \n\
                    | * \u{1f}cccc3333\u{1f}cccc333\u{1f}1795180000\u{1f}Ann\u{1f}Intake\u{1f}intake\n\
                    * | \u{1f}dddd4444\u{1f}dddd444\u{1f}1795170000\u{1f}Team\u{1f}Two-note auto\u{1f}\n\
                    |/  \n\
                    * \u{1f}eeee5555\u{1f}eeee555\u{1f}1795160000\u{1f}Team\u{1f}Start\u{1f}origin/HEAD\n";
        let rows = parse_graph(text);
        assert_eq!(rows.len(), 7);
        let commits: Vec<&Entry> = rows.iter().filter_map(|r| r.commit.as_ref()).collect();
        assert_eq!(commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["Lower the limit", "Merge the intake", "Intake", "Two-note auto", "Start"]);
        assert_eq!(commits[0].refs, ["main", "origin/main"], "HEAD -> taken off");
        assert_eq!(commits[2].refs, ["intake"]);
        assert!(commits[4].refs.is_empty(), "origin/HEAD is not a branch");
        assert_eq!((rows[2].lanes.as_str(), rows[2].commit.is_none()), ("|\\", true), "a row that only joins lanes");
        assert_eq!(rows[3].lanes, "| *");
        assert_eq!(commits[3].time, 1_795_170_000);
        // Up and down step over the joining rows.
        assert_eq!(HistoryScreen::step(&rows, 1, 1), Some(3));
        assert_eq!(HistoryScreen::step(&rows, 4, 1), Some(6));
        assert_eq!(HistoryScreen::step(&rows, 6, 1), None);
        assert_eq!(ago(1000, 1030), "now");
        assert_eq!(ago(0, 3 * 3600 + 5), "3 h");
        assert_eq!(ago(0, 2 * 86_400), "2 d");
    }

    #[test]
    fn a_real_history_with_a_merge_draws_its_branch() {
        let dir = std::env::temp_dir().join(format!("catalyst-git-graph-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        if crate::github::make_demo_repo(&dir).is_err() {
            return; // no git here
        }
        let rows = parse_graph(&git(&dir, &["log", "--graph", "--date-order", GRAPH_FORMAT, "--no-color"]).unwrap());
        assert!(rows.iter().any(|r| r.commit.is_none() && (r.lanes.contains('\\') || r.lanes.contains('/'))), "the merge shows as lanes joining: {rows:?}");
        assert!(rows.iter().filter_map(|r| r.commit.as_ref()).any(|c| c.subject == "Merge the intake"));
        let branches = parse_branches(&git(&dir, &["branch", "-a", "--format=%(HEAD)%1f%(refname)%1f%(upstream:short)"]).unwrap());
        assert_eq!(branches.iter().filter(|b| b.current).map(|b| b.name.as_str()).collect::<Vec<_>>(), ["main"]);
        assert!(branches.iter().any(|b| b.name == "shooter-tuning" && !b.remote_only));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn branches_put_local_ones_first_and_remote_ones_without_a_local_after() {
        let text = "*\u{1f}refs/heads/main\u{1f}origin/main\n \u{1f}refs/heads/intake\u{1f}\n \u{1f}refs/remotes/origin/HEAD\u{1f}\n \u{1f}refs/remotes/origin/main\u{1f}\n \u{1f}refs/remotes/origin/auto-paths\u{1f}\n";
        let b = parse_branches(text);
        assert_eq!(b.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["main", "intake", "origin/auto-paths"]);
        let checked_out_later = parse_branches(" \u{1f}refs/heads/intake\u{1f}\n*\u{1f}refs/heads/main\u{1f}\n");
        assert_eq!(checked_out_later[0].name, "main", "the one checked out comes first");
        assert!(b[0].current && b[0].upstream.as_deref() == Some("origin/main"));
        assert!(b[2].remote_only);
    }

    #[test]
    fn a_clone_says_how_far_it_has_got() {
        assert_eq!(parse_progress("Receiving objects:  45% (450/1000), 1.2 MiB | 800 KiB/s"), Some(Progress { stage: "receiving objects".into(), percent: 45 }));
        assert_eq!(parse_progress("remote: Counting objects: 100% (12/12), done."), Some(Progress { stage: "counting objects".into(), percent: 100 }));
        assert_eq!(parse_progress("Cloning into 'robot'..."), None);
        assert_eq!(parse_progress("fatal: repository not found"), None);
    }

    #[test]
    fn a_robot_repository_offers_a_deploy_action() {
        let a = deploy_action(Path::new("/data/code/robot-2027"));
        assert_eq!((a.name.as_str(), a.command.as_str()), ("Deploy robot-2027", "cd /data/code/robot-2027 && ./gradlew deploy"));
        assert!(a.critical(), "deploying changes the robot: the assistant asks first");
    }
}
