//! The glance page's cards (负一屏): each one value large, a line under it
//! saying what or whose, then a few rows of detail, to be read without
//! opening anything. Which cards, and in what order, is chosen in Settings ›
//! Home › Glance page; Enter on a card opens what it is about.

use bezel::{Align, Color, Rect};

use crate::gfx::Gfx;
use crate::robot::Robot;
use crate::screens::severity_color;
use crate::shell::{Cx, Screen, Widget};
use crate::wpilog::Level;

/// What a card shows.
pub struct Card {
    pub big: String,
    /// A unit after the large value ("v").
    pub unit: Option<&'static str>,
    pub under: String,
    /// A dot before the line under: linked or not, or an alliance's colour.
    pub dot: Option<Color>,
    /// Name, value, and whether the value needs attention ("WARN", "ERROR").
    pub rows: Vec<(String, String, Option<&'static str>)>,
    /// The large value itself needs attention.
    pub warn: bool,
}

pub fn draw(g: &mut Gfx, r: Rect, card: &Card, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let line = ty.caption.size + 4.0;
    let big = ty.display(28.0);
    let baseline = r.y + 8.0 + big.size * 0.78;
    let under = baseline + 5.0 + ty.caption.size * 0.5;
    g.surface(r, g.tile_radius(), t.surface1);
    let fitted = g.text.fit(&card.big, &big, r.w - 24.0);
    let color = if card.big == "–" { t.faint } else if card.warn { g.fault() } else { t.ink };
    let w = g.text(r.x + 10.0, baseline, &fitted, big, color);
    if let Some(unit) = card.unit {
        g.text(r.x + 13.0 + w, baseline, unit, ty.caption, t.dim);
    }
    let text_x = match card.dot {
        Some(dot) => {
            g.dot(r.x + 14.0, under, 2.5, dot);
            r.x + 22.0
        }
        None => r.x + 11.0,
    };
    g.text_fit(text_x, under, &card.under, ty.caption, t.dim, r.right() - 10.0 - text_x);
    let y0 = r.bottom() - 7.0 - card.rows.len() as f32 * line;
    for (i, (name, value, attention)) in card.rows.iter().enumerate() {
        let cy = y0 + (i as f32 + 0.5) * line;
        let name_w = g.text_at(r.x + 10.0, cy, Align::Left, name, ty.caption, t.dim);
        let value = g.text.fit(value, &ty.caption, r.w - name_w - 24.0);
        let color = attention.map_or(t.ink, |severity| severity_color(g, severity));
        g.text_at(r.right() - 10.0, cy, Align::Right, &value, ty.caption, color);
    }
    if focused {
        g.focus_ring(r, g.tile_radius());
    }
}

/// The note the glance page shows: a plain text file on the data partition,
/// written in the editor like any other.
pub fn note_file() -> std::path::PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        "/data/note.txt".into()
    } else {
        crate::config_dir().join("note.txt")
    }
}

fn level_attention(level: Level) -> Option<&'static str> {
    match level {
        Level::Fault => Some("ERROR"),
        Level::Warn => Some("WARN"),
        Level::Fine => None,
    }
}

