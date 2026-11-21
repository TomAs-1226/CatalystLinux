//! A universe running, on the whole panel: its bodies as soft dots with
//! fading trails, a quiet line at the top saying where you are, the keys
//! at the bottom, and an island for what just changed.
//!
//! The view is a camera over the universe: what it looks at (the centre of
//! mass, or a body it follows), how far it is zoomed (in half-doublings of
//! the universe's opening frame), and how far it still has to glide (a pan
//! in pixels that springs to nothing). Every change of view moves on a
//! Detent spring from wherever the view is, so a key pressed mid-glide
//! turns it; in the calm version it lands at once.

use catalyst_kit::bezel::{canvas::blend, Color, GlassShape, Tone};
use catalyst_kit::prelude::*;
use catalyst_kit::Theme;

use crate::help::Help;
use crate::sim::{radius_for_mass, visual_radius_for_mass, Hue, Sim, MAX_BODIES, TRAIL};
use crate::universe::{Universe, SPEEDS};

/// The middle of the view on the panel: a pixel below centre, between the
/// line at the top and the hint at the bottom.
const VIEW_X: f32 = W * 0.5;
const VIEW_Y: f32 = H * 0.5 + 1.0;
/// The room a universe's opening frame is fitted into, either side of the middle.
const FIT_W: f64 = 150.0;
const FIT_H: f64 = 62.0;
/// How far the zoom goes, in half-doublings from the opening frame:
/// out to a thirty-second (the Solar System's Neptune), in to sixteen times.
const ZOOM_OUT: f32 = -10.0;
const ZOOM_IN: f32 = 8.0;
/// How long the island's words stay: the kit's.
const MESSAGE_SECONDS: f32 = 2.4;
/// How far a trail may turn before it is let go: 300°.
const TRAIL_TURN: f32 = 300.0 * std::f32::consts::PI / 180.0;

/// Names for bodies set loose: minor planets, in order of discovery or near it.
const NAMES: [&str; 16] = ["Ceres", "Vesta", "Pallas", "Juno", "Hygiea", "Eris", "Sedna", "Haumea", "Makemake", "Quaoar", "Orcus", "Varuna", "Ixion", "Chaos", "Salacia", "Gonggong"];

/// A body's colour, as AstroBox gives it on dark space; on a light ground,
/// each is taken deep enough to stand on the page (3:1 or more against the
/// light grounds), white becoming graphite. Bodies are content, like a
/// photograph; everything around them is drawn from the tokens.
fn body_color(hue: Hue, tone: Tone) -> Color {
    let (dark, light) = match hue {
        Hue::White => (0xffffff, 0x4a525c),
        Hue::Yellow => (0xffe66d, 0xa87800),
        Hue::Orange => (0xff8c42, 0xd0560f),
        Hue::Blue => (0x2d8cff, 0x1d64d8),
        Hue::Teal => (0x4ecdc4, 0x0f8178),
        Hue::Red => (0xff4d4d, 0xc8302f),
    };
    Color::hex(if tone == Tone::Dark { dark } else { light })
}

/// A body's radius on the panel. Far out, a marker that grows a little with
/// mass, so a planet is never lost; close in, its drawn size in the
/// universe (AstroBox's visual radius), up to a limit.
fn dot_radius(mass: f64, scale: f64) -> f32 {
    let marker = (1.9 * mass.max(1e-9).powf(0.14)).clamp(1.3, 5.2) as f32;
    let true_size = (visual_radius_for_mass(mass) * scale) as f32;
    marker.max(true_size.min(15.0))
}

/// "×0.25", "×1", "×10".
pub fn speed_words(speed: f64) -> String {
    format!("×{speed}")
}

pub struct Space {
    universe: Universe,
    sim: Sim,
    speed: usize,
    paused: bool,
    /// The body the camera follows, by id; none: the centre of mass.
    follow: Option<u32>,
    zoom: Animated,
    pan: (Animated, Animated),
    /// The ring round a newly followed body closing in: 1 → 0.
    lock: Animated,
    /// The body last set loose, growing into being.
    fresh: Option<(u32, Animated)>,
    island: Island,
    rng: Rng,
    born: usize,
    calm: bool,
    ink: Ink,
    /// The wallpaper along the top and bottom edges, for fading into.
    edges: Vec<u32>,
}

