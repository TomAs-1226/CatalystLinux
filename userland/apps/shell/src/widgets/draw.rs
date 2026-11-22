//! Each widget at each size, drawn on a tile Home has already laid down:
//! Bezel's wall-panel tile, its label small, lowercase and in the mono voice
//! at the top left and its symbol at the top right, its reading large in
//! the display voice at the foot. A wide widget says two more things at its
//! right; a large one has room for a line under its reading and rows of
//! detail, or a picture of the day.
//!
//! Every layout is measured from the type scale, so it holds at each text
//! size; anything that could still be too long is cut short with an
//! ellipsis rather than run off the tile. Each drawing says when it next
//! changes by itself (a minute turning, a countdown's second), or `None`.

use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect, TextStyle};

use super::{Facts, Kind, Size};
use crate::gfx::Gfx;
use crate::robot::Robot;

/// What a widget is drawn with besides its facts.
pub struct Ctx<'a> {
    pub facts: &'a Facts,
    pub robot: &'a Robot,
    /// The robot value pinned to the glance page.
    pub pinned: Option<&'a str>,
    pub now: chrono::DateTime<chrono::Local>,
    pub focused: bool,
    /// The tile's corners, for a picture that fills it.
    pub radius: f32,
}

/// When a drawing next changes by itself.
pub type Next = Option<Instant>;

/// The text's inset from the tile's edge (Bezel's tile padding, scaled).
const IN: f32 = 9.0;
/// The reading's size, before the text size's scale: small and wide tiles,
/// and large ones.
const READING: f32 = 26.0;
const READING_LARGE: f32 = 34.0;

/// Draw `kind` at `size` on `r`. `r` is the size the widget is laid out
/// for; while it grows or shrinks to it, Home clips it to where it is.
pub fn content(g: &mut Gfx, r: Rect, kind: Kind, size: Size, cx: &Ctx) -> Next {
    match kind {
        Kind::NextMatch => next_match(g, r, size, cx),
        Kind::Battery => battery(g, r, size, cx),
        Kind::Music => music(g, r, size, cx),
        Kind::Weather => weather(g, r, size, cx),
        Kind::Robot => robot(g, r, size, cx),
        Kind::Reminders => reminders(g, r, size, cx),
        Kind::Clock => clock(g, r, size, cx),
        Kind::World => world(g, r, size, cx),
        Kind::Timers => timers(g, r, size, cx),
        Kind::Note => note(g, r, size, cx),
        Kind::System => system(g, r, size, cx),
        Kind::Photos => photos(g, r, size, cx),
        Kind::Actions => actions(g, r, size, cx),
        Kind::LastMatch => last_match(g, r, size, cx),
        Kind::Pinned => pinned(g, r, size, cx),
    }
}

// ---------------------------------------------------------------- pieces ---

/// The label's middle line.
fn label_cy(g: &Gfx, r: Rect) -> f32 {
    r.y + 7.0 + g.ty.caption.size * 0.5
}

/// Where the label's line ends, below.
fn label_bottom(g: &Gfx, r: Rect) -> f32 {
    r.y + 7.0 + g.ty.caption.size
}

/// The quiet colour, brighter while the tile has the focus.
fn quiet(g: &Gfx, focused: bool) -> Color {
    if focused {
        g.tokens.ink
    } else {
        g.tokens.dim
    }
}

