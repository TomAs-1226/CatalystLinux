//! What Spotlight finds out of the box: the apps, every page and switch in
//! Settings, the Actions, notes, saved SSH hosts, git repositories, photos
//! and the files under the storage (all read on the index's thread); and,
//! from what is typed, the robot's values, match logs, the event schedule,
//! sums and unit conversions, the web and the assistant (worked out as each
//! key lands, from memory).

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use super::index::{weight, Item, Preview, Provider, Query, TAIL};
use crate::robot::Robot;
use crate::shell::{Cx, Screen};

/// The providers Spotlight starts with, in the order their parts are read
/// (the quick ones first, so they are there at once).
pub fn builtin() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(Apps::default()),
        Arc::new(Settings),
        Arc::new(Actions::default()),
        Arc::new(Notes),
        Arc::new(Contacts),
        Arc::new(Repos),
        Arc::new(Photos),
        Arc::new(Files::default()),
        Arc::new(Values::default()),
        Arc::new(Logs::default()),
        Arc::new(Schedule),
        Arc::new(Sums),
        Arc::new(Elsewhere),
    ]
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A fingerprint of some files and folders as they are: their change times
/// and sizes (a folder's change time moves when something in it is added,
/// removed or written by way of a new file, as every save here is).
fn stamp_of<'a>(paths: impl IntoIterator<Item = &'a Path>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for path in paths {
        match std::fs::metadata(path) {
            Ok(meta) => (meta.modified().ok(), meta.len()).hash(&mut h),
            Err(_) => 0u8.hash(&mut h),
        }
    }
    h.finish()
}

/// A day as the rows say it: "nov 16".
fn day(unix: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(unix, 0).single().map(|t| t.format("%b %-d").to_string().to_lowercase()).unwrap_or_default()
}

fn unix(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The first lines of a text, as a preview shows them: tabs as spaces, blank
/// lines at the start left out, at most `n`.
fn first_lines(text: &str, n: usize) -> Vec<String> {
    text.lines().skip_while(|l| l.trim().is_empty()).take(n).map(|l| l.replace('\t', "    ").trim_end().chars().take(80).collect()).collect()
}

// ------------------------------------------------------------------- apps ---

/// Other names the built-in apps go by.
const ALSO: &[(&str, &str)] = &[
    ("Robot", "battery voltage identity"),
    ("Health", "faults errors warnings"),
    ("Logs", "matches wpilog"),
    ("Schedule", "matches event queue"),
    ("Tables", "networktables values"),
    ("Loop", "loop time"),
    ("Motors", "can devices"),
    ("Console", "robot log output"),
    ("Terminal", "shell console command line"),
    ("Files", "folder storage finder"),
    ("Git", "repositories code"),
    ("Catalogue", "store install apps"),
    ("Monitor", "cpu memory processes"),
    ("Net tools", "ping network scan"),
    ("Connect", "ssh serial"),
    ("Actions", "commands run"),
    ("Camera", "photo picture scan qr"),
    ("Photos", "pictures album gallery"),
    ("AprilTag", "tags vision"),
    ("Graphs", "plot chart"),
    ("Pit", "checklist timer"),
    ("Remote", "infrared ir tv"),
    ("Level", "bubble tilt"),
    ("Compass", "heading north"),
    ("Weather", "forecast rain"),
    ("Browse", "read page web"),
    ("Calculator", "math calc sum"),
    ("Clock", "timer stopwatch alarm"),
    ("Notes", "memo pit notes"),
    ("Units", "convert conversion"),
    ("HTTP", "api request curl"),
    ("Share", "qr code"),
    ("Assistant", "ai claude chatgpt openai ask"),
    ("Web", "browser lynx internet"),
    ("Settings", "preferences options"),
];

/// The built-in apps, and those found on the storage.
#[derive(Default)]
pub struct Apps {
    external: Mutex<Vec<crate::apps::App>>,
}

impl Provider for Apps {
    fn name(&self) -> &'static str {
        "apps"
    }

    fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = crate::screens::builtin_apps()
            .map(|(_, icon, name)| {
                let also = ALSO.iter().find(|(n, _)| *n == name).map_or("", |(_, w)| w);
                Item::new(format!("builtin:{name}"), name, icon, weight::APP).detail("app").words(also).fuzzy()
            })
            .collect();
        let external = crate::apps::find();
        items.extend(external.iter().map(|app| Item::new(format!("app:{}", app.name), app.name.clone(), Cow::Owned(app.icon.clone()), weight::APP).detail(app.about.clone()).fuzzy()));
        *lock(&self.external) = external;
        items
    }

    fn stamp(&self) -> Option<u64> {
        let folders = crate::apps::folders();
        let hardware = (crate::imu::has_motion() as u64) << 1 | crate::imu::has_compass() as u64;
        Some(stamp_of(folders.iter().map(PathBuf::as_path)) ^ hardware)
    }

    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        if let Some(name) = item.id.strip_prefix("builtin:") {
            let (at, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == name).ok_or("that app is not on this handheld")?;
            return Ok(crate::screens::open_builtin(at, cx.robot));
        }
        let name = item.id.strip_prefix("app:").unwrap_or_default();
        lock(&self.external).iter().find(|a| a.name == name).map(|app| app.open()).ok_or_else(|| format!("{name} is gone"))
    }
}

// --------------------------------------------------------------- settings ---

/// Every page and switch in Settings, opened at its row.
pub struct Settings;

impl Provider for Settings {
    fn name(&self) -> &'static str {
        "settings"
    }

    fn items(&self) -> Vec<Item> {
        let keys = crate::keys::Keys::load();
        crate::screens::setting_sections()
            .into_iter()
            .map(|(_, name, section)| {
                // A key's row says whether it is set; never what it is.
                let set = crate::keys::SERVICES.iter().any(|(service, label)| name == format!("{label} key") && keys.get(service).is_some());
                let detail = match section {
                    Some(section) => format!("settings › {}", short_section(section)),
                    None => "settings".to_owned(),
                };
                let detail = if set { format!("{detail} · set") } else { detail };
                let icon = if section.is_some() { "tune" } else { "settings" };
                Item::new(format!("setting:{name}"), name, icon, weight::SETTING).detail(detail).words(format!("{} settings", section.unwrap_or_default())).fuzzy()
            })
            .collect()
    }

    fn stamp(&self) -> Option<u64> {
        Some(stamp_of([crate::keys::path().as_path()]) ^ crate::imu::has_motion() as u64)
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let name = item.id.strip_prefix("setting:").unwrap_or_default();
        let (at, _) = crate::screens::setting_names().find(|(_, n)| *n == name).ok_or("that setting has moved")?;
        Ok(crate::screens::open_setting(at))
    }
}

