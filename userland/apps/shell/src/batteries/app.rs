//! The Batteries app's pages: the fleet as cards, one battery, the pick for
//! a match, and a new battery.

use std::sync::Mutex;
use std::time::Duration;

use bezel::{Align, Color, Rect};

use super::health::{Health, Measure};
use super::{minutes, now, read, short, with, Battery, Sort, State, CHARGE_SECONDS, PRACTICE, REST_SECONDS};
use crate::gfx::{Gfx, ListState, TextSize, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The narrowest a card's trend is drawn.
pub const SPARK_LEAST: f32 = 22.0;

/// How often a page looks in while nothing moves: its minutes count down.
const LOOK_EVERY: Duration = Duration::from_secs(5);

/// The battery just added, for the fleet to turn to.
static ADDED: Mutex<Option<String>> = Mutex::new(None);

/// The app as it opens: on the pick when the island has asked for a match's
/// battery and none is picked yet, else the fleet.
pub fn open(robot: &Robot) -> Box<dyn Screen> {
    match super::waiting(robot.schedule.view().as_ref(), now()) {
        Some(label) => Box::new(Pick::new(&label, robot, true)),
        None => Box::new(Fleet::new()),
    }
}

/// The next match's label, for a pick from the fleet or a battery's page.
fn next_label(robot: &Robot) -> String {
    robot.schedule.view().and_then(|v| v.next(now()).map(|m| m.label.clone())).unwrap_or_else(|| PRACTICE.into())
}

/// A state's colour, for its icon and meter (words stay in the text's ink).
pub fn state_color(g: &Gfx, s: State) -> Color {
    let t = g.tokens;
    match s {
        State::Charging { full: false } => t.signal,
        State::Charging { full: true } | State::Charged => g.good(),
        State::Resting => t.amber_ink,
        State::Used => t.dim,
    }
}

/// A state's fill, for its meter: ice for charged (on), the signal for
/// charging (progress), amber for the rest.
fn state_fill(g: &Gfx, s: State) -> Color {
    let t = g.tokens;
    match s {
        State::Charging { full: false } => t.signal,
        State::Charging { full: true } | State::Charged => t.ice,
        State::Resting => t.amber,
        State::Used => t.dim,
    }
}

/// "full in 28m", "resting 6m", "used · q14": a card's few words.
fn card_words(f: &super::Fleet, b: &Battery, now: i64) -> String {
    match b.state(now) {
        State::Charging { full: false } => format!("full in {}m", (CHARGE_SECONDS - b.charging_for(now).unwrap_or(0) + 59) / 60),
        State::Charging { full: true } => "take it off".into(),
        State::Resting => format!("resting {}", short(b.rest_left(now).unwrap_or(0))),
        State::Charged => "charged".into(),
        State::Used => f.last_use(&b.id).map_or_else(|| "used".into(), |u| format!("used · {}", u.label.to_lowercase())),
    }
}

/// Its health in a few characters: the latest resistance, else the latest
/// sag; "new" before its first match is read.
pub fn health_text(h: &Health) -> String {
    match (h.measure, h.trend.last()) {
        (Measure::Resistance, Some(r)) => format!("{r:.0} mOhm"),
        (Measure::Sag, Some(s)) => format!("−{s:.1} v"),
        _ => "new".into(),
    }
}

/// A trend as a sparkline in `r`: its last twelve matches, the line in the
/// quiet ink, the latest point in `last` (the signal, or the fault's colour
/// when it is flagged). Higher is worse for both measures, and a small
/// range is not stretched to look like a big one.
pub fn spark(g: &mut Gfx, r: Rect, values: &[f64], measure: Measure, last: Color) {
    let shown = &values[values.len().saturating_sub(12)..];
    let Some(&newest) = shown.last() else { return };
    let (mut lo, mut hi) = shown.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let least = if measure == Measure::Resistance { 4.0 } else { 0.4 };
    if hi - lo < least {
        let mid = (hi + lo) * 0.5;
        (lo, hi) = (mid - least * 0.5, mid + least * 0.5);
    }
    let y = |v: f64| r.bottom() - ((v - lo) / (hi - lo)) as f32 * r.h;
    let n = shown.len();
    let x = |i: usize| if n == 1 { r.right() } else { r.x + r.w * i as f32 / (n - 1) as f32 };
    if n >= 2 {
        let pts: Vec<(f32, f32)> = shown.iter().enumerate().map(|(i, v)| (x(i) + g.ox, y(*v))).collect();
        let dim = g.tokens.dim;
        g.canvas.polyline(&pts, 1.5, dim, 1.0);
    }
    // The latest, with a ring of the surface round it so it stands off the line.
    let (cx, cy) = (x(n - 1), y(newest));
    let ring = g.tokens.surface1;
    g.dot(cx, cy, 3.5, ring);
    g.dot(cx, cy, 2.5, last);
}

// ------------------------------------------------------------- the fleet ---

/// What a card shows, read from the fleet once a frame.
struct Card {
    id: String,
    name: String,
    state: State,
    words: String,
    health: Health,
    next: bool,
}

/// The fleet as cards, three across (two at the large text size): each
/// battery's name, its state's icon and words, its health and trend.
/// Enter opens one; `c` puts it on the charger or takes it off; `p` picks
/// the battery for the next match; `n` adds one; `s` sorts; `l` makes the
/// QR labels.
pub struct Fleet {
    focus: usize,
    list: ListState,
    /// The generation and minute last drawn: a change is drawn.
    drawn: (u64, i64),
    /// The labels being made: their outcome, once in.
    labels: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
}

impl Fleet {
    pub fn new() -> Fleet {
        Fleet { focus: 0, list: ListState::default(), drawn: (0, 0), labels: None }
    }

    fn cards(now: i64) -> Vec<Card> {
        read(|f| {
            let next = f.next(now).map(|b| b.id.clone());
            f.sorted(f.sort.unwrap_or(Sort::Health), now)
                .into_iter()
                .map(|i| {
                    let b = &f.batteries[i];
                    Card { id: b.id.clone(), name: b.name.clone(), state: b.state(now), words: card_words(f, b, now), health: f.health(&b.id), next: next.as_deref() == Some(b.id.as_str()) && b.state(now) == State::Charged }
                })
                .collect()
        })
    }

    fn focused_id(&self) -> Option<String> {
        let now = now();
        read(|f| f.sorted(f.sort.unwrap_or(Sort::Health), now).get(self.focus).map(|i| f.batteries[*i].id.clone()))
    }

    fn columns(g: &Gfx) -> usize {
        if g.text_size == TextSize::Large {
            2
        } else {
            3
        }
    }
}

/// A card's height: its name, its health line and its words.
pub fn card_h(g: &Gfx) -> f32 {
    let ty = g.ty;
    5.0 + ty.name.size + 3.0 + ty.label.size + 3.0 + ty.caption.size + 6.0
}

impl Screen for Fleet {
    fn title(&self) -> &str {
        "Batteries"
    }

    fn ambient(&self) -> Option<Duration> {
        Some(LOOK_EVERY)
    }

    /// Drawn again when the fleet changes, or a minute turns (its times
    /// count down); never on the frame beat.
    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        (super::generation(), now() / 60) != self.drawn || self.labels.is_some()
    }

    fn resume(&mut self) {
        self.drawn = (0, 0);
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(outcome) = self.labels.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.labels = None;
            *cx.say = Some(match outcome {
                Ok(_) => "labels saved · in companion, batteries".into(),
                Err(why) => format!("no labels: {why}"),
            });
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let len = read(|f| f.batteries.len());
        let cols = if cx.settings.text == TextSize::Large { 2 } else { 3 };
        match key {
            Key::Back => return Nav::Pop,
            Key::Left => self.focus = self.focus.saturating_sub(1),
            Key::Right => self.focus = (self.focus + 1).min(len.saturating_sub(1)),
            Key::Up => {
                if self.focus >= cols {
                    self.focus -= cols;
                } else {
                    self.list.up();
                }
            }
            Key::Down => {
                if self.focus + cols < len {
                    self.focus += cols;
                } else if self.focus / cols + 1 < len.div_ceil(cols) {
                    self.focus = len - 1;
                } else {
                    self.list.down(len.div_ceil(cols));
                }
            }
            Key::Home => self.focus = 0,
            Key::End => self.focus = len.saturating_sub(1),
            Key::Enter => {
                if let Some(id) = self.focused_id() {
                    return Nav::Push(Box::new(Detail::new(&id)));
                }
                return Nav::Push(Box::new(Add::new()));
            }
            Key::Char('n') => return Nav::Push(Box::new(Add::new())),
            Key::Char('c') => {
                if let Some(id) = self.focused_id() {
                    *cx.say = Some(super::charger(&id));
                }
            }
            Key::Char('p') if len > 0 => return Nav::Push(Box::new(Pick::new(&next_label(cx.robot), cx.robot, false))),
            Key::Char('s') if len > 0 => {
                let sort = with(|f| {
                    let s = f.sort.unwrap_or(Sort::Health).next();
                    f.sort = Some(s);
                    s
                });
                self.focus = 0;
                *cx.say = Some(sort.word().into());
            }
            Key::Char('l') if len > 0 && self.labels.is_none() => {
                let batteries = read(|f| {
                    let mut all = f.batteries.clone();
                    all.sort_by(|a, b| super::natural(&a.name, &b.name));
                    all
                });
                let team = crate::schedule::team_of(&cx.robot.address);
                let (tx, rx) = std::sync::mpsc::channel();
                let made = std::thread::Builder::new().name("battery-labels".into()).spawn(move || {
                    tx.send(super::label::save(&batteries, team).map(|p| p.display().to_string())).ok();
                });
                if made.is_ok() {
                    self.labels = Some(rx);
                    *cx.say = Some("making the labels".into());
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let now = now();
        self.drawn = (super::generation(), now / 60);
        let cards = Fleet::cards(now);
        if let Some(id) = ADDED.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.focus = cards.iter().position(|c| c.id == id).unwrap_or(self.focus);
        }
        if cards.is_empty() {
            g.empty("battery_charging_full", "no batteries yet: add each by its name or number, and its year");
            g.hint("n: new battery");
            return;
        }
        self.focus = self.focus.min(cards.len() - 1);
        let t = g.tokens;
        let ty = g.ty;
        let cols = Fleet::columns(g);
        let rows = cards.len().div_ceil(cols);
        let top = g.top();
        let bottom = g.above_hint();
        let h = card_h(g);
        let fits = ((bottom + crate::gfx::FOOT_AIR - crate::gfx::RING_ROOM - top + GAP) / (h + GAP)).floor().max(1.0) as usize;
        // Room for the scrollbar beside the cards when they scroll.
        let width = W - 2.0 * PAD - if rows > fits { 5.0 } else { 0.0 };
        let w = (width - (cols - 1) as f32 * GAP) / cols as f32;
        let radius = g.radius(11.0);
        self.list.focus = self.focus / cols;
        for (row, y) in self.list.layout(g, rows, top, h, GAP, bottom) {
            for col in 0..cols {
                let Some(card) = cards.get(row * cols + col) else { break };
                let r = Rect::new(PAD + col as f32 * (w + GAP), y, w, h);
                let focused = row * cols + col == self.focus;
                g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
                draw_card(g, r, card);
                if focused {
                    g.focus_ring(r, radius);
                }
            }
        }
        self.list.scrollbar(g, rows, top, bottom);
        let _ = ty;
        g.hint_any(&["enter: open · c: charger · p: pick · s: sort", "enter: open · c: charger · p: pick", "c: charger · p: pick"]);
    }
}

fn draw_card(g: &mut Gfx, r: Rect, c: &Card) {
    let t = g.tokens;
    let ty = g.ty;
    let x = r.x + CARD_IN;
    let right = r.right() - CARD_IN;
    let icon = ty.name.size - 3.0;
    // The name, and the state's icon at the right; for a battery to retire
    // soon, the flag in its place (its words below still say its state).
    let name_cy = r.y + 5.0 + ty.name.size * 0.5;
    let (mark, color) = if c.health.retire.is_some() { ("battery_alert", g.fault()) } else { (c.state.icon(), state_color(g, c.state)) };
    g.icon(mark, true, right - icon * 0.5, name_cy, icon, color);
    g.text_fit(x, name_cy, &c.name, ty.name, t.ink, right - icon - 4.0 - x);
    // Its health, and the trend beside it.
    let health_cy = name_cy + ty.name.size * 0.5 + 3.0 + ty.label.size * 0.5;
    let text = health_text(&c.health);
    let text_w = g.text_at(x, health_cy, Align::Left, &text, ty.label, if c.health.trend.is_empty() { t.dim } else { t.ink });
    // The trend takes what the value leaves, up to 40 px.
    let spark_w = (right - (x + text_w + SPARK_GAP)).min(40.0);
    let line = Rect::new(right - spark_w, health_cy - ty.label.size * 0.42, spark_w, ty.label.size * 0.84);
    if spark_w >= SPARK_LEAST {
        let last = if c.health.retire.is_some() { g.fault() } else { t.signal };
        spark(g, line, &c.health.trend, c.health.measure, last);
    }
    // Its state in words, or for the next one to put in, a mark saying so
    // (it is charged and rested).
    let words_cy = r.bottom() - 6.0 - ty.caption.size * 0.5;
    let room = right - x;
    if c.next {
        let chip = Rect::new(x - 3.0, words_cy - ty.caption.size * 0.5 - 2.0, g.measure("next", ty.caption) + 10.0, ty.caption.size + 4.0);
        g.surface(chip, chip.h * 0.5, t.ice);
        g.text_at(chip.cx(), chip.cy(), Align::Center, "next", ty.caption, t.on_ice);
    } else {
        g.text_fit(x, words_cy, &c.words, ty.caption, t.dim, room);
    }
}

/// A card's margin inside, and the gap between its health and its trend.
pub const CARD_IN: f32 = 8.0;
pub const SPARK_GAP: f32 = 5.0;

// ---------------------------------------------------------- one battery ---

/// One battery: its state (and how long), its health with its trend, and
/// its matches, newest first. `c` the charger, `p` puts it in for the next
/// match, `d` twice deletes it.
pub struct Detail {
    id: String,
    title: String,
    list: ListState,
    deleting: bool,
    drawn: (u64, i64),
}

impl Detail {
    pub fn new(id: &str) -> Detail {
        let title = read(|f| f.get(id).map(Battery::title)).unwrap_or_else(|| "Battery".into());
        Detail { id: id.to_owned(), title, list: ListState::default(), deleting: false, drawn: (0, 0) }
    }
}

/// A line of a battery's history: the flag, or a match.
struct Line {
    name: String,
    value: String,
    icon: Option<(&'static str, Color)>,
    strong: bool,
}

impl Screen for Detail {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Batteries"
    }

    fn ambient(&self) -> Option<Duration> {
        Some(LOOK_EVERY)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        (super::generation(), now() / 60) != self.drawn
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = std::mem::take(&mut self.deleting);
        let len = read(|f| f.uses.iter().filter(|u| u.battery == self.id).count()) + 2;
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('c') => *cx.say = Some(super::charger(&self.id)),
            Key::Char('p') => *cx.say = Some(super::pick(&self.id, &next_label(cx.robot))),
            Key::Char('d') => {
                if armed {
                    let gone = with(|f| f.remove(&self.id));
                    if let Some(r) = gone.as_ref().and_then(|b| b.reminder) {
                        super::forget_reminder(r);
                    }
                    *cx.say = Some(format!("deleted {}", gone.map(|b| b.name).unwrap_or_default()));
                    return Nav::Pop;
                }
                self.deleting = true;
                *cx.say = Some("d again deletes it".into());
            }
            other => crate::screens::list_key(&mut self.list, other, len),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let now = now();
        self.drawn = (super::generation(), now / 60);
        let t = g.tokens;
        let ty = g.ty;
        let fault = g.fault();
        let Some((b, health, lines)) = read(|f| {
            let b = f.get(&self.id)?.clone();
            let health = f.health(&b.id);
            let mut lines = Vec::new();
            if let Some(why) = &health.retire {
                lines.push(Line { name: "    retire soon".into(), value: why.clone(), icon: Some(("battery_alert", fault)), strong: true });
            }
            for u in f.uses.iter().rev().filter(|u| u.battery == b.id) {
                let label = if u.label == PRACTICE { "practice".to_owned() } else { u.label.to_lowercase() };
                // The match; the volts it started at and fell to, and its
                // resistance (or sag). A brownout in the name, marked.
                let (value, icon) = match &u.reading {
                    Some(r) => {
                        let volts = format!("{:.1}–{:.1} v", r.start_v, r.low_v);
                        let value = match r.mohm {
                            _ if r.brownouts > 0 => volts,
                            Some(m) => format!("{volts} · {m:.0} mOhm"),
                            None => format!("{volts} · sag {:.1}", r.sag_v),
                        };
                        (value, (r.brownouts > 0).then_some(("warning", fault)))
                    }
                    None if u.label == PRACTICE => ("no log read".to_owned(), None),
                    None => ("log not in yet".to_owned(), None),
                };
                let name = match &u.reading {
                    Some(r) if r.brownouts > 0 => format!("    {label} · brownout"),
                    _ => format!("    {label}"),
                };
                lines.push(Line { name, value, icon: icon.or(Some(("history", t.faint))), strong: false });
            }
            if f.uses.iter().all(|u| u.battery != b.id) {
                lines.push(Line { name: "    no matches yet".into(), value: "p puts it in".into(), icon: Some(("history", t.faint)), strong: false });
            }
            Some((b, health, lines))
        }) else {
            g.empty("battery_alert", "this battery was deleted");
            return;
        };
        let top = g.top();
        let display = ty.display(22.0);
        let tile_h = 6.0 + ty.caption.size + 4.0 + display.size + 9.0;
        let w = (W - 2.0 * PAD - GAP) / 2.0;
        let radius = g.radius(11.0);
        let label_cy = |r: Rect| r.y + 6.0 + ty.caption.size * 0.5;

        // Its state: the word large, how long in the label, a meter of the
        // charge or the rest.
        let state = b.state(now);
        let r = Rect::new(PAD, top, w, tile_h);
        g.surface(r, radius, t.surface1);
        let (said, fill) = match state {
            State::Charging { full: false } => (format!("{} of {}", minutes(b.charging_for(now).unwrap_or(0)), minutes(CHARGE_SECONDS)), b.charging_for(now).unwrap_or(0) as f32 / CHARGE_SECONDS as f32),
            State::Charging { full: true } => ("take it off".to_owned(), 1.0),
            State::Resting => (format!("{} to rest", minutes(b.rest_left(now).unwrap_or(0))), 1.0 - b.rest_left(now).unwrap_or(0) as f32 / REST_SECONDS as f32),
            State::Charged => ("rested".into(), 1.0),
            State::Used => (read(|f| f.last_use(&b.id).map(|u| format!("in {}", u.label.to_lowercase()))).unwrap_or_else(|| "needs a charge".into()), 0.0),
        };
        let icon = ty.caption.size + 2.0;
        g.icon(state.icon(), true, r.right() - 9.0 - icon * 0.5, label_cy(r), icon, state_color(g, state));
        g.text_fit(r.x + 10.0, label_cy(r), &said, ty.caption, t.dim, r.w - 24.0 - icon);
        let baseline = r.bottom() - 13.0;
        g.text(r.x + 9.0, baseline, state.word(), display, t.ink);
        g.meter(Rect::new(r.x + 10.0, r.bottom() - 8.0, r.w - 20.0, 3.0), fill, state_fill(g, state));

        // Its health: the score in the label, the latest measure large (its
        // resistance, or its sag), the trend beside it.
        let r = Rect::new(PAD + w + GAP, top, w, tile_h);
        g.surface(r, radius, t.surface1);
        let (value, unit) = match (health.measure, health.trend.last()) {
            (Measure::Resistance, Some(v)) => (format!("{v:.0}"), "mOhm"),
            (Measure::Sag, Some(v)) => (format!("{v:.1}"), "v sag"),
            _ => ("new".to_owned(), ""),
        };
        let label = health.score.map_or_else(|| "health".to_owned(), |s| format!("health {s:.0}%"));
        g.text_at(r.x + 10.0, label_cy(r), Align::Left, &label, ty.caption, t.dim);
        if health.retire.is_some() {
            g.icon("battery_alert", true, r.right() - 9.0 - icon * 0.5, label_cy(r), icon, fault);
        }
        let value_w = g.text(r.x + 9.0, baseline, &value, display, if health.trend.is_empty() { t.dim } else { t.ink });
        let unit_w = g.text(r.x + 11.0 + value_w, baseline, unit, ty.caption, t.dim);
        let spark_x = r.x + 18.0 + value_w + unit_w;
        let line = Rect::new(spark_x, baseline - display.size * 0.7, r.right() - 12.0 - spark_x, display.size * 0.7);
        if line.w > 14.0 {
            let last = if health.retire.is_some() { fault } else { t.signal };
            spark(g, line, &health.trend, health.measure, last);
        }

        // Its matches.
        let list_top = top + tile_h + GAP;
        let bottom = g.above_hint();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, lines.len(), list_top, row_h, 3.0, bottom) {
            let line = &lines[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            g.row(r, &line.name, &line.value, if line.strong { t.ink } else { t.dim }, i == self.list.focus);
            if let Some((icon, color)) = line.icon {
                g.icon(icon, line.strong, r.x + 13.0, r.cy(), ty.body.size, color);
            }
        }
        self.list.scrollbar(g, lines.len(), list_top, bottom);
        g.hint_any(&["c: charger · p: put it in · d: delete", "c: charger · p: put it in"]);
    }
}

// -------------------------------------------------------------- the pick ---

/// Which battery goes in for a match: the fleet readiest first, the one
/// suggested in focus. Enter puts it in; one not ready asks for a second
/// Enter.
pub struct Pick {
    label: String,
    title: String,
    /// When to queue (or the match), for the line at the top.
    when: Option<i64>,
    list: ListState,
    /// The order shown, fixed as it opened so nothing moves under the focus.
    order: Vec<String>,
    armed: Option<String>,
    /// Opened by the island's question: done, it closes.
    asked: bool,
}

impl Pick {
    pub fn new(label: &str, robot: &Robot, asked: bool) -> Pick {
        let now = now();
        let order = read(|f| f.ranked(now).into_iter().map(|i| f.batteries[i].id.clone()).collect());
        let when = robot.schedule.view().and_then(|v| v.matches.iter().find(|m| m.label == label).and_then(|m| m.queue_at.or(m.time)));
        let title = if label == PRACTICE { "Pick a battery".to_owned() } else { format!("Battery for {label}") };
        Pick { label: label.to_owned(), title, when, list: ListState::default(), order, armed: None, asked }
    }
}

impl Screen for Pick {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Batteries"
    }

    fn ambient(&self) -> Option<Duration> {
        Some(LOOK_EVERY)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.armed.take();
        match key {
            Key::Back => Nav::Pop,
            Key::Enter => {
                let Some(id) = self.order.get(self.list.focus).cloned() else { return Nav::Stay };
                let now = now();
                let (state, name) = read(|f| f.get(&id).map(|b| (b.state(now), b.name.clone()))).unwrap_or((State::Used, String::new()));
                let ready = matches!(state, State::Charged | State::Charging { full: true });
                if !ready && armed.as_deref() != Some(id.as_str()) {
                    self.armed = Some(id);
                    *cx.say = Some(format!("{name} is {} · enter again", state.word()));
                    return Nav::Stay;
                }
                *cx.say = Some(super::pick(&id, &self.label));
                Nav::Pop
            }
            Key::Char('n') if !self.asked => Nav::Push(Box::new(Add::new())),
            other => {
                crate::screens::list_key(&mut self.list, other, self.order.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let now = now();
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let rows: Vec<(String, State, Health)> = read(|f| self.order.iter().filter_map(|id| f.get(id)).map(|b| (b.name.clone(), b.state(now), f.health(&b.id))).collect());
        if rows.is_empty() {
            g.empty("battery_charging_full", "no batteries yet: add them in the fleet");
            g.hint("esc: back");
            return;
        }
        // What it is for, and when: the time, as the match's live activity
        // says it (its countdown is on the island).
        let hm = |at: i64| chrono::DateTime::from_timestamp(at, 0).map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
        let head = match self.when {
            _ if self.label == PRACTICE => "no match coming: for practice".to_owned(),
            Some(at) => format!("for {} · queue at {}", self.label.to_lowercase(), hm(at)),
            None => format!("for {}", self.label.to_lowercase()),
        };
        let head_h = ty.caption.size + 6.0;
        g.text_fit(PAD + 2.0, top + head_h * 0.5 - 1.0, &head, ty.caption, t.dim, W - 2.0 * PAD);
        let list_top = top + head_h + 2.0;
        let bottom = g.above_hint();
        let row_h = g.row_h();
        let fault = g.fault();
        let suggested = rows.first().filter(|r| r.1 != State::Used).map(|r| r.0.clone());
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom) {
            let (name, state, health) = &rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let next = suggested.as_deref() == Some(name.as_str()) && i == 0;
            // Charged is as a battery should be: its icon says so, and the
            // words are kept for one that is not.
            let name = if *state == State::Charged { format!("    {name}") } else { format!("    {name} · {}", state.word()) };
            // The suggested one says so, with its health where that fits too.
            let mut value = if next { format!("next · {}", health_text(health)) } else { health_text(health) };
            let bezel = g.theme == crate::look::Theme::Bezel;
            let (name_style, value_style) = if bezel { (ty.label, ty.body) } else { (ty.body, ty.label) };
            let shown = if bezel { name.to_lowercase() } else { name.clone() };
            if next && g.measure(&shown, name_style) + g.measure(&value, value_style) + 30.0 > r.w {
                value = "next".into();
            }
            g.row(r, &name, &value, if next { t.ink } else { t.dim }, i == self.list.focus);
            let (mark, color) = if health.retire.is_some() { ("battery_alert", fault) } else { (state.icon(), state_color(g, *state)) };
            g.icon(mark, true, r.x + 13.0, r.cy(), ty.body.size, color);
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom);
        g.hint_any(&["enter: put it in · esc: not now", "enter: put it in"]);
    }
}

// ---------------------------------------------------------- a new battery ---

/// A new battery: its name or number, and its year (this year to start).
/// Tab or ↑↓ moves between them; Enter on the name goes to the year, on
/// the year adds it.
pub struct Add {
    name: String,
    year: String,
    on_year: bool,
}

impl Add {
    pub fn new() -> Add {
        Add { name: String::new(), year: chrono::Local::now().format("%Y").to_string(), on_year: false }
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn typed(name: &str, on_year: bool) -> Add {
        Add { name: name.to_owned(), on_year, ..Add::new() }
    }
}

impl Screen for Add {
    fn title(&self) -> &str {
        "New battery"
    }

    fn app(&self) -> &str {
        "Batteries"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Tab | Key::Up | Key::Down => self.on_year = !self.on_year,
            Key::Backspace => {
                if self.on_year { self.year.pop() } else { self.name.pop() };
            }
            Key::Char(c) if self.on_year => {
                if c.is_ascii_digit() && self.year.len() < 4 {
                    self.year.push(c);
                }
            }
            Key::Char(c) if !c.is_control() && self.name.chars().count() < super::NAME_MOST => self.name.push(c),
            Key::Enter if !self.on_year => self.on_year = true,
            Key::Enter => {
                let year = self.year.parse::<u16>().ok().filter(|y| (1990..=2100).contains(y));
                if !self.year.is_empty() && year.is_none() {
                    *cx.say = Some("a year like 2025".into());
                    return Nav::Stay;
                }
                match with(|f| f.add(&self.name, year, now())) {
                    Ok(id) => {
                        *ADDED.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
                        *cx.say = Some(format!("{} added", self.name.trim()));
                        return Nav::Pop;
                    }
                    Err(why) => {
                        self.on_year = false;
                        *cx.say = Some(why);
                    }
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let field_h = ty.caption.size + ty.name.size + 18.0;
        let radius = g.radius(11.0);
        for (k, (label, value, on)) in [("name or number", &self.name, !self.on_year), ("year", &self.year, self.on_year)].into_iter().enumerate() {
            let r = Rect::new(PAD, top + k as f32 * (field_h + GAP), W - 2.0 * PAD, field_h);
            g.surface(r, radius, if on { t.surface2 } else { t.surface1 });
            g.text_at(r.x + 10.0, r.y + 7.0 + ty.caption.size * 0.5, Align::Left, label, ty.caption, t.dim);
            let cy = r.bottom() - 8.0 - ty.name.size * 0.5;
            let w = g.text_fit(r.x + 10.0, cy, value, ty.name, t.ink, r.w - 24.0);
            if on {
                g.surface(Rect::new(r.x + 11.0 + w, cy - ty.name.size * 0.5, 1.5, ty.name.size), 0.5, t.signal);
                g.focus_ring(r, radius);
            }
        }
        // What its QR label will read, below, where there is room for it.
        let below = top + 2.0 * (field_h + GAP);
        let strip = Rect::new(PAD, below, W - 2.0 * PAD, g.above_hint() - below);
        if strip.h >= ty.caption.size + 10.0 {
            g.surface(strip, radius.min(strip.h * 0.5), t.surface1);
            let icon = ty.caption.size + 3.0;
            g.icon("qr_code_2", false, strip.x + 10.0 + icon * 0.5, strip.cy(), icon, t.dim);
            let id = super::slug(&self.name);
            let words = if id.is_empty() { "l in the fleet makes QR labels".to_owned() } else { format!("on its QR label as {id}") };
            g.text_fit(strip.x + 16.0 + icon, strip.cy(), &words, ty.caption, t.dim, strip.w - 26.0 - icon);
        }
        g.hint_any(if self.on_year { &["enter: add it · tab: name · esc: cancel", "enter: add it · esc: cancel", "enter: add it"] } else { &["enter: next · tab: year · esc: cancel", "enter: next · esc: cancel", "enter: next"] });
    }
}