// What the widgets had to cut short (the tests hold them to none): only
// the words a widget chooses itself, never what people wrote (a song's
// title, a reminder), which is cut quietly where it must be.
#[cfg(test)]
thread_local! {
    pub static CUTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Words a widget chose, cut to `room` with an ellipsis only if they must
/// be (and noted, for the tests).
fn fit(g: &mut Gfx, text: &str, style: &TextStyle, room: f32) -> String {
    let fitted = g.text.fit(text, style, room.max(0.0));
    #[cfg(test)]
    if fitted != text {
        CUTS.with(|c| c.borrow_mut().push(text.to_owned()));
    }
    fitted
}

/// The first of `choices` that fits `room` whole, else the last cut short.
fn first_fit(g: &mut Gfx, choices: &[String], style: TextStyle, room: f32) -> String {
    match choices.iter().find(|c| g.measure(c, style) <= room + 0.5) {
        Some(c) => c.clone(),
        None => fit(g, choices.last().map_or("", String::as_str), &style, room),
    }
}

/// The label at the top left, lowercase, and a symbol at the top right.
fn head(g: &mut Gfx, r: Rect, label: &str, icon: Option<(&str, Color)>, focused: bool) {
    head_any(g, r, &[label.to_owned()], icon, focused);
}

/// The label: the first of `labels` that fits whole (longest first).
fn head_any(g: &mut Gfx, r: Rect, labels: &[String], icon: Option<(&str, Color)>, focused: bool) {
    let ty = g.ty;
    let cy = label_cy(g, r);
    let room = r.w - IN - if icon.is_some() { 26.0 } else { IN };
    let labels: Vec<String> = labels.iter().map(|l| l.to_lowercase()).collect();
    let label = first_fit(g, &labels, ty.caption, room);
    let color = quiet(g, focused);
    g.text_at(r.x + IN, cy, Align::Left, &label, ty.caption, color);
    if let Some((name, tint)) = icon {
        g.icon(name, focused, r.right() - 13.0, cy, 14.0, tint);
    }
}

/// A line of the widget's own words at `(x, cy)`, cut to `room` if it must be.
#[allow(clippy::too_many_arguments)]
fn line(g: &mut Gfx, x: f32, cy: f32, text: &str, style: TextStyle, color: Color, room: f32) {
    let fitted = fit(g, text, &style, room);
    g.text_at(x, cy, Align::Left, &fitted, style, color);
}

/// "in 23 min" as "23 min", "sat 10:42" as "10:42": a time said shorter.
fn short_when(when: &str) -> String {
    match when.strip_prefix("in ") {
        Some(rest) => rest.to_owned(),
        None => when.rsplit(' ').next().filter(|t| t.contains(':')).unwrap_or(when).to_owned(),
    }
}

/// A reading in the display voice at `size` (before the text size's scale)
/// or smaller to fit `room`, its unit after it in the caption voice. Its
/// width.
#[allow(clippy::too_many_arguments)]
fn reading(g: &mut Gfx, x: f32, baseline: f32, value: &str, unit: &str, size: f32, room: f32, color: Color) -> f32 {
    let (t, ty) = (g.tokens, g.ty);
    let unit_w = if unit.is_empty() { 0.0 } else { 3.0 + g.measure(unit, ty.caption) };
    let mut s = size;
    let mut style = ty.display(s);
    while g.measure(value, style) + unit_w > room && s > 16.0 {
        s -= 2.0;
        style = ty.display(s);
    }
    // Still too long for a unit beside it: the reading alone.
    let unit_w = if g.measure(value, style) + unit_w > room { 0.0 } else { unit_w };
    let fitted = fit(g, value, &style, room - unit_w);
    let color = if value == "–" { t.faint } else { color };
    let w = g.text(x, baseline, &fitted, style, color);
    if unit_w > 0.0 {
        g.text(x + w + 3.0, baseline, unit, ty.caption, t.dim);
    }
    w + unit_w
}

/// The first of `units` that fits beside `value` at full size (longest
/// first), else the last.
fn choose_unit(g: &mut Gfx, value: &str, units: &[String], size: f32, room: f32) -> String {
    let (ty, w) = (g.ty, g.measure(value, g.ty.display(size)));
    units.iter().find(|u| w + 3.0 + g.measure(u, ty.caption) <= room).or(units.last()).cloned().unwrap_or_default()
}

/// A small or wide tile's reading, at its foot.
fn foot_reading(g: &mut Gfx, r: Rect, value: &str, unit: &str, room: f32, color: Color) -> f32 {
    reading(g, r.x + IN - 1.0, r.bottom() - 8.0, value, unit, READING, room, color)
}

/// Where a large tile's reading sits, and the line under it.
fn large_lines(g: &mut Gfx, r: Rect) -> (f32, f32) {
    let style = g.ty.display(READING_LARGE);
    let baseline = (label_bottom(g, r) + 7.0 + g.text.cap_height(&style)).round();
    (baseline, baseline + 7.0 + g.ty.caption.size * 0.5)
}

/// Where a large tile's rows may begin: below the line under its reading.
fn rows_top(g: &mut Gfx, r: Rect) -> f32 {
    large_lines(g, r).1 + g.ty.caption.size * 0.5 + 4.0
}

/// Up to two lines at a wide tile's right, the last on the reading's
/// baseline, each cut to `room`.
fn side(g: &mut Gfx, r: Rect, lines: &[(String, Color)], room: f32) {
    let ty = g.ty;
    let step = ty.caption.size + 5.0;
    let last = r.bottom() - 8.0 - ty.caption.size * 0.36;
    for (i, (text, color)) in lines.iter().rev().enumerate() {
        let fitted = fit(g, text, &ty.caption, room);
        g.text_at(r.right() - IN, last - i as f32 * step, Align::Right, &fitted, ty.caption, *color);
    }
}

/// The first of each line's choices that fits (longest first), for [`side`].
fn side_any(g: &mut Gfx, r: Rect, lines: &[(Vec<String>, Color)], room: f32) {
    let ty = g.ty;
    let chosen: Vec<(String, Color)> = lines.iter().map(|(choices, c)| (first_fit(g, choices, ty.caption, room), *c)).collect();
    side(g, r, &chosen, room);
}

/// A short meter at a wide tile's right, where its second line would be.
fn side_meter(g: &mut Gfx, r: Rect, fraction: f32, fill: Color, room: f32) {
    let ty = g.ty;
    let cy = r.bottom() - 8.0 - ty.caption.size * 0.36;
    let w = room.clamp(24.0, 96.0);
    g.meter(Rect::new(r.right() - IN - w, (cy - 2.5).round(), w, 5.0), fraction, fill);
}

/// Rows of name and value along a large tile's foot, as many as fit below
/// `top`.
fn rows(g: &mut Gfx, r: Rect, top: f32, rows: &[(String, String, Color)]) {
    let (t, ty) = (g.tokens, g.ty);
    let line = ty.caption.size + 5.0;
    let n = (((r.bottom() - 7.0 - top) / line).floor().max(0.0) as usize).min(rows.len());
    let y0 = r.bottom() - 7.0 - n as f32 * line;
    for (i, (name, value, color)) in rows.iter().take(n).enumerate() {
        let cy = y0 + (i as f32 + 0.5) * line;
        let name = fit(g, name, &ty.caption, r.w * 0.45);
        let name_w = g.text_at(r.x + IN, cy, Align::Left, &name, ty.caption, t.dim);
        let value = fit(g, value, &ty.caption, r.w - 2.0 * IN - name_w - 10.0);
        g.text_at(r.right() - IN, cy, Align::Right, &value, ty.caption, *color);
    }
}

/// A few words where a reading would be: in the body voice on one or two
/// lines at the foot, or the caption's if they need it to fit whole.
fn words(g: &mut Gfx, r: Rect, text: &str, color: Color) {
    let ty = g.ty;
    let room = r.w - 2.0 * IN;
    let lines_room = ((r.bottom() - 8.0 - label_bottom(g, r) - 4.0) / (ty.body.size + 3.0)).floor().clamp(1.0, 4.0) as usize;
    for style in [ty.body, ty.caption] {
        let lines = g.text.wrap(text, &style, room, false);
        if lines.len() <= lines_room.min(if style.size == ty.body.size { 2 } else { 3 }) && lines.iter().all(|l| g.measure(l, style) <= room + 0.5) {
            return lines_at_foot(g, r, &lines, style, color);
        }
    }
    let one = fit(g, text, &ty.caption, room);
    lines_at_foot(g, r, &[one], ty.caption, color);
}

/// The first of `choices` that fits as [`words`] does.
fn words_any(g: &mut Gfx, r: Rect, choices: &[&str], color: Color) {
    let ty = g.ty;
    let room = r.w - 2.0 * IN;
    let lines_room = ((r.bottom() - 8.0 - label_bottom(g, r) - 4.0) / (ty.body.size + 3.0)).floor().clamp(1.0, 4.0) as usize;
    let whole = |g: &mut Gfx, text: &str| {
        [ty.body, ty.caption].iter().any(|style| {
            let lines = g.text.wrap(text, style, room, false);
            lines.len() <= lines_room.min(if style.size == ty.body.size { 2 } else { 3 }) && lines.iter().all(|l| g.measure(l, *style) <= room + 0.5)
        })
    };
    let pick = choices.iter().copied().find(|c| whole(g, c)).or(choices.last().copied()).unwrap_or("");
    words(g, r, pick, color);
}

fn lines_at_foot(g: &mut Gfx, r: Rect, lines: &[String], style: TextStyle, color: Color) {
    let step = style.size + 3.0;
    let last = r.bottom() - 8.0;
    for (i, line) in lines.iter().rev().enumerate() {
        g.text(r.x + IN, (last - i as f32 * step).round(), line, style, color);
    }
}

/// Text wrapped into `room`, at most `most` lines, the last cut short if
/// there is more.
fn wrapped(g: &mut Gfx, text: &str, style: TextStyle, room: f32, most: usize) -> Vec<String> {
    let mut lines = g.text.wrap(text, &style, room, true);
    if lines.len() > most {
        let rest = lines.split_off(most - 1).join(" ");
        lines.push(g.text.fit(&rest, &style, room));
    }
    lines
}

/// A ring meter round a point (a timer's, where the symbol would be).
fn ring(g: &mut Gfx, cx: f32, cy: f32, radius: f32, fraction: f32, color: Color) {
    let t = g.tokens;
    let x = cx + g.ox;
    g.canvas.arc(x, cy, radius, 2.0, 0.0, std::f32::consts::TAU, t.meter, 1.0);
    if fraction > 0.005 {
        g.canvas.arc(x, cy, radius, 2.0, -std::f32::consts::FRAC_PI_2, std::f32::consts::TAU * fraction.min(1.0), color, 1.0);
    }
}

/// When a whole minute next turns.
fn next_minute(now: chrono::DateTime<chrono::Local>) -> Next {
    use chrono::Timelike;
    let left = 60.0 - now.second() as f64 - now.nanosecond() as f64 / 1e9;
    Some(Instant::now() + Duration::from_secs_f64(left.max(0.0) + 0.02))
}

/// The time as the clock setting has it, and "am" or "pm" on the 12-hour clock.
fn clock_text(at: chrono::NaiveDateTime, h24: bool) -> (String, String) {
    if h24 {
        (at.format("%H:%M").to_string(), String::new())
    } else {
        (at.format("%-I:%M").to_string(), at.format("%P").to_string())
    }
}

// ----------------------------------------------------------------- kinds ---

fn next_match(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let t = g.tokens;
    let unix = cx.now.timestamp();
    let m = match crate::upnext::next_match_in(cx.facts.view.as_ref(), unix) {
        Ok(m) => m,
        Err(why) => {
            head(g, r, "next match", Some(("flag", quiet(g, cx.focused))), cx.focused);
            words_any(g, r, &[&format!("{why} · settings › robot"), &why], t.dim);
            return None;
        }
    };
    // Now queuing (or on deck, on the field) is what to act on.
    let urgent = matches!(&m.queue, Some(("now", s)) if !s.contains("soon"));
    // When, said as long as there is room for: "queue in 8 min", "queue 8 min", "8 min".
    let when: Vec<String> = match &m.queue {
        Some(("now", status)) => vec![status.clone()],
        Some((_, at)) => vec![format!("queue {at}"), format!("queue {}", short_when(at)), short_when(at)],
        None => vec![m.when.clone(), short_when(&m.when)],
    };
    let accent = if urgent { t.signal } else { t.ink };
    let side_word = m.side.unwrap_or("");
    let label_w = r.w - 2.0 * IN;
    match size {
        Size::Small => {
            head_any(g, r, &when, Some(("flag", m.color)), cx.focused);
            foot_reading(g, r, &m.label, side_word, label_w, accent);
        }
        Size::Wide => {
            head(g, r, "next match", Some(("flag", m.color)), cx.focused);
            let w = foot_reading(g, r, &m.label, side_word, label_w * 0.5, accent);
            let with = if m.with.is_empty() { vec![format!("team {}", m.team)] } else { vec![format!("with {}", crate::upnext::teams(&m.with)), crate::upnext::teams(&m.with)] };
            side_any(g, r, &[(when, accent), (with, t.dim)], r.w - 2.0 * IN - w - 12.0);
        }
        Size::Large => {
            let event = m.event.clone().map(|e| e.to_lowercase()).unwrap_or_else(|| "next match".into());
            head_any(g, r, &[event, "next match".into()], Some(("flag", m.color)), cx.focused);
            let (baseline, under) = large_lines(g, r);
            reading(g, r.x + IN - 1.0, baseline, &m.label, side_word, READING_LARGE, label_w, accent);
            let under_line = if m.side.is_some() { format!("{side_word} · {}", m.when) } else { m.when.clone() };
            line(g, r.x + IN, under, &under_line, g.ty.caption, t.dim, label_w);
            let mut list = Vec::new();
            if let Some((name, value)) = &m.queue {
                list.push(((*name).to_owned(), value.clone(), if urgent { t.signal } else { t.ink }));
            }
            if !m.with.is_empty() {
                list.push(("with".into(), crate::upnext::teams(&m.with), t.ink));
            }
            list.push(("against".into(), crate::upnext::teams(&m.against), t.ink));
            if let Some((label, when, _)) = m.after.first() {
                list.push(("after".into(), format!("{label} · {when}"), t.dim));
            }
            let top = rows_top(g, r);
            rows(g, r, top, &list);
        }
    }
    // The countdowns read in minutes: again as one turns.
    let view = cx.facts.view.as_ref()?;
    let next = view.next(unix)?;
    let soonest = [next.time, next.queue_at]
        .into_iter()
        .flatten()
        .map(|at| {
            let d = at - unix;
            if d > 0 {
                d % 60 + 1
            } else {
                60 - (-d) % 60
            }
        })
        .min()?;
    Some(Instant::now() + Duration::from_secs(soonest.clamp(1, 60) as u64))
}

fn battery(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    use crate::power::Flow;
    let t = g.tokens;
    let Some(b) = cx.facts.battery.clone() else {
        head(g, r, "battery", Some(("battery_full", quiet(g, cx.focused))), cx.focused);
        words(g, r, "no battery reading here", t.dim);
        return None;
    };
    let charging = b.flow == Flow::Charging;
    let low = b.flow.on_battery() && b.percent <= 15;
    let (icon, tint) = match () {
        _ if charging => ("battery_charging_full", g.good()),
        _ if low => ("battery_alert", g.fault()),
        _ => ("battery_full", quiet(g, cx.focused)),
    };
    let label = match b.flow {
        Flow::Charging => "charging",
        Flow::Full => "full",
        Flow::Holding => "plugged in",
        _ => "battery",
    };
    let fill = if low { g.fault() } else { t.ice };
    let value = b.percent.to_string();
    let color = if low { g.fault() } else { t.ink };
    let room = r.w - 2.0 * IN;
    let left: Vec<String> = match (b.flow, b.minutes_left, b.minutes_to_full) {
        (Flow::Discharging, Some(m), _) => vec![format!("{} left", crate::sys::minutes_text(m)), crate::sys::minutes_text(m)],
        (Flow::Charging, _, Some(m)) => vec![format!("full in {}", crate::sys::minutes_text(m)), crate::sys::minutes_text(m)],
        (Flow::Charging, _, None) => vec!["charging".into()],
        (Flow::Full, ..) => vec!["full".into()],
        (Flow::Holding, ..) => vec!["held at this".into(), "held".into()],
        _ => vec!["on battery".into()],
    };
    match size {
        Size::Small => {
            head(g, r, label, Some((icon, tint)), cx.focused);
            foot_reading(g, r, &value, "%", room, color);
        }
        Size::Wide => {
            head(g, r, label, Some((icon, tint)), cx.focused);
            let w = foot_reading(g, r, &value, "%", room * 0.5, color);
            let side_room = room - w - 14.0;
            side_any(g, r, &[(left, t.ink), (vec![String::new()], t.dim)], side_room);
            side_meter(g, r, b.percent as f32 / 100.0, fill, side_room);
        }
        Size::Large => {
            head(g, r, label, Some((icon, tint)), cx.focused);
            let (baseline, under) = large_lines(g, r);
            let w = reading(g, r.x + IN - 1.0, baseline, &value, "%", READING_LARGE, room * 0.5, color);
            // The charge as a meter beside the reading.
            let meter_x = r.x + IN + w + 12.0;
            let meter_w = r.right() - IN - meter_x;
            if meter_w > 30.0 {
                let cap = g.text.cap_height(&g.ty.display(READING_LARGE));
                g.meter(Rect::new(meter_x, (baseline - cap * 0.5 - 3.0).round(), meter_w, 6.0), b.percent as f32 / 100.0, fill);
            }
            line(g, r.x + IN, under, &b.status(), g.ty.caption, t.dim, room);
            let top = rows_top(g, r);
            day_chart(g, Rect::new(r.x + IN, top + 2.0, room, r.bottom() - 9.0 - top - 2.0), &cx.facts.history, cx.now.timestamp());
        }
    }
    None
}

/// The charge over the last day: a line from a day ago to now, the hours on
/// the charger in ice, with faint rules at full and empty.
fn day_chart(g: &mut Gfx, area: Rect, points: &[crate::power::Point], now: i64) {
    let (t, ty) = (g.tokens, g.ty);
    if area.h < 14.0 || points.len() < 2 {
        return;
    }
    let ox = g.ox;
    let label_w = g.measure("24 h", ty.caption) + 6.0;
    let chart = Rect::new(area.x + label_w, area.y, area.w - label_w, area.h);
    g.text_at(area.x, chart.y + ty.caption.size * 0.5, Align::Left, "100", ty.caption, t.faint);
    g.text_at(area.x, chart.bottom() - ty.caption.size * 0.5, Align::Left, "24 h", ty.caption, t.faint);
    for y in [chart.y, chart.bottom()] {
        g.canvas.fill_rect(Rect::new(chart.x + ox, y.round() - 0.5, chart.w, 1.0), t.meter, 1.0);
    }
    let from = now - 24 * 3600;
    let at = |p: &crate::power::Point| (chart.x + ox + ((p.0 - from) as f32 / (24.0 * 3600.0)).clamp(0.0, 1.0) * chart.w, chart.bottom() - p.1 as f32 / 100.0 * chart.h);
    let (on_charger, on_battery) = (g.good(), t.ink);
    let mut run: Vec<(f32, f32)> = Vec::new();
    let mut charging = points[0].2;
    for p in points {
        if p.2 != charging && !run.is_empty() {
            let last = *run.last().unwrap_or(&at(p));
            g.canvas.polyline(&run, 1.6, if charging { on_charger } else { on_battery }, 0.9);
            run.clear();
            run.push(last);
            charging = p.2;
        }
        run.push(at(p));
    }
    if run.len() > 1 {
        g.canvas.polyline(&run, 1.6, if charging { on_charger } else { on_battery }, 0.9);
    }
}

fn music(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let Some(np) = crate::nowplaying::get() else {
        head(g, r, "music", Some(("music_note", quiet(g, cx.focused))), cx.focused);
        words(g, r, if size == Size::Small { "nothing on" } else { "nothing playing · enter opens music" }, t.dim);
        return None;
    };
    let progress = if np.duration > 0.0 { (np.position_now() / np.duration).clamp(0.0, 1.0) } else { 0.0 };
    let bar_fill = if np.playing { t.signal } else { t.dim };
    let mark = match &np.cover {
        Some(path) => crate::liveactivity::Mark::Cover(path.clone(), "music_note"),
        None => crate::liveactivity::Mark::Icon("music_note", crate::liveactivity::Tint::Signal),
    };
    let detail = if np.artist.is_empty() { np.album.clone() } else { np.artist.clone() };
    let bar_w;
    match size {
        Size::Small | Size::Wide => {
            let side_ = (r.h - 16.0).round();
            let cover = Rect::new(r.x + 8.0, r.y + 8.0, side_, side_);
            draw_cover(g, cover, &mark, cx.radius);
            let x = cover.right() + 8.0;
            let glyph = if np.playing { "pause" } else { "play_arrow" };
            let roomy = size == Size::Wide || r.w >= 140.0;
            let glyph_w = if size == Size::Wide { 24.0 } else { 0.0 };
            let text_room = r.right() - IN - x - glyph_w;
            if roomy {
                let title_cy = cover.y + ty.body.size * 0.5 + 1.0;
                g.text_fit(x, title_cy, &np.title, ty.body, t.ink, text_room);
                g.text_fit(x, title_cy + ty.body.size * 0.5 + 3.0 + ty.caption.size * 0.5, &detail, ty.caption, t.dim, text_room);
            }
            if size == Size::Wide || !roomy {
                // Enter's act, where the eye looks for it.
                let (gx, gy, gs) = if roomy { (r.right() - IN - 9.0, cover.y + 11.0, 18.0) } else { ((x + r.right()) * 0.5, r.y + r.h * 0.42, 20.0) };
                g.icon(glyph, true, gx, gy, gs, t.ink);
            }
            let bar = Rect::new(x, cover.bottom() - 3.0, r.right() - IN - x, 3.0);
            bar_w = bar.w;
            g.meter(bar, progress, bar_fill);
        }
        Size::Large => {
            let side_ = (r.h - 62.0).clamp(40.0, 72.0).round();
            let cover = Rect::new(r.x + IN, r.y + IN, side_, side_);
            draw_cover(g, cover, &mark, cx.radius);
            let x = cover.right() + 10.0;
            let room = r.right() - IN - x;
            let title_cy = cover.y + ty.name.size * 0.5 + 1.0;
            g.text_fit(x, title_cy, &np.title, ty.name, t.ink, room);
            let detail_cy = title_cy + ty.name.size * 0.5 + 3.0 + ty.caption.size * 0.5;
            g.text_fit(x, detail_cy, &detail, ty.caption, t.dim, room);
            if !np.album.is_empty() && np.album != detail {
                g.text_fit(x, detail_cy + ty.caption.size + 4.0, &np.album, ty.caption, t.faint.mix(t.dim, 0.5), room);
            }
            // Music's keys, Fn + W, Q, E, under the words.
            let bar_y = (r.bottom() - 12.0 - ty.caption.size - 6.0).round();
            let keys_cy = (cover.bottom() - 7.0).min(bar_y - ty.caption.size - 16.0).round();
            let fw = g.measure("fn", ty.caption) + 8.0;
            let step = 26.0;
            let first = x + fw + 7.0;
            if first + 2.0 * step + 8.0 <= r.right() - IN {
                music_keys(g, [first, first + step, first + 2.0 * step], keys_cy, np.playing, 14.0);
            }
            let bar = Rect::new(r.x + IN, bar_y, r.w - 2.0 * IN, 3.0);
            bar_w = bar.w;
            g.meter(bar, progress, bar_fill);
            let times_cy = bar.bottom() + 6.0 + ty.caption.size * 0.5;
            let at = crate::liveactivity::clock_text(np.position_now() as f64);
            g.text_at(r.x + IN, times_cy, Align::Left, &at, ty.caption, t.dim);
            if np.duration > 0.0 {
                let left = format!("−{}", crate::liveactivity::clock_text((np.duration - np.position_now()).max(0.0) as f64));
                g.text_at(r.right() - IN, times_cy, Align::Right, &left, ty.caption, t.dim);
            }
        }
    }
    // A cover still being decoded: look again soon. Playing: the bar moves
    // on a pixel at a time, coarsely (and the large tile's seconds, each one).
    if np.cover.as_deref().is_some_and(crate::liveactivity::cover_pending) {
        return Some(Instant::now() + Duration::from_millis(150));
    }
    if !np.playing || np.duration <= 0.0 {
        return None;
    }
    let per_pixel = if size == Size::Large { 1.0 } else { (np.duration / bar_w.max(1.0)).clamp(1.0, 5.0) };
    Some(Instant::now() + Duration::from_secs_f32(per_pixel))
}

/// Music's three keys (Fn + W, Q, E) as the lock screen's card has them:
/// their marks centred on `xs`, the letters beneath and "fn" once before.
fn music_keys(g: &mut Gfx, xs: [f32; 3], cy: f32, playing: bool, size: f32) {
    let (t, ty) = (g.tokens, g.ty);
    let quiet = t.faint.mix(t.dim, 0.5);
    let below = cy + size * 0.5 + 5.0 + ty.caption.size * 0.36;
    for ((icon, key), x) in crate::liveactivity::MUSIC_KEYS.iter().zip(xs) {
        let middle = *icon == "play_arrow";
        let icon = if middle && playing { "pause" } else { icon };
        g.icon(icon, true, x, cy, if middle { size * 1.25 } else { size }, t.ink);
        g.text_at(x, below, Align::Center, key, ty.caption, quiet);
    }
    let fw = g.measure("w", ty.caption);
    g.text_at(xs[0] - fw * 0.5 - 5.0, below, Align::Right, "fn", ty.caption, quiet);
}

/// A cover, or its stand-in while it decodes.
fn draw_cover(g: &mut Gfx, r: Rect, mark: &crate::liveactivity::Mark, radius: f32) {
    let t = g.tokens;
    match mark {
        crate::liveactivity::Mark::Cover(path, _) => match crate::liveactivity::cover(path) {
            Some(pic) => crate::photos::draw_picture(g, r, &pic, crate::photos::whole(&pic), (radius * 0.6).clamp(3.0, 10.0), 1.0),
            None => {
                g.surface(r, (radius * 0.6).clamp(3.0, 10.0), t.surface3);
                g.icon("music_note", true, r.cx(), r.cy(), (r.w * 0.45).min(22.0), t.signal);
            }
        },
        crate::liveactivity::Mark::Icon(name, _) => {
            g.surface(r, (radius * 0.6).clamp(3.0, 10.0), t.surface2.mix(t.signal, 0.14));
            g.icon(name, true, r.cx(), r.cy(), (r.w * 0.45).min(22.0), t.signal);
        }
    }
}

fn weather(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let Some(w) = cx.facts.weather.clone() else {
        head(g, r, "weather", Some(("partly_cloudy_day", quiet(g, cx.focused))), cx.focused);
        words_any(g, r, &["no forecast yet · choose a place in weather", "no forecast yet"], t.dim);
        return None;
    };
    let room = r.w - 2.0 * IN;
    // The place, where it fits whole; else what this is.
    head_any(g, r, &[w.place.clone(), "weather".into()], Some((w.icon, quiet(g, cx.focused))), cx.focused);
    match size {
        Size::Small => {
            foot_reading(g, r, &w.temp, "", room, t.ink);
        }
        Size::Wide => {
            let used = foot_reading(g, r, &w.temp, "", room * 0.45, t.ink);
            let second = match (w.rain, &w.range) {
                (Some(p), _) => vec![format!("rain {p}%")],
                (None, Some(range)) => vec![range.clone(), range.replace(" · ", " ")],
                _ => vec![String::new()],
            };
            side_any(g, r, &[(vec![w.condition.to_owned()], t.ink), (second, t.dim)], room - used - 12.0);
        }
        Size::Large => {
            let (baseline, under) = large_lines(g, r);
            reading(g, r.x + IN - 1.0, baseline, &w.temp, "", READING_LARGE, room * 0.5, t.ink);
            let mut full = w.condition.to_owned();
            if let Some(range) = &w.range {
                full = format!("{full} · {range}");
            }
            let mut with_rain = full.clone();
            if let Some(p) = w.rain {
                with_rain = format!("{full} · rain {p}%");
            }
            let under_line = first_fit(g, &[with_rain, full, w.condition.to_owned()], ty.caption, room);
            g.text_at(r.x + IN, under, Align::Left, &under_line, ty.caption, t.dim);
            // The next hours across the foot: the hour, its weather, its temperature.
            let top = rows_top(g, r);
            let hours: Vec<_> = w.hours.iter().take(if r.w > 250.0 { 6 } else { 4 }).collect();
            if !hours.is_empty() && r.bottom() - 8.0 - top >= ty.caption.size * 2.0 + 16.0 {
                let col = room / hours.len() as f32;
                let (hour_cy, temp_cy) = (top + ty.caption.size * 0.5 + 1.0, r.bottom() - 8.0 - ty.caption.size * 0.5);
                let icon_cy = (hour_cy + temp_cy) * 0.5;
                for (i, (hour, icon, temp)) in hours.iter().enumerate() {
                    let x = r.x + IN + (i as f32 + 0.5) * col;
                    g.text_at(x, hour_cy, Align::Center, hour, ty.caption, t.dim);
                    g.icon(icon, false, x, icon_cy, 16.0, t.ink);
                    g.text_at(x, temp_cy, Align::Center, temp, ty.caption, t.ink);
                }
            }
        }
    }
    None
}

fn robot(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let robot = cx.robot;
    let linked = robot.linked();
    let room = r.w - 2.0 * IN;
    let name = robot.string("Robot/Identity/Name").map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).unwrap_or_else(|| "robot".into());
    if !linked {
        head(g, r, "robot", Some(("smart_toy", t.faint)), cx.focused);
        words(g, r, &robot.status(), t.dim);
        return None;
    }
    let volts = robot.f64("Systemcore/BatteryVolts");
    let (errors, warns) = (robot.i64("Health/ErrorCount").unwrap_or(0), robot.i64("Health/WarnCount").unwrap_or(0));
    let alert = match () {
        _ if errors > 0 => Some((format!("{errors} error{}", if errors == 1 { "" } else { "s" }), g.fault())),
        _ if warns > 0 => Some((format!("{warns} warn{}", if warns == 1 { "" } else { "s" }), t.amber_ink)),
        _ => None,
    };
    let running = robot.bool("OpMode/Running") == Some(true);
    let mode = match robot.string("OpMode/Active").map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty()) {
        Some(m) if running => m,
        _ if running => "enabled".into(),
        _ => "disabled".into(),
    };
    let link = match robot.rtt_ms() {
        Some(ms) => format!("linked · {ms:.1} ms"),
        None => "linked".into(),
    };
    let tint = alert.as_ref().map_or(g.good(), |(_, c)| *c);
    let value = volts.map_or("–".into(), |v| format!("{v:.1}"));
    let color = if volts.is_some_and(|v| v < 11.0) { g.fault() } else { t.ink };
    // Its name, where it fits whole.
    head_any(g, r, &[name, "robot".into()], Some(("smart_toy", tint)), cx.focused);
    match size {
        Size::Small => {
            foot_reading(g, r, &value, "v", room, color);
        }
        Size::Wide => {
            let used = foot_reading(g, r, &value, "v", room * 0.45, color);
            let second = match alert.clone() {
                Some((words, c)) => (vec![words], c),
                None => (vec![link.clone(), "linked".into()], t.dim),
            };
            side_any(g, r, &[(vec![mode.clone()], if running { t.signal } else { t.ink }), second], room - used - 12.0);
        }
        Size::Large => {
            let (baseline, under) = large_lines(g, r);
            reading(g, r.x + IN - 1.0, baseline, &value, "v", READING_LARGE, room * 0.5, color);
            let under_line = first_fit(g, &[format!("{mode} · {link}"), format!("{mode} · linked"), mode.clone()], ty.caption, room);
            g.text_at(r.x + IN, under, Align::Left, &under_line, ty.caption, t.dim);
            let health = match &alert {
                Some((words, c)) => (words.clone(), *c),
                None if robot.i64("Health/ErrorCount").is_some() => ("all clear".into(), g.good()),
                None => ("–".into(), t.dim),
            };
            let motors = match (robot.i64("Devices/Motors/Connected"), robot.i64("Devices/Motors/Expected")) {
                (Some(c), Some(e)) => (format!("{c} of {e} up"), if c < e { g.fault() } else { t.ink }),
                _ => ("–".into(), t.dim),
            };
            let lp = match robot.f64("Loop/Robot/AverageMs") {
                Some(ms) => (format!("{ms:.1} ms"), if robot.bool("Loop/Robot/OverBudget") == Some(true) { t.amber_ink } else { t.ink }),
                None => ("–".into(), t.dim),
            };
            let mut list = vec![("health".to_owned(), health.0, health.1), ("motors".to_owned(), motors.0, motors.1), ("loop".to_owned(), lp.0, lp.1)];
            if let Some((severity, subsystem, _, detail)) = robot.firing().into_iter().next() {
                // What the robot says of it, in its words: cut quietly.
                let what = if detail.is_empty() { subsystem.to_lowercase() } else { g.text.fit(&detail, &ty.caption, r.w * 0.55) };
                list.push((subsystem.to_lowercase(), what, crate::screens::severity_color(g, &severity)));
            }
            let top = rows_top(g, r);
            rows(g, r, top, &list);
        }
    }
    None
}

