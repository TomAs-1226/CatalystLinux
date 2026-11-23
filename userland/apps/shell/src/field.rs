//! The field: this season's FRC field (2026, REBUILT) drawn flat from its
//! published dimensions, the robot on it as its pose arrives, and the page
//! in the Robot app that shows it live (the replay draws the same field:
//! `replay.rs`).
//!
//! Where things are comes from published numbers, never a picture:
//! - the field's size, and the AprilTags that place each HUB, TOWER and
//!   OUTPOST: WPILib's 2026 welded field layout (`2026-rebuilt-welded.json`,
//!   BSD 3-Clause);
//! - the size of each element and the zones: the 2026 game manual, section 5;
//! - the one position the manual leaves out (the DEPOT along its wall):
//!   Littleton Robotics' published `FieldConstants` (MIT).
//!
//! Coordinates are WPILib's, always from the blue alliance's origin, as
//! Field2d and AdvantageKit publish them: x down the field from the blue
//! wall, y to a blue driver's left, in metres; headings counter-clockwise.

use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Duration;

use bezel::{Align, Canvas, Color, Rect, Tokens, Tone};
use detent::{role, Animated, Spring};

use crate::gfx::{spatial, Gfx, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::schedule::{BLUE, RED};
use crate::shell::{Cx, Nav, Screen};

// ------------------------------------------------------------ the field ---

/// Alliance wall to alliance wall, and guardrail to guardrail (WPILib's
/// 2026 welded layout; the manual's 651.2 × 317.7 in).
pub const LENGTH: f32 = 16.541;
pub const WIDTH: f32 = 8.069;
const IN: f32 = 0.0254;

/// Each alliance zone, from its wall (158.6 in); its edge is the robot
/// starting line.
pub const ZONE: f32 = 158.6 * IN;
/// The blue HUB: centred between its AprilTags (18 to 27), 47 in square.
const HUB_X: f32 = 4.6255;
const HUB: f32 = 47.0 * IN;
/// The BUMPs either side of each HUB (73.0 in across the field, 44.4 in
/// along it), and the TRENCHes from them to the guardrails (65.65 × 47.0 in).
const BUMP_W: f32 = 73.0 * IN;
const BUMP_D: f32 = 44.4 * IN;
const TRENCH_W: f32 = 65.65 * IN;
const TRENCH_D: f32 = 47.0 * IN;
/// The blue TOWER, in the wall: centred on tag 31, 49.25 in wide, its
/// uprights' front face 43.51 in out, 32.25 in apart.
const TOWER_Y: f32 = 3.746;
const TOWER_W: f32 = 49.25 * IN;
const TOWER_D: f32 = 43.51 * IN;
const TOWER_OPENING: f32 = 32.25 * IN;
/// The blue DEPOT, along the wall 75.93 in left of centre: 42.0 × 27.0 in.
const DEPOT_Y: f32 = WIDTH / 2.0 + 75.93 * IN;
const DEPOT_W: f32 = 42.0 * IN;
const DEPOT_D: f32 = 27.0 * IN;
/// The blue OUTPOST's chute, in the wall, centred on tag 29 (31.8 in).
const OUTPOST_Y: f32 = 0.666;
const OUTPOST_W: f32 = 31.8 * IN;
/// The FUEL laid out in the middle of the neutral zone (181.9 × 71.9 in).
const POOL_W: f32 = 181.9 * IN;
const POOL_D: f32 = 71.9 * IN;

/// A robot with its bumpers, about 34 in square.
const ROBOT: f32 = 0.86;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alliance {
    Red,
    Blue,
}

impl Alliance {
    /// From the Driver Station's `IsRedAlliance`.
    pub fn of_red(red: Option<bool>) -> Option<Alliance> {
        red.map(|red| if red { Alliance::Red } else { Alliance::Blue })
    }

    pub fn color(self) -> Color {
        match self {
            Alliance::Red => RED,
            Alliance::Blue => BLUE,
        }
    }
}

/// An angle brought into (−π, π].
pub fn wrap(a: f32) -> f32 {
    let a = a.rem_euclid(TAU);
    if a > PI {
        a - TAU
    } else {
        a
    }
}

/// Where a robot is: metres, and its heading in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub heading: f32,
}

impl Pose {
    pub const fn new(x: f32, y: f32, heading: f32) -> Self {
        Pose { x, y, heading }
    }

    /// The same place as the other alliance counts it: the field turned
    /// half round about its centre (what a robot that puts its origin at
    /// its own wall publishes on red).
    pub fn flipped(self) -> Pose {
        Pose { x: LENGTH - self.x, y: WIDTH - self.y, heading: wrap(self.heading + PI) }
    }

    /// Part way to `to`, turning the short way round.
    pub fn lerp(self, to: Pose, t: f32) -> Pose {
        Pose { x: self.x + (to.x - self.x) * t, y: self.y + (to.y - self.y) * t, heading: wrap(self.heading + wrap(to.heading - self.heading) * t) }
    }

    /// The heading in whole degrees, 0 to 359, as a driver reads it.
    pub fn degrees(&self) -> u32 {
        (self.heading.to_degrees().rem_euclid(360.0).round() as u32) % 360
    }
}

// ---------------------------------------------------- reading the pose ---

/// WPILib's Field2d on SmartDashboard: a `double[]` of x and y in metres
/// and the heading in degrees, the robot's pose first.
pub const FIELD2D: &str = "/SmartDashboard/Field/Robot";
/// AdvantageKit's odometry (`Logger.recordOutput("Odometry/Robot", pose)`),
/// as its NT4Publisher sends it: a `struct:Pose2d` (x, y and the heading in
/// radians, three little-endian doubles); a `double[]` of the same from
/// AdvantageKit before 2024.
pub const ADVANTAGEKIT: &str = "/AdvantageKit/RealOutputs/Odometry/Robot";
/// The pose topics the link asks for, the first published winning.
pub const POSE_TOPICS: [&str; 2] = [ADVANTAGEKIT, FIELD2D];

/// A pose that can be one: finite, on or near the field. Field2d's own
/// (0, 0, 0), shown before robot code sets a pose, is no pose.
fn checked(x: f64, y: f64, heading: f64) -> Option<Pose> {
    let (x, y, heading) = (x as f32, y as f32, heading as f32);
    if !(x.is_finite() && y.is_finite() && heading.is_finite()) || (x == 0.0 && y == 0.0 && heading == 0.0) {
        return None;
    }
    let near = |v: f32, max: f32| (-2.0..=max + 2.0).contains(&v);
    (near(x, LENGTH) && near(y, WIDTH)).then(|| Pose::new(x, y, wrap(heading)))
}

/// x, y and a heading from an array of doubles (Field2d's in degrees,
/// AdvantageKit's old arrays in radians). Only the first pose of several.
pub fn pose_from_doubles(v: &[f64], degrees: bool) -> Option<Pose> {
    let [x, y, h, ..] = v else { return None };
    checked(*x, *y, if degrees { h.to_radians() } else { *h })
}

/// A WPILib `Pose2d` struct: x, y, then the rotation in radians.
pub fn pose_from_struct(b: &[u8]) -> Option<Pose> {
    let d = |i: usize| b.get(i * 8..i * 8 + 8).map(|s| f64::from_le_bytes(s.try_into().unwrap()));
    checked(d(0)?, d(1)?, d(2)?)
}

/// A pose topic's value, as its name says to read it.
pub fn pose_from(topic: &str, value: &nt4::Value) -> Option<Pose> {
    let field2d = topic.ends_with("Field/Robot");
    match value {
        nt4::Value::Raw(bytes) if !field2d => pose_from_struct(bytes),
        nt4::Value::DoubleArray(v) => pose_from_doubles(v, field2d),
        _ => None,
    }
}

// ----------------------------------------------------- the demo's drive ---

