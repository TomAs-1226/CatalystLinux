//! Claude's pages: the sessions (and pairing, before there is a laptop),
//! the folders a new session can begin in, a conversation with its
//! composer and its permission sheet, and pairing by QR code or by code.
//!
//! Motion: the permission sheet grows out of the island's place on Bezel's
//! arriving spring (`apple-swiftui-snappy`, Detent's `role::release`), its
//! words in once it is past halfway, and goes back on the smooth one
//! (`role::smooth`); the transcript glides on the scroll spring the lists
//! use (`role::hold`). In the calm version all of it lands at once. At rest
//! nothing moves: while Claude works only its three dots do, looked in on
//! eight times a second ([`Screen::ambient`]), never on the frame beat.

use bezel::{Align, Rect, TextStyle};
use detent::{role, Animated};

use super::link::{self, Approval, Row, Trouble};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The longest message the composer takes.
const MESSAGE_MOST: usize = 4000;
/// Space between transcript rows.
const GAP: f32 = 5.0;
/// How far rows fade as they go under the transcript's top edge.
const FADE: f32 = 14.0;
/// Keys are let go this long after the permission sheet comes, so a key
/// meant for the composer never answers it (my own tuning, after Bezel's
/// 400 ms for a tap that ends a drag).
const SHEET_GUARD: f32 = 0.5;

/// The first line of some words, at most 60 characters: a title, a
/// notification's line.
pub fn first_words(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(59).collect::<String>())
    } else {
        line
    }
}

/// How long ago, from the laptop's clock: "now", "5 m", "2 h", "3 d".
fn ago(at: i64, now: i64) -> String {
    crate::notify::ago(at, now)
}

/// The keys along a page's foot: each a quiet keycap and what it does,
/// as many as fit (they are given most needed first), and the way to the
/// rest (`? keys`, or `fn 1 keys` where the page takes text) always at the
/// right. Lists end above it.
fn keys_bar(g: &mut Gfx, keys: &[(&str, &str)], typing: bool) {
    let style = g.ty.caption;
    let h = (style.size + 4.0).round();
    let cy = H - 2.0 - h * 0.5;
    g.take_foot(H - h - 4.0);
    let (cap, words) = if typing { ("fn 1", "keys") } else { ("?", "keys") };
    let words_w = g.measure(words, style);
    let cap_w = g.measure(cap, style) + 8.0;
    let right = W - PAD;
    let start = right - words_w - 4.0 - cap_w;
    crate::sheet::keys_line(g, start, right + 1.0, cy, &[(cap, words)], 1.0);
    crate::sheet::keys_line(g, PAD, start - 10.0, cy, keys, 1.0);
}

/// Three dots that breathe while Claude works (still in the calm version).
fn dots(g: &mut Gfx, x: f32, cy: f32, clock: f32) {
    let t = g.tokens;
    for i in 0..3 {
        let phase = clock * 4.5 - i as f32 * 0.8;
        let a = if g.calm { 0.7 } else { 0.35 + 0.65 * (0.5 + 0.5 * phase.sin()) };
        g.canvas.rrect(Rect::new(x + i as f32 * 7.0 - 2.0 + g.ox, cy - 2.0, 4.0, 4.0), 2.0, t.ink, a);
    }
}

/// The trouble, drawn where a page's content would be.
fn trouble_page(g: &mut Gfx, t: &Trouble) {
    g.empty(t.icon(), t.words());
}

// ------------------------------------------------------------------ app ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Way {
    Scan,
    Code,
}

const WAYS: [Way; 2] = [Way::Scan, Way::Code];

/// Claude, opened from Home: the laptop's sessions, or (unpaired) the ways
/// to pair with it.
pub struct ClaudeApp {
    list: ListState,
    held: bool,
    generation: u64,
    /// Unpair was pressed once: the next Enter on it unpairs.
    armed: bool,
}

impl Default for ClaudeApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeApp {
    pub fn new() -> ClaudeApp {
        let mut app = ClaudeApp { list: ListState::default(), held: false, generation: 0, armed: false };
        app.take_hold();
        if link::with(|s| s.paired.is_some()) {
            link::refresh();
        }
        app
    }

    fn take_hold(&mut self) {
        if !self.held {
            self.held = true;
            link::hold();
        }
    }
}

impl Drop for ClaudeApp {
    fn drop(&mut self) {
        if self.held {
            link::release();
        }
    }
}

impl Screen for ClaudeApp {
    fn title(&self) -> &str {
        "Claude"
    }

    fn shows_activity(&self) -> Option<crate::liveactivity::Kind> {
        Some(crate::liveactivity::Kind::Claude)
    }

    fn resume(&mut self) {
        self.take_hold();
        if link::with(|s| s.paired.is_some()) {
            link::refresh();
        }
    }

    fn set_aside(&mut self) {
        if self.held {
            self.held = false;
            link::release();
        }
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        let now = link::generation();
        let moved = now != self.generation;
        self.generation = now;
        moved || link::with(|s| s.asking && s.sessions.is_none())
    }

