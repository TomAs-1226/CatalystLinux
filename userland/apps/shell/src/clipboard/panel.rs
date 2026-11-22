//! The clipboard's history: Ctrl-Shift-H from anywhere brings it up, a
//! small sheet of glass growing out of the island as Spotlight does, over
//! the screen in front (held still behind it).
//!
//! The clips are listed newest first, pinned ones before them, each with
//! its first line and when it was copied; a secret on the clipboard shows
//! first, as dots, with the seconds it has left. Up and Down move; Enter
//! pastes the clip into the field in front (or the terminal), or only
//! copies it where nothing takes text; Tab or → shows all of it, ← goes
//! back; `p` pins or unpins it; `d` removes it (a pinned one asks again);
//! Esc, or Ctrl-Shift-H again, puts it away.

use std::time::Instant;

use bezel::Rect;
use detent::Animated;

use crate::gfx::{Fade, Gfx, RING_ROOM};
use crate::platform::Key;
use crate::sheet::{first_fitting, keys_line, keys_shown, text_right, Grown, FOOT, INSET, TOP};

use super::Item;

const ROW_GAP: f32 = 2.0;
const LIST_ABOVE: f32 = 4.0;
const LIST_BELOW: f32 = 4.0;

/// A row of the panel.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// The secret on the clipboard, and its seconds left.
    Secret(u64),
    Clip(Item),
}

/// What a key asks of the shell.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    /// Put it away.
    Close,
    /// This text into the screen in front (or only copied, where nothing
    /// takes text); the panel goes.
    Paste(String),
    /// Words for the island.
    Say(String),
}

pub struct Panel {
    sheet: Grown,
    rows: Vec<Row>,
    /// The board's generation the rows were read at.
    seen: u64,
    focus: usize,
    /// The first row in sight, and the highlight's row, both gliding.
    scroll: Animated,
    ring: Animated,
    placed: bool,
    /// The whole of the focused clip slid in over the list (0 → 1).
    slide: Animated,
    previewing: bool,
    /// A pinned clip `d` was pressed on once.
    armed: Option<String>,
    /// How many rows are wholly in sight at the text size last drawn.
    rows_most: usize,
    fade: Fade,
    /// What a key just did, said in the bar for a moment, and for how long more.
    note: Option<(String, f32)>,
    /// The foot of the screen behind (`Gfx::foot`): above its hint line, if
    /// it has one, else the panel's own. The sheet comes down no further.
    foot: f32,
}

/// How long the bar says what a key did.
const NOTE_SECONDS: f32 = 2.0;

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the sheet's foot may come down to over a screen whose foot is at
/// `foot`: clear of its hint line, and its own gap above the panel's edge.
fn lowest(foot: f32) -> f32 {
    foot.min(crate::gfx::H - FOOT)
}

/// The panel's measures down to `foot`: its bar's height, a row's, the
/// keys' line, and how many rows it holds (the last perhaps in part).
fn metrics(g: &Gfx, foot: f32) -> (f32, f32, f32, f32) {
    let bar = (g.ty.name.size + 15.0).round();
    let row = g.row_h();
    let keys = (g.ty.caption.size + 10.0).round();
    let room = lowest(foot) - TOP - bar - keys - LIST_ABOVE - LIST_BELOW;
    (bar, row, keys, (room + ROW_GAP) / (row + ROW_GAP))
}

