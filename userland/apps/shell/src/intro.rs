//! The start: the panel comes up out of black on the logo's ground, a trail
//! of light sweeps in and strikes the middle, the Catalyst mark ignites
//! there, and the name sets itself beneath it. Then it gives way to Home, or
//! on a first start to setting the handheld up.
//!
//! Everything that arrives does so on Detent's springs, by Bezel's roles:
//! the mark on `release` (it lands with a little life in it), the mark
//! making room for the name on `settle`, the words on `release`, staggered,
//! and the whole thing leaving on `smooth`. The one exception is the trail,
//! which is a thing thrown, not a thing placed: it accelerates into the
//! mark rather than easing to a stop. Any key skips to the end.

use bezel::Color;
use detent::{role, Animated};

use crate::brand::{self, CRIMSON, INK_FAINT};
use crate::gfx::{Gfx, H, W};

/// When the trail strikes, and the mark ignites.
const STRIKE: f32 = 0.52;
/// When the trail leaves the lower left.
const THROWN: f32 = 0.10;
/// When the mark moves up to make room for the name.
const MAKE_ROOM: f32 = 1.05;
/// When the name's two words arrive, and the rule under them.
const NAME: f32 = 1.16;
const THIN: f32 = 1.25;
const RULE: f32 = 1.34;
const CAPTION: f32 = 1.55;
/// When an ordinary start gives way to Home.
pub const DONE: f32 = 2.45;

/// The mark's place and size: first in the middle, struck; then above the name.
const STRUCK: (f32, f32, f32) = (W * 0.5, H * 0.5 - 2.0, 60.0);
const RESTING: (f32, f32, f32) = (W * 0.5, 54.0, 44.0);
const NAME_CY: f32 = 101.0;
const RULE_Y: f32 = 117.5;
const CAPTION_CY: f32 = 135.0;

pub struct Intro {
    t: f32,
    /// 1 while it covers the panel, going to 0 as it gives way.
    shown: Animated,
    leaving: bool,
    ground: Vec<u32>,
    /// The words under the name: what this is, or what comes next.
    caption: String,
}

/// How far a spring of this role, set going at `start`, has come by `t`: 0
/// before it starts, 1 at rest, past 1 where it overshoots.
fn arrived(spring: detent::Spring, start: f32, t: f32) -> f32 {
    if t <= start {
        return 0.0;
    }
    1.0 - spring.solve(1.0, 0.0, t - start).0
}

/// A point on the trail's path at `u` (0 at the lower left, 1 at the mark):
/// a cubic curve that sweeps up into the middle, as the motion profile does
/// on FrcCatalyst's banner.
fn path(u: f32) -> (f32, f32) {
    let p = [(-36.0, H + 6.0), (70.0, H + 4.0), (122.0, 118.0), (STRUCK.0, STRUCK.1)];
    let v = 1.0 - u;
    let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
    (a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0, a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1)
}

impl Intro {
    pub fn new(caption: impl Into<String>) -> Self {
        Intro { t: 0.0, shown: Animated::new(1.0, role::smooth()).with_rest(0.002), leaving: false, ground: brand::ground(), caption: caption.into() }
    }