/// The cards that are not Home's own (the device and robot cards are drawn
/// from what Home already keeps).
pub fn card(widget: Widget, robot: &Robot, pinned: Option<&str>) -> Card {
    let blank = |under: &str, rows: Vec<(String, String, Option<&'static str>)>| Card { big: "–".into(), unit: None, under: under.into(), dot: None, rows, warn: false };
    match widget {
        Widget::NextMatch => {
            let Some(view) = robot.schedule.view() else {
                return blank("no team set", vec![("set it".into(), "Settings › Robot".into(), None)]);
            };
            let now = chrono::Utc::now().timestamp();
            let Some(m) = view.next(now) else {
                let why = view.problem.clone().unwrap_or_else(|| "no match coming".into());
                return blank(&why, vec![("team".into(), view.team.to_string(), None)]);
            };
            let side = m.alliance(view.team).unwrap_or("–");
            let when = m.time.map(|t| crate::schedule::when(t, now)).unwrap_or_else(|| "time not set".into());
            let others = |teams: &[u32]| teams.iter().filter(|t| **t != view.team).map(u32::to_string).collect::<Vec<_>>().join(" ");
            let (ours, theirs) = if side == "red" { (&m.red, &m.blue) } else { (&m.blue, &m.red) };
            let mut rows = vec![("with".into(), others(ours), None), ("against".into(), others(theirs), None)];
            match (&m.status, m.queue_at) {
                (Some(status), _) => rows.insert(0, ("now".into(), status.to_lowercase(), Some("WARN"))),
                (None, Some(q)) => rows.insert(0, ("queue".into(), crate::schedule::when(q, now), None)),
                _ => {}
            }
            if let Some(q) = &view.now_queuing {
                rows.push(("queuing".into(), q.clone(), None));
            }
            Card {
                big: m.label.clone(),
                unit: None,
                under: format!("{side} · {when}"),
                dot: Some(match side {
                    "red" => crate::schedule::RED,
                    "blue" => crate::schedule::BLUE,
                    _ => Color::hex(0x5d616a),
                }),
                rows,
                warn: m.status.as_deref() == Some("Now queuing"),
            }
        }
        Widget::LastMatch => {
            let rows = robot.logs.list();
            let Some((row, digest)) = rows.iter().find_map(|r| r.digest.as_ref().map(|d| (r, d))) else {
                return blank("no match pulled yet", vec![("logs".into(), robot.logs.status(), None)]);
            };
            let mut lines: Vec<(String, String, Option<&'static str>)> = digest.highlights.iter().take(3).map(|(level, text)| ("·".into(), text.clone(), level_attention(*level))).collect();
            if lines.is_empty() {
                lines.push(("·".into(), "nothing stood out".into(), None));
            }
            Card {
                big: digest.battery_low.map_or("–".into(), |v| format!("{v:.1}")),
                unit: digest.battery_low.map(|_| "v low"),
                under: row.title(),
                dot: None,
                rows: lines,
                warn: digest.battery_low.is_some_and(|v| v < 7.0),
            }
        }
        Widget::Pinned => {
            let Some(key) = pinned else {
                return blank("nothing pinned", vec![("pin one".into(), "Tables › p".into(), None)]);
            };
            let (table, name) = key.rsplit_once('/').unwrap_or(("", key));
            match robot.get(key) {
                Some(value) => Card { big: value.display(), unit: None, under: name.into(), dot: None, rows: vec![("in".into(), table.into(), None)], warn: false },
                None => blank(name, vec![("in".into(), table.into(), None), ("now".into(), "not published".into(), None)]),
            }
        }
        Widget::Note => {
            let text = std::fs::read_to_string(note_file()).unwrap_or_default();
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let Some(first) = lines.next() else {
                return blank("no note yet", vec![("enter".into(), "writes one".into(), None)]);
            };
            Card { big: first.into(), unit: None, under: "note".into(), dot: None, rows: lines.take(3).map(|l| ("·".into(), l.to_owned(), None)).collect(), warn: false }
        }
        // Drawn by Home from what it keeps.
        Widget::Device | Widget::Robot => blank("", Vec::new()),
    }
}

/// What Enter on a card opens.
pub fn open(widget: Widget, cx: &mut Cx) -> Option<Box<dyn Screen>> {
    let named = |name: &str, robot: &Robot| crate::screens::builtin_apps().find(|(_, _, n)| *n == name).map(|(i, ..)| crate::screens::open_builtin(i, robot));
    match widget {
        Widget::Device => crate::screens::setting_names().find(|(_, n)| *n == "About this handheld").map(|(i, _)| crate::screens::open_setting(i)),
        Widget::Robot => named("Robot", cx.robot),
        Widget::NextMatch => named("Schedule", cx.robot),
        Widget::LastMatch => {
            let rows = cx.robot.logs.list();
            match rows.iter().find_map(crate::screens::LogDetail::of) {
                Some(detail) => Some(Box::new(detail)),
                None => named("Logs", cx.robot),
            }
        }
        Widget::Pinned => match &cx.settings.pinned {
            Some(key) => Some(Box::new(crate::screens::Tables::at(key, cx.robot))),
            None => named("Tables", cx.robot),
        },
        Widget::Note => {
            let file = note_file();
            if !file.exists() {
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).ok();
                }
                std::fs::write(&file, "").ok();
            }
            match crate::files::Editor::open(&file) {
                Ok(editor) => Some(Box::new(editor)),
                Err(why) => {
                    *cx.say = Some(why);
                    None
                }
            }
        }
    }
}
