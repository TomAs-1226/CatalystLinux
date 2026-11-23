//! Team's pages: the thread (with the next match and who is there along
//! its top), writing, the members, the invitation (QR code and six-digit
//! code), making and joining a team, and the calls on the number keys.

use std::sync::mpsc::Receiver;

use bezel::{Align, Rect};

use super::crypto::TeamKey;
use super::wire::{Body, MemberId, ShareKind, TEXT_MOST};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

// ------------------------------------------------------------ snapshot ---

/// A message as the thread shows it.
#[derive(Clone)]
struct Row {
    by: String,
    mine: bool,
    words: String,
    urgent: bool,
    wall: i64,
    share: Option<(ShareKind, MemberId, u32)>,
    /// Of this handheld's own: how many of the others have read it.
    seen_by: usize,
    /// Of its own: still being resent to someone here.
    sending: bool,
}

/// One member, as the pages show them.
#[derive(Clone)]
struct Member {
    name: String,
    me: bool,
    here: bool,
    /// Read to the newest message.
    seen: bool,
    /// Has the newest message.
    got: bool,
}

/// What the pages need of the engine, copied out under its lock.
#[derive(Clone, Default)]
struct Snapshot {
    team: String,
    rows: Vec<Row>,
    members: Vec<Member>,
}