/// The demo robot's cycle on the blue side: from the shooting spot in
/// front of its HUB, through the trench by the outpost, along the fuel in
/// the neutral zone, back over the bump and in to shoot again. Every other
/// cycle takes the other side (mirrored across the field).
const LOOP: [(f32, f32); 9] = [(2.75, 4.0345), (3.25, 1.62), (4.63, 0.84), (6.3, 1.1), (7.1, 2.35), (7.1, 4.3), (6.15, 5.55), (4.63, 5.56), (3.35, 5.0)];
/// Cruising speed (m/s), the time to reach it and stop from it, and how
/// long the robot stays to shoot. My own numbers for a believable robot.
const CRUISE: f32 = 2.3;
const RAMP: f32 = 0.7;
const SHOOT: f32 = 2.4;

/// The cycle as a closed Catmull-Rom curve, sampled evenly along it.
struct Track {
    pts: Vec<(f32, f32)>,
    along: Vec<f32>,
}

impl Track {
    fn get() -> &'static Track {
        static TRACK: OnceLock<Track> = OnceLock::new();
        TRACK.get_or_init(|| {
            let n = LOOP.len();
            let mut pts = Vec::new();
            for i in 0..n {
                let p = |k: usize| LOOP[(i + k + n - 1) % n];
                let (p0, p1, p2, p3) = (p(0), p(1), p(2), p(3));
                for s in 0..48 {
                    let t = s as f32 / 48.0;
                    let (t2, t3) = (t * t, t * t * t);
                    let c = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
                    pts.push((c(p0.0, p1.0, p2.0, p3.0), c(p0.1, p1.1, p2.1, p3.1)));
                }
            }
            pts.push(pts[0]);
            let mut along = vec![0.0];
            for w in pts.windows(2) {
                along.push(along.last().unwrap() + ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt());
            }
            Track { pts, along }
        })
    }

    fn length(&self) -> f32 {
        *self.along.last().unwrap()
    }

    /// The point `s` metres along, and the direction there.
    fn at(&self, s: f32) -> ((f32, f32), f32) {
        let s = s.clamp(0.0, self.length());
        let i = self.along.partition_point(|&a| a <= s).clamp(1, self.pts.len() - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let span = (self.along[i] - self.along[i - 1]).max(1e-6);
        let t = (s - self.along[i - 1]) / span;
        ((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t), (b.1 - a.1).atan2(b.0 - a.0))
    }

    /// How long a cycle takes, shooting included.
    fn period(&self) -> f32 {
        SHOOT + self.length() / CRUISE + RAMP
    }

    /// Where the robot is `t` seconds into a cycle, on the cycle's own side.
    fn pose(&self, t: f32, mirrored: bool) -> Pose {
        let len = self.length();
        let drive = len / CRUISE + RAMP;
        let accel = CRUISE / RAMP;
        // Shooting, then away: speeding up, cruising, slowing into the spot.
        let s = match t - SHOOT {
            d if d <= 0.0 => 0.0,
            d if d < RAMP => 0.5 * accel * d * d,
            d if d < drive - RAMP => 0.5 * CRUISE * RAMP + CRUISE * (d - RAMP),
            d => len - 0.5 * accel * (drive - d).max(0.0).powi(2),
        };
        let ((x, y), along) = self.at(s);
        // Facing the HUB to shoot, turning to the way it drives once away.
        let face = (WIDTH / 2.0 - LOOP[0].1).atan2(HUB_X - LOOP[0].0);
        let near = (s.min(len - s) / 1.1).clamp(0.0, 1.0);
        let w = near * near * (3.0 - 2.0 * near);
        let heading = wrap(face + wrap(along - face) * w);
        let pose = Pose::new(x, y, heading);
        if mirrored {
            Pose::new(pose.x, WIDTH - pose.y, wrap(-pose.heading))
        } else {
            pose
        }
    }
}

/// Where the demo robot is, `t` seconds into driving its cycles: the live
/// demo robot, on the blue alliance as its Driver Station says.
pub fn demo_live(t: f32) -> Pose {
    let track = Track::get();
    let period = track.period();
    let cycle = (t / period).floor();
    track.pose(t - cycle * period, cycle as i64 % 2 == 1)
}

/// Where the demo robot is `m` seconds into a match (the made-up logs): to
/// the shooting spot from its start, cycles through auto and teleop (still
/// in the pause between them), and to the TOWER to climb at the end.
pub fn demo_match(m: f32, seed: u64, red: bool) -> Pose {
    let track = Track::get();
    let period = track.period();
    let start = Pose::new(3.55, 5.9, 0.0);
    let spot = Pose::new(LOOP[0].0, LOOP[0].1, 0.0);
    let tower = Pose::new(TOWER_D + 0.5, TOWER_Y, PI);
    const TO_SPOT: f32 = 2.2;
    // Auto's 20 s, the half second's pause, then teleop: driving time.
    let driving = if m < 20.0 { m - TO_SPOT } else if m < 20.5 { 20.0 - TO_SPOT } else { m - 0.5 - TO_SPOT };
    // The last whole cycle that ends before the final 14 s, then the climb.
    let climb = ((160.5 - 14.0 - 0.5 - TO_SPOT) / period).floor() * period;
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let pose = if m <= 0.0 {
        start
    } else if m < TO_SPOT {
        start.lerp(spot, ease(m / TO_SPOT))
    } else if driving < climb {
        let cycle = (driving / period).floor();
        track.pose(driving - cycle * period, (cycle as u64 + seed) % 2 == 1)
    } else {
        let t = ((driving - climb) / 2.6).clamp(0.0, 1.0);
        spot.lerp(tower, ease(t))
    };
    if red {
        pose.flipped()
    } else {
        pose
    }
}

// ------------------------------------------------------- on the screen ---

/// How field metres land on the screen. The own alliance's wall is on the
/// left (`red_left` turns the field half round), as a driver stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Pixels a metre.
    pub scale: f32,
    /// Where the left wall's lower corner lands.
    pub ox: f32,
    pub oy: f32,
    pub red_left: bool,
}

impl View {
    /// The scale at which the whole field fits in `r`, `margin` px clear.
    pub fn fit_scale(r: Rect, margin: f32) -> f32 {
        ((r.w - 2.0 * margin) / LENGTH).min((r.h - 2.0 * margin) / WIDTH).max(0.1)
    }

    /// The field in `r` at `zoom` times the fitting scale, `centre` (along
    /// and across from the left wall's lower corner, metres) in the middle.
    pub fn new(r: Rect, margin: f32, zoom: f32, centre: (f32, f32), red_left: bool) -> View {
        let scale = Self::fit_scale(r, margin) * zoom;
        View { scale, ox: r.cx() - centre.0 * scale, oy: r.cy() + centre.1 * scale, red_left }
    }

    /// The same view drawn `dx`, `dy` further on.
    pub fn moved(self, dx: f32, dy: f32) -> View {
        View { ox: self.ox + dx, oy: self.oy + dy, ..self }
    }

    /// Along and across as shown (from the left wall's lower corner).
    pub fn oriented(&self, x: f32, y: f32) -> (f32, f32) {
        if self.red_left {
            (LENGTH - x, WIDTH - y)
        } else {
            (x, y)
        }
    }

    pub fn to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        let (u, v) = self.oriented(x, y);
        (self.ox + u * self.scale, self.oy - v * self.scale)
    }

    #[cfg(test)]
    pub fn to_field(&self, sx: f32, sy: f32) -> (f32, f32) {
        let (u, v) = ((sx - self.ox) / self.scale, (self.oy - sy) / self.scale);
        self.oriented(u, v)
    }

    /// A heading as an angle on the screen (y down).
    pub fn screen_angle(&self, heading: f32) -> f32 {
        if self.red_left {
            PI - heading
        } else {
            -heading
        }
    }

    /// A box on the field, corner to corner, as a rectangle on the screen.
    pub fn boxed(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        let (a, b) = (self.to_screen(x0, y0), self.to_screen(x1, y1));
        Rect::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
    }

    fn px(&self, metres: f32, least: f32) -> f32 {
        (metres * self.scale).max(least)
    }

    fn key(&self, h: &mut impl Hasher) {
        for v in [self.scale, self.ox, self.oy] {
            ((v * 8.0).round() as i32).hash(h);
        }
        self.red_left.hash(h);
    }
}

