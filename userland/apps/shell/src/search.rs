//! Search: Space (or /) on Home, then type. One list finds apps, the
//! handheld's own apps, everything in Settings, any value the robot
//! publishes, and match logs. Enter opens what is focused; Esc goes back.
//!
//! Matches are ranked the way a person reads a name: the start of it
//! first, then the start of a word in it, then anywhere in it.

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// What a result opens.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    App(usize),
    External(String),
    Setting(usize),
    Table(String),
    Log(String),
}

#[derive(Clone, Debug)]
struct Hit {
    icon: String,
    name: String,
    /// What kind of thing it is, shown at the right.
    kind: &'static str,
    /// A value to show for it, for robot values.
    value: Option<String>,
    target: Target,
    rank: (u8, u8, usize),
}

/// How well `name` matches `query` (both compared in lower case): 0 when it
/// starts with it, 1 when a word in it does, 2 anywhere; `None` if not at all.
pub fn rank(name: &str, query: &str) -> Option<u8> {
    let (name, query) = (name.to_lowercase(), query.trim().to_lowercase());
    if query.is_empty() {
        return None;
    }
    if name.starts_with(&query) {
        return Some(0);
    }
    let words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    if words.clone().any(|w| w.starts_with(&query)) {
        return Some(1);
    }
    name.contains(&query).then_some(2)
}

pub struct Search {
    query: String,
    list: ListState,
    hits: Vec<Hit>,
    /// The robot's data generation the results were found at.
    seen: u64,
}

impl Search {
    pub fn new(first: Option<char>) -> Self {
        Search { query: first.map(String::from).unwrap_or_default(), list: ListState::default(), hits: Vec::new(), seen: u64::MAX }
    }

    fn find(&mut self, robot: &Robot) {
        let q = self.query.clone();
        let mut hits = Vec::new();
        let mut add = |icon: &str, name: String, kind: &'static str, order: u8, value: Option<String>, target: Target| {
            if let Some(r) = rank(&name, &q) {
                let len = name.len();
                hits.push(Hit { icon: icon.to_owned(), name, kind, value, target, rank: (r, order, len) });
            }
        };
        for (i, icon, name) in crate::screens::builtin_apps() {
            add(icon, name.to_owned(), "app", 0, None, Target::App(i));
        }
        for app in crate::apps::find() {
            add(&app.icon, app.name.clone(), "app", 0, None, Target::External(app.name.clone()));
        }
        for (i, name) in crate::screens::setting_names() {
            add("settings", name.to_owned(), "setting", 1, None, Target::Setting(i));
        }
        for row in robot.logs.list() {
            add("description", row.title(), "match", 2, row.digest.as_ref().map(|d| d.headline().1), Target::Log(row.name.clone()));
        }
        // Robot values by their last part or their whole path.
        for (key, value) in robot.under("") {
            let leaf = key.rsplit('/').next().unwrap_or(&key).to_owned();
            if let Some(r) = rank(&leaf, &q).or_else(|| rank(&key, &q).map(|r| r.max(2))) {
                let len = key.len();
                hits.push(Hit { icon: "account_tree".into(), name: key.clone(), kind: "value", value: Some(value.display()), target: Target::Table(key), rank: (r, 3, len) });
            }
        }
        hits.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.name.cmp(&b.name)));
        hits.truncate(40);
        self.hits = hits;
        self.list.focus = self.list.focus.min(self.hits.len().saturating_sub(1));
    }

    fn open(&self, hit: &Hit, cx: &mut Cx) -> Nav {
        match &hit.target {
            Target::App(i) => Nav::Push(crate::screens::open_builtin(*i, cx.robot)),
            Target::External(name) => match crate::apps::find().into_iter().find(|a| &a.name == name) {
                Some(app) => Nav::Push(app.open()),
                None => Nav::Stay,
            },
            Target::Setting(i) => Nav::Push(crate::screens::open_setting(*i)),
            Target::Table(key) => Nav::Push(Box::new(crate::screens::Tables::at(key, cx.robot))),
            Target::Log(name) => {
                let rows = cx.robot.logs.list();
                match rows.iter().find(|r| &r.name == name).and_then(crate::screens::LogDetail::of) {
                    Some(detail) => Nav::Push(Box::new(detail)),
                    None => {
                        *cx.say = Some("not read yet".into());
                        Nav::Stay
                    }
                }
            }
        }
    }
}