impl Space {
    pub fn new(universe: Universe, calm: bool) -> Space {
        Space {
            universe,
            sim: universe.load(),
            speed: universe.start_speed(),
            paused: false,
            follow: None,
            zoom: Animated::new(0.0, spatial::default()).with_rest(0.002),
            pan: (Animated::new(0.0, role::smooth()).with_rest(0.05), Animated::new(0.0, role::smooth()).with_rest(0.05)),
            lock: Animated::new(0.0, role::release()).with_rest(0.01),
            fresh: None,
            island: Island::new(),
            rng: Rng::seeded(),
            born: 0,
            calm,
            ink: Ink::new(),
            edges: Vec::with_capacity((W * 60.0) as usize),
        }
    }

    pub fn followed(&self) -> Option<usize> {
        self.follow.and_then(|id| self.sim.index_of(id))
    }

    // ---- the camera ----

    fn base_scale(&self) -> f64 {
        let (hx, hy) = self.universe.frame();
        (FIT_W / hx).min(FIT_H / hy)
    }

    /// Pixels per unit of the universe, now.
    pub fn scale(&self) -> f64 {
        self.base_scale() * 2f64.powf(self.zoom.value() as f64 * 0.5)
    }

    fn looking_at(&self) -> (f64, f64) {
        match self.followed() {
            Some(i) => self.sim.pos(i),
            None => self.sim.centre_of_mass(),
        }
    }

    /// The point of the universe at the middle of the view.
    pub fn camera(&self) -> (f64, f64) {
        let (x, y) = self.looking_at();
        let s = self.scale();
        (x + self.pan.0.value() as f64 / s, y - self.pan.1.value() as f64 / s)
    }

    /// Look somewhere else, gliding there from where the view is now.
    fn look_from(&mut self, camera: (f64, f64)) {
        let (x, y) = self.looking_at();
        let s = self.scale();
        let want = (((camera.0 - x) * s) as f32, (-(camera.1 - y) * s) as f32);
        for (pan, want) in [(&mut self.pan.0, want.0), (&mut self.pan.1, want.1)] {
            if self.calm || !want.is_finite() {
                pan.jump(0.0);
            } else {
                // Keep the glide's speed, measured from the new place.
                pan.shift(want - pan.value());
                pan.set(0.0);
            }
        }
    }

    fn follow_body(&mut self, id: Option<u32>) {
        let camera = self.camera();
        self.follow = id;
        self.look_from(camera);
        if id.is_some() && !self.calm {
            self.lock.jump(1.0);
            self.lock.set(0.0);
        } else {
            self.lock.jump(0.0);
        }
    }

    fn zoom_by(&mut self, steps: f32) {
        let to = self.zoom.target() + steps;
        if !(ZOOM_OUT..=ZOOM_IN).contains(&to) {
            // At the end: a small give, and back.
            if !self.calm {
                self.zoom.nudge(steps * 3.0);
            }
            return;
        }
        if self.calm {
            self.zoom.jump(to);
        } else {
            self.zoom.set(to);
        }
    }

    // ---- what the keys do ----

    fn say(&mut self, words: impl Into<String>) {
        self.island.say(words.into(), self.calm);
    }

    fn follow_next(&mut self) {
        let next = match self.followed() {
            None => (self.sim.len() > 0).then_some(0),
            Some(i) if i + 1 < self.sim.len() => Some(i + 1),
            Some(_) => None,
        };
        let id = next.map(|i| self.sim.body(i).id);
        self.follow_body(id);
        let words = match next {
            Some(i) => format!("following {}", self.sim.body(i).name.to_lowercase()),
            None => "centre of mass".to_owned(),
        };
        self.say(words);
    }

