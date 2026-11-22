//! The Vault app's pages: the vault locked (or not made yet), its list, an
//! entry, the editor, the generator, importing and backing up.
//!
//! Every page is secret (Show key presses leaves its keys out), none is
//! kept among the recent apps (their cards are pictures of the screen), and
//! each gives way the moment the vault locks.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;

use bezel::{Align, Rect, TextStyle};
use detent::{role, Animated};
use zeroize::{Zeroize, Zeroizing};

use super::clip::{self, Sensitivity};
use super::entry::{Entry, Field, Kind};
use super::gen::{self, Recipe};
use super::import::{self, FileKind, Found};
use super::totp::Totp;
use super::{crypto, Outcome};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::lock::{Edit, Field as Capsule, Prompt, FIELD_H};
use crate::passcode::{self, Tries};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// What the island says when something is copied.
/// (Short enough for the island whole at the largest text.)
fn copied_words(what: &str) -> String {
    format!("{what} copied for {} s", clip::SECRET_FOR.as_secs())
}

/// What the vault says after it is made with a passcode shorter than this.
const SHORT_PASSCODE_UNDER: usize = 8;

const SHORT_PASSCODE: &str = "Your passcode is short. It keeps out someone at the keys, but someone with the card and a computer could try every passcode that short in minutes, and open the vault.\n\nA passcode of 8 characters or more, or a few words, keeps the vault safe even then. Settings\u{a0}› Lock\u{a0}screen\u{a0}› Change\u{a0}passcode moves the vault to the new one as it is set.";

/// The first of `texts` (fullest first) whose lines at `style` fit `width`
/// in `lines` lines; the last, cut to them, if none does.
fn fit_lines(g: &mut Gfx, texts: &[&str], style: TextStyle, width: f32, lines: usize) -> Vec<String> {
    for text in texts {
        let wrapped = crate::help::wrap(g, text, style, width);
        if wrapped.len() <= lines {
            return wrapped;
        }
    }
    let mut wrapped = crate::help::wrap(g, texts.last().copied().unwrap_or(""), style, width);
    wrapped.truncate(lines);
    wrapped
}

/// Lines of caption text centred under `cy`, one after another.
fn caption_lines(g: &mut Gfx, lines: &[String], cy: f32, align: Align) {
    let t = g.tokens;
    let style = g.ty.caption;
    let step = style.size + 4.0;
    for (i, line) in lines.iter().enumerate() {
        let x = if align == Align::Center { W * 0.5 } else { PAD + 4.0 };
        g.text_at(x, cy + i as f32 * step, align, line, style, t.dim);
    }
}

/// A code as it is read out: "482 913", "4829 1307".
pub fn grouped(code: &str) -> String {
    let n = code.len();
    let cut = if n >= 8 { n / 2 } else { 3.min(n) };
    format!("{} {}", &code[..cut], &code[cut..])
}

fn unix_f64() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The value as a row shows it: cut to fit beside its label.
fn fit_value(g: &mut Gfx, value: &str, room: f32) -> String {
    let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
    let first = value.lines().next().unwrap_or("");
    let more = value.contains('\n');
    let text = if more { format!("{first} …") } else { first.to_owned() };
    g.text.fit(&text, &style, room.max(24.0))
}

/// Masked: as many dots as never tells a length.
const MASK: &str = "••••••••••";

// ------------------------------------------------------------- an input ---

/// What a key did to an input.
enum Typed {
    More,
    Done,
    Cancelled,
}

/// A line typed into, as Settings' fields are, but for secrets: shown as
/// dots unless Tab shows it, held in memory that is wiped, and sized once so
/// typing never leaves a copy behind as it grows.
struct Input {
    label: &'static str,
    text: Zeroizing<String>,
    limit: usize,
    masked: bool,
    shown: bool,
}

impl Input {
    fn new(label: &'static str, text: &str, limit: usize, masked: bool) -> Input {
        let mut s = String::with_capacity((limit * 4).max(text.len()));
        s.push_str(text);
        Input { label, text: Zeroizing::new(s), limit, masked, shown: false }
    }

    fn key(&mut self, key: Key) -> Typed {
        match key {
            Key::Char(c) if !c.is_control() && self.text.chars().count() < self.limit => {
                if self.text.len() + c.len_utf8() <= self.text.capacity() {
                    self.text.push(c);
                }
            }
            Key::Backspace | Key::Delete => {
                self.text.pop();
            }
            Key::Tab if self.masked => self.shown = !self.shown,
            Key::Enter => return Typed::Done,
            Key::Back | Key::Leave => return Typed::Cancelled,
            _ => {}
        }
        Typed::More
    }

    fn height(g: &Gfx) -> f32 {
        g.ty.caption.size + g.ty.label.size + 20.0
    }

    /// At `y`, across the page, ringed, with its cursor.
    fn draw(&self, g: &mut Gfx, y: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let field = Rect::new(PAD, y, W - 2.0 * PAD, Self::height(g));
        g.surface(field, g.radius(14.0).min(field.h * 0.5), t.surface2);
        let mut label = self.label.to_owned();
        if self.masked {
            label.push_str(if self.shown { " · tab: hide" } else { " · tab: show" });
        }
        let label = g.text.fit(&label, &ty.caption, field.w - 20.0);
        g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, &label, ty.caption, t.dim);
        let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
        let room = field.w - 26.0;
        let masked: String;
        let whole: &str = if self.masked && !self.shown {
            masked = "•".repeat(self.text.chars().count());
            &masked
        } else {
            self.text.as_str()
        };
        // Only its last line, and its end, where the typing is.
        let mut shown: &str = whole.rsplit('\n').next().unwrap_or("");
        while g.measure(shown, ty.label) > room {
            let mut chars = shown.chars();
            chars.next();
            shown = chars.as_str();
        }
        let w = g.text_at(field.x + 10.0, cy, Align::Left, shown, ty.label, t.ink);
        g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
        g.focus_ring(field, g.radius(14.0).min(field.h * 0.5));
    }
}

// --------------------------------------------------- the passcode field ---

/// The capsule on a tonal plate (glass needs something to bend), its top
/// at `y`, as Settings' passcode page has it.
fn draw_capsule(g: &mut Gfx, field: &mut Capsule, y: f32, prompt: &Prompt) {
    let t = g.tokens;
    let plate = Rect::new(PAD + 6.0, y - 6.0, W - 2.0 * PAD - 12.0, FIELD_H + 12.0);
    g.surface(plate, plate.h * 0.5, t.surface1);
    field.draw(g, W * 0.5, y, plate.w - 12.0, prompt);
}

/// Where the capsule sits: its plate just above the hint line.
fn capsule_y(g: &Gfx) -> f32 {
    (g.above_hint() - 6.0 - FIELD_H - 2.0).round()
}

// ---------------------------------------------------------------- Vault ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// No passcode set: one comes first.
    NeedsPasscode,
    /// A passcode, no vault yet.
    Create,
    Locked,
    Open,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    New,
    Entry(u64),
    Generator,
    Tools,
    Lock,
}

/// What the list shows of an entry: never more than its id, name and summary.
struct Line {
    id: u64,
    name: String,
    summary: String,
}

/// The app: the vault locked or open, and its list.
pub struct Vault {
    field: Capsule,
    working: Option<(Receiver<Outcome>, bool)>,
    /// A passcode shorter than this made the vault: the island says so.
    made_short: bool,
    wrong: Option<String>,
    tries: Tries,
    shown_wait: u64,
    list: ListState,
    search: Option<Input>,
    query: String,
    passcode_set: bool,
    exists: bool,
    /// Wrapped under a passcode other than the one set now.
    earlier: bool,
    seen: (u64, u64),
    /// The seconds before it locks by itself, as last drawn (in its last minute).
    locks_drawn: Option<u64>,
}

impl Default for Vault {
    fn default() -> Self {
        Self::new()
    }
}

impl Vault {
    pub fn new() -> Vault {
        let mut v = Vault {
            field: Capsule::new(),
            working: None,
            made_short: false,
            wrong: None,
            tries: Tries::load(passcode::unix_now()),
            shown_wait: 0,
            list: ListState::default(),
            search: None,
            query: String::new(),
            passcode_set: false,
            exists: false,
            earlier: false,
            seen: (u64::MAX, u64::MAX),
            locks_drawn: None,
        };
        v.refresh();
        v
    }