/// The field's colours, from the look's tokens: tinted toward each
/// alliance, quiet enough that the robot is what the eye finds.
pub struct Look {
    pub tile: Color,
    carpet: Color,
    zone: [Color; 2],
    structure: Color,
    hub: [Color; 2],
    line: Color,
    fuel: Color,
    hollow: Color,
    /// Around the robot, to lift it off the carpet.
    pub ring: Color,
}

impl Look {
    pub fn of(t: &Tokens) -> Look {
        // My own tuning, by eye on the panel in both tones.
        let dark = t.tone == Tone::Dark;
        let carpet = if dark { t.surface2 } else { t.surface3 };
        let tint = if dark { 0.17 } else { 0.13 };
        Look {
            tile: t.surface1,
            carpet,
            zone: [carpet.mix(RED, tint), carpet.mix(BLUE, tint)],
            structure: if dark { t.surface3.mix(t.faint, 0.25) } else { carpet.mix(t.faint, 0.45) },
            hub: [carpet.mix(RED, 0.62), carpet.mix(BLUE, 0.62)],
            line: t.faint,
            fuel: if dark { t.faint } else { t.faint.mix(t.dim, 0.3) },
            hollow: if dark { t.ground } else { t.surface1 },
            ring: if dark { t.ground } else { t.surface1 },
        }
    }

    fn key(&self, h: &mut impl Hasher) {
        for c in [self.tile, self.carpet, self.structure, self.line, self.ring] {
            c.to_u32().hash(h);
        }
    }
}

/// Each alliance's boxes: (x0, y0, x1, y1) on the blue side, and the same
/// turned half round for red.
fn both(x0: f32, y0: f32, x1: f32, y1: f32) -> [(Alliance, [f32; 4]); 2] {
    [(Alliance::Blue, [x0, y0, x1, y1]), (Alliance::Red, [LENGTH - x1, WIDTH - y1, LENGTH - x0, WIDTH - y0])]
}

fn side(a: Alliance) -> usize {
    match a {
        Alliance::Red => 0,
        Alliance::Blue => 1,
    }
}

/// Paint the field, everything on it that does not move, onto `c` (a
/// cached layer: see [`Layer`]).
pub fn paint(c: &mut Canvas, view: &View, look: &Look) {
    c.clear(look.tile);
    let carpet = view.boxed(0.0, 0.0, LENGTH, WIDTH);
    let corner = view.px(0.25, 2.0);
    c.rrect(carpet, corner, look.carpet, 1.0);
    // The alliance zones, tinted, and their starting lines.
    for (a, [x0, y0, x1, y1]) in both(0.0, 0.0, ZONE, WIDTH) {
        let zone = view.boxed(x0, y0, x1, y1);
        c.fill_rect(zone, look.zone[side(a)], 1.0);
        let lx = if a == Alliance::Blue { x1 } else { x0 };
        let line = view.boxed(lx, 0.0, lx, WIDTH);
        let w = view.px(0.05, 1.2);
        c.fill_rect(Rect::new(line.x - w * 0.5, line.y, w, line.h), a.color(), 0.85);
    }
    // The centre line, and the fuel laid out across it.
    let mid = view.boxed(LENGTH / 2.0, 0.0, LENGTH / 2.0, WIDTH);
    c.fill_rect(Rect::new(mid.x - 0.5, mid.y, 1.0, mid.h), look.line, 0.7);
    let pool = view.boxed(LENGTH / 2.0 - POOL_D / 2.0, WIDTH / 2.0 - POOL_W / 2.0, LENGTH / 2.0 + POOL_D / 2.0, WIDTH / 2.0 + POOL_W / 2.0);
    c.rrect(pool, view.px(0.15, 2.0), look.fuel, 0.16);
    if view.scale >= 10.0 {
        let step = 0.3;
        let (cols, rows) = ((POOL_D / step) as i32, (POOL_W / step) as i32);
        let r = view.px(0.065, 0.9);
        for i in 0..cols {
            for j in 0..rows {
                let x = LENGTH / 2.0 - (cols - 1) as f32 * step / 2.0 + i as f32 * step;
                let y = WIDTH / 2.0 - (rows - 1) as f32 * step / 2.0 + j as f32 * step;
                let (sx, sy) = view.to_screen(x, y);
                c.circle(sx, sy, r, look.fuel, 0.75);
            }
        }
    }
    // The bumps and trenches either side of each HUB.
    let hub_lo = WIDTH / 2.0 - HUB / 2.0;
    let hub_hi = WIDTH / 2.0 + HUB / 2.0;
    for (y0, y1) in [(hub_hi, hub_hi + BUMP_W), (hub_lo - BUMP_W, hub_lo)] {
        for (_, [x0, a, x1, b]) in both(HUB_X - BUMP_D / 2.0, y0, HUB_X + BUMP_D / 2.0, y1) {
            let bump = view.boxed(x0, a, x1, b);
            c.rrect(bump, view.px(0.08, 1.0), look.structure, 1.0);
            // The ridge along its top.
            let ridge = view.boxed((x0 + x1) / 2.0, a, (x0 + x1) / 2.0, b);
            c.fill_rect(Rect::new(ridge.x - 0.5, ridge.y + 1.0, 1.0, (ridge.h - 2.0).max(0.0)), look.line, 0.5);
        }
    }
    for (y0, y1) in [(0.0, TRENCH_W), (WIDTH - TRENCH_W, WIDTH)] {
        for (_, [x0, a, x1, b]) in both(HUB_X - TRENCH_D / 2.0, y0, HUB_X + TRENCH_D / 2.0, y1) {
            // Its arm overhead, from the guardrail to the bump, on posts.
            let trench = view.boxed(x0, a, x1, b);
            c.fill_rect(trench, look.structure, 0.38);
            let arm = view.px(0.12, 1.5);
            let mid = trench.cx();
            c.fill_rect(Rect::new(mid - arm / 2.0, trench.y, arm, trench.h), look.structure, 1.0);
            c.stroke_rrect(trench, 1.0, 1.0, look.structure, 1.0);
        }
    }
    // The HUBs: each its alliance's, with its hexagonal opening.
    for (a, [x0, y0, x1, y1]) in both(HUB_X - HUB / 2.0, WIDTH / 2.0 - HUB / 2.0, HUB_X + HUB / 2.0, WIDTH / 2.0 + HUB / 2.0) {
        let hub = view.boxed(x0, y0, x1, y1);
        c.rrect(hub, view.px(0.1, 1.5), look.hub[side(a)], 1.0);
        let (cx, cy, r) = (hub.cx(), hub.cy(), hub.w.min(hub.h) * 0.4);
        if r >= 2.0 {
            let hex: Vec<(f32, f32)> = (0..6)
                .map(|k| {
                    let t = k as f32 * PI / 3.0 + PI / 6.0;
                    (cx + r * t.cos(), cy + r * t.sin())
                })
                .collect();
            c.polygon(&hex, look.hollow, 0.6);
        }
    }
    // On each wall: the DEPOT, the TOWER and the OUTPOST's chute.
    for (a, [x0, y0, x1, y1]) in both(0.0, DEPOT_Y - DEPOT_W / 2.0, DEPOT_D, DEPOT_Y + DEPOT_W / 2.0) {
        let depot = view.boxed(x0, y0, x1, y1);
        c.fill_rect(depot, a.color(), 0.14);
        c.stroke_rrect(depot, 1.0, 1.0, a.color(), 0.7);
    }
    for (a, [x0, y0, x1, y1]) in both(0.0, TOWER_Y - TOWER_W / 2.0, TOWER_D, TOWER_Y + TOWER_W / 2.0) {
        let tower = view.boxed(x0, y0, x1, y1);
        c.rrect(tower, view.px(0.06, 1.0), a.color(), 0.9);
        // The opening between its uprights, where a robot climbs.
        let gap = (TOWER_W - TOWER_OPENING) / 2.0;
        let (ix0, ix1) = if a == Alliance::Blue { (0.0, TOWER_D - 0.12) } else { (LENGTH - TOWER_D + 0.12, LENGTH) };
        let opening = view.boxed(ix0, y0 + gap, ix1, y1 - gap);
        if opening.w >= 2.0 && opening.h >= 2.0 {
            c.fill_rect(opening, look.hollow, 0.75);
        }
    }
    let wall = view.px(0.1, 2.0);
    for (a, [_, y0, _, y1]) in both(0.0, OUTPOST_Y - OUTPOST_W / 2.0, 0.2, OUTPOST_Y + OUTPOST_W / 2.0) {
        let x = if a == Alliance::Blue { 0.0 } else { LENGTH };
        let chute = view.boxed(x, y0, x, y1);
        c.fill_rect(Rect::new(chute.x - wall * 1.2, chute.y, wall * 2.4, chute.h), a.color(), 1.0);
    }
    // The guardrails, and the alliance walls in their colours.
    c.stroke_rrect(carpet, corner, 1.0, look.line, 1.0);
    for a in [Alliance::Blue, Alliance::Red] {
        let x = if a == Alliance::Blue { 0.0 } else { LENGTH };
        let w = view.boxed(x, 0.0, x, WIDTH);
        c.fill_rect(Rect::new(w.x - wall / 2.0, w.y, wall, w.h), a.color(), 1.0);
    }
}

