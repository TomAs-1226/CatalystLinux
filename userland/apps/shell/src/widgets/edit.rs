//! Editing a board from the keyboard.
//!
//! **The key.** `e` on Home (or on the glance page) starts editing, and so
//! does Settings › Home › Widgets. Enter held was the other candidate, and
//! it conflicts twice over: Enter opens an app as it lands (pressed in for
//! 70 ms, then the window grows out of the tile), so a hold could only be
//! told from a press by making every open wait for the key to come up; and
//! on a widget Enter already acts (play or pause, start a timer, tick a
//! reminder off). `e` does nothing on Home (its letters are f, x, z and c,
//! the arrows, and Space and `/` for search), it is where the edit key
//! already is in Connect, Actions and Remote, and Fn + E stays music's next.
//!
//! While editing, one widget is chosen (ringed, lifted a little):
//! - **← → ↑ ↓** move it: it takes the next place that way where it lands
//!   somewhere new, and everything else glides round it to make room.
//! - **r** gives it the next size round (small, wide, large).
//! - **a** adds one: the picker opens in place, a live widget where the new
//!   one will go; ← → turn through the kinds, ↑ ↓ (or r) change its size,
//!   Enter keeps it, Esc goes back.
//! - **d** twice takes it off; any other key in between keeps it.
//! - **Tab** chooses the next one.
//! - **Enter**, **Esc** or **e** again: done.
//!
//! Every change is saved as it is made. The tiles glide on Material's
//! spatial spring and settle with a little give; entering edit mode they
//! jiggle once, one after another, to say they can be moved. Nothing moves
//! continuously: once a change has settled the board holds still.

use std::collections::HashMap;

use bezel::Rect;
use detent::{role, Animated};

use super::{sequence, Board, Kind, Packed, Placed, Slot, ROWS};

/// A way to move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The widgets in a new order of slots, each widget's place among the apps
/// counted afresh; and where widget `j` went.
fn rebuild(widgets: &[Placed], seq: &[Slot], j: usize) -> (Vec<Placed>, usize) {
    let mut out = Vec::with_capacity(widgets.len());
    let mut apps = 0;
    let mut moved = 0;
    for slot in seq {
        match slot {
            Slot::App(_) => apps += 1,
            Slot::Widget(k) => {
                if *k == j {
                    moved = out.len();
                }
                out.push(Placed { after: apps, ..widgets[*k] });
            }
        }
    }
    (out, moved)
}