    /// Read again what the vault and the passcode are, when either changed.
    fn refresh(&mut self) -> bool {
        let now = (passcode::generation(), super::generation());
        if now == self.seen {
            return false;
        }
        self.seen = now;
        self.passcode_set = passcode::is_set();
        self.exists = super::exists();
        self.earlier = self.exists && self.passcode_set && !super::is_open() && !super::under_current_passcode();
        true
    }

    fn state(&self) -> State {
        if super::is_open() {
            State::Open
        } else if self.exists {
            State::Locked
        } else if self.passcode_set {
            State::Create
        } else {
            State::NeedsPasscode
        }
    }

    fn wait(&self) -> u64 {
        if matches!(self.state(), State::Locked | State::Create) {
            self.tries.wait(passcode::unix_now())
        } else {
            0
        }
    }

    fn lines(&self) -> Vec<Line> {
        super::with(|entries| entries.iter().filter(|e| e.matches(&self.query)).map(|e| Line { id: e.id, name: e.name().to_owned(), summary: e.summary() }).collect()).unwrap_or_default()
    }

    fn rows(&self, lines: &[Line]) -> Vec<Row> {
        let mut rows = vec![Row::New];
        rows.extend(lines.iter().map(|l| Row::Entry(l.id)));
        if self.query.is_empty() {
            rows.extend([Row::Generator, Row::Tools, Row::Lock]);
        }
        rows
    }

    fn submit(&mut self) {
        if self.field.len() < passcode::MIN_LEN {
            if !self.field.is_empty() {
                self.wrong = Some(format!("at least {} characters", passcode::MIN_LEN));
                self.field.wrong(false);
            }
            return;
        }
        let typed = Zeroizing::new(self.field.take());
        let making = self.state() == State::Create;
        self.made_short = making && typed.chars().count() < SHORT_PASSCODE_UNDER;
        let rx = if making { super::begin_create(typed) } else { super::begin_unlock(typed) };
        self.working = Some((rx, making));
    }

