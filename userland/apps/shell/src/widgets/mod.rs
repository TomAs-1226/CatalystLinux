//! Widgets: what Home and the glance page show without opening anything.
//!
//! - **A widget** is a kind (the next match, the battery, music…) at a size:
//!   small (one tile), wide (two tiles side by side) or large (two by two;
//!   on the glance page that is the page's whole width). Each kind draws
//!   itself at each size ([`draw`]), says when it next changes by itself (a
//!   minute turning, a countdown's second), reads what it shows through
//!   [`Facts`] on a refresh policy of its own (events, or a timestamp), and
//!   may act on Enter (play or pause, start a timer, tick a reminder off) or
//!   open its app.
//! - **Boards.** The glance page is a board of widgets alone, two cells
//!   across and two down a page, its pages running leftward from Home. Home's
//!   grid is a board too: its apps are one-cell tiles and its widgets sit
//!   among them, each before the app it was placed before. Cells are packed
//!   in reading order, the first free place that fits each in turn, so apps
//!   flow round a widget and fill the holes it would leave ([`pack`]).
//! - **Power.** Nothing is read or drawn while it is not in sight: Home asks
//!   only the widgets on the page showing, and the shell draws nothing with
//!   the screen off. A widget's next change is a time the shell's tick
//!   compares against (never a poll of what it reads), and nothing moves at
//!   rest but a clock's minute and music's progress, a step at a time.
//! - **Editing** (`e` on Home, or Settings › Home › Widgets): see
//!   [`edit`]. The layout is kept in the settings file as two lines.

use std::sync::atomic::{AtomicU32, Ordering};

use bezel::Rect;

use crate::gfx::{Gfx, GAP, H, PAD, W};

mod draw;
pub mod edit;
mod facts;
#[cfg(not(target_arch = "aarch64"))]
pub mod gallery;
mod page;
mod pictures;
#[cfg(test)]
mod tests;

pub use draw::{content, Ctx};
pub use facts::Facts;
pub use page::WidgetsPage;

// ------------------------------------------------------------------ kinds ---

/// What a widget shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    NextMatch,
    Battery,
    Music,
    Weather,
    Robot,
    Reminders,
    Clock,
    World,
    Timers,
    Note,
    System,
    Photos,
    Actions,
    LastMatch,
    Pinned,
}

impl Kind {
    /// In the order the picker offers them.
    pub const ALL: [Kind; 15] = [
        Kind::NextMatch,
        Kind::Battery,
        Kind::Music,
        Kind::Weather,
        Kind::Robot,
        Kind::Reminders,
        Kind::Clock,
        Kind::World,
        Kind::Timers,
        Kind::Note,
        Kind::System,
        Kind::Photos,
        Kind::Actions,
        Kind::LastMatch,
        Kind::Pinned,
    ];

    /// Its name in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Kind::NextMatch => "next_match",
            Kind::Battery => "battery",
            Kind::Music => "music",
            Kind::Weather => "weather",
            Kind::Robot => "robot",
            Kind::Reminders => "reminders",
            Kind::Clock => "clock",
            Kind::World => "world_clock",
            Kind::Timers => "timers",
            Kind::Note => "note",
            Kind::System => "system",
            Kind::Photos => "photos",
            Kind::Actions => "quick_actions",
            Kind::LastMatch => "last_match",
            Kind::Pinned => "pinned",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// What the picker and Settings call it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::NextMatch => "Next match",
            Kind::Battery => "Battery",
            Kind::Music => "Music",
            Kind::Weather => "Weather",
            Kind::Robot => "Robot",
            Kind::Reminders => "Reminders",
            Kind::Clock => "Clock",
            Kind::World => "World clock",
            Kind::Timers => "Timers",
            Kind::Note => "Pinned note",
            Kind::System => "System",
            Kind::Photos => "Photos",
            Kind::Actions => "Quick actions",
            Kind::LastMatch => "Last match",
            Kind::Pinned => "Pinned value",
        }
    }

    /// The symbol at its top right (some change it for what they show).
    pub fn icon(self) -> &'static str {
        match self {
            Kind::NextMatch => "flag",
            Kind::Battery => "battery_full",
            Kind::Music => "music_note",
            Kind::Weather => "partly_cloudy_day",
            Kind::Robot => "smart_toy",
            Kind::Reminders => "checklist",
            Kind::Clock => "schedule",
            Kind::World => "language",
            Kind::Timers => "timer",
            Kind::Note => "sticky_note_2",
            Kind::System => "memory",
            Kind::Photos => "photo",
            Kind::Actions => "search",
            Kind::LastMatch => "emoji_events",
            Kind::Pinned => "push_pin",
        }
    }

    /// The size it is added at.
    pub fn usual(self) -> Size {
        match self {
            Kind::Battery | Kind::Clock | Kind::Timers | Kind::Pinned | Kind::Weather => Size::Small,
            Kind::Photos => Size::Large,
            _ => Size::Wide,
        }
    }

    /// What Enter on it does, for the picker's line.
    pub fn enter_does(self) -> &'static str {
        match self {
            Kind::Music => "enter plays or pauses",
            Kind::Timers => "enter starts or pauses",
            Kind::Reminders => "enter ticks the first off",
            Kind::Actions => "enter searches",
            _ => "enter opens it",
        }
    }
}