fn reminders(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let list = &cx.facts.reminders;
    let Some(first) = list.first() else {
        head(g, r, "reminders", Some(("checklist", quiet(g, cx.focused))), cx.focused);
        words(g, r, "nothing coming", t.dim);
        return None;
    };
    let due_tint = |due: bool| if due { t.amber_ink } else { t.dim };
    if size == Size::Small {
        let label = if first.due { vec!["due now".to_owned(), "now".to_owned()] } else { vec![first.when.clone(), short_when(&first.when)] };
        head_any(g, r, &label, Some(("checklist", if first.due { t.amber_ink } else { quiet(g, cx.focused) })), cx.focused);
        let room = r.w - 2.0 * IN;
        let lines = wrapped(g, &first.text, ty.body, room, 2);
        lines_at_foot(g, r, &lines, ty.body, t.ink);
        return Some(Instant::now() + Duration::from_secs(30));
    }
    let more = if list.len() > 1 { vec![format!("reminders · {}", list.len()), "reminders".into()] } else { vec!["reminders".into()] };
    head_any(g, r, &more, Some(("checklist", if first.due { t.amber_ink } else { quiet(g, cx.focused) })), cx.focused);
    // One row each: a ring to tick off (the first's, Enter's), the words,
    // when at the right.
    let style = ty.body;
    let line = style.size + 5.0;
    let top = label_bottom(g, r) + 3.0;
    let n = (((r.bottom() - 5.0 - top) / line).floor().max(1.0) as usize).min(list.len());
    for (i, item) in list.iter().take(n).enumerate() {
        let cy = top + (i as f32 + 0.5) * line;
        let ring_x = r.x + IN + 4.0;
        g.canvas.stroke_rrect(Rect::new(ring_x - 4.0 + g.ox, cy - 4.0, 8.0, 8.0), 4.0, 1.3, if item.due { t.amber_ink } else { t.dim }, 1.0);
        if item.due {
            g.dot(ring_x, cy, 2.0, t.amber_ink);
        }
        let when = if item.due { "now".to_owned() } else { item.when.clone() };
        let when_w = g.measure(&when, ty.caption);
        g.text_at(r.right() - IN, cy, Align::Right, &when, ty.caption, due_tint(item.due));
        g.text_fit(r.x + IN + 14.0, cy, &item.text, style, t.ink, r.w - 2.0 * IN - 14.0 - when_w - 8.0);
    }
    Some(Instant::now() + Duration::from_secs(30))
}

