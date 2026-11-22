//! The clipboard: one for the whole handheld, shared by every screen, the
//! Terminal's programs and shell scripts.
//!
//! - **Copy** is Ctrl-C and **paste** Ctrl-V, in every text field, the
//!   editor, Spotlight and the Assistant. In a terminal, Ctrl-C stays the
//!   interrupt and Ctrl-V the next key as it is: there they are Ctrl-Shift-C
//!   and Ctrl-Shift-V, which work everywhere else too. Ctrl-Shift and a
//!   letter is the shell's: a program in a terminal is sent the same byte
//!   for it as for Ctrl and the letter, so taking it takes nothing a program
//!   could ask for (as Ctrl-Tab and Ctrl-Enter, for the same reason).
//! - **What is copied** where nothing is chosen (there is no selection: the
//!   fields here are one line, mostly with no cursor to move): the whole
//!   field being typed in; in the editor, the line the cursor is on; on a
//!   list, the value of the row in focus (a table's value, an address, a
//!   version); in the Terminal, the lines picked in its copy mode.
//! - **History**: the last twenty clips (no more than 256 kb of them: a big
//!   one makes room by letting the oldest go), and up to twenty pinned,
//!   newest first. Kept across restarts in `/data/catalyst/clipboard.json`,
//!   root's alone, written whole through a file beside it on a thread of its
//!   own (several changes at once are one write). Ctrl-Shift-H brings them
//!   up from anywhere, growing out of the island (`panel.rs`).
//! - **Secrets** (`catalyst-clip set --secret`, or an app copying a
//!   password) are never written anywhere, never join the history, are never
//!   handed to a script, and clear themselves 30 seconds after they were
//!   copied, or as soon as the handheld locks.
//! - **The lock**: nothing of the clipboard shows on the lock screen. Its
//!   sheets go and the island stops saying what was copied the moment it
//!   locks, and no key on the lock screen reaches them.
//! - **Scripts and terminal programs**: `catalyst-clip` (get, set, history,
//!   clear; `serve.rs`) and OSC 52 (a program printing `ESC ] 52 ; c ;
//!   <base64> BEL` sets it: tmux, vim and neovim can). Reading it back by
//!   OSC 52 is refused, as xterm does by default: a program printed to the
//!   screen should not learn what was copied elsewhere.
//!
//! **For other apps**: [`copy`] with [`Sensitivity::Normal`] or
//! [`Sensitivity::Secret`] (the island says it was copied, and for a secret
//! that it clears in 30 s); [`copy_as`] names what was copied ("password
//! copied"); [`paste_text`] is what a paste would put in.

pub mod panel;
pub mod serve;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub use panel::Panel;

use crate::platform::Key;

/// How long a secret stays on the clipboard.
pub const SECRET_FOR: Duration = Duration::from_secs(30);
/// Clips kept besides the pinned ones, and pinned ones at most.
pub const RECENT: usize = 20;
pub const PINS: usize = 20;
/// The most the clips besides the pinned ones hold together, in memory and
/// on the card: past it the oldest go. Four of the largest clips fit, so
/// the newest always stays.
pub const RECENT_BYTES: usize = 4 * MOST_BYTES;
/// The most a clip holds: a larger copy keeps its first 64 kb.
pub const MOST_BYTES: usize = 64 * 1024;
/// The most a paste types into a one-line field.
const MOST_TYPED: usize = 4096;

/// What is copied, as the clipboard should treat it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    /// Kept in the history, and on the card across restarts.
    Normal,
    /// A password or a key: never written anywhere, never in the history,
    /// never given to a script; gone 30 s after it was copied, or at the lock.
    Secret,
}

/// A clip in the history.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub text: String,
    pub pinned: bool,
    /// When it was copied (Unix seconds).
    pub at: i64,
}

/// Why a clip was not pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpinnable {
    /// It is no longer in the history.
    Gone,
    /// Twenty are pinned already.
    Full,
}

/// What a paste puts in just now.
#[derive(Clone, Debug)]
struct Current {
    text: String,
    secret: bool,
    /// A secret's end.
    until: Option<Instant>,
}

