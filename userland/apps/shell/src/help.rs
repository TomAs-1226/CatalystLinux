//! Help: a tour of the keys, offered once after first-boot setup and always
//! from Settings › Help, and the list of keyboard shortcuts, which `?` held
//! for five seconds brings up over any screen that is not taking text.

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

// ------------------------------------------------------------------ tour ---

/// What a step of the tour lights up.
#[derive(Clone, Copy, PartialEq)]
pub enum Spot {
    /// Nothing: the whole screen dims behind the card.
    Nothing,
    /// The tile Home has focused.
    Tile,
    /// The island at the top.
    Island,
    /// Home's page dots at the bottom.
    Pager,
}

/// Where the things a step can light up are on the screen just now.
#[derive(Clone, Copy, Default)]
pub struct Places {
    pub tile: Option<Rect>,
    pub island: Option<Rect>,
    pub pager: Option<Rect>,
}

struct Step {
    spot: Spot,
    title: &'static str,
    words: &'static str,
}

const STEPS: [Step; 8] = [
    Step { spot: Spot::Nothing, title: "A quick tour", words: "Half a minute on the keys. Enter goes on, Esc skips the tour." },
    Step { spot: Spot::Tile, title: "Your apps", words: "The arrows go through them in order, and Enter opens the one in the ring." },
    Step { spot: Spot::Nothing, title: "Going back", words: "Esc goes back a step. Held for a second, it leaves any app, even the terminal; so does Ctrl Q." },
    Step { spot: Spot::Island, title: "The island", words: "News arrives here: a robot linking, an update, a fault. It grows to say it, then settles." },
    Step { spot: Spot::Pager, title: "Pages", words: "Home turns its pages as you go. Left of the first app is the glance page: the robot and this handheld at a glance." },
    Step { spot: Spot::Nothing, title: "Control centre", words: "Ctrl Space brings it down over any screen: Wi-Fi, light mode, brightness. Up from Home's top row works too." },
    Step { spot: Spot::Nothing, title: "Search", words: "Space on Home searches the apps and settings. Just start typing." },
    Step { spot: Spot::Nothing, title: "Forgot a key?", words: "Hold ? for five seconds on any screen for the list of shortcuts. Settings › Help has this tour again." },
];

/// How dark the screen goes behind the tour.
const DIM: f32 = 0.62;

/// The tour: the screen dims, a spotlight glides from one thing to the next,
/// and a card says what each is. Enter (or Right) goes on, Left goes back,
/// Esc skips it.
pub struct Tour {
    step: usize,
    /// The spotlight's place, gliding from step to step, and how lit it is
    /// (0 when a step lights nothing up).
    spot: [Animated; 4],
    lit: Animated,
    spot_placed: bool,
    /// The tour arriving (to 1) and leaving (to 0).
    shown: Animated,
    /// The card's place (its top), and its words fading in after a step.
    card_y: Animated,
    card_placed: bool,
    words: Animated,
    leaving: bool,
    calm: bool,
}

impl Tour {
    pub fn new(calm: bool) -> Tour {
        let glide = || Animated::new(0.0, role::detent()).with_rest(0.05);
        let mut shown = Animated::new(0.0, role::smooth()).with_rest(0.004);
        shown.set(1.0);
        if calm {
            shown.jump(1.0);
        }
        Tour {
            step: 0,
            spot: [glide(), glide(), glide(), glide()],
            lit: Animated::new(0.0, role::smooth()).with_rest(0.004),
            spot_placed: false,
            shown,
            card_y: Animated::new(0.0, role::release()).with_rest(0.05),
            card_placed: false,
            words: Animated::new(1.0, role::smooth()).with_rest(0.004),
            leaving: false,
            calm,
        }
    }

    /// The tour takes every key while it shows.
    pub fn key(&mut self, key: Key) {
        if self.leaving {
            return;
        }
        match key {
            Key::Enter | Key::Right | Key::Down | Key::Char(' ') | Key::Tab => {
                if self.step + 1 < STEPS.len() {
                    self.go(self.step + 1);
                } else {
                    self.leave();
                }
            }
            Key::Left | Key::Up if self.step > 0 => self.go(self.step - 1),
            Key::Back | Key::Leave | Key::Ctrl('q') => self.leave(),
            _ => {}
        }
    }

    fn go(&mut self, step: usize) {
        self.step = step;
        if !self.calm {
            self.words.jump(0.0);
            self.words.set(1.0);
        }
    }

    fn leave(&mut self) {
        self.leaving = true;
        self.shown.set(0.0);
        if self.calm {
            self.shown.jump(0.0);
        }
    }

    /// Which step it is on, from 0.
    pub fn step(&self) -> usize {
        self.step
    }