/// Each world place's name, its time there, and how it stands to here: "+8 h", "tmrw".
fn place_times(cx: &Ctx, h24: bool) -> Vec<(String, String, String, String)> {
    let here = cx.now.offset().local_minus_utc() as i64;
    let local_day = cx.now.date_naive();
    cx.facts
        .places
        .iter()
        .map(|(zone, offset)| {
            let name = crate::clock::place_name(zone).to_lowercase();
            match offset {
                Some(offset) => {
                    let at = (cx.now.naive_utc() + chrono::Duration::seconds(*offset)).with_nanosecond_zero();
                    let (time, ampm) = clock_text(at, h24);
                    let day = at.date();
                    let relation = if day > local_day {
                        "tmrw".to_owned()
                    } else if day < local_day {
                        "yday".to_owned()
                    } else {
                        let hours = (*offset - here) as f32 / 3600.0;
                        match hours {
                            h if h.abs() < 0.01 => "here".to_owned(),
                            h if h.fract().abs() < 0.01 => format!("{:+} h", h as i32).replace('-', "−"),
                            h => format!("{h:+.1} h").replace('-', "−"),
                        }
                    };
                    (name, time, ampm, relation)
                }
                None => (name, "–".into(), String::new(), String::new()),
            }
        })
        .collect()
}

