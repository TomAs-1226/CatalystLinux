//! The pit: a checklist for before a match, and a match timer.
//!
//! The checklist starts with what a team checks before queueing (battery,
//! bumpers, radio, code, mechanisms...) and is the team's own to change:
//! `n` adds an item, `d` twice deletes one, `r` clears the ticks for the next
//! match. Where the robot can vouch for an item, the row says what it reads
//! (the battery's volts, whether the radio links, the code's version).
//!
//! The match timer counts a match as the field does, phase by phase, for
//! practice and drive-team drills. It runs on whatever screen is showing.
//! When the robot is linked and publishes its own match clock
//! (`/Catalyst/Match/TimeLeft`), that is shown instead.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};

use crate::gfx::{Gfx, ListState, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

// ----------------------------------------------------------------- timer ---

/// One period of a match, as the field clock counts it.
pub struct Phase {
    pub name: &'static str,
    pub seconds: f32,
    /// Part of TELEOP (the field clock then counts down the whole 2:20).
    pub teleop: bool,
}

/// FRC 2026 (REBUILT), from the game manual: AUTO is the first 20 seconds;
/// TELEOP the remaining 2:20, split into a 10 s transition shift, four 25 s
/// alliance shifts, and END GAME, the last 30 seconds.
pub const PHASES: [Phase; 7] = [
    Phase { name: "auto", seconds: 20.0, teleop: false },
    Phase { name: "transition", seconds: 10.0, teleop: true },
    Phase { name: "shift 1", seconds: 25.0, teleop: true },
    Phase { name: "shift 2", seconds: 25.0, teleop: true },
    Phase { name: "shift 3", seconds: 25.0, teleop: true },
    Phase { name: "shift 4", seconds: 25.0, teleop: true },
    Phase { name: "end game", seconds: 30.0, teleop: true },
];

/// The whole match, seconds.
pub const MATCH: f32 = 160.0;
const AUTO: f32 = 20.0;

/// When a phase begins, seconds into the match.
fn start_of(phase: usize) -> f32 {
    PHASES[..phase.min(PHASES.len())].iter().map(|p| p.seconds).sum()
}

/// Where a match stands, `elapsed` seconds in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub elapsed: f32,
    /// The phase, or `PHASES.len()` once the match is over.
    pub phase: usize,
    /// What the field clock shows: AUTO's seconds left, then TELEOP's.
    pub clock: f32,
    /// Seconds left in this phase.
    pub phase_left: f32,
}

impl Reading {
    pub fn at(elapsed: f32) -> Reading {
        let elapsed = elapsed.clamp(0.0, MATCH);
        let phase = (0..PHASES.len()).find(|&p| elapsed < start_of(p + 1)).unwrap_or(PHASES.len());
        // In AUTO the field counts its own seconds; in TELEOP, all of TELEOP's.
        let teleop = PHASES.get(phase).is_none_or(|p| p.teleop);
        let clock = if teleop { MATCH - elapsed } else { AUTO - elapsed };
        let phase_left = if phase < PHASES.len() { start_of(phase + 1) - elapsed } else { 0.0 };
        Reading { elapsed, phase, clock, phase_left }
    }

    pub fn over(&self) -> bool {
        self.phase >= PHASES.len()
    }

    pub fn phase_name(&self) -> &'static str {
        PHASES.get(self.phase).map_or("match over", |p| p.name)
    }
}

/// `m:ss`, as the field shows it.
pub fn clock_text(seconds: f32) -> String {
    let s = seconds.max(0.0).ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The practice clock: time banked from earlier runs, and when the running
/// one started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    banked: f32,
    since: Option<Instant>,
}

impl Clock {
    pub const fn new() -> Clock {
        Clock { banked: 0.0, since: None }
    }

    pub fn elapsed(&self, now: Instant) -> f32 {
        (self.banked + self.since.map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f32())).min(MATCH)
    }

    pub fn running(&self, now: Instant) -> bool {
        self.since.is_some() && self.elapsed(now) < MATCH
    }

    /// Start, pause, or (once the match is over) start again from the top.
    pub fn toggle(&mut self, now: Instant) {
        if self.elapsed(now) >= MATCH {
            *self = Clock { banked: 0.0, since: Some(now) };
        } else if self.since.is_some() {
            self.banked = self.elapsed(now);
            self.since = None;
        } else {
            self.since = Some(now);
        }
    }

    pub fn reset(&mut self) {
        *self = Clock::new();
    }

    /// To the start of a phase, running or not as it was.
    pub fn jump(&mut self, now: Instant, phase: usize) {
        self.banked = start_of(phase.min(PHASES.len() - 1));
        if self.since.is_some() {
            self.since = Some(now);
        }
    }
}