/// A section as the rows name it: short, lower case ("keys", not "Keys for
/// online services").
fn short_section(section: &str) -> String {
    let short = match section {
        "Robot settings" => "robot",
        "Keys for online services" => "keys",
        "About this handheld" => "about",
        "System update" => "updates",
        other => other,
    };
    short.to_lowercase()
}

// ---------------------------------------------------------------- actions ---

/// The Actions: the team's own and the built-in ones. Picked, each is asked
/// about on the confirm page before it runs.
#[derive(Default)]
pub struct Actions {
    known: Mutex<Vec<crate::commands::Saved>>,
}

impl Provider for Actions {
    fn name(&self) -> &'static str {
        "actions"
    }

    fn items(&self) -> Vec<Item> {
        let all = crate::commands::all();
        let items = all.iter().map(|s| Item::new(format!("action:{}", s.name), s.name.clone(), "play_circle", weight::ACTION).detail("action").words(s.command.clone()).fuzzy()).collect();
        *lock(&self.known) = all;
        items
    }

    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let name = item.id.strip_prefix("action:").unwrap_or_default();
        let saved = lock(&self.known).iter().find(|s| s.name == name).cloned().ok_or("that action is gone")?;
        let robot = crate::tools::robot_address(cx.robot);
        let line = crate::commands::expand(&saved.command, robot.as_deref(), crate::tools::team(cx.robot))?;
        Ok(Box::new(crate::ai::Confirm::to_run(&saved.name, &line, saved.critical())))
    }

    fn preview(&self, item: &Item) -> Option<Preview> {
        let name = item.id.strip_prefix("action:")?;
        let saved = lock(&self.known).iter().find(|s| s.name == name).cloned()?;
        let asks = if saved.critical() { "first · it could change something" } else { "no · it only looks" };
        Some(Preview::Facts(vec![("runs".into(), saved.command), ("asks".into(), asks.into())]))
    }
}

// ------------------------------------------------------------------ notes ---

/// Notes, by their titles and what they say.
pub struct Notes;

impl Provider for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn items(&self) -> Vec<Item> {
        crate::notes::load(&crate::notes::dir())
            .into_iter()
            .map(|n| {
                let body: String = n.text.lines().skip_while(|l| l.trim().is_empty()).skip(1).collect::<Vec<_>>().join(" ");
                Item::new(format!("note:{}", n.file), n.title, "sticky_note_2", weight::NOTE).detail(format!("note · {}", day(n.modified))).words(body)
            })
            .collect()
    }

    fn stamp(&self) -> Option<u64> {
        // Each note's size and time: a note written in place counts too.
        let dir = crate::notes::dir();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut entries: Vec<(String, Option<SystemTime>, u64)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((e.file_name().to_string_lossy().into_owned(), meta.modified().ok(), meta.len()))
            })
            .collect();
        entries.sort();
        entries.hash(&mut h);
        Some(h.finish())
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let file = item.id.strip_prefix("note:").unwrap_or_default();
        Ok(Box::new(crate::files::Editor::open(&crate::notes::dir().join(file))?))
    }

    fn preview(&self, item: &Item) -> Option<Preview> {
        let path = crate::notes::dir().join(item.id.strip_prefix("note:")?);
        let text = std::fs::read_to_string(&path).ok()?;
        let changed = std::fs::metadata(&path).ok()?.modified().ok().map(unix).map(day).unwrap_or_default();
        // The title is the preview's heading already: what follows it.
        let body: String = text.lines().skip_while(|l| l.trim().is_empty()).skip(1).collect::<Vec<_>>().join("\n");
        Some(Preview::Text { lines: first_lines(&body, 8), facts: vec![("changed".into(), format!("note · {changed}"))] })
    }
}

// --------------------------------------------------------------- contacts ---

/// The SSH hosts kept in Connect: the handheld's address book.
pub struct Contacts;

impl Provider for Contacts {
    fn name(&self) -> &'static str {
        "hosts"
    }

    fn items(&self) -> Vec<Item> {
        crate::connect::load_hosts()
            .into_iter()
            .map(|h| {
                let who = if h.user.is_empty() { h.host.clone() } else { format!("{}@{}", h.user, h.host) };
                Item::new(format!("host:{}", h.line()), who, "settings_ethernet", weight::CONTACT).detail("ssh").words(format!("ssh connect {}", h.line()))
            })
            .collect()
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let host = crate::connect::Host::parse(item.id.strip_prefix("host:").unwrap_or_default()).ok_or("that host is not right")?;
        Ok(Box::new(crate::connect::session(&host)))
    }
}

// ------------------------------------------------------------------ repos ---

/// Git repositories in the code folder.
pub struct Repos;

impl Provider for Repos {
    fn name(&self) -> &'static str {
        "repos"
    }

    fn items(&self) -> Vec<Item> {
        crate::git::find_repos(&crate::git::code_dir())
            .into_iter()
            .map(|path| {
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                Item::new(format!("repo:{}", path.display()), name, "commit", weight::REPO).detail("repository").words("git repository code")
            })
            .collect()
    }

    fn stamp(&self) -> Option<u64> {
        let root = crate::git::code_dir();
        // The folder and those in it: a repository cloned one level down shows too.
        let mut paths = vec![root.clone()];
        paths.extend(std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        Some(stamp_of(paths.iter().map(PathBuf::as_path)))
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let path = PathBuf::from(item.id.strip_prefix("repo:").unwrap_or_default());
        if !path.join(".git").exists() {
            return Err("that repository is gone".into());
        }
        Ok(Box::new(crate::git::RepoScreen::new(path)))
    }

    fn preview(&self, item: &Item) -> Option<Preview> {
        let path = PathBuf::from(item.id.strip_prefix("repo:")?);
        let head = std::fs::read_to_string(path.join(".git/HEAD")).unwrap_or_default();
        let branch = head.trim().strip_prefix("ref: refs/heads/").map_or_else(|| head.trim().chars().take(10).collect(), str::to_owned);
        Some(Preview::Facts(vec![("branch".into(), branch), ("folder".into(), path.display().to_string())]))
    }
}

// ----------------------------------------------------------------- photos ---

/// The album's photos, by when they were taken.
pub struct Photos;

impl Provider for Photos {
    fn name(&self) -> &'static str {
        "photos"
    }

    fn items(&self) -> Vec<Item> {
        crate::photos::list()
            .into_iter()
            .map(|p| Item::new(format!("photo:{}", p.name), crate::photos::when(p.taken, true), "photo", weight::PHOTO).detail("photo").words(format!("{} photo picture", p.name)))
            .collect()
    }

    fn stamp(&self) -> Option<u64> {
        Some(stamp_of([crate::photos::dir().as_path()]))
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let name = item.id.strip_prefix("photo:").unwrap_or_default();
        let photos = crate::photos::list();
        let at = photos.iter().position(|p| p.name == name).ok_or("that photo is gone")?;
        Ok(Box::new(crate::photos::Viewer::new(photos, at)))
    }

    fn preview(&self, item: &Item) -> Option<Preview> {
        let name = item.id.strip_prefix("photo:")?;
        let photo = crate::photos::list().into_iter().find(|p| p.name == name)?;
        let picture = crate::photos::thumbnail(&photo)?;
        let mut facts = vec![("size".into(), crate::photos::size_words(photo.bytes))];
        if let Some((w, h)) = crate::photos::dimensions(&photo.path) {
            facts.push(("pixels".into(), format!("{w} × {h}")));
        }
        facts.push(("name".into(), photo.name));
        Some(Preview::Picture { picture, facts })
    }
}

