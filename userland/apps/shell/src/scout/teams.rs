//! Teams as the records see them: a list sorted by any metric, each with
//! its line over the event; a page per team, every metric's average and
//! trend and the scouts' notes; and the picklist, put in order by keys.

use std::sync::Arc;

use bezel::{Align, Color, Rect};

use super::form::Form;
use super::stats::{self, Picklist, Team};
use super::store::Record;
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// The event's records, and its teams worked out from them.
fn load(event: &str) -> (Vec<Record>, Vec<Team>) {
    let form = super::form();
    let records: Vec<Record> = super::with_store(|s| s.of_event(event).cloned().collect());
    let refs: Vec<&Record> = records.iter().collect();
    let teams = stats::teams(&form, &refs);
    (records, teams)
}

/// A metric's values over a team's matches, as a line in `r`: quiet, with
/// the latest match marked in the accent. Scaled from nothing to `top`, the
/// most any team reached, so lines compare across teams.
pub fn spark(g: &mut Gfx, r: Rect, values: &[f32], top: f32) {
    let t = g.tokens;
    if values.is_empty() {
        return;
    }
    let top = top.max(f32::EPSILON);
    let y = |v: f32| r.bottom() - (v / top).clamp(0.0, 1.0) * r.h;
    let n = values.len();
    let step = if n > 1 { r.w / (n - 1) as f32 } else { 0.0 };
    let x0 = if n > 1 { r.x } else { r.right() };
    let pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| (x0 + i as f32 * step + g.ox, y(*v))).collect();
    // The baseline, faint, so a line at nothing still reads as a line.
    g.canvas.fill_rect(Rect::new(r.x + g.ox, r.bottom(), r.w, 1.0), t.faint, 0.5);
    if n > 1 {
        g.canvas.polyline(&pts, 1.5, t.dim, 1.0);
    }
    let (lx, ly) = pts[n - 1];
    g.canvas.circle(lx, ly, 2.5, t.signal, 1.0);
}

/// "↑", "↓" or nothing: how the metric has gone over the event.
fn arrow(team: &Team, metric: usize, top: f32) -> &'static str {
    match team.trend(metric) {
        Some(d) if d > top * 0.08 => "↑",
        Some(d) if d < -top * 0.08 => "↓",
        _ => "",
    }
}

// ------------------------------------------------------------------ teams ---

pub struct Teams {
    event: String,
    form: Arc<Form>,
    teams: Vec<Team>,
    metric: usize,
    top: f32,
    list: ListState,
}

impl Teams {
    pub fn new(event: String) -> Self {
        let form = super::form();
        let mut me = Teams { event, form, teams: Vec::new(), metric: 0, top: 0.0, list: ListState::default() };
        me.reload();
        me
    }

    fn reload(&mut self) {
        let focused = self.teams.get(self.list.focus).map(|t| t.team);
        let (_, mut teams) = load(&self.event);
        stats::sort_by(&self.form, &mut teams, self.metric);
        self.top = stats::scale(&teams, self.metric);
        self.teams = teams;
        if let Some(team) = focused {
            self.list.focus = self.teams.iter().position(|t| t.team == team).unwrap_or(0);
        }
    }
}