/// The one practice clock, which runs whichever screen is showing.
static CLOCK: Mutex<Clock> = Mutex::new(Clock::new());

/// The last reading of the robot's clock, and whether it was in TELEOP: the
/// robot's clock counts AUTO's 20 seconds and then TELEOP's 2:20, so 0:15
/// could be either, and is told by what came before.
static ROBOT_SEEN: Mutex<Option<(f32, bool)>> = Mutex::new(None);

/// Whether the robot's clock, at `left` seconds, is counting TELEOP: over
/// AUTO's 20 s it must be; under, it is whatever it was counting a moment
/// ago, and with nothing before, what the op mode's name says (else AUTO).
fn in_teleop(left: f32, before: Option<(f32, bool)>, op_mode: Option<&str>) -> bool {
    if left > AUTO + 0.5 {
        return true;
    }
    match before {
        Some((was, teleop)) if left <= was + 1.0 => teleop,
        _ => op_mode.is_some_and(|m| m.to_lowercase().contains("tele")),
    }
}

/// The robot's own match clock as a reading, while it is linked and publishes one.
pub fn robot_reading(robot: &Robot) -> Option<Reading> {
    let left = robot.f64("Match/TimeLeft").filter(|_| robot.linked())? as f32;
    let mut seen = ROBOT_SEEN.lock().unwrap();
    let teleop = in_teleop(left, *seen, robot.string("OpMode/Active").as_deref());
    *seen = Some((left, teleop));
    Some(Reading::at(if teleop { MATCH - left } else { AUTO - left }))
}

/// The match as shown: the robot's clock when it has one, else the practice
/// clock. And whether it is running, and whose it is.
pub fn reading(robot: &Robot) -> (Reading, bool, bool) {
    if let Some(r) = robot_reading(robot) {
        return (r, true, true);
    }
    let now = Instant::now();
    let clock = *CLOCK.lock().unwrap();
    (Reading::at(clock.elapsed(now)), clock.running(now), false)
}

// ------------------------------------------------------------- checklist ---

pub struct Item {
    /// What the robot can vouch for, for a default item; empty for the team's own.
    pub id: String,
    pub name: String,
    pub done: bool,
}

/// What a team checks before queueing.
const DEFAULTS: [(&str, &str); 10] = [
    ("battery", "Battery charged"),
    ("strap", "Battery strapped in"),
    ("bumpers", "Bumpers, right colour"),
    ("radio", "Radio on and linked"),
    ("code", "Code matches the repo"),
    ("zeroed", "Mechanisms zeroed"),
    ("loose", "Bolts and chains tight"),
    ("breaker", "Breaker on, CAN ok"),
    ("ds", "Driver station ready"),
    ("spares", "Spares and tools"),
];

/// The longest name an item may have, typed on the handheld.
const NAME_MOST: usize = 30;

fn file() -> std::path::PathBuf {
    crate::config_dir().join("pit-checklist")
}

pub fn load() -> Vec<Item> {
    match std::fs::read_to_string(file()) {
        Ok(text) => text
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let (done, id, name) = (parts.next()?, parts.next()?, parts.next()?);
                Some(Item { id: id.to_owned(), name: name.to_owned(), done: done == "x" })
            })
            .collect(),
        Err(_) => DEFAULTS.iter().map(|(id, name)| Item { id: (*id).to_owned(), name: (*name).to_owned(), done: false }).collect(),
    }
}

fn save(items: &[Item]) {
    let text: String = items.iter().map(|i| format!("{}\t{}\t{}\n", if i.done { "x" } else { "-" }, i.id, i.name.replace(['\t', '\n'], " "))).collect();
    let file = file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    crate::sys::write_atomic(&file, text.as_bytes()).ok();
}

/// Room before an item's name for its tick: in Bezel's mono voice a space
/// is wider, so fewer of them.
fn indent(g: &Gfx) -> &'static str {
    if g.theme == crate::look::Theme::Bezel { "   " } else { "     " }
}

