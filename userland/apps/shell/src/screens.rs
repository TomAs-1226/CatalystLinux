//! The built-in Catalyst apps. Each is a screen that reads the robot through
//! [`Robot`] and draws with [`Gfx`]; external apps are separate programs.
//!
//! Layouts are measured from the type scale (`g.ty`, `g.row_h()`, `g.top()`),
//! so every screen follows the text size chosen in System.

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::agent::Agent;
use crate::gfx::{Gfx, ListState, TextSize, GAP, H, PAD, W};
use crate::logs::Digest;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, HomeStyle, Nav, Screen};
use crate::widgets::edit::{app_key, widget_key, Dir, Editing, Glides};
use crate::widgets::{Act, Board, Cell, Grid, Kind, Layout, Packed, Placed, Slot};
use crate::wifi::Wifi;
use crate::wpilog::Level;

/// Settings › Camera › Grove port, as it stands (it changes at once).
fn grove_words() -> String {
    match (crate::smartcam::switching(), crate::smartcam::chosen()) {
        (true, _) => "changing over…".into(),
        (false, false) => "Console".into(),
        (false, true) => "Smart camera".into(),
    }
}

/// Settings › Camera's line along the bottom: what the row in focus does.
fn camera_hint(item: Option<Item>) -> &'static [&'static str] {
    match item {
        Some(Item::GrovePort) if crate::smartcam::chosen() => &["enter: console now · kernel messages from the next start", "enter: console now · kernel log next start", "console now · kernel log next start", "enter: console now"],
        Some(Item::GrovePort) => &["enter: smart camera now · login and console off it", "enter: smart camera now"],
        Some(Item::CameraProgram) => &["enter: its program over USB-A, no microSD card", "enter: its program over USB-A", "enter: over USB-A"],
        Some(Item::SmartCamera) => &["enter: its status · wakes with the Camera", "enter: its status"],
        _ => &["enter: auto, on or off · a photo of several frames", "enter: auto, on or off"],
    }
}

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "–".into())
}

/// Colour has jobs. On this device: red marks a fault (the signal, when the
/// accent is itself warm), amber a warning, ice what is on or well.
pub fn severity_color(g: &Gfx, severity: &str) -> Color {
    match severity {
        "ERROR" => g.fault(),
        "WARN" => g.tokens.amber_ink,
        _ => g.tokens.dim,
    }
}

/// Up, Down and the page keys over a list of `len` rows.
/// The keys `list_key` moves through a list with, for a key sheet.
pub const LIST_KEYS: &[(&str, &str)] = &[("↑ ↓", "Move"), ("pg up · pg dn", "Page by page"), ("home · end", "First or last")];

pub fn list_key(list: &mut ListState, key: Key, len: usize) {
    match key {
        Key::Up => list.up(),
        Key::Down => list.down(len),
        Key::PageUp => list.page(len, -1),
        Key::PageDown => list.page(len, 1),
        // Fn + K and Fn + N on the handheld: the first row, and the last.
        Key::Home => list.focus = 0,
        Key::End => list.focus = len.saturating_sub(1),
        _ => {}
    }
}

// ------------------------------------------------------------------ home ---

struct App {
    icon: &'static str,
    name: &'static str,
    open: fn(&Robot) -> Box<dyn Screen>,
}

const APPS: [App; 46] = [
    App { icon: "smart_toy", name: "Robot", open: |_| Box::new(RobotScreen::default()) },
    App { icon: "monitor_heart", name: "Health", open: |_| Box::new(Health::default()) },
    App { icon: "description", name: "Logs", open: |_| Box::new(LogsScreen::default()) },
    App { icon: "event", name: "Schedule", open: |_| Box::new(crate::schedule::ScheduleScreen::default()) },
    App { icon: "account_tree", name: "Tables", open: |_| Box::new(Tables::new()) },
    App { icon: "speed", name: "Loop", open: |_| Box::new(LoopTime) },
    App { icon: "precision_manufacturing", name: "Motors", open: |_| Box::new(Motors::default()) },
    App { icon: "developer_board", name: "Console", open: |robot| Box::new(Console::open(robot)) },
    App { icon: "terminal", name: "Terminal", open: |_| Box::new(crate::terminal::Terminal::new()) },
    App { icon: "folder", name: "Files", open: |_| Box::new(crate::files::Files::new()) },
    App { icon: "commit", name: "Git", open: |_| Box::new(crate::git::GitScreen::new()) },
    App { icon: "storefront", name: "Catalogue", open: |_| Box::new(crate::catalogue::CatalogueScreen::new()) },
    App { icon: "monitoring", name: "Monitor", open: |_| Box::new(crate::monitor::Monitor::new()) },
    App { icon: "network_check", name: "Net tools", open: |_| Box::new(crate::nettools::NetTools::new()) },
    App { icon: "settings_ethernet", name: "Connect", open: |_| Box::new(crate::connect::Connect::new()) },
    App { icon: "play_circle", name: "Actions", open: |_| Box::new(crate::commands::Commands::new()) },
    // The camera's apps side by side: photos (and QR codes), the album, tags.
    App { icon: "photo_camera", name: "Camera", open: |_| Box::new(crate::camera::Camera::new()) },
    App { icon: "photo_library", name: "Photos", open: |_| Box::new(crate::photos::Album::new()) },
    App { icon: "qr_code_scanner", name: "AprilTag", open: |_| Box::new(crate::apriltags::Reader::new()) },
    App { icon: "insights", name: "Graphs", open: |_| Box::new(crate::graphs::Graphs::new()) },
    App { icon: "checklist", name: "Pit", open: |_| Box::new(crate::pit::Pit::default()) },
    // The robot batteries: which went in for each match, their health from the logs, the next to use.
    App { icon: "battery_charging_full", name: "Batteries", open: crate::batteries::open },
    // Match scouting at speed on the keyboard, shared by QR with no internet.
    App { icon: "fact_check", name: "Scout", open: |_| Box::new(crate::scout::Scout::new()) },
    // The match's plan on the field, for the drive team (strategy/).
    App { icon: "strategy", name: "Strategy", open: |robot| Box::new(crate::strategy::Strategy::new(robot)) },
    // Messages between the team's handhelds at the event, sealed (team/).
    App { icon: "groups", name: "Team", open: |_| Box::new(crate::team::TeamScreen::new()) },
    App { icon: "settings_remote", name: "Remote", open: |_| Box::new(crate::irremote::Remotes::new()) },
    App { icon: "straighten", name: "Level", open: |_| Box::new(crate::level::Level::new()) },
    App { icon: "explore", name: "Compass", open: |_| Box::new(crate::level::Compass::new()) },
    App { icon: "sunny", name: "Weather", open: |_| Box::new(crate::weather::Weather::new()) },
    // The web read for you: an address made readable, a question answered.
    App { icon: "travel_explore", name: "Browse", open: |_| Box::new(crate::browse::Browse::new()) },
    // FIRST's game manual, kept on the handheld: read, searched, asked.
    App { icon: "menu_book", name: "Rulebook", open: |_| Box::new(crate::rulebook::Rulebook::new()) },
    App { icon: "calculate", name: "Calculator", open: |_| Box::new(crate::calc::Calculator::new()) },
    App { icon: "timer", name: "Clock", open: |_| Box::new(crate::clock::Clock::new()) },
    App { icon: "sticky_note_2", name: "Notes", open: |_| Box::new(crate::notes::Notes::new()) },
    App { icon: "notifications", name: "Reminders", open: |_| Box::new(crate::reminders::Reminders::new()) },
    // Passwords and secrets, sealed on the card, opened with the passcode.
    App { icon: "encrypted", name: "Vault", open: |_| Box::new(crate::vault::Vault::new()) },
    App { icon: "square_foot", name: "Units", open: |_| Box::new(crate::units::UnitsApp::new()) },
    // For the pit: a mechanism worked out from its motor, and a driver's controller checked.
    App { icon: "manufacturing", name: "Mech", open: |_| Box::new(crate::mech::MechApp::new()) },
    App { icon: "sports_esports", name: "Gamepad", open: |_| Box::new(crate::gamepad::Gamepad::new()) },
    App { icon: "api", name: "HTTP", open: |_| Box::new(crate::http::Http::new()) },
    App { icon: "qr_code_2", name: "Share", open: |robot| Box::new(crate::share::Share::new(robot)) },
    App { icon: "auto_awesome", name: "Assistant", open: |_| Box::new(crate::ai::Assistant::new()) },
    // Claude Code's sessions on the laptop, through its bridge (claude/).
    App { icon: "code_blocks", name: "Claude", open: |_| Box::new(crate::claude::ClaudeApp::new()) },
    // Pages as text: lynx in a terminal, on a start page of searches and FRC sites.
    App {
        icon: "language",
        name: "Web",
        open: |_| Box::new(crate::terminal::Terminal::running("Web", "lynx -accept_all_cookies -nopause file:///usr/share/catalyst/web.html", crate::connect::session_dir())),
    },
    // YouTube Music, playing on behind every other app (music.rs).
    App { icon: "music_note", name: "Music", open: |_| Box::new(crate::music::MusicScreen::new()) },
    App { icon: "settings", name: "Settings", open: |_| Box::new(SettingsApp::new()) },
];

/// Whether a built-in app is offered on this handheld: the Level only where
/// a motion sensor answers, the Compass only where its magnetometer does
/// too (the Lite has neither; the C154-V1 no magnetometer).
fn offered(name: &str) -> bool {
    match name {
        "Level" => crate::imu::has_motion(),
        "Compass" => crate::imu::has_compass(),
        _ => true,
    }
}

/// What the apps offered depend on, to lay Home out again if it changes.
fn hardware() -> (bool, bool) {
    (crate::imu::has_motion(), crate::imu::has_compass())
}

/// The built-in apps this handheld offers, for search: index, icon and name.
pub fn builtin_apps() -> impl Iterator<Item = (usize, &'static str, &'static str)> {
    APPS.iter().enumerate().filter(|(_, app)| offered(app.name)).map(|(i, app)| (i, app.icon, app.name))
}

/// Open a built-in app by its place in the list.
pub fn open_builtin(index: usize, robot: &Robot) -> Box<dyn Screen> {
    (APPS[index].open)(robot)
}

/// One of Home's tiles: a built-in app, or one found on the storage.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tile {
    Builtin(usize),
    External(usize),
}

/// The glance page's board: two cells across.
const GLANCE_COLUMNS: usize = 2;

pub struct Home {
    focus: usize,
    /// Which page of the grid is in view; turns on the page spring.
    page: Animated,
    /// Where the carousel has turned to, in apps: it glides to the focused
    /// one and lands like a dial on its detent.
    turn: Animated,
    /// 0 with the apps in view, 1 with the first glance page (one step to
    /// their left), 2 with the next one left of that, and so on.
    glance: Animated,
    glance_page: usize,
    /// The glance widget in focus: its place on the glance page's board.
    card: usize,
    /// Each glance widget where it was last drawn: Enter opens out of it.
    cards: Vec<Rect>,
    /// Which Home this is drawn as, as last seen in the settings.
    style: HomeStyle,
    /// Columns in the grid: fewer, larger tiles at the large text size.
    columns: usize,
    /// Where each tile was last drawn: an app opens out of its tile.
    rects: Vec<Rect>,
    /// Where each of Home's own widgets was last drawn.
    widget_rects: Vec<Rect>,
    /// The carousel's middle card.
    middle: Rect,
    /// Seconds since Home arrived, for the launcher cascade.
    arrived: f32,
    /// External apps found on the storage.
    external: Vec<crate::apps::App>,
    /// Home's tiles in the order chosen in Settings › Home › Arrange apps,
    /// those hidden there left out; and the arrangement they were laid out by.
    tiles: Vec<Tile>,
    arranged_by: crate::shell::Arrangement,
    /// The hardware the tiles were laid out for (`hardware()`).
    arranged_for: (bool, bool),
    /// Enter pressed on a tile: how long it has been pressed in. The app
    /// grows out of it once it has been, `PRESS_IN` later.
    opening: Option<f32>,
    /// Each tile's lift (1 while it has the focus), its pulse when what it
    /// reports changes for better or worse, and what it last reported.
    lift: Vec<Animated>,
    pulse: Vec<Animated>,
    seen: Vec<Option<String>>,
    /// Flow, the third style: album covers turning (flow.rs).
    flow: crate::flow::Flow,
    /// The widgets on both boards, as last seen in the settings.
    layout: Layout,
    /// The widget in focus among the grid's apps, by its place on Home's
    /// board; none while an app has it (`focus`).
    widget: Option<usize>,
    /// What the widgets show, read only while they are in sight.
    facts: crate::widgets::Facts,
    /// Every tile's and widget's place, gliding as a board is rearranged.
    glides: Glides,
    /// A board being edited.
    editing: Option<Editing>,
    /// When a widget in sight next changes by itself, and whether a frame
    /// has been asked for it (once: with the screen off, nothing more).
    next_change: Option<std::time::Instant>,
    asked: bool,
    /// A board was rearranged: its tiles glide until they settle.
    gliding: bool,
    /// Each board's measures as last drawn: Home's grid, the glance page.
    grids: (Option<Grid>, Option<Grid>),
}

/// The focused tile lifts on Bezel's detent spring: a touch larger and
/// lighter, with the smallest settle.
fn lift_spring() -> Animated {
    Animated::new(0.0, role::detent()).with_rest(0.01)
}

/// A tile's pulse fades on the smooth spring.
fn pulse_spring() -> Animated {
    Animated::new(0.0, role::smooth()).with_rest(0.01)
}

/// A tile as Bezel's wall panels have one: the app's name small, lowercase
/// and in the mono voice at the top, its icon at the top right (unless a
/// dot or a star is there), and what it reports below, in the display voice
/// and large when it is a reading.
fn bezel_tile(g: &mut Gfx, r: Rect, icon: &str, name: &str, status: &str, focused: bool, corner_taken: bool) {
    let (t, ty) = (g.tokens, g.ty);
    let quiet = if focused { t.ink } else { t.dim };
    let label_cy = r.y + 7.0 + ty.caption.size * 0.5;
    g.text_fit(r.x + 9.0, label_cy, &name.to_lowercase(), ty.caption, quiet, r.w - 34.0);
    if !corner_taken {
        g.icon(icon, focused, r.right() - 13.0, label_cy, 14.0, quiet);
    }
    let baseline = r.bottom() - 8.0;
    match split_reading(status) {
        Some((value, unit)) => {
            let size = (if r.w > 120.0 { 30.0 } else { 26.0f32 }).min(r.h - ty.caption.size - 20.0);
            let big = ty.display(size);
            let (value_w, unit_w) = (g.measure(value, big), g.measure(unit, ty.caption));
            if value_w + 3.0 + unit_w <= r.w - 18.0 {
                // The unit beside the number.
                let w = g.text(r.x + 8.0, baseline, value, big, t.ink);
                g.text(r.x + 11.0 + w, baseline, unit, ty.caption, t.dim);
            } else {
                // A long one ("in 8 min") on its own line under the name,
                // and the number a size smaller to make room.
                let unit = g.text.fit(unit, &ty.caption, r.w - 18.0);
                g.text_at(r.x + 9.0, label_cy + ty.caption.size + 3.0, Align::Left, &unit, ty.caption, t.dim);
                let big = ty.display(size.min(r.h - 2.0 * ty.caption.size - 22.0));
                let value = g.text.fit(value, &big, r.w - 18.0);
                g.text(r.x + 8.0, baseline, &value, big, t.ink);
            }
        }
        None => {
            g.text_fit(r.x + 9.0, baseline - ty.name.size * 0.36, status, ty.name, t.ink, r.w - 18.0);
        }
    }
}

/// A status as a reading: the number (or a match like "Q31", or "–") to
/// show large, and what follows it. `None` when it is words.
fn split_reading(status: &str) -> Option<(&str, &str)> {
    let (first, rest) = status.split_once(' ').unwrap_or((status, ""));
    let mut chars = first.chars();
    let lead = chars.next()?;
    let numeric = lead.is_ascii_digit() || first == "–";
    let label = lead.is_ascii_uppercase() && first.len() <= 4 && first.len() > 1 && chars.all(|c| c.is_ascii_digit());
    (numeric || label).then_some((first, rest))
}

/// How long a tile is pressed in before its app grows out of it: long enough
/// to be seen, short enough not to be waited for. (My own tuning.)
const PRESS_IN: f32 = 0.07;

/// Bezel's launcher cascade: tiles spring into place one after another,
/// spread over this long from the first to the last.
const CASCADE_SPREAD: f32 = 0.28;
const CASCADE_OVER: f32 = CASCADE_SPREAD + 0.9;

/// How far the widget chosen while editing lifts: past a focused tile's.
/// (My own tuning.)
const EDIT_LIFT: f32 = 1.6;

/// What the keys do while editing, along the foot: the longest that fits.
const EDIT_HINTS: [&str; 3] = ["arrows move · r size · a add · d remove · enter done", "arrows move · r size · a add · d remove", "r size · a add · d remove"];
const PICK_HINTS: [&str; 3] = ["← → widget · ↑ ↓ size · enter adds · esc back", "← → widget · ↑ ↓ size · enter adds", "← → · ↑ ↓ · enter"];

