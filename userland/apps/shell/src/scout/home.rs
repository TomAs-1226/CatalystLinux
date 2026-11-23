//! Scout's first page: the next match to scout, large, beside the rest of
//! the app; and the event's matches to choose another from.

use bezel::{Align, Rect};

use super::event::Event;
use super::store::Station;
use super::Me;
use crate::gfx::{Gfx, ListState, GAP, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::schedule::{Match, BLUE, RED};
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// The rows beside the next match.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Matches,
    Teams,
    Picklist,
    Share,
    Practice,
    Station,
    Name,
}

const ROWS: [Row; 7] = [Row::Matches, Row::Teams, Row::Picklist, Row::Share, Row::Practice, Row::Station, Row::Name];

/// A station's alliance colour.
pub fn station_ink(s: Station) -> bezel::Color {
    if s.red() { RED } else { BLUE }
}

/// "in 4 min", "now", or the score of a match played.
fn when(m: &Match, now: i64) -> String {
    match (m.score, m.time) {
        (Some((r, b)), _) if r >= 0 && b >= 0 => format!("{r}–{b}"),
        (_, Some(t)) => crate::schedule::when(t, now),
        _ => String::new(),
    }
}

pub struct Scout {
    me: Me,
    event: Option<Event>,
    generation: u64,
    on_list: bool,
    list: ListState,
    naming: Option<Field>,
    /// What the rows say, worked out when the page opens or comes back.
    counts: Counts,
}

#[derive(Default)]
struct Counts {
    records: usize,
    teams: usize,
    picks: usize,
}

impl Default for Scout {
    fn default() -> Self {
        Self::new()
    }
}

impl Scout {
    pub fn new() -> Self {
        Scout { me: Me::load(), event: None, generation: u64::MAX, on_list: false, list: ListState::default(), naming: None, counts: Counts::default() }
    }

    fn refresh(&mut self, robot: &Robot) {
        self.generation = super::event::generation();
        self.event = super::event::current(robot);
        let event = self.event.as_ref().map(|e| e.key.clone()).unwrap_or_default();
        let (records, teams) = super::with_store(|s| {
            let mut teams: Vec<u32> = s.of_event(&event).map(|r| r.team).collect();
            teams.sort_unstable();
            teams.dedup();
            (s.of_event(&event).count(), teams.len())
        });
        let picks = super::stats::Picklist::load(&super::dir(), &event).picks.len();
        self.counts = Counts { records, teams, picks };
    }

    /// The next match for this scout's station: its place in the list.
    fn next(&self) -> Option<usize> {
        self.event.as_ref()?.next(super::now())
    }

    fn scout_next(&self) -> Option<Nav> {
        let e = self.event.as_ref()?;
        let m = &e.matches[self.next()?];
        let team = self.me.station.team_in(m)?;
        Some(Nav::Push(Box::new(super::scouting::Scouting::new(&e.key, &m.key, &m.label, team, self.me.station))))
    }
}

impl Screen for Scout {
    fn title(&self) -> &str {
        "Scout"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.naming.is_some() {
            return vec![keys("your name", &[("enter", "Keep it"), ("esc", "Leave it as it was")])];
        }
        let first = if self.on_list { keys("the list", &[("enter", "Open it"), ("←", "The next match")]) } else { keys("next match", &[("enter", "Scout it"), ("→ · ↓", "The list")]) };
        vec![first, keys("scout", &[("s", "Next place"), ("shift s", "Last place")])]
    }

    fn resume(&mut self) {
        self.generation = u64::MAX;
    }

