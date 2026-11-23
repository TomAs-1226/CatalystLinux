//! Notifications: what the handheld's apps and the system have to tell,
//! kept until they are seen and put away.
//!
//! - **A notification** has its app (and the app's symbol), a title, what it
//!   says, when, and where it leads, if anywhere (`action`: a reminder, a
//!   note, a match's summary, an app, a place in Settings). A newer one with
//!   the same `tag` takes an older one's place (the robot linked again, an
//!   update a stage further).
//! - **Kept** newest first, at most [`CAP`], in
//!   `/data/catalyst/notifications.json`, written on a thread of its own
//!   whenever they change: they survive a restart, and nothing is read
//!   back but at start.
//! - **Posted** from any thread ([`post`]): the shell takes them in on its
//!   next turn (a post from another thread wakes it), says each in the
//!   island, and with the screen off blinks the Fn light until it wakes.
//!   Nothing is polled: each comes from the event it tells of.
//! - **Quiet** (do not disturb, in the control centre): they still collect,
//!   but the island and the lights say nothing, except for what breaks
//!   through (`urgent`): a reminder come due, a timer run out, and queue now.
//!   Those were asked for at a time, or cost a match if missed.
//! - **The notification centre** ([`Centre`]): the control centre's sheet,
//!   turned to them (Ctrl-Backspace, or Notifications on the control centre).
//!   By app, the app with the newest first; Enter goes where one leads (and
//!   puts it away), Fn-Backspace puts one away, Clear all at the foot.

use std::path::PathBuf;

use bezel::{Align, Rect};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::gfx::Gfx;
use crate::platform::Key;
use crate::shell::{Cx, Screen};

/// At most this many are kept; the oldest go first.
pub const CAP: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u64,
    /// The app it is from, by its name ("Reminders"), and its symbol.
    pub app: String,
    pub icon: String,
    pub title: String,
    pub body: String,
    /// When, in Unix seconds.
    pub at: i64,
    /// Where Enter takes it: "reminder:12", "note:file.md", "log:<name>",
    /// "builtin:<app>", "setting:<place>". None: its app, if built in.
    pub action: Option<String>,
    /// One of a kind: a newer one with the same tag replaces it.
    pub tag: Option<String>,
    pub read: bool,
    /// Breaks through Quiet.
    pub urgent: bool,
}

impl Notification {
    pub fn new(app: &str, icon: &str, title: impl Into<String>) -> Notification {
        Notification {
            id: 0,
            app: app.to_owned(),
            icon: icon.to_owned(),
            title: title.into(),
            body: String::new(),
            at: chrono::Utc::now().timestamp(),
            action: None,
            tag: None,
            read: false,
            urgent: false,
        }
    }

    /// From what the island says: its words up to the first " · " are the
    /// title, the rest what it says ("update 0.6.2 ready · restart in
    /// settings").
    pub fn said_as(app: &str, icon: &str, said: &str) -> Notification {
        let (title, body) = said.split_once(" · ").unwrap_or((said, ""));
        Notification::new(app, icon, title).body(body)
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn urgent(mut self) -> Self {
        self.urgent = true;
        self
    }

    /// When, set (the tests and the gallery).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn at(mut self, at: i64) -> Self {
        self.at = at;
        self
    }

    /// What the island says for it: its title, and what it says if short.
    pub fn said(&self) -> String {
        if self.body.is_empty() || self.body.chars().count() > 28 {
            self.title.clone()
        } else {
            format!("{} · {}", self.title, self.body)
        }
    }

    /// Where Enter takes it: its own place, else its app if built in.
    pub fn leads_to(&self) -> Option<String> {
        self.action.clone().or_else(|| crate::screens::builtin_apps().any(|(_, _, n)| n == self.app).then(|| format!("builtin:{}", self.app)))
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id, "app": self.app, "icon": self.icon, "title": self.title, "body": self.body,
            "at": self.at, "action": self.action, "tag": self.tag, "read": self.read, "urgent": self.urgent,
        })
    }

    fn from_json(v: &Value) -> Option<Notification> {
        let text = |k: &str| v[k].as_str().map(str::to_owned);
        Some(Notification {
            id: v["id"].as_u64()?,
            app: text("app")?,
            icon: text("icon").unwrap_or_else(|| "notifications".into()),
            title: text("title")?,
            body: text("body").unwrap_or_default(),
            at: v["at"].as_i64()?,
            action: text("action"),
            tag: text("tag"),
            read: v["read"].as_bool().unwrap_or(false),
            urgent: v["urgent"].as_bool().unwrap_or(false),
        })
    }
}