impl Home {
    pub fn new() -> Self {
        Home {
            focus: 0,
            // Pages turned with keys run a few pixels past and come back.
            page: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.002),
            turn: Animated::new(0.0, role::detent()).with_rest(0.002),
            glance: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.002),
            glance_page: 0,
            card: 0,
            cards: Vec::new(),
            style: HomeStyle::Grid,
            columns: 3,
            rects: Vec::new(),
            widget_rects: Vec::new(),
            middle: Rect::default(),
            arrived: 0.0,
            external: crate::apps::find(),
            tiles: Vec::new(),
            arranged_by: crate::shell::Arrangement::default(),
            arranged_for: (false, false),
            opening: None,
            lift: Vec::new(),
            pulse: Vec::new(),
            seen: Vec::new(),
            flow: crate::flow::Flow::new(),
            layout: Layout::starting(),
            widget: None,
            facts: crate::widgets::Facts::default(),
            glides: Glides::default(),
            editing: None,
            next_change: None,
            asked: false,
            gliding: false,
            grids: (None, None),
        }
        .arranged(&crate::shell::Arrangement::default())
    }

    /// Lay the tiles out: the apps named in `order` first, in that order,
    /// then the rest as they come (built-in, then external), leaving out
    /// those in `hidden`. Settings is never hidden: it is the way back.
    fn arranged(mut self, arrangement: &crate::shell::Arrangement) -> Self {
        self.arrange(arrangement);
        self
    }

    fn arrange(&mut self, arrangement: &crate::shell::Arrangement) {
        let focused = self.tiles.get(self.focus).map(|t| self.tile_name(*t).to_owned());
        self.arrange_around(arrangement, focused);
    }

    /// What a tile pulses for when it changes: for Health, Logs and Motors
    /// what they say (a new warning, a log arriving, a motor dropping out);
    /// for Robot whether one is linked; for every tile, its warning or fault
    /// coming or going. Never a live reading (volts, loop time, a countdown),
    /// which changes all the time.
    fn news(&self, index: usize, status: &str, attention: Option<&'static str>, robot: &Robot) -> String {
        let name = self.icon_and_name(index).1;
        let said = match name.as_str() {
            "Health" | "Logs" | "Motors" => status,
            "Robot" if robot.linked() => "linked",
            _ => "",
        };
        format!("{said}|{}", attention.unwrap_or(""))
    }

    /// Whether an app is one of the favourites.
    fn favourite(&self, index: usize) -> bool {
        self.tiles.get(index).is_some_and(|t| self.arranged_by.favourites.iter().any(|f| f == self.tile_name(*t)))
    }

    /// Arrange, keeping the focus on the app named `focused` wherever it now
    /// is (or as near its old place as there is a tile).
    fn arrange_around(&mut self, arrangement: &crate::shell::Arrangement, focused: Option<String>) {
        let mut tiles: Vec<Tile> = (0..APPS.len()).map(Tile::Builtin).chain((0..self.external.len()).map(Tile::External)).collect();
        let (order, hidden, favourites) = (&arrangement.order, &arrangement.hidden, &arrangement.favourites);
        // Favourites first, then the rest; each in the order chosen.
        let place = |t: &Tile| {
            let name = self.tile_name(*t);
            (!favourites.iter().any(|f| f == name), order.iter().position(|n| n == name).unwrap_or(usize::MAX))
        };
        tiles.sort_by_key(place);
        tiles.retain(|t| self.tile_name(*t) == "Settings" || !hidden.iter().any(|n| n == self.tile_name(*t)));
        // Apps for hardware this handheld does not have are not offered.
        tiles.retain(|t| !matches!(t, Tile::Builtin(i) if !offered(APPS[*i].name)));
        let changed = tiles != self.tiles;
        self.tiles = tiles;
        self.arranged_by = arrangement.clone();
        self.arranged_for = hardware();
        self.lift.clear();
        self.pulse.clear();
        self.seen.clear();
        // The tiles are counted afresh: each is placed at once.
        if changed {
            self.glides.forget();
        }
        let found = focused.and_then(|name| self.tiles.iter().position(|t| self.tile_name(*t) == name));
        let at = found.unwrap_or(self.focus).min(self.tiles.len().saturating_sub(1));
        if at != self.focus {
            self.focus = at;
            self.page.jump(self.page_of(Slot::App(at)) as f32);
            self.turn.jump(at as f32);
            self.flow.place(at);
        }
    }

    /// Arrange again if the settings' arrangement is not the one in use (or
    /// the motion sensor has just been found).
    fn follow_arrangement(&mut self, arrangement: &crate::shell::Arrangement) {
        if self.arranged_by != *arrangement || self.arranged_for != hardware() {
            self.arrange(arrangement);
        }
    }

    /// Take the widgets as the settings have them now. Changed from
    /// elsewhere (a value pinned in Tables, Start over), they glide there.
    fn follow_layout(&mut self, layout: &Layout) {
        if self.layout == *layout {
            return;
        }
        self.layout = layout.clone();
        self.card = self.card.min(self.layout.glance.len().saturating_sub(1));
        if self.widget.is_some_and(|j| j >= self.layout.home.len()) {
            self.widget = None;
        }
        if self.editing.is_none() {
            self.gliding = true;
        }
    }

    fn tile_name(&self, tile: Tile) -> &str {
        match tile {
            Tile::Builtin(i) => APPS[i].name,
            // An app gone from the storage since: no name.
            Tile::External(i) => self.external.get(i).map_or("", |app| app.name.as_str()),
        }
    }

    fn count(&self) -> usize {
        self.tiles.len()
    }

    #[cfg(test)]
    fn per_page(&self) -> usize {
        self.columns * 2
    }

    // ---- the boards ----

    /// A board's widgets: as edited while it is being edited, else as kept.
    fn board(&self, board: Board) -> &[Placed] {
        match &self.editing {
            Some(e) if e.board == board => &e.list,
            _ => self.layout.board(board),
        }
    }

    /// Home's grid packed: its apps, and (in the grid style) its widgets
    /// among them.
    fn grid(&self) -> Packed {
        let widgets: &[Placed] = if self.style == HomeStyle::Grid { self.board(Board::Home) } else { &[] };
        Packed::of(self.count(), widgets, self.columns)
    }

    /// The glance page's board packed.
    fn glance_board(&self) -> Packed {
        Packed::of(0, self.board(Board::Glance), GLANCE_COLUMNS)
    }

    /// The glance pages there are.
    fn glance_pages(&self) -> usize {
        self.glance_board().pages.max(1)
    }

    /// What has the focus on the grid: a widget, or an app.
    fn slot(&self) -> Slot {
        match self.widget {
            Some(j) if self.style == HomeStyle::Grid && j < self.board(Board::Home).len() => Slot::Widget(j),
            _ => Slot::App(self.focus),
        }
    }

    fn focus_slot(&mut self, slot: Slot) {
        match slot {
            Slot::App(i) => {
                self.focus = i;
                self.widget = None;
            }
            Slot::Widget(j) => self.widget = Some(j),
        }
    }

    /// The grid's page something is on.
    fn page_of(&self, slot: Slot) -> usize {
        self.grid().cell(slot).map_or(0, |c| c.page)
    }

    /// The grid's page the focus is on.
    fn grid_page(&self) -> usize {
        self.page_of(self.slot())
    }

    fn icon_and_name(&self, index: usize) -> (String, String) {
        match self.tiles.get(index) {
            Some(Tile::Builtin(i)) => (APPS[*i].icon.to_owned(), APPS[*i].name.to_owned()),
            Some(Tile::External(i)) => (self.external[*i].icon.clone(), self.external[*i].name.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// The one line under each app's name, and whether it needs attention.
    fn status(&self, index: usize, robot: &Robot) -> (String, Option<&'static str>) {
        let linked = robot.linked();
        let app = match self.tiles.get(index) {
            Some(Tile::Builtin(i)) => &APPS[*i],
            Some(Tile::External(i)) => return (self.external[*i].about.clone(), None),
            None => return (String::new(), None),
        };
        match app.name {
            "Robot" if linked => (dash(robot.f64("Systemcore/BatteryVolts").map(|v| format!("{v:.1} v"))), None),
            "Robot" => (robot.status(), None),
            "Health" => match (robot.i64("Health/ErrorCount"), robot.i64("Health/WarnCount")) {
                (Some(e), _) if e > 0 => (format!("{e} error{}", if e == 1 { "" } else { "s" }), Some("ERROR")),
                (_, Some(w)) if w > 0 => (format!("{w} warn"), Some("WARN")),
                (Some(_), _) => ("all clear".into(), None),
                _ => ("–".into(), None),
            },
            "Tables" => (if linked { "live values".into() } else { "–".into() }, None),
            "Loop" => (
                dash(robot.f64("Loop/Robot/AverageMs").map(|v| format!("{v:.1} ms"))),
                (robot.bool("Loop/Robot/OverBudget") == Some(true)).then_some("WARN"),
            ),
            "Motors" => match (robot.i64("Devices/Motors/Connected"), robot.i64("Devices/Motors/Expected")) {
                (Some(c), Some(e)) => (format!("{c} of {e} up"), (c < e).then_some("ERROR")),
                _ => ("–".into(), None),
            },
            "Schedule" => match robot.schedule.view() {
                Some(view) => match view.next(chrono::Utc::now().timestamp()) {
                    Some(m) => {
                        let now = chrono::Utc::now().timestamp();
                        let when = m.queue_at.or(m.time).map(|t| crate::schedule::when(t, now)).unwrap_or_default();
                        (format!("{} {when}", m.label).trim().to_owned(), (m.status.as_deref() == Some("Now queuing")).then_some("WARN"))
                    }
                    None => ("no match coming".into(), None),
                },
                None => ("–".into(), None),
            },
            "Graphs" => match robot.graphs.keys().len() {
                0 => ("none yet".into(), None),
                n => (format!("{n} live"), None),
            },
            "Pit" => crate::pit::tile(robot),
            "Batteries" => crate::batteries::tile(),
            "Scout" => ("matches · picks".into(), None),
            "Team" => crate::team::tile(),
            "Claude" => crate::claude::tile(),
            "Console" => ("robot log".into(), None),
            "Terminal" => ("shell".into(), None),
            "Files" => ("storage".into(), None),
            "Git" => ("repositories".into(), None),
            "Catalogue" => ("get apps".into(), None),
            "Monitor" => ("cpu · memory".into(), None),
            "Net tools" => ("find · ping".into(), None),
            "Connect" => ("ssh · serial".into(), None),
            "Actions" => ("one key".into(), None),
            "Level" => ("pitch · roll".into(), None),
            "Compass" => ("heading".into(), None),
            "Weather" => crate::weather::tile(),
            "Camera" => ("photo · scan".into(), None),
            "Photos" => crate::photos::tile(),
            "Clock" => crate::clock::tile(),
            "Calculator" => ("math · hex".into(), None),
            "Notes" => ("pit notes".into(), None),
            "Reminders" => crate::reminders::tile(),
            "Vault" => ((if crate::vault::is_open() { "open" } else { "passwords" }).into(), None),
            "Units" => ("in·lb · N·m".into(), None),
            "Mech" => ("ratio · arm · drive".into(), None),
            "Gamepad" => crate::gamepad::tile(),
            "HTTP" => ("GET · POST".into(), None),
            "Share" => ("as a QR code".into(), None),
            "Assistant" => crate::ai::tile(),
            "Browse" => ("read the web".into(), None),
            "Rulebook" => crate::rulebook::tile(),
            "Web" => ("pages as text".into(), None),
            "Music" => crate::music::tile(),
            "Logs" => {
                let rows = robot.logs.list();
                let worst = rows.iter().filter_map(|r| r.digest.as_ref()).map(|d| d.headline().0).min();
                let attention = match worst {
                    Some(Level::Fault) => Some("ERROR"),
                    Some(Level::Warn) => Some("WARN"),
                    _ => None,
                };
                (if rows.is_empty() { "none yet".into() } else { format!("{} kept", rows.len()) }, attention)
            }
            _ => ("this device".into(), None),
        }
    }

    fn on_glance(&self) -> bool {
        self.glance_page > 0
    }

    /// Bring the glance page in (its first page, the widget nearest Home in
    /// focus), or send it back out.
    fn show_glance(&mut self, show: bool, calm: bool) {
        if show {
            let packed = self.glance_board();
            if let Some(Slot::Widget(j)) = edge_of(&packed, 0, 0, true) {
                self.card = j;
            }
        }
        self.glance_to(if show { 1 } else { 0 }, calm);
    }

    /// Turn to glance page `page` (0: the apps).
    fn glance_to(&mut self, page: usize, calm: bool) {
        self.glance_page = page;
        if calm {
            self.glance.jump(page as f32);
        } else {
            self.glance.set(page as f32);
        }
    }

    /// Home's start: the first app in focus on the first page (the glance
    /// page sent back out if it is in), the page or the carousel turning
    /// there on the same springs as any turn. Already there: nothing moves.
    fn to_start(&mut self, calm: bool) {
        if self.on_glance() {
            self.show_glance(false, calm);
        }
        self.focus = 0;
        self.widget = None;
        if calm {
            self.page.jump(0.0);
            self.turn.jump(0.0);
        } else {
            self.page.set(0.0);
            self.turn.set(0.0);
        }
        self.flow.close(calm);
        self.flow.turn_to(&mut self.focus, 0, calm);
    }

    /// Keys on the glance page. Its pages run leftward from Home, two cells
    /// across and two down: Left goes to the widget on the left and on to
    /// the next page, Right back toward Home, Up and Down between its rows.
    fn glance_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let calm = cx.settings.calm;
        let packed = self.glance_board();
        let count = self.board(Board::Glance).len();
        if count == 0 {
            match key {
                Key::Right | Key::Tab => self.show_glance(false, calm),
                // Nothing here yet: Enter adds one.
                Key::Enter => return self.begin_edit(Board::Glance, cx),
                Key::Left if !calm => self.glance.nudge(1.6),
                _ => {}
            }
            return Nav::Stay;
        }
        self.card = self.card.min(count - 1);
        let here = Slot::Widget(self.card);
        let Some(cell) = packed.cell(here) else { return Nav::Stay };
        let to = match key {
            // (Esc is Home's own: back to the first app, see `to_start`.)
            Key::Left => match (cell.col > 0).then(|| packed.at(cell.page, cell.col - 1, cell.row)).flatten() {
                Some(s) => Some(s),
                None if cell.page + 1 < packed.pages => edge_of(&packed, cell.page + 1, cell.row, true),
                // Nothing further left: the page gives a little and comes back.
                None => {
                    if !calm {
                        self.glance.nudge(1.6);
                    }
                    None
                }
            },
            Key::Right => match (cell.col + cell.w < GLANCE_COLUMNS).then(|| packed.at(cell.page, cell.col + cell.w, cell.row)).flatten() {
                Some(s) => Some(s),
                None if cell.page > 0 => edge_of(&packed, cell.page - 1, cell.row, false),
                None => {
                    self.show_glance(false, calm);
                    return Nav::Stay;
                }
            },
            Key::Up => packed.vertical(here, false),
            Key::Down => packed.vertical(here, true),
            Key::Tab => packed.beside(here, 1).or_else(|| packed.reading().first().map(|(s, _)| *s)),
            Key::Enter => return self.enter_widget(Board::Glance, self.card, cx),
            _ => None,
        };
        if let Some(Slot::Widget(j)) = to {
            self.card = j;
        }
        // The page follows the focus.
        let page = packed.cell(Slot::Widget(self.card)).map_or(0, |c| c.page) + 1;
        if page != self.glance_page {
            self.glance_to(page, calm);
        }
        Nav::Stay
    }

    /// Open the focused app out of the tile (or card) it is drawn in, or do
    /// what the focused widget does.
    fn open(&mut self, cx: &mut Cx) -> Nav {
        if let Slot::Widget(j) = self.slot() {
            return self.enter_widget(Board::Home, j, cx);
        }
        let screen = match self.tiles.get(self.focus) {
            Some(Tile::Builtin(i)) => (APPS[*i].open)(cx.robot),
            Some(Tile::External(i)) => self.external[*i].open(),
            None => return Nav::Stay,
        };
        let from = match self.style {
            HomeStyle::Grid => self.rects.get(self.focus).copied(),
            HomeStyle::Carousel => Some(self.middle),
            // Out of the enlarged card, carrying on its motion; or the middle one.
            HomeStyle::Flow => Some(if self.flow.open_on(self.focus) { self.flow.enlarged() } else { self.flow.middle() }),
        };
        match from {
            Some(tile) if tile.w > 0.0 => Nav::Open(screen, tile),
            _ => Nav::Push(screen),
        }
    }

    /// Enter on a widget: its app grows out of it, or it acts in place.
    fn enter_widget(&mut self, board: Board, j: usize, cx: &mut Cx) -> Nav {
        let Some(w) = self.board(board).get(j).copied() else { return Nav::Stay };
        let from = match board {
            Board::Home => self.widget_rects.get(j),
            Board::Glance => self.cards.get(j),
        }
        .copied()
        .filter(|r| r.w > 0.0);
        match crate::widgets::enter(w.kind, cx, &self.facts) {
            Act::Open(screen) => match from {
                Some(r) => Nav::Open(screen, r),
                None => Nav::Push(screen),
            },
            Act::Done => {
                // It changed: read again, and drawn again in a moment, as
                // whatever it asked settles (a song pausing).
                self.facts.changed(w.kind);
                self.next_change = Some(std::time::Instant::now() + std::time::Duration::from_millis(200));
                self.asked = false;
                Nav::Stay
            }
            Act::Spotlight => Nav::Spotlight,
        }
    }

    /// Keys on the flow: Left and Right turn it (held, faster and faster);
    /// Tab, the page keys, Home and End move further.
    fn flow_key(&mut self, key: Key, calm: bool) {
        let count = self.count();
        let moved = match key {
            Key::Left => self.flow.step(&mut self.focus, count, -1, calm),
            Key::Right => self.flow.step(&mut self.focus, count, 1, calm),
            Key::Tab => {
                let to = if self.focus + 1 < count { self.focus + 1 } else { 0 };
                self.flow.close(calm);
                self.flow.turn_to(&mut self.focus, to, calm);
                crate::flow::Moved::Stay
            }
            Key::PageDown | Key::PageUp | Key::Home | Key::End => {
                let last = count.saturating_sub(1);
                let to = match key {
                    Key::PageDown => (self.focus + 6).min(last),
                    Key::PageUp => self.focus.saturating_sub(6),
                    Key::Home => 0,
                    _ => last,
                };
                self.flow.close(calm);
                self.flow.turn_to(&mut self.focus, to, calm);
                crate::flow::Moved::Stay
            }
            _ => crate::flow::Moved::Stay,
        };
        if moved == crate::flow::Moved::Glance {
            self.show_glance(true, calm);
        }
        self.turn.jump(self.focus as f32);
        self.page.jump(self.page_of(Slot::App(self.focus)) as f32);
    }

    /// Flow's frame: the cards, made for these apps; the enlarged one's
    /// back; then the title again over the ground, and what the card in
    /// the middle says, under it.
    fn draw_flow(&mut self, g: &mut Gfx, robot: &Robot) {
        let count = self.count();
        // Home in front again after the app opened out of the enlarged card
        // went back into it: now the card turns back into the flow.
        if self.flow.close_on_return && !g.focus_static {
            self.flow.close_on_return = false;
            self.flow.close(g.calm);
            g.started_moving();
        }
        let specs: Vec<crate::flow::art::CardSpec> = (0..count)
            .map(|i| {
                let (icon, name) = self.icon_and_name(i);
                let hue = crate::flow::hue_of(&name, &icon);
                crate::flow::art::CardSpec { icon, name, hue }
            })
            .collect();
        self.flow.make_cards(g, &specs);
        if self.flow.waiting() {
            // Painting the covers: look again next frame.
            g.started_moving();
        } else if std::mem::take(&mut self.flow.arrive_when_ready) {
            // Here now: they sweep in.
            self.flow.arrive(self.focus);
            g.started_moving();
        }
        if let Some(at) = self.flow.opened().filter(|at| *at < count) {
            let (status, attention) = self.status(at, robot);
            let about = match self.tiles.get(at) {
                Some(Tile::External(i)) => self.external.get(*i).map(|a| a.about.clone()).unwrap_or_default(),
                _ => crate::flow::about(&specs[at].name).unwrap_or_default().to_owned(),
            };
            let face = crate::flow::art::FaceSpec {
                card: specs[at].clone(),
                about,
                reading: split_reading(&status).map(|(v, u)| (v.to_owned(), u.to_owned())),
                status,
                attention: attention.map(|s| severity_color(g, s)),
                hint: "enter · open".into(),
            };
            self.flow.make_face(g, &face);
        }
        let colours: Vec<Color> = specs.iter().map(|s| crate::flow::art::colours(s.hue).3).collect();
        let press = if self.on_glance() { 0.0 } else { g.press() };
        let mut scene = self.flow.scene(g, count, self.focus, &colours, press);
        if self.flow.waiting() {
            scene.cards.clear();
        }
        g.flow(&scene, self.flow.atlases());
        self.middle = self.flow.middle();
        // The title, again over the ground the flow painted (only there).
        let ox = g.ox;
        g.ox = 0.0;
        g.canvas.set_clip(Rect::new(scene.cols.0 as f32, 0.0, (scene.cols.1 - scene.cols.0) as f32, H));
        crate::shell::draw_title(g, self.title());
        g.canvas.clear_clip();
        g.ox = ox;
        // What the card in the middle says, under its reflection: while the
        // flow is at rest on it, and not while one is enlarged.
        let (t, ty) = (g.tokens, g.ty);
        let off = (self.flow.pos.value() - self.focus as f32).abs();
        let speed = self.flow.pos.velocity().abs();
        let shown = (1.0 - off * 2.0).clamp(0.0, 1.0) * (1.0 - 6.0 * self.flow.openness().abs()).clamp(0.0, 1.0) * (1.0 - speed / 6.0).clamp(0.0, 1.0);
        if shown > 0.01 && count > 0 && !self.flow.waiting() {
            let (status, attention) = self.status(self.focus, robot);
            let status = g.text.fit(&status, &ty.caption, W - 4.0 * PAD);
            // Clear of the page track at the bottom (its platter starts 13 px up).
            let cy = (scene.cy + crate::flow::CARD as f32 * 0.5 + 23.0).min(H - 16.0 - ty.caption.size * 0.5);
            let w = g.measure(&status, ty.caption);
            let dot = if attention.is_some() { 10.0 } else { 0.0 };
            let x = (W - w - dot) * 0.5;
            if let Some(severity) = attention {
                let c = severity_color(g, severity);
                g.canvas.circle(x + 3.0 + g.ox, cy, 3.0, c, shown);
            }
            g.text_alpha(x + dot, cy, &status, ty.caption, t.dim, shown);
        }
    }

    /// Keys on the grid of tiles and widgets. True if the key pushed
    /// against an edge.
    fn grid_key(&mut self, key: Key, calm: bool) -> bool {
        let packed = self.grid();
        let here = self.slot();
        let to = match key {
            // Left and Right go through the grid in reading order: past a
            // row's end to the next row, past a page's last tile to the next
            // page (which turns), and before the first to the glance page.
            Key::Left => match packed.beside(here, -1) {
                Some(s) => Some(s),
                None => {
                    self.show_glance(true, calm);
                    None
                }
            },
            Key::Right => match packed.beside(here, 1) {
                Some(s) => Some(s),
                None => return true,
            },
            // Up and Down stay within the page, column by column; a short
            // last row, down from above its end, lands on its last tile.
            Key::Up => packed.vertical(here, false),
            Key::Down => packed.vertical(here, true),
            Key::Tab => packed.beside(here, 1).or_else(|| packed.reading().first().map(|(s, _)| *s)),
            _ => None,
        };
        if let Some(slot) = to {
            self.focus_slot(slot);
        }
        false
    }

    /// Keys on the carousel. True if the key pushed against an edge.
    fn carousel_key(&mut self, key: Key, calm: bool) -> bool {
        let count = self.count();
        match key {
            Key::Left if self.focus > 0 => self.focus -= 1,
            Key::Left => self.show_glance(true, calm),
            Key::Right | Key::Tab if self.focus + 1 < count => self.focus += 1,
            Key::Right => return true,
            Key::Tab => self.focus = 0,
            Key::PageDown => self.focus = (self.focus + 3).min(count - 1),
            Key::PageUp => self.focus = self.focus.saturating_sub(3),
            _ => {}
        }
        false
    }

    /// Whether the boards' tiles glide to their places just now (editing, or
    /// a moment after), rather than being there at once.
    fn glide(&self, g: &Gfx) -> bool {
        !g.calm && !g.focus_static && (self.editing.is_some() || self.gliding)
    }

    /// Home's grid: its app tiles and its widgets, on pages a panel's width
    /// apart. When a widget in sight next changes by itself.
    fn draw_grid(&mut self, g: &mut Gfx, robot: &Robot) -> Option<std::time::Instant> {
        let t = g.tokens;
        let count = self.count();
        let packed = self.grid();
        let grid = Grid::of(g, self.columns);
        self.grids.0 = Some(grid);
        if self.rects.len() != count {
            self.rects.resize(count, Rect::default());
        }
        if self.lift.len() != count {
            self.lift = (0..count).map(|_| lift_spring()).collect();
            self.pulse = (0..count).map(|_| pulse_spring()).collect();
            self.seen = vec![None; count];
        }
        let widgets: Vec<Placed> = self.board(Board::Home).to_vec();
        self.widget_rects.resize(widgets.len(), Rect::default());
        let press = g.press();
        let animate = self.glide(g);
        let editing = self.editing.as_ref().is_some_and(|e| e.board == Board::Home);
        // The pages are one strip, turned by `page`: the ring rides it. It is
        // drawn last, over every tile, as on its way it crosses others.
        let strip = -self.page.value() * W;
        let current = self.slot();
        let mut focus_ring = None;
        let mut next = None;
        // The widget chosen while editing is lifted over the rest: drawn last.
        let lifted = self.editing.as_ref().filter(|e| e.board == Board::Home).and_then(|e| e.selected);
        let mut held = None;
        for &(slot, cell) in &packed.slots {
            let along = grid.rect(&cell).offset(cell.page as f32 * W, 0.0);
            let key = match slot {
                Slot::App(i) => app_key(i),
                Slot::Widget(j) => widget_key(widgets[j].id),
            };
            let at = self.glides.place(key, along, animate).offset(strip, 0.0);
            // Where it rests on screen: what it opens out of.
            let resting = along.offset(strip, 0.0);
            match slot {
                Slot::App(i) => self.rects[i] = resting,
                Slot::Widget(j) => self.widget_rects[j] = resting,
            }
            if at.right() + g.ox <= 0.0 || at.x + g.ox >= W {
                continue;
            }
            // The cascade: each tile arrives a little after the one before it,
            // nearest the top left first, and springs up into its place.
            let at = if g.calm || self.arrived >= CASCADE_OVER {
                at
            } else {
                let order = cell.col as f32 + cell.row as f32 * 1.5;
                let since = self.arrived - CASCADE_SPREAD * order / (self.columns as f32 + 0.5);
                if since < 0.0 {
                    continue;
                }
                at.offset(0.0, 14.0 * role::release().solve(1.0, 0.0, since).0)
            };
            let focused = slot == current && !self.on_glance();
            match slot {
                Slot::App(i) => {
                    let ring = self.draw_tile(g, i, at, focused && !editing, press, robot);
                    if editing {
                        // While widgets are edited the apps step back.
                        g.canvas.rrect(at.offset(g.ox, 0.0), g.tile_radius(), t.ground, 0.55);
                    }
                    if let Some(ring) = ring {
                        focus_ring = Some(ring);
                    }
                }
                Slot::Widget(j) if lifted == Some(j) => held = Some((j, at)),
                Slot::Widget(j) => {
                    let (ring, when) = self.draw_widget(g, Board::Home, &widgets[j], at, grid.size_of(widgets[j].size), focused && !editing, press, robot);
                    next = earlier(next, when);
                    if let Some(ring) = ring {
                        focus_ring = Some(ring);
                    }
                }
            }
        }
        self.draw_leaving(g, Board::Home, strip);
        if let Some((j, at)) = held {
            let (ring, when) = self.draw_widget(g, Board::Home, &widgets[j], at, grid.size_of(widgets[j].size), true, press, robot);
            next = earlier(next, when);
            focus_ring = ring.or(focus_ring);
        }
        if let Some(ring) = focus_ring {
            ring_on(g, ring, Some(strip), editing);
        }
        next
    }

    /// One app's tile at `r`: lifted while focused, pulsing when what it
    /// says turns. The ring's place, if it has the focus.
    fn draw_tile(&mut self, g: &mut Gfx, i: usize, r: Rect, focused: bool, press: f32, robot: &Robot) -> Option<Rect> {
        let t = g.tokens;
        let ty = g.ty;
        let (icon, name) = self.icon_and_name(i);
        let (status, attention) = self.status(i, robot);
        // The focused tile lifts: a touch larger and lighter. Calm, or
        // drawn as a still picture, it is simply where it ends up.
        let want = if focused { 1.0 } else { 0.0 };
        if self.lift[i].target() != want {
            if g.calm || g.focus_static {
                self.lift[i].jump(want);
            } else {
                self.lift[i].set(want);
                g.started_moving();
            }
        }
        // What it reports turned worse, or better: it pulses once in the
        // colour of what it now says.
        let news = self.news(i, &status, attention, robot);
        if self.seen[i].as_ref().is_some_and(|before| *before != news) && !g.calm {
            self.pulse[i].jump(1.0);
            self.pulse[i].set(0.0);
            g.started_moving();
        }
        self.seen[i] = Some(news);
        let (lift, pulse) = (self.lift[i].value(), self.pulse[i].value().clamp(0.0, 1.0));
        let pressed = if focused { press } else { 0.0 };
        // The ring goes straight to where the lifted tile rests.
        let ring = r.inset(-1.5).offset(0.0, -1.0);
        let r = r.inset(-1.5 * lift + 2.0 * pressed).offset(0.0, -lift);
        let radius = (g.tile_radius() + 1.5 * lift - 2.0 * pressed).max(0.0);
        let tint = attention.map_or(t.ice, |severity| severity_color(g, severity));
        let fill = t.surface1.mix(t.surface2, lift * 0.7).mix(tint, 0.3 * pulse).mix(t.ink, 0.12 * pressed);
        // A lifted tile casts a soft shadow.
        g.shadow(r, radius, lift);
        g.surface(r, radius, fill);
        // Bezel's wall-panel tiles: the name small, lowercase and mono at
        // the top, what the app reports large below it.
        if g.theme == crate::look::Theme::Bezel {
            let corner_taken = attention.is_some() || self.favourite(i);
            bezel_tile(g, r, &icon, &name, &status, focused, corner_taken);
            if let Some(severity) = attention {
                let c = severity_color(g, severity);
                g.dot(r.right() - 12.0, r.y + 12.0, 3.5 + 1.5 * pulse, c);
            }
            if self.favourite(i) {
                let x = r.right() - if attention.is_some() { 25.0 } else { 12.0 };
                g.icon("star", true, x, r.y + 12.0, 11.0, t.dim);
            }
            return focused.then_some(ring);
        }
        // A selected item's icon fills in.
        let icon_color = if focused { t.ink } else { t.dim };
        let (text_x, name_cy, status_cy, text_w);
        if r.w > 120.0 {
            // A wide tile: the icon beside two lines.
            g.icon(&icon, focused, r.x + 23.0, r.cy(), 24.0, icon_color);
            text_x = r.x + 44.0;
            text_w = r.w - 52.0;
            name_cy = r.cy() - ty.caption.size * 0.5 - 1.0;
            status_cy = r.cy() + ty.name.size * 0.5 + 1.0;
        } else {
            g.icon(&icon, focused, r.x + 18.0, r.y + 15.0, 18.0, icon_color);
            text_x = r.x + 10.0;
            text_w = r.w - 16.0;
            status_cy = r.bottom() - 6.0 - ty.caption.size * 0.5;
            name_cy = status_cy - ty.caption.size * 0.5 - 2.0 - ty.name.size * 0.5;
        }
        if let Some(severity) = attention {
            let c = severity_color(g, severity);
            g.dot(r.right() - 12.0, r.y + 12.0, 3.5 + 1.5 * pulse, c);
        }
        if self.favourite(i) {
            let x = r.right() - if attention.is_some() { 25.0 } else { 12.0 };
            g.icon("star", true, x, r.y + 12.0, 11.0, t.dim);
        }
        g.text_fit(text_x, name_cy, &name, ty.name, t.ink, text_w);
        g.text_fit(text_x, status_cy, &status, ty.caption, t.dim, text_w);
        focused.then_some(ring)
    }

    /// One widget at `at` (where its motion has it), laid out for `size`
    /// (its cells' size on this board): its tile, lifted when `chosen`, and
    /// what it shows. The ring's place if it has the focus, and when it next
    /// changes by itself.
    #[allow(clippy::too_many_arguments)]
    fn draw_widget(&mut self, g: &mut Gfx, board: Board, w: &Placed, at: Rect, size: (f32, f32), chosen: bool, press: f32, robot: &Robot) -> (Option<Rect>, Option<std::time::Instant>) {
        let t = g.tokens;
        let editing = self.editing.as_ref().filter(|e| e.board == board);
        let in_edit = editing.is_some();
        let armed = editing.is_some_and(|e| e.armed) && chosen;
        let picking = editing.and_then(|e| e.picker.as_ref().filter(|_| chosen).map(|p| (p.from, p.dir, p.slide.value())));
        let animate = self.glide(g) || !g.calm && !g.focus_static;
        let lift_to = match (chosen, in_edit) {
            (true, true) => EDIT_LIFT,
            (true, false) => 1.0,
            _ => 0.0,
        };
        let lift = self.glides.lift(widget_key(w.id), lift_to, animate);
        if !self.glides.get(widget_key(w.id)).lift.is_settled() {
            g.started_moving();
        }
        let pressed = if chosen && !in_edit { press } else { 0.0 };
        let ring = at.inset(-1.5).offset(0.0, -1.0);
        let r = at.inset(-1.5 * lift + 2.0 * pressed).offset(0.0, -lift);
        let radius = (g.tile_radius() + 1.5 * lift - 2.0 * pressed).max(0.0);
        let fill = t.surface1.mix(t.surface2, (lift * 0.7).min(1.0)).mix(t.ink, 0.12 * pressed).mix(g.fault(), if armed { 0.22 } else { 0.0 });
        g.shadow(r, radius, lift.min(1.0));
        g.surface(r, radius, fill);
        // Read only now that it shows, and only what is due.
        let h24 = g.clock.0;
        let turn = !g.battery.saver;
        let kind = w.kind;
        let want = (size.0.round() as usize, size.1.round() as usize);
        self.facts.prepare(kind, robot, h24, want, turn);
        if let Some((from, _, _)) = picking {
            self.facts.prepare(from, robot, h24, want, false);
        }
        let pinned = g.pinned.clone();
        let ctx = crate::widgets::Ctx { facts: &self.facts, robot, pinned: pinned.as_deref(), now: chrono::Local::now(), focused: chosen, radius, laid: size };
        // Laid out for its size, at its top left; clipped to where it is
        // while it grows or shrinks to that. A photo fills its tile as it
        // glides, rising with the lift but not growing with it (the photo is
        // kept as a picture of that size): the lifted tile shows round it.
        let frame = |x: f32, kind: Kind| {
            if kind == Kind::Photos {
                Rect::new(x + at.x - r.x, at.y - lift, at.w, at.h)
            } else {
                Rect::new(x, r.y, size.0.max(r.w), size.1.max(r.h))
            }
        };
        g.canvas.set_clip(r.offset(g.ox, 0.0));
        let next = match picking {
            // The picker turning: the new kind slides in, the last one out.
            Some((from, dir, slide)) if slide.abs() > 0.002 => {
                let old = crate::widgets::content(g, frame(r.x + (slide - dir) * r.w, from), from, w.size, &ctx);
                let new = crate::widgets::content(g, frame(r.x + slide * r.w, kind), kind, w.size, &ctx);
                earlier(old, new)
            }
            _ => crate::widgets::content(g, frame(r.x, kind), kind, w.size, &ctx),
        };
        g.canvas.clear_clip();
        let next = earlier(next, self.facts.due(kind));
        (chosen.then_some(ring), next)
    }

    /// Widgets taken off, closing where they were.
    fn draw_leaving(&mut self, g: &mut Gfx, board: Board, strip: f32) {
        let t = g.tokens;
        let fault = g.fault();
        for l in self.glides.leaving.iter().filter(|l| l.board == board) {
            let k = l.left.value().clamp(0.0, 1.0);
            let r = l.rect.offset(strip, 0.0);
            let s = 0.55 + 0.45 * k;
            let shrunk = Rect::new(r.cx() - r.w * s * 0.5, r.cy() - r.h * s * 0.5, r.w * s, r.h * s);
            g.canvas.rrect(shrunk.offset(g.ox, 0.0), g.tile_radius() * s, t.surface1.mix(fault, 0.18), k);
        }
    }

    /// One app at a time: a large card in the middle, its neighbours peeking
    /// in at either side, smaller and without their words.
    fn draw_carousel(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, bottom) = (g.top(), if g.pager.is_some() { H - 16.0 } else { H - PAD });
        let cy = (top + bottom) * 0.5;
        let (cw, ch) = (150.0, bottom - top - 6.0);
        // A neighbour's near edge sits just clear of the middle card.
        const SMALL: f32 = 0.8;
        let spacing = cw * 0.5 + cw * SMALL * 0.5 + 6.0;
        self.middle = Rect::new((W - cw) * 0.5, cy - ch * 0.5, cw, ch);
        let turn = self.turn.value();
        // Farthest first, so the card nearest the middle is on top.
        let mut order: Vec<usize> = (0..self.count()).filter(|i| (*i as f32 - turn).abs() < 2.2).collect();
        order.sort_by(|a, b| (*b as f32 - turn).abs().total_cmp(&(*a as f32 - turn).abs()));
        for i in order {
            let away = i as f32 - turn;
            // 1 in the middle, 0 a whole step away.
            let near = 1.0 - away.abs().min(1.0);
            let scale = SMALL + (1.0 - SMALL) * near;
            let (w, h) = (cw * scale, ch * scale);
            let r = Rect::new(W * 0.5 + away * spacing - w * 0.5, cy - h * 0.5, w, h);
            g.surface(r, g.tile_radius() * scale + 2.0 * near, if near > 0.5 { t.surface2 } else { t.surface1 });
            let (icon, name) = self.icon_and_name(i);
            let (status, attention) = self.status(i, robot);
            // The icon rises as its card comes to the middle, and the words come
            // in with it. It keeps one size: an icon is drawn from its outline
            // once per size, and a size that changed every frame would be
            // drawn afresh every frame.
            let icon_cy = (r.cy() - 16.0 * near).round();
            g.icon_alpha(&icon, near > 0.5, r.cx().round(), icon_cy, 30.0, if near > 0.5 { t.ink } else { t.dim }, 0.55 + 0.45 * near);
            if near > 0.02 {
                let name = g.text.fit(&name, &ty.title, cw - 18.0);
                let status = g.text.fit(&status, &ty.caption, cw - 18.0);
                let (name_w, status_w) = (g.measure(&name, ty.title), g.measure(&status, ty.caption));
                let name_cy = r.cy() + 14.0;
                g.text_alpha(r.cx() - name_w * 0.5, name_cy, &name, ty.title, t.ink, near);
                g.text_alpha(r.cx() - status_w * 0.5, name_cy + ty.title.size * 0.5 + ty.caption.size * 0.5 + 4.0, &status, ty.caption, t.dim, near);
            }
            if let Some(severity) = attention {
                let c = severity_color(g, severity);
                g.dot(r.right() - 12.0 * scale, r.y + 12.0 * scale, 3.5, c);
            }
        }
        // The ring marks the middle: cards turn through it.
        if !self.on_glance() {
            g.focus_ring(self.middle, g.tile_radius() + 2.0);
        }
    }

    /// The glance pages: the widgets chosen for them, two cells across and
    /// two down a page, to be read without opening anything. Drawn with
    /// `g.ox` at the first glance page; each further page is a panel's
    /// width to its left. When a widget in sight next changes by itself.
    fn draw_glance(&mut self, g: &mut Gfx, robot: &Robot) -> Option<std::time::Instant> {
        let widgets: Vec<Placed> = self.board(Board::Glance).to_vec();
        self.cards.resize(widgets.len(), Rect::default());
        if widgets.is_empty() {
            g.empty("apps", "no widgets yet · e adds one");
            return None;
        }
        let packed = self.glance_board();
        let grid = Grid::of(g, GLANCE_COLUMNS);
        self.grids.1 = Some(grid);
        let animate = self.glide(g);
        let press = g.press();
        let editing = self.editing.as_ref().is_some_and(|e| e.board == Board::Glance);
        let mut next = None;
        let mut ring = None;
        // The widget chosen while editing is lifted over the rest: drawn last.
        let lifted = self.editing.as_ref().filter(|e| e.board == Board::Glance).and_then(|e| e.selected);
        let mut held = None;
        for &(slot, cell) in &packed.slots {
            let Slot::Widget(j) = slot else { continue };
            // Pages run leftward: the second is a panel's width left of the first.
            let along = grid.rect(&cell).offset(-(cell.page as f32) * W, 0.0);
            let at = self.glides.place(widget_key(widgets[j].id), along, animate);
            self.cards[j] = along.offset(g.ox, 0.0);
            if at.right() + g.ox <= 0.0 || at.x + g.ox >= W {
                continue;
            }
            if lifted == Some(j) {
                held = Some((j, at));
                continue;
            }
            let chosen = !editing && self.on_glance() && self.card == j;
            let (r, when) = self.draw_widget(g, Board::Glance, &widgets[j], at, grid.size_of(widgets[j].size), chosen, press, robot);
            next = earlier(next, when);
            if r.is_some() {
                ring = r;
            }
        }
        self.draw_leaving(g, Board::Glance, 0.0);
        if let Some((j, at)) = held {
            let (r, when) = self.draw_widget(g, Board::Glance, &widgets[j], at, grid.size_of(widgets[j].size), true, press, robot);
            next = earlier(next, when);
            ring = r.or(ring);
        }
        if let Some(r) = ring {
            ring_on(g, r, None, editing);
        }
        next
    }

    // ---- editing (widgets::edit has the keys) ----

    /// Columns on a board.
    fn board_columns(&self, board: Board) -> usize {
        match board {
            Board::Glance => GLANCE_COLUMNS,
            Board::Home => self.columns,
        }
    }

    /// Apps on a board.
    fn board_apps(&self, board: Board) -> usize {
        match board {
            Board::Glance => 0,
            Board::Home => self.count(),
        }
    }

    /// A board packed as it is drawn.
    fn packed_board(&self, board: Board) -> Packed {
        match board {
            Board::Glance => self.glance_board(),
            Board::Home => self.grid(),
        }
    }

    /// A board's measures as last drawn (the standard text size's until then).
    fn last_grid(&self, board: Board) -> Grid {
        let drawn = match board {
            Board::Home => self.grids.0,
            Board::Glance => self.grids.1,
        };
        drawn.unwrap_or_else(|| {
            let columns = self.board_columns(board);
            let top = 28.0;
            Grid { columns, top, cell_w: (W - 2.0 * PAD - GAP * (columns - 1) as f32) / columns as f32, cell_h: (H - 16.0 - top - GAP) / 2.0 }
        })
    }

    /// Start editing a board (Home's grid only in the grid style: elsewhere
    /// the glance page), the focused widget chosen, or the nearest; with
    /// none on it, the picker opens.
    fn begin_edit(&mut self, board: Board, cx: &mut Cx) -> Nav {
        let calm = cx.settings.calm;
        self.follow_layout(&cx.settings.widgets);
        let board = if board == Board::Home && cx.settings.home != HomeStyle::Grid { Board::Glance } else { board };
        self.opening = None;
        if board == Board::Glance && !self.on_glance() {
            self.show_glance(true, calm);
        }
        let list = self.layout.board(board).clone();
        let selected = match board {
            Board::Glance => (!list.is_empty()).then(|| self.card.min(list.len() - 1)),
            Board::Home => self.widget.filter(|j| *j < list.len()).or_else(|| {
                // The widget nearest the focused app.
                let packed = self.grid();
                let here = packed.cell(Slot::App(self.focus))?;
                let distance = |c: &Cell| (c.page as i32 - here.page as i32).abs() * 100 + (c.row as i32 - here.row as i32).abs() * 10 + (c.col as i32 - here.col as i32).abs();
                packed.slots.iter().filter_map(|(s, c)| if let Slot::Widget(j) = s { Some((*j, distance(c))) } else { None }).min_by_key(|(_, d)| *d).map(|(j, _)| j)
            }),
        };
        // They jiggle once, one after another: these can be moved.
        if !calm {
            let packed = Packed::of(self.board_apps(board), &list, self.board_columns(board));
            let order: Vec<u64> = packed.reading().into_iter().filter_map(|(s, _)| if let Slot::Widget(j) = s { Some(widget_key(list[j].id)) } else { None }).collect();
            self.glides.jiggle(&order);
        }
        self.editing = Some(Editing::new(board, list, selected));
        if selected.is_none() {
            self.open_picker(cx);
        }
        self.follow_selection(calm);
        Nav::Stay
    }

    /// Done editing: what was chosen keeps the focus.
    fn end_edit(&mut self, cx: &mut Cx) {
        let Some(e) = self.editing.take() else { return };
        if e.picker.is_some() {
            // The widget being chosen was never added: it closes.
            self.leave_picker_widget(&e);
        }
        *cx.settings.widgets.board_mut(e.board) = e.kept();
        self.layout = cx.settings.widgets.clone();
        crate::save_settings_soon(cx.settings);
        let selected = e.selected.filter(|_| e.picker.is_none()).filter(|j| *j < self.layout.board(e.board).len());
        match e.board {
            Board::Glance => self.card = selected.unwrap_or(self.card).min(self.layout.glance.len().saturating_sub(1)),
            Board::Home => self.widget = selected,
        }
        self.gliding = true;
    }

    /// Keep what is being edited: in the settings, and on the card (written
    /// on a thread of its own, so a move is never held up by the card).
    fn commit(&mut self, cx: &mut Cx) {
        let Some(e) = &self.editing else { return };
        *cx.settings.widgets.board_mut(e.board) = e.kept();
        self.layout = cx.settings.widgets.clone();
        crate::save_settings_soon(cx.settings);
    }

    /// The picker's widget closing where it was (the picker put away).
    fn leave_picker_widget(&mut self, e: &Editing) {
        let Some(j) = e.selected else { return };
        let m = self.glides.get(widget_key(e.list[j].id));
        let rect = Rect::new(m.x.value(), m.y.value(), m.w.value(), m.h.value());
        let mut left = Animated::new(1.0, role::smooth()).with_rest(0.01);
        left.set(0.0);
        self.glides.leaving.push(crate::widgets::edit::Leaving { rect, board: e.board, left });
    }

    /// The picker, in place: a live widget where the new one will go (after
    /// the one chosen; on Home with none, where the focused app is), of the
    /// first kind not on the board yet.
    fn open_picker(&mut self, cx: &mut Cx) {
        let Some(board) = self.editing.as_ref().map(|e| e.board) else { return };
        let (apps, focus) = (self.board_apps(board), self.focus);
        let Some(e) = &mut self.editing else { return };
        if Kind::ALL.iter().all(|k| e.list.iter().any(|w| w.kind == *k)) {
            *cx.say = Some("every widget is on this board".into());
            return;
        }
        let seq = crate::widgets::sequence(apps, &e.list);
        let before = match e.selected {
            Some(j) => seq.iter().position(|s| *s == Slot::Widget(j)).and_then(|at| seq.get(at + 1).copied()),
            None if board == Board::Home => Some(Slot::App(focus)),
            None => None,
        };
        let at = crate::widgets::edit::first_new(&e.list, 0);
        let kind = Kind::ALL[at];
        let (list, k) = crate::widgets::edit::insert(apps, &e.list, before, Placed::new(kind, kind.usual(), 0));
        let id = list[k].id;
        e.list = list;
        e.selected = Some(k);
        e.picker = Some(crate::widgets::edit::Picker::new(at));
        e.armed = false;
        // It grows in where it will go, and the rest glide to make room.
        if !cx.settings.calm {
            if let Some(cell) = self.packed_board(board).cell(Slot::Widget(k)) {
                let rect = self.last_grid(board).rect(&cell);
                let along = match board {
                    Board::Home => rect.offset(cell.page as f32 * W, 0.0),
                    Board::Glance => rect.offset(-(cell.page as f32) * W, 0.0),
                };
                self.glides.grow_from(widget_key(id), along);
            }
        }
        self.gliding = true;
    }

    /// The board's page turns to show what is chosen.
    fn follow_selection(&mut self, calm: bool) {
        let Some(e) = &self.editing else { return };
        let Some(j) = e.selected else { return };
        let board = e.board;
        let Some(cell) = self.packed_board(board).cell(Slot::Widget(j)) else { return };
        match board {
            Board::Home => {
                let page = cell.page as f32;
                if calm {
                    self.page.jump(page);
                } else {
                    self.page.set(page);
                }
            }
            Board::Glance => {
                if cell.page + 1 != self.glance_page {
                    self.glance_to(cell.page + 1, calm);
                }
            }
        }
    }

    /// Keys while a board is edited (`widgets::edit` says what each does).
    fn edit_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let calm = cx.settings.calm;
        if self.editing.as_ref().is_some_and(|e| e.picker.is_some()) {
            return self.picker_key(key, cx);
        }
        let Some(board) = self.editing.as_ref().map(|e| e.board) else { return Nav::Stay };
        let (apps, columns) = (self.board_apps(board), self.board_columns(board));
        let Some(e) = &mut self.editing else { return Nav::Stay };
        let armed = std::mem::take(&mut e.armed);
        match key {
            Key::Back | Key::Enter | Key::Char('e') | Key::Leave => {
                self.end_edit(cx);
                return Nav::Stay;
            }
            Key::Left | Key::Right | Key::Up | Key::Down => {
                let Some(j) = e.selected else { return Nav::Stay };
                let dir = match key {
                    Key::Left => Dir::Left,
                    Key::Right => Dir::Right,
                    Key::Up => Dir::Up,
                    _ => Dir::Down,
                };
                match crate::widgets::edit::step(apps, &e.list, j, dir, columns) {
                    Some((list, k)) => {
                        e.list = list;
                        e.selected = Some(k);
                        self.gliding = true;
                        self.commit(cx);
                    }
                    None if !calm => {
                        // No further that way: it gives a little and comes back.
                        let push = match dir {
                            Dir::Left => (-90.0, 0.0),
                            Dir::Right => (90.0, 0.0),
                            Dir::Up => (0.0, -70.0),
                            Dir::Down => (0.0, 70.0),
                        };
                        let m = self.glides.get(widget_key(e.list[j].id));
                        m.x.nudge(push.0);
                        m.y.nudge(push.1);
                    }
                    None => {}
                }
            }
            Key::Char('r') => {
                if let Some(j) = e.selected {
                    e.list[j].size = e.list[j].size.next();
                    let said = format!("{} · {}", e.list[j].kind.name().to_lowercase(), e.list[j].size.key());
                    self.gliding = true;
                    self.commit(cx);
                    *cx.say = Some(said);
                }
            }
            Key::Char('d') | Key::Delete => {
                if let Some(j) = e.selected {
                    if armed {
                        let gone = e.list.remove(j);
                        // The next one along is chosen, else the one before.
                        e.selected = if e.list.is_empty() { None } else { Some(j.min(e.list.len() - 1)) };
                        let m = self.glides.get(widget_key(gone.id));
                        let rect = Rect::new(m.x.value(), m.y.value(), m.w.value(), m.h.value());
                        let mut left = Animated::new(1.0, role::smooth()).with_rest(0.01);
                        if calm {
                            left.jump(0.0);
                        } else {
                            left.set(0.0);
                        }
                        self.glides.leaving.push(crate::widgets::edit::Leaving { rect, board, left });
                        self.gliding = true;
                        self.commit(cx);
                        *cx.say = Some(format!("{} taken off", gone.kind.name().to_lowercase()));
                    } else {
                        e.armed = true;
                        *cx.say = Some(format!("d again takes {} off", e.list[j].kind.name().to_lowercase()));
                    }
                }
            }
            Key::Char('a') | Key::Char('+') => self.open_picker(cx),
            Key::Tab => {
                if let Some(j) = e.selected {
                    let packed = Packed::of(apps, &e.list, columns);
                    let order: Vec<usize> = packed.reading().into_iter().filter_map(|(s, _)| if let Slot::Widget(k) = s { Some(k) } else { None }).collect();
                    let at = order.iter().position(|k| *k == j).unwrap_or(0);
                    e.selected = order.get((at + 1) % order.len().max(1)).copied();
                }
            }
            _ => {}
        }
        self.follow_selection(calm);
        Nav::Stay
    }

    /// Keys while the picker is open.
    fn picker_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let calm = cx.settings.calm;
        let Some(e) = &mut self.editing else { return Nav::Stay };
        let Some(j) = e.selected else { return Nav::Stay };
        match key {
            Key::Left | Key::Right => {
                // Only kinds not on this board yet are offered.
                let taken: Vec<Kind> = e.list.iter().enumerate().filter(|(i, _)| *i != j).map(|(_, w)| w.kind).collect();
                if let Some(picker) = e.picker.as_mut() {
                    picker.turn(if key == Key::Left { -1 } else { 1 }, calm, &taken);
                    e.list[j].kind = picker.kind();
                }
            }
            Key::Up | Key::Down | Key::Char('r') => {
                let size = e.list[j].size;
                let to = match key {
                    Key::Up => size.step(true),
                    Key::Down => size.step(false),
                    _ => size.next(),
                };
                if to != size {
                    e.list[j].size = to;
                    self.gliding = true;
                } else if !calm {
                    // No bigger, or no smaller: it gives a little.
                    let m = self.glides.get(widget_key(e.list[j].id));
                    m.y.nudge(if key == Key::Up { -60.0 } else { 60.0 });
                }
            }
            Key::Enter => {
                let name = e.list[j].kind.name().to_lowercase();
                e.picker = None;
                self.gliding = true;
                self.commit(cx);
                *cx.say = Some(format!("{name} added"));
            }
            Key::Back | Key::Char('e') | Key::Leave => {
                // Back to editing the board as it was: the new one closes.
                let Some(e) = self.editing.take() else { return Nav::Stay };
                self.leave_picker_widget(&e);
                let kept = e.kept();
                let empty = kept.is_empty();
                let selected = (!empty).then(|| j.saturating_sub(1).min(kept.len() - 1));
                self.editing = Some(Editing::new(e.board, kept, selected));
                self.gliding = true;
                if empty || key == Key::Leave {
                    // Nothing on the board to edit: done.
                    self.end_edit(cx);
                }
            }
            _ => {}
        }
        self.follow_selection(calm);
        Nav::Stay
    }
}

