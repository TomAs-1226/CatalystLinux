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
//!   somewhere new, and everything else glides round it to make room (up
//!   or down past a wide one, the wide one changes rows with it).
//! - **r** gives it the next size round (small, wide, large).
//! - **a** adds one: the picker opens in place, a live widget where the new
//!   one will go; ← → turn through the kinds not on the board yet, ↑ ↓ (or
//!   r) change its size, Enter keeps it, Esc goes back.
//! - **d** twice takes it off; any other key in between keeps it.
//! - **Tab** chooses the next one.
//! - **Enter**, **Esc** or **e** again: done.
//!
//! Every change is saved as it is made, on a thread of its own (the card's
//! write never holds a move up). The tiles glide on Material's spatial
//! spring and settle with a little give; entering edit mode they jiggle
//! once, one after another, to say they can be moved. The one chosen is
//! drawn over the rest, its ring part of it. Nothing moves continuously:
//! once a change has settled the board holds still.

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
/// across: the widgets as they then are, and where `j` is among them. Left
/// and right, it goes to the nearest place in its order that puts it
/// somewhere new that way. Up and down, to the row above or below (on to the
/// next page, or back to the one before), as near its column as it can; if
/// it cannot get there alone (a wide widget fills that row), the one in the
/// way changes places with it. None: no further.
pub fn step(apps: usize, widgets: &[Placed], j: usize, dir: Dir, columns: usize) -> Option<(Vec<Placed>, usize)> {
    let seq = sequence(apps, widgets);
    let from = seq.iter().position(|s| *s == Slot::Widget(j))?;
    let before = Packed::of(apps, widgets, columns);
    let now = before.cell(Slot::Widget(j))?;
    let mut rest = seq.clone();
    rest.remove(from);
    if matches!(dir, Dir::Left | Dir::Right) {
        let candidates: Vec<usize> = if dir == Dir::Right { (from + 1..=rest.len()).collect() } else { (0..from).rev().collect() };
        for q in candidates {
            let mut s = rest.clone();
            s.insert(q, Slot::Widget(j));
            let (list, k) = rebuild(widgets, &s, j);
            let Some(cell) = Packed::of(apps, &list, columns).cell(Slot::Widget(k)) else { continue };
            if cell.order() != now.order() && (cell.order() > now.order()) == (dir == Dir::Right) {
                return Some((list, k));
            }
        }
        return None;
    }
    // Up and down aim for a row: the one above or below on this page, else
    // the next page's top or the one before's foot.
    let (page, row) = match dir {
        Dir::Down if now.row + now.h < ROWS => (now.page, now.row + now.h),
        Dir::Down => (now.page + 1, 0),
        Dir::Up if now.row > 0 => (now.page, now.row - 1),
        Dir::Up if now.page > 0 => (now.page - 1, ROWS - now.h),
        _ => return None,
    };
    // Where everything was, to count what else a move disturbs.
    let key = |list: &[Placed], s: &Slot| match s {
        Slot::App(i) => *i as u64,
        Slot::Widget(k) => widget_key(list[*k].id),
    };
    let was: HashMap<u64, super::Cell> = before.slots.iter().map(|(s, c)| (key(widgets, s), *c)).collect();
    // Each candidate scored: it alone moving first, then how near its
    // column it lands, then how little else moves.
    type Scored = ((u8, usize, usize), Vec<Placed>, usize);
    let scored = |s: &[Slot], family: u8| -> Option<Scored> {
        let (list, k) = rebuild(widgets, s, j);
        let packed = Packed::of(apps, &list, columns);
        let cell = packed.cell(Slot::Widget(k))?;
        if cell.page != page || cell.row != row {
            return None;
        }
        let off = (cell.col as i32 - now.col as i32).unsigned_abs() as usize;
        let moved = packed.slots.iter().filter(|(s, c)| *s != Slot::Widget(k) && was.get(&key(&list, s)) != Some(c)).count();
        Some(((family, off, moved), list, k))
    };
    let mut best: Option<Scored> = None;
    let mut consider = |found: Option<Scored>| {
        if let Some(found) = found {
            if best.as_ref().is_none_or(|b| found.0 < b.0) {
                best = Some(found);
            }
        }
    };
    let mut alone = false;
    for q in 0..=rest.len() {
        let mut s = rest.clone();
        s.insert(q, Slot::Widget(j));
        let found = scored(&s, 0);
        alone |= found.is_some();
        consider(found);
    }
    if !alone {
        // Another widget makes room: taken out and put back anywhere else.
        for (at, other) in seq.iter().enumerate() {
            let Slot::Widget(o) = other else { continue };
            if *o == j {
                continue;
            }
            let mut without = seq.clone();
            without.remove(at);
            for q in 0..=without.len() {
                if q == at {
                    continue;
                }
                let mut s = without.clone();
                s.insert(q, *other);
                consider(scored(&s, 1));
            }
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

    /// Turn to the next kind (`by` −1 the one before) not in `taken` (those
    /// on the board already), sliding. Turned again mid-slide, it carries on
    /// from where the content is, at its speed. With every other kind taken
    /// it stays.
    pub fn turn(&mut self, by: i32, calm: bool, taken: &[Kind]) {
        let n = Kind::ALL.len() as i32;
        let Some(to) = (1..n).map(|i| (self.at as i32 + by.signum() * i).rem_euclid(n) as usize).find(|i| !taken.contains(&Kind::ALL[*i])) else { return };
        self.from = self.kind();
        self.at = to;
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