impl Screen for Search {
    fn title(&self) -> &str {
        "Search"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let changed = match key {
            Key::Back if self.query.is_empty() => return Nav::Pop,
            // Esc clears what was typed first, then leaves.
            Key::Back => {
                self.query.clear();
                true
            }
            Key::Char(c) if !c.is_control() && self.query.chars().count() < 40 => {
                self.query.push(c);
                true
            }
            Key::Backspace => self.query.pop().is_some(),
            Key::Enter => {
                return match self.hits.get(self.list.focus).cloned() {
                    Some(hit) => self.open(&hit, cx),
                    None => Nav::Stay,
                }
            }
            Key::Ctrl('q') | Key::Leave => return Nav::Pop,
            other => {
                crate::screens::list_key(&mut self.list, other, self.hits.len());
                false
            }
        };
        if changed {
            self.list = ListState::default();
            self.find(cx.robot);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        // Robot values change under the results: find them again when they do.
        let generation = robot.logs.generation() ^ robot.loop_history.len() as u64;
        if self.seen != generation {
            self.seen = generation;
            self.find(robot);
        }
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        // The field: what has been typed, and the caret.
        let field = Rect::new(PAD, top, W - 2.0 * PAD, ty.body.size + 12.0);
        g.surface(field, field.h * 0.5, t.surface2);
        g.icon("search", false, field.x + 13.0, field.cy(), 14.0, t.dim);
        let shown = if self.query.is_empty() { "apps, settings, robot values, matches" } else { &self.query };
        let w = g.text_at(field.x + 25.0, field.cy(), Align::Left, shown, ty.body, if self.query.is_empty() { t.faint } else { t.ink });
        let caret_x = if self.query.is_empty() { field.x + 25.0 } else { field.x + 26.0 + w };
        g.surface(Rect::new(caret_x, field.cy() - ty.body.size * 0.5, 1.5, ty.body.size), 0.75, t.signal);
        let list_top = field.bottom() + 6.0;
        let bottom = H - PAD;
        if self.query.trim().is_empty() {
            return;
        }
        if self.hits.is_empty() {
            g.empty("search", "nothing matches");
            return;
        }
        let row_h = g.row_h();
        let hits = self.hits.clone();
        for (i, y) in self.list.layout(g, hits.len(), list_top, row_h, 3.0, bottom) {
            let hit = &hits[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, row_h * 0.5, if focused { t.surface2 } else { t.surface1 });
            g.icon(&hit.icon, focused, r.x + 14.0, r.cy(), 14.0, if focused { t.ink } else { t.dim });
            let right = hit.value.clone().unwrap_or_else(|| hit.kind.to_owned());
            let rw = g.text.fit(&right, &ty.label, 110.0);
            let right_w = g.text_at(r.right() - 10.0, r.cy(), Align::Right, &rw, ty.label, t.dim);
            g.text_fit(r.x + 27.0, r.cy(), &hit.name, ty.body, t.ink, r.w - 44.0 - right_w);
            if focused {
                g.focus_ring(r, row_h * 0.5);
            }
        }
        self.list.scrollbar(g, hits.len(), list_top, bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_rank_by_where_the_query_is_found() {
        assert_eq!(rank("Wi-Fi networks", "wi"), Some(0));
        assert_eq!(rank("Wi-Fi networks", "net"), Some(1));
        assert_eq!(rank("Wi-Fi networks", "work"), Some(2));
        assert_eq!(rank("Wi-Fi networks", "zz"), None);
        assert_eq!(rank("Loop/Robot/LastMs", "last"), Some(1));
        assert_eq!(rank("anything", "  "), None);
    }

    #[test]
    fn search_finds_apps_settings_values_and_matches_in_that_order() {
        let mut search = Search::new(None);
        let robot = Robot::demo();
        search.query = "lo".into();
        search.find(&robot);
        let kinds: Vec<&str> = search.hits.iter().map(|h| h.kind).collect();
        assert_eq!(search.hits[0].name, "Logs", "{kinds:?}");
        assert_eq!(search.hits[1].name, "Loop");
        assert!(kinds.contains(&"value"), "robot values are found too: {kinds:?}");
        search.query = "q14".into();
        search.find(&robot);
        assert_eq!(search.hits.first().map(|h| h.kind), Some("match"));
        search.query = "brightness".into();
        search.find(&robot);
        let brightness = crate::screens::setting_names().find(|(_, n)| *n == "Brightness").map(|(i, _)| Target::Setting(i));
        assert_eq!(search.hits.first().map(|h| h.target.clone()), brightness);
    }
}
