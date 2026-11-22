//! Remote: the handheld as a universal remote. Any remote's buttons are
//! learned through the infrared receiver (point it at the handheld, press
//! the button) and kept under a remote's name ("TV", "Projector") with a
//! name of their own; Enter sends one back through the transmitter, and
//! held down, keeps sending it as the remote would.
//!
//! Remotes are kept in `remotes.json` beside the settings, each button as
//! the lengths of light and dark it was heard as (see [`crate::ir`]).

use std::path::PathBuf;
use std::time::Instant;

use bezel::{Align, Rect};
use detent::{role, Animated};
use serde_json::{json, Value};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::ir::{self, Learned, Protocol};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// One learned button.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub name: String,
    /// The carrier it is sent at when its protocol does not say.
    pub carrier: u32,
    /// Light, dark, light…: as it was heard, in microseconds.
    pub lengths: Vec<u32>,
}

impl Button {
    pub fn protocol(&self) -> Option<Protocol> {
        ir::decode(&self.lengths)
    }

    /// What it is, short, for its row: `nec 04 08`, or how long it is.
    fn what(&self) -> String {
        match self.protocol() {
            Some(p) => p.short(),
            None => format!("raw · {} ms", self.lengths.iter().map(|&l| l as u64).sum::<u64>().div_ceil(1000)),
        }
    }
}

/// A remote: its name and its buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub name: String,
    pub buttons: Vec<Button>,
}

/// The longest name a remote or a button is given.
const NAME_MOST: usize = 24;

fn file() -> PathBuf {
    crate::config_dir().join("remotes.json")
}

/// The remotes kept. On a computer, until any is kept, two to try the app with.
pub fn load() -> Vec<Remote> {
    match std::fs::read_to_string(file()) {
        Ok(text) => parse(&text),
        Err(_) if ir::parts().simulated => demo(),
        Err(_) => Vec::new(),
    }
}

