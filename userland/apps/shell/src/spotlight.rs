//! Spotlight: search from anywhere, as on a Mac. Ctrl-Enter brings it up
//! over any screen, an app, a text field or the terminal included (no
//! program in a terminal can tell Ctrl-Enter from Enter, so it takes nothing
//! from one); Space or / on Home bring it up too.
//!
//! It is a bar of regular glass that grows out of the island at the top:
//! the island stretches into it on Bezel's arriving spring while its glass
//! materializes, ramping its refraction (never its opacity), and the screen
//! behind is held still and dimmed a little. Results come as each key
//! lands, ranked: apps, Settings' pages and switches, Actions, notes, files,
//! photos, repositories, SSH hosts, robot values, match logs, the event
//! schedule, sums and unit conversions, and last the web and the assistant.
//! Up and Down move through them; Enter opens one, growing out of its row;
//! Tab or → shows a preview where there is one; Esc springs it back into the
//! island. What is picked is remembered, and found sooner.
//!
//! The index is built and kept up to date on a thread of its own
//! (`spotlight/index.rs`); what it can find comes from providers
//! (`spotlight/providers.rs`), and any app can add its own with [`register`].
//! It never opens over a locked screen ([`set_locked`], `Screen::locks`).

mod index;
mod providers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, OnceLock};

use bezel::{Align, Color, GlassShape, Rect, TextStyle};
use detent::{role, Animated};

// What another app needs to add its own results (`register`), and to rank
// them as these are.
#[allow(unused_imports)]
pub use index::{weight, Hit, Index, Item, Preview, Provider, Query, Stats, TAIL};

use crate::gfx::{Gfx, H, W};
use crate::platform::Key;
use crate::robot::Robot;

static INDEX: OnceLock<Arc<Index>> = OnceLock::new();

/// The index Spotlight searches. The first time it is asked for, its thread
/// starts and reads everything once.
pub fn index() -> &'static Arc<Index> {
    INDEX.get_or_init(|| {
        let index = Arc::new(Index::new(providers::builtin()));
        let worker = index.clone();
        std::thread::Builder::new().name("spotlight".into()).stack_size(256 * 1024).spawn(move || worker.run()).ok();
        index.poke();
        index
    })
}

/// Add a provider: what it finds is offered from then on, ranked with the
/// rest. For other apps (reminders, music, the apps platform), at start.
#[allow(dead_code)]
pub fn register(provider: Arc<dyn Provider>) {
    index().register(provider);
}

/// Read the index in the background ahead of the first search, so the first
/// one is as quick as any.
pub fn warm() {
    index();
}

static LOCK_HELD: AtomicBool = AtomicBool::new(false);

/// The handheld is locked (or unlocked again): Spotlight does not open while
/// it is, and one showing goes at once. For a lock drawn over the screens
/// rather than as a screen of its own (one that is says so with
/// `Screen::locks`).
#[allow(dead_code)]
pub fn set_locked(locked: bool) {
    LOCK_HELD.store(locked, Ordering::Relaxed);
}

/// Whether a lock says nothing may open over it.
pub fn lock_held() -> bool {
    LOCK_HELD.load(Ordering::Relaxed)
}

// ----------------------------------------------------------------- layout ---

/// The bar's place: across the panel, its top on the island's.
const SIDE: f32 = 6.0;
const TOP: f32 = 4.0;
/// Its corners, and its rows' (concentric: the corner less the row's inset).
const RADIUS: f32 = 16.0;
const ROW_INSET: f32 = 5.0;
const ROW_GAP: f32 = 2.0;
/// Below the bar's own line, before the rows and after them.
const LIST_ABOVE: f32 = 4.0;
const LIST_BELOW: f32 = 5.0;
/// Its foot stays this far above the panel's.
const FOOT: f32 = 4.0;
/// How much the screen behind dims as the bar comes (the control centre's
/// sheet, which covers far more, dims it 45%). My own tuning.
const DIM: f32 = 0.28;
/// How long after the bar starts back into the island its glass begins to
/// dematerialize (Bezel's island: 240 ms).
const GLASS_LINGERS: f32 = 0.24;
/// The most results found at once.
const LIMIT: usize = 40;
/// Picks offered with nothing typed.
const RECENT: usize = 8;
/// What is typed, at most.
const QUERY_MOST: usize = 60;

