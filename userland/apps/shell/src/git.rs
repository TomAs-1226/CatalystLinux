//! Git on the handheld: the repositories under the data partition, each with
//! its branch, how far it is from its upstream, what has changed (as diffs)
//! and its history; pulling with one key; cloning by address.
//!
//! One repository can be marked as the robot's code. FrcCatalyst publishes
//! the commit the robot was built from (`Robot/Software/RobotCodeVersion`,
//! from the team's build stamp), and the Robot page says how that stands
//! against the repository here: the same, so many commits behind, or built
//! from changes that were never committed.
//!
//! Git runs in a background thread: a status on a slow card never holds up
//! a frame.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

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
        .output()
        .map_err(|_| "git is not installed".to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let why = String::from_utf8_lossy(&out.stderr);
        Err(why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("git failed").trim().trim_start_matches("fatal: ").to_owned())
    }
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

/// One commit in the history: short hash, when, who, and its subject.
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub when: String,
    pub author: String,
    pub subject: String,
}

/// `git log --format=%h%x1f%cr%x1f%an%x1f%s`.
pub fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\u{1f}');
            Some(Commit { hash: f.next()?.into(), when: f.next()?.into(), author: f.next()?.into(), subject: f.next()?.into() })
        })
        .collect()
}

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

// -------------------------------------------------------------- screens ---

