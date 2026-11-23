//! Rulebook: FIRST's game manual on the handheld, for events where the
//! Wi-Fi is bad. Downloaded from FIRST on first open (never in the image),
//! read into its sections and rules, and kept in `/data/rulebook/` with an
//! index to search it (`store`). Its chapters to read, a rule's ID to jump
//! to ("G101"), bookmarks, search with the matches marked, and Ask: a
//! question answered from the rules the search finds, by Claude when online
//! or the on-device model when not, the rules cited shown under it.
//!
//! At rest it asks for no frames: only a download, an update check or an
//! answer under way has it looked at, a few times a second.

pub mod ask;
pub mod fetch;
mod pages;
pub mod parse;
pub mod sample;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
pub mod store;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

pub use pages::{Asking, Entries, ManualPage, Reader, Searching};
use store::{Book, Item};

/// How often a page with work under way is looked at.
const POLL: Duration = Duration::from_millis(150);

// -------------------------------------------------------------- bookmarks ---

fn bookmarks_path() -> PathBuf {
    store::dir().join("bookmarks")
}

/// A rule's bookmark is its ID; a section's, its number after "§".
pub fn mark_key(item: &Item) -> String {
    if item.is_rule() {
        item.id.clone()
    } else {
        format!("§{}", item.id)
    }
}

pub fn bookmarks() -> Vec<String> {
    std::fs::read_to_string(bookmarks_path()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// Bookmark it, or take its bookmark away: whether it is bookmarked now.
pub fn toggle_bookmark(key: &str) -> bool {
    let mut marks = bookmarks();
    let now = if let Some(at) = marks.iter().position(|m| m == key) {
        marks.remove(at);
        false
    } else {
        marks.push(key.to_owned());
        true
    };
    std::fs::create_dir_all(store::dir()).ok();
    crate::sys::write_atomic(&bookmarks_path(), (marks.join("\n") + "\n").as_bytes()).ok();
    now
}

/// The bookmarked entries of a book, in the order they were bookmarked.
pub fn bookmarked(book: &Book) -> Vec<usize> {
    bookmarks().iter().filter_map(|k| book.items.iter().position(|i| mark_key(i) == *k)).collect()
}

// ---------------------------------------------------------------- drawing ---

/// A rule's headline without the asterisk that marks it evergreen (the
/// same from season to season), and whether it had one.
pub fn headline(item: &Item) -> (&str, bool) {
    match item.title.strip_prefix('*') {
        Some(rest) => (rest.trim_start(), true),
        None => (item.title.as_str(), false),
    }
}

/// A row for a rule or a section: its ID, quiet and mono, then its title,
/// and a value at the right if any.
pub fn entry_row(g: &mut Gfx, r: Rect, column: f32, id: &str, title: &str, value: &str, focused: bool) {
    let (t, ty) = (g.tokens, g.ty);
    // The row's own surface, its lift under the ring and the ring.
    g.row(r, "", "", t.dim, focused);
    let column = column.max(g.measure(id, ty.label) + 8.0);
    g.text_at(r.x + 10.0, r.cy(), Align::Left, id, ty.label, t.dim);
    let value_w = if value.is_empty() { 0.0 } else { g.measure(value, ty.label) + 8.0 };
    let x = r.x + 10.0 + column;
    let title = title.strip_prefix('*').map_or(title, str::trim_start);
    let shown = g.text.fit(title, &ty.body, (r.right() - 10.0 - value_w - x).max(20.0));
    g.text_at(x, r.cy(), Align::Left, &shown, ty.body, t.ink);
    if !value.is_empty() {
        g.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, t.dim);
    }
    if focused {
        g.copy_row = Some(format!("{id} {title}"));
    }
}

/// The width of the ID column for a list of IDs: the widest, and a gap.
pub fn id_column<'a>(g: &mut Gfx, ids: impl Iterator<Item = &'a str>) -> f32 {
    let style = g.ty.label;
    ids.map(|id| g.measure(id, style)).fold(0.0, f32::max) + 8.0
}

