//! Photo documentation's pages: Settings › Photos and its named places, a
//! photo's caption written by hand and its place named, and the build log
//! with its export.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::caption::{self, money};
use super::log::{self, Day, Range};
use super::store::{self, Job, CAPS};
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::photos::{Photo, Picture};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

fn onoff(b: bool) -> String {
    (if b { "on" } else { "off" }).to_owned()
}

/// A quiet line in a list, on a row's place: a note under the row above it.
fn note(g: &mut Gfx, y: f32, row_h: f32, choices: &[&str]) {
    let (style, ink) = (g.ty.caption, g.tokens.dim);
    let room = W - 2.0 * PAD - 12.0;
    let words = choices.iter().copied().find(|c| g.measure(c, style) <= room).unwrap_or_else(|| choices.last().copied().unwrap_or(""));
    let words = g.text.fit(words, &style, room);
    g.text_at(PAD + 6.0, y + row_h * 0.38, Align::Left, &words, style, ink);
}

// --------------------------------------------------------- Settings › Photos ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Captions,
    CaptionsNote,
    Limit,
    Month,
    Waiting,
    Places,
    Wifi,
    Town,
    TownNote,
}

const ROWS: [Row; 9] = [Row::Captions, Row::CaptionsNote, Row::Limit, Row::Month, Row::Waiting, Row::Places, Row::Wifi, Row::Town, Row::TownNote];

fn is_note(r: Row) -> bool {
    matches!(r, Row::CaptionsNote | Row::TownNote)
}

const CAPTIONS_NOTE: [&str; 3] = ["photos are sent to anthropic to be captioned", "photos are sent to anthropic", "sent to anthropic"];
const TOWN_NOTE: [&str; 3] = ["sends nearby wi-fi addresses to beacondb.net", "sends nearby wi-fi to beacondb.net", "sent to beacondb.net"];

/// Settings › Photos.
pub struct PhotoSettings {
    pub(super) list: ListState,
    seen: u64,
    pub(super) has_key: bool,
}

/// What a caption usually costs in a batch: the shrunk photo and the
/// instruction in, about 150 tokens out (thinking at low effort, and the
/// caption).
fn typical() -> u64 {
    caption::cost(&serde_json::json!({"input_tokens": caption::image_tokens(1024, 768) + 300, "output_tokens": 150}), caption::MODEL, true)
}

impl PhotoSettings {
    pub fn new() -> Self {
        PhotoSettings { list: ListState::default(), seen: 0, has_key: crate::keys::Keys::load().get("claude").is_some() }
    }

    /// Opened at one of its rows (from search).
    pub fn at(mut self, row: usize) -> Self {
        self.list.focus = row.min(ROWS.len() - 1);
        if is_note(ROWS[self.list.focus]) {
            self.list.focus -= 1;
        }
        self
    }

    fn describe(&self, row: Row) -> (&'static str, String, bool) {
        let now = chrono::Utc::now().timestamp();
        super::with(|l| {
            let month = store::month_of(now);
            match row {
                Row::Captions => {
                    let v = match (l.prefs.captions, self.has_key) {
                        (true, false) => "on · no key".to_owned(),
                        (on, _) => onoff(on),
                    };
                    ("Caption photos with Claude", v, l.prefs.captions)
                }
                Row::Limit => ("Monthly limit", money(l.cap()), true),
                Row::Month => {
                    let done: Vec<u64> = l.records.values().filter(|r| r.cost > 0 && store::month_of(r.at) == month).map(|r| r.cost).collect();
                    let each = if done.is_empty() { format!("≈{} each", money(typical())) } else { format!("{} each", money(done.iter().sum::<u64>() / done.len() as u64)) };
                    ("This month", format!("{} · {each}", money(l.spent(&month))), l.spent(&month) > 0)
                }
                Row::Waiting => {
                    let n = l.records.values().filter(|r| r.caption.waiting()).count();
                    ("Waiting to caption", if n == 0 { "none".into() } else { format!("{n} photo{}", if n == 1 { "" } else { "s" }) }, n > 0)
                }
                Row::Places => ("Named places", format!("{} ›", l.places.list.len()), true),
                Row::Wifi => ("Note Wi-Fi with each photo", onoff(l.prefs.wifi), l.prefs.wifi),
                Row::Town => ("Find the town online", onoff(l.prefs.town), l.prefs.town),
                Row::CaptionsNote | Row::TownNote => ("", String::new(), false),
            }
        })
    }

    fn hint(&self, row: Row) -> Vec<String> {
        match row {
            Row::Captions if !self.has_key => vec!["needs a claude key: settings › keys · enter: on or off".into(), "needs a claude key: settings › keys".into()],
            Row::Captions => vec!["shrunk to 1024 px, sent in a batch at half price · enter".into(), "half price in batches · enter: on or off".into(), "enter: on or off".into()],
            Row::Limit => vec!["← →: the most a month · past it captions wait".into(), "← →: the monthly limit".into()],
            Row::Month => vec!["from what anthropic counted, at opus 5.5's prices".into(), "at opus 5.5's prices".into()],
            Row::Waiting => {
                let status = super::status();
                if status.is_empty() {
                    vec!["sent once online, idle and charged · enter: now".into(), "enter: look now".into()]
                } else {
                    vec![format!("{status} · enter: look now"), status]
                }
            }
            Row::Places => vec!["enter: rename or delete · n on a photo names one".into(), "enter: the places".into()],
            Row::Wifi => vec!["for named places · kept on this handheld only".into(), "kept on this handheld only".into()],
            Row::Town => vec!["only photos with no named place or event · enter".into(), "enter: on or off".into()],
            Row::CaptionsNote | Row::TownNote => vec![String::new()],
        }
    }
}