/// Something drawn once and laid down again each frame until what it shows
/// changes: the field, the charts. Its corners follow the tile's rounding.
#[derive(Default)]
pub struct Layer {
    canvas: Option<Canvas>,
    px: Vec<u32>,
    key: u64,
}

impl Layer {
    /// Lay the layer at `r` (whole pixels), painting it first if `key` has
    /// changed. True when it was painted.
    pub fn show(&mut self, g: &mut Gfx, r: Rect, radius: f32, key: u64, paint: impl FnOnce(&mut Canvas)) -> bool {
        let (w, h) = (r.w.round() as i32, r.h.round() as i32);
        if w <= 0 || h <= 0 {
            return false;
        }
        let sized = self.canvas.as_ref().is_some_and(|c| c.w == w && c.h == h);
        let painted = !sized || self.key != key;
        if painted {
            if !sized {
                self.canvas = Some(Canvas::new(w, h));
            }
            let canvas = self.canvas.as_mut().unwrap();
            paint(canvas);
            // Premultiplied, with what is left of the panel under it: all
            // of it outside the rounded corners, none inside.
            let local = Rect::new(0.0, 0.0, w as f32, h as f32);
            self.px.clear();
            self.px.extend(canvas.px.iter().copied());
            let rad = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
            let reach = rad.ceil() as i32 + 1;
            for y in 0..h {
                let in_band = y < reach || y >= h - reach;
                if !in_band {
                    continue;
                }
                for x in (0..reach.min(w)).chain((w - reach).max(reach)..w) {
                    let d = bezel::canvas::sd_rrect(x as f32 + 0.5, y as f32 + 0.5, &local, rad);
                    let cover = (0.5 - d).clamp(0.0, 1.0);
                    let i = (y * w + x) as usize;
                    let p = self.px[i];
                    let k = (cover * 256.0) as u32;
                    let ch = |s: u32| (((p >> s) & 0xff) * k) >> 8;
                    let keep = 255 - (cover * 255.0).round() as u32;
                    self.px[i] = keep << 24 | ch(16) << 16 | ch(8) << 8 | ch(0);
                }
            }
            self.key = key;
        }
        g.canvas.blit_premultiplied((r.x + g.ox).round() as i32, r.y.round() as i32, w, h, &self.px, 1.0);
        painted
    }
}

/// A key for a layer from anything hashable.
pub fn key_of(f: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut h = DefaultHasher::new();
    f(&mut h);
    h.finish()
}

/// The field's layer key: its view and its look.
pub fn field_key(view: &View, look: &Look, more: u64) -> u64 {
    key_of(|h| {
        view.key(h);
        look.key(h);
        more.hash(h);
    })
}

/// Where the robot has been: a path that fades over `seconds`.
pub struct Trail {
    points: VecDeque<(f32, Pose)>,
    pub seconds: f32,
}

impl Trail {
    pub fn new(seconds: f32) -> Self {
        Trail { points: VecDeque::new(), seconds }
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// The robot at `pose` at time `t`. The newest point is always where it
    /// is now; behind it, points closer than 50 ms are not kept (the line
    /// looks the same, and costs less to draw).
    pub fn push(&mut self, t: f32, pose: Pose) {
        let n = self.points.len();
        if n >= 2 && t - self.points[n - 2].0 < 0.05 {
            self.points.pop_back();
        }
        self.points.push_back((t, pose));
        while self.points.front().is_some_and(|(at, _)| t - at > self.seconds) {
            self.points.pop_front();
        }
    }

    /// Whether any of it is still fading at `now` (a path, not a robot that
    /// has stood where it is all along).
    pub fn fading(&self, now: f32) -> bool {
        self.points.len() >= 2 && self.points.front().is_some_and(|(at, _)| now - at < self.seconds)
    }

    pub fn points(&self) -> impl Iterator<Item = &(f32, Pose)> {
        self.points.iter()
    }
}

/// A path drawn fading: older is fainter, and nothing older than
/// `seconds` before `now`. Drawn in short runs, each at its own strength.
pub fn draw_trail<'a>(g: &mut Gfx, view: &View, points: impl Iterator<Item = &'a (f32, Pose)>, now: f32, seconds: f32, color: Color) {
    let pts: Vec<(f32, (f32, f32))> = points.filter(|(t, _)| now - t <= seconds && *t <= now + 1e-3).map(|(t, p)| (*t, view.to_screen(p.x, p.y))).collect();
    let run = 6;
    let mut i = 0;
    while i + 1 < pts.len() {
        let end = (i + run).min(pts.len() - 1);
        let age = (now - pts[(i + end) / 2].0) / seconds;
        let strength = (1.0 - age).clamp(0.0, 1.0);
        let line: Vec<(f32, f32)> = pts[i..=end].iter().map(|(_, p)| *p).collect();
        g.canvas.polyline(&line, 2.0, color, 0.85 * strength * strength);
        i = end;
    }
}

/// The robot: its footprint, and a chevron pointing the way it faces,
/// in its alliance's colour, ringed to stand off the carpet.
pub fn draw_robot(g: &mut Gfx, view: &View, pose: Pose, color: Color, ring: Color) {
    let (cx, cy) = view.to_screen(pose.x, pose.y);
    let size = (ROBOT * view.scale).max(10.0);
    let a = view.screen_angle(pose.heading);
    let (s, c) = a.sin_cos();
    let place = |u: f32, v: f32| (cx + (u * c - v * s) * size, cy + (u * s + v * c) * size);
    let square: Vec<(f32, f32)> = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].iter().map(|&(u, v)| place(u, v)).collect();
    let ringed: Vec<(f32, f32)> = [(-0.62, -0.62), (0.62, -0.62), (0.62, 0.62), (-0.62, 0.62)].iter().map(|&(u, v)| place(u, v)).collect();
    g.canvas.polygon(&ringed, ring, 0.55);
    g.canvas.polygon(&square, color, 0.3);
    let mut closed = square.clone();
    closed.push(square[0]);
    g.canvas.polyline(&closed, 1.2, color, 0.9);
    let chevron = |k: f32| -> Vec<(f32, f32)> { [(0.42 * k, 0.0), (-0.34 * k, 0.36 * k), (-0.14 * k, 0.0), (-0.34 * k, -0.36 * k)].iter().map(|&(u, v)| place(u, v)).collect() };
    g.canvas.polygon(&chevron(1.22), ring, 1.0);
    g.canvas.polygon(&chevron(1.0), color, 1.0);
}

