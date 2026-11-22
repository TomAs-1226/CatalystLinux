//! Browse: the web on a 320 by 170 panel, read for you. Type an address and
//! the assistant's service reads the page (Claude's `web_fetch`, OpenAI's
//! hosted search opening it) and gives it back as clean text: its title,
//! its sections, and the links worth following, which Enter follows. Type a
//! question and it searches and answers, with the pages it used as links.
//!
//! It is the assistant's engine (`crate::ai`, in Browse mode: the web only,
//! no robot tools) with one fresh conversation per page, so following links
//! does not pile up what each page cost; the address goes in the question
//! itself, which is what lets the service read it. Pages already seen are
//! kept for Backspace (or Esc) to go back to.
//!
//! The on-device model has no web: Browse needs an OpenAI or Claude key.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use bezel::{Align, Rect};
use detent::{role, Animated};
use serde_json::Value;

use crate::ai::{self, Chat, Effort, Mode, Prefs, Service, Source, Stop, WebStep};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::keys::Keys;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// Places to start from on an empty page.
const STARTS: [(&str, &str); 5] = [
    ("The Blue Alliance", "https://www.thebluealliance.com/"),
    ("WPILib documentation", "https://docs.wpilib.org/en/stable/"),
    ("Chief Delphi", "https://www.chiefdelphi.com/latest"),
    ("FRC Game Q&A", "https://frc-qa.firstinspires.org/"),
    ("FIRST news", "https://www.firstinspires.org/robotics/frc/blog"),
];

/// Times one page's paused turn is carried on.
const CONTINUATIONS: usize = 4;
const INPUT_LIMIT: usize = 400;
/// Pages kept to go back to.
const HISTORY: usize = 12;

/// A block of a page: a heading or a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(String),
    Para(String),
}

/// A link worth following.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub text: String,
    pub url: String,
}

/// A page as Browse shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// What was asked: the address, or the question.
    pub asked: String,
    pub title: String,
    pub blocks: Vec<Block>,
    pub links: Vec<Link>,
    pub steps: Vec<WebStep>,
}

/// The address typed, as a URL: a word with a dot in it, no spaces;
/// `https://` put in front when it has no scheme. Anything else is a question.
pub fn address(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.contains(char::is_whitespace) || t.ends_with(['?', '.', '!']) {
        return None;
    }
    if t.starts_with("https://") || t.starts_with("http://") {
        return Some(t.to_owned());
    }
    let host = t.split('/').next().unwrap_or(t);
    (host.contains('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c))).then(|| format!("https://{t}"))
}

/// A link line: "text | url", or a bare URL.
fn link_of(line: &str) -> Option<Link> {
    let line = line.trim().trim_start_matches(['-', '*', '·']).trim();
    let (text, url) = match line.rsplit_once('|') {
        Some((text, url)) => (text.trim().to_owned(), url.trim().to_owned()),
        None => (String::new(), line.to_owned()),
    };
    let url = url.trim_matches(['<', '>', '(', ')']).to_owned();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains(char::is_whitespace) {
        return None;
    }
    Some(Link { text: if text.is_empty() { ai::host_of(&url) } else { text }, url })
}

/// The service's answer laid out as a page: TITLE, paragraphs and headings,
/// then LINKS. An answer that ignored the form still reads as text, its
/// sources as its links.
pub fn parse(asked: &str, text: &str, steps: Vec<WebStep>, sources: &[Source]) -> Page {
    let mut title = String::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();
    let mut para = String::new();
    let mut in_links = false;
    let flush = |para: &mut String, blocks: &mut Vec<Block>| {
        let p = ai::plain(para.trim());
        if !p.is_empty() {
            blocks.push(Block::Para(p.replace('\n', " ")));
        }
        para.clear();
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if in_links {
            if let Some(link) = link_of(trimmed) {
                if !links.iter().any(|l: &Link| l.url == link.url) {
                    links.push(link);
                }
            }
            continue;
        }
        if title.is_empty() && blocks.is_empty() && para.is_empty() {
            if let Some(t) = trimmed.strip_prefix("TITLE:") {
                title = t.trim().to_owned();
                continue;
            }
        }
        if trimmed.eq_ignore_ascii_case("LINKS:") {
            flush(&mut para, &mut blocks);
            in_links = true;
        } else if let Some(h) = trimmed.strip_prefix("## ").or_else(|| trimmed.strip_prefix("# ")) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(h.trim().trim_matches('*').to_owned()));
        } else if trimmed.is_empty() {
            flush(&mut para, &mut blocks);
        } else {
            if !para.is_empty() {
                para.push('\n');
            }
            para.push_str(trimmed);
        }
    }
    flush(&mut para, &mut blocks);
    if title.is_empty() {
        title = ai::short(asked, 60);
    }
    for s in sources {
        if !links.iter().any(|l| l.url == s.url) {
            links.push(Link { text: s.title.clone(), url: s.url.clone() });
        }
    }
    links.truncate(12);
    Page { asked: asked.to_owned(), title, blocks, links, steps }
}