// ------------------------------------------------------------------ files ---

/// The most files and folders the index holds, and how deep it looks: a
/// robot project's sources are found, a whole build tree is not.
const FILES_MOST: usize = 4000;
const FILES_DEPTH: usize = 8;

/// Folders that are never looked into: version control, builds, caches.
const SKIPPED: [&str; 8] = ["lost+found", "node_modules", "build", "target", "__pycache__", "bin", "obj", "venv"];

/// One folder as last read, and when it changed then: what is in it, small
/// (its names in one string, a NUL between them, which no name can hold;
/// each one's size, and whether it is a folder).
struct Folder {
    changed: Option<SystemTime>,
    names: Box<str>,
    sizes: Box<[u64]>,
    folders: Box<[bool]>,
}

impl Folder {
    fn entries(&self) -> impl Iterator<Item = (&str, bool, u64)> {
        self.names.split('\0').filter(|n| !n.is_empty()).zip(self.folders.iter().zip(self.sizes.iter())).map(|(n, (d, s))| (n, *d, *s))
    }

    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.names.len() + 16 + self.sizes.len() * 8 + 16 + self.folders.len() + 16
    }
}

/// Files and folders under the storage, read a folder at a time: a folder
/// whose change time is as it was is not read again.
#[derive(Default)]
pub struct Files {
    folders: Mutex<HashMap<PathBuf, Folder>>,
    /// Where to look instead of the storage (the measurements).
    root: Option<PathBuf>,
}

impl Files {
    /// Where to look, and what to leave to its own provider or leave alone
    /// (the settings and keys, SSH's keys, notes, photos, apps).
    fn roots(&self) -> (PathBuf, Vec<PathBuf>) {
        let root = self.root.clone().unwrap_or_else(crate::files::storage);
        let mut skip = vec![crate::config_dir(), crate::connect::private_dir(), crate::notes::dir(), crate::photos::dir()];
        skip.extend(crate::apps::folders());
        (root, skip)
    }

    /// Where a file found is: its place under the storage.
    fn path_of(&self, item: &Item) -> Option<PathBuf> {
        let rel = item.id.strip_prefix("file:")?;
        (!rel.split('/').any(|part| part == "..")).then(|| self.roots().0.join(rel))
    }

    /// Every file and folder under `root` (but those under `skip`), breadth
    /// first so the nearest are kept if there are too many.
    fn walk(&self, root: &Path, skip: &[PathBuf]) -> Vec<(PathBuf, bool, u64)> {
        let root = root.to_owned();
        let mut folders = lock(&self.folders);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut queue = std::collections::VecDeque::from([(root, 0usize)]);
        while let Some((dir, depth)) = queue.pop_front() {
            if out.len() >= FILES_MOST {
                break;
            }
            seen.insert(dir.clone());
            let changed = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
            let fresh = folders.get(&dir).is_some_and(|f| f.changed.is_some() && f.changed == changed);
            if !fresh {
                let mut entries: Vec<(String, bool, u64)> = std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        // Not through links: a link to a folder above would never end.
                        let meta = e.metadata().ok()?;
                        (!name.starts_with('.') && !name.contains('\0')).then(|| (name, meta.is_dir(), meta.len()))
                    })
                    .collect();
                entries.sort();
                let names = entries.iter().map(|(n, ..)| n.as_str()).collect::<Vec<_>>().join("\0").into_boxed_str();
                let folder = Folder { changed, names, sizes: entries.iter().map(|e| e.2).collect(), folders: entries.iter().map(|e| e.1).collect() };
                folders.insert(dir.clone(), folder);
            }
            let Some(folder) = folders.get(&dir) else { continue };
            for (name, is_dir, bytes) in folder.entries() {
                let path = dir.join(name);
                if skip.iter().any(|s| path.starts_with(s)) {
                    continue;
                }
                out.push((path.clone(), is_dir, bytes));
                if is_dir && depth + 1 < FILES_DEPTH && !SKIPPED.contains(&name) {
                    queue.push_back((path, depth + 1));
                }
                if out.len() >= FILES_MOST {
                    break;
                }
            }
        }
        // Folders gone (or no longer reached) are forgotten.
        folders.retain(|dir, _| seen.contains(dir));
        out
    }
}

