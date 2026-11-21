//! The universe as numbers: bodies pulling on one another, stepped as
//! AstroBox steps them. Every pair attracts with a softened inverse square,
//! a += G·m·d / (|d|² + ε²)^1.5; time moves in fixed steps of velocity
//! Verlet (half a kick, a drift, the new forces, half a kick), fed from an
//! accumulator, at most a few dozen a frame; bodies that touch merge,
//! keeping their mass and momentum.
//!
//! The bodies are kept as columns of numbers (struct of arrays), every
//! column made once with room for the most bodies a universe may hold, so
//! stepping never allocates and the inner loop reads straight down memory.

/// How many points of its path each body remembers, for its trail.
pub const TRAIL: usize = 48;
/// The most bodies a universe holds: the Milky Way's 64, and room for
/// sixteen more set loose.
pub const MAX_BODIES: usize = 80;

/// A body's radius for touching, from its mass (AstroBox's `radius_for_mass`).
pub fn radius_for_mass(mass: f64) -> f64 {
    if !mass.is_finite() || mass <= 0.0 {
        return 0.006;
    }
    (0.026 * mass.cbrt()).clamp(0.006, 0.26)
}

/// A body's drawn radius in world units, from its mass (AstroBox's
/// `visual_radius_for_mass`): smaller than its radius for touching, so the
/// Sun does not swallow Mercury's orbit on screen.
pub fn visual_radius_for_mass(mass: f64) -> f64 {
    if !mass.is_finite() || mass <= 0.0 {
        return 0.006;
    }
    (0.026 * mass.powf(0.14)).clamp(0.006, 0.07)
}

/// How a universe's time runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// G.
    pub gravity: f64,
    /// ε: keeps the pull finite when two bodies pass close.
    pub softening: f64,
    /// The fixed step, in the universe's time.
    pub step: f64,
    /// The most steps one frame may take; past it, the backlog is dropped.
    pub max_steps: u32,
    /// A trail point every this many steps.
    pub trail_every: u32,
    /// The most universe time one frame may ask for.
    pub max_frame: f64,
    /// Bodies that touch merge.
    pub collisions: bool,
}

impl Default for Config {
    /// AstroBox's defaults: the figure of eight's.
    fn default() -> Self {
        Config { gravity: 1.0, softening: 0.001, step: 1.0 / 240.0, max_steps: 64, trail_every: 40, max_frame: 0.25, collisions: true }
    }
}

/// The colour a body is drawn in: AstroBox's six, by name. What each looks
/// like in a light or dark mode is the drawing's business.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hue {
    White,
    Yellow,
    Orange,
    Blue,
    Teal,
    Red,
}

impl Hue {
    pub const ALL: [Hue; 6] = [Hue::White, Hue::Yellow, Hue::Orange, Hue::Blue, Hue::Teal, Hue::Red];

    /// AstroBox's colour value, as its presets write it.
    pub fn from_rgb(rgb: u32) -> Hue {
        match rgb {
            0xFFE66D => Hue::Yellow,
            0xFF8C42 => Hue::Orange,
            0x2D8CFF => Hue::Blue,
            0x4ECDC4 => Hue::Teal,
            0xFF4D4D => Hue::Red,
            _ => Hue::White,
        }
    }
}

/// What a body is, beside its numbers.
#[derive(Clone, Debug)]
pub struct Body {
    /// Stays with the body for its life; indices shift when bodies merge.
    pub id: u32,
    pub name: String,
    pub hue: Hue,
    /// Stars shine; planets are lit.
    pub star: bool,
}

/// A body's last few places, newest last, in a ring that never grows.
#[derive(Clone, Copy)]
struct Trail {
    points: [[f32; 2]; TRAIL],
    head: usize,
    len: usize,
}

impl Trail {
    const EMPTY: Trail = Trail { points: [[0.0; 2]; TRAIL], head: 0, len: 0 };

    fn push(&mut self, at: [f32; 2]) {
        self.points[self.head] = at;
        self.head = (self.head + 1) % TRAIL;
        self.len = (self.len + 1).min(TRAIL);
    }