    /// Advance. True while anything moves.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.t += dt;
        let moving = self.shown.tick(dt);
        moving || !self.leaving || self.shown.value() > 0.0
    }

    /// Whether it has played through to the end of the name.
    pub fn finished(&self) -> bool {
        self.t >= DONE
    }

    /// Jump to where everything has arrived: a key pressed during it.
    pub fn skip(&mut self) {
        self.t = self.t.max(DONE);
    }

    /// Give way to what is underneath.
    pub fn leave(&mut self) {
        self.skip();
        self.leaving = true;
        self.shown.set(0.0);
    }

    pub fn leaving(&self) -> bool {
        self.leaving
    }

    /// Gone: nothing of it is left on the panel.
    pub fn gone(&self) -> bool {
        self.leaving && self.shown.value() <= 0.004
    }

    /// The mark's place and height just now.
    pub fn mark_at(&self) -> (f32, f32, f32) {
        let k = arrived(role::settle(), MAKE_ROOM, self.t);
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        (lerp(STRUCK.0, RESTING.0), lerp(STRUCK.1, RESTING.1), lerp(STRUCK.2, RESTING.2))
    }

    pub fn draw(&self, g: &mut Gfx) {
        let t = self.t;
        let all = self.shown.value().clamp(0.0, 1.0);
        // The ground, coming up out of black over the first third of a second.
        let rise = (t / 0.3).clamp(0.0, 1.0);
        let lift = rise * rise * (3.0 - 2.0 * rise);
        let px = &mut g.canvas.px;
        if all >= 1.0 && lift >= 1.0 {
            px.copy_from_slice(&self.ground);
        } else {
            let f = (lift * 256.0) as u32;
            for (p, &ground) in px.iter_mut().zip(&self.ground) {
                let shade = scale(ground, f);
                *p = if all >= 1.0 { shade } else { mix(*p, shade, (all * 256.0) as u32) };
            }
        }

        // The trail: thrown from the lower left, quickening into the middle.
        let flight = ((t - THROWN) / (STRIKE - THROWN)).clamp(0.0, 1.0);
        let head = flight.powf(1.7);
        let after = (t - STRIKE).max(0.0);
        // Swallowed by the strike: gone within two frames of it.
        let trail_fade = (1.0 - after / 0.05).clamp(0.0, 1.0);
        if t > THROWN && trail_fade > 0.0 {
            let tail = (head - 0.42).max(0.0);
            const PIECES: usize = 22;
            let mut last = path(tail);
            for i in 1..=PIECES {
                let k = i as f32 / PIECES as f32;
                let at = path(tail + (head - tail) * k);
                // Thin and faint at the tail, full at the head, with a soft
                // light around its last stretch.
                let width = 0.6 + 2.4 * k;
                if k > 0.7 {
                    g.canvas.polyline(&[last, at], width + 4.0, CRIMSON, all * trail_fade * (k - 0.7) * 0.5);
                }
                g.canvas.polyline(&[last, at], width, CRIMSON.mix(Color::WHITE, 0.3 * k), all * trail_fade * k * k * 0.95);
                last = at;
            }
            if after == 0.0 {
                let (hx, hy) = path(head);
                g.canvas.glow(hx, hy, 11.0, CRIMSON, all * 0.7);
                g.canvas.circle(hx, hy, 1.7, Color::WHITE, all);
            }
        }
        // The motion profile's dots: lit as the trail passes, then fading.
        for (i, u) in [0.2f32, 0.4, 0.6].into_iter().enumerate() {
            if head < u {
                continue;
            }
            let lit = ((head - u) / 0.08).clamp(0.0, 1.0);
            let fade = (1.0 - (after - 0.25 - i as f32 * 0.08).max(0.0) / 0.7).clamp(0.0, 1.0);
            let (x, y) = path(u);
            g.canvas.circle(x, y, 1.4, CRIMSON.mix(Color::WHITE, 0.35), all * lit * fade * 0.85);
        }

        // The strike: a flash, a ring going out, and the mark's own glow.
        let (cx, cy, height) = self.mark_at();
        if t >= STRIKE {
            let flash = (-after / 0.13).exp();
            g.canvas.glow(STRUCK.0, STRUCK.1, 96.0, CRIMSON.mix(Color::WHITE, 0.3 * flash), all * 0.8 * flash);
            let ring = arrived(role::smooth(), STRIKE, t).clamp(0.0, 1.0);
            if ring < 0.995 {
                let fade = (1.0 - ring).powf(1.5);
                g.canvas.arc(STRUCK.0, STRUCK.1, 14.0 + 74.0 * ring, 1.4, 0.0, std::f32::consts::TAU, CRIMSON.mix(Color::WHITE, 0.2), all * 0.5 * fade);
            }
            // The logo's glow: crimson at 0.30 in the middle, gone at its edge.
            let resting = arrived(role::settle(), STRIKE, t).clamp(0.0, 1.0);
            g.canvas.glow(cx, cy, height * 1.45, CRIMSON, all * 0.30 * resting);
            // The mark: lands on `release`, white-hot at first, cooling to crimson.
            let landed = arrived(role::release(), STRIKE, t);
            let size = height * (0.55 + 0.45 * landed);
            let heat = (-after / 0.16).exp();
            brand::mark(g, cx, cy, size, CRIMSON.mix(Color::hex(0xfff2f4), heat), all);
        }

        // The name: "Catalyst", then "Linux", each rising into place.
        let word = |start: f32| {
            let k = arrived(role::release(), start, t);
            ((k * 1.6).clamp(0.0, 1.0) * all, ((1.0 - k) * 9.0).round())
        };
        let (name, thin) = (word(NAME), word(THIN));
        brand::wordmark(g, W * 0.5, NAME_CY, name, thin);
        let drawn = arrived(role::settle(), RULE, t);
        if drawn > 0.0 {
            let (width, _) = brand::wordmark_width(g);
            brand::rule(g, (W - width) * 0.5, RULE_Y, width, drawn, all);
        }
        let caption = arrived(role::smooth(), CAPTION, t).clamp(0.0, 1.0);
        if caption > 0.0 {
            let style = bezel::TextStyle::code(9.0);
            let w = g.measure(&self.caption, style);
            g.text_alpha((W - w) * 0.5, CAPTION_CY, &self.caption, style, INK_FAINT, caption * all);
        }
    }
}

/// A pixel darkened toward black: `f` in 256ths of the way to full.
fn scale(p: u32, f: u32) -> u32 {
    let rb = ((p & 0x00ff_00ff) * f >> 8) & 0x00ff_00ff;
    let g = ((p & 0x0000_ff00) * f >> 8) & 0x0000_ff00;
    rb | g
}

/// `a` toward `b` by `f` 256ths.
fn mix(a: u32, b: u32, f: u32) -> u32 {
    let rb = (((a & 0x00ff_00ff) * (256 - f) + (b & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let g = (((a & 0x0000_ff00) * (256 - f) + (b & 0x0000_ff00) * f) >> 8) & 0x0000_ff00;
    rb | g
}
