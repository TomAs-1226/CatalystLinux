//! Strategy: a match's plan drawn on the field, for the drive team. Each of
//! the team's matches has a board, prefilled from the schedule: the
//! team's alliance on the left (as its drivers stand) and the opponents on
//! the right, a robot marker for each of the six with its team on it.
//!
//! The keyboard draws: a cursor (F X Z C, and with Fn the fine steps the
//! arrows give), robots placed and picked up with Enter, paths from waypoint
//! to waypoint in each robot's colour with an arrowhead at their end, drop
//! zones and short notes. Auto, teleop and the endgame are layers of their
//! own; Space plays them, each robot driving its paths on Detent springs.
//! A board is kept as a template for other matches, shared as QR codes for
//! another handheld's camera, and saved as a picture in `/data/strategy`
//! for Catalyst Companion. When it is nearly time to queue, the island
//! offers the match's board (`offer`).
//!
//! The field and everything at rest on it are one cached picture
//! (`field::Layer`), painted again only when the board, the view or the
//! look changes: a frame lays it down and draws only what moves (the
//! cursor, a robot picked up, robots playing), and at rest nothing asks
//! for frames.

mod board;
mod qr;
#[cfg(not(target_arch = "aarch64"))]
mod shots;

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};
use std::hash::Hash;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use bezel::{Align, Canvas, Color, Rect, TextStyle, Tokens, Tone};
use detent::{role, Animated, Spring};

pub use board::{Board, Phase};
use board::{Hit, Layer, Note, NoteKind, Route, Spot, MOST_LETTERS, MOST_NOTES, MOST_PATHS, MOST_POINTS, OURS, ROBOTS};
use crate::field::{self, Camera, Look, Pose, View, LENGTH, MARGIN, WIDTH};
use crate::gfx::{spatial, Gfx, ListState, TextSize, Type, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};
#[cfg(not(target_arch = "aarch64"))]
pub use shots::shots;

/// A cursor step in metres with the whole field in view: a plain arrow,
/// and with Fn. Zoomed in, they shrink with the view. My own tuning: about
/// eight pixels, and under two.
const COARSE: f32 = 0.5;
const FINE: f32 = 0.1;
/// How near the cursor a thing must be to be picked, in metres.
const REACH: f32 = 0.6;
/// A drop zone's radius, metres. My own.
const ZONE_R: f32 = 0.75;
/// How wide `field::draw_robot` draws a robot (its bumpers, 0.86 m), and
/// no narrower than 10 px.
const ROBOT_M: f32 = 0.86;
/// Boards kept to undo.
const UNDO_MOST: usize = 40;
/// How long the play waits at a phase's end before the next, seconds.
const PHASE_PAUSE: f32 = 0.7;

/// Each robot's colour: its alliance's (`schedule::RED`, `BLUE`) for the
/// first of its three, turned in hue for the others (red toward orange and
/// magenta, blue toward teal and violet), each as light as it needs to be
/// for 4:1 against the carpet in its tone, checked with Detent's
/// `bezel_contrast`. My own tuning. [tone][blue][robot in the alliance].
const COLORS: [[[u32; 3]; 2]; 2] = [
    [[0xe64c51, 0xe6994c, 0xe64cad], [0x5b9fd7, 0x5bd7c5, 0x8b6fdc]],
    [[0xcd1d23, 0xa15c17, 0xc91d89], [0x2a71ac, 0x1e7b6d, 0x522ebd]],
];

pub fn robot_color(board: &Board, robot: usize, tone: Tone) -> Color {
    Color::hex(COLORS[usize::from(tone == Tone::Light)][usize::from(!board.red(robot))][robot % OURS])
}

/// The words on a robot's colour: the dark tone's ground on its colours
/// (5:1 or more), white on the light tone's (5.1:1 or more).
fn on_robot(tone: Tone) -> Color {
    if tone == Tone::Dark {
        Tokens::dark().ground
    } else {
        Color::WHITE
    }
}

/// A step of the cursor on the screen (`right`, `up`: −1, 0 or 1) in
/// field metres. The board shows the team's wall on the left, so for a red
/// team the field is turned half round and every step goes the other way.
pub fn cursor_step(right: i32, up: i32, step: f32, red_left: bool) -> (f32, f32) {
    let s = if red_left { -step } else { step };
    (right as f32 * s, up as f32 * s)
}

/// Which way a robot faces at rest: away from its own wall.
fn facing(board: &Board, robot: usize) -> f32 {
    if board.red(robot) {
        PI
    } else {
        0.0
    }
}

fn distance(a: Spot, b: Spot) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

// ------------------------------------------------------------- the plan ---

/// The board being planned, shared by the board and the pages over it.
pub struct Plan {
    pub board: Board,
    pub phase: Phase,
    undo: Vec<Board>,
    dir: PathBuf,
    /// The menu closes itself on the way back (a match chosen, a template used).
    close_menu: bool,
}

type Shared = Rc<RefCell<Plan>>;

impl Plan {
    /// Change the board, kept to undo and saved, if anything changed.
    fn change(&mut self, f: impl FnOnce(&mut Board)) {
        let before = self.board.clone();
        f(&mut self.board);
        if self.board != before {
            self.undo.push(before);
            if self.undo.len() > UNDO_MOST {
                self.undo.remove(0);
            }
            self.save();
        }
    }

    fn save(&self) {
        if let Err(e) = board::save_board(&self.dir, &self.board) {
            eprintln!("catalyst-shell: strategy: could not save {}: {e}", self.board.key);
        }
    }

    fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else { return false };
        self.board = before;
        self.save();
        true
    }

    /// Switch to another board (another match's), saved or new.
    fn open(&mut self, board: Board) {
        self.board = board;
        self.undo.clear();
        self.phase = Phase::Auto;
    }
}

/// The board for a match: as planned, else fresh from the schedule.
fn board_for(dir: &std::path::Path, m: &crate::schedule::Match, team: u32) -> Board {
    board::load_board(dir, &m.key).unwrap_or_else(|| Board::of_match(m, team))
}

// -------------------------------------------------------------- drawing ---

/// A capsule of words: its size at `style`.
fn capsule_size(g: &mut Gfx, text: &str, style: &TextStyle) -> (f32, f32) {
    ((g.text.measure(text, style).ceil() + style.size * 0.8).round(), (style.size + 4.0).round())
}

/// A capsule of words filling `r`, in the canvas's own pixels.
fn capsule(g: &mut Gfx, r: Rect, text: &str, style: &TextStyle, fill: Color, ink: Color, alpha: f32) {
    let r = Rect::new(r.x.round(), r.y.round(), r.w, r.h);
    g.canvas.rrect(r, r.h / 2.0, fill, alpha);
    let baseline = (r.cy() + style.size * 0.36).round();
    g.text.draw_aligned(&mut g.canvas, r.cx(), baseline, Align::Center, text, style, ink, alpha);
}

/// How much of two rectangles is shared, in square pixels.
fn shared(a: &Rect, b: &Rect) -> f32 {
    (a.right().min(b.right()) - a.x.max(b.x)).max(0.0) * (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0)
}

/// Whether the segment from `a` to `b` passes through `r` (Liang–Barsky).
fn crosses(r: &Rect, a: (f32, f32), b: (f32, f32)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.0 - r.x), (dx, r.right() - a.0), (-dy, a.1 - r.y), (dy, r.bottom() - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Where words go on the field: of a few places beside what they name,
/// on the field, the one that covers least of what is placed already
/// (robots, other words, the chips along the foot) and crosses fewest
/// paths; the first of those equally good.
struct Placer {
    /// What is placed, and how much covering it costs: a robot or its team
    /// most, then the chips along the foot, then a note's words.
    taken: Vec<(Rect, f32)>,
    lines: Vec<((f32, f32), (f32, f32))>,
    bounds: Rect,
    /// What the next place counts as.
    weight: f32,
}

impl Placer {
    /// `keep`: the chips along the foot; `paths`: the phase's.
    fn new(bounds: Rect, keep: &[Rect], view: &View, paths: &[Route]) -> Placer {
        let lines = paths.iter().flat_map(|p| p.points.windows(2).map(|w| (view.to_screen(w[0].0, w[0].1), view.to_screen(w[1].0, w[1].1)))).collect();
        Placer { taken: keep.iter().map(|r| (r.inset(-2.0), 2.0)).collect(), lines, bounds, weight: 3.0 }
    }

    /// The best of `options`, whatever it covers.
    fn place(&mut self, options: &[Rect]) -> Rect {
        let pick = self.best(options, true, false).unwrap_or(options[0]);
        self.taken.push((pick, self.weight));
        pick
    }

    /// The best of `options` that covers nothing placed, if one does;
    /// `lines`: whether crossing a path counts against a place.
    fn place_clear(&mut self, options: &[Rect], lines: bool) -> Option<Rect> {
        let pick = self.best(options, lines, true)?;
        self.taken.push((pick, self.weight));
        Some(pick)
    }

    fn best(&self, options: &[Rect], lines: bool, clear: bool) -> Option<Rect> {
        let b = self.bounds;
        let inside = |r: &&Rect| r.x >= b.x + 1.0 && r.right() <= b.right() - 1.0 && r.y >= b.y + 1.0 && r.bottom() <= b.bottom() - 1.0;
        let covered = |r: &Rect| self.taken.iter().map(|(t, w)| shared(t, r) * w).sum::<f32>();
        // A path crossed costs as much as a corner of words covered. My own weighing.
        let crossed = |r: &Rect| if lines { 40.0 * self.lines.iter().filter(|(a, z)| crosses(r, *a, *z)).count() as f32 } else { 0.0 };
        options.iter().filter(inside).map(|r| (covered(r), crossed(r), *r)).filter(|(c, ..)| !clear || *c == 0.0).min_by(|a, z| (a.0 + a.1).total_cmp(&(z.0 + z.1))).map(|(.., r)| r)
    }
}

/// A line with an arrowhead at its end, as much of it as `shown` (0 to 1)
/// says, measured along it; its waypoints dotted.
fn draw_path(c: &mut Canvas, view: &View, points: &[Spot], color: Color, shown: f32) {
    if points.len() < 2 || shown <= 0.0 {
        return;
    }
    let pts: Vec<(f32, f32)> = points.iter().map(|&(x, y)| view.to_screen(x, y)).collect();
    let total: f32 = pts.windows(2).map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt()).sum();
    let mut left = total * shown.min(1.0);
    let mut drawn = vec![pts[0]];
    for p in pts.windows(2) {
        let d = ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt();
        if left >= d {
            drawn.push(p[1]);
            left -= d;
        } else {
            if d > 0.0 {
                let k = left / d;
                drawn.push((p[0].0 + (p[1].0 - p[0].0) * k, p[0].1 + (p[1].1 - p[0].1) * k));
            }
            break;
        }
    }
    let w = (view.scale * 0.12).clamp(2.0, 4.0);
    let n = drawn.len();
    if n < 2 {
        return;
    }
    // The line stops short of the tip, under the arrowhead's base.
    let (a, b) = (drawn[n - 2], drawn[n - 1]);
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let head = w * 2.6 + 3.0;
    let mut line = drawn.clone();
    if len > 0.5 {
        let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let back = (head * 0.6).min(len * 0.6);
        line[n - 1] = (b.0 - ux * back, b.1 - uy * back);
    }
    c.polyline(&line, w, color, 0.95);
    for p in &drawn[1..n - 1] {
        c.circle(p.0, p.1, w * 0.95, color, 1.0);
    }
    if len > 0.5 {
        let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let half = w * 1.7 + 2.0;
        let base = (b.0 - ux * head, b.1 - uy * head);
        c.polygon(&[b, (base.0 - uy * half, base.1 + ux * half), (base.0 + uy * half, base.1 - ux * half)], color, 1.0);
    }
}

