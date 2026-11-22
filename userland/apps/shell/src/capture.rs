//! Quick capture: Ctrl-Shift-N from anywhere (an app, the Terminal, a
//! full-screen page, over the control centre or Spotlight, never under the
//! lock) brings up a sheet of glass grown out of the island. Type, press
//! Enter, and it goes where it belongs:
//!
//! - **A note**, by default: a new note in Notes, stamped at its foot as
//!   any new note is (the match coming up, and when).
//! - **A reminder**, when it starts with "remind" or says a time ("swap
//!   battery in 10 min", "before Q42 charge the spare"): read by Reminders'
//!   own quick add, as it would be typed there. The sheet says which, and
//!   when, as it is typed; Tab turns a note into a reminder or back.
//! - **A photo**: Ctrl-P opens the camera out of the sheet; Enter (or
//!   Ctrl-P again) takes it into Photos, and the camera gives way at once.
//! - **Spoken**: the talk key (Alt) held in the sheet writes down what is
//!   said, there.
//! - **The clipboard**: Ctrl-V puts it in; Ctrl-C copies what is typed.
//!
//! Afterwards the island says where it went for a few seconds, and Ctrl-Enter
//! meanwhile opens it there (Ctrl-Enter in the sheet saves and opens it at
//! once). Esc puts the sheet away with what was typed kept for next time;
//! Fn-Backspace clears it.

use std::cell::RefCell;
use std::path::PathBuf;

use bezel::Rect;

use crate::gfx::Gfx;
use crate::platform::Key;
use crate::robot::Robot;
use crate::sheet::{first_fitting, fit_tail, keys_line, text_right, Grown, INSET};

/// The most a capture holds.
const MOST: usize = 2000;
/// Lines of what is typed in sight at once; the sheet grows to hold them.
const LINES: usize = 3;
/// The title quick capture's dictation goes by (`voice::Target`).
pub const TITLE: &str = "Quick capture";
/// How long the island says where a capture went, and Ctrl-Enter opens it.
pub const RECEIPT_SECONDS: f32 = 4.5;

/// Where a capture goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Note,
    Reminder,
}

/// What is typed reads as, just now.
#[derive(Clone, Debug, PartialEq)]
struct Reading {
    dest: Dest,
    /// When a reminder comes ("in 10 min"), or why it cannot be one.
    when: String,
    /// It cannot be saved as it stands (a reminder with nothing to say).
    trouble: Option<String>,
}

/// Where something captured went, for the island to say and Ctrl-Enter to open.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Note(PathBuf),
    Reminder(u64),
    Photo(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    pub place: Place,
    /// The island's words, longest first: the first that fits is said.
    pub words: Vec<String>,
}

impl Receipt {
    fn new(place: Place, said: &str) -> Receipt {
        let (said, short) = match &place {
            Place::Note(_) => (format!("saved to notes{said}"), "in notes"),
            Place::Reminder(_) => (format!("reminder{said}"), "in reminders"),
            Place::Photo(_) => (format!("saved to photos{said}"), "in photos"),
        };
        let words = vec![format!("{said} · ctrl enter opens"), format!("{short} · ctrl enter opens"), said, short.to_owned()];
        Receipt { place, words }
    }

    /// What it was saved in, open on it.
    pub fn open(&self) -> Result<Box<dyn crate::shell::Screen>, String> {
        match &self.place {
            Place::Note(path) => Ok(Box::new(crate::files::Editor::open(path)?)),
            Place::Reminder(id) => Ok(crate::reminders::open(*id)),
            Place::Photo(path) if path.exists() => Ok(Box::new(crate::photos::Album::at(path))),
            Place::Photo(_) => Err("that photo is gone".into()),
        }
    }
}

/// What a key asks of the shell.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    /// Put away, what was typed kept.
    Close,
    /// Saved: the island says where; `open`, and it opens there now.
    Saved(Receipt, bool),
    /// The camera, out of the sheet, for a photo.
    Photo,
    Say(String),
}