/// The clipboard: what a paste puts in, and the history.
pub struct Board {
    current: Option<Current>,
    /// Newest first.
    items: Vec<Item>,
    /// What the island says next (the first that fits), taken by the shell.
    words: Option<Vec<String>>,
    /// Counts every change, for the panel to notice one made elsewhere.
    pub generation: u64,
    /// Where the history is kept (none: in memory alone, as in the tests).
    path: Option<PathBuf>,
}

impl Board {
    pub fn memory() -> Board {
        Board { current: None, items: Vec::new(), words: None, generation: 0, path: None }
    }

    /// The history kept at `path`, and the clip last copied if it was not a
    /// secret (a secret never reaches the file).
    pub fn open(path: PathBuf) -> Board {
        let mut board = Board::memory();
        if let Some(v) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            board.items = v["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|i| Some(Item { text: clamp(i["text"].as_str()?), pinned: i["pinned"].as_bool().unwrap_or(false), at: i["at"].as_i64().unwrap_or(0) }))
                        .collect()
                })
                .unwrap_or_default();
            // The current clip is kept as its place in the history, which always holds it.
            let current = v["current"].as_u64().and_then(|at| board.items.get(at as usize)).map(|i| i.text.clone());
            board.items.retain(|i| !i.text.trim().is_empty());
            board.current = current.filter(|t| !t.trim().is_empty()).map(|text| Current { text, secret: false, until: None });
            board.trim();
            if board.current.as_ref().is_some_and(|c| !board.items.iter().any(|i| i.text == c.text)) {
                board.current = None;
            }
        }
        board.path = Some(path);
        board
    }

    /// The history and the current clip as the file keeps them: never a
    /// secret. The current clip is its place in the history, not a copy of it.
    pub fn file_text(&self) -> String {
        let current = self.current.as_ref().filter(|c| !c.secret).and_then(|c| self.items.iter().position(|i| i.text == c.text));
        let items: Vec<Value> = self.items.iter().map(|i| json!({ "text": i.text, "pinned": i.pinned, "at": i.at })).collect();
        serde_json::to_string(&json!({ "current": current, "items": items })).unwrap_or_default()
    }

    fn changed(&mut self, keep: bool) {
        self.generation += 1;
        if keep && self.path.is_some() {
            save_soon();
        }
    }

    /// Copy `text`. A normal clip goes to the top of the history (a copy
    /// of one already there moves it up, pinned or not); a secret only
    /// takes the current clip, until `now` + 30 s.
    pub fn copy(&mut self, text: &str, sensitivity: Sensitivity, now: Instant, unix: i64) -> bool {
        let text = clamp(text);
        if text.trim().is_empty() {
            return false;
        }
        let secret = sensitivity == Sensitivity::Secret;
        // A secret it replaces is wiped first.
        if self.current.as_ref().is_some_and(|c| c.secret) {
            self.forget_secret();
        }
        if secret {
            self.current = Some(Current { text, secret: true, until: Some(now + SECRET_FOR) });
            self.changed(false);
            return true;
        }
        let pinned = self.items.iter().position(|i| i.text == text).map(|at| self.items.remove(at).pinned).unwrap_or(false);
        self.items.insert(0, Item { text: text.clone(), pinned, at: unix });
        self.current = Some(Current { text, secret: false, until: None });
        self.trim();
        self.changed(true);
        true
    }

    /// Keep twenty clips besides the pinned, no more than [`RECENT_BYTES`]
    /// of them, and twenty pinned at most.
    fn trim(&mut self) {
        let (mut recent, mut bytes, mut pins) = (0, 0, 0);
        self.items.retain(|i| {
            if i.pinned {
                pins += 1;
                return pins <= PINS;
            }
            recent += 1;
            bytes += i.text.len();
            recent <= RECENT && bytes <= RECENT_BYTES
        });
    }

    /// What a paste puts in at `now`: a secret past its time is gone first.
    pub fn paste(&mut self, now: Instant) -> Option<String> {
        self.expire(now);
        self.current.as_ref().map(|c| c.text.clone())
    }

    /// A secret past its time, let go: its words wiped from memory. True if
    /// one went.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.current.as_ref().is_some_and(|c| c.until.is_some_and(|t| now >= t)) {
            self.forget_secret();
            return true;
        }
        false
    }

    /// The secret on the clipboard, gone now (the handheld locking, or its time up).
    pub fn forget_secret(&mut self) -> bool {
        if !self.current.as_ref().is_some_and(|c| c.secret) {
            return false;
        }
        if let Some(mut c) = self.current.take() {
            wipe(&mut c.text);
        }
        self.changed(false);
        true
    }

    /// How long the secret on the clipboard has left, if one is there.
    pub fn secret_left(&self, now: Instant) -> Option<Duration> {
        self.current.as_ref().filter(|c| c.secret).and_then(|c| c.until).map(|t| t.saturating_duration_since(now))
    }

    /// The current clip, if it is not a secret: what a script may read.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub fn plain(&self) -> Option<String> {
        self.current.as_ref().filter(|c| !c.secret).map(|c| c.text.clone())
    }

    /// The history as the panel lists it: pinned first, then the newest.
    pub fn items(&self) -> Vec<Item> {
        let mut items = self.items.clone();
        items.sort_by_key(|i| !i.pinned);
        items
    }

    /// The clip `text` is the current one.
    pub fn is_current(&self, text: &str) -> bool {
        self.current.as_ref().is_some_and(|c| !c.secret && c.text == text)
    }

    /// Pin a clip, or unpin it: its new state. Not when it is not there, nor
    /// a twenty-first pin (one is unpinned first: none is dropped unasked).
    pub fn toggle_pin(&mut self, text: &str) -> Result<bool, Unpinnable> {
        let pins = self.items.iter().filter(|i| i.pinned).count();
        let item = self.items.iter_mut().find(|i| i.text == text).ok_or(Unpinnable::Gone)?;
        if !item.pinned && pins >= PINS {
            return Err(Unpinnable::Full);
        }
        item.pinned = !item.pinned;
        let now = item.pinned;
        self.trim();
        self.changed(true);
        Ok(now)
    }

    /// Take a clip out of the history; the clipboard empties if it was the
    /// current one.
    pub fn remove(&mut self, text: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.text != text);
        let was_current = self.is_current(text);
        if was_current {
            self.current = None;
        }
        let gone = before != self.items.len() || was_current;
        if gone {
            self.changed(true);
        }
        gone
    }

    /// Everything gone but what is pinned; the clipboard emptied.
    pub fn clear(&mut self) {
        self.items.retain(|i| i.pinned);
        if let Some(mut c) = self.current.take() {
            wipe(&mut c.text);
        }
        self.changed(true);
    }
}