/// What the robot says about an item, when it can say: the reading, and
/// whether it looks wrong.
fn vouch(id: &str, robot: &Robot) -> Option<(String, bool)> {
    match id {
        "battery" => robot.f64("Systemcore/BatteryVolts").map(|v| (format!("{v:.1} v"), v < 12.3)),
        "radio" if !robot.address.is_empty() => Some(if robot.linked() { ("linked".into(), false) } else { ("no link".into(), true) }),
        "code" => robot.string("Robot/Software/RobotCodeVersion").map(|v| (v.chars().take(7).collect(), false)),
        "breaker" => robot.get("Systemcore/CanUtilization").and_then(|v| v.as_f64s().map(|a| a.iter().copied().fold(0.0, f64::max)).or_else(|| v.as_f64())).map(|can| (format!("can {:.0}%", can * 100.0), can >= 0.9)),
        _ => None,
    }
}

/// What Home's Pit tile says: the match while a clock runs, else the checklist.
pub fn tile(robot: &Robot) -> (String, Option<&'static str>) {
    let (r, running, _) = reading(robot);
    if running && !r.over() {
        return (format!("{} {}", clock_text(r.clock), r.phase_name()), (r.phase == PHASES.len() - 1).then_some("WARN"));
    }
    let items = load();
    let done = items.iter().filter(|i| i.done).count();
    match (done, items.len()) {
        (_, 0) => ("checklist".into(), None),
        (d, n) if d == n => ("all ready".into(), None),
        (d, n) => (format!("{d} of {n} ready"), None),
    }
}

// --------------------------------------------------------------- screens ---

/// A phase's colour: AUTO in ice, the shifts plain, END GAME amber.
fn phase_ink(g: &Gfx, phase: usize) -> Color {
    let t = g.tokens;
    match phase {
        0 => t.ice,
        p if p == PHASES.len() - 1 => t.amber_ink,
        _ => t.ink,
    }
}

/// The match as a bar of its phases, each in proportion, filled to `elapsed`.
fn phase_bar(g: &mut Gfx, r: Rect, elapsed: f32) {
    let t = g.tokens;
    let gap = 2.0;
    let usable = r.w - gap * (PHASES.len() - 1) as f32;
    let mut x = r.x + g.ox;
    for (i, p) in PHASES.iter().enumerate() {
        let w = usable * p.seconds / MATCH;
        let seg = Rect::new(x, r.y, w, r.h);
        g.canvas.rrect(seg, r.h * 0.5, t.meter, 1.0);
        let begun = start_of(i);
        let filled = ((elapsed - begun) / p.seconds).clamp(0.0, 1.0);
        if filled > 0.0 {
            g.canvas.rrect_clipped(Rect::new(seg.x, seg.y, (w * filled).max(r.h * 0.6), seg.h), 1.0, seg, r.h * 0.5, phase_ink(g, i), 1.0);
        }
        x += w + gap;
    }
}

/// The Pit app: the checklist and the match timer side by side; Enter
/// opens the one chosen.
#[derive(Default)]
pub struct Pit {
    timer: bool,
    drawn: Option<Instant>,
}

impl Screen for Pit {
    fn title(&self) -> &str {
        "Pit"
    }

    /// Looked at every beat only while the practice clock runs (the robot's
    /// clock redraws the screen as its values arrive).
    fn busy(&self) -> bool {
        CLOCK.lock().unwrap().running(Instant::now())
    }

