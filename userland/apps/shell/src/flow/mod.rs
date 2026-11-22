//! Flow: Home's third style, the fancy one. The apps are album covers in a
//! cover flow: the one in the middle faces you, its neighbours stand turned
//! toward it on either side, overlapping in perspective, and each is
//! reflected in the floor. The ground takes on the middle card's colour as a
//! glow that shifts as the flow turns.
//!
//! Keys: Left and Right (Z and C) turn it a card at a time; held, they
//! gather speed, so every app flies past in under a second and a half, and
//! let go, it glides to rest on a card on Detent's springs, carrying the
//! speed it had. Every key lands on the motion as it is, mid-glide included.
//! Enter brings the middle card forward and turns it over: its back says what
//! the app is, what it does and what it says just now (its Home tile's live
//! line). Enter again opens the app out of it; Esc turns it back.
//!
//! It is drawn on the GPU when the GPU is drawing and is quicker at it,
//! otherwise on the processor, to the same picture (`render.rs`).

pub mod art;
#[cfg(not(target_arch = "aarch64"))]
pub mod gallery;
mod palette;
pub mod render;

use bezel::{Color, Rect, TextStyle, Tone};
use detent::{role, Animated, Spring};

use crate::gfx::{Gfx, W};
use art::{Atlas, CardSpec, FaceSpec, Lettering};
pub use render::{Placed, Scene};

// ---------------------------------------------------------------- layout ---

/// A card's side in the middle at rest, in pixels; its picture is drawn at
/// exactly this many texels, so at rest it is shown texel for pixel.
pub const CARD: usize = 96;
/// The enlarged card, turned over: texel for pixel, too.
pub const FACE_W: usize = 236;
pub const FACE_H: usize = 116;
/// The cards either side stand turned this far toward the middle (radians,
/// about 62°), this far back, the first this far out and each next one this
/// much further. (My own tuning, by eye on the panel.)
const SIDE_ANGLE: f32 = 1.08;
const SIDE_DEPTH: f32 = 64.0;
const FIRST: f32 = 78.0;
const SPACING: f32 = 24.0;
/// Coming forward to turn over, the card rises this far toward the eye at
/// the middle of its turn.
const LIFT: f32 = 28.0;
/// The other cards draw this much further aside while one is enlarged.
const MAKE_WAY: f32 = 52.0;

/// Paint the cards' pictures on the thread that asks rather than one of
/// their own: for the previews, whose frames run far quicker than the
/// clock and must each show what the handheld would. (The tests always do.)
pub static PAINT_HERE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How many cards either side of the middle are drawn: the sixth out is
/// already wholly behind the fifth and past the panel's edge.
const REACH: f32 = 5.5;
/// Cards this far out and further have their reflections fade away, over
/// two cards: stacked so close they are only a blur under the nearer ones.
const REFLECTED: f32 = 2.5;

// ---------------------------------------------------------------- motion ---

/// A key in the same direction this soon after the last is a key held (they
/// repeat every 75 ms) or keys pressed as fast: the flow gathers speed.
const HELD_GAP: f32 = 0.13;
/// How often a held key repeats (`platform::REPEAT_EVERY`): between two,
/// the flow runs on toward where the next will be rather than waiting.
const REPEAT: f32 = 0.075;
/// A push against either end, in cards a second: the flow gives a little
/// and springs back.
const PUSH: f32 = 9.0;

/// A card turned to with a key: Bezel's detent role, the dial landing on its
/// detent (`apple-wwdc18-rotation`, response 0.4, damping 0.8).
fn tap_spring() -> Spring {
    role::detent()
}

/// While a key is held, and as it glides to rest once let go: one spring
/// throughout, between Bezel's hold role (response 0.15) and its detent
/// (0.4), so the flow trails the keys by a few cards at full speed and,
/// let go, glides on through them and lands on the last with no jolt (a
/// spring that was following a steady run starts its landing with no
/// braking at all) and no visible overshoot (0.15%, settled in 0.3 s).
/// Response 0.2, damping 0.9: my own tuning.
fn held_spring() -> Spring {
    Spring::response_damping(0.2, 0.9)
}

/// How many cards a step goes once a key has been held for `held` seconds.
fn stride(held: f32) -> usize {
    match held {
        h if h < 0.25 => 1,
        h if h < 0.5 => 2,
        h if h < 0.75 => 3,
        _ => 4,
    }
}