    fn ambient_tick(&mut self, _dt: f32, robot: &Robot) -> bool {
        if self.generation != super::event::generation() {
            self.refresh(robot);
            return true;
        }
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.generation == u64::MAX {
            self.refresh(cx.robot);
        }
        if let Some(field) = &mut self.naming {
            match field.key(key, |c| c.is_ascii_alphanumeric() || " -_.".contains(c)) {
                Typed::More => {}
                Typed::Cancelled => self.naming = None,
                Typed::Done(name) => {
                    self.naming = None;
                    if !name.is_empty() {
                        self.me.name = name;
                        self.me.save();
                    }
                }
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Left => self.on_list = false,
            Key::Right => self.on_list = true,
            Key::Char(c @ ('s' | 'S')) => {
                self.me.station = self.me.station.next(if c == 's' { 1 } else { -1 });
                self.me.save();
                *cx.say = Some(format!("scouting {}", self.me.station.label()));
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End if self.on_list => crate::screens::list_key(&mut self.list, key, ROWS.len()),
            Key::Down if !self.on_list => self.on_list = true,
            Key::Enter if !self.on_list => {
                return match self.scout_next() {
                    Some(nav) => nav,
                    None if self.event.is_some() => Nav::Push(Box::new(Matches::new(self.event.clone(), self.me.station))),
                    None => Nav::Push(Box::new(ByHand::new(self.me.station))),
                };
            }
            Key::Enter => match ROWS[self.list.focus.min(ROWS.len() - 1)] {
                Row::Matches => return Nav::Push(Box::new(Matches::new(self.event.clone(), self.me.station))),
                Row::Teams => return Nav::Push(Box::new(super::teams::Teams::new(self.event_key()))),
                Row::Picklist => return Nav::Push(Box::new(super::teams::PicklistScreen::new(self.event_key()))),
                Row::Share => return Nav::Push(Box::new(super::sharing::ShareMenu::new(self.event_key(), self.event.as_ref().and_then(|e| e.next(super::now()))))),
                Row::Practice => return Nav::Push(Box::new(super::practice::Practice::new())),
                Row::Station => {
                    self.me.station = self.me.station.next(1);
                    self.me.save();
                }
                Row::Name => self.naming = Some(Field::new("your name, on what you scout", &self.me.name, 20)),
            },
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        if self.generation == u64::MAX {
            self.refresh(robot);
        }
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.naming {
            field.draw(g, "enter: keep · esc: cancel");
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        let radius = g.tile_radius();
        let card_w = 132.0;
        let card = Rect::new(PAD, top, card_w, bottom - top);
        g.surface(card, radius, if self.on_list { t.surface1 } else { t.surface2 });
        let cap_cy = card.y + 9.0 + ty.caption.size * 0.5;
        let now = super::now();
        let next = self.next().and_then(|i| self.event.as_ref().map(|e| &e.matches[i]));
        match next {
            Some(m) => {
                g.text_at(card.x + 10.0, cap_cy, Align::Left, "next match", ty.caption, t.dim);
                let team = self.me.station.team_in(m);
                let big = ty.display(32.0);
                let baseline = cap_cy + ty.caption.size * 0.5 + 6.0 + big.size * 0.74;
                g.text(card.x + 9.0, baseline, &team.map_or("–".into(), |n| n.to_string()), big, t.ink);
                // The match and the place, the place's colour on a dot.
                let line_cy = baseline + 8.0 + ty.label.size * 0.5;
                let label_w = g.text_at(card.x + 10.0, line_cy, Align::Left, &m.label, ty.label, t.ink);
                let dot_x = card.x + 10.0 + label_w + 10.0;
                g.dot(dot_x, line_cy, 3.5, station_ink(self.me.station));
                g.text_at(dot_x + 7.0, line_cy, Align::Left, &self.me.station.label(), ty.label, t.dim);
                let done = team.is_some_and(|n| super::with_store(|s| s.get(&m.key, n).is_some()));
                let foot_cy = card.bottom() - 9.0 - ty.caption.size * 0.5;
                // Scouted already, or when it plays: under way once its time has come.
                let soon = match m.time {
                    Some(at) if at <= now => "on now".to_owned(),
                    _ => when(m, now),
                };
                let (icon, words, ink) = if done { ("check_circle", "scouted".to_owned(), g.good()) } else { ("schedule", soon, t.dim) };
                g.icon(icon, done, card.x + 17.0, foot_cy, 14.0, ink);
                g.text_fit(card.x + 29.0, foot_cy, &words, ty.caption, ink, card.w - 38.0);
            }
            None => {
                let (head, line) = match &self.event {
                    Some(e) if !e.matches.is_empty() => ("all played", "enter: the matches"),
                    _ => ("no schedule", "enter: a match by hand"),
                };
                g.text_at(card.x + 10.0, cap_cy, Align::Left, "next match", ty.caption, t.dim);
                g.icon("event", false, card.cx(), card.cy() - 6.0, 24.0, t.faint);
                g.text_at(card.cx(), card.cy() + 16.0, Align::Center, head, ty.label, t.ink);
                g.text_fit(card.x + 10.0, card.bottom() - 9.0 - ty.caption.size * 0.5, line, ty.caption, t.dim, card.w - 20.0);
            }
        }
        if !self.on_list {
            g.focus_ring(card, radius);
        }

        // The rest of the app, beside it.
        let x = card.right() + GAP;
        let row_w = W - PAD - x - 5.0;
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(x, y, row_w, row_h);
            let c = &self.counts;
            let (name, value) = match ROWS[i] {
                Row::Matches => ("Matches", self.event.as_ref().map_or_else(String::new, |e| format!("{}", e.matches.len()))),
                Row::Teams => ("Teams", if c.teams == 0 { String::new() } else { format!("{}", c.teams) }),
                Row::Picklist => ("Picklist", if c.picks == 0 { String::new() } else { format!("{}", c.picks) }),
                Row::Share => ("Share", if c.records == 0 { String::new() } else { "QR · CSV".into() }),
                Row::Practice => ("Practice", String::new()),
                Row::Station => ("Place", self.me.station.label()),
                Row::Name => ("Scout", self.me.name.clone()),
            };
            let value = g.text.fit(&value, &ty.label, row_w * 0.5);
            g.row(r, name, &value, t.dim, self.on_list && i == self.list.focus);
            if ROWS[i] == Row::Station {
                let vw = g.measure(&value, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label });
                g.dot(r.right() - 10.0 - vw - 8.0, r.cy(), 3.5, station_ink(self.me.station));
            }
        }
        self.list.scrollbar(g, ROWS.len(), top, bottom);
        g.hint_any(&["enter: open · s: next place · ←→: choose", "enter: open · s: next place", "enter: open"]);
    }
}

impl Scout {
    fn event_key(&self) -> String {
        self.event.as_ref().map(|e| e.key.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- matches ---

/// The event's matches, the team at a place in each; the next one first.
pub struct Matches {
    event: Option<Event>,
    station: Station,
    list: ListState,
    /// For a station, whether each match's team there is scouted.
    done: Option<(Station, Vec<bool>)>,
}

impl Matches {
    pub fn new(event: Option<Event>, station: Station) -> Self {
        let mut list = ListState::default();
        list.focus = event.as_ref().and_then(|e| e.next(super::now())).unwrap_or(0);
        Matches { event, station, list, done: None }
    }
}

impl Screen for Matches {
    fn title(&self) -> &str {
        "Matches"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    /// Back from scouting one: it may be scouted now.
    fn resume(&mut self) {
        self.done = None;
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("matches", &[("enter", "Scout it"), ("← →", "Another place"), ("n", "A match by hand")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let len = self.event.as_ref().map_or(0, |e| e.matches.len());
        match key {
            Key::Back => return Nav::Pop,
            Key::Left => self.station = self.station.next(-1),
            Key::Right => self.station = self.station.next(1),
            Key::Char('n') => return Nav::Push(Box::new(ByHand::new(self.station))),
            Key::Enter => {
                if let Some(e) = &self.event {
                    if let Some(m) = e.matches.get(self.list.focus) {
                        if let Some(team) = self.station.team_in(m) {
                            return Nav::Push(Box::new(super::scouting::Scouting::new(&e.key, &m.key, &m.label, team, self.station)));
                        }
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, len),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = g.above_hint();
        let Some(e) = &self.event else {
            g.empty("event", "no schedule yet · n: a match by hand");
            g.hint("n: a match by hand");
            return;
        };
        // The place being shown, above the list.
        let head_h = ty.caption.size + 6.0;
        let cy = top + head_h * 0.5 - 1.0;
        g.dot(PAD + 6.0, cy, 3.5, station_ink(self.station));
        g.text_at(PAD + 14.0, cy, Align::Left, &self.station.label(), ty.caption, t.ink);
        let name = g.text.fit(&e.name, &ty.caption, W * 0.6);
        g.text_at(W - PAD - 6.0, cy, Align::Right, &name, ty.caption, t.dim);
        let list_top = top + head_h + 2.0;
        let row_h = g.row_h();
        let now = super::now();
        let len = e.matches.len();
        let next = e.next(now);
        // Which of the place's teams are scouted, looked up once a station.
        if self.done.as_ref().is_none_or(|(s, _)| *s != self.station) {
            let station = self.station;
            let done = super::with_store(|s| e.matches.iter().map(|m| station.team_in(m).is_some_and(|n| s.get(&m.key, n).is_some())).collect());
            self.done = Some((station, done));
        }
        let done = self.done.as_ref().map(|(_, d)| d.clone()).unwrap_or_default();
        let rows: Vec<(String, String, bool, bool)> = e
            .matches
            .iter()
            .zip(done)
            .map(|(m, done)| {
                let team = self.station.team_in(m);
                (m.label.clone(), team.map_or("–".into(), |n| n.to_string()), done, m.score.is_some())
            })
            .collect();
        for (i, y) in self.list.layout(g, len, list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (label, team, done, played) = &rows[i];
            let soon = when(&e.matches[i], now);
            let name = if Some(i) == next { format!("{label}  ·  next") } else { format!("{label}  ·  {soon}") };
            g.row(r, &name, team, if *played && !done { t.faint } else { t.ink }, i == self.list.focus);
            if *done {
                let tw = g.measure(team, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label });
                g.icon("check_circle", true, r.right() - 10.0 - tw - 12.0, r.cy(), 13.0, g.good());
            }
        }
        self.list.scrollbar(g, len, list_top, bottom);
        g.hint_any(&["enter: scout · ←→: another place · n: by hand", "enter: scout · ←→: place", "enter: scout"]);
    }
}

// ---------------------------------------------------------------- by hand ---

/// A match not on the schedule (none came, or a replay): its number, then
/// the team, typed.
pub struct ByHand {
    station: Station,
    number: Option<u32>,
    field: Field,
}

impl ByHand {
    pub fn new(station: Station) -> Self {
        ByHand { station, number: None, field: Field::new("qualification match number", "", 3) }
    }
}

impl Screen for ByHand {
    fn title(&self) -> &str {
        "By hand"
    }

    fn app(&self) -> &str {
        "Scout"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let rows: &'static [(&str, &str)] = if self.number.is_none() { &[("enter", "On to the team"), ("← →", "Another place")] } else { &[("enter", "Scout it"), ("← →", "Another place")] };
        vec![keys("by hand", rows)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if matches!(key, Key::Left | Key::Right) {
            self.station = self.station.next(if key == Key::Left { -1 } else { 1 });
            return Nav::Stay;
        }
        match self.field.key(key, |c| c.is_ascii_digit()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) => match (self.number, text.parse::<u32>().ok().filter(|n| *n > 0)) {
                (_, None) => {
                    *cx.say = Some("a number, please".into());
                    Nav::Stay
                }
                (None, Some(n)) => {
                    self.number = Some(n);
                    self.field = Field::new("team number", "", 5);
                    Nav::Stay
                }
                (Some(m), Some(team)) => {
                    let event = super::event::current(cx.robot).map(|e| e.key).filter(|k| !k.is_empty()).unwrap_or_else(|| format!("{}local", chrono::Local::now().format("%Y")));
                    let key = format!("{event}_qm{m}");
                    Nav::Push(Box::new(super::scouting::Scouting::new(&event, &key, &format!("Q{m}"), team, self.station)))
                }
            },
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.field.draw_at(g, true);
        let y = g.top() + Field::height(g) + 14.0;
        let cy = y + ty.label.size * 0.5;
        g.dot(PAD + 8.0, cy, 3.5, station_ink(self.station));
        g.text_at(PAD + 16.0, cy, Align::Left, &self.station.label(), ty.label, t.ink);
        if let Some(n) = self.number {
            g.text_at(W - PAD - 4.0, cy, Align::Right, &format!("Q{n}"), ty.label, t.dim);
        }
        // What this is for, under the place.
        let words = "For a match the schedule does not have yet, or one played again. It is kept with the event's other records.";
        let line_h = ty.caption.size + 4.0;
        let mut y = cy + ty.label.size * 0.5 + 10.0 + ty.caption.size * 0.5;
        for line in crate::help::wrap(g, words, ty.caption, W - 2.0 * PAD - 8.0) {
            if y + ty.caption.size * 0.5 > g.above_hint() {
                break;
            }
            g.text_at(PAD + 4.0, y, Align::Left, &line, ty.caption, t.dim);
            y += line_h;
        }
        g.hint(if self.number.is_none() { "enter: on to the team · ←→: place" } else { "enter: scout it · ←→: place" });
    }
}
