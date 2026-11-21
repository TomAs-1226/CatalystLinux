//! The Reminders app: Today, Upcoming and Done side by side (←→), quick add
//! with what it read shown as it is typed, and the pit routine.

use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::parse::{self, Parsed};
use super::plan::{clock_text, when_text, Here, Now, LEAD, MORNING};
use super::{now, with, Link, List, Page, Reminder, Repeat, Row, When};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

const PAGES: [Page; 3] = [Page::Today, Page::Upcoming, Page::Done];

/// s pressed again within this long puts it off for longer.
const SNOOZE_AGAIN: Duration = Duration::from_secs(4);

/// The reminders on a list now, with the schedule as the robot has it.
fn rows(page: Page, filter: Option<List>, robot: &Robot, h24: bool) -> Vec<Row> {
    with(|s| {
        s.sync(&robot.schedule);
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        s.book.rows(page, filter, &n, h24)
    })
}

/// Work on the book as of now.
fn edit<T>(robot: &Robot, f: impl FnOnce(&mut super::Book, &Now) -> T) -> T {
    with(|s| {
        s.sync(&robot.schedule);
        let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
        f(&mut s.book, &n)
    })
}

/// Room before a row's words for its tick: in Bezel's mono voice a space is
/// wider, so fewer of them (as the pit checklist).
fn indent(g: &Gfx) -> &'static str {
    if g.theme == crate::look::Theme::Bezel { "   " } else { "     " }
}

/// What a snooze can be: a few waits, the next match, tomorrow morning.
fn snoozes(n: &Now, h24: bool) -> Vec<(i64, String)> {
    let mut all = vec![(n.at + 600, "10 min".to_owned()), (n.at + 1800, "30 min".to_owned()), (n.at + 3600, "1 hour".to_owned())];
    if let Some(m) = n.view.and_then(|v| v.next(n.at)) {
        if let Some(at) = m.time.map(|t| t - LEAD).filter(|at| *at > n.at + 3600) {
            all.push((at, format!("till {} is close", m.label)));
        }
    }
    let morning = n.today().succ_opt().and_then(|d| d.and_hms_opt(MORNING, 0, 0));
    if let Some(morning) = morning {
        all.push((n.zone.instant(morning), format!("till tmrw {}", clock_text(morning.time(), h24))));
    }
    all
}

/// What quick add read: when it comes, as words.
fn describe(p: &Parsed, n: &Now, h24: bool) -> String {
    let repeat = match p.repeat {
        Repeat::Daily => " · every day",
        Repeat::Weekdays => " · weekdays",
        Repeat::Weekly => " · every week",
        _ => "",
    };
    match (&p.when, p.repeat) {
        (When::Whenever, _) => "no time: on today's list".into(),
        (When::In(at), Repeat::Every(every)) => format!("every {} · first {}", parse::length_text(every), when_text(*at, n, h24)),
        (When::In(at), _) => when_text(*at, n, h24),
        (When::At(at), Repeat::Every(every)) => format!("every {} · first {}", parse::length_text(every), when_text(n.zone.instant(*at), n, h24)),
        (When::At(at), _) => format!("{}{repeat}", when_text(n.zone.instant(*at), n, h24)),
        (When::Match(cue, of), _) => {
            let mut r = Reminder { when: p.when.clone(), ..Reminder::blank(0, "", n.at) };
            super::bind(&mut r, n);
            let named = matches!(of, super::Of::One { .. });
            let cue = parse::cue_text(*cue, of);
            match (r.current(n), n.view) {
                // The match it comes on, unless the cue already names it.
                (Some(o), _) => {
                    let label = o.label.filter(|_| !named).map(|l| format!("{l} ")).unwrap_or_default();
                    format!("{cue} · {label}{}{}", if o.estimated { "~" } else { "" }, when_text(o.at, n, h24))
                }
                (None, None) => format!("{cue} · once there is a schedule"),
                (None, Some(_)) => format!("{cue} · not on the schedule yet"),
            }
        }
    }
}

/// Room either side of a list's name in the header.
const HEAD_PAD: f32 = 7.0;

/// The header's words: each list with how many are on it (Today's and
/// Upcoming's), and the one list shown when Tab chose one. At a larger text
/// size the counts go first, then "only".
fn header(g: &mut Gfx, counts: [usize; 2], filter: Option<List>) -> (Vec<String>, Option<String>) {
    let caption = g.ty.caption;
    let with = |n: usize, name: &str| if n == 0 { name.to_owned() } else { format!("{name} {n}") };
    let tries = [
        (vec![with(counts[0], "today"), with(counts[1], "upcoming"), "done".to_owned()], filter.map(|l| format!("{} only", l.name()))),
        (vec!["today".to_owned(), "upcoming".to_owned(), "done".to_owned()], filter.map(|l| format!("{} only", l.name()))),
        (vec!["today".to_owned(), "upcoming".to_owned(), "done".to_owned()], filter.map(|l| l.name().to_owned())),
    ];
    let room = W - 2.0 * PAD - 4.0;
    let last = tries.len() - 1;
    for (k, (labels, only)) in tries.into_iter().enumerate() {
        let w: f32 = labels.iter().map(|l| g.measure(l, caption) + 2.0 * HEAD_PAD + 2.0).sum::<f32>() + only.as_ref().map_or(0.0, |o| g.measure(o, caption) + 10.0);
        if w <= room || k == last {
            return (labels, only);
        }
    }
    unreachable!("the last try is always taken")
}