    /// Newest first.
    fn iter(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        (0..self.len).map(move |k| self.points[(self.head + TRAIL - 1 - k) % TRAIL])
    }
}

/// How many merges are remembered, so a camera following a body that was
/// swallowed can find what swallowed it.
const MERGE_LOG: usize = 16;

pub struct Sim {
    pub config: Config,
    bodies: Vec<Body>,
    x: Vec<f64>,
    y: Vec<f64>,
    vx: Vec<f64>,
    vy: Vec<f64>,
    ax: Vec<f64>,
    ay: Vec<f64>,
    m: Vec<f64>,
    r: Vec<f64>,
    trails: Vec<Trail>,
    accumulator: f64,
    time: f64,
    steps: u64,
    merges: u32,
    /// (absorbed, survivor) ids, the latest at `merges % MERGE_LOG`.
    merge_log: [(u32, u32); MERGE_LOG],
    next_id: u32,
    /// The accelerations are those of the bodies where they are now.
    ready: bool,
}

impl Sim {
    pub fn new(config: Config) -> Sim {
        let column = || Vec::with_capacity(MAX_BODIES);
        Sim {
            config,
            bodies: Vec::with_capacity(MAX_BODIES),
            x: column(),
            y: column(),
            vx: column(),
            vy: column(),
            ax: column(),
            ay: column(),
            m: column(),
            r: column(),
            trails: Vec::with_capacity(MAX_BODIES),
            accumulator: 0.0,
            time: 0.0,
            steps: 0,
            merges: 0,
            merge_log: [(u32::MAX, u32::MAX); MERGE_LOG],
            next_id: 0,
            ready: false,
        }
    }

    /// A body, if there is room for one. Its id.
    pub fn add(&mut self, name: impl Into<String>, pos: (f64, f64), vel: (f64, f64), mass: f64, hue: Hue, star: bool) -> Option<u32> {
        if self.len() >= MAX_BODIES {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.bodies.push(Body { id, name: name.into(), hue, star });
        self.x.push(pos.0);
        self.y.push(pos.1);
        self.vx.push(vel.0);
        self.vy.push(vel.1);
        self.ax.push(0.0);
        self.ay.push(0.0);
        self.m.push(mass);
        self.r.push(radius_for_mass(mass));
        self.trails.push(Trail::EMPTY);
        // Everyone now feels the newcomer: the next step starts from fresh forces.
        self.ready = false;
        Some(id)
    }

    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn body(&self, i: usize) -> &Body {
        &self.bodies[i]
    }

    pub fn pos(&self, i: usize) -> (f64, f64) {
        (self.x[i], self.y[i])
    }

    pub fn vel(&self, i: usize) -> (f64, f64) {
        (self.vx[i], self.vy[i])
    }

    pub fn mass(&self, i: usize) -> f64 {
        self.m[i]
    }

    pub fn radius(&self, i: usize) -> f64 {
        self.r[i]
    }

    pub fn set_vel(&mut self, i: usize, vel: (f64, f64)) {
        (self.vx[i], self.vy[i]) = vel;
    }

    /// See everything from a frame moved by `pos` and moving at `vel`.
    pub fn move_frame(&mut self, pos: (f64, f64), vel: (f64, f64)) {
        for i in 0..self.len() {
            self.x[i] -= pos.0;
            self.y[i] -= pos.1;
            self.vx[i] -= vel.0;
            self.vy[i] -= vel.1;
        }
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.bodies.iter().position(|b| b.id == id)
    }

    /// The heaviest body (the first of equals).
    pub fn heaviest(&self) -> Option<usize> {
        (0..self.len()).fold(None, |best: Option<usize>, i| match best {
            Some(b) if self.m[b] >= self.m[i] => Some(b),
            _ => Some(i),
        })
    }

    pub fn total_mass(&self) -> f64 {
        self.m.iter().sum()
    }

    pub fn centre_of_mass(&self) -> (f64, f64) {
        let total = self.total_mass();
        if total <= 0.0 {
            return (0.0, 0.0);
        }
        let sx: f64 = self.x.iter().zip(&self.m).map(|(x, m)| x * m).sum();
        let sy: f64 = self.y.iter().zip(&self.m).map(|(y, m)| y * m).sum();
        (sx / total, sy / total)
    }

    pub fn momentum(&self) -> (f64, f64) {
        let px: f64 = self.vx.iter().zip(&self.m).map(|(v, m)| v * m).sum();
        let py: f64 = self.vy.iter().zip(&self.m).map(|(v, m)| v * m).sum();
        (px, py)
    }

    /// Kinetic plus (softened) potential energy, as AstroBox counts it.
    pub fn energy(&self) -> f64 {
        let n = self.len();
        let e2 = self.config.softening * self.config.softening;
        let mut energy = 0.0;
        for i in 0..n {
            energy += 0.5 * self.m[i] * (self.vx[i] * self.vx[i] + self.vy[i] * self.vy[i]);
            for j in i + 1..n {
                let (dx, dy) = (self.x[j] - self.x[i], self.y[j] - self.y[i]);
                energy -= self.config.gravity * self.m[i] * self.m[j] / (dx * dx + dy * dy + e2).sqrt();
            }
        }
        energy
    }

    /// The universe's own clock.
    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn merges(&self) -> u32 {
        self.merges
    }

    /// What swallowed the body `id`, following merges along (if it is
    /// still remembered).
    pub fn survivor_of(&self, id: u32) -> Option<u32> {
        let mut id = id;
        for _ in 0..MERGE_LOG {
            if self.index_of(id).is_some() {
                return Some(id);
            }
            id = self.merge_log.iter().find(|(gone, _)| *gone == id)?.1;
        }
        self.index_of(id).map(|_| id)
    }

    /// Body `i`'s trail, newest first, in world units.
    pub fn trail(&self, i: usize) -> impl Iterator<Item = [f32; 2]> + '_ {
        self.trails[i].iter()
    }