/// Past either end the flow resists: iOS's rubber band
/// (`ios-rubberband`, c = 0.55), over one card.
pub fn rubber(pos: f32, count: usize) -> f32 {
    let last = count.saturating_sub(1) as f32;
    let band = |x: f32| (1.0 - 1.0 / (x * 0.55 + 1.0)) * 1.0;
    if pos < 0.0 {
        -band(-pos)
    } else if pos > last {
        last + band(pos - last)
    } else {
        pos
    }
}

/// What a key did to the flow, for Home to follow.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Moved {
    /// Nothing for Home to do.
    Stay,
    /// Left at the first card, not held: the glance page.
    Glance,
}

/// Which spring the flow is on just now.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Riding {
    Tap,
    Held,
}

pub struct Flow {
    /// Where the flow is turned to, in cards (the focused one is where it heads).
    pub pos: Animated,
    riding: Riding,
    /// A run of keys: which way, for how long, and the time since the last.
    dir: i32,
    held: f32,
    since_key: f32,
    /// While a key is held, the flow runs from here toward the focus in the
    /// time between two repeats.
    ramp_from: f32,
    /// 0 in the flow, 1 enlarged (turned over), and which card.
    open: Animated,
    opened: Option<usize>,
    /// Back from the app it opened: turn the card back once Home is in front again.
    pub close_on_return: bool,
    /// The glow's colour, easing from card to card.
    glow: [Animated; 3],
    glow_placed: bool,
    /// The pictures, and what they were made for; new ones being painted,
    /// and what for; and an empty one to draw from until the first come.
    pub cards: Option<Atlas>,
    cards_for: Option<(Vec<CardSpec>, u64)>,
    painting: Option<(std::sync::mpsc::Receiver<Atlas>, (Vec<CardSpec>, u64))>,
    nothing: Atlas,
    /// Home has just come to Flow: the covers sweep in once they are painted.
    pub arrive_when_ready: bool,
    pub face: Option<Atlas>,
    face_for: Option<(FaceSpec, u64)>,
    /// Where the middle card and the enlarged one rest, as last laid out.
    middle: Rect,
    enlarged: Rect,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    pub fn new() -> Self {
        let glow = || Animated::new(0.0, role::smooth()).with_rest(0.5);
        Flow {
            pos: Animated::new(0.0, tap_spring()).with_rest(0.002),
            riding: Riding::Tap,
            dir: 0,
            held: 0.0,
            since_key: f32::INFINITY,
            ramp_from: 0.0,
            open: Animated::new(0.0, role::release()).with_rest(0.002),
            opened: None,
            close_on_return: false,
            glow: [glow(), glow(), glow()],
            glow_placed: false,
            cards: None,
            cards_for: None,
            painting: None,
            nothing: Atlas::new(8, 8, 1),
            arrive_when_ready: false,
            face: None,
            face_for: None,
            middle: Rect::default(),
            enlarged: Rect::default(),
        }
    }

    fn ride(&mut self, riding: Riding, target: f32) {
        if self.riding != riding {
            self.riding = riding;
            let spring = match riding {
                Riding::Tap => tap_spring(),
                Riding::Held => held_spring(),
            };
            self.pos.set_with(target, spring);
        } else {
            self.pos.set(target);
        }
    }

    /// Where a held run has got to between two repeats: on its way from
    /// where the last one left it to the focus.
    fn ramp(&self, focus: usize) -> f32 {
        let t = (self.since_key / REPEAT).clamp(0.0, 1.0);
        self.ramp_from + (focus as f32 - self.ramp_from) * t
    }

    /// The flow is here at once (a different Home chosen, the apps arranged again).
    pub fn place(&mut self, focus: usize) {
        self.pos.jump(focus as f32);
        self.riding = Riding::Tap;
        self.since_key = f32::INFINITY;
        self.held = 0.0;
    }

    /// Arriving as Home's style: the cards sweep in from the right.
    pub fn arrive(&mut self, focus: usize) {
        self.place(focus);
        self.pos.jump(focus as f32 + 1.6);
        self.ride(Riding::Tap, focus as f32);
    }

    /// Head for card `to` (Esc back to the first, a robot linking): a glide.
    pub fn turn_to(&mut self, focus: &mut usize, to: usize, calm: bool) {
        *focus = to;
        self.since_key = f32::INFINITY;
        if calm {
            self.pos.jump(to as f32);
        } else {
            self.ride(Riding::Tap, to as f32);
        }
    }