/// A small capsule of text over the field.
pub fn chip(g: &mut Gfx, x: f32, cy: f32, align: Align, text: &str, color: Color) -> Rect {
    let t = g.tokens;
    let style = g.ty.caption;
    let w = g.measure(text, style) + 14.0;
    let h = style.size + 7.0;
    let left = match align {
        Align::Left => x,
        Align::Right => x - w,
        _ => x - w / 2.0,
    };
    let r = Rect::new(left, cy - h / 2.0, w, h);
    g.surface(r, h / 2.0, t.surface1);
    g.text_at(r.cx(), cy, Align::Center, text, style, color);
    r
}

// --------------------------------------------------------- zoom and pan ---

/// The zooms the field steps through.
const ZOOMS: [f32; 3] = [1.0, 2.0, 4.0];

/// The rubber band (Detent's `patterns`, *Momentum scrolling with
/// rubber-banding*): past an edge, a give that grows ever more slowly,
/// (1 − 1 / (x·c/d + 1))·d with c = 0.55.
pub fn rubber(x: f32, lo: f32, hi: f32, d: f32) -> f32 {
    let band = |over: f32| (1.0 - 1.0 / (over * 0.55 / d.max(1e-3) + 1.0)) * d;
    if x < lo {
        lo - band(lo - x)
    } else if x > hi {
        hi + band(x - hi)
    } else {
        x
    }
}

/// How the field is looked at: zoomed, and moved about by the keys. Moves
/// go on Material's expressive default spatial spring; past an edge the
/// view gives (the rubber band) and comes back on Detent's edge spring.
pub struct Camera {
    level: usize,
    zoom: Animated,
    u: Animated,
    v: Animated,
    /// Zoomed in, the view keeps the robot in the middle until moved.
    pub follow: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            level: 0,
            zoom: Animated::new(1.0, spatial::default()).with_rest(0.002),
            u: Animated::new(LENGTH / 2.0, spatial::default()).with_rest(0.004),
            v: Animated::new(WIDTH / 2.0, spatial::default()).with_rest(0.004),
            follow: true,
        }
    }
}

impl Camera {
    pub fn zoom(&self) -> f32 {
        ZOOMS[self.level]
    }

    /// How far the centre may go at `zoom` in `r`, along and across.
    fn bounds(r: Rect, margin: f32, zoom: f32) -> ([f32; 2], [f32; 2], [f32; 2]) {
        let scale = View::fit_scale(r, margin) * zoom;
        let (hw, hh) = ((r.w / 2.0 - margin) / scale, (r.h / 2.0 - margin) / scale);
        let span = |half: f32, len: f32| if half * 2.0 >= len { [len / 2.0, len / 2.0] } else { [half, len - half] };
        (span(hw, LENGTH), span(hh, WIDTH), [hw * 2.0, hh * 2.0])
    }

    /// The view as it stands, rubber-banded where it is past an edge.
    pub fn view(&self, r: Rect, margin: f32, red_left: bool) -> View {
        let zoom = rubber(self.zoom.value(), ZOOMS[0], ZOOMS[ZOOMS.len() - 1], 1.0);
        let (bu, bv, seen) = Self::bounds(r, margin, zoom);
        let centre = (rubber(self.u.value(), bu[0], bu[1], seen[0]), rubber(self.v.value(), bv[0], bv[1], seen[1]));
        View::new(r, margin, zoom, centre, red_left)
    }

    fn spring(calm: bool) -> Spring {
        if calm {
            spatial::default().calm()
        } else {
            spatial::default()
        }
    }

    /// Head for a centre, kept within the edges at the zoom being headed for.
    fn aim(&mut self, r: Rect, margin: f32, u: f32, v: f32, calm: bool) {
        let (bu, bv, _) = Self::bounds(r, margin, self.zoom());
        self.u.set_with(u.clamp(bu[0], bu[1]), Self::spring(calm));
        self.v.set_with(v.clamp(bv[0], bv[1]), Self::spring(calm));
    }

    /// Zoom a step in (+1) or out (−1). Past the last step, a give.
    pub fn step_zoom(&mut self, r: Rect, margin: f32, dir: i32, calm: bool) {
        let level = self.level as i32 + dir;
        if (0..ZOOMS.len() as i32).contains(&level) {
            self.level = level as usize;
            self.zoom.set_with(self.zoom(), Self::spring(calm));
            let (u, v) = (self.u.target(), self.v.target());
            self.aim(r, margin, u, v, calm);
        } else if !calm {
            self.zoom.set_with(self.zoom(), role::edge());
            self.zoom.nudge(dir as f32 * 2.4);
        }
    }

    /// Move the view a quarter of what it shows. Past an edge it gives,
    /// and comes back.
    pub fn pan(&mut self, r: Rect, margin: f32, du: i32, dv: i32, calm: bool) {
        self.follow = false;
        let (bu, bv, seen) = Self::bounds(r, margin, self.zoom());
        for (axis, d, b, seen) in [(&mut self.u, du, bu, seen[0]), (&mut self.v, dv, bv, seen[1])] {
            if d == 0 {
                continue;
            }
            let to = axis.target() + d as f32 * seen / 4.0;
            if (b[0] - 1e-3..=b[1] + 1e-3).contains(&to) {
                axis.set_with(to, Self::spring(calm));
            } else if !calm {
                // At the edge: thrown past it, and the edge spring brings it back.
                axis.set_with(to.clamp(b[0], b[1]), role::edge());
                axis.nudge(d as f32 * seen * 1.2);
            }
        }
    }

    /// Zoomed in and following: keep `at` (along and across as shown) in
    /// the middle, as far as the edges allow.
    pub fn track(&mut self, r: Rect, margin: f32, at: (f32, f32), calm: bool) {
        if self.follow && self.level > 0 {
            let (bu, bv, _) = Self::bounds(r, margin, self.zoom());
            let (u, v) = (at.0.clamp(bu[0], bu[1]), at.1.clamp(bv[0], bv[1]));
            if (u - self.u.target()).abs() > 0.01 || (v - self.v.target()).abs() > 0.01 {
                self.u.set_with(u, Self::spring(calm));
                self.v.set_with(v, Self::spring(calm));
            }
        }
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        self.zoom.tick(dt) | self.u.tick(dt) | self.v.tick(dt)
    }
}

// --------------------------------------------------------- the live page ---

/// The field's margin inside its tile, so the walls stand clear of its edge.
pub const MARGIN: f32 = 4.0;

/// The Robot app's field: the robot where its pose says, the last ten
/// seconds of its path behind it. Drawn when the pose moves, at the rate
/// it arrives, and not at all while the robot stands still.
pub struct FieldPage {
    camera: Camera,
    layer: Layer,
    trail: Trail,
    /// Time on the page, real (not motion's).
    clock: f32,
    pose: Option<Pose>,
    moved_at: f32,
    /// The last frame a fading trail was drawn at.
    faded_at: f32,
    viewport: Rect,
    calm: bool,
    red_left: bool,
}

impl Default for FieldPage {
    fn default() -> Self {
        FieldPage {
            camera: Camera::default(),
            layer: Layer::default(),
            trail: Trail::new(10.0),
            clock: 0.0,
            pose: None,
            moved_at: f32::NEG_INFINITY,
            faded_at: 0.0,
            viewport: Rect::new(PAD, 28.0, W - 2.0 * PAD, H - 32.0),
            calm: false,
            red_left: false,
        }
    }
}