impl Provider for Files {
    fn name(&self) -> &'static str {
        "files"
    }

    fn items(&self) -> Vec<Item> {
        let (root, skip) = self.roots();
        let top = root.file_name().map_or_else(|| "storage".to_owned(), |n| n.to_string_lossy().into_owned());
        self.walk(&root, &skip)
            .into_iter()
            .map(|(path, is_dir, bytes)| {
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                // Kept by its place under the storage: the storage's own
                // folder is said once, not in every file's name.
                let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                // Where it is, by the last two folders at most.
                let folders: Vec<&str> = rel.split('/').collect();
                let place = match folders.len() {
                    0 | 1 => top.clone(),
                    2 => folders[0].to_owned(),
                    n => format!("{}/{}", folders[n - 3], folders[n - 2]),
                };
                let (icon, detail) = if is_dir {
                    ("folder", format!("folder · in {place}"))
                } else if crate::photos::is_photo(&name) {
                    ("photo", format!("in {place}"))
                } else {
                    ("description", format!("{} · in {place}", crate::tools::bytes(bytes as f64)))
                };
                Item::new(format!("file:{rel}"), name, icon, weight::FILE).detail(detail)
            })
            .collect()
    }

    fn bytes(&self) -> usize {
        let folders = lock(&self.folders);
        folders.iter().map(|(dir, f)| dir.as_os_str().len() + 16 + std::mem::size_of::<PathBuf>() + f.bytes()).sum()
    }

    fn stamp(&self) -> Option<u64> {
        // Every folder as it was read, by its change time: a file added,
        // removed or renamed anywhere changes its folder's. One changed since
        // it was read gives a stamp never given before, so it is read again.
        static CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let folders = lock(&self.folders);
        let mut dirs: Vec<(&PathBuf, &Folder)> = folders.iter().collect();
        dirs.sort_by_key(|(dir, _)| *dir);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (dir, folder) in dirs {
            if std::fs::metadata(dir).and_then(|m| m.modified()).ok() != folder.changed {
                return Some(CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) | 1 << 63);
            }
            (dir, folder.changed).hash(&mut h);
        }
        Some(h.finish() & !(1 << 63))
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let path = self.path_of(item).ok_or("that file is gone")?;
        let meta = std::fs::metadata(&path).map_err(|_| "that file is gone".to_owned())?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if meta.is_dir() {
            return Ok(Box::new(crate::files::Files::at(&path, None)));
        }
        if name.as_deref().is_some_and(crate::photos::is_photo) {
            return Ok(Box::new(crate::photos::Viewer::single(path)));
        }
        // Text opens in the editor; anything else in Files, on it.
        match crate::files::Editor::open(&path) {
            Ok(editor) => Ok(Box::new(editor)),
            Err(_) => Ok(Box::new(crate::files::Files::at(path.parent().unwrap_or(&path), name.as_deref()))),
        }
    }

    fn preview(&self, item: &Item) -> Option<Preview> {
        use std::io::Read;
        let path = self.path_of(item)?;
        let meta = std::fs::metadata(&path).ok()?;
        let changed = meta.modified().ok().map(unix).map(day).unwrap_or_default();
        if meta.is_dir() {
            let count = std::fs::read_dir(&path).map(|d| d.count()).unwrap_or(0);
            return Some(Preview::Facts(vec![("holds".into(), format!("{count} item{}", if count == 1 { "" } else { "s" })), ("changed".into(), changed), ("folder".into(), path.display().to_string())]));
        }
        let facts = vec![("size".into(), crate::tools::bytes(meta.len() as f64)), ("changed".into(), changed)];
        if path.file_name().is_some_and(|n| crate::photos::is_photo(&n.to_string_lossy())) {
            let picture = crate::photos::decode(&path, crate::photos::THUMB_W * 2, crate::photos::THUMB_H * 2).ok()?.cover(crate::photos::THUMB_W, crate::photos::THUMB_H);
            return Some(Preview::Picture { picture, facts });
        }
        // The start of it, if it is text.
        let mut head = Vec::new();
        std::fs::File::open(&path).ok()?.take(4096).read_to_end(&mut head).ok()?;
        let text = match std::str::from_utf8(&head) {
            Ok(text) => text.to_owned(),
            // Cut mid-character at the end of what was read: still text.
            Err(e) if e.valid_up_to() + 4 >= head.len() && e.valid_up_to() > 0 => String::from_utf8_lossy(&head[..e.valid_up_to()]).into_owned(),
            Err(_) => return Some(Preview::Facts(facts)),
        };
        if text.contains('\0') {
            return Some(Preview::Facts(facts));
        }
        Some(Preview::Text { lines: first_lines(&text, 8), facts })
    }
}

// ----------------------------------------------------------------- values ---

/// How long the robot's keys and logs are kept between readings while typing.
const LIVE_FOR: f32 = 2.0;

/// The robot's names, read now and then while typing: the key, its last
/// part in lower case and as words, and the whole key in lower case; and
/// which robot they are of (its address, and whether it is linked).
#[derive(Default)]
struct KeyCache {
    at: Option<Instant>,
    of: (String, bool),
    keys: Vec<(String, String, String, String)>,
}

/// Whether what was read of the robot `at` some time, of the robot `of`,
/// is still good: read lately, of the robot there now.
fn still_good(at: Option<Instant>, of: &(String, bool), robot: &Robot) -> bool {
    at.is_some_and(|t| t.elapsed().as_secs_f32() <= LIVE_FOR) && of.0 == robot.address && of.1 == robot.linked()
}

/// Any value the robot publishes, by its name or its whole key, with the
/// value as it is now.
#[derive(Default)]
pub struct Values {
    cache: Mutex<KeyCache>,
}

/// At most this many values are offered at once.
const VALUES_SHOWN: usize = 12;

impl Provider for Values {
    fn name(&self) -> &'static str {
        "values"
    }

    fn answer(&self, query: &Query, robot: &Robot, out: &mut Vec<Item>) {
        let mut cache = lock(&self.cache);
        if !still_good(cache.at, &cache.of, robot) {
            cache.of = (robot.address.clone(), robot.linked());
            cache.keys = robot
                .under("")
                .into_iter()
                .map(|(key, _)| {
                    let leaf = key.rsplit('/').next().unwrap_or(&key).to_owned();
                    let (lower, split, whole) = (super::index::fold(&leaf), super::index::split_words(&leaf), super::index::split_words(&key));
                    (key, lower, split, whole)
                })
                .collect();
            cache.at = Some(Instant::now());
        }
        let mut found: Vec<(u32, usize)> = cache
            .keys
            .iter()
            .enumerate()
            .filter_map(|(i, (_, lower, split, whole))| query.rank(lower, split, whole, weight::VALUE, false).map(|r| (r, i)))
            .collect();
        found.sort();
        for (rank, i) in found.into_iter().take(VALUES_SHOWN) {
            let (key, ..) = &cache.keys[i];
            let leaf = key.rsplit('/').next().unwrap_or(key);
            let value = robot.get(key).map_or_else(|| "–".to_owned(), |v| v.display());
            out.push(Item::new(key.clone(), leaf, "account_tree", weight::VALUE).detail(value).words(key.clone()).ranked(rank));
        }
    }

    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        Ok(Box::new(crate::screens::Tables::at(&item.id, cx.robot)))
    }

    fn live_preview(&self, item: &Item, robot: &Robot) -> Option<Preview> {
        let value = robot.get(&item.id).map_or_else(|| "not published".to_owned(), |v| v.display());
        Some(Preview::Facts(vec![("now".into(), value), ("key".into(), item.id.clone())]))
    }
}

// ------------------------------------------------------------------- logs ---

/// The match logs pulled from the robot, by match.
#[derive(Default)]
pub struct Logs {
    /// When read, of which robot, and each log's name, title and headline.
    cache: Mutex<(Option<Instant>, (String, bool), Vec<(String, String, String)>)>,
}

impl Provider for Logs {
    fn name(&self) -> &'static str {
        "logs"
    }

    fn answer(&self, query: &Query, robot: &Robot, out: &mut Vec<Item>) {
        let mut cache = lock(&self.cache);
        if !still_good(cache.0, &cache.1, robot) {
            cache.2 = robot.logs.list().into_iter().map(|row| (row.name.clone(), row.title(), row.digest.as_ref().map_or_else(|| "match log".to_owned(), |d| d.headline().1))).collect();
            cache.0 = Some(Instant::now());
            cache.1 = (robot.address.clone(), robot.linked());
        }
        let mut found: Vec<(u32, usize)> = cache.2.iter().enumerate().filter_map(|(i, (_, title, _))| query.score(title, "match log", weight::LOG, false).map(|r| (r, i))).collect();
        found.sort();
        for (rank, i) in found.into_iter().take(6) {
            let (name, title, headline) = &cache.2[i];
            out.push(Item::new(name.clone(), title.clone(), "description", weight::LOG).detail(headline.clone()).ranked(rank));
        }
    }

    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let rows = cx.robot.logs.list();
        let detail = rows.iter().find(|r| r.name == item.id).and_then(crate::screens::LogDetail::of).ok_or("that log is not read yet")?;
        Ok(Box::new(detail))
    }
}

