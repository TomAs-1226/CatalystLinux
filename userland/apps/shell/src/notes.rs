//! Notes: quick plain-text notes, one file each in `/data/notes`, written in
//! the editor Files uses. The first line is a note's title. A new note is
//! stamped at its foot with when it was started and, when the event
//! schedule knows it, the match coming up ("— Q27 · nov 21 09:45"), so pit
//! notes find their match. Pinned notes stay at the top; `/` searches titles
//! and text alike. A note left empty is not kept.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bezel::Rect;

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// Where notes are kept (Spotlight and Reminders read them too).
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/notes")
    } else {
        crate::config_dir().join("notes")
    }
}

const PINNED: &str = ".pinned";

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub file: String,
    pub title: String,
    pub text: String,
    pub modified: i64,
    pub pinned: bool,
}

/// A note's title: its first line with words, without a heading's #.
pub fn title_of(text: &str) -> String {
    text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or("Empty note").chars().take(60).collect()
}

/// Every note in `dir`: pinned first, then the latest changed.
pub fn load(dir: &Path) -> Vec<Note> {
    let pinned: Vec<String> = std::fs::read_to_string(dir.join(PINNED)).unwrap_or_default().lines().map(str::to_owned).collect();
    let mut notes: Vec<Note> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let modified = e.metadata().ok()?.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as i64;
            let file = e.file_name().to_string_lossy().into_owned();
            Some(Note { pinned: pinned.contains(&file), title: title_of(&text), text, modified, file })
        })
        .collect();
    notes.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.modified.cmp(&a.modified)).then(a.file.cmp(&b.file)));
    notes
}

/// Pin a note, or unpin it.
pub fn toggle_pin(dir: &Path, file: &str) -> bool {
    let mut pinned: Vec<String> = std::fs::read_to_string(dir.join(PINNED)).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_owned).collect();
    let now = if let Some(i) = pinned.iter().position(|p| p == file) {
        pinned.remove(i);
        false
    } else {
        pinned.push(file.to_owned());
        true
    };
    std::fs::write(dir.join(PINNED), pinned.join("\n") + "\n").ok();
    now
}

/// Whether a note matches a search: its words, any case.
pub fn matches(note: &Note, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || note.text.to_lowercase().contains(&q) || note.title.to_lowercase().contains(&q)
}

/// The foot a new note gets: the match coming up, if known, and when.
pub fn stamp(label: Option<&str>, now: chrono::DateTime<chrono::Local>) -> String {
    let when = now.format("%b %-d %H:%M").to_string().to_lowercase();
    match label {
        Some(label) => format!("— {label} · {when}"),
        None => format!("— {when}"),
    }
}

/// When a note was last changed, as a list says it.
pub fn age(modified: i64, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::TimeZone;
    let Some(then) = chrono::Local.timestamp_opt(modified, 0).single() else { return String::new() };
    let days = (now.date_naive() - then.date_naive()).num_days();
    match days {
        0 => then.format("%H:%M").to_string(),
        1 => "yesterday".into(),
        2..=6 => then.format("%a").to_string().to_lowercase(),
        _ => then.format("%b %-d").to_string().to_lowercase(),
    }
}

pub struct Notes {
    dir: PathBuf,
    notes: Vec<Note>,
    list: ListState,
    search: Option<Field>,
    query: String,
    deleting: Option<String>,
    /// A note just started, and what it was started with: removed again on
    /// coming back if nothing was written.
    fresh: Option<(PathBuf, String)>,
}

impl Notes {
    pub fn new() -> Self {
        let dir = dir();
        Notes { notes: load(&dir), dir, list: ListState::default(), search: None, query: String::new(), deleting: None, fresh: None }
    }

    fn shown(&self) -> Vec<Note> {
        self.notes.iter().filter(|n| matches(n, &self.query)).cloned().collect()
    }