fn snapshot() -> Option<Snapshot> {
    let now = super::now_ms();
    let team = super::team_name()?;
    super::with(|e| {
        let newest = e.log.last().map(|x| (x.m.lamport, x.m.id));
        let mut rows: Vec<Row> = Vec::new();
        for x in &e.log {
            let share = match &x.m.body {
                Body::Share { kind, set, part, .. } => {
                    // A share's parts are one row: its first.
                    if *part != 1 {
                        continue;
                    }
                    Some((*kind, x.m.id.origin, *set))
                }
                _ => None,
            };
            let mine = x.m.id.origin == e.me;
            let seen_by = if mine { e.peers.values().filter(|p| p.seen >= x.m.lamport).count() } else { 0 };
            let sending = mine && e.is_sending(x.m.id);
            rows.push(Row { by: x.m.by.clone(), mine, words: x.m.words(), urgent: x.m.urgent, wall: x.m.wall, share, seen_by, sending });
        }
        let got = |id: MemberId| newest.is_none_or(|(_, nid)| e.log.last().is_some_and(|x| x.m.id == nid && x.got.contains(&id)));
        let mut members = vec![Member { name: e.name.clone(), me: true, here: true, seen: true, got: true }];
        for (id, p) in &e.peers {
            let seen = newest.is_none_or(|(l, _)| p.seen >= l);
            members.push(Member { name: if p.name.is_empty() { "someone".into() } else { p.name.clone() }, me: false, here: p.here(now), seen, got: seen || got(*id) });
        }
        // Here first, then by name.
        members[1..].sort_by(|a, b| b.here.cmp(&a.here).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Snapshot { team, rows, members }
    })
}

fn clock(wall: i64) -> String {
    chrono::DateTime::from_timestamp(wall, 0).map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()
}

/// The next match, along the top of the thread: its label, when to queue
/// (or when it is), and its alliance's colour.
fn next_match(robot: &Robot) -> Option<(String, String, Option<bezel::Color>, bool)> {
    let view = robot.schedule.view()?;
    let now = chrono::Utc::now().timestamp();
    let m = view.next(now)?;
    let queuing = m.status.as_deref() == Some("Now queuing");
    let when = match (m.queue_at, m.time) {
        _ if queuing => "queue now".to_owned(),
        (Some(q), _) => format!("queue {}", crate::schedule::when(q, now)),
        (None, Some(t)) => crate::schedule::when(t, now),
        _ => String::new(),
    };
    let side = match m.alliance(view.team) {
        Some("red") => Some(crate::schedule::RED),
        Some("blue") => Some(crate::schedule::BLUE),
        _ => None,
    };
    Some((m.label.clone(), when, side, queuing))
}

/// Read to the newest: a dot filled with ice, edged with the ink that goes
/// on ice so it holds on the light tone's pale surfaces too.
fn seen_dot(g: &mut Gfx, cx: f32, cy: f32, r: f32) {
    let t = g.tokens;
    g.dot(cx, cy, r, t.ice);
    if t.tone == bezel::Tone::Light {
        g.canvas.stroke_rrect(Rect::new(cx - r + g.ox, cy - r, 2.0 * r, 2.0 * r), r, 1.0, t.on_ice, 0.55);
    }
}

/// The strip along the top: the next match on the left, the members as
/// dots on the right (filled ice: here and read to the newest; ink: here;
/// faint: away). Its foot.
fn strip(g: &mut Gfx, robot: &Robot, members: &[Member]) -> f32 {
    let t = g.tokens;
    let ty = g.ty;
    let r = Rect::new(PAD, g.top(), W - 2.0 * PAD, ty.body.size + 12.0);
    let radius = g.radius(7.0).min(r.h * 0.5);
    g.surface(r, radius, t.surface1);
    // The members, right to left.
    let step = 9.0;
    let dots_w = members.len().min(10) as f32 * step;
    let mut x = r.right() - 10.0 - dots_w + step * 0.5;
    for m in members.iter().take(10) {
        match (m.here, m.seen) {
            (true, true) => seen_dot(g, x, r.cy(), 3.2),
            (true, false) => {
                g.canvas.stroke_rrect(Rect::new(x - 3.2 + g.ox, r.cy() - 3.2, 6.4, 6.4), 3.2, 1.3, t.ink, 1.0);
            }
            _ => g.dot(x, r.cy(), 2.2, t.faint),
        }
        x += step;
    }
    let here = members.iter().filter(|m| m.here).count();
    let count = format!("{here} here");
    let count_w = g.measure(&count, ty.caption);
    let count_x = r.right() - 16.0 - dots_w - count_w;
    g.text_at(count_x, r.cy(), Align::Left, &count, ty.caption, t.dim);
    // The match.
    let room = count_x - r.x - 16.0;
    match next_match(robot) {
        Some((label, when, side, queuing)) => {
            let mut x = r.x + 10.0;
            if let Some(c) = side {
                g.dot(x + 3.0, r.cy(), 3.5, c);
                x += 11.0;
            }
            let w = g.text_at(x, r.cy(), Align::Left, &label, ty.body, t.ink);
            let color = if queuing { t.amber_ink } else { t.dim };
            g.text_fit(x + w + 6.0, r.cy(), &when, ty.label, color, room - (x - r.x) - w - 6.0);
        }
        None => {
            g.text_fit(r.x + 10.0, r.cy(), "no match coming", ty.label, t.dim, room);
        }
    }
    r.bottom()
}

// ---------------------------------------------------------------- team ---

#[derive(Clone, Copy, PartialEq)]
enum Start {
    Create,
    Code,
    Scan,
}

const STARTS: [Start; 3] = [Start::Create, Start::Code, Start::Scan];

/// Team, opened from Home: the thread, or (in no team yet) how to start
/// one or join one.
pub struct TeamScreen {
    list: ListState,
    snap: Option<Snapshot>,
    generation: u64,
    /// The focus follows the newest message until moved off it.
    at_end: bool,
}

impl Default for TeamScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamScreen {
    pub fn new() -> TeamScreen {
        let mut s = TeamScreen { list: ListState::default(), snap: None, generation: u64::MAX, at_end: true };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        self.generation = super::generation();
        self.snap = snapshot();
        let n = self.snap.as_ref().map_or(0, |s| s.rows.len());
        if self.snap.is_some() && (self.at_end || self.list.focus >= n) {
            self.list.focus = n.saturating_sub(1);
        }
    }

    fn call(&self, k: usize, cx: &mut Cx) {
        let calls = super::calls();
        let Some((words, urgent)) = calls.get(k) else { return };
        *cx.say = Some(match super::send(Body::Call(words.clone()), *urgent) {
            Ok(()) => format!("sent · {words}"),
            Err(why) => why,
        });
    }

    fn open_share(&self, kind: ShareKind, origin: MemberId, set: u32, cx: &mut Cx) -> Nav {
        let texts = super::with(|e| super::share_texts(e, origin, set)).flatten();
        let Some(texts) = texts else {
            *cx.say = Some("still coming in".into());
            return Nav::Stay;
        };
        match kind {
            ShareKind::Strategy => {
                let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
                match crate::strategy::open_team_board(&texts) {
                    Ok(screen) => Nav::Push(screen),
                    Err(why) => {
                        *cx.say = Some(why);
                        Nav::Stay
                    }
                }
            }
            ShareKind::Scout => Nav::Push(Box::new(crate::scout::Scout::new())),
        }
    }
}

impl Screen for TeamScreen {
    fn title(&self) -> &str {
        "Team"
    }

    fn resume(&mut self) {
        self.refresh();
    }

    fn appear(&mut self) {
        super::read_all();
    }

    /// The thread changes from the network: looked in on four times a second.
    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        if super::generation() != self.generation {
            self.refresh();
            return true;
        }
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let Some(snap) = self.snap.clone() else {
            // No team yet: start one, or join one.
            match key {
                Key::Back => return Nav::Pop,
                Key::Enter => {
                    return match STARTS[self.list.focus.min(STARTS.len() - 1)] {
                        Start::Create => Nav::Push(Box::new(CreatePage::new())),
                        Start::Code => Nav::Push(Box::new(JoinPage::by_code())),
                        Start::Scan => Nav::Push(Box::new(crate::camera::Camera::scanning())),
                    }
                }
                other => crate::screens::list_key(&mut self.list, other, STARTS.len()),
            }
            return Nav::Stay;
        };
        let n = snap.rows.len();
        match key {
            Key::Back => return Nav::Pop,
            Key::Tab => return Nav::Push(Box::new(Members::new())),
            Key::Char(c @ '1'..='6') => self.call(c as usize - '1' as usize, cx),
            Key::Enter => {
                // On a share: open it. Anywhere else: write.
                if let Some((kind, origin, set)) = snap.rows.get(self.list.focus).and_then(|r| r.share) {
                    return self.open_share(kind, origin, set, cx);
                }
                return Nav::Push(Box::new(Compose::new(None)));
            }
            Key::Char(c) if !c.is_control() && c != ' ' => return Nav::Push(Box::new(Compose::new(Some(c)))),
            other => {
                crate::screens::list_key(&mut self.list, other, n);
                self.at_end = self.list.focus + 1 >= n;
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let Some(snap) = self.snap.clone() else {
            return draw_start(g, &mut self.list);
        };
        super::read_all();
        let top = strip(g, robot, &snap.members) + 5.0;
        let bottom = g.above_hint();
        if snap.rows.is_empty() {
            let cy = (top + bottom) * 0.5;
            g.icon("forum", false, W * 0.5, cy - 10.0, 22.0, t.faint);
            let line = g.text.fit("a number sends its call · type to write", &ty.caption, W - 2.0 * PAD);
            g.text_at(W * 0.5, cy + 12.0, Align::Center, &line, ty.caption, t.dim);
        } else {
            let row_h = g.row_h();
            let len = snap.rows.len();
            for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
                message_row(g, Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h), &snap.rows[i], i == self.list.focus);
            }
            self.list.scrollbar(g, len, top, bottom);
        }
        let focused_share = snap.rows.get(self.list.focus).is_some_and(|r| r.share.is_some());
        match super::trouble() {
            Some(why) => g.hint(&why),
            None if focused_share => g.hint_any(&["enter: open it · 1–6: calls · tab: members", "enter: open · tab: members", "enter: open"]),
            None => g.hint_any(&["1–6: calls · type to write · tab: members", "1–6: calls · type · tab: members", "1–6 · type · tab"]),
        }
    }
}