impl Screen for PhotoSettings {
    fn title(&self) -> &str {
        "Photos"
    }

    fn resume(&mut self) {
        self.has_key = crate::keys::Keys::load().get("claude").is_some();
        self.seen = 0;
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let row = ROWS[self.list.focus.min(ROWS.len() - 1)];
        let toggle = matches!(key, Key::Enter | Key::Left | Key::Right);
        match key {
            Key::Back => return Nav::Pop,
            _ if toggle && row == Row::Captions => {
                let on = super::with(|l| {
                    l.prefs.captions = !l.prefs.captions;
                    l.save();
                    l.prefs.captions
                });
                *cx.say = Some(if on { "captions on · new photos go to claude".into() } else { "captions off · nothing is sent".into() });
                if on {
                    super::nudge();
                }
            }
            Key::Left | Key::Right if row == Row::Limit => super::with(|l| {
                let at = CAPS.iter().position(|&c| c >= l.prefs.cap_cents).unwrap_or(2) as i32 + if key == Key::Left { -1 } else { 1 };
                l.prefs.cap_cents = CAPS[at.clamp(0, CAPS.len() as i32 - 1) as usize];
                l.save();
            }),
            Key::Enter if row == Row::Waiting => {
                super::nudge();
                *cx.say = Some("looking now".into());
            }
            Key::Enter | Key::Right if row == Row::Places => return Nav::Push(Box::new(PlacesPage::new())),
            _ if toggle && row == Row::Wifi => super::with(|l| {
                l.prefs.wifi = !l.prefs.wifi;
                l.save();
            }),
            _ if toggle && row == Row::Town => {
                let (on, queued) = super::with(|l| {
                    l.prefs.town = !l.prefs.town;
                    let queued = if l.prefs.town { l.queue_towns(chrono::Utc::now().timestamp()) } else { 0 };
                    l.save();
                    (l.prefs.town, queued)
                });
                *cx.say = Some(match (on, queued) {
                    (false, _) => "town lookup off · nothing is sent".into(),
                    (true, 0) => "town lookup on".into(),
                    (true, n) => format!("town lookup on · {n} to look up"),
                });
                if on {
                    super::nudge();
                }
            }
            other => crate::tools::step_over_headings(&mut self.list, other, ROWS.len(), |i| is_note(ROWS[i])),
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("photos", &[("enter · ← →", "Turn on or off"), ("← →", "The monthly limit"), ("enter", "Named places")])]
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let g = super::generation();
        let fresh = g != self.seen;
        self.seen = g;
        fresh
    }

    /// The work's status changes on its own thread: looked in on a few
    /// times a second, never holding the frame beat.
    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        self.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, ROWS.len(), top, row_h, 3.0, bottom) {
            match ROWS[i] {
                Row::CaptionsNote => note(g, y, row_h, &CAPTIONS_NOTE),
                Row::TownNote => note(g, y, row_h, &TOWN_NOTE),
                row => {
                    let (name, value, lit) = self.describe(row);
                    g.row(Rect::new(PAD, y, width, row_h), name, &value, if lit { t.ink } else { t.dim }, i == self.list.focus);
                }
            }
        }
        self.list.scrollbar(g, ROWS.len(), top, bottom);
        let hints = self.hint(ROWS[self.list.focus.min(ROWS.len() - 1)]);
        let refs: Vec<&str> = hints.iter().map(String::as_str).collect();
        g.hint_any(&refs);
    }
}

// ------------------------------------------------------------ named places ---

/// Settings › Photos › Named places: each with its photos; renamed, deleted.
pub struct PlacesPage {
    list: ListState,
    deleting: Option<u32>,
    seen: u64,
    places: Vec<(u32, String, usize)>,
}

impl PlacesPage {
    pub fn new() -> Self {
        let mut p = PlacesPage { list: ListState::default(), deleting: None, seen: 0, places: Vec::new() };
        p.refresh();
        p
    }

    fn refresh(&mut self) {
        self.places = super::with(|l| l.places.list.iter().map(|p| (p.id, p.name.clone(), l.photos_of(p.id))).collect());
        self.seen = super::generation();
    }
}