    /// A small planet, launched on a nearly circular orbit: round the body
    /// followed when it can hold one where it can be seen, otherwise round
    /// the heaviest body (near the one followed, if any).
    fn set_loose(&mut self) -> String {
        if self.sim.len() >= MAX_BODIES {
            return "no room for another body".to_owned();
        }
        let Some(primary) = self.sim.heaviest() else { return "nothing to orbit".to_owned() };
        let s = self.scale();
        let g = self.sim.config.gravity;
        let tau = std::f64::consts::TAU;
        let followed = self.followed();
        // A satellite of the followed body, if its pull can hold one far
        // enough out to see (well inside its Hill sphere).
        let satellite = followed.filter(|&h| h != primary).and_then(|h| {
            let (hx, hy) = self.sim.pos(h);
            let (px, py) = self.sim.pos(primary);
            let hill = (hx - px).hypot(hy - py) * (self.sim.mass(h) / (3.0 * self.sim.mass(primary))).cbrt();
            let mass = (self.sim.mass(h) * 1e-3).clamp(0.0005, 0.2);
            let near = 3.0 * (self.sim.radius(h) + radius_for_mass(mass));
            let d = (40.0 / s).min(0.4 * hill);
            (d >= near && d * s >= 10.0).then_some((h, d, mass))
        });
        let mut placed = None;
        for _ in 0..8 {
            let angle = self.rng.unit() as f64 * tau;
            let (sin, cos) = angle.sin_cos();
            let (host, pos, mass) = match (satellite, followed) {
                (Some((h, d, mass)), _) => {
                    let (hx, hy) = self.sim.pos(h);
                    (h, (hx + d * cos, hy + d * sin), mass)
                }
                // Beside the followed body, going round the heaviest.
                (None, Some(f)) if f != primary => {
                    let (fx, fy) = self.sim.pos(f);
                    let d = 28.0 / s;
                    (primary, (fx + d * cos, fy + d * sin), (self.sim.mass(primary) * 1e-3).clamp(0.0005, 0.2))
                }
                // Round the heaviest, somewhere in view.
                _ => {
                    let (px, py) = self.sim.pos(primary);
                    let mass = (self.sim.mass(primary) * 1e-3).clamp(0.0005, 0.2);
                    let near = 3.0 * (self.sim.radius(primary) + radius_for_mass(mass));
                    let d = ((0.35 + 0.45 * self.rng.unit() as f64) * FIT_H / s).max(near);
                    (primary, (px + d * cos, py + d * sin), mass)
                }
            };
            let clear = (0..self.sim.len()).all(|i| {
                let (x, y) = self.sim.pos(i);
                (x - pos.0).hypot(y - pos.1) > 3.0 * (self.sim.radius(i) + radius_for_mass(mass))
            });
            placed = Some((host, pos, mass));
            if clear {
                break;
            }
        }
        let Some((host, pos, mass)) = placed else { return "nothing to orbit".to_owned() };
        // Circular speed round the host, a little off so the orbit is its own,
        // turning the way everything else here turns.
        let (hx, hy) = self.sim.pos(host);
        let (hvx, hvy) = self.sim.vel(host);
        let (dx, dy) = (pos.0 - hx, pos.1 - hy);
        let d = dx.hypot(dy).max(1e-9);
        let speed = (g * (self.sim.mass(host) + mass) / d).sqrt() * (0.94 + 0.08 * self.rng.unit() as f64);
        let vel = (hvx - dy / d * speed, hvy + dx / d * speed);
        let round = self.born / NAMES.len();
        let name = match round {
            0 => NAMES[self.born % NAMES.len()].to_owned(),
            n => format!("{} {}", NAMES[self.born % NAMES.len()], n + 1),
        };
        let hue = Hue::ALL[self.rng.below(Hue::ALL.len())];
        let Some(id) = self.sim.add(name.clone(), pos, vel, mass, hue, false) else { return "no room for another body".to_owned() };
        self.born += 1;
        let mut grow = Animated::new(0.0, role::release()).with_rest(0.01);
        if self.calm {
            grow.jump(1.0);
        } else {
            grow.set(1.0);
        }
        self.fresh = Some((id, grow));
        format!("{} set loose", name.to_lowercase())
    }

    fn restart(&mut self) {
        let camera = self.camera();
        self.sim = self.universe.load();
        self.follow = None;
        self.fresh = None;
        self.born = 0;
        self.paused = false;
        self.lock.jump(0.0);
        self.look_from(camera);
        if self.calm {
            self.zoom.jump(0.0);
        } else {
            self.zoom.set(0.0);
        }
    }

    // ---- drawing ----

    /// The wallpaper under the top and bottom bands, kept to fade into.
    fn keep_edges(&mut self, ui: &Ui, top: usize, bottom: usize) {
        let w = W as usize;
        let px = &ui.canvas.px;
        self.edges.clear();
        self.edges.extend_from_slice(&px[..top * w]);
        self.edges.extend_from_slice(&px[px.len() - bottom * w..]);
    }

    /// Whatever moves under the top line and the hint fades into the
    /// wallpaper, so the words stay readable over a busy sky.
    fn fade_edges(&self, ui: &mut Ui, top: usize, bottom: usize) {
        let w = W as usize;
        let h = H as usize;
        // Wholly gone behind the words themselves, so no star sits among letters.
        let ease = |f: f32| {
            let f = f.clamp(0.0, 1.0);
            f * f * (3.0 - 2.0 * f)
        };
        let px = &mut ui.canvas.px;
        for y in 0..top {
            let a = ease((top - y) as f32 / (top as f32 * 0.55));
            let row = y * w;
            for x in 0..w {
                px[row + x] = mix(px[row + x], self.edges[row + x], a);
            }
        }
        let base = top * w;
        for k in 0..bottom {
            let y = h - bottom + k;
            let a = ease((k + 1) as f32 / (bottom as f32 * 0.55));
            for x in 0..w {
                px[y * w + x] = mix(px[y * w + x], self.edges[base + k * w + x], a);
            }
        }
    }