    /// Left (`dir` −1) or Right (+1), `count` cards in all. Held keys go
    /// further each step; against an end the flow gives and comes back.
    pub fn step(&mut self, focus: &mut usize, count: usize, dir: i32, calm: bool) -> Moved {
        if count == 0 {
            return Moved::Stay;
        }
        // Moving on from an enlarged card turns it back as the flow goes.
        self.close(calm);
        let held = dir == self.dir && self.since_key < HELD_GAP;
        let from_ramp = if self.riding == Riding::Held { self.ramp(*focus) } else { self.pos.target() };
        self.held = if held { self.held + self.since_key } else { 0.0 };
        self.dir = dir;
        self.since_key = 0.0;
        let last = count - 1;
        let at = *focus;
        let to = if dir < 0 { at.saturating_sub(stride(self.held)) } else { (at + stride(self.held)).min(last) };
        if to == at {
            // Left at the first card, pressed (not run into): the glance page.
            if dir < 0 && !held {
                return Moved::Glance;
            }
            // A run carries on toward the end from where it has got to.
            self.ramp_from = from_ramp;
            if !calm {
                self.pos.nudge(dir as f32 * PUSH);
            }
            return Moved::Stay;
        }
        *focus = to;
        if calm {
            self.pos.jump(to as f32);
        } else if held {
            self.ramp_from = from_ramp;
            self.ride(Riding::Held, from_ramp);
        } else {
            self.ride(Riding::Tap, to as f32);
        }
        Moved::Stay
    }

    /// Enter in the flow: the card in the middle comes forward and turns over.
    pub fn enlarge(&mut self, focus: usize, calm: bool) {
        self.opened = Some(focus);
        if calm {
            self.open.jump(1.0);
        } else {
            self.open.set(1.0);
        }
    }

    /// Turn the enlarged card back into the flow.
    pub fn close(&mut self, calm: bool) {
        if self.open.target() == 0.0 {
            return;
        }
        if calm {
            self.open.jump(0.0);
        } else {
            self.open.set(0.0);
        }
    }

    /// Whether a card is enlarged, or on its way to being.
    pub fn is_open(&self) -> bool {
        self.open.target() > 0.5
    }

    /// Whether the enlarged card is `focus`, turned over far enough to open from.
    pub fn open_on(&self, focus: usize) -> bool {
        self.is_open() && self.opened == Some(focus)
    }

    /// How far turned over the enlarged card is, 0 to 1.
    pub fn openness(&self) -> f32 {
        self.open.value()
    }

    /// Where the card in the middle rests, and the enlarged one.
    pub fn middle(&self) -> Rect {
        self.middle
    }
    pub fn enlarged(&self) -> Rect {
        self.enlarged
    }

    /// Whether everything has come to rest.
    pub fn settled(&self) -> bool {
        self.pos.is_settled() && self.open.is_settled()
    }

    pub fn tick(&mut self, dt: f32, focus: usize) -> bool {
        self.since_key += dt;
        if self.riding == Riding::Held {
            // On toward the card the last repeat chose; let go, the ramp has
            // reached it and the flow glides on to it on the same spring.
            let to = self.ramp(focus);
            self.pos.set(to);
        }
        let mut moving = self.pos.tick(dt) | self.open.tick(dt);
        for c in &mut self.glow {
            moving |= c.tick(dt);
        }
        if self.open.value() <= 0.0005 && self.open.target() == 0.0 {
            self.opened = None;
        }
        // A run still counted keeps time running, so the next key can tell
        // it apart from a fresh press even with nothing else moving.
        moving || self.since_key < HELD_GAP
    }

    // ------------------------------------------------------------ pictures ---

