//! Practice: a cycle timer for drive practice. Space starts the first cycle
//! and splits each one after; the cycle is the driver's chosen with Left
//! and Right. Enter ends the session, which is kept (`/data/scout/practice/`)
//! with each driver's cycles, average and best.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub driver: String,
    pub secs: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Session {
    /// When it began, Unix seconds.
    pub started: i64,
    pub splits: Vec<Split>,
}

/// How many cycles, their average and the best, seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub cycles: usize,
    pub mean: f32,
    pub best: f32,
}

impl Session {
    /// Every cycle's, or one driver's.
    pub fn summary(&self, driver: Option<&str>) -> Option<Summary> {
        let times: Vec<f32> = self.splits.iter().filter(|s| driver.is_none_or(|d| s.driver == d)).map(|s| s.secs).collect();
        (!times.is_empty()).then(|| Summary { cycles: times.len(), mean: times.iter().sum::<f32>() / times.len() as f32, best: times.iter().copied().fold(f32::MAX, f32::min) })
    }

    /// The drivers in it, in the order they first drove.
    pub fn drivers(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.splits {
            if !out.contains(&s.driver) {
                out.push(s.driver.clone());
            }
        }
        out
    }

    pub fn to_json(&self) -> Value {
        json!({"started": self.started, "splits": self.splits.iter().map(|s| json!({"driver": s.driver, "secs": (s.secs * 100.0).round() / 100.0})).collect::<Vec<_>>()})
    }

    pub fn from_json(v: &Value) -> Option<Session> {
        Some(Session {
            started: v["started"].as_i64()?,
            splits: v["splits"].as_array()?.iter().filter_map(|s| Some(Split { driver: s["driver"].as_str()?.to_owned(), secs: s["secs"].as_f64()? as f32 })).collect(),
        })
    }
}

fn folder() -> PathBuf {
    super::dir().join("practice")
}

/// The sessions kept, newest first.
pub fn sessions() -> Vec<Session> {
    let mut out: Vec<Session> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| Session::from_json(&v)))
        .collect();
    out.sort_by(|a, b| b.started.cmp(&a.started));
    out
}

pub fn keep(session: &Session) -> std::io::Result<()> {
    let dir = folder();
    std::fs::create_dir_all(&dir)?;
    crate::sys::write_atomic(&dir.join(format!("{}.json", session.started)), session.to_json().to_string().as_bytes())
}

/// The drivers named on this handheld.
pub fn drivers() -> Vec<String> {
    std::fs::read_to_string(folder().join("drivers")).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

fn keep_drivers(names: &[String]) {
    let dir = folder();
    std::fs::create_dir_all(&dir).ok();
    crate::sys::write_atomic(&dir.join("drivers"), names.join("\n").as_bytes()).ok();
}

/// "9.8", seconds to a tenth.
pub fn secs(s: f32) -> String {
    format!("{s:.1}")
}

/// "nov 22 14:05" for when a session began.
fn day(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map(|t| t.with_timezone(&chrono::Local).format("%b %-d %H:%M").to_string().to_lowercase()).unwrap_or_default()
}

// ------------------------------------------------------------------ timer ---

pub struct Practice {
    session: Session,
    /// When the cycle under way began.
    cycle: Option<Instant>,
    drivers: Vec<String>,
    driver: usize,
    naming: Option<Field>,
    list: ListState,
    shown: u32,
}

impl Default for Practice {
    fn default() -> Self {
        Self::new()
    }
}

impl Practice {
    pub fn new() -> Self {
        let mut drivers = drivers();
        if drivers.is_empty() {
            drivers.push("driver".into());
        }
        Practice { session: Session::default(), cycle: None, drivers, driver: 0, naming: None, list: ListState::default(), shown: u32::MAX }
    }

    fn running(&self) -> bool {
        self.cycle.is_some()
    }

    /// For the gallery: cycles already split, and one under way `for_secs`.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn demo(&mut self, splits: Vec<Split>, for_secs: f32) {
        self.drivers = splits.iter().fold(Vec::new(), |mut d, s| {
            if !d.contains(&s.driver) {
                d.push(s.driver.clone());
            }
            d
        });
        self.driver = self.drivers.iter().position(|d| Some(d) == splits.last().map(|s| &s.driver)).unwrap_or(0);
        self.session = Session { started: super::now(), splits };
        self.cycle = Instant::now().checked_sub(Duration::from_secs_f32(for_secs));
    }

    fn now_secs(&self) -> f32 {
        self.cycle.map_or(0.0, |c| c.elapsed().as_secs_f32())
    }

    fn end(&mut self, cx: &mut Cx) {
        self.cycle = None;
        if self.session.splits.is_empty() {
            return;
        }
        let n = self.session.splits.len();
        *cx.say = Some(match keep(&self.session) {
            Ok(()) => format!("kept: {n} {}", if n == 1 { "cycle" } else { "cycles" }),
            Err(_) => "the card would not take it".into(),
        });
        self.session = Session::default();
    }
}

