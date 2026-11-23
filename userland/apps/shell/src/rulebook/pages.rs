//! Rulebook's pages: a list of rules (a chapter, the bookmarks, what an
//! update changed), a rule to read, search, ask, and the manual itself.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::ask::{self, Answer, Who};
use super::parse::{self, mark, Kind};
use super::store::{self, Book, Found};
use super::{entry_row, id_column, marked_text, marks_in, Job, POLL};
use crate::gfx::{Gfx, ListState, TextSize, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The width of a page's text: the panel, less the scroll marker's lane.
fn page_w() -> f32 {
    W - 2.0 * PAD - 6.0
}

/// How much of a line centred at `cy` shows between `top` and `bottom`:
/// whole while it is in, fading over its last half-line as it leaves.
fn edge_fade(cy: f32, top: f32, bottom: f32, line_h: f32) -> f32 {
    let half = line_h * 0.5;
    ((cy - top) / half).min((bottom - cy) / half).clamp(0.0, 1.0)
}

/// A scrolled page's position marker.
fn scroll_marker(g: &mut Gfx, top: f32, view: f32, height: f32, at: f32, max: f32) {
    if height <= view {
        return;
    }
    let thumb = (view * view / height).max(10.0);
    let y = top + (view - thumb) * (at / max.max(1.0)).clamp(0.0, 1.0);
    let faint = g.tokens.faint;
    g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
}

/// A scrolling view's springs: where it is and where it is going.
struct Scroll {
    at: Animated,
    target: f32,
    bump: f32,
    placed: bool,
    max: f32,
}

impl Scroll {
    fn new() -> Self {
        Scroll { at: Animated::new(0.0, role::hold()).with_rest(0.1), target: 0.0, bump: 0.0, placed: false, max: 0.0 }
    }

    fn by(&mut self, by: f32) {
        let want = (self.target + by).clamp(0.0, self.max);
        if want == self.target && by != 0.0 {
            self.bump = by.signum();
        }
        self.target = want;
    }

    fn to(&mut self, y: f32) {
        self.target = y.clamp(0.0, self.max);
    }

    fn reset(&mut self) {
        self.target = 0.0;
        self.at.jump(0.0);
    }

    /// Before drawing: the furthest it goes now, and the spring set off.
    fn frame(&mut self, g: &mut Gfx, max: f32) -> f32 {
        self.max = max.max(0.0);
        self.target = self.target.clamp(0.0, self.max);
        if !self.placed || g.calm {
            self.at.jump(self.target);
            self.placed = true;
        } else if self.at.target() != self.target {
            self.at.set(self.target);
            g.started_moving();
        }
        let bump = std::mem::take(&mut self.bump);
        if bump != 0.0 && !g.calm {
            self.at.nudge(bump * 260.0);
            g.started_moving();
        }
        self.at.value()
    }
}

/// The field along the top that a search or a question is typed in: a
/// capsule with its icon, the words (their end, when long) and the caret.
fn draw_field(g: &mut Gfx, icon: &str, text: &str, empty: &str, typing: bool) -> Rect {
    let (t, ty) = (g.tokens, g.ty);
    let h = ty.body.size + 13.0;
    let r = Rect::new(PAD, g.top(), W - 2.0 * PAD, h);
    g.surface(r, h * 0.5, t.surface2);
    g.icon(icon, false, r.x + 13.0, r.cy(), 14.0, t.dim);
    let x = r.x + 25.0;
    let room = r.w - 36.0;
    if text.is_empty() {
        if typing {
            g.surface(Rect::new(x, r.cy() - ty.body.size * 0.5, 1.5, ty.body.size), 0.5, t.signal);
        }
        let shown = g.text.fit(empty, &ty.body, room - 5.0);
        g.text_at(x + 5.0, r.cy(), Align::Left, &shown, ty.body, t.faint);
    } else if !typing {
        // Asked: its start, cut short at the end.
        let shown = g.text.fit(text, &ty.body, room);
        g.text_at(x, r.cy(), Align::Left, &shown, ty.body, t.dim);
    } else {
        // Being typed: its end, where the caret is.
        let mut shown: &str = text;
        while g.measure(shown, ty.body) > room {
            let mut chars = shown.chars();
            chars.next();
            shown = chars.as_str();
        }
        let w = g.text_at(x, r.cy(), Align::Left, shown, ty.body, t.ink);
        {
            g.surface(Rect::new(x + w + 1.0, r.cy() - ty.body.size * 0.5, 1.5, ty.body.size), 0.5, t.signal);
            g.copy_field = Some(text.to_owned());
        }
    }
    r
}

// ------------------------------------------------------------------ list ---

/// One row of a list of rules.
#[derive(Clone, Debug)]
struct Line {
    /// The entry it opens; none for a rule an update took out.
    entry: Option<usize>,
    id: String,
    title: String,
    value: String,
}

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Chapter,
    Bookmarks,
    Changes,
}

/// A list of rules and sections: a chapter's, the bookmarks, or what the
/// last update changed.
pub struct Entries {
    book: Rc<Book>,
    kind: ListKind,
    title: String,
    lines: Vec<Line>,
    list: ListState,
}

impl Entries {
    fn line(book: &Book, i: usize, value: &str) -> Line {
        let item = &book.items[i];
        Line { entry: Some(i), id: item.id.clone(), title: item.title.clone(), value: value.to_owned() }
    }

    pub fn chapter(book: Rc<Book>, chapter: usize) -> Self {
        let lines = book.contents(chapter).into_iter().map(|i| Self::line(&book, i, "")).collect();
        let item = &book.items[chapter];
        // Short enough for the title's place beside the island.
        let title = if item.title.chars().count() <= 11 {
            item.title.clone()
        } else {
            match item.title.rsplit_once('(').map(|(_, x)| x.trim_end_matches(')')) {
                Some(letter) if letter.len() <= 2 => format!("{letter} rules"),
                _ => format!("Chapter {}", item.id),
            }
        };
        Entries { book, kind: ListKind::Chapter, title, lines, list: ListState::default() }
    }

