//! Drawing the waters: ten by ten squares on a tonal tile, at any size
//! between the large grid you aim at and the small one beside it (a grid in
//! between is one on its way from one to the other).
//!
//! Colour roles: water is a tonal surface; your ships afloat are ice (what
//! is fine); a hit is the signal (heat); a sunk ship is the signal cooled
//! into the water, one shape where it lay; a miss is a faint dot.

use catalyst_kit::detent::Spring;
use catalyst_kit::prelude::*;
use catalyst_kit::Tokens;

use crate::game::{Board, Mark, Ship, Shot, N};

/// The large grid's square: 12 px with a 2 px gap.
pub const BIG: f32 = 14.0;
/// The small grid's square: 6 px with a 1 px gap.
pub const SMALL: f32 = 7.0;
/// How long a shot's landing plays.
pub const SPLASH: f32 = 0.6;

/// Where a grid's squares lie, and their size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// The top-left square's top-left corner.
    pub x: f32,
    pub y: f32,
    pub pitch: f32,
    pub gap: f32,
    /// From the squares to the tile's edge.
    pub pad: f32,
    /// A square's corners.
    pub radius: f32,
}

impl Grid {
    /// The large grid, its tile's corner at (x, y).
    pub fn big(x: f32, y: f32, ui: &Ui) -> Grid {
        Grid { x: x + 3.0, y: y + 3.0, pitch: BIG, gap: 2.0, pad: 3.0, radius: ui.radius(3.0) }
    }

    /// The small grid, its tile's corner at (x, y).
    pub fn small(x: f32, y: f32, ui: &Ui) -> Grid {
        Grid { x: x + 2.0, y: y + 2.0, pitch: SMALL, gap: 1.0, pad: 2.0, radius: ui.radius(1.5) }
    }

    /// The tile's size for a square of `pitch`.
    pub fn extent(pitch: f32) -> f32 {
        if pitch >= BIG {
            N as f32 * BIG - 2.0 + 6.0
        } else {
            N as f32 * SMALL - 1.0 + 4.0
        }
    }

    pub fn lerp(&self, o: &Grid, t: f32) -> Grid {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Grid { x: l(self.x, o.x), y: l(self.y, o.y), pitch: l(self.pitch, o.pitch), gap: l(self.gap, o.gap), pad: l(self.pad, o.pad), radius: l(self.radius, o.radius) }
    }

    pub fn moved(self, dx: f32, dy: f32) -> Grid {
        Grid { x: self.x + dx, y: self.y + dy, ..self }
    }

    pub fn tile(&self) -> Rect {
        let side = N as f32 * self.pitch - self.gap + 2.0 * self.pad;
        Rect::new(self.x - self.pad, self.y - self.pad, side, side)
    }

    /// Concentric with the squares' corners.
    pub fn tile_radius(&self) -> f32 {
        self.radius + self.pad
    }

    /// The squares from (x, y), `w` across and `h` down, as one shape.
    pub fn span(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(self.x + x * self.pitch, self.y + y * self.pitch, w * self.pitch - self.gap, h * self.pitch - self.gap)
    }

    pub fn cell(&self, x: usize, y: usize) -> Rect {
        self.span(x as f32, y as f32, 1.0, 1.0)
    }

    pub fn ship(&self, s: Ship) -> Rect {
        let (w, h) = s.size();
        self.span(s.x as f32, s.y as f32, w as f32, h as f32)
    }
}

/// A capsule: half its short side.
pub fn capsule(r: Rect) -> f32 {
    r.w.min(r.h) * 0.5
}

/// A ship gone down: the signal cooled into the water.
pub fn sunk_color(t: &Tokens) -> Color {
    t.signal.mix(t.surface2, 0.5)
}

/// Your ships afloat: ice on dark, ice's own ink on light (ice alone is too
/// pale on a light tile to read as a shape).
pub fn afloat_color(ui: &Ui) -> Color {
    ui.good()
}

/// A shot landing: where, what it found, and how long ago.
#[derive(Clone, Copy, Debug)]
pub struct Splash {
    pub x: usize,
    pub y: usize,
    pub shot: Shot,
    pub age: f32,
}

impl Splash {
    pub fn new(x: usize, y: usize, shot: Shot) -> Splash {
        Splash { x, y, shot, age: 0.0 }
    }

    pub fn at(&self, x: usize, y: usize) -> bool {
        self.x == x && self.y == y
    }

    /// 0 as it lands to 1 when it has played out.
    pub fn progress(&self) -> f32 {
        (self.age / SPLASH).clamp(0.0, 1.0)
    }
}

/// Where a spring started at `from` (towards 1) is after `t` seconds.
pub fn sprung(spring: Spring, from: f32, t: f32) -> f32 {
    1.0 + spring.solve(from - 1.0, 0.0, t.max(0.0)).0
}

/// What to draw over the waters.
pub struct Waters<'a> {
    pub board: &'a Board,
    /// Your own waters: the ships show.
    pub own: bool,
    pub alpha: f32,
    /// The other side's ships not yet found, outlined (as a game ends).
    pub reveal: f32,
    pub splash: Option<&'a Splash>,
    /// Seconds since each ship was placed, for it to drop into place.
    pub placed: Option<&'a [f32; 5]>,
    /// The square its last shot hit, ringed.
    pub last: Option<(usize, usize)>,
}

