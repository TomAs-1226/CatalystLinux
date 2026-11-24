//! Under the photo: the viewer's sheet with what is said about it. ↓ brings
//! the sheet up and the photo, whole, shrinks into the room above it: the
//! caption, then where it was taken (the named place, the event, the
//! match), then who wrote the caption and what it cost. ↑ or Esc lets it
//! back down. It stays up from photo to photo, as a phone's does. `t`
//! writes the caption, `n` names the place, `a` asks Claude now.
//!
//! The sheet rises on Detent's `release` spring and goes on `smooth`
//! (arriving and leaving, as the island does), the photo following it
//! exactly; any key mid-way turns it from wherever it is. In the calm
//! version it lands at once.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::meta::About;
use super::store::Job;
use crate::gfx::{Gfx, H, W};
use crate::platform::Key;
use crate::shell::{Cx, Nav};

pub struct Under {
    path: PathBuf,
    name: String,
    about: About,
    caption: Job,
    cost: u64,
    has_wifi: bool,
    /// 0 down, 1 up.
    open: Animated,
    seen: u64,
    /// A caption asked for at once, coming.
    asking: Option<Receiver<Result<String, String>>>,
    /// Why the last one asked for did not come.
    failed: Option<String>,
    /// The sheet's height as last laid out.
    height: f32,
}

impl Default for Under {
    fn default() -> Self {
        Under {
            path: PathBuf::new(),
            name: String::new(),
            about: About::default(),
            caption: Job::None,
            cost: 0,
            has_wifi: false,
            open: Animated::new(0.0, role::release()).with_rest(0.002),
            seen: 0,
            asking: None,
            failed: None,
            height: 0.0,
        }
    }
}

/// The air between the photo and the panel's top, and the sheet, when up.
const AIR: f32 = 4.0;

impl Under {
    pub fn new() -> Under {
        Under::default()
    }

    /// The photo in sight now.
    pub fn arrive(&mut self, path: &Path) {
        self.path = path.to_path_buf();
        self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.failed = None;
        self.load();
    }

    fn load(&mut self) {
        self.about = super::meta::read(&self.path);
        let record = super::with(|l| l.records.get(&self.name).cloned());
        self.caption = record.as_ref().map(|r| r.caption.clone()).unwrap_or_default();
        self.cost = record.as_ref().map_or(0, |r| r.cost);
        self.has_wifi = record.is_some_and(|r| r.fp.len() >= 2);
        self.seen = super::generation();
    }

    pub fn is_open(&self) -> bool {
        self.open.target() > 0.5
    }

    fn show(&mut self, up: bool) {
        let spring = if up { role::release() } else { role::smooth() };
        self.open.set_with(if up { 1.0 } else { 0.0 }, spring);
    }