/// A line of dashes, for the leg about to be added.
fn dashes(c: &mut Canvas, a: (f32, f32), b: (f32, f32), w: f32, color: Color, alpha: f32) {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let mut at = 0.0;
    while at < len {
        let to = (at + 4.0).min(len);
        c.polyline(&[(a.0 + ux * at, a.1 + uy * at), (a.0 + ux * to, a.1 + uy * to)], w, color, alpha);
        at += 7.0;
    }
}

/// What the board draws, and what it leaves to be drawn live.
struct Scene<'a> {
    board: &'a Board,
    phase: Phase,
    /// The robot keys act on, ringed.
    picked: Option<usize>,
    /// Robots drawn live instead (picked up, or driving).
    skip: [bool; ROBOTS],
    /// The paths, unless they are drawing in (live).
    paths: bool,
    /// The field's tile, in the canvas's pixels.
    bounds: Rect,
    /// The chips along the foot, kept clear of words.
    keep: &'a [Rect],
}

/// A robot marker: its footprint and heading, as the field page draws a
/// robot, ringed when picked and lifted on a shadow when picked up. Its
/// footprint on the screen.
#[allow(clippy::too_many_arguments)]
fn draw_body(g: &mut Gfx, view: &View, board: &Board, robot: usize, at: Spot, heading: f32, picked: bool, lift: f32, ring: Color) -> Rect {
    let color = robot_color(board, robot, g.tokens.tone);
    let (sx, sy) = view.to_screen(at.0, at.1);
    let size = (ROBOT_M * view.scale).max(10.0);
    if lift > 0.01 {
        g.canvas.glow(sx, sy + 2.0 * lift, size * 1.1, Color::BLACK, 0.4 * lift.min(1.0));
    }
    field::draw_robot(g, view, Pose::new(at.0, at.1, heading), color, ring);
    if picked {
        let ring_color = g.ring_color();
        g.canvas.arc(sx, sy, size * 0.82 + 2.5 + lift * 1.5, 1.5, 0.0, TAU, ring_color, 0.95);
    }
    let half = size * 0.62;
    Rect::new(sx - half, sy - half, 2.0 * half, 2.0 * half)
}

/// Each robot's team on a chip of its colour, beside its footprint: above
/// it, else below, right or left, wherever is clear.
fn robot_chips(g: &mut Gfx, view: &View, board: &Board, robots: &[(usize, Spot)], placer: &mut Placer) {
    let tone = g.tokens.tone;
    let style = g.ty.caption;
    for &(robot, at) in robots {
        let name = board.name(robot);
        let (w, h) = capsule_size(g, &name, &style);
        let (sx, sy) = view.to_screen(at.0, at.1);
        let half = (ROBOT_M * view.scale).max(10.0) * 0.62 + 2.0;
        let r = placer.place(&[
            Rect::new(sx - w / 2.0, sy - half - h, w, h),
            Rect::new(sx - w / 2.0, sy + half, w, h),
            Rect::new(sx + half, sy - h / 2.0, w, h),
            Rect::new(sx - half - w, sy - h / 2.0, w, h),
            Rect::new(sx + half * 0.5, sy - half - h, w, h),
            Rect::new(sx - half * 0.5 - w, sy - half - h, w, h),
            Rect::new(sx + half * 0.5, sy + half, w, h),
            Rect::new(sx - half * 0.5 - w, sy + half, w, h),
        ]);
        capsule(g, r, &name, &style, robot_color(board, robot, tone), on_robot(tone), 1.0);
    }
}

/// A drop zone's ring.
fn draw_zone(g: &mut Gfx, view: &View, board: &Board, note: &Note) {
    let color = robot_color(board, note.robot, g.tokens.tone);
    let (sx, sy) = view.to_screen(note.at.0, note.at.1);
    let r = ZONE_R * view.scale;
    g.canvas.circle(sx, sy, r, color, 0.16);
    g.canvas.arc(sx, sy, r, 1.4, 0.0, TAU, color, 0.9);
}

/// The notes' words: a zone's in its ring, a note's beside its pin.
fn note_chips(g: &mut Gfx, view: &View, board: &Board, notes: &[Note], placer: &mut Placer) {
    let t = g.tokens;
    let style = g.ty.caption;
    for note in notes {
        let (sx, sy) = view.to_screen(note.at.0, note.at.1);
        if note.kind == NoteKind::Label {
            let color = robot_color(board, note.robot, t.tone);
            g.canvas.circle(sx, sy, 3.4, t.surface1, 1.0);
            g.canvas.circle(sx, sy, 2.4, color, 1.0);
        }
        if note.text.is_empty() {
            continue;
        }
        // Words with no clear place stay a pin: the chip along the foot
        // reads them out with the cursor on it.
        let (w, h) = capsule_size(g, &note.text, &style);
        let ring = ZONE_R * view.scale;
        let r = match note.kind {
            NoteKind::Zone => placer.place_clear(&[Rect::new(sx - w / 2.0, sy - h / 2.0, w, h), Rect::new(sx - w / 2.0, sy + ring + 1.0, w, h), Rect::new(sx - w / 2.0, sy - ring - 1.0 - h, w, h)], false),
            NoteKind::Label => placer.place_clear(&[
                Rect::new(sx + 5.0, sy - h / 2.0, w, h),
                Rect::new(sx - 5.0 - w, sy - h / 2.0, w, h),
                Rect::new(sx - w / 2.0, sy - 5.0 - h, w, h),
                Rect::new(sx - w / 2.0, sy + 5.0, w, h),
                Rect::new(sx + 3.0, sy - 3.0 - h, w, h),
                Rect::new(sx + 3.0, sy + 3.0, w, h),
                Rect::new(sx - 3.0 - w, sy - 3.0 - h, w, h),
                Rect::new(sx - 3.0 - w, sy + 3.0, w, h),
            ], true),
        };
        if let Some(r) = r {
            capsule(g, r, &note.text, &style, t.surface1, t.ink, 0.94);
        }
    }
}

/// Everything at rest on the board, in order: zones, paths, robots, and
/// the words over them. What the notes' words took, for words drawn later.
fn draw_scene(g: &mut Gfx, view: &View, s: &Scene, ring: Color) -> Vec<Rect> {
    let layer = s.board.layer(s.phase);
    let tone = g.tokens.tone;
    for note in layer.notes.iter().filter(|n| n.kind == NoteKind::Zone) {
        draw_zone(g, view, s.board, note);
    }
    if s.paths {
        for p in &layer.paths {
            draw_path(&mut g.canvas, view, &p.points, robot_color(s.board, p.robot, tone), 1.0);
        }
    }
    let mut placer = Placer::new(s.bounds, s.keep, view, &layer.paths);
    // The picked robot first: its words take the best place.
    let mut robots: Vec<(usize, Spot)> = (0..ROBOTS).filter(|r| !s.skip[*r]).filter_map(|r| s.board.start_of(s.phase, r).map(|at| (r, at))).collect();
    robots.sort_by_key(|(r, _)| Some(*r) != s.picked);
    for &(r, at) in &robots {
        let body = draw_body(g, view, s.board, r, at, facing(s.board, r), s.picked == Some(r), 0.0, ring);
        placer.taken.push((body, 3.0));
    }
    robot_chips(g, view, s.board, &robots, &mut placer);
    let notes_from = placer.taken.len();
    placer.weight = 1.0;
    note_chips(g, view, s.board, &layer.notes, &mut placer);
    placer.taken.split_off(notes_from).into_iter().map(|(r, _)| r).collect()
}

/// The cursor: a ring with four ticks, in the focus ring's colour.
fn draw_cursor(c: &mut Canvas, x: f32, y: f32, color: Color, back: Color) {
    c.arc(x, y, 5.5, 3.2, 0.0, TAU, back, 0.55);
    c.arc(x, y, 5.5, 1.5, 0.0, TAU, color, 1.0);
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        c.polyline(&[(x + dx * 8.0, y + dy * 8.0), (x + dx * 11.0, y + dy * 11.0)], 1.5, color, 1.0);
    }
    c.circle(x, y, 1.2, color, 1.0);
}

// ------------------------------------------------------------- the play ---

/// The robots driving a phase's paths: one spring carries them from leg to
/// leg, set on to the next as it nears each, so they flow through their
/// waypoints. Robots with fewer legs finish first.
struct Play {
    phase: Phase,
    routes: Vec<(usize, Vec<Spot>)>,
    legs: usize,
    leg: Animated,
    /// At the end, the pause before the next phase.
    waited: f32,
}

impl Play {
    /// `m3-expressive-slow-spatial` (`spatial::slow`): large moves.
    fn spring(calm: bool) -> Spring {
        if calm {
            spatial::slow().calm()
        } else {
            spatial::slow()
        }
    }

    fn of(board: &Board, phase: Phase, calm: bool) -> Option<Play> {
        let routes: Vec<(usize, Vec<Spot>)> = (0..ROBOTS).map(|r| (r, board.route(phase, r))).filter(|(_, route)| route.len() >= 2).collect();
        let legs = routes.iter().map(|(_, r)| r.len() - 1).max()?;
        let mut leg = Animated::new(0.0, Self::spring(calm)).with_rest(0.004);
        leg.set(1.0);
        Some(Play { phase, routes, legs, leg, waited: 0.0 })
    }