/// Words with the matches of a search marked: an ice chip behind each,
/// its word in the ice's own ink.
#[allow(clippy::too_many_arguments)]
pub fn marked_text(g: &mut Gfx, x: f32, cy: f32, text: &str, marks: &[(usize, usize)], style: bezel::TextStyle, color: bezel::Color, alpha: f32) {
    let t = g.tokens;
    g.text_alpha(x, cy, text, style, color, alpha);
    for &(a, b) in marks {
        if b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        let x0 = x + g.measure(&text[..a], style);
        let w = g.measure(&text[a..b], style);
        let h = style.size + 3.0;
        g.canvas.rrect(Rect::new(x0 - 2.0 + g.ox, cy - h * 0.5, w + 4.0, h), 3.0, t.ice, alpha);
        g.text_alpha(x0, cy, &text[a..b], style, t.on_ice, alpha);
    }
}

/// Where the words of a search are in a line.
pub fn marks_in(text: &str, terms: &[String]) -> Vec<(usize, usize)> {
    if terms.is_empty() {
        return Vec::new();
    }
    store::spans(text).filter(|(a, b)| store::matches(&text[*a..*b], terms)).collect()
}

// -------------------------------------------------------------------- home ---

thread_local! {
    static TILE: std::cell::RefCell<Option<(std::time::Instant, bool)>> = const { std::cell::RefCell::new(None) };
}

/// Home's tile: whether the manual is here.
pub fn tile() -> (String, Option<&'static str>) {
    let here = TILE.with(|cell| {
        let mut cell = cell.borrow_mut();
        match *cell {
            Some((at, here)) if at.elapsed() < Duration::from_secs(5) => here,
            _ => {
                let here = store::dir().join("manual.json").is_file();
                *cell = Some((std::time::Instant::now(), here));
                here
            }
        }
    });
    ((if here { "rebuilt · offline" } else { "get the manual" }).into(), None)
}

// --------------------------------------------------------------- the app ---

/// A download, or a look for a newer manual, on its own thread.
pub(crate) struct Job {
    /// None for the pictures' download, held still.
    rx: Option<Receiver<Result<fetch::Outcome, String>>>,
    pub progress: Arc<fetch::Progress>,
    /// A look on opening: the page carries on as it is while it runs.
    pub quiet: bool,
}

impl Job {
    pub fn start(again: bool, quiet: bool) -> Job {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(fetch::Progress::default());
        let p = progress.clone();
        std::thread::Builder::new()
            .name("rulebook".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                tx.send(fetch::fetch(&store::dir(), again, &p)).ok();
            })
            .ok();
        Job { rx: Some(rx), progress, quiet }
    }

    /// A download held still part-way, for the pictures.
    pub fn still(fraction: f32) -> Job {
        let progress = Arc::new(fetch::Progress::default());
        progress.total.store(fetch::SIZE_GUESS, std::sync::atomic::Ordering::Relaxed);
        progress.bytes.store((fetch::SIZE_GUESS as f32 * fraction) as u64, std::sync::atomic::Ordering::Relaxed);
        progress.stage.store(1, std::sync::atomic::Ordering::Relaxed);
        Job { rx: None, progress, quiet: false }
    }

    /// The outcome, once there is one.
    pub fn poll(&self) -> Option<Result<fetch::Outcome, String>> {
        match self.rx.as_ref()?.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the download stopped".into())),
        }
    }

    /// The value its row shows.
    pub fn value(&self) -> String {
        let p = &self.progress;
        match p.stage.load(std::sync::atomic::Ordering::Relaxed) {
            0 => "asking FIRST…".into(),
            1 => {
                let mb = |b: u64| b as f64 / 1_000_000.0;
                format!("{:.1} of {:.1} MB", mb(p.bytes.load(std::sync::atomic::Ordering::Relaxed)), mb(p.total.load(std::sync::atomic::Ordering::Relaxed)))
            }
            _ => "reading the rules…".into(),
        }
    }
}

/// What the words of an update say: "3 rules changed, 1 new".
pub fn changes_words(c: &store::Changes) -> String {
    let mut parts = Vec::new();
    if !c.changed.is_empty() {
        parts.push(format!("{} changed", c.changed.len()));
    }
    if !c.added.is_empty() {
        parts.push(format!("{} new", c.added.len()));
    }
    if !c.removed.is_empty() {
        parts.push(format!("{} removed", c.removed.len()));
    }
    parts.join(", ")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Search,
    Ask,
    Bookmarks,
    Chapter(usize),
    Manual,
}

/// The app: the manual's chapters, with search, ask and bookmarks above
/// them; with no manual yet, getting it.
pub struct Rulebook {
    book: Option<Rc<Book>>,
    list: ListState,
    job: Option<Job>,
    problem: Option<String>,
    marks: usize,
    who: ask::Who,
    /// Something arrived from a thread: drawn at the next look.
    fresh: bool,
    shown_progress: String,
}

