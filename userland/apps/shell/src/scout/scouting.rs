//! The form, filled in during a match: the page for the phase the match
//! clock is in, its fields as tiles with their keys, and the whole-match
//! ones (fouls) beneath. A key adds one; Shift, or the key under it, takes
//! one away; Del (Fn + Backspace) undoes the last change. Space starts the clock as
//! the match starts (or the first count in AUTO does), and the form turns
//! with it from AUTO to TELEOP to END GAME, and to after the match for the
//! ratings and notes. Left and Right turn it by hand.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use super::form::{Form, Kind};
use super::store::{Record, Station};
use super::Me;
use crate::gfx::{Gfx, ListState, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// The match clock: time banked, and when the running stretch began.
#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    banked: f32,
    since: Option<Instant>,
}

impl Clock {
    fn elapsed(&self, now: Instant, most: f32) -> f32 {
        (self.banked + self.since.map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f32())).min(most)
    }

    fn started(&self) -> bool {
        self.since.is_some() || self.banked > 0.0
    }

    fn toggle(&mut self, now: Instant, most: f32) {
        match self.since {
            Some(_) => {
                self.banked = self.elapsed(now, most);
                self.since = None;
            }
            None => self.since = Some(now),
        }
    }
}

/// `m:ss`, as the field shows it.
fn clock_text(seconds: f32) -> String {
    let s = seconds.max(0.0).ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

pub struct Scouting {
    /// The team's number: the robot being watched.
    title: String,
    form: Arc<Form>,
    me: Me,
    record: Record,
    /// The record as it was opened, for its history.
    opened: Record,
    /// As it was last written to the card.
    kept_as: Record,
    /// It is on the card (else it is kept only once something is in it),
    /// and it was when this opened.
    existed: bool,
    opened_existed: bool,
    pages: Vec<usize>,
    page: usize,
    /// The page the clock was last in: when it moves on, so does the form.
    clock_page: usize,
    clock: Clock,
    /// What was shown last, to draw again only when it changes.
    shown: (u32, usize),
    /// Changes, newest last, for Backspace: the column and its value before.
    undo: Vec<(String, f32)>,
    /// Each field's state layer, lit by its key.
    flash: Vec<Animated>,
    /// After the match: the row in focus.
    list: ListState,
    notes: Option<Field>,
    saved_at: Instant,
    dirty: bool,
}

/// How long changes wait to be written while keys come quickly.
const SAVE_EVERY: Duration = Duration::from_secs(3);

impl Scouting {
    pub fn new(event: &str, match_key: &str, label: &str, team: u32, station: Station) -> Self {
        let form = super::form();
        let me = Me::load();
        let kept = super::with_store(|s| s.get(match_key, team).cloned());
        let existed = kept.is_some();
        let record = kept.unwrap_or_else(|| Record {
            event: event.to_owned(),
            match_key: match_key.to_owned(),
            label: label.to_owned(),
            team,
            station,
            form: form.id.clone(),
            scout: me.name.clone(),
            by: me.name.clone(),
            ..Record::default()
        });
        let pages = form.pages();
        let flash = form.fields.iter().map(|_| Animated::new(0.0, role::effect()).with_rest(0.01)).collect();
        // A record kept already opens after the match, for its ratings and notes.
        let page = if existed { pages.len().saturating_sub(1) } else { 0 };
        Scouting { title: team.to_string(), form, me, opened: record.clone(), kept_as: record.clone(), record, existed, opened_existed: existed, pages, page, clock_page: 0, clock: Clock::default(), shown: (u32::MAX, usize::MAX), undo: Vec::new(), flash, list: ListState::default(), notes: None, saved_at: Instant::now(), dirty: false }
    }

    /// For the gallery: values in, and the clock `elapsed` seconds in and
    /// running (stopped once the match is over).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn demo(&mut self, values: &[(&str, f32)], elapsed: f32, page: Option<usize>) {
        for (c, v) in values {
            self.record.values.insert((*c).to_owned(), *v);
        }
        let over = elapsed >= self.form.match_seconds();
        self.clock = Clock { banked: elapsed, since: (!over).then(Instant::now) };
        self.clock_page = usize::MAX;
        self.follow_clock();
        if let Some(p) = page {
            self.page = p;
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn notes_text(&mut self, words: &str) {
        if let Some(f) = self.form.fields.iter().find(|f| matches!(f.kind, Kind::Text(_))) {
            self.record.texts.insert(f.column(&self.form), words.to_owned());
        }
    }

    fn section(&self) -> usize {
        self.pages.get(self.page).copied().unwrap_or(0)
    }

    fn elapsed(&self) -> f32 {
        self.clock.elapsed(Instant::now(), self.form.match_seconds())
    }

    fn running(&self) -> bool {
        self.clock.since.is_some() && self.elapsed() < self.form.match_seconds()
    }

    /// The page the clock says, once it has started.
    fn page_of_clock(&self) -> usize {
        let s = self.form.section_at(self.elapsed());
        self.pages.iter().position(|p| *p == s).unwrap_or(0)
    }

    /// The form follows the clock into each new phase.
    fn follow_clock(&mut self) -> bool {
        if !self.clock.started() {
            return false;
        }
        let at = self.page_of_clock();
        if at != self.clock_page {
            self.clock_page = at;
            self.page = at;
            return true;
        }
        false
    }

    /// Change a field by a step; remembered for undo.
    fn step(&mut self, field: usize, by: i32) -> String {
        let form = self.form.clone();
        let f = &form.fields[field];
        let column = f.column(&form);
        let before = self.record.value(&column);
        let after = f.stepped(before, by);
        if after != before {
            self.undo.push((column.clone(), before));
            self.set(&column, after);
        }
        self.flash[field].jump(1.0);
        self.flash[field].set(0.0);
        format!("{} {}", f.label, f.shown(after))
    }

    fn set(&mut self, column: &str, value: f32) {
        if value == 0.0 {
            self.record.values.remove(column);
        } else {
            self.record.values.insert(column.to_owned(), value);
        }
        self.dirty = true;
    }

    /// Keep what has changed: on leaving, with a line in its history.
    fn save(&mut self, leaving: bool) {
        if !self.dirty && !leaving {
            return;
        }
        self.dirty = false;
        self.saved_at = Instant::now();
        let since_open = !self.record.same_content(&self.opened);
        let since_kept = !self.record.same_content(&self.kept_as);
        let blank = self.record.values.is_empty() && self.record.texts.values().all(String::is_empty);
        // Nothing to keep: a match opened and left as it was, or never filled in.
        if !since_kept && !(leaving && since_open) || (!self.existed && blank) {
            return;
        }
        let at = super::now();
        if leaving && since_open {
            let change = if self.opened_existed { self.record.change_from(&self.opened, &self.form) } else { "scouted".to_owned() };
            self.record.stamp(at, &self.me.name, change);
            self.opened = self.record.clone();
            self.opened_existed = true;
        } else {
            self.record.updated = at;
            self.record.by = self.me.name.clone();
        }
        let record = self.record.clone();
        if super::with_store(|s| s.put(record)).is_ok() {
            self.kept_as = self.record.clone();
            self.existed = true;
        }
    }

    /// The fields on a timed page's first row: its own and those it
    /// inherits (not the whole-match ones).
    fn page_fields(&self, section: usize) -> Vec<usize> {
        let form = &self.form;
        let mut own: Vec<usize> = (0..form.fields.len()).filter(|&i| form.fields[i].section == section).collect();
        if let Some(from) = form.sections[section].inherits {
            own.extend((0..form.fields.len()).filter(|&i| form.fields[i].section == from));
        }
        own
    }

    /// The whole-match fields.
    fn always_fields(&self) -> Vec<usize> {
        (0..self.form.fields.len()).filter(|&i| self.form.sections[self.form.fields[i].section].always).collect()
    }

    /// The rows after the match: the points, then the section's own fields.
    fn after_rows(&self) -> Vec<Option<usize>> {
        let s = self.section();
        std::iter::once(None).chain((0..self.form.fields.len()).filter(|&i| self.form.fields[i].section == s).map(Some)).collect()
    }

    fn timed_page(&self) -> bool {
        self.form.sections[self.section()].seconds > 0.0
    }
}

impl Screen for Scouting {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn wants_text(&self) -> bool {
        self.notes.is_some()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.notes.is_some() {
            return vec![keys("notes", &[("enter", "Keep them")])];
        }
        const COUNTS: &[(&str, &str)] = &[("its key", "Add one"), ("shift its key", "Take one away"), ("fn bksp", "Undo")];
        if self.timed_page() {
            let pages = keys("pages", &[("← →", "Another phase"), ("enter", "Next phase")]);
            if !self.clock.started() {
                return vec![keys("the clock", &[("space", "Start the clock")]), keys("counting", COUNTS), pages];
            }
            return vec![keys("counting", COUNTS), keys("the clock", &[("space", "Pause or go on")]), pages];
        }
        let row = match self.after_rows().get(self.list.focus).copied().flatten().map(|i| &self.form.fields[i].kind) {
            Some(Kind::Rating(_)) => keys("a rating", &[("1 to 9", "Rate it"), ("0", "Not rated"), ("enter", "One higher")]),
            Some(Kind::Text(_)) => keys("notes", &[("enter", "Write them")]),
            Some(_) => keys("this row", &[("enter", "Change it")]),
            None => keys("the points", &[("enter", "Done, it is kept")]),
        };
        vec![row, keys("after the match", &[("↑ ↓", "Another row"), ("← →", "Another page")]), keys("counting", COUNTS)]
    }

    fn keeps_awake(&self) -> bool {
        self.running()
    }

    /// The clock is looked at four times a second while it runs; it never
    /// holds the frame beat.
    fn ambient(&self) -> Option<Duration> {
        self.running().then_some(Duration::from_millis(250))
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        let turned = self.follow_clock();
        let shown = (self.elapsed().ceil() as u32, self.page);
        let changed = shown != self.shown;
        self.shown = shown;
        turned || changed
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for f in &mut self.flash {
            moving |= f.tick(dt);
        }
        moving
    }

    fn set_aside(&mut self) {
        self.save(true);
    }

    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        self.notes.as_ref().map(|f| f.text.clone())
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.notes {
            match field.key(key, |c| !c.is_control()) {
                Typed::More => {}
                Typed::Cancelled => self.notes = None,
                Typed::Done(text) => {
                    self.notes = None;
                    if let Some(f) = self.form.fields.iter().find(|f| matches!(f.kind, Kind::Text(_)) && f.section == self.section()) {
                        let c = f.column(&self.form);
                        if text.is_empty() {
                            self.record.texts.remove(&c);
                        } else {
                            self.record.texts.insert(c, text);
                        }
                        self.dirty = true;
                        self.save(false);
                    }
                }
            }
            return Nav::Stay;
        }
        self.follow_clock();
        let section = self.section();
        match key {
            Key::Back => {
                self.save(true);
                return Nav::Pop;
            }
            Key::Char(' ') => {
                let most = self.form.match_seconds();
                if self.elapsed() >= most {
                    *cx.say = Some("the match is over".into());
                } else {
                    self.clock.toggle(Instant::now(), most);
                    if self.clock.since.is_some() {
                        self.clock_page = usize::MAX;
                        self.follow_clock();
                    }
                    *cx.say = Some((if self.clock.since.is_some() { "clock running" } else { "clock paused" }).into());
                }
            }
            Key::Left | Key::Right => {
                let to = if key == Key::Left { self.page.saturating_sub(1) } else { (self.page + 1).min(self.pages.len() - 1) };
                if to != self.page {
                    self.page = to;
                    self.save(false);
                }
            }
            // Del (Fn + Backspace) on the handheld: Backspace alone is Esc here.
            Key::Backspace | Key::Delete => match self.undo.pop() {
                Some((column, before)) => {
                    self.set(&column, before);
                    if let Some(i) = self.form.fields.iter().position(|f| f.column(&self.form) == column) {
                        self.flash[i].jump(1.0);
                        self.flash[i].set(0.0);
                        *cx.say = Some(format!("undone: {} {}", self.form.fields[i].label, self.form.fields[i].shown(before)));
                    }
                }
                None => *cx.say = Some("nothing to undo".into()),
            },
            Key::Up | Key::Down if !self.timed_page() => {
                let len = self.after_rows().len();
                crate::screens::list_key(&mut self.list, key, len);
            }
            Key::Char(c @ '0'..='9') if !self.timed_page() => {
                if let Some(Some(i)) = self.after_rows().get(self.list.focus).copied() {
                    if let Kind::Rating(max) = self.form.fields[i].kind {
                        let v = (c as u32 - '0' as u32).min(max) as f32;
                        let column = self.form.fields[i].column(&self.form);
                        self.undo.push((column.clone(), self.record.value(&column)));
                        self.set(&column, v);
                        self.flash[i].jump(1.0);
                        self.flash[i].set(0.0);
                    }
                }
            }
            Key::Enter if !self.timed_page() => match self.after_rows().get(self.list.focus).copied() {
                Some(Some(i)) => match self.form.fields[i].kind {
                    Kind::Text(most) => {
                        let c = self.form.fields[i].column(&self.form);
                        self.notes = Some(Field::new("notes", self.record.texts.get(&c).map_or("", String::as_str), most));
                    }
                    Kind::Rating(max) => {
                        // Enter goes round the ratings: 1, 2 … max, then none.
                        let column = self.form.fields[i].column(&self.form);
                        let before = self.record.value(&column);
                        self.undo.push((column.clone(), before));
                        self.set(&column, ((before as u32 + 1) % (max + 1)) as f32);
                        self.flash[i].jump(1.0);
                        self.flash[i].set(0.0);
                    }
                    _ => {
                        let said = self.step(i, 1);
                        *cx.say = Some(said);
                    }
                },
                _ => {
                    self.save(true);
                    return Nav::Pop;
                }
            },
            Key::Enter => {
                // Enter in a phase: on to the next page.
                self.page = (self.page + 1).min(self.pages.len() - 1);
            }
            Key::Char(c) => match self.form.key_in(section, c) {
                Some((field, by)) => {
                    // The first count in AUTO starts the clock, if Space was missed.
                    if !self.clock.started() && self.page == 0 && by > 0 {
                        self.clock.toggle(Instant::now(), self.form.match_seconds());
                        self.clock_page = 0;
                    }
                    let said = self.step(field, by);
                    if self.saved_at.elapsed() >= SAVE_EVERY {
                        self.save(false);
                    }
                    // After the match a whole-match key has no tile to light: it says.
                    if !self.timed_page() {
                        *cx.say = Some(said);
                    }
                }
                None => {}
            },
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.notes {
            field.draw(g, "enter: keep · esc: cancel");
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        let form = self.form.clone();
        let section = self.section();
        let elapsed = self.elapsed();
        let running = self.running();

        // The phase, the match as a bar of its phases, the clock.
        let strip_h = ty.label.size + 6.0;
        let cy = top + strip_h * 0.5 - 1.0;
        // The match, then the phase (the team is the title).
        let label_w = g.text_at(PAD + 2.0, cy, Align::Left, &self.record.label, ty.label, t.ink) + 8.0;
        let name = &form.sections[section].name;
        let name_w = label_w + g.text_at(PAD + 2.0 + label_w, cy, Align::Left, name, ty.label, if running { t.ink } else { t.dim });
        let first = form.timed().first().copied();
        let shown_clock = if !self.clock.started() {
            first.map_or(0.0, |s| form.sections[s].seconds)
        } else if Some(form.section_at(elapsed)) == first {
            first.map_or(0.0, |s| form.sections[s].seconds) - elapsed
        } else {
            form.match_seconds() - elapsed
        };
        let clock = clock_text(shown_clock);
        let clock_w = g.text_at(W - PAD - 2.0, cy, Align::Right, &clock, ty.label, if running { t.ink } else { t.dim });
        // Running, or waiting for Space; once the match is over, neither.
        if elapsed < form.match_seconds() {
            let state_icon = if running { "pause" } else { "play_arrow" };
            g.icon(state_icon, true, W - PAD - clock_w - 12.0, cy, 13.0, if running { t.signal } else { t.dim });
        }
        let bar = Rect::new(PAD + name_w + 12.0, cy - 2.5, W - 2.0 * PAD - name_w - clock_w - 36.0, 5.0);
        phase_bar(g, &form, bar, elapsed, self.clock.started(), section);

        let grid_top = top + strip_h + 4.0;
        if self.timed_page() {
            let main = self.page_fields(section);
            let always = self.always_fields();
            let room = bottom - grid_top;
            let lower_h = if always.is_empty() { 0.0 } else { (ty.caption.size + ty.display(17.0).size + 16.0).min(room * 0.45) };
            let upper_h = room - if always.is_empty() { 0.0 } else { lower_h + GAP };
            self.tiles(g, &main, Rect::new(PAD, grid_top, W - 2.0 * PAD, upper_h), 28.0, true);
            if !always.is_empty() {
                self.tiles(g, &always, Rect::new(PAD, grid_top + upper_h + GAP, W - 2.0 * PAD, lower_h), 18.0, false);
            }
            let hint: &[&str] = if !self.clock.started() {
                &["space: start the clock · shift+key: take one away", "space: start · shift+key: take away", "space: start the clock"]
            } else {
                &["del: undo · shift+key: take one away · ←→ page", "del: undo · shift+key: take away", "del: undo · ←→ page"]
            };
            g.hint_any(hint);
        } else {
            self.after(g, grid_top, bottom);
        }
    }
}

impl Scouting {
    /// Fields as a row of tiles in `r`: each with its key, its value and its name.
    fn tiles(&mut self, g: &mut Gfx, fields: &[usize], r: Rect, value_size: f32, main: bool) {
        let t = g.tokens;
        let ty = g.ty;
        let n = fields.len().max(1) as f32;
        let w = ((r.w - GAP * (n - 1.0)) / n).floor();
        let radius = g.radius(12.0).min(r.h * 0.5);
        for (k, &i) in fields.iter().enumerate() {
            let f = &self.form.fields[i];
            let tile = Rect::new(r.x + k as f32 * (w + GAP), r.y, w, r.h);
            let lit = self.flash[i].value().clamp(0.0, 1.0);
            g.surface(tile, radius, if main { t.surface2 } else { t.surface1 });
            if lit > 0.0 {
                // The state layer: the tile answers its key.
                g.canvas.rrect(tile.offset(g.ox, 0.0), radius, t.ink, 0.14 * lit);
            }
            // The key, as a cap in the corner.
            let cap = ty.caption.size + 4.0;
            let cap_r = Rect::new(tile.x + 6.0, tile.y + 6.0, cap, cap);
            g.surface(cap_r, g.radius(4.0).min(cap * 0.5), t.surface3);
            let key = f.key.map(String::from).unwrap_or_default();
            g.text_at(cap_r.cx(), cap_r.cy(), Align::Center, &key, ty.caption, t.ink);
            let value = self.record.value(&f.column(&self.form));
            // The name along the foot; the value on the right above it, as
            // large as the tile leaves room for (figures stand about three
            // quarters of the type's size) and never over the key.
            let label_cy = tile.bottom() - 6.0 - ty.caption.size * 0.5;
            g.text_fit(tile.x + 7.0, label_cy, &f.label, ty.caption, t.dim, tile.w - 14.0);
            let baseline = tile.bottom() - 6.0 - ty.caption.size - 4.0;
            let words = match f.kind {
                Kind::Toggle => (if value > 0.5 { "yes" } else { "no" }).to_owned(),
                _ => f.shown(value),
            };
            let ink = match f.kind {
                Kind::Toggle if value > 0.5 => g.good(),
                Kind::Toggle => t.faint,
                _ if value == 0.0 => t.dim,
                _ if f.lower_better => t.amber_ink,
                _ => t.ink,
            };
            let mut style = ty.display(value_size.min((baseline - tile.y - 5.0) / 0.74 / ty.display_scale));
            let room = tile.w - cap - 22.0;
            let wide = g.measure(&words, style);
            if wide > room {
                style = bezel::TextStyle { size: style.size * room / wide, ..style };
            }
            let value_w = g.measure(&words, style);
            g.text(tile.right() - 8.0 - value_w, baseline, &words, style, ink);
        }
    }

    /// After the match: the points it made, then ratings and notes.
    fn after(&mut self, g: &mut Gfx, top: f32, bottom: f32) {
        let t = g.tokens;
        let form = self.form.clone();
        let rows = self.after_rows();
        let row_h = g.row_h();
        for (k, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = k == self.list.focus;
            match rows[k] {
                None => {
                    let points = form.totals.first().map_or(0.0, |total| form.total(total, &self.record.values));
                    let parts: Vec<String> = form.totals.iter().skip(1).filter(|x| !x.lower_better).map(|x| format!("{}", form.total(x, &self.record.values) as i64)).collect();
                    let value = if parts.is_empty() { format!("{}", points as i64) } else { format!("{} = {}", points as i64, parts.join(" + ")) };
                    g.row(r, "points", &value, t.ink, focused);
                }
                Some(i) => {
                    let f = &form.fields[i];
                    let c = f.column(&form);
                    let value = match f.kind {
                        Kind::Text(_) => self.record.texts.get(&c).cloned().unwrap_or_default(),
                        _ => f.shown(self.record.value(&c)),
                    };
                    let lit = self.flash[i].value().clamp(0.0, 1.0);
                    match f.kind {
                        Kind::Text(_) if value.is_empty() => g.row(r, &f.label, "enter: write", t.faint, focused),
                        Kind::Text(_) => {
                            // The notes themselves, as much as fits after the name.
                            let name_w = g.measure(&f.label, g.ty.body) + 24.0;
                            let fitted = g.text.fit(&value, &g.ty.label, r.w - name_w - 14.0);
                            g.row(r, &f.label, &fitted, t.dim, focused);
                        }
                        Kind::Rating(_) => {
                            g.row(r, &f.label, &value, if self.record.value(&c) > 0.0 { t.ink } else { t.faint }, focused);
                            if lit > 0.0 {
                                g.canvas.rrect(r.offset(g.ox, 0.0), r.h * 0.5, t.ink, 0.12 * lit);
                            }
                        }
                        _ => g.row(r, &f.label, &value, t.ink, focused),
                    }
                }
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let rate;
        let hint: Vec<&str> = match rows.get(self.list.focus).copied().flatten().map(|i| &form.fields[i].kind) {
            Some(Kind::Rating(max)) => {
                rate = [format!("1–{max}: rate · 0: not rated · ←: the match"), format!("1–{max}: rate · 0: none")];
                vec![&rate[0], &rate[1]]
            }
            Some(Kind::Text(_)) => vec!["enter: write notes · esc: done", "enter: write"],
            _ => vec!["esc: done, it is kept · ←: the match", "esc: done · ←: the match"],
        };
        g.hint_any(&hint);
    }
}

/// The match's timed phases as a bar, each in proportion, filled as far as
/// the clock has come; the page shown, before the clock starts, marked.
fn phase_bar(g: &mut Gfx, form: &Form, r: Rect, elapsed: f32, started: bool, section: usize) {
    let t = g.tokens;
    let timed = form.timed();
    let total = form.match_seconds().max(1.0);
    let gap = 2.0;
    let usable = r.w - gap * (timed.len().saturating_sub(1)) as f32;
    let mut x = r.x + g.ox;
    let mut begun = 0.0;
    for (k, &s) in timed.iter().enumerate() {
        let secs = form.sections[s].seconds;
        let w = usable * secs / total;
        let seg = Rect::new(x, r.y, w, r.h);
        g.canvas.rrect(seg, r.h * 0.5, t.meter, 1.0);
        let filled = if started { ((elapsed - begun) / secs).clamp(0.0, 1.0) } else if s == section { 1.0 } else { 0.0 };
        if filled > 0.0 {
            let ink: Color = if !started { t.faint } else if k == timed.len() - 1 { t.amber } else if k == 0 { t.ice } else { t.signal };
            g.canvas.rrect_clipped(Rect::new(seg.x, seg.y, (w * filled).max(r.h * 0.6), seg.h), 1.0, seg, r.h * 0.5, ink, 1.0);
        }
        x += w + gap;
        begun += secs;
    }
}