/// A request under way: the conversation for the page, and what was asked.
struct Job {
    rx: Receiver<Result<Value, String>>,
    chat: Chat,
    asked: String,
    /// What the page says it is doing meanwhile.
    doing: String,
    continuations: usize,
    steps: Vec<WebStep>,
    text: String,
    sources: Vec<Source>,
}

/// The page laid out: its rows from the top, each a line of text with its
/// style and colour, or a link.
#[derive(Clone)]
enum Row {
    Title(String),
    Steps(String),
    Heading(String),
    Line(String),
    Gap,
    LinksHead,
    Link(usize, String, String),
}

pub struct Browse {
    service: Option<(Service, String)>,
    effort: Effort,
    input: String,
    editing: bool,
    pages: Vec<Page>,
    job: Option<Job>,
    problem: Option<String>,
    /// Down from the top of the page, in pixels.
    scroll: Animated,
    /// The link in focus, if any; on an empty page, the start in focus.
    link: Option<usize>,
    starts: ListState,
    clock: f32,
    waited: f32,
    laid: Option<(usize, f32, usize, Vec<(Row, f32, f32)>)>,
}

impl Browse {
    pub fn new() -> Self {
        let prefs = Prefs::load();
        let service = ai::answering(&prefs, &Keys::load())
            .filter(|(s, _)| s.online())
            .or_else(|| {
                // The on-device model chosen: the services with a key instead.
                let keys = Keys::load();
                [Service::OpenAi, Service::Claude].into_iter().find_map(|s| keys.get(s.key()).map(|k| (s, k.to_owned())))
            });
        Self::with(service, prefs.effort)
    }

    fn with(service: Option<(Service, String)>, effort: Effort) -> Self {
        Browse {
            service,
            effort,
            input: String::new(),
            editing: true,
            pages: Vec::new(),
            job: None,
            problem: None,
            scroll: Animated::new(0.0, role::settle()).with_rest(0.3),
            link: None,
            starts: ListState::default(),
            clock: 0.0,
            waited: 0.0,
            laid: None,
        }
    }

    fn page(&self) -> Option<&Page> {
        self.pages.last()
    }

    /// Go: an address read, or a question searched.
    fn go(&mut self, typed: &str) {
        let typed = typed.trim();
        if typed.is_empty() {
            return;
        }
        let Some((service, key)) = self.service.clone() else { return };
        let mut chat = Chat::for_mode(service, Mode::Browse);
        let (asked, doing) = match address(typed) {
            // The address in the question itself: the service may only read
            // pages the conversation has named.
            Some(url) => {
                chat.ask(&format!("Open {url}"));
                (url.clone(), format!("reading {}", ai::host_of(&url)))
            }
            None => {
                chat.ask(typed);
                (typed.to_owned(), "searching the web".to_owned())
            }
        };
        let rx = spawn(service, &key, chat.body(self.effort));
        self.job = Some(Job { rx, chat, asked, doing, continuations: 0, steps: Vec::new(), text: String::new(), sources: Vec::new() });
        self.problem = None;
        self.editing = false;
        self.input.clear();
        self.waited = 0.0;
    }