fn parse(text: &str) -> Vec<Remote> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let name = |v: &Value| v.get("name").and_then(Value::as_str).map(|s| s.trim().chars().take(NAME_MOST).collect::<String>()).filter(|s| !s.is_empty());
    let button = |b: &Value| {
        let lengths: Vec<u32> = b.get("lengths")?.as_array()?.iter().map(|l| l.as_u64().filter(|&l| l > 0 && l < 1_000_000).map(|l| l as u32)).collect::<Option<_>>()?;
        let carrier = b.get("carrier").and_then(Value::as_u64).filter(|c| (20_000..=60_000).contains(c)).unwrap_or(ir::CARRIER as u64) as u32;
        // Light first and last: an even count is no burst.
        if lengths.len() % 2 == 0 {
            return None;
        }
        Some(Button { name: name(b)?, carrier, lengths })
    };
    v.get("remotes")
        .and_then(Value::as_array)
        .map(|remotes| {
            remotes
                .iter()
                .filter_map(|r| {
                    let buttons = r.get("buttons").and_then(Value::as_array).map(|bs| bs.iter().filter_map(button).collect()).unwrap_or_default();
                    Some(Remote { name: name(r)?, buttons })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn save(remotes: &[Remote]) -> bool {
    let doc = json!({
        "remotes": remotes.iter().map(|r| json!({
            "name": r.name,
            "buttons": r.buttons.iter().map(|b| json!({ "name": b.name, "carrier": b.carrier, "lengths": b.lengths })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    std::fs::create_dir_all(crate::config_dir()).ok();
    serde_json::to_vec_pretty(&doc).is_ok_and(|bytes| crate::sys::write_atomic(&file(), &bytes).is_ok())
}

/// The simulator's remotes: a TV's (NEC) and a projector's (Sony).
fn demo() -> Vec<Remote> {
    let button = |name: &str, p: Protocol| Button { name: name.into(), carrier: p.carrier(), lengths: p.encode() };
    let tv = |name: &str, command: u8| button(name, Protocol::Nec { address: 0x04, command });
    let projector = |name: &str, command: u8| button(name, Protocol::Sony { bits: 15, address: 0x54, command });
    vec![
        Remote {
            name: "TV".into(),
            buttons: vec![tv("Power", 0x08), tv("Volume up", 0x02), tv("Volume down", 0x03), tv("Mute", 0x09), tv("Input", 0x0B), tv("Channel up", 0x00)],
        },
        Remote { name: "Projector".into(), buttons: vec![projector("Power", 0x15), projector("Input", 0x25), projector("Picture off", 0x7D)] },
    ]
}

/// A name's characters: anything printable.
fn name_char(c: char) -> bool {
    !c.is_control()
}

/// "3 buttons", "1 button", "no buttons".
fn count(n: usize) -> String {
    match n {
        0 => "no buttons".into(),
        1 => "1 button".into(),
        n => format!("{n} buttons"),
    }
}

/// A few quiet lines in the room a page has left below its rows.
fn note(g: &mut Gfx, words: &str, from: f32, to: f32) {
    let t = g.tokens;
    let style = g.ty.caption;
    let lines = crate::help::wrap(g, words, style, W - 2.0 * PAD - 40.0);
    let line_h = style.size + 4.0;
    let icon = 22.0;
    let block = icon + 8.0 + lines.len() as f32 * line_h;
    if to - from < block + 8.0 {
        return;
    }
    let mut y = (from + to - block) * 0.5;
    g.icon("settings_remote", false, W * 0.5, y + icon * 0.5, icon, t.faint);
    y += icon + 8.0 + line_h * 0.5;
    for line in lines {
        g.text_at(W * 0.5, y, Align::Center, &line, style, t.dim);
        y += line_h;
    }
}

// --------------------------------------------------------------- remotes ---

/// The app's first page: the remotes kept.
pub struct Remotes {
    remotes: Vec<Remote>,
    list: ListState,
    deleting: Option<usize>,
    /// A name being typed: for a new remote, or the one at this place.
    naming: Option<(Field, Option<usize>)>,
    parts: ir::Parts,
}

impl Remotes {
    pub fn new() -> Self {
        Remotes { remotes: load(), list: ListState::default(), deleting: None, naming: None, parts: ir::parts() }
    }
}

impl Screen for Remotes {
    fn title(&self) -> &str {
        "Remote"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn resume(&mut self) {
        self.remotes = load();
        self.list.focus = self.list.focus.min(self.remotes.len());
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, which)) = &mut self.naming {
            let which = *which;
            match field.key(key, name_char) {
                Typed::Done(name) if !name.is_empty() => {
                    self.naming = None;
                    match which {
                        Some(i) if i < self.remotes.len() => {
                            self.remotes[i].name = name;
                            save(&self.remotes);
                        }
                        _ => {
                            self.remotes.push(Remote { name: name.clone(), buttons: Vec::new() });
                            save(&self.remotes);
                            self.list.focus = self.remotes.len() - 1;
                            *cx.say = Some(format!("now learn {}'s buttons", name.to_lowercase()));
                            return Nav::Push(Box::new(Buttons::new(self.remotes.len() - 1)));
                        }
                    }
                }
                Typed::Done(_) | Typed::Cancelled => self.naming = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let armed = self.deleting.take();
        let at = self.list.focus;
        let len = self.remotes.len();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right if at < len => return Nav::Push(Box::new(Buttons::new(at))),
            Key::Enter | Key::Right | Key::Char('n') => self.naming = Some((Field::new("its name, as TV or Projector", "", NAME_MOST), None)),
            Key::Char('e') if at < len => self.naming = Some((Field::new("its name", &self.remotes[at].name, NAME_MOST), Some(at))),
            Key::Char('d') if at < len => {
                if armed == Some(at) {
                    let gone = self.remotes.remove(at);
                    save(&self.remotes);
                    *cx.say = Some(format!("removed {}", gone.name.to_lowercase()));
                } else {
                    *cx.say = Some(format!("d again removes {}", self.remotes[at].name.to_lowercase()));
                    self.deleting = Some(at);
                }
            }
            other => crate::screens::list_key(&mut self.list, other, len + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some((field, _)) = &self.naming {
            return field.draw(g, "enter: done · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let len = self.remotes.len() + 1;
        let mut end = top;
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match self.remotes.get(i) {
                Some(remote) => g.row(r, &remote.name, &count(remote.buttons.len()), t.dim, i == self.list.focus),
                None => g.row(r, "New remote", "n", t.dim, i == self.list.focus),
            }
            end = end.max(r.bottom());
        }
        self.list.scrollbar(g, len, top, bottom);
        if self.remotes.is_empty() {
            note(g, "Learn the buttons of any remote: a TV, a projector, the lights. Each is kept here under its remote's name.", end + 4.0, bottom);
        }
        let hint = if !self.parts.transmitter {
            "no IR transmitter in this system"
        } else if self.list.focus < self.remotes.len() {
            "enter: open · e: rename · d: remove"
        } else {
            "enter: name a new remote"
        };
        g.hint(hint);
    }
}

// --------------------------------------------------------------- buttons ---

/// How long the beam drawn for a send lasts.
const BEAM_SECONDS: f32 = 0.7;

/// One remote's buttons, each sent with Enter.
pub struct Buttons {
    index: usize,
    remotes: Vec<Remote>,
    list: ListState,
    deleting: Option<usize>,
    naming: Option<Field>,
    /// The transmitter while the page is open (its driver with it).
    tx: Option<ir::Transmitter>,
    /// RC5's and RC6's toggle bit: flipped on each new press.
    toggle: bool,
    /// The last button sent, and when.
    last: Option<(usize, Instant)>,
    /// A beam leaving each button sent: its row, and how old it is.
    beams: Vec<(usize, f32)>,
}

impl Buttons {
    pub fn new(index: usize) -> Self {
        let tx = ir::parts().transmitter.then(ir::Transmitter::open);
        Buttons { index, remotes: load(), list: ListState::default(), deleting: None, naming: None, tx, toggle: false, last: None, beams: Vec::new() }
    }

    fn buttons(&self) -> &[Button] {
        self.remotes.get(self.index).map_or(&[], |r| &r.buttons)
    }

    fn send(&mut self, at: usize, cx: &mut Cx) {
        let Some(button) = self.buttons().get(at).cloned() else { return };
        // Enter down for longer than a key takes to repeat: this is the
        // button held, not pressed again.
        let held = self.last.is_some_and(|(i, _)| i == at)
            && crate::platform::enter_held_for().is_some_and(|d| d >= crate::platform::REPEAT_AFTER.mul_f32(0.9));
        if !held {
            self.toggle = !self.toggle;
        }
        self.last = Some((at, Instant::now()));
        let Some(tx) = &self.tx else {
            if !held {
                *cx.say = Some("no IR transmitter in this system".into());
            }
            return;
        };
        if let Some(why) = tx.state().trouble.filter(|_| !held) {
            *cx.say = Some(why);
        }
        tx.send(ir::burst(&button.lengths, button.carrier, held, self.toggle));
        // A new beam each press, and every few repeats while held.
        if !held || self.beams.iter().all(|(row, age)| *row != at || *age > 0.18) {
            self.beams.push((at, 0.0));
        }
    }
}

impl Screen for Buttons {
    fn title(&self) -> &str {
        self.remotes.get(self.index).map_or("Remote", |r| &r.name)
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    /// Held down on a button, Enter keeps sending it, as the remote would.
    fn repeats_enter(&self) -> bool {
        self.naming.is_none() && self.list.focus < self.buttons().len()
    }

    fn resume(&mut self) {
        let before = self.buttons().len();
        self.remotes = load();
        let now = self.buttons().len();
        // Back from learning one: the focus on the button just kept.
        self.list.focus = if now > before { now - 1 } else { self.list.focus.min(now) };
    }

    fn busy(&self) -> bool {
        !self.beams.is_empty()
    }

    fn tick(&mut self, dt: f32) -> bool {
        if self.beams.is_empty() {
            return false;
        }
        for (_, age) in &mut self.beams {
            *age += dt;
        }
        self.beams.retain(|(_, age)| *age < BEAM_SECONDS);
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.naming {
            match field.key(key, name_char) {
                Typed::Done(name) if !name.is_empty() => {
                    self.naming = None;
                    let (index, at) = (self.index, self.list.focus);
                    if let Some(button) = self.remotes.get_mut(index).and_then(|r| r.buttons.get_mut(at)) {
                        button.name = name;
                        save(&self.remotes);
                    }
                }
                Typed::Done(_) | Typed::Cancelled => self.naming = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let armed = self.deleting.take();
        let at = self.list.focus;
        let len = self.buttons().len();
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if at < len => self.send(at, cx),
            Key::Enter | Key::Right if at == len => return Nav::Push(Box::new(Learn::new(self.index))),
            Key::Char('l') => return Nav::Push(Box::new(Learn::new(self.index))),
            Key::Char('e') if at < len => self.naming = Some(Field::new("its name", &self.buttons()[at].name, NAME_MOST)),
            Key::Char('d') if at < len => {
                if armed == Some(at) {
                    let index = self.index;
                    if let Some(remote) = self.remotes.get_mut(index) {
                        let gone = remote.buttons.remove(at);
                        save(&self.remotes);
                        *cx.say = Some(format!("removed {}", gone.name.to_lowercase()));
                    }
                } else {
                    *cx.say = Some(format!("d again removes {}", self.buttons()[at].name.to_lowercase()));
                    self.deleting = Some(at);
                }
            }
            other => crate::screens::list_key(&mut self.list, other, len + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some(field) = &self.naming {
            return field.draw(g, "enter: done · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let buttons = self.buttons().to_vec();
        let len = buttons.len() + 1;
        let mut end = top;
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            let Some(button) = buttons.get(i) else {
                g.row(r, "Learn a button", "l", t.dim, focused);
                end = end.max(r.bottom());
                continue;
            };
            // While a beam leaves it, the row's value gives way to it.
            let age = self.beams.iter().filter(|(row, _)| *row == i).map(|(_, a)| *a).fold(f32::MAX, f32::min);
            let beaming = age < BEAM_SECONDS;
            let what = if beaming { String::new() } else { button.what() };
            g.row(r, &button.name, &what, t.dim, focused);
            if beaming {
                beam(g, r, age);
            }
            end = end.max(r.bottom());
        }
        self.list.scrollbar(g, len, top, bottom);
        if buttons.is_empty() {
            note(g, "Point this remote at the handheld and press a button: it is learned, named, and kept here.", end + 4.0, bottom);
        }
        let trouble = self.tx.as_ref().and_then(|tx| tx.state().trouble);
        let hint = match trouble {
            _ if self.tx.is_none() => "no IR transmitter in this system".to_owned(),
            Some(why) => why,
            None if self.list.focus < buttons.len() => "enter: send · e: rename · d: remove".into(),
            None => "enter: learn a button from its remote".into(),
        };
        g.hint(&hint);
    }
}

/// The beam leaving a button as it is sent: three arcs at the row's right
/// end, lit one after another from the inside out, and fading.
fn beam(g: &mut Gfx, r: Rect, age: f32) {
    let (cx, cy) = (r.right() - 26.0 + g.ox, r.cy());
    let ink = g.ring_color();
    g.canvas.circle(cx, cy, 2.2, ink, 1.0);
    for k in 0..3 {
        // In the calm version, all three at once, fading together.
        let lit = if g.calm { 1.0 - age / BEAM_SECONDS } else { ((age - k as f32 * 0.07) / 0.12).clamp(0.0, 1.0) * (1.0 - age / BEAM_SECONDS).clamp(0.0, 1.0) };
        if lit <= 0.0 {
            continue;
        }
        let radius = 5.0 + k as f32 * 4.5;
        let sweep = 1.25 - k as f32 * 0.12;
        g.canvas.arc(cx, cy, radius, 1.6, std::f32::consts::FRAC_PI_2 - sweep * 0.5, sweep, ink, lit);
    }
}

// ----------------------------------------------------------------- learn ---

/// The learn page's receiver disc: its centre and radius, its rings clear of
/// the panel's edge; and where the words beside it start.
const DISC_X: f32 = PAD + 32.0;
const DISC_R: f32 = 19.0;
const TEXT_X: f32 = DISC_X + DISC_R + 16.0;

/// Learning a button: listening for it, then what was heard, then its name.
pub struct Learn {
    index: usize,
    rx: Option<ir::Receiver>,
    heard: Option<Learned>,
    naming: Option<Field>,
    /// What stops it listening, if anything does.
    trouble: Option<String>,
    /// Seconds spent listening, for the rings.
    listening: f32,
    /// What was heard arriving: 0 → 1 as it is drawn out.
    arrived: Animated,
}

impl Learn {
    pub fn new(index: usize) -> Self {
        let parts = ir::parts();
        let rx = parts.receiver.then(ir::Receiver::open);
        let trouble = (!parts.receiver).then(|| "no IR receiver in this system".to_owned());
        Learn { index, rx, heard: None, naming: None, trouble, listening: 0.0, arrived: Animated::new(0.0, role::settle()).with_rest(0.002) }
    }

    fn listen_again(&mut self) {
        self.heard = None;
        self.arrived.jump(0.0);
        self.listening = 0.0;
        if let Some(rx) = &self.rx {
            rx.clear();
        }
    }

    /// The button heard, kept on its remote under `name`.
    fn keep(&self, name: String) -> bool {
        let Some(heard) = &self.heard else { return false };
        let mut remotes = load();
        let Some(remote) = remotes.get_mut(self.index) else { return false };
        remote.buttons.push(Button { name, carrier: heard.protocol.map_or(ir::CARRIER, |p| p.carrier()), lengths: heard.lengths.clone() });
        save(&remotes)
    }
}

impl Screen for Learn {
    fn title(&self) -> &str {
        "Learn"
    }

    fn wants_text(&self) -> bool {
        self.naming.is_some()
    }

    fn busy(&self) -> bool {
        self.trouble.is_none() && self.heard.is_none()
    }

    /// Someone is pointing a remote at it.
    fn keeps_awake(&self) -> bool {
        self.trouble.is_none() && self.heard.is_none()
    }

    /// Leaving it stops the receiver, which would otherwise go on listening unseen.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.arrived.tick(dt);
        if self.heard.is_none() && self.trouble.is_none() {
            self.listening += dt;
            if let Some(rx) = &self.rx {
                if let Err(why) = rx.ready() {
                    self.trouble = Some(why);
                }
                // A flicker of light is no button: keep listening.
                if let Some(heard) = rx.take().and_then(|burst| ir::learn(&burst)) {
                    self.heard = Some(heard);
                    self.arrived.set(1.0);
                }
            }
            moving = true;
        }
        moving
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.naming {
            match field.key(key, name_char) {
                Typed::Done(name) => {
                    let name = if name.is_empty() { "Button".to_owned() } else { name };
                    *cx.say = Some(if self.keep(name.clone()) { format!("kept {}", name.to_lowercase()) } else { "could not keep it".into() });
                    return Nav::Pop;
                }
                Typed::Cancelled => self.naming = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if self.heard.is_some() => self.naming = Some(Field::new("its name, as Power or Volume up", "", NAME_MOST)),
            Key::Char('r') | Key::Char(' ') if self.heard.is_some() => self.listen_again(),
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.naming {
            return field.draw(g, "enter: keep it · esc: back");
        }
        if g.calm {
            self.arrived.jump(self.arrived.target());
        }
        let (top, bottom) = (g.top(), g.above_hint());
        let arrived = self.arrived.value().clamp(0.0, 1.0);
        // The lower part: what was heard, drawn out as it arrives.
        let wave = Rect::new(PAD, bottom - 30.0, W - 2.0 * PAD, 30.0);
        // The upper: the receiver, and what to do.
        let (cx, cy) = (DISC_X, (top + wave.y) * 0.5);
        let disc = DISC_R;
        if self.heard.is_none() && self.trouble.is_none() {
            // Listening: rings going out from it, slow and faint.
            if g.calm {
                g.canvas.circle(cx + g.ox, cy, disc + 7.0, t.surface2, 0.6);
            } else {
                for k in 0..3 {
                    let phase = (self.listening * 0.55 + k as f32 / 3.0).fract();
                    let radius = disc + 2.0 + phase * 14.0;
                    g.canvas.arc(cx + g.ox, cy, radius, 1.4, 0.0, std::f32::consts::TAU, t.dim, (1.0 - phase) * 0.45);
                }
            }
        }
        // The disc fills with ice as light is heard: something is on.
        let fill = t.surface2.mix(t.ice, arrived);
        g.canvas.circle(cx + g.ox, cy, disc, fill, 1.0);
        let icon_ink = t.dim.mix(t.on_ice, arrived);
        g.icon("settings_remote", self.heard.is_some(), cx, cy, 22.0, icon_ink);

        let x = TEXT_X;
        let room = W - PAD - x;
        let (title, words): (String, String) = match (&self.trouble, &self.heard) {
            (Some(why), _) => ("Can't listen".into(), format!("{why}.")),
            (None, None) => ("Point the remote here".into(), "and press the button to learn, from a hand's width away.".into()),
            (None, Some(heard)) => match heard.protocol {
                Some(p) => (p.label(), details(heard)),
                None => ("Learned as heard".into(), format!("not a protocol it knows: sent back just so · {}", details(heard))),
            },
        };
        let title = g.text.fit(&title, &ty.name, room);
        let lines = crate::help::wrap(g, &words, ty.caption, room);
        let line_h = ty.caption.size + 4.0;
        let block = ty.name.size + 6.0 + lines.len().min(3) as f32 * line_h;
        let mut y = cy - block * 0.5 + ty.name.size * 0.5;
        g.text_at(x, y, Align::Left, &title, ty.name, t.ink);
        y += ty.name.size * 0.5 + 6.0 + line_h * 0.5;
        for line in lines.iter().take(3) {
            g.text_at(x, y, Align::Left, line, ty.caption, t.dim);
            y += line_h;
        }

        // What was heard, light as bars along a track, drawn out from the left.
        g.surface(wave, g.radius(8.0).min(wave.h * 0.5), t.surface1);
        let inner = wave.inset(8.0);
        if let Some(heard) = &self.heard {
            g.surface(Rect::new(inner.x, inner.bottom() - 1.0, inner.w, 1.0), 0.5, t.faint);
            let total: f32 = heard.lengths.iter().map(|&l| l as f32).sum::<f32>().max(1.0);
            let reach = inner.x + inner.w * arrived;
            let mut at = 0.0;
            for (k, &l) in heard.lengths.iter().enumerate() {
                let (x0, x1) = (inner.x + inner.w * at / total, inner.x + inner.w * (at + l as f32) / total);
                at += l as f32;
                if k % 2 == 1 || x0 >= reach {
                    continue;
                }
                let bar = Rect::new(x0, inner.y, (x1.min(reach) - x0).max(1.0), inner.h);
                g.surface(bar, 0.6, t.ice);
            }
        } else if self.trouble.is_none() {
            g.text_at(wave.cx(), wave.cy(), Align::Center, "waiting for light", ty.caption, t.dim);
        }

        let hint = match (&self.trouble, &self.heard) {
            (Some(_), _) => "esc: back",
            (None, None) => "listening · esc: back",
            (None, Some(_)) => "enter: name it · r: listen again",
        };
        g.hint(hint);
    }
}

/// How a learned button was heard: `38 kHz · 68 ms · 34 pulses`.
fn details(heard: &Learned) -> String {
    let ms = heard.lengths.iter().map(|&l| l as u64).sum::<u64>().div_ceil(1000);
    let pulses = heard.lengths.len().div_ceil(2);
    let khz = heard.protocol.map_or(ir::CARRIER, |p| p.carrier()) / 1000;
    format!("{khz} kHz · {ms} ms · {pulses} pulses")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::TextSize;
    use std::time::Duration;

    /// The tests that keep remotes share one file: one at a time.
    static FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn remotes_are_kept_and_read_back() {
        let _one = FILE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::shell::tests::sandbox();
        std::fs::remove_file(file()).ok();
        let mine = vec![
            Remote { name: "Projector".into(), buttons: vec![Button { name: "Power".into(), carrier: 38_000, lengths: Protocol::Nec { address: 1, command: 2 }.encode() }] },
            Remote { name: "Lights".into(), buttons: Vec::new() },
        ];
        assert!(save(&mine));
        assert_eq!(load(), mine);
        // A file with something wrong in it keeps what it can.
        std::fs::write(file(), r#"{"remotes":[{"name":"TV","buttons":[{"name":"Bad","lengths":[500,500]},{"name":"Ok","lengths":[9000,4500,560]}]},{"buttons":[]}]}"#).unwrap();
        let read = load();
        assert_eq!(read.len(), 1, "a remote with no name is dropped");
        assert_eq!(read[0].buttons.len(), 1, "an even burst is no button");
        assert_eq!(read[0].buttons[0].carrier, ir::CARRIER);
        std::fs::remove_file(file()).ok();
    }

    #[test]
    fn the_simulator_has_remotes_to_try() {
        let demo = demo();
        assert!(demo.iter().all(|r| r.buttons.iter().all(|b| b.protocol().is_some())));
        assert_eq!(demo[0].buttons[0].what(), "nec 04 08");
    }

    #[test]
    fn every_line_fits_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for hint in [
                "enter: send · e: rename · d: remove",
                "enter: open · e: rename · d: remove",
                "enter: learn a button from its remote",
                "enter: name it · r: listen again",
                "no IR transmitter in this system",
                "enter: done · esc: cancel",
            ] {
                assert!(g.measure(hint, ty.caption) <= W - 2.0 * PAD, "{size:?}: {hint}");
            }
            // A row: the longest name beside the longest value.
            let row_w = W - 2.0 * PAD - 5.0;
            for value in ["rc6 800f040c", "necx 1234 12", "samsung 07 02", "raw · 450 ms", "no buttons"] {
                let style = ty.label;
                assert!(g.measure("Volume down", ty.body) + g.measure(value, style) + 30.0 <= row_w, "{size:?}: {value}");
            }
            // The learn page's title beside the receiver: the longest label.
            let room = W - PAD - TEXT_X;
            for label in ["RC6-6-32 · 0x800F040C", "Samsung · 0x07 · 0x02", "Point the remote here"] {
                assert!(g.measure(label, ty.name) <= room, "{size:?}: {label}");
            }
            // And its words, in three lines at most.
            let heard = Learned { lengths: Protocol::Nec { address: 4, command: 8 }.encode(), protocol: None };
            for words in ["and press the button to learn, from a hand's width away.".to_owned(), format!("not a protocol it knows: sent back just so · {}", details(&heard))] {
                assert!(crate::help::wrap(&mut g, &words, ty.caption, room).len() <= 3, "{size:?}: {words}");
            }
        }
    }

    #[test]
    fn learning_hears_the_simulated_remote_and_keeps_the_button() {
        // Only where the remote is simulated (a handheld's own test run has
        // its real receiver, or none).
        if !ir::parts().simulated {
            return;
        }
        let _one = FILE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::shell::tests::sandbox();
        std::fs::remove_file(file()).ok();
        assert!(save(&[Remote { name: "TV".into(), buttons: Vec::new() }]));
        let mut learn = Learn::new(0);
        let started = Instant::now();
        while learn.heard.is_none() && started.elapsed() < Duration::from_secs(5) {
            learn.tick(0.016);
            std::thread::sleep(Duration::from_millis(20));
        }
        let heard = learn.heard.clone().expect("the simulated remote was heard");
        assert!(matches!(heard.protocol, Some(Protocol::Nec { address: 0x04, .. })));
        assert!(learn.keep("Power".into()));
        let kept = load();
        assert_eq!(kept[0].buttons.len(), 1);
        assert_eq!(kept[0].buttons[0].protocol(), heard.protocol);
        std::fs::remove_file(file()).ok();
    }
}