trait NoNanos {
    fn with_nanosecond_zero(self) -> Self;
}

impl NoNanos for chrono::NaiveDateTime {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

fn clock(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let h24 = g.clock.0;
    let (time, ampm) = clock_text(cx.now.naive_local(), h24);
    let dates = [cx.now.format("%a %-d %b").to_string().to_lowercase(), cx.now.format("%-d %b").to_string().to_lowercase()];
    let room = r.w - 2.0 * IN;
    head_any(g, r, &dates, Some(("schedule", quiet(g, cx.focused))), cx.focused);
    // The other places, here itself left out.
    let elsewhere: Vec<_> = place_times(cx, h24).into_iter().filter(|p| p.3 != "here").collect();
    match size {
        Size::Small => {
            reading(g, r.x + IN - 1.0, r.bottom() - 8.0, &time, &ampm, READING + 2.0, room, t.ink);
        }
        Size::Wide => {
            let used = reading(g, r.x + IN - 1.0, r.bottom() - 8.0, &time, &ampm, READING + 4.0, room * 0.55, t.ink);
            // Each place as long as there is room: "new york 9:41", "york 9:41", "9:41".
            let lines: Vec<(Vec<String>, Color)> = elsewhere
                .iter()
                .take(2)
                .map(|(name, time, ampm, _)| {
                    let last = name.rsplit(' ').next().unwrap_or(name);
                    (vec![format!("{name} {time}{ampm}"), format!("{last} {time}{ampm}"), format!("{time}{ampm}")], t.dim)
                })
                .collect();
            side_any(g, r, &lines, room - used - 12.0);
        }
        Size::Large => {
            let (baseline, under) = large_lines(g, r);
            reading(g, r.x + IN - 1.0, baseline, &time, &ampm, READING_LARGE + 6.0, room, t.ink);
            let zone = crate::device::zone();
            let zone_line = first_fit(g, &[crate::device::zone_label(&zone).to_lowercase(), crate::clock::place_name(&zone).to_lowercase()], ty.caption, room);
            g.text_at(r.x + IN, under, Align::Left, &zone_line, ty.caption, t.dim);
            let list: Vec<(String, String, Color)> = elsewhere.iter().map(|(name, time, ampm, rel)| (name.clone(), format!("{time}{ampm} · {rel}"), t.ink)).collect();
            let top = rows_top(g, r);
            rows(g, r, top, &list);
        }
    }
    next_minute(cx.now)
}

fn world(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let h24 = g.clock.0;
    let places = place_times(cx, h24);
    if places.is_empty() {
        head(g, r, "world", Some(("language", quiet(g, cx.focused))), cx.focused);
        words(g, r, "add places in clock", t.dim);
        return None;
    }
    let cells: Vec<Rect> = match size {
        Size::Small => vec![r],
        Size::Wide => vec![Rect::new(r.x, r.y, r.w * 0.5, r.h), Rect::new(r.x + r.w * 0.5, r.y, r.w * 0.5, r.h)],
        Size::Large => {
            let (w, h) = (r.w * 0.5, r.h * 0.5);
            vec![Rect::new(r.x, r.y, w, h), Rect::new(r.x + w, r.y, w, h), Rect::new(r.x, r.y + h, w, h), Rect::new(r.x + w, r.y + h, w, h)]
        }
    };
    // The symbol once, at the tile's top right: the place there leaves it room.
    g.icon("language", cx.focused, r.right() - 13.0, label_cy(g, r), 14.0, quiet(g, cx.focused));
    for (i, (cell, (name, time, ampm, rel))) in cells.iter().zip(places.iter()).enumerate() {
        let top_right = cell.right() >= r.right() - 1.0 && cell.y <= r.y + 1.0;
        let label_room = if top_right { cell.w - IN - 26.0 } else { cell.w - 2.0 * IN };
        let last = name.rsplit(' ').next().unwrap_or(name).to_owned();
        let label = first_fit(g, &[name.clone(), last], ty.caption, label_room);
        let color = quiet(g, cx.focused);
        g.text_at(cell.x + IN, label_cy(g, *cell), Align::Left, &label, ty.caption, color);
        let _ = i;
        let unit = if ampm.is_empty() { rel.clone() } else { ampm.clone() };
        reading(g, cell.x + IN - 1.0, cell.bottom() - 8.0, time, &unit, if size == Size::Small { READING } else { READING - 2.0 }, cell.w - 2.0 * IN, t.ink);
        if i > 0 && cell.x > r.x + 1.0 {
            // A hairline between places side by side.
            g.canvas.fill_rect(Rect::new(cell.x + g.ox - 0.5, cell.y + 8.0, 1.0, cell.h - 16.0), t.meter, 1.0);
        }
    }
    next_minute(cx.now)
}

fn timers(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let timers = crate::clock::timers_now();
    let (sw, sw_running) = crate::clock::stopwatch_now();
    let room = r.w - 2.0 * IN;
    let ring_at = (r.right() - 13.0, label_cy(g, r));
    let first = timers.first().copied();
    // What it leads with: the soonest timer, else the stopwatch, else Enter's offer.
    match (first, sw > 0.0) {
        (Some((left, total, running, done)), _) => {
            let label = match () {
                _ if done => vec!["timer done".to_owned(), "done".to_owned()],
                _ if !running => vec!["paused".to_owned()],
                _ if timers.len() > 1 => vec![format!("timer · {} more", timers.len() - 1), format!("timer +{}", timers.len() - 1), "timer".to_owned()],
                _ => vec!["timer".to_owned()],
            };
            // (The ring stands where the symbol would.)
            head_any(g, r, &label, Some(("", t.dim)), cx.focused);
            let last_minute = running && left < 60.0;
            let color = match () {
                _ if done => t.amber_ink,
                _ if last_minute => t.signal,
                _ if !running => t.dim,
                _ => t.ink,
            };
            let fraction = if total > 0.0 { (1.0 - left / total) as f32 } else { 0.0 };
            ring(g, ring_at.0, ring_at.1, 6.0, if done { 1.0 } else { fraction }, if last_minute || done { t.signal } else { g.good() });
            let value = crate::clock::length(left.ceil());
            match size {
                Size::Small => {
                    foot_reading(g, r, &value, "", room, color);
                }
                Size::Wide => {
                    let used = foot_reading(g, r, &value, "", room * 0.5, color);
                    let second = if sw_running { vec![format!("stopwatch {}", crate::clock::length(sw.floor())), crate::clock::length(sw.floor())] } else { vec![format!("of {}", crate::clock::length(total))] };
                    let act = match () {
                        _ if done => ["enter: again", "again"],
                        _ if running => ["enter: pause", "pause"],
                        _ => ["enter: go on", "go on"],
                    };
                    side_any(g, r, &[(act.map(str::to_owned).to_vec(), t.dim), (second, t.ink)], room - used - 12.0);
                }
                Size::Large => {
                    let (baseline, under) = large_lines(g, r);
                    reading(g, r.x + IN - 1.0, baseline, &value, "", READING_LARGE, room, color);
                    line(g, r.x + IN, under, &format!("of {}", crate::clock::length(total)), ty.caption, t.dim, room);
                    let mut list: Vec<(String, String, Color)> = timers
                        .iter()
                        .skip(1)
                        .map(|(left, total, running, done)| {
                            let state = if *done { "done".to_owned() } else if *running { format!("{} left", crate::clock::length(left.ceil())) } else { "paused".to_owned() };
                            (format!("timer {}", crate::clock::length(*total)), state, if *done { t.amber_ink } else { t.ink })
                        })
                        .collect();
                    if sw > 0.0 {
                        list.push(("stopwatch".into(), crate::clock::length(sw.floor()), if sw_running { t.ink } else { t.dim }));
                    }
                    let top = rows_top(g, r);
                    rows(g, r, top, &list);
                }
            }
        }
        (None, true) => {
            let label = if sw_running { vec!["stopwatch".to_owned()] } else { vec!["stopwatch · stopped".to_owned(), "stopped".to_owned()] };
            head_any(g, r, &label, Some(("timer", if sw_running { g.good() } else { quiet(g, cx.focused) })), cx.focused);
            let value = crate::clock::length(sw.floor());
            let used = foot_reading(g, r, &value, "", if size == Size::Small { room } else { room * 0.5 }, if sw_running { t.ink } else { t.dim });
            if size != Size::Small {
                side_any(g, r, &[(vec!["enter: a 5 min timer".into(), "enter: 5 min timer".into(), "5 min timer".into()], t.dim)], room - used - 12.0);
            }
        }
        (None, false) => {
            head(g, r, "timer", Some(("timer", quiet(g, cx.focused))), cx.focused);
            let quick = crate::clock::length(crate::clock::QUICK_TIMER);
            let used = foot_reading(g, r, &quick, if size == Size::Small { "enter" } else { "" }, if size == Size::Small { room } else { room * 0.5 }, t.dim);
            if size != Size::Small {
                side_any(g, r, &[(vec!["enter starts one".into(), "enter starts".into()], t.dim)], room - used - 12.0);
            }
        }
    }
    // A countdown or the stopwatch counting: again as its second turns.
    let mut soonest: Option<f64> = None;
    if let Some((left, _, true, _)) = first {
        soonest = Some(left.fract().max(0.02));
    }
    if sw_running {
        let s = (1.0 - sw.fract()).max(0.02);
        soonest = Some(soonest.map_or(s, |x| x.min(s)));
    }
    soonest.map(|s| Instant::now() + Duration::from_secs_f64(s))
}

fn note(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let Some(n) = cx.facts.note.clone() else {
        head(g, r, "note", Some(("sticky_note_2", quiet(g, cx.focused))), cx.focused);
        words(g, r, "pin a note in notes", t.dim);
        return None;
    };
    let icon = if n.pinned { "push_pin" } else { "sticky_note_2" };
    head(g, r, "note", Some((icon, quiet(g, cx.focused))), cx.focused);
    let room = r.w - 2.0 * IN;
    match size {
        Size::Small => {
            let lines = wrapped(g, &n.title, ty.body, room, 2);
            lines_at_foot(g, r, &lines, ty.body, t.ink);
        }
        Size::Wide => {
            let first = n.lines.first().cloned().unwrap_or_default();
            let lines = if first.is_empty() { wrapped(g, &n.title, ty.body, room, 2) } else { vec![g.text.fit(&n.title, &ty.body, room)] };
            if first.is_empty() {
                lines_at_foot(g, r, &lines, ty.body, t.ink);
            } else {
                let last = r.bottom() - 8.0;
                let first = g.text.fit(&first, &ty.caption, room);
                g.text(r.x + IN, last.round(), &first, ty.caption, t.dim);
                g.text(r.x + IN, (last - ty.caption.size - 5.0).round(), &lines[0], ty.body, t.ink);
            }
        }
        Size::Large => {
            let top = label_bottom(g, r) + 4.0;
            let title = wrapped(g, &n.title, ty.name, room, 2);
            let mut y = top;
            for line in &title {
                y += ty.name.size + 2.0;
                g.text(r.x + IN, y.round(), line, ty.name, t.ink);
            }
            y += 4.0;
            let step = ty.caption.size + 4.0;
            let most = ((r.bottom() - 8.0 - y) / step).floor().max(0.0) as usize;
            let body: Vec<String> = n.lines.iter().flat_map(|l| g.text.wrap(l, &ty.caption, room, true)).collect();
            for (i, line) in body.iter().take(most).enumerate() {
                let line = if i + 1 == most && body.len() > most { g.text.fit(&format!("{line} …"), &ty.caption, room) } else { line.clone() };
                y += step;
                g.text(r.x + IN, y.round(), &line, ty.caption, t.dim);
            }
        }
    }
    None
}

fn system(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let s = &cx.facts.system;
    let room = r.w - 2.0 * IN;
    let cpu = s.cpu.map_or("–".into(), |c| format!("{:.0}%", c * 100.0));
    let memory = s.memory.map_or("–".into(), |(used, _)| format!("{used:.0}"));
    let temp = s.celsius.map_or("–".into(), |c| format!("{c:.0}°"));
    let storage = s.storage.map_or("–".into(), |(used, total)| format!("{:.0}%", used / total.max(0.001) * 100.0));
    let fault = g.fault();
    let hot = |c: f32| if c >= 75.0 { fault } else if c >= 65.0 { t.amber_ink } else { t.ink };
    match size {
        Size::Small => {
            head(g, r, "memory", Some(("memory", quiet(g, cx.focused))), cx.focused);
            // "of 427 mb" where there is room, else "mb".
            let units = s.memory.map_or(vec![String::new()], |(_, total)| vec![format!("of {total:.0} mb"), "mb".into()]);
            let unit = choose_unit(g, &memory, &units, READING, room);
            foot_reading(g, r, &memory, &unit, room, t.ink);
        }
        Size::Wide => {
            // Four readings side by side, each named above.
            let columns = [("cpu", cpu, t.ink), ("memory", s.memory.map_or("–".into(), |(u, _)| format!("{u:.0} mb")), t.ink), ("temp", temp, s.celsius.map_or(t.ink, hot)), ("disk", storage, t.ink)];
            let col = room / columns.len() as f32;
            let color = quiet(g, cx.focused);
            for (i, (name, value, value_color)) in columns.iter().enumerate() {
                let x = r.x + IN + i as f32 * col;
                line(g, x, label_cy(g, r), name, ty.caption, color, col - 4.0);
                reading(g, x - 1.0, r.bottom() - 8.0, value, "", 20.0, col - 4.0, *value_color);
            }
        }
        Size::Large => {
            head(g, r, "system", Some(("memory", quiet(g, cx.focused))), cx.focused);
            let meters: [(&str, String, Option<f32>, Color); 4] = [
                ("cpu", cpu, s.cpu, t.ice),
                ("memory", s.memory.map_or("–".into(), |(u, total)| format!("{u:.0} of {total:.0} mb")), s.memory.map(|(u, total)| u / total.max(1.0)), t.ice),
                ("temp", s.celsius.map_or("–".into(), |c| format!("{c:.0} °c")), s.celsius.map(|c| ((c - 30.0) / 55.0).clamp(0.0, 1.0)), s.celsius.map_or(t.ice, |c| if c >= 65.0 { hot(c) } else { t.ice })),
                ("storage", s.storage.map_or("–".into(), |(u, total)| format!("{u:.1} of {total:.0} gb")), s.storage.map(|(u, total)| u / total.max(0.001)), t.ice),
            ];
            let top = label_bottom(g, r) + 6.0;
            let step = (r.bottom() - 6.0 - top) / meters.len() as f32;
            let name_w = meters.iter().map(|(n, ..)| g.measure(n, ty.caption)).fold(0.0, f32::max) + 8.0;
            let value_w = meters.iter().map(|(_, v, ..)| g.measure(v, ty.caption)).fold(0.0, f32::max).min(room * 0.45) + 8.0;
            for (i, (name, value, fraction, fill)) in meters.iter().enumerate() {
                let cy = top + (i as f32 + 0.5) * step;
                g.text_at(r.x + IN, cy, Align::Left, name, ty.caption, t.dim);
                let value = g.text.fit(value, &ty.caption, value_w - 8.0);
                g.text_at(r.right() - IN, cy, Align::Right, &value, ty.caption, t.ink);
                let meter = Rect::new(r.x + IN + name_w, (cy - 2.5).round(), room - name_w - value_w, 5.0);
                if meter.w > 16.0 {
                    g.meter(meter, fraction.unwrap_or(0.0), *fill);
                }
            }
        }
    }
    None
}

fn photos(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let t = g.tokens;
    let p = &cx.facts.photo;
    let Some((_, _, pic)) = &p.shown else {
        if p.list.is_empty() {
            head(g, r, "photos", Some(("photo_library", quiet(g, cx.focused))), cx.focused);
            words_any(g, r, &["no photos yet · take one with camera", "no photos yet", "none yet"], t.dim);
        } else {
            // Decoding: the tile waits, quietly.
            g.icon("photo", false, r.cx(), r.cy(), 20.0, t.faint);
        }
        return None;
    };
    let radius = cx.radius;
    let crop = |pic: &crate::photos::Picture| {
        // The middle of the picture with the tile's shape.
        let (pw, ph) = (pic.w as f32, pic.h as f32);
        let (vw, vh) = if pw / ph > r.w / r.h { (ph * r.w / r.h, ph) } else { (pw, pw * r.h / r.w) };
        [(pw - vw) * 0.5, (ph - vh) * 0.5, vw, vh]
    };
    let fade = p.fade.value().clamp(0.0, 1.0);
    if let Some(before) = p.before.as_ref().filter(|_| fade < 0.999) {
        crate::photos::draw_picture(g, r, before, crop(before), radius, 1.0);
    }
    crate::photos::draw_picture(g, r, pic, crop(pic), radius, if p.before.is_some() { fade } else { 1.0 });
    if size != Size::Small {
        if let Some(photo) = p.list.get(p.index) {
            // When it was taken, on a band of shade at its foot.
            let ty = g.ty;
            let band = Rect::new(r.x, r.bottom() - ty.caption.size - 12.0, r.w, ty.caption.size + 12.0).offset(g.ox, 0.0);
            g.canvas.rrect_clipped(band, 0.0, r.offset(g.ox, 0.0), radius, Color::BLACK, 0.42);
            let when = crate::photos::when(photo.taken, g.clock.0);
            line(g, r.x + IN, band.cy(), &when, ty.caption, Color::hex(0xf2f3f5), r.w - 2.0 * IN);
        }
    }
    None
}

fn actions(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let (t, ty) = (g.tokens, g.ty);
    let room = r.w - 2.0 * IN;
    // A bar like Spotlight's own: where search begins.
    let bar_h = (ty.caption.size + 12.0).round();
    let bar = if size == Size::Small { Rect::new(r.x + IN - 2.0, r.bottom() - 8.0 - bar_h + 2.0, room + 4.0, bar_h) } else { Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, bar_h) };
    if size == Size::Small {
        head_any(g, r, &["quick actions".into(), "actions".into()], None, cx.focused);
    }
    g.surface(bar, bar_h * 0.5, if cx.focused { t.surface3 } else { t.surface2 });
    g.icon("search", true, bar.x + 11.0, bar.cy(), 13.0, if cx.focused { t.ink } else { t.dim });
    let prompts = ["search apps, files, a sum".to_owned(), "search apps, files".to_owned(), "search".to_owned()];
    let prompt = first_fit(g, &prompts, ty.caption, bar.w - 28.0);
    g.text_at(bar.x + 21.0, bar.cy(), Align::Left, &prompt, ty.caption, t.dim);
    if size == Size::Small {
        return None;
    }
    let picks = &cx.facts.actions;
    let below = bar.bottom() + 4.0;
    if picks.is_empty() {
        let r2 = Rect::new(r.x, below, r.w, r.bottom() - below);
        words_any(g, r2, &["what you pick in search comes back here", "your picks come back here", "picks come back here"], t.dim);
        return None;
    }
    match size {
        Size::Wide => {
            // The latest picks side by side, each its symbol and name.
            let cy = r.bottom() - 8.0 - ty.caption.size * 0.5;
            let mut x = r.x + IN;
            for (icon, title) in picks.iter().take(3) {
                let w = (g.measure(title, ty.caption) + 20.0).min(r.right() - IN - x);
                if w < 40.0 {
                    break;
                }
                g.icon(icon, false, x + 6.0, cy, 13.0, t.ink);
                g.text_fit(x + 16.0, cy, title, ty.caption, t.ink, w - 18.0);
                x += w + 8.0;
            }
        }
        _ => {
            let step = ty.body.size + 7.0;
            let n = (((r.bottom() - 6.0 - below) / step).floor().max(0.0) as usize).min(picks.len());
            for (i, (icon, title)) in picks.iter().take(n).enumerate() {
                let cy = below + (i as f32 + 0.5) * step;
                g.icon(icon, false, r.x + IN + 6.0, cy, 14.0, t.dim);
                g.text_fit(r.x + IN + 18.0, cy, title, ty.body, t.ink, room - 18.0);
            }
        }
    }
    None
}