    /// The answer, laid out as the page now shown; or its trouble said.
    fn took(&mut self, result: Result<Value, String>) {
        let Some(mut job) = self.job.take() else { return };
        let reply = match result.and_then(|v| job.chat.take(&v)) {
            Ok(reply) => reply,
            Err(why) => {
                self.problem = Some(why);
                return;
            }
        };
        job.steps.extend(reply.steps);
        if !reply.text.trim().is_empty() {
            if !job.text.is_empty() {
                job.text.push('\n');
            }
            job.text.push_str(&reply.text);
        }
        job.sources.extend(reply.sources);
        match reply.stop {
            Stop::Paused if job.continuations < CONTINUATIONS => {
                // Its own loop ran long: the conversation goes back as it is.
                job.continuations += 1;
                if let Some((service, key)) = &self.service {
                    job.rx = spawn(*service, key, job.chat.body(self.effort));
                    self.job = Some(job);
                }
                return;
            }
            Stop::Refused(why) => {
                self.problem = Some(format!("it would not read that: {why}"));
                return;
            }
            _ => {}
        }
        if job.text.trim().is_empty() {
            self.problem = Some("nothing came back for that".into());
            return;
        }
        let page = parse(&job.asked, &job.text, job.steps, &job.sources);
        self.pages.push(page);
        if self.pages.len() > HISTORY {
            self.pages.remove(0);
        }
        self.link = None;
        self.scroll.jump(0.0);
        self.laid = None;
    }

    fn back(&mut self) -> bool {
        if self.pages.len() > 1 {
            self.pages.pop();
            self.link = None;
            self.scroll.jump(0.0);
            self.laid = None;
            true
        } else {
            false
        }
    }

    /// The link before or after the one in focus, scrolled into view.
    fn move_link(&mut self, by: isize) {
        let n = self.page().map_or(0, |p| p.links.len());
        if n == 0 {
            return;
        }
        let at = match self.link {
            None if by > 0 => 0,
            None => n - 1,
            Some(i) => (i as isize + by).clamp(0, n as isize - 1) as usize,
        };
        self.link = Some(at);
        if let Some((_, _, _, rows)) = &self.laid {
            if let Some((_, y, h)) = rows.iter().find(|(r, _, _)| matches!(r, Row::Link(i, _, _) if *i == at)) {
                let view = VIEW.with(std::cell::Cell::get);
                let target = self.scroll.target();
                if *y < target {
                    self.scroll.set(*y - 4.0);
                } else if y + h > target + view {
                    self.scroll.set(y + h - view + 4.0);
                }
            }
        }
    }

    fn scroll_by(&mut self, by: f32) {
        let most = self.laid.as_ref().map_or(0.0, |(_, _, _, rows)| rows.last().map_or(0.0, |(_, y, h)| y + h));
        let view = VIEW.with(std::cell::Cell::get);
        self.scroll.set((self.scroll.target() + by).clamp(0.0, (most - view).max(0.0)));
    }