impl FieldPage {
    /// The page's own clock, and whether the robot moved within `within` s.
    fn moving(&self, within: f32) -> bool {
        self.clock - self.moved_at < within
    }
}

impl Screen for FieldPage {
    fn title(&self) -> &str {
        "Field"
    }

    fn app(&self) -> &str {
        "Robot"
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("+ − zoom · arrows move")
    }

    /// Copy: where the robot is, as WPILib writes a Pose2d.
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        self.pose.map(|p| format!("Pose2d({:.2}, {:.2}, Rotation2d.fromDegrees({}))", p.x, p.y, p.degrees()))
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let (r, calm) = (self.local(), self.calm);
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('+' | '=') => self.camera.step_zoom(r, MARGIN, 1, calm),
            Key::Char('-' | '_') => self.camera.step_zoom(r, MARGIN, -1, calm),
            Key::Left => self.camera.pan(r, MARGIN, -1, 0, calm),
            Key::Right => self.camera.pan(r, MARGIN, 1, 0, calm),
            Key::Up => self.camera.pan(r, MARGIN, 0, 1, calm),
            Key::Down => self.camera.pan(r, MARGIN, 0, -1, calm),
            // Back to following the robot.
            Key::Enter => {
                self.camera.follow = true;
                if let Some(p) = self.pose {
                    let view = self.camera.view(r, MARGIN, self.red_left);
                    self.camera.track(r, MARGIN, view.oriented(p.x, p.y), calm);
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.camera.tick(dt)
    }

    fn ambient_tick(&mut self, dt: f32, robot: &Robot) -> bool {
        self.clock += dt;
        let pose = robot.pose();
        let moved = pose != self.pose;
        if moved {
            self.pose = pose;
            if let Some(p) = pose {
                self.moved_at = self.clock;
                self.trail.push(self.clock, p);
                let view = self.camera.view(self.local(), MARGIN, self.red_left);
                self.camera.track(self.local(), MARGIN, view.oriented(p.x, p.y), self.calm);
            } else {
                self.trail.clear();
            }
            return true;
        }
        // Standing still, the trail behind it still fades: a frame every
        // quarter second until it is gone.
        if self.trail.fading(self.clock) && self.clock - self.faded_at >= 0.25 {
            self.faded_at = self.clock;
            return true;
        }
        false
    }

    /// While the robot moves, a look as often as its pose arrives (50 times
    /// a second); standing still, ten looks a second to see it set off; with
    /// no pose, the main loop's own.
    fn ambient(&self) -> Option<Duration> {
        match self.pose {
            Some(_) if self.moving(0.3) => Some(Duration::from_millis(20)),
            Some(_) => Some(Duration::from_millis(100)),
            None => None,
        }
    }

    /// A robot driving is watched: the screen stays lit while it moves.
    fn keeps_awake(&self) -> bool {
        self.pose.is_some() && self.moving(30.0)
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        self.calm = g.calm;
        let top = g.top().round();
        // The whole height, to just above the panel's foot.
        self.viewport = Rect::new(PAD, top, W - 2.0 * PAD, (H - 4.0 - top).round());
        let r = self.viewport;
        let alliance = Alliance::of_red(robot.fms("IsRedAlliance").and_then(|v| v.as_bool()));
        self.red_left = alliance == Some(Alliance::Red);
        let look = Look::of(&t);
        let view = self.camera.view(self.local(), MARGIN, self.red_left);
        let radius = g.tile_radius();
        self.layer.show(g, r, radius, field_key(&view, &look, 0), |c| paint(c, &view, &look));
        let on = view.moved(r.x + g.ox, r.y);
        let color = alliance.map_or(t.ink, Alliance::color);
        g.canvas.set_clip(r.offset(g.ox, 0.0));
        if let Some(p) = self.pose {
            draw_trail(g, &on, self.trail.points(), self.clock, self.trail.seconds, color);
            draw_robot(g, &on, p, color, look.ring);
        }
        g.canvas.clear_clip();
        let ty = g.ty;
        match self.pose {
            Some(p) => {
                // Where it is, in a corner the robot is not in.
                let text = format!("{:.2}, {:.2} m · {}°", p.x, p.y, p.degrees());
                let (sx, sy) = view.moved(r.x, r.y).to_screen(p.x, p.y);
                let h = ty.caption.size + 7.0;
                let low = r.bottom() - 5.0 - h / 2.0;
                let high = r.y + 5.0 + h / 2.0;
                let w = g.measure(&text, ty.caption) + 14.0;
                let under = sx < r.x + 5.0 + w + 12.0 && sy > low - h;
                chip(g, r.x + 5.0, if under { high } else { low }, Align::Left, &text, t.dim);
            }
            None => {
                let (line, why) = if robot.linked() { ("no pose on the link", "field2d or advantagekit odometry".to_owned()) } else { ("no robot linked", robot.status()) };
                let w = g.measure(line, ty.label).max(g.measure(&why, ty.caption)) + 26.0;
                let h = ty.label.size + ty.caption.size + 16.0;
                let card = Rect::new(r.cx() - w / 2.0, r.cy() - h / 2.0, w, h);
                g.surface(card, 12.0, t.surface1);
                g.text_at(card.cx(), card.y + 6.0 + ty.label.size / 2.0, Align::Center, line, ty.label, t.ink);
                g.text_at(card.cx(), card.bottom() - 6.0 - ty.caption.size / 2.0, Align::Center, &why, ty.caption, t.dim);
            }
        }
        if self.camera.zoom() > 1.0 {
            let words = if self.camera.follow { format!("{}× · following", self.camera.zoom()) } else { format!("{}×", self.camera.zoom()) };
            chip(g, r.right() - 5.0, r.y + 5.0 + (ty.caption.size + 7.0) / 2.0, Align::Right, &words, t.dim);
        }
    }
}

impl FieldPage {
    /// The viewport as the camera sees it: its own size, at the origin.
    fn local(&self) -> Rect {
        Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h)
    }
}