    fn open_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(input) = &mut self.search {
            match input.key(key) {
                Typed::Done => {
                    self.query = input.text.trim().to_owned();
                    self.search = None;
                }
                Typed::Cancelled => {
                    self.search = None;
                    self.query.clear();
                }
                Typed::More => self.query = input.text.trim().to_owned(),
            }
            self.list.focus = 0;
            return Nav::Stay;
        }
        let lines = self.lines();
        let rows = self.rows(&lines);
        let focused = rows.get(self.list.focus).copied();
        match key {
            Key::Back if !self.query.is_empty() => {
                self.query.clear();
                self.list.focus = 0;
            }
            Key::Back => return Nav::Pop,
            Key::Char('/') => self.search = Some(Input::new("search titles and usernames", &self.query, 40, false)),
            Key::Char('n') => return Nav::Push(Box::new(EditPage::new(None))),
            Key::Char('g') => return Nav::Push(Box::new(GeneratorPage::new(false))),
            Key::Char('i') => return Nav::Push(Box::new(ToolsPage::new())),
            Key::Char('l') => {
                super::lock();
                *cx.say = Some("vault locked".into());
            }
            Key::Enter | Key::Right => match focused {
                Some(Row::New) => return Nav::Push(Box::new(EditPage::new(None))),
                Some(Row::Entry(id)) => return Nav::Push(Box::new(EntryPage::new(id))),
                Some(Row::Generator) => return Nav::Push(Box::new(GeneratorPage::new(false))),
                Some(Row::Tools) => return Nav::Push(Box::new(ToolsPage::new())),
                Some(Row::Lock) => {
                    super::lock();
                    *cx.say = Some("vault locked".into());
                }
                None => {}
            },
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn draw_open(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let searching = self.search.is_some();
        if let Some(input) = &self.search {
            input.draw(g, g.top());
        }
        let lines = self.lines();
        let rows = self.rows(&lines);
        let top = if searching { g.top() + Input::height(g) + 6.0 } else { g.top() };
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = !searching && i == self.list.focus;
            match rows[i] {
                Row::New => g.row(r, "New item", "n", t.dim, focused),
                Row::Entry(id) => {
                    let Some(line) = lines.iter().find(|l| l.id == id) else { continue };
                    let summary = fit_value(g, &line.summary, r.w * 0.46);
                    g.row(r, &line.name, &summary, t.dim, focused);
                }
                Row::Generator => g.row(r, "Password generator", "g ›", t.dim, focused),
                Row::Tools => g.row(r, "Import and back up", "i ›", t.dim, focused),
                Row::Lock => {
                    // In its last minute, when it will lock by itself.
                    let value = match super::locks_in() {
                        Some(s) if s < 60 => format!("in {} · l", passcode::wait_text(s)),
                        _ => "l".to_owned(),
                    };
                    g.row(r, "Lock now", &value, t.dim, focused);
                }
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let count = lines.len();
        if searching {
            let found = format!("{count} found");
            g.hint_any(&[&format!("{found} · enter: done · esc: cancel"), &format!("{found} · esc: cancel")]);
        } else if count == 0 && !self.query.is_empty() {
            g.hint_any(&[&format!("nothing has \"{}\" · esc: everything", self.query), "nothing found · esc: everything"]);
        } else {
            match rows.get(self.list.focus) {
                Some(Row::Entry(_)) => g.hint_any(&["enter: open · /: search · n: new · l: lock", "enter: open · /: search · l: lock", "enter: open · l: lock"]),
                Some(Row::New) if count == 0 => g.hint_any(&["enter: your first item · i: import · l: lock", "enter: first item · l: lock"]),
                _ => g.hint_any(&["enter: open · /: search · n: new · l: lock", "enter: open · l: lock"]),
            }
        }
    }

    fn draw_locked(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let state = self.state();
        let top = g.top();
        let field_y = capsule_y(g);
        // The vault's mark, on a tonal disc.
        let room = field_y - 6.0 - top;
        let lines = match state {
            // (No-break spaces keep a last word from a line of its own.)
            State::Create => fit_lines(
                g,
                &[
                    "Passwords, codes and PINs, sealed on the card and opened with your passcode alone. Forget it and the vault is lost: no one can open it, not even over\u{a0}SSH.",
                    "Sealed on the card, opened with your passcode alone. Forget it and the vault is lost for\u{a0}good.",
                    "Opened with your passcode alone. Forget it, lose\u{a0}the\u{a0}vault.",
                ],
                ty.caption,
                W - 2.0 * PAD - 8.0,
                ((room - 30.0) / (ty.caption.size + 4.0)).floor().max(1.0) as usize,
            ),
            _ if self.earlier => fit_lines(g, &["Locked with the passcode it had before yours was set again over SSH: type\u{a0}that\u{a0}one.", "Locked with an earlier passcode: type\u{a0}that\u{a0}one."], ty.caption, W - 2.0 * PAD - 8.0, 2),
            _ => fit_lines(g, &["Forget the passcode and the vault is lost: no one can open it, not even over\u{a0}SSH.", "Forget the passcode, lose\u{a0}the\u{a0}vault."], ty.caption, W - 2.0 * PAD - 8.0, 2),
        };
        let line_h = ty.caption.size + 4.0;
        let words_h = lines.len() as f32 * line_h;
        let disc = 15.0;
        let block = disc * 2.0 + 8.0 + words_h;
        let y0 = top + ((room - block) * 0.5).max(0.0);
        let cy = y0 + disc;
        g.dot(W * 0.5, cy, disc, t.surface1);
        let icon = if self.working.is_some() { "lock_open" } else { "encrypted" };
        g.icon(icon, true, W * 0.5, cy, 18.0, if state == State::Create { t.signal } else { t.ink });
        caption_lines(g, &lines, y0 + disc * 2.0 + 8.0 + ty.caption.size * 0.5, Align::Center);
        let wait = self.wait();
        let prompt = if wait > 0 {
            Prompt::Wait(wait)
        } else if let Some((_, making)) = &self.working {
            if *making {
                Prompt::Asks("making the vault".into())
            } else {
                Prompt::Checking
            }
        } else if let Some(words) = &self.wrong {
            Prompt::Wrong(words.clone())
        } else if state == State::Create {
            Prompt::Asks("type your passcode to make it".into())
        } else {
            Prompt::Asks("type your passcode".into())
        };
        draw_capsule(g, &mut self.field, field_y, &prompt);
        if state == State::Create {
            g.hint_any(&["enter: make the vault · esc: back", "enter: make it · esc: back"]);
        } else {
            g.hint_any(&["enter: open · esc: clear, then back", "enter: open · esc: back"]);
        }
    }

    fn draw_needs_passcode(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let row_h = g.row_h();
        let row_y = g.above_hint() - 4.0 - row_h;
        let room = row_y - 6.0 - top;
        let lines = fit_lines(
            g,
            &[
                "The vault opens with your lock screen passcode, and nothing else. Set one first: here, or in Settings › Lock\u{a0}screen.",
                "The vault opens with your passcode alone. Set\u{a0}one\u{a0}first.",
            ],
            ty.caption,
            W - 2.0 * PAD - 8.0,
            ((room - 38.0) / (ty.caption.size + 4.0)).floor().max(1.0) as usize,
        );
        let disc = 15.0;
        let block = disc * 2.0 + 8.0 + lines.len() as f32 * (ty.caption.size + 4.0);
        let y0 = top + ((room - block) * 0.5).max(0.0);
        g.dot(W * 0.5, y0 + disc, disc, t.surface1);
        g.icon("encrypted", true, W * 0.5, y0 + disc, 18.0, t.dim);
        caption_lines(g, &lines, y0 + disc * 2.0 + 8.0 + ty.caption.size * 0.5, Align::Center);
        g.row(Rect::new(PAD, row_y, W - 2.0 * PAD, row_h), "Set a passcode", "›", t.ink, true);
        g.hint("enter: set a passcode · esc: back");
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        // The app closed: locked at once.
        super::lock();
    }
}

impl Screen for Vault {
    fn title(&self) -> &str {
        "Vault"
    }

    fn wants_text(&self) -> bool {
        match self.state() {
            State::Create | State::Locked => true,
            State::Open => self.search.is_some(),
            State::NeedsPasscode => false,
        }
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        self.working.is_some() || self.field.moving() || super::is_open()
    }

    fn set_aside(&mut self) {
        // The app was left: locked at once (what was copied stays its 30 s).
        super::lock();
        self.search = None;
        self.query.clear();
    }

    fn resume(&mut self) {
        self.seen = (u64::MAX, u64::MAX);
        self.refresh();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        match self.state() {
            State::NeedsPasscode => match key {
                Key::Back => Nav::Pop,
                Key::Enter | Key::Right => Nav::Push(Box::new(crate::lockpages::PasscodeFlow::new(crate::lockpages::Purpose::Set))),
                _ => Nav::Stay,
            },
            State::Create | State::Locked => {
                if self.working.is_some() {
                    return Nav::Stay;
                }
                if key == Key::Back && self.field.is_empty() {
                    return Nav::Pop;
                }
                if self.wait() > 0 {
                    return Nav::Stay;
                }
                match self.field.key(key) {
                    Edit::Typed => self.wrong = None,
                    Edit::Submit => self.submit(),
                    Edit::Nothing => {}
                }
                Nav::Stay
            }
            State::Open => self.open_key(key, cx),
        }
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(why) = super::take_write_error() {
            *cx.say = Some(why);
        }
        let Some((rx, making)) = &self.working else { return Nav::Stay };
        let making = *making;
        let outcome = match rx.try_recv() {
            Ok(o) => o,
            Err(TryRecvError::Empty) => return Nav::Stay,
            Err(TryRecvError::Disconnected) => Outcome::Failed("could not open it".into()),
        };
        self.working = None;
        match outcome {
            Outcome::Opened(opened) => {
                opened.install();
                self.tries.passed();
                self.wrong = None;
                self.list = ListState::default();
                self.refresh();
                if making {
                    *cx.say = Some("vault made".into());
                    // Made with a short passcode: what that means, once.
                    if self.made_short {
                        return Nav::Push(Box::new(TextPage::words("Passcode", SHORT_PASSCODE)));
                    }
                }
            }
            Outcome::Wrong => {
                self.tries.failed(passcode::unix_now());
                self.wrong = Some(match self.tries.before_wait() {
                    Some(1) => "wrong · 1 try before a wait".to_owned(),
                    Some(n) => format!("wrong · {n} tries before a wait"),
                    None if making => "not your passcode".to_owned(),
                    None => "not that one".to_owned(),
                });
                self.field.wrong(cx.settings.calm);
            }
            Outcome::Failed(why) => {
                self.wrong = Some(why);
                self.field.wrong(cx.settings.calm);
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.refresh();
        let wait = self.wait();
        changed |= wait != self.shown_wait;
        self.shown_wait = wait;
        let locks = super::locks_in().filter(|s| *s < 60);
        changed |= locks != self.locks_drawn;
        self.locks_drawn = locks;
        self.field.tick(dt, false) | self.working.is_some() | changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        match self.state() {
            State::Open => self.draw_open(g),
            State::Locked | State::Create => self.draw_locked(g),
            State::NeedsPasscode => self.draw_needs_passcode(g),
        }
    }
}

// ------------------------------------------------------------ the entry ---

/// Text shown whole, a line at a time: a note, or what a page explains.
struct TextView {
    first: usize,
    scroll: Animated,
    lines: usize,
    fit: usize,
}

impl TextView {
    fn new() -> TextView {
        TextView { first: 0, scroll: Animated::new(0.0, role::hold()).with_rest(0.02), lines: 0, fit: 1 }
    }

    fn key(&mut self, key: Key) -> bool {
        let most = self.lines.saturating_sub(self.fit);
        match key {
            Key::Up if self.first > 0 => self.first -= 1,
            Key::Down if self.first < most => self.first += 1,
            Key::PageUp => self.first = self.first.saturating_sub(self.fit),
            Key::PageDown => self.first = (self.first + self.fit).min(most),
            Key::Home => self.first = 0,
            Key::End => self.first = most,
            _ => return false,
        }
        true
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }

    /// `text` wrapped in `r`, scrolled to where it is.
    fn draw(&mut self, g: &mut Gfx, text: &str, r: Rect, style: TextStyle) {
        let t = g.tokens;
        let line_h = style.size + 5.0;
        let mut lines = Vec::new();
        for para in text.split('\n') {
            if para.trim().is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(crate::help::wrap(g, para, style, r.w));
            }
        }
        self.lines = lines.len();
        self.fit = ((r.h + 2.0) / line_h).floor().max(1.0) as usize;
        self.first = self.first.min(self.lines.saturating_sub(self.fit));
        let want = self.first as f32;
        if (want - self.scroll.target()).abs() > 0.01 {
            self.scroll.set(want);
            g.started_moving();
        }
        if g.calm {
            self.scroll.jump(want);
        }
        let s = self.scroll.value();
        // At rest, whole lines only; on the move, clipped to the page.
        let at_rest = (s - want).abs() < 0.02;
        g.canvas.set_clip(Rect::new(0.0, r.y - 2.0, W, r.h + 4.0));
        for (i, line) in lines.iter().enumerate() {
            let y = r.y + (i as f32 - s) * line_h + line_h * 0.5;
            if y < r.y - line_h || y > r.bottom() + line_h || (at_rest && (i < self.first || i >= self.first + self.fit)) {
                continue;
            }
            g.text_at(r.x, y, Align::Left, line, style, t.ink);
        }
        g.canvas.clear_clip();
        if self.lines > self.fit {
            let track = r.h;
            let thumb = (track * self.fit as f32 / self.lines as f32).max(10.0);
            let y = r.y + (track - thumb) * (s / (self.lines - self.fit) as f32).clamp(0.0, 1.0);
            let faint = t.faint;
            g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
        }
    }
}

/// One entry: its fields, the password masked until Space shows it, the
/// two-factor code with its countdown ring; Enter copies what is focused.
pub struct EntryPage {
    id: u64,
    entry: Option<Entry>,
    totp: Option<Totp>,
    seen: u64,
    list: ListState,
    revealed: bool,
    deleting: bool,
    text: TextView,
    /// The code shown, and the one before it while it gives way.
    code: (String, String),
    flip: Animated,
    /// How full the ring was last drawn, and its length around: it is
    /// drawn again only once its end has moved most of a pixel.
    ring: (f32, f32),
    kind_title: &'static str,
}

impl EntryPage {
    pub fn new(id: u64) -> EntryPage {
        let mut page = EntryPage {
            id,
            entry: None,
            totp: None,
            seen: u64::MAX,
            list: ListState::default(),
            revealed: false,
            deleting: false,
            text: TextView::new(),
            code: (String::new(), String::new()),
            flip: Animated::new(1.0, role::smooth()).with_rest(0.01),
            ring: (-1.0, 0.0),
            kind_title: "Item",
        };
        page.load();
        page
    }

    fn load(&mut self) {
        if self.seen == super::generation() {
            return;
        }
        self.seen = super::generation();
        self.entry = super::get(self.id);
        self.totp = self.entry.as_ref().filter(|e| !e.totp.is_empty()).and_then(|e| Totp::parse(&e.totp).ok());
        self.kind_title = self.entry.as_ref().map_or("Item", |e| e.kind.title());
    }

    fn fields(&self) -> Vec<Field> {
        let Some(e) = &self.entry else { return Vec::new() };
        e.kind.fields().iter().copied().filter(|f| *f != Field::Title).filter(|f| match f {
            Field::Totp => self.totp.is_some(),
            _ => !e.get(*f).is_empty(),
        }).collect()
    }

    fn copy(&self, field: Field, cx: &mut Cx) {
        let Some(e) = &self.entry else { return };
        let kind = e.kind;
        match field {
            Field::Totp => {
                if let Some(t) = &self.totp {
                    let code = Zeroizing::new(t.code(unix_f64() as i64));
                    clip::copy(&code, Sensitivity::Secret);
                    *cx.say = Some(copied_words("code"));
                }
            }
            Field::Notes => {
                clip::copy(&e.notes, Sensitivity::Secret);
                *cx.say = Some(copied_words(kind.label(field)));
            }
            f => {
                clip::copy(e.get(f), Sensitivity::Secret);
                *cx.say = Some(copied_words(kind.label(f)));
            }
        }
    }

    fn draw_totp(&mut self, g: &mut Gfx, r: Rect, label: &str, focused: bool) {
        let t = g.tokens;
        let ty = g.ty;
        let Some(totp) = &self.totp else { return };
        let now = unix_f64();
        let code = grouped(&totp.code(now as i64));
        if code != self.code.0 {
            let before = std::mem::replace(&mut self.code.0, code.clone());
            self.code.1 = before;
            if self.code.1.is_empty() || g.calm {
                self.flip.jump(1.0);
            } else {
                self.flip.jump(0.0);
                self.flip.set(1.0);
                g.started_moving();
            }
        }
        g.row(r, label, "", t.dim, focused);
        let bezel = g.theme == crate::look::Theme::Bezel;
        let style = if bezel { ty.body } else { ty.label };
        let (left, frac) = totp.left(now);
        let late = left <= 5;
        let color = if late { g.fault() } else { t.ink };
        let w = g.measure(&self.code.0, style);
        let x = r.right() - 10.0 - w;
        // A new code rises into place as the old one lifts away and fades
        // (no clip of its own here: the list's clip holds while its rows draw).
        let k = self.flip.value().clamp(0.0, 1.0);
        if k < 0.999 && !self.code.1.is_empty() {
            let ow = g.measure(&self.code.1, style);
            g.text_alpha(r.right() - 10.0 - ow, r.cy() - 4.0 * k, &self.code.1, style, t.dim, (1.0 - k) * (1.0 - k));
        }
        g.text_alpha(x, r.cy() + 4.0 * (1.0 - k), &self.code.0, style, color, k * k);
        // The ring: what is left of the code's 30 seconds, draining.
        let radius = (r.h * 0.5 - 6.0).max(4.0);
        let cx = x - 8.0 - radius;
        g.canvas.arc(cx + g.ox, r.cy(), radius, 2.0, 0.0, std::f32::consts::TAU, t.meter, 1.0);
        g.canvas.arc(cx + g.ox, r.cy(), radius, 2.0, std::f32::consts::TAU * (1.0 - frac), std::f32::consts::TAU * frac, if late { g.fault() } else { t.signal }, 1.0);
        self.ring = (frac, radius * std::f32::consts::TAU);
    }
}

impl Screen for EntryPage {
    fn title(&self) -> &str {
        self.kind_title
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        true
    }

    fn set_aside(&mut self) {
        self.revealed = false;
    }

    fn resume(&mut self) {
        // Back from the editor or a note: masked again.
        self.revealed = false;
        self.seen = u64::MAX;
        self.load();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        let armed = std::mem::take(&mut self.deleting);
        let Some(kind) = self.entry.as_ref().map(|e| e.kind) else { return Nav::Pop };
        if kind == Kind::Note {
            if self.text.key(key) {
                return Nav::Stay;
            }
        }
        let fields = self.fields();
        let focused = fields.get(self.list.focus).copied();
        match key {
            Key::Back => return Nav::Pop,
            Key::Char(' ') => {
                if fields.contains(&Field::Password) {
                    self.revealed = !self.revealed;
                }
            }
            Key::Char('e') => return Nav::Push(Box::new(EditPage::new(Some(self.id)))),
            Key::Char('d') => {
                if armed {
                    let id = self.id;
                    let name = self.entry.as_ref().map(|e| e.name().to_lowercase()).unwrap_or_default();
                    match super::change(|entries| entries.retain(|e| e.id != id)) {
                        Ok(()) => *cx.say = Some(format!("deleted {name}")),
                        Err(why) => *cx.say = Some(why),
                    }
                    return Nav::Pop;
                }
                self.deleting = true;
                *cx.say = Some("d again deletes it".into());
            }
            Key::Enter => match (kind, focused) {
                (Kind::Note, _) => self.copy(Field::Notes, cx),
                (_, Some(Field::Notes)) => return Nav::Push(Box::new(TextPage::notes(self.id))),
                (_, Some(f)) => self.copy(f, cx),
                (_, None) => {}
            },
            Key::Right if focused == Some(Field::Notes) => return Nav::Push(Box::new(TextPage::notes(self.id))),
            other => crate::screens::list_key(&mut self.list, other, fields.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        if !super::is_open() {
            return Nav::Pop;
        }
        self.load();
        if self.entry.is_none() {
            return Nav::Pop;
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.flip.tick(dt) | self.text.tick(dt);
        if let Some(t) = &self.totp {
            // Drawn again as the ring's end moves most of a pixel, or the code changes.
            let now = unix_f64();
            let (_, frac) = t.left(now);
            moving |= (frac - self.ring.0).abs() * self.ring.1 >= 0.8 || grouped(&t.code(now as i64)) != self.code.0;
        }
        moving | (self.seen != super::generation())
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.load();
        let Some(e) = self.entry.clone() else { return };
        let top = g.top();
        // Its name, with what it is.
        let head_cy = top + ty.name.size * 0.5 + 1.0;
        g.icon(e.kind.icon(), true, PAD + 8.0, head_cy, 14.0, t.dim);
        let name = g.text.fit(e.name(), &ty.name, W - 2.0 * PAD - 24.0);
        g.text_at(PAD + 20.0, head_cy, Align::Left, &name, ty.name, t.ink);
        let list_top = (top + ty.name.size + 9.0).round();
        let bottom = g.above_hint();
        if e.kind == Kind::Note {
            let body = Rect::new(PAD + 4.0, list_top + 2.0, W - 2.0 * PAD - 12.0, bottom - list_top - 4.0);
            if e.notes.trim().is_empty() {
                g.text_at(PAD + 4.0, list_top + 10.0, Align::Left, "an empty note", ty.caption, t.dim);
            } else {
                self.text.draw(g, &e.notes, body, ty.body);
            }
            g.take_foot(bottom);
            g.hint_any(&["enter: copy · e: edit · d: delete · esc: back", "enter: copy · e: edit · d: delete", "e: edit · d: delete"]);
            return;
        }
        let fields = self.fields();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, fields.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let f = fields[i];
            let label = e.kind.label(f);
            let focused = i == self.list.focus;
            match f {
                Field::Totp => self.draw_totp(g, r, label, focused),
                Field::Password if !self.revealed => g.row(r, label, MASK, t.dim, focused),
                f => {
                    let label_w = g.measure(label, if g.theme == crate::look::Theme::Bezel { ty.label } else { ty.body });
                    let value = fit_value(g, e.get(f), r.w - label_w - 34.0);
                    g.row(r, label, &value, if f == Field::Password { t.ink } else { t.dim }, focused);
                }
            }
        }
        self.list.scrollbar(g, fields.len(), list_top, bottom);
        if fields.is_empty() {
            g.text_at(PAD + 4.0, list_top + 10.0, Align::Left, "nothing in it yet · e: edit", ty.caption, t.dim);
        }
        let show = if self.revealed { "space: hide" } else { "space: show" };
        match fields.get(self.list.focus) {
            Some(Field::Password) => g.hint_any(&[&format!("enter: copy · {show} · e: edit · d: delete"), &format!("enter: copy · {show} · e: edit"), &format!("enter: copy · {show}")]),
            Some(Field::Notes) => g.hint_any(&["enter: read it all · e: edit · d: delete", "enter: read · e: edit"]),
            Some(_) => g.hint_any(&["enter: copy · e: edit · d: delete · esc: back", "enter: copy · e: edit · d: delete", "enter: copy · e: edit"]),
            None => g.hint("e: edit · d: delete · esc: back"),
        }
    }
}

// ---------------------------------------------------------- a text page ---

enum Text {
    /// An entry's notes, read from the vault each time.
    Notes(u64),
    /// Words of the app's own.
    Words(&'static str, &'static str),
}

/// A page of text: an entry's notes, or how importing works.
pub struct TextPage {
    text: Text,
    view: TextView,
}

impl TextPage {
    fn notes(id: u64) -> TextPage {
        TextPage { text: Text::Notes(id), view: TextView::new() }
    }

    fn words(title: &'static str, words: &'static str) -> TextPage {
        TextPage { text: Text::Words(title, words), view: TextView::new() }
    }
}

impl Screen for TextPage {
    fn title(&self) -> &str {
        match self.text {
            Text::Notes(_) => "Notes",
            Text::Words(title, _) => title,
        }
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        matches!(self.text, Text::Notes(_))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        if self.view.key(key) {
            return Nav::Stay;
        }
        match (key, &self.text) {
            (Key::Back, _) => Nav::Pop,
            (Key::Enter, Text::Notes(id)) => {
                if let Some(e) = super::get(*id) {
                    clip::copy(&e.notes, Sensitivity::Secret);
                    *cx.say = Some(copied_words("notes"));
                }
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        match self.text {
            Text::Notes(_) if !super::is_open() => Nav::Pop,
            _ => Nav::Stay,
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.view.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let ty = g.ty;
        let top = g.top() + 2.0;
        let bottom = g.above_hint();
        let r = Rect::new(PAD + 4.0, top, W - 2.0 * PAD - 12.0, bottom - top - 2.0);
        match &self.text {
            Text::Notes(id) => {
                let notes = super::get(*id).map(|e| Zeroizing::new(e.notes.clone()));
                if let Some(notes) = notes {
                    self.view.draw(g, &notes, r, ty.body);
                }
                g.take_foot(bottom);
                g.hint_any(&["↑↓ read · enter: copy · esc: back", "enter: copy · esc: back"]);
            }
            Text::Words(_, words) => {
                self.view.draw(g, words, r, ty.caption);
                g.take_foot(bottom);
                g.hint("↑↓ read · esc: back");
            }
        }
    }
}

// ----------------------------------------------------------- the editor ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditRow {
    Kind,
    Field(Field),
    Save,
}

/// What the generator hands the editor when "Use it" is chosen.
static PICKED: Mutex<Option<Zeroizing<String>>> = Mutex::new(None);

/// A new entry, or one being changed: each field a row, Enter types into
/// it, and Save keeps it. On the password row, g makes one.
pub struct EditPage {
    id: Option<u64>,
    draft: Entry,
    list: ListState,
    editing: Option<(Field, Input)>,
    error: Option<String>,
    changed: bool,
    leaving: bool,
}

impl EditPage {
    pub fn new(id: Option<u64>) -> EditPage {
        let draft = id.and_then(super::get).unwrap_or_else(|| Entry::new(Kind::Login, super::entry::new_id(), passcode::unix_now()));
        let mut page = EditPage { id, draft, list: ListState::default(), editing: None, error: None, changed: false, leaving: false };
        // A new entry starts on its title, its kind above.
        if id.is_none() {
            page.list.focus = 1;
        }
        page
    }

    fn rows(&self) -> Vec<EditRow> {
        let mut rows = Vec::new();
        if self.id.is_none() {
            rows.push(EditRow::Kind);
        }
        rows.extend(self.draft.kind.fields().iter().map(|f| EditRow::Field(*f)));
        rows.push(EditRow::Save);
        rows
    }

    fn save(&mut self, cx: &mut Cx) -> Nav {
        let d = &self.draft;
        if d.title.trim().is_empty() && d.username.is_empty() && d.password.is_empty() && d.notes.trim().is_empty() && d.totp.is_empty() {
            *cx.say = Some("nothing to keep yet".into());
            return Nav::Stay;
        }
        let mut entry = self.draft.clone();
        entry.modified = passcode::unix_now();
        // What its kind does not show is not kept (a login made a note).
        for f in [Field::Username, Field::Password, Field::Url, Field::Notes, Field::Totp] {
            if !entry.kind.fields().contains(&f) {
                entry.set(f, "");
            }
        }
        let id = entry.id;
        let result = super::change(move |entries| match entries.iter_mut().find(|e| e.id == id) {
            Some(have) => *have = entry,
            None => entries.push(entry),
        });
        match result {
            Ok(()) => {
                *cx.say = Some(if self.id.is_some() { "saved".into() } else { format!("{} kept", self.draft.kind.word()) });
                Nav::Pop
            }
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }
}

impl Screen for EditPage {
    fn title(&self) -> &str {
        if self.id.is_some() {
            "Edit"
        } else {
            "New item"
        }
    }

    fn wants_text(&self) -> bool {
        self.editing.is_some()
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        true
    }

    fn resume(&mut self) {
        if let Some(picked) = PICKED.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.draft.set(Field::Password, &picked);
            self.changed = true;
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        if let Some((field, input)) = &mut self.editing {
            let field = *field;
            match input.key(key) {
                Typed::More => self.error = None,
                Typed::Cancelled => {
                    self.editing = None;
                    self.error = None;
                }
                Typed::Done => {
                    let text = input.text.trim().to_owned();
                    let text = Zeroizing::new(text);
                    if field == Field::Totp && !text.is_empty() {
                        match Totp::parse(&text) {
                            Ok(t) => {
                                // Named as the site names it, if it had no name yet.
                                if self.draft.title.trim().is_empty() && !t.issuer.is_empty() {
                                    self.draft.set(Field::Title, &t.issuer);
                                }
                                self.draft.set(Field::Totp, &t.uri());
                            }
                            Err(why) => {
                                self.error = Some(why.to_owned());
                                return Nav::Stay;
                            }
                        }
                    } else {
                        self.draft.set(field, &text);
                    }
                    self.changed = true;
                    self.editing = None;
                }
            }
            return Nav::Stay;
        }
        let rows = self.rows();
        let focused = rows.get(self.list.focus).copied();
        let leaving = std::mem::take(&mut self.leaving);
        match (key, focused) {
            (Key::Back, _) if self.changed && !leaving => {
                self.leaving = true;
                *cx.say = Some("esc again drops the changes".into());
            }
            (Key::Back, _) => return Nav::Pop,
            (Key::Left | Key::Right, Some(EditRow::Kind)) => {
                self.draft.kind = self.draft.kind.step(if key == Key::Left { -1 } else { 1 });
                self.changed = true;
            }
            (Key::Enter, Some(EditRow::Kind)) => {
                self.draft.kind = self.draft.kind.step(1);
                self.changed = true;
            }
            (Key::Enter | Key::Right, Some(EditRow::Field(f))) => {
                let label = self.draft.kind.label(f);
                let now = if f == Field::Totp { String::new() } else { self.draft.get(f).to_owned() };
                let mut now = Zeroizing::new(now);
                self.editing = Some((f, Input::new(label, &now, f.limit(), f == Field::Password)));
                now.zeroize();
            }
            (Key::Char('g'), Some(EditRow::Field(Field::Password))) => return Nav::Push(Box::new(GeneratorPage::new(true))),
            (Key::Delete, Some(EditRow::Field(f))) if f != Field::Title || self.draft.kind == Kind::Note => {
                self.draft.set(f, "");
                self.changed = true;
            }
            (Key::Enter, Some(EditRow::Save)) => return self.save(cx),
            (Key::Ctrl('s'), _) => return self.save(cx),
            (other, _) => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        if super::is_open() {
            Nav::Stay
        } else {
            Nav::Pop
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let editing = self.editing.is_some();
        if let Some((_, input)) = &self.editing {
            input.draw(g, g.top());
        }
        let rows = self.rows();
        let top = if editing { g.top() + Input::height(g) + 6.0 } else { g.top() };
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        let kind = self.draft.kind;
        let editing_field = self.editing.as_ref().map(|(f, _)| *f);
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = !editing && i == self.list.focus;
            match rows[i] {
                EditRow::Kind => g.row(r, "kind", &format!("‹ {} ›", kind.word()), t.ink, focused),
                EditRow::Field(f) => {
                    let label = kind.label(f);
                    let have = self.draft.get(f);
                    let value = match f {
                        _ if editing_field == Some(f) => "typing".to_owned(),
                        Field::Password if !have.is_empty() => MASK.to_owned(),
                        Field::Password => "g: make one".to_owned(),
                        Field::Totp if !have.is_empty() => Totp::parse(have).map_or_else(|_| "set".to_owned(), |t| format!("{} digits · {} s", t.digits, t.period)),
                        Field::Totp => "otpauth:// or secret".to_owned(),
                        _ if have.is_empty() => "–".to_owned(),
                        _ => {
                            let label_w = g.measure(label, if g.theme == crate::look::Theme::Bezel { ty.label } else { ty.body });
                            fit_value(g, have, r.w - label_w - 34.0)
                        }
                    };
                    g.row(r, label, &value, if have.is_empty() { t.faint } else { t.dim }, focused);
                }
                EditRow::Save => g.row(r, "Save", if self.changed { "enter" } else { "no changes" }, if self.changed { t.ink } else { t.dim }, focused),
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        if let Some(why) = &self.error {
            let fault = g.fault();
            g.take_foot(g.above_hint());
            let style = ty.caption;
            let words = g.text.fit(why, &style, W - 2.0 * PAD);
            g.text_at(W * 0.5, H - 2.0 - style.size * 0.5, Align::Center, &words, style, fault);
            return;
        }
        match (editing_field, rows.get(self.list.focus)) {
            (Some(Field::Password), _) => g.hint_any(&["enter: done · tab: show · esc: cancel", "enter: done · esc: cancel"]),
            (Some(_), _) => g.hint_any(&["enter: done · esc: cancel"]),
            (None, Some(EditRow::Kind)) => g.hint_any(&["←→ what it is · ↓ its fields · esc: back", "←→ what it is · esc: back"]),
            (None, Some(EditRow::Field(Field::Password))) => g.hint_any(&["enter: type · g: make one · ctrl s: save", "enter: type · g: make one"]),
            (None, Some(EditRow::Field(_))) => g.hint_any(&["enter: type · ctrl s: save · esc: back", "enter: type · ctrl s: save"]),
            (None, _) => g.hint_any(&["enter: save · esc: back", "enter: save"]),
        }
    }
}

// --------------------------------------------------------- the generator ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GenRow {
    Kind,
    Size,
    Signs,
    Again,
    Copy,
    Use,
}

/// The recipe last used, for the next time the generator opens.
static RECIPE: Mutex<(usize, bool, usize, bool)> = Mutex::new((20, true, 6, false));

/// New passwords: characters (a length, signs or not) or words; Enter makes
/// another, and it is copied, or used in the entry being edited.
pub struct GeneratorPage {
    len: usize,
    signs: bool,
    words: usize,
    use_words: bool,
    value: Zeroizing<String>,
    list: ListState,
    for_editor: bool,
    /// The new password's arrival: 0 just made, 1 settled.
    arrive: Animated,
}

impl GeneratorPage {
    pub fn new(for_editor: bool) -> GeneratorPage {
        let (len, signs, words, use_words) = *RECIPE.lock().unwrap_or_else(|e| e.into_inner());
        let mut page = GeneratorPage { len, signs, words, use_words, value: Zeroizing::new(String::new()), list: ListState::default(), for_editor, arrive: Animated::new(1.0, role::smooth()).with_rest(0.01) };
        page.make(true);
        page
    }

    fn recipe(&self) -> Recipe {
        if self.use_words {
            Recipe::Words { words: self.words }
        } else {
            Recipe::Chars { len: self.len, signs: self.signs }
        }
    }

    fn make(&mut self, calm: bool) {
        if let Ok(v) = gen::make(self.recipe()) {
            self.value = v;
        }
        *RECIPE.lock().unwrap_or_else(|e| e.into_inner()) = (self.len, self.signs, self.words, self.use_words);
        if calm {
            self.arrive.jump(1.0);
        } else {
            self.arrive.jump(0.0);
            self.arrive.set(1.0);
        }
    }

    fn rows(&self) -> Vec<GenRow> {
        let mut rows = vec![GenRow::Kind, GenRow::Size];
        if !self.use_words {
            rows.push(GenRow::Signs);
        }
        rows.extend([GenRow::Again, GenRow::Copy]);
        if self.for_editor {
            rows.push(GenRow::Use);
        }
        rows
    }
}

impl Screen for GeneratorPage {
    fn title(&self) -> &str {
        "Generator"
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        let calm = cx.settings.calm;
        let rows = self.rows();
        let focused = rows.get(self.list.focus).copied();
        let step = match key {
            Key::Left => -1i32,
            Key::Right => 1,
            _ => 0,
        };
        match (key, focused) {
            (Key::Back, _) => return Nav::Pop,
            (Key::Left | Key::Right, Some(GenRow::Kind)) | (Key::Enter, Some(GenRow::Kind)) => {
                self.use_words = !self.use_words;
                self.make(calm);
            }
            (Key::Left | Key::Right, Some(GenRow::Size)) => {
                if self.use_words {
                    self.words = (self.words as i32 + step).clamp(gen::WORDS_RANGE.0 as i32, gen::WORDS_RANGE.1 as i32) as usize;
                } else {
                    self.len = (self.len as i32 + step * if self.len >= 32 { 4 } else { 1 }).clamp(gen::LEN_RANGE.0 as i32, gen::LEN_RANGE.1 as i32) as usize;
                }
                self.make(calm);
            }
            (Key::Left | Key::Right | Key::Enter, Some(GenRow::Signs)) => {
                self.signs = !self.signs;
                self.make(calm);
            }
            (Key::Enter, Some(GenRow::Again) | Some(GenRow::Size)) | (Key::Char(' '), _) => self.make(calm),
            (Key::Enter, Some(GenRow::Copy)) => {
                clip::copy(&self.value, Sensitivity::Secret);
                *cx.say = Some(copied_words("password"));
            }
            (Key::Enter, Some(GenRow::Use)) => {
                *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.value.clone());
                return Nav::Pop;
            }
            (other, _) => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        if super::is_open() {
            Nav::Stay
        } else {
            Nav::Pop
        }
    }

    fn busy(&self) -> bool {
        true
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.arrive.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        // The password on a card of its own, in the mono voice, broken
        // where it must be: up to three lines.
        let style = TextStyle::label(ty.name.size);
        let card_w = W - 2.0 * PAD;
        let inner = card_w - 20.0;
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        for c in self.value.chars() {
            line.push(c);
            if g.measure(&line, style) > inner {
                // A phrase breaks after a dash, between its words; random
                // characters wherever the line is full.
                let cut = match line.rfind('-') {
                    Some(at) if self.use_words && at + 1 < line.len() => at + 1,
                    _ => line.len() - c.len_utf8(),
                };
                let rest = line.split_off(cut);
                lines.push(std::mem::take(&mut line));
                line = rest;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        let line_h = style.size + 4.0;
        let card_h = (lines.len().max(1) as f32 * line_h + 22.0 + ty.caption.size).round();
        let card = Rect::new(PAD, top, card_w, card_h);
        g.surface(card, g.tile_radius(), t.surface1);
        let k = self.arrive.value().clamp(0.0, 1.0);
        for (i, l) in lines.iter().enumerate() {
            let cy = card.y + 9.0 + line_h * (i as f32 + 0.5) + 4.0 * (1.0 - k);
            g.text_alpha(card.x + 10.0, cy, l, style, t.ink, k);
        }
        for l in &mut lines {
            l.zeroize();
        }
        let recipe = self.recipe();
        let strength = format!("{} · {:.0} bits", recipe.strength(), recipe.bits());
        let color = if recipe.bits() >= 60.0 { g.good() } else { t.amber_ink };
        g.text_at(card.x + 10.0, card.bottom() - 7.0 - ty.caption.size * 0.5, Align::Left, &strength, ty.caption, color);
        let rows = self.rows();
        let list_top = card.bottom() + 5.0;
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match rows[i] {
                GenRow::Kind => g.row(r, "made of", if self.use_words { "‹ words ›" } else { "‹ characters ›" }, t.ink, focused),
                GenRow::Size if self.use_words => g.row(r, "words", &format!("‹ {} ›", self.words), t.ink, focused),
                GenRow::Size => g.row(r, "length", &format!("‹ {} ›", self.len), t.ink, focused),
                GenRow::Signs => g.row(r, "signs", if self.signs { "‹ on ›" } else { "‹ off ›" }, t.ink, focused),
                GenRow::Again => g.row(r, "Make another", "space", t.dim, focused),
                GenRow::Copy => g.row(r, "Copy it", "30 s", t.dim, focused),
                GenRow::Use => g.row(r, "Use it", "›", t.ink, focused),
            }
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom);
        match rows.get(self.list.focus) {
            Some(GenRow::Kind | GenRow::Size | GenRow::Signs) => g.hint_any(&["←→ change · space: another · esc: back", "←→ change · space: another"]),
            Some(GenRow::Use) => g.hint_any(&["enter: put it in the entry · esc: back", "enter: use it · esc: back"]),
            _ => g.hint_any(&["enter: choose · space: another · esc: back", "enter · space: another"]),
        }
    }
}

// --------------------------------------------- importing and backing up ---

const ABOUT_IMPORT: &str = "To bring passwords in, export them from Bitwarden, 1Password or KeePass as CSV (or your authenticator's codes as a text of otpauth:// lines), and copy the file to /data over SSH or USB. It shows here; Enter reads it into the vault.\n\nThe CSV holds every password in plain text, so once it is in, the vault offers to shred it: overwritten, then deleted. A memory card can keep old copies of its blocks as it wears, so shredding is the best the handheld can do: delete the export on your computer too, and empty its bin.\n\nA backup is the vault sealed under a password of its own (at least 10 characters), written to /data for you to copy off. It opens here, in this or another vault, with that password. Forget it and the backup is lost.";

#[derive(Clone, PartialEq, Debug)]
enum ToolRow {
    File(PathBuf, FileKind),
    NoFiles,
    About,
    Backup,
    Delete,
}

/// Import and back up: the exports and backups found in /data, a backup
/// made, the vault deleted.
pub struct ToolsPage {
    files: Vec<(PathBuf, FileKind)>,
    list: ListState,
    deleting: bool,
    reading: Option<(Receiver<Result<Found, String>>, PathBuf)>,
    looked: std::time::Instant,
}

impl ToolsPage {
    pub fn new() -> ToolsPage {
        ToolsPage { files: import::files(&crate::files::storage()), list: ListState::default(), deleting: false, reading: None, looked: std::time::Instant::now() }
    }

    fn rows(&self) -> Vec<ToolRow> {
        let mut rows: Vec<ToolRow> = self.files.iter().map(|(p, k)| ToolRow::File(p.clone(), *k)).collect();
        if rows.is_empty() {
            rows.push(ToolRow::NoFiles);
        }
        rows.extend([ToolRow::About, ToolRow::Backup, ToolRow::Delete]);
        rows
    }
}

impl Default for ToolsPage {
    fn default() -> Self {
        Self::new()
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Screen for ToolsPage {
    fn title(&self) -> &str {
        "Import"
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        true
    }

    fn resume(&mut self) {
        self.files = import::files(&crate::files::storage());
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        if self.reading.is_some() {
            return Nav::Stay;
        }
        let rows = self.rows();
        let focused = rows.get(self.list.focus).cloned();
        let armed = std::mem::take(&mut self.deleting);
        match (key, focused) {
            (Key::Back, _) => return Nav::Pop,
            (Key::Enter | Key::Right, Some(ToolRow::File(path, FileKind::Export))) => {
                let p = path.clone();
                let rx = super::work("vault-import", move || import::read_file(&p, passcode::unix_now()));
                self.reading = Some((rx, path));
            }
            (Key::Enter | Key::Right, Some(ToolRow::File(path, FileKind::Backup))) => return Nav::Push(Box::new(PasswordPage::restore(path))),
            (Key::Enter | Key::Right, Some(ToolRow::About | ToolRow::NoFiles)) => return Nav::Push(Box::new(TextPage::words("Importing", ABOUT_IMPORT))),
            (Key::Enter | Key::Right, Some(ToolRow::Backup)) => return Nav::Push(Box::new(PasswordPage::backup())),
            (Key::Enter, Some(ToolRow::Delete)) => {
                if armed {
                    return match super::delete() {
                        Ok(()) => {
                            *cx.say = Some("vault deleted".into());
                            Nav::Pop
                        }
                        Err(e) => {
                            *cx.say = Some(format!("could not delete it: {e}"));
                            Nav::Stay
                        }
                    };
                }
                self.deleting = true;
                *cx.say = Some("enter again deletes it all".into());
            }
            (other, _) => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if !super::is_open() {
            return Nav::Pop;
        }
        let Some((rx, path)) = &self.reading else { return Nav::Stay };
        let found = match rx.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return Nav::Stay,
            Err(TryRecvError::Disconnected) => Err("could not read it".into()),
        };
        let path = path.clone();
        self.reading = None;
        match found {
            Ok(found) => {
                let source = found.source;
                let bad = found.bad_codes;
                let mut counts = (0, 0);
                let result = super::change(|entries| counts = import::merge(entries, found.entries));
                match result {
                    Ok(()) => Nav::Push(Box::new(ShredPage::new(path, source, counts, bad))),
                    Err(why) => {
                        *cx.say = Some(why);
                        Nav::Stay
                    }
                }
            }
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // The storage looked at again now and then: a file copied over SSH shows.
        if self.looked.elapsed().as_secs() >= 3 {
            self.looked = std::time::Instant::now();
            let files = import::files(&crate::files::storage());
            if files != self.files {
                self.files = files;
                return true;
            }
        }
        self.reading.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let rows = self.rows();
        let top = g.top();
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            match &rows[i] {
                ToolRow::File(path, kind) => {
                    let value = match (kind, &self.reading) {
                        (_, Some((_, reading))) if reading == path => "reading",
                        (FileKind::Export, _) => "import ›",
                        (FileKind::Backup, _) => "restore ›",
                    };
                    g.row(r, &file_name(path), value, t.dim, focused);
                }
                ToolRow::NoFiles => g.row(r, "No exports in /data yet", "?", t.dim, focused),
                ToolRow::About => g.row(r, "How importing works", "›", t.dim, focused),
                ToolRow::Backup => g.row(r, "Back up to /data", "›", t.dim, focused),
                ToolRow::Delete => g.row(r, "Delete the vault", if self.deleting { "enter again" } else { "" }, g.fault(), focused),
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        match rows.get(self.list.focus) {
            Some(ToolRow::File(_, FileKind::Export)) => g.hint_any(&["enter: read it into the vault · esc: back", "enter: import · esc: back"]),
            Some(ToolRow::File(_, FileKind::Backup)) => g.hint_any(&["enter: restore from it · esc: back", "enter: restore · esc: back"]),
            Some(ToolRow::Delete) => g.hint_any(&["enter twice: delete every entry · esc: back", "enter twice: delete it all"]),
            _ => g.hint("enter: open · esc: back"),
        }
    }
}

/// After an import: how many came in, and the CSV shredded or kept.
pub struct ShredPage {
    path: PathBuf,
    /// What it says, fullest first: the one that fits is shown.
    said: [String; 2],
    list: ListState,
}

impl ShredPage {
    fn new(path: PathBuf, source: import::Source, (added, there): (usize, usize), bad: usize) -> ShredPage {
        let items = |n: usize| if n == 1 { "1\u{a0}item".to_owned() } else { format!("{n}\u{a0}items") };
        let from = match source {
            import::Source::Csv | import::Source::Authenticator => String::new(),
            s => format!(" from {}", s.name()),
        };
        let mut said = match (added, there) {
            (0, 0) => "Nothing in it was new.".to_owned(),
            (0, t) => format!("All {t}{from} were in the vault already."),
            (a, 0) => format!("{}{from} {} in the vault now.", items(a), if a == 1 { "is" } else { "are" }),
            (a, t) => format!("{}{from} {} in the vault now; {t} {} there already.", items(a), if a == 1 { "is" } else { "are" }, if t == 1 { "was" } else { "were" }),
        };
        if bad > 0 {
            said.push_str(&format!(" {bad} 2FA secret{} could not be read.", if bad == 1 { "" } else { "s" }));
        }
        said.push_str(" The file still holds every one in plain\u{a0}text.");
        let short = match added {
            0 => "Nothing new. The file is still plain\u{a0}text.".to_owned(),
            a => format!("{} in. The file is still plain\u{a0}text.", items(a)),
        };
        ShredPage { path, said: [said, short], list: ListState::default() }
    }
}

impl Screen for ShredPage {
    fn title(&self) -> &str {
        "Imported"
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        super::touch();
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right if self.list.focus == 0 => {
                *cx.say = Some(match import::shred(&self.path) {
                    Ok(()) => "file shredded".to_owned(),
                    Err(e) => format!("could not shred it: {e}"),
                });
                Nav::Pop
            }
            Key::Enter | Key::Right => {
                *cx.say = Some("kept · still in plain text".into());
                Nav::Pop
            }
            other => {
                crate::screens::list_key(&mut self.list, other, 2);
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        let rows_h = 2.0 * row_h + 3.0;
        let room = bottom - top - rows_h - 8.0;
        let lines = fit_lines(g, &[&self.said[0], &self.said[1]], ty.body, W - 2.0 * PAD - 8.0, (room / (ty.body.size + 5.0)).floor().max(1.0) as usize);
        let line_h = ty.body.size + 5.0;
        for (i, line) in lines.iter().enumerate() {
            g.text_at(PAD + 4.0, top + line_h * (i as f32 + 0.5), Align::Left, line, ty.body, t.ink);
        }
        let list_top = (top + lines.len() as f32 * line_h + 6.0).round();
        let name = file_name(&self.path);
        for (i, y) in self.list.layout(g, 2, list_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if i == 0 {
                let label = g.text.fit(&format!("Shred {name}"), &ty.body, r.w - 60.0);
                g.row(r, &label, "enter", t.ink, focused);
            } else {
                g.row(r, "Keep it", "", t.dim, focused);
            }
        }
        self.list.scrollbar(g, 2, list_top, bottom);
        g.hint_any(&["shredding is best effort on a memory card", "best effort on a card"]);
    }
}

// --------------------------------------------- a backup's password page ---

enum Job {
    Backup(Receiver<Result<PathBuf, String>>),
    Restore(Receiver<Result<Vec<Entry>, crypto::Error>>),
}

/// A backup's password: chosen twice to make one, typed once to restore.
pub struct PasswordPage {
    restore: Option<PathBuf>,
    again: bool,
    input: Input,
    first: Zeroizing<String>,
    job: Option<Job>,
    wrong: Option<String>,
}

impl PasswordPage {
    fn backup() -> PasswordPage {
        PasswordPage { restore: None, again: false, input: Input::new("the backup's password", "", 128, true), first: Zeroizing::new(String::new()), job: None, wrong: None }
    }

    fn restore(path: PathBuf) -> PasswordPage {
        PasswordPage { restore: Some(path), again: false, input: Input::new("its password", "", 128, true), first: Zeroizing::new(String::new()), job: None, wrong: None }
    }

    fn words(&self) -> (String, String) {
        match (&self.restore, self.again) {
            (Some(p), _) => ("Type the backup's password".to_owned(), file_name(p)),
            (None, false) => ("A password for the backup".to_owned(), format!("{} characters or more · a phrase is best", super::BACKUP_MIN)),
            (None, true) => ("Type it again".to_owned(), "forget it and the backup is lost".to_owned()),
        }
    }

    fn submit(&mut self) {
        let typed = Zeroizing::new(self.input.text.as_str().to_owned());
        self.input = Input::new(self.input.label, "", 128, true);
        if let Some(path) = &self.restore {
            if typed.is_empty() {
                return;
            }
            self.job = Some(Job::Restore(super::begin_restore(typed, path.clone())));
            return;
        }
        if !self.again {
            if typed.chars().count() < super::BACKUP_MIN {
                self.wrong = Some(format!("at least {} characters", super::BACKUP_MIN));
                return;
            }
            self.first = typed;
            self.again = true;
            self.wrong = None;
            return;
        }
        if *typed != *self.first {
            self.first.zeroize();
            self.again = false;
            self.wrong = Some("not the same · choose it again".into());
            return;
        }
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        let dest = import::backup_path(&crate::files::storage(), &day);
        self.first.zeroize();
        match super::begin_backup(typed, dest) {
            Some(rx) => self.job = Some(Job::Backup(rx)),
            None => self.wrong = Some("the vault is locked".into()),
        }
    }
}

impl Screen for PasswordPage {
    fn title(&self) -> &str {
        if self.restore.is_some() {
            "Restore"
        } else {
            "Back up"
        }
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn secret(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        super::touch();
        if self.job.is_some() {
            return Nav::Stay;
        }
        match self.input.key(key) {
            Typed::Cancelled if self.input.text.is_empty() => return Nav::Pop,
            Typed::Cancelled => self.input = Input::new(self.input.label, "", 128, true),
            Typed::Done => self.submit(),
            Typed::More => self.wrong = None,
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if !super::is_open() {
            return Nav::Pop;
        }
        match &self.job {
            Some(Job::Backup(rx)) => match rx.try_recv() {
                Ok(Ok(_)) => {
                    // The file is in /data, listed on the page this goes back to.
                    self.job = None;
                    *cx.say = Some("backed up to /data".into());
                    return Nav::Pop;
                }
                Ok(Err(why)) => {
                    self.job = None;
                    self.again = false;
                    self.wrong = Some(why);
                }
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.wrong = Some("could not make the backup".into());
                }
                Err(TryRecvError::Empty) => {}
            },
            Some(Job::Restore(rx)) => match rx.try_recv() {
                Ok(Ok(entries)) => {
                    self.job = None;
                    let mut counts = (0, 0);
                    let result = super::change(|have| counts = import::merge(have, entries));
                    *cx.say = Some(match (result, counts) {
                        (Err(why), _) => why,
                        (Ok(()), (a, 0)) => format!("restored {a}"),
                        (Ok(()), (a, t)) => format!("restored {a} · {t} already here"),
                    });
                    return Nav::Pop;
                }
                Ok(Err(e)) => {
                    self.job = None;
                    self.wrong = Some(if e == crypto::Error::Wrong { "not that password".into() } else { e.words().to_owned() });
                }
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.wrong = Some("could not open the backup".into());
                }
                Err(TryRecvError::Empty) => {}
            },
            None => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let (say, under) = self.words();
        let cy = top + ty.name.size * 0.5 + 6.0;
        let say = g.text.fit(&say, &ty.name, W - 2.0 * PAD);
        g.text_at(W * 0.5, cy, Align::Center, &say, ty.name, t.ink);
        // Whole, or its first part where the whole is too wide.
        let under = if g.measure(&under, ty.caption) <= W - 2.0 * PAD { under } else { under.split(" · ").next().unwrap_or("").to_owned() };
        let under = g.text.fit(&under, &ty.caption, W - 2.0 * PAD);
        g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 10.0, Align::Center, &under, ty.caption, t.dim);
        let y = (cy + ty.name.size * 0.5 + 22.0).round();
        self.input.draw(g, y);
        let (words, color) = match (&self.job, &self.wrong) {
            (Some(Job::Backup(_)), _) => ("sealing the backup".to_owned(), t.dim),
            (Some(Job::Restore(_)), _) => ("opening the backup".to_owned(), t.dim),
            (None, Some(w)) => (w.clone(), g.fault()),
            (None, None) => (String::new(), t.dim),
        };
        if !words.is_empty() {
            let words = g.text.fit(&words, &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, y + Input::height(g) + 10.0, Align::Center, &words, ty.caption, color);
        }
        g.hint_any(&["enter: next · tab: show · esc: back", "enter: next · esc: back"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_read_out_in_groups() {
        assert_eq!(grouped("482913"), "482 913");
        assert_eq!(grouped("48291307"), "4829 1307");
        assert_eq!(grouped("4829130"), "482 9130");
    }
}