    /// The page's rows at the text size in use (laid out again only when the
    /// page or the size changes).
    fn lay_out(&mut self, g: &mut Gfx) {
        let size = ai::chat_style(g).size;
        let key = (self.pages.len(), size, self.page().map_or(0, |p| p.links.len()));
        if self.laid.as_ref().is_some_and(|(n, s, l, _)| (*n, *s, *l) == key) {
            return;
        }
        let Some(page) = self.page().cloned() else { return };
        let ty = g.ty;
        let chat = ai::chat_style(g);
        let lh = ai::chat_line(g);
        let width = W - 2.0 * PAD - 6.0;
        let mut rows: Vec<(Row, f32, f32)> = Vec::new();
        let mut y = 0.0;
        let mut put = |row: Row, h: f32, rows: &mut Vec<(Row, f32, f32)>| {
            rows.push((row, y, h));
            y += h;
        };
        for line in crate::help::wrap(g, &page.title, ty.name, width).into_iter().take(3) {
            put(Row::Title(line), ty.name.size + 5.0, &mut rows);
        }
        if !page.steps.is_empty() {
            let words: Vec<&str> = page.steps.iter().map(|s| s.words.as_str()).collect();
            put(Row::Steps(words.join(" · ")), ty.caption.size + 6.0, &mut rows);
        }
        put(Row::Gap, 4.0, &mut rows);
        for block in &page.blocks {
            match block {
                Block::Heading(h) => {
                    put(Row::Gap, 3.0, &mut rows);
                    for line in crate::help::wrap(g, h, ty.body, width) {
                        put(Row::Heading(line), ty.body.size + 5.0, &mut rows);
                    }
                }
                Block::Para(p) => {
                    for line in crate::help::wrap(g, p, chat, width) {
                        put(Row::Line(line), lh, &mut rows);
                    }
                    put(Row::Gap, lh * 0.5, &mut rows);
                }
            }
        }
        if !page.links.is_empty() {
            put(Row::Gap, 4.0, &mut rows);
            put(Row::LinksHead, ty.caption.size + 6.0, &mut rows);
            for (i, link) in page.links.iter().enumerate() {
                put(Row::Link(i, link.text.clone(), ai::host_of(&link.url)), g.row_h() + 3.0, &mut rows);
            }
        }
        self.laid = Some((key.0, key.1, key.2, rows));
    }

    fn draw_start(&mut self, g: &mut Gfx, area: Rect) {
        let t = g.tokens;
        let ty = g.ty;
        g.text_at(PAD + 2.0, area.y + 2.0 + ty.caption.size * 0.5, Align::Left, "a question, or an address", ty.caption, t.dim);
        let top = area.y + ty.caption.size + 10.0;
        let focused = self.focus_on_starts();
        // A list like any other: the chosen one glides into view.
        for (i, y) in self.starts.layout(g, STARTS.len(), top, g.row_h(), 3.0, area.bottom()) {
            let (name, url) = STARTS[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, g.row_h());
            g.row(r, name, &ai::host_of(url), t.dim, focused && i == self.starts.focus);
        }
        self.starts.scrollbar(g, STARTS.len(), top, area.bottom());
    }

    /// On an empty page with nothing typed, Up and Down choose a start.
    fn focus_on_starts(&self) -> bool {
        self.pages.is_empty() && self.input.is_empty()
    }