/// The bar's measures at the text size in use: its line's height, a row's,
/// and how many rows it holds at most.
fn metrics(g: &Gfx) -> (f32, f32, usize) {
    let (bar, row, rows) = room_in_rows(g);
    (bar, row, rows.floor().max(1.0) as usize)
}

/// The rows the bar holds reaching down to its foot, the last perhaps in
/// part: with more results than whole rows, it runs on there and the next
/// one fades out under its foot, rather than stopping a part-row short.
fn room_in_rows(g: &Gfx) -> (f32, f32, f32) {
    let bar = (g.ty.name.size + 15.0).round();
    let row = g.row_h();
    let room = H - TOP - FOOT - bar - LIST_ABOVE - LIST_BELOW;
    (bar, row, (room + ROW_GAP) / (row + ROW_GAP))
}

/// The bar at rest, `rows` rows of results tall.
fn rest_rect(g: &Gfx, rows: f32) -> Rect {
    let (bar, row, _) = metrics(g);
    let list = if rows > 0.0 { LIST_ABOVE + rows * row + (rows - 1.0).max(0.0) * ROW_GAP + LIST_BELOW } else { 0.0 };
    Rect::new(SIDE, TOP, W - 2.0 * SIDE, bar + list)
}

/// What the island grows from as it comes, and shrinks back into.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// What a key did.
pub enum Outcome {
    Stay,
    /// Put it away: it springs back into the island.
    Close,
    /// Open a result, out of where it is drawn.
    Open(Hit, Rect),
}

// -------------------------------------------------------------- spotlight ---

pub struct Spotlight {
    open: bool,
    /// 0 the island alone, 1 the bar grown to its whole shape.
    grow: Animated,
    /// Its glass: 0 → 1 as it materializes (refraction, frost and rim).
    glass: Animated,
    /// Seconds since it was put away, for its glass to linger as it shrinks.
    since_closed: f32,
    /// How many rows tall the bar is, gliding as results come and go.
    rows: Animated,
    /// The most rows it holds at the text size last drawn.
    rows_most: usize,
    query: String,
    hits: Vec<Hit>,
    /// The results are the picks offered with nothing typed.
    recent: bool,
    focus: usize,
    /// The first row in sight, and the highlight's row, both gliding.
    scroll: Animated,
    ring: Animated,
    /// The highlight, and the list, land at once next time they are drawn.
    placed: bool,
    /// The preview shown, for which result; one being read on a thread (the
    /// one shown stays until it comes); and how far the preview has slid
    /// in over the results (0 → 1).
    preview: Option<(Hit, Preview)>,
    reading: Option<(Hit, Receiver<Option<Preview>>)>,
    slide: Animated,
    /// The index's generation the results were found at.
    seen: u64,
    /// Results found before, by what was typed: Backspace is instant.
    cache: Vec<(String, Vec<Hit>)>,
    /// Where each row in sight was last drawn, for a result to open out of.
    drawn: Vec<(usize, Rect)>,
    /// Where the preview, and the line typed into, were last drawn.
    preview_at: Rect,
    bar_at: Rect,
    /// Seconds until the robot's values shown are read again.
    values_age: f32,
    calm: bool,
    /// The foot of the results, where a row part-way in fades out.
    fade: crate::gfx::Fade,
}