    fn draw_sky(&mut self, ui: &mut Ui) {
        let tone = ui.tokens.tone;
        let dark = tone == Tone::Dark;
        let s = self.scale();
        let (cx, cy) = self.camera();
        let to_screen = |x: f64, y: f64| (VIEW_X + ((x - cx) * s) as f32, VIEW_Y - ((y - cy) * s) as f32);
        let followed = self.followed();
        let phase = self.sim.trail_phase();
        // Trails first, under every body.
        let mut path = [(0.0f32, 0.0f32, 0.0f32); TRAIL + 2];
        for i in 0..self.sim.len() {
            let color = body_color(self.sim.body(i).hue, tone);
            let chosen = followed == Some(i);
            let head = if chosen { 0.75 } else if dark { 0.5 } else { 0.55 };
            let half = if chosen { 0.85 } else { 0.62 };
            // The path back from the body, as (x, y, age in trail points).
            // Points closer than a pixel and a half to the last one kept are
            // passed over (the last of them is kept, so the trail still
            // reaches its end).
            let (x, y) = self.sim.pos(i);
            let at = to_screen(x, y);
            path[0] = (at.0, at.1, 0.0);
            let mut n = 1;
            let mut skipped = None;
            for (k, p) in self.sim.trail(i).enumerate() {
                let at = to_screen(p[0] as f64, p[1] as f64);
                let age = k as f32 + phase;
                let last = path[n - 1];
                if (at.0 - last.0).hypot(at.1 - last.1) < 1.5 {
                    skipped = Some((at.0, at.1, age));
                    continue;
                }
                path[n] = (at.0, at.1, age);
                n += 1;
                skipped = None;
            }
            if let Some(p) = skipped {
                path[n] = p;
                n += 1;
            }
            // A trail stops before it closes on itself: once the path has
            // turned most of a full turn, the rest is let go (so Mercury
            // wears an arc, not a ring, and the figure of eight's three
            // stars each draw their own part of the eight).
            let mut end = (n - 1, path[n - 1].2);
            let mut turned = 0.0f32;
            for j in 1..n.saturating_sub(1) {
                let (a, b, c) = (path[j - 1], path[j], path[j + 1]);
                let (ux, uy, vx, vy) = (b.0 - a.0, b.1 - a.1, c.0 - b.0, c.1 - b.1);
                let turn = (ux * vy - uy * vx).atan2(ux * vx + uy * vy).abs();
                if turned + turn > TRAIL_TURN {
                    // Cut partway along the next segment, so the end moves smoothly.
                    let f = ((TRAIL_TURN - turned) / turn.max(1e-6)).clamp(0.0, 1.0);
                    path[j + 1] = (b.0 + (c.0 - b.0) * f, b.1 + (c.1 - b.1) * f, b.2 + (c.2 - b.2) * f);
                    end = (j + 1, path[j + 1].2);
                    break;
                }
                turned += turn;
            }
            // Fading to nothing at its end: the oldest point, or where it was cut.
            let span = end.1.min(TRAIL as f32).max(1e-3);
            let alpha = |age: f32| head * (1.0 - age / span).max(0.0).powf(1.3);
            // Through the points on a Catmull-Rom curve, in pieces of a few
            // pixels, so a trail far apart in its points (zoomed in, or fast)
            // is still a curve.
            self.ink.begin();
            for j in 0..end.0 {
                let (p0, p1, p2) = (path[j.saturating_sub(1)], path[j], path[j + 1]);
                let p3 = if j + 1 < end.0 { path[j + 2] } else { p2 };
                let pieces = ((p2.0 - p1.0).hypot(p2.1 - p1.1) / 4.0).ceil().clamp(1.0, 16.0) as usize;
                let mut from = (p1.0, p1.1, p1.2);
                for m in 1..=pieces {
                    let t = m as f32 / pieces as f32;
                    let curve = |a: f32, b: f32, c: f32, d: f32| {
                        let (t2, t3) = (t * t, t * t * t);
                        0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
                    };
                    let to = (curve(p0.0, p1.0, p2.0, p3.0), curve(p0.1, p1.1, p2.1, p3.1), p1.2 + (p2.2 - p1.2) * t);
                    self.ink.segment((from.0, from.1), (to.0, to.1), alpha(from.2), alpha(to.2), half);
                    from = to;
                }
            }
            self.ink.flush(&mut ui.canvas.px, color);
        }
        // Then the bodies. Big ones (the Sun, a galaxy's core) are noted, so
        // a small body over one steps back, as if behind it: zoomed out, the
        // inner planets go into the Sun's light instead of speckling it.
        let fresh = self.fresh.as_ref().map(|(id, grow)| (*id, grow.value()));
        let mut big = [(0.0f32, 0.0f32, 0.0f32); 4];
        let mut bigs = 0;
        for i in 0..self.sim.len() {
            let r = dot_radius(self.sim.mass(i), s);
            if r >= 3.0 && bigs < big.len() {
                let (x, y) = to_screen(self.sim.pos(i).0, self.sim.pos(i).1);
                big[bigs] = (x, y, r);
                bigs += 1;
            }
        }
        for i in 0..self.sim.len() {
            let body = self.sim.body(i);
            let (x, y) = to_screen(self.sim.pos(i).0, self.sim.pos(i).1);
            let grow = match fresh {
                Some((id, g)) if id == body.id => g.max(0.0),
                _ => 1.0,
            };
            let r = dot_radius(self.sim.mass(i), s) * grow;
            let reach = r * 3.5 + 3.0;
            if x < -reach || x > W + reach || y < -reach || y > H + reach || r <= 0.05 {
                continue;
            }
            let behind = big[..bigs]
                .iter()
                .filter(|(_, _, br)| *br > r + 0.5)
                .map(|(bx, by, br)| ((br + r - (x - bx).hypot(y - by)) / (2.0 * r)).clamp(0.0, 1.0))
                .fold(0.0f32, f32::max);
            let shown = 1.0 - 0.85 * behind;
            let color = body_color(body.hue, tone);
            let canvas = &mut ui.canvas;
            // Small bodies glow less, so a crowd of stars stays a crowd of points.
            let glow = ((r - 1.0) / 3.0).clamp(0.3, 1.0) * shown;
            if dark {
                canvas.glow(x, y, r * 3.2 + 2.0, color, glow * if body.star { 0.42 } else { 0.26 });
            } else {
                canvas.glow(x, y, r * 2.6 + 2.0, color, glow * 0.18);
            }
            canvas.circle(x, y, r, color, shown);
            if dark && body.star && r >= 2.5 {
                // A star is hotter at its heart.
                canvas.circle(x, y, r * 0.55, color.mix(Color::WHITE, 0.7), 0.9 * shown);
            }
        }
        // The ring round the body followed: it closes in when the camera
        // takes it up, the focus ring's colour.
        if let Some(i) = followed {
            let (x, y) = to_screen(self.sim.pos(i).0, self.sim.pos(i).1);
            let lock = self.lock.value().max(0.0);
            let r = dot_radius(self.sim.mass(i), s) + 4.0 + lock * 10.0;
            let ring = Rect::new(x - r, y - r, 2.0 * r, 2.0 * r);
            let color = ui.ring_color();
            ui.canvas.stroke_rrect(ring, r, 1.25, color, (0.9 * (1.0 - lock * 0.7)).clamp(0.0, 1.0));
        }
    }