impl Screen for PlacesPage {
    fn title(&self) -> &str {
        "Named places"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn resume(&mut self) {
        self.refresh();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let focused = self.places.get(self.list.focus).cloned();
        match key {
            Key::Back if armed.is_some() => {}
            Key::Back => return Nav::Pop,
            Key::Enter => {
                if let Some((id, name, _)) = focused {
                    return Nav::Push(Box::new(RenamePlace::new(id, &name)));
                }
            }
            Key::Char('d') => {
                if let Some((id, name, _)) = focused {
                    if armed == Some(id) {
                        super::with(|l| {
                            l.places.delete(id);
                            l.apply_places();
                            l.save();
                        });
                        self.refresh();
                        *cx.say = Some(format!("{} forgotten · its photos keep no name", name.to_lowercase()));
                    } else {
                        self.deleting = Some(id);
                    }
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.places.len()),
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.deleting.is_some() {
            return vec![keys("forgetting", &[("d", "Forget the place"), ("esc", "Keep it")])];
        }
        vec![keys("places", &[("enter", "Rename it"), ("d twice", "Forget it")])]
    }

    fn tick(&mut self, _dt: f32) -> bool {
        if super::generation() != self.seen {
            self.refresh();
            return true;
        }
        false
    }

    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        self.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.places.is_empty() {
            g.empty("location_on", "no named places yet · n on a photo names where it was taken");
            g.hint("esc: back");
            return;
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, self.places.len(), top, row_h, 3.0, bottom) {
            let (id, name, n) = &self.places[i];
            let value = if self.deleting == Some(*id) { "d again: forget".to_owned() } else { format!("{n} photo{}", if *n == 1 { "" } else { "s" }) };
            g.row(Rect::new(PAD, y, width, row_h), name, &value, if self.deleting == Some(*id) { g.fault() } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, self.places.len(), top, bottom);
        if self.deleting.is_some() {
            g.hint_any(&["d again forgets it · its photos keep no name", "d again: forget it"]);
        } else {
            g.hint_any(&["enter: rename · d twice: forget · on this handheld only", "enter: rename · d twice: forget", "enter: rename"]);
        }
    }
}

/// A named place given a new name: every photo of it takes the new one.
pub struct RenamePlace {
    id: u32,
    field: Field,
    done: bool,
}

impl RenamePlace {
    pub fn new(id: u32, name: &str) -> Self {
        RenamePlace { id, field: Field::new("its new name", name, super::places::NAME_MOST), done: false }
    }
}

impl Screen for RenamePlace {
    fn title(&self) -> &str {
        "Rename"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("rename", &[("enter", "Keep the name"), ("esc", "Leave it")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) => {
                let ok = super::with(|l| {
                    let ok = l.places.rename(self.id, &text);
                    if ok {
                        l.apply_places();
                        l.save();
                    }
                    ok
                });
                if !ok {
                    *cx.say = Some("another place has that name".into());
                    return Nav::Stay;
                }
                Nav::Pop
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        self.field.draw(g, "enter: rename · every photo of it takes the name");
    }
}

/// "Name this place": a photo's place named, or said to be one already
/// named. Every photo there, before and after, takes the name.
pub struct NamePlace {
    photo: String,
    pub(super) field: Field,
    /// Places already named, then names often given: (name, what it is).
    rows: Vec<(String, String)>,
    pub(super) list: ListState,
    /// The focus is in the list (else on the field).
    pub(super) in_list: bool,
    done: bool,
}

/// Names a team often gives, offered until used.
const OFTEN: [&str; 4] = ["Shop", "Pit", "Home", "School"];

impl NamePlace {
    pub fn new(photo: &str) -> Self {
        let named: Vec<(String, usize)> = super::with(|l| l.places.list.iter().map(|p| (p.name.clone(), l.photos_of(p.id))).collect());
        let photos = |n: usize| format!("{n} photo{}", if n == 1 { "" } else { "s" });
        let mut rows: Vec<(String, String)> = named.iter().map(|(name, n)| (name.clone(), photos(*n))).collect();
        rows.extend(OFTEN.iter().filter(|o| !named.iter().any(|(n, _)| n.eq_ignore_ascii_case(o))).map(|o| ((*o).to_owned(), "new".to_owned())));
        // The place it is now, if it is one, first.
        let now = super::meta::read(&crate::photos::dir().join(photo)).place;
        if let Some(at) = rows.iter().position(|(n, _)| *n == now) {
            let mut r = rows.remove(at);
            r.1 = format!("here · {}", r.1);
            rows.insert(0, r);
        }
        NamePlace { photo: photo.to_owned(), field: Field::new("name this place", "", super::places::NAME_MOST), rows, list: ListState::default(), in_list: false, done: false }
    }

    fn name(&mut self, name: &str, cx: &mut Cx) -> Nav {
        let result = super::with(|l| {
            let r = l.name_place(&self.photo, name).map(|id| (l.places.get(id).map(|p| p.name.clone()).unwrap_or_default(), l.photos_of(id)));
            l.save();
            r
        });
        match result {
            Ok((name, n)) => {
                *cx.say = Some(format!("{} · {n} photo{} there", name.to_lowercase(), if n == 1 { "" } else { "s" }));
                self.done = true;
                Nav::Pop
            }
            Err(why) => {
                *cx.say = Some(why);
                Nav::Stay
            }
        }
    }
}

impl Screen for NamePlace {
    fn title(&self) -> &str {
        "Name this place"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn wants_text(&self) -> bool {
        !self.in_list
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("name it", &[("enter", "Keep the name"), ("↓ ↑", "A name given before"), ("esc", "Leave it")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if self.in_list {
            match key {
                Key::Up if self.list.focus == 0 => self.in_list = false,
                Key::Enter => {
                    let name = self.rows.get(self.list.focus).map(|r| r.0.clone()).unwrap_or_default();
                    return self.name(&name, cx);
                }
                Key::Back => return Nav::Pop,
                Key::Char(c) if !c.is_control() => {
                    self.in_list = false;
                    self.field.text.push(c);
                }
                other => crate::screens::list_key(&mut self.list, other, self.rows.len()),
            }
            return Nav::Stay;
        }
        if key == Key::Down && !self.rows.is_empty() {
            self.in_list = true;
            return Nav::Stay;
        }
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) if text.is_empty() => Nav::Stay,
            Typed::Done(text) => self.name(&text, cx),
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        self.field.draw_at(g, !self.in_list);
        let top = g.top() + Field::height(g) + 6.0;
        let (row_h, bottom) = (g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 5.0;
        let focus = if self.in_list { Some(self.list.focus) } else { None };
        for (i, y) in self.list.layout(g, self.rows.len(), top, row_h, 3.0, bottom) {
            let (name, what) = &self.rows[i];
            g.row(Rect::new(PAD, y, width, row_h), name, what, t.dim, focus == Some(i));
        }
        self.list.scrollbar(g, self.rows.len(), top, bottom);
        if self.in_list {
            g.hint_any(&["enter: this place · its photos take the name", "enter: this place"]);
        } else {
            g.hint_any(&["enter: name it · ↓: a name given before", "enter: name it · ↓: before", "enter: name it"]);
        }
    }
}

// -------------------------------------------------------------- the caption ---

/// A photo's caption written (or rewritten) by hand. Kept as yours: Claude
/// never writes over it.
pub struct CaptionPage {
    photo: PathBuf,
    field: Field,
    done: bool,
}

impl CaptionPage {
    pub fn new(photo: &Path, text: &str) -> Self {
        CaptionPage { photo: photo.to_path_buf(), field: Field::new("caption", text, caption::CAPTION_MOST), done: false }
    }
}

/// Keep a caption written by hand (none, if it is empty: and none is asked
/// for again).
pub fn keep_caption(photo: &Path, text: &str) -> std::io::Result<()> {
    let text = caption::tidy(text);
    let mut about = super::meta::read(photo);
    about.by = if text.is_empty() { String::new() } else { "you".into() };
    about.caption = text.clone();
    super::meta::write(photo, &about)?;
    let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::with(|l| {
        if let Some(r) = l.records.get_mut(&name) {
            r.caption = if text.is_empty() { Job::Skipped("cleared by you".into()) } else { Job::Done };
        }
        l.save();
    });
    Ok(())
}

impl Screen for CaptionPage {
    fn title(&self) -> &str {
        "Caption"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("caption", &[("enter", "Keep it"), ("esc", "Leave it as it was")])]
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(text) => {
                *cx.say = Some(match keep_caption(&self.photo, &text) {
                    Ok(()) if text.is_empty() => "caption cleared".into(),
                    Ok(()) => "caption kept".into(),
                    Err(_) => "the caption could not be kept".into(),
                });
                self.done = true;
                Nav::Pop
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        // The whole caption, wrapped, the cursor at its end.
        let top = g.top();
        let bottom = g.above_hint() - 2.0;
        let r = Rect::new(PAD, top, W - 2.0 * PAD, bottom - top);
        g.surface(r, 14.0, t.surface2);
        g.text_at(r.x + 10.0, r.y + 7.0 + ty.caption.size * 0.5, Align::Left, self.field.label, ty.caption, t.dim);
        let width = r.w - 24.0;
        let line_h = ty.body.size + 4.0;
        let mut lines = crate::help::wrap(g, &self.field.text, ty.body, width);
        if lines.is_empty() {
            lines.push(String::new());
        }
        let first_y = r.y + 12.0 + ty.caption.size;
        let room = (((r.bottom() - 8.0) - first_y) / line_h).floor().max(1.0) as usize;
        let shown = &lines[lines.len().saturating_sub(room)..];
        for (k, line) in shown.iter().enumerate() {
            g.text_at(r.x + 10.0, first_y + (k as f32 + 0.5) * line_h, Align::Left, line, ty.body, t.ink);
        }
        let last = shown.last().map_or("", String::as_str);
        let cx = r.x + 11.0 + g.measure(last, ty.body);
        let cy = first_y + (shown.len() as f32 - 0.5) * line_h;
        g.surface(Rect::new(cx, cy - ty.body.size * 0.5, 1.5, ty.body.size), 0.5, t.signal);
        g.focus_ring(r, 14.0);
        g.copy_field = Some(self.field.text.clone());
        let left = caption::CAPTION_MOST.saturating_sub(self.field.text.chars().count());
        let hint = format!("enter: keep it · {left} left");
        g.hint_any(&[&format!("enter: keep it as yours · {left} left"), &hint, "enter: keep it"]);
    }
}

// -------------------------------------------------------------- build log ---

/// Thumbnails made on a thread for the entries in sight.
struct Thumbs {
    shared: Arc<Mutex<(Vec<Photo>, HashMap<String, Picture>, HashSet<String>, u64)>>,
    stop: Arc<AtomicBool>,
}

impl Thumbs {
    fn new() -> Thumbs {
        let shared: Arc<Mutex<(Vec<Photo>, HashMap<String, Picture>, HashSet<String>, u64)>> = Arc::new(Mutex::new((Vec::new(), HashMap::new(), HashSet::new(), 0)));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("log-thumbs".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                while !st.load(Ordering::Relaxed) {
                    let next = s.lock().ok().and_then(|x| x.0.iter().find(|p| !x.1.contains_key(&p.name) && !x.2.contains(&p.name)).cloned());
                    let Some(photo) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        continue;
                    };
                    let made = crate::photos::thumbnail(&photo);
                    let Ok(mut x) = s.lock() else { return };
                    match made {
                        Some(t) => {
                            x.1.insert(photo.name.clone(), t);
                        }
                        None => {
                            x.2.insert(photo.name.clone());
                        }
                    }
                    x.3 += 1;
                }
            })
            .ok();
        Thumbs { shared, stop }
    }

    fn want(&self, photos: Vec<Photo>) {
        if let Ok(mut x) = self.shared.lock() {
            if x.0 != photos {
                x.0 = photos;
            }
        }
    }

    fn get(&self, name: &str) -> Option<Picture> {
        self.shared.lock().ok()?.1.get(name).cloned()
    }

    fn generation(&self) -> u64 {
        self.shared.lock().map_or(0, |x| x.3)
    }

    /// Some wanted are not made yet.
    fn pending(&self) -> bool {
        self.shared.lock().is_ok_and(|x| x.0.iter().any(|p| !x.1.contains_key(&p.name) && !x.2.contains(&p.name)))
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A line of the log.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    Day(usize),
    Place(usize, usize),
    Photo(usize, usize, usize),
}

/// The build log: days, then places, each photo with its caption.
pub struct BuildLog {
    photos: Vec<Photo>,
    days: Vec<Day>,
    lines: Vec<Line>,
    /// Which line has the focus (a photo's).
    focus: usize,
    scroll: Animated,
    thumbs: Thumbs,
    seen: (u64, u64),
    /// `s` once: sending the focused day waits for `s` again.
    pub(super) sending: Option<chrono::NaiveDate>,
    states: HashMap<String, Job>,
}

impl BuildLog {
    pub fn new() -> Self {
        #[cfg(not(target_arch = "aarch64"))]
        {
            crate::photos::seed_demo();
            super::seed_demo();
        }
        super::reconcile();
        let mut log = BuildLog { photos: Vec::new(), days: Vec::new(), lines: Vec::new(), focus: 0, scroll: Animated::new(0.0, role::settle()).with_rest(0.1), thumbs: Thumbs::new(), seen: (0, 0), sending: None, states: HashMap::new() };
        log.refresh();
        log
    }

    /// Opened on a photo's day.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn at(mut self, name: &str) -> Self {
        if let Some(i) = self.lines.iter().position(|l| matches!(*l, Line::Photo(d, g, e) if self.days[d].groups[g].entries[e].name == name)) {
            self.focus = i;
        }
        self
    }

    fn refresh(&mut self) {
        let focused = self.focused().map(|e| e.name.clone());
        self.photos = crate::photos::list();
        self.days = log::gather(&self.photos);
        self.lines.clear();
        for (d, day) in self.days.iter().enumerate() {
            self.lines.push(Line::Day(d));
            for (g, group) in day.groups.iter().enumerate() {
                self.lines.push(Line::Place(d, g));
                for e in 0..group.entries.len() {
                    self.lines.push(Line::Photo(d, g, e));
                }
            }
        }
        self.states = super::with(|l| l.records.iter().map(|(n, r)| (n.clone(), r.caption.clone())).collect());
        let again = focused.and_then(|name| self.lines.iter().position(|l| matches!(*l, Line::Photo(d, g, e) if self.days[d].groups[g].entries[e].name == name)));
        self.focus = again.or_else(|| self.lines.iter().position(|l| matches!(l, Line::Photo(..)))).unwrap_or(0);
        self.seen.0 = super::generation();
    }

    fn focused(&self) -> Option<&log::Entry> {
        match self.lines.get(self.focus)? {
            Line::Photo(d, g, e) => self.days.get(*d)?.groups.get(*g)?.entries.get(*e),
            _ => None,
        }
    }

    fn focused_day(&self) -> Option<&Day> {
        match self.lines.get(self.focus)? {
            Line::Photo(d, ..) | Line::Place(d, _) | Line::Day(d) => self.days.get(*d),
        }
    }

    fn step(&mut self, by: isize) {
        let mut i = self.focus as isize;
        loop {
            i += by;
            if i < 0 || i >= self.lines.len() as isize {
                return;
            }
            if matches!(self.lines[i as usize], Line::Photo(..)) {
                self.focus = i as usize;
                return;
            }
        }
    }

    fn height(&self, g: &Gfx, line: Line) -> f32 {
        let ty = g.ty;
        match line {
            Line::Day(_) => ty.name.size + 14.0,
            Line::Place(..) => ty.caption.size + 10.0,
            Line::Photo(..) => ty.caption.size + 2.0 * (ty.body.size + 3.0) + 14.0,
        }
    }

    fn state_words(&self, name: &str) -> &'static str {
        match self.states.get(name) {
            Some(Job::Waiting { .. }) => "a caption is on its way",
            Some(Job::Sent(_)) => "claude is writing its caption",
            Some(Job::Skipped(_)) | Some(Job::Failed(_)) => "no caption · t on the photo writes one",
            _ if crate::photos::is_pixel(name) => "a pixel picture",
            _ => "no caption · t on the photo writes one",
        }
    }
}

impl Screen for BuildLog {
    fn title(&self) -> &str {
        "Build log"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn resume(&mut self) {
        self.refresh();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let sending = self.sending.take();
        match key {
            Key::Back if sending.is_some() => {}
            Key::Back => return Nav::Pop,
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::Home => {
                self.focus = 0;
                self.step(1);
            }
            Key::End => {
                self.focus = self.lines.len();
                self.step(-1);
            }
            Key::PageDown => (0..4).for_each(|_| self.step(1)),
            Key::PageUp => (0..4).for_each(|_| self.step(-1)),
            Key::Enter => {
                if let Some(e) = self.focused() {
                    let at = self.photos.iter().position(|p| p.path == e.path).unwrap_or(0);
                    return Nav::Push(Box::new(crate::photos::Viewer::new(self.photos.clone(), at)));
                }
            }
            Key::Char('e') => {
                if let Some(day) = self.focused_day() {
                    return Nav::Push(Box::new(ExportPage::new(self.days.clone(), day.date)));
                }
            }
            Key::Char('s') => {
                let Some(day) = self.focused_day().cloned() else { return Nav::Stay };
                if sending != Some(day.date) {
                    self.sending = Some(day.date);
                    return Nav::Stay;
                }
                *cx.say = Some(send_to_team(&day, cx.settings.clock24));
            }
            _ => {}
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.sending.is_some() {
            return vec![keys("sending", &[("s", "Send the day to Team"), ("esc", "Not now")])];
        }
        vec![keys("build log", &[("enter", "Open the photo"), ("e", "Export as Markdown"), ("s twice", "Send the day to Team"), ("fn k · fn n", "First, last")])]
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.scroll.tick(dt);
        if super::generation() != self.seen.0 {
            self.refresh();
            changed = true;
        }
        let t = self.thumbs.generation();
        if t != self.seen.1 {
            self.seen.1 = t;
            changed = true;
        }
        changed
    }

    fn busy(&self) -> bool {
        // Thumbnails in sight still coming from their thread.
        self.thumbs.pending()
    }

    /// Captions arriving, places named: looked in on a few times a second.
    fn ambient_tick(&mut self, _dt: f32, _robot: &Robot) -> bool {
        if super::generation() != self.seen.0 {
            self.refresh();
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if self.lines.is_empty() {
            g.empty("menu_book", "no photos yet · the camera's photos make the log");
            g.hint("esc: back");
            return;
        }
        let top = g.top();
        let bottom = g.above_hint();
        // Where each line is, and the scroll that keeps the focus (and its
        // day and place above it, when it is the first) in sight.
        let mut ys = Vec::with_capacity(self.lines.len());
        let mut y = 0.0;
        for line in &self.lines {
            ys.push(y);
            y += self.height(g, *line) + 3.0;
        }
        let total = y;
        let room = bottom - top - crate::gfx::RING_ROOM;
        let fy = ys[self.focus];
        let fh = self.height(g, self.lines[self.focus]);
        // The headings just above the focused photo come into sight with it.
        let mut head = self.focus;
        while head > 0 && !matches!(self.lines[head - 1], Line::Photo(..)) {
            head -= 1;
        }
        let want_top = ys[head];
        let mut target = self.scroll.target();
        if want_top < target {
            target = want_top;
        }
        if fy + fh > target + room {
            target = fy + fh - room;
        }
        target = target.clamp(0.0, (total - room).max(0.0));
        // At rest, the line at the top is whole: a heading or a photo, never
        // the foot of one (what runs on past the bottom fades out there).
        target = ys.iter().copied().filter(|y| *y >= target - 0.5 && *y <= fy).fold(fy, f32::min);
        if (target - self.scroll.target()).abs() > 0.5 {
            self.scroll.set(target);
        }
        if g.calm {
            self.scroll.jump(self.scroll.target());
        }
        let scroll = self.scroll.value();
        let foot = bottom + crate::gfx::FOOT_AIR;
        g.canvas.set_clip(Rect::new(0.0, top - 4.0, W, foot - top + 4.0));
        let mut fade = crate::gfx::Fade::default();
        fade.take(g, foot - 14.0, foot);
        let mut wanted = Vec::new();
        let mut ring = None;
        let width = W - 2.0 * PAD - 5.0;
        for (i, line) in self.lines.iter().enumerate() {
            let y = top + ys[i] - scroll;
            let h = self.height(g, *line);
            if y + h < top - 4.0 || y > foot {
                continue;
            }
            match *line {
                Line::Day(d) => {
                    let day = &self.days[d];
                    g.text_at(PAD + 2.0, y + h * 0.56, Align::Left, &day.words(), ty.name, t.ink);
                    let n = day.count();
                    let words = format!("{n} photo{}", if n == 1 { "" } else { "s" });
                    g.text_at(W - PAD - 6.0, y + h * 0.56, Align::Right, &words, ty.caption, t.dim);
                }
                Line::Place(d, gi) => {
                    let title = &self.days[d].groups[gi].title;
                    let (icon, words) = if title.is_empty() { ("info", "place not known".to_owned()) } else { ("location_on", title.to_lowercase()) };
                    g.icon(icon, false, PAD + 8.0, y + h * 0.5, 11.0, t.dim);
                    g.text_fit(PAD + 18.0, y + h * 0.5, &words, ty.caption, t.dim, width - 18.0);
                }
                Line::Photo(d, gi, ei) => {
                    let e = &self.days[d].groups[gi].entries[ei];
                    let r = Rect::new(PAD, y, width, h);
                    let focused = i == self.focus;
                    let radius = g.radius(9.0).min(h * 0.3);
                    g.surface(r, radius, if focused { t.surface2 } else { t.surface1 });
                    if focused {
                        ring = Some((r, radius));
                    }
                    // The thumbnail, 4:3, its corners inside the row's.
                    let th = h - 8.0;
                    let tw = (th * 4.0 / 3.0).round();
                    let tr = Rect::new(r.x + 4.0, r.y + 4.0, tw, th);
                    let inner = (radius - 4.0).max(3.0);
                    if let Some(p) = self.photos.iter().find(|p| p.name == e.name) {
                        wanted.push(p.clone());
                    }
                    match self.thumbs.get(&e.name) {
                        // Pictures are drawn straight to the panel, past the
                        // list's clip: cut to it here.
                        Some(pic) => {
                            let (from, to) = ((top - 2.0).max(tr.y), foot.min(tr.bottom()));
                            if to > from {
                                let k = pic.h as f32 / tr.h;
                                let part = Rect::new(tr.x, from, tr.w, to - from);
                                let view = [0.0, (from - tr.y) * k, pic.w as f32, (to - from) * k];
                                let r = if part.h < tr.h - 0.5 { 0.0 } else { inner };
                                if e.pixel() {
                                    crate::photos::draw_picture_crisp(g, part, &pic, view, r, 1.0);
                                } else {
                                    crate::photos::draw_picture(g, part, &pic, view, r, 1.0);
                                }
                            }
                        }
                        None => {
                            g.surface(tr, inner, t.surface3);
                            g.icon("photo", false, tr.cx(), tr.cy(), 12.0, t.faint);
                        }
                    }
                    let x = tr.right() + 8.0;
                    let text_w = r.right() - 8.0 - x;
                    let clock = if g.clock.0 { e.at.format("%H:%M").to_string() } else { e.at.format("%-I:%M %P").to_string() };
                    let meta = if e.about.context.is_empty() { clock } else { format!("{clock} · {}", e.about.context) };
                    let line1 = r.y + 6.0 + ty.caption.size * 0.5;
                    g.text_fit(x, line1, &meta, ty.caption, t.dim, text_w);
                    let body_h = ty.body.size + 3.0;
                    let first = r.y + 8.0 + ty.caption.size + body_h * 0.5;
                    if e.about.caption.is_empty() {
                        g.text_fit(x, first, self.state_words(&e.name), ty.caption, t.faint, text_w);
                    } else {
                        let mut lines = crate::help::wrap(g, &e.about.caption, ty.body, text_w);
                        if lines.len() > 2 {
                            let rest = lines.split_off(1).join(" ");
                            lines.push(g.text.fit(&rest, &ty.body, text_w));
                        }
                        for (k, l) in lines.iter().enumerate() {
                            g.text_at(x, first + k as f32 * body_h, Align::Left, l, ty.body, t.ink);
                        }
                    }
                }
            }
        }
        g.canvas.clear_clip();
        fade.apply(g);
        if let Some((r, radius)) = ring {
            g.focus_ring(r, radius);
        }
        self.thumbs.want(wanted);
        // The focused day along the foot, its heading being out of sight at times.
        let day = self.focused_day().map(|d| d.words()).unwrap_or_default();
        if self.sending.is_some() {
            g.hint_any(&[&format!("s again sends {day} to the team · esc: not now"), &format!("s again sends {day} to the team"), "s again: send"]);
        } else {
            g.hint_any(&[&format!("{day} · enter: open · e: export · s twice: team"), &format!("{day} · e: export · s twice: team"), &format!("{day} · e: export"), "e: export"]);
        }
    }
}

/// A day sent to Team: what it said.
pub fn send_to_team(day: &Day, h24: bool) -> String {
    if !crate::team::member() {
        return "join a team first, in Team".into();
    }
    let texts = log::team_texts(day, h24);
    for text in &texts {
        if let Err(why) = crate::team::send(crate::team::wire::Body::Text(text.clone()), false) {
            return why;
        }
    }
    format!("{} sent to the team", day.words())
}

// ----------------------------------------------------------------- export ---

/// Export a day, its event, its week or everything, as Markdown with photos.
pub struct ExportPage {
    days: Vec<Day>,
    at: chrono::NaiveDate,
    ranges: Vec<Range>,
    list: ListState,
    working: Option<Receiver<Result<(PathBuf, usize), String>>>,
    done: Option<Result<(PathBuf, usize), String>>,
}

impl ExportPage {
    pub fn new(days: Vec<Day>, at: chrono::NaiveDate) -> Self {
        let has_event = days.iter().find(|d| d.date == at).and_then(Day::event).is_some();
        let ranges = [Range::Day, Range::Event, Range::Week, Range::All].into_iter().filter(|r| *r != Range::Event || has_event).collect();
        ExportPage { days, at, ranges, list: ListState::default(), working: None, done: None }
    }

    fn describe(&self, range: Range) -> (String, String) {
        let chosen = log::select(&self.days, range, self.at);
        let n: usize = chosen.iter().map(|d| d.count()).sum();
        let photos = format!("{n} photo{}", if n == 1 { "" } else { "s" });
        let day = self.days.iter().find(|d| d.date == self.at);
        let name = match range {
            Range::Day => format!("This day · {}", day.map(Day::words).unwrap_or_default()),
            Range::Event => day.and_then(Day::event).unwrap_or("The event").to_owned(),
            Range::Week => "The week to this day".into(),
            Range::All => "Everything".into(),
        };
        (name, photos)
    }

    /// Where it was written, as Files and Companion find it.
    fn where_words(path: &Path) -> String {
        let shown = path.strip_prefix(crate::photos::dir()).map(|p| format!("photos/{}", p.display())).unwrap_or_else(|_| path.display().to_string());
        shown.replace('\\', "/")
    }

    /// Export now, on this thread (the pictures).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn export_now(&mut self, range: Range, h24: bool) {
        let chosen = log::select(&self.days, range, self.at);
        self.done = Some(log::export(&chosen, range, &log::exports_dir(), h24));
        self.list.focus = self.ranges.iter().position(|r| *r == range).unwrap_or(0);
    }
}

impl Screen for ExportPage {
    fn title(&self) -> &str {
        "Export"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if self.working.is_none() => {
                let range = self.ranges[self.list.focus.min(self.ranges.len() - 1)];
                let days: Vec<Day> = log::select(&self.days, range, self.at).into_iter().cloned().collect();
                let h24 = cx.settings.clock24;
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name("log-export".into())
                    .spawn(move || {
                        crate::sched::enter(crate::sched::Class::Background);
                        let refs: Vec<&Day> = days.iter().collect();
                        tx.send(log::export(&refs, range, &log::exports_dir(), h24)).ok();
                    })
                    .ok();
                self.working = Some(rx);
                self.done = None;
            }
            other => crate::screens::list_key(&mut self.list, other, self.ranges.len()),
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![keys("export", &[("enter", "Write it, with its photos"), ("↑ ↓", "Day, event, week or all")])]
    }

    fn tick(&mut self, _dt: f32) -> bool {
        if let Some(rx) = &self.working {
            if let Ok(result) = rx.try_recv() {
                self.working = None;
                self.done = Some(result);
                return true;
            }
        }
        false
    }

    fn busy(&self) -> bool {
        self.working.is_some()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let row_h = g.row_h();
        // What came of the last export, along the foot above the hint.
        // (icon, first line, second line, colour)
        let status = match (&self.working, &self.done) {
            (Some(_), _) => Some(("downloading", "writing it…".to_owned(), String::new(), t.dim)),
            (_, Some(Ok((path, n)))) => {
                let shown = Self::where_words(path);
                let (folder, name) = shown.rsplit_once('/').unwrap_or(("", &shown));
                Some(("download_done", format!("{n} photo{} · in {folder}", if *n == 1 { "" } else { "s" }), name.to_owned(), t.ink))
            }
            (_, Some(Err(why))) => Some(("error", why.clone(), String::new(), g.fault())),
            _ => None,
        };
        let lines = status.as_ref().map_or(0.0, |s| if s.2.is_empty() { 1.0 } else { 2.0 });
        let status_h = if status.is_some() { lines * (ty.caption.size + 4.0) + 10.0 } else { 0.0 };
        let bottom = g.above_hint() - status_h;
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, self.ranges.len(), top, row_h, 3.0, bottom) {
            let (name, value) = self.describe(self.ranges[i]);
            g.row(Rect::new(PAD, y, width, row_h), &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, self.ranges.len(), top, bottom);
        if let Some((icon, first, second, color)) = status {
            let r = Rect::new(PAD, bottom + 3.0, W - 2.0 * PAD, status_h - 4.0);
            g.take_foot(r.y);
            g.surface(r, g.radius(8.0).min(r.h * 0.5), t.surface1);
            let line = ty.caption.size + 4.0;
            let y0 = r.y + 3.0 + line * 0.5;
            g.icon(icon, false, r.x + 12.0, y0, 12.0, color);
            g.text_fit(r.x + 24.0, y0, &first, ty.caption, color, r.w - 30.0);
            if !second.is_empty() {
                g.text_fit(r.x + 24.0, y0 + line, &second, ty.caption, t.dim, r.w - 30.0);
            }
        }
        g.hint_any(&["enter: markdown and photos, in photos/build log", "enter: write it · in photos/build log", "enter: write it"]);
    }
}