// --------------------------------------------------------------- schedule ---

/// The event schedule: a match by its number ("q14", "qm 14", "match 14"),
/// a team's matches by its number, and the next match.
pub struct Schedule;

/// A match's label as typed, made like the schedule's ("q14", "m3", "f1").
fn match_label(text: &str) -> Option<String> {
    let squashed: String = text.split_whitespace().collect::<Vec<_>>().join("");
    let rest = squashed.strip_prefix("match").map(|n| format!("q{n}")).unwrap_or(squashed);
    let rest = rest.strip_prefix("qm").map(|n| format!("q{n}")).unwrap_or(rest);
    let letters: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &rest[letters.len()..];
    (!letters.is_empty() && letters.len() <= 2 && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '-')).then_some(rest)
}

impl Provider for Schedule {
    fn name(&self) -> &'static str {
        "schedule"
    }

    fn answer(&self, query: &Query, robot: &Robot, out: &mut Vec<Item>) {
        let Some(view) = robot.schedule.view() else { return };
        let now = chrono::Utc::now().timestamp();
        // How it went; else when to queue, as Home's tile and the schedule
        // say it; else when it plays.
        let when = |m: &crate::schedule::Match| match (m.result(view.team), m.queue_at.filter(|_| m.score.is_none()), m.time) {
            (Some(result), ..) => result,
            (None, Some(queue), _) => format!("queue {}", crate::schedule::when(queue, now)),
            (None, None, Some(at)) => crate::schedule::when(at, now),
            _ => "not yet".into(),
        };
        let side = |m: &crate::schedule::Match| m.alliance(view.team).unwrap_or("–");
        // A match by its number.
        if let Some(label) = match_label(&query.text) {
            if let Some(m) = view.matches.iter().find(|m| m.label.to_lowercase() == label) {
                out.push(Item::new(m.key.clone(), m.label.clone(), "event", weight::SCHEDULE).detail(format!("{} · {}", side(m), when(m))).ranked(4));
            }
        }
        // A team's matches, the next ones first.
        if query.text.len() <= 5 && query.text.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(team) = query.text.parse::<u32>() {
                let mut theirs: Vec<&crate::schedule::Match> = view.matches.iter().filter(|m| m.red.contains(&team) || m.blue.contains(&team)).collect();
                theirs.sort_by_key(|m| (m.score.is_some(), m.time.unwrap_or(i64::MAX)));
                let ours = team == view.team;
                for (k, m) in theirs.into_iter().take(4).enumerate() {
                    let with = if ours {
                        side(m).to_owned()
                    } else if m.alliance(view.team).is_some() && m.alliance(view.team) == m.alliance(team) {
                        format!("with {team}")
                    } else if m.alliance(view.team).is_some() {
                        format!("against {team}")
                    } else {
                        format!("{team} {}", m.alliance(team).unwrap_or(""))
                    };
                    out.push(Item::new(m.key.clone(), format!("{} · {with}", m.label), "event", weight::SCHEDULE).detail(when(m)).ranked(20 + k as u32));
                }
            }
        }
        // The next match.
        if "next match".starts_with(&query.text) && query.text.len() >= 3 {
            if let Some(m) = view.next(now) {
                out.push(Item::new(m.key.clone(), format!("Next match · {}", m.label), "event", weight::SCHEDULE).detail(format!("{} · {}", side(m), when(m))).ranked(30));
            }
        }
    }

    fn open(&self, item: &Item, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        let view = cx.robot.schedule.view().ok_or("no schedule yet")?;
        let at = view.matches.iter().position(|m| m.key == item.id).ok_or("that match is no longer listed")?;
        Ok(Box::new(crate::schedule::ScheduleScreen::at(at)))
    }

    fn live_preview(&self, item: &Item, robot: &Robot) -> Option<Preview> {
        let view = robot.schedule.view()?;
        let m = view.matches.iter().find(|m| m.key == item.id)?;
        let teams = |t: &[u32]| t.iter().map(u32::to_string).collect::<Vec<_>>().join(" · ");
        let now = chrono::Utc::now().timestamp();
        let mut facts = vec![("red".into(), teams(&m.red)), ("blue".into(), teams(&m.blue))];
        match (m.result(view.team), m.time) {
            (Some(result), _) => facts.push(("result".into(), result)),
            (None, Some(at)) => facts.push(("plays".into(), crate::schedule::when(at, now))),
            _ => {}
        }
        if let Some(q) = m.queue_at.filter(|_| m.score.is_none()) {
            facts.push(("queue".into(), crate::schedule::when(q, now)));
        }
        Some(Preview::Facts(facts))
    }
}

// ------------------------------------------------------------------- sums ---

/// Sums worked out as they are typed, with the Calculator's own engine, and
/// amounts in other units, with the Units app's.
pub struct Sums;

/// Whether what is typed is meant as a sum: an operator, a bracket or a
/// function with numbers, or a number written in hex, binary or octal. A
/// bare number (a team, a match) is not.
fn looks_like_math(text: &str) -> bool {
    let t = text.trim();
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let radix = ["0x", "0b", "0o"].iter().any(|p| t.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(p) && w.len() > 2));
    let operator = t.chars().any(|c| "+*/×÷^%()&|~<>−".contains(c)) || t.trim_start_matches('-').contains('-');
    let function = ["sqrt", "sin", "cos", "tan", "log", "ln", "exp", "abs", "pi"].iter().any(|f| t.to_lowercase().contains(f));
    (has_digit && (operator || function || radix)) || (function && operator)
}

/// The calculator's mode for a sum: whole numbers at 64 bits for one written
/// in other bases or with bit operators, else the standard one.
fn mode_for(text: &str) -> crate::calc::Mode {
    let lower = text.to_lowercase();
    let bits = ["0x", "0b", "0o", "&", "|", "~", "<<", ">>"].iter().any(|p| lower.contains(p));
    if bits {
        crate::calc::Mode::Programmer { bits: 64 }
    } else {
        crate::calc::Mode::Standard { degrees: true }
    }
}

