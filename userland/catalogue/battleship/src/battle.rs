//! One game, on the whole panel: placing your fleet, the battle, and how it
//! ended.
//!
//! - Placing: your waters fill the left, the fleet's list the right. The
//!   ship in hand glides under the arrows; R turns it, Enter puts it down,
//!   Backspace takes the last one back, A puts the rest down at random.
//! - The battle: your waters fly into the small grid at the right and the
//!   handheld's take the left, where you aim. After each of your shots a
//!   short beat, then its shot lands in your waters, ringed.
//! - The end: their ships still hidden are outlined, and the panel turns
//!   into the result. Enter plays again; Esc goes back to the levels.
//!
//! The original's two-player game over Bluetooth or Wi-Fi is left out: the
//! handheld here plays you itself.

use catalyst_kit::prelude::*;
use catalyst_kit::{TextStyle, Theme};

use crate::ai::{self, Level};
use crate::game::{Board, Kind, Ship, Shot, FLEET, N};
use crate::record::Record;
use crate::sea::{self, capsule, Grid, Splash, Waters, SMALL};

/// The beat between your shot and the handheld's, so its shot is seen.
const BEAT: f32 = 0.55;
/// How long the last shot of a game shows before the result comes in.
const FINALE: f32 = 0.9;
/// How long Esc stays armed to leave a game under way.
const LEAVE_ARMED: f32 = 2.5;
/// How long a note stays ("on the carrier", "fired there already").
const NOTE: f32 = 1.8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Placing,
    Playing,
    Over { won: bool },
}

const SANK_THE: [&str; 5] = ["sank the carrier", "sank the battleship", "sank the cruiser", "sank the submarine", "sank the destroyer"];
const HIT_YOUR: [&str; 5] = ["hit your carrier", "hit your battleship", "hit your cruiser", "hit your submarine", "hit your destroyer"];
const SANK_YOUR: [&str; 5] = ["sank your carrier", "sank your battleship", "sank your cruiser", "sank your submarine", "sank your destroyer"];
const ON_THE: [&str; 5] = ["it's on the carrier", "it's on the battleship", "it's on the cruiser", "it's on the submarine", "it's on the destroyer"];
const TITLES: [&str; 5] = ["Carrier", "Battleship", "Cruiser", "Submarine", "Destroyer"];

/// Your shot, in words: longest first.
fn your_words(shot: Shot) -> [&'static str; 2] {
    match shot {
        Shot::Miss => ["miss", "miss"],
        Shot::Hit(_) => ["hit", "hit"],
        Shot::Sunk(kind) => [SANK_THE[kind.index()], "sank one"],
    }
}

/// Its shot, in words: longest first.
fn their_words(shot: Shot) -> [&'static str; 2] {
    match shot {
        Shot::Miss => ["missed", "missed"],
        Shot::Hit(kind) => [HIT_YOUR[kind.index()], "hit one"],
        Shot::Sunk(kind) => [SANK_YOUR[kind.index()], "sank one"],
    }
}

/// Head for `to`, or land there at once in the calm version.
fn go(a: &mut Animated, to: f32, calm: bool) {
    if calm {
        a.jump(to);
    } else {
        a.set(to);
    }
}

/// Where everything sits: the large grid at the left, the panel beside it,
/// the small grid at the panel's top. Measured from the hint line.
struct Layout {
    big: Grid,
    small: Grid,
    panel: Rect,
}

fn layout(ui: &Ui) -> Layout {
    let room = H - ui.hint_h() - 2.0;
    let side = Grid::extent(sea::BIG);
    let y = ((room - side) * 0.5).floor().max(2.0);
    let px = PAD + side + 8.0;
    Layout { big: Grid::big(PAD, y, ui), small: Grid::small(px, y, ui), panel: Rect::new(px, y, W - PAD - px, side) }
}

/// The aiming ring stretches along its motion, as the kit's focus ring does
/// (its numbers).
fn stretched(r: Rect, vx: f32, vy: f32) -> Rect {
    const STRETCH: f32 = 0.04;
    const MOST: f32 = 30.0 * W / 720.0;
    let along = |v: f32, size: f32| (v.abs() * STRETCH).min(MOST).min(size / 3.0);
    let (sx, sy) = (along(vx, r.w), along(vy, r.h));
    let (thin_w, thin_h) = ((sy * 0.5).min(r.w * 0.08), (sx * 0.5).min(r.h * 0.08));
    let x = if vx > 0.0 { r.x - sx } else { r.x } + thin_w * 0.5;
    let y = if vy > 0.0 { r.y - sy } else { r.y } + thin_h * 0.5;
    Rect::new(x, y, r.w + sx - thin_w, r.h + sy - thin_h)
}