    /// A key on the viewer, not zoomed in: `Some` when it was the sheet's.
    pub fn key(&mut self, key: Key, cx: &mut Cx) -> Option<Nav> {
        match key {
            // Zooming in, or the details: the sheet goes down for them.
            Key::Char('+' | '=' | 'i') | Key::Enter if self.is_open() => {
                self.show(false);
                return None;
            }
            Key::Down if !self.is_open() => self.show(true),
            Key::Up | Key::Back if self.is_open() => self.show(false),
            Key::Down if self.is_open() => {}
            Key::Char('t') => return Some(Nav::Push(Box::new(super::ui::CaptionPage::new(&self.path, &self.about.caption)))),
            Key::Char('n') => {
                if !self.has_wifi {
                    *cx.say = Some(if super::with(|l| l.prefs.wifi) { "no wi-fi was noted with this photo".into() } else { "wi-fi isn't noted: Settings › Photos".into() });
                    return Some(Nav::Stay);
                }
                return Some(Nav::Push(Box::new(super::ui::NamePlace::new(&self.name))));
            }
            Key::Char('a') => {
                if self.asking.is_some() {
                    return Some(Nav::Stay);
                }
                if let Err(why) = super::captionable(&self.name) {
                    *cx.say = Some(format!("not captioned: {why}"));
                    return Some(Nav::Stay);
                }
                if !super::with(|l| l.prefs.captions) {
                    *cx.say = Some("captions are off: Settings › Photos".into());
                    return Some(Nav::Stay);
                }
                let Some(api) = super::caption::Api::from_keys() else {
                    *cx.say = Some("no claude key: Settings › Keys".into());
                    return Some(Nav::Stay);
                };
                let (tx, rx) = mpsc::channel();
                let name = self.name.clone();
                std::thread::Builder::new()
                    .name("caption-now".into())
                    .stack_size(512 * 1024)
                    .spawn(move || {
                        crate::sched::enter(crate::sched::Class::Background);
                        tx.send(super::caption::now_for(super::ledger(), &api, &name, chrono::Utc::now().timestamp())).ok();
                    })
                    .ok();
                self.asking = Some(rx);
                self.failed = None;
                *cx.say = Some("asking claude for a caption".into());
                self.show(true);
            }
            _ => return None,
        }
        Some(Nav::Stay)
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.open.tick(dt);
        if let Some(rx) = &self.asking {
            if let Ok(result) = rx.try_recv() {
                self.asking = None;
                self.failed = result.err();
                self.load();
                changed = true;
            }
        }
        if super::generation() != self.seen {
            self.load();
            changed = true;
        }
        changed
    }

    pub fn busy(&self) -> bool {
        self.asking.is_some()
    }

    /// How far up the sheet is, 0 to 1 (a little past 1 as it lands).
    fn up(&self, g: &Gfx) -> f32 {
        if g.calm {
            return if self.is_open() { 1.0 } else { 0.0 };
        }
        self.open.value().clamp(0.0, 1.1)
    }

    /// Where the photo goes while the sheet is up: shrunk about the
    /// panel's top middle by this much, and this far down from its top.
    /// (1, 0) with the sheet down.
    pub fn room(&self, g: &Gfx) -> (f32, f32) {
        let up = self.up(g);
        let k = ((H - 2.0 * AIR * up.min(1.0) - self.height * up) / H).clamp(0.2, 1.0);
        (k, AIR * up.min(1.0))
    }

    /// Where it was taken, in a line: the match first (the venue is what a
    /// narrow panel can leave out), then the place line.
    fn place_words(&self) -> Option<String> {
        let place = self.about.place_line();
        let context = (!self.about.context.is_empty()).then(|| self.about.context.clone());
        match (place, context) {
            (Some(p), Some(c)) if !self.about.place.is_empty() && !self.about.event.is_empty() => Some(format!("{c} · {p} · {}", self.about.event)),
            (Some(p), Some(c)) => Some(format!("{c} · {p}")),
            (Some(p), None) if !self.about.place.is_empty() && !self.about.event.is_empty() => Some(format!("{p} · {}", self.about.event)),
            (p, c) => p.or(c),
        }
    }

    /// Who wrote the caption, and what it cost.
    fn source_words(&self) -> Option<String> {
        match self.about.by.as_str() {
            "" => None,
            "you" => Some("written by you".to_owned()),
            _ if self.cost > 0 => Some(format!("by claude · {}", super::caption::money(self.cost))),
            _ => Some("by claude".to_owned()),
        }
    }

    /// What the sheet says with no caption.
    fn no_caption(&self) -> String {
        if self.asking.is_some() {
            return "asking claude…".into();
        }
        if let Some(why) = &self.failed {
            return format!("no caption: {why}");
        }
        match &self.caption {
            Job::Waiting { .. } => "caption on its way, once online".into(),
            Job::Sent(_) => "claude is writing its caption".into(),
            Job::Failed(why) | Job::Skipped(why) => format!("no caption: {why}"),
            _ => "no caption yet".into(),
        }
    }