/// A unit by what was typed for it: its own name in any case, without its
/// dot or dash ("nm", "inlb"), or a word for it ("inches", "celsius").
fn unit_named(typed: &str) -> Vec<(usize, usize)> {
    let norm = |s: &str| s.chars().filter(|c| !"·-. ".contains(*c)).collect::<String>().to_lowercase();
    let word = match typed.to_lowercase().as_str() {
        "inch" | "inches" | "\"" => "in".to_owned(),
        "foot" | "feet" | "'" => "ft".into(),
        "meter" | "meters" | "metre" | "metres" => "m".into(),
        "pound" | "pounds" | "lbs" => "lb".into(),
        "newton" | "newtons" => "n".into(),
        "celsius" | "degc" | "c°" => "°c".into(),
        "fahrenheit" | "degf" => "°f".into(),
        "kelvin" => "k".into(),
        "kph" | "kmh" => "km/h".into(),
        "degree" | "degrees" => "deg".into(),
        "volts" | "volt" => "v".into(),
        "amps" | "amp" => "a".into(),
        "watts" | "watt" => "w".into(),
        "seconds" | "second" | "sec" => "s".into(),
        "minutes" | "minute" => "min".into(),
        "hours" | "hour" | "hr" => "h".into(),
        other => norm(other),
    };
    let mut found = Vec::new();
    for (k, kind) in crate::units::KINDS.iter().enumerate() {
        for (u, unit) in kind.units.iter().enumerate() {
            // Exactly as written first (mN·m is not MN·m), then in any case.
            if unit.name == typed || norm(unit.name) == word {
                found.push((k, u));
            }
        }
    }
    found
}

/// "3 in to mm", "3in mm", "12 ft·lb in N·m": an amount, its unit, and (if
/// given) the unit wanted.
fn parse_amount(text: &str) -> Option<(f64, String, Option<String>)> {
    let t = text.trim();
    let end = t.char_indices().find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || *c == ',' || (*i == 0 && *c == '-'))).map_or(t.len(), |(i, _)| i);
    let value: f64 = t[..end].replace(',', ".").parse().ok()?;
    let rest = t[end..].trim();
    if rest.is_empty() {
        return None;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    // The word between the two units ("in" is a unit too, so not the first word).
    let joins = ["to", "in", "as", "into", "=", "→", "->"];
    match words.iter().skip(1).position(|w| joins.contains(&w.to_lowercase().as_str())).map(|at| at + 1) {
        Some(at) if at + 1 < words.len() => Some((value, words[..at].join(""), Some(words[at + 1..].join("")))),
        Some(_) => None,
        None if words.len() == 2 => Some((value, words[0].to_owned(), Some(words[1].to_owned()))),
        None => Some((value, words.join(""), None)),
    }
}

/// The conversions an amount typed asks for: (kind, from unit, to unit).
fn conversions(text: &str) -> Option<(f64, usize, usize, Vec<usize>)> {
    let (value, from, to) = parse_amount(text)?;
    let froms = unit_named(&from);
    match to {
        Some(to) => {
            let tos = unit_named(&to);
            let (k, f, t) = froms.iter().find_map(|&(k, f)| tos.iter().find(|&&(k2, _)| k2 == k).map(|&(_, t)| (k, f, t)))?;
            Some((value, k, f, vec![t]))
        }
        None => {
            let &(k, f) = froms.first()?;
            // Into the kind's other units, the first few.
            let others: Vec<usize> = (0..crate::units::KINDS[k].units.len()).filter(|&u| u != f).take(3).collect();
            Some((value, k, f, others))
        }
    }
}

impl Provider for Sums {
    fn name(&self) -> &'static str {
        "sums"
    }

    fn answer(&self, query: &Query, _robot: &Robot, out: &mut Vec<Item>) {
        let typed = query.raw.as_str();
        if looks_like_math(typed) {
            let text = typed.trim_start_matches('=').trim();
            if let Ok(v) = crate::calc::eval(text, mode_for(text), None) {
                // A whole number in hex beside it, without the leading zeros of its width.
                let detail = match v {
                    crate::calc::Value::Int(n, _) if n >= 0 => format!("0x{n:X}"),
                    crate::calc::Value::Int(n, bits) => crate::calc::forms(n, bits)[0].1.clone(),
                    crate::calc::Value::Real(_) => "calculator".to_owned(),
                };
                let shown = match v {
                    crate::calc::Value::Int(n, _) => n.to_string(),
                    crate::calc::Value::Real(_) => crate::calc::shown(v),
                };
                out.push(Item::new(format!("sum:{text}"), format!("= {shown}"), "calculate", weight::OTHER).detail(detail).ranked(1));
            }
        }
        if let Some((value, kind, from, tos)) = conversions(typed) {
            let units = crate::units::KINDS[kind].units;
            for (k, &to) in tos.iter().enumerate() {
                let amount = crate::units::convert(value, &units[from], &units[to]);
                let title = format!("= {} {}", crate::units::amount(amount), units[to].name);
                out.push(Item::new(format!("units:{kind}:{from}:{to}:{value}"), title, "square_foot", weight::OTHER).detail(format!("{} {}", crate::units::amount(value), units[from].name)).ranked(2 + k as u32));
            }
        }
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        if let Some(text) = item.id.strip_prefix("sum:") {
            return Ok(Box::new(crate::calc::Calculator::with_input(text, mode_for(text))));
        }
        let parts: Vec<&str> = item.id.strip_prefix("units:").unwrap_or_default().split(':').collect();
        let [kind, from, _, value] = parts[..] else { return Err("not a conversion".into()) };
        let (kind, from) = (kind.parse().unwrap_or(0), from.parse().unwrap_or(0));
        Ok(Box::new(crate::units::Converter::at(kind, from, value)))
    }

    fn live_preview(&self, item: &Item, _robot: &Robot) -> Option<Preview> {
        if let Some(text) = item.id.strip_prefix("sum:") {
            return match crate::calc::eval(text, mode_for(text), None).ok()? {
                crate::calc::Value::Int(n, bits) => Some(Preview::Facts(crate::calc::forms(n, bits).into_iter().map(|(k, v)| (k.to_owned(), v)).collect())),
                v => Some(Preview::Facts(vec![("sum".into(), text.to_owned()), ("comes to".into(), crate::calc::shown(v))])),
            };
        }
        let parts: Vec<&str> = item.id.strip_prefix("units:")?.split(':').collect();
        let [kind, from, _, value] = parts[..] else { return None };
        let (kind, from, value): (usize, usize, f64) = (kind.parse().ok()?, from.parse().ok()?, value.parse().ok()?);
        let units = crate::units::KINDS.get(kind)?.units;
        Some(Preview::Facts(units.iter().map(|u| (u.name.to_owned(), crate::units::amount(crate::units::convert(value, &units[from], u)))).collect()))
    }
}

// -------------------------------------------------------------- elsewhere ---

/// The last two rows, whatever is typed: search the web for it, in Web, and
/// ask the assistant, which sends it there.
pub struct Elsewhere;

/// Text made safe for an address: letters, digits and `-_.~` as they are,
/// spaces as `+`, the rest as `%XX`.
fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