fn message_row(g: &mut Gfx, r: Rect, row: &Row, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    let radius = g.radius(7.0).min(r.h * 0.5);
    g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
    if focused {
        g.focus_ring(r, radius);
    }
    let mut x = r.x + 8.0;
    // An urgent call: a bar of the signal (no words sit on it).
    if row.urgent {
        g.surface(Rect::new(x, r.y + 5.0, 3.0, r.h - 10.0), 1.5, t.signal);
        x += 8.0;
    }
    let who = if row.mine { "you".to_owned() } else { row.by.to_lowercase() };
    let who = g.text.fit(&who, &ty.label, 58.0);
    let who_w = g.text_at(x, r.cy(), Align::Left, &who, ty.label, t.dim);
    x += who_w.max(30.0) + 8.0;
    // On the right: when, or (for this handheld's own) who has read it.
    let right = match (row.mine, row.seen_by) {
        (true, n) if n > 0 => format!("seen {n}"),
        (true, _) if row.sending => "sending".to_owned(),
        _ => clock(row.wall),
    };
    let right_w = g.text_at(r.right() - 8.0, r.cy(), Align::Right, &right, ty.caption, t.dim);
    let room = r.right() - 14.0 - right_w - x;
    if let Some((kind, ..)) = row.share {
        g.icon(if kind == ShareKind::Strategy { "strategy" } else { "fact_check" }, false, x + 6.0, r.cy(), 13.0, t.ink);
        x += 16.0;
    }
    g.text_fit(x, r.cy(), &row.words, ty.body, t.ink, room - if row.share.is_some() { 16.0 } else { 0.0 });
}