    /// The sheet (and, down, the chip that says it is there), under the
    /// photo whose frame is `frame`, at `shade` of the chrome; the chips
    /// along the photo's foot end at `left_end`.
    pub fn draw(&mut self, g: &mut Gfx, frame: Rect, shade: f32, left_end: f32) {
        let t = g.tokens;
        let ty = g.ty;
        let pad = 10.0;
        let width = W - 2.0 * pad;
        // Laid out every frame: the text size may have changed. The
        // caption whole up to three lines, cut at the third past that.
        let blank = self.about.caption.is_empty();
        let style = if blank { ty.caption } else { ty.body };
        let words = if blank { self.no_caption() } else { self.about.caption.clone() };
        let mut lines = crate::help::wrap(g, &words, style, width);
        if lines.len() > 3 {
            let rest = lines.split_off(2).join(" ");
            lines.push(g.text.fit(&rest, &style, width));
        }
        let line_h = style.size + 4.0;
        let place = self.place_words();
        let fact_h = ty.caption.size + 7.0;
        let facts = 1.0 + if place.is_some() { 1.0 } else { 0.0 };
        self.height = (pad + lines.len() as f32 * line_h + 5.0 + facts * fact_h + 5.0).round();
        let up = self.up(g);
        // Down: a chip at the photo's foot, with the chrome.
        if up < 0.02 {
            if shade > 0.0 {
                let words = if blank { "↓ about" } else { "↓ caption" };
                let w = g.measure(words, ty.caption) + 14.0;
                let h = ty.caption.size + 8.0;
                let x = frame.right() - 6.0 - w;
                if x > left_end + 4.0 {
                    let y = frame.bottom() - 6.0 - h;
                    g.canvas.rrect(Rect::new(x + g.ox, y, w, h), h * 0.5, bezel::Color(0, 0, 0), 0.55 * shade);
                    g.text_alpha(x + 7.0, y + h * 0.5, words, ty.caption, bezel::Color(240, 240, 240), shade);
                }
            }
            return;
        }
        let top = H - self.height * up;
        let radius = g.radius(14.0);
        let sheet = Rect::new(0.0, top, W, H - top + radius);
        g.surface(sheet, radius, t.surface1);
        let mut y = top + pad;
        for line in &lines {
            g.text_at(pad, y + line_h * 0.5, Align::Left, line, style, if blank { t.dim } else { t.ink });
            y += line_h;
        }
        y += 5.0;
        if let Some(place) = &place {
            g.icon("location_on", false, pad + 6.0, y + fact_h * 0.5, 12.0, t.dim);
            g.text_fit(pad + 17.0, y + fact_h * 0.5, &place.to_lowercase(), ty.caption, t.dim, width - 17.0);
            y += fact_h;
        }
        // Last: who wrote it, and the keys, as far as they fit beside it.
        let source = self.source_words();
        let mut left = pad;
        if let Some(s) = &source {
            g.icon("auto_awesome", false, pad + 6.0, y + fact_h * 0.5, 12.0, t.dim);
            left = pad + 17.0 + g.text_fit(pad + 17.0, y + fact_h * 0.5, s, ty.caption, t.dim, width - 17.0) + 12.0;
        }
        let keys: &[&str] = if blank { &["t: write · n: name the place · a: ask claude", "t: write · n: place · a: ask", "t · n · a"] } else { &["t: edit · n: name the place", "t: edit · n: place", "t · n"] };
        if let Some(k) = keys.iter().find(|k| left + g.measure(k, ty.caption) <= W - pad) {
            g.text_at(W - pad, y + fact_h * 0.5, Align::Right, k, ty.caption, t.faint);
        }
    }

    pub fn key_groups(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.is_open() {
            vec![keys("about it", &[("↑ · esc", "Put it away"), ("t", "Write the caption"), ("n", "Name this place"), ("a", "Caption it now")])]
        } else {
            vec![keys("about it", &[("↓", "Caption and place"), ("t", "Write the caption"), ("n", "Name this place"), ("a", "Caption it now")])]
        }
    }
}