/// Text at an opacity, placed by its left, centre or right.
#[allow(clippy::too_many_arguments)]
fn words_at(ui: &mut Ui, x: f32, cy: f32, align: Align, s: &str, style: TextStyle, color: Color, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let w = ui.measure(s, style);
    let left = match align {
        Align::Left => x,
        Align::Center => x - w * 0.5,
        Align::Right => x - w,
    };
    ui.text_alpha(left, cy, s, style, color, alpha);
    w
}

/// The first of `choices` (longest first) that fits `width`.
fn fitting<'a>(ui: &mut Ui, choices: &[&'a str], style: TextStyle, width: f32) -> &'a str {
    choices.iter().copied().find(|s| ui.measure(s, style) <= width).or(choices.last().copied()).unwrap_or("")
}

pub struct Battle {
    level: Level,
    rng: Rng,
    mine: Board,
    theirs: Board,
    phase: Phase,
    /// The ship being placed.
    hand: Ship,
    /// Seconds since each of your ships went down into place.
    placed_age: [f32; 5],
    /// What the ship in hand was refused for, and for how long it is said.
    refused: Option<Kind>,
    note: f32,
    aim: (usize, usize),
    /// Seconds until the handheld fires, while it is its turn.
    their_turn: Option<f32>,
    yours: Option<Shot>,
    theirs_last: Option<Shot>,
    /// Your last shot was at a square fired at already: said for a moment.
    again: f32,
    /// The square the handheld fired at last, ringed in your waters.
    incoming: Option<(usize, usize)>,
    /// Your shot landing in its waters, and its in yours.
    splash_theirs: Option<Splash>,
    splash_mine: Option<Splash>,
    /// Esc was pressed once in a game under way: again to leave.
    leave: f32,
    finale: f32,
    /// The result's words, made once as the game ends: the record (long and
    /// short), the shots and the accuracy.
    result: [String; 4],
    /// The ship in hand: x, y, across and down, in squares.
    ship: [Animated; 4],
    aim_x: Animated,
    aim_y: Animated,
    /// 0 placing, 1 in battle: your waters fly to the small grid.
    morph: Animated,
    /// 0 in battle, 1 the result shown.
    ending: Animated,
    /// A new game rising in.
    arrive: Animated,
    /// 1 while it is the handheld's turn.
    turn: Animated,
    calm: bool,
}

impl Battle {
    pub fn new(level: Level, calm: bool) -> Battle {
        let mut rng = Rng::seeded();
        let theirs = Board::random(&mut rng);
        let hand = Ship::new(FLEET[0], 0, 0, true);
        let (w, h) = hand.size();
        let at = |v: f32, spring| Animated::new(v, spring).with_rest(0.004);
        let unit = |v: f32, spring| Animated::new(v, spring).with_rest(0.002);
        Battle {
            level,
            rng,
            mine: Board::new(),
            theirs,
            phase: Phase::Placing,
            hand,
            placed_age: [1.0; 5],
            refused: None,
            note: 0.0,
            aim: (4, 4),
            their_turn: None,
            yours: None,
            theirs_last: None,
            again: 0.0,
            incoming: None,
            splash_theirs: None,
            splash_mine: None,
            leave: 0.0,
            finale: 0.0,
            result: Default::default(),
            ship: [at(hand.x as f32, spatial::fast()), at(hand.y as f32, spatial::fast()), at(w as f32, spatial::default()), at(h as f32, spatial::default())],
            aim_x: at(4.0, spatial::fast()),
            aim_y: at(4.0, spatial::fast()),
            morph: unit(0.0, spatial::default()),
            ending: unit(0.0, spatial::default()),
            arrive: unit(1.0, spatial::default()),
            turn: unit(0.0, role::effect()),
            calm,
        }
    }

    // ---- placing ----

    fn follow_hand(&mut self) {
        let (w, h) = self.hand.size();
        let calm = self.calm;
        for (spring, to) in self.ship.iter_mut().zip([self.hand.x, self.hand.y, w, h]) {
            go(spring, to as f32, calm);
        }
    }

