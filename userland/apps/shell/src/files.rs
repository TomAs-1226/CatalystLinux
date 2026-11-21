//! Files and the editor: browse the handheld's storage, and read or change
//! a text file on it.

use std::fs;
use std::path::{Path, PathBuf};

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The largest file the editor opens: it holds the whole file in memory.
const EDITABLE: u64 = 512 * 1024;

// ----------------------------------------------------------------- files ---

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// An ordinary file, which the editor may open. Not a device, a pipe or
    /// a socket: reading `/dev/zero` would never end.
    pub file: bool,
    pub bytes: u64,
}

/// A folder's contents: folders first, then files, each by name. A link is
/// shown as what it points at, so a linked folder can be entered.
pub fn list(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = fs::metadata(e.path()).or_else(|_| e.metadata()).ok()?;
            Some(Entry { name: e.file_name().to_string_lossy().into_owned(), dir: meta.is_dir(), file: meta.is_file(), bytes: meta.len() })
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} mb", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} kb", b as f64 / 1024.0),
        b => format!("{b} b"),
    }
}

/// Where browsing starts, and the folder it may not climb above.
fn roots() -> (PathBuf, PathBuf) {
    if let Some(dir) = std::env::var_os("CATALYST_FILES") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).ok();
        return (dir.clone(), dir);
    }
    if cfg!(target_os = "linux") {
        // The whole system can be looked at; storage that persists is /data.
        let start = if Path::new("/data").is_dir() { "/data" } else { "/" };
        return (start.into(), "/".into());
    }
    let dir = std::env::temp_dir().join("catalyst-files");
    fs::create_dir_all(&dir).ok();
    (dir.clone(), dir)
}

pub struct Files {
    dir: PathBuf,
    root: PathBuf,
    entries: Vec<Entry>,
    list: ListState,
    /// The name being typed for a new file.
    naming: Option<String>,
    /// The entry `d` was pressed on once; a second press deletes it.
    deleting: Option<String>,
    title: String,
}

impl Files {
    pub fn new() -> Self {
        let (start, root) = roots();
        let mut files = Files { dir: PathBuf::new(), root, entries: Vec::new(), list: ListState::default(), naming: None, deleting: None, title: String::new() };
        files.enter(start, None);
        files
    }

    /// Show a folder, with the focus on `select` if it is there.
    fn enter(&mut self, dir: PathBuf, select: Option<&str>) {
        self.entries = list(&dir);
        self.title = dir.file_name().map_or_else(|| "Files".to_owned(), |n| n.to_string_lossy().into_owned());
        self.list = ListState::default();
        if let Some(at) = select.and_then(|name| self.entries.iter().position(|e| e.name == name)) {
            self.list.focus = at;
        }
        self.dir = dir;
        self.deleting = None;
    }

    fn up(&mut self) -> bool {
        if self.dir == self.root {
            return false;
        }
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else { return false };
        let from = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.enter(parent, from.as_deref());
        true
    }
}