/// A widget's size, in cells: small one, wide two across, large two by two.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Size {
    Small,
    Wide,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Wide, Size::Large];

    /// Cells across and down.
    pub fn span(self) -> (usize, usize) {
        match self {
            Size::Small => (1, 1),
            Size::Wide => (2, 1),
            Size::Large => (2, 2),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Wide => "wide",
            Size::Large => "large",
        }
    }

    pub fn from_key(key: &str) -> Option<Size> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The next size round (r in edit mode).
    pub fn next(self) -> Size {
        match self {
            Size::Small => Size::Wide,
            Size::Wide => Size::Large,
            Size::Large => Size::Small,
        }
    }

    /// One size up or down, stopping at the ends.
    pub fn step(self, up: bool) -> Size {
        match (self, up) {
            (Size::Small, true) => Size::Wide,
            (Size::Wide | Size::Large, true) => Size::Large,
            (Size::Large, false) => Size::Wide,
            (Size::Wide | Size::Small, false) => Size::Small,
        }
    }
}

// ----------------------------------------------------------------- layout ---

/// A widget where it is: its kind and size, and on Home how many apps come
/// before it. `id` names it while the shell runs (its springs follow it as
/// it moves); it is not kept in the file.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placed {
    pub kind: Kind,
    pub size: Size,
    pub after: usize,
    pub id: u32,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// A name for a widget, new each time.
pub fn new_id() -> u32 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

impl Placed {
    pub fn new(kind: Kind, size: Size, after: usize) -> Placed {
        Placed { kind, size, after, id: new_id() }
    }
}

/// Which board: the glance page, or Home's grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Board {
    Glance,
    Home,
}

/// Every widget, on each board.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub glance: Vec<Placed>,
    pub home: Vec<Placed>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout::starting()
    }
}

impl Layout {
    /// A new handheld's: on the glance page's first page, the next match
    /// across the top and this battery and the robot under it, what a team
    /// looks at between matches; on the next, what is playing over
    /// reminders and the timers. Home's grid as it was.
    pub fn starting() -> Layout {
        let w = |kind, size| Placed::new(kind, size, 0);
        Layout {
            glance: vec![
                w(Kind::NextMatch, Size::Wide),
                w(Kind::Battery, Size::Small),
                w(Kind::Robot, Size::Small),
                w(Kind::Music, Size::Wide),
                w(Kind::Reminders, Size::Small),
                w(Kind::Timers, Size::Small),
            ],
            home: Vec::new(),
        }
    }

    pub fn board(&self, board: Board) -> &Vec<Placed> {
        match board {
            Board::Glance => &self.glance,
            Board::Home => &self.home,
        }
    }

    pub fn board_mut(&mut self, board: Board) -> &mut Vec<Placed> {
        match board {
            Board::Glance => &mut self.glance,
            Board::Home => &mut self.home,
        }
    }