    fn move_hand(&mut self, dx: isize, dy: isize) {
        let (w, h) = self.hand.size();
        let x = (self.hand.x as isize + dx).clamp(0, (N - w) as isize) as usize;
        let y = (self.hand.y as isize + dy).clamp(0, (N - h) as isize) as usize;
        if (x, y) == (self.hand.x, self.hand.y) {
            // At the edge: it gives a little.
            if !self.calm {
                self.ship[0].nudge(dx as f32 * 6.0);
                self.ship[1].nudge(dy as f32 * 6.0);
            }
            return;
        }
        self.hand.x = x;
        self.hand.y = y;
        self.follow_hand();
    }

    fn rotate(&mut self) {
        self.hand = Ship { across: !self.hand.across, ..self.hand }.kept_on_board();
        self.follow_hand();
    }

    fn put_down(&mut self) {
        if self.mine.place(self.hand) {
            let i = self.mine.ships().len() - 1;
            self.placed_age[i] = if self.calm { 1.0 } else { 0.0 };
            self.refused = None;
            if let Some(next) = self.mine.nearest_open(self.hand) {
                self.hand = next;
                self.follow_hand();
            }
        } else {
            self.refused = self.mine.clash(self.hand).map(|s| s.kind);
            self.note = NOTE;
            if !self.calm {
                // A shake: no.
                self.ship[0].nudge(if self.hand.across { 14.0 } else { 9.0 });
            }
        }
    }

    fn take_back(&mut self) {
        if let Some(ship) = self.mine.unplace() {
            self.placed_age[self.mine.ships().len()] = 1.0;
            self.hand = ship;
            self.refused = None;
            self.follow_hand();
        }
    }

    fn rest_at_random(&mut self) {
        let before = self.mine.ships().len();
        self.mine.place_rest_at_random(&mut self.rng);
        for i in before..FLEET.len() {
            // One after another, as if dealt.
            self.placed_age[i] = if self.calm { 1.0 } else { -0.07 * (i - before) as f32 };
        }
        self.refused = None;
    }

    fn start_battle(&mut self) {
        self.phase = Phase::Playing;
        go(&mut self.morph, 1.0, self.calm);
        self.aim = (4, 4);
        self.aim_x.jump(4.0);
        self.aim_y.jump(4.0);
    }

    // ---- the battle ----

    fn move_aim(&mut self, dx: isize, dy: isize) {
        let x = (self.aim.0 as isize + dx).clamp(0, N as isize - 1) as usize;
        let y = (self.aim.1 as isize + dy).clamp(0, N as isize - 1) as usize;
        if (x, y) == self.aim {
            if !self.calm {
                self.aim_x.nudge(dx as f32 * 5.0);
                self.aim_y.nudge(dy as f32 * 5.0);
            }
            return;
        }
        self.aim = (x, y);
        go(&mut self.aim_x, x as f32, self.calm);
        go(&mut self.aim_y, y as f32, self.calm);
    }

    fn fire(&mut self) {
        if self.their_turn.is_some() {
            return;
        }
        let (x, y) = self.aim;
        let Some(shot) = self.theirs.fire(x, y) else {
            self.again = NOTE;
            return;
        };
        self.again = 0.0;
        self.yours = Some(shot);
        self.splash_theirs = (!self.calm).then(|| Splash::new(x, y, shot));
        if self.theirs.all_sunk() {
            self.finish(true);
        } else {
            self.their_turn = Some(BEAT);
            go(&mut self.turn, 1.0, self.calm);
        }
    }

    /// The handheld's shot.
    fn reply(&mut self) {
        self.their_turn = None;
        go(&mut self.turn, 0.0, self.calm);
        let Some((x, y)) = ai::choose(self.level, &self.mine.sight(), &mut self.rng) else { return };
        let Some(shot) = self.mine.fire(x, y) else { return };
        self.theirs_last = Some(shot);
        self.incoming = Some((x, y));
        self.splash_mine = (!self.calm).then(|| Splash::new(x, y, shot));
        if self.mine.all_sunk() {
            self.finish(false);
        }
    }