impl Provider for Elsewhere {
    fn name(&self) -> &'static str {
        "elsewhere"
    }

    fn answer(&self, query: &Query, _robot: &Robot, out: &mut Vec<Item>) {
        if query.is_empty() {
            return;
        }
        let q = query.raw.as_str();
        out.push(Item::new(format!("web:{q}"), format!("Search the web for “{q}”"), "travel_explore", weight::OTHER).detail("web").ranked(TAIL + 1));
        out.push(Item::new(format!("ask:{q}"), format!("Ask the Assistant “{q}”"), "auto_awesome", weight::OTHER).detail("assistant").ranked(TAIL + 2));
    }

    fn open(&self, item: &Item, _cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
        if let Some(q) = item.id.strip_prefix("web:") {
            let url = format!("https://lite.duckduckgo.com/lite/?q={}", url_encode(q));
            let line = format!("lynx -accept_all_cookies -nopause {}", crate::tools::shell_quote(&url));
            return Ok(Box::new(crate::terminal::Terminal::running("Web", &line, crate::connect::session_dir())));
        }
        let q = item.id.strip_prefix("ask:").unwrap_or_default();
        Ok(Box::new(crate::ai::Assistant::asking(q)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(provider: &dyn Provider, typed: &str, robot: &Robot) -> Vec<Item> {
        let mut out = Vec::new();
        provider.answer(&Query::new(typed), robot, &mut out);
        out
    }

    fn titles(items: &[Item]) -> Vec<String> {
        items.iter().map(|i| i.title.clone()).collect()
    }

    #[test]
    fn sums_are_worked_out_but_a_bare_number_is_not_one() {
        let none = Robot::none();
        assert_eq!(titles(&answers(&Sums, "12*7", &none)), ["= 84"]);
        assert_eq!(titles(&answers(&Sums, "(1+2)^2", &none)), ["= 9"]);
        assert!(titles(&answers(&Sums, "sqrt(2)", &none))[0].starts_with("= 1.41421"));
        let hex = answers(&Sums, "0x1f", &none);
        assert_eq!(titles(&hex), ["= 31"]);
        assert_eq!(hex[0].detail, "0x1F", "and in hex");
        assert!(answers(&Sums, "254", &none).is_empty(), "a team, not a sum");
        assert!(answers(&Sums, "q14", &none).is_empty());
        assert!(answers(&Sums, "wi-fi", &none).is_empty());
        assert!(looks_like_math("2 - 3") && !looks_like_math("-3") && !looks_like_math("pit notes"));
    }

    #[test]
    fn amounts_are_given_in_other_units() {
        let none = Robot::none();
        assert_eq!(titles(&answers(&Sums, "3 in to mm", &none)), ["= 76.2 mm"]);
        assert_eq!(titles(&answers(&Sums, "3in mm", &none)), ["= 76.2 mm"]);
        assert_eq!(titles(&answers(&Sums, "12 ft·lb in nm", &none)), ["= 16.26982 N·m"]);
        assert_eq!(titles(&answers(&Sums, "100 c to f", &none)), Vec::<String>::new(), "c is coulombs, not a temperature");
        assert_eq!(titles(&answers(&Sums, "100 celsius to fahrenheit", &none)), ["= 212 °F"]);
        // A unit alone: the kind's next units.
        let lb = answers(&Sums, "5 lb", &none);
        assert_eq!(lb.len(), 3);
        assert_eq!(lb[0].title, "= 2.267962 kg");
        assert!(answers(&Sums, "5 apples", &none).is_empty());
        // Enter opens the converter there.
        assert!(matches!(conversions("3 in to mm"), Some((v, 0, 4, ref to)) if v == 3.0 && to == &vec![2]));
    }

    #[test]
    fn match_numbers_are_read_as_the_schedule_writes_them() {
        assert_eq!(match_label("q14").as_deref(), Some("q14"));
        assert_eq!(match_label("qm 14").as_deref(), Some("q14"));
        assert_eq!(match_label("match 14").as_deref(), Some("q14"));
        assert_eq!(match_label("m3").as_deref(), Some("m3"));
        assert_eq!(match_label("f1").as_deref(), Some("f1"));
        assert_eq!(match_label("notes"), None);
        assert_eq!(match_label("254"), None);
    }

    #[test]
    fn a_team_or_a_match_number_finds_the_schedule() {
        let robot = Robot::demo();
        let view = robot.schedule.view().expect("the demo robot has a schedule");
        let first = &view.matches[0];
        let found = answers(&Schedule, &first.label, &robot);
        assert_eq!(found.first().map(|i| i.id.clone()), Some(first.key.clone()), "{:?}", titles(&found));
        let partner = first.red.iter().chain(&first.blue).find(|t| **t != view.team).copied().unwrap();
        let theirs = answers(&Schedule, &partner.to_string(), &robot);
        assert!(!theirs.is_empty() && theirs.iter().all(|i| i.title.contains(&partner.to_string())), "{:?}", titles(&theirs));
        assert!(!answers(&Schedule, "next", &robot).is_empty(), "the next match");
        assert!(answers(&Schedule, "q14", &Robot::none()).is_empty(), "no schedule, nothing");
    }

    #[test]
    fn robot_values_are_found_by_name_with_their_value() {
        let robot = Robot::demo();
        let found = answers(&Values::default(), "volts", &robot);
        let battery = found.iter().find(|i| i.id == "Systemcore/BatteryVolts").expect("found by the last part of its key");
        assert_eq!(battery.title, "BatteryVolts");
        assert!(battery.detail.starts_with("12."), "with its value: {}", battery.detail);
        assert!(found.len() <= VALUES_SHOWN);
        assert!(answers(&Values::default(), "systemcore", &robot).len() > 1, "and by any part of the key");
        assert!(answers(&Values::default(), "volts", &Robot::none()).is_empty());
    }

    #[test]
    fn the_web_and_the_assistant_are_offered_for_anything_typed() {
        let rows = answers(&Elsewhere, "swerve module offsets", &Robot::none());
        assert_eq!(titles(&rows), ["Search the web for “swerve module offsets”", "Ask the Assistant “swerve module offsets”"]);
        assert!(rows.iter().all(|r| r.rank.is_some_and(|r| r > TAIL)));
        assert!(answers(&Elsewhere, "   ", &Robot::none()).is_empty());
        assert_eq!(url_encode("q14 & 5805?"), "q14+%26+5805%3F");
    }

    #[test]
    fn every_page_and_switch_in_settings_is_found_with_its_section() {
        crate::shell::tests::sandbox();
        let items = Settings.items();
        let brightness = items.iter().find(|i| i.title == "Brightness").expect("Brightness");
        assert_eq!(brightness.detail, "settings › display");
        let display = items.iter().find(|i| i.title == "Display").expect("the section itself");
        assert_eq!(display.detail, "settings");
        assert!(items.iter().any(|i| i.title == "The Blue Alliance key" && i.detail.starts_with("settings › keys")));
        assert_eq!(items.len(), crate::screens::setting_names().count());
    }

    #[test]
    fn apps_go_by_other_names_too() {
        crate::shell::tests::sandbox();
        let items = Apps::default().items();
        let calc = items.iter().find(|i| i.title == "Calculator").unwrap();
        assert!(calc.words.contains("math") && calc.fuzzy);
        assert_eq!(items.iter().filter(|i| i.id.starts_with("builtin:")).count(), crate::screens::builtin_apps().count());
    }

    #[test]
    fn files_are_read_a_folder_at_a_time_and_skip_what_is_private() {
        let root = std::env::temp_dir().join(format!("catalyst-spotlight-files-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("code/robot/src")).unwrap();
        std::fs::create_dir_all(root.join("code/robot/build/classes")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("code/robot/src/Robot.java"), "class Robot {}\n").unwrap();
        std::fs::write(root.join("code/robot/build/classes/Robot.class"), [0u8, 1, 2]).unwrap();
        std::fs::write(root.join(".hidden/secret.txt"), "no").unwrap();
        std::fs::write(root.join("pit.txt"), "elevator ran hot\nswap battery 3\n").unwrap();
        let files = Files { root: Some(root.clone()), ..Files::default() };
        let found = |files: &Files| -> Vec<PathBuf> {
            let mut paths: Vec<PathBuf> = files.walk(&root, &[root.join("code/private")]).into_iter().map(|(p, ..)| p.strip_prefix(&root).unwrap().to_owned()).collect();
            paths.sort();
            paths
        };
        std::fs::create_dir_all(root.join("code/private")).unwrap();
        std::fs::write(root.join("code/private/key"), "secret").unwrap();
        let first = found(&files);
        assert!(!first.iter().any(|p| p.starts_with("code/private")), "what is left alone is not listed: {first:?}");
        assert!(first.contains(&PathBuf::from("code/robot/src/Robot.java")), "{first:?}");
        assert!(first.contains(&PathBuf::from("code/robot/build")), "a build folder is listed…");
        assert!(!first.iter().any(|p| p.ends_with("Robot.class")), "…but not looked into");
        assert!(!first.iter().any(|p| p.to_string_lossy().contains("hidden")));
        let stamp = files.stamp();
        assert_eq!(files.stamp(), stamp, "nothing changed");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("code/new.txt"), "x").unwrap();
        assert_ne!(files.stamp(), stamp, "a file added changes its folder");
        assert!(found(&files).contains(&PathBuf::from("code/new.txt")));
        // Found by their place under the storage, with where they are.
        let items = files.items();
        let java = items.iter().find(|i| i.title == "Robot.java").expect("Robot.java");
        assert_eq!((java.id.as_str(), java.detail.as_str()), ("file:code/robot/src/Robot.java", "15 b · in robot/src"));
        // A preview: the start of a text file, facts for anything else.
        let text = files.preview(&Item::new("file:pit.txt", "pit.txt", "description", weight::FILE));
        assert!(matches!(text, Some(Preview::Text { ref lines, .. }) if lines[0] == "elevator ran hot"), "{text:?}");
        let binary = files.preview(&Item::new("file:code/robot/build/classes/Robot.class", "Robot.class", "description", weight::FILE));
        assert!(matches!(binary, Some(Preview::Facts(_))));
        assert!(files.preview(&Item::new("file:../outside.txt", "outside.txt", "description", weight::FILE)).is_none(), "nothing above the storage");
        assert!(files.bytes() > 0);
        std::fs::remove_dir_all(&root).ok();
    }

    /// The index at its fullest: every built-in provider, the demo robot,
    /// and 4,000 files in 200 folders (the most it holds). How long it takes
    /// to read, to read again when one file is added, and to search; and how
    /// much memory it holds. `cargo test … index_at_its_fullest -- --nocapture`
    /// prints the numbers.
    #[test]
    fn index_at_its_fullest_is_small_and_quick() {
        use super::super::index::Index;
        crate::shell::tests::sandbox();
        let root = std::env::temp_dir().join(format!("catalyst-spotlight-measure-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let words = ["robot", "elevator", "intake", "swerve", "auto", "vision", "climber", "shooter", "notes", "match"];
        for d in 0..200 {
            let dir = root.join(format!("{}/{}-{d}", words[d % 10], words[(d / 10) % 10]));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..19 {
                std::fs::write(dir.join(format!("{}{}_{f}.java", words[(d + f) % 10], words[f % 10])), "x").unwrap();
            }
        }
        let files = Arc::new(Files { root: Some(root.clone()), ..Files::default() });
        let mut providers: Vec<Arc<dyn Provider>> = builtin().into_iter().filter(|p| p.name() != "files").collect();
        providers.push(files.clone());
        let index = Index::new(providers);
        let began = Instant::now();
        index.refresh(true);
        let cold = began.elapsed();
        let stats = index.stats();
        // The files' part on its own (the other parts read folders other
        // tests write to meanwhile): nothing changed, then one file added.
        let alone = Index::new(vec![files.clone()]);
        alone.refresh(true);
        let began = Instant::now();
        assert!(!alone.refresh(true), "nothing changed, nothing read");
        let unchanged = began.elapsed();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("robot/robot-0/added.txt"), "x").unwrap();
        let began = Instant::now();
        assert!(alone.refresh(false), "the file added is read");
        let one_added = began.elapsed();
        assert!(alone.search(&Query::new("added"), &Robot::none(), 40).iter().any(|h| h.item.title == "added.txt"));
        // Searching: each query as a key would send it, a hundred times.
        let robot = Robot::demo();
        let mut timings = Vec::new();
        for typed in ["e", "ro", "swe", "robot", "elevatorint", "swerve offsets", "volts", "12*7", "q31"] {
            let q = Query::new(typed);
            let began = Instant::now();
            let mut found = 0;
            for _ in 0..100 {
                found = index.search(&q, &robot, 40).len();
            }
            timings.push((typed, began.elapsed() / 100, found));
        }
        println!("spotlight index: {} items, {:.0} kb held", stats.items, stats.bytes as f64 / 1024.0);
        println!("  read cold {cold:?} · nothing changed {unchanged:?} · one file added {one_added:?}");
        for (typed, each, found) in &timings {
            println!("  search {typed:>16}: {each:?} ({found} results)");
        }
        assert!(stats.items > 4000 && stats.items < 4400, "{}", stats.items);
        assert!(stats.bytes < 1536 * 1024, "{} bytes", stats.bytes);
        let slowest = timings.iter().map(|t| t.1).max().unwrap();
        assert!(slowest < std::time::Duration::from_millis(15), "slowest search {slowest:?}");
        std::fs::remove_dir_all(&root).ok();
    }
}