/// The repositories on the handheld, and cloning another.
pub struct GitScreen {
    list: ListState,
    repos: Vec<PathBuf>,
    /// The address being typed for a clone, and a clone under way.
    typing: Option<String>,
    cloning: Option<Job<Result<PathBuf, String>>>,
}

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
        if let Some(text) = &mut self.typing {
            match key {
                Key::Char(c) if !c.is_control() && text.len() < 200 => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Back => self.typing = None,
                Key::Enter => {
                    let url = text.trim().to_owned();
                    self.typing = None;
                    // An address, never something git would take as an option.
                    if url.starts_with('-') {
                        *cx.say = Some("that is not an address".into());
                    } else if !url.is_empty() {
                        let into = code_dir();
                        self.cloning = Some(Job::start(move || {
                            std::fs::create_dir_all(&into).map_err(|e| e.to_string())?;
                            let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or("repo").trim_end_matches(".git").to_owned();
                            let dest = into.join(if name.is_empty() { "repo" } else { &name });
                            git(&into, &["clone", "--depth", "100", "--", &url, &dest.to_string_lossy()]).map(|_| dest)
                        }));
                        *cx.say = Some("cloning…".into());
                    }
                }
                _ => {}
            }
            return Nav::Stay;
        }
        // The rows: each repository, then "clone a repository".
        let count = self.repos.len() + 1;
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if self.list.focus == self.repos.len() => self.typing = Some("https://github.com/".into()),
            Key::Enter => {
                if let Some(repo) = self.repos.get(self.list.focus) {
                    return Nav::Push(Box::new(RepoScreen::new(repo.clone())));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, count),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(done) = self.cloning.as_ref().and_then(|job| job.get()) else { return Nav::Stay };
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
        let ty = g.ty;
        let top = g.top();
        if let Some(text) = &self.typing {
            let field = Rect::new(PAD, top, W - 2.0 * PAD, ty.caption.size + ty.label.size + 20.0);
            g.surface(field, 14.0, t.surface2);
            g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, "address of the repository", ty.caption, t.dim);
            let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
            // The end of what is typed, where the typing is.
            let shown: String = text.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
            let w = g.text_at(field.x + 10.0, cy, Align::Left, &shown, ty.label, t.ink);
            g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
            g.focus_ring(field, 14.0);
            g.hint("enter: clone · esc: cancel");
            return;
        }
        let bottom = H - PAD - g.hint_h();
        let row_h = g.row_h();
        let rows = self.repos.len() + 1;
        let robots = robot_repo();
        for (i, y) in self.list.layout(g, rows, top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match self.repos.get(i) {
                Some(repo) => {
                    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mark = if robots.as_deref() == Some(repo.as_path()) { "robot code" } else { "" };
                    g.row(r, &name, mark, t.dim, focused);
                }
                None => g.row(r, if self.cloning.is_some() { "Cloning…" } else { "Clone a repository" }, "›", t.dim, focused),
            }
        }
        self.list.scrollbar(g, rows, top, bottom);
        let place = code_dir();
        let line = if place.starts_with("/data") { format!("repositories live in {}", place.display()) } else { "repositories live beside the settings".to_owned() };
        let fitted = g.text.fit(&line, &g.ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// One repository: where it stands, pulling, its changes and history, and
/// whether it is the robot's code.
pub struct RepoScreen {
    repo: PathBuf,
    name: String,
    list: ListState,
    status: Job<Result<Status, String>>,
    pulling: Option<Job<Result<String, String>>>,
}

impl RepoScreen {
    pub fn new(repo: PathBuf) -> Self {
        let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repository".into());
        let status = Self::read(&repo);
        RepoScreen { repo, name, list: ListState::default(), status, pulling: None }
    }

    fn read(repo: &Path) -> Job<Result<Status, String>> {
        let repo = repo.to_owned();
        Job::start(move || git(&repo, &["status", "--porcelain=v1", "--branch"]).map(|t| parse_status(&t)))
    }
}

/// The rows of a repository's page.
const REPO_ROWS: [&str; 4] = ["Pull", "Changes", "History", "The robot's code"];

impl Screen for RepoScreen {
    fn title(&self) -> &str {
        &self.name
    }

    fn resume(&mut self) {
        self.status = Self::read(&self.repo);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => match self.list.focus {
                0 if self.pulling.is_none() => {
                    let repo = self.repo.clone();
                    self.pulling = Some(Job::start(move || git(&repo, &["pull", "--ff-only"])));
                    *cx.say = Some("pulling…".into());
                }
                1 => {
                    let changes = self.status.get().and_then(Result::ok).map(|s| s.changes).unwrap_or_default();
                    return Nav::Push(Box::new(TextScreen::changes(&self.repo, &changes)));
                }
                2 => return Nav::Push(Box::new(TextScreen::history(&self.repo))),
                3 => {
                    let mine = robot_repo().as_deref() == Some(self.repo.as_path());
                    set_robot_repo((!mine).then_some(self.repo.as_path()));
                    *cx.say = Some(if mine { "no longer the robot's code".into() } else { "marked as the robot's code".into() });
                }
                _ => {}
            },
            other => crate::screens::list_key(&mut self.list, other, REPO_ROWS.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.pulling.as_ref().and_then(|job| job.get()) {
            self.pulling = None;
            forget_comparison();
            *cx.say = Some(match done {
                Ok(out) if out.contains("Already up to date") => "already up to date".into(),
                Ok(_) => "pulled".into(),
                Err(why) => why,
            });
            self.status = Self::read(&self.repo);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.pulling.is_some() || self.status.get().is_none()
    }

    fn busy(&self) -> bool {
        self.pulling.is_some() || self.status.get().is_none()
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
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
        let mine = robot_repo().as_deref() == Some(self.repo.as_path());
        let changes = status.as_ref().and_then(|s| s.as_ref().ok()).map_or(0, |s| s.changes.len());
        for (i, y) in self.list.layout(g, REPO_ROWS.len(), list_top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let value = match i {
                0 if self.pulling.is_some() => "pulling…".to_owned(),
                0 => "fast-forward".to_owned(),
                1 if changes == 0 => "none".to_owned(),
                1 => changes.to_string(),
                2 => "›".to_owned(),
                _ => (if mine { "yes" } else { "no" }).to_owned(),
            };
            g.row(r, REPO_ROWS[i], &value, if i == 3 && mine { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, REPO_ROWS.len(), list_top, bottom);
        // The robot's build against this repository, when it is the robot's code.
        let line = match (mine, robot.string("Robot/Software/RobotCodeVersion")) {
            (true, Some(version)) => robot_code_line(&version).map_or_else(|| "on the robot: comparing…".into(), |l| format!("on the robot: {l}")),
            (true, None) => "the robot does not say its version".into(),
            _ => self.repo.display().to_string(),
        };
        let fitted = g.text.fit(&line, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

/// Lines of git's output, scrolled through: a diff, or the history.
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

    fn history(repo: &Path) -> Self {
        let repo = repo.to_owned();
        let lines = Job::start(move || match git(&repo, &["log", "-n", "200", "--format=%h%x1f%cr%x1f%an%x1f%s"]) {
            Ok(text) => parse_log(&text).into_iter().map(|c| format!("{} {} · {} · {}", c.hash, c.subject, c.author, c.when)).collect(),
            Err(why) => vec![why],
        });
        TextScreen { title: "History".into(), lines, first: 0, left: 0, rows: 8 }
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
}
