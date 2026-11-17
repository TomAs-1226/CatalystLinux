//! The built-in Catalyst apps. Each is a screen that reads the robot through
//! [`Robot`] and draws with [`Gfx`]; external apps are separate programs.
//!
//! Layouts are measured from the type scale (`g.ty`, `g.row_h()`, `g.top()`),
//! so every screen follows the text size chosen in System.

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::agent::Agent;
use crate::gfx::{Gfx, ListState, TextSize, GAP, H, PAD, TILE_RADIUS, W};
use crate::logs::Digest;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::wpilog::Level;

fn dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "–".into())
}

/// Colour has jobs. On this device: signal marks a fault, amber a warning,
/// ice what is on or well.
fn severity_color(g: &Gfx, severity: &str) -> Color {
    match severity {
        "ERROR" => g.tokens.signal,
        "WARN" => g.tokens.amber_ink,
        _ => g.tokens.dim,
    }
}

/// Up, Down and the page keys over a list of `len` rows.
fn list_key(list: &mut ListState, key: Key, len: usize) {
    match key {
        Key::Up => list.up(),
        Key::Down => list.down(len),
        Key::PageUp => list.page(len, -1),
        Key::PageDown => list.page(len, 1),
        _ => {}
    }
}

// ------------------------------------------------------------------ home ---

struct App {
    icon: &'static str,
    name: &'static str,
    open: fn(&Robot) -> Box<dyn Screen>,
}

const APPS: [App; 8] = [
    App { icon: "smart_toy", name: "Robot", open: |_| Box::new(RobotScreen::default()) },
    App { icon: "monitor_heart", name: "Health", open: |_| Box::new(Health::default()) },
    App { icon: "description", name: "Logs", open: |_| Box::new(LogsScreen::default()) },
    App { icon: "account_tree", name: "Tables", open: |_| Box::new(Tables::new()) },
    App { icon: "speed", name: "Loop", open: |_| Box::new(LoopTime) },
    App { icon: "precision_manufacturing", name: "Motors", open: |_| Box::new(Motors::default()) },
    App { icon: "terminal", name: "Console", open: |robot| Box::new(Console::open(robot)) },
    App { icon: "settings", name: "Settings", open: |_| Box::new(SettingsApp::new()) },
];

pub struct Home {
    focus: usize,
    /// Which page is in view; turns on the page spring.
    page: Animated,
    /// Columns in the grid: fewer, larger tiles at the large text size.
    columns: usize,
    /// Where each tile was last drawn: an app opens out of its tile.
    rects: Vec<Rect>,
    /// Seconds since Home arrived, for the launcher cascade.
    arrived: f32,
}

/// Bezel's launcher cascade: tiles spring into place one after another,
/// spread over this long from the first to the last.
const CASCADE_SPREAD: f32 = 0.28;
const CASCADE_OVER: f32 = CASCADE_SPREAD + 0.9;

impl Home {
    pub fn new() -> Self {
        Home { focus: 0, page: Animated::new(0.0, role::page()).with_rest(0.002), columns: 3, rects: Vec::new(), arrived: 0.0 }
    }

    fn per_page(&self) -> usize {
        self.columns * 2
    }