fn last_match(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let t = g.tokens;
    let Some((title, digest)) = cx.facts.last_match.clone() else {
        head(g, r, "last match", Some(("emoji_events", quiet(g, cx.focused))), cx.focused);
        words(g, r, "no match pulled yet", t.dim);
        return None;
    };
    let fault = g.fault();
    let level_color = |level: crate::wpilog::Level| match level {
        crate::wpilog::Level::Fault => fault,
        crate::wpilog::Level::Warn => t.amber_ink,
        crate::wpilog::Level::Fine => t.ink,
    };
    let room = r.w - 2.0 * IN;
    head_any(g, r, &[title, "last match".into()], Some(("emoji_events", quiet(g, cx.focused))), cx.focused);
    let value = digest.battery_low.map_or("–".into(), |v| format!("{v:.1}"));
    let color = if digest.battery_low.is_some_and(|v| v < 7.0) { g.fault() } else { t.ink };
    let ty = g.ty;
    // What stood out, as the log's summary says it: cut quietly to the room
    // there is (it is the log's to say, not the widget's).
    let highlights: Vec<(String, Color)> = if digest.highlights.is_empty() { vec![("nothing stood out".into(), t.dim)] } else { digest.highlights.iter().map(|(l, text)| (text.clone(), level_color(*l))).collect() };
    let cut = |g: &mut Gfx, list: &[(String, Color)], room: f32| -> Vec<(String, Color)> { list.iter().map(|(text, c)| (g.text.fit(text, &ty.caption, room.max(0.0)), *c)).collect() };
    match size {
        Size::Small => {
            let unit = choose_unit(g, &value, &["v low".into(), "v".into()], READING, room);
            foot_reading(g, r, &value, &unit, room, color);
        }
        Size::Wide => {
            let used = foot_reading(g, r, &value, "v low", room * 0.5, color);
            let side_room = room - used - 12.0;
            let lines = cut(g, &highlights.iter().take(2).cloned().collect::<Vec<_>>(), side_room);
            side(g, r, &lines, side_room);
        }
        Size::Large => {
            let (baseline, under) = large_lines(g, r);
            let unit = choose_unit(g, &value, &["v at its lowest".into(), "v low".into()], READING_LARGE, room);
            reading(g, r.x + IN - 1.0, baseline, &value, &unit, READING_LARGE, room, color);
            line(g, r.x + IN, under, &format!("{:.0} s logged", digest.seconds), ty.caption, t.dim, room);
            let list: Vec<(String, String, Color)> = cut(g, &highlights, room - 24.0).into_iter().map(|(text, c)| ("·".to_owned(), text, c)).collect();
            let top = rows_top(g, r);
            rows(g, r, top, &list);
        }
    }
    None
}