impl Rulebook {
    pub fn new() -> Self {
        let mut page = Self::waiting();
        match &page.book {
            // On first open, the manual is fetched at once.
            None => page.job = Some(Job::start(false, false)),
            // And a kept one looked at again, now and then, when online.
            Some(book) if chrono::Utc::now().timestamp() - book.meta.checked > fetch::CHECK_EVERY && fetch::live() => page.job = Some(Job::start(false, true)),
            _ => {}
        }
        page
    }

    /// The app as it opens, before any download starts (the pictures).
    pub fn waiting() -> Self {
        let book = Book::open(&store::dir()).ok().map(Rc::new);
        let marks = book.as_ref().map_or(0, |b| bookmarked(b).len());
        Rulebook { book, list: ListState::default(), job: None, problem: None, marks, who: ask::who(), fresh: false, shown_progress: String::new() }
    }

    /// The pictures' downloading page, held part-way.
    pub fn downloading(fraction: f32) -> Self {
        let mut page = Self::waiting();
        page.book = None;
        page.job = Some(Job::still(fraction));
        page
    }

    fn rows(&self) -> Vec<Row> {
        let Some(book) = &self.book else { return Vec::new() };
        let mut rows = vec![Row::Search, Row::Ask];
        if self.marks > 0 {
            rows.push(Row::Bookmarks);
        }
        rows.extend(book.chapters().into_iter().map(Row::Chapter));
        rows.push(Row::Manual);
        rows
    }

    fn reload(&mut self) {
        self.book = Book::open(&store::dir()).ok().map(Rc::new);
        self.marks = self.book.as_ref().map_or(0, |b| bookmarked(b).len());
        TILE.with(|t| *t.borrow_mut() = None);
    }

    fn open_row(&mut self, row: Row) -> Nav {
        let Some(book) = self.book.clone() else { return Nav::Stay };
        match row {
            Row::Search => Nav::Push(Box::new(Searching::new(book, ""))),
            Row::Ask => Nav::Push(Box::new(Asking::new(book))),
            Row::Bookmarks => Nav::Push(Box::new(Entries::bookmarks(book))),
            Row::Chapter(c) => Nav::Push(Box::new(Entries::chapter(book, c))),
            Row::Manual => Nav::Push(Box::new(ManualPage::new(book))),
        }
    }

    fn ask_value(&self) -> String {
        match &self.who {
            ask::Who::Claude(_) => crate::ai::Service::Claude.label().into(),
            ask::Who::Local => "on device".into(),
            ask::Who::Nobody => "rules only".into(),
        }
    }

    fn manual_value(&self, book: &Book) -> String {
        if let Some(job) = &self.job {
            return if job.progress.stage.load(std::sync::atomic::Ordering::Relaxed) == 0 { "checking…".into() } else { job.value() };
        }
        match &book.meta.changes {
            Some(c) if !c.is_empty() && c.at == book.meta.fetched => format!("{} changes", c.count()),
            _ => book.meta.modified.as_deref().and_then(fetch::short_date).unwrap_or_default(),
        }
    }

    /// With no manual: what it is, and the row that gets it.
    fn draw_get(&mut self, g: &mut Gfx) {
        let (t, ty) = (g.tokens, g.ty);
        let top = g.top();
        let width = W - 2.0 * PAD - 4.0;
        let words = match &self.problem {
            Some(why) => format!("Could not get it: {why}."),
            None => "FIRST’s 2026 game manual, REBUILT: downloaded from FIRST and kept on this handheld, to read, search and ask about where there is no Wi-Fi.".into(),
        };
        let lh = ty.caption.size + 4.0;
        let lines = crate::help::wrap(g, &words, ty.caption, width);
        for (k, line) in lines.iter().enumerate() {
            let color = if self.problem.is_some() && k == 0 { t.ink } else { t.dim };
            g.text_at(PAD + 2.0, top + 1.0 + (k as f32 + 0.5) * lh, Align::Left, line, ty.caption, color);
        }
        let rows_top = top + 6.0 + lines.len() as f32 * lh;
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        let value = match &self.job {
            Some(job) => job.value(),
            None if self.problem.is_some() => "try again".into(),
            None => format!("{:.1} MB", fetch::SIZE_GUESS as f64 / 1_000_000.0),
        };
        for (_, y) in self.list.layout(g, 1, rows_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            g.row(r, "Download the manual", &value, t.dim, true);
            if let Some(job) = &self.job {
                g.meter(Rect::new(r.x + 12.0, r.bottom() - 3.5, r.w - 24.0, 2.0), job.progress.fraction(), t.signal);
            }
        }
        self.list.scrollbar(g, 1, rows_top, bottom);
        if self.job.is_some() {
            g.hint_any(&["from firstinspires.org · about 1.2 MB", "from FIRST · 1.2 MB"]);
        } else {
            g.hint_any(&["enter: download from FIRST", "enter: download"]);
        }
    }
}