    /// The two lines the settings file keeps: `widgets=next_match:small,…`
    /// and `home_widgets=3:weather:wide,…` (each after so many apps).
    pub fn lines(&self) -> [(&'static str, String); 2] {
        let glance = self.glance.iter().map(|w| format!("{}:{}", w.kind.key(), w.size.key())).collect::<Vec<_>>().join(",");
        let home = self.home.iter().map(|w| format!("{}:{}:{}", w.after, w.kind.key(), w.size.key())).collect::<Vec<_>>().join(",");
        [("widgets", glance), ("home_widgets", home)]
    }

    /// The glance page's widgets from their line; anything not understood is passed over.
    pub fn parse_glance(text: &str) -> Vec<Placed> {
        text.split(',')
            .filter_map(|item| {
                let (kind, size) = item.trim().split_once(':')?;
                Some(Placed::new(Kind::from_key(kind)?, Size::from_key(size)?, 0))
            })
            .collect()
    }

    /// Home's widgets from their line, in the order they are among the apps.
    pub fn parse_home(text: &str) -> Vec<Placed> {
        let mut list: Vec<Placed> = text
            .split(',')
            .filter_map(|item| {
                let mut parts = item.trim().splitn(3, ':');
                let after = parts.next()?.parse::<usize>().ok()?.min(999);
                Some(Placed::new(Kind::from_key(parts.next()?)?, Size::from_key(parts.next()?)?, after))
            })
            .collect();
        list.sort_by_key(|w| w.after);
        list
    }

    /// The glance page's cards before widgets (0.6 and before), as widgets:
    /// each a wide one, so a page still holds two. The cards a handheld came
    /// with, never changed, give way to the new start instead.
    pub fn from_cards(text: &str) -> Vec<Placed> {
        let names: Vec<&str> = text.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
        if names == ["device", "robot", "next_match", "last_match"] {
            return Layout::starting().glance;
        }
        let mut out: Vec<Placed> = Vec::new();
        for name in names {
            let kind = match name {
                "device" => Kind::System,
                "robot" => Kind::Robot,
                "next_match" => Kind::NextMatch,
                "last_match" => Kind::LastMatch,
                "pinned" => Kind::Pinned,
                "note" => Kind::Note,
                _ => continue,
            };
            if !out.iter().any(|w| w.kind == kind) {
                out.push(Placed::new(kind, Size::Wide, 0));
            }
        }
        out
    }

    /// A value pinned in Tables: on the glance page, if it is not there already.
    pub fn pin_value(&mut self) {
        if !self.glance.iter().any(|w| w.kind == Kind::Pinned) {
            self.glance.push(Placed::new(Kind::Pinned, Size::Small, 0));
        }
    }
}

// ------------------------------------------------------------------ enter ---

/// What Enter on a widget did.
pub enum Act {
    /// Its app (or the page it is about), to grow out of the widget.
    Open(Box<dyn crate::shell::Screen>),
    /// It acted in place (and said so in the island, if there was anything to say).
    Done,
    /// Search, from the quick actions.
    Spotlight,
}

/// Enter on a widget of `kind`: music plays or pauses, the timer starts or
/// pauses, the first reminder is ticked off, quick actions search; the rest
/// open what they are about.
pub fn enter(kind: Kind, cx: &mut crate::shell::Cx, facts: &Facts) -> Act {
    let app = |name: &str, robot: &crate::robot::Robot| match crate::screens::builtin_apps().find(|(_, _, n)| *n == name) {
        Some((i, ..)) => Act::Open(crate::screens::open_builtin(i, robot)),
        None => Act::Done,
    };
    match kind {
        Kind::NextMatch => app("Schedule", cx.robot),
        Kind::Battery => Act::Open(Box::new(crate::power::BatteryPage::new())),
        Kind::Music if crate::nowplaying::get().is_some() => {
            crate::nowplaying::control(crate::nowplaying::Control::PlayPause);
            Act::Done
        }
        Kind::Music => app("Music", cx.robot),
        Kind::Weather => app("Weather", cx.robot),
        Kind::Robot => Act::Open(Box::new(crate::screens::RobotScreen::default())),
        Kind::Reminders => match facts.reminders.first() {
            Some(first) => {
                *cx.say = Some(match crate::reminders::done(first.id) {
                    Some(said) if said == "done" => format!("done · {}", first.text),
                    Some(said) => said,
                    None => "that reminder is gone".into(),
                });
                Act::Done
            }
            None => app("Reminders", cx.robot),
        },
        Kind::Clock => app("Clock", cx.robot),
        Kind::World => Act::Open(Box::new(crate::clock::Clock::on_world())),
        Kind::Timers => {
            *cx.say = Some(crate::clock::widget_press());
            Act::Done
        }
        Kind::Note => match &facts.note {
            Some(note) => match crate::files::Editor::open(&note.file) {
                Ok(editor) => Act::Open(Box::new(editor)),
                Err(why) => {
                    *cx.say = Some(why);
                    Act::Done
                }
            },
            None => app("Notes", cx.robot),
        },
        Kind::System => app("Monitor", cx.robot),
        Kind::Photos => app("Photos", cx.robot),
        Kind::Actions => Act::Spotlight,
        Kind::LastMatch => match cx.robot.logs.list().iter().find_map(crate::screens::LogDetail::of) {
            Some(detail) => Act::Open(Box::new(detail)),
            None => app("Logs", cx.robot),
        },
        Kind::Pinned => match cx.settings.pinned.clone() {
            Some(key) => Act::Open(Box::new(crate::screens::Tables::at(&key, cx.robot))),
            None => app("Tables", cx.robot),
        },
    }
}

// ---------------------------------------------------------------- packing ---

/// A board's page is two cells down.
pub const ROWS: usize = 2;

/// Where something is on a board: its page, its first column and row, and
/// how many cells it spans.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub page: usize,
    pub col: usize,
    pub row: usize,
    pub w: usize,
    pub h: usize,
}

