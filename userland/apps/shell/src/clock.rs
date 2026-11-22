//! Clock: a stopwatch with laps, countdown timers, and the time elsewhere.
//! Three pages, turned with ←→.
//!
//! The stopwatch and the timers live with the shell, not the page: they run
//! on when the app is left, and a timer that runs out is announced in the
//! island with the keyboard's lights blinking, wherever the handheld is (with
//! the screen off, Fn blinks until it wakes). They last until the shell
//! restarts. The world clock's places are kept beside the settings; each
//! place's offset is asked of the zone database (`TZ=… date +%z`), and kept
//! for ten minutes.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

// ---------------------------------------------------------------- shared ---

#[derive(Clone, Debug, Default)]
pub struct Stopwatch {
    started: Option<Instant>,
    banked: f64,
    /// Each lap's total at the moment it was marked.
    laps: Vec<f64>,
}

impl Stopwatch {
    pub fn elapsed(&self) -> f64 {
        self.banked + self.started.map_or(0.0, |s| s.elapsed().as_secs_f64())
    }

    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    fn toggle(&mut self) {
        match self.started.take() {
            Some(s) => self.banked += s.elapsed().as_secs_f64(),
            None => self.started = Some(Instant::now()),
        }
    }

    fn lap(&mut self) {
        if self.running() {
            self.laps.push(self.elapsed());
        }
    }

    fn reset(&mut self) {
        *self = Stopwatch::default();
    }

    /// Each lap as (its own time, the total), newest first.
    pub fn splits(&self) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = self.laps.iter().scan(0.0, |before, total| {
            let lap = total - *before;
            *before = *total;
            Some((lap, *total))
        }).collect();
        out.reverse();
        out
    }
}

#[derive(Clone, Debug)]
pub struct Timer {
    pub id: u32,
    pub total: f64,
    /// Running: when it ends. Paused: None, with `left` kept.
    ends: Option<Instant>,
    left: f64,
    pub done: bool,
    announced: bool,
}

impl Timer {
    pub fn left(&self) -> f64 {
        match self.ends {
            Some(at) => at.saturating_duration_since(Instant::now()).as_secs_f64(),
            None => self.left,
        }
    }

    pub fn running(&self) -> bool {
        self.ends.is_some() && !self.done
    }

    fn pause_or_go(&mut self) {
        if self.done {
            return;
        }
        match self.ends.take() {
            Some(at) => self.left = at.saturating_duration_since(Instant::now()).as_secs_f64(),
            None => self.ends = Some(Instant::now() + Duration::from_secs_f64(self.left)),
        }
    }

    fn restart(&mut self) {
        self.left = self.total;
        self.ends = Some(Instant::now() + Duration::from_secs_f64(self.total));
        self.done = false;
        self.announced = false;
    }
}

struct Shared {
    stopwatch: Stopwatch,
    timers: Vec<Timer>,
    next_id: u32,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { stopwatch: Stopwatch { started: None, banked: 0.0, laps: Vec::new() }, timers: Vec::new(), next_id: 1 });