    pub fn bookmarks(book: Rc<Book>) -> Self {
        let lines = super::bookmarked(&book).into_iter().map(|i| Self::line(&book, i, "")).collect();
        Entries { book, kind: ListKind::Bookmarks, title: "Bookmarks".into(), lines, list: ListState::default() }
    }

    pub fn changes(book: Rc<Book>) -> Self {
        let mut lines = Vec::new();
        if let Some(c) = book.meta.changes.clone() {
            let find = |key: &str| book.items.iter().position(|i| if i.is_rule() { i.id == key } else { format!("{} {}", i.id, i.title) == key });
            for (keys, value) in [(&c.changed, "changed"), (&c.added, "new")] {
                for k in keys {
                    if let Some(i) = find(k) {
                        lines.push(Self::line(&book, i, value));
                    }
                }
            }
            for k in &c.removed {
                let (id, title) = k.split_once(' ').unwrap_or((k, ""));
                lines.push(Line { entry: None, id: id.into(), title: if title.is_empty() { "taken out".into() } else { title.into() }, value: "removed".into() });
            }
        }
        Entries { book, kind: ListKind::Changes, title: "Changes".into(), lines, list: ListState::default() }
    }

    fn openable(&self) -> Vec<usize> {
        self.lines.iter().filter_map(|l| l.entry).collect()
    }
}

impl Screen for Entries {
    fn title(&self) -> &str {
        &self.title
    }