fn draw_start(g: &mut Gfx, list: &mut ListState) {
    let t = g.tokens;
    let ty = g.ty;
    let top = g.top();
    // What Team is, above the three ways in: as much of it, whole, as leaves
    // the three rows whole.
    let line_h = ty.caption.size + 3.0;
    let row_h = g.row_h();
    let bottom = g.above_hint();
    let room = ((bottom - top - 8.0 - STARTS.len() as f32 * (row_h + 3.0)) / line_h).floor().max(0.0) as usize;
    let mut lines = Vec::new();
    for words in ["messages between handhelds at the event, sealed with the team's own key", "messages between the team's handhelds, sealed"] {
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
    for (i, y) in list.layout(g, STARTS.len(), rows_top, row_h, 3.0, bottom) {
        let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
        let (name, value) = match STARTS[i] {
            Start::Create => ("Start a team space", "new key"),
            Start::Code => ("Join with a code", "6 digits"),
            Start::Scan => ("Join by its QR code", "camera"),
        };
        g.row(r, name, value, t.dim, i == list.focus);
    }
    list.scrollbar(g, STARTS.len(), rows_top, bottom);
    g.hint_any(&["enter: go · a member's Invite shows the code", "enter: go"]);
}

// ------------------------------------------------------------- compose ---

/// Writing to the team: the words, and the calls (a number alone sends its
/// call).
pub struct Compose {
    field: Field,
    done: bool,
}

impl Compose {
    fn new(first: Option<char>) -> Compose {
        let mut field = Field::new("to the team", "", TEXT_MOST);
        if let Some(c) = first {
            field.text.push(c);
        }
        Compose { field, done: false }
    }
}

impl Screen for Compose {
    fn title(&self) -> &str {
        "Write"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        // A number on its own sends its call, as on the thread.
        if let (Key::Char(c @ '1'..='6'), true) = (key, self.field.text.is_empty()) {
            let calls = super::calls();
            let (words, urgent) = calls[c as usize - '1' as usize].clone();
            *cx.say = Some(match super::send(Body::Call(words.clone()), urgent) {
                Ok(()) => format!("sent · {words}"),
                Err(why) => why,
            });
            return Nav::Pop;
        }
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) => {
                let text = super::wire::clean_text(&text);
                if text.is_empty() {
                    return Nav::Pop;
                }
                if let Err(why) = super::send(Body::Text(text), false) {
                    *cx.say = Some(why);
                }
                Nav::Pop
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.field.draw_at(g, true);
        // The calls, two across.
        let top = g.top() + Field::height(g) + 6.0;
        let bottom = g.above_hint();
        let calls = super::calls();
        let rows = 3.0;
        let gap = 4.0;
        let h = ((bottom - top - gap * (rows - 1.0)) / rows).min(ty.body.size + 12.0);
        let w = (W - 2.0 * PAD - gap) * 0.5;
        let radius = g.radius(7.0).min(h * 0.5);
        for (k, (words, urgent)) in calls.iter().enumerate() {
            let (col, row) = ((k / 3) as f32, (k % 3) as f32);
            let r = Rect::new(PAD + col * (w + gap), top + row * (h + gap), w, h);
            g.surface(r, radius, t.surface1);
            let mut x = r.x + 8.0;
            let n = format!("{}", k + 1);
            let nw = g.text_at(x, r.cy(), Align::Left, &n, ty.label, t.dim);
            x += nw + 7.0;
            if *urgent {
                g.surface(Rect::new(x, r.y + 5.0, 3.0, r.h - 10.0), 1.5, t.signal);
                x += 8.0;
            }
            g.text_fit(x, r.cy(), words, ty.body, t.ink, r.right() - 6.0 - x);
        }
        if self.field.text.is_empty() {
            g.hint_any(&["a number sends its call · enter: send · esc: back", "a number sends its call · enter: send", "number: call · enter: send"]);
        } else {
            g.hint_any(&["enter: send to the team · esc: back", "enter: send"]);
        }
    }
}

// ------------------------------------------------------------- members ---

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Invite,
    Calls,
    Leave,
}

const ACTIONS: [Action; 3] = [Action::Invite, Action::Calls, Action::Leave];

