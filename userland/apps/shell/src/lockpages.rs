//! Settings › Lock screen and Settings › StandBy: the passcode (set,
//! changed, taken off: each asks for the one there is first), the lock
//! screen's editor with itself as the live preview, whether notices' words
//! show on it, StandBy on the charger and its night, and StandBy shown now.

use std::sync::mpsc::Receiver;

use bezel::{Align, GlassShape, Rect};
use detent::{role, Animated};

use crate::faces::{Faces, Feed};
use crate::gfx::{Gfx, H, PAD, W};
use crate::lock::{draw_face, Choices, Edit, Field, Info, Part, Prompt, Show, FIELD_H, FIELD_Y};
use crate::passcode::{self, Tries};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// What Settings says of the passcode: what it does, what it does not
/// cover, and how one forgotten comes off. Numbers keep to their units.
pub const LOCK_NOTE: &str = "With a passcode, the handheld asks for it once the screen has been off. It keeps out someone at the keys. It does not cover SSH, the setup page a computer opens, or the card itself: anyone can take the card out and read it. Forgotten, it comes off over SSH (rm /data/catalyst/passcode) or from the card on a Linux computer.";
const NO_PASSCODE: &str = "No passcode: a key wakes it straight to where it was.";

fn standby_note(h24: bool) -> String {
    format!(
        "On the charger and left alone, it shows a clock and more instead of going dark. ↑↓ turn the faces, ←→ their style; any other key goes back. At night ({}) it dims and turns red. Deep standby waits until it is off the charger.",
        crate::faces::night_words(h24)
    )
}

// ----------------------------------------------------------- rows and a note ---

/// A page of rows with a note of plain words beneath. The rows scroll a
/// whole row at a time, as Settings' lists do; past the last row, the note
/// comes up whole (and, where it is longer than the page, a line at a time).
struct Rows {
    focus: usize,
    /// Note lines that do not fit at once: steps past the note's first.
    extra: usize,
    first: usize,
    scroll: Animated,
}

impl Rows {
    fn new() -> Rows {
        Rows { focus: 0, extra: 0, first: 0, scroll: Animated::new(0.0, role::hold()).with_rest(0.05) }
    }

    /// Up and Down along `rows` rows and then the note.
    fn key(&mut self, key: Key, rows: usize) -> bool {
        let last = rows + self.extra;
        match key {
            Key::Up if self.focus > 0 => self.focus = self.focus.min(last) - 1,
            Key::Down if self.focus < last => self.focus += 1,
            Key::Home => self.focus = 0,
            Key::End => self.focus = last,
            _ => return false,
        }
        true
    }

    fn draw(&mut self, g: &mut Gfx, rows: &[(String, String, bool)], note: &str) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let (row_h, gap) = (g.row_h(), 3.0);
        let pitch = row_h + gap;
        let bottom = H - PAD + 2.0;
        let line_h = ty.caption.size + 4.0;
        let lines = crate::help::wrap(g, note, ty.caption, W - 2.0 * PAD - 8.0);
        let view = bottom - top;
        let fit = (((view - 6.0) / line_h).floor().max(1.0)) as usize;
        self.extra = lines.len().saturating_sub(fit);
        self.focus = self.focus.min(rows.len() + self.extra);
        let visible = ((view + gap) / pitch).floor().max(1.0) as usize;
        let want = if self.focus >= rows.len() {
            rows.len() as f32 * pitch + (self.focus - rows.len()) as f32 * line_h
        } else {
            if self.focus < self.first {
                self.first = self.focus;
            } else if self.focus >= self.first + visible {
                self.first = self.focus + 1 - visible;
            }
            self.first as f32 * pitch
        };
        if (want - self.scroll.target()).abs() > 0.5 {
            self.scroll.set(want);
        }
        if g.calm {
            self.scroll.jump(want);
        }
        // Clipped while it glides; at rest nothing is cut, as the rows and
        // the note land whole.
        let s = self.scroll.value();
        g.canvas.set_clip(Rect::new(0.0, top - 4.0, W, view + 6.0));
        for (i, (name, value, on)) in rows.iter().enumerate() {
            let y = top + i as f32 * pitch - s;
            if y + row_h < top - 4.0 || y > bottom {
                continue;
            }
            g.row(Rect::new(PAD, y, W - 2.0 * PAD, row_h), name, value, if *on { t.ink } else { t.dim }, i == self.focus);
        }
        let mut y = top + rows.len() as f32 * pitch + 6.0 - s;
        for line in &lines {
            if y >= top - 1.0 && y + line_h <= bottom + 3.0 {
                g.text_alpha(PAD + 4.0, y + line_h * 0.5, line, ty.caption, t.dim, 1.0);
            }
            y += line_h;
        }
        g.canvas.clear_clip();
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }
}