    /// Advance its motion; true while anything still moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.shown.tick(dt) | self.lit.tick(dt) | self.card_y.tick(dt) | self.words.tick(dt);
        for s in &mut self.spot {
            moving |= s.tick(dt);
        }
        moving
    }

    /// Gone: it was left, and has faded out.
    pub fn gone(&self) -> bool {
        self.leaving && self.shown.value() <= 0.004
    }

    pub fn draw(&mut self, g: &mut Gfx, places: &Places) {
        let t = g.tokens;
        let ty = g.ty;
        let step = &STEPS[self.step];
        let shown = self.shown.value().clamp(0.0, 1.0);
        let target = match step.spot {
            Spot::Nothing => None,
            Spot::Tile => places.tile.map(|r| r.inset(-3.0)),
            Spot::Island => places.island.map(|r| r.inset(-3.0)),
            Spot::Pager => places.pager.map(|r| r.inset(-4.0)),
        };
        // The spotlight glides to its next place, and dims away when a step
        // has nothing to light up (keeping its place for the next that does).
        if let Some(r) = target {
            let want = [r.x, r.y, r.w, r.h];
            for (s, v) in self.spot.iter_mut().zip(want) {
                if !self.spot_placed || self.calm {
                    s.jump(v);
                } else if s.target() != v {
                    s.set(v);
                    g.started_moving();
                }
            }
            self.spot_placed = true;
        }
        let lit_target = if target.is_some() { 1.0 } else { 0.0 };
        if self.lit.target() != lit_target {
            self.lit.set(lit_target);
            g.started_moving();
        }
        if self.calm {
            self.lit.jump(lit_target);
        }
        let spot = Rect::new(self.spot[0].value(), self.spot[1].value(), self.spot[2].value().max(0.0), self.spot[3].value().max(0.0));
        let radius = (spot.h * 0.5).min(g.tile_radius() + 3.0);
        dim_around(g, spot, radius, self.lit.value().clamp(0.0, 1.0), DIM * shown);
        if self.lit.value() > 0.01 {
            let ring = g.ring_color();
            g.canvas.stroke_rrect(spot.inset(-1.5), radius + 1.5, 1.5, ring, self.lit.value().clamp(0.0, 1.0) * shown);
        }

        // The card: where it leaves what is lit in sight (below it if it
        // fits, else above), or in the middle when nothing is lit. The keys
        // are said on the first step and the last; the dots say where it is.
        let inner = W - 2.0 * PAD - 20.0;
        let lines = wrap(g, step.words, ty.body, inner);
        let line_h = ty.body.size + 4.0;
        let foot = self.step == 0 || self.step + 1 == STEPS.len();
        let card_h = 10.0 + ty.name.size + 6.0 + lines.len() as f32 * line_h + if foot { 4.0 + ty.caption.size } else { 0.0 } + 8.0;
        let y = match target {
            None => ((H - card_h) * 0.5).round(),
            Some(r) if H - r.bottom() - 8.0 >= card_h => (H - card_h - 6.0).round(),
            Some(r) if r.y - 8.0 >= card_h => 6.0,
            Some(r) if r.cy() > H * 0.55 => 6.0,
            Some(_) => (H - card_h - 6.0).round(),
        };
        if !self.card_placed || self.calm {
            self.card_y.jump(y);
            self.card_placed = true;
        } else if self.card_y.target() != y {
            self.card_y.set(y);
            g.started_moving();
        }
        // Arriving, it rises a little into place as it fades in.
        let card = Rect::new(PAD, self.card_y.value() + (1.0 - shown) * 10.0, W - 2.0 * PAD, card_h);
        let corner = g.radius(14.0).min(card.h * 0.5);
        g.shadow(card, corner, shown);
        g.canvas.rrect(card, corner, t.surface2, shown);
        let words = self.words.value().clamp(0.0, 1.0) * shown;
        let mut cy = card.y + 10.0 + ty.name.size * 0.5;
        g.text_alpha(card.x + 10.0, cy, step.title, ty.name, t.ink, words);
        // Where it is in the tour, on the title's line.
        for i in 0..STEPS.len() {
            let x = card.right() - 13.0 - (STEPS.len() - 1 - i) as f32 * 9.0;
            let (r, ink) = if i == self.step { (3.0, t.signal) } else { (2.0, t.faint) };
            g.canvas.circle(x, cy, r, ink, shown);
        }
        cy += ty.name.size * 0.5 + 6.0 + line_h * 0.5;
        for line in &lines {
            g.text_alpha(card.x + 10.0, cy, line, ty.body, t.dim.mix(t.ink, 0.45), words);
            cy += line_h;
        }
        if foot {
            let keys = if self.step + 1 == STEPS.len() { "enter: done" } else { "enter: next · esc: skip" };
            let y = card.bottom() - 8.0 - ty.caption.size * 0.5;
            g.text_alpha(card.x + 10.0, y, keys, ty.caption, t.dim, shown);
        }
    }
}