/// Who is in the team, who is here and who has read the newest message;
/// and inviting, the calls, leaving.
pub struct Members {
    list: ListState,
    snap: Snapshot,
    generation: u64,
    leaving: bool,
    done: bool,
}

impl Members {
    fn new() -> Members {
        Members { list: ListState::default(), snap: snapshot().unwrap_or_default(), generation: super::generation(), leaving: false, done: false }
    }

    fn len(&self) -> usize {
        self.snap.members.len() + ACTIONS.len()
    }
}

impl Screen for Members {
    fn title(&self) -> &str {
        "Members"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn resume(&mut self) {
        self.snap = snapshot().unwrap_or_default();
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        if super::generation() != self.generation {
            self.generation = super::generation();
            self.snap = snapshot().unwrap_or_default();
            return true;
        }
        false
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let action = self.list.focus.checked_sub(self.snap.members.len()).and_then(|k| ACTIONS.get(k).copied());
        if action != Some(Action::Leave) {
            self.leaving = false;
        }
        match key {
            Key::Back | Key::Tab => return Nav::Pop,
            Key::Enter => match action {
                Some(Action::Invite) => return Nav::Push(Box::new(InvitePage::new())),
                Some(Action::Calls) => return Nav::Push(Box::new(CallsPage::new())),
                Some(Action::Leave) if self.leaving => {
                    super::leave();
                    *cx.say = Some(format!("left {}", self.snap.team));
                    self.done = true;
                }
                Some(Action::Leave) => {
                    self.leaving = true;
                    *cx.say = Some("enter again to leave · the key goes from here".into());
                }
                None => {}
            },
            other => {
                let n = self.len();
                crate::screens::list_key(&mut self.list, other, n);
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let bottom = g.above_hint();
        let row_h = g.row_h();
        let members = self.snap.members.len();
        let n = self.len();
        for (i, y) in self.list.layout(g, n, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            if let Some(m) = self.snap.members.get(i) {
                let value = match (m.me, m.here, m.seen, m.got) {
                    (true, ..) => "you".to_owned(),
                    (_, true, true, _) => "here · seen".into(),
                    (_, true, false, true) => "here · got it".into(),
                    (_, true, false, false) => "here".into(),
                    (_, false, true, _) => "away · seen".into(),
                    _ => "away".into(),
                };
                g.row(r, &m.name, &value, if m.here { t.ink } else { t.dim }, focused);
                // Read to the newest: a dot of ice before the words, as along
                // the thread's top (the row's value is in Sans in Bezel, mono
                // in Catalyst's look).
                if m.seen && !m.me {
                    let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
                    let w = g.measure(&value, style);
                    seen_dot(g, r.right() - 18.0 - w, r.cy(), 3.0);
                }
            } else {
                let (name, value) = match ACTIONS[i - members] {
                    Action::Invite => ("Invite someone".to_owned(), "qr · code".to_owned()),
                    Action::Calls => ("Calls on 1–6".to_owned(), "edit".to_owned()),
                    Action::Leave => (format!("Leave {}", self.snap.team), if self.leaving { "enter again".into() } else { String::new() }),
                };
                g.row(r, &name, &value, if self.leaving { t.amber_ink } else { t.dim }, focused);
            }
        }
        self.list.scrollbar(g, n, top, bottom);
        let here = self.snap.members.iter().filter(|m| m.here).count();
        let seen = self.snap.members.iter().filter(|m| m.seen && !m.me).count();
        let line = format!("{here} of {} here · {seen} read the newest", self.snap.members.len());
        g.hint_any(&[&format!("{line} · tab: thread"), &line, "enter: go · tab: thread"]);
    }
}

// -------------------------------------------------------------- invite ---

/// The team's QR code and a six-digit code, for someone to join with.
pub struct InvitePage {
    qr: Option<qrcodegen::QrCode>,
    team: String,
    code: (String, u64),
    since: std::time::Instant,
}

impl InvitePage {
    pub fn new() -> InvitePage {
        let m = super::Membership::load();
        let team = super::team_name().unwrap_or_default();
        let qr = m.and_then(|m| qrcodegen::QrCode::encode_text(&m.qr_text(), qrcodegen::QrCodeEcc::Medium).ok());
        InvitePage { qr, team, code: super::invite_code().unwrap_or_default(), since: std::time::Instant::now() }
    }

    /// For the screenshots: a team's code shown as it would be.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn shown(qr_text: &str, team: &str, code: &str, left: u64) -> InvitePage {
        let qr = qrcodegen::QrCode::encode_text(qr_text, qrcodegen::QrCodeEcc::Medium).ok();
        InvitePage { qr, team: team.to_owned(), code: (code.to_owned(), left), since: std::time::Instant::now() }
    }
}

impl Drop for InvitePage {
    fn drop(&mut self) {
        super::invite_closed();
    }
}

impl Screen for InvitePage {
    fn title(&self) -> &str {
        "Invite"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    /// The code's time left, once a second.
    fn ambient(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(1))
    }

    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        if self.since.elapsed().as_secs() >= 1 && super::member() {
            self.since = std::time::Instant::now();
            if let Some(code) = super::invite_code() {
                self.code = code;
            }
            return true;
        }
        false
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Enter => Nav::Pop,
            _ => Nav::Stay,
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let side = H - top - 4.0;
        let Some(qr) = self.qr.clone() else {
            g.empty("qr_code_2", "not in a team");
            return;
        };
        let code = crate::share::draw_qr(g, &qr, Rect::new(PAD, top, side, side));
        let x = code.right() + 12.0;
        let width = W - PAD - x;
        let mut y = code.y + ty.name.size * 0.5;
        g.text_fit(x, y, &self.team, ty.name, t.ink, width);
        y += ty.name.size * 0.5 + 8.0 + ty.caption.size * 0.5;
        g.text_at(x, y, Align::Left, "or type this code", ty.caption, t.dim);
        let big = ty.display(28.0);
        let (digits, left) = &self.code;
        let shown = if digits.len() == 6 { format!("{} {}", &digits[..3], &digits[3..]) } else { digits.clone() };
        y += ty.caption.size * 0.5 + 4.0 + big.size * 0.5;
        g.text_fit(x, y, &shown, big, t.ink, width);
        y += big.size * 0.5 + 6.0;
        let life = (*left as f32 / (super::join::LIFETIME / 1000) as f32).clamp(0.0, 1.0);
        g.meter(Rect::new(x, y, width, 4.0), life, t.ice);
        y += 4.0 + 6.0 + ty.caption.size * 0.5;
        g.text_at(x, y, Align::Left, &format!("good for {}:{:02}", left / 60, left % 60), ty.caption, t.dim);
        // Along the foot, in what room is left: who should see it, whole.
        let line_h = ty.caption.size + 3.0;
        let free = code.bottom() - 2.0 - (y + ty.caption.size * 0.5 + 6.0);
        for words in ["show it only to your team: it holds the key", "only for your team's eyes"] {
            let lines = crate::help::wrap(g, words, ty.caption, width);
            if lines.len() as f32 * line_h <= free {
                let foot = code.bottom() - 2.0 - ty.caption.size * 0.5;
                let k = lines.len();
                for (i, l) in lines.iter().enumerate() {
                    g.text_at(x, foot - (k - 1 - i) as f32 * line_h, Align::Left, l, ty.caption, t.dim);
                }
                break;
            }
        }
    }
}

// -------------------------------------------------------------- create ---

/// A new team space: its name, then this handheld's.
pub struct CreatePage {
    field: Field,
    team: Option<String>,
    done: bool,
}

impl Default for CreatePage {
    fn default() -> Self {
        Self::new()
    }
}

impl CreatePage {
    pub fn new() -> CreatePage {
        CreatePage { field: Field::new("the team space's name", "5805 pit", super::wire::NAME_MOST), team: None, done: false }
    }
}

impl Screen for CreatePage {
    fn title(&self) -> &str {
        "New team"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled if self.team.is_some() => {
                let team = self.team.take().unwrap_or_default();
                self.field = Field::new("the team space's name", &team, super::wire::NAME_MOST);
                Nav::Stay
            }
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) if text.trim().is_empty() => Nav::Stay,
            Typed::Done(text) => match self.team.take() {
                None => {
                    self.team = Some(text);
                    self.field = Field::new("your name, as the team sees it", &crate::device::name(), super::wire::NAME_MOST);
                    Nav::Stay
                }
                Some(team) => match super::create(&team, &text) {
                    Ok(()) => {
                        self.done = true;
                        Nav::Push(Box::new(InvitePage::new()))
                    }
                    Err(why) => {
                        *cx.say = Some(why);
                        self.team = Some(team);
                        Nav::Stay
                    }
                },
            },
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        self.field.draw_at(g, true);
        let words = match &self.team {
            None => "a new 256-bit key is made here: everything said in the team is sealed with it".to_owned(),
            Some(team) => format!("{team} · its key is made when you go on, and its QR code shown to invite the others"),
        };
        explain(g, &[&words, "a new key is made here, and its QR code shown to invite the others"]);
        g.hint_any(&["enter: next · esc: back", "enter: next"]);
    }
}