    fn resume(&mut self) {
        if self.kind == ListKind::Bookmarks {
            let focus = self.list.focus;
            *self = Self::bookmarks(self.book.clone());
            self.list.focus = focus;
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Left => Nav::Pop,
            Key::Enter | Key::Right => {
                let Some(entry) = self.lines.get(self.list.focus).and_then(|l| l.entry) else {
                    if !self.lines.is_empty() {
                        *cx.say = Some("taken out of the manual".into());
                    }
                    return Nav::Stay;
                };
                let seq = self.openable();
                let at = seq.iter().position(|&e| e == entry).unwrap_or(0);
                Nav::Push(Box::new(Reader::new(self.book.clone(), seq, at, Vec::new())))
            }
            Key::Char('b') if self.kind == ListKind::Bookmarks => {
                if let Some(entry) = self.lines.get(self.list.focus).and_then(|l| l.entry) {
                    super::toggle_bookmark(&super::mark_key(&self.book.items[entry]));
                    *cx.say = Some(format!("{} bookmark removed", self.book.items[entry].id));
                    self.resume();
                }
                Nav::Stay
            }
            other => {
                crate::screens::list_key(&mut self.list, other, self.lines.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.lines.is_empty() {
            match self.kind {
                ListKind::Bookmarks => g.empty("bookmark", "b on a rule bookmarks it"),
                _ => g.empty("menu_book", "nothing here"),
            }
            return;
        }
        let hint = match self.kind {
            ListKind::Bookmarks => Some(["enter: read · b: remove the bookmark", "b: remove bookmark"]),
            _ => None,
        };
        let (top, row_h) = (g.top(), g.row_h());
        let bottom = if hint.is_some() { g.above_hint() } else { H };
        let column = id_column(g, self.lines.iter().map(|l| l.id.as_str()));
        for (i, y) in self.list.layout(g, self.lines.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let line = &self.lines[i];
            entry_row(g, r, column, &line.id, &line.title, &line.value, i == self.list.focus);
            if line.entry.is_none() {
                // Taken out: struck through, quietly.
                let w = g.measure(&line.id, g.ty.label);
                g.surface(Rect::new(r.x + 10.0, r.cy() - 0.5, w, 1.0), 0.5, t.dim);
            }
        }
        self.list.scrollbar(g, self.lines.len(), top, bottom);
        if let Some(h) = hint {
            g.hint_any(&h);
        }
    }
}

// ---------------------------------------------------------------- reader ---

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Context,
    Head,
    Body,
    Row,
}

struct Laid {
    key: (usize, TextSize, crate::look::Theme, bool),
    /// (where its middle is from the page's top, how far in, its words, its role, the matches in it)
    lines: Vec<(f32, f32, String, Role, Vec<(usize, usize)>)>,
    /// Blue boxes, violations and tables: (top, height, a blue box).
    cards: Vec<(f32, f32, bool)>,
    height: f32,
}

/// A rule (or a section) to read: its place in the manual, its headline,
/// its words, its blue boxes, violation and tables; ←→ the one before and
/// after in the list it was opened from.
pub struct Reader {
    book: Rc<Book>,
    seq: Vec<usize>,
    at: usize,
    terms: Vec<String>,
    title: String,
    body: String,
    marked: bool,
    scroll: Scroll,
    slide: Animated,
    laid: Option<Laid>,
    step: f32,
    page: f32,
}

impl Reader {
    pub fn new(book: Rc<Book>, seq: Vec<usize>, at: usize, terms: Vec<String>) -> Self {
        let mut r = Reader {
            book,
            seq,
            at,
            terms,
            title: String::new(),
            body: String::new(),
            marked: false,
            scroll: Scroll::new(),
            slide: Animated::new(0.0, role::page()).with_rest(0.004),
            laid: None,
            step: 30.0,
            page: 100.0,
        };
        r.load();
        r
    }

    fn entry(&self) -> usize {
        self.seq.get(self.at).copied().unwrap_or(0)
    }

    fn load(&mut self) {
        let i = self.entry();
        let item = &self.book.items[i];
        self.title = if item.id.is_empty() { item.title.clone() } else { item.id.clone() };
        self.body = self.book.body(i);
        self.marked = super::bookmarks().contains(&super::mark_key(item));
        self.laid = None;
    }

    fn turn(&mut self, by: isize) {
        let to = self.at as isize + by;
        if to < 0 || to >= self.seq.len() as isize {
            self.scroll.bump = by.signum() as f32;
            return;
        }
        self.at = to as usize;
        self.load();
        self.scroll.reset();
        self.slide.jump(by as f32);
        self.slide.set(0.0);
    }

    /// The line above the headline: the section it is in, and whether it is
    /// an evergreen rule.
    fn context(&self) -> String {
        let i = self.entry();
        let item = &self.book.items[i];
        let mut words = match item.parent {
            Some(p) => self.book.items[p].label(),
            None => "chapter".into(),
        };
        if item.kind == Kind::Rule && super::headline(item).1 {
            words.push_str(" · evergreen");
        }
        words.to_lowercase()
    }

    fn lay_out(&self, g: &mut Gfx) -> Laid {
        let ty = g.ty;
        let width = page_w();
        let item = &self.book.items[self.entry()];
        let mut lines = Vec::new();
        let mut cards = Vec::new();
        let marks = |text: &str| marks_in(text, &self.terms);
        let mut y = 0.0;
        let context = g.text.fit(&self.context(), &ty.caption, width - 18.0);
        lines.push((y + ty.caption.size * 0.5, 0.0, context, Role::Context, Vec::new()));
        y += ty.caption.size + 6.0;
        let (head, _) = super::headline(item);
        let head_lh = ty.name.size + 4.0;
        for l in crate::help::wrap(g, head, ty.name, width) {
            let m = marks(&l);
            lines.push((y + head_lh * 0.5, 0.0, l, Role::Head, m));
            y += head_lh;
        }
        y += 5.0;
        let body_lh = ty.body.size + 4.0;
        let row_lh = ty.label.size + 5.0;
        // The card being filled: its kind (BLUE, VIOLATION, ROW) and top.
        let mut card: Option<(u8, f32)> = None;
        let close = |card: &mut Option<(u8, f32)>, y: &mut f32, cards: &mut Vec<(f32, f32, bool)>| {
            if let Some((kind, from)) = card.take() {
                *y += 4.0;
                cards.push((from, *y - from, kind == mark::BLUE));
                *y += 6.0;
            }
        };
        for (m, text) in parse::paragraphs(&self.body) {
            let kind = match m {
                mark::BLUE | mark::BLUE_ITEM => Some(mark::BLUE),
                mark::VIOLATION => Some(mark::VIOLATION),
                mark::ROW => Some(mark::ROW),
                _ => None,
            };
            if card.map(|(k, _)| k) != kind {
                close(&mut card, &mut y, &mut cards);
                if let Some(k) = kind {
                    card = Some((k, y));
                    y += 5.0;
                }
            }
            let (inset, style, role, lh) = match m {
                mark::BLUE => (13.0, ty.body, Role::Body, body_lh),
                mark::BLUE_ITEM => (25.0, ty.body, Role::Body, body_lh),
                mark::VIOLATION => (10.0, ty.body, Role::Body, body_lh),
                mark::ROW => (10.0, ty.label, Role::Row, row_lh),
                mark::ITEM => (12.0, ty.body, Role::Body, body_lh),
                mark::CAPTION => (0.0, ty.caption, Role::Context, ty.caption.size + 5.0),
                _ => (0.0, ty.body, Role::Body, body_lh),
            };
            let right = if kind.is_some() { 8.0 } else { 0.0 };
            for l in crate::help::wrap(g, text, style, width - inset - right) {
                let m = if role == Role::Context { Vec::new() } else { marks(&l) };
                lines.push((y + lh * 0.5, inset, l, role, m));
                y += lh;
            }
            y += if kind.is_some() { 2.0 } else { 5.0 };
        }
        close(&mut card, &mut y, &mut cards);
        Laid { key: (self.entry(), g.text_size, g.theme, self.marked), lines, cards, height: y }
    }
}

impl Screen for Reader {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Up => self.scroll.by(-self.step),
            Key::Down => self.scroll.by(self.step),
            Key::PageUp => self.scroll.by(-self.page),
            Key::PageDown | Key::Enter | Key::Char(' ') => self.scroll.by(self.page),
            Key::Home => self.scroll.to(0.0),
            Key::End => self.scroll.to(f32::MAX),
            Key::Left => self.turn(-1),
            Key::Right => self.turn(1),
            Key::Char('b') => {
                let item = &self.book.items[self.entry()];
                self.marked = super::toggle_bookmark(&super::mark_key(item));
                *cx.say = Some(if self.marked { format!("bookmarked {}", self.title) } else { "bookmark removed".into() });
                self.laid = None;
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.at.tick(dt) | self.slide.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        let key = (self.entry(), g.text_size, g.theme, self.marked);
        if self.laid.as_ref().is_none_or(|l| l.key != key) {
            self.laid = Some(self.lay_out(g));
        }
        let laid = self.laid.as_ref().expect("laid out above");
        let top = g.top();
        let bottom = H - g.hint_h() - 2.0;
        let view = bottom - top;
        self.step = (ty.body.size + 4.0) * 2.0;
        self.page = (view - ty.body.size - 4.0).max(20.0);
        let s = self.scroll.frame(g, laid.height - view + 4.0);
        if g.calm {
            self.slide.jump(0.0);
        }
        let ox = g.ox;
        g.ox += self.slide.value() * 18.0;
        g.canvas.set_clip(Rect::new(0.0, top - 1.0, W, view + 1.0));
        let radius = g.radius(10.0).min(12.0);
        for &(y, h, blue) in &laid.cards {
            let r = Rect::new(PAD, top - s + y, page_w(), h);
            if r.bottom() < top || r.y > bottom {
                continue;
            }
            g.surface(r, radius, if blue { t.surface1 } else { t.surface2 });
            if blue {
                // FIRST's blue box: a bar of ice down its side.
                g.surface(Rect::new(r.x + 4.0, r.y + 5.0, 3.0, h - 10.0), 1.5, t.ice);
            }
        }
        for (y, x, words, role, marks) in &laid.lines {
            let cy = top - s + y;
            let (style, color) = match role {
                Role::Context => (ty.caption, t.dim),
                Role::Head => (ty.name, t.ink),
                Role::Body => (ty.body, t.ink.mix(t.dim, 0.15)),
                Role::Row => (ty.label, t.ink),
            };
            let alpha = edge_fade(cy, top, bottom, style.size + 4.0);
            if alpha <= 0.0 {
                continue;
            }
            marked_text(g, PAD + x, cy, words, marks, style, color, alpha);
        }
        if self.marked {
            // Bookmarked: on, so an ice chip with the bookmark in its ink.
            let cy = top - s + ty.caption.size * 0.5;
            let alpha = edge_fade(cy, top, bottom, ty.caption.size + 4.0);
            let h = ty.caption.size + 4.0;
            let chip = Rect::new(PAD + page_w() - h * 1.6, cy - h * 0.5, h * 1.6, h);
            g.canvas.rrect(chip.offset(g.ox, 0.0), h * 0.5, t.ice, alpha);
            g.icon_alpha("bookmark", true, chip.cx(), cy, ty.caption.size, t.on_ice, alpha);
        }
        g.canvas.clear_clip();
        g.ox = ox;
        scroll_marker(g, top, view, laid.height, s, self.scroll.max);
        let near = |k: isize| {
            let at = self.at as isize + k;
            (at >= 0).then(|| self.seq.get(at as usize)).flatten().map(|&i| self.book.items[i].id.to_lowercase())
        };
        let b = if self.marked { "b: unbookmark" } else { "b: bookmark" };
        let full = match (near(-1), near(1)) {
            (Some(a), Some(z)) => format!("← {a} · {z} → · {b}"),
            (None, Some(z)) => format!("{z} → · {b}"),
            (Some(a), None) => format!("← {a} · {b}"),
            (None, None) => format!("↑↓ read · {b}"),
        };
        g.hint_any(&[&full, b]);
    }
}

// ---------------------------------------------------------------- search ---

/// Search: what is typed searched as it is typed. A rule's ID comes first
/// (Enter opens it); words are ranked and marked in each result.
pub struct Searching {
    book: Rc<Book>,
    text: String,
    found: Found,
    list: ListState,
    snippets: HashMap<usize, (String, Vec<(usize, usize)>)>,
}

/// Results a search lists at most.
const RESULTS: usize = 40;

impl Searching {
    pub fn new(book: Rc<Book>, typed: &str) -> Self {
        let mut s = Searching { book, text: typed.to_owned(), found: Found::default(), list: ListState::default(), snippets: HashMap::new() };
        s.run();
        s
    }

    fn run(&mut self) {
        self.found = if self.text.trim().is_empty() { Found::default() } else { self.book.search(&self.text, RESULTS) };
        self.snippets.clear();
        self.list.focus = 0;
    }

    fn rows(&self) -> usize {
        self.found.hits.len() + usize::from(!self.text.trim().is_empty())
    }

    fn row_h(g: &Gfx) -> f32 {
        g.ty.body.size + g.ty.caption.size + 15.0
    }
}

impl Screen for Searching {
    fn title(&self) -> &str {
        "Search"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Char(c) if !c.is_control() && self.text.chars().count() < 60 => {
                self.text.push(c);
                self.run();
            }
            Key::Backspace => {
                self.text.pop();
                self.run();
            }
            Key::Enter | Key::Right => {
                let hits: Vec<usize> = self.found.hits.iter().map(|h| h.entry).collect();
                if self.list.focus < hits.len() {
                    return Nav::Push(Box::new(Reader::new(self.book.clone(), hits, self.list.focus, self.found.terms.clone())));
                }
                if !self.text.trim().is_empty() {
                    return Nav::Push(Box::new(Asking::with_question(self.book.clone(), &self.text)));
                }
            }
            other => {
                let rows = self.rows();
                crate::screens::list_key(&mut self.list, other, rows);
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        let field = draw_field(g, "search", &self.text, "a rule, like G101, or words", true);
        if self.text.trim().is_empty() {
            let cy = (field.bottom() + H) * 0.5;
            let lines = crate::help::wrap(g, "The whole manual, on this handheld: type a rule’s ID to jump to it, or words to find them.", ty.caption, W - 4.0 * PAD);
            let lh = ty.caption.size + 4.0;
            let y0 = cy - lines.len() as f32 * lh * 0.5;
            for (k, l) in lines.iter().enumerate() {
                g.text_at(W * 0.5, y0 + (k as f32 + 0.5) * lh, Align::Center, l, ty.caption, t.dim);
            }
            return;
        }
        let top = field.bottom() + 6.0;
        let row_h = Self::row_h(g);
        let bottom = g.above_hint();
        let rows = self.rows();
        let column = id_column(g, self.found.hits.iter().map(|h| self.book.items[h.entry].id.as_str()));
        for (i, y) in self.list.layout(g, rows, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.row(r, "", "", t.dim, focused);
            let cy1 = r.y + 7.0 + ty.body.size * 0.5;
            let cy2 = r.bottom() - 7.0 - ty.caption.size * 0.5;
            let Some(hit) = self.found.hits.get(i) else {
                g.icon("auto_awesome", false, r.x + 16.0, cy1, 13.0, t.dim);
                let ask = g.text.fit(&format!("Ask: {}", self.text.trim()), &ty.body, r.w - 42.0);
                g.text_at(r.x + 10.0 + column, cy1, Align::Left, &ask, ty.body, t.ink);
                g.text_at(r.x + 10.0 + column, cy2, Align::Left, "an answer from the rules it finds", ty.caption, t.dim);
                continue;
            };
            let item = &self.book.items[hit.entry];
            g.text_at(r.x + 10.0, cy1, Align::Left, &item.id, ty.label, t.dim);
            let room = r.w - 20.0 - column;
            let head = g.text.fit(super::headline(item).0, &ty.body, room);
            let marks = marks_in(&head, &self.found.terms);
            marked_text(g, r.x + 10.0 + column, cy1, &head, &marks, ty.body, t.ink, 1.0);
            if !self.snippets.contains_key(&hit.entry) {
                let body = self.book.body(hit.entry);
                let mut snip = store::snippet(&body, &self.found.terms, 120);
                // A section with no words of its own (its rules follow it): where it is.
                if snip.0.trim().is_empty() {
                    let place = item.parent.map(|p| self.book.items[p].label()).unwrap_or_else(|| format!("{} rules", self.book.contents(hit.entry).iter().filter(|&&k| self.book.items[k].is_rule()).count()));
                    snip = (place.to_lowercase(), Vec::new());
                }
                self.snippets.insert(hit.entry, snip);
            }
            let (snip, marks) = &self.snippets[&hit.entry];
            let room = r.w - 20.0;
            let shown = g.text.fit(snip, &ty.caption, room);
            let keep = if shown == *snip { shown.len() } else { shown.len().saturating_sub('…'.len_utf8()) };
            let marks: Vec<(usize, usize)> = marks.iter().copied().filter(|&(_, b)| b <= keep).collect();
            marked_text(g, r.x + 10.0, cy2, &shown, &marks, ty.caption, t.dim, 1.0);
        }
        self.list.scrollbar(g, rows, top, bottom);
        let n = self.found.hits.len();
        let found = match n {
            0 => "nothing found".to_owned(),
            1 => "1 found".to_owned(),
            n if n >= RESULTS => format!("{n}+ found"),
            n => format!("{n} found"),
        };
        g.hint_any(&[&format!("{found} · enter: open · esc: back"), &format!("{found} · enter: open"), &found]);
    }
}

// ------------------------------------------------------------------- ask ---

/// What the page under the question holds.
#[derive(Clone, PartialEq)]
enum Part {
    /// The answer, or how it stands: its caption, then its lines.
    Answer(String, Vec<String>),
    Label(&'static str),
    Rule(usize),
    Note(Vec<String>),
}

/// Ask: a question in plain words; the rules the search finds, a short
/// answer from them citing their IDs, and the rules under it to check.
pub struct Asking {
    book: Rc<Book>,
    who: Who,
    question: String,
    typing: bool,
    asked: String,
    found: Vec<usize>,
    answer: Option<Result<Answer, String>>,
    cited: Vec<usize>,
    job: Option<Receiver<Result<Answer, String>>>,
    started: Instant,
    shown_seconds: u64,
    fresh: bool,
    /// 0 the answer, then the rules under it.
    focus: usize,
    scroll: Scroll,
    laid: Option<(TextSize, crate::look::Theme, Vec<(f32, f32, Part)>, f32)>,
}

impl Asking {
    pub fn new(book: Rc<Book>) -> Self {
        Asking {
            book,
            who: ask::who(),
            question: String::new(),
            typing: true,
            asked: String::new(),
            found: Vec::new(),
            answer: None,
            cited: Vec::new(),
            job: None,
            started: Instant::now(),
            shown_seconds: 0,
            fresh: false,
            focus: 0,
            scroll: Scroll::new(),
            laid: None,
        }
    }

    pub fn with_question(book: Rc<Book>, question: &str) -> Self {
        let mut a = Self::new(book);
        a.question = question.trim().to_owned();
        a.submit();
        a
    }

    /// For the pictures: answered by someone in particular.
    pub fn by(mut self, who: Who) -> Self {
        self.who = who;
        self
    }

    /// For the pictures: a question asked as it opens.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn asking(mut self, question: &str) -> Self {
        self.question = question.to_owned();
        self.submit();
        self
    }

    /// Ask what is typed: the rules found at once, the answer on a thread.
    pub fn submit(&mut self) {
        let question = self.question.trim().to_owned();
        if question.is_empty() {
            return;
        }
        let n = match self.who {
            Who::Local => crate::offline::model::find().map_or(3, |m| crate::offline::model::tier(crate::offline::model::size_m(&m.name())).lines),
            _ => ask::PASSAGES_ONLINE,
        };
        self.found = ask::retrieve(&self.book, &question, n.max(3));
        self.asked = question.clone();
        self.typing = false;
        self.answer = None;
        self.cited.clear();
        self.focus = 0;
        self.scroll.reset();
        self.laid = None;
        self.job = None;
        if self.who == Who::Nobody || self.found.is_empty() {
            return;
        }
        let passages = ask::passages(&self.book, &self.found);
        let who = self.who.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("rulebook-ask".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                tx.send(ask::answer(&who, &passages, &question)).ok();
            })
            .ok();
        self.job = Some(rx);
        self.started = Instant::now();
        self.shown_seconds = 0;
    }

    /// The rules under the answer: those it cites, then the rest it found.
    fn rules(&self) -> (Vec<usize>, Vec<usize>) {
        let rest = self.found.iter().copied().filter(|i| !self.cited.contains(i)).collect();
        (self.cited.clone(), rest)
    }

    fn answer_words(&self) -> (String, Vec<String>) {
        let by = self.who.name().to_lowercase();
        match (&self.answer, &self.job) {
            (_, Some(_)) => {
                let n = self.found.len();
                let s = self.started.elapsed().as_secs();
                (format!("{by} · reading {n} {} · {s} s", if n == 1 { "rule" } else { "rules" }), Vec::new())
            }
            (Some(Ok(a)), _) => {
                let caption = if a.offline { format!("{} · offline, on this handheld", a.by.to_lowercase()) } else if self.who == Who::Local { format!("{} · on this handheld", a.by.to_lowercase()) } else { a.by.to_lowercase() };
                (caption, vec![a.text.clone()])
            }
            (Some(Err(why)), _) => ("no answer".into(), vec![format!("{}: the rules the search found are below.", capitalized(why))]),
            (None, None) if self.found.is_empty() => ("nothing found".into(), vec!["No rule in the manual matches those words. Try other words, or a rule’s ID.".into()]),
            (None, None) => ("the rules found".into(), vec!["No model on this handheld to answer with: Claude needs its key and the internet, the on-device model a download (Settings › On-device model). The rules the search found are below.".into()]),
        }
    }

    fn lay_out(&self, g: &mut Gfx) -> (Vec<(f32, f32, Part)>, f32) {
        let ty = g.ty;
        let mut parts = Vec::new();
        let mut y = 0.0;
        let (caption, words) = self.answer_words();
        let lines: Vec<String> = words.iter().flat_map(|w| crate::help::wrap(g, w, ty.body, page_w() - 20.0)).collect();
        let h = 8.0 + ty.caption.size + if lines.is_empty() { 8.0 } else { 6.0 + lines.len() as f32 * (ty.body.size + 4.0) + 6.0 };
        parts.push((y, h, Part::Answer(caption, lines)));
        y += h + 8.0;
        let (cited, rest) = self.rules();
        let row = g.row_h();
        for (label, list) in [("cited", &cited), (if cited.is_empty() { "found in the manual" } else { "also found" }, &rest)] {
            if list.is_empty() {
                continue;
            }
            parts.push((y, ty.caption.size + 6.0, Part::Label(label)));
            y += ty.caption.size + 6.0;
            for &i in list {
                parts.push((y, row, Part::Rule(i)));
                y += row + 3.0;
            }
            y += 5.0;
        }
        let note = crate::help::wrap(g, "Answers can be wrong: read the rule. At an event, the head referee has the final word.", ty.caption, page_w() - 4.0);
        let h = note.len() as f32 * (ty.caption.size + 4.0);
        parts.push((y, h, Part::Note(note)));
        y += h + 4.0;
        (parts, y)
    }

    fn focusables(&self) -> Vec<(f32, f32)> {
        self.laid.as_ref().map(|(_, _, parts, _)| parts.iter().filter(|(_, _, p)| matches!(p, Part::Answer(..) | Part::Rule(_))).map(|(y, h, _)| (*y, *h)).collect()).unwrap_or_default()
    }

    /// Up or down: through the answer while it runs past the view, then
    /// from row to row, the view following.
    fn step(&mut self, by: isize, view: f32, line: f32) {
        let items = self.focusables();
        let Some(&(y, h)) = items.get(self.focus) else { return };
        let s = self.scroll.target;
        if by > 0 {
            if y + h > s + view + 0.5 {
                self.scroll.by(line * 2.0);
                return;
            }
            if self.focus + 1 < items.len() {
                self.focus += 1;
                let (ny, nh) = items[self.focus];
                if ny + nh > s + view {
                    self.scroll.to(ny + nh - view + 2.0);
                }
            } else {
                self.scroll.by(line * 2.0);
            }
        } else {
            if y < s - 0.5 {
                self.scroll.by(-line * 2.0);
                return;
            }
            if self.focus > 0 {
                self.focus -= 1;
                let (ny, _) = items[self.focus];
                if ny < s {
                    self.scroll.to(ny - 2.0);
                }
            } else {
                self.scroll.by(-line * 2.0);
            }
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

thread_local! {
    /// The view's height and a line's, as last drawn, for the keys.
    static ASK_VIEW: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((90.0, 17.0)) };
}

impl Screen for Asking {
    fn title(&self) -> &str {
        "Ask"
    }

    fn wants_text(&self) -> bool {
        self.typing
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if self.typing {
            match key {
                Key::Back | Key::Leave if self.asked.is_empty() => return Nav::Pop,
                Key::Back | Key::Leave => {
                    self.typing = false;
                    self.question = self.asked.clone();
                }
                Key::Enter => self.submit(),
                Key::Backspace => {
                    self.question.pop();
                }
                Key::Char(c) if !c.is_control() && self.question.chars().count() < 300 => self.question.push(c),
                _ => {}
            }
            return Nav::Stay;
        }
        let (view, line) = ASK_VIEW.with(|v| v.get());
        match key {
            Key::Back => return Nav::Pop,
            Key::Up => self.step(-1, view, line),
            Key::Down => self.step(1, view, line),
            Key::Enter | Key::Right if self.focus == 0 => {
                // Ask again, the question as it was to change.
                self.typing = true;
            }
            Key::Enter | Key::Right => {
                let (cited, rest) = self.rules();
                let seq: Vec<usize> = cited.into_iter().chain(rest).collect();
                if let Some(&entry) = seq.get(self.focus - 1) {
                    return Nav::Push(Box::new(Reader::new(self.book.clone(), seq.clone(), self.focus - 1, Vec::new()).at_entry(entry)));
                }
            }
            Key::Char(c) if c.is_alphanumeric() => {
                self.typing = true;
                self.question = c.to_string();
            }
            _ => {}
        }
        Nav::Stay
    }

    fn set_aside(&mut self) {
        // An answer on this handheld is given up on once nobody looks.
        if self.job.is_some() && self.who == Who::Local {
            crate::local::cancel();
        }
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        let Some(rx) = &self.job else { return Nav::Stay };
        let secs = self.started.elapsed().as_secs();
        if secs != self.shown_seconds {
            self.shown_seconds = secs;
            self.laid = None;
            self.fresh = true;
        }
        let outcome = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return Nav::Stay,
            Err(TryRecvError::Disconnected) => Err("the answer stopped".into()),
        };
        self.job = None;
        if let Ok(a) = &outcome {
            self.cited = ask::cited(&self.book, &a.text);
        }
        self.answer = Some(outcome);
        self.laid = None;
        self.fresh = true;
        Nav::Stay
    }

    fn ambient(&self) -> Option<Duration> {
        self.job.as_ref().map(|_| POLL)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        std::mem::take(&mut self.fresh)
    }

    fn keeps_awake(&self) -> bool {
        self.job.is_some()
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.at.tick(dt)
    }

    fn dictation(&self) -> Option<crate::voice::Dictation> {
        Some(crate::voice::Dictation::Text)
    }

    fn dictated(&mut self, text: &str, _cx: &mut Cx) -> Nav {
        self.question = text.trim().to_owned();
        self.submit();
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        let shown = if self.typing { self.question.clone() } else { self.asked.clone() };
        let field = draw_field(g, "auto_awesome", &shown, "a question about the rules", self.typing);
        if self.typing {
            g.focus_ring(field, field.h * 0.5);
        }
        if self.typing && self.asked.is_empty() {
            let by = match &self.who {
                Who::Claude(_) => format!("{} answers when online, from the rules the search finds.", crate::ai::Service::Claude.model_name()),
                Who::Local => format!("{} answers on this handheld, from the rules the search finds.", self.who.name()),
                Who::Nobody => "With no model here, the rules the search finds are the answer.".into(),
            };
            let words = format!("Ask in plain words: “can we reach into the field?” {by} The rules are shown under the answer.");
            let lines = crate::help::wrap(g, &words, ty.caption, W - 2.0 * PAD - 4.0);
            let lh = ty.caption.size + 4.0;
            for (k, l) in lines.iter().enumerate() {
                g.text_at(PAD + 2.0, field.bottom() + 8.0 + (k as f32 + 0.5) * lh, Align::Left, l, ty.caption, t.dim);
            }
            g.hint_any(&["enter: ask · esc: back", "enter: ask"]);
            return;
        }
        let key = (g.text_size, g.theme);
        if self.laid.as_ref().is_none_or(|(s, th, _, _)| (*s, *th) != key) {
            let (parts, height) = self.lay_out(g);
            self.laid = Some((key.0, key.1, parts, height));
        }
        let top = field.bottom() + 6.0;
        let bottom = H - g.hint_h() - 2.0;
        let view = bottom - top;
        ASK_VIEW.with(|v| v.set((view, ty.body.size + 4.0)));
        let height = self.laid.as_ref().map_or(0.0, |l| l.3);
        let s = self.scroll.frame(g, height - view);
        let parts = self.laid.as_ref().map(|l| l.2.clone()).unwrap_or_default();
        let column = id_column(g, parts.iter().filter_map(|(_, _, p)| if let Part::Rule(i) = p { Some(self.book.items[*i].id.as_str()) } else { None }));
        g.canvas.set_clip(Rect::new(0.0, top - 3.0, W, view + 3.0));
        let mut k = 0;
        for (y, h, part) in &parts {
            let r = Rect::new(PAD, top - s + y, page_w(), *h);
            let visible = r.bottom() >= top - 4.0 && r.y <= bottom + 4.0;
            match part {
                Part::Answer(caption, lines) => {
                    let focused = !self.typing && self.focus == 0;
                    if visible {
                        let radius = g.radius(10.0).min(12.0);
                        g.surface(r, radius, t.surface1);
                        let cy = r.y + 8.0 + ty.caption.size * 0.5;
                        let a = edge_fade(cy, top, bottom, ty.caption.size + 4.0);
                        let c = g.text.fit(caption, &ty.caption, r.w - 20.0);
                        g.text_alpha(r.x + 10.0, cy, &c, ty.caption, t.dim, a);
                        let lh = ty.body.size + 4.0;
                        for (n, l) in lines.iter().enumerate() {
                            let cy = r.y + 14.0 + ty.caption.size + (n as f32 + 0.5) * lh;
                            let a = edge_fade(cy, top, bottom, lh);
                            if a > 0.0 {
                                g.text_alpha(r.x + 10.0, cy, l, ty.body, t.ink, a);
                            }
                        }
                        if focused {
                            g.focus_ring(r, radius);
                        }
                    }
                    k += 1;
                }
                Part::Label(words) => {
                    if visible {
                        let cy = r.cy();
                        let a = edge_fade(cy, top, bottom, ty.caption.size + 4.0);
                        g.text_alpha(PAD + 2.0, cy, words, ty.caption, t.dim, a);
                    }
                }
                Part::Rule(i) => {
                    if visible {
                        let item = &self.book.items[*i];
                        entry_row(g, r, column, &item.id, &item.title, "", !self.typing && self.focus == k);
                    }
                    k += 1;
                }
                Part::Note(lines) => {
                    let lh = ty.caption.size + 4.0;
                    for (n, l) in lines.iter().enumerate() {
                        let cy = r.y + (n as f32 + 0.5) * lh;
                        let a = edge_fade(cy, top, bottom, lh);
                        if a > 0.0 {
                            g.text_alpha(PAD + 2.0, cy, l, ty.caption, t.dim, a);
                        }
                    }
                }
            }
        }
        g.canvas.clear_clip();
        scroll_marker(g, top, view, height, s, self.scroll.max);
        if self.typing {
            g.hint_any(&["enter: ask · esc: back", "enter: ask"]);
        } else if self.focus == 0 {
            g.hint_any(&["↓ the rules · enter: ask again", "enter: ask again"]);
        } else {
            g.hint_any(&["enter: read the rule · esc: back", "enter: read it"]);
        }
    }
}

// ---------------------------------------------------------------- manual ---

#[derive(Clone, Copy, PartialEq)]
enum ManualRow {
    Check,
    Changes,
    Remove,
}

/// The manual itself: where it came from, a look for a newer one, what the
/// last update changed, and taking it off the handheld.
pub struct ManualPage {
    book: Rc<Book>,
    list: ListState,
    job: Option<Job>,
    problem: Option<String>,
    removing: bool,
    fresh: bool,
    shown: String,
    up_to_date: bool,
}

impl ManualPage {
    pub fn new(book: Rc<Book>) -> Self {
        ManualPage { book, list: ListState::default(), job: None, problem: None, removing: false, fresh: false, shown: String::new(), up_to_date: false }
    }

    fn rows(&self) -> Vec<ManualRow> {
        let mut rows = vec![ManualRow::Check];
        if self.book.meta.changes.as_ref().is_some_and(|c| !c.is_empty()) {
            rows.push(ManualRow::Changes);
        }
        rows.push(ManualRow::Remove);
        rows
    }

    fn check_value(&self) -> String {
        if let Some(job) = &self.job {
            return job.value();
        }
        if let Some(why) = &self.problem {
            return if why.starts_with("no internet") || why.starts_with("could not reach") { "offline".into() } else { "failed".into() };
        }
        if self.up_to_date {
            return "up to date".into();
        }
        let ago = chrono::Utc::now().timestamp() - self.book.meta.checked;
        match ago {
            ..=59 => "just now".into(),
            60..=3599 => format!("{} min ago", ago / 60),
            3600..=86_399 => format!("{} h ago", ago / 3600),
            _ => format!("{} days ago", ago / 86_400),
        }
    }
}

impl Screen for ManualPage {
    fn title(&self) -> &str {
        "Manual"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = self.rows();
        let row = rows.get(self.list.focus).copied();
        if row != Some(ManualRow::Remove) || !matches!(key, Key::Enter | Key::Right) {
            self.removing = false;
        }
        match key {
            Key::Back | Key::Left => Nav::Pop,
            Key::Enter | Key::Right => match row {
                Some(ManualRow::Check) if self.job.is_none() => {
                    self.problem = None;
                    self.up_to_date = false;
                    self.job = Some(Job::start(false, false));
                    Nav::Stay
                }
                Some(ManualRow::Changes) => Nav::Push(Box::new(Entries::changes(self.book.clone()))),
                Some(ManualRow::Remove) if self.removing => {
                    store::remove(&store::dir());
                    *cx.say = Some("the manual is off this handheld".into());
                    Nav::Pop
                }
                Some(ManualRow::Remove) => {
                    self.removing = true;
                    Nav::Stay
                }
                _ => Nav::Stay,
            },
            other => {
                crate::screens::list_key(&mut self.list, other, rows.len());
                Nav::Stay
            }
        }
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(job) = &self.job else { return Nav::Stay };
        let v = job.value();
        if v != self.shown {
            self.shown = v;
            self.fresh = true;
        }
        let Some(outcome) = job.poll() else { return Nav::Stay };
        self.job = None;
        self.fresh = true;
        match outcome {
            Ok(super::fetch::Outcome::Kept(meta)) => {
                if let Ok(book) = Book::open(&store::dir()) {
                    self.book = Rc::new(book);
                }
                *cx.say = Some(match meta.changes.filter(|c| !c.is_empty() && c.at == meta.fetched) {
                    Some(c) => format!("manual updated · {}", super::changes_words(&c)),
                    None => "manual downloaded again".into(),
                });
            }
            Ok(super::fetch::Outcome::Current) => {
                self.up_to_date = true;
                if let Ok(book) = Book::open(&store::dir()) {
                    self.book = Rc::new(book);
                }
            }
            Err(why) => {
                *cx.say = Some(why.clone());
                self.problem = Some(why);
            }
        }
        Nav::Stay
    }

    fn ambient(&self) -> Option<Duration> {
        self.job.as_ref().map(|_| POLL)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        std::mem::take(&mut self.fresh)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let (t, ty) = (g.tokens, g.ty);
        let top = g.top();
        let width = W - 2.0 * PAD - 4.0;
        let meta = &self.book.meta;
        let name_lh = ty.body.size + 4.0;
        let title_lines = crate::help::wrap(g, &self.book.manual_title, ty.body, width);
        let mut y = top + 1.0;
        for l in &title_lines {
            g.text_at(PAD + 2.0, y + name_lh * 0.5, Align::Left, l, ty.body, t.ink);
            y += name_lh;
        }
        let date = meta.modified.as_deref().and_then(super::fetch::short_date).map(|d| format!("revised {d}"));
        let from = if meta.sample { "a sample, not FIRST’s" } else { "FIRST’s, downloaded from FIRST" };
        let facts: Vec<String> = [Some(from.to_owned()), date, Some(format!("{} rules", self.book.rules()))].into_iter().flatten().collect();
        let line = facts.join(" · ");
        let fitted = if g.measure(&line, ty.caption) <= width { line } else { facts[1..].join(" · ") };
        let cap_lh = ty.caption.size + 4.0;
        g.text_at(PAD + 2.0, y + cap_lh * 0.5, Align::Left, &fitted, ty.caption, t.dim);
        y += cap_lh + 4.0;
        let rows = self.rows();
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        for (i, ry) in self.list.layout(g, rows.len(), y, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, ry, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match rows[i] {
                ManualRow::Check => {
                    let v = self.check_value();
                    g.row(r, "Check for updates", &v, t.dim, focused);
                    if let Some(job) = &self.job {
                        if job.progress.stage.load(std::sync::atomic::Ordering::Relaxed) == 1 {
                            g.meter(Rect::new(r.x + 12.0, r.bottom() - 3.5, r.w - 24.0, 2.0), job.progress.fraction(), t.signal);
                        }
                    }
                }
                ManualRow::Changes => {
                    let c = meta.changes.clone().unwrap_or_default();
                    let when = chrono::DateTime::from_timestamp(c.at, 0).map(|d| d.with_timezone(&chrono::Local).format("%b %-d").to_string().to_lowercase()).unwrap_or_default();
                    let v = format!("{} · {when}", c.count());
                    g.row(r, "What changed", &v, t.dim, focused);
                }
                ManualRow::Remove => {
                    let v = if self.removing { "enter again" } else { "" };
                    g.row(r, "Remove from this handheld", v, if self.removing { g.fault() } else { t.dim }, focused);
                }
            }
        }
        self.list.scrollbar(g, rows.len(), y, bottom);
        match rows.get(self.list.focus) {
            Some(ManualRow::Check) => g.hint_any(&["enter: ask FIRST for a newer one", "enter: check now"]),
            Some(ManualRow::Changes) => g.hint_any(&["enter: the rules the update changed", "enter: see them"]),
            _ => g.hint_any(&["enter twice: remove · bookmarks stay", "enter twice: remove"]),
        }
    }
}

impl Reader {
    /// Opened at an entry, wherever it is in the list.
    pub fn at_entry(mut self, entry: usize) -> Self {
        if let Some(at) = self.seq.iter().position(|&e| e == entry) {
            self.at = at;
            self.load();
        }
        self
    }
}