    /// Its dots breathe while the laptop is first asked: eight looks a
    /// second then; otherwise only what the link changes is drawn.
    fn ambient(&self) -> Option<std::time::Duration> {
        link::with(|s| s.asking && s.sessions.is_none()).then_some(std::time::Duration::from_millis(125))
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if link::with(|s| s.paired.is_none()) {
            return vec![keys("claude", &[("↑ ↓", "Choose"), ("enter", "Pair that way")])];
        }
        vec![keys("sessions", &[("enter", "Open it"), ("n", "New session"), ("r", "Ask the laptop again"), ("↑ ↓", "Choose"), ("enter on unpair", "Twice: forget the laptop")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let (paired, sessions, trouble) = link::with(|s| (s.paired.is_some(), s.sessions.clone(), s.trouble.clone()));
        if key == Key::Back || key == Key::Leave {
            return Nav::Pop;
        }
        if !paired {
            match key {
                Key::Enter => {
                    return match WAYS[self.list.focus.min(1)] {
                        Way::Scan => Nav::Push(Box::new(crate::camera::Camera::scanning())),
                        Way::Code => Nav::Push(Box::new(PairPage::by_code())),
                    }
                }
                other => crate::screens::list_key(&mut self.list, other, WAYS.len()),
            }
            return Nav::Stay;
        }
        if trouble == Some(Trouble::NotPaired) && key == Key::Enter {
            link::unpair();
            self.list.focus = 0;
            return Nav::Stay;
        }
        let list = sessions.unwrap_or_default();
        let n = list.len() + 2;
        if key != Key::Enter {
            self.armed = false;
        }
        match key {
            Key::Char('r') => link::refresh(),
            Key::Char('n') => return Nav::Push(Box::new(Projects::new())),
            Key::Enter if self.list.focus == 0 => return Nav::Push(Box::new(Projects::new())),
            Key::Enter if self.list.focus == n - 1 => {
                if self.armed {
                    link::unpair();
                    self.armed = false;
                    self.list.focus = 0;
                } else {
                    self.armed = true;
                }
            }
            Key::Enter => {
                if let Some(s) = list.get(self.list.focus - 1) {
                    link::open(s);
                    return Nav::Push(Box::new(Conversation::new()));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, n),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (paired, sessions, trouble, asking, working, approvals, now) = link::with(|s| (s.paired.clone(), s.sessions.clone(), s.trouble.clone(), s.asking, s.working.clone(), s.approvals.clone(), s.laptop_now));
        let Some(paired) = paired else {
            return draw_welcome(g, &mut self.list);
        };
        let top = g.top();
        if trouble == Some(Trouble::NotPaired) {
            g.empty("link_off", "the laptop no longer knows this handheld");
            keys_bar(g, &[("enter", "pair again"), ("esc", "back")], false);
            return;
        }
        let Some(list) = sessions else {
            match &trouble {
                Some(t) if !asking => trouble_page(g, t),
                _ => {
                    let cy = (top + H) * 0.5 - 6.0;
                    g.icon("laptop_windows", false, W * 0.5, cy - 12.0, 24.0, t.faint);
                    let words = format!("asking {}", paired.computer.to_lowercase());
                    let w = g.measure(&words, ty.caption);
                    g.text_at(W * 0.5 - 8.0, cy + 12.0, Align::Center, &words, ty.caption, t.dim);
                    dots(g, W * 0.5 - 8.0 + w * 0.5 + 6.0, cy + 12.0, crate::claude::ui::clock());
                }
            }
            keys_bar(g, &[("r", "try again"), ("esc", "back")], false);
            return;
        };
        let n = list.len() + 2;
        self.list.focus = self.list.focus.min(n - 1);
        // A line over the list when the laptop cannot be reached just now.
        let mut rows_top = top;
        if let Some(tr) = &trouble {
            let cy = top + ty.caption.size * 0.5 + 1.0;
            g.icon(tr.icon(), false, PAD + 7.0, cy, 12.0, g.fault());
            let with_key = format!("{} · r: try again", tr.words());
            let words = crate::sheet::first_fitting(g, &[&with_key, tr.words()], ty.caption, W - 2.0 * PAD - 17.0).to_owned();
            g.text_fit(PAD + 17.0, cy, &words, ty.caption, t.dim, W - 2.0 * PAD - 17.0);
            rows_top += ty.caption.size + 6.0;
        }
        keys_bar(g, &[("enter", "open"), ("n", "new"), ("r", "refresh")], false);
        let bottom = g.foot.min(H);
        let row_h = two_line_h(g);
        for (i, y) in self.list.layout(g, n, rows_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if i == 0 {
                two_line_row(g, r, Some(("add", t.dim, false)), "New session", "begin it in a folder", t.dim, focused);
                continue;
            }
            if i == n - 1 {
                let (sub, color) = if self.armed { ("enter again to forget it".to_owned(), g.fault()) } else { (format!("{} · enter twice", paired.computer.to_lowercase()), t.dim) };
                two_line_row(g, r, Some(("link_off", t.dim, false)), "Unpair the laptop", &sub, color, focused);
                continue;
            }
            let s = &list[i - 1];
            let (waiting, working) = (approvals.iter().any(|a| a.session == s.id), working.contains_key(&s.id));
            let icon = if waiting {
                Some(("front_hand", t.signal, true))
            } else if working {
                Some(("hourglass_top", g.good(), false))
            } else {
                None
            };
            let project = s.project.to_lowercase();
            let sub = if waiting {
                format!("{project} · waiting on an answer")
            } else if working {
                format!("{project} · Claude is working")
            } else {
                format!("{project} · {}", ago(s.last, now))
            };
            two_line_row(g, r, icon, &s.title, &sub, if waiting { t.signal } else { t.dim }, focused);
        }
        self.list.scrollbar(g, n, rows_top, bottom);
    }
}

/// A two-line row's height: its name, and a quiet line under it.
fn two_line_h(g: &Gfx) -> f32 {
    (g.ty.body.size + g.ty.caption.size + 13.0).round()
}

/// A row of two lines: a symbol on the left if it has one (a hand in the
/// signal colour for a session waiting on an answer, an hourglass while
/// Claude works in it), its name, and under it what it is (a session's
/// project and when it was last worked in).
fn two_line_row(g: &mut Gfx, r: Rect, icon: Option<(&str, bezel::Color, bool)>, name: &str, sub: &str, sub_color: bezel::Color, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let radius = g.radius(7.0).min(r.h * 0.5);
    g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
    if focused {
        g.focus_ring(r, radius);
    }
    let x = r.x + if icon.is_some() { 26.0 } else { 10.0 };
    let name_cy = r.y + 6.0 + ty.body.size * 0.5;
    let sub_cy = r.bottom() - 6.0 - ty.caption.size * 0.5;
    if let Some((icon, color, filled)) = icon {
        g.icon(icon, filled, r.x + 13.0, r.cy(), 13.0, color);
    }
    g.text_fit(x, name_cy, name, ty.body, t.ink, r.right() - 10.0 - x);
    g.text_fit(x, sub_cy, sub, ty.caption, sub_color, r.right() - 10.0 - x);
}

fn draw_welcome(g: &mut Gfx, list: &mut ListState) {
    let t = g.tokens;
    let ty = g.ty;
    let top = g.top();
    keys_bar(g, &[("enter", "go"), ("↑↓", "choose")], false);
    let bottom = g.foot.min(H);
    let row_h = g.row_h();
    let line_h = ty.caption.size + 3.0;
    let room = ((bottom - top - 8.0 - WAYS.len() as f32 * (row_h + 3.0)) / line_h).floor().max(0.0) as usize;
    let mut lines = Vec::new();
    for words in [
        "your Claude Code sessions on the laptop, from here. Pair once: Catalyst Companion on the laptop shows the code",
        "your laptop's Claude Code sessions, from here. Pair once with Catalyst Companion",
        "your laptop's Claude Code sessions",
    ] {
        lines = crate::help::wrap(g, words, ty.caption, W - 2.0 * PAD);
        if lines.len() <= room {
            break;
        }
    }
    if lines.len() > room {
        lines.clear();
    }
    for (k, l) in lines.iter().enumerate() {
        g.text_at(PAD + 2.0, top + 4.0 + line_h * (k as f32 + 0.5), Align::Left, l, ty.caption, t.dim);
    }
    let rows_top = top + if lines.is_empty() { 0.0 } else { 8.0 + line_h * lines.len() as f32 };
    for (i, y) in list.layout(g, WAYS.len(), rows_top, row_h, 3.0, bottom) {
        let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
        let (name, value) = match WAYS[i] {
            Way::Scan => ("Scan the laptop's QR code", "camera"),
            Way::Code => ("Type its address and code", "keyboard"),
        };
        g.row(r, name, value, t.dim, i == list.focus);
    }
    list.scrollbar(g, WAYS.len(), rows_top, bottom);
}

/// Seconds since the handheld's app began, for the dots.
pub fn clock() -> f32 {
    use std::sync::OnceLock;
    static BEGAN: OnceLock<std::time::Instant> = OnceLock::new();
    BEGAN.get_or_init(std::time::Instant::now).elapsed().as_secs_f32()
}

// ------------------------------------------------------------- projects ---

/// The folders sessions have worked in: a new session begins in one.
pub struct Projects {
    list: ListState,
    held: bool,
    generation: u64,
}

impl Projects {
    pub fn new() -> Projects {
        link::hold();
        if link::with(|s| s.projects.is_none()) {
            link::fetch_projects();
        }
        // Until the laptop answers, the folders the list already knows.
        link::with(|s| {
            if s.projects.is_none() {
                if let Some(list) = &s.sessions {
                    let mut seen: Vec<(String, String, i64)> = Vec::new();
                    for x in list {
                        if !x.cwd.is_empty() && !seen.iter().any(|(c, ..)| c.eq_ignore_ascii_case(&x.cwd)) {
                            seen.push((x.cwd.clone(), x.project.clone(), x.last));
                        }
                    }
                    s.projects = Some(seen);
                }
            }
        });
        Projects { list: ListState::default(), held: true, generation: 0 }
    }
}

impl Drop for Projects {
    fn drop(&mut self) {
        if self.held {
            link::release();
        }
    }
}

impl Screen for Projects {
    fn title(&self) -> &str {
        "New"
    }

    fn app(&self) -> &str {
        "Claude"
    }

    fn set_aside(&mut self) {
        if self.held {
            self.held = false;
            link::release();
        }
    }

    fn resume(&mut self) {
        if !self.held {
            self.held = true;
            link::hold();
        }
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        let now = link::generation();
        std::mem::replace(&mut self.generation, now) != now
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("new session", &[("enter", "Begin it in that folder"), ("↑ ↓", "Choose")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let projects = link::with(|s| s.projects.clone()).unwrap_or_default();
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter => {
                if let Some((cwd, name, _)) = projects.get(self.list.focus) {
                    link::begin(cwd, name);
                    return Nav::Push(Box::new(Conversation::new()));
                }
            }
            other => crate::screens::list_key(&mut self.list, other, projects.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (projects, trouble, now) = link::with(|s| (s.projects.clone(), s.trouble.clone(), s.laptop_now));
        let projects = match projects {
            Some(p) if !p.is_empty() => p,
            Some(_) => {
                g.empty("folder", "Claude Code has not worked in a folder on the laptop yet");
                return keys_bar(g, &[("esc", "back")], false);
            }
            None => {
                match trouble {
                    Some(tr) => trouble_page(g, &tr),
                    None => g.empty("folder", "asking the laptop for its folders"),
                }
                return keys_bar(g, &[("esc", "back")], false);
            }
        };
        keys_bar(g, &[("enter", "begin here"), ("esc", "back")], false);
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.foot.min(H));
        self.list.focus = self.list.focus.min(projects.len() - 1);
        for (i, y) in self.list.layout(g, projects.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (_, name, last) = &projects[i];
            g.row(r, name, &ago(*last, now), t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, projects.len(), top, bottom);
    }
}

// --------------------------------------------------------- conversation ---

/// A row laid out: its lines at the width it was wrapped to, and height.
#[derive(Clone)]
struct Laid {
    id: String,
    rev: u64,
    expanded: bool,
    lines: Vec<String>,
    /// For a tool row opened: its detail's lines, each marked as a line
    /// added (1), removed (2) or neither (0), as the line it wraps from.
    more: Vec<(String, u8)>,
    h: f32,
}

/// A conversation: its rows (Claude's words, its tool calls one line each,
/// yours), the composer, and a permission request as a sheet over them.
pub struct Conversation {
    held: bool,
    generation: u64,
    input: String,
    /// The row in focus, counted from the newest (None: following the newest).
    focus: Option<usize>,
    /// Tool rows opened, by id.
    open: Vec<String>,
    /// Up from the newest, in pixels.
    scroll: Animated,
    laid: Vec<Laid>,
    laid_for: (f32, f32),
    /// The sheet: which request, how far grown, its words, its choice, and
    /// how long it has shown.
    sheet_for: Option<String>,
    sheet: Animated,
    sheet_shown: f32,
    allow: bool,
    sheet_detail: bool,
    /// The newest request seen, kept while the sheet goes back.
    shown_approval: Option<Approval>,
    /// As last drawn: the transcript's height, the dots' under it, a line.
    area_h: f32,
    dots_h: f32,
    line: f32,
    /// A row has just been opened: the next drawing shows it from its top.
    land: bool,
}

impl Conversation {
    pub fn new() -> Conversation {
        link::hold();
        Conversation {
            held: true,
            generation: 0,
            input: String::new(),
            focus: None,
            open: Vec::new(),
            scroll: Animated::new(0.0, role::hold()).with_rest(0.3),
            laid: Vec::new(),
            laid_for: (0.0, 0.0),
            sheet_for: None,
            sheet: Animated::new(0.0, role::release()).with_rest(0.002),
            sheet_shown: 0.0,
            allow: true,
            sheet_detail: false,
            shown_approval: None,
            area_h: 100.0,
            dots_h: 0.0,
            line: 14.0,
            land: false,
        }
    }

    /// For the row `f` from the newest: the scroll that puts its foot at the
    /// view's foot, and the one that puts its top at the view's top (the
    /// larger, for a row taller than the view).
    fn range(&self, f: usize) -> Option<(f32, f32)> {
        let n = self.laid.len();
        if f >= n {
            return None;
        }
        let low = self.dots_h + self.laid[n - f..].iter().map(|l| l.h + GAP).sum::<f32>();
        let high = low + self.laid[n - 1 - f].h + GAP - self.area_h + 2.0;
        Some((low, high))
    }

    fn convo() -> Option<link::Convo> {
        link::with(|s| s.convo.clone())
    }

    /// The request waiting in this conversation, if any.
    fn waiting(&self) -> Option<Approval> {
        link::with(|s| {
            let id = s.convo.as_ref()?.session.clone()?;
            s.approval_in(&id).cloned()
        })
    }

    fn working(&self) -> bool {
        link::with(|s| s.convo.as_ref().is_some_and(|c| c.unsent.is_some() || c.session.as_ref().is_some_and(|id| s.is_working(id))))
    }

    fn send(&mut self, cx: &mut Cx) {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if self.working() {
            *cx.say = Some("Claude busy · ctrl x stops it".into());
            return;
        }
        if let Some(t) = link::with(|s| s.trouble.clone()) {
            if t == Trouble::NotPaired {
                *cx.say = Some(t.words().to_owned());
                return;
            }
        }
        link::send(&text);
        // Shown at once, as it will be once the laptop has it.
        link::with(|s| {
            if let Some(c) = s.convo.as_mut() {
                c.rows.push(Row { id: format!("local-{}", c.rows.len()), kind: "you".into(), text: text.clone(), ..Row::default() });
            }
        });
        self.input.clear();
        self.focus = None;
        self.scroll.set(0.0);
    }

    /// Lay the rows out again where they changed (or the text size did).
    fn lay_out(&mut self, g: &mut Gfx, rows: &[Row]) {
        let ty = g.ty;
        let key = (ty.caption.size, W);
        if self.laid_for != key {
            self.laid.clear();
            self.laid_for = key;
        }
        let chat = TextStyle::body(ty.caption.size);
        let lh = ty.caption.size + 3.5;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let expanded = self.open.contains(&r.id);
            if let Some(l) = self.laid.iter().find(|l| l.id == r.id && l.rev == r.rev && l.expanded == expanded) {
                out.push(l.clone());
                continue;
            }
            let (lines, more, h) = match r.kind.as_str() {
                "you" => {
                    let lines = wrap_text(g, &r.text, chat, W * 0.74 - 18.0);
                    let h = lines.len() as f32 * lh + 8.0;
                    (lines, Vec::new(), h)
                }
                "tool" => {
                    let mut more = Vec::new();
                    if expanded {
                        for source in r.detail.lines() {
                            let mark = if source.starts_with("+ ") { 1 } else if source.starts_with("− ") { 2 } else { 0 };
                            let wrapped = crate::help::wrap(g, &source.replace('\t', "  "), ty.caption, W - 2.0 * PAD - 22.0);
                            if wrapped.is_empty() {
                                more.push((String::new(), 0));
                            }
                            more.extend(wrapped.into_iter().map(|l| (l, mark)));
                        }
                    }
                    let shown = more.len().min(24);
                    let h = ty.caption.size + 9.0 + if expanded { shown as f32 * (ty.caption.size + 3.0) + 8.0 } else { 0.0 };
                    (Vec::new(), more, h)
                }
                "note" => {
                    let lines = crate::help::wrap(g, &r.text, ty.caption, W - 2.0 * PAD - 16.0);
                    let h = lines.len() as f32 * (ty.caption.size + 3.0) + 4.0;
                    (lines, Vec::new(), h)
                }
                _ => {
                    let lines = wrap_text(g, &r.text, chat, W - 2.0 * PAD - 10.0);
                    let h = lines.iter().map(|l| if l.is_empty() { lh * 0.5 } else { lh }).sum::<f32>().max(lh) + 2.0;
                    (lines, Vec::new(), h)
                }
            };
            out.push(Laid { id: r.id.clone(), rev: r.rev, expanded, lines, more, h });
        }
        self.laid = out;
    }

    fn sheet_key(&mut self, key: Key, a: &Approval, cx: &mut Cx) {
        if self.sheet_shown < SHEET_GUARD {
            return;
        }
        let decide = |allow: bool, cx: &mut Cx| {
            link::answer(&a.id, allow);
            *cx.say = Some(if allow { "allowed" } else { "denied" }.into());
        };
        match key {
            Key::Char('y' | 'a') => decide(true, cx),
            Key::Char('n' | 'd') | Key::Back => decide(false, cx),
            Key::Left | Key::Right | Key::Char('z' | 'c') => self.allow = !self.allow,
            Key::Tab => self.sheet_detail = !self.sheet_detail,
            Key::Enter => decide(self.allow, cx),
            _ => {}
        }
    }
}

impl Drop for Conversation {
    fn drop(&mut self) {
        if self.held {
            link::release();
        }
    }
}

/// Words wrapped to `width`, their own line breaks kept (a blank line a
/// short gap), Markdown's marks left out.
fn wrap_text(g: &mut Gfx, text: &str, style: TextStyle, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if trimmed.trim().is_empty() {
            if out.last().is_some_and(|l: &String| !l.is_empty()) {
                out.push(String::new());
            }
            continue;
        }
        let plain = if fence { trimmed.replace('\t', "  ") } else { trimmed.trim_start_matches('#').trim_start().replace("**", "").replace('`', "") };
        let plain = if !fence && (plain.starts_with("- ") || plain.starts_with("* ")) { format!("• {}", &plain[2..]) } else { plain };
        let wrapped = crate::help::wrap(g, &plain, style, width);
        if wrapped.is_empty() {
            out.push(plain);
        } else {
            out.extend(wrapped);
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

impl Screen for Conversation {
    fn title(&self) -> &str {
        "Claude"
    }

    fn app(&self) -> &str {
        "Claude"
    }

    fn shows_activity(&self) -> Option<crate::liveactivity::Kind> {
        Some(crate::liveactivity::Kind::Claude)
    }

    fn wants_text(&self) -> bool {
        self.sheet_for.is_none()
    }

    fn dictation(&self) -> Option<crate::voice::Dictation> {
        self.sheet_for.is_none().then_some(crate::voice::Dictation::Text)
    }

    /// What was said lands in the composer, after what is typed; Enter sends it.
    fn dictated(&mut self, text: &str, cx: &mut Cx) -> Nav {
        let joined = if self.input.trim().is_empty() { text.trim().to_owned() } else { format!("{} {}", self.input.trim_end(), text.trim()) };
        self.input = joined.chars().filter(|c| !c.is_control()).take(MESSAGE_MOST).collect();
        *cx.say = Some("enter sends it".into());
        Nav::Stay
    }

    fn set_aside(&mut self) {
        if self.held {
            self.held = false;
            link::release();
        }
    }

    fn resume(&mut self) {
        if !self.held {
            self.held = true;
            link::hold();
        }
    }

    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        if !self.input.trim().is_empty() {
            return Some(self.input.clone());
        }
        let c = Self::convo()?;
        let i = c.rows.len().checked_sub(1 + self.focus.unwrap_or(0))?;
        let r = c.rows.get(i)?;
        Some(if r.kind == "tool" { format!("{}\n{}", r.text, r.detail) } else { r.text.clone() })
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.sheet_for.is_some() {
            return vec![keys("permission", &[("y", "Allow it, once"), ("n · esc", "Deny it"), ("← →", "Choose"), ("enter", "Do what is chosen"), ("tab", "The whole of it")])];
        }
        vec![
            keys("writing", &[("type", "Your message"), ("enter", "Send it"), ("alt", "Talk, then enter sends"), ("fn bksp", "Clear it")]),
            keys("the conversation", &[("↑ ↓", "Through it"), ("enter", "Open a tool's row"), ("tab", "Ask for everything, or accept edits"), ("ctrl x", "Stop Claude"), ("esc", "Back to the sessions")]),
        ]
    }

    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        let now = link::generation();
        let moved = now != self.generation;
        self.generation = now;
        if self.sheet_for.is_some() {
            self.sheet_shown += dt;
        }
        moved || self.working() || self.sheet_for.is_some()
    }

    /// While Claude works its dots breathe: eight looks a second. With only
    /// a request's sheet up, its time left: one a second. Otherwise the usual.
    fn ambient(&self) -> Option<std::time::Duration> {
        if self.working() {
            Some(std::time::Duration::from_millis(125))
        } else {
            self.sheet_for.is_some().then_some(std::time::Duration::from_secs(1))
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt) | self.sheet.tick(dt)
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let (Some(a), true) = (self.waiting(), self.sheet_for.is_some()) {
            if key == Key::Leave {
                return Nav::Pop;
            }
            self.sheet_key(key, &a, cx);
            return Nav::Stay;
        }
        let c = Self::convo();
        let n = c.as_ref().map_or(0, |c| c.rows.len());
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Ctrl('x') => {
                if self.working() {
                    link::interrupt();
                    *cx.say = Some("stopping".into());
                } else {
                    *cx.say = Some("Claude is not working".into());
                }
            }
            Key::Tab => {
                let next = if c.as_ref().is_some_and(|c| c.mode == "edits") { "ask" } else { "edits" };
                link::set_mode(next);
                *cx.say = Some(if next == "edits" { "accepts edits in its folder" } else { "asks for everything" }.into());
            }
            Key::Enter if self.input.trim().is_empty() => {
                // On a tool's row: open it, or close it.
                if let Some(r) = c.as_ref().and_then(|c| c.rows.get(n.checked_sub(1 + self.focus?)?)) {
                    if r.kind == "tool" {
                        if let Some(i) = self.open.iter().position(|x| *x == r.id) {
                            self.open.remove(i);
                        } else {
                            self.open.push(r.id.clone());
                            self.land = true;
                        }
                    }
                }
            }
            Key::Enter => self.send(cx),
            // Through a row taller than the view three lines at a time,
            // else to the row before (or after).
            Key::Up => match self.focus.and_then(|f| self.range(f)) {
                Some((low, high)) if high > low && self.scroll.target() < high - 1.0 => self.scroll.set((self.scroll.target() + 3.0 * self.line).min(high)),
                _ => {
                    self.focus = Some(self.focus.map_or(0, |f| (f + 1).min(n.saturating_sub(1))));
                }
            },
            Key::Down => match self.focus.and_then(|f| self.range(f)) {
                Some((low, high)) if high > low && self.scroll.target() > low + 1.0 => self.scroll.set((self.scroll.target() - 3.0 * self.line).max(low)),
                _ => {
                    self.focus = match self.focus {
                        Some(0) | None => None,
                        Some(f) => Some(f - 1),
                    };
                }
            },
            Key::PageUp => self.focus = Some(self.focus.map_or(3, |f| f + 4).min(n.saturating_sub(1))),
            Key::PageDown => self.focus = self.focus.and_then(|f| f.checked_sub(4)),
            Key::Home => self.focus = Some(n.saturating_sub(1)),
            Key::End => self.focus = None,
            Key::Backspace => {
                self.input.pop();
            }
            Key::Delete => self.input.clear(),
            Key::Char(ch) if !ch.is_control() => {
                if self.input.chars().count() < MESSAGE_MOST {
                    self.input.push(ch);
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let Some(c) = Self::convo() else {
            return g.empty("code_blocks", "no conversation open");
        };
        let (trouble, working) = link::with(|s| (s.trouble.clone(), c.session.as_ref().is_some_and(|id| s.is_working(id)) || c.unsent.is_some()));
        let elsewhere = link::with(|s| s.approvals.iter().find(|a| Some(&a.session) != c.session.as_ref()).map(|a| (a.session.clone(), s.sessions.iter().flatten().find(|x| x.id == a.session).map(|x| x.title.clone()).unwrap_or_default())));
        // The sheet: up for a request in this conversation.
        let waiting = self.waiting();
        match (&waiting, &self.sheet_for) {
            (Some(a), None) => {
                self.sheet_for = Some(a.id.clone());
                self.sheet_shown = 0.0;
                self.allow = true;
                self.sheet_detail = false;
                self.shown_approval = Some(a.clone());
                if g.calm {
                    self.sheet.jump(1.0);
                } else {
                    self.sheet.set_with(1.0, role::release());
                    g.started_moving();
                }
            }
            (Some(a), Some(id)) if a.id != *id => {
                self.sheet_for = Some(a.id.clone());
                self.sheet_shown = 0.0;
                self.allow = true;
                self.shown_approval = Some(a.clone());
            }
            (None, Some(_)) => {
                self.sheet_for = None;
                if g.calm {
                    self.sheet.jump(0.0);
                } else {
                    self.sheet.set_with(0.0, role::smooth());
                    g.started_moving();
                }
            }
            _ => {}
        }
        // The keys along the foot, then the composer above them.
        let typing = self.sheet_for.is_none();
        if working {
            keys_bar(g, &[("enter", "send"), ("ctrl x", "stop"), ("tab", if c.mode == "edits" { "ask" } else { "edits" }), ("alt", "talk")], typing);
        } else {
            keys_bar(g, &[("enter", "send"), ("tab", if c.mode == "edits" { "ask" } else { "edits" }), ("alt", "talk"), ("↑↓", "scroll")], typing);
        }
        let chat = TextStyle::body(ty.caption.size);
        let lh = ty.caption.size + 3.5;
        let composer = self.draw_composer(g, &c, working, trouble.as_ref());
        let mut top = g.top();
        if let Some((_, title)) = &elsewhere {
            let cy = top + ty.caption.size * 0.5 + 1.0;
            g.icon("front_hand", true, PAD + 7.0, cy, 12.0, t.signal);
            g.text_fit(PAD + 17.0, cy, &format!("an answer is waiting in {}", if title.is_empty() { "another session" } else { title }), ty.caption, t.dim, W - 2.0 * PAD - 17.0);
            top += ty.caption.size + 6.0;
        }
        let area = Rect::new(0.0, top, W, composer.y - 4.0 - top);
        if !c.loaded {
            let words = match (&trouble, &c.error) {
                (_, Some(e)) => e.clone(),
                (Some(tr), _) => tr.words().to_owned(),
                _ => "fetching the conversation".to_owned(),
            };
            let cy = area.cy();
            g.icon(trouble.as_ref().map_or("history", |t| t.icon()), false, W * 0.5, cy - 9.0, 20.0, t.faint);
            let words = g.text.fit(&words, &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, cy + 11.0, Align::Center, &words, ty.caption, t.dim);
        } else if c.rows.is_empty() && !working {
            let cy = area.cy();
            g.icon("code_blocks", false, W * 0.5, cy - 9.0, 20.0, t.faint);
            let words = format!("a new session in {}", c.project);
            let words = g.text.fit(&words, &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, cy + 11.0, Align::Center, &words, ty.caption, t.dim);
        } else {
            self.lay_out(g, &c.rows);
            self.draw_rows(g, &c, area, working, chat, lh);
        }
        if let Some(e) = c.error.as_ref().filter(|_| c.loaded) {
            // What went wrong, just over the composer.
            let r = Rect::new(PAD, composer.y - ty.caption.size - 10.0, W - 2.0 * PAD, ty.caption.size + 7.0);
            g.surface(r, g.radius(6.0).min(r.h * 0.5), t.surface2);
            g.icon("error", false, r.x + 9.0, r.cy(), 11.0, g.fault());
            g.text_fit(r.x + 18.0, r.cy(), e, ty.caption, t.ink, r.w - 24.0);
        }
        if self.sheet.value() > 0.002 {
            if let Some(a) = self.shown_approval.clone() {
                self.draw_sheet(g, &a);
            }
        }
    }
}

impl Conversation {
    /// The composer along the foot, above the keys: the way it asks on its
    /// left, the words (up to three lines, the newest), and the way to send
    /// or stop on its right. Where it is.
    fn draw_composer(&mut self, g: &mut Gfx, c: &link::Convo, working: bool, trouble: Option<&Trouble>) -> Rect {
        let t = g.tokens;
        let ty = g.ty;
        let chat = TextStyle::body(ty.caption.size);
        let lh = ty.caption.size + 3.5;
        let mode = if c.mode == "edits" { "edits" } else { "ask" };
        let chip_w = g.measure(mode, ty.caption) + 12.0;
        let room = W - 2.0 * PAD - 16.0 - chip_w - 26.0;
        let mut lines = if self.input.is_empty() { Vec::new() } else { crate::help::wrap(g, &self.input, chat, room) };
        if self.input.ends_with(' ') && !lines.is_empty() {
            // The space just typed shows where the next word goes.
            if let Some(last) = lines.last_mut() {
                last.push(' ');
            }
        }
        let shown = lines.len().clamp(1, 3);
        let bar_h = (chat.size + 13.0 + (shown - 1) as f32 * lh).round();
        let foot = g.foot.min(H);
        let bar = Rect::new(PAD, foot - 3.0 - bar_h, W - 2.0 * PAD, bar_h);
        let radius = ((chat.size + 13.0) * 0.5).min(bar.h * 0.5);
        g.surface(bar, radius, t.surface2);
        // The way it asks.
        let chip = Rect::new(bar.x + 5.0, bar.bottom() - (chat.size + 13.0) * 0.5 - (ty.caption.size + 5.0) * 0.5, chip_w, ty.caption.size + 5.0);
        g.surface(chip, chip.h * 0.5, if mode == "edits" { t.ice } else { t.surface3 });
        g.text_at(chip.cx(), chip.cy(), Align::Center, mode, ty.caption, if mode == "edits" { t.on_ice } else { t.dim });
        let x = chip.right() + 7.0;
        let focused = self.sheet_for.is_none();
        if self.input.is_empty() {
            let words = match trouble {
                Some(tr) => tr.words().to_owned(),
                None if working => "Claude is working · ctrl x stops it".to_owned(),
                None if c.session.is_none() => "the first message begins it".to_owned(),
                None => "message Claude · alt talks".to_owned(),
            };
            let cy = bar.bottom() - (chat.size + 13.0) * 0.5;
            if focused {
                g.surface(Rect::new(x, cy - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
            }
            g.text_fit(x + 5.0, cy, &words, chat, t.faint, room);
        } else {
            let skip = lines.len().saturating_sub(3);
            let lines = lines.split_off(skip);
            let mut end = (x, bar.y);
            for (k, l) in lines.iter().enumerate() {
                let cy = bar.y + (chat.size + 13.0) * 0.5 + k as f32 * lh;
                let w = g.text_at(x, cy, Align::Left, l, chat, t.ink);
                end = (x + w, cy);
            }
            if focused {
                g.surface(Rect::new(end.0 + 1.0, end.1 - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
                g.copy_field = Some(self.input.clone());
            }
        }
        // Send, or (while Claude works) stop.
        let cy = bar.bottom() - (chat.size + 13.0) * 0.5;
        let (icon, color) = if working { ("stop_circle", t.ink) } else if self.input.trim().is_empty() { ("send", t.faint) } else { ("send", t.signal) };
        g.icon(icon, !working && !self.input.trim().is_empty(), bar.right() - 13.0, cy, 15.0, color);
        if focused && self.focus.is_none() {
            g.focus_ring(bar, radius);
        }
        bar
    }

    /// The rows, newest at the foot, scrolled to keep the one in focus in
    /// view; Claude's dots under them while it works.
    fn draw_rows(&mut self, g: &mut Gfx, c: &link::Convo, area: Rect, working: bool, chat: TextStyle, lh: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let n = self.laid.len();
        let dots_h = if working { ty.caption.size + 8.0 } else { 0.0 };
        (self.area_h, self.dots_h, self.line) = (area.h, dots_h, lh);
        let total: f32 = self.laid.iter().map(|l| l.h + GAP).sum::<f32>() + dots_h;
        let max = (total - area.h).max(0.0);
        // The row in focus in view: wholly if it fits, else as far as it
        // has been read through.
        if let Some(f) = self.focus {
            self.focus = Some(f.min(n.saturating_sub(1)));
        }
        let land = std::mem::take(&mut self.land);
        let target = match self.focus.and_then(|f| self.range(f)) {
            None => 0.0,
            // A row just opened shows from its top.
            Some((low, high)) if land => high.max(low),
            Some((low, high)) => self.scroll.target().clamp(high.max(0.0).min(low), low.max(high)),
        };
        let target = target.clamp(0.0, max);
        if (self.scroll.target() - target).abs() > 0.5 {
            if g.calm {
                self.scroll.jump(target);
            } else {
                self.scroll.set(target);
                g.started_moving();
            }
        }
        let scroll = self.scroll.value().clamp(0.0, max.max(self.scroll.value()));
        // What is beneath the area's top edge, for the rows to go under it
        // rather than be cut there.
        let band = ((area.y.ceil() as usize).min(H as usize), (FADE as usize).min(area.h.max(0.0) as usize));
        let beneath: Vec<u32> = g.canvas.px[band.0 * W as usize..(band.0 + band.1) * W as usize].to_vec();
        // And above its foot, for rows scrolled up past the composer.
        let foot_band = ((area.bottom().floor() as usize).saturating_sub(band.1).max(band.0 + band.1), band.1);
        let foot_beneath: Vec<u32> = g.canvas.px[foot_band.0 * W as usize..(foot_band.0 + foot_band.1).min(H as usize) * W as usize].to_vec();
        g.canvas.set_clip(area);
        let mut y = area.bottom() + scroll;
        if working {
            y -= dots_h;
            let cy = y + dots_h * 0.5;
            dots(g, PAD + 6.0, cy, clock());
            let words = if c.status.is_empty() { "working" } else { c.status.as_str() };
            g.text_fit(PAD + 30.0, cy, words, ty.caption, t.dim, W - 2.0 * PAD - 30.0);
        }
        for i in (0..n).rev() {
            let l = self.laid[i].clone();
            y -= l.h + GAP;
            if y + l.h < area.y {
                break;
            }
            if y > area.bottom() {
                continue;
            }
            let Some(row) = c.rows.get(i) else { continue };
            let focused = self.focus == Some(n - 1 - i);
            draw_row(g, row, &l, Rect::new(PAD, y, W - 2.0 * PAD, l.h), focused, chat, lh);
        }
        g.canvas.set_clip(Rect::new(0.0, 0.0, W, H));
        // The top edge: what scrolled past it goes under, fading out.
        let w = W as usize;
        if scroll < max - 0.5 {
            for k in 0..band.1 {
                let f = 1.0 - (k as f32 + 0.5) / band.1 as f32;
                let at = (band.0 + k) * w;
                bezel::simd::mix_into(&mut g.canvas.px[at..at + w], Some(&beneath[k * w..(k + 1) * w]), (f * f * 256.0) as u32);
            }
        }
        // The foot: what is scrolled down under the composer fades there.
        if scroll > 0.5 && foot_beneath.len() == foot_band.1 * w {
            for k in 0..foot_band.1 {
                let f = (k as f32 + 0.5) / foot_band.1 as f32;
                let at = (foot_band.0 + k) * w;
                bezel::simd::mix_into(&mut g.canvas.px[at..at + w], Some(&foot_beneath[k * w..(k + 1) * w]), (f * f * 256.0) as u32);
            }
        }
        // Where along it, when it runs past.
        if max > 1.0 {
            let track = Rect::new(W - 4.0, area.y + 3.0, 2.0, area.h - 6.0);
            let shown = (area.h / total).clamp(0.08, 1.0);
            let thumb_h = track.h * shown;
            let along = 1.0 - (scroll / max).clamp(0.0, 1.0);
            g.canvas.rrect(track, 1.0, t.ink, 0.06);
            g.canvas.rrect(Rect::new(track.x, track.y + (track.h - thumb_h) * along, 2.0, thumb_h), 1.0, t.ink, 0.35);
        }
    }

    /// The permission request, on a sheet grown out of the island's place:
    /// what Claude wants to do, where, the command or file itself, and
    /// allow or deny.
    fn draw_sheet(&mut self, g: &mut Gfx, a: &Approval) {
        let t = g.tokens;
        let ty = g.ty;
        let grow = self.sheet.value().max(0.0);
        let words = ((grow - 0.55) / 0.45).clamp(0.0, 1.0);
        // The screen behind steps back.
        let dim = (0.28 * grow.min(1.0) * 256.0) as u32;
        if dim > 0 {
            bezel::simd::mix_into(&mut g.canvas.px, None, dim);
        }
        // From just under the island, whose line (the title, the island,
        // the clock) stays in sight over it.
        let under = 4.0 + g.island_h() + 4.0;
        let rest = Rect::new(crate::sheet::SIDE, under, W - 2.0 * crate::sheet::SIDE, H - under - crate::sheet::FOOT);
        let island = Rect::new(W * 0.5 - 30.0, under - 2.0, 60.0, g.island_h());
        let lerp = |a: f32, b: f32| a + (b - a) * grow;
        let r = Rect::new(lerp(island.x, rest.x).max(1.0), lerp(island.y, rest.y).max(1.0), lerp(island.w, rest.w).min(W - 2.0), lerp(island.h, rest.h).clamp(1.0, H - 2.0));
        let radius = (island.h * 0.5 + (g.radius(crate::sheet::RADIUS) - island.h * 0.5) * grow.clamp(0.0, 1.0)).min(r.h * 0.5);
        g.shadow(r, radius, grow.min(1.0));
        g.canvas.rrect(r, radius, t.surface2, 1.0);
        g.canvas.stroke_rrect(r, radius, 1.0, t.ink, 0.08 * grow.min(1.0));
        if words <= 0.0 {
            return;
        }
        g.canvas.set_clip(r.inset(1.0));
        let x0 = r.x + 12.0;
        let w = r.w - 24.0;
        // What it wants to do, and where.
        let cy = r.y + 7.0 + ty.name.size * 0.5;
        g.icon_alpha("front_hand", true, x0 + 7.0, cy, 15.0, t.signal, words);
        let what = format!("Claude wants to {}", a.what);
        let title = crate::sheet::first_fitting(g, &[&what, &a.what, "Allow it?"], ty.name, w - 22.0).to_owned();
        g.text_alpha(x0 + 20.0, cy, &title, ty.name, t.ink, words);
        let place_cy = cy + ty.name.size * 0.5 + 4.0 + ty.caption.size * 0.5;
        let place = g.text.fit(&a.place, &ty.caption, w - 20.0);
        g.text_alpha(x0 + 20.0, place_cy, &place, ty.caption, t.dim, words);
        // The command, file or address itself, mono, as much as fits; under
        // it how long it waits for an answer.
        let buttons_h = ty.body.size + 12.0;
        let cap_lh = ty.caption.size + 3.0;
        let box_top = place_cy + ty.caption.size * 0.5 + 6.0;
        let box_bottom = r.bottom() - 10.0 - buttons_h - 6.0 - cap_lh - 2.0;
        let text = if self.sheet_detail && !a.detail.is_empty() { a.detail.clone() } else { a.line.clone() };
        let mut lines = wrap_text(g, &text, ty.caption, w - 16.0);
        let fits = (((box_bottom - box_top) - 8.0) / cap_lh).floor().max(1.0) as usize;
        if lines.len() > fits {
            lines.truncate(fits);
            if let Some(last) = lines.last_mut() {
                *last = g.text.fit(&format!("{last}…"), &ty.caption, w - 16.0);
            }
        }
        let box_h = (lines.len() as f32 * cap_lh + 8.0).min(box_bottom - box_top);
        let bx = Rect::new(x0, box_top, w, box_h.max(cap_lh + 8.0));
        g.canvas.rrect(bx, g.radius(6.0).min(8.0), t.surface3, words);
        for (k, l) in lines.iter().enumerate() {
            g.text_alpha(bx.x + 8.0, bx.y + 4.0 + (k as f32 + 0.5) * cap_lh, l, ty.caption, t.ink, words);
        }
        let left = (a.until - link::laptop_now()).max(0);
        let wait = format!("{}:{:02}", left / 60, left % 60);
        let more = if a.detail.is_empty() || a.detail == a.line { "" } else if self.sheet_detail { " · tab: less" } else { " · tab: all of it" };
        let note = crate::sheet::first_fitting(g, &[&format!("denied unanswered in {wait}{more}"), &format!("denied in {wait}{more}"), &format!("denied in {wait}")], ty.caption, w).to_owned();
        g.text_alpha(x0 + 2.0, bx.bottom() + 3.0 + cap_lh * 0.5, &note, ty.caption, if left < 60 { t.amber_ink } else { t.dim }, words);
        // Allow, deny: capsules, the one chosen ringed.
        let by = r.bottom() - 10.0 - buttons_h;
        let half = (w - 8.0) * 0.5;
        for (k, (icon, label, key)) in [("check_circle", "allow", "y"), ("block", "deny", "n")].iter().enumerate() {
            let b = Rect::new(x0 + k as f32 * (half + 8.0), by, half, buttons_h);
            let chosen = (k == 0) == self.allow;
            g.canvas.rrect(b, b.h * 0.5, if chosen { t.surface3.mix(t.ink, 0.06) } else { t.surface1 }, words);
            let style = if g.theme == crate::look::Theme::Bezel { ty.label } else { ty.body };
            let lw = g.measure(label, style);
            let kw = g.measure(key, ty.caption) + 8.0;
            let start = b.cx() - (14.0 + lw + 6.0 + kw) * 0.5;
            let color = if k == 0 { t.signal } else { g.fault() };
            g.icon_alpha(icon, chosen, start + 6.0, b.cy(), 13.0, color, words);
            g.text_alpha(start + 15.0, b.cy(), label, style, t.ink, words);
            let cap = Rect::new(start + 14.0 + lw + 6.0, b.cy() - (ty.caption.size + 4.0) * 0.5, kw, ty.caption.size + 4.0);
            g.canvas.rrect(cap, 3.5, t.ink, 0.10 * words);
            g.text_alpha(cap.x + 4.0, b.cy(), key, ty.caption, t.dim, words);
            if chosen && words >= 1.0 {
                g.focus_ring(b, b.h * 0.5);
            }
        }
        g.canvas.set_clip(Rect::new(0.0, 0.0, W, H));
    }
}

/// One row of the transcript.
fn draw_row(g: &mut Gfx, row: &Row, l: &Laid, r: Rect, focused: bool, chat: TextStyle, lh: f32) {
    let t = g.tokens;
    let ty = g.ty;
    match row.kind.as_str() {
        "you" => {
            // Yours: on the right, on a tonal bubble.
            let w = l.lines.iter().map(|x| g.measure(x, chat)).fold(0.0f32, f32::max) + 16.0;
            let b = Rect::new(r.right() - w, r.y, w, l.h);
            let radius = g.radius(9.0).min(lh * 0.5 + 4.0);
            g.surface(b, radius, t.surface2);
            for (k, line) in l.lines.iter().enumerate() {
                g.text_at(b.x + 8.0, b.y + 4.0 + (k as f32 + 0.5) * lh, Align::Left, line, chat, t.ink);
            }
            if focused {
                g.focus_ring(b, radius);
            }
        }
        "tool" => {
            let line_h = ty.caption.size + 9.0;
            let head = Rect::new(r.x, r.y, r.w, line_h);
            let radius = g.radius(6.0).min(line_h * 0.5);
            g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
            let cy = head.cy();
            let icon = if row.icon.is_empty() { "build" } else { row.icon.as_str() };
            g.icon(icon, false, head.x + 9.0, cy, 11.0, t.dim);
            // How it went, on the right.
            let (mark, color, filled) = match row.state.as_str() {
                "ok" => ("check_circle", g.good(), false),
                "error" => ("error", g.fault(), false),
                "denied" => ("block", g.fault(), false),
                "cut" => ("stop_circle", t.faint, false),
                _ => ("hourglass_top", t.dim, false),
            };
            g.icon(mark, filled, head.right() - 9.0, cy, 11.0, color);
            let more = if !row.detail.is_empty() && !l.expanded { 10.0 } else { 0.0 };
            g.text_fit(head.x + 19.0, cy, &row.text, ty.caption, t.ink, head.w - 36.0 - more);
            if more > 0.0 {
                g.icon("chevron_right", false, head.right() - 21.0, cy, 10.0, t.faint);
            }
            if l.expanded {
                let cap_lh = ty.caption.size + 3.0;
                let shown = l.more.len().min(24);
                for (k, (line, mark)) in l.more.iter().take(shown).enumerate() {
                    let color = match mark {
                        1 => g.good(),
                        2 => g.fault(),
                        _ => t.dim,
                    };
                    g.text_at(r.x + 19.0, head.bottom() + 2.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, color);
                }
            }
            if focused {
                g.focus_ring(r, radius);
            }
        }
        "note" => {
            for (k, line) in l.lines.iter().enumerate() {
                g.text_at(W * 0.5, r.y + 2.0 + (k as f32 + 0.5) * (ty.caption.size + 3.0), Align::Center, line, ty.caption, t.faint);
            }
            if focused {
                g.focus_ring(r, 6.0);
            }
        }
        _ => {
            let mut y = r.y + 1.0;
            for line in &l.lines {
                if line.is_empty() {
                    y += lh * 0.5;
                    continue;
                }
                g.text_at(r.x + 2.0, y + lh * 0.5, Align::Left, line, chat, t.ink);
                y += lh;
            }
            if focused {
                g.focus_ring(r.inset(-2.0), g.radius(6.0));
            }
        }
    }
}

// ----------------------------------------------------------------- pair ---

/// Pairing with the laptop: from its QR code read in the Camera (shown,
/// then Enter), or by its address and the six-digit code it shows.
pub struct PairPage {
    invite: Option<catalyst_companion_proto::claude::Invite>,
    address: crate::tools::Field,
    code: crate::tools::Field,
    /// Which field takes the keys (by code): 0 the address, 1 the code.
    at: usize,
    generation: u64,
    started: bool,
}

impl PairPage {
    pub fn scanned(invite: catalyst_companion_proto::claude::Invite) -> PairPage {
        link::with(|s| s.pairing = None);
        PairPage { invite: Some(invite), address: crate::tools::Field::new("laptop's address", "", 120), code: crate::tools::Field::new("code", "", 7), at: 0, generation: 0, started: false }
    }

    pub fn by_code() -> PairPage {
        link::with(|s| s.pairing = None);
        let before = link::with(|s| s.paired.as_ref().map(|p| p.address.trim_start_matches("https://").to_owned())).unwrap_or_default();
        PairPage { invite: None, address: crate::tools::Field::new("laptop's address · from Companion", &before, 120), code: crate::tools::Field::new("six-digit code", "", 7), at: if before.is_empty() { 0 } else { 1 }, generation: 0, started: false }
    }

    fn outcome() -> Option<Result<String, String>> {
        link::with(|s| s.pairing.clone())
    }
}

impl Screen for PairPage {
    fn title(&self) -> &str {
        "Pair"
    }

    fn app(&self) -> &str {
        "Claude"
    }

    fn wants_text(&self) -> bool {
        self.invite.is_none() && !matches!(Self::outcome(), Some(Ok(ref n)) if !n.is_empty())
    }

    fn secret(&self) -> bool {
        self.invite.is_none() && self.at == 1
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        let now = link::generation();
        std::mem::replace(&mut self.generation, now) != now || matches!(Self::outcome(), Some(Ok(ref n)) if n.is_empty())
    }

    fn ambient(&self) -> Option<std::time::Duration> {
        matches!(Self::outcome(), Some(Ok(ref n)) if n.is_empty()).then_some(std::time::Duration::from_millis(125))
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.invite.is_some() {
            return vec![keys("pair", &[("enter", "Pair with this laptop"), ("esc", "Not now")])];
        }
        vec![keys("pair by code", &[("type", "The address, then the code"), ("tab · ↓ ↑", "The other field"), ("enter", "Next, then pair")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let outcome = Self::outcome();
        if let Some(Ok(name)) = &outcome {
            if !name.is_empty() {
                // Paired: on to the sessions.
                return match key {
                    Key::Enter => Nav::Push(Box::new(ClaudeApp::new())),
                    Key::Back | Key::Leave => Nav::Pop,
                    _ => Nav::Stay,
                };
            }
            // Under way.
            return if matches!(key, Key::Back | Key::Leave) { Nav::Pop } else { Nav::Stay };
        }
        if let Some(invite) = &self.invite {
            return match key {
                Key::Enter => {
                    self.started = true;
                    link::pair(super::Paired { address: invite.address.clone(), key: invite.key.clone(), computer: invite.computer.clone(), since: link::laptop_now() });
                    Nav::Stay
                }
                Key::Back | Key::Leave => Nav::Pop,
                _ => Nav::Stay,
            };
        }
        if matches!(outcome, Some(Err(_))) && !matches!(key, Key::Back | Key::Leave) {
            link::with(|s| s.pairing = None);
        }
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Tab | Key::Down if self.at == 0 => self.at = 1,
            Key::Tab | Key::Up if self.at == 1 => self.at = 0,
            Key::Enter if self.at == 0 => self.at = 1,
            Key::Enter => {
                let address = catalyst_companion_proto::claude::clean_address(&self.address.text);
                let code = catalyst_companion_proto::clean_code(&self.code.text);
                match (address, code) {
                    (None, _) => {
                        link::with(|s| s.pairing = Some(Err("that address is not one: it looks like laptop.tail1234.ts.net".into())));
                        self.at = 0;
                    }
                    (_, None) => link::with(|s| s.pairing = Some(Err("the code is six digits".into()))),
                    (Some(a), Some(c)) => {
                        self.started = true;
                        link::pair_by_code(&a, &c);
                    }
                }
            }
            other => {
                let field = if self.at == 0 { &mut self.address } else { &mut self.code };
                let allowed: fn(char) -> bool = if self.at == 0 { |c| crate::tools::host_char(c) || c == '/' } else { |c| c.is_ascii_digit() || c == ' ' };
                field.key(other, allowed);
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let outcome = Self::outcome();
        let top = g.top();
        if let Some(Ok(name)) = &outcome {
            if !name.is_empty() {
                let cy = (top + H) * 0.5 - 8.0;
                g.icon("check_circle", true, W * 0.5, cy - 14.0, 24.0, g.good());
                let words = g.text.fit(&format!("paired with {}", name.to_lowercase()), &ty.body, W - 2.0 * PAD);
                g.text_at(W * 0.5, cy + 12.0, Align::Center, &words, ty.body, t.ink);
                return keys_bar(g, &[("enter", "its sessions")], false);
            }
        }
        let busy = matches!(&outcome, Some(Ok(n)) if n.is_empty());
        if let Some(invite) = &self.invite {
            // The laptop the code named, and what pairing means.
            keys_bar(g, &[("enter", "pair"), ("esc", "not now")], false);
            let card = Rect::new(PAD, top, W - 2.0 * PAD, ty.name.size + ty.caption.size + 22.0);
            g.surface(card, g.radius(9.0), t.surface1);
            g.icon("laptop_windows", false, card.x + 16.0, card.cy(), 18.0, t.ink);
            let name_cy = card.y + 9.0 + ty.name.size * 0.5;
            g.text_fit(card.x + 32.0, name_cy, &invite.computer, ty.name, t.ink, card.w - 40.0);
            g.text_fit(card.x + 32.0, name_cy + ty.name.size * 0.5 + 4.0 + ty.caption.size * 0.5, invite.address.trim_start_matches("https://"), ty.caption, t.dim, card.w - 40.0);
            let mut y = card.bottom() + 8.0;
            let lh = ty.caption.size + 3.0;
            let words = match &outcome {
                Some(Err(why)) => why.clone(),
                _ if busy => "pairing…".to_owned(),
                _ => "its Claude Code sessions, from here. Only text crosses, sealed with a key only the two hold.".to_owned(),
            };
            let color = if matches!(outcome, Some(Err(_))) { g.fault() } else { t.dim };
            for line in crate::help::wrap(g, &words, ty.caption, W - 2.0 * PAD).into_iter().take(3) {
                g.text_at(PAD + 2.0, y + lh * 0.5, Align::Left, &line, ty.caption, color);
                y += lh;
            }
            if busy {
                let x = PAD + 4.0 + g.measure("pairing…", ty.caption) + 8.0;
                dots(g, x, y - lh * 0.5, clock());
            }
            return;
        }
        // By code: the address, the code.
        keys_bar(g, &[("enter", if self.at == 0 { "next" } else { "pair" }), ("tab", "other field"), ("esc", "back")], true);
        let fh = crate::tools::Field::height(g);
        for (k, field) in [&self.address, &self.code].into_iter().enumerate() {
            let r = Rect::new(PAD, top + k as f32 * (fh + 5.0), W - 2.0 * PAD, fh);
            let focused = k == self.at && !busy;
            g.surface(r, 14.0, t.surface2);
            g.text_at(r.x + 10.0, r.y + 7.0 + ty.caption.size * 0.5, Align::Left, field.label, ty.caption, t.dim);
            let cy = r.bottom() - 8.0 - ty.label.size * 0.5;
            let shown = if k == 1 { field.text.chars().map(|c| if c == ' ' { ' ' } else { '•' }).collect::<String>() } else { field.text.clone() };
            let shown = crate::sheet::fit_tail(g, &shown, ty.label, r.w - 26.0);
            let w = g.text_at(r.x + 10.0, cy, Align::Left, &shown, ty.label, t.ink);
            if focused {
                g.surface(Rect::new(r.x + 11.0 + w, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
                g.focus_ring(r, 14.0);
            }
        }
        let mut y = top + 2.0 * (fh + 5.0) + 2.0;
        let (words, color) = match &outcome {
            Some(Err(why)) => (why.clone(), g.fault()),
            _ if busy => ("pairing…".to_owned(), t.dim),
            _ => {
                let w = W - 2.0 * PAD - 4.0;
                (crate::sheet::first_fitting(g, &["Companion on the laptop shows both", "Companion shows both"], ty.caption, w).to_owned(), t.dim)
            }
        };
        // As many whole lines as fit above the keys.
        let lh = ty.caption.size + 3.0;
        let room = ((g.foot.min(H) - y - 2.0) / lh).floor().max(1.0) as usize;
        let mut lines = crate::help::wrap(g, &words, ty.caption, W - 2.0 * PAD - 4.0);
        if lines.len() > room {
            let rest = lines.split_off(room - 1).join(" ");
            lines.push(g.text.fit(&rest, &ty.caption, W - 2.0 * PAD - 4.0));
        }
        for line in &lines {
            g.text_at(PAD + 2.0, y + lh * 0.5 + 1.0, Align::Left, line, ty.caption, color);
            y += lh;
        }
    }
}