    /// How far, from 0 to 1, time is from the last trail point to the next.
    pub fn trail_phase(&self) -> f32 {
        let every = self.config.trail_every.max(1) as u64;
        (self.steps % every) as f32 / every as f32
    }

    /// Advance by `elapsed` of the universe's time, in fixed steps; the
    /// remainder waits for the next frame. Returns how many steps were taken.
    pub fn advance(&mut self, elapsed: f64) -> u32 {
        self.accumulator += elapsed.max(0.0).min(self.config.max_frame);
        let mut steps = 0;
        while self.accumulator >= self.config.step && steps < self.config.max_steps {
            self.step();
            self.accumulator -= self.config.step;
            steps += 1;
        }
        // Falling behind: drop the backlog rather than chase it.
        if steps == self.config.max_steps {
            self.accumulator = 0.0;
        }
        steps
    }

    /// One step of velocity Verlet: kick, drift, (merge,) forces, kick.
    pub fn step(&mut self) {
        let dt = self.config.step;
        if !self.ready {
            self.settle_forces();
        }
        let half = 0.5 * dt;
        for i in 0..self.len() {
            self.vx[i] += self.ax[i] * half;
            self.vy[i] += self.ay[i] * half;
            self.x[i] += self.vx[i] * dt;
            self.y[i] += self.vy[i] * dt;
        }
        self.settle_forces();
        for i in 0..self.len() {
            self.vx[i] += self.ax[i] * half;
            self.vy[i] += self.ay[i] * half;
        }
        self.time += dt;
        self.steps += 1;
        let every = self.config.trail_every as u64;
        if every > 0 && self.steps % every == 0 {
            for i in 0..self.len() {
                self.trails[i].push([self.x[i] as f32, self.y[i] as f32]);
            }
        }
    }

    /// The forces where the bodies are now, merging any that touch first.
    fn settle_forces(&mut self) {
        let touching = self.forces();
        if touching && self.config.collisions {
            self.merge_touching();
            self.forces();
        }
        self.ready = true;
    }