pub struct Capture {
    sheet: Grown,
    text: String,
    /// Tab chose where it goes, over what it reads as.
    chosen: Option<Dest>,
    reading: Reading,
    /// What the reading was worked out for.
    read_for: Option<(String, Option<Dest>)>,
    /// Lines of the text as last laid out, for the sheet's height.
    lines: usize,
    /// What a key just said (the island is under the sheet), shown where
    /// it goes for a moment, and for how long more.
    note: Option<(String, f32)>,
}

/// How long the sheet says what a key did.
const NOTE_SECONDS: f32 = 2.4;

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

/// The sheet's measures: a line of what is typed, where it goes, the keys.
fn metrics(g: &Gfx) -> (f32, f32, f32) {
    let line = (g.ty.name.size + 5.0).round();
    let dest = g.row_h();
    let keys = (g.ty.caption.size + 10.0).round();
    (line, dest, keys)
}

/// The words that start a reminder ("remind me to …", "reminder: …"): the
/// rest, as Reminders' quick add should read it, if the line starts so.
fn after_remind(text: &str) -> Option<String> {
    let t = text.trim_start();
    let low = t.to_lowercase();
    // "remind me to" and "remind me" are quick add's own to take out.
    if low.starts_with("remind me") {
        return Some(t.to_owned());
    }
    for lead in ["reminder", "remind"] {
        if let Some(rest) = low.strip_prefix(lead) {
            if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || ":,-–—".contains(c)) {
                return Some(t[lead.len()..].trim_start_matches(|c: char| c.is_whitespace() || ":,-–—".contains(c)).to_owned());
            }
        }
    }
    None
}

/// Where `text` goes, and what to say of it: a reminder if it starts with
/// "remind" or says a time (as Reminders reads one), else a note. One of
/// several lines is a note, whatever its words.
fn read(text: &str, chosen: Option<Dest>, robot: &Robot, h24: bool) -> Reading {
    let note = Reading { dest: Dest::Note, when: String::new(), trouble: None };
    if text.trim().is_empty() {
        return Reading { dest: chosen.unwrap_or(Dest::Note), when: String::new(), trouble: None };
    }
    let reminded = after_remind(text);
    let typed = reminded.clone().unwrap_or_else(|| text.to_owned());
    let several = text.trim().contains('\n');
    let reading = if several { None } else { Some(crate::reminders::capture_reading(&typed, robot, h24)) };
    let reminder = |reading: Option<Result<(bool, String), String>>| match reading {
        Some(Ok((_, when))) => Reading { dest: Dest::Reminder, when: when.replace("no time: on today's list", "today's list"), trouble: None },
        Some(Err(why)) => Reading { dest: Dest::Reminder, when: String::new(), trouble: Some(why) },
        None => Reading { dest: Dest::Reminder, when: String::new(), trouble: Some("a reminder is one line".into()) },
    };
    match (chosen, &reading) {
        (Some(Dest::Note), _) => note,
        (Some(Dest::Reminder), _) => reminder(reading),
        (None, Some(Ok((timed, _)))) if *timed || reminded.is_some() => reminder(reading),
        (None, Some(Err(_))) if reminded.is_some() => reminder(reading),
        _ => note,
    }
}