    /// The address bar along the bottom: what is typed, or the page's
    /// address (or question) while reading.
    fn draw_bar(&self, g: &mut Gfx, bar: Rect) {
        let t = g.tokens;
        let chat = ai::chat_style(g);
        let radius = bar.h * 0.5;
        g.surface(bar, radius, t.surface2);
        let cy = bar.cy();
        g.icon("travel_explore", false, bar.x + 13.0, cy, 14.0, t.dim);
        let x = bar.x + 25.0;
        let room = bar.w - 36.0;
        if self.editing {
            let shown = if self.input.is_empty() { None } else { Some(tail_fit(g, &self.input, chat, room)) };
            match shown {
                None => {
                    g.surface(Rect::new(x, cy - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
                    g.text_at(x + 5.0, cy, Align::Left, "ask, or type an address", chat, t.faint);
                }
                Some(text) => {
                    let w = g.text_at(x, cy, Align::Left, &text, chat, t.ink);
                    g.surface(Rect::new(x + w + 1.0, cy - chat.size * 0.5, 1.5, chat.size), 0.5, t.signal);
                }
            }
            // On the empty page with nothing typed, the ring is on a start.
            if !self.focus_on_starts() {
                g.focus_ring(bar, radius);
            }
        } else {
            let here = self.job.as_ref().map(|j| j.asked.clone()).or_else(|| self.page().map(|p| p.asked.clone())).unwrap_or_default();
            let shown = g.text.fit(&here, &chat, room);
            g.text_at(x, cy, Align::Left, &shown, chat, t.dim);
        }
    }
}

/// The end of a long line that fits `room`: where the typing is.
fn tail_fit(g: &mut Gfx, text: &str, style: bezel::TextStyle, room: f32) -> String {
    let mut shown: &str = text;
    while g.measure(shown, style) > room {
        let mut chars = shown.chars();
        chars.next();
        shown = chars.as_str();
    }
    shown.to_owned()
}

/// One request on a thread of its own.
fn spawn(service: Service, key: &str, body: Value) -> Receiver<Result<Value, String>> {
    let (tx, rx) = mpsc::channel();
    let key = key.to_owned();
    std::thread::Builder::new()
        .name("browse".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            tx.send(ai::request(service, &key, &body)).ok();
        })
        .ok();
    rx
}

thread_local! {
    /// The height of the page's view and of a line, as last drawn, for
    /// scrolling.
    static VIEW: std::cell::Cell<f32> = const { std::cell::Cell::new(100.0) };
    static LINE: std::cell::Cell<f32> = const { std::cell::Cell::new(15.0) };
}

impl Screen for Browse {
    fn title(&self) -> &str {
        "Browse"
    }

    fn wants_text(&self) -> bool {
        // Letters type an address or a question whenever the page is shown.
        self.service.is_some()
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.job.is_some()
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if self.service.is_none() {
            return match key {
                Key::Enter | Key::Right => Nav::Push(Box::new(crate::screens::SettingsApp::keys_page())),
                Key::Back | Key::Leave => Nav::Pop,
                _ => Nav::Stay,
            };
        }
        let line = LINE.with(std::cell::Cell::get) * 3.0;
        match key {
            Key::Leave => return Nav::Pop,
            // Reading: Esc stops it.
            Key::Back if self.job.is_some() => {
                self.job = None;
                self.problem = Some("stopped".into());
            }
            Key::Back if self.editing && !self.input.is_empty() => self.input.clear(),
            Key::Back if self.editing && !self.pages.is_empty() => self.editing = false,
            Key::Back => {
                if !self.back() {
                    return Nav::Pop;
                }
            }
            Key::Up if self.focus_on_starts() => self.starts.up(),
            Key::Down if self.focus_on_starts() => self.starts.down(STARTS.len()),
            Key::Up => self.scroll_by(-line),
            Key::Down => self.scroll_by(line),
            Key::PageUp => self.scroll_by(-line * 3.0),
            Key::PageDown => self.scroll_by(line * 3.0),
            Key::Left if !self.editing || self.input.is_empty() => self.move_link(-1),
            Key::Right if !self.editing || self.input.is_empty() => self.move_link(1),
            Key::Tab => {
                self.editing = true;
                self.input.clear();
            }
            Key::Enter if self.job.is_some() => {}
            Key::Enter if self.editing && !self.input.trim().is_empty() => {
                let typed = std::mem::take(&mut self.input);
                self.go(&typed);
            }
            Key::Enter if self.focus_on_starts() => {
                let url = STARTS[self.starts.focus].1;
                self.go(url);
            }
            Key::Enter => {
                if let Some(link) = self.link.and_then(|i| self.page().and_then(|p| p.links.get(i)).cloned()) {
                    self.go(&link.url);
                } else {
                    self.editing = true;
                }
            }
            Key::Backspace if self.editing && !self.input.is_empty() => {
                self.input.pop();
            }
            Key::Backspace => {
                self.back();
            }
            Key::Char(c) if !c.is_control() && self.input.chars().count() < INPUT_LIMIT => {
                self.editing = true;
                self.input.push(c);
            }
            _ => {}
        }
        Nav::Stay
    }