    /// On by `dt`; false once the phase is played and paused after.
    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.leg.tick(dt);
        let target = self.leg.target();
        if target < self.legs as f32 && self.leg.value() > target - 0.3 {
            self.leg.set(target + 1.0);
            return true;
        }
        if !moving && target >= self.legs as f32 {
            self.waited += dt;
            return self.waited < PHASE_PAUSE;
        }
        true
    }

    /// Where a route has got to, and which way it is heading.
    fn along(&self, route: &[Spot]) -> (Spot, Option<f32>) {
        let s = self.leg.value().clamp(0.0, (route.len() - 1) as f32);
        let i = (s.floor() as usize).min(route.len() - 2);
        let k = s - i as f32;
        let (a, b) = (route[i], route[i + 1]);
        ((a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k), Some((b.1 - a.1).atan2(b.0 - a.0)))
    }
}

// ------------------------------------------------------------ the board ---

#[derive(Clone)]
enum Mode {
    Idle,
    /// A path being drawn for `robot` from `start`; `at` is its place among
    /// the phase's paths once it has a waypoint.
    Drawing { robot: usize, start: Spot, at: Option<usize> },
    /// A robot picked up (Esc puts it back where it stood).
    Moving { robot: usize },
    /// Words for a note, typed.
    Typing { field: Box<FieldState>, kind: NoteKind, at: Spot, edit: Option<usize> },
}

/// A note's words as they are typed (boxed: the field is large beside the
/// other modes).
#[derive(Clone)]
struct FieldState {
    label: &'static str,
    text: String,
}

/// The cached picture: the field and everything at rest on it.
#[derive(Default)]
struct Cache {
    layer: field::Layer,
    scratch: Option<Canvas>,
    key: Option<u64>,
    /// Where the notes' words are in it, in the tile's own pixels.
    words: Vec<Rect>,
}

pub struct Strategy {
    plan: Shared,
    /// The board's key when last looked at: another means a fresh start.
    key: String,
    title: String,
    /// The robot the keys act on.
    picked: usize,
    /// Where the cursor is headed, and where it is drawn.
    cursor: Spot,
    cur: [Animated; 2],
    mode: Mode,
    play: Option<Play>,
    /// A phase's paths drawing in as it is turned to, 0 to 1.
    reveal: Animated,
    /// A robot picked up rising off the field, 0 to 1.
    lift: Animated,
    /// The two chips along the foot step back while the cursor is under them.
    chips: [Animated; 2],
    chip_rects: [Rect; 2],
    camera: Camera,
    cache: Cache,
    viewport: Rect,
    view: Option<View>,
    calm: bool,
}

impl Strategy {
    /// The board for the team's next match, else for practice.
    pub fn new(robot: &Robot) -> Strategy {
        let dir = board::home();
        let now = crate::reminders::now();
        let board = match robot.schedule.view().and_then(|v| v.next(now).cloned().map(|m| (v.team, m))) {
            Some((team, m)) => board_for(&dir, &m, team),
            None => board::load_board(&dir, "practice").unwrap_or_else(|| Board::practice(crate::tools::team(robot))),
        };
        Strategy::with_board(board, dir)
    }

    pub fn with_board(board: Board, dir: PathBuf) -> Strategy {
        let start = board.start_of(Phase::Auto, 0).unwrap_or((LENGTH / 2.0, WIDTH / 2.0));
        let cursor = |v: f32| Animated::new(v, spatial::fast()).with_rest(0.003);
        let chip = || Animated::new(1.0, role::effect()).with_rest(0.004);
        Strategy {
            key: board.key.clone(),
            title: board.label.clone(),
            plan: Rc::new(RefCell::new(Plan { board, phase: Phase::Auto, undo: Vec::new(), dir, close_menu: false })),
            picked: 0,
            cursor: start,
            cur: [cursor(start.0), cursor(start.1)],
            mode: Mode::Idle,
            play: None,
            reveal: Animated::new(1.0, role::settle()).with_rest(0.002),
            lift: Animated::new(0.0, spatial::fast()).with_rest(0.004),
            chips: [chip(), chip()],
            chip_rects: [Rect::new(0.0, 0.0, 0.0, 0.0); 2],
            camera: Camera::default(),
            cache: Cache::default(),
            viewport: Rect::new(PAD, 28.0, W - 2.0 * PAD, H - 32.0),
            view: None,
            calm: false,
        }
    }