/// A clip no larger than the clipboard holds, cut at a character.
pub(crate) fn clamp(text: &str) -> String {
    if text.len() <= MOST_BYTES {
        return text.to_owned();
    }
    let mut end = MOST_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// A secret's bytes overwritten before its memory is let go.
fn wipe(text: &mut String) {
    // SAFETY: zero bytes are valid UTF-8, and the length is unchanged.
    unsafe {
        for b in text.as_bytes_mut() {
            std::ptr::write_volatile(b, 0);
        }
    }
    text.clear();
}

// ------------------------------------------------------------- the board ---

/// Where the history is kept: beside the settings, the notifications and
/// the reminders (`/data/catalyst`).
pub fn path() -> PathBuf {
    crate::config_dir().join("clipboard.json")
}

// One board for the shell, the Terminal's programs and the socket that
// `catalyst-clip` talks to. The tests run many shells at once, one to a
// thread: each has its own, in memory.
#[cfg(not(test))]
static BOARD: std::sync::Mutex<Option<Board>> = std::sync::Mutex::new(None);

#[cfg(not(test))]
pub fn with<T>(f: impl FnOnce(&mut Board) -> T) -> T {
    let mut board = BOARD.lock().unwrap_or_else(|e| e.into_inner());
    f(board.get_or_insert_with(|| Board::open(path())))
}

#[cfg(test)]
thread_local! {
    static BOARD: std::cell::RefCell<Option<Board>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn with<T>(f: impl FnOnce(&mut Board) -> T) -> T {
    BOARD.with(|b| f(b.borrow_mut().get_or_insert_with(Board::memory)))
}

/// Write the history soon, on a thread of its own: the card's flush never
/// holds up a frame. Each write takes the board as it is then, one at a
/// time, so the last change is always the one left on the card; changes
/// made while a write waits its turn are in that write, not one each (a
/// script setting the clipboard in a loop writes the card a few times, not
/// once a line).
fn save_soon() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static WAITING: AtomicBool = AtomicBool::new(false);
    if WAITING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("clipboard-save".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let _one_at_a_time = SAVING.lock().unwrap_or_else(|e| e.into_inner());
            // From here a change needs a write of its own: this one may have read the board already.
            WAITING.store(false, Ordering::Release);
            let (path, text) = with(|b| (b.path.clone(), b.file_text()));
            if let Some(path) = path {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).ok();
                }
                crate::sys::write_private(&path, text.as_bytes()).ok();
            }
        });
    if spawned.is_err() {
        WAITING.store(false, Ordering::Release);
    }
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ------------------------------------------------------------ for apps ---