// ----------------------------------------------------------------- store ---

/// The notifications kept, newest first, and Quiet.
pub struct Store {
    list: Vec<Notification>,
    next: u64,
    quiet: bool,
    /// Where they are kept (none: in memory only, as in the tests).
    path: Option<PathBuf>,
    /// When the lock last engaged (or StandBy came): what came after is said
    /// in the island once it is unlocked.
    mark: i64,
    /// Bumped on every change, for whoever draws them.
    pub generation: u64,
}

impl Store {
    pub fn memory() -> Store {
        Store { list: Vec::new(), next: 1, quiet: false, path: None, mark: 0, generation: 0 }
    }

    /// Those kept on the card (in memory alone for the tests: each test's
    /// shell has its own).
    pub fn kept() -> Store {
        if cfg!(test) {
            Store::memory()
        } else {
            Store::open(crate::config_dir().join("notifications.json"))
        }
    }

    pub fn open(path: PathBuf) -> Store {
        let mut store = Store { path: Some(path.clone()), ..Store::memory() };
        let Ok(text) = std::fs::read_to_string(&path) else { return store };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { return store };
        store.list = v["list"].as_array().map(|a| a.iter().filter_map(Notification::from_json).collect()).unwrap_or_default();
        store.list.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
        store.list.truncate(CAP);
        store.quiet = v["quiet"].as_bool().unwrap_or(false);
        let highest = store.list.iter().map(|n| n.id).max().unwrap_or(0);
        store.next = v["next"].as_u64().unwrap_or(1).max(highest + 1);
        store
    }

    fn text(&self) -> String {
        let v = json!({ "next": self.next, "quiet": self.quiet, "list": self.list.iter().map(Notification::to_json).collect::<Vec<_>>() });
        serde_json::to_string(&v).unwrap_or_default()
    }