    #[cfg(test)]
    fn plan(&self) -> std::cell::Ref<'_, Plan> {
        self.plan.borrow()
    }

    fn local(&self) -> Rect {
        Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h)
    }

    fn phase(&self) -> Phase {
        self.plan.borrow().phase
    }

    fn red_left(&self) -> bool {
        self.plan.borrow().board.ours_red
    }

    fn change(&mut self, f: impl FnOnce(&mut Board)) {
        self.plan.borrow_mut().change(f);
    }

    fn spring(&self, spring: Spring) -> Spring {
        if self.calm {
            spring.calm()
        } else {
            spring
        }
    }

    /// Send the cursor to `to`, on Material's fast spatial spring (small
    /// moves); zoomed in, the view follows it near an edge.
    fn aim(&mut self, to: Spot) {
        self.cursor = board::clamp(to);
        let spring = self.spring(spatial::fast());
        self.cur[0].set_with(self.cursor.0, spring);
        self.cur[1].set_with(self.cursor.1, spring);
        if self.camera.zoom() > 1.0 {
            let r = self.local();
            let view = self.camera.view(r, MARGIN, self.red_left());
            let (sx, sy) = view.to_screen(self.cursor.0, self.cursor.1);
            let inner = r.inset(r.h * 0.22);
            if sx < inner.x || sx > inner.right() || sy < inner.y || sy > inner.bottom() {
                self.camera.follow = true;
                self.camera.track(r, MARGIN, view.oriented(self.cursor.0, self.cursor.1), self.calm);
            }
        }
        self.chips_aside();
    }

    fn step(&mut self, right: i32, up: i32, fine: bool) {
        let step = if fine { FINE } else { COARSE } / self.camera.zoom();
        let (dx, dy) = cursor_step(right, up, step, self.red_left());
        self.aim((self.cursor.0 + dx, self.cursor.1 + dy));
    }

    /// The chips step back from a cursor under them, and return.
    fn chips_aside(&mut self) {
        let Some(view) = self.view else { return };
        let (sx, sy) = view.to_screen(self.cursor.0, self.cursor.1);
        for (chip, r) in self.chips.iter_mut().zip(self.chip_rects) {
            let r = r.inset(-10.0);
            let under = sx >= r.x && sx <= r.right() && sy >= r.y && sy <= r.bottom();
            chip.set(if under { 0.2 } else { 1.0 });
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.finish_path();
        self.cancel_move();
        if phase == self.phase() {
            return;
        }
        self.plan.borrow_mut().phase = phase;
        // Its paths draw in from their robots (keys change at once in
        // Bezel; Thomas's boards keep it fluid), on the settle spring.
        if self.calm {
            self.reveal.jump(1.0);
        } else {
            self.reveal.jump(0.0);
            self.reveal.set_with(1.0, role::settle());
        }
    }

    fn finish_path(&mut self) {
        if matches!(self.mode, Mode::Drawing { .. }) {
            self.mode = Mode::Idle;
        }
    }

    fn cancel_move(&mut self) {
        if let Mode::Moving { robot, .. } = self.mode {
            self.mode = Mode::Idle;
            self.lift.set(0.0);
            // The cursor goes back with it to where it stands.
            if let Some(at) = self.plan.borrow().board.start_of(self.phase(), robot) {
                self.cursor = at;
            }
            let at = self.cursor;
            self.aim(at);
        }
    }

    fn pick(&mut self, robot: usize) {
        if matches!(self.mode, Mode::Moving { .. }) {
            return;
        }
        self.finish_path();
        self.picked = robot;
    }

    /// Put the picked robot where the cursor is, for this phase.
    fn place(&mut self) {
        let (phase, robot, at) = (self.phase(), self.picked, self.cursor);
        self.finish_path();
        self.change(|b| b.layer_mut(phase).markers[robot] = Some(at));
    }

    fn toggle_path(&mut self) {
        if matches!(self.mode, Mode::Drawing { .. }) {
            return self.finish_path();
        }
        self.cancel_move();
        let start = self.plan.borrow().board.end_of(self.phase(), self.picked).unwrap_or(self.cursor);
        self.mode = Mode::Drawing { robot: self.picked, start, at: None };
    }

    /// The path being drawn's last point.
    fn path_end(&self) -> Option<Spot> {
        let Mode::Drawing { start, at, .. } = self.mode else { return None };
        let plan = self.plan.borrow();
        Some(at.and_then(|i| plan.board.layer(plan.phase).paths.get(i)?.points.last().copied()).unwrap_or(start))
    }

    fn add_waypoint(&mut self, cx: &mut Cx) {
        let Mode::Drawing { robot, start, at } = self.mode else { return };
        let (phase, p) = (self.phase(), self.cursor);
        if self.path_end().is_some_and(|end| distance(end, p) < 0.05) {
            return;
        }
        match at {
            None => {
                let count = self.plan.borrow().board.layer(phase).paths.len();
                if count >= MOST_PATHS {
                    *cx.say = Some(format!("{} has as many paths as a board holds", phase.name()));
                    return;
                }
                self.change(|b| b.layer_mut(phase).paths.push(Route { robot, points: vec![start, p] }));
                self.mode = Mode::Drawing { robot, start, at: Some(count) };
            }
            Some(i) => {
                let full = self.plan.borrow().board.layer(phase).paths.get(i).is_none_or(|r| r.points.len() >= MOST_POINTS);
                if full {
                    *cx.say = Some("that path has as many waypoints as it holds".into());
                    return;
                }
                self.change(|b| b.layer_mut(phase).paths[i].points.push(p));
            }
        }
    }

    fn enter(&mut self, cx: &mut Cx) {
        let phase = self.phase();
        match self.mode.clone() {
            Mode::Moving { robot, .. } => {
                let at = self.cursor;
                self.change(|b| b.layer_mut(phase).markers[robot] = Some(at));
                self.mode = Mode::Idle;
                self.lift.set(0.0);
            }
            Mode::Drawing { .. } => self.add_waypoint(cx),
            Mode::Typing { .. } => {}
            Mode::Idle => {
                let (hit, layer) = {
                    let plan = self.plan.borrow();
                    (plan.board.hit(phase, self.cursor, REACH), plan.board.layer(phase).clone())
                };
                match hit {
                    Some(Hit::Marker(robot)) => {
                        let at = self.plan.borrow().board.start_of(phase, robot);
                        self.picked = robot;
                        self.mode = Mode::Moving { robot };
                        self.lift.set_with(1.0, self.spring(spatial::fast()));
                        if let Some(at) = at {
                            self.aim(at);
                        }
                    }
                    Some(Hit::Note(i)) => {
                        let note = &layer.notes[i];
                        self.mode = Mode::Typing { field: Box::new(FieldState { label: label_of(note.kind), text: note.text.clone() }), kind: note.kind, at: note.at, edit: Some(i) };
                    }
                    _ => {
                        self.toggle_path();
                        self.add_waypoint(cx);
                    }
                }
            }
        }
    }

    fn remove(&mut self, cx: &mut Cx) {
        let phase = self.phase();
        match self.mode.clone() {
            Mode::Drawing { robot, start, at: Some(i) } => {
                let len = self.plan.borrow().board.layer(phase).paths.get(i).map_or(0, |p| p.points.len());
                if len > 2 {
                    self.change(|b| {
                        b.layer_mut(phase).paths[i].points.pop();
                    });
                } else {
                    self.change(|b| {
                        b.layer_mut(phase).paths.remove(i);
                    });
                    self.mode = Mode::Drawing { robot, start, at: None };
                }
            }
            Mode::Drawing { at: None, .. } => self.mode = Mode::Idle,
            Mode::Moving { .. } => self.cancel_move(),
            Mode::Typing { .. } => {}
            Mode::Idle => {
                let hit = self.plan.borrow().board.hit(phase, self.cursor, REACH);
                match hit {
                    Some(Hit::Note(i)) => self.change(|b| {
                        b.layer_mut(phase).notes.remove(i);
                    }),
                    Some(Hit::Path(i)) => self.change(|b| {
                        b.layer_mut(phase).paths.remove(i);
                    }),
                    Some(Hit::Marker(r)) if self.plan.borrow().board.layer(phase).markers[r].is_some() => self.change(|b| b.layer_mut(phase).markers[r] = None),
                    Some(Hit::Marker(_)) => *cx.say = Some(format!("placed in an earlier phase: remove it there, or move it in {}", phase.name())),
                    None => *cx.say = Some("nothing under the cursor".into()),
                }
            }
        }
    }

    fn start_typing(&mut self, kind: NoteKind) {
        self.finish_path();
        self.cancel_move();
        self.mode = Mode::Typing { field: Box::new(FieldState { label: label_of(kind), text: String::new() }), kind, at: self.cursor, edit: None };
    }

    fn typing_key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let Mode::Typing { field, kind, at, edit } = &mut self.mode else { return Nav::Stay };
        let mut typed = Field::new(field.label, &field.text, MOST_LETTERS);
        let done = typed.key(key, |c| !c.is_control());
        field.text = typed.text;
        let (kind, at, edit, robot, phase) = (*kind, *at, *edit, self.picked, self.phase());
        match done {
            Typed::More => {}
            Typed::Cancelled => self.mode = Mode::Idle,
            Typed::Done(text) => {
                self.mode = Mode::Idle;
                let full = self.plan.borrow().board.layer(phase).notes.len() >= MOST_NOTES;
                match edit {
                    Some(i) if text.is_empty() && kind == NoteKind::Label => self.change(|b| {
                        b.layer_mut(phase).notes.remove(i);
                    }),
                    Some(i) => self.change(|b| b.layer_mut(phase).notes[i].text = text),
                    None if text.is_empty() && kind == NoteKind::Label => {}
                    None if full => *cx.say = Some(format!("{} has as many notes as a board holds", phase.name())),
                    None => self.change(|b| b.layer_mut(phase).notes.push(Note { kind, robot, at, text })),
                }
            }
        }
        Nav::Stay
    }

    fn start_play(&mut self, cx: &mut Cx) {
        self.finish_path();
        self.cancel_move();
        let from = self.phase().index();
        let found = {
            let plan = self.plan.borrow();
            Phase::ALL[from..].iter().find_map(|p| Play::of(&plan.board, *p, self.calm))
        };
        match found {
            Some(play) => {
                if play.phase != self.phase() {
                    self.set_phase(play.phase);
                }
                self.play = Some(play);
            }
            None => *cx.say = Some(format!("no paths from {} on to play", Phase::ALL[from].name())),
        }
    }

    /// Back to where things stand when the board is left or another opens.
    fn settle_modes(&mut self) {
        self.finish_path();
        self.cancel_move();
        if matches!(self.mode, Mode::Typing { .. }) {
            self.mode = Mode::Idle;
        }
        self.play = None;
    }

    fn zoom(&mut self, dir: i32) {
        let r = self.local();
        self.camera.step_zoom(r, MARGIN, dir, self.calm);
        let at = self.cursor;
        self.camera.follow = true;
        let view = self.camera.view(r, MARGIN, self.red_left());
        self.camera.track(r, MARGIN, view.oriented(at.0, at.1), self.calm);
    }

    // ---- the chips along the foot ----

    fn status(&self, board: &Board) -> String {
        let name = board.name(self.picked);
        match &self.mode {
            _ if self.play.is_some() => "playing · space stops".into(),
            Mode::Drawing { at: None, .. } => format!("{name} · path: enter adds"),
            Mode::Drawing { .. } => format!("{name} · path · p ends"),
            Mode::Moving { robot, .. } => format!("{} · enter drops", board.name(*robot)),
            // On a note: its words, which may have had no room on the field.
            _ => match board.hit(self.phase(), self.cursor, REACH) {
                Some(Hit::Note(i)) if !board.layer(self.phase()).notes[i].text.is_empty() => board.layer(self.phase()).notes[i].text.clone(),
                _ => name,
            },
        }
    }

    /// Where the chips along the foot go in the tile `r`, and the words
    /// on the right one.
    fn chip_layout(&self, g: &mut Gfx, board: &Board, phase: Phase, r: Rect) -> ([Rect; 2], String) {
        let style = g.ty.caption;
        let h = (style.size + 7.0).round();
        let y = (r.bottom() - 5.0 - h).round();
        let word_w = g.text.measure(phase.name(), &style).ceil();
        let left = Rect::new(r.x + 5.0, y, (word_w + 40.0).round(), h);
        let room = r.w - left.w - 20.0 - 24.0;
        let words = g.text.fit(&self.status(board), &style, room);
        let words_w = g.text.measure(&words, &style).ceil();
        let right = Rect::new((r.right() - 5.0 - words_w - 27.0).round(), y, (words_w + 27.0).round(), h);
        ([left, right], words)
    }

    /// The phase (three dots, the one shown filled) and the picked robot
    /// with what the keys are doing, at the tile's foot, in canvas pixels.
    fn draw_chips(&mut self, g: &mut Gfx, board: &Board, phase: Phase, r: Rect) {
        let t = g.tokens;
        let style = g.ty.caption;
        let ([left, right], words) = self.chip_layout(g, board, phase, r);
        let (h, cy) = (left.h, left.cy());
        let word_w = g.text.measure(phase.name(), &style).ceil();
        let a = self.chips[0].value();
        g.canvas.rrect(left, h / 2.0, t.surface1, 0.95 * a);
        let base = (cy + style.size * 0.36).round();
        g.text.draw(&mut g.canvas, left.x + 9.0, base, phase.name(), &style, t.ink, a);
        for (k, p) in Phase::ALL.iter().enumerate() {
            let on = *p == phase;
            g.canvas.circle(left.x + word_w + 16.0 + k as f32 * 7.0, cy, 2.3, if on { t.ink } else { t.faint }, a);
        }
        let b = self.chips[1].value();
        g.canvas.rrect(right, h / 2.0, t.surface1, 0.95 * b);
        g.canvas.circle(right.x + 10.0, cy, 3.6, robot_color(board, self.picked, t.tone), b);
        g.text.draw(&mut g.canvas, right.x + 18.0, base, &words, &style, t.ink, b);
        let o = Rect::new(r.x, r.y, 0.0, 0.0);
        self.chip_rects = [left.offset(-o.x, -o.y), right.offset(-o.x, -o.y)];
    }

    /// The words of a note being typed, along the tile's foot.
    fn draw_typing(&self, g: &mut Gfx, r: Rect) {
        let Mode::Typing { field, .. } = &self.mode else { return };
        let t = g.tokens;
        let ty = g.ty;
        let h = (ty.label.size + 12.0).round();
        let bar = Rect::new(r.x + 5.0, r.bottom() - 5.0 - h, r.w - 10.0, h);
        g.canvas.rrect(bar, h / 2.0, t.surface2, 0.97);
        g.canvas.stroke_rrect(bar, h / 2.0, 1.5, g.ring_color(), 1.0);
        let cy = bar.cy();
        let label_w = g.text.draw(&mut g.canvas, bar.x + 12.0, (cy + ty.caption.size * 0.36).round(), field.label, &ty.caption, t.dim, 1.0);
        let keys = "enter · esc";
        let keys_w = g.text.measure(keys, &ty.caption);
        g.text.draw(&mut g.canvas, bar.right() - 12.0 - keys_w, (cy + ty.caption.size * 0.36).round(), keys, &ty.caption, t.dim, 1.0);
        let x = bar.x + 12.0 + label_w + 8.0;
        let room = bar.right() - 12.0 - keys_w - 8.0 - x;
        let mut shown: &str = &field.text;
        while g.text.measure(shown, &ty.label) > room - 3.0 {
            let mut chars = shown.chars();
            chars.next();
            shown = chars.as_str();
        }
        let w = g.text.draw(&mut g.canvas, x, (cy + ty.label.size * 0.36).round(), shown, &ty.label, t.ink, 1.0);
        g.canvas.fill_rect(Rect::new(x + w + 1.0, cy - ty.label.size * 0.5, 1.5, ty.label.size), t.signal, 1.0);
        g.copy_field = Some(field.text.clone());
    }
}

fn label_of(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Label => "note",
        NoteKind::Zone => "drop zone",
    }
}