/// A few lines under a field, saying what it is for: the first of
/// `choices` (longest first) that fits whole above the hint.
fn explain(g: &mut Gfx, choices: &[&str]) {
    let t = g.tokens;
    let ty = g.ty;
    let y = g.top() + Field::height(g) + 10.0;
    let line_h = ty.caption.size + 4.0;
    let room = ((g.above_hint() - y) / line_h).floor().max(1.0) as usize;
    let mut lines = Vec::new();
    for words in choices {
        lines = crate::help::wrap(g, words, ty.caption, W - 2.0 * PAD - 8.0);
        if lines.len() <= room {
            break;
        }
    }
    for (k, l) in lines.iter().take(room).enumerate() {
        g.text_at(PAD + 4.0, y + line_h * (k as f32 + 0.5), Align::Left, l, ty.caption, t.dim);
    }
}

// ---------------------------------------------------------------- join ---

enum Step {
    Code,
    Name,
    Asking(Receiver<Result<String, String>>),
}

/// Joining: with the six digits another member's Invite shows, or from its
/// QR code read in the Camera. Then this handheld's name.
pub struct JoinPage {
    step: Step,
    field: Field,
    /// From a QR code: the key and the team's name.
    scanned: Option<(TeamKey, String)>,
    code: String,
    done: bool,
    dots: f32,
}