    fn start(&mut self, robot: &Robot) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|_| "cannot make the notes folder".to_owned())?;
        let now = chrono::Local::now();
        let label = robot.schedule.view().and_then(|v| v.next(chrono::Utc::now().timestamp()).map(|m| m.label.clone()));
        let text = format!("\n\n{}\n", stamp(label.as_deref(), now));
        let mut name = now.format("%Y-%m-%d-%H%M%S").to_string();
        while self.dir.join(format!("{name}.txt")).exists() {
            name.push('b');
        }
        let path = self.dir.join(format!("{name}.txt"));
        std::fs::write(&path, &text).map_err(|_| "cannot write the note".to_owned())?;
        self.fresh = Some((path.clone(), text));
        Ok(path)
    }
}

impl Screen for Notes {
    fn title(&self) -> &str {
        "Notes"
    }

    fn wants_text(&self) -> bool {
        self.search.is_some()
    }

    fn resume(&mut self) {
        if let Some((path, text)) = self.fresh.take() {
            if std::fs::read_to_string(&path).is_ok_and(|now| now.trim() == text.trim()) {
                std::fs::remove_file(&path).ok();
            }
        }
        self.notes = load(&self.dir);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.search {
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) => {
                    self.query = text;
                    self.search = None;
                    self.list.focus = 0;
                }
                Typed::Cancelled => self.search = None,
                Typed::More => {
                    self.query = field.text.clone();
                    self.list.focus = 0;
                }
            }
            return Nav::Stay;
        }
        let shown = self.shown();
        let armed = self.deleting.take();
        let focused = self.list.focus.checked_sub(1).and_then(|i| shown.get(i)).cloned();
        match key {
            Key::Back if !self.query.is_empty() => self.query.clear(),
            Key::Back => return Nav::Pop,
            Key::Char('n') => return self.open_new(cx),
            Key::Enter | Key::Right => match focused {
                None => return self.open_new(cx),
                Some(note) => match crate::files::Editor::open(&self.dir.join(&note.file)) {
                    Ok(editor) => return Nav::Push(Box::new(editor)),
                    Err(why) => *cx.say = Some(why),
                },
            },
            Key::Char('/') => self.search = Some(Field::new("search notes", &self.query, 40)),
            // A reminder for the note, which opens it when it comes.
            Key::Char('r') => {
                if let Some(note) = focused {
                    return Nav::Push(Box::new(crate::reminders::Compose::for_note(&note.file, &note.title)));
                }
            }
            Key::Char('p') => {
                if let Some(note) = focused {
                    let now = toggle_pin(&self.dir, &note.file);
                    *cx.say = Some(if now { "pinned".into() } else { "unpinned".into() });
                    self.notes = load(&self.dir);
                    self.list.focus = 1 + self.shown().iter().position(|n| n.file == note.file).unwrap_or(0);
                }
            }
            Key::Char('d') => {
                if let Some(note) = focused {
                    if armed.as_deref() == Some(note.file.as_str()) {
                        std::fs::remove_file(self.dir.join(&note.file)).ok();
                        if note.pinned {
                            toggle_pin(&self.dir, &note.file);
                        }
                        crate::reminders::note_deleted(&note.file);
                        *cx.say = Some(format!("deleted {}", note.title.to_lowercase()));
                        self.notes = load(&self.dir);
                        self.list.focus = self.list.focus.min(self.shown().len());
                    } else {
                        *cx.say = Some("d again deletes it".into());
                        self.deleting = Some(note.file);
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, shown.len() + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some(field) = &self.search {
            field.draw_at(g, true);
        }
        let shown = self.shown();
        let top = if self.search.is_some() { g.top() + Field::height(g) + 6.0 } else { g.top() };
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        let now = chrono::Local::now();
        // While a search is typed, the ring is on the field, not a row.
        let searching = self.search.is_some();
        let reminded = crate::reminders::noted();
        for (i, y) in self.list.layout(g, shown.len() + 1, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = !searching && i == self.list.focus;
            match i.checked_sub(1).and_then(|k| shown.get(k)) {
                None => g.row(r, "New note", "n", t.dim, focused),
                Some(note) => {
                    let when = age(note.modified, now);
                    let value = if note.pinned { format!("pinned · {when}") } else { when };
                    let value_w = g.measure(&value, g.ty.label);
                    // A note with a reminder wears a small bell before its time.
                    let bell = reminded.contains(&note.file);
                    let title = g.text.fit(&note.title, &g.ty.body, (r.w - value_w - 34.0 - if bell { 14.0 } else { 0.0 }).max(40.0));
                    g.row(r, &title, &value, if note.pinned { t.ink } else { t.dim }, focused);
                    if bell {
                        let value_w = g.measure(&value, if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label });
                        g.icon("notifications", true, r.right() - 10.0 - value_w - 9.0, r.cy(), 11.0, t.dim);
                    }
                }
            }
        }
        self.list.scrollbar(g, shown.len() + 1, top, bottom);
        if searching {
            let found = format!("{} found", shown.len());
            g.hint_any(&[&format!("{found} · enter: done · esc: cancel"), &format!("{found} · esc: cancel")]);
        } else if shown.is_empty() && !self.query.is_empty() {
            g.hint_any(&[&format!("nothing has \"{}\" · esc: all notes", self.query), "nothing found · esc: all notes"]);
        } else if self.list.focus == 0 {
            g.hint(if self.notes.is_empty() { "enter: your first note · /: search" } else { "enter: a new note · /: search" });
        } else {
            g.hint_any(&["enter: open · r: remind · p: pin · d: delete", "r: remind · p: pin · d: delete", "p: pin · d: delete · /: search"]);
        }
    }

    /// Push-to-talk on the list dictates a new note (in a note, the editor
    /// types what was said at the cursor).
    fn dictation(&self) -> Option<crate::voice::Dictation> {
        self.search.is_none().then_some(crate::voice::Dictation::Text)
    }

    /// A new note of what was said, kept at once and open to carry on.
    fn dictated(&mut self, text: &str, cx: &mut Cx) -> Nav {
        match self.start(cx.robot).and_then(|path| crate::files::Editor::open(&path)) {
            Ok(mut editor) => {
                editor.dictated(text, cx);
                if !editor.save() {
                    *cx.say = Some("could not keep the note".into());
                }
                Nav::Push(Box::new(editor))
            }
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }
}

impl Notes {
    fn open_new(&mut self, cx: &mut Cx) -> Nav {
        match self.start(cx.robot) {
            Ok(path) => match crate::files::Editor::open(&path) {
                Ok(editor) => Nav::Push(Box::new(editor)),
                Err(why) => {
                    *cx.say = Some(why);
                    Nav::Stay
                }
            },
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_notes_title_is_its_first_line_with_words() {
        assert_eq!(title_of("\n\n# Elevator\nbelt slipping"), "Elevator");
        assert_eq!(title_of("   \n"), "Empty note");
    }

    #[test]
    fn notes_list_pinned_first_then_latest_and_search_finds_words() {
        let dir = std::env::temp_dir().join(format!("catalyst-notes-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "Battery 3\nsagged to 6 V in Q14").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join("b.txt"), "Intake\nmove it off can_s0").unwrap();
        std::fs::write(dir.join("ignored.md"), "not a note").unwrap();
        let notes = load(&dir);
        assert_eq!(notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Intake", "Battery 3"]);
        assert!(toggle_pin(&dir, "a.txt"));
        assert_eq!(load(&dir)[0].title, "Battery 3", "pinned comes first");
        assert!(!toggle_pin(&dir, "a.txt"));
        let notes = load(&dir);
        assert!(matches(&notes[0], "CAN_S0"));
        assert!(!matches(&notes[0], "battery"));
        assert!(matches(&notes[1], "q14"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_note_is_stamped_with_the_match_and_the_time() {
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 21, 9, 45, 0).unwrap();
        assert_eq!(stamp(Some("Q27"), at), "— Q27 · nov 21 09:45");
        assert_eq!(stamp(None, at), "— nov 21 09:45");
        assert_eq!(age(at.timestamp(), at), "09:45");
        assert_eq!(age(at.timestamp() - 86_400, at), "yesterday");
        assert_eq!(age(at.timestamp() - 30 * 86_400, at), "oct 22");
    }
}