    fn finish(&mut self, won: bool) {
        self.phase = Phase::Over { won };
        self.finale = FINALE;
        self.leave = 0.0;
        let mut record = Record::open();
        // Kept if it can be; the result is shown either way.
        let _ = record.add(self.level, won);
        let (w, l) = record.of(self.level);
        let shots = self.theirs.shots();
        let accuracy = if shots == 0 { 0 } else { (self.theirs.hits() * 100 + shots / 2) / shots };
        self.result = [format!("{}: won {w}, lost {l}", self.level.key()), format!("won {w} · lost {l}"), shots.to_string(), format!("{accuracy}%")];
    }

    fn show_result(&mut self) {
        self.finale = 0.0;
        go(&mut self.ending, 1.0, self.calm);
    }

    fn new_game(&mut self) {
        let calm = self.calm;
        *self = Battle::new(self.level, calm);
        if !calm {
            self.arrive.jump(0.0);
            self.arrive.set(1.0);
        }
    }

    fn over(&self) -> Option<bool> {
        match self.phase {
            Phase::Over { won } => Some(won),
            _ => None,
        }
    }

    // ---- drawing ----

    fn draw_hand(&self, ui: &mut Ui, g: &Grid, alpha: f32) {
        let [x, y, w, h] = self.ship.map(|a| a.value());
        let r = g.span(x, y, w, h).offset(ui.ox, 0.0);
        let color = if self.mine.clash(self.hand).is_none() { ui.ring_color() } else { ui.fault() };
        let radius = capsule(r);
        // A solid ghost: the water's squares do not show through it.
        let under = ui.tokens.surface1;
        ui.canvas.rrect(r, radius, under, alpha);
        ui.canvas.rrect(r, radius, color, 0.3 * alpha);
        ui.canvas.stroke_rrect(r.inset(-1.0), radius + 1.0, 1.5, color, alpha);
    }

    fn draw_aim(&self, ui: &mut Ui, g: &Grid, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        // Clear of the square by a gap, so it reads around a hit as well.
        let r = g.span(self.aim_x.value(), self.aim_y.value(), 1.0, 1.0).inset(-3.0);
        let r = if self.calm { r } else { stretched(r, self.aim_x.velocity() * g.pitch, self.aim_y.velocity() * g.pitch) };
        // Dimmed while the handheld takes its turn.
        let a = alpha * (1.0 - 0.6 * self.turn.value().clamp(0.0, 1.0));
        let color = ui.ring_color();
        ui.canvas.stroke_rrect(r.offset(ui.ox, 0.0), g.radius + 3.0, 1.5, color, a);
    }

    /// While placing: the fleet, one ship a row, the one in hand lifted.
    fn draw_fleet(&self, ui: &mut Ui, panel: Rect, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let (t, ty) = (ui.tokens, ui.ty);
        let bezel = ui.look.theme == Theme::Bezel;
        let placed = self.mine.ships().len();
        let head_cy = panel.y + ty.name.size * 0.5 + 2.0;
        let (head, ink) = match (self.refused, self.note > 0.0) {
            (Some(kind), true) => (fitting(ui, &[ON_THE[kind.index()], "it's on a ship", "taken"], ty.name, panel.w - 4.0), ui.fault()),
            _ if self.mine.complete() => ("fleet ready", t.ink),
            _ => (fitting(ui, &["place your fleet", "your fleet"], ty.name, panel.w - 4.0), t.ink),
        };
        words_at(ui, panel.x + 2.0, head_cy, Align::Left, head, ty.name, ink, alpha);
        let name_style = if bezel { ty.label } else { ty.body };
        let pitch = (name_style.size + 10.0).min((panel.bottom() - head_cy - ty.name.size * 0.5 - 4.0) / FLEET.len() as f32);
        let top = panel.bottom() - pitch * FLEET.len() as f32;
        let afloat = sea::afloat_color(ui);
        for (i, kind) in FLEET.iter().enumerate() {
            let row = Rect::new(panel.x, top + pitch * i as f32 + 1.0, panel.w, pitch - 2.0);
            let here = i == placed;
            if here {
                ui.canvas.rrect(row.offset(ui.ox, 0.0), ui.radius(7.0).min(row.h * 0.5), t.surface1, alpha);
            }
            let pip_w = kind.len() as f32 * SMALL - 1.0;
            let pip = Rect::new(row.right() - 8.0 - pip_w, row.cy() - (SMALL - 1.0) * 0.5, pip_w, SMALL - 1.0).offset(ui.ox, 0.0);
            let name = if bezel { kind.name() } else { TITLES[i] };
            words_at(ui, row.x + 8.0, row.cy(), Align::Left, name, name_style, if here { t.ink } else { t.dim }, alpha);
            if i < placed {
                ui.canvas.rrect(pip, capsule(pip), afloat, alpha);
            } else if here {
                let ring = ui.ring_color();
                ui.canvas.rrect(pip, capsule(pip), ring, 0.3 * alpha);
                ui.canvas.stroke_rrect(pip.inset(-0.5), capsule(pip) + 0.5, 1.0, ring, alpha);
            } else {
                // Still to come: outlined, as their ships still afloat are.
                ui.canvas.stroke_rrect(pip, capsule(pip), 1.0, t.dim, alpha);
            }
        }
    }