    /// The cards' pictures for these apps, made again only when what they
    /// show or the lettering changes. They are painted on a thread of their
    /// own (about a tenth of a second on the handheld), so no frame waits
    /// for them: until new ones are ready the last ones stay in use, and the
    /// very first time the flow shows its ground alone until they come.
    /// True when new ones have just arrived.
    pub fn make_cards(&mut self, g: &mut Gfx, specs: &[CardSpec]) -> bool {
        let wanted = (specs.to_vec(), lettering_key(g));
        if self.cards.is_some() && self.cards_for.as_ref() == Some(&wanted) {
            return false;
        }
        if let Some((painted, of)) = &self.painting {
            if *of == wanted {
                match painted.try_recv() {
                    Ok(atlas) => {
                        (self.cards, self.cards_for, self.painting) = (Some(atlas), Some(wanted), None);
                        return true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                    // The painter gave up: paint them here.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => self.painting = None,
                }
            }
        }
        let (lettering, letters) = (lettering(g), (g.look.letters.round(), g.look.width.scale()));
        let paint = move |specs: Vec<CardSpec>| {
            let mut text = bezel::Text::new();
            text.set_letters(letters.0, letters.1);
            // A set of icons of its own, so the large ghosts are not kept in
            // the shell's for every other screen.
            let mut icons = bezel::Icons::new();
            let mut atlas = Atlas::new(CARD, CARD, specs.len());
            for (i, spec) in specs.iter().enumerate() {
                atlas.put(i, &art::card(&mut text, &mut icons, spec, &lettering, CARD));
            }
            atlas
        };
        let (send, painted) = std::sync::mpsc::channel();
        let specs = wanted.0.clone();
        // The tests (and a system that will not start a thread) paint them here and now.
        let here = cfg!(test) || PAINT_HERE.load(std::sync::atomic::Ordering::Relaxed);
        let started = !here && std::thread::Builder::new().name("flow-cards".into()).spawn(move || send.send(paint(specs)).ok()).is_ok();
        if started {
            self.painting = Some((painted, wanted));
            return false;
        }
        (self.cards, self.cards_for) = (Some(paint(wanted.0.clone())), Some(wanted));
        true
    }

    /// The cards' pictures, or (until the first are painted) none to draw from.
    pub fn atlases(&self) -> [&Atlas; 2] {
        let cards = self.cards.as_ref().unwrap_or(&self.nothing);
        [cards, self.face.as_ref().unwrap_or(cards)]
    }

    /// Whether the cards' pictures are being painted and none are in use yet.
    pub fn waiting(&self) -> bool {
        self.cards.is_none()
    }

    /// The back of the enlarged card, made again when what it says changes.
    pub fn make_face(&mut self, g: &mut Gfx, spec: &FaceSpec) {
        let key = lettering_key(g);
        if self.face.is_some() && self.face_for.as_ref().is_some_and(|(s, k)| s == spec && *k == key) {
            return;
        }
        let lettering = lettering(g);
        let mut icons = bezel::Icons::new();
        let mut atlas = self.face.take().unwrap_or_else(|| Atlas::new(FACE_W, FACE_H, 1));
        let px = art::face(&mut g.text, &mut icons, spec, &lettering, FACE_W, FACE_H);
        atlas.put(0, &px);
        self.face = Some(atlas);
        self.face_for = Some((spec.clone(), key));
    }

    /// Which card is enlarged (or turning back), if one is.
    pub fn opened(&self) -> Option<usize> {
        self.opened
    }

    /// Let go of the pictures (Home in another style).
    pub fn forget(&mut self) {
        (self.cards, self.cards_for, self.face, self.face_for, self.painting) = (None, None, None, None, None);
        self.glow_placed = false;
        self.arrive_when_ready = false;
    }

    // --------------------------------------------------------------- scene ---

    /// The frame as it stands: every card in view placed, back to front.
    /// `colours`: each card's glow colour; `press`: Enter landing (1 as it lands).
    pub fn scene(&mut self, g: &mut Gfx, count: usize, focus: usize, colours: &[Color], press: f32) -> Scene {
        let dark = g.tokens.tone == Tone::Dark;
        let cy = (g.top() + 2.0 + CARD as f32 * 0.5).round();
        let cx = W * 0.5 + g.ox;
        let shown = rubber(self.pos.value(), count);
        let open = self.open.value();
        let o = open.clamp(0.0, 1.0);
        self.middle = Rect::new(W * 0.5 - CARD as f32 * 0.5 + g.ox, cy - CARD as f32 * 0.5, CARD as f32, CARD as f32);
        let face_cy = (cy - CARD as f32 * 0.5 + FACE_H as f32 * 0.5).round();
        self.enlarged = Rect::new(W * 0.5 - FACE_W as f32 * 0.5 + g.ox, face_cy - FACE_H as f32 * 0.5, FACE_W as f32, FACE_H as f32);

        // The glow: the colour of the card in the middle, between two as it turns.
        let at = shown.clamp(0.0, count.saturating_sub(1) as f32);
        let (i0, i1) = (at.floor() as usize, (at.ceil() as usize).min(count.saturating_sub(1)));
        let pick = |i: usize| colours.get(i).copied().unwrap_or(g.tokens.signal);
        let want = pick(i0).mix(pick(i1), at - at.floor());
        let want = [want.0 as f32, want.1 as f32, want.2 as f32];
        for (spring, to) in self.glow.iter_mut().zip(want) {
            if !self.glow_placed || g.calm || g.focus_static {
                spring.jump(to);
            } else if spring.target() != to {
                spring.set(to);
                g.started_moving();
            }
        }
        self.glow_placed = true;
        let glow = Color(self.glow[0].value().round().clamp(0.0, 255.0) as u8, self.glow[1].value().round().clamp(0.0, 255.0) as u8, self.glow[2].value().round().clamp(0.0, 255.0) as u8);

        let mut cards = Vec::new();
        let first = (shown - REACH).floor().max(0.0) as usize;
        let last = ((shown + REACH).ceil().max(0.0) as usize).min(count.saturating_sub(1));
        for i in first..=last.min(count.saturating_sub(1)) {
            if count == 0 {
                break;
            }
            let d = i as f32 - shown;
            let a = d.abs();
            let k = a.min(1.0);
            let sign = d.signum();
            // Across: from the middle out to the first side place, then a
            // card's spacing each. The curve meets the spacing's slope where
            // the sides begin, so the cards never jerk as they pass; and it
            // leaves the middle slowly while the card turns, so the two
            // cards either side of the middle overlap all the way across,
            // with no gap between them halfway.
            let x = sign * if a < 1.0 { FIRST * hermite(k, 0.0, SPACING / FIRST) } else { FIRST + (a - 1.0) * SPACING };
            let z = SIDE_DEPTH * smooth(k);
            // A card to the left turns its right edge away from the eye.
            let angle = -sign * SIDE_ANGLE * hermite(k, 1.0, 0.0);
            let reflect = (1.0 - (a - REFLECTED) * 0.5).clamp(0.0, 1.0);
            let mut p = Placed { atlas: 0, sprite: i, x, y: 0.0, z, angle, w: CARD as f32, h: CARD as f32, mirror: false, shade: 1.0, fog: 0.0, reflect };
            if self.opened == Some(i) && o > 0.0 {
                // Coming forward, turning over and growing to the back's size.
                let turn = std::f32::consts::PI * open;
                p.angle = angle * (1.0 - o) + turn;
                p.x = x * (1.0 - o);
                p.z = z * (1.0 - o) - LIFT * (std::f32::consts::PI * o).sin();
                p.y = (face_cy - cy) * o;
                p.w = CARD as f32 + (FACE_W - CARD) as f32 * o;
                p.h = CARD as f32 + (FACE_H - CARD) as f32 * o;
                p.reflect *= 1.0 - o;
                if p.angle.cos() < 0.0 {
                    (p.atlas, p.sprite, p.mirror) = (1, 0, true);
                }
            } else if o > 0.0 && self.opened.is_some() {
                // The others make way, and step back into the dark.
                let side = if self.opened.is_some_and(|at| i < at) { -1.0 } else { 1.0 };
                p.x += side * MAKE_WAY * o;
                p.reflect *= 1.0 - 0.6 * o;
            }
            if i == self.opened.unwrap_or(focus) {
                // Enter landing: the card gives a little.
                let squash = 1.0 - 0.04 * press;
                (p.w, p.h) = (p.w * squash, p.h * squash);
            }
            let aside = self.opened.is_some_and(|at| at != i);
            p.shade = light(p.angle, p.mirror) * if aside { 1.0 - 0.4 * o } else { 1.0 };
            p.fog = ((p.z / SIDE_DEPTH).clamp(0.0, 1.0) * if dark { 0.3 } else { 0.45 } + if aside { 0.25 * o } else { 0.0 }).min(0.9);
            cards.push((if self.opened == Some(i) { -1.0 } else { a }, p));
        }
        // Back to front: the farthest first, the enlarged card last of all.
        cards.sort_by(|a, b| b.0.total_cmp(&a.0));
        let t = g.tokens;
        // The enlarged card, lifted off the ground, casts a soft shadow on it
        // (a little below, as the light comes from above).
        // It is as wide as the card shows itself: none while it is edge-on.
        let shadow = self.opened.filter(|_| o > 0.01).map(|_| {
            let r = self.middle.lerp(&self.enlarged, o).offset(0.0, 4.0);
            let shows = (std::f32::consts::PI * open).cos().abs();
            let r = Rect::new(r.cx() - r.w * shows * 0.5, r.y, r.w * shows, r.h);
            (r, 12.0, o * shows * if dark { 0.55 } else { 0.22 })
        });
        Scene {
            cx,
            cy,
            cols: ((g.ox.floor() as i32).max(0), ((W + g.ox).ceil() as i32).min(W as i32)),
            ground: t.ground,
            glow,
            // While a card is enlarged the glow steps back, so the card stands out.
            glow_strength: if dark { 0.5 } else { 0.32 } * (1.0 - 0.3 * o),
            glow_at: (cx, cy - 6.0),
            glow_reach: (230.0, 150.0),
            floor_dim: if dark { 0.42 } else { 0.08 },
            floor_reach: CARD as f32 * 0.5 + 34.0,
            reflection: if dark { 0.34 } else { 0.26 },
            shadow,
            cards: cards.into_iter().map(|(_, p)| p).collect(),
        }
    }
}

/// Smoothstep: 0 to 1 with no speed at either end.
fn smooth(k: f32) -> f32 {
    k * k * (3.0 - 2.0 * k)
}

/// A curve from 0 to 1 over `k` in 0..1, leaving 0 with slope `m0` and
/// arriving at 1 with slope `m1` (a cubic Hermite).
fn hermite(k: f32, m0: f32, m1: f32) -> f32 {
    let (k2, k3) = (k * k, k * k * k);
    m0 * (k3 - 2.0 * k2 + k) + (-2.0 * k3 + 3.0 * k2) + m1 * (k3 - k2)
}

/// How lit a card is, turned `angle` (seen from behind when `mirror`): the
/// one light rests above and to the left, so a card turned to face it is
/// brightest and one turned away from it darkest. 1 facing the eye.
pub fn light(angle: f32, mirror: bool) -> f32 {
    // The way the seen face points, across and toward the eye.
    let (mut nx, mut nz) = (angle.sin(), -angle.cos());
    if mirror {
        (nx, nz) = (-nx, -nz);
    }
    // Toward the light: to the left and toward the eye.
    let (lx, lz) = (-0.5f32, -0.866f32);
    let facing = (nx * lx + nz * lz).max(0.0);
    ((0.7 + 0.3 * facing) / (0.7 + 0.3 * 0.866)).min(1.0)
}

/// What the cards' lettering depends on, for knowing when to make them again.
fn lettering_key(g: &Gfx) -> u64 {
    let size = g.text_size as u64;
    let theme = g.theme as u64;
    (size << 8) | (theme << 4) | ((g.radius(10.0) * 4.0) as u64) << 16 | ((g.look.letters as u64) << 32) | ((g.look.width as u64) << 40)
}

/// How a card's words are set at this text size and look.
pub fn lettering(g: &Gfx) -> Lettering {
    let ty = g.ty;
    let bezel = g.theme == crate::look::Theme::Bezel;
    // A long name first goes a size down, then a touch narrower (Google Sans
    // Flex's width axis): fitted, never cut.
    let narrow = |s: TextStyle| TextStyle { wdth: 90.0, ..s };
    let names = if bezel {
        [TextStyle::label(ty.label.size + 1.0), TextStyle::label(ty.label.size), TextStyle::label(ty.caption.size)]
    } else {
        [TextStyle::sans(ty.name.size, 650.0), TextStyle::sans(ty.body.size, 650.0), narrow(TextStyle::sans(ty.caption.size + 1.0, 650.0))]
    };
    Lettering {
        names,
        lowercase: bezel,
        radius: g.radius(10.0),
        // The mono voice runs wider: its names sit a little nearer the edge.
        margin: if bezel { 8.0 } else { 10.0 },
        title: if bezel { TextStyle::label(ty.label.size + 1.0) } else { ty.title },
        words: ty.name,
        reading: ty.display(30.0),
        caption: ty.caption,
    }
}

// ------------------------------------------------------------- the apps ---

/// Each built-in app's hue (0 to 71, every 5°): its own colour, chosen for
/// what it is and so that neighbours on Home differ.
pub fn hue_of(name: &str, icon: &str) -> u8 {
    let degrees: u32 = match name {
        "Robot" => 350,
        "Health" => 150,
        "Logs" => 40,
        "Schedule" => 210,
        "Tables" => 250,
        "Loop" => 185,
        "Motors" => 20,
        "Console" => 195,
        "Terminal" => 130,
        "Files" => 45,
        "Git" => 10,
        "Catalogue" => 270,
        "Monitor" => 165,
        "Net tools" => 200,
        "Connect" => 230,
        "Actions" => 85,
        "Camera" => 335,
        "Photos" => 300,
        "AprilTag" => 220,
        "Graphs" => 280,
        "Pit" => 30,
        "Remote" => 355,
        "Level" => 100,
        "Compass" => 215,
        "Weather" => 195,
        "Browse" => 170,
        "Calculator" => 25,
        "Clock" => 240,
        "Notes" => 50,
        "Reminders" => 5,
        "Units" => 60,
        "HTTP" => 290,
        "Share" => 185,
        "Assistant" => 265,
        "Web" => 220,
        "Music" => 345,
        "Settings" => 205,
        // Another app: from its icon, else its name, always the same.
        _ => {
            let seed = if icon.is_empty() { name } else { icon };
            let h = seed.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            (h % 72) * 5
        }
    };
    (degrees / 5) as u8
}

/// What a built-in app does, in a few plain words, for the back of its card.
pub fn about(name: &str) -> Option<&'static str> {
    Some(match name {
        "Robot" => "the link, its battery and the Systemcore",
        "Health" => "faults and warnings as they happen",
        "Logs" => "match logs, summed up after each match",
        "Schedule" => "your matches, and when to queue",
        "Tables" => "every NetworkTables value, live",
        "Loop" => "how long each robot loop takes",
        "Motors" => "every motor, and any that drop out",
        "Console" => "the robot's own console log",
        "Terminal" => "a shell on this handheld",
        "Files" => "the storage: open, edit and move",
        "Git" => "repositories, history, diffs and push",
        "Catalogue" => "more apps to download and keep",
        "Monitor" => "processor, memory and processes",
        "Net tools" => "find hosts, ping and ports",
        "Connect" => "SSH and serial to other machines",
        "Actions" => "saved commands, one key each",
        "Camera" => "photos, and QR codes read",
        "Photos" => "the photos taken here",
        "AprilTag" => "AprilTags read with the camera",
        "Graphs" => "robot values drawn over time",
        "Pit" => "the pit checklist and match timer",
        "Remote" => "an infrared universal remote",
        "Level" => "pitch and roll from the motion sensor",
        "Compass" => "a heading from the magnetometer",
        "Weather" => "the forecast where you are",
        "Browse" => "the web, read for you",
        "Calculator" => "sums, and a programmer's mode",
        "Clock" => "timers, a stopwatch and alarms",
        "Notes" => "notes for the pit",
        "Reminders" => "reminders that come when due",
        "Units" => "torque, length and weight converted",
        "HTTP" => "requests sent, replies read",
        "Share" => "anything as a QR code",
        "Assistant" => "ask; it uses the robot's tools",
        "Web" => "pages as text, in lynx",
        "Music" => "YouTube Music, playing on behind",
        "Settings" => "how this handheld looks and works",
        _ => return None,
    })
}