#[cfg(test)]
impl Home {
    /// When a widget in sight next changes by itself, as last drawn.
    pub fn next_change_for_tests(&self) -> Option<std::time::Instant> {
        self.next_change
    }

    /// As if that time had come.
    pub fn pretend_due(&mut self) {
        self.next_change = Some(std::time::Instant::now());
    }
}

/// The focus ring round `r`: on Home's grid riding the strip of pages
/// (`along` it), on the glance page as on any screen. While a board is
/// edited it is part of the widget chosen, which it never lags: drawn where
/// the widget is, its own glide left where it was until editing ends, when
/// it glides on from there.
fn ring_on(g: &mut Gfx, r: Rect, along: Option<f32>, editing: bool) {
    let radius = g.tile_radius() + 1.5;
    let still = g.focus_static;
    g.focus_static = still || editing;
    match along {
        Some(along) => g.focus_ring_on(r, radius, along),
        None => g.focus_ring(r, radius),
    }
    g.focus_static = still;
}

/// The earlier of two times, either perhaps none.
fn earlier(a: Option<std::time::Instant>, b: Option<std::time::Instant>) -> Option<std::time::Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What a board has at the left or right end of a row on a page (else
/// whatever is on that page): where Left or Right lands coming onto it.
fn edge_of(packed: &Packed, page: usize, row: usize, rightmost: bool) -> Option<Slot> {
    let on_page: Vec<&(Slot, Cell)> = packed.slots.iter().filter(|(_, c)| c.page == page).collect();
    let in_row: Vec<&(Slot, Cell)> = on_page.iter().copied().filter(|(_, c)| (c.row..c.row + c.h).contains(&row)).collect();
    let pool = if in_row.is_empty() { on_page } else { in_row };
    let pick = if rightmost { pool.iter().max_by_key(|(_, c)| (c.col + c.w, c.row)) } else { pool.iter().min_by_key(|(_, c)| (c.col, c.row)) };
    pick.map(|(s, _)| *s)
}

impl Screen for Home {
    fn spot(&self) -> Option<Rect> {
        if self.on_glance() {
            return None;
        }
        match self.style {
            HomeStyle::Grid => match self.slot() {
                Slot::Widget(j) => self.widget_rects.get(j).copied().filter(|r| r.w > 0.0),
                Slot::App(i) => self.rects.get(i).copied().filter(|r| r.w > 0.0),
            },
            HomeStyle::Carousel => Some(self.middle).filter(|r| r.w > 0.0),
            HomeStyle::Flow => Some(if self.flow.open_on(self.focus) { self.flow.enlarged() } else { self.flow.middle() }).filter(|r| r.w > 0.0),
        }
    }

    fn title(&self) -> &str {
        match &self.editing {
            Some(e) if e.picker.is_some() => "Add",
            Some(_) => "Widgets",
            None if self.on_glance() => "Glance",
            None => "Catalyst",
        }
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        match &self.editing {
            Some(e) if e.picker.is_some() => {
                return vec![keys("adding a widget", &[("← →", "Choose one"), ("↑ ↓ · r", "Its size"), ("enter", "Add it"), ("e", "Don't add it")])];
            }
            Some(_) => {
                return vec![keys(
                    "editing widgets",
                    &[("arrows", "Move the widget"), ("tab", "Next widget"), ("r", "Its size"), ("d twice", "Take it off"), ("a · +", "Add a widget"), ("enter · e", "Done")],
                )];
            }
            None => {}
        }
        let search = keys("also", &[("space · /", "Search"), ("e", "Edit the widgets"), ("↑ at the top", "Control centre"), ("esc", "Back to the start")]);
        if self.on_glance() {
            return vec![keys("glance", &[("enter", "Open the widget"), ("arrows", "Move"), ("tab", "Next widget"), ("→ at the end", "The apps")]), search];
        }
        let apps = match self.style {
            HomeStyle::Grid => keys("apps", &[("enter", "Open"), ("arrows", "Move"), ("tab", "Next"), ("← on the first", "Glance page")]),
            HomeStyle::Carousel => keys("apps", &[("enter", "Open"), ("← →", "Turn"), ("tab", "Next"), ("pg up · pg dn", "Skip three"), ("← on the first", "Glance page")]),
            HomeStyle::Flow if self.flow.is_open() => keys("apps", &[("enter", "Open"), ("esc", "Turn it back"), ("← →", "Turn the flow"), ("tab", "Next")]),
            HomeStyle::Flow => keys("apps", &[("enter", "Turn the card over"), ("← →", "Turn the flow"), ("tab", "Next"), ("pg up · pg dn", "Skip six"), ("home · end", "First or last")]),
        };
        vec![apps, search]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let calm = cx.settings.calm;
        self.follow_arrangement(&cx.settings.arrangement());
        self.follow_layout(&cx.settings.widgets);
        // Editing takes every key.
        if self.editing.is_some() {
            return self.edit_key(key, cx);
        }
        // The style in the settings, which a draw may not have caught up with yet.
        let style = cx.settings.home;
        // e edits the widgets, on the board in view (widgets::edit says why e).
        if key == Key::Char('e') {
            let board = if self.on_glance() || style != HomeStyle::Grid { Board::Glance } else { Board::Home };
            return self.begin_edit(board, cx);
        }
        // Up from the top of Home pulls the control centre down.
        let at_top = if self.on_glance() {
            self.glance_board().cell(Slot::Widget(self.card)).is_none_or(|c| c.row == 0)
        } else {
            style.one_at_a_time() || self.grid().cell(self.slot()).is_none_or(|c| c.row == 0)
        };
        if key == Key::Up && at_top {
            return Nav::Control;
        }
        // Space or / brings Spotlight up, as Ctrl-Enter does anywhere; the
        // bare letters stay arrows here.
        if key == Key::Char(' ') || key == Key::Char('/') {
            return Nav::Spotlight;
        }
        // Flow: Esc first turns an enlarged card back, the flow staying where it is.
        if key == Key::Back && style == HomeStyle::Flow && !self.on_glance() && self.flow.is_open() {
            self.opening = None;
            self.flow.close(calm);
            return Nav::Stay;
        }
        // Esc anywhere on Home: back to its start, the first app of the first
        // page (or card), gliding there as a page turn does.
        if key == Key::Back {
            self.opening = None;
            self.to_start(calm);
            return Nav::Stay;
        }
        if self.on_glance() {
            return self.glance_key(key, cx);
        }
        // Any key but Enter changes its mind about a tile still pressed in.
        if key != Key::Enter {
            self.opening = None;
        }
        // Flow: Enter first brings the card forward and turns it over; only
        // Enter on the card turned over opens the app.
        if key == Key::Enter && style == HomeStyle::Flow && !self.flow.open_on(self.focus) {
            self.flow.enlarge(self.focus, calm);
            return Nav::Stay;
        }
        if style == HomeStyle::Flow && key != Key::Enter {
            self.flow_key(key, calm);
            return Nav::Stay;
        }
        if key == Key::Enter {
            // Pressed in for an instant, then the app grows out of the tile.
            // (At once in the calm version, and while a window is still on
            // its way back into its tile: that brings the same one back.)
            if calm || !cx.at_rest {
                return self.open(cx);
            }
            self.opening.get_or_insert(0.0);
            return Nav::Stay;
        }
        let pushed = match style {
            HomeStyle::Grid => self.grid_key(key, calm),
            HomeStyle::Carousel | HomeStyle::Flow => self.carousel_key(key, calm),
        };
        let (page, turn) = (self.grid_page() as f32, self.focus as f32);
        if calm {
            self.page.jump(page);
            self.turn.jump(turn);
        } else {
            self.page.set(page);
            self.turn.set(turn);
            // Past the last app: a push against the end, which gives and returns.
            if pushed {
                self.page.nudge(1.6);
                self.turn.nudge(7.0);
            }
        }
        Nav::Stay
    }

    fn appear(&mut self) {
        // The launcher cascade, now that Home can be seen.
        self.arrived = 0.0;
    }

    fn parallax(&self) -> Option<f32> {
        // Home is one strip: the glance pages on the left, then the grid's
        // pages (or the carousel's cards). Where it is turned to, 0 to 1.
        let glance_pages = self.glance_pages() as f32;
        let (at, stops) = match self.style {
            HomeStyle::Grid => (self.page.value(), self.grid().pages as f32),
            HomeStyle::Carousel => (self.turn.value(), self.count() as f32),
            HomeStyle::Flow => (self.flow.pos.value(), self.count() as f32),
        };
        let place = at - self.glance.value() + glance_pages;
        Some((place / (glance_pages + stops - 1.0).max(1.0)).clamp(0.0, 1.0))
    }

    fn robot_linked(&mut self, calm: bool) -> Nav {
        let robot = || Box::new(RobotScreen::default());
        // Not while widgets are being edited: the arrival is said instead.
        if self.editing.is_some() {
            return Nav::Stay;
        }
        // On the glance page: out of the robot's widget if it is on this page.
        if self.on_glance() {
            let packed = self.glance_board();
            let card = self
                .board(Board::Glance)
                .iter()
                .enumerate()
                .find(|(j, w)| w.kind == Kind::Robot && packed.cell(Slot::Widget(*j)).is_some_and(|c| c.page + 1 == self.glance_page))
                .and_then(|(j, _)| self.cards.get(j))
                .copied()
                .filter(|r| r.w > 0.0);
            return match card {
                Some(from) => Nav::Open(robot(), from),
                None => Nav::Push(robot()),
            };
        }
        let Some(at) = self.tiles.iter().position(|t| matches!(t, Tile::Builtin(i) if APPS[*i].name == "Robot")) else {
            return Nav::Push(robot());
        };
        // An enlarged card in the flow turns back first.
        self.flow.close(calm);
        if self.focus != at || self.widget.is_some() {
            // Turn to Robot's tile first, so the app comes out of it and
            // goes back into it.
            self.focus = at;
            self.widget = None;
            let (page, turn) = (self.page_of(Slot::App(at)) as f32, at as f32);
            if calm {
                self.page.jump(page);
                self.turn.jump(turn);
            } else {
                self.page.set(page);
                self.turn.set(turn);
            }
            self.flow.turn_to(&mut self.focus, at, calm);
            return Nav::Stay;
        }
        let flow_still = self.style != HomeStyle::Flow || self.flow.settled();
        if !(self.page.is_settled() && self.turn.is_settled() && self.glance.is_settled() && flow_still) {
            return Nav::Stay;
        }
        let from = match self.style {
            HomeStyle::Grid => self.rects.get(at).copied().unwrap_or_default(),
            HomeStyle::Carousel => self.middle,
            HomeStyle::Flow => self.flow.middle(),
        };
        // Not drawn there yet: ask again after the next frame.
        let on_screen = from.w > 0.0 && from.x >= 0.0 && from.right() <= W;
        if on_screen {
            Nav::Open(robot(), from)
        } else {
            Nav::Stay
        }
    }

    fn resume(&mut self) {
        // Back on Home: an app may have been added or removed meanwhile.
        let focused = self.tiles.get(self.focus).map(|t| self.tile_name(*t).to_owned());
        self.external = crate::apps::find();
        let arrangement = self.arranged_by.clone();
        self.arrange_around(&arrangement, focused);
        let focus = self.focus.min(self.count().saturating_sub(1));
        if focus != self.focus {
            // The focused app went: its page or card may be gone with it.
            self.focus = focus;
            self.page.jump(self.page_of(Slot::App(focus)) as f32);
            self.turn.jump(focus as f32);
            self.flow.place(focus);
        }
        // Back from an app opened out of an enlarged card: once the app has
        // gone back into it, the card turns back into the flow.
        if self.style == HomeStyle::Flow && self.flow.is_open() {
            self.flow.close_on_return = true;
        }
        // What an app may have changed (a note, a reminder, a photo) is read again.
        self.facts.invalidate();
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        self.follow_layout(&cx.settings.widgets);
        // Settings › Home › Widgets sent Home here to edit a board.
        if let Some(board) = cx.settings.edit_widgets.take() {
            if self.editing.is_some() {
                self.end_edit(cx);
            }
            return self.begin_edit(board, cx);
        }
        match self.opening {
            Some(pressed) if pressed >= PRESS_IN => {
                self.opening = None;
                self.open(cx)
            }
            _ => Nav::Stay,
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        // Each spring must be advanced, so no short-circuit.
        let mut moving = self.page.tick(dt) | self.turn.tick(dt) | self.glance.tick(dt) | self.glides.tick(dt) | self.facts.tick(dt);
        if let Some(picker) = self.editing.as_mut().and_then(|e| e.picker.as_mut()) {
            moving |= picker.slide.tick(dt);
        }
        if self.style == HomeStyle::Flow {
            moving |= self.flow.tick(dt, self.focus);
        }
        if let Some(pressed) = &mut self.opening {
            *pressed += dt.max(0.001);
            moving = true;
        }
        for spring in self.lift.iter_mut().chain(self.pulse.iter_mut()) {
            moving |= spring.tick(dt);
        }
        if self.arrived < CASCADE_OVER {
            self.arrived += dt;
            moving = true;
        }
        // Rearranged a moment ago, and settled now: the tiles hold still.
        if self.gliding && self.editing.is_none() && !self.glides.moving() {
            self.gliding = false;
        }
        // A widget in sight changes by itself now (a minute turned, a
        // countdown's second): one frame for it. Asked once; with the screen
        // off the shell draws nothing, and nothing more is asked until it has.
        if let Some(at) = self.next_change {
            if !self.asked && std::time::Instant::now() >= at {
                self.asked = true;
                moving = true;
            }
        }
        moving
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let arrangement = g.arrangement.clone();
        self.follow_arrangement(&arrangement);
        self.follow_layout(&g.widgets);
        // The battery widget shows the shell's own reading.
        if self.facts.battery != g.charge {
            self.facts.battery.clone_from(&g.charge);
        }
        // Two larger tiles across when chosen, and always at the large text size.
        let columns = if g.text_size == TextSize::Large { 2 } else { g.columns.clamp(2, 3) };
        if columns != self.columns {
            // Every place is counted afresh.
            self.glides.forget();
        }
        if columns != self.columns || g.home != self.style {
            // A different Home: everything is where the focus is, at once.
            let first = self.arrived == 0.0;
            let was = self.style;
            self.columns = columns;
            self.style = g.home;
            // Home's widgets live in the grid: elsewhere, an app has the focus.
            if self.style != HomeStyle::Grid {
                self.widget = None;
            }
            self.page.jump(self.grid_page() as f32);
            self.turn.jump(self.focus as f32);
            if self.style == HomeStyle::Carousel {
                // No cascade here. Arriving, the carousel turns into place instead.
                self.arrived = CASCADE_OVER;
                if first && !g.calm && !g.focus_static {
                    self.turn.jump(self.focus as f32 + 0.9);
                    self.turn.set(self.focus as f32);
                    g.started_moving();
                }
            }
            if self.style == HomeStyle::Flow && was != HomeStyle::Flow {
                // Nor here: arriving, the covers sweep in from the right.
                self.arrived = CASCADE_OVER;
                self.flow.close(true);
                self.flow.place(self.focus);
                self.flow.arrive_when_ready = first && !g.calm && !g.focus_static;
            }
            if was == HomeStyle::Flow && self.style != HomeStyle::Flow {
                // Its pictures are let go.
                self.flow.forget();
            }
        }
        // The pager counts the glance pages first (the farthest leftmost),
        // then the grid's pages or the carousel's apps. Widgets taken away
        // meanwhile: never left on a page, or a widget, that is no longer there.
        let glance_pages = self.glance_pages();
        if self.glance_page > glance_pages {
            self.glance_page = glance_pages;
            self.glance.jump(glance_pages as f32);
        }
        self.card = self.card.min(self.board(Board::Glance).len().saturating_sub(1));
        let stops = match self.style {
            HomeStyle::Grid => self.grid().pages,
            HomeStyle::Carousel | HomeStyle::Flow => self.count(),
        };
        let at = match self.style {
            HomeStyle::Grid => self.grid_page(),
            HomeStyle::Carousel | HomeStyle::Flow => self.focus,
        };
        let current = if self.on_glance() { glance_pages - self.glance_page } else { glance_pages + at };
        if self.editing.is_some() {
            // Editing: what its keys do goes along the foot, where the pager rests.
            g.pager = None;
        } else if stops + glance_pages < 24 {
            g.pager = Some((stops + glance_pages, current, g.ox));
        } else if self.style == HomeStyle::Flow {
            // Too many for dots: the flow's place along its track, as it moves.
            let total = (stops + glance_pages).saturating_sub(1).max(1) as f32;
            let place = glance_pages as f32 - self.glance.value() + crate::flow::rubber(self.flow.pos.value(), self.count()).clamp(0.0, (stops - 1) as f32);
            g.pager = Some((stops + glance_pages, current, g.ox));
            g.pager_at = Some((place / total).clamp(0.0, 1.0));
        }
        // The apps and the glance pages are one strip: the glance pages to the left.
        let base = g.ox;
        let shift = self.glance.value() * W;
        let mut next = None;
        if shift < W - 0.5 {
            g.ox = base + shift;
            match self.style {
                HomeStyle::Grid => next = self.draw_grid(g, robot),
                HomeStyle::Carousel => self.draw_carousel(g, robot),
                HomeStyle::Flow => self.draw_flow(g, robot),
            }
        }
        // The glance pages, the first just left of the apps.
        if shift > 0.5 {
            g.ox = base + shift - W;
            next = earlier(next, self.draw_glance(g, robot));
        }
        g.ox = base;
        if let Some(e) = &self.editing {
            g.hint_any(if e.picker.is_some() { &PICK_HINTS } else { &EDIT_HINTS });
        }
        if self.glides.moving() || self.editing.as_ref().is_some_and(|e| e.picker.as_ref().is_some_and(|p| !p.slide.is_settled())) {
            g.started_moving();
        }
        self.next_change = next;
        self.asked = false;
    }
}

// ----------------------------------------------------------------- robot ---

/// The robot itself, in three dimensions, each part coloured by what it
/// reports, with its mode, link, battery and what needs attention around
/// it (`robot3d`); the Systemcore's vital signs beside it; the robot's
/// address is set here.
#[derive(Default)]
pub struct RobotScreen {
    editing: Option<String>,
    hero: crate::robot3d::Hero,
    /// As last drawn: the calm version, and whether a robot is linked (it
    /// turns only then).
    calm: bool,
    linked: bool,
}

/// A team number becomes the Systemcore's static address, 10.TE.AM.2.
pub fn resolve_address(input: &str) -> String {
    match input.parse::<u32>() {
        Ok(team) if input.len() <= 5 => format!("10.{}.{}.2", team / 100, team % 100),
        _ => input.to_owned(),
    }
}

impl Screen for RobotScreen {
    fn title(&self) -> &str {
        "Robot"
    }

    fn wants_text(&self) -> bool {
        self.editing.is_some()
    }

    /// Copy: the address being typed, else the robot's.
    fn copy(&mut self, robot: &Robot) -> Option<String> {
        self.editing.clone().or_else(|| Some(robot.address.clone()))
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.editing.is_some() {
            return vec![keys("the address", &[("enter", "Connect"), ("esc", "Cancel"), ("type demo", "A pretend robot")])];
        }
        vec![keys("robot", &[("enter", "Robot address"), ("→", "The field")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(text) = &mut self.editing {
            match key {
                Key::Char(c) if text.len() < 32 && (c.is_ascii_alphanumeric() || c == '.' || c == '-') => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Back => self.editing = None,
                Key::Enter => {
                    let input = self.editing.take().unwrap_or_default();
                    if input.is_empty() {
                        return Nav::Stay;
                    }
                    let address = resolve_address(&input);
                    *cx.robot = if input == "demo" { Robot::demo() } else { Robot::connect(&address) };
                    let saved = crate::save_robot_address(&input);
                    *cx.say = Some(if saved { format!("robot set to {address}") } else { format!("using {address}, not saved") });
                }
                _ => {}
            }
            return Nav::Stay;
        }
        match key {
            Key::Enter => self.editing = Some(String::new()),
            Key::Back => return Nav::Pop,
            // The field, with the robot where its pose says.
            Key::Right | Key::Char('f') => return Nav::Push(Box::new(crate::field::FieldPage::default())),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        // It turns slowly on its stand while a robot is linked.
        self.hero.tick(dt, self.calm, self.linked)
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = g.above_hint();
        (self.calm, self.linked) = (g.calm, robot.linked());

        // The robot on its stage: the team's own if one is installed, each
        // part as the robot reports it, its mode, link and battery around it.
        let stage = Rect::new(PAD, top, 166.0, bottom - top);
        g.surface(stage, g.tile_radius(), t.surface1);
        let object = self.hero.object(g);
        let parts: Vec<String> = g.objects.parts(object).into_iter().map(|(name, _)| name).collect();
        let state = crate::robot3d::read(robot, &parts);
        g.canvas.set_clip(stage.offset(g.ox, 0.0));
        let size = if g.objects.px(object, "hero").is_some() { "hero" } else { "m" };
        self.hero.draw(g, object, size, stage.cx(), stage.cy() + 4.0, &state, 1.0);
        g.canvas.clear_clip();
        crate::robot3d::draw_captions(g, stage, &state, robot);

        // Beside it: the code on the robot and the Systemcore's vital
        // signs, and the address, the one thing set on this screen.
        let x = stage.right() + GAP;
        let w = W - PAD - x;
        let field_h = ty.caption.size + ty.label.size + 14.0;
        let field = Rect::new(x, bottom - field_h, w, field_h);
        g.surface(field, g.tile_radius(), if self.editing.is_some() { t.surface2 } else { t.surface1 });
        g.text_at(field.x + 9.0, field.y + 5.0 + ty.caption.size * 0.5, Align::Left, "address", ty.caption, t.dim);
        let value_cy = field.bottom() - 5.0 - ty.label.size * 0.5;
        match &self.editing {
            Some(text) => {
                let shown = g.text.fit(text, &ty.label, field.w - 22.0);
                let w = g.text_at(field.x + 9.0, value_cy, Align::Left, &shown, ty.label, t.ink);
                g.surface(Rect::new(field.x + 10.0 + w, value_cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
                g.focus_ring(field, g.tile_radius());
            }
            None => {
                let shown = if robot.address.is_empty() { "not set".to_owned() } else { robot.address.clone() };
                g.text_fit(field.x + 9.0, value_cy, &shown, ty.label, t.ink, field.w - 18.0);
            }
        }
        let can = robot.get("Systemcore/CanUtilization").and_then(|v| v.as_f64s().map(|u| u.iter().cloned().fold(0.0, f64::max)));
        let cpu = robot.f64("Systemcore/CpuPercent");
        let temp = robot.f64("Systemcore/TempCelsius");
        let rows: [(&str, String, Option<f32>); 4] = [
            // The code on the robot: its commit, and against the repository
            // marked as the robot's code, how far behind it is.
            ("code", dash(robot.string("Robot/Software/RobotCodeVersion").map(|v| crate::git::robot_code_line(&v).unwrap_or_else(|| v.chars().take(9).collect()))), None),
            ("cpu", dash(cpu.map(|v| format!("{v:.0}%"))), cpu.map(|v| v as f32 / 100.0)),
            ("temp", dash(temp.map(|v| format!("{v:.0} °c"))), temp.map(|v| v as f32 / 90.0)),
            ("can", dash(can.map(|v| format!("{:.0}%", v * 100.0))), can.map(|v| v as f32)),
        ];
        let gap = 3.0;
        let rows_bottom = field.y - GAP;
        let row_h = (rows_bottom - top - gap * 3.0) / 4.0;
        let label_w = g.measure("temp", ty.label) + 16.0;
        let value_w = g.measure("100 °c", ty.label) + 14.0;
        for (i, (label, value, fraction)) in rows.iter().enumerate() {
            let r = Rect::new(x, top + i as f32 * (row_h + gap), w, row_h);
            g.surface(r, (row_h * 0.5).min(11.0), t.surface1);
            g.text_at(r.x + 9.0, r.cy(), Align::Left, label, ty.label, t.dim);
            match fraction {
                Some(f) => {
                    g.text_at(r.right() - 9.0, r.cy(), Align::Right, value, ty.label, t.ink);
                    let fill = if *f > 0.85 { t.signal } else { t.ice };
                    g.meter(Rect::new(r.x + label_w, r.cy() - 3.0, (r.w - label_w - value_w).max(8.0), 6.0), *f, fill);
                }
                None => {
                    g.text_fit(r.x + label_w, r.cy(), value, ty.label, t.ink, r.w - label_w - 8.0);
                }
            }
        }
        if self.editing.is_some() {
            g.hint("team number, ip or host · enter");
        } else {
            g.hint_any(&["enter: robot address · →: the field", "enter: address · →: field", "→: the field"]);
        }
    }
}

// ---------------------------------------------------------------- health ---

/// What is firing on the robot, worst first, then what fired recently. Rows
/// carry the check's full name; the focused row's detail reads out below.
#[derive(Default)]
pub struct Health {
    list: ListState,
}

impl Screen for Health {
    fn title(&self) -> &str {
        "Health"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("faults", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        let len = cx.robot.firing().len() + cx.robot.strings("Health/History").len();
        list_key(&mut self.list, key, len);
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h) = (g.top(), g.row_h());

        // One line of counts: errors, warnings, and whether preflight passed.
        let preflight = match (robot.string("Preflight/Summary"), robot.bool("Preflight/Ready")) {
            (Some(s), Some(false)) => (s, g.fault()),
            (Some(s), _) => (s, t.ink),
            (None, _) => ("not run".to_owned(), t.faint),
        };
        let count = |n: Option<i64>, color: Color| match n {
            Some(n) if n > 0 => (n.to_string(), color),
            Some(_) => ("0".to_owned(), t.ink),
            None => ("–".to_owned(), t.faint),
        };
        let chips = [
            ("err", count(robot.i64("Health/ErrorCount"), g.fault())),
            ("warn", count(robot.i64("Health/WarnCount"), t.amber_ink)),
            ("pre", preflight),
        ];
        let mut x = PAD;
        for (i, (label, (value, ink))) in chips.iter().enumerate() {
            let label_w = g.measure(label, ty.caption);
            let w = if i == 2 { W - PAD - x } else { label_w + g.measure("00", ty.label) + 26.0 };
            let r = Rect::new(x, top, w, row_h);
            g.surface(r, (row_h * 0.5).min(11.0), t.surface1);
            g.text_at(r.x + 9.0, r.cy(), Align::Left, label, ty.caption, t.dim);
            g.text_fit(r.x + 15.0 + label_w, r.cy(), value, ty.label, *ink, r.w - label_w - 22.0);
            x += w + GAP;
        }

        let firing = robot.firing();
        let history = robot.strings("Health/History");
        let list_top = top + row_h + GAP;
        if firing.is_empty() && history.is_empty() {
            let (icon, line) = if robot.linked() { ("check_circle", "nothing firing") } else { ("link_off", "no robot") };
            let cy = (list_top + H) * 0.5;
            g.icon(icon, false, W * 0.5, cy - 12.0, 24.0, t.faint);
            g.text_at(W * 0.5, cy + 12.0, Align::Center, line, ty.caption, t.dim);
            return;
        }
        let len = firing.len() + history.len();
        let list_bottom = g.above_hint();
        let mut detail = String::new();
        for (i, y) in self.list.layout(g, len, list_top, row_h, 3.0, list_bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if let Some((severity, subsystem, id, why)) = firing.get(i) {
                let color = severity_color(g, severity);
                g.row(r, &format!("    {subsystem} · {id}"), "", t.dim, focused);
                g.dot(r.x + 12.0, r.cy(), 3.0, color);
                if focused {
                    detail = if why.is_empty() { "firing now".into() } else { why.to_lowercase() };
                }
            } else {
                // "time|KIND|SEVERITY|subsystem|id|detail"
                let parts: Vec<&str> = history[i - firing.len()].split('|').collect();
                if let [time, kind, _severity, subsystem, id, why, ..] = parts[..] {
                    let fired = kind == "FIRED";
                    g.row(r, &format!("{subsystem} · {id}"), if fired { "fired" } else { "cleared" }, t.dim, focused);
                    if focused {
                        let at = format!("{} at {:.0} s", if fired { "fired" } else { "cleared" }, time.parse::<f32>().unwrap_or(0.0));
                        detail = if why.is_empty() { at } else { format!("{at} · {}", why.to_lowercase()) };
                    }
                }
            }
        }
        self.list.scrollbar(g, len, list_top, list_bottom);
        let fitted = g.text.fit(&detail, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

// ------------------------------------------------------------------ logs ---

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 10.0 { format!("{mb:.0} mb") } else { format!("{mb:.1} mb") }
}

fn level_color(g: &Gfx, level: Level) -> Color {
    match level {
        Level::Fault => g.fault(),
        Level::Warn => g.tokens.amber_ink,
        Level::Fine => g.tokens.dim,
    }
}

/// The match logs pulled from the robot, newest first. Each row carries the
/// worst thing its match had; Enter opens the whole summary. Once two
/// matches have been read, a row above them opens their trends, saying what
/// is getting worse; the focus starts on the newest match, under it.
#[derive(Default)]
pub struct LogsScreen {
    list: ListState,
    seen: u64,
    placed: bool,
}

impl LogsScreen {
    /// The focus starts on the newest match, below the trends row if there is one.
    fn place(&mut self, trends: bool) {
        if !self.placed {
            self.list.focus = trends as usize;
            self.placed = true;
        }
    }
}

impl Screen for LogsScreen {
    fn title(&self) -> &str {
        "Logs"
    }

    /// Copy: the match in focus and what stood out in it (clipboard.rs).
    fn copy(&mut self, robot: &Robot) -> Option<String> {
        let rows = robot.logs.list();
        let first = crate::trends::available(&rows) as usize;
        let row = rows.get(self.list.focus.checked_sub(first)?)?;
        Some(match &row.digest {
            Some(digest) => format!("{} · {}", row.title(), digest.headline().1),
            None => row.title(),
        })
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("logs", &[("enter", "Open the match")]), keys("moving", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = cx.robot.logs.list();
        let trends = crate::trends::available(&rows);
        self.place(trends);
        let first = trends as usize;
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if trends && self.list.focus == 0 => return Nav::Push(Box::new(crate::trends::Trends::new())),
            Key::Enter => {
                if let Some(row) = self.list.focus.checked_sub(first).and_then(|i| rows.get(i)) {
                    match LogDetail::of(row) {
                        Some(detail) => return Nav::Push(Box::new(detail)),
                        None => *cx.say = Some("not read yet".into()),
                    }
                }
            }
            other => list_key(&mut self.list, other, rows.len() + first),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.seen = robot.logs.generation();
        let rows = robot.logs.list();
        let status = robot.logs.status();
        if rows.is_empty() {
            g.empty("description", if status.is_empty() { "no robot set" } else { &status });
            return;
        }
        let trends = crate::trends::available(&rows);
        self.place(trends);
        let first = trends as usize;
        let top = g.top();
        let bottom = g.above_hint();
        let row_h = ty.body.size + ty.caption.size + 12.0;
        for (i, y) in self.list.layout(g, rows.len() + first, top, row_h, 4.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, 12.0, if focused { t.surface2 } else { t.surface1 });
            let (line1, line2) = (r.y + 5.0 + ty.body.size * 0.5, r.bottom() - 5.0 - ty.caption.size * 0.5);
            if trends && i == 0 {
                // Trends: how many matches, and what is getting worse.
                let read = rows.iter().filter(|r| r.digest.is_some()).count();
                let count_w = g.text_at(r.right() - 10.0, line1, Align::Right, &format!("{read} matches ›"), ty.label, t.dim);
                g.icon("trending_up", focused, r.x + 16.0, line1, 14.0, t.ink);
                g.text_fit(r.x + 28.0, line1, "Trends", ty.body, t.ink, r.w - 48.0 - count_w);
                let (worse, line) = crate::trends::headline(&rows);
                g.dot(r.x + 13.0, line2, 2.5, if worse { t.amber_ink } else { t.ice });
                g.text_fit(r.x + 21.0, line2, &line, ty.caption, t.dim, r.w - 31.0);
                if focused {
                    g.focus_ring(r, 12.0);
                }
                continue;
            }
            let row = &rows[i - first];
            let size_w = g.text_at(r.right() - 10.0, line1, Align::Right, &megabytes(row.bytes), ty.label, t.dim);
            g.text_fit(r.x + 10.0, line1, &row.title(), ty.body, t.ink, r.w - 30.0 - size_w);
            match &row.digest {
                Some(digest) => {
                    let (level, text) = digest.headline();
                    let color = level_color(g, level);
                    g.dot(r.x + 13.0, line2, 2.5, color);
                    g.text_fit(r.x + 21.0, line2, &text, ty.caption, t.dim, r.w - 31.0);
                }
                None => {
                    g.text_fit(r.x + 10.0, line2, "waiting to be read", ty.caption, t.faint, r.w - 20.0);
                }
            }
            if focused {
                g.focus_ring(r, 12.0);
            }
        }
        self.list.scrollbar(g, rows.len() + first, top, bottom);
        let fitted = g.text.fit(&status, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }

    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
}

/// One match: how long, how low the battery went, the worst loop, and
/// everything that stood out, worst first, each that happened at a moment
/// with its time. Enter replays the match there (the first row, from the
/// start); the focus starts on the worst finding.
pub struct LogDetail {
    title: String,
    name: String,
    digest: Digest,
    list: ListState,
}

impl LogDetail {
    /// A log's summary, once it has been read.
    pub fn of(row: &crate::logs::Row) -> Option<Self> {
        let digest = row.digest.clone()?;
        let mut list = ListState::default();
        list.focus = (!digest.highlights.is_empty()) as usize;
        Some(LogDetail { title: row.title(), name: row.name.clone(), digest, list })
    }

    /// The match as the replay names it: "Q14", else its log's time.
    fn short_title(&self) -> String {
        crate::offline::facts::label_from_name(&self.name).unwrap_or_else(|| self.title.clone())
    }
}

impl Screen for LogDetail {
    fn title(&self) -> &str {
        "Match"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("the match", &[("enter", "Replay from here")]), keys("moving", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                let at = self.list.focus.checked_sub(1).and_then(|i| self.digest.highlights.get(i)).and_then(|(_, text)| self.digest.moment(text));
                match crate::replay::Replay::of_log(&self.name, &self.short_title(), &cx.robot.logs, at) {
                    Some(replay) => return Nav::Push(Box::new(replay)),
                    None => *cx.say = Some("the log is not on this handheld".into()),
                }
            }
            other => list_key(&mut self.list, other, self.digest.highlights.len() + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let d = &self.digest;
        let minutes = |s: f64| format!("{}:{:02}", (s / 60.0) as u32, (s % 60.0) as u32);
        let chips = [
            ("enabled", d.enabled_seconds.map(minutes).unwrap_or_else(|| minutes(d.seconds))),
            ("battery", dash(d.battery_low.map(|v| format!("{v:.1} v")))),
            ("loop", dash(d.worst_loop_ms.map(|v| format!("{v:.0} ms")))),
        ];
        let chip_h = ty.caption.size + ty.label.size + 12.0;
        let w = (W - 2.0 * PAD - 2.0 * GAP) / 3.0;
        for (i, (label, value)) in chips.iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (w + GAP), top, w, chip_h);
            g.surface(r, 11.0, t.surface1);
            g.text_at(r.x + 9.0, r.y + 5.0 + ty.caption.size * 0.5, Align::Left, label, ty.caption, t.dim);
            g.text_fit(r.x + 9.0, r.bottom() - 5.0 - ty.label.size * 0.5, value, ty.label, t.ink, w - 14.0);
        }
        let list_top = top + chip_h + GAP;
        let bottom = g.above_hint();
        let row_h = g.row_h();
        let rows = d.highlights.len() + 1;
        for (i, y) in self.list.layout(g, rows, list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            let Some((level, text)) = i.checked_sub(1).and_then(|k| d.highlights.get(k)) else {
                g.row(r, "    replay the match", "", t.dim, focused);
                g.icon("play_arrow", true, r.x + 13.0, r.cy(), 13.0, t.ink);
                continue;
            };
            let color = level_color(g, *level);
            // When it happened, to replay it there.
            let when = d.moment(text).map(|at| crate::replay::clock(at as f32)).unwrap_or_default();
            g.row(r, &format!("    {text}"), &when, t.dim, focused);
            g.dot(r.x + 12.0, r.cy(), 3.0, color);
        }
        self.list.scrollbar(g, rows, list_top, bottom);
        let fitted = g.text.fit(&self.title, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

// ---------------------------------------------------------------- tables ---

/// A NetworkTables browser over `/Catalyst/`: tables open into their
/// children, values show live. The title is the table being looked at.
pub struct Tables {
    path: String,
    title: String,
    list: ListState,
    /// Where the focus was at each level above, to return to.
    trail: Vec<usize>,
}

enum Node {
    Table(String, usize),
    Value(String, String),
}

impl Tables {
    pub fn new() -> Self {
        Tables { path: String::new(), title: "Tables".into(), list: ListState::default(), trail: Vec::new() }
    }

    fn nodes(&self, robot: &Robot) -> Vec<Node> {
        let mut nodes: Vec<Node> = Vec::new();
        for (key, value) in robot.under(&self.path) {
            let rest = &key[self.path.len()..];
            match rest.split_once('/') {
                Some((table, _)) => match nodes.last_mut() {
                    Some(Node::Table(name, count)) if name == table => *count += 1,
                    _ => nodes.push(Node::Table(table.to_owned(), 1)),
                },
                None => nodes.push(Node::Value(rest.to_owned(), value.display())),
            }
        }
        nodes
    }

    fn enter(&mut self, path: String, focus: usize) {
        self.title = path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("Tables").to_owned();
        self.path = path;
        self.list = ListState::default();
        self.list.focus = focus;
    }

    /// Tables opened at a key (`Loop/Robot/LastMs`): in its table, focused
    /// on it. Back climbs the tables above it.
    pub fn at(key: &str, robot: &Robot) -> Self {
        let mut tables = Tables::new();
        let (path, name) = key.rsplit_once('/').map_or((String::new(), key), |(p, n)| (format!("{p}/"), n));
        tables.enter(path, 0);
        tables.trail = vec![0; tables.path.matches('/').count()];
        let at = tables.nodes(robot).iter().position(|n| matches!(n, Node::Value(v, _) if v == name)).unwrap_or(0);
        tables.list.focus = at;
        tables
    }
}

impl Screen for Tables {
    fn title(&self) -> &str {
        &self.title
    }

    /// Copy: the focused value as it is now, whole (its row may cut it
    /// short); on a table, its key.
    fn copy(&mut self, robot: &Robot) -> Option<String> {
        match self.nodes(robot).into_iter().nth(self.list.focus)? {
            Node::Value(_, value) => Some(value),
            Node::Table(name, _) => Some(format!("{}{name}", self.path)),
        }
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![
            keys(
                "tables",
                &[("enter · →", "Open the table"), ("enter", "Graph the value"), ("p", "Pin it to Glance"), ("g", "The graphs"), ("←", "Up a level")],
            ),
            keys("moving", LIST_KEYS),
        ]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let nodes = self.nodes(cx.robot);
        match key {
            Key::Enter | Key::Right => match nodes.get(self.list.focus) {
                Some(Node::Table(name, _)) => {
                    self.trail.push(self.list.focus);
                    self.enter(format!("{}{name}/", self.path), 0);
                }
                // Enter on a value puts it on the live graph, or takes it off.
                Some(Node::Value(name, _)) if key == Key::Enter => {
                    // Short words: a long message squeezes the table's name.
                    use crate::graphs::Toggled;
                    let key = format!("{}{name}", self.path);
                    let numeric = cx.robot.get(&key).and_then(|v| crate::graphs::number(&v)).is_some();
                    *cx.say = Some(match cx.robot.graphs.toggle(&key, numeric) {
                        Toggled::Added => "graphed".into(),
                        Toggled::Removed => "off the graph".into(),
                        Toggled::Full => "graphs full".into(),
                        Toggled::NotANumber => "not a number".into(),
                    });
                }
                _ => {}
            },
            Key::Char('g') => return Nav::Push(Box::new(crate::graphs::Graphs::new())),
            // p pins a value to the glance page (again unpins it).
            Key::Char('p') => {
                if let Some(Node::Value(name, _)) = nodes.get(self.list.focus) {
                    let key = format!("{}{name}", self.path);
                    let unpin = cx.settings.pinned.as_deref() == Some(key.as_str());
                    cx.settings.pinned = (!unpin).then_some(key);
                    if !unpin {
                        cx.settings.widgets.pin_value();
                    }
                    crate::save_settings(cx.settings);
                    *cx.say = Some(if unpin { "unpinned".into() } else { format!("{name} pinned to the glance page") });
                }
            }
            Key::Back | Key::Left => {
                if self.path.is_empty() {
                    if key == Key::Back {
                        return Nav::Pop;
                    }
                } else {
                    let parent = self.path.trim_end_matches('/').rsplit_once('/').map(|(p, _)| format!("{p}/")).unwrap_or_default();
                    let focus = self.trail.pop().unwrap_or(0);
                    self.enter(parent, focus);
                }
            }
            other => list_key(&mut self.list, other, nodes.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let nodes = self.nodes(robot);
        if nodes.is_empty() {
            g.empty("link_off", if robot.linked() { "nothing published here" } else { "no robot" });
            return;
        }
        let (top, row_h) = (g.top(), g.row_h());
        let bottom = g.above_hint();
        let graphed = robot.graphs.keys().to_vec();
        for (i, y) in self.list.layout(g, nodes.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match &nodes[i] {
                Node::Table(name, count) => g.row(r, &format!("{name}/"), &format!("{count} ›"), t.dim, i == self.list.focus),
                Node::Value(name, value) => {
                    // The value gets up to half the row; the name takes the rest.
                    let shown = g.text.fit(value, &ty.label, r.w * 0.5);
                    g.row(r, name, &shown, t.ink, i == self.list.focus);
                    // On the graph: its line's colour, just before the value.
                    let key = format!("{}{name}", self.path);
                    if let Some(at) = graphed.iter().position(|k| *k == key) {
                        let value_w = g.measure(&shown, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label });
                        g.dot(r.right() - 10.0 - value_w - 8.0, r.cy(), 3.0, crate::graphs::colour(&t, at));
                    }
                }
            }
        }
        self.list.scrollbar(g, nodes.len(), top, bottom);
        let on_value = matches!(nodes.get(self.list.focus), Some(Node::Value(..)));
        g.hint(if on_value { "enter: graph · g: graphs · p: pin" } else { "g: graphs" });
    }
}

// ------------------------------------------------------------------ loop ---

/// Robot loop timing against its 20 ms budget.
pub struct LoopTime;

impl Screen for LoopTime {
    fn title(&self) -> &str {
        "Loop"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("loop time", &[("esc", "Done watching")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if key == Key::Back { Nav::Pop } else { Nav::Stay }
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        const BUDGET: f32 = 20.0;
        let top = g.top();
        let tile_h = ty.caption.size + 22.0 * ty.display_scale + 12.0;
        let sw = (W - 2.0 * PAD - 2.0 * GAP) / 3.0;
        let stats = [("last", "Loop/Robot/LastMs"), ("average", "Loop/Robot/AverageMs"), ("worst", "Loop/Robot/MaxMs")];
        for (i, (label, key)) in stats.iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (sw + GAP), top, sw, tile_h);
            g.stat(r, label, robot.f64(key).map(|v| format!("{v:.1}")), "ms", 22.0);
        }
        let plot = Rect::new(PAD, top + tile_h + GAP, W - 2.0 * PAD, H - PAD - top - tile_h - GAP);
        g.surface(plot, g.tile_radius(), t.surface1);
        if robot.loop_history.len() < 2 {
            let line = match (robot.linked(), robot.f64("Loop/Robot/LastMs")) {
                (false, _) => "no robot",
                (true, Some(_)) => "collecting…",
                (true, None) => "no loop monitor on this robot",
            };
            g.text_at(plot.cx(), plot.cy(), Align::Center, line, ty.caption, t.dim);
            return;
        }
        // Scale so the budget line is always in view.
        let peak = robot.loop_history.iter().cloned().fold(BUDGET * 1.15, f32::max);
        let inner = Rect::new(plot.x + 9.0, plot.y + 8.0, plot.w - 18.0, plot.h - 16.0);
        let y_of = |ms: f32| inner.bottom() - (ms / peak).min(1.0) * inner.h;
        let by = y_of(BUDGET);
        let mut x = inner.x;
        while x < inner.right() {
            g.surface(Rect::new(x, by, 3.0, 1.0), 0.0, t.faint);
            x += 6.0;
        }
        g.text_at(inner.right(), by + 3.0 + ty.caption.size * 0.5, Align::Right, "20 ms budget", ty.caption, t.dim);
        let step = inner.w / 119.0;
        let start = 120 - robot.loop_history.len();
        let pts: Vec<(f32, f32)> =
            robot.loop_history.iter().enumerate().map(|(i, ms)| (inner.x + (start + i) as f32 * step + g.ox, y_of(*ms))).collect();
        let over = robot.bool("Loop/Robot/OverBudget") == Some(true);
        g.canvas.polyline(&pts, 2.0, if over { t.signal } else { t.ice }, 1.0);
    }
}

// ---------------------------------------------------------------- motors ---

/// Catalyst's motor history: lifetime hours and peaks per motor.
#[derive(Default)]
pub struct Motors {
    list: ListState,
}

impl Screen for Motors {
    fn title(&self) -> &str {
        "Motors"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("motors", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        list_key(&mut self.list, key, cx.robot.strings("MotorHistory/Rows").len());
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = robot.strings("MotorHistory/Rows");
        if rows.is_empty() {
            g.empty("precision_manufacturing", if robot.linked() { "no motor history published" } else { "no robot" });
            return;
        }
        let top = g.top();
        let row_h = ty.name.size + ty.caption.size + 12.0;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 4.0, H) {
            // serial|model|kind|bus|id|name|firmware|poweredSeconds|runningSeconds|…|peakStatorAmps|peakTempC|…
            let f: Vec<&str> = rows[i].split('|').collect();
            let field = |n: usize| f.get(n).copied().unwrap_or("");
            let hours = field(8).parse::<f64>().ok().map(|s| format!("{:.1} h", s / 3600.0));
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, 12.0, if focused { t.surface2 } else { t.surface1 });
            let (line1, line2) = (r.y + 5.0 + ty.name.size * 0.5, r.bottom() - 5.0 - ty.caption.size * 0.5);
            let hours_w = g.text_at(r.right() - 10.0, line1, Align::Right, &dash(hours), ty.label, t.ink);
            g.text_fit(r.x + 10.0, line1, field(5), ty.name, t.ink, r.w - 30.0 - hours_w);
            let peak_temp = field(12).parse::<f64>().ok();
            let hot = peak_temp.is_some_and(|c| c >= 80.0);
            let peaks = format!("{} a · {} °c", field(11), dash(peak_temp.map(|c| format!("{c:.0}"))));
            let peaks_w = g.text_at(r.right() - 10.0, line2, Align::Right, &peaks, ty.caption, if hot { t.signal } else { t.dim });
            g.text_fit(r.x + 10.0, line2, &format!("{} · id {}", field(1), field(4)).to_lowercase(), ty.caption, t.dim, r.w - 30.0 - peaks_w);
            if focused {
                g.focus_ring(r, 12.0);
            }
        }
        self.list.scrollbar(g, rows.len(), top, H);
    }
}

// --------------------------------------------------------------- console ---

/// The robot program as the Systemcore sees it, from the Catalyst Agent:
/// whether it is running, how often it restarted, and its recent log.
pub struct Console {
    agent: Option<Agent>,
    seen: u64,
    /// Lines scrolled back from the newest.
    back: usize,
    /// Lines the log panel showed last time, for paging.
    fit: usize,
}

impl Console {
    pub fn open(robot: &Robot) -> Self {
        let agent = if robot.is_demo() {
            Some(Agent::demo())
        } else if robot.address.is_empty() {
            None
        } else {
            let host = robot.address.split(':').next().unwrap_or_default();
            Some(Agent::connect(host))
        };
        Console { agent, seen: u64::MAX, back: 0, fit: 4 }
    }
}

/// Break a line at spaces into pieces of at most `columns` characters;
/// continuation pieces are indented.
fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word: String = word.to_owned();
        loop {
            let used = current.chars().count();
            let space = usize::from(used > 0 && !current.ends_with(' '));
            if used + space + word.chars().count() <= columns {
                if space == 1 {
                    current.push(' ');
                }
                current.push_str(&word);
                break;
            }
            if used > 2 {
                out.push(std::mem::replace(&mut current, "  ".into()));
                continue;
            }
            // A single word longer than the line: cut it.
            let take = columns.saturating_sub(used).max(1);
            let head: String = word.chars().take(take).collect();
            word = word.chars().skip(take).collect();
            current.push_str(&head);
            out.push(std::mem::replace(&mut current, "  ".into()));
            if word.is_empty() {
                break;
            }
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn duration(seconds: i64) -> String {
    match seconds {
        s if s >= 3600 => format!("{} h {} m", s / 3600, s % 3600 / 60),
        s if s >= 60 => format!("{} m {} s", s / 60, s % 60),
        s => format!("{s} s"),
    }
}

impl Screen for Console {
    fn title(&self) -> &str {
        "Console"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        vec![crate::help::keys("console", &[("↑ ↓", "Scroll back"), ("pg up · pg dn", "Page by page")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Up => self.back += 1,
            Key::Down => self.back = self.back.saturating_sub(1),
            Key::PageUp => self.back += self.fit,
            Key::PageDown => self.back = self.back.saturating_sub(self.fit),
            Key::Back => return Nav::Pop,
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let generation = self.agent.as_ref().map_or(0, Agent::generation);
        let changed = generation != self.seen;
        self.seen = generation;
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let snapshot = self.agent.as_ref().and_then(Agent::snapshot);
        let Some(snapshot) = snapshot else {
            let why = self.agent.as_ref().map_or("no robot set".to_owned(), Agent::status);
            g.empty("terminal", &why);
            return;
        };
        let program = &snapshot["robotProgram"];
        let running = program["subState"].as_str() == Some("running");
        let state = program["subState"].as_str().or(program["state"].as_str()).unwrap_or("–").to_owned();
        let chips = [
            ("program", state, if running { t.ink } else { t.signal }),
            ("restarts", dash(program["restarts"].as_i64().map(|n| n.to_string())), t.ink),
            ("up", dash(program["runningForSeconds"].as_i64().map(duration)), t.ink),
        ];
        let top = g.top();
        let chip_h = ty.caption.size + ty.label.size + 12.0;
        let side = g.measure("restarts", ty.caption) + 18.0;
        let widths = [(W - 2.0 * PAD - 2.0 * GAP - side) * 0.5, side, (W - 2.0 * PAD - 2.0 * GAP - side) * 0.5];
        let mut x = PAD;
        for ((label, value, ink), w) in chips.iter().zip(widths) {
            let r = Rect::new(x, top, w, chip_h);
            g.surface(r, 11.0, t.surface1);
            g.text_at(r.x + 9.0, r.y + 5.0 + ty.caption.size * 0.5, Align::Left, label, ty.caption, t.dim);
            g.text_fit(r.x + 9.0, r.bottom() - 5.0 - ty.label.size * 0.5, value, ty.label, *ink, w - 14.0);
            x += w + GAP;
        }

        // The log: newest at the bottom, the message without its journal prefix.
        let panel = Rect::new(PAD, top + chip_h + GAP, W - 2.0 * PAD, H - PAD - top - chip_h - GAP);
        g.surface(panel, g.tile_radius(), t.surface1);
        let empty = Vec::new();
        let lines = program["log"].as_array().unwrap_or(&empty);
        let line_h = ty.caption.size + 3.0;
        self.fit = (((panel.h - 10.0) / line_h) as usize).max(1);
        // Mono type: a line holds a whole number of characters.
        let columns = ((panel.w - 18.0) / g.measure("0", ty.caption)).floor().max(8.0) as usize;
        let mut wrapped: Vec<(String, Color)> = Vec::new();
        for line in lines {
            let line = line.as_str().unwrap_or_default();
            let message = line.split_once("]: ").map_or(line, |(_, m)| m);
            let ink = if message.contains("ERROR") || message.contains("FAIL") || message.contains("Exception") {
                g.fault()
            } else if message.to_ascii_lowercase().contains("warn") {
                t.amber_ink
            } else {
                t.dim
            };
            for piece in wrap(message, columns) {
                wrapped.push((piece, ink));
            }
        }
        self.back = self.back.min(wrapped.len().saturating_sub(self.fit));
        let end = wrapped.len() - self.back;
        for (row, (piece, ink)) in wrapped[end.saturating_sub(self.fit)..end].iter().enumerate() {
            g.text_at(panel.x + 9.0, panel.y + 5.0 + line_h * (row as f32 + 0.5), Align::Left, piece, ty.caption, *ink);
        }
        if lines.is_empty() {
            g.text_at(panel.cx(), panel.cy(), Align::Center, "no log lines", ty.caption, t.dim);
        }
    }
}

// -------------------------------------------------------------- settings ---

/// One thing on a settings page.
#[derive(Clone, Copy, PartialEq)]
enum Item {
    // The sections.
    RobotSection,
    Appearance,
    TextSection,
    HomeSection,
    Display,
    Network,
    KeysSection,
    Access,
    /// Settings › System update: where it stands, the next step, the choices.
    UpdateSection,
    UpdateState,
    UpdateAction,
    UpdateNote,
    UpdateChannel,
    UpdateAuto,
    UpdateOvernight,
    About,
    /// Settings › Help: the tour again, the keyboard shortcuts, and the
    /// keyboard lesson from setup.
    Help,
    TakeTour,
    KeyboardShortcuts,
    KeyboardLesson,
    /// Settings › Name & time: the handheld's name, its time zone, and
    /// how its clock was last set.
    NameSection,
    DeviceName,
    TimeZone,
    ClockNote,
    /// Developer options, once About › Build number has been pressed seven
    /// times: the switch for them, the animation scale, Show key presses.
    DeveloperSection,
    DevMode,
    MotionScale,
    ShowKeys,
    /// Developer options › Lower voltage: the processor's core a step lower,
    /// tried before it is kept.
    Undervolt,
    /// Developer options › Tear-free panel: the panel driven upright and in
    /// step with its refresh, tried before it is kept.
    TearFree,
    /// Settings › Battery: a page of its own (power.rs).
    BatterySection,
    /// Settings › Lock screen and Settings › StandBy: pages of their own
    /// (lockpages.rs).
    LockSection,
    StandBySection,
    /// Settings › On-device model: a page of its own (local.rs).
    OnDeviceModel,
    /// Settings › Offline dictation: a page of its own (whisper_local.rs).
    OfflineDictation,
    /// Settings › Voice commands: a page of its own (voicecmd.rs).
    VoiceCommands,
    /// Settings › Sound and Settings › Bluetooth: pages of their own
    /// (sound.rs, bluetooth.rs).
    SoundSection,
    BluetoothSection,
    /// Settings › Music: what Music keeps on the card (music/settings.rs).
    MusicSection,
    /// Settings › Camera: detail mode, the Grove port, the smart camera.
    CameraSection,
    DetailModeRow,
    GrovePort,
    SmartCamera,
    /// The camera's program, installed over USB-A (unitv.rs).
    CameraProgram,
    /// Setup's page offered to a computer, outside first start.
    FromComputer,
    /// Settings › Computer companion: a page of its own (companion.rs).
    CompanionSection,
    // Appearance.
    ThemeChoice,
    Mode,
    AccentColor,
    /// Your own colour: a hue around the wheel.
    OwnColour,
    WallpaperChoice,
    CornersChoice,
    // Text.
    LettersChoice,
    WidthChoice,
    // Home.
    HomeScreen,
    Columns,
    /// Arrange apps: Home's tiles in order, any of them hidden.
    ArrangeApps,
    /// An app on that page, by its place in `SettingsApp::apps`.
    AppTile(usize),
    ClockFormat,
    Seconds,
    IslandChoice,
    Greeting,
    /// Settings › Home › Widgets: a page of its own (widgets/page.rs).
    WidgetsPage,
    TextScale,
    Calm,
    // Display.
    Brightness,
    /// Graphics: the GPU when there is one, or the processor alone.
    Graphics,
    /// Standby: how long with no key before the screen goes off, and how
    /// the processor chooses its speed.
    ScreenOff,
    Processor,
    /// The motion sensor's part in standby (where there is one): picked up,
    /// the screen comes on; put face down, it goes off.
    LiftWake,
    FaceDown,
    // Network.
    WifiSetup,
    /// Settings › Network › Tailscale: a page of its own (tailscale.rs).
    Tailscale,
    /// Settings › USB: what the handheld is to a computer, and whether one has it.
    UsbSection,
    UsbMode,
    UsbLink,
    // Access.
    Ssh,
    // Keys.
    KeyFor(usize),
    KeysFromFile,
    // Robot.
    DemoRobot,
    OpenRobot,
    MatchReport,
    /// A read-only line of what the device reports.
    Info(usize),
}

/// Settings for the handheld itself: how it looks, its screen, its network,
/// who can get in, and what it is. One page type serves the list of sections
/// and each section.
pub struct SettingsApp {
    title: &'static str,
    items: Vec<Item>,
    list: ListState,
    /// Read-only lines, re-read every couple of seconds while the page shows.
    info: Vec<(&'static str, String)>,
    read_at: std::time::Instant,
    /// What the section rows summarise, re-read with the rest.
    network: String,
    backlight: Option<u32>,
    /// Where a system update stands, re-read with the rest.
    update: crate::sysupdate::Status,
    /// Keys for online services, and one being typed: which, and what so far.
    keys: crate::keys::Keys,
    typing: Option<(usize, String)>,
    /// The handheld's name, while a new one is typed.
    naming: Option<String>,
    /// On Arrange apps: every app Home can show, built-in then external.
    apps: Vec<String>,
}

/// Every app Home can show, by name: the built-in ones, then those found on
/// the storage.
fn app_names() -> Vec<String> {
    builtin_apps().map(|(_, _, name)| name.to_owned()).chain(crate::apps::find().into_iter().map(|a| a.name)).collect()
}

/// Arrange apps' rows as Home has them: the favourites first, then the rest,
/// each in the order chosen, anything not yet placed after them as it comes.
fn arrange_items(apps: &[String], arrangement: &crate::shell::Arrangement) -> Vec<Item> {
    let mut at: Vec<usize> = (0..apps.len()).collect();
    at.sort_by_key(|i| {
        let name = &apps[*i];
        (!arrangement.favourites.contains(name), arrangement.order.iter().position(|n| n == name).unwrap_or(usize::MAX))
    });
    at.into_iter().map(Item::AppTile).collect()
}

/// A Home style as a small picture of itself in `r` (Settings › Home): the
/// grid's tiles, the carousel's three cards, the flow's turned covers. The
/// chosen one is in ink and the signal, and ringed; the others are faint.
fn home_swatch(g: &mut Gfx, style: HomeStyle, r: Rect, chosen: bool) {
    let t = g.tokens;
    let (ink, accent) = if chosen { (t.ink, t.signal) } else { (t.faint, t.faint) };
    let r = r.offset(g.ox, 0.0);
    match style {
        HomeStyle::Grid => {
            let (w, h) = ((r.w - 4.0) / 3.0, (r.h - 2.0) / 2.0);
            for i in 0..6 {
                let cell = Rect::new(r.x + (i % 3) as f32 * (w + 2.0), r.y + (i / 3) as f32 * (h + 2.0), w, h);
                g.canvas.rrect(cell, 1.5, if i == 0 { accent } else { ink }, if i == 0 { 1.0 } else { 0.55 });
            }
        }
        HomeStyle::Carousel => {
            let side = Rect::new(r.x, r.y + 2.5, 4.0, r.h - 5.0);
            g.canvas.rrect(side, 1.5, ink, 0.55);
            g.canvas.rrect(side.offset(r.w - 4.0, 0.0), 1.5, ink, 0.55);
            g.canvas.rrect(Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h), 2.0, accent, 1.0);
        }
        HomeStyle::Flow => {
            let ox = g.ox;
            g.ox = 0.0;
            crate::flow::swatch(g, r, ink, accent);
            g.ox = ox;
        }
    }
    if chosen {
        g.canvas.stroke_rrect(r.inset(-2.5), 4.0, 1.0, t.ink, 0.9);
    }
}

/// The hue of "your own colour" nearest a named accent's own, to start from.
fn nearest_hue(accent: bezel::Accent) -> u8 {
    let c = accent.seed();
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let hue = if max == min {
        0.0
    } else if max == r {
        60.0 * (((g - b) / (max - min)).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / (max - min) + 2.0)
    } else {
        60.0 * ((r - g) / (max - min) + 4.0)
    };
    ((hue / bezel::hues::STEP as f32).round() as u32 % bezel::Accent::HUES as u32) as u8
}

/// Where everything in Settings is, for search: what it is called, the
/// section it is in, and its place there (none: the section itself).
const SETTING_PLACES: [(&str, Item, Option<usize>); 94] = [
    ("Robot settings", Item::RobotSection, None),
    ("Open Robot on link", Item::RobotSection, Some(0)),
    ("Match report after a match", Item::RobotSection, Some(1)),
    ("Demo robot", Item::RobotSection, Some(2)),
    ("Appearance", Item::Appearance, None),
    ("Theme", Item::Appearance, Some(0)),
    ("Light or dark mode", Item::Appearance, Some(1)),
    ("Accent colour", Item::Appearance, Some(2)),
    ("Your own colour", Item::Appearance, Some(3)),
    ("Wallpaper", Item::Appearance, Some(4)),
    ("Corners", Item::Appearance, Some(5)),
    ("Calm version", Item::Appearance, Some(6)),
    ("Text", Item::TextSection, None),
    ("Text size", Item::TextSection, Some(0)),
    ("Letter style", Item::TextSection, Some(1)),
    ("Letter width", Item::TextSection, Some(2)),
    ("Home", Item::HomeSection, None),
    ("Home screen style", Item::HomeSection, Some(0)),
    ("Tiles per row", Item::HomeSection, Some(1)),
    ("Arrange apps", Item::ArrangeApps, None),
    ("Widgets", Item::WidgetsPage, None),
    ("Clock format", Item::HomeSection, Some(4)),
    ("Clock seconds", Item::HomeSection, Some(5)),
    ("Island at rest", Item::HomeSection, Some(6)),
    ("Greeting", Item::HomeSection, Some(7)),
    ("Display", Item::Display, None),
    ("Brightness", Item::Display, Some(0)),
    ("Graphics and the GPU", Item::Display, Some(1)),
    ("Screen off after", Item::Display, Some(2)),
    ("Power saving", Item::Display, Some(3)),
    ("Battery", Item::BatterySection, None),
    ("Battery saver", Item::BatterySection, Some(0)),
    ("Turn off at 5%", Item::BatterySection, Some(2)),
    ("Deep standby", Item::BatterySection, Some(crate::power::DEEP_ROW)),
    ("Red light", Item::BatterySection, Some(crate::power::DEEP_ROW + 1)),
    ("Lift to wake", Item::Display, Some(4)),
    ("Face down to sleep", Item::Display, Some(5)),
    ("Lock screen", Item::LockSection, None),
    ("Passcode", Item::LockSection, Some(0)),
    ("Customise lock screen", Item::LockSection, Some(1)),
    ("StandBy", Item::StandBySection, None),
    ("StandBy on the charger", Item::StandBySection, Some(0)),
    ("Night mode for StandBy", Item::StandBySection, Some(1)),
    ("Network", Item::Network, None),
    ("Wi-Fi networks", Item::Network, Some(0)),
    ("Tailscale", Item::Network, Some(1)),
    ("Sound", Item::SoundSection, None),
    ("Volume", Item::SoundSection, Some(0)),
    ("USB headphones", Item::SoundSection, Some(crate::sound::USB_ROW)),
    ("Spatial sound", Item::SoundSection, Some(crate::sound::SPATIAL_ROW)),
    ("Equalizer", Item::SoundSection, Some(crate::sound::EQ_ROW)),
    ("Loudness match", Item::SoundSection, Some(crate::sound::LOUDNESS_ROW)),
    ("Crossfade", Item::SoundSection, Some(crate::sound::CROSSFADE_ROW)),
    ("Test sound", Item::SoundSection, Some(crate::sound::TEST_ROW)),
    ("Microphone", Item::SoundSection, Some(crate::sound::MIC_ROW)),
    ("Bluetooth", Item::BluetoothSection, None),
    ("Bluetooth headphones and speakers", Item::BluetoothSection, Some(0)),
    ("Music", Item::MusicSection, None),
    ("Played songs kept offline", Item::MusicSection, Some(0)),
    ("Clear played songs", Item::MusicSection, Some(2)),
    ("Music downloads", Item::MusicSection, Some(3)),
    ("Lyrics on the island", Item::MusicSection, Some(6)),
    ("Camera", Item::CameraSection, None),
    ("Detail mode", Item::CameraSection, Some(0)),
    ("Grove port", Item::CameraSection, Some(1)),
    ("Smart camera", Item::CameraSection, Some(2)),
    ("UnitV program (smart camera)", Item::CameraSection, Some(3)),
    ("Name & time", Item::NameSection, None),
    ("Handheld name", Item::NameSection, Some(0)),
    ("Time zone", Item::NameSection, Some(1)),
    ("Keys for online services", Item::KeysSection, None),
    ("The Blue Alliance key", Item::KeysSection, Some(0)),
    ("FRC Nexus key", Item::KeysSection, Some(1)),
    ("OpenAI key", Item::KeysSection, Some(2)),
    ("Claude key", Item::KeysSection, Some(3)),
    ("GitHub key", Item::KeysSection, Some(4)),
    ("Read keys from a file", Item::KeysSection, Some(5)),
    ("On-device model", Item::OnDeviceModel, None),
    ("Offline dictation", Item::OfflineDictation, None),
    ("Voice commands", Item::VoiceCommands, None),
    ("SSH access", Item::Access, Some(0)),
    ("System update", Item::UpdateSection, None),
    ("Check for updates", Item::UpdateSection, Some(1)),
    ("Update channel", Item::UpdateSection, Some(3)),
    ("About this handheld", Item::About, None),
    ("Version", Item::About, Some(0)),
    ("USB", Item::UsbSection, None),
    ("Computer companion", Item::CompanionSection, None),
    ("Pair a computer", Item::CompanionSection, Some(1)),
    ("Help", Item::Help, None),
    ("Take the tour", Item::Help, Some(0)),
    ("Keyboard shortcuts", Item::Help, Some(1)),
    ("Keyboard lesson", Item::Help, Some(2)),
    ("Set up from a computer", Item::FromComputer, None),
];

/// The names search finds Settings by (the motion sensor's only where it is).
pub fn setting_names() -> impl Iterator<Item = (usize, &'static str)> {
    let motion = crate::imu::has_motion();
    SETTING_PLACES.iter().enumerate().filter(move |(_, place)| motion || !matches!(place.0, "Lift to wake" | "Face down to sleep")).map(|(i, place)| (i, place.0))
}

/// Everything search finds in Settings: its place (for `open_setting`), its
/// name, and the section it is in, by that section's own name (none: a
/// section itself, or a row with no section of its own).
pub fn setting_sections() -> Vec<(usize, &'static str, Option<&'static str>)> {
    setting_names()
        .map(|(i, name)| {
            let (_, section, place) = SETTING_PLACES[i];
            let parent = place.and_then(|_| SETTING_PLACES.iter().find(|(_, s, p)| *s == section && p.is_none()).map(|(n, _, _)| *n));
            (i, name, parent)
        })
        .collect()
}

/// Where a section is in Settings' own list, by its name in `SETTING_PLACES`
/// ("About this handheld", "Help"): the scripted tour counts its way there,
/// so a section added before it does not throw the tour off.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn settings_row(name: &str) -> usize {
    let (_, section, _) = SETTING_PLACES.iter().find(|(n, _, place)| *n == name && place.is_none()).expect("a section of Settings");
    SettingsApp::new().items.iter().position(|it| it == section).expect("listed in Settings")
}

/// Settings opened at one of `SETTING_PLACES`.
pub fn open_setting(index: usize) -> Box<dyn Screen> {
    let (_, section, place) = SETTING_PLACES[index];
    // About is a page of its own; its version, the page about the version.
    if section == Item::About {
        let kind = if place.is_some() { crate::about::Kind::Version } else { crate::about::Kind::Main };
        return Box::new(crate::about::About::new(kind));
    }
    if section == Item::BatterySection {
        return Box::new(crate::power::BatteryPage::new().at(place.unwrap_or(0)));
    }
    if section == Item::LockSection {
        return Box::new(crate::lockpages::LockPage::new().at_place(place.unwrap_or(0)));
    }
    if section == Item::StandBySection {
        return Box::new(crate::lockpages::StandByPage::new().at(place.unwrap_or(0)));
    }
    if section == Item::OnDeviceModel {
        return Box::new(crate::local::ModelPage::new());
    }
    if section == Item::OfflineDictation {
        return Box::new(crate::whisper_local::DictationPage::new());
    }
    if section == Item::VoiceCommands {
        return Box::new(crate::voicecmd::CommandsPage::new());
    }
    if section == Item::SoundSection {
        return Box::new(crate::sound::SoundScreen::new().at(place.unwrap_or(0)));
    }
    if section == Item::BluetoothSection {
        return Box::new(crate::bluetooth::BluetoothScreen::new());
    }
    if section == Item::CompanionSection {
        return Box::new(crate::companion::CompanionScreen::new().at(place.unwrap_or(0)));
    }
    if section == Item::MusicSection {
        return Box::new(crate::music::MusicSettings::new().at(place.unwrap_or(0)));
    }
    if section == Item::WidgetsPage {
        return Box::new(crate::widgets::WidgetsPage::new());
    }
    match SettingsApp::section(section) {
        Some(mut page) => {
            page.list.focus = place.unwrap_or(0);
            Box::new(page)
        }
        None => Box::new(crate::setup::Setup::from_computer()),
    }
}

/// The file Settings reads keys from: put there over SSH or from a USB drive.
fn keys_file() -> std::path::PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        "/data/keys.txt".into()
    } else {
        crate::config_dir().join("keys.txt")
    }
}

impl SettingsApp {
    pub fn new() -> Self {
        use Item::*;
        Self::page(
            "Settings",
            vec![RobotSection, Appearance, TextSection, HomeSection, Display, LockSection, StandBySection, BatterySection, Network, SoundSection, BluetoothSection, MusicSection, CameraSection, NameSection, UsbSection, CompanionSection, KeysSection, OnDeviceModel, OfflineDictation, VoiceCommands, Access, UpdateSection, Help, About, FromComputer],
        )
    }

    /// Settings › Camera on its own (the camera's shots).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn camera_page() -> Self {
        Self::section(Item::CameraSection).unwrap_or_else(Self::new)
    }

    /// For the screenshots: the focus on a row.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn focused(mut self, at: usize) -> Self {
        self.list.focus = at.min(self.items.len().saturating_sub(1));
        self
    }

    /// Settings › Keys on its own, for the assistant to send someone to.
    pub fn keys_page() -> Self {
        Self::section(Item::KeysSection).unwrap_or_else(Self::new)
    }

    fn page(title: &'static str, items: Vec<Item>) -> Self {
        let mut page = SettingsApp {
            title,
            items,
            list: ListState::default(),
            info: Vec::new(),
            read_at: std::time::Instant::now(),
            network: String::new(),
            backlight: None,
            update: crate::sysupdate::Status::default(),
            keys: crate::keys::Keys::load(),
            typing: None,
            naming: None,
            apps: Vec::new(),
        };
        page.read_info();
        page
    }

    fn section(item: Item) -> Option<Self> {
        use Item::*;
        Some(match item {
            RobotSection => Self::page("Robot", vec![OpenRobot, MatchReport, DemoRobot]),
            Appearance => Self::page("Appearance", vec![ThemeChoice, Mode, AccentColor, OwnColour, WallpaperChoice, CornersChoice, Calm]),
            TextSection => Self::page("Text", vec![TextScale, LettersChoice, WidthChoice]),
            HomeSection => Self::page("Home", vec![HomeScreen, Columns, ArrangeApps, WidgetsPage, ClockFormat, Seconds, IslandChoice, Greeting]),
            ArrangeApps => {
                let apps = app_names();
                let mut page = Self::page("Arrange apps", arrange_items(&apps, &crate::shell::Arrangement::default()));
                page.apps = apps;
                page
            }
            Display => {
                let motion: &[Item] = if crate::imu::has_motion() { &[LiftWake, FaceDown] } else { &[] };
                Self::page("Display", [Brightness, Graphics, ScreenOff, Processor].into_iter().chain(motion.iter().copied()).collect())
            }
            Network => Self::page("Network", [WifiSetup, Tailscale].into_iter().chain((0..4).map(Info)).collect()),
            KeysSection => Self::page("Keys", (0..crate::keys::SERVICES.len()).map(KeyFor).chain(std::iter::once(KeysFromFile)).collect()),
            Access => Self::page("Access", vec![Ssh]),
            UpdateSection => Self::page("Updates", vec![UpdateState, UpdateAction, UpdateNote, UpdateChannel, UpdateAuto, UpdateOvernight]),
            Help => Self::page("Help", vec![TakeTour, KeyboardShortcuts, KeyboardLesson]),
            NameSection => Self::page("Name & time", vec![DeviceName, TimeZone, ClockNote]),
            UsbSection => Self::page("USB", vec![UsbMode, UsbLink]),
            CameraSection => Self::page("Camera", vec![DetailModeRow, GrovePort, SmartCamera, CameraProgram]),
            DeveloperSection => Self::page("Developer", vec![DevMode, MotionScale, ShowKeys, Undervolt, TearFree]),
            _ => return None,
        })
    }

    fn read_info(&mut self) {
        use crate::sys;
        self.read_at = std::time::Instant::now();
        self.network = sys::address("eth0").filter(|a| a.contains('.')).or_else(|| sys::address("wlan0")).unwrap_or_else(|| "–".into());
        self.backlight = sys::brightness();
        self.update = crate::sysupdate::read();
        self.info = match self.title {
            "Network" => vec![
                ("Name", dash(sys::hostname())),
                ("Ethernet", dash(sys::address("eth0"))),
                ("Wi-Fi", dash(sys::address("wlan0"))),
                ("USB link", dash(sys::address("usb0"))),
            ],
            _ => Vec::new(),
        };
    }

    /// What a row shows: its name, its value, and whether the value is "on".
    fn describe(&self, item: Item, g: &Gfx, robot: &Robot) -> (String, String, bool) {
        use Item::*;
        let onoff = |on: bool| (if on { "on" } else { "off" }).to_owned();
        let section = |name: &str, summary: String| (name.to_owned(), format!("{summary} ›"), false);
        match item {
            RobotSection => section("Robot", if robot.is_demo() { "demo".to_owned() } else { dash(Some(robot.address.clone()).filter(|a| !a.is_empty())) }),
            Appearance => section("Appearance", format!("{} · {}", g.theme.name(), if g.tokens.tone == bezel::Tone::Light { "light" } else { "dark" })),
            Display => section("Display", dash(self.backlight.map(|b| format!("{b}%")))),
            Network => section("Network", self.network.clone()),
            Access => section("Access", format!("ssh {}", onoff(crate::ssh_enabled()))),
            UpdateSection => section("System update", crate::sysupdate::summary(&self.update)),
            UpdateState => ("Status".into(), crate::sysupdate::summary(&self.update), self.update.state != "failed"),
            UpdateAction => {
                let name = match self.update.state.as_str() {
                    "available" => "Download and install",
                    "ready" => "Restart to update",
                    "checking" | "downloading" | "installing" | "trying" => "Working on it",
                    _ => "Check now",
                };
                (name.into(), "›".into(), true)
            }
            // What an update brings (or this system brought): opens its notes.
            UpdateNote => crate::changelog::update_row(&self.update),
            UpdateChannel => ("Channel".into(), crate::sysupdate::pref("channel", "stable"), true),
            UpdateAuto => {
                let on = crate::sysupdate::pref("auto", "on") == "on";
                ("Download automatically".into(), onoff(on), on)
            }
            UpdateOvernight => {
                let on = crate::sysupdate::pref("overnight", "off") == "on";
                ("Restart overnight".into(), onoff(on), on)
            }
            KeysSection => {
                let set = crate::keys::SERVICES.iter().filter(|(name, _)| self.keys.get(name).is_some()).count();
                section("Keys", format!("{set} of {} set", crate::keys::SERVICES.len()))
            }
            FromComputer => section("Set up from a computer", String::new()),
            KeyFor(i) => {
                let (name, label) = crate::keys::SERVICES[i];
                let value = match &self.typing {
                    Some((row, text)) if *row == i => format!("{}▏", text.chars().rev().take(16).collect::<Vec<_>>().into_iter().rev().collect::<String>()),
                    _ => self.keys.shown(name),
                };
                (label.to_owned(), value, self.keys.get(name).is_some())
            }
            KeysFromFile => ("Read from a file".into(), keys_file().display().to_string(), false),
            About => section("About", crate::sysupdate::version()),
            Help => section("Help", "tour · keys".to_owned()),
            NameSection => section("Name & time", crate::device::name()),
            DeviceName => match &self.naming {
                Some(typed) => ("Name".into(), format!("{typed}▏"), true),
                None => ("Name".into(), crate::device::name(), true),
            },
            TimeZone => ("Time zone".into(), crate::device::zone_label(&crate::device::zone()), true),
            ClockNote => ("Clock".into(), crate::device::clock_note(), false),
            KeyboardLesson => ("Keyboard lesson".into(), "›".into(), true),
            UsbSection => section("USB", crate::usb::MODES[crate::usb::mode()].1.to_owned()),
            UsbMode => {
                let at = crate::usb::mode();
                ("Plugged into a computer".into(), crate::usb::MODES[at].1.into(), at != 0)
            }
            UsbLink => match crate::usb::connected() {
                // The controller takes the device role at the next start.
                _ if crate::usb::restart_needed() => ("Computer".into(), "after a restart".into(), false),
                Some(true) => ("Computer".into(), "connected".into(), true),
                Some(false) => ("Computer".into(), "not connected".into(), false),
                None => ("Computer".into(), "on the handheld".into(), false),
            },
            TakeTour => ("Take the tour".into(), "›".into(), true),
            KeyboardShortcuts => ("Keyboard shortcuts".into(), "›".into(), true),
            DeveloperSection => section("Developer options", "on".to_owned()),
            DevMode => ("Use developer options".into(), onoff(g.developer), g.developer),
            MotionScale => (
                "Animation scale".into(),
                if g.motion_scale == 1.0 { "1× · as designed".to_owned() } else { format!("{}×", g.motion_scale) },
                g.motion_scale != 1.0,
            ),
            ShowKeys => ("Show key presses".into(), onoff(g.show_keys), g.show_keys),
            TearFree => {
                let state = crate::panel::trial();
                ("Tear-free panel".into(), crate::panel::line(), state.on())
            }
            Undervolt => ("Lower voltage".into(), crate::power::undervolt_line(), crate::power::undervolt().on()),
            BatterySection => section("Battery", crate::power::summary()),
            LockSection => section("Lock screen", (if crate::passcode::file().exists() { "passcode" } else { "no passcode" }).to_owned()),
            StandBySection => section("StandBy", (if g.lock.standby { "on the charger" } else { "off" }).to_owned()),
            OnDeviceModel => section("On-device model", crate::local::summary()),
            OfflineDictation => section("Offline dictation", crate::whisper_local::summary()),
            VoiceCommands => section("Voice commands", crate::voicecmd::summary()),
            SoundSection => section("Sound", crate::sound::summary()),
            BluetoothSection => section("Bluetooth", crate::bluetooth::summary()),
            MusicSection => section("Music", crate::music::summary()),
            CameraSection => section("Camera", format!("detail {}", g.detail.name())),
            DetailModeRow => ("Detail mode".into(), g.detail.words().into(), g.detail != crate::burst::DetailMode::Off),
            GrovePort => ("Grove port".into(), grove_words(), crate::smartcam::chosen()),
            SmartCamera => ("Smart camera".into(), crate::camera::smart_words(), false),
            CameraProgram => {
                let words = crate::unitv::words();
                let lit = words.starts_with("install") || words.starts_with("update");
                ("UnitV".into(), words, lit)
            }
            CompanionSection => section("Computer companion", crate::companion::summary()),
            Mode => ("Mode".into(), (if g.tokens.tone == bezel::Tone::Light { "light" } else { "dark" }).into(), true),
            AccentColor => ("Accent".into(), g.accent.name().into(), true),
            HomeScreen => ("Home screen".into(), g.home.name().into(), true),
            Columns => ("Tiles per row".into(), g.columns.to_string(), true),
            ThemeChoice => ("Theme".into(), g.theme.name().into(), true),
            OwnColour => (
                "Your own colour".into(),
                match g.accent {
                    bezel::Accent::Hue(_) => format!("{}°", g.accent.degrees().unwrap_or(0)),
                    _ => "off".into(),
                },
                matches!(g.accent, bezel::Accent::Hue(_)),
            ),
            WallpaperChoice => ("Wallpaper".into(), g.look.wallpaper.name().into(), true),
            CornersChoice => ("Corners".into(), g.look.corners.name().into(), true),
            LettersChoice => ("Letters".into(), g.look.letters.name().into(), true),
            WidthChoice => ("Letter width".into(), g.look.width.name().into(), true),
            TextSection => section("Text", format!("{} · {}", g.text_size.name(), g.look.letters.name())),
            HomeSection => section("Home", format!("{} · {} across", g.home.name(), g.columns)),
            WidgetsPage => section(
                "Widgets",
                match (g.widgets.glance.len(), g.widgets.home.len()) {
                    (0, 0) => "none".to_owned(),
                    (n, 0) => format!("{n} on the glance page"),
                    (0, n) => format!("{n} among the apps"),
                    (glance, home) => format!("{glance} glance · {home} home"),
                },
            ),
            ArrangeApps => section(
                "Arrange apps",
                match g.arrangement.hidden.len() {
                    0 => "all shown".to_owned(),
                    n => format!("{n} hidden"),
                },
            ),
            AppTile(i) => {
                let name = self.apps.get(i).cloned().unwrap_or_default();
                let hidden = |n: &str| n != "Settings" && g.arrangement.hidden.iter().any(|h| h == n);
                if hidden(&name) {
                    (name, "hidden".into(), false)
                } else {
                    // Its place among those shown.
                    let place = self
                        .items
                        .iter()
                        .take_while(|it| **it != item)
                        .filter(|it| matches!(it, AppTile(j) if !hidden(self.apps.get(*j).map_or("", String::as_str))))
                        .count();
                    (name, format!("on · {}", place + 1), true)
                }
            }
            ClockFormat => ("Clock".into(), (if g.clock.0 { "24-hour" } else { "12-hour" }).into(), true),
            Seconds => ("Seconds on the clock".into(), onoff(g.clock.1), g.clock.1),
            IslandChoice => ("Island at rest".into(), g.island.name().into(), true),
            Greeting => ("Greeting".into(), onoff(g.greeting), g.greeting),
            TextScale => ("Text size".into(), g.text_size.name().into(), true),
            Calm => ("Calm version".into(), onoff(g.calm), g.calm),
            Brightness => ("Brightness".into(), dash(self.backlight.map(|b| format!("{b}%"))), true),
            // What draws, and why (About has the timings that decided it).
            Graphics => ("Graphics".into(), crate::gpu::plain(g.gpu_wanted(), g.gpu.as_ref()), g.gpu.is_some()),
            ScreenOff => ("Screen off after".into(), crate::standby::screen_off_name(g.screen_off), g.screen_off != 0),
            Processor => ("Processor".into(), g.power.name().into(), true),
            LiftWake => ("Lift to wake".into(), onoff(g.lift_wake), g.lift_wake),
            FaceDown => ("Face down to sleep".into(), onoff(g.face_down), g.face_down),
            WifiSetup => section("Wi-Fi networks", "join".to_owned()),
            Tailscale => section("Tailscale", (if crate::tailscale::enabled() { "on" } else { "off" }).to_owned()),
            Ssh => ("SSH access".into(), onoff(crate::ssh_enabled()), crate::ssh_enabled()),
            DemoRobot => ("Demo robot".into(), onoff(robot.is_demo()), robot.is_demo()),
            OpenRobot => ("Open Robot on link".into(), onoff(g.open_robot), g.open_robot),
            MatchReport => ("Match report after a match".into(), onoff(g.match_report), g.match_report),
            Info(i) => self.info.get(i).map(|(name, value)| ((*name).to_owned(), value.clone(), false)).unwrap_or_default(),
        }
    }

    /// Enter, Left or Right on a row. `step` is −1 or +1; Enter is +1.
    fn change(&mut self, item: Item, step: i32, cx: &mut Cx) {
        use Item::*;
        match item {
            Mode => cx.settings.light = !cx.settings.light,
            AccentColor => {
                let all = bezel::Accent::ALL;
                let at = all.iter().position(|a| *a == cx.settings.accent).unwrap_or(0) as i32;
                cx.settings.accent = all[(at + step).rem_euclid(all.len() as i32) as usize];
            }
            HomeScreen => cx.settings.home = cx.settings.home.step(step),
            Columns => cx.settings.columns = if cx.settings.columns == 3 { 2 } else { 3 },
            ClockFormat => cx.settings.clock24 = !cx.settings.clock24,
            Seconds => cx.settings.seconds = !cx.settings.seconds,
            IslandChoice => cx.settings.island = cx.settings.island.step(step),
            Greeting => cx.settings.greeting = !cx.settings.greeting,
            ThemeChoice => {
                // A theme brings its own wallpaper and accent.
                cx.settings.theme = cx.settings.theme.step(step);
                (cx.settings.wallpaper, cx.settings.accent) = cx.settings.theme.defaults();
            }
            OwnColour => {
                let hues = bezel::Accent::HUES as i32;
                cx.settings.accent = match cx.settings.accent {
                    bezel::Accent::Hue(i) => bezel::Accent::Hue((i as i32 + step).rem_euclid(hues) as u8),
                    // Starting from your own colour: the hue nearest the accent in use.
                    _ => bezel::Accent::Hue(nearest_hue(cx.settings.accent)),
                };
            }
            WallpaperChoice => cx.settings.wallpaper = cx.settings.wallpaper.step(step),
            CornersChoice => cx.settings.corners = cx.settings.corners.step(step),
            LettersChoice => cx.settings.letters = cx.settings.letters.step(step),
            WidthChoice => cx.settings.width = cx.settings.width.step(step),
            TextScale => cx.settings.text = if step > 0 { cx.settings.text.next() } else { cx.settings.text.next().next() },
            Calm => cx.settings.calm = !cx.settings.calm,
            Graphics => cx.settings.gpu = !cx.settings.gpu,
            ScreenOff => {
                let all = crate::standby::SCREEN_OFF_CHOICES;
                let at = all.iter().position(|s| *s == cx.settings.screen_off).unwrap_or(1) as i32;
                cx.settings.screen_off = all[(at + step).rem_euclid(all.len() as i32) as usize];
            }
            Processor => {
                cx.settings.power = cx.settings.power.step(step);
                // The kernel's governors are on the handheld; a computer has none to set.
                if crate::sys::governors().is_empty() {
                    *cx.say = Some("the processor is set on the handheld".into());
                }
            }
            Brightness => match crate::sys::brightness() {
                Some(now) => {
                    crate::sys::set_brightness((now as i32 + step * 10).clamp(10, 100) as u32);
                    self.backlight = crate::sys::brightness();
                    // Kept, for the next start (saved below).
                    cx.settings.brightness = self.backlight.unwrap_or(now);
                }
                None => *cx.say = Some("the backlight is on the device".into()),
            },
            UpdateAction => {
                use crate::sysupdate::run;
                let u = &self.update;
                let started = match u.state.as_str() {
                    "available" => run("install").map(|()| format!("downloading {}", u.version)),
                    "ready" => run("apply").map(|()| format!("restarting into {}", u.version)),
                    "checking" | "downloading" | "installing" | "trying" => Ok("already working on it".into()),
                    _ => run("check").map(|()| "checking for updates".into()),
                };
                *cx.say = Some(started.unwrap_or_else(|why| why));
            }
            UpdateChannel => {
                let next = if crate::sysupdate::pref("channel", "stable") == "stable" { "nightly" } else { "stable" };
                crate::sysupdate::set_pref("channel", next);
                // What is newest depends on the channel: look again.
                crate::sysupdate::run("check").ok();
            }
            UpdateAuto => crate::sysupdate::set_pref("auto", if crate::sysupdate::pref("auto", "on") == "on" { "off" } else { "on" }),
            UpdateOvernight => crate::sysupdate::set_pref("overnight", if crate::sysupdate::pref("overnight", "off") == "on" { "off" } else { "on" }),
            Ssh => {
                // Off unless switched on here; key-only logins.
                let on = !crate::ssh_enabled();
                *cx.say = Some(match crate::set_ssh(on) {
                    Ok(()) if on => "ssh on · keys in /data/ssh".into(),
                    Ok(()) => "ssh off".into(),
                    Err(why) => why,
                });
            }
            DemoRobot => {
                *cx.robot = if cx.robot.is_demo() { Robot::none() } else { Robot::demo() };
                *cx.say = Some(if cx.robot.is_demo() { "demo robot on".into() } else { "demo robot off".into() });
            }
            KeyFor(i) => self.typing = Some((i, String::new())),
            KeysFromFile => {
                let file = keys_file();
                *cx.say = Some(match crate::keys::Keys::read(&file) {
                    Some(found) => {
                        let count = crate::keys::SERVICES.iter().filter(|(name, _)| found.get(name).is_some()).count();
                        self.keys.merge(&found);
                        match self.keys.save() {
                            Ok(()) => format!("{count} key{} read", if count == 1 { "" } else { "s" }),
                            Err(_) => "could not keep the keys".into(),
                        }
                    }
                    None => format!("no {} yet", file.display()),
                });
            }
            OpenRobot => cx.settings.open_robot = !cx.settings.open_robot,
            DetailModeRow => cx.settings.detail = cx.settings.detail.step(step),
            // At once, off the screen's thread; the island says how it went.
            GrovePort => crate::smartcam::switch_grove(!crate::smartcam::chosen()),
            MatchReport => cx.settings.match_report = !cx.settings.match_report,
            LiftWake => cx.settings.lift_wake = !cx.settings.lift_wake,
            FaceDown => cx.settings.face_down = !cx.settings.face_down,
            MotionScale => {
                let all = crate::shell::MOTION_SCALES;
                let at = all.iter().position(|s| *s == cx.settings.motion_scale).unwrap_or(1) as i32;
                cx.settings.motion_scale = all[(at + step).clamp(0, all.len() as i32 - 1) as usize];
            }
            ShowKeys => cx.settings.show_keys = !cx.settings.show_keys,
            // Switching it on asks first and restarts (the key handler); here, off.
            Undervolt => *cx.say = Some(crate::power::set_undervolt(false).unwrap_or_else(|why| why)),
            // On: the handheld restarts into it and asks whether to keep it.
            TearFree => *cx.say = Some(crate::panel::switch(!crate::panel::trial().on()).unwrap_or_else(|why| why)),
            UsbMode => {
                let n = crate::usb::MODES.len() as i32;
                let next = (crate::usb::mode() as i32 + step).rem_euclid(n) as usize;
                *cx.say = Some(match crate::usb::set(next) {
                    // A computer mode: the switch to the left, and a restart
                    // the first time (the controller's role is set at start).
                    Ok(()) if next != 0 && std::path::Path::new("/usr/sbin/catalyst-usb").exists() && !crate::usb::running_as_device() => {
                        "restart · usb switch to the left".to_owned()
                    }
                    Ok(()) => format!("usb: {}", crate::usb::MODES[next].1),
                    Err(why) => why,
                });
            }
            _ => {}
        }
        crate::save_settings(cx.settings);
    }
}

impl Screen for SettingsApp {
    fn title(&self) -> &str {
        self.title
    }

    fn wants_text(&self) -> bool {
        self.typing.is_some() || self.naming.is_some()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.naming.is_some() {
            return vec![keys("naming it", &[("enter", "Keep the name"), ("esc", "Leave it as it was"), ("bksp", "Delete a letter")])];
        }
        if self.typing.is_some() {
            return vec![keys("the key", &[("enter", "Keep the key"), ("esc", "Leave it as it was"), ("bksp", "Delete a letter")])];
        }
        let here = match self.items.get(self.list.focus) {
            Some(Item::AppTile(_)) => keys("this app", &[("enter", "Show or hide it"), ("space", "Favourite or not"), ("← →", "Move it")]),
            Some(Item::DeviceName) => keys("this row", &[("enter", "Rename the handheld")]),
            Some(Item::ClockNote) => keys("this row", &[("enter · →", "Set the clock now")]),
            _ => keys("settings", &[("enter · →", "Open the page"), ("← →", "Change the value"), ("enter", "Next value")]),
        };
        vec![here, keys("moving", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        // A new name being typed: Enter keeps it, Esc leaves the one there was.
        if let Some(name) = &mut self.naming {
            match key {
                Key::Char(c) => crate::device::type_name(name, c),
                Key::Backspace => {
                    name.pop();
                }
                Key::Enter => {
                    let typed = std::mem::take(name);
                    self.naming = None;
                    *cx.say = Some(match crate::device::set_name(&typed) {
                        Ok(kept) => format!("this handheld is {kept}"),
                        Err(why) => why,
                    });
                }
                Key::Back | Key::Leave => self.naming = None,
                _ => {}
            }
            return Nav::Stay;
        }
        // A key being typed: Enter keeps it, Esc leaves it as it was.
        if let Some((service, text)) = &mut self.typing {
            match key {
                Key::Char(c) if !c.is_control() && text.len() < 200 => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Enter => {
                    let (name, value) = (crate::keys::SERVICES[*service].0, text.clone());
                    self.keys.set(name, &value);
                    self.typing = None;
                    *cx.say = Some(if self.keys.save().is_ok() { "key kept".into() } else { "could not keep the key".into() });
                }
                Key::Back => self.typing = None,
                _ => {}
            }
            return Nav::Stay;
        }
        let Some(&item) = self.items.get(self.list.focus) else { return Nav::Pop };
        // An app on Arrange apps: Enter shows or hides it; Space makes it a
        // favourite or not; Left and Right move it earlier or later among the
        // favourites or among the rest. The focus stays on it.
        if let Item::AppTile(i) = item {
            let name = self.apps[i].clone();
            let mut order: Vec<String> = self.items.iter().filter_map(|it| if let Item::AppTile(j) = it { self.apps.get(*j).cloned() } else { None }).collect();
            let at = order.iter().position(|n| *n == name).unwrap_or(0);
            let favourite = |n: &String| cx.settings.favourites.contains(n);
            let same_side = |a: usize, b: usize| order.get(b).is_some_and(|n| favourite(n) == favourite(&order[a]));
            let (earlier, later) = (at > 0 && same_side(at, at - 1), same_side(at, at + 1));
            match key {
                Key::Char(' ') => match cx.settings.favourites.iter().position(|f| *f == name) {
                    Some(f) => {
                        cx.settings.favourites.remove(f);
                        *cx.say = Some(format!("{} is not a favourite", name.to_lowercase()));
                    }
                    None => {
                        cx.settings.favourites.push(name.clone());
                        *cx.say = Some(format!("{} pinned first", name.to_lowercase()));
                    }
                },
                Key::Back => return Nav::Pop,
                Key::Enter if name == "Settings" => {
                    *cx.say = Some("settings stays on home".into());
                    return Nav::Stay;
                }
                Key::Enter => match cx.settings.hidden.iter().position(|h| *h == name) {
                    Some(h) => {
                        cx.settings.hidden.remove(h);
                    }
                    None => cx.settings.hidden.push(name.clone()),
                },
                Key::Left if earlier => order.swap(at, at - 1),
                Key::Right if later => order.swap(at, at + 1),
                other => {
                    list_key(&mut self.list, other, self.items.len());
                    return Nav::Stay;
                }
            }
            cx.settings.order = order;
            self.items = arrange_items(&self.apps, &cx.settings.arrangement());
            self.list.focus = self.items.iter().position(|it| *it == Item::AppTile(i)).unwrap_or(0);
            crate::save_settings(cx.settings);
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right if item == Item::WifiSetup => return Nav::Push(Box::new(WifiScreen::new())),
            Key::Enter | Key::Right if item == Item::Tailscale => return Nav::Push(Box::new(crate::tailscale::TailscaleScreen::new())),
            Key::Enter | Key::Right if item == Item::FromComputer => return Nav::Push(Box::new(crate::setup::Setup::from_computer())),
            Key::Enter | Key::Right if item == Item::About => return Nav::Push(Box::new(crate::about::About::new(crate::about::Kind::Main))),
            Key::Enter | Key::Right if item == Item::UpdateNote => {
                if let Some(page) = crate::changelog::update_page(&self.update) {
                    return Nav::Push(Box::new(page));
                }
            }
            Key::Enter | Key::Right if item == Item::TakeTour => return Nav::Tour,
            Key::Enter | Key::Right if item == Item::WidgetsPage => return Nav::Push(Box::new(crate::widgets::WidgetsPage::new())),
            Key::Enter | Key::Right if item == Item::KeyboardShortcuts => return Nav::Push(Box::new(crate::help::Shortcuts::new())),
            Key::Enter | Key::Right if item == Item::BatterySection => return Nav::Push(Box::new(crate::power::BatteryPage::new())),
            Key::Enter | Key::Right if item == Item::LockSection => return Nav::Push(Box::new(crate::lockpages::LockPage::new())),
            Key::Enter | Key::Right if item == Item::StandBySection => return Nav::Push(Box::new(crate::lockpages::StandByPage::new())),
            Key::Enter | Key::Right if item == Item::OnDeviceModel => return Nav::Push(Box::new(crate::local::ModelPage::new())),
            Key::Enter | Key::Right if item == Item::OfflineDictation => return Nav::Push(Box::new(crate::whisper_local::DictationPage::new())),
            Key::Enter | Key::Right if item == Item::VoiceCommands => return Nav::Push(Box::new(crate::voicecmd::CommandsPage::new())),
            Key::Enter | Key::Right if item == Item::CameraProgram => return Nav::Push(Box::new(crate::unitv::InstallScreen::new())),
            Key::Enter | Key::Right if item == Item::SmartCamera => return Nav::Push(Box::new(crate::smartcam_status::StatusPage::new())),
            Key::Enter | Key::Right if item == Item::SoundSection => return Nav::Push(Box::new(crate::sound::SoundScreen::new())),
            Key::Enter | Key::Right if item == Item::BluetoothSection => return Nav::Push(Box::new(crate::bluetooth::BluetoothScreen::new())),
            Key::Enter | Key::Right if item == Item::MusicSection => return Nav::Push(Box::new(crate::music::MusicSettings::new())),
            Key::Enter | Key::Right if item == Item::CompanionSection => return Nav::Push(Box::new(crate::companion::CompanionScreen::new())),
            // Lower voltage, switched on: what it does, and a restart to try it.
            Key::Enter | Key::Right if item == Item::Undervolt && crate::power::undervolt().asks_to_try() => return Nav::Push(Box::new(crate::power::TryUndervolt::new())),
            Key::Enter | Key::Right if item == Item::KeyboardLesson => return Nav::Push(Box::new(crate::setup::Setup::lesson())),
            Key::Enter | Key::Right if item == Item::TimeZone => return Nav::Push(Box::new(crate::device::ZonePicker::new())),
            Key::Enter if item == Item::DeviceName => self.naming = Some(crate::device::name()),
            // The clock set from the network now: the time servers, else the
            // web (for a network that blocks them). The row says how it went.
            Key::Enter | Key::Right if item == Item::ClockNote => *cx.say = Some(crate::device::set_clock_now()),
            Key::Left if item == Item::ClockNote => {}
            // Switched off, developer options go, and their page with them.
            Key::Enter | Key::Right | Key::Left if item == Item::DevMode => {
                let s = &mut *cx.settings;
                (s.developer, s.motion_scale, s.show_keys) = (false, 1.0, false);
                crate::save_settings(s);
                *cx.say = Some("developer options off".into());
                return Nav::Pop;
            }
            Key::Enter | Key::Right | Key::Left => {
                if let Some(page) = Self::section(item) {
                    if key != Key::Left {
                        return Nav::Push(Box::new(page));
                    }
                } else {
                    self.change(item, if key == Key::Left { -1 } else { 1 }, cx);
                }
            }
            other => list_key(&mut self.list, other, self.items.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        if self.read_at.elapsed().as_secs_f32() < 2.0 {
            return false;
        }
        let before = (self.info.clone(), self.network.clone(), self.backlight);
        self.read_info();
        before != (self.info.clone(), self.network.clone(), self.backlight)
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let (top, row_h) = (g.top(), g.row_h());
        // Developer options are listed after About once they are unlocked.
        if self.title == "Settings" && g.developer != self.items.contains(&Item::DeveloperSection) {
            let focused = self.items.get(self.list.focus).copied();
            if g.developer {
                let at = self.items.iter().position(|it| *it == Item::About).map_or(self.items.len(), |a| a + 1);
                self.items.insert(at, Item::DeveloperSection);
            } else {
                self.items.retain(|it| *it != Item::DeveloperSection);
            }
            self.list.focus = focused.and_then(|f| self.items.iter().position(|it| *it == f)).unwrap_or(0);
        }
        let arranging = self.title == "Arrange apps";
        if arranging {
            let order = arrange_items(&self.apps, &g.arrangement);
            if order != self.items {
                let focused = self.items.get(self.list.focus).copied();
                self.items = order;
                self.list.focus = focused.and_then(|f| self.items.iter().position(|it| *it == f)).unwrap_or(0);
            }
        }
        let items = self.items.clone();
        // Arrange apps says what its keys do along the bottom, USB what the
        // mode does, Name & time what the name is for; the rest run on to it.
        let hinted = arranging || matches!(self.title, "USB" | "Name & time" | "Camera") || self.naming.is_some();
        let bottom = if hinted { g.above_hint() } else { H };
        for (i, y) in self.list.layout(g, items.len(), top, row_h, 3.0, bottom) {
            let item = items[i];
            let (name, value, on) = self.describe(item, g, robot);
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match item {
                // A favourite's star, just before its place.
                Item::AppTile(j) => {
                    g.row(r, &name, &value, if on { t.ink } else { t.dim }, focused);
                    if self.apps.get(j).is_some_and(|n| g.arrangement.favourites.contains(n)) {
                        let value_w = g.measure(&value, g.ty.label);
                        g.icon("star", true, r.right() - 10.0 - value_w - 10.0, r.cy(), 12.0, t.ink);
                    }
                }
                // The three Homes as pictures of themselves, the chosen one
                // lit and ringed, its name beside them.
                Item::HomeScreen => {
                    g.row(r, &name, &value, t.ink, focused);
                    let value_w = g.measure(&value, if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label });
                    let (cw, gap) = (22.0, 5.0);
                    let styles = HomeStyle::ALL;
                    for (n, style) in styles.iter().enumerate() {
                        let x = r.right() - 10.0 - value_w - 10.0 - (styles.len() - n) as f32 * (cw + gap) + gap;
                        home_swatch(g, *style, Rect::new(x, r.cy() - 7.0, cw, 14.0), *style == g.home);
                    }
                }
                // The accents themselves, the chosen one ringed.
                Item::AccentColor => {
                    g.row(r, &name, "", t.dim, focused);
                    let all = bezel::Accent::ALL;
                    for (n, accent) in all.iter().enumerate() {
                        let cx = r.right() - 14.0 - (all.len() - 1 - n) as f32 * 17.0;
                        let swatch = bezel::Tokens::of(*accent, t.tone).signal;
                        if *accent == g.accent {
                            g.dot(cx, r.cy(), 7.5, t.ink);
                            g.dot(cx, r.cy(), 6.0, if focused { t.surface2 } else { t.surface1 });
                        }
                        g.dot(cx, r.cy(), 4.5, swatch);
                    }
                }
                // The wheel laid flat: a strip running through every hue in
                // its own signal colour, the chosen one marked, its angle
                // beside it.
                Item::OwnColour => {
                    g.row(r, &name, &value, if on { t.ink } else { t.dim }, focused);
                    let hues = bezel::Accent::HUES as usize;
                    let value_w = if value.is_empty() { 0.0 } else { g.measure(&value, g.ty.label) + 8.0 };
                    let strip = Rect::new(r.right() - 10.0 - value_w - 96.0, r.cy() - 3.0, 96.0, 6.0);
                    let track = strip.offset(g.ox, 0.0);
                    for k in 0..strip.w as usize {
                        let hue = bezel::Accent::Hue((k * hues / strip.w as usize) as u8);
                        let column = Rect::new(track.x + k as f32, track.y, 1.0, track.h);
                        g.canvas.rrect_clipped(column, 0.0, track, track.h * 0.5, bezel::Tokens::of(hue, t.tone).signal, 1.0);
                    }
                    if let bezel::Accent::Hue(i) = g.accent {
                        let x = strip.x + (i as f32 + 0.5) * strip.w / hues as f32;
                        g.dot(x, r.cy(), 6.0, t.ink);
                        g.dot(x, r.cy(), 4.5, t.signal);
                    }
                }
                // While it downloads or installs, how far it has come.
                Item::UpdateState if matches!(self.update.state.as_str(), "downloading" | "installing") => {
                    let value_w = g.measure(&value, g.ty.label) + 22.0;
                    g.row(r, &name, &value, t.ink, focused);
                    let name_w = g.measure(&name, g.ty.body) + 22.0;
                    g.meter(Rect::new(r.x + name_w, r.cy() - 3.0, (r.w - name_w - value_w).max(10.0), 6.0), self.update.progress, t.ice);
                }
                Item::Brightness => {
                    let value_w = g.measure("100%", g.ty.label) + 22.0;
                    g.row(r, &name, &value, t.ink, focused);
                    let name_w = g.measure(&name, g.ty.body) + 22.0;
                    let level = self.backlight.unwrap_or(0) as f32 / 100.0;
                    g.meter(Rect::new(r.x + name_w, r.cy() - 3.0, r.w - name_w - value_w, 6.0), level, t.ice);
                }
                _ => g.row(r, &name, &value, if on { t.ink } else { t.dim }, focused),
            }
        }
        self.list.scrollbar(g, items.len(), top, bottom - 2.0);
        if arranging {
            g.hint("enter: hide · space: favourite · ←→: move");
        }
        if self.title == "Camera" {
            g.hint_any(camera_hint(self.items.get(self.list.focus).copied()));
        }
        if self.title == "USB" {
            g.hint(crate::usb::MODES[crate::usb::mode()].2);
        }
        if self.naming.is_some() {
            g.hint("letters, digits, hyphens · enter keeps it · esc: as it was");
        } else if self.title == "Name & time" {
            g.hint("the name a network knows it by");
        }
    }
}

// ----------------------------------------------------------------- wi-fi ---

/// Wi-Fi: switch it on, see what is in range, join a network, forget one.
/// Passwords are typed on the keyboard and kept on the data partition by the
/// system's Wi-Fi service.
pub struct WifiScreen {
    wifi: Wifi,
    list: ListState,
    seen: u64,
    /// The network a password is being typed for, and what has been typed.
    entering: Option<(String, String)>,
    /// Opened from setup: once a network chosen here is joined and has an
    /// address, back to setup by itself (after a moment saying so).
    back_when_joined: bool,
    joining: bool,
    online_for: Option<f32>,
}

/// How long "connected" shows before the page goes back to setup.
const JOINED_PAUSE: f32 = 1.2;

impl WifiScreen {
    pub fn new() -> Self {
        WifiScreen { wifi: Wifi::open(), list: ListState::default(), seen: u64::MAX, entering: None, back_when_joined: false, joining: false, online_for: None }
    }

    /// From setup: back there by itself once joined.
    pub fn for_setup() -> Self {
        WifiScreen { back_when_joined: true, ..Self::new() }
    }

    /// A network a QR code named, joined now: the list showing it, and back
    /// by itself once that network (not just any) has given an address.
    pub fn joining(ssid: &str, password: Option<String>) -> Self {
        let page = WifiScreen { back_when_joined: true, joining: true, ..Self::new() };
        page.wifi.join(ssid, password);
        JOINING.with(|j| *j.borrow_mut() = Some(ssid.to_owned()));
        page
    }
}

thread_local! {
    /// The network `WifiScreen::joining` waits for.
    static JOINING: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

impl Screen for WifiScreen {
    fn title(&self) -> &str {
        "Wi-Fi"
    }

    fn wants_text(&self) -> bool {
        self.entering.is_some()
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.entering.is_some() {
            return vec![keys("the password", &[("enter", "Join"), ("esc", "Cancel"), ("bksp", "Delete a letter")])];
        }
        let here = if self.list.focus == 0 {
            keys("wi-fi", &[("enter", "Wi-Fi on or off"), ("r", "Look again")])
        } else {
            keys("a network", &[("enter", "Join it"), ("tab", "Forget it"), ("r", "Look again")])
        };
        vec![here, keys("moving", LIST_KEYS)]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((ssid, password)) = &mut self.entering {
            match key {
                // WPA passwords are 8 to 63 printable characters.
                Key::Char(c) if password.len() < 63 && (' '..='~').contains(&c) => password.push(c),
                Key::Backspace => {
                    password.pop();
                }
                Key::Back => self.entering = None,
                Key::Enter if password.len() >= 8 => {
                    self.wifi.join(ssid, Some(password.clone()));
                    *cx.say = Some(format!("joining {ssid}"));
                    self.entering = None;
                    (self.joining, self.online_for) = (true, None);
                }
                Key::Enter => *cx.say = Some("at least 8 characters".into()),
                _ => {}
            }
            return Nav::Stay;
        }
        let state = self.wifi.state();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if self.list.focus == 0 => self.wifi.enable(!state.enabled),
            Key::Enter => match state.networks.get(self.list.focus - 1) {
                Some(network) if network.current => *cx.say = Some("already connected".into()),
                Some(network) if network.saved.is_some() || !network.secured => {
                    self.wifi.join(&network.ssid, None);
                    *cx.say = Some(format!("joining {}", network.ssid));
                    (self.joining, self.online_for) = (true, None);
                }
                Some(network) => self.entering = Some((network.ssid.clone(), String::new())),
                None => {}
            },
            Key::Tab => {
                if let Some((network, id)) = state.networks.get(self.list.focus.wrapping_sub(1)).and_then(|n| Some((n, n.saved?))) {
                    self.wifi.forget(id);
                    *cx.say = Some(format!("forgot {}", network.ssid));
                }
            }
            Key::Char('r') => self.wifi.scan(),
            other => list_key(&mut self.list, other, state.networks.len() + 1),
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let generation = self.wifi.generation();
        let changed = generation != self.seen;
        self.seen = generation;
        // From setup: joined, with an address, counts the moment before going back.
        if self.back_when_joined && self.joining {
            let state = self.wifi.state();
            // Joining one by name: that one, current, before it counts.
            let on_it = JOINING.with(|j| j.borrow().as_ref().is_none_or(|want| state.networks.iter().any(|n| n.current && &n.ssid == want)));
            match (&mut self.online_for, state.online() && on_it) {
                (Some(t), true) => *t += dt,
                (None, true) => self.online_for = Some(0.0),
                (_, false) => self.online_for = None,
            }
        }
        changed
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if self.online_for.is_some_and(|t| t >= JOINED_PAUSE) {
            self.online_for = None;
            self.joining = false;
            JOINING.with(|j| *j.borrow_mut() = None);
            *cx.say = Some(format!("connected · {}", self.wifi.state().status));
            return Nav::Pop;
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h) = (g.top(), g.row_h());
        if let Some((ssid, password)) = &self.entering {
            // One thing on the page: the password.
            let label = g.text.fit(&format!("password for {ssid}"), &ty.caption, W - 2.0 * PAD - 18.0);
            let field = Rect::new(PAD, top, W - 2.0 * PAD, ty.caption.size + ty.label.size + 20.0);
            g.surface(field, g.tile_radius(), t.surface2);
            g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, &label, ty.caption, t.dim);
            let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
            // The end of a long password stays in view.
            let room = field.w - 26.0;
            let mut shown: &str = password;
            while g.measure(shown, ty.label) > room && !shown.is_empty() {
                shown = &shown[shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i)..];
            }
            let w = g.text_at(field.x + 10.0, cy, Align::Left, shown, ty.label, t.ink);
            g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
            g.focus_ring(field, g.tile_radius());
            let count = if password.len() >= 8 { format!("{} characters", password.len()) } else { format!("{} characters · needs 8 or more", password.len()) };
            g.text_at(PAD + 10.0, field.bottom() + 6.0 + ty.caption.size * 0.5, Align::Left, &count, ty.caption, if password.len() >= 8 { t.dim } else { t.faint });
            g.hint("enter: join · esc: cancel");
            return;
        }
        let state = self.wifi.state();
        let bottom = g.above_hint();
        let rows = state.networks.len() + 1;
        for (i, y) in self.list.layout(g, rows, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if i == 0 {
                g.row(r, "Wi-Fi", if state.enabled { "on" } else { "off" }, if state.enabled { t.ink } else { t.dim }, focused);
                continue;
            }
            let network = &state.networks[i - 1];
            const STRENGTH: [&str; 5] = ["out of range", "weak", "fair", "good", "strong"];
            let value = if network.current {
                "connected".to_owned()
            } else {
                let kind = if network.saved.is_some() { "saved · " } else if !network.secured { "open · " } else { "" };
                format!("{kind}{}", STRENGTH[network.bars()])
            };
            g.row(r, &network.ssid, &value, if network.current { t.ink } else { t.dim }, focused);
        }
        self.list.scrollbar(g, rows, top, bottom);
        // The hint says what Enter would do to the row in focus.
        let hint = match self.list.focus.checked_sub(1).and_then(|i| state.networks.get(i)) {
            _ if !state.enabled => "enter: switch wi-fi on".to_owned(),
            Some(network) if network.saved.is_some() && !network.current => "enter: join · tab: forget".to_owned(),
            Some(network) if network.current => format!("{} · tab: forget", state.status),
            Some(_) => "enter: join".to_owned(),
            None => state.status.clone(),
        };
        let fitted = g.text.fit(&hint, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `?` over any built-in app shows keys of its own, each row whole on
    /// one line of the sheet at every text size.
    #[test]
    fn every_built_in_app_lists_its_keys_and_they_fit() {
        let robot = Robot::demo();
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut missing = Vec::new();
        for (i, app) in APPS.iter().enumerate() {
            let screen = open_builtin(i, &robot);
            let groups = screen.key_sheet();
            if groups.is_empty() {
                missing.push(app.name);
                continue;
            }
            if let Err(why) = crate::keysheet::fits(&mut g, &groups) {
                panic!("{}: {why}", app.name);
            }
        }
        assert!(missing.is_empty(), "no keys listed for {missing:?}");
    }

    #[test]
    fn name_and_time_rows_fit_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        // Its title beside the island: no wider than Settings' own "Appearance".
        let ty = g.ty;
        assert!(g.measure("Name & time", ty.title) <= g.measure("Appearance", ty.title) + 9.0);
        assert!(g.measure("Time zone", ty.title) <= g.measure("Appearance", ty.title));
        let width = W - 2.0 * PAD - 5.0;
        let notes = ["network, 59 min ago", "network, 23 h ago", "network, just now", "clock chip", "this computer", "not set yet"];
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            let fits = |g: &mut Gfx, name: &str, value: &str| g.measure(name, ty.body) + 30.0 + g.measure(value, ty.label) <= width;
            for zone in crate::device::ZONES {
                let value = crate::device::zone_label(zone.id);
                assert!(fits(&mut g, "Time zone", &value), "{value} at {size:?}");
            }
            for note in notes {
                assert!(fits(&mut g, "Clock", note), "{note} at {size:?}");
            }
            let longest = "a".repeat(crate::device::NAME_MOST);
            assert!(fits(&mut g, "Name", &format!("{longest}▏")), "a name as long as one can be, at {size:?}");
        }
    }

    #[test]
    fn the_handheld_is_renamed_in_settings() {
        crate::shell::tests::sandbox();
        let mut page = SettingsApp::section(Item::NameSection).unwrap();
        assert!(page.items == [Item::DeviceName, Item::TimeZone, Item::ClockNote]);
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Enter, &mut cx);
        assert!(page.wants_text(), "typing a name");
        for _ in 0..crate::device::NAME_MOST {
            page.key(Key::Backspace, &mut cx);
        }
        for c in "Bench Two".chars() {
            page.key(Key::Char(c), &mut cx);
        }
        assert_eq!(page.naming.as_deref(), Some("bench-two"));
        page.key(Key::Enter, &mut cx);
        assert!(!page.wants_text());
        assert_eq!(say.as_deref(), Some("this handheld is bench-two"));
        // Esc while typing leaves it as it was.
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Enter, &mut cx);
        page.key(Key::Char('x'), &mut cx);
        page.key(Key::Back, &mut cx);
        assert!(page.naming.is_none());
        // The time zone row opens the picker; the clock's row is only to read.
        page.list.focus = 1;
        assert!(matches!(page.key(Key::Enter, &mut cx), Nav::Push(_)));
        page.list.focus = 2;
        assert!(matches!(page.key(Key::Enter, &mut cx), Nav::Stay));
    }

    fn names(home: &Home) -> Vec<String> {
        (0..home.count()).map(|i| home.icon_and_name(i).1).collect()
    }

    #[test]
    fn home_follows_its_arrangement_and_the_focus_stays_on_its_app() {
        let mut home = Home::new();
        let all = names(&home);
        assert_eq!(all[0], "Robot", "as they come: the built-in apps first");
        home.focus = all.iter().position(|n| n == "Logs").unwrap();
        let arrangement = |order: &[&str], hidden: &[&str], favourites: &[&str]| crate::shell::Arrangement {
            order: order.iter().map(|s| s.to_string()).collect(),
            hidden: hidden.iter().map(|s| s.to_string()).collect(),
            favourites: favourites.iter().map(|s| s.to_string()).collect(),
        };
        home.arrange(&arrangement(&["Schedule", "Logs"], &["Tables", "Settings"], &[]));
        let arranged = names(&home);
        assert_eq!(arranged[..2], ["Schedule", "Logs"]);
        assert!(!arranged.iter().any(|n| n == "Tables"), "hidden");
        assert!(arranged.iter().any(|n| n == "Settings"), "Settings is never hidden: it is the way back");
        assert_eq!(arranged.len(), all.len() - 1);
        assert_eq!(arranged[home.focus], "Logs", "the focus stays on the same app");
        // Its own app hidden: the focus stays where it was, on whatever is there now.
        home.arrange(&arrangement(&[], &["Logs"], &[]));
        assert!(home.focus < home.count());
        // Favourites go first, in the order chosen, and wear a star.
        home.arrange(&arrangement(&["Schedule", "Git", "Robot"], &[], &["Git", "Health"]));
        let pinned = names(&home);
        assert_eq!(pinned[..3], ["Git", "Health", "Schedule"]);
        assert!(home.favourite(0) && home.favourite(1) && !home.favourite(2));
        // Back as it came.
        home.arrange(&arrangement(&[], &[], &[]));
        assert_eq!(names(&home), all);
    }

    #[test]
    fn every_app_name_fits_its_tile_three_across() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        // Three across at the standard size: the name gets the tile less its margins.
        let room = (W - 2.0 * PAD - 2.0 * GAP) / 3.0 - 16.0;
        for (_, _, name) in builtin_apps() {
            let w = g.measure(name, g.ty.name);
            assert!(w <= room, "{name} is {w:.0} px, the tile has {room:.0}");
        }
        // And the lines under the tools' names, which never change.
        for line in ["cpu · memory", "find · ping", "ssh · serial", "one key", "pitch · roll", "heading"] {
            assert!(g.measure(line, g.ty.caption) <= room, "{line}");
        }
    }

    #[test]
    fn every_icon_home_an_app_or_a_settings_page_asks_for_is_built_in() {
        crate::shell::tests::sandbox();
        let robot = Robot::demo();
        // Each app's tile icon by name, whatever Home draws.
        for app in &APPS {
            assert!(bezel::Icons::has(app.icon), "{}'s icon {:?} is not built in", app.name, app.icon);
        }
        let mut g = Gfx::new(bezel::Tokens::dark());
        let look = |screen: &mut dyn Screen, g: &mut Gfx| {
            for _ in 0..6 {
                screen.tick(1.0 / 60.0);
                screen.draw(g, &robot);
            }
        };
        // Home, every style, every page of it.
        let mut home = Home::new();
        for style in crate::shell::HomeStyle::ALL {
            home.style = style;
            for focus in 0..home.count() {
                home.focus = focus;
                look(&mut home, &mut g);
            }
        }
        // Every built-in app as it opens (not those that start a shell, a
        // browser or the camera; their icons are on Home's tiles above).
        for app in APPS.iter().filter(|a| !matches!(a.name, "Terminal" | "Web" | "AprilTag")) {
            look((app.open)(&robot).as_mut(), &mut g);
        }
        // Settings and each of its pages.
        let mut settings = SettingsApp::new();
        look(&mut settings, &mut g);
        for item in settings.items.clone() {
            if let Some(mut page) = SettingsApp::section(item) {
                look(&mut page, &mut g);
            }
        }
        assert!(g.missing_icons().is_empty(), "asked for but not built in: {:?}", g.missing_icons());
        // And a name that is not one is caught.
        g.icon("no_such_symbol", false, 10.0, 10.0, 12.0, g.tokens.ink);
        assert!(g.missing_icons().contains("no_such_symbol"));
    }

    #[test]
    fn esc_on_home_glides_back_to_the_first_app_of_the_first_page() {
        crate::shell::tests::sandbox();
        let (mut robot, mut settings, mut say) = (Robot::demo(), crate::shell::Settings::default(), None);
        for style in crate::shell::HomeStyle::ALL {
            settings.home = style;
            settings.calm = false;
            let mut home = Home::new();
            home.style = style;
            let far = home.per_page() + 1;
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            let along = |home: &Home| match style {
                crate::shell::HomeStyle::Grid => home.page.value(),
                crate::shell::HomeStyle::Carousel => home.turn.value(),
                crate::shell::HomeStyle::Flow => home.flow.pos.value(),
            };
            let settle = |home: &mut Home| {
                for _ in 0..240 {
                    home.tick(1.0 / 60.0);
                }
            };
            // Along to an app on the second page (a card well along).
            for _ in 0..far {
                home.key(Key::Right, &mut cx);
            }
            settle(&mut home);
            assert_eq!(home.focus, far);
            let start = along(&home);
            assert!(start >= 1.0 - 1e-3, "{style:?}: turned along, {start}");
            // Esc: the first app, and the page glides back to it.
            assert!(matches!(home.key(Key::Back, &mut cx), Nav::Stay), "{style:?}: Esc stays on Home");
            assert_eq!(home.focus, 0, "{style:?}");
            home.tick(1.0 / 60.0);
            let mid = along(&home);
            assert!(mid > 0.0 && mid < start, "{style:?}: on its way, not there at once: {mid}");
            settle(&mut home);
            assert!(along(&home).abs() < 1e-3, "{style:?}: at the start: {}", along(&home));
            // At the start, Esc changes nothing.
            home.key(Key::Back, &mut cx);
            assert_eq!(home.focus, 0);
            assert!(home.page.is_settled() && home.turn.is_settled() && !home.on_glance());
            // From the glance page: out of it, onto the first app.
            home.key(Key::Left, &mut cx);
            assert!(home.on_glance(), "{style:?}: Left from the first app is the glance page");
            settle(&mut home);
            home.key(Key::Back, &mut cx);
            assert!(!home.on_glance() && home.focus == 0, "{style:?}");
            // The calm version goes there at once.
            cx.settings.calm = true;
            for _ in 0..far {
                home.key(Key::Right, &mut cx);
            }
            home.key(Key::Back, &mut cx);
            assert!(home.focus == 0 && along(&home).abs() < 1e-6, "{style:?}: calm, at once");
            cx.settings.calm = false;
        }
    }

    #[test]
    fn left_and_right_go_through_home_apps_in_order() {
        let mut home = Home::new();
        let (columns, per_page, count) = (home.columns, home.per_page(), home.count());
        assert!(count > per_page, "more than one page of apps");
        // Right at the end of a row goes on to the next row, not the next page.
        home.focus = columns - 1;
        assert!(!home.grid_key(Key::Right, false));
        assert_eq!(home.focus, columns);
        // Left at the start of a row goes back to the end of the row before.
        home.grid_key(Key::Left, false);
        assert_eq!(home.focus, columns - 1);
        // From a page's last tile on to the next page's first, and back.
        home.focus = per_page - 1;
        home.grid_key(Key::Right, false);
        assert_eq!(home.focus, per_page);
        home.grid_key(Key::Left, false);
        assert_eq!(home.focus, per_page - 1);
        // Past the last app, Right pushes against the edge.
        home.focus = count - 1;
        assert!(home.grid_key(Key::Right, false));
        assert_eq!(home.focus, count - 1);
        // Up and Down stay within the page, column by column.
        home.focus = 1;
        home.grid_key(Key::Down, false);
        assert_eq!(home.focus, 1 + columns);
        home.grid_key(Key::Up, false);
        assert_eq!(home.focus, 1);
    }

    #[test]
    fn apps_for_hardware_that_is_not_there_are_not_offered() {
        use crate::imu::{pretend, Found};
        // Home's built-in tiles by name (whatever external apps are about).
        let builtin = |home: &Home| -> Vec<String> {
            (0..home.count()).map(|i| home.icon_and_name(i).1).filter(|n| APPS.iter().any(|a| a.name == n)).collect()
        };
        // The Lite: no motion sensor. Neither app anywhere; the rest as ever.
        pretend(Some(Found::Nothing));
        let shown = builtin(&Home::new());
        assert!(!shown.iter().any(|n| n == "Level" || n == "Compass"), "{shown:?}");
        assert_eq!(shown.len(), 44, "the forty-four the emulator's run counts on (all but Level and Compass are on every unit)");
        assert_eq!(shown.last().map(String::as_str), Some("Settings"), "Settings stays last");
        assert_eq!(builtin_apps().position(|(_, _, n)| n == "Terminal"), Some(8), "Terminal where it was");
        assert!(!builtin_apps().any(|(_, _, n)| n == "Level"), "nor in search");
        assert!(!app_names().iter().any(|n| n == "Compass"), "nor in Arrange apps");
        let display = SettingsApp::section(Item::Display).unwrap();
        assert!(!display.items.contains(&Item::LiftWake) && !display.items.contains(&Item::FaceDown));
        assert!(!setting_names().any(|(_, n)| n == "Lift to wake"));
        // The C154-V1: the Level, not the Compass.
        pretend(Some(Found::Motion));
        let shown = builtin(&Home::new());
        assert!(shown.iter().any(|n| n == "Level") && !shown.iter().any(|n| n == "Compass"), "{shown:?}");
        // The C154: both, before Settings, and the standby switches.
        pretend(Some(Found::Compass));
        let shown = builtin(&Home::new());
        let at = |name: &str| shown.iter().position(|n| n == name).unwrap();
        assert!(at("Pit") < at("Level") && at("Level") < at("Compass") && at("Compass") < at("Settings"));
        let display = SettingsApp::section(Item::Display).unwrap();
        assert!(display.items.ends_with(&[Item::LiftWake, Item::FaceDown]));
        // Found once Home is already up (the sensor starts behind the logo,
        // and the first start has nothing remembered): it is laid out again,
        // the focus on the app it was on.
        pretend(Some(Found::Nothing));
        let mut home = Home::new();
        home.focus = builtin(&home).iter().position(|n| n == "Pit").unwrap();
        pretend(Some(Found::Compass));
        home.follow_arrangement(&crate::shell::Arrangement::default());
        assert!(builtin(&home).iter().any(|n| n == "Compass"));
        assert_eq!(home.icon_and_name(home.focus).1, "Pit");
        pretend(None);
    }

    #[test]
    fn arrange_apps_lists_them_in_the_order_chosen_then_the_rest() {
        let apps: Vec<String> = ["A", "B", "C", "D"].map(String::from).to_vec();
        let arrangement = crate::shell::Arrangement { order: vec!["C".into(), "A".into(), "gone".into()], hidden: vec![], favourites: vec!["D".into()] };
        let items = arrange_items(&apps, &arrangement);
        assert!(items == vec![Item::AppTile(3), Item::AppTile(2), Item::AppTile(0), Item::AppTile(1)]);
    }

    #[test]
    fn every_app_has_a_card_in_the_flow_one_of_your_own_and_the_catalogues_too() {
        crate::shell::tests::sandbox();
        let robot = Robot::demo();
        let mut g = Gfx::new(bezel::Tokens::dark());
        g.home = HomeStyle::Flow;
        let mut home = Home::new();
        // An app of your own (or from the catalogue or the store) beside the built-in ones.
        home.external.push(crate::apps::App {
            name: "Sudoku".into(),
            icon: "grid_on".into(),
            about: "one answer, three levels".into(),
            run: "./catalyst-sudoku".into(),
            dir: std::env::temp_dir(),
            kind: crate::apps::Kind::Own,
            id: "sudoku".into(),
            version: "1.0.0".into(),
            permissions: Vec::new(),
        });
        home.arrange(&crate::shell::Arrangement::default());
        home.draw(&mut g, &robot);
        let atlas = home.flow.cards.as_ref().expect("the cards are made");
        let count = home.count();
        assert_eq!(home.icon_and_name(count - 1).1, "Sudoku");
        // Each has its picture: opaque in its middle, clear at its corner.
        for i in 0..count {
            let (x, y, _) = atlas.level(i, 0);
            let at = |dx: usize, dy: usize| atlas.texel(x + 1 + dx, y + 1 + dy);
            assert_eq!(at(48, 48) >> 24, 255, "{}'s card", home.icon_and_name(i).1);
            assert_eq!(at(0, 0) >> 24, 0, "{}'s rounded corner", home.icon_and_name(i).1);
        }
        // Turned over, the app of your own says what it does in its own words.
        home.focus = count - 1;
        home.flow.place(count - 1);
        home.flow.enlarge(count - 1, true);
        home.draw(&mut g, &robot);
        assert!(home.flow.face.is_some());
        assert!(g.missing_icons().is_empty(), "{:?}", g.missing_icons());
    }

    #[test]
    fn settings_offers_flow_beside_grid_and_carousel() {
        crate::shell::tests::sandbox();
        assert_eq!(HomeStyle::from_name("flow"), Some(HomeStyle::Flow));
        assert_eq!(HomeStyle::Grid.step(1), HomeStyle::Carousel);
        assert_eq!(HomeStyle::Carousel.step(1), HomeStyle::Flow);
        assert_eq!(HomeStyle::Flow.step(1), HomeStyle::Grid);
        assert_eq!(HomeStyle::Grid.step(-1), HomeStyle::Flow, "Left goes the other way round");
        let mut page = SettingsApp::section(Item::HomeSection).unwrap();
        assert!(page.items[0] == Item::HomeScreen);
        let (mut robot, mut settings, mut say) = (Robot::demo(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Right, &mut cx);
        page.key(Key::Right, &mut cx);
        assert_eq!(cx.settings.home, HomeStyle::Flow);
        page.key(Key::Left, &mut cx);
        assert_eq!(cx.settings.home, HomeStyle::Carousel);
        // Each style's name fits its row beside the three pictures, at every
        // size, in both looks (the pictures take 3 × 27 px and a gap).
        let mut g = Gfx::new(bezel::Tokens::dark());
        let width = W - 2.0 * PAD - 5.0;
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = theme;
            for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
                g.set_text_size(size);
                let ty = g.ty;
                let (name_w, value_style) = match theme {
                    crate::look::Theme::Bezel => (g.measure("home screen", ty.label), ty.body),
                    _ => (g.measure("Home screen", ty.body), ty.label),
                };
                for style in HomeStyle::ALL {
                    let room = name_w + 30.0 + g.measure(style.name(), value_style) + 10.0 + 3.0 * 27.0;
                    assert!(room <= width, "{style:?} at {size:?} in {theme:?}: {room} of {width}");
                }
            }
        }
    }
}