    /// The quiet line at the top: the universe (and the body followed) at
    /// the left, time and how many bodies at the right. Where the island
    /// grows over a side, that side steps back.
    fn draw_line(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let cy = 4.0 + ui.island_h() * 0.5;
        let bezel = ui.look.theme == Theme::Bezel;
        let island = self.island.rect(ui);
        let presence = self.island.presence();
        let under = |x0: f32, x1: f32| match island {
            Some(r) if x1 > r.x - 6.0 && x0 < r.right() + 6.0 => 1.0 - presence,
            _ => 1.0,
        };
        // Right: how fast, and how many.
        let n = self.sim.len();
        let count = if n == 1 { "1 body".to_owned() } else { format!("{n} bodies") };
        let time = if self.paused { "paused".to_owned() } else { speed_words(SPEEDS[self.speed]) };
        let right = format!("{time} · {count}");
        let caption = ty.caption;
        let right_w = ui.measure(&right, caption);
        let icon = if self.paused { 13.0 } else { 0.0 };
        let right_x = W - PAD - 2.0 - right_w;
        let a = under(right_x - icon, W - PAD);
        if self.paused {
            ui.icon_alpha("pause", true, right_x - 8.0, cy, 12.0, t.dim, a);
        }
        ui.text_alpha(right_x, cy, &right, caption, t.dim, a);
        // Left: the universe, and the body followed.
        let (style, name) = if bezel { (ty.label, self.universe.name().to_lowercase()) } else { (ty.body, self.universe.name().to_owned()) };
        let room = right_x - icon - 12.0 - (PAD + 2.0);
        let followed = self.followed().map(|i| self.sim.body(i).name.to_lowercase());
        let name_w = ui.measure(&name, style);
        let mark = 11.0;
        let follow_w = followed.as_ref().map_or(0.0, |f| mark + ui.measure(f, caption));
        let x = PAD + 2.0;
        // Short of room, the body followed matters more than the universe.
        let show_name = followed.is_none() || name_w + 10.0 + follow_w <= room;
        let mut at = x;
        if show_name {
            let fitted = ui.text.fit(&name, &style, room);
            let w = ui.measure(&fitted, style);
            ui.text_alpha(at, cy, &fitted, style, t.ink, under(at, at + w));
            at += w + 10.0;
        }
        if let Some(f) = followed {
            let fitted = ui.text.fit(&f, &caption, (room - (at - x) - mark).max(0.0));
            let w = ui.measure(&fitted, caption);
            let a = under(at, at + mark + w);
            let ring = ui.ring_color();
            ui.canvas.stroke_rrect(Rect::new(at, cy - 3.5, 7.0, 7.0), 3.5, 1.2, ring, a);
            ui.text_alpha(at + mark, cy, &fitted, caption, t.dim, a);
        }
    }