impl Spotlight {
    pub fn new() -> Self {
        Spotlight {
            open: false,
            grow: Animated::new(0.0, role::release()).with_rest(0.002),
            glass: Animated::new(0.0, role::layer()).with_rest(0.004),
            since_closed: f32::MAX,
            rows: Animated::new(0.0, role::release()).with_rest(0.003),
            rows_most: 5,
            query: String::new(),
            hits: Vec::new(),
            recent: false,
            focus: 0,
            scroll: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.003),
            ring: Animated::new(0.0, crate::gfx::ring_spring()).with_rest(0.003),
            placed: false,
            preview: None,
            reading: None,
            slide: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.003),
            seen: u64::MAX,
            cache: Vec::new(),
            drawn: Vec::new(),
            preview_at: Rect::default(),
            bar_at: Rect::new(SIDE, TOP, W - 2.0 * SIDE, 30.0),
            values_age: 0.0,
            calm: false,
            fade: crate::gfx::Fade::default(),
        }
    }

    /// Taking keys.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// On screen at all, on its way out included.
    pub fn visible(&self) -> bool {
        self.open || self.grow.value() > 0.002 || self.glass.value() > 0.004
    }

    /// How far it has grown out of the island, 0 to 1.
    pub fn shown(&self) -> f32 {
        self.grow.value().clamp(0.0, 1.0)
    }

    /// What is typed (for the tests).
    #[cfg(test)]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The results as they stand (for the tests).
    #[cfg(test)]
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Bring it up: out of the island, or (on its way back into it) turned
    /// round from wherever it is, with what was typed still there.
    pub fn summon(&mut self, calm: bool, robot: &Robot) {
        self.calm = calm;
        let returning = !self.open && self.visible();
        self.open = true;
        self.since_closed = f32::MAX;
        if !returning {
            self.query.clear();
            self.preview = None;
            self.reading = None;
            self.slide.jump(0.0);
            self.cache.clear();
            self.placed = false;
            self.find(robot);
            self.rows.jump(0.0);
        }
        if calm {
            self.grow.jump(1.0);
            self.glass.jump(1.0);
        } else {
            self.grow.set_with(1.0, role::release());
            self.glass.set_with(1.0, role::layer());
        }
        let index = index();
        index.poke();
        index.set_active(true);
    }

    /// Put it away: it shrinks back into the island on the smooth spring,
    /// and its glass dematerializes a moment later.
    pub fn dismiss(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.since_closed = 0.0;
        if self.calm {
            self.grow.jump(0.0);
            self.glass.jump(0.0);
        } else {
            self.grow.set_with(0.0, role::smooth());
        }
        index().set_active(false);
    }

    /// Gone at once: a result is opening out of it, or something else is
    /// taking the screen.
    pub fn gone_now(&mut self) {
        if self.open {
            index().set_active(false);
        }
        self.open = false;
        self.grow.jump(0.0);
        self.glass.jump(0.0);
        self.rows.jump(0.0);
        self.preview = None;
        self.reading = None;
        self.slide.jump(0.0);
        self.since_closed = f32::MAX;
    }

    /// A preview shows, or is on its way.
    pub(crate) fn previewing(&self) -> bool {
        self.preview.is_some() || self.reading.is_some()
    }

    /// Search again for what is typed: the picks with nothing typed.
    fn find(&mut self, robot: &Robot) {
        let query = Query::new(&self.query);
        self.seen = index().generation();
        self.recent = query.is_empty();
        self.hits = if self.recent {
            index().recent(RECENT)
        } else if let Some((_, hits)) = self.cache.iter().find(|(q, _)| *q == query.text) {
            hits.clone()
        } else {
            let hits = index().search(&query, robot, LIMIT);
            self.cache.push((query.text.clone(), hits.clone()));
            if self.cache.len() > QUERY_MOST {
                self.cache.remove(0);
            }
            hits
        };
        self.focus = 0;
        // A new list: its top row in sight and in the highlight, at once.
        self.placed = false;
    }

    /// A key, while it is open.
    pub fn key(&mut self, key: Key, robot: &Robot) -> Outcome {
        let previewing = self.previewing();
        let len = self.hits.len();
        match key {
            Key::Back | Key::Leave | Key::Ctrl('q') | Key::Ctrl('\n') => return Outcome::Close,
            Key::Enter => {
                let Some(hit) = self.hits.get(self.focus).cloned() else { return Outcome::Stay };
                let from = if self.preview.is_some() && self.slide.target() > 0.5 {
                    self.preview_at
                } else {
                    self.drawn.iter().find(|(i, _)| *i == self.focus).map_or_else(|| self.bar_rect(), |(_, r)| *r)
                };
                return Outcome::Open(hit, from);
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End if len > 0 => {
                let before = self.focus;
                let page = self.rows_most.max(1);
                self.focus = match key {
                    Key::Up => self.focus.saturating_sub(1),
                    Key::Down => (self.focus + 1).min(len - 1),
                    Key::PageUp => self.focus.saturating_sub(page),
                    Key::PageDown => (self.focus + page).min(len - 1),
                    Key::Home => 0,
                    _ => len - 1,
                };
                if self.focus == before && !self.calm {
                    // Nothing further: the highlight gives a little and comes back.
                    self.ring.nudge(if key == Key::Up || key == Key::PageUp { -6.0 } else { 6.0 });
                }
                if previewing && self.focus != before {
                    self.show_preview(robot);
                }
            }
            Key::Tab | Key::Right if !previewing && len > 0 => self.show_preview(robot),
            Key::Tab | Key::Left if previewing => self.hide_preview(),
            Key::Char(c) if !c.is_control() && self.query.chars().count() < QUERY_MOST => {
                self.query.push(c);
                self.typed(robot);
            }
            Key::Backspace if !self.query.is_empty() => {
                self.query.pop();
                self.typed(robot);
            }
            // Fn + Backspace: all of it.
            Key::Delete if !self.query.is_empty() => {
                self.query.clear();
                self.typed(robot);
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn typed(&mut self, robot: &Robot) {
        self.hide_preview();
        self.find(robot);
    }

    /// Tab: the focused result's preview, from memory at once, else read on
    /// a thread and slid in when it comes. Where there is nothing to show,
    /// the list only gives a little.
    fn show_preview(&mut self, robot: &Robot) {
        let Some(hit) = self.hits.get(self.focus).cloned() else { return };
        let Some(provider) = index().provider(hit.provider) else { return };
        if let Some(preview) = provider.live_preview(&hit.item, robot) {
            self.reading = None;
            self.preview = Some((hit, preview));
            self.slide_to(1.0);
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let item = hit.item.clone();
        let started = std::thread::Builder::new().name("spotlight-preview".into()).stack_size(256 * 1024).spawn(move || {
            tx.send(provider.preview(&item)).ok();
        });
        if started.is_ok() {
            self.reading = Some((hit, rx));
        }
    }

    fn hide_preview(&mut self) {
        self.reading = None;
        if self.preview.take().is_some() {
            self.slide_to(0.0);
        }
    }

    fn slide_to(&mut self, to: f32) {
        if self.calm {
            self.slide.jump(to);
        } else {
            self.slide.set(to);
        }
    }

    /// Advance its motion. True while anything moves (or changed).
    pub fn tick(&mut self, dt: f32, robot: &Robot) -> bool {
        let mut moving = self.grow.tick(dt) | self.glass.tick(dt) | self.rows.tick(dt) | self.scroll.tick(dt) | self.ring.tick(dt) | self.slide.tick(dt);
        // The glass lingers as the bar shrinks, then dematerializes.
        if !self.open && self.since_closed < f32::MAX {
            self.since_closed += dt.max(0.001);
            if self.since_closed >= GLASS_LINGERS && self.glass.target() > 0.0 {
                self.glass.set_with(0.0, role::effect());
            }
            moving = true;
            if !self.visible() {
                self.since_closed = f32::MAX;
            }
        }
        if !self.open {
            return moving;
        }
        // The index changed under the results: find them again (the focus
        // stays on the same result if it is still there).
        if index().generation() != self.seen {
            let focused = self.hits.get(self.focus).map(|h| (h.provider, h.item.id.clone()));
            self.cache.clear();
            self.find(robot);
            if let Some(at) = focused.and_then(|f| self.hits.iter().position(|h| (h.provider, &h.item.id) == (f.0, &f.1))) {
                self.focus = at;
            }
            moving = true;
        }
        // A preview read on its thread has come: it slides in (or replaces
        // the one shown). Nothing to show: the preview goes, or, if none
        // showed, the results give a little.
        if let Some((_, rx)) = &self.reading {
            let came = match rx.try_recv() {
                Ok(preview) => Some(preview),
                Err(TryRecvError::Disconnected) => Some(None),
                Err(TryRecvError::Empty) => None,
            };
            moving = true;
            if let Some(came) = came {
                let (hit, _) = self.reading.take().expect("checked above");
                match came {
                    Some(preview) => {
                        self.preview = Some((hit, preview));
                        self.slide_to(1.0);
                    }
                    None if self.preview.is_some() => self.hide_preview(),
                    None if !self.calm => self.slide.nudge(0.8),
                    None => {}
                }
            }
        }
        // The robot's values in sight, and a preview read from memory, kept
        // current, the two at once so they always agree.
        self.values_age -= dt;
        if self.values_age <= 0.0 {
            self.values_age = 0.5;
            for hit in &mut self.hits {
                if index().provider(hit.provider).is_some_and(|p| p.name() == "values") {
                    let now = robot.get(&hit.item.id).map_or_else(|| "–".to_owned(), |v| v.display());
                    if now != hit.item.detail {
                        hit.item.detail = now;
                        moving = true;
                    }
                }
            }
            if let Some((hit, shown)) = &mut self.preview {
                if let Some(now) = index().provider(hit.provider).and_then(|p| p.live_preview(&hit.item, robot)) {
                    *shown = now;
                    moving = true;
                }
            }
        }
        moving
    }

    /// The bar's own line, as last drawn.
    fn bar_rect(&self) -> Rect {
        self.bar_at
    }

    /// Its glass, for the glass pass: grown out of `island` as far as it
    /// has, as materialized as it is. It goes first among the island's
    /// group, so where the two meet it is the bar's.
    pub fn glass(&self, g: &Gfx, island: Rect) -> GlassShape {
        let t = self.grow.value();
        let rest = rest_rect(g, self.rows.value().max(0.0));
        // Past its rest (the spring runs a little over), it grows a little
        // more, never past the panel's edges.
        let (x, y) = (lerp(island.x, rest.x, t).max(1.0), lerp(island.y, rest.y, t).max(1.0));
        let rect = Rect::new(x, y, lerp(island.w, rest.w, t).min(W - 1.0 - x), lerp(island.h, rest.h, t).clamp(1.0, H - 2.0 - y));
        let radius = lerp(island.h * 0.5, g.radius(RADIUS), t.clamp(0.0, 1.0)).min(rect.h * 0.5).min(rect.w * 0.5);
        GlassShape { rect, radius, group: 0, strength: self.glass.value().clamp(0.0, 1.0), tint: None, clear: false }
    }

    /// How much the screen behind is dimmed just now.
    pub fn dim(&self) -> f32 {
        DIM * self.shown()
    }

    /// What sits on the glass: the line typed into, the results, a preview.
    pub fn draw(&mut self, g: &mut Gfx, island: Rect) {
        let (bar_h, row_h, most) = metrics(g);
        self.rows_most = most;
        // As many rows as there are results, gliding there; a preview gets
        // all the room there is.
        let want = if self.preview.is_some() || self.hits.len() > most { room_in_rows(g).2 } else { self.hits.len() as f32 };
        if self.rows.target() != want && self.open {
            let growing = want > self.rows.target();
            if self.calm || self.grow.value() < 0.05 {
                self.rows.jump(want);
            } else {
                self.rows.set_with(want, if growing { role::release() } else { role::smooth() });
                g.started_moving();
            }
        }
        let shape = self.glass(g, island);
        let panel = shape.rect;
        // Words come once it is past halfway, as the island's do.
        let words = ((self.grow.value() - 0.55) / 0.45).clamp(0.0, 1.0);
        self.drawn.clear();
        if words <= 0.0 {
            return;
        }
        let t = g.tokens;
        let ty = g.ty;
        g.canvas.set_clip(panel.inset(1.0));
        // The line typed into.
        self.bar_at = Rect::new(panel.x, panel.y, panel.w, bar_h);
        let bar_cy = panel.y + bar_h * 0.5;
        g.icon_alpha("search", false, panel.x + 16.0, bar_cy, 15.0, t.dim, words);
        let text_x = panel.x + 30.0;
        let room = panel.w - 30.0 - 14.0 - if self.recent && !self.hits.is_empty() { 50.0 } else { 0.0 };
        let style = ty.name;
        let typed_w = if self.query.is_empty() {
            // The caret first, then what to do, as a placeholder sits.
            g.text_alpha(text_x + 4.0, bar_cy, "Search", style, t.faint, words);
            -1.0
        } else {
            let shown = fit_tail(g, &self.query, style, room - 4.0);
            g.text_alpha(text_x, bar_cy, &shown, style, t.ink, words);
            g.measure(&shown, style)
        };
        // The caret, steady: nothing here blinks.
        let caret = Rect::new((text_x + typed_w + 1.0).round(), (bar_cy - style.size * 0.55).round(), 1.5, (style.size * 1.1).round());
        g.canvas.rrect(caret, 0.75, t.signal, words);
        if self.recent && !self.hits.is_empty() {
            text_right(g, panel.right() - 14.0, bar_cy, "recent", ty.caption, t.faint, words);
        }
        // The results, or a preview of one slid in over them.
        let list = Rect::new(panel.x, panel.y + bar_h, panel.w, (panel.bottom() - panel.y - bar_h).max(0.0));
        if list.h > 1.0 {
            // A hairline under the line typed into.
            g.canvas.rrect(Rect::new(panel.x + 12.0, list.y, panel.w - 24.0, 1.0), 0.5, t.ink, 0.10 * words);
            g.canvas.set_clip(list.inset(1.0));
            let slide = self.slide.value();
            if slide < 0.999 {
                self.draw_results(g, list, row_h, -slide * list.w, words);
            }
            if slide > 0.001 {
                self.draw_preview(g, list, (1.0 - slide) * list.w, words);
            }
        }
        g.canvas.clear_clip();
    }

    fn draw_results(&mut self, g: &mut Gfx, list: Rect, row_h: f32, dx: f32, alpha: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let step = row_h + ROW_GAP;
        let top = list.y + LIST_ABOVE;
        let in_sight = ((list.h - LIST_ABOVE - LIST_BELOW + ROW_GAP) / step).floor().max(1.0) as usize;
        let most = self.rows_most.max(1);
        // The first row in sight follows the focus, gliding there.
        let mut first = self.scroll.target().round() as usize;
        if self.focus < first {
            first = self.focus;
        } else if self.focus >= first + most {
            first = self.focus + 1 - most;
        }
        let first = first.min(self.hits.len().saturating_sub(most));
        let (first_f, focus_f) = (first as f32, self.focus as f32);
        if !self.placed || self.calm {
            self.scroll.jump(first_f);
            self.ring.jump(focus_f);
            self.placed = true;
        } else {
            if self.scroll.target() != first_f {
                self.scroll.set(first_f);
                g.started_moving();
            }
            if self.ring.target() != focus_f {
                self.ring.set(focus_f);
                g.started_moving();
            }
        }
        let scroll = self.scroll.value();
        let x = list.x + ROW_INSET + dx;
        let w = list.w - 2.0 * ROW_INSET;
        let radius = (g.radius(RADIUS) - ROW_INSET).max(2.0).min(row_h * 0.5);
        // The highlight: a state layer in the words' colour, and the ring,
        // gliding from row to row.
        if !self.hits.is_empty() {
            let y = top + (self.ring.value() - scroll) * step;
            let r = Rect::new(x, y, w, row_h);
            g.canvas.rrect(r, radius, t.ink, 0.12 * alpha);
            g.canvas.stroke_rrect(r.inset(-1.5), radius + 1.5, 1.5, g.ring_color(), alpha);
        }
        let from = scroll.floor().max(0.0) as usize;
        // Under the whole rows, the next one fades out into the glass.
        let whole_end = top + in_sight as f32 * step - ROW_GAP;
        self.fade.take(g, whole_end + crate::gfx::RING_ROOM, list.bottom() - 1.0);
        for i in from..(from + in_sight + 1).min(self.hits.len()) {
            let y = top + (i as f32 - scroll) * step;
            if y > list.bottom() || y + row_h < list.y {
                continue;
            }
            let r = Rect::new(x, y, w, row_h);
            self.drawn.push((i, r));
            let hit = &self.hits[i];
            let focused = i == self.focus;
            let cy = r.cy();
            g.icon_alpha(&hit.item.icon, focused, r.x + 13.0, cy, 14.0, if focused { t.ink } else { t.dim }, alpha);
            // The name and what it is share the row: what it is takes the
            // room the name leaves (a third at least), and the name the rest.
            let room = w - 26.0 - 9.0 - 12.0;
            let title_w = g.measure(&hit.item.title, ty.body);
            let detail = g.text.fit(&hit.item.detail, &ty.caption, (room - title_w).max(room * 0.34));
            let detail_w = if detail.is_empty() { 0.0 } else { g.measure(&detail, ty.caption) };
            text_right(g, r.right() - 9.0, cy, &detail, ty.caption, t.dim, alpha);
            let title = g.text.fit(&hit.item.title, &ty.body, room - detail_w);
            g.text_alpha(r.x + 26.0, cy, &title, ty.body, t.ink, alpha);
        }
        self.fade.apply(g);
        // More below or above: a thin bar along the right, inside the glass,
        // beside the whole rows.
        if self.hits.len() > most {
            let track = Rect::new(list.right() - 3.5 + dx, top, 2.0, (whole_end - top).min(list.h - LIST_ABOVE - LIST_BELOW));
            let share = most as f32 / self.hits.len() as f32;
            let at = scroll / (self.hits.len() - most) as f32;
            let thumb = Rect::new(track.x, track.y + (track.h - track.h * share) * at.clamp(0.0, 1.0), 2.0, (track.h * share).max(8.0));
            g.canvas.rrect(thumb, 1.0, t.ink, 0.35 * alpha);
        }
    }

    fn draw_preview(&mut self, g: &mut Gfx, list: Rect, dx: f32, alpha: f32) {
        let Some((hit, preview)) = &self.preview else { return };
        let t = g.tokens;
        let ty = g.ty;
        let area = Rect::new(list.x + ROW_INSET + dx, list.y + LIST_ABOVE, list.w - 2.0 * ROW_INSET, list.h - LIST_ABOVE - LIST_BELOW);
        self.preview_at = area.offset(-dx, 0.0);
        // What it is, on the first line; the rest says the rest.
        let head_cy = area.y + ty.name.size * 0.5 + 4.0;
        g.icon_alpha(&hit.item.icon, true, area.x + 13.0, head_cy, 15.0, t.ink, alpha);
        let title = g.text.fit(&hit.item.title, &ty.name, area.w - 26.0 - 9.0);
        g.text_alpha(area.x + 26.0, head_cy, &title, ty.name, t.ink, alpha);
        let body = Rect::new(area.x + 9.0, area.y + ty.name.size + 12.0, area.w - 18.0, area.bottom() - (area.y + ty.name.size + 12.0));
        let line_h = ty.caption.size + 4.0;
        match preview {
            Preview::Text { lines, facts } => {
                let fit = ((body.h - line_h) / line_h).floor().max(1.0) as usize;
                for (k, line) in lines.iter().take(fit).enumerate() {
                    let line = g.text.fit(line, &ty.caption, body.w);
                    g.text_alpha(body.x, body.y + (k as f32 + 0.5) * line_h, &line, ty.caption, t.ink.mix(t.dim, 0.35), alpha);
                }
                let foot = facts.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(" · ");
                let foot = g.text.fit(&foot, &ty.caption, body.w);
                g.text_alpha(body.x, body.bottom() - line_h * 0.5, &foot, ty.caption, t.faint, alpha);
            }
            Preview::Picture { picture, facts } => {
                let ph = body.h.min(picture.h as f32);
                let pw = (ph * picture.w as f32 / picture.h.max(1) as f32).round();
                let dest = Rect::new(body.x, body.y, pw, ph);
                crate::photos::draw_picture(g, dest, picture, crate::photos::whole(picture), 6.0, alpha);
                draw_facts(g, Rect::new(dest.right() + 10.0, body.y, body.right() - dest.right() - 10.0, body.h), facts, alpha);
            }
            Preview::Facts(facts) => draw_facts(g, body, facts, alpha),
        }
    }
}

/// Facts as label and value, a line each (a long value over two).
fn draw_facts(g: &mut Gfx, r: Rect, facts: &[(String, String)], alpha: f32) {
    let t = g.tokens;
    let ty = g.ty;
    let line_h = ty.body.size + 6.0;
    let label_w = facts.iter().map(|(k, _)| g.measure(k, ty.caption)).fold(0.0, f32::max).min(r.w * 0.35) + 10.0;
    let mut y = r.y;
    for (label, value) in facts {
        if y + line_h > r.bottom() + 1.0 {
            break;
        }
        let cy = y + line_h * 0.5;
        g.text_alpha(r.x, cy, label, ty.caption, t.dim, alpha);
        let room = r.w - label_w;
        let lines = wrap_chars(g, value, ty.body, room);
        let two = lines.len() > 1 && y + 2.0 * line_h <= r.bottom() + 1.0;
        let first = if two || lines.len() <= 1 { lines.first().cloned().unwrap_or_default() } else { g.text.fit(value, &ty.body, room) };
        g.text_alpha(r.x + label_w, cy, &first, ty.body, t.ink, alpha);
        y += line_h;
        if two {
            let rest = g.text.fit(&lines[1..].concat(), &ty.body, room);
            g.text_alpha(r.x + label_w, y + line_h * 0.5, &rest, ty.body, t.ink, alpha);
            y += line_h;
        }
    }
}

/// A value broken into lines by width alone: a path or a command may have
/// no spaces to break at.
fn wrap_chars(g: &mut Gfx, text: &str, style: TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for c in text.chars() {
        line.push(c);
        if g.measure(&line, style) > width && line.chars().count() > 1 {
            line.pop();
            lines.push(std::mem::take(&mut line));
            line.push(c);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The end of what was typed, as much as fits, an ellipsis before it if
/// some is left out: the caret's end is what matters.
fn fit_tail(g: &mut Gfx, text: &str, style: TextStyle, width: f32) -> String {
    if g.measure(text, style) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let tail: String = std::iter::once('…').chain(chars[start..].iter().copied()).collect();
        if g.measure(&tail, style) <= width {
            return tail;
        }
    }
    chars.last().map(|c| c.to_string()).unwrap_or_default()
}

/// Right-aligned text centred on `cy`, at an opacity.
fn text_right(g: &mut Gfx, right: f32, cy: f32, s: &str, style: TextStyle, color: Color, alpha: f32) {
    let baseline = (cy + style.size * 0.36).round();
    g.text.draw_aligned(&mut g.canvas, right, baseline, Align::Right, s, &style, color, alpha);
}
