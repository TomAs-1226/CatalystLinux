//! HTTP: try a web request by hand. A method, an address (`{robot}` is the
//! robot's, as in Actions), headers and a body; Send shows the status, how
//! long it took, the headers, and the body (JSON pretty-printed). The last
//! twenty requests are kept to send again.
//!
//! Sent with curl on a thread of its own, at most twenty seconds; nothing
//! is followed (a redirect is shown as one).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use bezel::{Align, Rect};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
const KEPT: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    /// "Name: value" pairs, separated by `;`.
    pub headers: String,
    pub body: String,
}

impl Default for Request {
    fn default() -> Self {
        Request { method: "GET".into(), url: "http://{robot}:9010/api/system".into(), headers: String::new(), body: String::new() }
    }
}

impl Request {
    fn to_json(&self) -> Value {
        json!({"method": self.method, "url": self.url, "headers": self.headers, "body": self.body})
    }

    fn from_json(v: &Value) -> Option<Self> {
        let s = |k: &str| v[k].as_str().map(str::to_owned);
        Some(Request { method: s("method")?, url: s("url")?, headers: s("headers").unwrap_or_default(), body: s("body").unwrap_or_default() })
    }

    /// The headers as (name, value).
    pub fn header_list(&self) -> Vec<(String, String)> {
        self.headers.split(';').filter_map(|h| h.split_once(':')).map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned())).filter(|(n, _)| !n.is_empty()).collect()
    }
}

/// What came back.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub seconds: f64,
}

/// curl's `-i` output (and `-w`'s time on the last line) read as a response:
/// any 1xx answers before the real one are passed over.
pub fn parse_response(raw: &str) -> Option<Response> {
    let (text, time) = raw.rsplit_once('\n').unwrap_or((raw, "0"));
    let seconds = time.trim().parse().unwrap_or(0.0);
    let mut rest = text;
    loop {
        let (head, body) = match rest.find("\r\n\r\n") {
            Some(i) => (&rest[..i], &rest[i + 4..]),
            None => match rest.find("\n\n") {
                Some(i) => (&rest[..i], &rest[i + 2..]),
                None => (rest, ""),
            },
        };
        let mut lines = head.lines();
        let status_line = lines.next()?.trim();
        let mut parts = status_line.splitn(3, ' ');
        let _version = parts.next()?;
        let status: u16 = parts.next()?.parse().ok()?;
        let reason = parts.next().unwrap_or("").to_owned();
        if (100..200).contains(&status) && body.starts_with("HTTP/") {
            rest = body;
            continue;
        }
        let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned())).collect();
        return Some(Response { status, reason, headers, body: body.to_owned(), seconds });
    }
}

/// A body as it reads best: JSON indented, anything else as it came.
pub fn pretty(body: &str) -> String {
    match serde_json::from_str::<Value>(body.trim()) {
        Ok(v) if !body.trim().is_empty() => serde_json::to_string_pretty(&v).unwrap_or_else(|_| body.to_owned()),
        _ => body.to_owned(),
    }
}

fn history_path() -> PathBuf {
    crate::config_dir().join("http-history.json")
}

pub fn load_history() -> Vec<Request> {
    let text = std::fs::read_to_string(history_path()).unwrap_or_default();
    serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_array().map(|a| a.iter().filter_map(Request::from_json).collect())).unwrap_or_default()
}

fn remember(req: &Request) {
    let mut all = load_history();
    all.retain(|r| r != req);
    all.insert(0, req.clone());
    all.truncate(KEPT);
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&history_path(), Value::Array(all.iter().map(Request::to_json).collect()).to_string().as_bytes()).ok();
}

/// The simulator answers the demo robot's agent itself.
fn demo(url: &str) -> Option<Response> {
    if crate::sys::on_handheld() || std::env::var_os("CATALYST_HTTP_LIVE").is_some() || !url.contains("10.58.5.2") {
        return None;
    }
    let body = crate::agent::Agent::demo().snapshot().map(|v| v.to_string()).unwrap_or_default();
    Some(Response {
        status: 200,
        reason: "OK".into(),
        headers: vec![("Content-Type".into(), "application/json".into()), ("Content-Length".into(), body.len().to_string()), ("Server".into(), "catalyst-agent/2.0.3".into())],
        body,
        seconds: 0.018,
    })
}