/// The icon for what a reminder hangs on: a match, a time, nothing.
fn kind_icon(when: &When) -> &'static str {
    match when {
        When::Match(..) => "event",
        When::Whenever => "notifications",
        _ => "schedule",
    }
}

// ------------------------------------------------------------- the app ---

/// Today, Upcoming and Done; Enter ticks one off.
pub struct Reminders {
    page: usize,
    turn: Animated,
    lists: [ListState; 3],
    /// One list only, or both.
    filter: Option<List>,
    /// d pressed once on this one: again deletes it.
    deleting: Option<u64>,
    /// s pressed on this one, the snooze it is on, and when.
    snoozing: Option<(u64, usize, Instant)>,
    /// A row gone from its list (ticked off, deleted): the rows below close
    /// up over it. Its page, its place, and how far they have still to rise.
    closing: Option<(usize, usize, Animated)>,
    /// The minute and the book's edits last drawn at.
    drawn: (i64, u64),
}

impl Default for Reminders {
    fn default() -> Self {
        Self::new()
    }
}

impl Reminders {
    pub fn new() -> Self {
        Reminders {
            page: 0,
            turn: Animated::new(0.0, role::page()).with_rest(0.002),
            lists: Default::default(),
            filter: None,
            deleting: None,
            snoozing: None,
            closing: None,
            drawn: (0, u64::MAX),
        }
    }

    /// Rows above the reminders on a page: Today and Upcoming start with "New reminder".
    fn first(page: usize) -> usize {
        (PAGES[page] != Page::Done) as usize
    }

    fn go(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
        self.turn.set(self.page as f32);
        self.deleting = None;
    }

    /// Put the focus on a reminder, wherever it is (turning to its list).
    fn show(&mut self, id: u64, robot: &Robot, h24: bool) -> bool {
        for (p, page) in PAGES.iter().enumerate() {
            if let Some(i) = rows(*page, self.filter, robot, h24).iter().position(|r| r.id == id) {
                if p != self.page {
                    self.go(p);
                }
                self.lists[p].focus = i + Self::first(p);
                return true;
            }
        }
        false
    }

    /// Something new to show: a reminder just said, added, or asked for.
    fn place(&mut self, robot: &Robot, h24: bool) {
        let wanted = with(|s| s.show.take().or_else(|| (!s.ringing.is_empty()).then(|| std::mem::take(&mut s.ringing)[0])));
        if let Some(id) = wanted {
            if !self.show(id, robot, h24) {
                // Filtered out: show every list.
                self.filter = None;
                self.show(id, robot, h24);
            }
        }
    }

    /// The row is gone from where it was: close up over it.
    fn closed(&mut self, page: usize, at: usize) {
        let mut shift = Animated::new(1.0, crate::gfx::spatial::default()).with_rest(0.01);
        shift.set(0.0);
        self.closing = Some((page, at, shift));
    }
}

impl Screen for Reminders {
    fn title(&self) -> &str {
        "Reminders"
    }