impl Cell {
    /// Reading order: page, then row, then column.
    pub fn order(&self) -> (usize, usize, usize) {
        (self.page, self.row, self.col)
    }

    pub fn covers(&self, page: usize, col: usize, row: usize) -> bool {
        page == self.page && (self.col..self.col + self.w).contains(&col) && (self.row..self.row + self.h).contains(&row)
    }
}

/// Lay out things `spans` cells in size (across, down), in order, on pages
/// `columns` across and [`ROWS`] down: each goes in the first free place it
/// fits, reading from the first page, so later small things fill the holes
/// a wide one leaves.
pub fn pack(spans: &[(usize, usize)], columns: usize) -> Vec<Cell> {
    let columns = columns.max(1);
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::with_capacity(spans.len());
    for &(w, h) in spans {
        let (w, h) = (w.clamp(1, columns), h.clamp(1, ROWS));
        let mut page = 0;
        let cell = loop {
            if page == taken.len() {
                taken.push(vec![false; columns * ROWS]);
            }
            let free = |row: usize, col: usize| (row..row + h).all(|r| (col..col + w).all(|c| !taken[page][r * columns + c]));
            if let Some((row, col)) = (0..=ROWS - h).flat_map(|row| (0..=columns - w).map(move |col| (row, col))).find(|&(row, col)| free(row, col)) {
                break Cell { page, col, row, w, h };
            }
            page += 1;
        };
        for r in cell.row..cell.row + h {
            for c in cell.col..cell.col + w {
                taken[cell.page][r * columns + c] = true;
            }
        }
        out.push(cell);
    }
    out
}

/// Something on a board: one of Home's apps (by its place among them), or a
/// widget (by its place in the board's list).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Slot {
    App(usize),
    Widget(usize),
}

/// A board's slots in order: `apps` apps with the widgets among them, each
/// widget before the app it was placed before (on the glance page, no apps).
pub fn sequence(apps: usize, widgets: &[Placed]) -> Vec<Slot> {
    let mut order: Vec<usize> = (0..widgets.len()).collect();
    order.sort_by_key(|&j| widgets[j].after.min(apps));
    let mut out = Vec::with_capacity(apps + widgets.len());
    let mut k = 0;
    for i in 0..=apps {
        while k < order.len() && widgets[order[k]].after.min(apps) == i {
            out.push(Slot::Widget(order[k]));
            k += 1;
        }
        if i < apps {
            out.push(Slot::App(i));
        }
    }
    out
}