// ---------------------------------------------------------- Lock screen ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LockRow {
    Set,
    Change,
    TurnOff,
    Customise,
    Previews,
}

fn lock_rows(armed: bool) -> &'static [LockRow] {
    if armed {
        &[LockRow::Change, LockRow::TurnOff, LockRow::Customise, LockRow::Previews]
    } else {
        &[LockRow::Set, LockRow::Customise, LockRow::Previews]
    }
}

/// Settings › Lock screen.
pub struct LockPage {
    rows: Rows,
    armed: bool,
    generation: u64,
}

impl Default for LockPage {
    fn default() -> Self {
        Self::new()
    }
}

impl LockPage {
    pub fn new() -> LockPage {
        LockPage { rows: Rows::new(), armed: passcode::is_set(), generation: passcode::generation() }
    }

    /// Opened at a place (from search): 0 the passcode, 1 customising.
    pub fn at_place(mut self, place: usize) -> LockPage {
        let rows = lock_rows(self.armed);
        self.rows.focus = if place == 1 { rows.iter().position(|r| *r == LockRow::Customise).unwrap_or(0) } else { 0 };
        self
    }
}

impl Screen for LockPage {
    fn title(&self) -> &str {
        "Lock screen"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = lock_rows(self.armed);
        if key == Key::Back {
            return Nav::Pop;
        }
        if self.rows.key(key, rows.len()) {
            return Nav::Stay;
        }
        let Some(row) = rows.get(self.rows.focus).copied() else { return Nav::Stay };
        match (row, key) {
            (LockRow::Set, Key::Enter | Key::Right) => Nav::Push(Box::new(PasscodeFlow::new(Purpose::Set))),
            (LockRow::Change, Key::Enter | Key::Right) => Nav::Push(Box::new(PasscodeFlow::new(Purpose::Change))),
            (LockRow::TurnOff, Key::Enter | Key::Right) => Nav::Push(Box::new(PasscodeFlow::new(Purpose::Remove))),
            (LockRow::Customise, Key::Enter | Key::Right) => Nav::Push(Box::new(Editor::new())),
            (LockRow::Previews, Key::Enter | Key::Left | Key::Right) => {
                cx.settings.lock.previews = !cx.settings.lock.previews;
                crate::save_settings(cx.settings);
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn resume(&mut self) {
        self.generation = u64::MAX;
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.rows.tick(dt);
        if self.generation != passcode::generation() {
            self.generation = passcode::generation();
            let armed = passcode::is_set();
            changed |= armed != self.armed;
            self.armed = armed;
        }
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let c = g.lock;
        let rows: Vec<(String, String, bool)> = lock_rows(self.armed)
            .iter()
            .map(|row| match row {
                LockRow::Set => ("Set a passcode".to_owned(), "off ›".to_owned(), false),
                LockRow::Change => ("Change passcode".to_owned(), "on ›".to_owned(), true),
                LockRow::TurnOff => ("Turn passcode off".to_owned(), "›".to_owned(), true),
                LockRow::Customise => ("Customise lock screen".to_owned(), format!("{} ›", c.lock_face.name()), true),
                LockRow::Previews => ("Notices on it".to_owned(), (if c.previews { "words shown" } else { "how many" }).to_owned(), c.previews),
            })
            .collect();
        let note = if self.armed { LOCK_NOTE.to_owned() } else { format!("{NO_PASSCODE} {LOCK_NOTE}") };
        self.rows.draw(g, &rows, &note);
    }
}

// --------------------------------------------------------- the passcode ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    Set,
    Change,
    Remove,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Current,
    New,
    Confirm,
}

/// Setting, changing or taking off the passcode: the one there is first
/// (counted as the lock screen counts wrong tries), then a new one twice.
pub struct PasscodeFlow {
    purpose: Purpose,
    step: Step,
    field: Field,
    first: String,
    wrong: Option<String>,
    checking: Option<Receiver<bool>>,
    making: Option<Receiver<Result<(), String>>>,
    tries: Tries,
    shown_wait: u64,
}

impl PasscodeFlow {
    pub fn new(purpose: Purpose) -> PasscodeFlow {
        let step = if purpose == Purpose::Set { Step::New } else { Step::Current };
        PasscodeFlow { purpose, step, field: Field::new(), first: String::new(), wrong: None, checking: None, making: None, tries: Tries::load(passcode::unix_now()), shown_wait: 0 }
    }

    fn wait(&self) -> u64 {
        if self.step == Step::Current {
            self.tries.wait(passcode::unix_now())
        } else {
            0
        }
    }

    fn working(&self) -> bool {
        self.checking.is_some() || self.making.is_some()
    }

    fn words(&self) -> (&'static str, String) {
        match self.step {
            Step::Current => ("Type the passcode you have now", String::new()),
            Step::New => ("Type a new passcode", format!("{} to {} digits or letters", passcode::MIN_LEN, passcode::MAX_LEN)),
            Step::Confirm => ("Type it again", "to be sure of it".to_owned()),
        }
    }

    fn submit(&mut self, calm: bool) {
        match self.step {
            Step::Current => {
                let Some(phc) = passcode::stored() else {
                    // Taken off meanwhile: nothing to ask for.
                    self.step = Step::New;
                    return;
                };
                if self.field.is_empty() {
                    return;
                }
                let mut typed = self.field.take();
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new().name("passcode".into()).stack_size(256 * 1024).spawn(move || {
                    let right = passcode::check(&typed, &phc);
                    passcode::wipe(&mut typed);
                    tx.send(right).ok();
                });
                if spawned.is_ok() {
                    self.checking = Some(rx);
                }
            }
            Step::New => {
                if self.field.len() < passcode::MIN_LEN {
                    if !self.field.is_empty() {
                        self.wrong = Some(format!("at least {} characters", passcode::MIN_LEN));
                        self.field.wrong(calm);
                    }
                    return;
                }
                self.first = self.field.take();
                self.wrong = None;
                self.step = Step::Confirm;
            }
            Step::Confirm => {
                let mut again = self.field.take();
                if again != self.first {
                    passcode::wipe(&mut again);
                    passcode::wipe(&mut self.first);
                    self.wrong = Some("not the same · type a new one".into());
                    self.field.wrong(calm);
                    self.step = Step::New;
                    return;
                }
                passcode::wipe(&mut again);
                let mut new = std::mem::take(&mut self.first);
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new().name("passcode".into()).stack_size(256 * 1024).spawn(move || {
                    let made = passcode::make(&new);
                    passcode::wipe(&mut new);
                    let result = match made {
                        Some((phc, _, _)) => passcode::keep(&phc).map_err(|e| format!("could not keep it: {e}")),
                        None => Err("could not make the passcode".into()),
                    };
                    tx.send(result).ok();
                });
                if spawned.is_ok() {
                    self.making = Some(rx);
                }
            }
        }
    }
}

impl Drop for PasscodeFlow {
    fn drop(&mut self) {
        passcode::wipe(&mut self.first);
    }
}

impl Screen for PasscodeFlow {
    fn title(&self) -> &str {
        "Passcode"
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

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.working() {
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
            Edit::Submit => self.submit(cx.settings.calm),
            Edit::Nothing => {}
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(rx) = &self.checking {
            match rx.try_recv() {
                Ok(true) => {
                    self.checking = None;
                    self.tries.passed();
                    if self.purpose == Purpose::Remove {
                        return match passcode::remove() {
                            Ok(()) => {
                                *cx.say = Some("passcode off".into());
                                Nav::Pop
                            }
                            Err(e) => {
                                *cx.say = Some(format!("could not take it off: {e}"));
                                Nav::Pop
                            }
                        };
                    }
                    self.step = Step::New;
                }
                Ok(false) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.checking = None;
                    self.tries.failed(passcode::unix_now());
                    self.wrong = Some("not that one".into());
                    self.field.wrong(cx.settings.calm);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.making {
            match rx.try_recv() {
                Ok(result) => {
                    self.making = None;
                    *cx.say = Some(match result {
                        Ok(()) if self.purpose == Purpose::Change => "passcode changed".into(),
                        Ok(()) => "passcode on · asked for once the screen has been off".into(),
                        Err(why) => why,
                    });
                    return Nav::Pop;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.making = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let wait = self.wait();
        let counted = wait != self.shown_wait;
        self.shown_wait = wait;
        self.field.tick(dt, false) | self.working() | counted
    }

    fn busy(&self) -> bool {
        self.checking.is_some() || self.making.is_some() || self.field.moving()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let (say, under) = self.words();
        let cy = top + 22.0;
        g.text_at(W * 0.5, cy, Align::Center, say, ty.name, t.ink);
        if !under.is_empty() {
            g.text_at(W * 0.5, cy + ty.name.size * 0.5 + 10.0, Align::Center, &under, ty.caption, t.dim);
        }

        let prompt = if self.wait() > 0 {
            Prompt::Wait(self.wait())
        } else if self.checking.is_some() {
            Prompt::Checking
        } else if self.making.is_some() {
            Prompt::Asks("keeping it".into())
        } else if let Some(w) = &self.wrong {
            Prompt::Wrong(w.clone())
        } else {
            Prompt::Asks("type it here".into())
        };
        // On a tonal plate of its own: glass needs something to bend.
        let y = top + 62.0;
        let plate = Rect::new(PAD + 6.0, y - 6.0, W - 2.0 * PAD - 12.0, FIELD_H + 12.0);
        g.canvas.rrect(plate, plate.h * 0.5, t.surface1, 1.0);
        self.field.draw(g, W * 0.5, y, plate.w - 12.0, &prompt);
        g.hint_any(&["enter: next · esc: clear, then back", "enter: next · esc: back"]);
    }
}

// ------------------------------------------------------------ the editor ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Param {
    Clock,
    Colour,
    Item(Show),
}

fn params() -> Vec<Param> {
    let mut out = vec![Param::Clock, Param::Colour];
    for s in Show::ALL {
        if *s != Show::Reminder || crate::upnext::reminders_available() {
            out.push(Param::Item(*s));
        }
    }
    out
}

/// What the lock screen shows besides the time, read now (for the editor's
/// preview, and the shell's lock screen).
pub fn info_now(robot: &Robot, h24: bool, reading: Option<crate::power::Reading>) -> Info {
    let unix = chrono::Utc::now().timestamp();
    Info { h24, reading, next: crate::upnext::next_match(robot, unix).ok(), reminder: crate::upnext::next_reminder(unix), ..Info::default() }
}

/// Settings › Lock screen › Customise: the lock screen itself, live, with
/// a bar along the foot. ↑↓ choose what to change, ←→ change it (the clock,
/// its colour, each thing shown with it); the part being changed is ringed.
pub struct Editor {
    at: usize,
    info: Option<(Info, std::time::Instant)>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Editor {
        Editor { at: 0, info: None }
    }
}

impl Screen for Editor {
    fn title(&self) -> &str {
        "Customise"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let all = params();
        self.at = self.at.min(all.len() - 1);
        let step = match key {
            Key::Back => return Nav::Pop,
            Key::Up => {
                self.at = (self.at + all.len() - 1) % all.len();
                return Nav::Stay;
            }
            Key::Down | Key::Tab => {
                self.at = (self.at + 1) % all.len();
                return Nav::Stay;
            }
            Key::Left => -1,
            Key::Right | Key::Enter | Key::Char(' ') => 1,
            _ => return Nav::Stay,
        };
        let c = &mut cx.settings.lock;
        match all[self.at] {
            Param::Clock => c.lock_face = c.lock_face.step(step),
            Param::Colour => c.colour = c.colour.step(step),
            Param::Item(s) => c.toggle(s),
        }
        crate::save_settings(cx.settings);
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let choices = g.lock;
        if self.info.as_ref().is_none_or(|(_, at)| at.elapsed().as_secs() >= 10) {
            self.info = Some((info_now(robot, g.clock.0, crate::power::reading_now()), std::time::Instant::now()));
        }
        let info = self.info.as_ref().map(|(i, _)| i.clone()).unwrap_or_default();
        let layout = draw_face(g, &choices, &info, crate::faces::local_now(), true);
        let places = layout.places;
        let all = params();
        let param = all[self.at.min(all.len() - 1)];
        let part = match param {
            Param::Clock | Param::Colour => Part::Time,
            Param::Item(Show::Date) => Part::Date,
            Param::Item(s) => Part::Item(s),
        };
        if let Some((_, r)) = places.iter().find(|(p, _)| *p == part) {
            let radius = if matches!(part, Part::Item(_)) { r.h * 0.5 } else { g.radius(10.0) };
            g.focus_ring(*r, radius);
        }
        // The bar, where the field goes: what is being changed, its value,
        // the way out (in fewer words where the bar is narrower).
        let (cx, most) = layout.field;
        let bar = Rect::new((cx - most * 0.5).round(), FIELD_Y, most.round(), FIELD_H);
        g.glass(&[GlassShape::new(bar, bar.h * 0.5, 2)]);
        let (name, value) = match param {
            Param::Clock => ("clock", choices.lock_face.name().to_owned()),
            Param::Colour => ("colour", choices.colour.name().to_owned()),
            Param::Item(s) => (s.name(), (if choices.shows(s) { "shown" } else { "hidden" }).to_owned()),
        };
        let cy = (bar.cy() + ty.caption.size * 0.36).round();
        let value_cy = (bar.cy() + ty.label.size * 0.36).round();
        let boxed = format!("‹ {value} ›");
        // As much as fits: the keys, what, its value and the way out; or
        // what and its value, and "esc"; or what and its value; or (the
        // largest text, beside the stacked clock) the value alone, the ring
        // saying what it is of.
        for (left, right) in bar_words(name) {
            let lw = if left.is_empty() { 0.0 } else { g.measure(&left, ty.caption) + 14.0 };
            let rw = if right.is_empty() { 0.0 } else { g.measure(right, ty.caption) + 14.0 };
            if lw + g.measure(&boxed, ty.label) + rw + 28.0 <= bar.w || left.is_empty() {
                g.text.draw(&mut g.canvas, bar.x + 14.0, cy, &left, &ty.caption, t.dim, 1.0);
                g.text.draw_aligned(&mut g.canvas, bar.right() - 14.0, cy, Align::Right, right, &ty.caption, t.dim, 1.0);
                // The value in the room the others leave.
                let middle = (bar.x + 14.0 + lw + bar.right() - 14.0 - rw) * 0.5;
                g.text.draw_aligned(&mut g.canvas, middle, value_cy, Align::Center, &boxed, &ty.label, t.ink, 1.0);
                return;
            }
        }
    }
}

/// The editor bar's words about `name`, fullest first.
fn bar_words(name: &str) -> [(String, &'static str); 4] {
    [(format!("↑↓ {name}"), "esc done"), (name.to_owned(), "esc"), (name.to_owned(), ""), (String::new(), "")]
}

// --------------------------------------------------------------- StandBy ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StandByRow {
    On,
    Night,
    Show,
}

const STANDBY_ROWS: [StandByRow; 3] = [StandByRow::On, StandByRow::Night, StandByRow::Show];

/// Settings › StandBy.
pub struct StandByPage {
    rows: Rows,
}

impl Default for StandByPage {
    fn default() -> Self {
        Self::new()
    }
}

impl StandByPage {
    pub fn new() -> StandByPage {
        StandByPage { rows: Rows::new() }
    }

    pub fn at(mut self, row: usize) -> StandByPage {
        self.rows.focus = row.min(STANDBY_ROWS.len() - 1);
        self
    }
}

impl Screen for StandByPage {
    fn title(&self) -> &str {
        "StandBy"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if key == Key::Back {
            return Nav::Pop;
        }
        if self.rows.key(key, STANDBY_ROWS.len()) {
            return Nav::Stay;
        }
        let Some(row) = STANDBY_ROWS.get(self.rows.focus).copied() else { return Nav::Stay };
        let c = &mut cx.settings.lock;
        match (row, key) {
            (StandByRow::On, Key::Enter | Key::Left | Key::Right) => c.standby = !c.standby,
            (StandByRow::Night, Key::Enter | Key::Left | Key::Right) => c.night = !c.night,
            (StandByRow::Show, Key::Enter | Key::Right) => return Nav::Push(Box::new(Preview::new(*c, cx.settings.calm))),
            _ => return Nav::Stay,
        }
        crate::save_settings(cx.settings);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.rows.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let c = g.lock;
        let onoff = |on: bool| (if on { "on" } else { "off" }).to_owned();
        let rows = vec![
            ("On the charger".to_owned(), onoff(c.standby), c.standby),
            ("Night".to_owned(), (if c.night { "dim and red" } else { "off" }).to_owned(), c.night),
            ("Show it now".to_owned(), "›".to_owned(), true),
        ];
        let note = standby_note(g.clock.0);
        self.rows.draw(g, &rows, &note);
    }
}

/// StandBy shown now, off the charger too: its faces as they show there,
/// the arrows working; any other key goes back.
pub struct Preview {
    faces: Faces,
    choices: Choices,
    reading: Option<crate::power::Reading>,
    read_at: std::time::Instant,
    calm: bool,
    h24: bool,
    ticked: std::time::Instant,
    /// Music, for its face, read as the shell reads it.
    acts: Vec<crate::liveactivity::Activity>,
}

impl Preview {
    pub fn new(choices: Choices, calm: bool) -> Preview {
        let mut faces = Faces::new();
        faces.show(&choices, true);
        let now = std::time::Instant::now();
        Preview { faces, choices, reading: crate::power::reading_now(), read_at: now, calm, h24: true, ticked: now, acts: Vec::new() }
    }
}

impl Screen for Preview {
    fn title(&self) -> &str {
        "StandBy"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.faces.key(key, &mut cx.settings.lock, cx.settings.calm) {
            self.choices = cx.settings.lock;
            crate::save_settings(cx.settings);
            return Nav::Stay;
        }
        Nav::Pop
    }

    fn tick(&mut self, dt: f32) -> bool {
        if self.read_at.elapsed().as_secs() >= 5 {
            self.reading = crate::power::reading_now();
            self.read_at = std::time::Instant::now();
        }
        // The clock's time for the page dots' tucking away (the shell gives
        // screens no time while nothing moves).
        let wall = self.ticked.elapsed().as_secs_f32().min(0.25);
        self.ticked = std::time::Instant::now();
        self.acts = crate::liveactivity::music(crate::nowplaying::get().as_ref()).into_iter().collect();
        let feed = Feed { reading: self.reading.as_ref(), h24: self.h24, notices: 0, generation: 0, activities: &self.acts, opened: None, open: 0.0, level: 1.0 };
        let (moving, changed) = self.faces.tick(wall, dt, &feed, &self.choices, self.calm);
        moving || changed
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        self.h24 = g.clock.0;
        let feed = Feed { reading: self.reading.as_ref(), h24: self.h24, notices: 0, generation: robot.schedule.generation(), activities: &self.acts, opened: None, open: 0.0, level: 1.0 };
        self.choices = g.lock;
        self.faces.draw(g, robot, &feed, &self.choices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pages_fit_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            // Titles beside the island: no wider than Settings' own "Appearance".
            for title in ["Lock screen", "StandBy", "Passcode"] {
                assert!(g.measure(title, ty.title) <= g.measure("Appearance", ty.title) + 9.0, "{title} at {size:?}");
            }
            let rows = [
                ("Set a passcode", "off ›"),
                ("Change passcode", "on ›"),
                ("Turn passcode off", "›"),
                ("Customise lock screen", "stacked ›"),
                ("Notices on it", "words shown"),
                ("On the charger", "off"),
                ("Night", "dim and red"),
                ("Show it now", "›"),
            ];
            for (name, value) in rows {
                assert!(g.measure(name, ty.body) + 30.0 + g.measure(value, ty.label) <= W - 2.0 * PAD, "{name} at {size:?}");
            }
            // The editor's bar, at its narrowest (beside the stacked clock,
            // about 196 px): its value always whole, and what it is of too,
            // but at the largest text size.
            let narrow = 196.0;
            for (name, value) in [("next match", "‹ hidden ›"), ("next reminder", "‹ shown ›"), ("clock", "‹ stacked ›"), ("colour", "‹ accent ›")] {
                assert!(g.measure(value, ty.label) + 28.0 <= narrow);
                if size != crate::gfx::TextSize::Large {
                    assert!(g.measure(name, ty.caption) + 14.0 + g.measure(value, ty.label) + 28.0 <= narrow, "{name} at {size:?}");
                }
            }
            for words in ["Type the passcode you have now", "Type a new passcode", "Type it again"] {
                assert!(g.measure(words, ty.name) <= W - 2.0 * PAD, "{words} at {size:?}");
            }
            assert!(g.measure("enter: next · esc: back", ty.caption) <= W - 2.0 * PAD);
        }
    }

    #[test]
    fn the_notes_say_what_the_passcode_does_not_cover() {
        for words in ["SSH", "setup page", "card", "rm /data/catalyst/passcode"] {
            assert!(LOCK_NOTE.contains(words), "{words}");
        }
        assert!(standby_note(false).contains("10 pm to 7 am"));
    }
}