impl Screen for Practice {
    fn title(&self) -> &str {
        "Practice"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.running()
    }

    /// The running cycle's tenths, looked in on ten times a second; the
    /// frame beat rests.
    fn ambient(&self) -> Option<Duration> {
        self.running().then_some(Duration::from_millis(100))
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        let tenths = (self.now_secs() * 10.0) as u32;
        let changed = tenths != self.shown;
        self.shown = tenths;
        changed
    }

    fn set_aside(&mut self) {
        if !self.session.splits.is_empty() {
            keep(&self.session).ok();
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.naming {
            match field.key(key, |c| c.is_alphanumeric() || " -.'".contains(c)) {
                Typed::More => {}
                Typed::Cancelled => self.naming = None,
                Typed::Done(name) => {
                    self.naming = None;
                    if !name.is_empty() {
                        self.drivers.retain(|d| d != "driver");
                        if !self.drivers.contains(&name) {
                            self.drivers.push(name.clone());
                        }
                        self.driver = self.drivers.iter().position(|d| *d == name).unwrap_or(0);
                        keep_drivers(&self.drivers);
                    }
                }
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => {
                self.end(cx);
                return Nav::Pop;
            }
            Key::Char(' ') => {
                let now = Instant::now();
                match self.cycle {
                    None => {
                        if self.session.splits.is_empty() {
                            self.session.started = super::now();
                        }
                        self.cycle = Some(now);
                    }
                    Some(began) => {
                        let s = now.saturating_duration_since(began).as_secs_f32();
                        self.session.splits.push(Split { driver: self.drivers[self.driver].clone(), secs: s });
                        self.cycle = Some(now);
                        let best = self.session.summary(Some(&self.drivers[self.driver])).map(|x| x.best);
                        if best == Some(s) && self.session.splits.len() > 1 {
                            *cx.say = Some(format!("best yet: {} s", secs(s)));
                        }
                    }
                }
                self.list.focus = 0;
            }
            Key::Enter => self.end(cx),
            Key::Backspace => {
                if self.session.splits.pop().is_some() {
                    *cx.say = Some("last cycle taken off".into());
                }
            }
            Key::Left | Key::Right => {
                let n = self.drivers.len() as i32;
                self.driver = (self.driver as i32 + if key == Key::Left { -1 } else { 1 }).rem_euclid(n) as usize;
            }
            Key::Char('n') => self.naming = Some(Field::new("a driver's name", "", 16)),
            Key::Char('h') => return Nav::Push(Box::new(Sessions::new())),
            other => crate::screens::list_key(&mut self.list, other, self.session.splits.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.naming {
            field.draw(g, "enter: add · esc: cancel");
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        let radius = g.tile_radius();
        let card = Rect::new(PAD, top, 148.0, bottom - top);
        g.surface(card, radius, t.surface2);
        let cap_cy = card.y + 9.0 + ty.caption.size * 0.5;
        let n = self.session.splits.len();
        let head = if self.running() { format!("cycle {}", n + 1) } else if n > 0 { "paused".into() } else { "ready".into() };
        g.text_at(card.x + 10.0, cap_cy, Align::Left, &head, ty.caption, if self.running() { t.ink } else { t.dim });
        let driver = &self.drivers[self.driver];
        let room = card.w - 30.0 - g.measure(&head, ty.caption);
        let driver_fit = g.text.fit(driver, &ty.caption, room);
        g.text_at(card.right() - 10.0, cap_cy, Align::Right, &driver_fit, ty.caption, t.ink);
        let big = ty.display(36.0);
        let baseline = cap_cy + ty.caption.size * 0.5 + 6.0 + big.size * 0.74;
        let w = g.text(card.x + 9.0, baseline, &secs(self.now_secs()), big, if self.running() { t.ink } else { t.dim });
        g.text(card.x + 13.0 + w, baseline, "s", ty.label, t.dim);
        let foot_cy = card.bottom() - 9.0 - ty.caption.size * 0.5;
        match self.session.summary(Some(driver)) {
            Some(s) => {
                // The driver's average over the best, a line each.
                let line_h = ty.caption.size + 4.0;
                g.text_at(card.x + 10.0, foot_cy - line_h, Align::Left, &format!("avg {} s", secs(s.mean)), ty.caption, t.dim);
                g.text_at(card.x + 10.0, foot_cy, Align::Left, &format!("best {} s", secs(s.best)), ty.caption, g.good());
            }
            None => {
                g.text_fit(card.x + 10.0, foot_cy, if self.running() { "space: split" } else { "space: start" }, ty.caption, t.dim, card.w - 20.0);
            }
        }

        // The cycles, newest first.
        let x = card.right() + GAP;
        let row_w = W - PAD - x - 5.0;
        let row_h = g.row_h();
        let best = self.session.summary(None).map(|s| s.best);
        let order: Vec<usize> = (0..n).rev().collect();
        if n == 0 {
            let cx = (x + W - PAD) * 0.5;
            g.icon("sports_score", false, cx, card.cy() - 8.0, 22.0, t.faint);
            g.text_at(cx, card.cy() + 14.0, Align::Center, "cycles show here", ty.caption, t.dim);
        }
        for (k, y) in self.list.layout(g, n, top, row_h, 3.0, bottom) {
            let i = order[k];
            let s = &self.session.splits[i];
            let r = Rect::new(x, y, row_w, row_h);
            let name = format!("{}  {}", i + 1, s.driver);
            let ink = if Some(s.secs) == best { g.good() } else { t.ink };
            g.row(r, &name, &format!("{} s", secs(s.secs)), ink, k == self.list.focus);
        }
        self.list.scrollbar(g, n, top, bottom);
        g.hint_any(&["space: split · ←→ driver · n: new driver · enter: end · h: past", "space: split · ←→ driver · enter: end", "space: split · enter: end"]);
    }
}

// --------------------------------------------------------------- sessions ---

pub struct Sessions {
    sessions: Vec<Session>,
    list: ListState,
}

impl Sessions {
    pub fn new() -> Self {
        Sessions { sessions: sessions(), list: ListState::default() }
    }
}

impl Screen for Sessions {
    fn title(&self) -> &str {
        "Sessions"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter => match self.sessions.get(self.list.focus) {
                Some(s) => Nav::Push(Box::new(SessionPage { title: "Session".into(), session: s.clone(), list: ListState::default() })),
                None => Nav::Stay,
            },
            other => {
                crate::screens::list_key(&mut self.list, other, self.sessions.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let bottom = g.above_hint();
        if self.sessions.is_empty() {
            g.empty("timer", "sessions are kept here as they end");
            g.hint("esc: back");
            return;
        }
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, self.sessions.len(), top, row_h, 3.0, bottom) {
            let s = &self.sessions[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let name = day(s.started);
            let value = s.summary(None).map_or_else(String::new, |x| {
                // "7 cycles · avg 10.4 s", or "7 · avg 10.4 s" where the date would not fit beside it.
                let long = format!("{} {} · avg {} s", x.cycles, if x.cycles == 1 { "cycle" } else { "cycles" }, secs(x.mean));
                let bezel = g.theme == crate::look::Theme::Bezel;
                let (ns, vs) = if bezel { (g.ty.label, g.ty.body) } else { (g.ty.body, g.ty.label) };
                if g.measure(&name, ns) + g.measure(&long, vs) + 34.0 <= r.w { long } else { format!("{} · avg {} s", x.cycles, secs(x.mean)) }
            });
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, self.sessions.len(), top, bottom);
        g.hint("enter: each driver's cycles");
    }
}

/// One session: each driver's average and best, then every cycle.
struct SessionPage {
    title: String,
    session: Session,
    list: ListState,
}

impl Screen for SessionPage {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let len = self.session.drivers().len() + self.session.splits.len();
        match key {
            Key::Back => Nav::Pop,
            other => {
                crate::screens::list_key(&mut self.list, other, len);
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let row_h = g.row_h();
        let drivers = self.session.drivers();
        let best = self.session.summary(None).map(|s| s.best);
        let len = drivers.len() + self.session.splits.len();
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, crate::gfx::H) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if let Some(d) = drivers.get(i) {
                let s = self.session.summary(Some(d)).expect("a driver in it drove");
                g.row(r, d, &format!("{} · avg {} · best {}", s.cycles, secs(s.mean), secs(s.best)), t.ink, focused);
            } else {
                let k = i - drivers.len();
                let s = &self.session.splits[k];
                let ink = if Some(s.secs) == best { g.good() } else { t.dim };
                g.row(r, &format!("{}  {}", k + 1, s.driver), &format!("{} s", secs(s.secs)), ink, focused);
            }
        }
        self.list.scrollbar(g, len, top, crate::gfx::H);
    }
}