impl Screen for Rulebook {
    fn title(&self) -> &str {
        "Rulebook"
    }

    fn resume(&mut self) {
        // A bookmark made, or a manual replaced, on a page above.
        let stamp = self.book.as_ref().map(|b| b.meta.fetched);
        let fresh = Book::open(&store::dir()).ok().map(|b| b.meta.fetched);
        if stamp != fresh {
            self.reload();
        } else if let Some(book) = &self.book {
            self.marks = bookmarked(book).len();
        }
        self.who = ask::who();
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if self.book.is_none() {
            return match key {
                Key::Back => Nav::Pop,
                Key::Enter | Key::Right if self.job.is_none() => {
                    self.problem = None;
                    self.job = Some(Job::start(false, false));
                    Nav::Stay
                }
                _ => Nav::Stay,
            };
        }
        let rows = self.rows();
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right => match rows.get(self.list.focus) {
                Some(&row) => self.open_row(row),
                None => Nav::Stay,
            },
            Key::Char('/') => self.open_row(Row::Search),
            Key::Char('?') => self.open_row(Row::Ask),
            // A rule's ID, or words, typed straight away: the search, begun.
            Key::Char(c) if c.is_alphanumeric() => {
                let book = self.book.clone().expect("a manual");
                Nav::Push(Box::new(Searching::new(book, &c.to_string())))
            }
            other => {
                crate::screens::list_key(&mut self.list, other, rows.len());
                Nav::Stay
            }
        }
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(job) = &self.job else { return Nav::Stay };
        let value = job.value();
        if value != self.shown_progress {
            self.shown_progress = value;
            self.fresh = true;
        }
        let Some(outcome) = job.poll() else { return Nav::Stay };
        let quiet = job.quiet;
        self.job = None;
        self.fresh = true;
        match outcome {
            Ok(fetch::Outcome::Kept(meta)) => {
                self.reload();
                match meta.changes.filter(|c| !c.is_empty() && c.at == meta.fetched) {
                    Some(c) => *cx.say = Some(format!("manual updated · {}", changes_words(&c))),
                    None if !quiet => *cx.say = Some("the manual is on this handheld".into()),
                    None => {}
                }
            }
            Ok(fetch::Outcome::Current) => {}
            // A look on opening that found no internet says nothing.
            Err(_) if quiet => {}
            Err(why) => self.problem = Some(why),
        }
        Nav::Stay
    }

    fn ambient(&self) -> Option<Duration> {
        self.job.as_ref().map(|_| POLL)
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        std::mem::take(&mut self.fresh)
    }

    fn keeps_awake(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.quiet)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let Some(book) = self.book.clone() else {
            self.draw_get(g);
            return;
        };
        let t = g.tokens;
        let rows = self.rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), crate::gfx::H);
        let column = id_column(g, rows.iter().filter_map(|r| if let Row::Chapter(c) = r { Some(book.items[*c].id.as_str()) } else { None }));
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match rows[i] {
                Row::Search => g.row(r, "Search", "g101 · words", t.dim, focused),
                Row::Ask => {
                    let value = self.ask_value();
                    g.row(r, "Ask", &value, t.dim, focused)
                }
                Row::Bookmarks => g.row(r, "Bookmarks", &self.marks.to_string(), t.dim, focused),
                Row::Chapter(c) => {
                    let rules = book.contents(c).into_iter().filter(|&k| book.items[k].is_rule()).count();
                    let value = if rules > 0 { format!("{rules}") } else { String::new() };
                    let item = &book.items[c];
                    entry_row(g, r, column, &item.id, &item.title, &value, focused);
                }
                Row::Manual => {
                    let value = self.manual_value(&book);
                    let updated = book.meta.changes.as_ref().is_some_and(|c| !c.is_empty() && c.at == book.meta.fetched);
                    g.row(r, "Manual", &value, if updated { t.ink } else { t.dim }, focused);
                }
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
    }
}