impl Panel {
    pub fn new() -> Self {
        Panel {
            sheet: Grown::new(),
            rows: Vec::new(),
            seen: u64::MAX,
            focus: 0,
            scroll: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.003),
            ring: Animated::new(0.0, crate::gfx::ring_spring()).with_rest(0.003),
            placed: false,
            slide: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.003),
            previewing: false,
            armed: None,
            rows_most: 4,
            fade: Fade::default(),
            note: None,
            foot: crate::gfx::H,
        }
    }

    pub fn is_open(&self) -> bool {
        self.sheet.is_open()
    }

    pub fn visible(&self) -> bool {
        self.sheet.visible()
    }

    pub fn shown(&self) -> f32 {
        self.sheet.shown()
    }

    pub fn dim(&self) -> f32 {
        self.sheet.dim()
    }

    pub fn glass(&self, g: &Gfx, island: Rect) -> bezel::GlassShape {
        self.sheet.glass(g, island)
    }

    /// Its sheet of glass, for quick capture's to take over.
    pub fn grown(&self) -> &Grown {
        &self.sheet
    }

    /// Bring it up, on the newest clip (or, on its way back into the
    /// island, turned round where it was); in place of quick capture's
    /// sheet, `over`, if that is showing.
    pub fn summon(&mut self, g: &Gfx, calm: bool, over: Option<&Grown>) {
        let returning = self.sheet.returning();
        // The screen in front, as it was last drawn.
        self.foot = g.foot;
        self.read();
        if !returning {
            self.focus = 0;
            self.placed = false;
            self.previewing = false;
            self.slide.jump(0.0);
            self.armed = None;
        }
        let height = self.height_for(g, self.rows.len());
        self.sheet.summon(calm, height, over);
    }

    pub fn dismiss(&mut self) {
        self.sheet.dismiss();
    }

    pub fn gone_now(&mut self) {
        self.sheet.gone_now();
        self.slide.jump(0.0);
        self.previewing = false;
    }

    /// The rows as the board has them now; the focus stays on the clip it
    /// was on, where it is still there.
    fn read(&mut self) {
        let now = Instant::now();
        let focused = self.rows.get(self.focus).cloned();
        let (rows, generation) = super::with(|b| {
            b.expire(now);
            let secret = b.secret_left(now).map(|left| Row::Secret(left.as_secs_f32().ceil() as u64));
            (secret.into_iter().chain(b.items().into_iter().map(Row::Clip)).collect::<Vec<_>>(), b.generation)
        });
        self.rows = rows;
        self.seen = generation;
        let same = |a: &Row, b: &Row| match (a, b) {
            (Row::Secret(_), Row::Secret(_)) => true,
            (Row::Clip(a), Row::Clip(b)) => a.text == b.text,
            _ => false,
        };
        if let Some(at) = focused.and_then(|f| self.rows.iter().position(|r| same(r, &f))) {
            self.focus = at;
        }
        self.focus = self.focus.min(self.rows.len().saturating_sub(1));
    }

    /// Its height for `rows` rows: its bar, the rows (or the line saying
    /// there are none), and the keys along its foot. With more than it
    /// holds whole, or one clip in whole, it runs on to the foot of the
    /// screen behind, as search's results do: they scroll, with a thumb,
    /// and the next row fades out above the keys. That foot is above the
    /// screen's hint line, if it has one: the sheet never comes down on it.
    fn height_for(&self, g: &Gfx, rows: usize) -> f32 {
        let (bar, row, keys, most) = metrics(g, self.foot);
        let full = lowest(self.foot) - TOP;
        let shown = rows.max(1) as f32;
        if self.previewing || shown > most.floor().max(1.0) {
            return full;
        }
        (bar + LIST_ABOVE + shown * row + (shown - 1.0) * ROW_GAP + LIST_BELOW + keys).min(full)
    }

    /// A key while it is open. What it says is said in the bar too.
    pub fn key(&mut self, key: Key) -> Outcome {
        let outcome = self.act(key);
        if let Outcome::Say(words) = &outcome {
            self.note = Some((words.clone(), NOTE_SECONDS));
        }
        outcome
    }

    fn act(&mut self, key: Key) -> Outcome {
        // Nothing is typed here: the letters that are arrows outside typing are arrows.
        let key = match key {
            Key::Char('f') => Key::Up,
            Key::Char('x') => Key::Down,
            Key::Char('z') => Key::Left,
            Key::Char('c') => Key::Right,
            Key::Backspace => Key::Back,
            other => other,
        };
        let armed = self.armed.take();
        let len = self.rows.len();
        match key {
            Key::Back | Key::Leave | Key::Ctrl('q') | super::HISTORY_KEY => return Outcome::Close,
            Key::Enter => {
                return match self.rows.get(self.focus) {
                    Some(Row::Clip(item)) => Outcome::Paste(item.text.clone()),
                    Some(Row::Secret(_)) => match super::paste_text() {
                        Some(text) => Outcome::Paste(text),
                        None => Outcome::Say("the secret has cleared".into()),
                    },
                    None => Outcome::Close,
                };
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
                if self.focus == before && !self.sheet.calm {
                    // Nothing further: the highlight gives a little and comes back.
                    self.ring.nudge(if matches!(key, Key::Up | Key::PageUp | Key::Home) { -6.0 } else { 6.0 });
                }
            }
            Key::Tab | Key::Right if !self.previewing && matches!(self.rows.get(self.focus), Some(Row::Clip(_))) => self.slide_to(true),
            Key::Tab | Key::Left if self.previewing => self.slide_to(false),
            Key::Char('p') => match self.rows.get(self.focus) {
                Some(Row::Clip(item)) => {
                    let text = item.text.clone();
                    let now = super::with(|b| b.toggle_pin(&text));
                    self.read();
                    if let Some(at) = self.rows.iter().position(|r| matches!(r, Row::Clip(i) if i.text == text)) {
                        self.focus = at;
                    }
                    return Outcome::Say(match now {
                        Ok(true) => "pinned".into(),
                        Ok(false) => "unpinned".into(),
                        Err(super::Unpinnable::Full) => format!("{} pinned already · unpin one", super::PINS),
                        Err(super::Unpinnable::Gone) => "that clip is gone".into(),
                    });
                }
                Some(Row::Secret(_)) => return Outcome::Say("a secret is never kept".into()),
                None => {}
            },
            Key::Char('d') | Key::Delete => match self.rows.get(self.focus).cloned() {
                Some(Row::Clip(item)) if item.pinned && armed.as_deref() != Some(item.text.as_str()) => {
                    self.armed = Some(item.text);
                    return Outcome::Say("pinned · d again removes it".into());
                }
                Some(Row::Clip(item)) => {
                    super::with(|b| b.remove(&item.text));
                    self.read();
                    if self.rows.is_empty() {
                        self.slide_to(false);
                    }
                    return Outcome::Say("removed".into());
                }
                Some(Row::Secret(_)) => {
                    super::with(|b| b.forget_secret());
                    self.read();
                    return Outcome::Say("the secret is gone".into());
                }
                None => {}
            },
            _ => {}
        }
        Outcome::Stay
    }

    fn slide_to(&mut self, preview: bool) {
        self.previewing = preview;
        let to = if preview { 1.0 } else { 0.0 };
        if self.sheet.calm {
            self.slide.jump(to);
        } else {
            self.slide.set(to);
        }
    }

    /// Advance its motion (`motion_dt`) and its clock (`dt`), and read the
    /// board again when it has changed (a copy from a terminal while it
    /// shows, a secret's seconds).
    pub fn tick(&mut self, dt: f32, motion_dt: f32) -> bool {
        let mut moving = self.sheet.tick(motion_dt) | self.scroll.tick(motion_dt) | self.ring.tick(motion_dt) | self.slide.tick(motion_dt);
        // What a key did goes on the clock, not the motion.
        if let Some((_, left)) = &mut self.note {
            *left -= dt;
            if *left <= 0.0 {
                self.note = None;
                moving = true;
            }
        }
        if self.sheet.is_open() {
            let secret = self.rows.iter().any(|r| matches!(r, Row::Secret(_)));
            let generation = super::generation();
            // A secret counts its seconds down while it shows.
            let ticking = secret && super::with(|b| b.secret_left(Instant::now())).map(|l| l.as_secs_f32().ceil() as u64) != self.rows.first().and_then(|r| if let Row::Secret(s) = r { Some(*s) } else { None });
            if generation != self.seen || ticking {
                super::tick();
                self.read();
                moving = true;
            }
        }
        moving
    }

    /// What sits on the glass: the bar, the clips or one in whole, the keys;
    /// over a screen whose foot (`Gfx::foot`, as it was drawn) is at `foot`.
    pub fn draw(&mut self, g: &mut Gfx, island: Rect, foot: f32) {
        self.foot = foot;
        let (bar_h, row_h, keys_h, room) = metrics(g, foot);
        self.rows_most = room.floor().max(1.0) as usize;
        let height = self.height_for(g, self.rows.len());
        self.sheet.set_height(g, height);
        let panel = self.sheet.glass(g, island).rect;
        let words = self.sheet.words();
        if words <= 0.0 {
            return;
        }
        let t = g.tokens;
        let ty = g.ty;
        g.canvas.set_clip(panel.inset(1.0));
        // The bar: what it is, and how many.
        let bar_cy = panel.y + bar_h * 0.5;
        g.icon_alpha("content_paste", false, panel.x + 16.0, bar_cy, 15.0, t.dim, words);
        g.text_alpha(panel.x + 30.0, bar_cy, "Clipboard", ty.name, t.ink, words);
        // Beside it, how many; or for a moment, what a key just did (the
        // island is under the sheet).
        let clips = self.rows.iter().filter(|r| matches!(r, Row::Clip(_))).count();
        let (said, ink) = match &self.note {
            Some((note, _)) => (note.clone(), t.ink),
            None if clips == 0 => (String::new(), t.dim),
            None if clips == 1 => ("1 clip".to_owned(), t.dim),
            None => (format!("{clips} clips"), t.dim),
        };
        let title_end = panel.x + 30.0 + g.measure("Clipboard", ty.name) + 10.0;
        let said = g.text.fit(&said, &ty.caption, (panel.right() - 14.0 - title_end).max(0.0));
        text_right(g, panel.right() - 14.0, bar_cy, &said, ty.caption, ink, words);
        // Laid out as it rests, from its top, and clipped to the glass: as
        // it grows (or its height glides) more of it shows.
        let rest_h = self.sheet.rest().h;
        let list = Rect::new(panel.x, panel.y + bar_h, panel.w, (rest_h - keys_h - bar_h).max(0.0));
        if list.h > 1.0 {
            g.canvas.rrect(Rect::new(panel.x + 12.0, list.y, panel.w - 24.0, 1.0), 0.5, t.ink, 0.10 * words);
            g.canvas.set_clip(list.inset(1.0));
            let slide = self.slide.value();
            if slide < 0.999 {
                self.draw_rows(g, list, row_h, -slide * list.w, words);
            }
            if slide > 0.001 {
                self.draw_whole(g, list, (1.0 - slide) * list.w, words);
            }
            g.canvas.set_clip(panel.inset(1.0));
        }
        // The keys, along its foot (as it rests), in once the glass reaches them.
        let keys_cy = panel.y + rest_h - keys_h * 0.5 - 1.0;
        let focused_secret = matches!(self.rows.get(self.focus), Some(Row::Secret(_)));
        let keys: &[(&str, &str)] = match (self.rows.is_empty(), self.previewing, focused_secret) {
            (true, _, _) => &[("esc", "close")],
            (_, true, _) => &[("enter", "paste"), ("←", "back"), ("p", "pin")],
            (_, _, true) => &[("enter", "paste"), ("d", "forget")],
            _ => &[("enter", "paste"), ("p", "pin"), ("d", "remove"), ("tab", "all")],
        };
        keys_line(g, panel.x + 12.0, panel.right() - 12.0, keys_cy, keys, words * keys_shown(panel, keys_cy, keys_h));
        g.canvas.clear_clip();
    }

    fn draw_rows(&mut self, g: &mut Gfx, list: Rect, row_h: f32, dx: f32, alpha: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let step = row_h + ROW_GAP;
        let top = list.y + LIST_ABOVE;
        if self.rows.is_empty() {
            let line = first_fitting(g, &["nothing copied yet · ctrl c copies", "nothing copied yet"], ty.caption, list.w - 24.0);
            g.text_alpha(list.x + 14.0 + dx, top + row_h * 0.5, line, ty.caption, t.dim, alpha);
            return;
        }
        let in_sight = ((list.h - LIST_ABOVE - LIST_BELOW + ROW_GAP) / step).floor().max(1.0) as usize;
        let most = self.rows_most.max(1);
        // The first row in sight follows the focus, gliding there.
        let mut first = self.scroll.target().round() as usize;
        if self.focus < first {
            first = self.focus;
        } else if self.focus >= first + most {
            first = self.focus + 1 - most;
        }
        let first = first.min(self.rows.len().saturating_sub(most));
        let (first_f, focus_f) = (first as f32, self.focus as f32);
        if !self.placed || self.sheet.calm {
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
        let x = list.x + INSET + dx;
        let w = list.w - 2.0 * INSET;
        let radius = (g.radius(crate::sheet::RADIUS) - INSET).max(2.0).min(row_h * 0.5);
        // The highlight: a state layer in the words' colour, and the ring.
        let y = top + (self.ring.value() - scroll) * step;
        let r = Rect::new(x, y, w, row_h);
        g.canvas.rrect(r, radius, t.ink, 0.12 * alpha);
        g.canvas.stroke_rrect(r.inset(-1.5), radius + 1.5, 1.5, g.ring_color(), alpha);
        let from = scroll.floor().max(0.0) as usize;
        let whole_end = top + in_sight as f32 * step - ROW_GAP;
        self.fade.take(g, whole_end + RING_ROOM, list.bottom() - 1.0);
        let now = chrono::Local::now();
        for i in from..(from + in_sight + 1).min(self.rows.len()) {
            let y = top + (i as f32 - scroll) * step;
            if y > list.bottom() || y + row_h < list.y {
                continue;
            }
            let r = Rect::new(x, y, w, row_h);
            let focused = i == self.focus;
            let cy = r.cy();
            let (icon, filled, title, detail) = match &self.rows[i] {
                Row::Secret(left) => ("key", true, "••••••••".to_owned(), format!("clears in {left} s")),
                Row::Clip(item) => {
                    let lines = item.text.trim_end().lines().count();
                    let icon = if item.pinned {
                        "push_pin"
                    } else if lines > 1 {
                        "notes"
                    } else if looks_like_link(&item.text) {
                        "link"
                    } else {
                        "content_paste"
                    };
                    let when = age(item.at, now);
                    let detail = match (item.pinned, lines > 1) {
                        (true, _) => "pinned".to_owned(),
                        (false, true) => format!("{lines} lines · {when}"),
                        _ => when,
                    };
                    (icon, item.pinned, first_line(&item.text), detail)
                }
            };
            g.icon_alpha(icon, filled || focused, r.x + 13.0, cy, 14.0, if focused { t.ink } else { t.dim }, alpha);
            let room = w - 26.0 - 9.0 - 12.0;
            let detail_w = g.measure(&detail, ty.caption).min(room * 0.45);
            let detail = g.text.fit(&detail, &ty.caption, detail_w + 0.5);
            text_right(g, r.right() - 9.0, cy, &detail, ty.caption, t.dim, alpha);
            let title = g.text.fit(&title, &ty.body, room - detail_w);
            g.text_alpha(r.x + 26.0, cy, &title, ty.body, t.ink, alpha);
        }
        self.fade.apply(g);
        if self.rows.len() > most {
            let track = Rect::new(list.right() - 3.5 + dx, top, 2.0, (whole_end - top).min(list.h - LIST_ABOVE - LIST_BELOW));
            let share = most as f32 / self.rows.len() as f32;
            let at = scroll / (self.rows.len() - most) as f32;
            let thumb = Rect::new(track.x, track.y + (track.h - track.h * share) * at.clamp(0.0, 1.0), 2.0, (track.h * share).max(8.0));
            g.canvas.rrect(thumb, 1.0, t.ink, 0.35 * alpha);
        }
    }

    /// The focused clip in whole: as many of its lines as fit, wrapped, and
    /// how long it is and when it was copied.
    fn draw_whole(&mut self, g: &mut Gfx, list: Rect, dx: f32, alpha: f32) {
        let Some(Row::Clip(item)) = self.rows.get(self.focus) else { return };
        let t = g.tokens;
        let ty = g.ty;
        let area = Rect::new(list.x + INSET + 9.0 + dx, list.y + LIST_ABOVE + 2.0, list.w - 2.0 * INSET - 18.0, list.h - LIST_ABOVE - LIST_BELOW - 2.0);
        let line_h = ty.caption.size + 4.0;
        let fit = ((area.h - line_h) / line_h).floor().max(1.0) as usize;
        let mut lines = Vec::new();
        for paragraph in item.text.lines() {
            if paragraph.trim().is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(crate::help::wrap(g, &paragraph.replace('\t', "  "), ty.caption, area.w));
            }
            if lines.len() > fit {
                break;
            }
        }
        let more = lines.len() > fit;
        for (k, line) in lines.iter().take(fit).enumerate() {
            let line = if more && k + 1 == fit { g.text.fit(&format!("{line} …"), &ty.caption, area.w) } else { line.clone() };
            g.text_alpha(area.x, area.y + (k as f32 + 0.5) * line_h, &line, ty.caption, t.ink.mix(t.dim, 0.25), alpha);
        }
        let count = item.text.chars().count();
        let n_lines = item.text.trim_end().lines().count();
        let when = age(item.at, chrono::Local::now());
        let when = if when == "now" { "just now".to_owned() } else { format!("{when} ago").replace("yesterday ago", "yesterday") };
        let characters = if count == 1 { "1 character".to_owned() } else { format!("{count} characters") };
        let foot = [
            if n_lines > 1 { format!("{n_lines} lines · {characters} · {when}") } else { format!("{characters} · {when}") },
            format!("{characters} · {when}"),
            when.clone(),
        ];
        let choices: Vec<&str> = foot.iter().map(String::as_str).collect();
        let foot = first_fitting(g, &choices, ty.caption, area.w);
        g.text_alpha(area.x, area.bottom() - line_h * 0.5, foot, ty.caption, t.dim, alpha);
    }
}