    /// In battle: their fleet beside your small grid, whose turn it is, and
    /// the last two shots.
    fn draw_status(&self, ui: &mut Ui, panel: Rect, small: &Grid, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let (t, ty) = (ui.tokens, ui.ty);
        // Your waters glow at their edge while the handheld aims at them.
        let turn = self.turn.value().clamp(0.0, 1.0);
        if turn > 0.0 {
            let ring = ui.ring_color();
            let tile = small.tile().offset(ui.ox, 0.0);
            ui.canvas.stroke_rrect(tile.inset(-1.5), small.tile_radius() + 1.5, 1.5, ring, alpha * turn);
        }
        // Their fleet: each ship drawn to the small grid's scale, lit when sunk.
        let x = small.tile().right() + 10.0;
        let sunk = sea::sunk_color(&t);
        for (i, ship) in self.theirs.ships().iter().enumerate() {
            let w = ship.kind.len() as f32 * SMALL - 1.0;
            let r = Rect::new(x, small.y + (2 * i) as f32 * SMALL + 0.5 * SMALL, w, SMALL - 1.0).offset(ui.ox, 0.0);
            if self.theirs.is_sunk(*ship) {
                ui.canvas.rrect(r, capsule(r), sunk, alpha);
            } else {
                ui.canvas.stroke_rrect(r, capsule(r), 1.0, t.dim, alpha);
            }
        }
        // Whose turn, and the last shots.
        let top = small.tile().bottom() + 6.0;
        let head_cy = top + ty.name.size * 0.5 + 2.0;
        let head = match self.over() {
            Some(true) => "you won",
            Some(false) => "you lost",
            None if self.their_turn.is_some() => "its turn",
            None => "your turn",
        };
        words_at(ui, panel.x + 2.0, head_cy, Align::Left, head, ty.name, t.ink, alpha);
        let style = ty.body;
        let pitch = (style.size * 1.45).round();
        let icon = (style.size + 1.0).round();
        let text_x = panel.x + 2.0 + icon + 5.0;
        let room = panel.right() - text_x;
        let lines: [(&str, Option<[&str; 2]>, Color); 2] = [
            ("person", if self.again > 0.0 { Some(["fired there already", "fired there"]) } else { self.yours.map(your_words) }, if self.again > 0.0 { t.dim } else { t.ink }),
            ("smart_toy", self.theirs_last.map(their_words), t.dim),
        ];
        let mut cy = head_cy + ty.name.size * 0.5 + 4.0 + pitch * 0.5;
        for (name, said, ink) in lines {
            let Some(said) = said else { continue };
            ui.icon_alpha(name, false, panel.x + 2.0 + icon * 0.5, cy, icon, t.dim, alpha);
            let s = fitting(ui, &said, style, room);
            words_at(ui, text_x, cy, Align::Left, s, style, ink, alpha);
            cy += pitch;
        }
    }