/// How far into place a ship just placed is: its scale and opacity.
fn drop_in(age: f32) -> (f32, f32) {
    if age >= 0.6 {
        return (1.0, 1.0);
    }
    if age < 0.0 {
        return (1.0, 0.0);
    }
    let p = sprung(role::release(), 0.0, age);
    (1.0 + 0.18 * (1.0 - p), (age / 0.12).min(1.0))
}

pub fn draw(ui: &mut Ui, g: &Grid, w: &Waters) {
    let t = ui.tokens;
    let (ox, alpha) = (ui.ox, w.alpha);
    if alpha <= 0.0 {
        return;
    }
    let afloat = afloat_color(ui);
    let sunk = sunk_color(&t);
    let board = w.board;
    let c = &mut ui.canvas;
    c.rrect(g.tile().offset(ox, 0.0), g.tile_radius(), t.surface1, alpha);
    let splash_at = |x: usize, y: usize| w.splash.filter(|s| s.at(x, y));
    let dot = (g.pitch * 0.12).max(1.1);
    // The water, the misses, and (in the other side's waters) the hits.
    for y in 0..N {
        for x in 0..N {
            let r = g.cell(x, y).offset(ox, 0.0);
            match board.mark(x, y) {
                Mark::Unknown => c.rrect(r, g.radius, t.surface2, alpha),
                Mark::Miss => {
                    let grow = splash_at(x, y).map_or(1.0, |s| sprung(role::settle(), 0.0, s.age));
                    c.circle(r.cx(), r.cy(), dot * grow, t.faint, alpha);
                }
                Mark::Hit if !w.own => {
                    let grow = splash_at(x, y).map_or(1.0, |s| sprung(spatial::fast(), 0.4, s.age));
                    let r = r.scaled(grow);
                    c.rrect(r, g.radius * grow, t.signal, alpha);
                }
                Mark::Hit | Mark::Sunk => {}
            }
        }
    }
    // The ships: yours afloat, any gone down, and theirs revealed at the end.
    for (i, ship) in board.ships().iter().enumerate() {
        let r = g.ship(*ship).offset(ox, 0.0);
        let radius = capsule(r);
        if board.is_sunk(*ship) {
            // Going down, the heat cools into the water.
            let heat = w.splash.filter(|s| s.shot == Shot::Sunk(ship.kind)).map_or(0.0, |s| 1.0 - s.progress());
            c.rrect(r, radius, sunk.mix(t.signal, heat), alpha);
        } else if w.own {
            let (scale, a) = w.placed.map_or((1.0, 1.0), |ages| drop_in(ages[i]));
            c.rrect(r.scaled(scale), radius * scale, afloat, alpha * a);
        } else if w.reveal > 0.0 {
            c.stroke_rrect(r.inset(-0.5), radius + 0.5, 1.0, t.dim, alpha * w.reveal);
        }
    }
    // Hits on your ships still afloat, over them.
    if w.own {
        for ship in board.ships().iter().filter(|s| !board.is_sunk(**s)) {
            for (x, y) in ship.cells().filter(|(x, y)| board.fired(*x, *y)) {
                let grow = splash_at(x, y).map_or(1.0, |s| sprung(spatial::fast(), 0.4, s.age));
                let r = g.cell(x, y).offset(ox, 0.0).scaled(grow);
                c.rrect(r, g.radius * grow, t.signal, alpha);
            }
        }
    }
    // The last shot that came in, ringed until the next.
    if let Some((x, y)) = w.last {
        let r = g.cell(x, y).offset(ox, 0.0).inset(-1.5);
        c.stroke_rrect(r, g.radius + 1.5, 1.0, t.ink, alpha);
    }
    // The landing: a ring spreading from a hit or a sinking, a ripple from a miss.
    if let Some(s) = w.splash {
        let p = s.progress();
        if p < 1.0 {
            let fade = (1.0 - p) * (1.0 - p);
            let cell = g.cell(s.x, s.y).offset(ox, 0.0);
            match s.shot {
                Shot::Miss => {
                    let r = cell.scaled(0.6 + 1.0 * p);
                    c.stroke_rrect(r, r.w * 0.5, 1.0, t.dim, alpha * fade * 0.9);
                }
                Shot::Hit(_) => {
                    let r = cell.inset(-1.0 - g.pitch * 0.45 * p);
                    c.stroke_rrect(r, g.radius + 1.0 + g.pitch * 0.45 * p, 1.5, t.signal, alpha * fade);
                }
                Shot::Sunk(kind) => {
                    if let Some(ship) = board.ships().iter().find(|sh| sh.kind == kind) {
                        let grow = 1.0 + g.pitch * 0.5 * p;
                        let r = g.ship(*ship).offset(ox, 0.0).inset(-grow);
                        c.stroke_rrect(r, capsule(r), 1.5, t.signal, alpha * fade);
                    }
                }
            }
        }
    }
}