/// A clip's first line with words, as a row shows it.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).take(200).collect()
}

fn looks_like_link(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("ssh://")) && !t.contains(char::is_whitespace)
}

/// When a clip was copied, in a few characters: "now", "4 min", "2 h",
/// "yesterday", "nov 21".
pub fn age(at: i64, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::TimeZone;
    let seconds = now.timestamp() - at;
    match seconds {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{} min", s / 60),
        s if s < 6 * 3600 => format!("{} h", s / 3600),
        _ => {
            let Some(then) = chrono::Local.timestamp_opt(at, 0).single() else { return String::new() };
            match (now.date_naive() - then.date_naive()).num_days() {
                0 => then.format("%H:%M").to_string(),
                1 => "yesterday".into(),
                _ => then.format("%b %-d").to_string().to_lowercase(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{copy, Sensitivity};

    fn panel() -> (Panel, Gfx) {
        let g = Gfx::new(bezel::Tokens::dark());
        let mut p = Panel::new();
        p.summon(&g, true, None);
        (p, g)
    }

    #[test]
    fn enter_pastes_the_clip_in_focus_and_the_arrows_move() {
        copy("first", Sensitivity::Normal);
        copy("second", Sensitivity::Normal);
        let (mut p, _) = panel();
        assert_eq!(p.key(Key::Char('x')), Outcome::Stay, "x is down here");
        assert_eq!(p.key(Key::Enter), Outcome::Paste("first".into()));
        assert_eq!(p.key(Key::Up), Outcome::Stay);
        assert_eq!(p.key(Key::Enter), Outcome::Paste("second".into()), "newest first");
        assert_eq!(p.key(Key::Back), Outcome::Close);
        assert_eq!(p.key(crate::clipboard::HISTORY_KEY), Outcome::Close, "its own key puts it away");
    }

    #[test]
    fn p_pins_to_the_top_and_d_removes_a_pinned_one_only_when_asked_twice() {
        copy("keep me", Sensitivity::Normal);
        copy("newer", Sensitivity::Normal);
        let (mut p, _) = panel();
        p.key(Key::Down);
        assert_eq!(p.key(Key::Char('p')), Outcome::Say("pinned".into()));
        assert_eq!(p.focus, 0, "it moved to the top, and the focus with it");
        assert!(matches!(&p.rows[0], Row::Clip(i) if i.text == "keep me" && i.pinned));
        assert_eq!(p.key(Key::Char('d')), Outcome::Say("pinned · d again removes it".into()));
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.key(Key::Char('d')), Outcome::Say("removed".into()));
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.key(Key::Char('d')), Outcome::Say("removed".into()), "an unpinned one goes at once");
        assert!(p.rows.is_empty());
        assert_eq!(p.key(Key::Enter), Outcome::Close, "nothing to paste");
    }

    #[test]
    fn a_secret_shows_as_dots_first_and_pastes_but_is_never_pinned() {
        copy("plain", Sensitivity::Normal);
        copy("hunter2", Sensitivity::Secret);
        let (mut p, _) = panel();
        assert!(matches!(p.rows[0], Row::Secret(30)));
        assert_eq!(p.rows.len(), 2, "the secret and the history, without it");
        assert_eq!(p.key(Key::Char('p')), Outcome::Say("a secret is never kept".into()));
        assert_eq!(p.key(Key::Enter), Outcome::Paste("hunter2".into()));
        assert_eq!(p.key(Key::Char('d')), Outcome::Say("the secret is gone".into()));
        assert_eq!(crate::clipboard::paste_text(), None);
        assert_eq!(p.rows.len(), 1);
    }

    #[test]
    fn its_height_holds_its_rows_and_never_comes_down_on_a_hint_line() {
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            let mut g = Gfx::new(bezel::Tokens::dark());
            g.set_text_size(size);
            // Over a screen with a hint along its foot, and one without (the Terminal).
            for foot in [g.above_hint(), crate::gfx::H] {
                let mut p = Panel::new();
                p.foot = foot;
                for rows in 0..40 {
                    let h = p.height_for(&g, rows);
                    assert!(TOP + h <= crate::gfx::H - FOOT + 0.5, "{rows} rows at {}: {h}", size.name());
                    assert!(TOP + h <= foot + 0.5, "{rows} rows at {}: its foot at {} comes down on the hint at {foot}", size.name(), TOP + h);
                }
                assert!(p.height_for(&g, 1) < p.height_for(&g, 2));
                assert_eq!(TOP + p.height_for(&g, 40), lowest(foot), "more than it holds: on to the foot");
                p.previewing = true;
                assert_eq!(TOP + p.height_for(&g, 1), lowest(foot), "one clip in whole: all the room there is");
            }
        }
    }

    #[test]
    fn ages_read_in_a_few_characters() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 21, 15, 0, 0).unwrap();
        let at = now.timestamp();
        assert_eq!(age(at - 10, now), "now");
        assert_eq!(age(at - 600, now), "10 min");
        assert_eq!(age(at - 7200, now), "2 h");
        assert_eq!(age(at - 86_400, now), "yesterday");
        assert_eq!(age(at - 30 * 86_400, now), "oct 22");
    }
}