fn pinned(g: &mut Gfx, r: Rect, size: Size, cx: &Ctx) -> Next {
    let t = g.tokens;
    let room = r.w - 2.0 * IN;
    let Some(key) = cx.pinned else {
        head(g, r, "pinned", Some(("push_pin", quiet(g, cx.focused))), cx.focused);
        words(g, r, "p in tables pins a value", t.dim);
        return None;
    };
    let (table, name) = key.rsplit_once('/').unwrap_or(("", key));
    // Its name as the robot publishes it ("LastMs" is "last ms").
    let spoken = name.chars().fold(String::new(), |mut out, c| {
        if c.is_uppercase() && out.chars().last().is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
        out
    });
    head_any(g, r, &[spoken, "pinned".into()], Some(("push_pin", quiet(g, cx.focused))), cx.focused);
    let value = cx.robot.get(key).map_or("–".into(), |v| v.display());
    let table = table.to_lowercase();
    match size {
        Size::Small => {
            foot_reading(g, r, &value, "", room, t.ink);
        }
        Size::Wide => {
            let used = foot_reading(g, r, &value, "", room * 0.6, t.ink);
            let last = table.rsplit('/').next().unwrap_or(&table).to_owned();
            side_any(g, r, &[(vec![table.clone(), last], t.dim)], room - used - 12.0);
        }
        Size::Large => {
            let (baseline, under) = large_lines(g, r);
            reading(g, r.x + IN - 1.0, baseline, &value, "", READING_LARGE, room, t.ink);
            let last = table.rsplit('/').next().unwrap_or(&table).to_owned();
            let under_line = first_fit(g, &[format!("in {table}"), format!("in {last}")], g.ty.caption, room);
            g.text_at(r.x + IN, under, Align::Left, &under_line, g.ty.caption, t.dim);
        }
    }
    None
}