    /// Written on a thread of its own (never the frame's time), through a
    /// file beside it so a write cut short never loses what was kept. The
    /// threads may run in any order: one that finds a newer change after it
    /// leaves the writing to that one's.
    fn changed(&mut self) {
        self.generation += 1;
        let Some(path) = self.path.clone() else { return };
        let text = self.text();
        let seq = {
            let mut newest = NEWEST.lock().unwrap_or_else(|e| e.into_inner());
            let seq = newest.iter().map(|(_, s)| *s).max().unwrap_or(0) + 1;
            newest.retain(|(p, _)| *p != path);
            newest.push((path.clone(), seq));
            seq
        };
        std::thread::Builder::new()
            .name("notifications".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let newest = NEWEST.lock().unwrap_or_else(|e| e.into_inner());
                if !newest.iter().any(|(p, s)| *p == path && *s == seq) {
                    return;
                }
                let part = path.with_extension("json.part");
                if std::fs::write(&part, text).is_ok() {
                    std::fs::rename(&part, &path).ok();
                }
            })
            .ok();
    }

    /// Kept, newest first: in place of one with its tag, if there is one.
    /// `read`: seen already (its app is in front). Its id.
    pub fn add(&mut self, mut n: Notification, read: bool) -> u64 {
        if let Some(tag) = &n.tag {
            self.list.retain(|old| old.tag.as_ref() != Some(tag));
        }
        n.id = self.next;
        n.read = read;
        self.next += 1;
        let id = n.id;
        let at = self.list.iter().position(|old| old.at <= n.at).unwrap_or(self.list.len());
        self.list.insert(at, n);
        self.list.truncate(CAP);
        self.changed();
        id
    }

    pub fn list(&self) -> &[Notification] {
        &self.list
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get(&self, id: u64) -> Option<&Notification> {
        self.list.iter().find(|n| n.id == id)
    }

    /// Put away.
    pub fn dismiss(&mut self, id: u64) -> Option<Notification> {
        let at = self.list.iter().position(|n| n.id == id)?;
        let n = self.list.remove(at);
        self.changed();
        Some(n)
    }

    pub fn clear(&mut self) {
        if !self.list.is_empty() {
            self.list.clear();
            self.changed();
        }
    }

    /// All seen (the notification centre was opened on them).
    pub fn read_all(&mut self) {
        if self.list.iter().any(|n| !n.read) {
            self.list.iter_mut().for_each(|n| n.read = true);
            self.changed();
        }
    }

    pub fn unread(&self) -> usize {
        self.list.iter().filter(|n| !n.read).count()
    }

    /// Those not yet seen, newest first.
    pub fn unread_list(&self) -> Vec<Notification> {
        self.list.iter().filter(|n| !n.read).cloned().collect()
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    pub fn set_quiet(&mut self, on: bool) {
        if self.quiet != on {
            self.quiet = on;
            self.changed();
        }
    }

    /// From now: what comes is said once it is unlocked.
    pub fn mark(&mut self) {
        self.mark = chrono::Utc::now().timestamp();
    }

    /// Not yet seen, and come since the mark: newest first.
    pub fn since_mark(&self) -> Vec<&Notification> {
        self.list.iter().filter(|n| !n.read && n.at >= self.mark).collect()
    }

    /// By app: each app's (name, symbol, its notifications' places in the
    /// list), the app with the newest first.
    pub fn groups(&self) -> Vec<(String, String, Vec<usize>)> {
        let mut out: Vec<(String, String, Vec<usize>)> = Vec::new();
        for (i, n) in self.list.iter().enumerate() {
            match out.iter_mut().find(|(app, ..)| *app == n.app) {
                Some((.., at)) => at.push(i),
                None => out.push((n.app.clone(), n.icon.clone(), vec![i])),
            }
        }
        out
    }
}

/// Each file's newest change, by number: only its writer writes (one write
/// at a time, under this lock).
static NEWEST: std::sync::Mutex<Vec<(PathBuf, u64)>> = std::sync::Mutex::new(Vec::new());

// ----------------------------------------------------------------- posts ---

#[cfg(not(test))]
static POSTED: std::sync::Mutex<Vec<Notification>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
thread_local! {
    static POSTED: std::cell::RefCell<Vec<Notification>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// From any thread: a notification, for the shell to take in at its next
/// turn (and to wake it for, if it is waiting on the keys).
pub fn post(n: Notification) {
    #[cfg(not(test))]
    POSTED.lock().unwrap_or_else(|e| e.into_inner()).push(n);
    #[cfg(test)]
    POSTED.with(|p| p.borrow_mut().push(n));
    crate::platform::wake();
}

/// What was posted since last asked, oldest first.
pub fn take_posted() -> Vec<Notification> {
    #[cfg(not(test))]
    return std::mem::take(&mut *POSTED.lock().unwrap_or_else(|e| e.into_inner()));
    #[cfg(test)]
    POSTED.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// How long ago, as a notification says it: "now", "5 m", "2 h", then the
/// day ("yesterday", "3 d").
pub fn ago(at: i64, now: i64) -> String {
    let s = (now - at).max(0);
    match s {
        0..=59 => "now".to_owned(),
        60..=3599 => format!("{} m", s / 60),
        3600..=86_399 => format!("{} h", s / 3600),
        86_400..=172_799 => "yesterday".to_owned(),
        _ => format!("{} d", s / 86_400),
    }
}

/// Where a notification leads, opened: a reminder, a note, a match's
/// summary, a built-in app, a place in Settings.
pub fn open(action: &str, cx: &mut Cx) -> Result<Box<dyn Screen>, String> {
    let (kind, what) = action.split_once(':').ok_or("nowhere to go")?;
    match kind {
        "builtin" => {
            let (at, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == what).ok_or("that app is not on this handheld")?;
            Ok(crate::screens::open_builtin(at, cx.robot))
        }
        "setting" => {
            let (at, _) = crate::screens::setting_names().find(|(_, n)| *n == what).ok_or("that setting has moved")?;
            Ok(crate::screens::open_setting(at))
        }
        "reminder" => {
            let id = what.parse().map_err(|_| "that reminder is gone")?;
            Ok(crate::reminders::open(id))
        }
        // The smart camera on USB-A: its install page.
        "unitv" => Ok(Box::new(crate::unitv::InstallScreen::new())),
        "note" => Ok(Box::new(crate::files::Editor::open(&crate::notes::dir().join(what)).map_err(|_| "that note is gone")?)),
        "log" => {
            let row = cx.robot.logs.list().into_iter().find(|r| r.name == what).ok_or("that log is gone")?;
            Ok(Box::new(crate::screens::LogDetail::of(&row).ok_or("that log has no summary yet")?))
        }
        _ => Err("nowhere to go".into()),
    }
}

// ----------------------------------------------------------------- robot ---

/// The robot, as it tells of itself: a brownout seen live (the controller
/// cutting the motors to keep itself running), once each time it begins.
#[derive(Default)]
pub struct RobotWatch {
    browned: bool,
}

impl RobotWatch {
    /// Looked at as the robot's values come in (the shell's link watch).
    pub fn look(&mut self, robot: &crate::robot::Robot) -> Option<Notification> {
        let now = robot.linked() && robot.bool("Systemcore/BrownedOut") == Some(true);
        let began = now && !self.browned;
        self.browned = now;
        if !began {
            return None;
        }
        let name = robot.string("Robot/Identity/Name").unwrap_or_else(|| "the robot".into());
        let volts = robot.f64("Systemcore/BatteryVolts").map(|v| format!("{name}'s battery at {v:.1} V")).unwrap_or_else(|| format!("{name} cut its motors"));
        Some(Notification::new("Robot", "bolt", "Brownout").body(volts).action("builtin:Robot"))
    }
}

// ---------------------------------------------------------------- centre ---

/// A line of the notification centre: an app's heading, one of its
/// notifications (its place in the list), or Clear all.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Line {
    Head(usize),
    Note(usize),
    Clear,
}

/// What a key in the notification centre did.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Did {
    Nothing,
    /// Put it away.
    Close,
    /// Turned to the control centre (Ctrl-Space).
    Controls,
    /// Go where this one leads (it is put away).
    Open(u64),
}

/// The notification centre's page of the sheet: its focus, and the list
/// gliding to keep it in view.
pub struct Centre {
    focus: usize,
    scroll: Animated,
    ring: [Animated; 4],
    ring_placed: bool,
    /// Those not yet seen as it opened: each keeps its dot while it shows.
    fresh: Vec<u64>,
}

impl Default for Centre {
    fn default() -> Self {
        Self::new()
    }
}

impl Centre {
    pub fn new() -> Centre {
        Centre {
            focus: 0,
            scroll: Animated::new(0.0, role::smooth()).with_rest(0.3),
            ring: std::array::from_fn(|_| Animated::new(0.0, role::hold()).with_rest(0.05)),
            ring_placed: false,
            fresh: Vec::new(),
        }
    }

    /// Turned to: from the top, the newest first, and everything in it seen.
    pub fn opened(&mut self, store: &mut Store) {
        self.focus = 0;
        self.scroll.jump(0.0);
        self.ring_placed = false;
        self.fresh = store.list().iter().filter(|n| !n.read).map(|n| n.id).collect();
        store.read_all();
    }

    /// By app, the newest app first; an app with more than one has its
    /// heading (one alone shows its symbol, which says whose it is).
    fn lines(store: &Store) -> Vec<Line> {
        let mut out = Vec::new();
        for (g, (_, _, at)) in store.groups().iter().enumerate() {
            if at.len() > 1 {
                out.push(Line::Head(g));
            }
            out.extend(at.iter().map(|i| Line::Note(*i)));
        }
        if !out.is_empty() {
            out.push(Line::Clear);
        }
        out
    }

    /// The lines that take the focus, by their place among the lines.
    fn stops(lines: &[Line]) -> Vec<usize> {
        lines.iter().enumerate().filter(|(_, l)| !matches!(l, Line::Head(_))).map(|(i, _)| i).collect()
    }

    /// A key while it shows.
    pub fn key(&mut self, key: Key, store: &mut Store) -> Did {
        let lines = Self::lines(store);
        let stops = Self::stops(&lines);
        self.focus = self.focus.min(stops.len().saturating_sub(1));
        let at = stops.get(self.focus).map(|i| lines[*i]);
        match key {
            Key::Back | Key::Leave | Key::Ctrl('\u{8}') => Did::Close,
            Key::Ctrl(' ') => Did::Controls,
            // Up past the top pushes the sheet back where it came from.
            Key::Up if self.focus == 0 => Did::Close,
            Key::Up => {
                self.focus -= 1;
                Did::Nothing
            }
            Key::Down => {
                self.focus = (self.focus + 1).min(stops.len().saturating_sub(1));
                Did::Nothing
            }
            Key::PageUp | Key::Home => {
                self.focus = 0;
                Did::Nothing
            }
            Key::PageDown | Key::End => {
                self.focus = stops.len().saturating_sub(1);
                Did::Nothing
            }
            Key::Enter | Key::Char(' ') => match at {
                Some(Line::Note(i)) => Did::Open(store.list()[i].id),
                Some(Line::Clear) => {
                    store.clear();
                    self.focus = 0;
                    Did::Nothing
                }
                _ => Did::Nothing,
            },
            Key::Delete => {
                if let Some(Line::Note(i)) = at {
                    let id = store.list()[i].id;
                    store.dismiss(id);
                }
                Did::Nothing
            }
            _ => Did::Nothing,
        }
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.scroll.tick(dt);
        for spring in &mut self.ring {
            moving |= spring.tick(dt);
        }
        moving
    }

    /// The page on the sheet's glass, `inner` the sheet's room (where it is
    /// now, the sheet moving). Each notification: its app's symbol, its
    /// title and when, and under them what it says (or its app); an app
    /// with more than one has a heading over them. The list fades out at
    /// its top and foot as it moves, never cut.
    pub fn draw(&mut self, g: &mut Gfx, store: &Store, inner: Rect, quiet: bool) {
        let t = g.tokens;
        let ty = g.ty;
        // The heading: the name, and Quiet if it is on.
        let head_cy = inner.y + 8.0;
        g.text_at(inner.x + 4.0, head_cy, Align::Left, "Notifications", ty.name, t.ink);
        if quiet {
            let w = g.measure("quiet", ty.caption);
            let r = Rect::new(inner.right() - w - 26.0, head_cy - 9.0, w + 26.0, 18.0);
            g.canvas.rrect(r, 9.0, t.surface2, 0.9);
            g.icon("bedtime", true, r.x + 10.0, r.cy(), 11.0, t.ink);
            g.text_at(r.x + 18.0, r.cy(), Align::Left, "quiet", ty.caption, t.ink);
        }
        let top = (inner.y + ty.name.size * 0.5 + 16.0).round();
        let lines = Self::lines(store);
        if lines.is_empty() {
            let cy = (top + inner.bottom()) * 0.5;
            g.icon("notifications", false, inner.cx(), cy - 22.0, 22.0, t.dim);
            g.text_at(inner.cx(), cy + 4.0, Align::Center, "No notifications", ty.body, t.ink);
            let words = if quiet { "quiet: they wait here, unsaid" } else { "what apps tell you waits here" };
            let words = g.text.fit(words, &ty.caption, inner.w - 8.0);
            g.text_at(inner.cx(), cy + 22.0, Align::Center, &words, ty.caption, t.dim);
            return;
        }
        let groups = store.groups();
        let stops = Self::stops(&lines);
        self.focus = self.focus.min(stops.len().saturating_sub(1));
        let focused = stops[self.focus];
        // Each line's height and place in the list.
        let (head_h, note_h, clear_h) = (ty.caption.size + 8.0, ty.body.size + ty.caption.size + 13.0, ty.body.size + 14.0);
        let gap = 4.0;
        let mut ys = Vec::with_capacity(lines.len());
        let mut y = 0.0;
        for line in &lines {
            let h = match line {
                Line::Head(_) => head_h,
                Line::Note(_) => note_h,
                Line::Clear => clear_h,
            };
            ys.push((y, h));
            y += h + gap;
        }
        let total = y - gap;
        let view = inner.bottom() + 4.0 - top;
        // Keep the focus in view, with its app's heading if it is the first.
        let (fy, fh) = ys[focused];
        let want_top = if focused > 0 && matches!(lines[focused - 1], Line::Head(_)) { ys[focused - 1].0 } else { fy };
        let mut scroll = self.scroll.target();
        if want_top - 4.0 < scroll {
            scroll = want_top - 4.0;
        } else if fy + fh + 6.0 > scroll + view {
            scroll = fy + fh + 6.0 - view;
        }
        let scroll = scroll.clamp(0.0, (total - view + 8.0).max(0.0));
        if (scroll - self.scroll.target()).abs() > 0.5 {
            if g.calm {
                self.scroll.jump(scroll);
            } else {
                self.scroll.set(scroll);
                g.started_moving();
            }
        }
        let off = self.scroll.value();
        // What is beneath the list's two edges, to fade the rows into there.
        let band = |g: &Gfx, from: f32, to: f32| -> (usize, Vec<u32>) {
            let (a, b) = ((from.max(0.0) as usize).min(crate::gfx::H as usize), (to.max(0.0) as usize).min(crate::gfx::H as usize));
            let w = crate::gfx::W as usize;
            (a, g.canvas.px[a * w..b * w].to_vec())
        };
        let (fade_top, fade_foot) = (6.0, 14.0);
        let above = band(g, top - 4.0, top - 4.0 + fade_top);
        let below = band(g, inner.bottom() + 4.0 - fade_foot, inner.bottom() + 4.0);
        g.canvas.set_clip(Rect::new(inner.x - 4.0, top - 4.0, inner.w + 8.0, view + 4.0));
        let now = chrono::Utc::now().timestamp();
        let mut ring = None;
        for (i, line) in lines.iter().enumerate() {
            let (ly, h) = ys[i];
            let y = top + ly - off;
            if y > inner.bottom() + 6.0 || y + h < top - 6.0 {
                continue;
            }
            let r = Rect::new(inner.x, y, inner.w, h);
            match *line {
                Line::Head(gi) => {
                    let (app, _, at) = &groups[gi];
                    g.text_at(r.x + 6.0, r.cy() + 1.0, Align::Left, &app.to_lowercase(), ty.caption, t.dim);
                    g.text_at(r.right() - 6.0, r.cy() + 1.0, Align::Right, &at.len().to_string(), ty.caption, t.faint);
                }
                Line::Note(ni) => {
                    let n = &store.list()[ni];
                    g.canvas.rrect(r, 12.0, t.surface1, 0.72);
                    let first_cy = r.y + 6.5 + ty.body.size * 0.5;
                    if self.fresh.contains(&n.id) {
                        g.canvas.circle(r.x + 7.0, first_cy, 2.5, t.signal, 1.0);
                    }
                    g.icon(&n.icon, true, r.x + 19.0, first_cy, 13.0, t.ink);
                    let x = r.x + 32.0;
                    let when = ago(n.at, now);
                    let ww = g.measure(&when, ty.caption);
                    g.text_at(r.right() - 10.0, first_cy, Align::Right, &when, ty.caption, t.dim);
                    let title = g.text.fit(&n.title, &ty.body, r.right() - 10.0 - ww - 8.0 - x);
                    g.text_at(x, first_cy, Align::Left, &title, ty.body, t.ink);
                    let body = if n.body.is_empty() { n.app.to_lowercase() } else { n.body.lines().next().unwrap_or("").to_owned() };
                    let body = g.text.fit(&body, &ty.caption, r.right() - 10.0 - x);
                    g.text_at(x, r.bottom() - 6.0 - ty.caption.size * 0.5, Align::Left, &body, ty.caption, t.dim);
                }
                Line::Clear => {
                    g.canvas.rrect(r, 12.0, t.surface1, 0.72);
                    let w = g.measure("Clear all", ty.body);
                    let x = r.cx() - (w + 20.0) * 0.5;
                    g.icon("delete", false, x + 7.0, r.cy(), 14.0, t.ink);
                    g.text_at(x + 20.0, r.cy(), Align::Left, "Clear all", ty.body, t.ink);
                }
            }
            if i == focused {
                ring = Some((r, ly));
            }
        }
        if let Some((r, ly)) = ring {
            // The ring glides between lines, its place kept as in the list
            // (it rides the scroll and the sheet with them).
            let at = [r.x, ly, r.w, r.h];
            for (spring, to) in self.ring.iter_mut().zip(at) {
                if !self.ring_placed || g.calm {
                    spring.jump(to);
                } else if spring.target() != to {
                    spring.set(to);
                    g.started_moving();
                }
            }
            self.ring_placed = true;
            let [x, y, w, h] = [0, 1, 2, 3].map(|i| self.ring[i].value());
            g.canvas.stroke_rrect(Rect::new(x, top + y - off, w, h).inset(-2.5), 14.5, 1.5, g.ring_color(), 1.0);
        }
        g.canvas.clear_clip();
        // The edges: what scrolled under them fades into the glass there,
        // only where there is more beyond.
        let w = crate::gfx::W as usize;
        let fade = |g: &mut Gfx, (from, under): &(usize, Vec<u32>), down: bool| {
            let rows = under.len() / w;
            for k in 0..rows {
                let t = (k as f32 + 0.5) / rows as f32;
                let gone = if down { t * (2.0 - t) } else { 1.0 - t * t };
                let at = (from + k) * w;
                bezel::simd::mix_into(&mut g.canvas.px[at..at + w], Some(&under[k * w..(k + 1) * w]), (gone * 256.0) as u32);
            }
        };
        if off > 0.5 {
            fade(g, &above, false);
        }
        if off + view < total + 6.0 {
            fade(g, &below, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(app: &str, title: &str, at: i64) -> Notification {
        Notification::new(app, "notifications", title).at(at)
    }

    #[test]
    fn kept_newest_first_at_most_the_cap_and_a_tag_takes_its_place() {
        let mut s = Store::memory();
        for i in 0..(CAP as i64 + 5) {
            s.add(n("Clock", &format!("timer {i}"), 1_000 + i), false);
        }
        assert_eq!(s.list().len(), CAP);
        assert_eq!(s.list()[0].title, format!("timer {}", CAP + 4), "the newest first");
        assert!(s.list().windows(2).all(|w| w[0].at >= w[1].at));
        // One that arrives late (its time earlier) goes in its place.
        s.add(n("Logs", "late", 1_010), false);
        assert!(s.list().iter().position(|x| x.title == "late").unwrap() > 0);
        // The robot linked twice: one notification.
        s.add(n("Robot", "Atlas linked", 2_000).tag("robot:link"), false);
        s.add(n("Robot", "Atlas linked again", 2_001).tag("robot:link"), false);
        assert_eq!(s.list().iter().filter(|x| x.tag.as_deref() == Some("robot:link")).count(), 1);
        assert_eq!(s.list()[0].title, "Atlas linked again");
    }

    #[test]
    fn grouped_by_app_the_newest_app_first() {
        let mut s = Store::memory();
        s.add(n("Clock", "a", 1), false);
        s.add(n("Reminders", "b", 2), false);
        s.add(n("Clock", "c", 3), false);
        let groups: Vec<(String, usize)> = s.groups().into_iter().map(|(app, _, at)| (app, at.len())).collect();
        assert_eq!(groups, vec![("Clock".to_owned(), 2), ("Reminders".to_owned(), 1)]);
    }

    #[test]
    fn they_survive_a_restart_with_quiet() {
        let dir = std::env::temp_dir().join(format!("catalyst-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notifications.json");
        std::fs::remove_file(&path).ok();
        let mut s = Store::open(path.clone());
        let id = s.add(n("Reminders", "swap the battery", 100).body("after Q31").action("reminder:4").urgent(), false);
        s.add(n("Clock", "Timer done", 200), true);
        s.set_quiet(true);
        // Written on a thread: wait for it.
        for _ in 0..200 {
            if Store::open(path.clone()).list().len() == 2 && Store::open(path.clone()).quiet() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let back = Store::open(path.clone());
        assert_eq!(back.list().len(), 2);
        assert!(back.quiet());
        let r = back.get(id).expect("kept");
        assert_eq!((r.title.as_str(), r.body.as_str(), r.action.as_deref(), r.urgent, r.read), ("swap the battery", "after Q31", Some("reminder:4"), true, false));
        assert_eq!(back.unread(), 1);
        // New ones go on from the highest id.
        let mut back = back;
        assert!(back.add(n("Logs", "x", 300), false) > id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seen_put_away_and_cleared() {
        let mut s = Store::memory();
        let a = s.add(n("Clock", "a", 1), false);
        s.add(n("Clock", "b", 2), false);
        assert_eq!(s.unread(), 2);
        s.read_all();
        assert_eq!(s.unread(), 0);
        assert!(s.dismiss(a).is_some() && s.get(a).is_none());
        s.clear();
        assert!(s.list().is_empty());
    }

    #[test]
    fn the_centre_walks_its_list_opens_puts_away_and_clears() {
        let mut s = Store::memory();
        let first = s.add(n("Clock", "Timer done", 1).action("builtin:Clock"), false);
        let second = s.add(n("Reminders", "swap the battery", 2), false);
        let mut c = Centre::new();
        c.opened(&mut s);
        assert_eq!(s.unread(), 0, "seen once opened");
        assert_eq!(c.fresh.len(), 2, "with their dots while it shows");
        // The newest app's first: Reminders, then Clock, then Clear all.
        assert_eq!(c.key(Key::Enter, &mut s), Did::Open(second));
        assert_eq!(c.key(Key::Down, &mut s), Did::Nothing);
        assert_eq!(c.key(Key::Enter, &mut s), Did::Open(first));
        c.key(Key::Delete, &mut s);
        assert!(s.get(first).is_none(), "Fn-Backspace puts it away");
        // Down to Clear all: everything goes.
        c.key(Key::Down, &mut s);
        c.key(Key::Enter, &mut s);
        assert!(s.list().is_empty());
        // Up from the top, Esc, Ctrl-Backspace: away. Ctrl-Space: the controls.
        assert_eq!(c.key(Key::Up, &mut s), Did::Close);
        assert_eq!(c.key(Key::Back, &mut s), Did::Close);
        assert_eq!(c.key(Key::Ctrl('\u{8}'), &mut s), Did::Close);
        assert_eq!(c.key(Key::Ctrl(' '), &mut s), Did::Controls);
    }

    #[test]
    fn said_as_time_passes() {
        assert_eq!(ago(100, 130), "now");
        assert_eq!(ago(0, 300), "5 m");
        assert_eq!(ago(0, 7_300), "2 h");
        assert_eq!(ago(0, 90_000), "yesterday");
        assert_eq!(ago(0, 3 * 86_400 + 5), "3 d");
        let t = n("Clock", "Timer done", 0).body("5 min");
        assert_eq!(t.said(), "Timer done · 5 min");
        assert_eq!(n("Assistant", "Answer ready", 0).body("a very long answer that goes on and on").said(), "Answer ready");
    }

    #[test]
    fn a_brownout_is_told_once_as_it_begins() {
        let robot = crate::robot::Robot::demo();
        let mut w = RobotWatch::default();
        assert!(w.look(&robot).is_none(), "the demo robot is not browned out");
        w.browned = false;
        assert!(w.look(&crate::robot::Robot::none()).is_none());
    }

    #[test]
    fn where_one_leads() {
        assert_eq!(n("Clock", "Timer done", 0).leads_to().as_deref(), Some("builtin:Clock"), "its app, built in");
        assert_eq!(n("Clock", "x", 0).action("reminder:3").leads_to().as_deref(), Some("reminder:3"));
        assert_eq!(n("Battery", "x", 0).leads_to(), None, "no app of that name");
    }
}