    /// How it ended: who won, the shots it took, and the record.
    fn draw_result(&self, ui: &mut Ui, panel: Rect, won: bool, e: f32) {
        let alpha = e.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let (t, ty) = (ui.tokens, ui.ty);
        let dy = (1.0 - e) * 12.0;
        let panel = panel.offset(0.0, dy);
        let display = ty.display(24.0);
        let head_cy = panel.y + display.size * 0.5 + 2.0;
        let icon = (display.size * 0.9).round();
        let (name, head, sub) = if won { ("emoji_events", "you won", ["every ship found", "all found"]) } else { ("smart_toy", "you lost", ["it found your fleet", "fleet lost"]) };
        ui.icon_alpha(name, true, panel.x + 2.0 + icon * 0.5, head_cy, icon, if won { t.signal } else { t.dim }, alpha);
        words_at(ui, panel.x + 2.0 + icon + 6.0, head_cy, Align::Left, head, display, t.ink, alpha);
        let sub_cy = head_cy + display.size * 0.5 + 4.0 + ty.caption.size * 0.5;
        let sub = fitting(ui, &sub, ty.caption, panel.w - 4.0);
        words_at(ui, panel.x + 2.0, sub_cy, Align::Left, sub, ty.caption, t.dim, alpha);
        // The record at the bottom, the two figures between.
        let record_cy = panel.bottom() - ty.caption.size * 0.5 - 1.0;
        let [long, short, shots, accuracy] = &self.result;
        let record = fitting(ui, &[long.as_str(), short.as_str()], ty.caption, panel.w - 4.0);
        words_at(ui, panel.x + 2.0, record_cy, Align::Left, record, ty.caption, t.dim, alpha);
        let figures_top = sub_cy + ty.caption.size * 0.5 + 6.0;
        let figures_bottom = record_cy - ty.caption.size * 0.5 - 6.0;
        let gap = 4.0;
        let h = ((figures_bottom - figures_top - gap) * 0.5).min(30.0);
        let figure = ty.display(18.0);
        let radius = ui.radius(9.0).min(h * 0.5);
        for (i, (label, value)) in [("shots", shots), ("accuracy", accuracy)].into_iter().enumerate() {
            let r = Rect::new(panel.x, figures_top + (h + gap) * i as f32, panel.w, h);
            ui.canvas.rrect(r.offset(ui.ox, 0.0), radius, t.surface1, alpha);
            words_at(ui, r.x + 10.0, r.cy(), Align::Left, label, ty.caption, t.dim, alpha);
            words_at(ui, r.right() - 10.0, r.cy(), Align::Right, value, figure, t.ink, alpha);
        }
    }

    fn draw_hint(&self, ui: &mut Ui) {
        let placed = self.mine.ships().len();
        let choices: &[&str] = match self.phase {
            Phase::Placing if self.mine.complete() => &["enter: start the battle · bksp: undo · esc: back", "enter: start the battle · bksp: undo", "enter: start · bksp: undo", "enter: start"],
            Phase::Placing if placed == 0 => &["enter: place · r: rotate · a: at random · esc: back", "enter: place · r: rotate · a: at random", "enter: place · r: rotate · a: random", "enter · r: rotate · a: random"],
            Phase::Placing => &["enter: place · r: rotate · a: random · bksp: undo", "enter · r: rotate · a: random · bksp: undo", "enter · r: turn · a: random · bksp: undo", "enter · r: rotate · a: random"],
            Phase::Playing if self.leave > 0.0 => &["esc again: leave this game · enter: stay", "esc again: leave · enter: stay", "esc again: leave"],
            Phase::Playing => &["enter: fire · arrows: aim · esc: leave", "enter: fire · esc: leave", "enter: fire"],
            Phase::Over { .. } if self.finale > 0.0 => &["enter: the result · esc: levels", "enter · esc: levels"],
            Phase::Over { .. } => &["enter: play again · esc: levels", "enter: again · esc: levels", "enter · esc"],
        };
        ui.hint_any(choices);
    }
}

impl Page for Battle {
    fn title(&self) -> &str {
        "Battleship"
    }

    fn full_screen(&self) -> bool {
        true
    }