/// Dim the screen toward black by `amount`, except inside `spot` (a rounded
/// rectangle), as far as `lit` says: a spotlight with a soft edge.
fn dim_around(g: &mut Gfx, spot: Rect, radius: f32, lit: f32, amount: f32) {
    let w = W as usize;
    let f = (amount.clamp(0.0, 1.0) * 256.0) as u32;
    if f == 0 {
        return;
    }
    // Rows the spotlight cannot reach dim whole; only its own rows are worked out a pixel at a time.
    let (top, bottom) = if lit > 0.01 { ((spot.y - 3.0).floor() as i32, (spot.bottom() + 3.0).ceil() as i32) } else { (i32::MAX, i32::MIN) };
    for y in 0..H as i32 {
        let row = &mut g.canvas.px[y as usize * w..(y as usize + 1) * w];
        if y < top || y >= bottom {
            bezel::simd::mix_into(row, None, f);
            continue;
        }
        let fy = y as f32 + 0.5;
        for (x, px) in row.iter_mut().enumerate() {
            let d = sd_rrect(x as f32 + 0.5, fy, &spot, radius);
            // Inside, the spotlight lets the screen through; a 2-pixel edge.
            let open = ((1.0 - d) * 0.5).clamp(0.0, 1.0) * lit;
            let k = (f as f32 * (1.0 - open)) as u32;
            if k > 0 {
                *px = bezel::simd::mix_px(*px, 0, k);
            }
        }
    }
}

/// Signed distance from a point to a rounded rectangle: negative inside.
fn sd_rrect(px: f32, py: f32, r: &Rect, radius: f32) -> f32 {
    let k = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let (qx, qy) = ((px - r.cx()).abs() - (r.w * 0.5 - k), (py - r.cy()).abs() - (r.h * 0.5 - k));
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - k
}