/// Copy `text`. The island says it was copied (a secret, that it clears in
/// 30 s). For a password: `copy(password, Sensitivity::Secret)`.
pub fn copy(text: &str, sensitivity: Sensitivity) {
    let words = copied_words(text, sensitivity, None);
    put(text, sensitivity, Some(words));
}

/// The same, the island naming what was copied: "password copied".
#[allow(dead_code)]
pub fn copy_as(text: &str, sensitivity: Sensitivity, what: &str) {
    let words = copied_words(text, sensitivity, Some(what));
    put(text, sensitivity, Some(words));
}

/// The same, with nothing said (the screen says its own words).
pub fn copy_quietly(text: &str, sensitivity: Sensitivity) {
    put(text, sensitivity, None);
}

fn put(text: &str, sensitivity: Sensitivity, words: Option<Vec<String>>) {
    let now = Instant::now();
    with(|b| {
        if b.copy(text, sensitivity, now, unix_now()) {
            b.words = words;
        }
    });
}

/// What a paste would put in just now (a secret too, until it clears).
pub fn paste_text() -> Option<String> {
    with(|b| b.paste(Instant::now()))
}

/// What the island should say about the clipboard, if anything (the
/// longest of the ways to say it that fits): taken once.
pub fn take_words() -> Option<Vec<String>> {
    with(|b| b.words.take())
}

/// Every look in (a few times a second): a secret past its time goes, and
/// the island says the clipboard was cleared.
pub fn tick() {
    with(|b| {
        if b.expire(Instant::now()) {
            b.words = Some(vec!["clipboard cleared".into()]);
        }
    });
}

/// The handheld locked: a secret on the clipboard goes at once.
pub fn lock() {
    with(|b| {
        b.forget_secret();
    });
}

/// The board's generation, to tell when it changed.
pub fn generation() -> u64 {
    with(|b| b.generation)
}

/// How the island says a copy: what was copied, if it is short enough to
/// read in passing, else how much; a secret, that it clears in 30 s.
pub fn copied_words(text: &str, sensitivity: Sensitivity, what: Option<&str>) -> Vec<String> {
    let copied = what.map_or_else(|| "copied".to_owned(), |w| format!("{w} copied"));
    if sensitivity == Sensitivity::Secret {
        return vec![format!("{copied} · clears in {} s", SECRET_FOR.as_secs()), copied];
    }
    let mut words = Vec::new();
    let one: String = text.trim().chars().filter(|c| !c.is_control()).collect();
    let lines = text.trim_end().lines().count();
    if what.is_none() && lines <= 1 && !one.is_empty() && one.chars().count() <= 28 {
        words.push(format!("copied “{one}”"));
    }
    if lines > 1 {
        words.push(format!("{copied} · {lines} lines"));
    } else if what.is_none() {
        let n = text.chars().count();
        if n > 28 {
            words.push(format!("copied · {n} characters"));
        }
    }
    words.push(copied);
    words
}

/// Text to type into a one-line field: line breaks and tabs as spaces,
/// other control characters left out, no longer than a field could want.
pub fn one_line(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.trim().chars() {
        if c == '\n' || c == '\r' || c == '\t' {
            if !space {
                out.push(' ');
            }
            space = true;
            continue;
        }
        if c.is_control() {
            continue;
        }
        space = c == ' ';
        out.push(c);
        if out.len() >= MOST_TYPED {
            break;
        }
    }
    out
}