impl Screen for Strategy {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn greeting(&self) -> Option<&'static str> {
        Some("? shows the keys")
    }

    /// Always: the plain F X Z C must reach the board as letters to tell
    /// them from Fn's arrows (coarse steps and fine), and its other keys
    /// are letters too. Backspace removes; Esc leaves.
    fn wants_text(&self) -> bool {
        true
    }

    fn keeps_awake(&self) -> bool {
        self.play.is_some()
    }

    /// Only a note being typed takes a paste: anywhere else its letters
    /// would be the board's keys.
    fn paste(&mut self, text: &str, cx: &mut Cx) -> Nav {
        if matches!(self.mode, Mode::Typing { .. }) {
            for c in text.chars() {
                self.typing_key(Key::Char(if c.is_control() { ' ' } else { c }), cx);
            }
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if matches!(self.mode, Mode::Typing { .. }) {
            return self.typing_key(key, cx);
        }
        // Any key stops a play; Space and Esc only that.
        if self.play.take().is_some() && matches!(key, Key::Char(' ') | Key::Back) {
            return Nav::Stay;
        }
        match key {
            Key::Back => match self.mode {
                Mode::Moving { .. } => self.cancel_move(),
                Mode::Drawing { .. } => self.finish_path(),
                _ => return Nav::Pop,
            },
            Key::Leave => return Nav::Pop,
            Key::Char('f' | 'F') => self.step(0, 1, false),
            Key::Char('x' | 'X') => self.step(0, -1, false),
            Key::Char('z' | 'Z') => self.step(-1, 0, false),
            Key::Char('c' | 'C') => self.step(1, 0, false),
            Key::Up => self.step(0, 1, true),
            Key::Down => self.step(0, -1, true),
            Key::Left => self.step(-1, 0, true),
            Key::Right => self.step(1, 0, true),
            Key::Enter => self.enter(cx),
            Key::Char(c @ '1'..='6') => self.pick(c as usize - '1' as usize),
            Key::Char('r' | 'R') => self.place(),
            Key::Char('p' | 'P') => self.toggle_path(),
            Key::Char('n' | 'N') => self.start_typing(NoteKind::Label),
            Key::Char('d' | 'D') => self.start_typing(NoteKind::Zone),
            Key::Backspace | Key::Delete => self.remove(cx),
            Key::Char('u' | 'U') => {
                self.finish_path();
                self.cancel_move();
                if !self.plan.borrow_mut().undo() {
                    *cx.say = Some("nothing to undo".into());
                }
            }
            Key::Char('a' | 'A') => self.set_phase(Phase::Auto),
            Key::Char('t' | 'T') => self.set_phase(Phase::Teleop),
            Key::Char('e' | 'E') => self.set_phase(Phase::Endgame),
            Key::Tab => {
                let next = self.phase().next();
                self.set_phase(next);
            }
            Key::Char(' ') => self.start_play(cx),
            Key::Char('+' | '=') => self.zoom(1),
            Key::Char('-' | '_') => self.zoom(-1),
            Key::Char('m' | 'M') => {
                self.settle_modes();
                return Nav::Push(Box::new(Menu::new(self.plan.clone())));
            }
            Key::Char('?') => return Nav::Push(Box::new(KeysPage::default())),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.cur[0].tick(dt) | self.cur[1].tick(dt);
        moving |= self.camera.tick(dt);
        moving |= self.reveal.tick(dt) | self.lift.tick(dt);
        moving |= self.chips[0].tick(dt) | self.chips[1].tick(dt);
        if let Some(play) = &mut self.play {
            moving = true;
            if !play.tick(dt) {
                let next = play.phase.index() + 1;
                let following = {
                    let plan = self.plan.borrow();
                    Phase::ALL.get(next..).and_then(|rest| rest.iter().find_map(|p| Play::of(&plan.board, *p, self.calm)))
                };
                self.play = None;
                if let Some(play) = following {
                    self.set_phase(play.phase);
                    self.play = Some(play);
                }
            }
        }
        moving
    }

    fn resume(&mut self) {
        let (key, label) = {
            let plan = self.plan.borrow();
            (plan.board.key.clone(), plan.board.label.clone())
        };
        self.title = label;
        if key != self.key {
            // Another match's board: from its start, the team's robot picked.
            self.key = key;
            self.picked = 0;
            self.mode = Mode::Idle;
            self.play = None;
            let start = self.plan.borrow().board.start_of(Phase::Auto, 0);
            if let Some(at) = start {
                self.aim(at);
            }
        }
    }

    fn set_aside(&mut self) {
        self.settle_modes();
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        self.calm = g.calm;
        let top = g.top().round();
        // The whole height, to just above the panel's foot.
        self.viewport = Rect::new(PAD, top, W - 2.0 * PAD, (H - 4.0 - top).round());
        let r = self.viewport;
        let local = self.local();
        let plan = self.plan.clone();
        let plan = plan.borrow();
        let (board, phase) = (&plan.board, plan.phase);
        let look = Look::of(&t);
        let view = self.camera.view(local, MARGIN, board.ours_red);
        self.view = Some(view);
        let revealing = self.reveal.value() < 0.999;
        let moving = match self.mode {
            Mode::Moving { robot, .. } => Some(robot),
            _ => None,
        };
        let mut skip = [self.play.is_some(); ROBOTS];
        if let Some(m) = moving {
            skip[m] = true;
        }
        let picked = self.play.is_none().then_some(self.picked);
        let ring = g.ring_color();
        let (keep, _) = self.chip_layout(g, board, phase, local);
        let scene = Scene { board, phase, picked, skip, paths: !revealing, bounds: local, keep: &keep };
        // The picture at rest: painted again only when what it shows changes.
        let key = field::field_key(
            &view,
            &look,
            field::key_of(|h| {
                board.phase_hash(phase).hash(h);
                (phase, picked, skip, revealing, g.text_size as u8, ring.to_u32()).hash(h);
                for r in keep {
                    [r.x, r.w].map(|v| v as i32).hash(h);
                }
            }),
        );
        let (w, h) = (r.w.round() as i32, r.h.round() as i32);
        if self.cache.key != Some(key) || self.cache.scratch.as_ref().is_none_or(|c| c.w != w || c.h != h) {
            let mut scratch = self.cache.scratch.take().filter(|c| c.w == w && c.h == h).unwrap_or_else(|| Canvas::new(w, h));
            field::paint(&mut scratch, &view, &look);
            // Drawn with the panel's own helpers onto the picture instead.
            let screen = std::mem::replace(&mut g.canvas, scratch);
            let ox = std::mem::replace(&mut g.ox, 0.0);
            self.cache.words = draw_scene(g, &view, &scene, look.ring);
            g.ox = ox;
            self.cache.scratch = Some(std::mem::replace(&mut g.canvas, screen));
            self.cache.key = Some(key);
        }
        let radius = g.tile_radius();
        let Cache { layer, scratch, .. } = &mut self.cache;
        let scratch = scratch.as_ref().expect("painted");
        layer.show(g, r, radius, key, |c| c.px.copy_from_slice(&scratch.px));
        // What moves, over it.
        let bounds = r.offset(g.ox, 0.0);
        let on = view.moved(bounds.x, bounds.y);
        g.canvas.set_clip(bounds);
        if revealing {
            let shown = self.reveal.value();
            for p in &board.layer(phase).paths {
                draw_path(&mut g.canvas, &on, &p.points, robot_color(board, p.robot, t.tone), shown);
            }
        }
        // Words placed live keep clear of the notes' at rest.
        let keep_live = keep.map(|r| r.offset(bounds.x, bounds.y));
        let mut placer = Placer::new(bounds, &keep_live, &on, &board.layer(phase).paths);
        placer.taken.extend(self.cache.words.iter().map(|r| (r.offset(bounds.x, bounds.y), 2.5)));
        if let Some(play) = &self.play {
            let mut driving = Vec::new();
            for robot in 0..ROBOTS {
                let route = play.routes.iter().find(|(r, _)| *r == robot).map(|(_, route)| route);
                let (at, heading) = match (route, board.start_of(phase, robot)) {
                    (Some(route), _) => play.along(route),
                    (None, Some(at)) => (at, None),
                    (None, None) => continue,
                };
                let body = draw_body(g, &on, board, robot, at, heading.unwrap_or_else(|| facing(board, robot)), false, 0.0, look.ring);
                placer.taken.push((body, 3.0));
                driving.push((robot, at));
            }
            robot_chips(g, &on, board, &driving, &mut placer);
        }
        let cursor = (self.cur[0].value(), self.cur[1].value());
        if let Some(robot) = moving {
            let body = draw_body(g, &on, board, robot, cursor, facing(board, robot), true, self.lift.value(), look.ring);
            placer.taken.push((body, 3.0));
            robot_chips(g, &on, board, &[(robot, cursor)], &mut placer);
        }
        if let (Some(end), Mode::Drawing { robot, .. }) = (self.path_end(), &self.mode) {
            let w = (on.scale * 0.12).clamp(2.0, 4.0);
            let (a, b) = (on.to_screen(end.0, end.1), on.to_screen(cursor.0, cursor.1));
            dashes(&mut g.canvas, a, b, w * 0.8, robot_color(board, *robot, t.tone), 0.75);
        }
        if self.play.is_none() {
            let (sx, sy) = on.to_screen(cursor.0, cursor.1);
            draw_cursor(&mut g.canvas, sx, sy, ring, look.ring);
        }
        g.canvas.clear_clip();
        if matches!(self.mode, Mode::Typing { .. }) {
            self.draw_typing(g, bounds);
        } else {
            self.draw_chips(g, board, phase, bounds);
        }
    }
}

// ------------------------------------------------------------- the menu ---

const MENU: [&str; 6] = ["Match", "Use a template", "Save as a template", "Share as QR codes", "Save a picture", "Clear"];

/// The board's menu: another match, templates, sharing, a picture.
struct Menu {
    plan: Shared,
    list: ListState,
    done: bool,
    /// Make the picture at the next frame (it needs the panel's drawing).
    export: bool,
    saving: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    /// Templates kept, and codes the board takes, as of opening or coming back.
    counts: (usize, usize),
}

impl Menu {
    fn new(plan: Shared) -> Menu {
        let mut menu = Menu { plan, list: ListState::default(), done: false, export: false, saving: None, counts: (0, 0) };
        menu.count();
        menu
    }

    fn count(&mut self) {
        let plan = self.plan.borrow();
        self.counts = (board::templates(&plan.dir).len(), qr::chunks(&plan.board).map_or(0, |c| c.len()));
    }
}

impl Screen for Menu {
    fn title(&self) -> &str {
        "Board"
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn resume(&mut self) {
        if std::mem::take(&mut self.plan.borrow_mut().close_menu) {
            self.done = true;
        }
        self.count();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => match self.list.focus {
                0 => return Nav::Push(Box::new(Matches::new(self.plan.clone(), cx.robot))),
                1 => return Nav::Push(Box::new(Templates::new(self.plan.clone()))),
                2 => return Nav::Push(Box::new(Naming::new(self.plan.clone()))),
                3 => {
                    let plan = self.plan.borrow();
                    match qr::chunks(&plan.board) {
                        Ok(texts) => return Nav::Push(Box::new(Codes::new(&plan.board.label, &texts))),
                        Err(why) => *cx.say = Some(why),
                    }
                }
                4 => self.export = true,
                _ => {
                    let mut plan = self.plan.borrow_mut();
                    let phase = plan.phase;
                    if plan.board.layer(phase).is_empty() {
                        *cx.say = Some(format!("{} is empty already", phase.name()));
                    } else {
                        plan.change(|b| *b.layer_mut(phase) = Layer::default());
                        *cx.say = Some(format!("{} cleared · u undoes it", phase.name()));
                        self.done = true;
                    }
                }
            },
            // Straight to the team, over Team, rather than as codes.
            Key::Char('t') => {
                let plan = self.plan.borrow();
                let label = format!("board {}", plan.board.label);
                *cx.say = Some(match qr::chunks(&plan.board).and_then(|texts| crate::team::share(crate::team::wire::ShareKind::Strategy, &label, &texts)) {
                    Ok(_) => "sent to the team".into(),
                    Err(why) => why,
                });
            }
            other => crate::screens::list_key(&mut self.list, other, MENU.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.export
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(Ok(done)) = self.saving.as_ref().map(|rx| rx.try_recv()) {
            self.saving = None;
            *cx.say = Some(match done {
                Ok(path) => format!("saved {}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())),
                Err(why) => why,
            });
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if std::mem::take(&mut self.export) {
            let (picture, file) = {
                let plan = self.plan.borrow();
                (picture(g, &plan.board), pictures().join(format!("{}-strategy.png", board::slug(&plan.board.label))))
            };
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                tx.send(write_png(&picture, &file).map(|_| file)).ok();
            });
            self.saving = Some(rx);
        }
        let plan = self.plan.borrow();
        let board = &plan.board;
        let layer = board.layer(plan.phase);
        let (templates, codes) = self.counts;
        let values = [
            board.label.clone(),
            match templates {
                0 => "none yet".into(),
                n => format!("{n} kept"),
            },
            String::new(),
            format!("{codes} {} · t: team", if codes == 1 { "code" } else { "codes" }),
            "for companion".into(),
            format!("{} · {} paths", plan.phase.name(), layer.paths.len()),
        ];
        let top = g.top();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, MENU.len(), top, row_h, 3.0, H) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let name = if i == 5 { format!("Clear {}", plan.phase.name()) } else { MENU[i].to_owned() };
            let value = g.text.fit(&values[i], &g.ty.label, r.w * 0.45);
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, MENU.len(), top, H);
    }
}

/// The team's matches, to open another's board.
struct Matches {
    plan: Shared,
    list: ListState,
    rows: Vec<(Option<crate::schedule::Match>, bool)>,
    team: u32,
    known: bool,
}

impl Matches {
    fn new(plan: Shared, robot: &Robot) -> Matches {
        let view = robot.schedule.view();
        let known = view.is_some();
        let team = view.as_ref().map_or(0, |v| v.team);
        let (dir, open) = {
            let p = plan.borrow();
            (p.dir.clone(), p.board.key.clone())
        };
        let mut rows: Vec<(Option<crate::schedule::Match>, bool)> = view.map(|v| v.matches).unwrap_or_default().into_iter().map(|m| (board::has_board(&dir, &m.key), m)).map(|(planned, m)| (Some(m), planned)).collect();
        rows.push((None, board::has_board(&dir, "practice")));
        let mut list = ListState::default();
        list.focus = rows.iter().position(|(m, _)| m.as_ref().map_or("practice", |m| m.key.as_str()) == open).unwrap_or(0);
        Matches { plan, list, rows, team, known }
    }
}

impl Screen for Matches {
    fn title(&self) -> &str {
        "Matches"
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                let Some((m, _)) = self.rows.get(self.list.focus) else { return Nav::Stay };
                let mut plan = self.plan.borrow_mut();
                let board = match m {
                    Some(m) => board_for(&plan.dir, m, self.team),
                    None => board::load_board(&plan.dir, "practice").unwrap_or_else(|| Board::practice(crate::tools::team(cx.robot))),
                };
                plan.open(board);
                plan.close_menu = true;
                return Nav::Pop;
            }
            other => crate::screens::list_key(&mut self.list, other, self.rows.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let now = crate::reminders::now();
        let top = g.top();
        let row_h = g.row_h();
        let open = self.plan.borrow().board.key.clone();
        let bottom = g.above_hint();
        for (i, y) in self.list.layout(g, self.rows.len(), top, row_h, 3.0, bottom) {
            let (m, planned) = &self.rows[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            g.surface(r, row_h * 0.5, if focused { t.surface2 } else { t.surface1 });
            let (label, side, key) = match m {
                Some(m) => (m.label.as_str(), m.alliance(self.team), m.key.as_str()),
                None => ("Practice", None, "practice"),
            };
            g.dot(r.x + 12.0, r.cy(), 3.5, match side {
                Some("red") => crate::schedule::RED,
                Some("blue") => crate::schedule::BLUE,
                _ => t.faint,
            });
            g.text_at(r.x + 22.0, r.cy(), Align::Left, label, ty.name, t.ink);
            let (value, color) = match (key == open, planned, m.as_ref().and_then(|m| m.time)) {
                (true, ..) => ("open".to_owned(), t.ink),
                (false, true, _) => ("planned".to_owned(), t.ink),
                (false, false, Some(at)) => (crate::schedule::when(at, now), t.dim),
                _ => (String::new(), t.dim),
            };
            g.text_at(r.right() - 10.0, r.cy(), Align::Right, &value, ty.label, color);
            if focused {
                g.focus_ring(r, row_h * 0.5);
            }
        }
        self.list.scrollbar(g, self.rows.len(), top, bottom);
        g.hint(if self.known { "enter: open its board" } else { "no schedule: set the team in settings › robot" });
    }
}

/// The templates, to put one on the board.
struct Templates {
    plan: Shared,
    list: ListState,
    all: Vec<Board>,
    /// Backspace once asks; again removes.
    removing: Option<usize>,
}

impl Templates {
    fn new(plan: Shared) -> Templates {
        let all = board::templates(&plan.borrow().dir);
        Templates { plan, list: ListState::default(), all, removing: None }
    }
}

impl Screen for Templates {
    fn title(&self) -> &str {
        "Templates"
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let Some(chosen) = self.all.get(self.list.focus).cloned() else {
            return if key == Key::Back || key == Key::Enter { Nav::Pop } else { Nav::Stay };
        };
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => {
                let mut plan = self.plan.borrow_mut();
                plan.change(|b| b.apply(&chosen));
                plan.close_menu = true;
                *cx.say = Some(format!("{}: {}", plan.board.label, chosen.label));
                return Nav::Pop;
            }
            Key::Backspace | Key::Delete if self.removing == Some(self.list.focus) => {
                board::remove_template(&self.plan.borrow().dir, &chosen);
                self.all.remove(self.list.focus);
                self.removing = None;
                *cx.say = Some(format!("removed {}", chosen.label));
            }
            Key::Backspace | Key::Delete => {
                self.removing = Some(self.list.focus);
                *cx.say = Some(format!("backspace again removes {}", chosen.label));
            }
            other => {
                self.removing = None;
                crate::screens::list_key(&mut self.list, other, self.all.len());
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.all.is_empty() {
            g.empty("strategy", "no templates yet: keep a board as one with save as a template");
            return;
        }
        let top = g.top();
        let row_h = g.row_h();
        let bottom = g.above_hint();
        for (i, y) in self.list.layout(g, self.all.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let plan = &self.all[i];
            let paths: usize = plan.layers.iter().map(|l| l.paths.len()).sum();
            let value = format!("{paths} {}", if paths == 1 { "path" } else { "paths" });
            let name = g.text.fit(&plan.label, &g.ty.body, r.w * 0.66);
            g.row(r, &name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, self.all.len(), top, bottom);
        let label = self.plan.borrow().board.label.clone();
        let on = format!("enter: use on {label} · backspace: remove");
        g.hint_any(&[&on, "enter: use · backspace: remove", "enter: use"]);
    }
}

/// A name for the board kept as a template.
struct Naming {
    plan: Shared,
    field: Field,
}

impl Naming {
    fn new(plan: Shared) -> Naming {
        Naming { plan, field: Field::new("template name", "", 32) }
    }
}

impl Screen for Naming {
    fn title(&self) -> &str {
        "Save as a template"
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match self.field.key(key, |c| !c.is_control()) {
            Typed::More => Nav::Stay,
            Typed::Cancelled => Nav::Pop,
            Typed::Done(name) if name.is_empty() => Nav::Stay,
            Typed::Done(name) => {
                let plan = self.plan.borrow();
                *cx.say = Some(match board::save_template(&plan.dir, &plan.board.template(&name)) {
                    Ok(()) => format!("kept as a template: {name}"),
                    Err(_) => "could not keep the template".into(),
                });
                Nav::Pop
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        self.field.draw(g, "enter: keep it · esc: cancel");
        let y = g.top() + Field::height(g) + 8.0;
        let lines = crate::help::wrap(g, "all three phases, turned to fit the team's side whichever alliance it is on", ty.caption, W - 2.0 * PAD - 8.0);
        for (k, line) in lines.iter().take(3).enumerate() {
            g.text_at(PAD + 4.0, y + ty.caption.size * 0.5 + k as f32 * (ty.caption.size + 4.0), Align::Left, line, ty.caption, t.dim);
        }
    }
}

/// The board's QR codes, one at a time, for another handheld's camera.
struct Codes {
    title: String,
    codes: Vec<qrcodegen::QrCode>,
    page: usize,
    /// The marker under the dots, gliding to the code shown.
    dot: Animated,
}

impl Codes {
    fn new(label: &str, texts: &[String]) -> Codes {
        let codes = texts.iter().filter_map(|t| qrcodegen::QrCode::encode_text(t, qrcodegen::QrCodeEcc::Medium).ok()).collect();
        Codes { title: format!("Share {label}"), codes, page: 0, dot: Animated::new(0.0, spatial::fast()).with_rest(0.003) }
    }
}

impl Screen for Codes {
    fn title(&self) -> &str {
        &self.title
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let n = self.codes.len();
        match key {
            Key::Back | Key::Enter => return Nav::Pop,
            Key::Left | Key::Up if self.page > 0 => self.page -= 1,
            Key::Right | Key::Down | Key::Char(' ') if self.page + 1 < n => self.page += 1,
            _ => return Nav::Stay,
        }
        self.dot.set(self.page as f32);
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.dot.tick(dt)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let n = self.codes.len();
        let Some(qr) = self.codes.get(self.page) else { return };
        let side = H - top - 4.0;
        let code = crate::share::draw_qr(g, qr, Rect::new(PAD, top, side, side));
        let x = code.right() + 12.0;
        let width = W - PAD - x;
        let display = ty.display(30.0);
        let big = format!("{} of {n}", self.page + 1);
        g.text_fit(x, code.y + display.size * 0.55, &big, display, t.ink, width);
        let line_h = ty.caption.size + 4.0;
        let words = crate::help::wrap(g, "scan each in the camera, in any order", ty.caption, width);
        let mut y = code.y + display.size * 1.15 + 8.0;
        for line in words.iter().take(3) {
            g.text_at(x, y + line_h * 0.5, Align::Left, line, ty.caption, t.dim);
            y += line_h;
        }
        // The codes as dots, the one shown marked; with arrows to turn.
        let cy = code.bottom() - 8.0;
        let gap = 10.0;
        for k in 0..n {
            g.dot(x + 4.0 + k as f32 * gap, cy, 2.2, t.faint);
        }
        let at = x + 4.0 + self.dot.value() * gap;
        g.surface(Rect::new(at - 5.0, cy - 2.6, 10.0, 5.2), 2.6, t.ink);
        if n > 1 {
            let keys = g.text.fit("← → turns", &ty.caption, (width - n as f32 * gap - 8.0).max(0.0));
            g.text_at(W - PAD, cy, Align::Right, &keys, ty.caption, t.dim);
        }
    }
}

/// What each key does on the board.
#[derive(Default)]
struct KeysPage {
    list: ListState,
}

const KEYS: [(&str, &str); 14] = [
    ("Move the cursor", "f x z c"),
    ("Fine moves", "fn + f x z c"),
    ("Pick a robot", "1 – 6"),
    ("Pick up · drop", "enter"),
    ("Put the robot here", "r"),
    ("Path: start · end", "p"),
    ("Add a waypoint", "enter"),
    ("Note · drop zone", "n · d"),
    ("Remove · undo", "bksp · u"),
    ("Auto · teleop · endgame", "a t e"),
    ("Next phase", "tab"),
    ("Play", "space"),
    ("Zoom", "+ −"),
    ("Templates, share", "m"),
];

impl Screen for KeysPage {
    fn title(&self) -> &str {
        "Keys"
    }

    fn app(&self) -> &str {
        "Strategy"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back | Key::Enter => Nav::Pop,
            other => {
                crate::screens::list_key(&mut self.list, other, KEYS.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let top = g.top();
        let row_h = g.row_h();
        for (i, y) in self.list.layout(g, KEYS.len(), top, row_h, 3.0, H) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (what, keys) = KEYS[i];
            let name = g.text.fit(what, &g.ty.body, r.w * 0.6);
            g.row(r, &name, keys, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, KEYS.len(), top, H);
    }
}

// -------------------------------------------------------------- sharing ---

/// Where pictures go for Catalyst Companion: `/data/strategy`, or beside
/// the settings on a computer.
pub fn pictures() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        "/data/strategy".into()
    } else {
        board::home()
    }
}

/// The board as a picture: its three phases one above the other, in the
/// light tone (for a page or a projector), two and a half times the
/// panel's scale. Drawn with the panel's own helpers on a canvas of its
/// own.
pub fn picture(g: &mut Gfx, board: &Board) -> Canvas {
    const K: f32 = 2.5;
    const FIELD_W: f32 = 900.0;
    let pad = 24.0;
    let margin = MARGIN * K;
    let field_h = ((FIELD_W - 2.0 * margin) / LENGTH * WIDTH + 2.0 * margin).round();
    let mut ty = Type::of(TextSize::Standard);
    for s in [&mut ty.caption, &mut ty.label, &mut ty.body, &mut ty.name, &mut ty.title] {
        s.size *= K;
    }
    ty.display_scale *= K;
    let head_h = (ty.title.size + ty.caption.size + 26.0).round();
    let phase_h = (ty.label.size + 18.0).round();
    let (w, h) = (FIELD_W + 2.0 * pad, pad + head_h + 3.0 * (phase_h + field_h + pad));
    let saved = (std::mem::replace(&mut g.canvas, Canvas::new(w as i32, h as i32)), g.ox, g.ty, g.tokens);
    g.ox = 0.0;
    g.ty = ty;
    g.tokens = Tokens::light();
    let t = g.tokens;
    g.canvas.clear(t.ground);
    let look = Look::of(&t);
    // The match, and who is in it.
    let names = |range: std::ops::Range<usize>| range.map(|r| board.name(r)).collect::<Vec<_>>().join("  ");
    g.text(pad, pad + ty.title.size * 0.8, &board.label, ty.title, t.ink);
    let (ours, theirs) = (if board.ours_red { "red" } else { "blue" }, if board.ours_red { "blue" } else { "red" });
    let line = format!("{ours}: {}   ·   {theirs}: {}", names(0..OURS), names(OURS..ROBOTS));
    g.text(pad, pad + ty.title.size + 12.0 + ty.caption.size * 0.8, &line, ty.caption, t.dim);
    let local = Rect::new(0.0, 0.0, FIELD_W, field_h);
    let view = View::new(local, margin, 1.0, (LENGTH / 2.0, WIDTH / 2.0), board.ours_red);
    let mut layer = field::Layer::default();
    let key = field::field_key(&view, &look, 0);
    let mut y = pad + head_h;
    for phase in Phase::ALL {
        g.text(pad + 4.0, y + phase_h * 0.5 + ty.label.size * 0.36, phase.name(), ty.label, t.ink);
        y += phase_h;
        let r = Rect::new(pad, y, FIELD_W, field_h);
        layer.show(g, r, 14.0 * K, key, |c| field::paint(c, &view, &look));
        let on = view.moved(r.x, r.y);
        let scene = Scene { board, phase, picked: None, skip: [false; ROBOTS], paths: true, bounds: r, keep: &[] };
        draw_scene(g, &on, &scene, look.ring);
        y += field_h + pad;
    }
    let (canvas, ox, ty, tokens) = saved;
    g.ox = ox;
    g.ty = ty;
    g.tokens = tokens;
    std::mem::replace(&mut g.canvas, canvas)
}

pub fn write_png(canvas: &Canvas, path: &std::path::Path) -> Result<(), String> {
    let mut rgb = Vec::with_capacity(canvas.px.len() * 3);
    for p in &canvas.px {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, canvas.w as u32, canvas.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|_| "could not make the picture".to_owned())?;
        writer.write_image_data(&rgb).map_err(|_| "could not make the picture".to_owned())?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| "could not make the strategy folder".to_owned())?;
    }
    crate::sys::write_atomic(path, &bytes).map_err(|_| "could not save the picture".to_owned())
}

// ---------------------------------------------------- the rest of the OS ---

/// The island's queue notice for a match with a plan: it offers the board
/// (Enter opens Strategy on it) instead of the schedule.
pub fn offer(n: crate::notify::Notification, key: &str, label: &str) -> crate::notify::Notification {
    if board::has_board(&board::home(), key) {
        n.body(format!("{label}'s plan is on the strategy board")).action("builtin:Strategy")
    } else {
        n
    }
}

/// Whether a QR code the camera read is one of a board's.
pub fn is_board_code(text: &str) -> bool {
    qr::parse(text).is_some()
}

/// The camera's card for a board's code: its symbol, its line, and what
/// Enter does.
pub fn card(text: &str) -> (&'static str, String, &'static str) {
    let line = qr::parse(text).map_or_else(String::new, |c| format!("Strategy board · code {} of {}", c.k, c.n));
    ("strategy", line, "enter: add to Strategy")
}

/// A board sent over Team (team/): its codes' texts, read as the camera
/// reads them (but gathered apart from the camera's), and kept for its
/// match.
fn gather_team(texts: &[&str]) -> Result<Board, String> {
    let mut gather = qr::Gather::default();
    let mut last = None;
    for t in texts {
        last = Some(gather.add(qr::parse(t).ok_or("not a Strategy board")?));
    }
    match last {
        Some(qr::Scan::Done(board)) => {
            board::save_board(&board::home(), &board).map_err(|_| "could not keep the board".to_owned())?;
            Ok(*board)
        }
        Some(qr::Scan::Broken) => Err("a board this handheld cannot read".into()),
        _ => Err("part of a board is missing".into()),
    }
}

/// A board from Team kept for its match; its label.
pub fn take_team_board(texts: &[&str]) -> Result<String, String> {
    gather_team(texts).map(|b| b.label)
}

/// A board from Team, opened (Enter on it in Team's thread).
pub fn open_team_board(texts: &[&str]) -> Result<Box<dyn Screen>, String> {
    gather_team(texts).map(|b| Box::new(Strategy::with_board(b, board::home())) as Box<dyn Screen>)
}

/// Enter on a board's code in the camera: kept, and with the last of them
/// the board is saved for its match and opens. Otherwise what to say.
pub fn scanned(text: &str) -> Result<Box<dyn Screen>, String> {
    match qr::scanned(text) {
        None => Err("not a Strategy board".into()),
        Some(qr::Scan::Kept { have, of }) => Err(format!("board: {have} of {of} codes · scan the next")),
        Some(qr::Scan::Broken) => Err("those codes did not make a board: scan them again".into()),
        Some(qr::Scan::Done(board)) => {
            let dir = board::home();
            board::save_board(&dir, &board).map_err(|_| "could not keep the board".to_owned())?;
            Ok(Box::new(Strategy::with_board(*board, dir)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::board::tests::full;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("catalyst-strategy-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    fn press(s: &mut Strategy, keys: &[Key]) -> Option<String> {
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut said = None;
        for k in keys {
            let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut said, at_rest: true };
            s.key(*k, &mut cx);
        }
        said
    }

    fn settle(s: &mut Strategy) {
        for _ in 0..240 {
            s.tick(1.0 / 60.0);
        }
    }

    #[test]
    fn the_cursor_steps_the_way_the_key_points_on_the_screen_on_either_side() {
        let r = Rect::new(0.0, 0.0, 304.0, 138.0);
        for red_left in [false, true] {
            let view = View::new(r, MARGIN, 1.0, (LENGTH / 2.0, WIDTH / 2.0), red_left);
            let at = (5.0, 3.0);
            let (sx, sy) = view.to_screen(at.0, at.1);
            for (right, up) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (dx, dy) = cursor_step(right, up, COARSE, red_left);
                assert!((dx.abs() + dy.abs() - COARSE).abs() < 1e-6);
                let (tx, ty) = view.to_screen(at.0 + dx, at.1 + dy);
                // Right is right on the screen, up is up (y grows down there).
                assert!(((tx - sx) * right as f32) > 0.0 || right == 0, "{red_left} {right} {up}");
                assert!(((sy - ty) * up as f32) > 0.0 || up == 0, "{red_left} {right} {up}");
                // Back from the screen to the field: the same place.
                let back = view.to_field(tx, ty);
                assert!((back.0 - at.0 - dx).abs() < 1e-3 && (back.1 - at.1 - dy).abs() < 1e-3);
            }
        }
        // Red: the team's wall on the left, so right goes toward blue's (x falls).
        assert_eq!(cursor_step(1, 0, 0.5, true), (-0.5, 0.0));
        assert_eq!(cursor_step(0, 1, 0.1, false), (0.0, 0.1));
    }

    #[test]
    fn the_cursor_stays_on_the_field_and_fine_steps_are_finer() {
        let mut s = Strategy::with_board(Board::practice(Some(5805)), dir("cursor"));
        let from = s.cursor;
        press(&mut s, &[Key::Char('c')]);
        assert!((s.cursor.0 - from.0 - COARSE).abs() < 1e-4, "blue: right is along");
        press(&mut s, &[Key::Left]);
        assert!((s.cursor.0 - from.0 - COARSE + FINE).abs() < 1e-4, "fn: a fine step back");
        press(&mut s, &[Key::Char('f'); 40]);
        assert_eq!(s.cursor.1, WIDTH, "held at the guardrail");
        settle(&mut s);
        assert!((s.cur[1].value() - WIDTH).abs() < 0.01, "the drawn cursor arrives");
    }

    #[test]
    fn robots_are_placed_picked_up_and_dropped_and_undone() {
        let d = dir("markers");
        let mut s = Strategy::with_board(Board::practice(Some(5805)), d.clone());
        // Robot 2 put at the cursor in teleop.
        press(&mut s, &[Key::Char('t'), Key::Char('2'), Key::Char('c'), Key::Char('c'), Key::Char('r')]);
        let at = s.cursor;
        assert_eq!(s.plan().board.layer(Phase::Teleop).markers[1], Some(at));
        // Enter on it picks it up; moved, Enter drops it there.
        press(&mut s, &[Key::Enter]);
        assert!(matches!(s.mode, Mode::Moving { robot: 1, .. }));
        press(&mut s, &[Key::Char('f'), Key::Char('f'), Key::Enter]);
        let dropped = s.plan().board.layer(Phase::Teleop).markers[1].unwrap();
        assert!((dropped.1 - at.1 - 2.0 * COARSE).abs() < 1e-4);
        // Esc while carrying puts it back.
        press(&mut s, &[Key::Enter, Key::Char('z'), Key::Back]);
        assert_eq!(s.plan().board.layer(Phase::Teleop).markers[1], Some(dropped));
        assert!(matches!(s.mode, Mode::Idle));
        // Saved as it goes; undo takes the drop back.
        let kept = board::load_board(&d, "practice").unwrap().layer(Phase::Teleop).markers[1].unwrap();
        assert!(distance(kept, dropped) < 0.01, "kept to the centimetre");
        press(&mut s, &[Key::Char('u')]);
        assert_eq!(s.plan().board.layer(Phase::Teleop).markers[1], Some(at));
        // Backspace on it (the cursor back down to it) takes it off this phase.
        press(&mut s, &[Key::Char('x'), Key::Char('x'), Key::Backspace]);
        assert_eq!(s.plan().board.layer(Phase::Teleop).markers[1], None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_path_starts_at_its_robot_gains_waypoints_and_ends() {
        let mut s = Strategy::with_board(Board::practice(Some(5805)), dir("paths"));
        let start = s.plan().board.start_of(Phase::Auto, 0).unwrap();
        press(&mut s, &[Key::Char('p')]);
        assert!(s.plan().board.layer(Phase::Auto).paths.is_empty(), "nothing kept until a waypoint");
        press(&mut s, &[Key::Char('c'), Key::Char('c'), Key::Enter, Key::Char('f'), Key::Enter, Key::Enter]);
        {
            let plan = s.plan();
            let p = &plan.board.layer(Phase::Auto).paths[0];
            assert_eq!((p.robot, p.points.len(), p.points[0]), (0, 3, start), "a repeat is not a waypoint");
        }
        // Backspace takes the last back; p ends it.
        press(&mut s, &[Key::Backspace, Key::Char('p')]);
        assert_eq!(s.plan().board.layer(Phase::Auto).paths[0].points.len(), 2);
        assert!(matches!(s.mode, Mode::Idle));
        // Enter on open field starts the next from where that one ended, for robot 4.
        let end = s.plan().board.end_of(Phase::Auto, 0).unwrap();
        press(&mut s, &[Key::Char('4'), Key::Char('x'), Key::Char('x'), Key::Char('x'), Key::Enter, Key::Back]);
        let start4 = s.plan().board.start_of(Phase::Auto, 3).unwrap();
        assert_eq!(s.plan().board.layer(Phase::Auto).paths[1].points[0], start4);
        assert_ne!(start4, end);
        // In teleop, robot 1's path starts where auto left it.
        press(&mut s, &[Key::Tab, Key::Char('1'), Key::Char('p'), Key::Char('z'), Key::Enter, Key::Char('p')]);
        assert_eq!(s.plan().board.layer(Phase::Teleop).paths[0].points[0], end);
    }

    #[test]
    fn notes_and_zones_are_typed_edited_and_removed() {
        let mut s = Strategy::with_board(Board::practice(None), dir("notes"));
        press(&mut s, &[Key::Char('c'), Key::Char('c'), Key::Char('c'), Key::Char('n')]);
        assert!(s.wants_text());
        let typed: Vec<Key> = "0:05 shoot".chars().map(Key::Char).collect();
        press(&mut s, &typed);
        press(&mut s, &[Key::Enter]);
        let note = s.plan().board.layer(Phase::Auto).notes[0].clone();
        assert_eq!((note.text.as_str(), note.kind, note.at), ("0:05 shoot", NoteKind::Label, s.cursor));
        // Enter on it edits its words; emptied, it goes.
        press(&mut s, &[Key::Enter, Key::Backspace, Key::Backspace, Key::Backspace, Key::Backspace, Key::Backspace, Key::Enter]);
        assert_eq!(s.plan().board.layer(Phase::Auto).notes[0].text, "0:05");
        press(&mut s, &[Key::Enter, Key::Backspace, Key::Backspace, Key::Backspace, Key::Backspace, Key::Enter]);
        assert!(s.plan().board.layer(Phase::Auto).notes.is_empty());
        // A drop zone needs no words; Esc while typing keeps nothing.
        press(&mut s, &[Key::Char('d'), Key::Enter, Key::Char('z'), Key::Char('z'), Key::Char('n'), Key::Char('x'), Key::Back]);
        let kinds: Vec<NoteKind> = s.plan().board.layer(Phase::Auto).notes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoteKind::Zone]);
    }

    #[test]
    fn each_phase_is_its_own_layer_and_play_drives_them_in_turn() {
        let mut s = Strategy::with_board(full(), dir("play"));
        press(&mut s, &[Key::Char('e')]);
        assert_eq!(s.phase(), Phase::Endgame);
        press(&mut s, &[Key::Char('a')]);
        // Space plays auto, then teleop (the endgame has no paths: it stops there).
        let mut said = press(&mut s, &[Key::Char(' ')]);
        assert!(said.is_none() && s.play.is_some());
        let mut phases = vec![Phase::Auto];
        let mut seen_moving = false;
        for _ in 0..60 * 30 {
            if !s.tick(1.0 / 60.0) {
                break;
            }
            if let Some(play) = &s.play {
                seen_moving |= play.leg.value() > 0.4 && play.leg.value() < 0.9;
                if phases.last() != Some(&play.phase) {
                    phases.push(play.phase);
                }
            }
        }
        assert!(seen_moving, "between waypoints");
        assert_eq!(phases, [Phase::Auto, Phase::Teleop]);
        assert!(s.play.is_none() && !s.tick(1.0 / 60.0), "done, and at rest");
        // In the endgame with nothing to drive: said so.
        said = press(&mut s, &[Key::Char('e'), Key::Char(' ')]);
        assert!(said.is_some_and(|w| w.contains("no paths")) && s.play.is_none());
        // A key stops a play.
        press(&mut s, &[Key::Char('a'), Key::Char(' ')]);
        assert!(s.play.is_some());
        press(&mut s, &[Key::Char(' ')]);
        assert!(s.play.is_none());
    }

    #[test]
    fn the_board_draws_from_its_cache_and_the_frame_beat_rests() {
        crate::shell::tests::sandbox();
        let mut shell = crate::fullheight::shell("catalyst", TextSize::Standard);
        shell.show(Box::new(Strategy::with_board(full(), dir("rest"))));
        for _ in 0..330 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        assert!(!shell.moving() && !shell.animating(), "at rest, no frames");
        assert_eq!(shell.ambient(), None);
        // A step of the cursor: frames while it moves, then rest again.
        shell.key(Key::Char('c'));
        assert!(shell.tick(1.0 / 60.0));
        for _ in 0..120 {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
        assert!(!shell.moving() && !shell.animating());
    }

    #[test]
    fn the_cached_picture_is_painted_only_when_the_board_changes() {
        let mut g = Gfx::new(Tokens::dark());
        let robot = Robot::none();
        let mut s = Strategy::with_board(full(), dir("cache"));
        s.draw(&mut g, &robot);
        let first = s.cache.key;
        // The cursor moving: the same picture under it.
        press(&mut s, &[Key::Char('c')]);
        settle(&mut s);
        s.draw(&mut g, &robot);
        assert_eq!(s.cache.key, first);
        // A robot placed: painted again.
        press(&mut s, &[Key::Char('r')]);
        s.draw(&mut g, &robot);
        assert_ne!(s.cache.key, first);
    }

    #[test]
    fn a_picture_of_the_board_shows_all_three_phases() {
        let mut g = Gfx::new(Tokens::dark());
        let pic = picture(&mut g, &full());
        assert_eq!(pic.w, 948);
        assert!(pic.h > 1300 && pic.h < 1700, "{}", pic.h);
        assert_eq!((g.canvas.w, g.tokens.tone, g.ox), (W as i32, Tone::Dark, 0.0), "the panel's own back as it was");
        let file = dir("picture").join("q31-strategy.png");
        write_png(&pic, &file).unwrap();
        assert!(std::fs::metadata(&file).unwrap().len() > 10_000);
        std::fs::remove_dir_all(file.parent().unwrap()).ok();
    }

    #[test]
    fn the_queue_notice_offers_the_board_once_a_plan_is_made() {
        crate::shell::tests::sandbox();
        let n = || crate::notify::Notification::new("Schedule", "flag", "queue for Q77 in 5 min").action("builtin:Schedule");
        let mut board = full();
        board.key = "2026test_q77".into();
        assert_eq!(offer(n(), &board.key, "Q77").action.as_deref(), Some("builtin:Schedule"), "no plan: the schedule, as ever");
        board::save_board(&board::home(), &board).unwrap();
        let offered = offer(n(), &board.key, "Q77");
        assert_eq!(offered.action.as_deref(), Some("builtin:Strategy"));
        assert!(offered.body.contains("Q77"));
        std::fs::remove_file(board::home().join("boards").join("2026test-q77.board")).ok();
    }

    #[test]
    fn a_scanned_board_opens_once_its_codes_are_all_in() {
        let board = full();
        let codes = qr::chunks(&board).unwrap();
        assert!(codes.iter().all(|c| is_board_code(c)));
        assert!(!is_board_code("https://www.thebluealliance.com"));
        assert_eq!(card(&codes[0]).1, format!("Strategy board · code 1 of {}", codes.len()));
    }
}
