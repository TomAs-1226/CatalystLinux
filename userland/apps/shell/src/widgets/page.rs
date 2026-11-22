//! Settings › Home › Widgets: where each board stands, a way into editing
//! it (back on Home, in edit mode), and a way back to the start.

use bezel::Rect;

use super::{Board, Layout};
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, HomeStyle, Nav, Screen};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Glance,
    Home,
    StartOver,
}

const ROWS: [Row; 3] = [Row::Glance, Row::Home, Row::StartOver];

pub struct WidgetsPage {
    list: ListState,
    /// Enter pressed once on Start over: again puts the boards back.
    armed: bool,
}

impl WidgetsPage {
    pub fn new() -> Self {
        WidgetsPage { list: ListState::default(), armed: false }
    }
}

fn count(n: usize) -> String {
    match n {
        0 => "none".into(),
        1 => "1 widget".into(),
        n => format!("{n} widgets"),
    }
}

impl Screen for WidgetsPage {
    fn title(&self) -> &str {
        "Widgets"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        let armed = std::mem::take(&mut self.armed);
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right => match row {
                Row::Glance => {
                    cx.settings.edit_widgets = Some(Board::Glance);
                    return Nav::Home;
                }
                Row::Home if cx.settings.home != HomeStyle::Grid => {
                    *cx.say = Some("widgets sit among the apps in the grid style".into());
                }
                Row::Home => {
                    cx.settings.edit_widgets = Some(Board::Home);
                    return Nav::Home;
                }
                Row::StartOver if armed => {
                    cx.settings.widgets = Layout::starting();
                    crate::save_settings(cx.settings);
                    *cx.say = Some("widgets as they started".into());
                }
                Row::StartOver => {
                    self.armed = true;
                    *cx.say = Some("enter again starts over".into());
                }
            },
            other => crate::screens::list_key(&mut self.list, other, ROWS.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h) = (g.top(), g.row_h());
        let bottom = g.above_hint();
        let grid = g.home == HomeStyle::Grid;
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            let (name, value, on) = match ROWS[i] {
                Row::Glance => ("Glance page", format!("{} · edit ›", count(g.widgets.glance.len())), true),
                Row::Home if grid => ("Home's grid", format!("{} · edit ›", count(g.widgets.home.len())), true),
                Row::Home => ("Home's grid", "grid style only".to_owned(), false),
                Row::StartOver => ("Start over", (if self.armed { "enter again" } else { "as new ›" }).to_owned(), self.armed),
            };
            g.row(r, name, &value, if on { t.ink } else { t.dim }, focused);
        }
        self.list.scrollbar(g, ROWS.len(), top, bottom - 2.0);
        g.hint_any(&["e on home or the glance page edits them too", "e on home edits them too"]);
    }
}