    fn draw_hint(&self, ui: &mut Ui) {
        // Longest first; paused, the way to play again leads.
        let choices: &[&str] = if self.paused {
            &[
                "space: play · ←→: time · ↑↓: zoom · tab: follow · h: keys",
                "space: play · ←→: time · ↑↓: zoom · h: keys",
                "space: play · tab: follow · h: keys",
                "space: play · h: keys",
                "space: play",
            ]
        } else {
            &[
                "space: pause · ←→: time · ↑↓: zoom · tab: follow · h: keys",
                "←→: time · ↑↓: zoom · tab: follow · h: keys",
                "space: pause · tab: follow · h: keys",
                "tab: follow · h: keys",
                "h: keys",
            ]
        };
        ui.hint_any(choices);
    }
}

impl Page for Space {
    fn title(&self) -> &str {
        self.universe.name()
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.calm = cx.calm();
        match key {
            Key::Back => return Flow::Pop,
            Key::Char(' ') => {
                self.paused = !self.paused;
                let words = if self.paused { "paused".to_owned() } else { format!("time {}", speed_words(SPEEDS[self.speed])) };
                self.say(words);
            }
            Key::Left | Key::Right => {
                let faster = key == Key::Right;
                let words = match (faster, self.speed) {
                    (true, s) if s + 1 >= SPEEDS.len() => format!("{} is the fastest", speed_words(SPEEDS[s])),
                    (false, 0) => format!("{} is the slowest", speed_words(SPEEDS[0])),
                    (true, s) => {
                        self.speed = s + 1;
                        format!("time {}", speed_words(SPEEDS[s + 1]))
                    }
                    (false, s) => {
                        self.speed = s - 1;
                        format!("time {}", speed_words(SPEEDS[s - 1]))
                    }
                };
                self.say(words);
            }
            Key::Up | Key::Char('+') | Key::Char('=') => self.zoom_by(1.0),
            Key::Down | Key::Char('-') | Key::Char('_') => self.zoom_by(-1.0),
            Key::Tab => self.follow_next(),
            Key::Char('n') | Key::Char('N') => {
                let words = self.set_loose();
                self.say(words);
            }
            Key::Char('r') | Key::Char('R') => {
                self.restart();
                self.say("started again");
            }
            Key::Char('h') | Key::Char('H') | Key::Char('?') | Key::F(1) => return Flow::Push(Box::new(Help)),
            _ => {}
        }
        Flow::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        if !self.paused {
            let camera = self.camera();
            self.sim.advance(dt as f64 * SPEEDS[self.speed]);
            // The body followed was swallowed: follow what swallowed it,
            // gliding from where the view was.
            if let Some(id) = self.follow.filter(|id| self.sim.index_of(*id).is_none()) {
                self.follow = self.sim.survivor_of(id);
                self.look_from(camera);
            }
            moving = true;
        }
        moving |= self.zoom.tick(dt);
        moving |= self.pan.0.tick(dt);
        moving |= self.pan.1.tick(dt);
        moving |= self.lock.tick(dt);
        if let Some((_, grow)) = &mut self.fresh {
            if !grow.tick(dt) {
                self.fresh = None;
            } else {
                moving = true;
            }
        }
        moving |= self.island.tick(dt, self.calm);
        moving
    }