    fn resume(&mut self) {
        self.drawn.1 = u64::MAX;
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if with(|s| s.show.is_some() || !s.ringing.is_empty()) {
            self.place(cx.robot, cx.settings.clock24);
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let h24 = cx.settings.clock24;
        let page = self.page;
        let first = Self::first(page);
        let shown = rows(PAGES[page], self.filter, cx.robot, h24);
        let at = self.lists[page].focus;
        let focused = at.checked_sub(first).and_then(|i| shown.get(i)).cloned();
        let armed = self.deleting.take();
        // After a change, the focus follows the reminder if it stays here.
        let filter = self.filter;
        let stays = |id: u64, robot: &Robot| rows(PAGES[page], filter, robot, h24).iter().position(|r| r.id == id);
        match key {
            Key::Back => return Nav::Pop,
            Key::Left if page > 0 => self.go(page - 1),
            Key::Right if page + 1 < PAGES.len() => self.go(page + 1),
            Key::Tab => {
                self.filter = match self.filter {
                    None => Some(List::Pit),
                    Some(List::Pit) => Some(List::Personal),
                    Some(List::Personal) => None,
                };
                self.lists = Default::default();
                *cx.say = Some(self.filter.map_or("both lists".to_owned(), |l| format!("{} list", l.name())));
            }
            Key::Char('n') => return Nav::Push(Box::new(Compose::new())),
            Key::Char('p') => return Nav::Push(Box::new(Routine::new())),
            Key::Enter if focused.is_none() && first == 1 => return Nav::Push(Box::new(Compose::new())),
            Key::Enter => {
                if let Some(row) = focused {
                    let said = edit(cx.robot, |book, n| book.tick(row.id, n, h24));
                    with(|s| s.ringing.retain(|id| *id != row.id));
                    match stays(row.id, cx.robot) {
                        Some(i) => self.lists[page].focus = i + first,
                        None => self.closed(page, at),
                    }
                    *cx.say = said.map(|s| if s == "done" { format!("done · {}", row.text) } else { s });
                }
            }
            Key::Char('s') if !matches!(PAGES[page], Page::Done) => {
                if let Some(row) = focused {
                    let again = self.snoozing.filter(|(id, _, when)| *id == row.id && when.elapsed() < SNOOZE_AGAIN).map(|(_, k, _)| k);
                    let (label, choice) = edit(cx.robot, |book, n| {
                        let choices = snoozes(n, h24);
                        let choice = again.map_or(0, |k| (k + 1) % choices.len());
                        let (until, label) = choices[choice].clone();
                        book.snooze(row.id, until);
                        (label, choice)
                    });
                    self.snoozing = Some((row.id, choice, Instant::now()));
                    with(|s| s.ringing.retain(|id| *id != row.id));
                    match stays(row.id, cx.robot) {
                        Some(i) => self.lists[page].focus = i + first,
                        None => self.closed(page, at),
                    }
                    *cx.say = Some(format!("snoozed {label}"));
                }
            }
            Key::Char('d') => {
                if let Some(row) = focused {
                    if armed == Some(row.id) {
                        edit(cx.robot, |book, _| book.delete(row.id));
                        self.closed(page, at);
                        *cx.say = Some(format!("deleted {}", row.text));
                    } else {
                        self.deleting = Some(row.id);
                        *cx.say = Some("d again deletes it".into());
                    }
                }
            }
            Key::Char('e') => {
                if let Some(row) = focused {
                    return Nav::Push(Box::new(Compose::edit(row.id, cx.robot)));
                }
            }
            Key::Char('r') => {
                if let Some(row) = focused {
                    *cx.say = edit(cx.robot, |book, n| book.cycle_repeat(row.id, n));
                    if let Some(i) = stays(row.id, cx.robot) {
                        self.lists[page].focus = i + first;
                    }
                }
            }
            Key::Char('l') => {
                if let Some(row) = focused {
                    let list = edit(cx.robot, |book, _| book.switch_list(row.id));
                    *cx.say = list.map(|l| format!("on the {} list", l.name()));
                    if self.filter.is_some() {
                        self.closed(page, at);
                    }
                }
            }
            Key::Char('o') => {
                if let Some(row) = focused {
                    match super::opens(&row.link) {
                        Ok(screen) => return Nav::Push(screen),
                        Err(why) => *cx.say = Some(why.into()),
                    }
                }
            }
            other => crate::screens::list_key(&mut self.lists[page], other, shown.len() + first),
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.turn.tick(dt);
        if let Some((.., shift)) = &mut self.closing {
            let rising = shift.tick(dt);
            moving |= rising;
            if !rising {
                self.closing = None;
            }
        }
        // The times move with the clock, and what the shell said changes the lists.
        let stamp = (now() / 60, with(|s| s.book.edits));
        let changed = stamp != self.drawn;
        self.drawn = stamp;
        moving || changed
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let h24 = g.clock.0;
        self.place(robot, h24);
        if g.calm {
            self.turn.jump(self.page as f32);
            self.closing = None;
        }
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let all: Vec<Vec<Row>> = PAGES.iter().map(|p| rows(*p, self.filter, robot, h24)).collect();

        // The three lists, the one in view on a tonal pill that glides with the page.
        let head_h = ty.caption.size + 6.0;
        let (counts, only) = header(g, [all[0].len(), all[1].len()], self.filter);
        let mut x = PAD;
        let mut slots = Vec::new();
        for label in &counts {
            let w = g.measure(label, ty.caption) + 2.0 * HEAD_PAD;
            slots.push(Rect::new(x, top, w, head_h));
            x += w + 2.0;
        }
        let along = self.turn.value().clamp(0.0, (PAGES.len() - 1) as f32);
        let (k, f) = (along.floor() as usize, along.fract());
        let (a, b) = (slots[k], slots[(k + 1).min(slots.len() - 1)]);
        let pill = Rect::new(a.x + (b.x - a.x) * f, top, a.w + (b.w - a.w) * f, head_h);
        g.surface(pill, head_h * 0.5, t.surface2);
        for (i, (label, slot)) in counts.iter().zip(&slots).enumerate() {
            let near = (along - i as f32).abs() < 0.5;
            g.text_at(slot.cx(), slot.cy(), Align::Center, label, ty.caption, if near { t.ink } else { t.dim });
        }
        if let Some(only) = only {
            g.text_at(W - PAD - 4.0, top + head_h * 0.5, Align::Right, &only, ty.caption, t.ink);
        }

        let list_top = top + head_h + 3.0;
        let bottom = H - PAD - g.hint_h();
        let base = g.ox;
        for (p, rows) in all.iter().enumerate() {
            let ox = ((p as f32 - self.turn.value()) * W).round();
            if ox.abs() >= W {
                continue;
            }
            g.ox = base + ox;
            self.draw_page(g, p, rows, list_top, bottom);
        }
        g.ox = base;

        // The keys for what is focused.
        let page = PAGES[self.page];
        let first = Self::first(self.page);
        let focused = self.lists[self.page].focus.checked_sub(first).and_then(|i| all[self.page].get(i));
        match (page, focused) {
            (Page::Done, Some(_)) => g.hint_any(&["enter: put back · d: delete · ←: lists", "enter: put back · d: delete"]),
            (Page::Done, None) => g.hint("←: today and upcoming"),
            (_, None) => g.hint_any(&["enter: new · p: pit routine · tab: lists · ←→", "enter: new · p: pit routine · ←→", "n: new · p: routine"]),
            (_, Some(row)) => {
                let open = match row.link {
                    Link::Note(_) => Some("o: open note"),
                    Link::Checklist => Some("o: checklist"),
                    Link::None => None,
                };
                match open {
                    Some(open) => g.hint_any(&[&format!("{open} · enter: done · s: snooze · e: edit"), &format!("{open} · enter: done · s: snooze"), open]),
                    None => g.hint_any(&["enter: done · s: snooze · e: edit · d: delete", "enter: done · s: snooze · d: delete", "enter: done · s: snooze"]),
                }
            }
        }
    }
}

impl Reminders {
    fn draw_page(&mut self, g: &mut Gfx, p: usize, rows: &[Row], top: f32, bottom: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let row_h = g.row_h();
        let first = Self::first(p);
        let len = rows.len() + first;
        let bezel = g.theme == crate::look::Theme::Bezel;
        let (name_style, value_style) = if bezel { (ty.label, ty.body) } else { (ty.body, ty.label) };
        let radius = if bezel { g.radius(7.0).min(row_h * 0.5) } else { row_h * 0.5 };
        // In view, the focus is drawn; turning between lists, it rides along.
        let current = p == self.page;
        let settled = g.ox.abs() < 0.5;
        let shift = match &self.closing {
            Some((page, at, shift)) if *page == p => Some((*at, shift.value() * (row_h + 3.0))),
            _ => None,
        };
        let indent = indent(g);
        let indent_w = g.measure(indent, name_style);
        for (i, y) in self.lists[p].layout(g, len, top, row_h, 3.0, bottom + 2.0) {
            let y = match shift {
                Some((at, by)) if i >= at => y + by,
                _ => y,
            };
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = current && i == self.lists[p].focus;
            if focused && !settled {
                let ox = g.ox;
                g.ox = 0.0;
                g.focus_ring_on(r.offset(ox, 0.0), radius, ox);
                g.ox = ox;
            }
            let ring = focused && settled;
            let Some(row) = i.checked_sub(first).and_then(|k| rows.get(k)) else {
                g.row(r, &format!("{indent}New reminder"), "n", t.dim, ring);
                g.icon("notification_add", ring, r.x + 15.0, r.cy(), 14.0, t.dim);
                continue;
            };
            let value_color = if row.due { t.amber_ink } else if row.waiting { t.faint } else { t.dim };
            // Marks beside the time: it repeats, it opens a note or the checklist.
            let marks: Vec<&str> = [row.repeats.then_some("sync"), match row.link {
                Link::Note(_) => Some("sticky_note_2"),
                Link::Checklist => Some("checklist"),
                Link::None => None,
            }]
            .into_iter()
            .flatten()
            .collect();
            let value_w = g.measure(&row.when, value_style);
            let marks_w = marks.len() as f32 * 14.0;
            let room = (r.w - 30.0 - value_w - marks_w - indent_w).max(40.0);
            let text = g.text.fit(&row.text, &name_style, room);
            g.row(r, &format!("{indent}{text}"), &row.when, value_color, ring);
            let (icon, filled, ink) = if row.done {
                ("check_circle", true, g.good())
            } else if row.due {
                ("notifications", true, t.amber_ink)
            } else {
                ("radio_button_unchecked", false, t.faint)
            };
            g.icon(icon, filled, r.x + 15.0, r.cy(), 14.0, ink);
            for (k, mark) in marks.iter().enumerate() {
                g.icon(mark, false, r.right() - 10.0 - value_w - 9.0 - k as f32 * 14.0, r.cy(), 11.0, t.faint);
            }
        }
        self.lists[p].scrollbar(g, len, top, bottom);
        // Nothing on the list: what it is for.
        if rows.is_empty() {
            let (line, hint) = match PAGES[p] {
                Page::Today => ("nothing due today", "try: swap battery in 10 min"),
                Page::Upcoming => ("nothing coming up", "try: before Q42 charge spare"),
                Page::Done => ("nothing ticked off yet", ""),
            };
            let room_top = if first == 1 { top + row_h + 3.0 } else { top };
            let cy = (room_top + bottom) * 0.5;
            if PAGES[p] == Page::Done {
                g.icon("check_circle", false, W * 0.5, cy - 11.0, 22.0, t.faint);
                g.text_at(W * 0.5, cy + 12.0, Align::Center, line, ty.caption, t.dim);
            } else {
                g.text_at(W * 0.5, cy - 7.0, Align::Center, line, ty.caption, t.dim);
                g.text_at(W * 0.5, cy + 8.0, Align::Center, hint, ty.caption, t.faint);
            }
        }
    }
}

// --------------------------------------------------------- quick add ---

enum Mode {
    New,
    Edit(u64),
    /// A note's reminder: its words are the note's title; only when is typed.
    Note { file: String, title: String, existing: Option<u64> },
    /// A line of the pit routine (a new one, or the one at this place).
    Line(Option<usize>),
}

/// Quick add: a line typed as it would be said, and what it was read as,
/// shown as it is typed.
pub struct Compose {
    field: Field,
    mode: Mode,
    /// What the line reads as just now.
    read: Result<Vec<Parsed>, String>,
    /// The line last read, so it is read again only when it changes.
    seen: Option<String>,
}

impl Compose {
    pub fn new() -> Self {
        Self::with(Field::new("remind me · swap battery in 10 min", "", 90), Mode::New)
    }

    fn with(field: Field, mode: Mode) -> Self {
        Compose { field, mode, read: Err(String::new()), seen: None }
    }

    /// A reminder typed again, from how it reads.
    fn edit(id: u64, robot: &Robot) -> Self {
        let text = edit(robot, |book, n| book.get(id).map(|r| parse::phrase(r, n))).unwrap_or_default();
        Self::with(Field::new("change it · what, then when", &text, 90), Mode::Edit(id))
    }

    /// A note's reminder: when, for the note in `file` titled `title`.
    pub fn for_note(file: &str, title: &str) -> Self {
        let existing = with(|s| {
            let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
            s.book.for_note(file).map(|r| (r.id, parse::when_text(r, &n)))
        });
        let (id, typed) = existing.map_or((None, String::new()), |(id, w)| (Some(id), w));
        Self::with(Field::new("remind me · in 30 min, 8am, before Q42", &typed, 60), Mode::Note { file: file.to_owned(), title: title.to_owned(), existing: id })
    }

    /// A line of the pit routine.
    fn line(at: Option<usize>, text: &str) -> Self {
        Self::with(Field::new("a line · after each match: swap battery", text, 90), Mode::Line(at))
    }

    fn reread(&mut self, robot: &Robot) {
        if self.seen.as_deref() == Some(self.field.text.as_str()) {
            return;
        }
        self.seen = Some(self.field.text.clone());
        let text = self.field.text.clone();
        let title = match &self.mode {
            Mode::Note { title, .. } => Some(title.clone()),
            _ => None,
        };
        self.read = with(|s| {
            s.sync(&robot.schedule);
            let n = Now { at: now(), zone: &Here, view: s.view.as_ref() };
            match &title {
                Some(title) => parse::when_only(&text, &n).map(|p| vec![Parsed { text: title.clone(), link: Link::None, ..p }]).ok_or_else(|| "say when: in 30 min, 8am, before Q42".to_owned()),
                None => parse::parse(&text, &n),
            }
        });
    }

    /// Enter: add it, change it, or keep the line. What to say, and whether it is done.
    fn finish(&mut self, robot: &Robot, h24: bool) -> (Option<String>, bool) {
        self.seen = None;
        self.reread(robot);
        let typed = self.field.text.trim().to_owned();
        if let Mode::Note { existing, .. } = &self.mode {
            if typed.is_empty() {
                let existing = *existing;
                if let Some(id) = existing {
                    edit(robot, |book, _| book.delete(id));
                    return (Some("reminder off".into()), true);
                }
                return (None, true);
            }
        }
        let items = match &self.read {
            Ok(items) => items.clone(),
            Err(why) => return (Some(if why.is_empty() { parse::NO_TEXT.to_owned() } else { why.clone() }), false),
        };
        match &self.mode {
            Mode::New => {
                let (count, first) = (items.len(), items[0].text.clone());
                let shown = edit(robot, |book, n| items.into_iter().map(|p| book.add(p, n)).last());
                with(|s| s.show = shown);
                (Some(if count == 1 { format!("added · {first}") } else { format!("added {count}") }), true)
            }
            Mode::Edit(id) => {
                let id = *id;
                let mut items = items.into_iter();
                edit(robot, |book, n| {
                    if let Some(p) = items.next() {
                        book.replace(id, p, n);
                    }
                    for p in items {
                        book.add(p, n);
                    }
                });
                with(|s| s.show = Some(id));
                (Some("changed".into()), true)
            }
            Mode::Note { file, title, existing } => {
                let (file, title, existing) = (file.clone(), title.clone(), *existing);
                let p = items.into_iter().next().map(|p| Parsed { text: title, link: Link::Note(file), ..p });
                let said = edit(robot, |book, n| {
                    let p = p?;
                    let said = describe(&p, n, h24);
                    match existing {
                        Some(id) => {
                            book.replace(id, p, n);
                        }
                        None => {
                            book.add(p, n);
                        }
                    }
                    Some(said)
                });
                (said.map(|s| format!("reminder · {s}")), true)
            }
            Mode::Line(at) => {
                let mut lines = load_routine();
                match at {
                    Some(i) if *i < lines.len() => lines[*i] = typed,
                    _ => lines.push(typed),
                }
                save_routine(&lines);
                (None, true)
            }
        }
    }
}

impl Default for Compose {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for Compose {
    fn title(&self) -> &str {
        // Short enough to sit beside the island.
        match self.mode {
            Mode::Edit(_) => "Change it",
            Mode::Note { .. } => "Remind me",
            Mode::Line(_) => "Pit routine",
            Mode::New => "Quick add",
        }
    }

    fn wants_text(&self) -> bool {
        true
    }

    /// A passing page: Reminders is kept among the recent apps, not it.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match self.field.key(key, |c| !c.is_control()) {
            Typed::Done(_) => {
                let (said, done) = self.finish(cx.robot, cx.settings.clock24);
                *cx.say = said;
                if done {
                    return Nav::Pop;
                }
            }
            Typed::Cancelled => return Nav::Pop,
            Typed::More => self.reread(cx.robot),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        self.reread(robot);
        let t = g.tokens;
        let ty = g.ty;
        let h24 = g.clock.0;
        self.field.draw_at(g, true);
        let card = Rect::new(PAD, g.top() + Field::height(g) + 6.0, W - 2.0 * PAD, H - PAD - g.hint_h() - 4.0 - (g.top() + Field::height(g) + 6.0));
        g.surface(card, g.radius(12.0), t.surface1);
        let line = |k: f32| card.y + 11.0 + k * (ty.caption.size + 5.0);
        let room = card.w - 40.0;
        let x = card.x + 30.0;
        let typed_nothing = self.field.text.trim().is_empty();
        match (&self.read, &self.mode) {
            (_, Mode::Note { title, existing, .. }) if typed_nothing => {
                g.icon("sticky_note_2", false, card.x + 15.0, line(0.0), 13.0, t.dim);
                g.text_fit(x, line(0.0), title, ty.body, t.ink, room);
                let say = if existing.is_some() { "left empty, enter takes it off" } else { "when should it come back?" };
                g.text_fit(x, line(1.1), say, ty.caption, t.dim, room);
                g.text_fit(x, line(2.1), "after each match · tomorrow 8am", ty.caption, t.faint, room);
            }
            (Err(_), _) if typed_nothing => {
                // What can be typed.
                g.icon("notification_add", false, card.x + 15.0, line(0.0), 13.0, t.dim);
                g.text_fit(x, line(0.0), "what, then when", ty.caption, t.dim, room);
                for (k, example) in ["tomorrow 8am check bumpers", "after each match: swap battery"].iter().enumerate() {
                    g.text_fit(x, line(1.1 + k as f32), example, ty.caption, t.faint, room);
                }
            }
            (Err(why), _) => {
                g.icon("notification_add", false, card.x + 15.0, line(0.0), 13.0, t.dim);
                g.text_fit(x, line(0.0), why, ty.caption, t.amber_ink, room);
                g.text_fit(x, line(1.1), "a time: in 10 min, 8am, every day", ty.caption, t.faint, room);
                g.text_fit(x, line(2.1), "a match: before Q42, after Q31", ty.caption, t.faint, room);
            }
            (Ok(items), mode) => {
                let n_view = with(|s| s.view.clone());
                let n = Now { at: now(), zone: &Here, view: n_view.as_ref() };
                let p = &items[0];
                let icon = if matches!(mode, Mode::Note { .. }) { "sticky_note_2" } else { kind_icon(&p.when) };
                let good = g.good();
                g.icon(icon, true, card.x + 15.0, line(0.0) + 1.0, 13.0, good);
                let head = if items.len() > 1 { format!("{} reminders", items.len()) } else { p.text.clone() };
                g.text_fit(x, line(0.0) + 1.0, &head, ty.body, t.ink, room);
                g.text_fit(x, line(1.1), &describe(p, &n, h24), ty.caption, t.dim, room);
                let below = if items.len() > 1 {
                    items.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(", ")
                } else {
                    let list = p.list.unwrap_or(if n.event_today() { List::Pit } else { List::Personal });
                    match (&p.link, mode) {
                        (_, Mode::Note { .. }) => "opens the note".to_owned(),
                        (Link::Checklist, _) => format!("{} list · opens the checklist", list.name()),
                        _ => format!("{} list", list.name()),
                    }
                };
                g.text_fit(x, line(2.1), &below, ty.caption, t.faint, room);
            }
        }
        g.hint(match self.mode {
            Mode::New => "enter: add · esc: cancel",
            Mode::Edit(_) => "enter: change it · esc: leave it",
            Mode::Note { .. } => "enter: set · esc: cancel",
            Mode::Line(_) => "enter: keep · esc: cancel",
        });
    }
}

// --------------------------------------------------------- pit routine ---

/// What a pit does every match, ready to put on the lists in one go.
const ROUTINE: [&str; 3] = [
    "after each match: swap battery, check bumpers, download logs",
    "20 min before each match: run the pit checklist",
    "between matches: charge the spare batteries",
];

fn routine_path() -> std::path::PathBuf {
    crate::config_dir().join("pit-routine")
}

pub fn load_routine() -> Vec<String> {
    match std::fs::read_to_string(routine_path()) {
        Ok(text) => text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect(),
        Err(_) => ROUTINE.iter().map(|l| (*l).to_owned()).collect(),
    }
}

fn save_routine(lines: &[String]) {
    let path = routine_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    crate::sys::write_atomic(&path, (lines.join("\n") + "\n").as_bytes()).ok();
}

/// Put the routine's reminders on the lists, but none already there. How many.
pub fn apply_routine(lines: &[String], robot: &Robot) -> usize {
    edit(robot, |book, n| {
        let mut added = 0;
        for line in lines {
            for p in parse::parse(line, n).unwrap_or_default() {
                if !book.has_like(&p) {
                    book.add(p, n);
                    added += 1;
                }
            }
        }
        added
    })
}

/// A line of the routine as its row shows it: the things to do, and when.
fn routine_row(line: &str, n: &Now) -> (String, String) {
    match parse::parse(line, n) {
        Ok(items) => {
            let what = items.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(", ");
            let when = match &items[0].when {
                When::Match(super::Cue::Before(LEAD), super::Of::Each) => "before each".to_owned(),
                When::Match(super::Cue::Before(lead), super::Of::Each) => format!("{} before", parse::length_text(*lead)),
                When::Match(super::Cue::Queue, _) => "at queueing".to_owned(),
                When::Match(super::Cue::After, super::Of::Each) => "after each".to_owned(),
                When::Match(super::Cue::Between, _) => "between".to_owned(),
                When::Match(cue, of) => parse::cue_text(*cue, of),
                _ => "once".to_owned(),
            };
            (what, when)
        }
        Err(_) => (line.to_owned(), "–".to_owned()),
    }
}

/// The pit routine: Enter on the first row puts it all on the lists.
pub struct Routine {
    lines: Vec<String>,
    list: ListState,
    deleting: Option<usize>,
}

impl Routine {
    pub fn new() -> Self {
        Routine { lines: load_routine(), list: ListState::default(), deleting: None }
    }
}

impl Default for Routine {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen for Routine {
    fn title(&self) -> &str {
        "Pit routine"
    }

    fn resume(&mut self) {
        self.lines = load_routine();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let at = self.list.focus;
        let armed = self.deleting.take();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if at == 0 => {
                let added = apply_routine(&self.lines, cx.robot);
                *cx.say = Some(match added {
                    0 => "already on the lists".into(),
                    1 => "1 reminder added".into(),
                    k => format!("{k} reminders added"),
                });
                return if added > 0 { Nav::Pop } else { Nav::Stay };
            }
            Key::Enter => {
                if let Some(line) = self.lines.get(at - 1) {
                    return Nav::Push(Box::new(Compose::line(Some(at - 1), line)));
                }
            }
            Key::Char('n') => return Nav::Push(Box::new(Compose::line(None, ""))),
            Key::Char('d') if at >= 1 && at <= self.lines.len() => {
                if armed == Some(at) {
                    self.lines.remove(at - 1);
                    save_routine(&self.lines);
                    *cx.say = Some("taken off the routine".into());
                } else {
                    self.deleting = Some(at);
                    *cx.say = Some("d again takes it off".into());
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.lines.len() + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let row_h = g.row_h();
        let bottom = H - PAD - g.hint_h();
        let view = with(|s| {
            s.sync(&robot.schedule);
            s.view.clone()
        });
        let n = Now { at: now(), zone: &Here, view: view.as_ref() };
        let rows: Vec<(String, String)> = self.lines.iter().map(|l| routine_row(l, &n)).collect();
        let count: usize = self.lines.iter().map(|l| parse::parse(l, &n).map_or(0, |v| v.len())).sum();
        for (i, y) in self.list.layout(g, rows.len() + 1, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if i == 0 {
                let good = g.good();
                g.row(r, "Put it all on the lists", &format!("{count} reminders"), good, focused);
                continue;
            }
            let (what, when) = &rows[i - 1];
            g.row(r, what, when, t.dim, focused);
        }
        self.list.scrollbar(g, rows.len() + 1, top, bottom);
        if self.list.focus == 0 {
            g.hint_any(&["enter: add them all · n: a new line", "enter: add them all"]);
        } else {
            g.hint_any(&["enter: edit · d: take off · n: new line", "enter: edit · d: take off"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::TextSize;
    use crate::shell::Settings;

    fn cx_parts() -> (Robot, Settings, Option<String>) {
        crate::shell::tests::sandbox();
        (Robot::demo(), Settings::default(), None)
    }

    fn press(screen: &mut dyn Screen, keys: &[Key], robot: &mut Robot, settings: &mut Settings, say: &mut Option<String>) -> Vec<Nav> {
        keys.iter().map(|k| screen.key(*k, &mut Cx { robot, settings, say, at_rest: true })).collect()
    }

    fn typed(text: &str) -> Vec<Key> {
        text.chars().map(Key::Char).chain([Key::Enter]).collect()
    }

    #[test]
    fn quick_add_adds_what_was_typed_and_the_app_shows_it() {
        let (mut robot, mut settings, mut say) = cx_parts();
        with(|s| s.book = super::super::Book::memory());
        let mut compose = Compose::new();
        let navs = press(&mut compose, &typed("swap battery in 10 min"), &mut robot, &mut settings, &mut say);
        assert!(matches!(navs.last(), Some(Nav::Pop)));
        assert_eq!(say.as_deref(), Some("added · swap battery"));
        let mut app = Reminders::new();
        let mut g = Gfx::new(bezel::Tokens::dark());
        app.draw(&mut g, &robot);
        assert_eq!(app.lists[0].focus, 1, "the one just added has the focus");
        // Enter: done, and on the Done list.
        press(&mut app, &[Key::Enter], &mut robot, &mut settings, &mut say);
        assert_eq!(say.as_deref(), Some("done · swap battery"));
        assert!(app.closing.is_some(), "the rows close up over it");
        assert!(rows(Page::Today, None, &robot, true).is_empty());
        assert_eq!(rows(Page::Done, None, &robot, true).len(), 1);
    }

    #[test]
    fn s_snoozes_and_again_puts_it_off_longer() {
        let (mut robot, mut settings, mut say) = cx_parts();
        with(|s| {
            s.book = super::super::Book::memory();
            let n = Now { at: now(), zone: &Here, view: None };
            s.book.add(Parsed { text: "stretch".into(), when: When::In(now() - 5), repeat: Repeat::Never, list: None, link: Link::None }, &n);
        });
        let mut app = Reminders::new();
        let mut g = Gfx::new(bezel::Tokens::dark());
        app.draw(&mut g, &robot);
        press(&mut app, &[Key::Down, Key::Char('s')], &mut robot, &mut settings, &mut say);
        assert_eq!(say.as_deref(), Some("snoozed 10 min"));
        press(&mut app, &[Key::Char('s')], &mut robot, &mut settings, &mut say);
        assert_eq!(say.as_deref(), Some("snoozed 30 min"));
        let until = with(|s| s.book.reminders[0].snooze.unwrap());
        assert!((until - now() - 1800).abs() <= 2);
    }

    #[test]
    fn the_routine_goes_on_in_one_go_and_not_twice() {
        let (robot, ..) = cx_parts();
        with(|s| s.book = super::super::Book::memory());
        let lines: Vec<String> = ROUTINE.iter().map(|l| (*l).to_owned()).collect();
        assert_eq!(apply_routine(&lines, &robot), 5);
        assert_eq!(apply_routine(&lines, &robot), 0, "already on");
        let texts: Vec<String> = with(|s| s.book.reminders.iter().map(|r| r.text.clone()).collect());
        assert_eq!(texts, ["swap battery", "check bumpers", "download logs", "run the pit checklist", "charge the spare batteries"]);
        assert_eq!(with(|s| s.book.reminders[3].link.clone()), Link::Checklist, "it opens the checklist");
    }

    #[test]
    fn a_note_gets_a_reminder_and_loses_it() {
        let (mut robot, mut settings, mut say) = cx_parts();
        with(|s| s.book = super::super::Book::memory());
        let mut compose = Compose::for_note("2026-11-20-1130.txt", "Elevator belt");
        press(&mut compose, &typed("in 30 min"), &mut robot, &mut settings, &mut say);
        assert!(say.as_deref().is_some_and(|s| s.starts_with("reminder · ")), "{say:?}");
        assert_eq!(super::super::noted(), ["2026-11-20-1130.txt"]);
        let r = with(|s| s.book.reminders[0].clone());
        assert_eq!((r.text.as_str(), r.link), ("Elevator belt", Link::Note("2026-11-20-1130.txt".into())));
        // Again: it shows its time; emptied, the reminder is off.
        let mut compose = Compose::for_note("2026-11-20-1130.txt", "Elevator belt");
        assert!(compose.field.text.starts_with("today at") || compose.field.text.starts_with("tomorrow at"), "{}", compose.field.text);
        let clear: Vec<Key> = std::iter::repeat_n(Key::Backspace, 40).chain([Key::Enter]).collect();
        press(&mut compose, &clear, &mut robot, &mut settings, &mut say);
        assert_eq!(say.as_deref(), Some("reminder off"));
        assert!(super::super::noted().is_empty());
    }

    #[test]
    fn every_line_and_hint_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        // The titles beside the island: no wider than Settings' own "Appearance".
        let ty = g.ty;
        for title in ["Reminders", "Quick add", "Change it", "Remind me", "Pit routine"] {
            assert!(g.measure(title, ty.title) <= g.measure("Appearance", ty.title) + 9.0, "{title}");
        }
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for hint in [
                "enter: put back · d: delete",
                "←: today and upcoming",
                "n: new · p: routine",
                "o: open note",
                "enter: done · s: snooze",
                "enter: add · esc: cancel",
                "enter: change it · esc: leave it",
                "enter: set · esc: cancel",
                "enter: keep · esc: cancel",
                "enter: add them all",
                "enter: edit · d: take off",
            ] {
                assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{hint} at {size:?}");
            }
            // The three lists' labels side by side, with the one list shown,
            // giving up their counts and then "only" where the text is larger.
            for filter in [None, Some(List::Pit), Some(List::Personal)] {
                let (labels, only) = header(&mut g, [12, 12], filter);
                let w: f32 = labels.iter().map(|l| g.measure(l, ty.caption) + 2.0 * HEAD_PAD + 2.0).sum::<f32>() + only.as_ref().map_or(0.0, |o| g.measure(o, ty.caption) + 10.0);
                assert!(w <= W - 2.0 * PAD - 4.0, "{labels:?} {only:?} at {size:?}");
                if size == TextSize::Standard {
                    assert_eq!(labels[0], "today 12", "the counts show at the usual size");
                }
            }
            // What the island says, within the room it gives a message.
            for said in ["snoozed till tmrw 09:00", "snoozed till Q40 is close", "d again deletes it", "on the personal list", "already on the lists", "5 reminders added", "just the next match", "give it a time first: e"] {
                assert!(g.measure(said, ty.caption) <= 250.0, "{said}");
            }
            // The empty lists' words.
            for line in ["try: swap battery in 10 min", "try: before Q42 charge spare", "nothing ticked off yet"] {
                assert!(g.measure(line, ty.caption) <= W - 2.0 * PAD, "{line}");
            }
            // The compose card's lines.
            let room = W - 2.0 * PAD - 40.0;
            for line in ["what, then when", "tomorrow 8am check bumpers", "before Q42 charge spare", "a time: in 10 min, 8am, every day", "a match: before Q42, after Q31", "when should it come back?", "left empty, enter takes it off", "after each match · tomorrow 8am", "after each match: swap battery"] {
                assert!(g.measure(line, ty.caption) <= room, "{line} at {size:?}");
            }
            // A row's time at its longest, with a mark beside it, leaves its
            // words room for a couple.
            let row_w = W - 2.0 * PAD - 5.0;
            for when in ["Q100 ~tmrw 14:05", "Q100 not yet","no schedule yet", "no match coming", "Q31 12:05 pm", "tmrw 12:05 pm"] {
                let room = row_w - 30.0 - g.measure(when, ty.label) - 14.0 - g.measure(indent(&g), ty.body);
                assert!(room >= g.measure("swap battery", ty.body), "{when} at {size:?}");
            }
        }
    }

    #[test]
    fn the_app_draws_every_list_without_missing_an_icon() {
        let (robot, ..) = cx_parts();
        with(|s| {
            s.book = super::super::Book::memory();
            let n = Now { at: now(), zone: &Here, view: None };
            for (text, when, link) in [
                ("swap battery", When::In(now() - 30), Link::None),
                ("Elevator belt", When::In(now() + 900), Link::Note("x.txt".into())),
                ("run the checklist", When::Match(super::super::Cue::Before(LEAD), super::super::Of::Each), Link::Checklist),
                ("buy zip ties", When::Whenever, Link::None),
            ] {
                s.book.add(Parsed { text: text.into(), when, repeat: Repeat::Never, list: None, link }, &n);
            }
            let id = s.book.add(Parsed { text: "charge spare".into(), when: When::In(now() + 60), repeat: Repeat::Never, list: None, link: Link::None }, &n);
            s.book.tick(id, &n, true);
            s.book.fire(&n, true);
        });
        let mut g = Gfx::new(bezel::Tokens::dark());
        for theme in [crate::look::Theme::Catalyst, crate::look::Theme::Bezel] {
            g.theme = theme;
            let mut app = Reminders::new();
            for page in 0..3 {
                app.go(page);
                for _ in 0..40 {
                    app.tick(1.0 / 60.0);
                    app.draw(&mut g, &robot);
                }
            }
            let mut compose = Compose::new();
            compose.draw(&mut g, &robot);
            Routine::new().draw(&mut g, &robot);
        }
        assert!(g.missing_icons().is_empty(), "{:?}", g.missing_icons());
    }
}