    /// Every body's acceleration, each pair worked out once and applied to
    /// both. True if any two bodies touch (found on the way, for free).
    fn forces(&mut self) -> bool {
        let n = self.len();
        let g = self.config.gravity;
        let e2 = self.config.softening * self.config.softening;
        let (x, y, m, r) = (&self.x[..n], &self.y[..n], &self.m[..n], &self.r[..n]);
        let (ax, ay) = (&mut self.ax[..n], &mut self.ay[..n]);
        ax.fill(0.0);
        ay.fill(0.0);
        let mut touching = false;
        for i in 0..n {
            let (xi, yi, mi, ri) = (x[i], y[i], m[i], r[i]);
            let (mut sx, mut sy) = (0.0, 0.0);
            let rest = x[i + 1..].iter().zip(&y[i + 1..]).zip(m[i + 1..].iter().zip(&r[i + 1..]));
            for (((xj, yj), (mj, rj)), (axj, ayj)) in rest.zip(ax[i + 1..].iter_mut().zip(ay[i + 1..].iter_mut())) {
                let (dx, dy) = (xj - xi, yj - yi);
                let d2 = dx * dx + dy * dy;
                let reach = ri + rj;
                touching |= d2 < reach * reach;
                let s = d2 + e2;
                let k = g / (s * s.sqrt());
                sx += mj * k * dx;
                sy += mj * k * dy;
                *axj -= mi * k * dx;
                *ayj -= mi * k * dy;
            }
            ax[i] += sx;
            ay[i] += sy;
        }
        touching
    }