impl Screen for Teams {
    fn title(&self) -> &str {
        "Teams"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn resume(&mut self) {
        self.reload();
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let metrics = self.form.metrics().len();
        match key {
            Key::Back => return Nav::Pop,
            Key::Left | Key::Right => {
                self.metric = (self.metric as i32 + if key == Key::Left { -1 } else { 1 }).rem_euclid(metrics as i32) as usize;
                self.reload();
            }
            Key::Enter => {
                if let Some(t) = self.teams.get(self.list.focus) {
                    return Nav::Push(Box::new(TeamPage::new(self.event.clone(), t.team)));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.teams.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = g.above_hint();
        if self.teams.is_empty() {
            g.empty("groups", "teams appear here once matches are scouted");
            g.hint("esc: back");
            return;
        }
        // What the list is sorted by, over it.
        let metrics = self.form.metrics();
        let m = &metrics[self.metric.min(metrics.len() - 1)];
        let head_h = ty.caption.size + 6.0;
        let cy = top + head_h * 0.5 - 1.0;
        let w = g.text_at(PAD + 4.0, cy, Align::Left, "by", ty.caption, t.dim);
        g.text_at(PAD + 10.0 + w, cy, Align::Left, &self.form.metric_label(m), ty.caption, t.ink);
        let order = if self.form.lower_better(m) { "fewest first" } else { "most first" };
        g.text_at(W - PAD - 6.0, cy, Align::Right, order, ty.caption, t.dim);
        let list_top = top + head_h + 2.0;
        let row_h = g.row_h();
        let bezel = g.theme == crate::look::Theme::Bezel;
        let value_style = if bezel { ty.body } else { ty.label };
        for (i, y) in self.list.layout(g, self.teams.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let team = &self.teams[i];
            let avg = self.form.metric_text(m, team.avg(self.metric));
            let value = format!("{}{avg}", arrow(team, self.metric, self.top));
            let name = format!("{}", team.team);
            g.row(r, &name, &value, t.ink, i == self.list.focus);
            // The team's line over its matches, between its number and its average.
            let name_w = g.measure(&name, if bezel { ty.label } else { ty.body });
            let value_w = g.measure("↑000.0", value_style);
            let x = r.x + 10.0 + name_w.max(g.measure("00000", if bezel { ty.label } else { ty.body })) + 14.0;
            let right = r.right() - 10.0 - value_w - 12.0;
            spark(g, Rect::new(x, r.y + 5.0, (right - x).max(20.0), r.h - 10.0), &team.values(self.metric), self.top);
        }
        self.list.scrollbar(g, self.teams.len(), list_top, bottom);
        g.hint_any(&["enter: the team · ←→: sort by another", "enter: team · ←→: sort", "←→: sort"]);
    }
}

// --------------------------------------------------------------- one team ---

#[derive(Clone)]
enum Line {
    Metric(usize),
    Note(String, String),
}

pub struct TeamPage {
    event: String,
    team: u32,
    form: Arc<Form>,
    stats: Option<Team>,
    /// Each metric's highest at the event, for the lines' scale.
    tops: Vec<f32>,
    lines: Vec<Line>,
    pick: Option<(Option<usize>, bool)>,
    list: ListState,
    title: String,
}

impl TeamPage {
    pub fn new(event: String, team: u32) -> Self {
        let form = super::form();
        let mut page = TeamPage { event, team, form, stats: None, tops: Vec::new(), lines: Vec::new(), pick: None, list: ListState::default(), title: format!("{team}") };
        page.reload();
        page
    }

    fn reload(&mut self) {
        let (records, teams) = load(&self.event);
        let metrics = self.form.metrics().len();
        self.tops = (0..metrics).map(|m| stats::scale(&teams, m)).collect();
        self.stats = teams.into_iter().find(|t| t.team == self.team);
        self.lines = (0..metrics).map(Line::Metric).collect();
        let list = Picklist::load(&super::dir(), &self.event);
        // The picklist's note on the team, then the scouts', match by match.
        if let Some(p) = list.picks.iter().find(|p| p.team == self.team && !p.note.is_empty()) {
            self.lines.push(Line::Note("picklist".into(), p.note.clone()));
        }
        let text_cols: Vec<String> = self.form.fields.iter().filter(|f| matches!(f.kind, super::form::Kind::Text(_))).map(|f| f.column(&self.form)).collect();
        for r in records.iter().filter(|r| r.team == self.team) {
            for c in &text_cols {
                if let Some(words) = r.texts.get(c).filter(|w| !w.trim().is_empty()) {
                    self.lines.push(Line::Note(r.label.clone(), words.clone()));
                }
            }
        }
        self.pick = list.picks.iter().position(|p| p.team == self.team).map(|at| (list.rank(at), list.picks[at].dnp));
    }
}

impl Screen for TeamPage {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn resume(&mut self) {
        self.reload();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('x') => {
                let root = super::dir();
                let mut list = Picklist::load(&root, &self.event);
                if list.picks.iter().all(|p| p.team != self.team) {
                    list.add_missing(&[self.team]);
                }
                if let Some(at) = list.picks.iter().position(|p| p.team == self.team) {
                    let to = list.toggle_dnp(at);
                    *cx.say = Some((if list.picks[to].dnp { "marked: do not pick" } else { "back on the picklist" }).into());
                    list.save(&root).ok();
                }
                self.reload();
            }
            Key::Enter => {
                if let Some(Line::Note(label, words)) = self.lines.get(self.list.focus) {
                    return Nav::Push(Box::new(NotePage { title: format!("{} · {label}", self.team), words: words.clone() }));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.lines.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = g.above_hint();
        let Some(stats) = &self.stats else {
            g.empty("groups", "no matches scouted for this team yet");
            g.hint("esc: back");
            return;
        };
        // How many matches, and where it is on the picklist.
        let head_h = ty.caption.size + 6.0;
        let cy = top + head_h * 0.5 - 1.0;
        let n = stats.matches.len();
        g.text_at(PAD + 4.0, cy, Align::Left, &format!("{n} {}", if n == 1 { "match" } else { "matches" }), ty.caption, t.dim);
        let (pick_words, pick_ink): (String, Color) = match self.pick {
            Some((_, true)) => ("do not pick".into(), g.fault()),
            Some((Some(rank), false)) => (format!("pick {rank}"), t.ink),
            _ => (String::new(), t.dim),
        };
        if !pick_words.is_empty() {
            let w = g.text_at(W - PAD - 6.0, cy, Align::Right, &pick_words, ty.caption, pick_ink);
            if matches!(self.pick, Some((_, true))) {
                g.icon("block", false, W - PAD - 14.0 - w, cy, 12.0, pick_ink);
            }
        }
        let list_top = top + head_h + 2.0;
        let row_h = g.row_h();
        let metrics = self.form.metrics();
        let bezel = g.theme == crate::look::Theme::Bezel;
        let (name_style, value_style) = if bezel { (ty.label, ty.body) } else { (ty.body, ty.label) };
        for (i, y) in self.list.layout(g, self.lines.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match &self.lines[i] {
                Line::Metric(k) => {
                    let m = &metrics[*k];
                    let label = self.form.metric_label(m);
                    let top_v = self.tops.get(*k).copied().unwrap_or(0.0);
                    let value = format!("{}{}", arrow(stats, *k, top_v), self.form.metric_text(m, stats.avg(*k)));
                    g.row(r, &label, &value, t.ink, focused);
                    let shown = if bezel { label.to_lowercase() } else { label.clone() };
                    let x = r.x + 10.0 + g.measure(&shown, name_style) + 12.0;
                    let right = r.right() - 10.0 - g.measure("↑000.0", value_style) - 8.0;
                    let w = (right - x).min(84.0);
                    if w >= 24.0 {
                        spark(g, Rect::new(right - w, r.y + 5.0, w, r.h - 10.0), &stats.values(*k), top_v);
                    }
                }
                Line::Note(label, words) => {
                    let room = r.w - g.measure(label, name_style) - 34.0;
                    let preview = g.text.fit(words, &value_style, room);
                    g.row(r, label, &preview, t.dim, focused);
                }
            }
        }
        self.list.scrollbar(g, self.lines.len(), list_top, bottom);
        let note = matches!(self.lines.get(self.list.focus), Some(Line::Note(..)));
        g.hint_any(if note { &["enter: read it all · x: do not pick", "enter: read · x: do not pick", "enter: read"] } else { &["x: do not pick, or pick again", "x: do not pick"] });
    }
}

/// A scout's notes, whole.
struct NotePage {
    title: String,
    words: String,
}

impl Screen for NotePage {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Enter | Key::Left => Nav::Pop,
            _ => Nav::Stay,
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let card = Rect::new(PAD, top, W - 2.0 * PAD, crate::gfx::H - top - PAD);
        g.surface(card, g.tile_radius(), t.surface1);
        let lines = crate::help::wrap(g, &self.words, ty.body, card.w - 20.0);
        let line_h = ty.body.size + 5.0;
        for (k, line) in lines.iter().enumerate() {
            let cy = card.y + 10.0 + line_h * (k as f32 + 0.5);
            if cy + line_h * 0.5 > card.bottom() - 4.0 {
                break;
            }
            g.text_at(card.x + 10.0, cy, Align::Left, line, ty.body, t.ink);
        }
    }
}

// --------------------------------------------------------------- picklist ---

pub struct PicklistScreen {
    event: String,
    form: Arc<Form>,
    list: Picklist,
    teams: Vec<Team>,
    rows: ListState,
    note: Option<Field>,
}

impl PicklistScreen {
    pub fn new(event: String) -> Self {
        let form = super::form();
        let mut me = PicklistScreen { list: Picklist::load(&super::dir(), &event), event, form, teams: Vec::new(), rows: ListState::default(), note: None };
        me.reload();
        me
    }

    /// Teams scouted since join the list, best by points first.
    fn reload(&mut self) {
        let (_, mut teams) = load(&self.event);
        stats::sort_by(&self.form, &mut teams, 0);
        let ranked: Vec<u32> = teams.iter().map(|t| t.team).collect();
        if self.list.add_missing(&ranked) {
            self.list.save(&super::dir()).ok();
        }
        self.teams = teams;
    }

    fn save(&self) {
        self.list.save(&super::dir()).ok();
    }
}

impl Screen for PicklistScreen {
    fn title(&self) -> &str {
        "Picklist"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn wants_text(&self) -> bool {
        self.note.is_some()
    }

    fn resume(&mut self) {
        self.list = Picklist::load(&super::dir(), &self.event);
        self.reload();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.note {
            match field.key(key, |c| !c.is_control()) {
                Typed::More => {}
                Typed::Cancelled => self.note = None,
                Typed::Done(text) => {
                    self.note = None;
                    if let Some(p) = self.list.picks.get_mut(self.rows.focus) {
                        p.note = text;
                        self.save();
                    }
                }
            }
            return Nav::Stay;
        }
        let at = self.rows.focus;
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('u') => {
                self.rows.focus = self.list.shift(at, -1);
                self.save();
            }
            Key::Char('d') => {
                self.rows.focus = self.list.shift(at, 1);
                self.save();
            }
            Key::Char('t') => {
                self.rows.focus = self.list.to_top(at);
                self.save();
            }
            Key::Char('x') if at < self.list.picks.len() => {
                self.rows.focus = self.list.toggle_dnp(at);
                let p = &self.list.picks[self.rows.focus];
                *cx.say = Some(if p.dnp { format!("{}: do not pick", p.team) } else { format!("{} back on the list", p.team) });
                self.save();
            }
            Key::Char('n') if at < self.list.picks.len() => {
                self.note = Some(Field::new("a note on this team", &self.list.picks[at].note, 60));
            }
            Key::Enter => {
                if let Some(p) = self.list.picks.get(at) {
                    return Nav::Push(Box::new(TeamPage::new(self.event.clone(), p.team)));
                }
            }
            other => crate::screens::list_key(&mut self.rows, other, self.list.picks.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.note {
            field.draw(g, "enter: keep · esc: cancel");
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        if self.list.picks.is_empty() {
            g.empty("leaderboard", "teams join the list once matches are scouted");
            g.hint("esc: back");
            return;
        }
        let row_h = g.row_h();
        let bezel = g.theme == crate::look::Theme::Bezel;
        let (name_style, value_style) = if bezel { (ty.label, ty.body) } else { (ty.body, ty.label) };
        let points = self.form.metrics().first().cloned();
        let rank_w = g.measure("00", name_style);
        let len = self.list.picks.len();
        for (i, y) in self.rows.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let p = &self.list.picks[i];
            let avg = self.teams.iter().find(|x| x.team == p.team).and_then(|x| x.avg(0));
            let value = points.as_ref().map_or_else(String::new, |m| if avg.is_some() { format!("{} pts", self.form.metric_text(m, avg)) } else { String::new() });
            let value = if p.dnp { "do not pick".to_owned() } else { value };
            // The rank (or the mark), then the team, drawn over the row.
            g.row(r, "", &value, if p.dnp { g.fault() } else { t.dim }, i == self.rows.focus);
            match self.list.rank(i) {
                Some(rank) => {
                    g.text_at(r.x + 10.0 + rank_w, r.cy(), Align::Right, &rank.to_string(), name_style, t.dim);
                }
                None => g.icon("block", false, r.x + 10.0 + rank_w * 0.5, r.cy(), 13.0, g.fault()),
            }
            let team = p.team.to_string();
            let team_w = g.text_at(r.x + 20.0 + rank_w, r.cy(), Align::Left, &team, name_style, if p.dnp { t.dim } else { t.ink });
            if !p.note.is_empty() {
                let x = r.x + 20.0 + rank_w + team_w + 12.0;
                let right = r.right() - 10.0 - g.measure(&value, value_style) - 12.0;
                if right - x > 30.0 {
                    let words = g.text.fit(&p.note, &ty.caption, right - x);
                    g.text_at(x, r.cy(), Align::Left, &words, ty.caption, t.dim);
                }
            }
        }
        self.rows.scrollbar(g, len, top, bottom);
        g.hint_any(&["u d: move · t: top · x: do not pick · n: note", "u d: move · t: top · x: no pick", "u d: move · x: no pick"]);
    }
}