    fn draw(&mut self, ui: &mut Ui) {
        self.calm = ui.calm();
        let top = (ui.island_h() + 10.0) as usize;
        let bottom = (ui.hint_h() + 8.0) as usize;
        self.keep_edges(ui, top, bottom);
        self.draw_sky(ui);
        self.fade_edges(ui, top, bottom);
        self.draw_line(ui);
        self.draw_hint(ui);
        self.island.draw(ui);
    }
}

/// `a` taken toward `b` by `t`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    blend(a, Color::hex(b), t)
}

/// Trails are drawn as coverage first: each segment's pixels keep the
/// strongest of what is laid on them, fading along the segment from one
/// end's alpha to the other's, and the whole trail goes onto the panel in
/// one colour at the end. So a trail fades smoothly from head to tail and
/// its joints never darken where segments overlap.
struct Ink {
    alpha: Vec<u8>,
    /// What this trail has touched: x0, y0, x1, y1 (exclusive).
    touched: (i32, i32, i32, i32),
}

impl Ink {
    fn new() -> Ink {
        Ink { alpha: vec![0; (W * H) as usize], touched: (i32::MAX, i32::MAX, i32::MIN, i32::MIN) }
    }

    fn begin(&mut self) {
        self.touched = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    }

    /// A line from `a` to `b`, `half` its width either side, at alphas
    /// `alpha_a` to `alpha_b`.
    fn segment(&mut self, a: (f32, f32), b: (f32, f32), alpha_a: f32, alpha_b: f32, half: f32) {
        if alpha_a <= 0.004 && alpha_b <= 0.004 {
            return;
        }
        let reach = half + 1.0;
        let x0 = (a.0.min(b.0) - reach).floor().max(0.0) as i32;
        let y0 = (a.1.min(b.1) - reach).floor().max(0.0) as i32;
        let x1 = ((a.0.max(b.0) + reach).ceil() as i32).min(W as i32);
        let y1 = ((a.1.max(b.1) + reach).ceil() as i32).min(H as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (bx, by) = (b.0 - a.0, b.1 - a.1);
        let inv = 1.0 / (bx * bx + by * by).max(1e-6);
        let w = W as usize;
        for y in y0..y1 {
            let py = y as f32 + 0.5 - a.1;
            let row = y as usize * w;
            for x in x0..x1 {
                let px = x as f32 + 0.5 - a.0;
                let t = ((px * bx + py * by) * inv).clamp(0.0, 1.0);
                let (dx, dy) = (px - bx * t, py - by * t);
                let cover = (0.5 + half - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let v = ((alpha_a + (alpha_b - alpha_a) * t) * cover * 255.0) as u8;
                let cell = &mut self.alpha[row + x as usize];
                *cell = (*cell).max(v);
            }
        }
        let t = &mut self.touched;
        *t = (t.0.min(x0), t.1.min(y0), t.2.max(x1), t.3.max(y1));
    }

    /// Lay what was drawn onto `px` in `color`, and clear it for the next trail.
    fn flush(&mut self, px: &mut [u32], color: Color) {
        let (x0, y0, x1, y1) = self.touched;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let w = W as usize;
        for y in y0 as usize..y1 as usize {
            let row = y * w;
            for x in x0 as usize..x1 as usize {
                let cell = &mut self.alpha[row + x];
                if *cell > 1 {
                    px[row + x] = blend(px[row + x], color, *cell as f32 / 255.0);
                }
                *cell = 0;
            }
        }
        self.begin();
    }
}

/// The island, drawn by the page: on a full-screen page the kit leaves its
/// own out, and with it whatever `cx.say` said. It grows out of a small
/// glass drop at the top edge with the words, stays a moment, and shrinks
/// back into nothing, as the shell's does.
struct Island {
    words: String,
    left: f32,
    width: Animated,
    glass: Animated,
    /// The width the words started from, for fading them in.
    from: f32,
    shown: String,
}

impl Island {
    fn new() -> Island {
        Island {
            words: String::new(),
            left: 0.0,
            width: Animated::new(0.0, role::release()).with_rest(0.3),
            glass: Animated::new(0.0, role::effect()).with_rest(0.01),
            from: 0.0,
            shown: String::new(),
        }
    }

    fn say(&mut self, words: String, calm: bool) {
        self.words = words;
        self.left = MESSAGE_SECONDS;
        if calm {
            self.glass.jump(1.0);
        } else {
            self.glass.set(1.0);
        }
    }

    fn tick(&mut self, dt: f32, calm: bool) -> bool {
        let showing = self.left > 0.0;
        if showing {
            self.left -= dt;
            if self.left <= 0.0 {
                if calm {
                    self.glass.jump(0.0);
                } else {
                    self.glass.set(0.0);
                }
            }
        }
        let moving = self.width.tick(dt) | self.glass.tick(dt);
        moving || showing
    }

    fn visible(&self) -> bool {
        self.left > 0.0 || self.glass.value() > 0.01
    }

    fn presence(&self) -> f32 {
        self.glass.value().clamp(0.0, 1.0)
    }

    /// Where it is now, if it is.
    fn rect(&self, ui: &Ui) -> Option<Rect> {
        if !self.visible() {
            return None;
        }
        let h = ui.island_h();
        let w = self.width.value().max(h);
        Some(Rect::new((W - w) * 0.5, 4.0, w, h))
    }

    fn draw(&mut self, ui: &mut Ui) {
        if !self.visible() {
            self.width.jump(0.0);
            return;
        }
        let (t, caption) = (ui.tokens, ui.ty.caption);
        let h = ui.island_h();
        let words = ui.text.fit(&self.words, &caption, 250.0);
        let full = ui.measure(&words, caption) + 34.0;
        let want = if self.left > 0.0 { full } else { h };
        if words != self.shown {
            self.shown = words.clone();
            self.from = self.width.value().max(h);
        }
        if self.width.value() < 1.0 {
            // Appearing: from a drop the island's height across.
            self.width.jump(h);
            self.from = h;
        }
        if (want - self.width.target()).abs() > 0.5 {
            if ui.calm() {
                self.width.jump(want);
            } else {
                let growing = want > self.width.target();
                self.width.set_with(want, if growing { role::release() } else { role::smooth() });
                ui.started_moving();
            }
        }
        let Some(island) = self.rect(ui) else { return };
        let mut shape = GlassShape::new(island, h * 0.5, 0);
        shape.strength = self.presence();
        ui.glass(&[shape]);
        // The words come in once the island is past halfway to its width,
        // and go as it shrinks away.
        let travel = full - self.from;
        let grown = if travel.abs() < 1.0 { 1.0 } else { ((self.width.value() - self.from) / travel).clamp(0.0, 1.0) };
        let leaving = if self.left > 0.0 { 1.0 } else { ((self.width.value() - h) / (full - h).max(1.0)).clamp(0.0, 1.0) };
        let alpha = ((grown - 0.55) / 0.45).clamp(0.0, 1.0) * ((leaving - 0.4) / 0.6).clamp(0.0, 1.0) * self.presence();
        ui.canvas.set_clip(island.inset(2.0));
        let dot_x = island.x + 12.0f32.min(island.w * 0.5);
        ui.canvas.circle(dot_x, island.cy(), 3.0, t.signal, self.presence());
        ui.text_alpha(island.x + 22.0, island.cy(), &words, caption, t.ink, alpha);
        ui.canvas.clear_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trail_fades_without_darkening_its_joints() {
        let mut ink = Ink::new();
        ink.segment((10.0, 10.5), (20.0, 10.5), 0.5, 0.5, 0.6);
        ink.segment((20.0, 10.5), (30.0, 10.5), 0.5, 0.5, 0.6);
        let at = |x: usize| ink.alpha[10 * W as usize + x];
        assert_eq!(at(20), at(15), "the joint is no darker than the line");
        let mut px = vec![0u32; (W * H) as usize];
        ink.flush(&mut px, Color::WHITE);
        assert!(px[10 * W as usize + 15] > 0);
        assert!(ink.alpha.iter().all(|a| *a == 0), "flushed clean for the next trail");
    }

    #[test]
    fn speeds_read_as_they_should() {
        assert_eq!(speed_words(0.1), "×0.1");
        assert_eq!(speed_words(0.25), "×0.25");
        assert_eq!(speed_words(1.0), "×1");
        assert_eq!(speed_words(10.0), "×10");
    }

    #[test]
    fn dots_are_never_lost_and_never_huge() {
        for mass in [0.0005, 0.2, 1.0, 2.0, 1000.0] {
            for scale in [0.5, 35.0, 130.0, 2000.0] {
                let r = dot_radius(mass, scale);
                assert!((1.3..=15.0).contains(&r), "mass {mass} at {scale}: {r}");
            }
        }
        assert!(dot_radius(1000.0, 35.0) > dot_radius(0.2, 35.0), "the Sun is drawn larger than a planet");
    }
}