impl Screen for Files {
    fn title(&self) -> &str {
        &self.title
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(name) = &mut self.naming {
            match key {
                Key::Char(c) if name.len() < 40 && (c.is_ascii_alphanumeric() || "._- ".contains(c)) => name.push(c),
                Key::Backspace => {
                    name.pop();
                }
                Key::Back => self.naming = None,
                Key::Enter => {
                    let name = self.naming.take().unwrap_or_default().trim().to_owned();
                    let path = self.dir.join(&name);
                    if name.is_empty() {
                    } else if path.exists() {
                        *cx.say = Some(format!("{name} is already here"));
                    } else if fs::write(&path, "").is_ok() {
                        let dir = self.dir.clone();
                        self.enter(dir, Some(&name));
                        return Nav::Push(Box::new(Editor::open(&path).expect("just made")));
                    } else {
                        *cx.say = Some("cannot make a file here".into());
                    }
                }
                _ => {}
            }
            return Nav::Stay;
        }
        let focused = self.entries.get(self.list.focus).cloned();
        let armed = self.deleting.take();
        match key {
            Key::Back | Key::Left => {
                if !self.up() && key == Key::Back {
                    return Nav::Pop;
                }
            }
            Key::Enter | Key::Right => match focused {
                Some(entry) if entry.dir => self.enter(self.dir.join(&entry.name), None),
                Some(entry) if !entry.file => *cx.say = Some("not a file the editor can open".into()),
                // A photo opens in the Photos viewer, however large.
                Some(entry) if crate::photos::is_photo(&entry.name) => return Nav::Push(Box::new(crate::photos::Viewer::single(self.dir.join(&entry.name)))),
                Some(entry) if entry.bytes > EDITABLE => *cx.say = Some(format!("{} is too large to edit · h shows its bytes", size(entry.bytes))),
                Some(entry) => match Editor::open(&self.dir.join(&entry.name)) {
                    Ok(editor) => return Nav::Push(Box::new(editor)),
                    Err(why) => *cx.say = Some(why),
                },
                None => {}
            },
            Key::Char('n') => self.naming = Some(String::new()),
            // Any file as bytes, however large (only what is on screen is read).
            // (Not x: outside typing, bare X is the Cardputer's down arrow.)
            Key::Char('h') => match focused {
                Some(entry) if entry.file => match crate::hex::HexView::open(&self.dir.join(&entry.name)) {
                    Ok(view) => return Nav::Push(Box::new(view)),
                    Err(why) => *cx.say = Some(why),
                },
                Some(_) => *cx.say = Some("only a file opens as hex".into()),
                None => {}
            },
            Key::Char('d') => {
                if let Some(entry) = focused {
                    if armed.as_deref() == Some(entry.name.as_str()) {
                        let path = self.dir.join(&entry.name);
                        // A folder only goes if it is empty: nothing is removed wholesale.
                        let gone = if entry.dir { fs::remove_dir(&path) } else { fs::remove_file(&path) };
                        *cx.say = Some(match gone {
                            Ok(()) => format!("deleted {}", entry.name),
                            Err(_) if entry.dir => "only an empty folder can be deleted".into(),
                            Err(_) => format!("cannot delete {}", entry.name),
                        });
                        let (dir, at) = (self.dir.clone(), self.list.focus);
                        self.enter(dir, None);
                        self.list.focus = at.min(self.entries.len().saturating_sub(1));
                    } else {
                        *cx.say = Some(format!("d again deletes {}", entry.name));
                        self.deleting = Some(entry.name);
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.entries.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h) = (g.top(), g.row_h());
        if let Some(name) = &self.naming {
            let field = Rect::new(PAD, top, W - 2.0 * PAD, ty.caption.size + ty.label.size + 20.0);
            g.surface(field, 14.0, t.surface2);
            g.text_at(field.x + 10.0, field.y + 7.0 + ty.caption.size * 0.5, Align::Left, "name for the new file", ty.caption, t.dim);
            let cy = field.bottom() - 8.0 - ty.label.size * 0.5;
            let w = g.text_at(field.x + 10.0, cy, Align::Left, name, ty.label, t.ink);
            g.surface(Rect::new(field.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
            g.focus_ring(field, 14.0);
            g.hint("enter: make it · esc: cancel");
            return;
        }
        let bottom = H - PAD - g.hint_h();
        if self.entries.is_empty() {
            g.empty("folder", "nothing in this folder");
        }
        let entries = self.entries.clone();
        for (i, y) in self.list.layout(g, entries.len(), top, row_h, 3.0, bottom + 2.0) {
            let entry = &entries[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = if entry.dir { (format!("{}/", entry.name), "›".to_owned()) } else { (entry.name.clone(), size(entry.bytes)) };
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, entries.len(), top, bottom);
        g.hint("n: new file · d: delete · h: as hex");
    }
}

// ---------------------------------------------------------------- editor ---

/// A text file being edited: its lines, and where the cursor is in them.
/// The file comes back out as it went in, apart from what was changed: its
/// tabs stay tabs, its line endings stay what they were, and a file that did
/// not end in a newline does not gain one.
pub struct Buffer {
    pub lines: Vec<String>,
    /// Line, and column counted in characters.
    pub cursor: (usize, usize),
    pub modified: bool,
    /// Windows line endings.
    crlf: bool,
    /// Whether the last line ends in a newline.
    final_newline: bool,
    /// Whether the file indents with tabs, so the Tab key types one.
    pub tabs: bool,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        // `lines` takes "\r\n" as one line end, as it should.
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let tabs = text.lines().any(|l| l.starts_with('\t'));
        Buffer { lines, cursor: (0, 0), modified: false, crlf: text.contains("\r\n"), final_newline: text.is_empty() || text.ends_with('\n'), tabs }
    }

    pub fn text(&self) -> String {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        let mut text = self.lines.join(eol);
        if self.final_newline {
            text.push_str(eol);
        }
        text
    }

    fn line_len(&self, line: usize) -> usize {
        self.lines[line].chars().count()
    }

    /// The byte offset of a character column in a line.
    fn byte_at(&self, line: usize, col: usize) -> usize {
        self.lines[line].char_indices().nth(col).map_or(self.lines[line].len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let (line, col) = self.cursor;
        let at = self.byte_at(line, col);
        self.lines[line].insert(at, c);
        self.cursor.1 += 1;
        self.modified = true;
    }

    /// Enter: split the line at the cursor, keeping the indent.
    pub fn new_line(&mut self) {
        let (line, col) = self.cursor;
        let at = self.byte_at(line, col);
        let rest = self.lines[line].split_off(at);
        let indent: String = self.lines[line].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let indent_len = indent.len().min(col);
        self.lines.insert(line + 1, format!("{}{rest}", &indent[..indent_len]));
        self.cursor = (line + 1, indent_len);
        self.modified = true;
    }

    /// Backspace: the character before the cursor, or join with the line above.
    pub fn delete_back(&mut self) {
        let (line, col) = self.cursor;
        if col > 0 {
            let at = self.byte_at(line, col - 1);
            self.lines[line].remove(at);
            self.cursor.1 -= 1;
        } else if line > 0 {
            let tail = self.lines.remove(line);
            self.cursor = (line - 1, self.line_len(line - 1));
            self.lines[line - 1].push_str(&tail);
        } else {
            return;
        }
        self.modified = true;
    }

    /// Delete: the character after the cursor, or join the next line on.
    pub fn delete_forward(&mut self) {
        let before = self.cursor;
        self.step(0, 1);
        if self.cursor != before {
            self.delete_back();
        }
    }

    /// Remove the whole line the cursor is on.
    pub fn delete_line(&mut self) {
        let line = self.cursor.0;
        if self.lines.len() > 1 {
            self.lines.remove(line);
        } else {
            self.lines[0].clear();
        }
        self.cursor = (line.min(self.lines.len() - 1), 0);
        self.modified = true;
    }

    /// Move by lines and columns; off the end of a line wraps to the next.
    pub fn step(&mut self, lines: isize, cols: isize) {
        let (mut line, mut col) = self.cursor;
        if cols < 0 && col == 0 && line > 0 {
            line -= 1;
            col = self.line_len(line);
        } else if cols > 0 && col >= self.line_len(line) && line + 1 < self.lines.len() {
            line += 1;
            col = 0;
        } else {
            col = (col as isize + cols).max(0) as usize;
        }
        line = (line as isize + lines).clamp(0, self.lines.len() as isize - 1) as usize;
        self.cursor = (line, col.min(self.line_len(line)));
    }
}

pub struct Editor {
    path: PathBuf,
    title: String,
    buffer: Buffer,
    /// The first line and column in view.
    view: (usize, usize),
    /// Ctrl-Q was pressed on unsaved changes; again discards them.
    leaving: bool,
    rows: usize,
}

impl Editor {
    pub fn open(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let title = path.file_name().map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
        // Through a link to the file itself, so saving changes it and leaves the link.
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        match fs::metadata(&path) {
            Ok(meta) if !meta.is_file() => return Err("not a file the editor can open".into()),
            Ok(meta) if meta.len() > EDITABLE => return Err(format!("{} is too large to open", size(meta.len()))),
            Ok(_) => {}
            Err(_) => return Err("cannot read that file".into()),
        }
        // Read no more than the editor holds, whatever the file says its size is.
        let mut bytes = Vec::new();
        fs::File::open(&path).and_then(|f| f.take(EDITABLE + 1).read_to_end(&mut bytes)).map_err(|_| "cannot read that file".to_string())?;
        if bytes.len() as u64 > EDITABLE {
            return Err("too large to open".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "not a text file".to_string())?;
        Ok(Editor { path, title, buffer: Buffer::from_text(&text), view: (0, 0), leaving: false, rows: 8 })
    }

    /// Write the file by way of a temporary one, so a power cut mid-save
    /// leaves the old contents rather than half of the new. The file keeps
    /// its permissions: a script stays runnable.
    fn save(&mut self) -> bool {
        let ok = crate::sys::write_atomic(&self.path, self.buffer.text().as_bytes()).is_ok();
        if ok {
            self.buffer.modified = false;
        }
        ok
    }
}

impl Screen for Editor {
    fn title(&self) -> &str {
        &self.title
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("ctrl-s saves · ctrl-q closes")
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let leaving = std::mem::take(&mut self.leaving);
        let b = &mut self.buffer;
        match key {
            Key::Ctrl('q') => {
                if !b.modified || leaving {
                    return Nav::Pop;
                }
                self.leaving = true;
                *cx.say = Some("unsaved · ctrl-q again discards".into());
            }
            Key::Ctrl('s') if !b.modified => *cx.say = Some("no changes to save".into()),
            Key::Ctrl('s') => *cx.say = Some(if self.save() { "saved".into() } else { "could not save".into() }),
            Key::Ctrl('k') => b.delete_line(),
            Key::Ctrl('a') => b.cursor.1 = 0,
            Key::Ctrl('e') => b.cursor.1 = b.line_len(b.cursor.0),
            Key::Char(c) if !c.is_control() => b.insert(c),
            Key::Char(_) => {}
            Key::Tab if b.tabs => b.insert('\t'),
            Key::Tab => (0..4).for_each(|_| b.insert(' ')),
            Key::Enter => b.new_line(),
            Key::Backspace => b.delete_back(),
            Key::Up => b.step(-1, 0),
            Key::Down => b.step(1, 0),
            Key::Left => b.step(0, -1),
            Key::Right => b.step(0, 1),
            Key::PageUp => b.step(-(self.rows as isize - 1), 0),
            Key::PageDown => b.step(self.rows as isize - 1, 0),
            Key::Home => b.cursor.1 = 0,
            Key::End => b.cursor.1 = b.line_len(b.cursor.0),
            Key::Delete => b.delete_forward(),
            // Esc does nothing here, so a slip does not close an unsaved file.
            // Neither does holding it: the way out is Ctrl-Q, which asks first.
            Key::Back | Key::Leave | Key::Ctrl(_) | Key::F(_) | Key::BrightnessDown | Key::BrightnessUp => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let style = g.ty.caption;
        let cell_w = g.measure("0", style);
        let cell_h = style.size + 2.0;
        let panel = Rect::new(4.0, g.top() - 3.0, W - 8.0, H - g.top() - g.hint_h() - 1.0);
        g.surface(panel, 10.0, t.surface1);
        let cols = ((panel.w - 8.0) / cell_w).floor().max(2.0) as usize;
        self.rows = ((panel.h - 6.0) / cell_h).floor().max(1.0) as usize;
        // Keep the cursor in view, both ways.
        let (line, col) = self.buffer.cursor;
        let (mut first, mut left) = self.view;
        first = first.clamp((line + 1).saturating_sub(self.rows), line);
        left = left.clamp((col + 1).saturating_sub(cols), col);
        self.view = (first, left);
        let (x0, y0) = (panel.x + 4.0, panel.y + 3.0);
        for row in 0..self.rows {
            let Some(text) = self.buffer.lines.get(first + row) else { break };
            let cy = y0 + (row as f32 + 0.5) * cell_h;
            if first + row == line {
                // The cursor: a block in the signal colour, behind its character.
                g.surface(Rect::new(x0 + (col - left) as f32 * cell_w, cy - cell_h * 0.5, cell_w + 0.5, cell_h), 2.0, t.signal);
            }
            // A tab takes one cell, shown as an arrow, so it can be seen and
            // the cursor counts it as the one character it is.
            let shown: String = text.chars().skip(left).take(cols).map(|c| if c == '\t' { '→' } else { c }).collect();
            g.text_at(x0, cy, Align::Left, &shown, style, t.ink);
            if first + row == line {
                if let Some(under) = text.chars().nth(col).map(|c| if c == '\t' { '→' } else { c }) {
                    let mut utf8 = [0u8; 4];
                    g.text_at(x0 + (col - left) as f32 * cell_w, cy, Align::Left, under.encode_utf8(&mut utf8), style, t.on_signal);
                }
            }
        }
        let status = format!("line {} of {} · col {}{}", line + 1, self.buffer.lines.len(), col + 1, if self.buffer.modified { " · unsaved" } else { "" });
        g.hint(&status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_splitting_and_joining_lines() {
        let mut b = Buffer::from_text("fn main() {\n}\n");
        b.step(0, 11);
        b.new_line();
        "    go();".chars().for_each(|c| b.insert(c));
        assert_eq!(b.text(), "fn main() {\n    go();\n}\n");
        // Enter keeps the indent.
        b.new_line();
        assert_eq!(b.cursor, (2, 4));
        assert_eq!(b.lines[2], "    ");
        // Backspace at the start of a line joins it to the one above.
        (0..4).for_each(|_| b.delete_back());
        b.delete_back();
        assert_eq!(b.text(), "fn main() {\n    go();\n}\n");
        assert_eq!(b.cursor, (1, 9));
        assert!(b.modified);
    }

    #[test]
    fn the_cursor_wraps_between_lines_and_stays_inside_them() {
        let mut b = Buffer::from_text("long line here\nab\n\nz");
        b.step(0, 14);
        b.step(0, 1);
        assert_eq!(b.cursor, (1, 0), "right at the end of a line goes to the next");
        b.step(0, -1);
        assert_eq!(b.cursor, (0, 14), "left at the start goes to the end of the one above");
        b.step(1, 0);
        assert_eq!(b.cursor, (1, 2), "a shorter line pulls the column in");
        b.step(10, 0);
        assert_eq!(b.cursor.0, 3);
        b.step(-10, 0);
        assert_eq!(b.cursor.0, 0);
    }

    #[test]
    fn characters_beyond_ascii_are_whole_characters() {
        let mut b = Buffer::from_text("50 °C");
        b.step(0, 4);
        b.insert('~');
        assert_eq!(b.lines[0], "50 °~C");
        b.delete_back();
        b.delete_back();
        assert_eq!(b.lines[0], "50 C");
    }

    #[test]
    fn a_file_comes_back_out_as_it_went_in() {
        // Tabs, Windows line endings, no newline at the end: all kept.
        for text in ["all:\n\tcc -o app app.c\n", "one\r\ntwo\r\n", "no newline at the end", "", "a\n\n"] {
            let b = Buffer::from_text(text);
            assert_eq!(b.text(), if text.is_empty() { "\n" } else { text }, "{text:?}");
        }
        // In a file indented with tabs, Tab types one; Enter keeps the tab indent.
        let mut b = Buffer::from_text("all:\n\tcc\n");
        assert!(b.tabs);
        b.step(1, 0);
        b.cursor.1 = 3;
        b.new_line();
        assert_eq!(b.lines[2], "\t");
        // Windows endings stay Windows endings after an edit.
        let mut b = Buffer::from_text("one\r\ntwo\r\n");
        b.insert('!');
        assert_eq!(b.text(), "!one\r\ntwo\r\n");
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_a_script_runnable_and_a_link_a_link() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("catalyst-save-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("check.sh");
        fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&script, dir.join("link.sh")).unwrap();
        let mut editor = Editor::open(&dir.join("link.sh")).unwrap();
        editor.buffer.insert('#');
        assert!(editor.save());
        assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(fs::symlink_metadata(dir.join("link.sh")).unwrap().file_type().is_symlink());
        assert!(fs::read_to_string(&script).unwrap().starts_with("##!/bin/sh"));
        // A device is listed but not opened: reading it would never end.
        let entries = list(Path::new("/dev"));
        let zero = entries.iter().find(|e| e.name == "zero").expect("/dev/zero");
        assert!(!zero.file && !zero.dir);
        assert!(Editor::open(Path::new("/dev/zero")).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_is_opened_changed_saved_and_found_by_the_browser() {
        let dir = std::env::temp_dir().join(format!("catalyst-files-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("logs")).unwrap();
        fs::write(dir.join("notes.txt"), "first\n").unwrap();
        fs::write(dir.join("Alpha.txt"), vec![0u8; 3000]).unwrap();
        // Folders first, then files by name whatever their case.
        let names: Vec<String> = list(&dir).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["logs", "Alpha.txt", "notes.txt"]);
        assert_eq!(size(3000), "3 kb");

        let mut editor = Editor::open(&dir.join("notes.txt")).unwrap();
        editor.buffer.step(1, 0);
        editor.buffer.step(0, 5);
        editor.buffer.new_line();
        "second".chars().for_each(|c| editor.buffer.insert(c));
        assert!(editor.save());
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "first\nsecond\n");
        assert!(!editor.buffer.modified);
        // Nothing is left behind by the save.
        assert_eq!(list(&dir).len(), 3);
        // Something that is not text is refused, not shown as rubbish.
        fs::write(dir.join("blob.bin"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(Editor::open(&dir.join("blob.bin")).is_err());
        fs::remove_dir_all(&dir).ok();
    }
}