    fn update(&mut self, _cx: &mut Cx) -> Nav {
        if ai::demo() && self.waited < ai::DEMO_PAUSE {
            return Nav::Stay;
        }
        let got = match &self.job {
            Some(job) => match job.rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err("the request stopped".into())),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        if let Some(result) = got {
            self.took(result);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.waited += dt;
        self.scroll.tick(dt) || self.job.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if self.service.is_none() {
            g.empty("travel_explore", "browsing needs an OpenAI or Claude key");
            g.hint("enter: settings › keys");
            return;
        }
        if g.calm {
            self.scroll.jump(self.scroll.target());
        }
        let chat = ai::chat_style(g);
        let bar_h = chat.size + 13.0;
        let bar = Rect::new(PAD, H - PAD - bar_h, W - 2.0 * PAD, bar_h);
        let area = Rect::new(0.0, g.top(), W, bar.y - 4.0 - g.top());
        VIEW.with(|v| v.set(area.h));
        LINE.with(|l| l.set(ai::chat_line(g)));
        // Reading or searching: what it is doing, gently pulsing.
        let status = self.job.as_ref().map(|j| j.doing.clone());
        if self.pages.is_empty() {
            match (&status, &self.problem) {
                (Some(doing), _) => draw_doing(g, area, doing, self.clock),
                (None, Some(why)) => {
                    g.icon("warning", false, W * 0.5, area.cy() - 10.0, 18.0, t.dim);
                    let line = g.text.fit(why, &ty.caption, W - 2.0 * PAD);
                    g.text_at(W * 0.5, area.cy() + 12.0, Align::Center, &line, ty.caption, t.dim);
                }
                (None, None) => self.draw_start(g, area),
            }
            self.draw_bar(g, bar);
            return;
        }
        self.lay_out(g);
        let Some((_, _, _, rows)) = self.laid.clone() else { return };
        let scroll = self.scroll.value();
        g.canvas.set_clip(area);
        let edge = 12.0;
        let fade = |cy: f32| ((cy - area.y) / edge).clamp(0.0, 1.0) * ((area.bottom() - cy) / edge).clamp(0.0, 1.0).max(if scroll + area.h >= rows.last().map_or(0.0, |(_, y, h)| y + h) - 1.0 { 1.0 } else { 0.0 });
        let mut ring = None;
        for (row, y, h) in &rows {
            let top = area.y + y - scroll;
            if top + h < area.y || top > area.bottom() {
                continue;
            }
            let cy = top + h * 0.5;
            let a = fade(cy);
            match row {
                Row::Title(line) => g.text_alpha(PAD + 2.0, cy, line, ty.name, t.ink, a),
                Row::Steps(words) => {
                    g.icon_alpha("language", false, PAD + 8.0, cy, 12.0, t.faint, a);
                    let words = g.text.fit(words, &ty.caption, W - 2.0 * PAD - 22.0);
                    g.text_alpha(PAD + 19.0, cy, &words, ty.caption, t.faint, a);
                }
                Row::Heading(line) => g.text_alpha(PAD + 2.0, cy, line, ty.body, t.ink, a),
                Row::Line(line) => g.text_alpha(PAD + 2.0, cy, line, chat, t.ink, a),
                Row::Gap => {}
                Row::LinksHead => g.text_alpha(PAD + 2.0, cy, "links · ←→ choose · enter follows", ty.caption, t.dim, a),
                Row::Link(i, text, host) => {
                    let r = Rect::new(PAD, top + 1.5, W - 2.0 * PAD, h - 3.0);
                    let focused = self.link == Some(*i);
                    g.canvas.rrect(r.offset(g.ox, 0.0), (r.h * 0.5).min(12.0), t.surface1, a);
                    let host_w = g.measure(host, ty.caption).min(r.w * 0.4);
                    let host = g.text.fit(host, &ty.caption, r.w * 0.4);
                    g.text_alpha(r.right() - 10.0 - host_w, cy, &host, ty.caption, t.dim, a);
                    let text = g.text.fit(text, &chat, r.w - host_w - 30.0);
                    g.text_alpha(r.x + 10.0, cy, &text, chat, t.ink, a);
                    if focused {
                        ring = Some((r, (r.h * 0.5).min(12.0)));
                    }
                }
            }
        }
        g.canvas.clear_clip();
        if let Some((r, radius)) = ring {
            g.focus_ring(r, radius);
        }
        if let Some(doing) = &status {
            // Over the page being left: a pill saying what is coming.
            let w = g.measure(doing, ty.caption) + 40.0;
            let pill = Rect::new(W * 0.5 - w * 0.5, bar.y - 6.0 - (ty.caption.size + 10.0), w, ty.caption.size + 10.0);
            g.surface(pill, pill.h * 0.5, t.surface2);
            draw_dots(g, pill.x + 14.0, pill.cy(), self.clock);
            g.text_at(pill.x + 30.0, pill.cy(), Align::Left, doing, ty.caption, t.dim);
        } else if let Some(why) = &self.problem {
            let line = g.text.fit(why, &ty.caption, W - 2.0 * PAD - 20.0);
            let w = g.measure(&line, ty.caption) + 24.0;
            let pill = Rect::new(W * 0.5 - w * 0.5, bar.y - 6.0 - (ty.caption.size + 10.0), w, ty.caption.size + 10.0);
            g.surface(pill, pill.h * 0.5, t.surface2);
            g.text_at(W * 0.5, pill.cy(), Align::Center, &line, ty.caption, t.dim);
        }
        self.draw_bar(g, bar);
    }
}

/// Three dots breathing, as the assistant's are.
fn draw_dots(g: &mut Gfx, x: f32, cy: f32, clock: f32) {
    let t = g.tokens;
    for i in 0..3 {
        let phase = clock * 4.5 - i as f32 * 0.8;
        let a = if g.calm { 0.7 } else { 0.35 + 0.65 * (0.5 + 0.5 * phase.sin()) };
        g.canvas.rrect(Rect::new(x + i as f32 * 7.0 - 2.0 + g.ox, cy - 2.0, 4.0, 4.0), 2.0, t.ink, a);
    }
}

fn draw_doing(g: &mut Gfx, area: Rect, doing: &str, clock: f32) {
    let t = g.tokens;
    let ty = g.ty;
    let w = g.measure(doing, ty.caption) + 26.0;
    let x = W * 0.5 - w * 0.5;
    draw_dots(g, x + 4.0, area.cy(), clock);
    g.text_at(x + 26.0, area.cy(), Align::Left, doing, ty.caption, t.dim);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_told_from_a_question() {
        assert_eq!(address("docs.wpilib.org").as_deref(), Some("https://docs.wpilib.org"));
        assert_eq!(address("thebluealliance.com/team/5805").as_deref(), Some("https://thebluealliance.com/team/5805"));
        assert_eq!(address("http://10.58.5.2:9010/api/system").as_deref(), Some("http://10.58.5.2:9010/api/system"));
        assert_eq!(address("how long is autonomous?"), None);
        assert_eq!(address("rev.robotics"), Some("https://rev.robotics".into()));
        assert_eq!(address("e.g."), None, "a word ending in a dot is not an address");
        assert_eq!(address("hello"), None);
    }

    #[test]
    fn a_page_is_laid_out_from_its_form_and_falls_back_to_plain_text() {
        let text = "TITLE: WPILib Documentation\nWPILib is the library.\nIt has two lines.\n\n## Getting started\nInstall it.\n\nLINKS:\nZero to Robot | https://docs.wpilib.org/zero\n- Hardware | https://docs.wpilib.org/hw\nnot a link\n";
        let steps = vec![WebStep { words: "read docs.wpilib.org".into(), ok: true }];
        let page = parse("https://docs.wpilib.org", text, steps.clone(), &[]);
        assert_eq!(page.title, "WPILib Documentation");
        assert_eq!(page.blocks, [Block::Para("WPILib is the library. It has two lines.".into()), Block::Heading("Getting started".into()), Block::Para("Install it.".into())]);
        assert_eq!(page.links, [Link { text: "Zero to Robot".into(), url: "https://docs.wpilib.org/zero".into() }, Link { text: "Hardware".into(), url: "https://docs.wpilib.org/hw".into() }]);
        assert_eq!(page.steps, steps);
        // An answer that ignored the form: its words as text, the question as
        // its title, its sources as links.
        let loose = parse("who makes the falcon?", "**CTR Electronics** makes it.", Vec::new(), &[Source { title: "CTRE".into(), url: "https://ctr-electronics.com/".into() }]);
        assert_eq!(loose.title, "who makes the falcon?");
        assert_eq!(loose.blocks, [Block::Para("CTR Electronics makes it.".into())]);
        assert_eq!(loose.links, [Link { text: "CTRE".into(), url: "https://ctr-electronics.com/".into() }]);
    }

    fn wait(page: &mut Browse) {
        let (mut robot, mut settings, mut say) = (Robot::demo(), crate::shell::Settings::default(), None);
        for _ in 0..300 {
            page.tick(0.1);
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
            page.update(&mut cx);
            if page.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
    }

    #[test]
    fn an_address_is_read_and_its_links_followed_and_back_again_with_either_service() {
        crate::shell::tests::sandbox();
        for service in [Service::OpenAi, Service::Claude] {
            let mut page = Browse::with(Some((service, "sk-test".into())), Effort::Light);
            page.go("docs.wpilib.org");
            assert_eq!(page.job.as_ref().map(|j| j.doing.as_str()), Some("reading docs.wpilib.org"));
            wait(&mut page);
            let first = page.page().cloned().expect("a page");
            assert_eq!(first.title, "WPILib Documentation", "{service:?}");
            assert_eq!(first.asked, "https://docs.wpilib.org");
            assert!(first.steps.iter().any(|s| s.words == "read docs.wpilib.org"), "{:?}", first.steps);
            assert!(first.links.len() >= 4);
            // Follow the second link: a fresh page asked for by its address.
            page.move_link(1);
            page.move_link(1);
            let follow = first.links[1].url.clone();
            page.go(&follow);
            wait(&mut page);
            assert_eq!(page.pages.len(), 2);
            assert_eq!(page.page().map(|p| p.asked.clone()), Some(follow));
            assert!(page.back(), "back to the first page");
            assert_eq!(page.page().map(|p| p.title.clone()), Some(first.title.clone()));
            assert!(!page.back(), "nothing before the first");
            // A question: searched, answered, its sources as links.
            page.go("how long is autonomous in 2026?");
            assert_eq!(page.job.as_ref().map(|j| j.doing.as_str()), Some("searching the web"));
            wait(&mut page);
            let answer = page.page().cloned().unwrap();
            assert_eq!(answer.title, "How long autonomous is in 2026");
            assert!(answer.links.iter().any(|l| l.url.contains("firstinspires.org")));
        }
    }

    #[test]
    fn browse_asks_for_the_web_alone_in_its_own_form() {
        let mut chat = Chat::for_mode(Service::Claude, Mode::Browse);
        chat.ask("Open https://docs.wpilib.org");
        let body = chat.body(Effort::Light);
        assert_eq!(body["system"], ai::BROWSE_SYSTEM);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.iter().map(|t| t["name"].as_str().unwrap_or_default()).collect::<Vec<_>>(), ["web_search", "web_fetch"], "no robot tools in Browse");
        assert_eq!(body["messages"][0]["content"], "Open https://docs.wpilib.org", "the address is in the question, so it may be read");
        let mut chat = Chat::for_mode(Service::OpenAi, Mode::Browse);
        chat.ask("hi");
        let body = chat.body(Effort::Light);
        assert_eq!(body["tools"], serde_json::json!([{"type": "web_search"}]));
    }
}