// ----------------------------------------------------------------- keys ---

/// What a key is to the clipboard and to quick capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Copy,
    Paste,
    /// The clipboard's history (Ctrl-Shift-H), from anywhere.
    History,
    /// Quick capture (Ctrl-Shift-N), from anywhere.
    Capture,
}

/// The history's key and quick capture's: Ctrl-Shift-H and Ctrl-Shift-N.
pub const HISTORY_KEY: Key = Key::Ctrl('H');
pub const CAPTURE_KEY: Key = Key::Ctrl('N');

/// What `key` asks of the clipboard over a screen; `keeps_ctrl`: the screen
/// is a terminal, whose programs keep Ctrl-C and Ctrl-V (it takes Ctrl-Shift-C
/// and V itself, as keys).
pub fn hotkey(key: Key, keeps_ctrl: bool) -> Option<Hotkey> {
    match key {
        HISTORY_KEY => Some(Hotkey::History),
        CAPTURE_KEY => Some(Hotkey::Capture),
        _ if keeps_ctrl => None,
        Key::Ctrl('c' | 'C') => Some(Hotkey::Copy),
        Key::Ctrl('v' | 'V') => Some(Hotkey::Paste),
        _ => None,
    }
}

/// What a value on a row copies: the value, unless it is only a mark
/// ("›", "3 ›") or was cut short to fit; then the row's name.
pub fn row_copy(name: &str, value: &str) -> Option<String> {
    let value = value.trim();
    let cut = value.ends_with('…');
    // "3 ›" (a table's count), "›", "–": marks, not values.
    let mark = value.ends_with('›') || !value.chars().any(char::is_alphanumeric);
    let name = name.trim().trim_end_matches('›').trim();
    if !value.is_empty() && !cut && !mark {
        Some(value.to_owned())
    } else if !name.is_empty() && !name.ends_with('…') {
        Some(name.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> (Board, Instant) {
        (Board::memory(), Instant::now())
    }

    #[test]
    fn a_copy_goes_to_the_top_and_a_copy_again_moves_it_up() {
        let (mut b, now) = board();
        b.copy("10.58.5.2", Sensitivity::Normal, now, 1);
        b.copy("swap battery", Sensitivity::Normal, now, 2);
        b.copy("10.58.5.2", Sensitivity::Normal, now, 3);
        let texts: Vec<String> = b.items().into_iter().map(|i| i.text).collect();
        assert_eq!(texts, ["10.58.5.2", "swap battery"], "no copy twice");
        assert_eq!(b.paste(now).as_deref(), Some("10.58.5.2"));
        assert!(!b.copy("   ", Sensitivity::Normal, now, 4), "nothing to copy");
        assert_eq!(b.items().len(), 2);
    }

    #[test]
    fn twenty_are_kept_and_pins_besides_them_first() {
        let (mut b, now) = board();
        for i in 0..30 {
            b.copy(&format!("clip {i}"), Sensitivity::Normal, now, i);
        }
        assert_eq!(b.items().len(), RECENT);
        assert_eq!(b.items()[0].text, "clip 29");
        assert_eq!(b.toggle_pin("clip 15"), Ok(true));
        for i in 30..60 {
            b.copy(&format!("clip {i}"), Sensitivity::Normal, now, i);
        }
        let items = b.items();
        assert_eq!(items[0].text, "clip 15", "the pinned one stays, first");
        assert!(items[0].pinned);
        assert_eq!(items.len(), RECENT + 1);
        assert_eq!(items[1].text, "clip 59");
        // A pinned clip copied again stays pinned.
        b.copy("clip 15", Sensitivity::Normal, now, 61);
        assert!(b.items()[0].pinned);
        assert_eq!(b.toggle_pin("clip 15"), Ok(false));
        assert_eq!(b.toggle_pin("not there"), Err(Unpinnable::Gone));
    }

    #[test]
    fn a_twenty_first_pin_is_refused_rather_than_one_dropped() {
        let (mut b, now) = board();
        for i in 0..PINS + 1 {
            b.copy(&format!("pin {i}"), Sensitivity::Normal, now, i as i64);
            if i < PINS {
                assert_eq!(b.toggle_pin(&format!("pin {i}")), Ok(true));
            }
        }
        assert_eq!(b.toggle_pin(&format!("pin {PINS}")), Err(Unpinnable::Full));
        assert_eq!(b.items().iter().filter(|i| i.pinned).count(), PINS, "every pin kept");
        assert_eq!(b.toggle_pin("pin 0"), Ok(false));
        assert_eq!(b.toggle_pin(&format!("pin {PINS}")), Ok(true), "room once one is unpinned");
    }

    #[test]
    fn big_clips_make_room_by_letting_the_oldest_go_but_the_newest_stays() {
        let (mut b, now) = board();
        b.copy("small and old", Sensitivity::Normal, now, 1);
        b.toggle_pin("small and old").unwrap();
        let big = |c: char| c.to_string().repeat(MOST_BYTES);
        for (i, c) in ['a', 'b', 'c', 'd', 'e'].into_iter().enumerate() {
            b.copy(&big(c), Sensitivity::Normal, now, 2 + i as i64);
        }
        let kept: usize = b.items().iter().filter(|i| !i.pinned).map(|i| i.text.len()).sum();
        assert!(kept <= RECENT_BYTES, "{kept} bytes kept");
        assert_eq!(b.items().iter().filter(|i| !i.pinned).count(), RECENT_BYTES / MOST_BYTES);
        assert!(b.items()[1].text.starts_with('e'), "newest first, after the pin");
        assert!(b.items().iter().any(|i| i.pinned && i.text == "small and old"), "a pin is never let go for room");
    }

    #[test]
    fn a_secret_is_never_in_the_history_nor_the_file_and_clears_after_30_s() {
        let (mut b, now) = board();
        b.copy("bumpers", Sensitivity::Normal, now, 1);
        b.copy("hunter2", Sensitivity::Secret, now, 2);
        assert_eq!(b.paste(now).as_deref(), Some("hunter2"), "it pastes");
        assert!(b.items().iter().all(|i| i.text != "hunter2"), "not in the history");
        assert!(!b.file_text().contains("hunter2"), "never written");
        assert!(b.plain().is_none(), "never given to a script");
        assert_eq!(b.secret_left(now + Duration::from_secs(10)), Some(Duration::from_secs(20)));
        assert_eq!(b.paste(now + Duration::from_secs(29)).as_deref(), Some("hunter2"));
        assert_eq!(b.paste(now + Duration::from_secs(30)), None, "gone at 30 s");
        assert_eq!(b.items()[0].text, "bumpers", "the history is as it was");
        // And at the lock, at once.
        b.copy("hunter3", Sensitivity::Secret, now, 3);
        assert!(b.forget_secret());
        assert_eq!(b.paste(now), None);
        assert!(!b.forget_secret(), "nothing more to forget");
    }

    #[test]
    fn the_history_survives_a_restart_without_its_secrets() {
        let dir = std::env::temp_dir().join(format!("catalyst-clipboard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clipboard.json");
        let (mut b, now) = board();
        b.copy("10.58.5.2", Sensitivity::Normal, now, 1);
        b.copy("line one\nline two", Sensitivity::Normal, now, 2);
        b.toggle_pin("10.58.5.2").unwrap();
        b.copy("hunter2", Sensitivity::Secret, now, 3);
        std::fs::write(&path, b.file_text()).unwrap();
        let back = Board::open(path.clone());
        assert_eq!(back.items(), b.items());
        assert_eq!(back.plain(), None, "a secret was current: nothing comes back for it");
        b.copy("swap battery", Sensitivity::Normal, now, 4);
        std::fs::write(&path, b.file_text()).unwrap();
        assert_eq!(Board::open(path.clone()).plain().as_deref(), Some("swap battery"));
        // The current clip is written once, as its place in the history.
        assert_eq!(b.file_text().matches("swap battery").count(), 1);
        // A file cut short, or not the clipboard's: an empty history, not a failure.
        std::fs::write(&path, &b.file_text()[..20]).unwrap();
        assert!(Board::open(path.clone()).items().is_empty());
        std::fs::write(&path, r#"{"current": 7, "items": [{"text": "only one"}]}"#).unwrap();
        let odd = Board::open(path);
        assert_eq!((odd.items().len(), odd.plain()), (1, None), "a place past the end is no current clip");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removing_the_current_clip_empties_the_clipboard() {
        let (mut b, now) = board();
        b.copy("a", Sensitivity::Normal, now, 1);
        b.copy("b", Sensitivity::Normal, now, 2);
        assert!(b.remove("a"));
        assert_eq!(b.paste(now).as_deref(), Some("b"), "not the current one: the clipboard keeps b");
        assert!(b.remove("b"));
        assert_eq!(b.paste(now), None);
        b.copy("c", Sensitivity::Normal, now, 3);
        b.toggle_pin("c").unwrap();
        b.copy("d", Sensitivity::Normal, now, 4);
        b.clear();
        assert_eq!(b.items().iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["c"], "pins stay");
        assert_eq!(b.paste(now), None);
    }

    #[test]
    fn a_huge_copy_keeps_its_first_64_kb_whole_characters() {
        let (mut b, now) = board();
        let big = "é".repeat(MOST_BYTES);
        b.copy(&big, Sensitivity::Normal, now, 1);
        let kept = b.paste(now).unwrap();
        assert!(kept.len() <= MOST_BYTES && kept.len() > MOST_BYTES - 4);
        assert!(kept.chars().all(|c| c == 'é'));
    }

    #[test]
    fn the_island_says_what_was_copied_in_a_few_words() {
        assert_eq!(copied_words("10.58.5.2", Sensitivity::Normal, None), ["copied “10.58.5.2”", "copied"]);
        assert_eq!(copied_words("a\nb\nc", Sensitivity::Normal, None), ["copied · 3 lines", "copied"]);
        assert_eq!(copied_words(&"x".repeat(40), Sensitivity::Normal, None), ["copied · 40 characters", "copied"]);
        assert_eq!(copied_words("pw", Sensitivity::Secret, Some("password")), ["password copied · clears in 30 s", "password copied"]);
        assert_eq!(copied_words("pw", Sensitivity::Secret, None)[0], "copied · clears in 30 s");
    }

    #[test]
    fn a_paste_into_a_field_is_one_line() {
        assert_eq!(one_line("  swap\nthe\r\n\tbattery \x1b[31m "), "swap the battery [31m");
        assert_eq!(one_line("a  b"), "a  b", "its own spaces are kept");
        assert!(one_line(&"y".repeat(10_000)).len() <= MOST_TYPED);
    }

    #[test]
    fn the_keys_leave_a_terminal_its_ctrl_c_and_ctrl_v() {
        assert_eq!(hotkey(Key::Ctrl('c'), false), Some(Hotkey::Copy));
        assert_eq!(hotkey(Key::Ctrl('C'), false), Some(Hotkey::Copy));
        assert_eq!(hotkey(Key::Ctrl('v'), false), Some(Hotkey::Paste));
        assert_eq!(hotkey(Key::Ctrl('c'), true), None, "the interrupt");
        assert_eq!(hotkey(Key::Ctrl('v'), true), None);
        assert_eq!(hotkey(Key::Ctrl('C'), true), None, "the terminal takes Ctrl-Shift-C itself");
        assert_eq!(hotkey(Key::Ctrl('H'), true), Some(Hotkey::History));
        assert_eq!(hotkey(Key::Ctrl('N'), true), Some(Hotkey::Capture));
        assert_eq!(hotkey(Key::Ctrl('h'), false), None, "without Shift it is the screen's");
        assert_eq!(hotkey(Key::Char('c'), false), None);
    }

    #[test]
    fn a_row_copies_its_value_or_else_its_name() {
        assert_eq!(row_copy("Robot address", "10.58.5.2").as_deref(), Some("10.58.5.2"));
        assert_eq!(row_copy("Drive/", "3 ›").as_deref(), Some("Drive/"));
        assert_eq!(row_copy("Wi-Fi", "›").as_deref(), Some("Wi-Fi"));
        assert_eq!(row_copy("Team", "5805").as_deref(), Some("5805"), "a number is a value");
        assert_eq!(row_copy("Name", "a long value cut…").as_deref(), Some("Name"));
        assert_eq!(row_copy("", ""), None);
    }
}