    /// While the match clock runs, someone is timing by it: the screen stays on.
    fn keeps_awake(&self) -> bool {
        CLOCK.lock().unwrap().running(Instant::now())
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("pit", &[("enter", "Open"), ("← →", "Checklist or timer")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Left | Key::Up => {
                self.timer = false;
                Nav::Stay
            }
            Key::Right | Key::Down => {
                self.timer = true;
                Nav::Stay
            }
            Key::Enter if self.timer => Nav::Push(Box::new(Timer::new())),
            Key::Enter => Nav::Push(Box::new(Checklist::new())),
            _ => Nav::Stay,
        }
    }

    /// The clock on the card moves: a frame a second's tenth while it runs.
    fn tick(&mut self, _dt: f32) -> bool {
        let running = CLOCK.lock().unwrap().running(Instant::now());
        if running && self.drawn.is_none_or(|d| d.elapsed() >= Duration::from_millis(100)) {
            self.drawn = Some(Instant::now());
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = g.above_hint();
        let w = (W - 2.0 * PAD - GAP) / 2.0;
        let radius = g.tile_radius();
        let cards = [Rect::new(PAD, top, w, bottom - top), Rect::new(PAD + w + GAP, top, w, bottom - top)];
        let label_cy = |r: Rect| r.y + 9.0 + ty.caption.size * 0.5;
        let big = ty.display(30.0);

        // The checklist: how many are ready.
        let items = load();
        let done = items.iter().filter(|i| i.done).count();
        let r = cards[0];
        g.surface(r, radius, if self.timer { t.surface1 } else { t.surface2 });
        g.text_at(r.x + 10.0, label_cy(r), Align::Left, "checklist", ty.caption, t.dim);
        g.icon("checklist", !self.timer, r.right() - 14.0, label_cy(r), 14.0, t.dim);
        let baseline = r.bottom() - 34.0;
        let n_w = g.text(r.x + 9.0, baseline, &done.to_string(), big, t.ink);
        g.text(r.x + 13.0 + n_w, baseline, &format!("of {}", items.len()), ty.caption, t.dim);
        g.meter(Rect::new(r.x + 10.0, r.bottom() - 24.0, r.w - 20.0, 5.0), if items.is_empty() { 0.0 } else { done as f32 / items.len() as f32 }, t.ice);
        let left = items.len() - done;
        let words = if items.is_empty() { "nothing on it".to_owned() } else if left == 0 { "all ready".to_owned() } else { format!("{left} to go") };
        g.text_at(r.x + 10.0, r.bottom() - 10.0, Align::Left, &words, ty.caption, if left == 0 && !items.is_empty() { t.ice } else { t.dim });
        if !self.timer {
            g.focus_ring(r, radius);
        }

        // The match timer: the clock, and its phase.
        let (reading, running, robots) = reading(robot);
        let r = cards[1];
        g.surface(r, radius, if self.timer { t.surface2 } else { t.surface1 });
        g.text_at(r.x + 10.0, label_cy(r), Align::Left, if robots { "robot clock" } else { "match timer" }, ty.caption, if robots { t.ice } else { t.dim });
        g.icon("timer", self.timer, r.right() - 14.0, label_cy(r), 14.0, t.dim);
        let shown = if reading.elapsed == 0.0 && !running { clock_text(AUTO) } else { clock_text(reading.clock) };
        g.text(r.x + 9.0, baseline, &shown, big, if running { phase_ink(g, reading.phase) } else { t.ink });
        phase_bar(g, Rect::new(r.x + 10.0, r.bottom() - 24.0, r.w - 20.0, 5.0), reading.elapsed);
        let state = match (running, reading.elapsed > 0.0, reading.over()) {
            (_, _, true) => "match over",
            (true, _, _) => reading.phase_name(),
            (false, true, _) => "paused",
            (false, false, _) => "ready",
        };
        g.text_at(r.x + 10.0, r.bottom() - 10.0, Align::Left, state, ty.caption, t.dim);
        if self.timer {
            g.focus_ring(r, radius);
        }
        g.hint("enter: open · ←→: choose");
    }
}

/// The checklist: Enter ticks an item; `n` adds one, `d` twice deletes one,
/// `r` clears every tick for the next match.
pub struct Checklist {
    items: Vec<Item>,
    list: ListState,
    naming: Option<String>,
    deleting: Option<usize>,
}

impl Checklist {
    pub fn new() -> Self {
        Checklist { items: load(), list: ListState::default(), naming: None, deleting: None }
    }
}

impl Screen for Checklist {
    fn title(&self) -> &str {
        "Checklist"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    /// Copy: the item being named (else the shell copies the one in focus).
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        self.naming.clone()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.naming.is_some() {
            return vec![keys("a new item", &[("enter", "Add it"), ("esc", "Cancel")])];
        }
        vec![
            keys("checklist", &[("enter", "Tick or untick"), ("n", "New item"), ("d twice", "Delete it"), ("r", "Untick them all")]),
            keys("moving", crate::screens::LIST_KEYS),
        ]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(name) = &mut self.naming {
            match key {
                Key::Char(c) if name.chars().count() < NAME_MOST && !c.is_control() => name.push(c),
                Key::Backspace => {
                    name.pop();
                }
                Key::Back => self.naming = None,
                Key::Enter => {
                    let name = self.naming.take().unwrap_or_default().trim().to_owned();
                    if !name.is_empty() {
                        self.items.push(Item { id: String::new(), name, done: false });
                        self.list.focus = self.items.len() - 1;
                        save(&self.items);
                    }
                }
                _ => {}
            }
            return Nav::Stay;
        }
        let armed = self.deleting.take();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                if let Some(item) = self.items.get_mut(self.list.focus) {
                    item.done = !item.done;
                    save(&self.items);
                    if self.items.iter().all(|i| i.done) {
                        *cx.say = Some("all ready · r clears it".into());
                    }
                }
            }
            Key::Char('n') => self.naming = Some(String::new()),
            Key::Char('d') if !self.items.is_empty() => {
                let at = self.list.focus.min(self.items.len() - 1);
                if armed == Some(at) {
                    let gone = self.items.remove(at);
                    save(&self.items);
                    self.list.focus = at.min(self.items.len().saturating_sub(1));
                    *cx.say = Some(format!("deleted {}", gone.name.to_lowercase()));
                } else {
                    self.deleting = Some(at);
                    *cx.say = Some("d again deletes it".into());
                }
            }
            Key::Char('r') => {
                self.items.iter_mut().for_each(|i| i.done = false);
                save(&self.items);
                *cx.say = Some("cleared for the next match".into());
            }
            other => crate::screens::list_key(&mut self.list, other, self.items.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        if let Some(name) = &self.naming {
            let field = Rect::new(PAD, top, W - 2.0 * PAD, ty.caption.size + ty.label.size + 20.0);
            g.surface(field, 14.0, t.surface2);
            g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, "a new item", ty.caption, t.dim);
            let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
            let w = g.text_at(field.x + 10.0, cy, Align::Left, name, ty.label, t.ink);
            g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
            g.focus_ring(field, 14.0);
            g.hint("enter: add it · esc: cancel");
            return;
        }
        // How far along: a bar and a count over the list.
        let done = self.items.iter().filter(|i| i.done).count();
        let head_h = ty.caption.size + 6.0;
        let count = format!("{done} of {} ready", self.items.len());
        let count_w = g.text_at(W - PAD - 6.0, top + head_h * 0.5 - 1.0, Align::Right, &count, ty.caption, if done == self.items.len() && done > 0 { t.ice } else { t.dim });
        g.meter(Rect::new(PAD + 2.0, top + head_h * 0.5 - 3.5, W - 2.0 * PAD - count_w - 18.0, 5.0), if self.items.is_empty() { 0.0 } else { done as f32 / self.items.len() as f32 }, t.ice);
        let list_top = top + head_h + 3.0;
        let bottom = g.above_hint();
        if self.items.is_empty() {
            g.empty("checklist", "n: add the first item");
        }
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, self.items.len(), list_top, row_h, 3.0, bottom) {
            let item = &self.items[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (value, wrong) = vouch(&item.id, robot).unwrap_or_default();
            g.row(r, &format!("{}{}", indent(g), item.name), &value, if wrong { t.amber_ink } else { t.dim }, i == self.list.focus);
            let ink = if item.done { t.ice } else { t.faint };
            g.icon(if item.done { "check_circle" } else { "radio_button_unchecked" }, item.done, r.x + 15.0, r.cy(), 14.0, ink);
        }
        self.list.scrollbar(g, self.items.len(), list_top, bottom);
        // r (clear every tick) is said once everything is ticked, and in the shortcut list.
        g.hint("enter: tick · n: new · d: delete");
    }
}

/// The match timer, large: the field clock, its phase, what comes next, and
/// the match as a bar of phases. Enter starts and pauses; Left and Right go
/// to a phase before or after; `r` resets.
pub struct Timer {
    drawn: Option<Instant>,
    /// The phase last seen, to say when a new one begins.
    phase: Option<usize>,
}

impl Timer {
    pub fn new() -> Self {
        Timer { drawn: None, phase: None }
    }
}

impl Screen for Timer {
    fn title(&self) -> &str {
        "Match timer"
    }