fn send(req: Request, url: String) -> Result<Response, String> {
    if let Some(r) = demo(&url) {
        std::thread::sleep(Duration::from_millis(300));
        return Ok(r);
    }
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sS", "-i", "--max-time", "20", "-w", "\n%{time_total}"]);
    if req.method == "HEAD" {
        cmd.arg("-I");
    } else {
        cmd.args(["-X", &req.method]);
    }
    for (n, v) in req.header_list() {
        cmd.arg("-H").arg(format!("{n}: {v}"));
    }
    let has_body = !req.body.is_empty() && req.method != "GET" && req.method != "HEAD";
    if has_body {
        cmd.args(["--data-binary", "@-"]);
        if !req.header_list().iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type")) && serde_json::from_str::<Value>(&req.body).is_ok() {
            cmd.args(["-H", "Content-Type: application/json"]);
        }
    }
    cmd.arg(&url).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|_| "curl could not start".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if has_body {
            stdin.write_all(req.body.as_bytes()).ok();
        }
    }
    let out = child.wait_with_output().map_err(|_| "the request stopped".to_owned())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.trim().trim_start_matches("curl: ").lines().next().unwrap_or("it failed").to_owned());
    }
    parse_response(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "the answer could not be read".to_owned())
}

// ---------------------------------------------------------------- screens ---

const ROWS: usize = 6;

pub struct Http {
    req: Request,
    list: ListState,
    typing: Option<(Field, usize)>,
    waiting: Option<Receiver<Result<Response, String>>>,
    clock: f32,
}

impl Http {
    pub fn new() -> Self {
        let req = load_history().into_iter().next().unwrap_or_default();
        Http { req, list: ListState::default(), typing: None, waiting: None, clock: 0.0 }
    }

    fn go(&mut self, cx: &mut Cx) {
        let robot = crate::tools::robot_address(cx.robot).or_else(|| cx.robot.is_demo().then(|| "10.58.5.2".to_owned()));
        let url = match crate::commands::expand(&self.req.url, robot.as_deref(), crate::tools::team(cx.robot)) {
            Ok(u) => u,
            Err(why) => {
                *cx.say = Some(why);
                return;
            }
        };
        let url = if url.contains("://") { url } else { format!("http://{url}") };
        remember(&self.req);
        let (tx, rx) = mpsc::channel();
        let req = self.req.clone();
        std::thread::spawn(move || {
            tx.send(send(req, url)).ok();
        });
        self.waiting = Some(rx);
    }
}

impl Screen for Http {
    fn title(&self) -> &str {
        "HTTP"
    }

    fn wants_text(&self) -> bool {
        self.typing.is_some()
    }

    fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    fn resume(&mut self) {
        if let Some(picked) = PICKED.with(|p| p.borrow_mut().take()) {
            self.req = picked;
            self.list.focus = ROWS - 2;
        }
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.typing.is_some() {
            return vec![keys("typing", &[("enter", "Done"), ("esc", "Cancel")])];
        }
        let here = match self.list.focus {
            0 => keys("method", &[("← → · enter", "Another method")]),
            4 => keys("send", &[("enter", "Send it")]),
            5 => keys("earlier", &[("enter · →", "Send one again")]),
            _ => keys("field", &[("enter · →", "Change it")]),
        };
        vec![here, keys("request", &[("ctrl s", "Send it"), ("↑ ↓", "Move")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, row)) = &mut self.typing {
            let row = *row;
            match field.key(key, |c| !c.is_control()) {
                Typed::Done(text) => {
                    match row {
                        1 => self.req.url = text,
                        2 => self.req.headers = text,
                        _ => self.req.body = text,
                    }
                    self.typing = None;
                }
                Typed::Cancelled => self.typing = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        match (key, self.list.focus) {
            (Key::Back, _) => return Nav::Pop,
            (Key::Enter | Key::Right, 0) | (Key::Left, 0) => {
                let at = METHODS.iter().position(|m| *m == self.req.method).unwrap_or(0);
                let step = if key == Key::Left { METHODS.len() - 1 } else { 1 };
                self.req.method = METHODS[(at + step) % METHODS.len()].into();
            }
            (Key::Enter | Key::Right, 1) => self.typing = Some((Field::new("address · {robot} is the robot's", &self.req.url, 300), 1)),
            (Key::Enter | Key::Right, 2) => self.typing = Some((Field::new("headers · Name: value; Name: value", &self.req.headers, 400), 2)),
            (Key::Enter | Key::Right, 3) => self.typing = Some((Field::new("body · JSON or text", &self.req.body, 2000), 3)),
            (Key::Enter, 4) | (Key::Ctrl('s'), _) => {
                if self.waiting.is_none() {
                    self.go(cx);
                }
            }
            (Key::Enter | Key::Right, 5) => return Nav::Push(Box::new(History::new())),
            (other, _) => crate::screens::list_key(&mut self.list, other, ROWS),
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.waiting.is_some()
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Some(rx) = &self.waiting else { return Nav::Stay };
        match rx.try_recv() {
            Ok(Ok(resp)) => {
                self.waiting = None;
                return Nav::Push(Box::new(ResponseView::new(resp)));
            }
            Ok(Err(why)) => {
                self.waiting = None;
                *cx.say = Some(why);
            }
            Err(TryRecvError::Disconnected) => {
                self.waiting = None;
                *cx.say = Some("the request stopped".into());
            }
            Err(TryRecvError::Empty) => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some((field, _)) = &self.typing {
            return field.draw(g, "enter: done · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let style = g.ty.label;
        for (i, y) in self.list.layout(g, ROWS, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let room = r.w - 90.0;
            let (name, value, color) = match i {
                0 => ("Method", self.req.method.clone(), t.ink),
                // http:// goes without saying; https:// is kept.
                1 => ("Address", g.text.fit(self.req.url.strip_prefix("http://").unwrap_or(&self.req.url), &style, room), t.dim),
                2 => ("Headers", if self.req.headers.is_empty() { "none".into() } else { g.text.fit(&self.req.headers, &style, room) }, t.dim),
                3 => ("Body", if self.req.body.is_empty() { "none".into() } else { g.text.fit(&self.req.body, &style, room) }, t.dim),
                4 => ("Send", if self.waiting.is_some() { "sending…".into() } else { "›".into() }, t.ink),
                _ => ("Earlier requests", format!("{}", load_history().len()), t.dim),
            };
            g.row(r, name, &value, color, i == self.list.focus);
        }
        self.list.scrollbar(g, ROWS, top, bottom);
        g.hint_any(&[match self.list.focus {
            0 => "enter: another method",
            4 => "enter: send it · ctrl s from anywhere",
            5 => "enter: send one again",
            _ => "enter: change · ctrl s: send",
        }, "ctrl s: send"]);
    }
}

thread_local! {
    /// A request picked from the history, for the request page to take up.
    static PICKED: std::cell::RefCell<Option<Request>> = const { std::cell::RefCell::new(None) };
}

/// The requests sent before, newest first.
pub struct History {
    all: Vec<Request>,
    list: ListState,
}

impl History {
    fn new() -> Self {
        History { all: load_history(), list: ListState::default() }
    }
}

impl Screen for History {
    fn title(&self) -> &str {
        "Earlier"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("earlier requests", &[("enter · →", "Use it"), ("↑ ↓", "Move")])]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right => {
                if let Some(r) = self.all.get(self.list.focus).cloned() {
                    PICKED.with(|p| *p.borrow_mut() = Some(r));
                }
                Nav::Pop
            }
            other => {
                crate::screens::list_key(&mut self.list, other, self.all.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.all.is_empty() {
            g.empty("history", "nothing sent yet");
            return;
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let all = self.all.clone();
        for (i, y) in self.list.layout(g, all.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let url = all[i].url.split("://").nth(1).unwrap_or(&all[i].url).to_owned();
            let url = g.text.fit(&url, &g.ty.label, r.w - 80.0);
            g.row(r, &all[i].method, &url, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, all.len(), top, bottom);
        g.hint("enter: use it");
    }
}

/// A response: its status and timing, then its body or its headers.
pub struct ResponseView {
    resp: Response,
    lines: Option<(f32, Vec<String>)>,
    headers: bool,
    scroll: Animated,
    target: f32,
    max: f32,
}

impl ResponseView {
    fn new(resp: Response) -> Self {
        ResponseView { resp, lines: None, headers: false, scroll: Animated::new(0.0, role::hold()).with_rest(0.1), target: 0.0, max: 0.0 }
    }

    fn text(&self) -> String {
        if self.headers {
            self.resp.headers.iter().map(|(n, v)| format!("{n}: {v}")).collect::<Vec<_>>().join("\n")
        } else if self.resp.body.is_empty() {
            "(no body)".into()
        } else {
            pretty(&self.resp.body)
        }
    }

    fn scroll_by(&mut self, by: f32) {
        self.target = (self.target + by).clamp(0.0, self.max);
        self.scroll.set(self.target);
    }
}

/// Text cut into lines of at most `columns` characters (mono), long lines
/// carried on to the next.
pub fn hard_wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(8);
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            out.push(String::new());
        }
        for chunk in chars.chunks(columns) {
            out.push(chunk.iter().collect());
        }
    }
    out
}

impl Screen for ResponseView {
    fn title(&self) -> &str {
        "Response"
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        let rows: &'static [(&'static str, &'static str)] = if self.headers {
            &[("↑ ↓", "Scroll"), ("← →", "A page at a time"), ("tab · h", "The body"), ("home · end", "Top or end")]
        } else {
            &[("↑ ↓", "Scroll"), ("← →", "A page at a time"), ("tab · h", "The headers"), ("home · end", "Top or end")]
        };
        vec![keys("response", rows)]
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let line = 15.0;
        match key {
            Key::Back => return Nav::Pop,
            Key::Tab | Key::Char('h') => {
                self.headers = !self.headers;
                self.lines = None;
                self.target = 0.0;
                self.scroll.jump(0.0);
            }
            Key::Up => self.scroll_by(-line * 2.0),
            Key::Down => self.scroll_by(line * 2.0),
            Key::PageUp | Key::Left => self.scroll_by(-line * 6.0),
            Key::PageDown | Key::Right => self.scroll_by(line * 6.0),
            Key::Home => self.scroll_by(-f32::MAX),
            Key::End => self.scroll_by(f32::MAX),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.scroll.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        if g.calm {
            self.scroll.jump(self.target);
        }
        // The status, coloured by what it means, the time and the size.
        let r = &self.resp;
        let ink = match r.status {
            200..=299 => g.good(),
            300..=399 => t.dim,
            400..=499 => t.amber_ink,
            _ => g.fault(),
        };
        let cy = top + ty.name.size * 0.5;
        let w = g.text_at(PAD + 2.0, cy, Align::Left, &r.status.to_string(), ty.name, ink);
        let reason = g.text.fit(&r.reason, &ty.body, W * 0.45);
        g.text_at(PAD + 8.0 + w, cy, Align::Left, &reason, ty.body, t.ink);
        let meta = format!("{:.0} ms · {}", r.seconds * 1000.0, crate::tools::bytes(r.body.len() as f64));
        g.text_at(W - PAD - 2.0, cy, Align::Right, &meta, ty.caption, t.dim);
        // The body, or the headers, in mono, scrolled.
        let style = ty.caption;
        let area = Rect::new(PAD, top + ty.name.size + 6.0, W - 2.0 * PAD, g.above_hint() - (top + ty.name.size + 6.0));
        g.surface(area, 10.0, t.surface1);
        let char_w = g.measure("0000000000", style) / 10.0;
        let columns = ((area.w - 16.0) / char_w).floor() as usize;
        let lines = match &self.lines {
            Some((w, lines)) if *w == area.w => lines.clone(),
            _ => {
                let lines = hard_wrap(&self.text(), columns);
                self.lines = Some((area.w, lines.clone()));
                lines
            }
        };
        let line_h = style.size + 4.0;
        self.max = (lines.len() as f32 * line_h - (area.h - 10.0)).max(0.0);
        let scroll = self.scroll.value().clamp(0.0, self.max.max(0.0));
        g.canvas.set_clip(Rect::new(area.x + g.ox, area.y + 3.0, area.w, area.h - 6.0));
        let first = (scroll / line_h).floor() as usize;
        for (k, line) in lines.iter().enumerate().skip(first) {
            let y = area.y + 5.0 + k as f32 * line_h - scroll;
            if y > area.bottom() {
                break;
            }
            g.text_at(area.x + 8.0, y + line_h * 0.5, Align::Left, line, style, if self.headers { t.dim } else { t.ink });
        }
        g.canvas.clear_clip();
        g.hint(if self.headers { "tab: the body · ↑↓: scroll" } else { "tab: the headers · ↑↓: scroll" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curls_answer_reads_as_a_response() {
        let raw = "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Thing: a: b\r\n\r\n{\"ok\":true}\n0.042317";
        let r = parse_response(raw).unwrap();
        assert_eq!((r.status, r.reason.as_str()), (201, "Created"));
        assert_eq!(r.headers, [("Content-Type".to_owned(), "application/json".to_owned()), ("X-Thing".to_owned(), "a: b".to_owned())]);
        assert_eq!(r.body, "{\"ok\":true}");
        assert!((r.seconds - 0.042317).abs() < 1e-9);
        let head = parse_response("HTTP/2 404 \r\nserver: x\r\n\r\n\n0.1").unwrap();
        assert_eq!(head.status, 404);
        assert!(parse_response("garbage").is_none());
    }

    #[test]
    fn json_is_indented_and_text_left_alone() {
        assert_eq!(pretty("{\"a\":[1,2]}"), "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(pretty("plain words"), "plain words");
        assert_eq!(hard_wrap("abcdefghijkl\n\nxy", 8), ["abcdefgh", "ijkl", "", "xy"]);
    }

    #[test]
    fn headers_are_typed_on_one_line() {
        let req = Request { headers: "Accept: application/json; X-Team: 5805 ;bad".into(), ..Request::default() };
        assert_eq!(req.header_list(), [("Accept".to_owned(), "application/json".to_owned()), ("X-Team".to_owned(), "5805".to_owned())]);
        let back = Request::from_json(&req.to_json()).unwrap();
        assert_eq!(back, req);
    }
}