/// Widget `j` moved one step `dir` on a board of `apps` apps, `columns`
/// across: the widgets as they then are, and where `j` is among them. It
/// goes to the nearest place in its order that puts it somewhere new that
/// way; up and down, to the row above or below (on to the next page, or
/// back to the one before), as near its column as it can. None: no further.
pub fn step(apps: usize, widgets: &[Placed], j: usize, dir: Dir, columns: usize) -> Option<(Vec<Placed>, usize)> {
    let seq = sequence(apps, widgets);
    let from = seq.iter().position(|s| *s == Slot::Widget(j))?;
    let now = Packed::of(apps, widgets, columns).cell(Slot::Widget(j))?;
    let mut rest = seq.clone();
    rest.remove(from);
    let candidates: Vec<usize> = match dir {
        Dir::Right | Dir::Down => (from + 1..=rest.len()).collect(),
        Dir::Left | Dir::Up => (0..from).rev().collect(),
    };
    // Up and down aim for a row: the one above or below on this page, else
    // the next page's top or the last one's foot.
    let target = match dir {
        Dir::Down if now.row + now.h < ROWS => Some((now.page, now.row + 1)),
        Dir::Down => Some((now.page + 1, 0)),
        Dir::Up if now.row > 0 => Some((now.page, now.row - 1)),
        Dir::Up if now.page > 0 => Some((now.page - 1, ROWS - now.h)),
        Dir::Up => None,
        _ => None,
    };
    if matches!(dir, Dir::Up | Dir::Down) && target.is_none() {
        return None;
    }
    let mut best: Option<(usize, Vec<Placed>, usize)> = None;
    for q in candidates {
        let mut s = rest.clone();
        s.insert(q, Slot::Widget(j));
        let (list, k) = rebuild(widgets, &s, j);
        let Some(cell) = Packed::of(apps, &list, columns).cell(Slot::Widget(k)) else { continue };
        match (dir, target) {
            (Dir::Left | Dir::Right, _) => {
                let later = cell.order() > now.order();
                if cell.order() != now.order() && later == (dir == Dir::Right) {
                    return Some((list, k));
                }
            }
            (_, Some((page, row))) => {
                if cell.page == page && cell.row == row {
                    let off = (cell.col as i32 - now.col as i32).unsigned_abs() as usize;
                    if best.as_ref().is_none_or(|(b, ..)| off < *b) {
                        best = Some((off, list, k));
                    }
                    if off == 0 {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    best.map(|(_, list, k)| (list, k))
}

/// `placed` put into the board before slot `before` (at the end if none):
/// the widgets as they then are, and where it is among them.
pub fn insert(apps: usize, widgets: &[Placed], before: Option<Slot>, placed: Placed) -> (Vec<Placed>, usize) {
    let mut seq = sequence(apps, widgets);
    let at = before.and_then(|b| seq.iter().position(|s| *s == b)).unwrap_or(seq.len());
    let j = widgets.len();
    seq.insert(at, Slot::Widget(j));
    let mut all = widgets.to_vec();
    all.push(placed);
    rebuild(&all, &seq, j)
}

// ---------------------------------------------------------------- state ---

/// A board being edited.
pub struct Editing {
    pub board: Board,
    /// The board as edited: what is saved, and the picker's widget while
    /// one is being chosen.
    pub list: Vec<Placed>,
    /// The widget chosen, by its place in `list`.
    pub selected: Option<usize>,
    /// Choosing a widget to add: it is `list[selected]`.
    pub picker: Option<Picker>,
    /// `d` pressed once: again takes it off.
    pub armed: bool,
}

/// The picker: which kind is shown, and the slide between kinds.
pub struct Picker {
    pub at: usize,
    /// The kind turned from, which way (+1 on to the next), and the slide:
    /// where the new kind is, in tile widths, springing to 0 (the old one
    /// a width behind it).
    pub from: Kind,
    pub dir: f32,
    pub slide: Animated,
}

impl Picker {
    pub fn new(at: usize) -> Picker {
        Picker { at, from: Kind::ALL[at], dir: 1.0, slide: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.002) }
    }

    pub fn kind(&self) -> Kind {
        Kind::ALL[self.at]
    }

    /// Turn to the next kind (`by` −1 the one before), sliding. Turned again
    /// mid-slide, it carries on from where the content is, at its speed.
    pub fn turn(&mut self, by: i32, calm: bool) {
        self.from = self.kind();
        let n = Kind::ALL.len() as i32;
        self.at = (self.at as i32 + by).rem_euclid(n) as usize;
        self.dir = by.signum() as f32;
        if calm {
            self.slide.jump(0.0);
        } else {
            self.slide.shift(self.dir);
            self.slide.set(0.0);
        }
    }
}

impl Editing {
    pub fn new(board: Board, list: Vec<Placed>, selected: Option<usize>) -> Editing {
        Editing { board, list, selected, picker: None, armed: false }
    }

    /// The board without the picker's widget: what is kept.
    pub fn kept(&self) -> Vec<Placed> {
        match (&self.picker, self.selected) {
            (Some(_), Some(j)) => self.list.iter().enumerate().filter(|(i, _)| *i != j).map(|(_, w)| *w).collect(),
            _ => self.list.clone(),
        }
    }
}

// ---------------------------------------------------------------- motion ---

/// One thing's motion on a board: where it is (its rectangle, along the
/// strip of pages), how lifted, and its jiggle.
pub struct Motion {
    pub x: Animated,
    pub y: Animated,
    pub w: Animated,
    pub h: Animated,
    pub lift: Animated,
    pub jiggle: Animated,
    placed: bool,
}

impl Motion {
    fn new() -> Motion {
        let place = || Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.05);
        Motion {
            x: place(),
            y: place(),
            w: place(),
            h: place(),
            lift: Animated::new(0.0, role::detent()).with_rest(0.01),
            jiggle: Animated::new(0.0, role::wobble()).with_rest(0.02),
            placed: false,
        }
    }

    fn springs(&mut self) -> [&mut Animated; 6] {
        [&mut self.x, &mut self.y, &mut self.w, &mut self.h, &mut self.lift, &mut self.jiggle]
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x.value() + self.jiggle.value(), self.y.value(), self.w.value(), self.h.value())
    }
}

/// A widget taken off, closing where it was.
pub struct Leaving {
    /// Where it was, along the strip of pages.
    pub rect: Rect,
    pub board: Board,
    /// 1 as it goes, to 0 gone.
    pub left: Animated,
}

/// Every board's things and their motion, by a key of their own: an app's
/// place, or a widget's id.
#[derive(Default)]
pub struct Glides {
    motion: HashMap<u64, Motion>,
    pub leaving: Vec<Leaving>,
}

/// The key of an app's tile, and of a widget, for [`Glides`].
pub fn app_key(i: usize) -> u64 {
    i as u64
}

pub fn widget_key(id: u32) -> u64 {
    (1u64 << 40) | id as u64
}

impl Glides {
    /// Where `key` is drawn now, heading for `target` (along the strip of
    /// pages): gliding there when `animate`, else there at once.
    pub fn place(&mut self, key: u64, target: Rect, animate: bool) -> Rect {
        let m = self.motion.entry(key).or_insert_with(Motion::new);
        let want = [target.x, target.y, target.w, target.h];
        let springs = [&mut m.x, &mut m.y, &mut m.w, &mut m.h];
        if !m.placed || !animate {
            for (s, to) in springs.into_iter().zip(want) {
                s.jump(to);
            }
            m.placed = true;
        } else {
            for (s, to) in springs.into_iter().zip(want) {
                s.set(to);
            }
        }
        m.rect()
    }

    /// `key` appears: it grows into `target` from its middle.
    pub fn grow_from(&mut self, key: u64, target: Rect) {
        let m = self.motion.entry(key).or_insert_with(Motion::new);
        let small = Rect::new(target.cx() - target.w * 0.3, target.cy() - target.h * 0.3, target.w * 0.6, target.h * 0.6);
        for (s, to) in [&mut m.x, &mut m.y, &mut m.w, &mut m.h].into_iter().zip([small.x, small.y, small.w, small.h]) {
            s.jump(to);
        }
        m.placed = true;
    }

    pub fn get(&mut self, key: u64) -> &mut Motion {
        self.motion.entry(key).or_insert_with(Motion::new)
    }

    /// Lift `key` toward `to` (1 chosen, 0 at rest).
    pub fn lift(&mut self, key: u64, to: f32, animate: bool) -> f32 {
        let m = self.get(key);
        if animate {
            m.lift.set(to);
        } else {
            m.lift.jump(to);
        }
        m.lift.value()
    }

    /// The jiggle that says "these can be moved": each in turn, a little
    /// after the one before.
    pub fn jiggle(&mut self, keys: &[u64]) {
        for (i, key) in keys.iter().enumerate() {
            let m = self.get(*key);
            m.jiggle.jump(0.0);
            // Alternating, and less for each one after: a ripple, not a shake.
            let push = if i % 2 == 0 { 40.0 } else { -40.0 } * (1.0 - 0.06 * i.min(8) as f32);
            m.jiggle.nudge(push);
        }
    }

    /// Something glides just now.
    pub fn moving(&self) -> bool {
        self.motion.values().any(|m| !(m.x.is_settled() && m.y.is_settled() && m.w.is_settled() && m.h.is_settled() && m.jiggle.is_settled())) || !self.leaving.is_empty()
    }

    /// Everything placed afresh next time (the columns changed).
    pub fn forget(&mut self) {
        self.motion.clear();
        self.leaving.clear();
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for m in self.motion.values_mut() {
            for s in m.springs() {
                moving |= s.tick(dt);
            }
        }
        for l in &mut self.leaving {
            moving |= l.left.tick(dt);
        }
        self.leaving.retain(|l| l.left.value() > 0.02 || !l.left.is_settled());
        moving
    }
}

/// The picker's next kind: the first not on the board yet, from `from`.
pub fn first_new(list: &[Placed], from: usize) -> usize {
    (0..Kind::ALL.len()).map(|i| (from + i) % Kind::ALL.len()).find(|i| !list.iter().any(|w| w.kind == Kind::ALL[*i])).unwrap_or(from)
}