fn shared<T>(f: impl FnOnce(&mut Shared) -> T) -> T {
    f(&mut SHARED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Start a timer of `seconds`.
pub fn add_timer(seconds: f64) -> u32 {
    shared(|s| {
        let id = s.next_id;
        s.next_id += 1;
        let mut t = Timer { id, total: seconds, ends: None, left: seconds, done: false, announced: false };
        t.restart();
        s.timers.push(t);
        id
    })
}

/// A timer that has just run out, to announce (once each): what the island
/// says. The shell asks every second or so, wherever it is.
pub fn due() -> Option<String> {
    shared(|s| {
        let t = s.timers.iter_mut().find(|t| t.ends.is_some() && !t.announced && t.left() <= 0.0)?;
        t.done = true;
        t.announced = true;
        t.ends = None;
        t.left = 0.0;
        Some(format!("timer {} done", length(t.total)))
    })
}

/// A timer is counting down (deep standby waits for it: it must be said
/// when it runs out). The stopwatch need not be watched: it is a time kept.
#[cfg_attr(test, allow(dead_code))]
pub fn any_running() -> bool {
    shared(|s| s.timers.iter().any(Timer::running))
}

/// What Home's tile says: a timer under way, the stopwatch, or nothing new.
/// Twelve characters at most, all a tile's line holds.
pub fn tile() -> (String, Option<&'static str>) {
    shared(|s| {
        if let Some(t) = s.timers.iter().filter(|t| t.running()).min_by(|a, b| a.left().total_cmp(&b.left())) {
            return (format!("{} left", length(t.left().ceil())), None);
        }
        if s.timers.iter().any(|t| t.done) {
            return ("timer done".into(), Some("WARN"));
        }
        if s.stopwatch.running() || s.stopwatch.elapsed() > 0.0 {
            let seconds = s.stopwatch.elapsed().floor();
            return match seconds < 3600.0 {
                true => (format!("timing {}", length(seconds)), None),
                false => (format!("{} h {} min", (seconds / 3600.0) as u64, (seconds % 3600.0 / 60.0) as u64), None),
            };
        }
        ("timer · laps".into(), None)
    })
}

// ------------------------------------------------------------- reading ---

/// "1:05", "12:00", "1:02:03" for whole seconds.
pub fn length(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A stopwatch's reading, to the hundredth: "1:05.42".
pub fn precise(seconds: f64) -> String {
    let cs = (seconds.max(0.0) * 100.0).floor() as u64;
    let (m, s, c) = (cs / 6000, (cs / 100) % 60, cs % 100);
    if m >= 60 {
        format!("{}:{:02}:{s:02}.{c:02}", m / 60, m % 60)
    } else {
        format!("{m}:{s:02}.{c:02}")
    }
}

/// A length typed for a timer: "5" (minutes), "2:30", "90s", "1h20", "1h 20m 5s".
pub fn parse_length(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase().replace(' ', "");
    if t.is_empty() {
        return None;
    }
    if t.contains(':') {
        let parts: Vec<f64> = t.split(':').map(|p| p.parse::<f64>().ok()).collect::<Option<_>>()?;
        let secs = match parts.as_slice() {
            [m, s] => m * 60.0 + s,
            [h, m, s] => h * 3600.0 + m * 60.0 + s,
            _ => return None,
        };
        return (secs > 0.0).then_some(secs);
    }
    if let Ok(minutes) = t.parse::<f64>() {
        return (minutes > 0.0).then_some(minutes * 60.0);
    }
    // Numbers each followed by h, m or s; a last bare number after h is minutes.
    let mut total = 0.0;
    let mut number = String::new();
    let mut last_unit = ' ';
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let n: f64 = number.parse().ok()?;
        number.clear();
        total += n * match c {
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        last_unit = c;
    }
    if !number.is_empty() {
        let n: f64 = number.parse().ok()?;
        total += n * if last_unit == 'h' { 60.0 } else if last_unit == 'm' { 1.0 } else { 60.0 };
    }
    (total > 0.0).then_some(total)
}

// ----------------------------------------------------------------- world ---

const DEFAULT_PLACES: [&str; 6] = ["UTC", "America/Los_Angeles", "America/Chicago", "America/New_York", "Europe/London", "Asia/Taipei"];

fn places_path() -> std::path::PathBuf {
    crate::config_dir().join("world-clock")
}

pub fn load_places() -> Vec<String> {
    match std::fs::read_to_string(places_path()) {
        Ok(text) => text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect(),
        Err(_) => DEFAULT_PLACES.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn save_places(places: &[String]) {
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&places_path(), (places.join("\n") + "\n").as_bytes()).ok();
}

/// A zone's name as people say it: its city, without underscores.
pub fn place_name(zone: &str) -> String {
    if zone == "UTC" {
        return "UTC".into();
    }
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

/// "+0800" as seconds east of UTC.
pub fn offset_of(text: &str) -> Option<i64> {
    let t = text.trim();
    let (sign, rest) = match t.chars().next()? {
        '+' => (1, &t[1..]),
        '-' => (-1, &t[1..]),
        _ => return None,
    };
    if rest.len() != 4 {
        return None;
    }
    let (h, m): (i64, i64) = (rest[..2].parse().ok()?, rest[2..].parse().ok()?);
    Some(sign * (h * 3600 + m * 60))
}

/// Where the zone database cannot be asked (a computer), the usual offsets.
fn known_offset(zone: &str) -> Option<i64> {
    Some(match zone {
        "UTC" => 0,
        "America/Los_Angeles" => -8 * 3600,
        "America/Denver" => -7 * 3600,
        "America/Chicago" => -6 * 3600,
        "America/New_York" | "America/Toronto" => -5 * 3600,
        "Europe/London" => 0,
        "Europe/Berlin" | "Europe/Paris" => 3600,
        "Asia/Taipei" | "Asia/Shanghai" => 8 * 3600,
        "Asia/Tokyo" => 9 * 3600,
        "Australia/Sydney" => 11 * 3600,
        "America/Mexico_City" => -6 * 3600,
        "America/Sao_Paulo" => -3 * 3600,
        "Asia/Kolkata" => 19_800,
        "Asia/Jerusalem" => 7200,
        _ => return None,
    })
}

/// A zone's offset from UTC now, in seconds.
fn ask_offset(zone: &str) -> Option<i64> {
    if !crate::sys::on_handheld() {
        return known_offset(zone);
    }
    let out = std::process::Command::new("date").arg("+%z").env("TZ", zone).output().ok()?;
    offset_of(&String::from_utf8_lossy(&out.stdout))
}

/// A zone typed as part of its name ("berlin", "new york") found among those
/// the handheld knows.
pub fn find_zone(typed: &str, known: &[String]) -> Option<String> {
    let want = typed.trim().to_lowercase().replace(' ', "_");
    if want.is_empty() {
        return None;
    }
    known.iter().find(|z| z.to_lowercase() == want).or_else(|| known.iter().find(|z| z.to_lowercase().rsplit('/').next() == Some(want.as_str()))).or_else(|| known.iter().find(|z| z.to_lowercase().contains(&want))).cloned()
}

fn known_zones() -> Vec<String> {
    let mut zones = crate::device::known_zones();
    if zones.is_empty() {
        zones = ["UTC", "America/Los_Angeles", "America/Denver", "America/Chicago", "America/New_York", "America/Toronto", "America/Mexico_City", "America/Sao_Paulo", "Europe/London", "Europe/Berlin", "Europe/Paris", "Asia/Jerusalem", "Asia/Kolkata", "Asia/Shanghai", "Asia/Taipei", "Asia/Tokyo", "Australia/Sydney"].iter().map(|s| (*s).to_owned()).collect();
    }
    if !zones.iter().any(|z| z == "UTC") {
        zones.insert(0, "UTC".into());
    }
    zones
}

// ---------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Stopwatch,
    Timers,
    World,
}

const PAGES: [Page; 3] = [Page::Stopwatch, Page::Timers, Page::World];

pub struct Clock {
    page: usize,
    turn: Animated,
    timers: ListState,
    world: ListState,
    places: Vec<String>,
    offsets: Vec<Option<i64>>,
    read_at: Option<Instant>,
    typing: Option<(Field, bool)>,
    deleting: Option<usize>,
    /// The second last drawn, so the page is drawn again as it changes.
    drawn: u64,
}

impl Clock {
    pub fn new() -> Self {
        let places = load_places();
        let mut clock = Clock {
            page: 0,
            turn: Animated::new(0.0, role::page()).with_rest(0.002),
            timers: ListState::default(),
            world: ListState::default(),
            offsets: Vec::new(),
            places,
            read_at: None,
            typing: None,
            deleting: None,
            drawn: 0,
        };
        // A timer under way: open on the timers.
        if shared(|s| s.timers.iter().any(|t| t.running() || t.done)) {
            clock.page = 1;
            clock.turn.jump(1.0);
        }
        clock.read_offsets();
        clock
    }

    fn read_offsets(&mut self) {
        self.offsets = self.places.iter().map(|z| ask_offset(z)).collect();
        self.read_at = Some(Instant::now());
    }

    fn go(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
        self.turn.set(self.page as f32);
        self.deleting = None;
    }
}

impl Screen for Clock {
    /// Each page names itself (no pager dots: they would sit on the keys).
    fn title(&self) -> &str {
        match PAGES[self.page] {
            Page::Stopwatch => "Stopwatch",
            Page::Timers => "Timers",
            Page::World => "World",
        }
    }

    fn wants_text(&self) -> bool {
        self.typing.is_some()
    }

    fn keeps_awake(&self) -> bool {
        shared(|s| s.stopwatch.running())
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, is_timer)) = &mut self.typing {
            let is_timer = *is_timer;
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) => {
                    self.typing = None;
                    if is_timer {
                        match parse_length(&text) {
                            Some(secs) if secs <= 24.0 * 3600.0 => {
                                add_timer(secs);
                                *cx.say = Some(format!("timer {} started", length(secs)));
                            }
                            _ => *cx.say = Some("a length like 5, 2:30, 90s or 1h20".into()),
                        }
                    } else {
                        match find_zone(&text, &known_zones()) {
                            Some(zone) if !self.places.contains(&zone) => {
                                self.places.push(zone.clone());
                                save_places(&self.places);
                                self.read_offsets();
                                self.world.focus = self.places.len();
                                *cx.say = Some(format!("added {}", place_name(&zone).to_lowercase()));
                            }
                            Some(_) => *cx.say = Some("already here".into()),
                            None => *cx.say = Some(format!("no place called {}", text.trim())),
                        }
                    }
                }
                Typed::Cancelled => self.typing = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let armed = self.deleting.take();
        match (PAGES[self.page], key) {
            (_, Key::Back) => return Nav::Pop,
            (_, Key::Left) if self.page > 0 => self.go(self.page - 1),
            (_, Key::Right) if self.page + 1 < PAGES.len() => self.go(self.page + 1),
            (Page::Stopwatch, Key::Enter | Key::Char(' ')) => shared(|s| s.stopwatch.toggle()),
            (Page::Stopwatch, Key::Char('l') | Key::Down) => shared(|s| s.stopwatch.lap()),
            (Page::Stopwatch, Key::Char('r')) => {
                if shared(|s| s.stopwatch.running()) {
                    *cx.say = Some("stop it first · enter".into());
                } else {
                    shared(|s| s.stopwatch.reset());
                }
            }
            (Page::Timers, key) => {
                let timers = shared(|s| s.timers.clone());
                let at = self.timers.focus;
                match key {
                    Key::Enter if at == timers.len() => self.typing = Some((Field::new("how long · 5, 2:30, 90s, 1h20", "", 12), true)),
                    Key::Char('n') => self.typing = Some((Field::new("how long · 5, 2:30, 90s, 1h20", "", 12), true)),
                    Key::Enter | Key::Char(' ') => {
                        if let Some(t) = timers.get(at) {
                            let id = t.id;
                            shared(|s| {
                                if let Some(t) = s.timers.iter_mut().find(|t| t.id == id) {
                                    if t.done {
                                        t.restart();
                                    } else {
                                        t.pause_or_go();
                                    }
                                }
                            });
                        }
                    }
                    Key::Char('r') => {
                        if let Some(t) = timers.get(at) {
                            let id = t.id;
                            shared(|s| s.timers.iter_mut().filter(|t| t.id == id).for_each(Timer::restart));
                        }
                    }
                    Key::Char('d') if at < timers.len() => {
                        let id = timers[at].id;
                        shared(|s| s.timers.retain(|t| t.id != id));
                    }
                    other => crate::screens::list_key(&mut self.timers, other, timers.len() + 1),
                }
            }
            (Page::World, key) => {
                let at = self.world.focus;
                match key {
                    Key::Char('a') => self.typing = Some((Field::new("a city or zone · berlin, tokyo", "", 30), false)),
                    Key::Enter if at == self.places.len() + 1 => self.typing = Some((Field::new("a city or zone · berlin, tokyo", "", 30), false)),
                    Key::Char('d') if at >= 1 && at <= self.places.len() => {
                        if armed == Some(at) {
                            let gone = self.places.remove(at - 1);
                            save_places(&self.places);
                            self.read_offsets();
                            *cx.say = Some(format!("removed {}", place_name(&gone).to_lowercase()));
                        } else {
                            *cx.say = Some("d again removes it".into());
                            self.deleting = Some(at);
                        }
                    }
                    other => crate::screens::list_key(&mut self.world, other, self.places.len() + 2),
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.turn.tick(dt);
        if self.read_at.is_none_or(|at| at.elapsed() > Duration::from_secs(600)) {
            self.read_offsets();
        }
        // Moving numbers: a running stopwatch every frame (its hundredths),
        // timers and clocks when the second turns.
        let running = shared(|s| s.stopwatch.running());
        let second = chrono::Utc::now().timestamp() as u64;
        let changed = second != self.drawn;
        self.drawn = second;
        moving || changed || (running && self.page == 0)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        if let Some((field, is_timer)) = &self.typing {
            return field.draw(g, if *is_timer { "enter: start it · esc: cancel" } else { "enter: add it · esc: cancel" });
        }
        if g.calm {
            self.turn.jump(self.page as f32);
        }
        let base = g.ox;
        let at = self.turn.value();
        for (i, page) in PAGES.iter().enumerate() {
            let ox = (i as f32 - at) * W;
            if ox.abs() >= W {
                continue;
            }
            g.ox = base + ox.round();
            match page {
                Page::Stopwatch => self.draw_stopwatch(g),
                Page::Timers => self.draw_timers(g),
                Page::World => self.draw_world(g),
            }
        }
        g.ox = base;
        g.hint_any(match PAGES[self.page] {
            Page::Stopwatch => &["enter: go/stop · l: lap · r: reset · →", "l: lap · r: reset · →"],
            Page::Timers => &["enter: pause · n: new · d: remove · ←→", "n: new · d: remove · ←→"],
            Page::World => &["a: add a place · d: remove · ←", "a: add · d: remove · ←"],
        });
    }
}

impl Clock {
    fn draw_stopwatch(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let sw = shared(|s| s.stopwatch.clone());
        let big = ty.display(34.0);
        let cy = top + 6.0 + big.size * 0.5;
        let color = if sw.running() { t.ink } else if sw.elapsed() > 0.0 { t.dim } else { t.faint };
        g.text_at(W * 0.5, cy, Align::Center, &precise(sw.elapsed()), big, color);
        let state = if sw.running() { "running" } else if sw.elapsed() > 0.0 { "stopped" } else { "ready" };
        g.text_at(W * 0.5, cy + big.size * 0.5 + 4.0 + ty.caption.size * 0.5, Align::Center, state, ty.caption, t.dim);
        let splits = sw.splits();
        let line = ty.caption.size + 5.0;
        let mut y = cy + big.size * 0.5 + 10.0 + ty.caption.size + line * 0.5;
        let bottom = g.above_hint();
        // The fastest lap in ice, the slowest in amber: only once they differ
        // (to the hundredth shown), so even laps are marked as neither.
        let laps: Vec<f64> = splits.iter().map(|s| (s.0 * 100.0).floor()).collect();
        let (fast, slow) = (laps.iter().copied().fold(f64::MAX, f64::min), laps.iter().copied().fold(f64::MIN, f64::max));
        let (fast, slow) = if splits.len() >= 2 && fast < slow { (fast, slow) } else { (-1.0, -1.0) };
        for (k, (lap, total)) in splits.iter().enumerate() {
            if y + line * 0.5 > bottom {
                break;
            }
            let n = splits.len() - k;
            let shown = (lap * 100.0).floor();
            let ink = if shown == fast { g.good() } else if shown == slow { t.amber_ink } else { t.dim };
            g.text_at(PAD + 8.0, y, Align::Left, &format!("lap {n}"), ty.caption, t.dim);
            g.text_at(W * 0.5 + 20.0, y, Align::Right, &precise(*lap), ty.caption, ink);
            g.text_at(W - PAD - 8.0, y, Align::Right, &precise(*total), ty.caption, t.faint);
            y += line;
        }
    }

    fn draw_timers(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let timers = shared(|s| s.timers.clone());
        let len = timers.len() + 1;
        for (i, y) in self.timers.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match timers.get(i) {
                Some(timer) => {
                    let left = timer.left();
                    let (value, color) = if timer.done {
                        ("done".to_owned(), t.signal)
                    } else if timer.running() {
                        (length(left.ceil()), t.ink)
                    } else {
                        (format!("paused {}", length(left.ceil())), t.dim)
                    };
                    g.row(r, &format!("{} timer", length(timer.total)), &value, color, i == self.timers.focus);
                    // How far it has gone, along the row's foot.
                    let done = if timer.total > 0.0 { (1.0 - left / timer.total).clamp(0.0, 1.0) as f32 } else { 1.0 };
                    let bar = Rect::new(r.x + 12.0, r.bottom() - 4.0, r.w - 24.0, 2.0);
                    g.surface(bar, 1.0, t.meter);
                    if done > 0.01 {
                        g.surface(Rect::new(bar.x, bar.y, (bar.w * done).max(2.0), bar.h), 1.0, if timer.done { t.signal } else { t.ice });
                    }
                }
                None => g.row(r, "New timer", "n", t.dim, i == self.timers.focus),
            }
        }
        self.timers.scrollbar(g, len, top, bottom);
    }

    fn draw_world(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let now = chrono::Utc::now();
        let local = chrono::Local::now();
        let here = local.offset().local_minus_utc() as i64;
        let h24 = g.clock.0;
        let fmt = |offset: i64| {
            let at = now.naive_utc() + chrono::Duration::seconds(offset);
            let day = (at.date() - local.date_naive()).num_days();
            let time = if h24 { at.format("%H:%M").to_string() } else { at.format("%-I:%M %P").to_string() };
            match day {
                0 => time,
                1 => format!("{time} tmrw"),
                -1 => format!("{time} yday"),
                _ => time,
            }
        };
        let len = self.places.len() + 2;
        for (i, y) in self.world.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            if i == 0 {
                let name = if crate::sys::on_handheld() { format!("Here · {}", place_name(&crate::device::zone())) } else { "Here · this computer".to_owned() };
                g.row(r, &name, &fmt(here), t.ink, i == self.world.focus);
                continue;
            }
            if i == len - 1 {
                g.row(r, "Add a place", "a", t.dim, i == self.world.focus);
                continue;
            }
            let zone = &self.places[i - 1];
            let value = match self.offsets.get(i - 1).copied().flatten() {
                Some(offset) => {
                    let diff = (offset - here) as f64 / 3600.0;
                    let rel = if diff == 0.0 { "same".to_owned() } else if diff.fract() == 0.0 { format!("{diff:+}h") } else { format!("{diff:+.1}h") };
                    format!("{} · {rel}", fmt(offset))
                }
                None => "–".into(),
            };
            g.row(r, &place_name(zone), &value, t.dim, i == self.world.focus);
        }
        self.world.scrollbar(g, len, top, bottom);
    }
}

/// A shell ticking on another thread announces a timer that runs out too
/// (`Shell::tick`): a test that keeps shells ticking for long holds this,
/// and so does the test that waits for its own timer's announcement.
#[cfg(test)]
pub(crate) static TIMERS_ANNOUNCED: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_and_type_naturally() {
        assert_eq!(length(65.0), "1:05");
        assert_eq!(length(3723.0), "1:02:03");
        assert_eq!(precise(65.427), "1:05.42");
        assert_eq!(precise(3600.5), "1:00:00.50");
        assert_eq!(parse_length("5"), Some(300.0));
        assert_eq!(parse_length("2:30"), Some(150.0));
        assert_eq!(parse_length("1:00:00"), Some(3600.0));
        assert_eq!(parse_length("90s"), Some(90.0));
        assert_eq!(parse_length("1h20"), Some(4800.0));
        assert_eq!(parse_length("1h 20m 5s"), Some(4805.0));
        assert_eq!(parse_length("2m30"), Some(150.0), "a bare number after m is seconds");
        assert_eq!(parse_length("0"), None);
        assert_eq!(parse_length("soon"), None);
    }

    #[test]
    fn the_stopwatch_keeps_laps_newest_first() {
        let mut sw = Stopwatch { started: None, banked: 10.0, laps: vec![3.0, 7.5, 10.0] };
        assert_eq!(sw.elapsed(), 10.0);
        assert_eq!(sw.splits(), [(2.5, 10.0), (4.5, 7.5), (3.0, 3.0)]);
        sw.lap();
        assert_eq!(sw.laps.len(), 3, "no lap while stopped");
        sw.reset();
        assert_eq!(sw.elapsed(), 0.0);
    }

    #[test]
    fn a_running_timer_is_seen_by_deep_standby() {
        let id = add_timer(3600.0);
        assert!(any_running());
        shared(|s| s.timers.retain(|t| t.id != id));
    }

    #[test]
    fn a_timer_that_runs_out_is_announced_once() {
        let _alone = TIMERS_ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
        let id = add_timer(0.05);
        std::thread::sleep(Duration::from_millis(80));
        let mut said = None;
        for _ in 0..20 {
            if let Some(s) = due() {
                said = Some(s);
                break;
            }
        }
        assert_eq!(said.as_deref(), Some("timer 0:00 done"));
        assert!(shared(|s| s.timers.iter().find(|t| t.id == id).is_some_and(|t| t.done)));
        assert!(shared(|s| !s.timers.iter().any(|t| t.id == id && !t.announced)));
        shared(|s| s.timers.retain(|t| t.id != id));
    }

    #[test]
    fn places_and_offsets_read_from_the_zone_database() {
        assert_eq!(offset_of("+0800"), Some(28_800));
        assert_eq!(offset_of("-0530"), Some(-19_800));
        assert_eq!(offset_of("0800"), None);
        assert_eq!(place_name("America/Los_Angeles"), "Los Angeles");
        let known: Vec<String> = ["UTC", "Europe/Berlin", "America/New_York", "America/North_Dakota/New_Salem"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(find_zone("berlin", &known).as_deref(), Some("Europe/Berlin"));
        assert_eq!(find_zone("New York", &known).as_deref(), Some("America/New_York"));
        assert_eq!(find_zone("utc", &known).as_deref(), Some("UTC"));
        assert_eq!(find_zone("atlantis", &known), None);
    }
}