    /// While placing, the letters (R, A) and Backspace (undo) are the page's own.
    fn wants_text(&self) -> bool {
        self.phase == Phase::Placing
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        self.calm = cx.calm();
        let key = if key == Key::Backspace { key } else { key.plain() };
        match self.phase {
            Phase::Placing => match key {
                Key::Back => return Flow::Pop,
                Key::Backspace if self.mine.ships().is_empty() => return Flow::Pop,
                Key::Backspace => self.take_back(),
                Key::Enter if self.mine.complete() => self.start_battle(),
                _ if self.mine.complete() => {}
                Key::Enter => self.put_down(),
                Key::Up => self.move_hand(0, -1),
                Key::Down => self.move_hand(0, 1),
                Key::Left => self.move_hand(-1, 0),
                Key::Right => self.move_hand(1, 0),
                Key::Char('r' | 'R') => self.rotate(),
                Key::Char('a' | 'A') => self.rest_at_random(),
                _ => {}
            },
            Phase::Playing => {
                if matches!(key, Key::Back | Key::Backspace) {
                    if self.leave > 0.0 {
                        return Flow::Pop;
                    }
                    self.leave = LEAVE_ARMED;
                    return Flow::Stay;
                }
                let armed = std::mem::take(&mut self.leave) > 0.0;
                match key {
                    Key::Enter if !armed => self.fire(),
                    Key::Up => self.move_aim(0, -1),
                    Key::Down => self.move_aim(0, 1),
                    Key::Left => self.move_aim(-1, 0),
                    Key::Right => self.move_aim(1, 0),
                    _ => {}
                }
            }
            Phase::Over { .. } => match key {
                Key::Back | Key::Backspace => return Flow::Pop,
                Key::Enter if self.finale > 0.0 => self.show_result(),
                Key::Enter => self.new_game(),
                _ => {}
            },
        }
        Flow::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for spring in self.ship.iter_mut().chain([&mut self.aim_x, &mut self.aim_y, &mut self.morph, &mut self.ending, &mut self.arrive, &mut self.turn]) {
            moving |= spring.tick(dt);
        }
        for age in &mut self.placed_age {
            if *age < 1.0 {
                *age += dt;
                moving = true;
            }
        }
        for splash in [&mut self.splash_theirs, &mut self.splash_mine] {
            if let Some(s) = splash.as_mut() {
                s.age += dt;
                moving = true;
                if s.age >= sea::SPLASH {
                    *splash = None;
                }
            }
        }
        for timer in [&mut self.note, &mut self.again, &mut self.leave] {
            if *timer > 0.0 {
                *timer = (*timer - dt).max(0.0);
                moving = true;
            }
        }
        if let Some(left) = self.their_turn {
            moving = true;
            if left - dt <= 0.0 {
                self.reply();
            } else {
                self.their_turn = Some(left - dt);
            }
        }
        if self.finale > 0.0 {
            moving = true;
            self.finale -= dt;
            if self.finale <= 0.0 {
                self.show_result();
            }
        }
        moving
    }

    fn draw(&mut self, ui: &mut Ui) {
        self.calm = ui.calm();
        let lay = layout(ui);
        let (m, e) = (self.morph.value(), self.ending.value().clamp(0.0, 1.0));
        let arrive = self.arrive.value();
        let shown = arrive.clamp(0.0, 1.0);
        let rise = (1.0 - arrive) * 10.0;
        let big = lay.big.moved(0.0, rise);
        let small = lay.small.moved(0.0, rise);
        let panel = lay.panel.offset(0.0, rise);
        // Their waters come in as yours move aside.
        let theirs_alpha = m.clamp(0.0, 1.0) * shown;
        if theirs_alpha > 0.0 {
            let waters = Waters { board: &self.theirs, own: false, alpha: theirs_alpha, reveal: e, splash: self.splash_theirs.as_ref(), placed: None, last: None };
            sea::draw(ui, &big, &waters);
            self.draw_aim(ui, &big, theirs_alpha * (1.0 - e));
        }
        // The panel: the fleet's list gives way to the battle's state, which
        // comes in as your waters land beside it.
        self.draw_fleet(ui, panel, shown * (1.0 - 2.0 * m).clamp(0.0, 1.0));
        // As a game ends, the battle's panel goes first and the result rises
        // in after it, never the two over each other.
        let (going, coming) = ((1.0 - 2.0 * e).clamp(0.0, 1.0), ((e - 0.45) / 0.55).clamp(0.0, 1.0));
        self.draw_status(ui, panel, &small, shown * ((m - 0.6) / 0.4).clamp(0.0, 1.0) * going);
        // Your waters: the large grid while placing, the small one in battle,
        // flying over the panel between the two.
        let yours = big.lerp(&small, m);
        let waters = Waters { board: &self.mine, own: true, alpha: shown * going, reveal: 0.0, splash: self.splash_mine.as_ref(), placed: Some(&self.placed_age), last: self.incoming };
        sea::draw(ui, &yours, &waters);
        if self.phase == Phase::Placing && !self.mine.complete() {
            self.draw_hand(ui, &yours, shown);
        }
        if let Some(won) = self.over() {
            self.draw_result(ui, panel, won, coming);
        }
        self.draw_hint(ui);
    }
}