/// The gallery (`--field-shots <dir>`): the live field on the demo robot,
/// zoomed, with no pose; a match's summary, and its replay at the brownout
/// (opened from the summary, as a person would), in auto while playing and
/// in the endgame. Both tones at the three text sizes, each shot on its
/// own and all of one tone and size on a sheet.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let settle = |shell: &mut Shell, frames: usize| {
        for _ in 0..frames {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    // Real time passing, for what is ambient (the robot driving, a replay
    // playing, a log being read on its thread).
    let wait = |shell: &mut Shell, seconds: f32| {
        for _ in 0..(seconds * 50.0) as usize {
            std::thread::sleep(Duration::from_millis(2));
            if shell.tick(0.02) {
                shell.draw();
            }
        }
    };
    let app = |shell: &mut Shell, name: &str| {
        let (index, ..) = crate::screens::builtin_apps().find(|(_, _, n)| *n == name).expect("a built-in app");
        let screen = crate::screens::open_builtin(index, &shell.robot);
        shell.show(screen);
    };
    for light in [false, true] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let tone = if light { "light" } else { "dark" };
            let fresh = |robot: Robot| {
                let mut shell = Shell::new(robot);
                shell.settings.light = light;
                shell.settings.text = size;
                shell.apply_settings();
                settle(&mut shell, 100);
                shell
            };
            let mut taken: Vec<Canvas> = Vec::new();
            let mut save = |shell: &mut Shell, what: &str| {
                shell.draw();
                let name = format!("{tone}-{}-{what}", size.name());
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 2);
                taken.push(shell.gfx.canvas.clone());
            };
            // Live: the Robot app, → to the field, the demo robot driving.
            let mut shell = fresh(Robot::demo());
            app(&mut shell, "Robot");
            settle(&mut shell, 90);
            shell.key(Key::Right);
            settle(&mut shell, 90);
            // Into its second cycle, driving, its path behind it.
            wait(&mut shell, 10.0);
            save(&mut shell, "1-live");
            // What a frame of it costs, the robot driving, against painting
            // the field itself (once, then laid down).
            if !light && size == TextSize::Standard {
                let started = std::time::Instant::now();
                for _ in 0..200 {
                    shell.tick(0.02);
                    shell.draw();
                }
                let frame = started.elapsed().as_secs_f64() * 1000.0 / 200.0;
                let look = Look::of(&shell.gfx.tokens);
                let view = View::new(Rect::new(0.0, 0.0, 304.0, 138.0), MARGIN, 1.0, (LENGTH / 2.0, WIDTH / 2.0), false);
                let mut c = Canvas::new(304, 138);
                let started = std::time::Instant::now();
                for _ in 0..50 {
                    paint(&mut c, &view, &look);
                }
                let painting = started.elapsed().as_secs_f64() * 1000.0 / 50.0;
                println!("live field on this computer: a whole frame {frame:.2} ms; painting the field {painting:.2} ms (once, then laid down)");
            }
            shell.key(Key::Char('+'));
            settle(&mut shell, 60);
            wait(&mut shell, 1.5);
            save(&mut shell, "2-live-zoomed");
            // A robot linked that publishes no pose.
            let mut robot = Robot::demo();
            robot.demo_without_pose();
            let mut shell = fresh(robot);
            app(&mut shell, "Robot");
            settle(&mut shell, 90);
            shell.key(Key::Right);
            settle(&mut shell, 120);
            save(&mut shell, "3-no-pose");
            // Logs: Q14's summary, the focus on its brownout at 1:18; Enter
            // replays it there.
            let mut shell = fresh(Robot::demo());
            app(&mut shell, "Logs");
            settle(&mut shell, 90);
            shell.key(Key::Enter);
            settle(&mut shell, 90);
            save(&mut shell, "4-match-summary");
            shell.key(Key::Enter);
            settle(&mut shell, 60);
            // Read, and the island's word on the keys come and gone.
            wait(&mut shell, 3.5);
            save(&mut shell, "5-replay-brownout");
            // From the start, playing at 2× into auto.
            shell.key(Key::Home);
            settle(&mut shell, 60);
            shell.key(Key::Char('2'));
            shell.key(Key::Char(' '));
            wait(&mut shell, 4.5);
            save(&mut shell, "6-replay-auto-playing");
            // Paused in the endgame, climbing.
            shell.key(Key::Char(' '));
            shell.key(Key::End);
            settle(&mut shell, 60);
            for _ in 0..3 {
                shell.key(Key::Left);
            }
            settle(&mut shell, 90);
            save(&mut shell, "7-replay-endgame");
            // All of one tone and size on a sheet, two across.
            let (cols, gap) = (2usize, 6i32);
            let rows = taken.len().div_ceil(cols);
            let mut sheet = Canvas::new(cols as i32 * (320 + gap) - gap, rows as i32 * (170 + gap) - gap);
            sheet.clear(Color::hex(0x808080));
            for (k, shot) in taken.iter().enumerate() {
                let (ox, oy) = ((k % cols) as i32 * (320 + gap), (k / cols) as i32 * (170 + gap));
                for y in 0..shot.h {
                    let from = (y * shot.w) as usize;
                    let to = ((oy + y) * sheet.w + ox) as usize;
                    sheet.px[to..to + shot.w as usize].copy_from_slice(&shot.px[from..from + shot.w as usize]);
                }
            }
            crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{tone}-{}.png", size.name())), 2);
            println!("{tone} {}", size.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_field_is_where_the_published_numbers_put_it() {
        // The HUBs sit just past each alliance zone, 158.6 in from the wall.
        assert!(near(HUB_X - HUB / 2.0, 4.029) && (HUB_X - HUB / 2.0 - ZONE).abs() < 0.01);
        // Bumps and trenches fill the HUB's line from guardrail to guardrail
        // (to within the trench's frame overlapping the bump).
        let filled = HUB + 2.0 * BUMP_W + 2.0 * TRENCH_W;
        assert!((filled - WIDTH).abs() < 0.2, "{filled}");
        // The red elements are the blue ones turned half round.
        let [(_, blue), (_, red)] = both(0.0, TOWER_Y - TOWER_W / 2.0, TOWER_D, TOWER_Y + TOWER_W / 2.0);
        assert!(near(red[2], LENGTH) && near(red[0], LENGTH - TOWER_D) && near(blue[0], 0.0));
        assert!(near((red[1] + red[3]) / 2.0, WIDTH - TOWER_Y), "the red tower faces tag 15/16's side");
    }

    #[test]
    fn the_alliance_flip_turns_the_field_half_round() {
        let p = Pose::new(2.0, 1.5, 0.3);
        let f = p.flipped();
        assert!(near(f.x, LENGTH - 2.0) && near(f.y, WIDTH - 1.5) && near(f.heading, wrap(0.3 + PI)));
        let back = f.flipped();
        assert!(near(back.x, p.x) && near(back.y, p.y) && near(back.heading, p.heading), "twice is where it was");
        // The centre stays put, and the blue wall's corner becomes red's.
        let centre = Pose::new(LENGTH / 2.0, WIDTH / 2.0, 0.0).flipped();
        assert!(near(centre.x, LENGTH / 2.0) && near(centre.y, WIDTH / 2.0));
        assert!(near(wrap(3.0 * PI), PI) && near(wrap(-PI / 2.0 - TAU), -PI / 2.0));
    }

    #[test]
    fn field_metres_land_on_the_screen_and_back() {
        let r = Rect::new(0.0, 0.0, 300.0, 140.0);
        let blue = View::new(r, 4.0, 1.0, (LENGTH / 2.0, WIDTH / 2.0), false);
        // Fitted by height (the field is wider than 2:1 is tall here), centred.
        assert!(near(blue.scale, 132.0 / WIDTH));
        let (cx, cy) = blue.to_screen(LENGTH / 2.0, WIDTH / 2.0);
        assert!(near(cx, 150.0) && near(cy, 70.0));
        // Blue's wall on the left, y up: the origin at the lower left.
        let (ox, oy) = blue.to_screen(0.0, 0.0);
        assert!(ox < cx && oy > cy && near(oy, 70.0 + WIDTH / 2.0 * blue.scale));
        // Red's turns it half round: the origin at the upper right.
        let red = View::new(r, 4.0, 1.0, (LENGTH / 2.0, WIDTH / 2.0), true);
        let (rx, ry) = red.to_screen(0.0, 0.0);
        assert!(rx > cx && ry < cy);
        for view in [blue, red, View::new(r, 4.0, 4.0, (3.0, 2.0), true)] {
            let (sx, sy) = view.to_screen(5.25, 1.75);
            let (x, y) = view.to_field(sx, sy);
            assert!(near(x, 5.25) && near(y, 1.75), "back again: {x} {y}");
        }
        // Facing down the field (0) points right on blue, left on red; a
        // quarter turn to the left (+y) points up the screen on blue.
        assert!(near(blue.screen_angle(0.0).cos(), 1.0) && near(red.screen_angle(0.0).cos(), -1.0));
        assert!(near(blue.screen_angle(PI / 2.0).sin(), -1.0));
        // Zoomed, the centre asked for is in the middle.
        let zoomed = View::new(r, 4.0, 2.0, (3.0, 2.0), false);
        let (zx, zy) = zoomed.to_screen(3.0, 2.0);
        assert!(near(zx, 150.0) && near(zy, 70.0) && near(zoomed.scale, blue.scale * 2.0));
    }

    #[test]
    fn poses_read_from_field2d_and_advantagekit() {
        // Field2d: metres and degrees; the robot first of several poses.
        let v = nt4::Value::DoubleArray(vec![4.2, 2.5, 90.0, 9.0, 9.0, 0.0]);
        let p = pose_from(FIELD2D, &v).unwrap();
        assert!(near(p.x, 4.2) && near(p.y, 2.5) && near(p.heading, PI / 2.0) && p.degrees() == 90);
        // AdvantageKit's struct:Pose2d: three little-endian doubles, radians.
        let bytes: Vec<u8> = [12.1f64, 6.0, -PI as f64 / 2.0].iter().flat_map(|d| d.to_le_bytes()).collect();
        let p = pose_from(ADVANTAGEKIT, &nt4::Value::Raw(bytes)).unwrap();
        assert!(near(p.x, 12.1) && near(p.y, 6.0) && p.degrees() == 270);
        // AdvantageKit's old double[]: radians, not degrees.
        let p = pose_from(ADVANTAGEKIT, &nt4::Value::DoubleArray(vec![1.0, 2.0, PI as f64])).unwrap();
        assert_eq!(p.degrees(), 180);
        // Not a pose: Field2d's default, too short, off the field, not finite.
        assert_eq!(pose_from(FIELD2D, &nt4::Value::DoubleArray(vec![0.0, 0.0, 0.0])), None);
        assert_eq!(pose_from(FIELD2D, &nt4::Value::DoubleArray(vec![1.0, 2.0])), None);
        assert_eq!(pose_from(FIELD2D, &nt4::Value::DoubleArray(vec![40.0, 2.0, 0.0])), None);
        assert_eq!(pose_from(FIELD2D, &nt4::Value::DoubleArray(vec![f64::NAN, 2.0, 0.0])), None);
        assert_eq!(pose_from_struct(&[0u8; 16]), None, "a struct cut short");
        assert_eq!(pose_from(FIELD2D, &nt4::Value::Double(3.0)), None);
    }

    #[test]
    fn the_demo_robot_drives_on_the_field_and_rests_to_shoot() {
        let track = Track::get();
        let period = track.period();
        assert!((8.0..12.0).contains(&period), "{period}");
        let mut last = demo_live(0.0);
        let mut fastest = 0.0f32;
        for k in 1..(period * 2.0 * 50.0) as usize {
            let p = demo_live(k as f32 / 50.0);
            assert!((0.3..LENGTH / 2.0).contains(&p.x) && (0.3..WIDTH - 0.3).contains(&p.y), "on its own half: {p:?}");
            fastest = fastest.max(((p.x - last.x).powi(2) + (p.y - last.y).powi(2)).sqrt() * 50.0);
            last = p;
        }
        assert!((2.0..3.2).contains(&fastest), "a believable speed: {fastest}");
        // It stands still to shoot, facing its HUB: nothing to draw then.
        assert_eq!(demo_live(0.5), demo_live(1.5));
        assert!(demo_live(1.0).heading.abs() < 0.05);
        // In a match: the start, then the climb at the end, on red turned round.
        let start = demo_match(-3.0, 0, false);
        assert!(near(start.x, 3.55));
        let climb = demo_match(158.0, 0, false);
        assert!(climb.x < 2.0 && near(climb.heading.abs(), PI), "{climb:?}");
        let red = demo_match(158.0, 0, true);
        assert!(red.x > LENGTH - 2.0);
    }

    #[test]
    fn the_view_rubber_bands_past_its_edges_and_springs_back() {
        let r = Rect::new(0.0, 0.0, 300.0, 140.0);
        let mut cam = Camera::default();
        // Whole: nowhere to go, so an arrow is a give and back.
        cam.pan(r, 4.0, 1, 0, false);
        let mut furthest = 0.0f32;
        for _ in 0..120 {
            cam.tick(1.0 / 60.0);
            let v = cam.view(r, 4.0, false);
            furthest = furthest.max((v.to_screen(LENGTH / 2.0, 0.0).0 - 150.0).abs());
        }
        assert!(furthest > 1.0 && furthest < 20.0, "gave {furthest} px");
        assert!(!cam.tick(1.0 / 60.0), "and settled");
        let v = cam.view(r, 4.0, false);
        assert!(near(v.to_screen(LENGTH / 2.0, WIDTH / 2.0).0, 150.0), "back in the middle");
        // Zoomed in, the centre moves a quarter of the view, and stops at the edge.
        cam.step_zoom(r, 4.0, 1, false);
        for _ in 0..8 {
            cam.pan(r, 4.0, -1, 0, false);
            for _ in 0..90 {
                cam.tick(1.0 / 60.0);
            }
        }
        let v = cam.view(r, 4.0, false);
        let (left, _) = v.to_screen(0.0, 0.0);
        assert!(near(left, 4.0), "the left wall at the margin, not past it: {left}");
        // The band: ever less give the further past.
        assert!(rubber(1.0, 0.0, 0.5, 4.0) - 0.5 < 0.5);
        assert!(rubber(10.0, 0.0, 0.5, 4.0) < 0.5 + 4.0);
        assert!(near(rubber(0.25, 0.0, 0.5, 4.0), 0.25));
    }

    #[test]
    fn the_field_is_painted_once_and_laid_down_after() {
        crate::shell::tests::sandbox();
        let mut g = Gfx::new(Tokens::dark());
        let r = Rect::new(8.0, 28.0, 304.0, 138.0);
        let look = Look::of(&g.tokens);
        let view = View::new(Rect::new(0.0, 0.0, r.w, r.h), MARGIN, 1.0, (LENGTH / 2.0, WIDTH / 2.0), false);
        let mut layer = Layer::default();
        let key = field_key(&view, &look, 0);
        assert!(layer.show(&mut g, r, 14.0, key, |c| paint(c, &view, &look)), "painted the first time");
        assert!(!layer.show(&mut g, r, 14.0, key, |_| panic!("not painted again")), "laid down after");
        // The tile's corner is the panel's, its middle the field's.
        let at = |g: &Gfx, x: i32, y: i32| g.canvas.px[(y * g.canvas.w + x) as usize];
        g.canvas.clear(Color::hex(0x123456));
        layer.show(&mut g, r, 14.0, key, |_| {});
        assert_eq!(at(&g, 8, 28), 0x123456, "outside the rounded corner");
        assert_ne!(at(&g, 160, 97), 0x123456);
        // Zoomed, it is another picture.
        let zoomed = View::new(Rect::new(0.0, 0.0, r.w, r.h), MARGIN, 2.0, (4.0, 4.0), false);
        assert!(layer.show(&mut g, r, 14.0, field_key(&zoomed, &look, 0), |c| paint(c, &zoomed, &look)));
    }

    #[test]
    fn the_live_field_draws_as_the_pose_arrives_and_rests_without_one() {
        crate::shell::tests::sandbox();
        let mut shell = crate::fullheight::shell("catalyst", crate::gfx::TextSize::Standard);
        shell.show(Box::new(FieldPage::default()));
        // Opened, and the island's word on the keys come and gone.
        for _ in 0..330 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        // The demo robot is driving: each look (at its 50 a second) draws,
        // and none of it is motion, so the frame beat is not held.
        let mut drawn = 0;
        for _ in 0..50 {
            if shell.tick(0.02) {
                drawn += 1;
                shell.draw();
            }
            assert!(!shell.moving() && !shell.animating(), "a pose is not motion");
        }
        assert_eq!(drawn, 50);
        assert_eq!(shell.ambient(), Some(Duration::from_millis(20)));
        // A robot with no pose: nothing to draw, the main loop's own looks.
        let mut robot = Robot::demo();
        robot.demo_without_pose();
        shell.robot = robot;
        for _ in 0..30 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        assert_eq!(shell.ambient(), None);
        let page = FieldPage { pose: Some(Pose::new(3.0, 2.0, 0.0)), clock: 100.0, moved_at: 90.0, ..FieldPage::default() };
        assert_eq!(page.ambient(), Some(Duration::from_millis(100)), "standing still: looks, no frames");
    }
}