/// A board packed: each slot and its cell, in the board's order.
#[derive(Clone, Debug, PartialEq)]
pub struct Packed {
    pub slots: Vec<(Slot, Cell)>,
    pub pages: usize,
    pub columns: usize,
}

impl Packed {
    pub fn of(apps: usize, widgets: &[Placed], columns: usize) -> Packed {
        let seq = sequence(apps, widgets);
        let spans: Vec<(usize, usize)> = seq.iter().map(|s| if let Slot::Widget(j) = s { widgets[*j].size.span() } else { (1, 1) }).collect();
        let cells = pack(&spans, columns);
        let pages = cells.iter().map(|c| c.page + 1).max().unwrap_or(1);
        Packed { slots: seq.into_iter().zip(cells).collect(), pages, columns }
    }

    pub fn cell(&self, slot: Slot) -> Option<Cell> {
        self.slots.iter().find(|(s, _)| *s == slot).map(|(_, c)| *c)
    }

    /// Every slot in reading order.
    pub fn reading(&self) -> Vec<(Slot, Cell)> {
        let mut all = self.slots.clone();
        all.sort_by_key(|(_, c)| c.order());
        all
    }

    /// What covers a cell, if anything.
    pub fn at(&self, page: usize, col: usize, row: usize) -> Option<Slot> {
        self.slots.iter().find(|(_, c)| c.covers(page, col, row)).map(|(s, _)| *s)
    }

    /// The next slot from `from` in reading order (`by` −1 the one before).
    pub fn beside(&self, from: Slot, by: i32) -> Option<Slot> {
        let order = self.reading();
        let at = order.iter().position(|(s, _)| *s == from)? as i32 + by;
        (at >= 0).then(|| order.get(at as usize).map(|(s, _)| *s)).flatten()
    }

    /// Up or Down from `from`: what is in the row above or below on its
    /// page, nearest its column; none at the page's top or bottom. Down onto
    /// a short last row lands on that row's last thing.
    pub fn vertical(&self, from: Slot, down: bool) -> Option<Slot> {
        let cell = self.cell(from)?;
        let row = if down { cell.row + cell.h } else { cell.row.checked_sub(1)? };
        if row >= ROWS {
            return None;
        }
        if let Some(s) = self.at(cell.page, cell.col, row) {
            return Some(s);
        }
        // Nothing straight above or below: the nearest in that row.
        self.slots
            .iter()
            .filter(|(_, c)| c.page == cell.page && (c.row..c.row + c.h).contains(&row))
            .min_by_key(|(_, c)| (c.col as i32 - cell.col as i32).abs())
            .map(|(s, _)| *s)
    }
}

/// Where a board's cells are on the panel: below the island to above the
/// pager, `columns` across with Home's gaps.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Grid {
    pub columns: usize,
    pub top: f32,
    pub cell_w: f32,
    pub cell_h: f32,
}

impl Grid {
    pub fn of(g: &Gfx, columns: usize) -> Grid {
        let columns = columns.max(1);
        let top = g.top();
        // The grid ends above where the pager rests.
        let bottom = H - 16.0;
        Grid { columns, top, cell_w: (W - 2.0 * PAD - GAP * (columns - 1) as f32) / columns as f32, cell_h: (bottom - top - GAP * (ROWS - 1) as f32) / ROWS as f32 }
    }

    /// A cell's place on its page (each page a panel's width along).
    pub fn rect(&self, c: &Cell) -> Rect {
        Rect::new(
            PAD + c.col as f32 * (self.cell_w + GAP),
            self.top + c.row as f32 * (self.cell_h + GAP),
            c.w as f32 * self.cell_w + (c.w - 1) as f32 * GAP,
            c.h as f32 * self.cell_h + (c.h - 1) as f32 * GAP,
        )
    }

    /// The size a widget is drawn at on this grid.
    pub fn size_of(&self, size: Size) -> (f32, f32) {
        let (w, h) = size.span();
        let r = self.rect(&Cell { page: 0, col: 0, row: 0, w: w.min(self.columns), h });
        (r.w, r.h)
    }
}