    fn busy(&self) -> bool {
        CLOCK.lock().unwrap().running(Instant::now())
    }

    /// While the match clock runs, someone is timing by it: the screen stays on.
    fn keeps_awake(&self) -> bool {
        CLOCK.lock().unwrap().running(Instant::now())
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("practice clock", &[("enter · space", "Start or pause"), ("← →", "Phase before or after"), ("r", "Reset")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        // While the robot's clock shows, the practice clock waits.
        if robot_reading(cx.robot).is_some() && matches!(key, Key::Enter | Key::Char(' ') | Key::Char('r') | Key::Left | Key::Right) {
            *cx.say = Some("robot's clock showing".into());
            return Nav::Stay;
        }
        let now = Instant::now();
        let mut clock = CLOCK.lock().unwrap();
        match key {
            Key::Enter | Key::Char(' ') => clock.toggle(now),
            Key::Char('r') => clock.reset(),
            Key::Left | Key::Right => {
                let at = Reading::at(clock.elapsed(now));
                // Left: back to this phase's start, or the one before if just begun.
                let phase = if key == Key::Right {
                    (at.phase + 1).min(PHASES.len() - 1)
                } else if at.elapsed - start_of(at.phase.min(PHASES.len() - 1)) > 2.0 {
                    at.phase.min(PHASES.len() - 1)
                } else {
                    at.phase.saturating_sub(1)
                };
                clock.jump(now, phase);
            }
            _ => {}
        }
        self.phase = None;
        Nav::Stay
    }

    /// Says when a new phase begins.
    fn update(&mut self, cx: &mut Cx) -> Nav {
        let (r, running, _) = reading(cx.robot);
        if running {
            if self.phase.is_some_and(|p| p != r.phase) {
                *cx.say = Some(if r.over() { "match over".into() } else { r.phase_name().into() });
                // And the keyboard's lights blink: a cue felt from the corner of an eye.
                crate::lights::warn();
            }
            self.phase = Some(r.phase);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let running = CLOCK.lock().unwrap().running(Instant::now());
        if running && self.drawn.is_none_or(|d| d.elapsed() >= Duration::from_millis(100)) {
            self.drawn = Some(Instant::now());
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let (r, running, robots) = reading(robot);
        let fresh = r.elapsed == 0.0 && !running;
        // Whose clock, small, above it.
        let whose = if robots { "robot clock" } else if running { "practice" } else if fresh { "practice · ready" } else if r.over() { "practice · over" } else { "practice · paused" };
        g.text_at(W * 0.5, top + ty.caption.size * 0.5, Align::Center, whose, ty.caption, if robots { t.ice } else { t.dim });
        // The clock, as the field shows it; its last ten seconds in amber.
        let big = ty.display(50.0);
        let shown = if fresh { clock_text(AUTO) } else { clock_text(r.clock) };
        let ink = if r.over() { t.dim } else if running && r.phase_left <= 10.0 { t.amber_ink } else { t.ink };
        let w = g.measure(&shown, big);
        g.text(((W - w) * 0.5).round(), top + 64.0, &shown, big, ink);
        // The phase, and what comes next.
        let name = if fresh { "auto" } else { r.phase_name() };
        g.text_at(W * 0.5, top + 84.0, Align::Center, name, ty.name, phase_ink(g, r.phase.min(PHASES.len() - 1)));
        let next = match PHASES.get(r.phase + 1) {
            Some(p) if !r.over() => format!("{} in {}", p.name, clock_text(r.phase_left)),
            _ if r.over() => "enter: again".to_owned(),
            _ => format!("match over in {}", clock_text(r.phase_left)),
        };
        g.text_at(W * 0.5, top + 101.0, Align::Center, &next, ty.caption, t.dim);
        phase_bar(g, Rect::new(PAD + 4.0, top + 113.0, W - 2.0 * PAD - 8.0, 6.0), r.elapsed);
        g.hint(if running { "enter: pause · ←→: phase · r: reset" } else { "enter: start · ←→: phase · r: reset" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_2026_match_is_counted_as_the_field_counts_it() {
        assert_eq!(start_of(PHASES.len()), MATCH);
        let at = |s: f32| Reading::at(s);
        assert_eq!((at(0.0).phase_name(), clock_text(at(0.0).clock)), ("auto", "0:20".to_owned()));
        assert_eq!((at(12.4).phase_name(), clock_text(at(12.4).clock)), ("auto", "0:08".to_owned()));
        // TELEOP's clock starts at 2:20.
        assert_eq!((at(20.0).phase_name(), clock_text(at(20.0).clock)), ("transition", "2:20".to_owned()));
        assert_eq!(at(30.0).phase_name(), "shift 1");
        assert_eq!(at(129.9).phase_name(), "shift 4");
        assert_eq!((at(130.0).phase_name(), clock_text(at(130.0).clock)), ("end game", "0:30".to_owned()));
        assert!(at(160.0).over());
        assert_eq!(at(140.0).phase_left, 20.0);
    }

    #[test]
    fn the_practice_clock_starts_pauses_jumps_and_resets() {
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut c = Clock::new();
        c.toggle(at(0.0));
        assert!((c.elapsed(at(5.0)) - 5.0).abs() < 1e-3);
        c.toggle(at(5.0));
        assert!((c.elapsed(at(60.0)) - 5.0).abs() < 1e-3, "paused, it holds");
        c.jump(at(60.0), PHASES.len() - 1);
        assert_eq!(Reading::at(c.elapsed(at(61.0))).phase_name(), "end game");
        c.toggle(at(61.0));
        assert!(Reading::at(c.elapsed(at(200.0))).over());
        assert!(!c.running(at(200.0)), "over, it stops");
        c.toggle(at(200.0));
        assert!((c.elapsed(at(201.0)) - 1.0).abs() < 1e-3, "and enter starts it again");
        c.reset();
        assert_eq!(c, Clock::new());
    }

    #[test]
    fn the_robots_clock_is_read_by_its_period() {
        let phase = |left: f32, teleop: bool| Reading::at(if teleop { MATCH - left } else { AUTO - left }).phase_name();
        // Over 20 s left: TELEOP, and its phase by the field's table.
        assert!(in_teleop(100.0, None, None));
        assert_eq!(phase(100.0, true), "shift 2");
        // Counting on down into the last 20 s: still TELEOP, in end game.
        assert!(in_teleop(15.0, Some((15.1, true)), None));
        assert_eq!(phase(15.0, true), "end game");
        // A new match, first seen in AUTO: AUTO, as it counts on.
        assert!(!in_teleop(15.0, None, Some("Two-piece auto")));
        assert!(!in_teleop(14.0, Some((15.0, false)), None));
        assert_eq!(phase(14.0, false), "auto");
        // AUTO ends and TELEOP's clock jumps back up to 2:20.
        assert!(in_teleop(140.0, Some((0.0, false)), None));
        // First seen under 20 s in a TELEOP op mode.
        assert!(in_teleop(12.0, None, Some("Teleop")));
    }

    #[test]
    fn every_line_of_the_pit_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let row_w = W - 2.0 * PAD - 5.0;
            for (id, name) in DEFAULTS {
                // Indented past the tick, beside the widest thing the robot
                // says of it: in Catalyst's Sans with a mono value, and in
                // Bezel's lowercase mono with a Sans value (as `Gfx::row` draws them).
                let said = match id {
                    "battery" => "12.8 v",
                    "radio" => "no link",
                    "code" => "4f2c9e1",
                    "breaker" => "can 100%",
                    _ => "",
                };
                g.theme = crate::look::Theme::Catalyst;
                let catalyst = row_w - 30.0 - g.measure(said, ty.label);
                assert!(g.measure(&format!("{}{name}", indent(&g)), ty.body) <= catalyst, "{name}");
                g.theme = crate::look::Theme::Bezel;
                let bezel = row_w - 30.0 - g.measure(said, ty.body);
                assert!(g.measure(&format!("{}{}", indent(&g), name.to_lowercase()), ty.label) <= bezel, "{name} in Bezel's voice");
                // And the tick still clears the name.
                assert!(g.measure(indent(&g), ty.label) >= 15.0);
            }
            let typed = "W".repeat(NAME_MOST);
            assert!(g.measure(&typed, ty.label) <= W - 2.0 * PAD - 22.0, "a typed name fits its field");
            for hint in ["enter: tick · n: new · d: delete", "enter: start · ←→: phase · r: reset", "enter: pause · ←→: phase · r: reset", "enter: open · ←→: choose"] {
                assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{hint}");
            }
            let card = (W - 2.0 * PAD - GAP) / 2.0 - 20.0;
            for word in PHASES.iter().map(|p| p.name).chain(["match over", "paused", "ready", "robot clock", "match timer", "all ready", "nothing on it"]) {
                assert!(g.measure(word, ty.caption) <= card, "{word}");
            }
            // What the island says, within the room it gives a message.
            for said in ["cleared for the next match", "all ready · r clears it", "d again deletes it", "robot's clock showing", "match over"] {
                assert!(g.measure(said, ty.caption) <= 250.0 - 34.0, "{said}");
            }
        }
    }
}