impl Capture {
    pub fn new() -> Self {
        Capture { sheet: Grown::new(), text: String::new(), chosen: None, reading: Reading { dest: Dest::Note, when: String::new(), trouble: None }, read_for: None, lines: 1, note: None }
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

    /// What is typed (the tests, the gallery).
    #[allow(dead_code)]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where it would go just now (the tests).
    #[cfg(test)]
    pub fn dest(&self) -> Dest {
        self.reading.dest
    }

    /// Bring it up, with what was typed last time still there.
    pub fn summon(&mut self, g: &Gfx, calm: bool, robot: &Robot, h24: bool) {
        self.reread(robot, h24);
        let height = self.height_for(g, self.lines);
        self.sheet.summon(calm, height);
    }

    pub fn dismiss(&mut self) {
        self.sheet.dismiss();
    }

    pub fn gone_now(&mut self) {
        self.sheet.gone_now();
    }

    /// Where the sheet is, for the camera to open out of.
    pub fn rect(&self) -> Rect {
        self.sheet.rest()
    }

    fn height_for(&self, g: &Gfx, lines: usize) -> f32 {
        let (line, dest, keys) = metrics(g);
        let lines = lines.clamp(1, LINES) as f32;
        (lines * line + 10.0).max((g.ty.name.size + 15.0).round()) + 1.0 + dest + 4.0 + keys
    }

    fn reread(&mut self, robot: &Robot, h24: bool) {
        let key = (self.text.clone(), self.chosen);
        if self.read_for.as_ref() == Some(&key) {
            return;
        }
        self.reading = read(&self.text, self.chosen, robot, h24);
        self.read_for = Some(key);
    }

    /// Words put in where the typing is: what was pasted (its lines kept) or said.
    pub fn put(&mut self, words: &str, robot: &Robot, h24: bool) {
        let words: String = words.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
        let words = words.trim_end_matches('\n');
        if words.trim().is_empty() {
            return;
        }
        if !self.text.is_empty() && !self.text.ends_with([' ', '\n']) && !words.starts_with([' ', '\n']) {
            self.text.push(' ');
        }
        for c in words.chars() {
            if self.text.chars().count() >= MOST {
                break;
            }
            self.text.push(c);
        }
        self.reread(robot, h24);
    }

    /// What was said, written down, for the sheet: in at the end of the line.
    pub fn dictated(&mut self, words: &str, robot: &Robot, h24: bool) -> String {
        self.put(words.trim(), robot, h24);
        let n = words.split_whitespace().count();
        if n == 1 { "typed 1 word".into() } else { format!("typed {n} words") }
    }

    /// A key while it is open. What it says is shown on the sheet too.
    pub fn key(&mut self, key: Key, robot: &Robot, h24: bool) -> Outcome {
        let outcome = self.act(key, robot, h24);
        self.note = match &outcome {
            Outcome::Say(words) => Some((words.clone(), NOTE_SECONDS)),
            Outcome::Stay => None,
            _ => self.note.take(),
        };
        outcome
    }

    fn act(&mut self, key: Key, robot: &Robot, h24: bool) -> Outcome {
        match key {
            Key::Back | Key::Leave | Key::Ctrl('q') | crate::clipboard::CAPTURE_KEY => return Outcome::Close,
            Key::Enter | Key::Ctrl('\n') => return self.save(robot, h24, key == Key::Ctrl('\n')),
            Key::Ctrl('p' | 'P') => return Outcome::Photo,
            Key::Tab => {
                self.reread(robot, h24);
                self.chosen = Some(if self.reading.dest == Dest::Note { Dest::Reminder } else { Dest::Note });
            }
            Key::Ctrl('v' | 'V') => match crate::clipboard::paste_text() {
                Some(text) => self.put(&text, robot, h24),
                None => return Outcome::Say("the clipboard is empty".into()),
            },
            Key::Ctrl('c' | 'C') if !self.text.trim().is_empty() => crate::clipboard::copy(self.text.trim(), crate::clipboard::Sensitivity::Normal),
            Key::Char(c) if !c.is_control() && self.text.chars().count() < MOST => self.text.push(c),
            Key::Backspace => {
                self.text.pop();
                if self.text.trim().is_empty() {
                    self.chosen = None;
                }
            }
            // Fn-Backspace: all of it.
            Key::Delete => {
                self.text.clear();
                self.chosen = None;
            }
            _ => {}
        }
        self.reread(robot, h24);
        Outcome::Stay
    }

    /// Enter: it goes where it reads as going. What was typed is cleared.
    fn save(&mut self, robot: &Robot, h24: bool, open: bool) -> Outcome {
        self.reread(robot, h24);
        let text = self.text.trim().to_owned();
        if text.is_empty() {
            return Outcome::Say("type something first".into());
        }
        if let Some(why) = &self.reading.trouble {
            return Outcome::Say(why.clone());
        }
        let receipt = match self.reading.dest {
            Dest::Note => match crate::notes::capture(&text, robot) {
                Ok(path) => Receipt::new(Place::Note(path), ""),
                Err(why) => return Outcome::Say(why),
            },
            Dest::Reminder => {
                let typed = after_remind(&text).unwrap_or(text);
                match crate::reminders::capture_add(&typed, robot, h24) {
                    Ok((id, count, when)) => {
                        let when = when.replace("no time: on today's list", "on today's list");
                        let said = if count > 1 { format!("s · {count} · {when}") } else { format!(" · {when}") };
                        Receipt::new(Place::Reminder(id), &said)
                    }
                    Err(why) => return Outcome::Say(why),
                }
            }
        };
        self.text.clear();
        self.chosen = None;
        self.read_for = None;
        Outcome::Saved(receipt, open)
    }

    /// Advance its motion (`motion_dt`) and its clock (`dt`).
    pub fn tick(&mut self, dt: f32, motion_dt: f32) -> bool {
        let mut moving = self.sheet.tick(motion_dt);
        // What a key said goes on the clock, not the motion.
        if let Some((_, left)) = &mut self.note {
            *left -= dt;
            if *left <= 0.0 {
                self.note = None;
                moving = true;
            }
        }
        moving
    }

    /// What sits on the glass: what is typed, where it goes, the keys. With
    /// the talk key held, it says it is listening, its meter live.
    pub fn draw(&mut self, g: &mut Gfx, island: Rect, talk: Option<&crate::voice::Island>) {
        let (line_h, dest_h, keys_h) = metrics(g);
        let t = g.tokens;
        let ty = g.ty;
        let style = ty.name;
        // What is typed, wrapped to the sheet at rest (so its lines hold
        // still as it grows), its last lines in sight.
        let room = self.sheet.rest().w - 30.0 - 14.0;
        let mut lines: Vec<String> = Vec::new();
        for paragraph in self.text.split('\n') {
            let wrapped = crate::help::wrap(g, paragraph, style, room - 4.0);
            if wrapped.is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(wrapped);
            }
        }
        // A long last word with no space to break at: its end, where the typing is.
        if let Some(last) = lines.last_mut() {
            if g.measure(last, style) > room - 4.0 {
                *last = fit_tail(g, last, style, room - 4.0);
            }
        }
        // A space typed last (wrap leaves it out): the caret goes after it.
        let trailing = self.text.ends_with(' ') && !lines.last().is_some_and(|l| l.ends_with(' '));
        self.lines = lines.len().max(1);
        let height = self.height_for(g, self.lines);
        self.sheet.set_height(g, height);
        let panel = self.sheet.glass(g, island).rect;
        let words = self.sheet.words();
        if words <= 0.0 {
            return;
        }
        // Laid out from its top as it is at rest, and clipped to the glass:
        // as it grows (or its height glides) more of it shows, nothing moves
        // into anything else.
        let rest_h = self.sheet.rest().h;
        let text_x = panel.x + 30.0;
        g.canvas.set_clip(panel.inset(1.0));
        let shown = self.lines.min(LINES);
        let text_h = (shown as f32 * line_h + 10.0).max((ty.name.size + 15.0).round());
        let first_cy = panel.y + (text_h - shown as f32 * line_h) * 0.5 + line_h * 0.5;
        // What it is, or, with the talk key held, the microphone (live: the signal colour).
        let hot = talk.is_some_and(|t| t.hot);
        let lead = if talk.is_some() { "mic" } else { "edit_note" };
        g.icon_alpha(lead, hot, panel.x + 16.0, first_cy, 15.0, if hot { t.signal } else { t.dim }, words);
        if self.text.is_empty() {
            let placeholder = first_fitting(g, &["A note, or remind me in 10 min", "A note, or a reminder", "Write it down"], style, room - 8.0);
            g.text_alpha(text_x + 4.0, first_cy, placeholder, style, t.faint, words);
            let caret = Rect::new(text_x.round(), (first_cy - style.size * 0.55).round(), 1.5, (style.size * 1.1).round());
            g.canvas.rrect(caret, 0.75, t.signal, words);
        } else {
            let from = lines.len().saturating_sub(LINES);
            for (k, line) in lines[from..].iter().enumerate() {
                let cy = first_cy + k as f32 * line_h;
                // More above than is in sight: the first line in sight fades,
                // as a list's row going under glass does.
                let ink = if k == 0 && from > 0 { 0.45 } else { 1.0 };
                g.text_alpha(text_x, cy, line, style, t.ink, words * ink);
                if k + from + 1 == lines.len() {
                    let w = g.measure(line, style) + if trailing { g.measure(" ", style) } else { 0.0 };
                    let caret = Rect::new((text_x + w + 1.0).round(), (cy - style.size * 0.55).round(), 1.5, (style.size * 1.1).round());
                    g.canvas.rrect(caret, 0.75, t.signal, words);
                }
            }
        }
        // Listening: the meter, at the end of the first line.
        if let Some(talk) = talk {
            let most = 12.0_f32;
            let mut x = panel.right() - 14.0 - (crate::voice::METER_ROOM - 8.0);
            for level in talk.bars {
                let h = (2.0 + (most - 2.0) * level).round().max(2.0);
                g.canvas.rrect(Rect::new(x, first_cy - h * 0.5, 2.0, h), 1.0, g.good(), words);
                x += 4.5;
            }
        }
        // Where it goes.
        let rule_y = panel.y + text_h;
        g.canvas.rrect(Rect::new(panel.x + 12.0, rule_y, panel.w - 24.0, 1.0), 0.5, t.ink, 0.10 * words);
        let row = Rect::new(panel.x + INSET, rule_y + 3.0, panel.w - 2.0 * INSET, dest_h);
        let (icon, name) = match self.reading.dest {
            Dest::Note => ("sticky_note_2", "New note"),
            Dest::Reminder => ("notifications", "Reminder"),
        };
        // Beside it: what letting go of the talk key does, or when the
        // reminder comes (why it cannot be one), or where a note goes.
        let (detail, detail_ink) = match (talk, self.reading.dest, &self.reading.trouble) {
            (Some(talk), _, _) => (talk.words.clone(), t.dim),
            _ if self.note.is_some() => (self.note.as_ref().map(|(n, _)| n.clone()).unwrap_or_default(), t.ink),
            (None, Dest::Note, _) => ("notes".to_owned(), t.dim),
            (None, Dest::Reminder, Some(why)) => (why.clone(), g.fault()),
            (None, Dest::Reminder, None) if self.text.trim().is_empty() => ("say what, and when".to_owned(), t.dim),
            (None, Dest::Reminder, None) => (self.reading.when.clone(), t.ink),
        };
        let radius = (g.radius(crate::sheet::RADIUS) - INSET).max(2.0).min(row.h * 0.5);
        g.canvas.rrect(row, radius, t.ink, 0.07 * words);
        g.icon_alpha(icon, true, row.x + 13.0, row.cy(), 14.0, t.ink, words);
        let name_w = g.measure(name, ty.body);
        g.text_alpha(row.x + 26.0, row.cy(), name, ty.body, t.ink, words);
        let detail_room = row.w - 26.0 - name_w - 12.0 - 9.0;
        let detail = g.text.fit(&detail, &ty.caption, detail_room.max(0.0));
        text_right(g, row.right() - 9.0, row.cy(), &detail, ty.caption, detail_ink, words);
        // The keys, along its foot (as it rests).
        let keys_cy = panel.y + rest_h - keys_h * 0.5 - 1.0;
        let other = if self.reading.dest == Dest::Note { "reminder" } else { "note" };
        keys_line(g, panel.x + 12.0, panel.right() - 12.0, keys_cy, &[("enter", "save"), ("tab", other), ("ctrl p", "photo"), ("ctrl v", "paste"), ("alt", "talk")], words);
        g.canvas.clear_clip();
    }
}