impl JoinPage {
    pub fn by_code() -> JoinPage {
        JoinPage { step: Step::Code, field: code_field(""), scanned: None, code: String::new(), done: false, dots: 0.0 }
    }

    pub fn scanned(key: TeamKey, team: String) -> JoinPage {
        JoinPage { step: Step::Name, field: name_field(), scanned: Some((key, team)), code: String::new(), done: false, dots: 0.0 }
    }

    /// For the screenshots: asking with a code, as it looks while it waits.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn waiting(code: &str) -> JoinPage {
        let (_tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(_tx);
        JoinPage { step: Step::Asking(rx), field: name_field(), scanned: None, code: code.to_owned(), done: false, dots: 0.0 }
    }
}

fn code_field(text: &str) -> Field {
    Field::new("the six digits Invite shows", text, 7)
}

fn name_field() -> Field {
    Field::new("your name, as the team sees it", &crate::device::name(), super::wire::NAME_MOST)
}

impl Screen for JoinPage {
    fn title(&self) -> &str {
        "Join"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn wants_text(&self) -> bool {
        !matches!(self.step, Step::Asking(_))
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn tick(&mut self, dt: f32) -> bool {
        if matches!(self.step, Step::Asking(_)) {
            self.dots += dt;
            return true;
        }
        false
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        let Step::Asking(rx) = &self.step else { return Nav::Stay };
        match rx.try_recv() {
            Ok(Ok(team)) => {
                *cx.say = Some(format!("in {team}"));
                self.done = true;
                Nav::Push(Box::new(TeamScreen::new()))
            }
            Ok(Err(why)) => {
                *cx.say = Some(why);
                self.step = Step::Code;
                self.field = code_field(&self.code);
                Nav::Stay
            }
            Err(_) => Nav::Stay,
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if matches!(self.step, Step::Asking(_)) {
            return if key == Key::Back { Nav::Pop } else { Nav::Stay };
        }
        let digits = matches!(self.step, Step::Code);
        match self.field.key(key, |c| if digits { c.is_ascii_digit() || c == ' ' || c == '-' } else { !c.is_control() }) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) => match self.step {
                Step::Code => match super::join::clean_code(&text) {
                    Some(code) => {
                        self.code = code;
                        self.step = Step::Name;
                        self.field = name_field();
                        Nav::Stay
                    }
                    None => {
                        *cx.say = Some("six digits, as Invite shows them".into());
                        Nav::Stay
                    }
                },
                Step::Name if text.trim().is_empty() => Nav::Stay,
                Step::Name => match self.scanned.clone() {
                    Some((key, team)) => match super::join_team(key, &team, &text) {
                        Ok(()) => {
                            *cx.say = Some(format!("in {team}"));
                            self.done = true;
                            Nav::Push(Box::new(TeamScreen::new()))
                        }
                        Err(why) => {
                            *cx.say = Some(why);
                            Nav::Stay
                        }
                    },
                    None => {
                        self.step = Step::Asking(super::join_by_code(self.code.clone(), text));
                        self.dots = 0.0;
                        Nav::Stay
                    }
                },
                Step::Asking(_) => Nav::Stay,
            },
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        match &self.step {
            Step::Asking(_) => {
                let cy = (g.top() + g.above_hint()) * 0.5;
                let shown = format!("{} {}", &self.code[..3.min(self.code.len())], self.code.get(3..).unwrap_or(""));
                let big = ty.display(28.0);
                g.text_at(W * 0.5, cy - 10.0, Align::Center, &shown, big, t.ink);
                // Three dots, one lit in turn: asking.
                for k in 0..3 {
                    let lit = ((self.dots * 3.0) as usize % 3) == k;
                    g.dot(W * 0.5 - 10.0 + k as f32 * 10.0, cy + big.size * 0.5 + 6.0, 2.5, if lit { t.ink } else { t.faint });
                }
                g.text_at(W * 0.5, cy + big.size * 0.5 + 22.0, Align::Center, "asking the handhelds nearby", ty.caption, t.dim);
                g.hint("esc: stop");
            }
            Step::Code => {
                self.field.draw_at(g, true);
                explain(
                    g,
                    &[
                        "on a member's handheld: Team › tab › Invite someone. The code never crosses the network: both ends prove they have it",
                        "on a member's handheld: Team › tab › Invite someone. The code never crosses the network",
                        "on a member's handheld: Team › tab › Invite someone",
                    ],
                );
                g.hint_any(&["enter: next · esc: back", "enter: next"]);
            }
            Step::Name => {
                self.field.draw_at(g, true);
                let words = match &self.scanned {
                    Some((_, team)) => format!("joining {team} from its QR code"),
                    None => "the code is checked when you go on".to_owned(),
                };
                explain(g, &[&words]);
                g.hint_any(&["enter: join · esc: back", "enter: join"]);
            }
        }
    }
}

// --------------------------------------------------------------- calls ---

/// The six calls: their words, and which break through as urgent.
pub struct CallsPage {
    calls: Vec<(String, bool)>,
    list: ListState,
    editing: Option<Field>,
}

impl CallsPage {
    pub fn new() -> CallsPage {
        CallsPage { calls: super::calls(), list: ListState::default(), editing: None }
    }
}

impl Screen for CallsPage {
    fn title(&self) -> &str {
        "Calls"
    }

    fn app(&self) -> &str {
        "Team"
    }

    fn wants_text(&self) -> bool {
        self.editing.is_some()
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let at = self.list.focus.min(self.calls.len() - 1);
        if let Some(field) = &mut self.editing {
            match field.key(key, |c| !c.is_control()) {
                Typed::More => {}
                Typed::Cancelled => self.editing = None,
                Typed::Done(text) => {
                    let text = super::wire::clean_text(&text);
                    if !text.is_empty() {
                        self.calls[at].0 = text;
                        super::save_calls(&self.calls);
                    }
                    self.editing = None;
                }
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => self.editing = Some(Field::new("the call's words", &self.calls[at].0, 40)),
            Key::Char(' ') => {
                self.calls[at].1 = !self.calls[at].1;
                super::save_calls(&self.calls);
            }
            other => crate::screens::list_key(&mut self.list, other, self.calls.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some(field) = &self.editing {
            field.draw_at(g, true);
            explain(g, &[&format!("sent by {} on the thread", self.list.focus.min(5) + 1)]);
            g.hint_any(&["enter: keep · esc: back", "enter: keep"]);
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, self.calls.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (words, urgent) = &self.calls[i];
            g.row(r, &format!("{}  {words}", i + 1), if *urgent { "urgent" } else { "" }, t.dim, i == self.list.focus);
            if *urgent {
                let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
                let w = g.measure("urgent", style);
                g.surface(Rect::new(r.right() - 18.0 - w, r.y + 6.0, 3.0, r.h - 12.0), 1.5, t.signal);
            }
        }
        self.list.scrollbar(g, self.calls.len(), top, bottom);
        g.hint_any(&["enter: words · space: urgent or not", "enter: words · space: urgent"]);
    }
}