/// Words broken into lines no wider than `width`.
pub fn wrap(g: &mut Gfx, words: &str, style: bezel::TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in words.split_whitespace() {
        let tried = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if g.measure(&tried, style) <= width || line.is_empty() {
            line = tried;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ------------------------------------------------------------- shortcuts ---

/// A line of the shortcut list: a heading, or what a key does and the key.
enum Line {
    Head(&'static str),
    Keys(&'static str, &'static str),
}

/// Every shortcut, by where it works. Keys are written as they are printed
/// on a keyboard; " · " between two ways of doing the same thing.
const SHORTCUTS: &[Line] = &[
    Line::Head("everywhere"),
    Line::Keys("Move", "↑ ↓ ← →"),
    Line::Keys("Open, choose", "enter"),
    Line::Keys("Back", "esc"),
    Line::Keys("Leave any app", "esc held · ctrl q"),
    Line::Keys("Control centre", "ctrl space"),
    Line::Keys("This list, after 5 s", "? held"),
    Line::Head("home"),
    Line::Keys("Next or last app", "← →"),
    Line::Keys("Up and down a page", "↑ ↓"),
    Line::Keys("Glance page", "← on the first"),
    Line::Keys("Control centre", "↑ on the top row"),
    Line::Keys("Search", "space"),
    Line::Head("lists"),
    Line::Keys("A page at a time", "pg up · pg dn"),
    Line::Keys("Hold to keep going", "↑ ↓ held"),
    Line::Head("arrange apps"),
    Line::Keys("Hide or show", "enter"),
    Line::Keys("Favourite", "space"),
    Line::Keys("Earlier or later", "← →"),
    Line::Head("terminal"),
    Line::Keys("Scroll back", "pg up · pg dn"),
    Line::Keys("Back to Home", "ctrl q"),
    Line::Head("files"),
    Line::Keys("New file", "n"),
    Line::Keys("Delete", "d"),
    Line::Head("on the handheld"),
    Line::Keys("Arrows, outside typing", "f x z c"),
    Line::Keys("Back, outside typing", "bksp"),
];

/// The keyboard shortcuts, by where they work, each key drawn as a keycap.
pub struct Shortcuts {
    list: ListState,
}

impl Shortcuts {
    pub fn new() -> Self {
        let mut list = ListState::default();
        list.focus = 1;
        Shortcuts { list }
    }

    /// Move the focus, stepping over headings.
    fn step(&mut self, key: Key) {
        let len = SHORTCUTS.len();
        let before = self.list.focus;
        crate::screens::list_key(&mut self.list, key, len);
        let down = self.list.focus >= before;
        while matches!(SHORTCUTS.get(self.list.focus), Some(Line::Head(_))) {
            match (down, self.list.focus) {
                (true, f) if f + 1 < len => self.list.focus += 1,
                (false, 0) => {
                    self.list.focus = 1;
                    break;
                }
                (false, _) => self.list.focus -= 1,
                _ => break,
            }
        }
    }
}

impl Screen for Shortcuts {
    fn title(&self) -> &str {
        "Shortcuts"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            other => {
                self.step(other);
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h) = (g.top(), g.row_h());
        let bottom = H - PAD + 2.0;
        for (i, y) in self.list.layout(g, SHORTCUTS.len(), top, row_h, 3.0, bottom) {
            match SHORTCUTS[i] {
                Line::Head(name) => {
                    g.text_at(PAD + 4.0, y + row_h * 0.62, Align::Left, name, ty.caption, t.dim);
                }
                Line::Keys(what, keys) => {
                    let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
                    let focused = i == self.list.focus;
                    g.row(r, what, "", t.ink, focused);
                    keycaps(g, r.right() - 8.0, r.cy(), keys, focused);
                }
            }
        }
        self.list.scrollbar(g, SHORTCUTS.len(), top, bottom - 2.0);
    }
}

/// Keys as keycaps, right-aligned at `right`: each key on its own cap, a dot
/// between two ways of doing the same thing, plain words as they are.
fn keycaps(g: &mut Gfx, right: f32, cy: f32, keys: &str, focused: bool) {
    let t = g.tokens;
    let style = g.ty.caption;
    let cap_h = style.size + 5.0;
    // Laid out right to left.
    let mut x = right;
    let groups: Vec<&str> = keys.split(" · ").collect();
    for (gi, group) in groups.iter().enumerate().rev() {
        for word in group.split(' ').rev() {
            let is_key = is_key_name(word);
            let w = g.measure(word, style);
            if is_key {
                let cap = Rect::new(x - w - 8.0, cy - cap_h * 0.5, w + 8.0, cap_h);
                let fill = if focused { t.surface3 } else { t.surface2 };
                g.canvas.rrect(cap, 3.5, fill, 1.0);
                g.canvas.rrect(Rect::new(cap.x, cap.bottom() - 1.0, cap.w, 1.0), 0.5, Color::BLACK, 0.18);
                g.text_at(cap.cx(), cy, Align::Center, word, style, t.ink);
                x = cap.x - 3.0;
            } else {
                g.text_at(x, cy, Align::Right, word, style, t.dim);
                x -= w + 4.0;
            }
        }
        if gi > 0 {
            g.text_at(x - 1.0, cy, Align::Right, "·", style, t.faint);
            x -= g.measure("·", style) + 6.0;
        }
    }
}

/// Whether a word in a shortcut is a key (drawn on a cap) or a plain word
/// ("held", "on the first").
fn is_key_name(word: &str) -> bool {
    word.chars().count() == 1 || matches!(word, "enter" | "esc" | "ctrl" | "space" | "tab" | "pg" | "up" | "dn" | "bksp" | "shift" | "fn")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tour_goes_through_its_steps_and_can_be_skipped() {
        let mut tour = Tour::new(true);
        assert_eq!(tour.step(), 0);
        tour.key(Key::Enter);
        tour.key(Key::Right);
        assert_eq!(tour.step(), 2);
        tour.key(Key::Left);
        assert_eq!(tour.step(), 1, "and back");
        for _ in 0..STEPS.len() {
            tour.key(Key::Enter);
        }
        assert!(tour.gone(), "enter on the last step ends it");
        let mut skipped = Tour::new(true);
        skipped.key(Key::Back);
        assert!(skipped.gone(), "esc skips it");
    }

    #[test]
    fn every_line_of_the_tour_and_the_shortcuts_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for step in &STEPS {
                let lines = wrap(&mut g, step.words, ty.body, W - 2.0 * PAD - 20.0);
                assert!(lines.len() <= 4, "{}: {} lines", step.title, lines.len());
            }
            // A shortcut's words and keycaps share a row without meeting.
            for line in SHORTCUTS {
                if let Line::Keys(what, keys) = line {
                    let caps: f32 = keys.split([' ']).map(|k| g.measure(k, ty.caption) + 11.0).sum();
                    let room = W - 2.0 * PAD - 5.0 - 18.0 - caps;
                    assert!(g.measure(what, ty.body) <= room, "{what} and {keys}");
                }
            }
        }
    }

    #[test]
    fn the_shortcut_list_steps_over_its_headings() {
        let mut list = Shortcuts::new();
        assert!(matches!(SHORTCUTS[list.list.focus], Line::Keys(..)));
        for _ in 0..SHORTCUTS.len() {
            list.step(Key::Down);
            assert!(matches!(SHORTCUTS[list.list.focus], Line::Keys(..)));
        }
        for _ in 0..SHORTCUTS.len() {
            list.step(Key::Up);
            assert!(matches!(SHORTCUTS[list.list.focus], Line::Keys(..)));
        }
        assert_eq!(list.list.focus, 1, "back at the first shortcut");
    }
}