// ---------------------------------------------------------------- photos ---

thread_local! {
    /// A photo the camera took for quick capture, for the shell to say.
    static TAKEN: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The camera took quick capture's photo (camera.rs, on the shell's thread).
pub fn photo_taken(path: PathBuf) {
    TAKEN.with_borrow_mut(|t| *t = Some(path));
}

/// The photo quick capture's camera took, once: what the island says.
pub fn take_photo() -> Option<Receipt> {
    TAKEN.with_borrow_mut(Option::take).map(|path| Receipt::new(Place::Photo(path), ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(c: &mut Capture, robot: &Robot, text: &str) {
        for ch in text.chars() {
            c.key(Key::Char(ch), robot, true);
        }
    }

    #[test]
    fn plain_words_are_a_note_and_a_time_or_remind_makes_a_reminder() {
        let robot = Robot::none();
        let mut c = Capture::new();
        typed(&mut c, &robot, "intake belt slips at full speed");
        assert_eq!(c.dest(), Dest::Note);
        c.key(Key::Delete, &robot, true);
        typed(&mut c, &robot, "swap battery in 10 min");
        assert_eq!(c.dest(), Dest::Reminder);
        assert_eq!(c.reading.when, "in 10 min");
        c.key(Key::Delete, &robot, true);
        typed(&mut c, &robot, "remind me to charge the spare");
        assert_eq!(c.dest(), Dest::Reminder, "remind, with no time: today's list");
        assert_eq!(c.reading.when, "today's list");
        c.key(Key::Delete, &robot, true);
        typed(&mut c, &robot, "reminder: zip ties");
        assert_eq!(c.dest(), Dest::Reminder);
        assert_eq!(after_remind("reminder: zip ties").as_deref(), Some("zip ties"));
        assert_eq!(after_remind("reminders are good"), None, "a word that only starts so");
        assert_eq!(after_remind("Remind me to x").as_deref(), Some("Remind me to x"), "quick add takes that out itself");
    }

    #[test]
    fn tab_turns_a_note_into_a_reminder_and_back() {
        let robot = Robot::none();
        let mut c = Capture::new();
        typed(&mut c, &robot, "check bumpers");
        assert_eq!(c.dest(), Dest::Note);
        c.key(Key::Tab, &robot, true);
        assert_eq!(c.dest(), Dest::Reminder);
        c.key(Key::Tab, &robot, true);
        assert_eq!(c.dest(), Dest::Note);
        // Lines pasted in make a note, whatever their words.
        c.key(Key::Delete, &robot, true);
        c.put("swap battery in 10 min\nand the bumpers", &robot, true);
        assert_eq!(c.dest(), Dest::Note);
    }

    #[test]
    fn enter_saves_a_reminder_through_quick_add_and_clears_the_sheet() {
        let robot = Robot::none();
        let mut c = Capture::new();
        typed(&mut c, &robot, "swap battery in 10 min");
        let Outcome::Saved(receipt, false) = c.key(Key::Enter, &robot, true) else { panic!("saved") };
        let Place::Reminder(id) = receipt.place else { panic!("a reminder") };
        assert_eq!(receipt.words[0], "reminder · in 10 min · ctrl enter opens");
        assert!(crate::reminders::with(|s| s.book.get(id).is_some_and(|r| r.text == "swap battery")));
        assert!(c.text().is_empty(), "ready for the next");
        assert_eq!(c.key(Key::Enter, &robot, true), Outcome::Say("type something first".into()));
    }

    #[test]
    fn esc_keeps_what_was_typed_and_the_keys_paste_and_photo() {
        let robot = Robot::none();
        let mut c = Capture::new();
        typed(&mut c, &robot, "half");
        assert_eq!(c.key(Key::Back, &robot, true), Outcome::Close);
        assert_eq!(c.text(), "half", "kept for next time");
        crate::clipboard::copy("10.58.5.2", crate::clipboard::Sensitivity::Normal);
        c.key(Key::Ctrl('v'), &robot, true);
        assert_eq!(c.text(), "half 10.58.5.2");
        assert_eq!(c.key(Key::Ctrl('p'), &robot, true), Outcome::Photo);
        assert_eq!(c.key(crate::clipboard::CAPTURE_KEY, &robot, true), Outcome::Close);
        assert_eq!(c.dictated("and the belt", &robot, true), "typed 3 words");
        assert_eq!(c.text(), "half 10.58.5.2 and the belt");
    }

    #[test]
    fn its_height_fits_the_panel_at_every_text_size() {
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            let mut g = Gfx::new(bezel::Tokens::dark());
            g.set_text_size(size);
            let c = Capture::new();
            let most = c.height_for(&g, 10);
            assert!(crate::sheet::TOP + most <= crate::gfx::H - crate::sheet::FOOT, "{}: {most}", size.name());
        }
    }
}