    /// The one line under each app's name, and whether it needs attention.
    fn status(index: usize, robot: &Robot) -> (String, Option<&'static str>) {
        let linked = robot.linked();
        match APPS[index].name {
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
            "Console" => ("robot log".into(), None),
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
}

impl Screen for Home {
    fn title(&self) -> &str {
        "Catalyst"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let (columns, per_page) = (self.columns, self.per_page());
        let page = self.focus / per_page;
        let local = self.focus % per_page;
        let (col, row) = (local % columns, local / columns);
        let page_end = ((page + 1) * per_page).min(APPS.len());
        match key {
            Key::Left if col > 0 => self.focus -= 1,
            // Past the edge of the grid is the next page.
            Key::Left if page > 0 => self.focus = (page - 1) * per_page + row * columns + columns - 1,
            Key::Right if col + 1 < columns && self.focus + 1 < page_end => self.focus += 1,
            // The same row on the next page, or its first tile if that row is not there.
            Key::Right if page_end < APPS.len() => self.focus = if page_end + row * columns < APPS.len() { page_end + row * columns } else { page_end },
            Key::Up if row > 0 => self.focus -= columns,
            Key::Down if self.focus + columns < page_end => self.focus += columns,
            Key::Tab => self.focus = (self.focus + 1) % APPS.len(),
            Key::Enter => {
                let screen = (APPS[self.focus].open)(cx.robot);
                return match self.rects.get(self.focus) {
                    Some(tile) => Nav::Open(screen, *tile),
                    None => Nav::Push(screen),
                };
            }
            _ => {}
        }
        let target = (self.focus / per_page) as f32;
        if cx.settings.calm {
            self.page.jump(target);
        } else {
            self.page.set(target);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.page.tick(dt);
        if self.arrived < CASCADE_OVER {
            self.arrived += dt;
            moving = true;
        }
        moving
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let columns = if g.text_size == TextSize::Large { 2 } else { 3 };
        if columns != self.columns {
            self.columns = columns;
            self.page.jump((self.focus / self.per_page()) as f32);
        }
        let per_page = self.per_page();
        let pages = APPS.len().div_ceil(per_page);
        // With more than one page, the grid ends above where the pager rests.
        let bottom = if pages > 1 { H - 16.0 } else { H - PAD };
        let top = g.top();
        let tw = (W - 2.0 * PAD - GAP * (columns - 1) as f32) / columns as f32;
        let th = (bottom - top - GAP) / 2.0;
        if pages > 1 {
            g.pager = Some((pages, self.focus / per_page, g.ox));
        }
        for (i, app) in APPS.iter().enumerate() {
            let local = i % per_page;
            let slide = ((i / per_page) as f32 - self.page.value()) * W;
            if slide.abs() >= W {
                continue;
            }
            let r = Rect::new(PAD + (local % columns) as f32 * (tw + GAP) + slide, top + (local / columns) as f32 * (th + GAP), tw, th);
            if self.rects.len() != APPS.len() {
                self.rects.resize(APPS.len(), Rect::default());
            }
            self.rects[i] = r;
            // The cascade: each tile arrives a little after the one before it,
            // nearest the top left first, and springs up into its place.
            let r = if g.calm || self.arrived >= CASCADE_OVER {
                r
            } else {
                let order = (local % columns) as f32 + (local / columns) as f32 * 1.5;
                let since = self.arrived - CASCADE_SPREAD * order / (columns as f32 + 0.5);
                if since < 0.0 {
                    continue;
                }
                r.offset(0.0, 14.0 * role::release().solve(1.0, 0.0, since).0)
            };
            let focused = i == self.focus;
            let (status, attention) = Self::status(i, robot);
            g.surface(r, TILE_RADIUS, t.surface1);
            // A selected item's icon fills in.
            let icon_color = if focused { t.ink } else { t.dim };
            let (text_x, name_cy, status_cy, text_w);
            if tw > 120.0 {
                // A wide tile: the icon beside two lines.
                g.icon(app.icon, focused, r.x + 23.0, r.cy(), 24.0, icon_color);
                text_x = r.x + 44.0;
                text_w = r.w - 52.0;
                name_cy = r.cy() - ty.caption.size * 0.5 - 1.0;
                status_cy = r.cy() + ty.name.size * 0.5 + 1.0;
            } else {
                g.icon(app.icon, focused, r.x + 18.0, r.y + 15.0, 18.0, icon_color);
                text_x = r.x + 10.0;
                text_w = r.w - 16.0;
                status_cy = r.bottom() - 6.0 - ty.caption.size * 0.5;
                name_cy = status_cy - ty.caption.size * 0.5 - 2.0 - ty.name.size * 0.5;
            }
            if let Some(severity) = attention {
                let c = severity_color(g, severity);
                g.dot(r.right() - 12.0, r.y + 12.0, 3.5, c);
            }
            g.text_fit(text_x, name_cy, app.name, ty.name, t.ink, text_w);
            g.text_fit(text_x, status_cy, &status, ty.caption, t.dim, text_w);
            if focused {
                g.focus_ring(r, TILE_RADIUS);
            }
        }
    }
}

// ----------------------------------------------------------------- robot ---

/// Link, identity and the Systemcore's vital signs; the robot's address is
/// set here.
#[derive(Default)]
pub struct RobotScreen {
    editing: Option<String>,
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
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let bottom = H - PAD - g.hint_h();
        let left_w = 122.0;

        // Address: the one thing set on this screen.
        let field_h = ty.caption.size + ty.label.size + 14.0;
        let field = Rect::new(PAD, bottom - field_h, left_w, field_h);
        let battery = Rect::new(PAD, top, left_w, field.y - GAP - top);
        g.stat(battery, "battery", robot.f64("Systemcore/BatteryVolts").map(|v| format!("{v:.1}")), "v", (battery.h - ty.caption.size - 14.0).min(36.0));
        if robot.bool("Systemcore/BrownedOut") == Some(true) {
            g.text_at(battery.right() - 9.0, battery.y + 5.0 + ty.caption.size * 0.5, Align::Right, "brownout", ty.caption, t.signal);
        }
        g.surface(field, TILE_RADIUS, if self.editing.is_some() { t.surface2 } else { t.surface1 });
        g.text_at(field.x + 9.0, field.y + 5.0 + ty.caption.size * 0.5, Align::Left, "address", ty.caption, t.dim);
        let value_cy = field.bottom() - 5.0 - ty.label.size * 0.5;
        match &self.editing {
            Some(text) => {
                let w = g.text_at(field.x + 9.0, value_cy, Align::Left, text, ty.label, t.ink);
                g.surface(Rect::new(field.x + 10.0 + w, value_cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
                g.focus_ring(field, TILE_RADIUS);
            }
            None => {
                let shown = if robot.address.is_empty() { "not set".to_owned() } else { robot.address.clone() };
                g.text_fit(field.x + 9.0, value_cy, &shown, ty.label, t.ink, field.w - 18.0);
            }
        }

        let x = PAD + left_w + GAP;
        let w = W - PAD - x;
        let can = robot.get("Systemcore/CanUtilization").and_then(|v| v.as_f64s().map(|u| u.iter().cloned().fold(0.0, f64::max)));
        let cpu = robot.f64("Systemcore/CpuPercent");
        let temp = robot.f64("Systemcore/TempCelsius");
        let rows: [(&str, String, Option<f32>); 5] = [
            ("link", if robot.linked() { dash(robot.rtt_ms().map(|ms| format!("{ms:.1} ms"))) } else { robot.status() }, None),
            ("team", dash(robot.i64("Robot/Identity/TeamNumber").map(|n| n.to_string())), None),
            ("cpu", dash(cpu.map(|v| format!("{v:.0}%"))), cpu.map(|v| v as f32 / 100.0)),
            ("temp", dash(temp.map(|v| format!("{v:.0} °c"))), temp.map(|v| v as f32 / 90.0)),
            ("can", dash(can.map(|v| format!("{:.0}%", v * 100.0))), can.map(|v| v as f32)),
        ];
        let gap = 3.0;
        let row_h = (bottom - top - gap * 4.0) / 5.0;
        let label_w = g.measure("temp", ty.label) + 18.0;
        let value_w = g.measure("100 °c", ty.label) + 16.0;
        for (i, (label, value, fraction)) in rows.iter().enumerate() {
            let r = Rect::new(x, top + i as f32 * (row_h + gap), w, row_h);
            g.surface(r, (row_h * 0.5).min(11.0), t.surface1);
            g.text_at(r.x + 10.0, r.cy(), Align::Left, label, ty.label, t.dim);
            match fraction {
                Some(f) => {
                    g.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, t.ink);
                    let fill = if *f > 0.85 { t.signal } else { t.ice };
                    g.meter(Rect::new(r.x + label_w, r.cy() - 3.0, r.w - label_w - value_w, 6.0), *f, fill);
                }
                None => {
                    g.text_fit(r.x + label_w, r.cy(), value, ty.label, t.ink, r.w - label_w - 8.0);
                }
            }
        }
        g.hint(if self.editing.is_some() { "team number, ip or host · enter" } else { "enter: set the robot address" });
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
            (Some(s), Some(false)) => (s, t.signal),
            (Some(s), _) => (s, t.ink),
            (None, _) => ("not run".to_owned(), t.faint),
        };
        let count = |n: Option<i64>, color: Color| match n {
            Some(n) if n > 0 => (n.to_string(), color),
            Some(_) => ("0".to_owned(), t.ink),
            None => ("–".to_owned(), t.faint),
        };
        let chips = [
            ("err", count(robot.i64("Health/ErrorCount"), t.signal)),
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
        let list_bottom = H - PAD - g.hint_h();
        let mut detail = String::new();
        for (i, y) in self.list.layout(len, list_top, row_h, 3.0, list_bottom + 2.0) {
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
        Level::Fault => g.tokens.signal,
        Level::Warn => g.tokens.amber_ink,
        Level::Fine => g.tokens.dim,
    }
}

/// The match logs pulled from the robot, newest first. Each row carries the
/// worst thing its match had; Enter opens the whole summary.
#[derive(Default)]
pub struct LogsScreen {
    list: ListState,
    seen: u64,
}

impl Screen for LogsScreen {
    fn title(&self) -> &str {
        "Logs"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = cx.robot.logs.list();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                if let Some(row) = rows.get(self.list.focus) {
                    match &row.digest {
                        Some(digest) => return Nav::Push(Box::new(LogDetail { title: row.title(), digest: digest.clone(), list: ListState::default() })),
                        None => *cx.say = Some("not read yet".into()),
                    }
                }
            }
            other => list_key(&mut self.list, other, rows.len()),
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
        let top = g.top();
        let bottom = H - PAD - g.hint_h();
        let row_h = ty.body.size + ty.caption.size + 12.0;
        for (i, y) in self.list.layout(rows.len(), top, row_h, 4.0, bottom + 2.0) {
            let row = &rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, 12.0, if focused { t.surface2 } else { t.surface1 });
            let (line1, line2) = (r.y + 5.0 + ty.body.size * 0.5, r.bottom() - 5.0 - ty.caption.size * 0.5);
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
        self.list.scrollbar(g, rows.len(), top, bottom);
        let fitted = g.text.fit(&status, &ty.caption, W - 2.0 * PAD);
        g.hint(&fitted);
    }

    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
}

/// One match: how long, how low the battery went, the worst loop, and
/// everything that stood out, worst first.
pub struct LogDetail {
    title: String,
    digest: Digest,
    list: ListState,
}

impl Screen for LogDetail {
    fn title(&self) -> &str {
        "Match"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        list_key(&mut self.list, key, self.digest.highlights.len());
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
        let bottom = H - PAD - g.hint_h();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(d.highlights.len(), list_top, row_h, 3.0, bottom + 2.0) {
            let (level, text) = &d.highlights[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let color = level_color(g, *level);
            g.row(r, &format!("    {text}"), "", t.dim, i == self.list.focus);
            g.dot(r.x + 12.0, r.cy(), 3.0, color);
        }
        self.list.scrollbar(g, d.highlights.len(), list_top, bottom);
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
}

impl Screen for Tables {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let nodes = self.nodes(cx.robot);
        match key {
            Key::Enter | Key::Right => {
                if let Some(Node::Table(name, _)) = nodes.get(self.list.focus) {
                    self.trail.push(self.list.focus);
                    self.enter(format!("{}{name}/", self.path), 0);
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
        for (i, y) in self.list.layout(nodes.len(), top, row_h, 3.0, H - PAD + 2.0) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match &nodes[i] {
                Node::Table(name, count) => g.row(r, &format!("{name}/"), &format!("{count} ›"), t.dim, i == self.list.focus),
                Node::Value(name, value) => {
                    // The value gets up to half the row; the name takes the rest.
                    let shown = g.text.fit(value, &ty.label, r.w * 0.5);
                    g.row(r, name, &shown, t.ink, i == self.list.focus)
                }
            }
        }
        self.list.scrollbar(g, nodes.len(), top, H - PAD);
    }
}

// ------------------------------------------------------------------ loop ---

/// Robot loop timing against its 20 ms budget.
pub struct LoopTime;

impl Screen for LoopTime {
    fn title(&self) -> &str {
        "Loop"
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
        g.surface(plot, TILE_RADIUS, t.surface1);
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
        for (i, y) in self.list.layout(rows.len(), top, row_h, 4.0, H - PAD + 2.0) {
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
        self.list.scrollbar(g, rows.len(), top, H - PAD);
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
        g.surface(panel, TILE_RADIUS, t.surface1);
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
                t.signal
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
    Appearance,
    Display,
    Network,
    Access,
    About,
    // Appearance.
    Mode,
    AccentColor,
    TextScale,
    Calm,
    // Display.
    Brightness,
    // Access.
    Ssh,
    DemoRobot,
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
}

impl SettingsApp {
    pub fn new() -> Self {
        use Item::*;
        Self::page("Settings", vec![Appearance, Display, Network, Access, About])
    }

    fn page(title: &'static str, items: Vec<Item>) -> Self {
        let mut page = SettingsApp { title, items, list: ListState::default(), info: Vec::new(), read_at: std::time::Instant::now(), network: String::new(), backlight: None };
        page.read_info();
        page
    }

    fn section(item: Item) -> Option<Self> {
        use Item::*;
        Some(match item {
            Appearance => Self::page("Appearance", vec![Mode, AccentColor, TextScale, Calm]),
            Display => Self::page("Display", vec![Brightness]),
            Network => Self::page("Network", (0..4).map(Info).collect()),
            Access => Self::page("Access", vec![Ssh, DemoRobot]),
            About => Self::page("About", (0..9).map(Info).collect()),
            _ => return None,
        })
    }

    fn read_info(&mut self) {
        use crate::sys;
        self.read_at = std::time::Instant::now();
        self.network = sys::address("eth0").filter(|a| a.contains('.')).or_else(|| sys::address("wlan0")).unwrap_or_else(|| "–".into());
        self.backlight = sys::brightness();
        self.info = match self.title {
            "Network" => vec![
                ("Name", dash(sys::hostname())),
                ("Ethernet", dash(sys::address("eth0"))),
                ("Wi-Fi", dash(sys::address("wlan0"))),
                ("USB link", dash(sys::address("usb0"))),
            ],
            "About" => vec![
                ("Catalyst Linux", env!("CARGO_PKG_VERSION").to_owned()),
                ("Device", dash(sys::model())),
                ("Kernel", dash(sys::kernel())),
                ("Up for", dash(sys::uptime())),
                ("Memory in use", dash(sys::memory())),
                ("This shell", dash(sys::shell_memory())),
                ("Storage", dash(sys::storage())),
                ("Battery", dash(sys::battery())),
                ("Processor", dash(sys::cpu_temperature())),
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
            Appearance => section("Appearance", format!("{} · {}", if g.tokens.tone == bezel::Tone::Light { "light" } else { "dark" }, g.accent.name())),
            Display => section("Display", dash(self.backlight.map(|b| format!("{b}%")))),
            Network => section("Network", self.network.clone()),
            Access => section("Access", format!("ssh {}", onoff(crate::ssh_enabled()))),
            About => section("About", env!("CARGO_PKG_VERSION").to_owned()),
            Mode => ("Mode".into(), (if g.tokens.tone == bezel::Tone::Light { "light" } else { "dark" }).into(), true),
            AccentColor => ("Accent".into(), g.accent.name().into(), true),
            TextScale => ("Text size".into(), g.text_size.name().into(), true),
            Calm => ("Calm version".into(), onoff(g.calm), g.calm),
            Brightness => ("Brightness".into(), dash(self.backlight.map(|b| format!("{b}%"))), true),
            Ssh => ("SSH access".into(), onoff(crate::ssh_enabled()), crate::ssh_enabled()),
            DemoRobot => ("Demo robot".into(), onoff(robot.is_demo()), robot.is_demo()),
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
            TextScale => cx.settings.text = if step > 0 { cx.settings.text.next() } else { cx.settings.text.next().next() },
            Calm => cx.settings.calm = !cx.settings.calm,
            Brightness => match crate::sys::brightness() {
                Some(now) => {
                    crate::sys::set_brightness((now as i32 + step * 10).clamp(10, 100) as u32);
                    self.backlight = crate::sys::brightness();
                }
                None => *cx.say = Some("the backlight is on the device".into()),
            },
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
            _ => {}
        }
        crate::save_settings(cx.settings);
    }
}

impl Screen for SettingsApp {
    fn title(&self) -> &str {
        self.title
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let Some(&item) = self.items.get(self.list.focus) else { return Nav::Pop };
        match key {
            Key::Back => return Nav::Pop,
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
        let items = self.items.clone();
        for (i, y) in self.list.layout(items.len(), top, row_h, 3.0, H - PAD + 2.0) {
            let item = items[i];
            let (name, value, on) = self.describe(item, g, robot);
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match item {
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
        self.list.scrollbar(g, items.len(), top, H - PAD);
    }
}