    /// Merge touching pairs, one at a time, until none touch: the heavier
    /// (or, between equals, the older) keeps its name, colour and trail.
    fn merge_touching(&mut self) {
        loop {
            let n = self.len();
            let mut pair = None;
            'find: for i in 0..n {
                for j in i + 1..n {
                    let (dx, dy) = (self.x[j] - self.x[i], self.y[j] - self.y[i]);
                    let reach = self.r[i] + self.r[j];
                    if dx * dx + dy * dy < reach * reach {
                        pair = Some((i, j));
                        break 'find;
                    }
                }
            }
            let Some((i, j)) = pair else { return };
            let (keep, gone) = if self.m[j] > self.m[i] || (self.m[j] == self.m[i] && self.bodies[j].id < self.bodies[i].id) { (j, i) } else { (i, j) };
            let (m1, m2) = (self.m[keep], self.m[gone]);
            let total = m1 + m2;
            let mix = |a: f64, b: f64| (a * m1 + b * m2) / total;
            self.x[keep] = mix(self.x[keep], self.x[gone]);
            self.y[keep] = mix(self.y[keep], self.y[gone]);
            self.vx[keep] = mix(self.vx[keep], self.vx[gone]);
            self.vy[keep] = mix(self.vy[keep], self.vy[gone]);
            self.m[keep] = total;
            self.r[keep] = radius_for_mass(total);
            self.merge_log[self.merges as usize % MERGE_LOG] = (self.bodies[gone].id, self.bodies[keep].id);
            self.merges += 1;
            self.remove(gone);
        }
    }

    fn remove(&mut self, i: usize) {
        self.bodies.remove(i);
        for column in [&mut self.x, &mut self.y, &mut self.vx, &mut self.vy, &mut self.ax, &mut self.ay, &mut self.m, &mut self.r] {
            column.remove(i);
        }
        self.trails.remove(i);
        self.ready = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_grows_with_the_cube_root_of_mass_within_bounds() {
        assert!((radius_for_mass(1.0) - 0.026).abs() < 1e-12);
        assert!((radius_for_mass(8.0) - 0.052).abs() < 1e-12);
        assert!((radius_for_mass(1000.0) - 0.26).abs() < 1e-12);
        assert_eq!(radius_for_mass(1e6), 0.26, "the largest is held at 0.26");
        assert_eq!(radius_for_mass(1e-6), 0.006, "the smallest at 0.006");
        assert_eq!(radius_for_mass(0.0), 0.006);
        assert_eq!(radius_for_mass(f64::NAN), 0.006);
        assert!(visual_radius_for_mass(1000.0) <= 0.07 && visual_radius_for_mass(1000.0) < radius_for_mass(1000.0));
    }

    #[test]
    fn a_merge_keeps_mass_and_momentum_and_weighs_the_place() {
        let mut sim = Sim::new(Config { gravity: 0.0, ..Config::default() });
        // Two bodies already touching, moving apart and across.
        let a = sim.add("a", (0.0, 0.0), (1.0, 0.5), 3.0, Hue::White, true).unwrap();
        sim.add("b", (0.05, 0.0), (-2.0, 0.25), 1.0, Hue::Red, false).unwrap();
        let (p0, mass0) = (sim.momentum(), sim.total_mass());
        // Where the weighted centre will be after one drift.
        let dt = sim.config.step;
        let centre = ((3.0 * (1.0 * dt) + 1.0 * (0.05 - 2.0 * dt)) / 4.0, (3.0 * 0.5 * dt + 0.25 * dt) / 4.0);
        sim.step();
        assert_eq!(sim.len(), 1, "they merged");
        assert_eq!(sim.merges(), 1);
        assert_eq!(sim.body(0).id, a, "the heavier survives");
        assert!((sim.total_mass() - mass0).abs() < 1e-12);
        let p1 = sim.momentum();
        assert!((p1.0 - p0.0).abs() < 1e-12 && (p1.1 - p0.1).abs() < 1e-12, "{p0:?} → {p1:?}");
        let (x, y) = sim.pos(0);
        assert!((x - centre.0).abs() < 1e-12 && (y - centre.1).abs() < 1e-12, "({x}, {y}) vs {centre:?}");
        assert!((sim.radius(0) - radius_for_mass(4.0)).abs() < 1e-15);
        assert_eq!(sim.survivor_of(1), Some(a), "the swallowed body points to its survivor");
    }

    #[test]
    fn momentum_survives_a_merge_under_gravity_too() {
        let mut sim = Sim::new(Config::default());
        sim.add("a", (-0.2, 0.0), (0.6, 0.1), 1.0, Hue::White, true).unwrap();
        sim.add("b", (0.2, 0.0), (-0.9, 0.0), 0.5, Hue::Blue, false).unwrap();
        sim.add("c", (0.0, 3.0), (0.3, 0.0), 0.2, Hue::Red, false).unwrap();
        let p0 = sim.momentum();
        for _ in 0..240 {
            sim.step();
        }
        assert!(sim.merges() >= 1, "a and b ran into each other");
        let p1 = sim.momentum();
        assert!((p1.0 - p0.0).abs() < 1e-9 && (p1.1 - p0.1).abs() < 1e-9, "{p0:?} → {p1:?}");
    }

    #[test]
    fn steps_never_allocate_columns_past_their_room() {
        let mut sim = Sim::new(Config::default());
        for i in 0..MAX_BODIES {
            assert!(sim.add(format!("{i}"), (i as f64, 0.0), (0.0, 0.0), 1e-3, Hue::White, false).is_some());
        }
        assert!(sim.add("one more", (0.0, 9.0), (0.0, 0.0), 1.0, Hue::White, false).is_none());
        let room = sim.x.capacity();
        for _ in 0..100 {
            sim.step();
        }
        assert_eq!(sim.x.capacity(), room);
    }

    #[test]
    fn the_accumulator_drops_a_backlog_it_cannot_take() {
        let mut sim = Sim::new(Config::default());
        sim.add("a", (0.0, 0.0), (0.0, 0.0), 1.0, Hue::White, true);
        // A quarter second is 60 steps at 1/240: within the 64 allowed.
        assert_eq!(sim.advance(0.2501), 60);
        // Ten seconds asked: held to a quarter, and at most the steps allowed.
        sim.config.max_steps = 32;
        assert_eq!(sim.advance(10.0), 32);
        assert_eq!(sim.advance(0.0), 0, "the backlog was dropped");
    }

    #[test]
    fn a_trail_keeps_its_last_points_newest_first() {
        let mut t = Trail::EMPTY;
        for k in 0..TRAIL + 5 {
            t.push([k as f32, 0.0]);
        }
        let points: Vec<f32> = t.iter().map(|p| p[0]).collect();
        assert_eq!(points.len(), TRAIL);
        assert_eq!(points[0], (TRAIL + 4) as f32);
        assert_eq!(points[TRAIL - 1], 5.0);
    }
}