/// Flow's own icon for Settings › Home: the style as a picture, a card in
/// the middle and two turned either side, in `r`.
pub fn swatch(g: &mut Gfx, r: Rect, ink: Color, accent: Color) {
    let c = &mut g.canvas;
    let (cx, top) = (r.cx() + g.ox, r.y);
    // A square cover in the middle, its reflection under it, and a turned
    // one either side: tall at its outer edge, short at its inner.
    let s = (r.h * 0.72).round();
    let cover = Rect::new((cx - s * 0.5).round(), top, s, s);
    for side in [-1.0f32, 1.0] {
        let (xi, xo) = (cx + side * (s * 0.5 + 1.0), cx + side * (s * 0.5 + 5.0));
        let (inner, outer) = (s * 0.55, s * 0.95);
        let mid = top + s * 0.5;
        c.polygon(&[(xi, mid - inner * 0.5), (xo, mid - outer * 0.5), (xo, mid + outer * 0.5), (xi, mid + inner * 0.5)], ink, 0.7);
    }
    c.rrect(cover, 1.5, accent, 1.0);
    c.rrect(Rect::new(cover.x, cover.bottom() + 1.0, s, r.bottom() - cover.bottom() - 1.0), 1.0, accent, 0.3);
}

#[cfg(test)]
mod tests;
