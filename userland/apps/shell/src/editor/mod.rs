//! The photo editor: Photos' `e` on a photo (1.0).
//!
//! Four panels beside the picture, chosen with Tab (or 1 to 4), as mode
//! buttons along the panel's top:
//!
//! - adjust: exposure, contrast, highlights, shadows, saturation, warmth,
//!   tint, vibrance, sharpness and a vignette, each a row with its track
//!   (← → by five, Backspace back to nothing);
//! - looks: the Camera's fifteen, each row showing it over this photo (↑ ↓
//!   lays it on; ← → turn Tint's colour);
//! - crop: the crop's shape (free, 4:3, 3:2, 16:9, 1:1; Enter moves and
//!   sizes it), quarter turns, straightening by ±15° over a grid, flips;
//! - markup: arrows, boxes, circles and short labels in the signal colour
//!   or ice, for documenting a robot, placed with a cursor (Enter at the
//!   start, Enter at the end; a label is typed in).
//!
//! Nothing is written to the photo. The edit is kept beside it in a small
//! sidecar (edits.rs), taken up again when the photo is next edited; `x`
//! exports a new full-size JPEG beside it (`IMG_…_edit.jpg`, export.rs) on
//! the cores, its progress along the picture's foot. `o` shows the photo
//! as it was.
//!
//! The preview is made from a working copy (the photo decoded at a quarter
//! of its size, and that halved: whichever is nearest above the view), at
//! the view's own size, about 160 × 120: the geometry when it changes, the
//! tone on every change, on the preview's own thread (its rows on two
//! cores), the newest edit sent as soon as the last picture is made. The
//! tests time a change at about 0.35 ms here; on the handheld, reckoned
//! fifteen times slower, about 5 ms, so the new picture is in a frame or
//! two after the key, and the frame itself (about 6 ms) never waits on it.

mod develop;
pub mod edits;
mod export;
mod markup;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use bezel::{Align, Color, Rect, TextStyle};
use detent::{role, Animated};

pub use self::develop::{by_rows, cores};
use self::develop::Tone;
use self::edits::{Aspect, Edits, Ink, Mark, Shape, ADJUSTS, ASPECTS, SHAPES, WHOLE};
use crate::filters::{Look, LOOKS};
use crate::gfx::{Gfx, ListState, PAD, W};
use crate::photos::{draw_picture, whole, Picture};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// The panels, in their buttons' order, with their icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Adjust,
    Looks,
    Geometry,
    Markup,
}

const PANELS: [(Panel, &str); 4] = [(Panel::Adjust, "tune"), (Panel::Looks, "photo_filter"), (Panel::Geometry, "crop_rotate"), (Panel::Markup, "draw")];

impl Panel {
    fn index(self) -> usize {
        PANELS.iter().position(|(p, _)| *p == self).unwrap_or(0)
    }

    fn rows(self) -> usize {
        match self {
            Panel::Adjust => ADJUSTS.len(),
            Panel::Looks => LOOKS.len(),
            Panel::Geometry => GEO_ROWS.len(),
            Panel::Markup => MARK_ROWS.len(),
        }
    }
}

const GEO_ROWS: [&str; 4] = ["crop", "rotate", "straighten", "flip"];
const MARK_ROWS: [&str; 3] = ["tool", "colour", "marks"];
const STRAIGHTEN: usize = 2;

/// A key's step on an adjustment, and the cursor's (of the picture).
const STEP: i32 = 5;
const CURSOR_STEP: f32 = 0.025;
const NOTE_FOR: f32 = 2.2;

/// The crop being moved, or sized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CropTool {
    Move,
    Size,
}

/// A mark being placed: its start, its end, or a label's words.
#[derive(Clone, Debug, PartialEq)]
enum Placing {
    Start,
    End((f32, f32)),
    Typing((f32, f32), String),
}

/// The working copies: the photo at about a quarter of its size (whole
/// when it is small) and that halved, and the photo's own size.
pub struct Working {
    hi: Picture,
    lo: Picture,
    full: (usize, usize),
}

/// What the geometry's picture was made for.
#[derive(Clone, Debug, PartialEq)]
struct GeoKey {
    geometry: (u8, bool, bool, i32, [i32; 4]),
    whole: bool,
    out: (usize, usize),
}

pub struct Editor {
    path: PathBuf,
    edits: Edits,
    /// As the sidecar last had it, and when it last changed since.
    kept: Edits,
    changed_at: Option<f32>,
    clock: f32,
    loading: Option<Receiver<Result<Working, String>>>,
    work: Option<Arc<Working>>,
    /// The preview's own thread (none: made in the frame, for the shots).
    toner: Option<Toner>,
    trouble: Option<String>,
    /// The preview: the geometry's picture at the view's size, and it toned.
    geo: Picture,
    geo_for: Option<GeoKey>,
    shown: Picture,
    tone_dirty: bool,
    panel: Panel,
    /// The mode buttons: each one's colour and its shape, morphing.
    tab_on: [Animated; 4],
    tab_shape: [Animated; 4],
    lists: [ListState; 4],
    /// The adjustments' tracks, gliding to their values.
    fills: [Animated; 10],
    crop_tool: Option<CropTool>,
    crop_box: [Animated; 4],
    placing: Option<Placing>,
    cursor: (f32, f32),
    cursor_at: [Animated; 2],
    tool: Shape,
    ink: Ink,
    before: bool,
    export: Option<export::Job>,
    bar: Animated,
    note: Option<(String, f32)>,
    /// The looks over this photo, small, and what they were made for.
    thumbs: Vec<Picture>,
    thumbs_for: Option<(GeoKey, [i32; 10], i32)>,
    /// The look made fast, and for which look and colour.
    look_made: Option<((Look, i32), Option<crate::filters::Compiled>)>,
    /// Every look made fast, for the looks' rows, and for which colour.
    looks_made: Option<(i32, Vec<crate::filters::Compiled>)>,
}

impl Editor {
    /// The editor on a photo: its working copy decoded on a thread.
    pub fn open(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let p = path.clone();
        std::thread::Builder::new()
            .name("photo-edit".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                tx.send(working(&p)).ok();
                crate::platform::wake();
            })
            .ok();
        let mut editor = Self::blank(path);
        editor.loading = Some(rx);
        editor.toner = Some(Toner::new());
        editor
    }

    /// The editor with its working copy made now (the shots, the tests).
    pub fn open_now(path: PathBuf) -> Self {
        let mut editor = Self::blank(path);
        match working(&editor.path) {
            Ok(w) => editor.work = Some(Arc::new(w)),
            Err(why) => editor.trouble = Some(why),
        }
        editor
    }

    fn blank(path: PathBuf) -> Self {
        let edits = edits::load(&path);
        let spring = |v: f32, s: detent::Spring| Animated::new(v, s).with_rest(0.002);
        let mut editor = Editor {
            path,
            kept: edits.clone(),
            edits,
            changed_at: None,
            clock: 0.0,
            loading: None,
            work: None,
            toner: None,
            trouble: None,
            geo: Picture::default(),
            geo_for: None,
            shown: Picture::default(),
            tone_dirty: true,
            panel: Panel::Adjust,
            // Mode buttons: colour on the effects spring, shape on snappy.
            tab_on: std::array::from_fn(|i| spring(if i == 0 { 1.0 } else { 0.0 }, role::effect())),
            tab_shape: std::array::from_fn(|i| spring(if i == 0 { 1.0 } else { 0.0 }, role::release())),
            lists: Default::default(),
            fills: std::array::from_fn(|_| spring(0.0, role::release())),
            crop_tool: None,
            crop_box: std::array::from_fn(|i| spring(WHOLE[i], role::release()).with_rest(0.0005)),
            placing: None,
            cursor: (0.5, 0.5),
            cursor_at: std::array::from_fn(|_| spring(0.5, role::hold()).with_rest(0.0005)),
            tool: Shape::Arrow,
            ink: Ink::Signal,
            before: false,
            export: None,
            bar: spring(0.0, role::settle()),
            note: None,
            thumbs: Vec::new(),
            thumbs_for: None,
            look_made: None,
            looks_made: None,
        };
        editor.lists[Panel::Looks.index()].focus = LOOKS.iter().position(|l| *l == editor.edits.look).unwrap_or(0);
        editor.sync(true);
        editor
    }

    /// The springs and the preview follow the edit; `now` lands them at once.
    fn sync(&mut self, now: bool) {
        for (i, a) in ADJUSTS.iter().enumerate() {
            self.fills[i].set(self.edits.get(*a) as f32 / 100.0);
        }
        for (i, v) in self.edits.crop.iter().enumerate() {
            self.crop_box[i].set(*v);
        }
        if now {
            for a in self.fills.iter_mut().chain(self.crop_box.iter_mut()) {
                a.jump(a.target());
            }
        }
        self.tone_dirty = true;
    }

    /// An edit made: the preview follows, and the sidecar soon after.
    fn changed(&mut self) {
        self.sync(false);
        self.changed_at = Some(self.clock);
    }

    /// The sidecar written, when the edit differs from what it holds.
    fn keep(&mut self) {
        self.changed_at = None;
        if self.edits != self.kept {
            match edits::save(&self.path, &self.edits) {
                Ok(()) => self.kept = self.edits.clone(),
                Err(e) => self.say(format!("the edit could not be kept: {e}")),
            }
        }
    }

    fn say(&mut self, words: impl Into<String>) {
        self.note = Some((words.into(), 0.0));
    }

    fn set_panel(&mut self, panel: Panel) {
        self.panel = panel;
        for (i, (p, _)) in PANELS.iter().enumerate() {
            let on = if *p == panel { 1.0 } else { 0.0 };
            self.tab_on[i].set(on);
            self.tab_shape[i].set(on);
        }
        if panel == Panel::Looks {
            self.lists[panel.index()].focus = LOOKS.iter().position(|l| *l == self.edits.look).unwrap_or(0);
        }
    }

    fn row(&self) -> usize {
        self.lists[self.panel.index()].focus
    }

    /// The straightened frame's size, in the photo's own pixels.
    fn frame(&self) -> (f32, f32) {
        let (w, h) = self.work.as_ref().map_or((4, 3), |w| w.full);
        let (tw, th) = develop::turned(w, h, self.edits.turns);
        (tw as f32, th as f32)
    }

    /// The largest box of the crop's shape, centred.
    fn aspect_box(&self) -> [f32; 4] {
        aspect_box(self.edits.aspect, self.frame(), (0.5, 0.5), 1.0)
    }

    // ---- keys ----

    /// ← → on the focused row.
    fn change(&mut self, by: i32) {
        let row = self.row();
        match self.panel {
            Panel::Adjust => {
                let a = ADJUSTS[row];
                self.edits.set(a, self.edits.get(a) + by * STEP);
            }
            Panel::Looks if self.edits.look == Look::Tint => self.edits.hue = (self.edits.hue + by as f32 * 15.0).rem_euclid(360.0),
            Panel::Looks => return,
            Panel::Geometry => match row {
                0 => {
                    let at = ASPECTS.iter().position(|a| *a == self.edits.aspect).unwrap_or(0) as i32 + by;
                    self.edits.aspect = ASPECTS[at.rem_euclid(ASPECTS.len() as i32) as usize];
                    self.edits.crop = self.aspect_box();
                }
                1 => self.turn(by),
                STRAIGHTEN => self.edits.straighten = ((self.edits.straighten + by as f32 * 0.5) * 2.0).round().clamp(-30.0, 30.0) / 2.0,
                _ => {
                    // off → horizontal → vertical → both, and back.
                    let now = usize::from(self.edits.flip_h) + 2 * usize::from(self.edits.flip_v);
                    let next = [0, 1, 3, 2];
                    let at = next.iter().position(|n| *n == now).unwrap_or(0) as i32 + by;
                    let to = next[at.rem_euclid(4) as usize];
                    self.change_flip(to & 1 == 1, to & 2 == 2);
                }
            },
            Panel::Markup => match row {
                0 => {
                    let at = SHAPES.iter().position(|s| *s == self.tool).unwrap_or(0) as i32 + by;
                    self.tool = SHAPES[at.rem_euclid(SHAPES.len() as i32) as usize];
                    return;
                }
                1 => {
                    self.ink = if self.ink == Ink::Signal { Ink::Ice } else { Ink::Signal };
                    return;
                }
                _ => return,
            },
        }
        self.changed();
    }

    /// A quarter turn (clockwise for 1), the crop turning with the picture.
    fn turn(&mut self, by: i32) {
        self.edits.turns = (i32::from(self.edits.turns) + by).rem_euclid(4) as u8;
        let [x, y, w, h] = self.edits.crop;
        self.edits.crop = if by > 0 { [1.0 - y - h, x, h, w] } else { [y, 1.0 - x - w, h, w] };
        if self.edits.aspect != Aspect::Free {
            let c = (self.edits.crop[0] + self.edits.crop[2] * 0.5, self.edits.crop[1] + self.edits.crop[3] * 0.5);
            let scale = (self.edits.crop[2] * self.edits.crop[3]).sqrt().max(0.05) / box_area(self.aspect_box()).sqrt().max(0.05);
            self.edits.crop = aspect_box(self.edits.aspect, self.frame(), c, scale.min(1.0));
        }
    }

    /// Backspace on the focused row: back to how it was.
    fn reset(&mut self) {
        let row = self.row();
        match self.panel {
            Panel::Adjust => self.edits.set(ADJUSTS[row], 0),
            Panel::Looks => {
                self.edits.look = Look::Original;
                self.lists[1].focus = 0;
            }
            Panel::Geometry => match row {
                0 => {
                    self.edits.aspect = Aspect::Free;
                    self.edits.crop = WHOLE;
                }
                1 => {
                    while self.edits.turns != 0 {
                        self.turn(-1);
                    }
                }
                STRAIGHTEN => self.edits.straighten = 0.0,
                _ => {
                    self.change_flip(false, false);
                }
            },
            Panel::Markup => {
                if self.edits.marks.pop().is_none() {
                    self.say("no marks to take back");
                    return;
                }
                self.say("mark taken back");
            }
        }
        self.changed();
    }

    fn change_flip(&mut self, h: bool, v: bool) {
        if h != self.edits.flip_h {
            self.edits.crop[0] = 1.0 - self.edits.crop[0] - self.edits.crop[2];
        }
        if v != self.edits.flip_v {
            self.edits.crop[1] = 1.0 - self.edits.crop[1] - self.edits.crop[3];
        }
        (self.edits.flip_h, self.edits.flip_v) = (h, v);
    }

    /// The crop tool's keys.
    fn crop_key(&mut self, tool: CropTool, key: Key) {
        let fixed = self.edits.aspect != Aspect::Free;
        let [x, y, w, h] = self.edits.crop;
        let centre = (x + w * 0.5, y + h * 0.5);
        let scaled = |me: &Self, k: f32| {
            if fixed {
                let now = (box_area(me.edits.crop) / box_area(me.aspect_box()).max(1e-6)).sqrt();
                aspect_box(me.edits.aspect, me.frame(), centre, (now * k).clamp(0.1, 1.0))
            } else {
                let (nw, nh) = ((w * k).clamp(0.05, 1.0), (h * k).clamp(0.05, 1.0));
                [centre.0 - nw * 0.5, centre.1 - nh * 0.5, nw, nh]
            }
        };
        let step = 0.02;
        self.edits.crop = match (tool, key) {
            (_, Key::Char('+' | '=')) => scaled(self, 1.05),
            (_, Key::Char('-' | '_')) => scaled(self, 1.0 / 1.05),
            (CropTool::Move, Key::Left) => [x - step, y, w, h],
            (CropTool::Move, Key::Right) => [x + step, y, w, h],
            (CropTool::Move, Key::Up) => [x, y - step, w, h],
            (CropTool::Move, Key::Down) => [x, y + step, w, h],
            (CropTool::Size, Key::Right | Key::Up) if fixed => scaled(self, 1.05),
            (CropTool::Size, Key::Left | Key::Down) if fixed => scaled(self, 1.0 / 1.05),
            (CropTool::Size, Key::Left) => [x + step * 0.5, y, w - step, h],
            (CropTool::Size, Key::Right) => [x - step * 0.5, y, w + step, h],
            (CropTool::Size, Key::Up) => [x, y - step * 0.5, w, h + step],
            (CropTool::Size, Key::Down) => [x, y + step * 0.5, w, h - step],
            (_, Key::Backspace) => self.aspect_box(),
            _ => return,
        };
        self.edits.crop = edits::clamp_crop(self.edits.crop);
        self.changed();
    }

    /// The placing keys: the cursor moves; Enter sets the start, the end, or
    /// (for a label) starts the words.
    fn place_key(&mut self, placing: Placing, key: Key) {
        let (cx, cy) = self.cursor;
        match key {
            Key::Left => self.cursor.0 = (cx - CURSOR_STEP).max(0.0),
            Key::Right => self.cursor.0 = (cx + CURSOR_STEP).min(1.0),
            Key::Up => self.cursor.1 = (cy - CURSOR_STEP).max(0.0),
            Key::Down => self.cursor.1 = (cy + CURSOR_STEP).min(1.0),
            Key::Back | Key::Leave => {
                self.placing = None;
                self.say("no mark made");
                return;
            }
            Key::Enter => {
                self.placing = match placing {
                    Placing::Start if self.tool == Shape::Text => Some(Placing::Typing(self.cursor, String::new())),
                    Placing::Start => Some(Placing::End(self.cursor)),
                    Placing::End(a) => {
                        if (a.0 - cx).abs() + (a.1 - cy).abs() < 0.01 {
                            self.say("move the cursor to where it ends");
                            return;
                        }
                        self.edits.marks.push(Mark { shape: self.tool, ink: self.ink, a, b: self.cursor, text: String::new() });
                        self.changed();
                        self.say(format!("{} added", self.tool.name()));
                        None
                    }
                    Placing::Typing(..) => Some(placing),
                };
                return;
            }
            _ => return,
        }
        for (i, v) in [self.cursor.0, self.cursor.1].into_iter().enumerate() {
            self.cursor_at[i].set(v);
        }
    }

    fn type_key(&mut self, at: (f32, f32), mut words: String, key: Key) {
        match key {
            Key::Char(c) if !c.is_control() && words.chars().count() < 28 => words.push(c),
            Key::Backspace => {
                words.pop();
            }
            Key::Back | Key::Leave => {
                self.placing = None;
                self.say("no label made");
                return;
            }
            Key::Enter if words.trim().is_empty() => {
                self.say("type the label first");
            }
            Key::Enter => {
                self.edits.marks.push(Mark { shape: Shape::Text, ink: self.ink, a: at, b: at, text: words.trim().to_owned() });
                self.placing = None;
                self.changed();
                self.say("label added");
                return;
            }
            _ => {}
        }
        self.placing = Some(Placing::Typing(at, words));
    }

    fn start_export(&mut self) {
        if self.export.is_some() {
            self.say("already exporting");
            return;
        }
        if self.edits.is_plain() {
            self.say("no edits yet: nothing to export");
            return;
        }
        self.keep();
        self.export = Some(export::start(self.path.clone(), self.edits.clone()));
        self.bar.jump(0.0);
    }

    /// The preview made for a view of `room` (its width and height): the
    /// geometry when it changed, then the tone. On the preview's own thread
    /// when it has one (the picture in sight until the new one comes, a
    /// frame or two later), else here.
    fn refresh(&mut self, room: (f32, f32)) {
        let Some(work) = self.work.clone() else { return };
        let whole_frame = self.crop_tool.is_some();
        let (tw, th) = develop::turned(work.full.0, work.full.1, self.edits.turns);
        let crop = if whole_frame { WHOLE } else { self.edits.crop };
        let out = develop::fit(crop[2] * tw as f32, crop[3] * th as f32, room.0, room.1);
        let mut geometry = self.edits.geometry();
        if whole_frame {
            geometry.4 = [0; 4];
        }
        let key = GeoKey { geometry, whole: whole_frame, out };
        let reshape = self.geo_for.as_ref() != Some(&key);
        if reshape || self.tone_dirty {
            // The look made again only when it changes.
            let made = (self.edits.look, self.edits.hue.round() as i32);
            if self.look_made.as_ref().map(|(k, _)| *k) != Some(made) {
                self.look_made = Some((made, (self.edits.look != Look::Original).then(|| self.edits.look.compiled(self.edits.hue))));
            }
            let order = Order { work, edits: self.edits.clone(), key: key.clone(), geo: (!reshape).then(|| self.geo.clone()), look: self.look_made.as_ref().and_then(|(_, c)| c.clone()) };
            match &mut self.toner {
                // The first picture is made here: nothing would show meanwhile.
                Some(toner) if !self.shown.is_empty() => {
                    if !toner.busy && toner.orders.send(order).is_ok() {
                        toner.busy = true;
                        self.tone_dirty = false;
                    }
                }
                _ => {
                    let made = order.make();
                    (self.geo, self.geo_for, self.shown) = (made.geo, Some(made.key), made.shown);
                    self.tone_dirty = false;
                }
            }
        }
        // The looks' rows: each look over this photo, small.
        let want = (key, self.edits.values, self.edits.hue as i32);
        if self.panel == Panel::Looks && self.thumbs_for.as_ref() != Some(&want) && !self.geo.is_empty() {
            let plain = Edits { look: Look::Original, ..self.edits.clone() };
            let small = develop::develop(self.geo.cover(32, 24), &Tone::of(&plain), 1);
            let hue = self.edits.hue.round() as i32;
            if self.looks_made.as_ref().map(|(h, _)| *h) != Some(hue) {
                self.looks_made = Some((hue, LOOKS.iter().map(|l| l.compiled(self.edits.hue)).collect()));
            }
            let looks = self.looks_made.as_ref().map(|(_, c)| c.as_slice()).unwrap_or(&[]);
            self.thumbs = looks
                .iter()
                .map(|c| {
                    let mut t = small.clone();
                    for p in t.rgb.chunks_exact_mut(3) {
                        let out = c.pixel(p[0], p[1], p[2]);
                        p.copy_from_slice(&out);
                    }
                    t
                })
                .collect();
            self.thumbs_for = Some(want);
        }
    }

    /// The preview's time (the tests' timing): the tone over it, made again.
    #[cfg(test)]
    fn retone(&mut self) {
        self.tone_dirty = true;
        let room = (self.geo.w as f32, self.geo.h as f32);
        self.refresh(room);
    }
}

/// The working copy of a photo: decoded at about a quarter (the decoder
/// scales as it reads), and that halved.
fn working(path: &Path) -> Result<Working, String> {
    let (fw, fh) = crate::photos::dimensions(path).ok_or("the photo cannot be read")?;
    let k = (820.0 / fw.max(fh) as f32).min(1.0);
    let hi = crate::photos::decode(path, ((fw as f32 * k) as usize).max(1), ((fh as f32 * k) as usize).max(1))?;
    let lo = if hi.w.max(hi.h) > 480 { develop::halve(&hi) } else { hi.clone() };
    Ok(Working { hi, lo, full: (fw, fh) })
}

/// The largest box of `aspect` in a frame `frame` (pixels), times `scale`,
/// centred on `c` (0 to 1) as far as the frame allows.
fn aspect_box(aspect: Aspect, frame: (f32, f32), c: (f32, f32), scale: f32) -> [f32; 4] {
    let Some(r) = aspect.ratio() else {
        return if scale >= 0.999 { WHOLE } else { edits::clamp_crop([c.0 - scale * 0.5, c.1 - scale * 0.5, scale, scale]) };
    };
    let (tw, th) = frame;
    let (bw, bh) = if tw >= th {
        let bw = tw.min(th * r);
        (bw, bw / r)
    } else {
        let bh = th.min(tw * r);
        (bh / r, bh)
    };
    let (w, h) = (bw / tw * scale, bh / th * scale);
    [(c.0 - w * 0.5).clamp(0.0, 1.0 - w), (c.1 - h * 0.5).clamp(0.0, 1.0 - h), w, h]
}

/// The size a `pw` × `ph` picture takes fitted whole in `rw` × `rh`.
pub fn fit_size(pw: f32, ph: f32, rw: f32, rh: f32) -> (f32, f32) {
    let s = (rw / pw.max(1.0)).min(rh / ph.max(1.0));
    (pw * s, ph * s)
}

fn box_area(b: [f32; 4]) -> f32 {
    b[2] * b[3]
}

/// Where the parts go: the picture's room on the left, the panel on the
/// right (as wide as its longest row needs), both down to the hint line.
struct Layout {
    image: Rect,
    panel: Rect,
    tab_h: f32,
    list_top: f32,
    bottom: f32,
}

fn layout(g: &mut Gfx) -> Layout {
    let ty = g.ty;
    let pw = (g.measure("highlights", ty.label) + g.measure("−1.0", ty.label) + 34.0).max(118.0).round();
    let bottom = g.above_hint();
    let panel = Rect::new(W - PAD - pw, PAD, pw, bottom - PAD);
    let image = Rect::new(PAD, PAD, panel.x - 2.0 * PAD, bottom - PAD);
    let tab_h = (ty.label.size + 10.0).round();
    Layout { image, panel, tab_h, list_top: PAD + tab_h + 6.0, bottom }
}

impl Screen for Editor {
    fn title(&self) -> &str {
        "Edit"
    }

    fn app(&self) -> &str {
        "Photos"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn wants_text(&self) -> bool {
        matches!(self.placing, Some(Placing::Typing(..)))
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if let Some(Placing::Typing(at, words)) = self.placing.clone() {
            self.type_key(at, words, key);
            return Nav::Stay;
        }
        if self.work.is_none() {
            return if matches!(key, Key::Back | Key::Leave) { Nav::Pop } else { Nav::Stay };
        }
        if let Some(placing) = self.placing.clone() {
            self.place_key(placing, key);
            return Nav::Stay;
        }
        if let Some(tool) = self.crop_tool {
            match key {
                Key::Enter | Key::Back | Key::Leave => {
                    self.crop_tool = None;
                    self.keep();
                }
                Key::Tab => {
                    self.crop_tool = Some(if tool == CropTool::Move { CropTool::Size } else { CropTool::Move });
                    self.say(if tool == CropTool::Move { "arrows size the crop" } else { "arrows move the crop" });
                }
                _ => self.crop_key(tool, key),
            }
            return Nav::Stay;
        }
        let rows = self.panel.rows();
        match key {
            Key::Back if self.before => self.before = false,
            Key::Back | Key::Leave => {
                self.keep();
                return Nav::Pop;
            }
            Key::Tab => self.set_panel(PANELS[(self.panel.index() + 1) % PANELS.len()].0),
            Key::Char(c @ '1'..='4') => self.set_panel(PANELS[c as usize - '1' as usize].0),
            Key::Char('o') => self.before = !self.before,
            Key::Char('x') => self.start_export(),
            Key::Up | Key::Down => {
                let list = &mut self.lists[self.panel.index()];
                if key == Key::Up {
                    list.up();
                } else {
                    list.down(rows);
                }
                if self.panel == Panel::Looks {
                    let look = LOOKS[self.lists[1].focus];
                    if look != self.edits.look {
                        self.edits.look = look;
                        self.changed();
                    }
                }
            }
            Key::Left => self.change(-1),
            Key::Right => self.change(1),
            Key::Backspace | Key::Delete => self.reset(),
            Key::Enter if self.panel == Panel::Geometry && self.row() == 0 => {
                self.crop_tool = Some(CropTool::Move);
                if self.edits.crop == WHOLE && self.edits.aspect == Aspect::Free {
                    self.edits.crop = [0.1, 0.1, 0.8, 0.8];
                    self.changed();
                }
                self.say("arrows move · + − size · tab: size");
            }
            Key::Enter if self.panel == Panel::Markup => {
                self.placing = Some(Placing::Start);
                self.say(if self.tool == Shape::Text { "move to the label's place · enter" } else { "move to where it starts · enter" });
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        let mut changed = false;
        for a in self.tab_on.iter_mut().chain(self.tab_shape.iter_mut()).chain(self.fills.iter_mut()).chain(self.crop_box.iter_mut()).chain(self.cursor_at.iter_mut()) {
            changed |= a.tick(dt);
        }
        changed |= self.bar.tick(dt);
        if let Some(rx) = &self.loading {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(w) => self.work = Some(Arc::new(w)),
                    Err(why) => self.trouble = Some(why),
                }
                self.loading = None;
                changed = true;
            }
        }
        // The preview's thread made the newest picture.
        if let Some(toner) = &mut self.toner {
            if let Ok(made) = toner.made.try_recv() {
                toner.busy = false;
                (self.geo, self.geo_for, self.shown) = (made.geo, Some(made.key), made.shown);
                changed = true;
            }
        }
        if let Some(job) = &self.export {
            self.bar.set(job.fraction());
            if let Some(done) = job.done() {
                self.export = None;
                match done {
                    Ok(path) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.say(format!("saved as {name}"));
                    }
                    Err(why) => self.say(why),
                }
                changed = true;
            }
        }
        if let Some((_, age)) = &mut self.note {
            *age += dt;
            changed = true;
            if *age > NOTE_FOR + 0.4 {
                self.note = None;
            }
        }
        if self.changed_at.is_some_and(|at| self.clock - at > 0.8) {
            self.keep();
        }
        changed
    }

    fn busy(&self) -> bool {
        self.loading.is_some() || self.export.is_some() || self.toner.as_ref().is_some_and(|t| t.busy)
    }

    fn set_aside(&mut self) {
        self.keep();
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if g.calm {
            for a in self.tab_on.iter_mut().chain(self.tab_shape.iter_mut()).chain(self.fills.iter_mut()).chain(self.crop_box.iter_mut()).chain(self.cursor_at.iter_mut()) {
                a.jump(a.target());
            }
        }
        g.canvas.fill_rect(Rect::new(g.ox, 0.0, W, crate::gfx::H), t.ground, 1.0);
        let lay = layout(g);
        let room = lay.image;
        match (&self.work, &self.trouble) {
            (None, Some(why)) => {
                let why = g.text.fit(why, &ty.caption, room.w - 16.0);
                g.icon("broken_image", false, room.cx(), room.cy() - 12.0, 20.0, t.faint);
                g.text_at(room.cx(), room.cy() + 12.0, Align::Center, &why, ty.caption, t.dim);
            }
            (None, None) => {
                g.icon("photo", false, room.cx(), room.cy() - 12.0, 20.0, t.faint);
                g.text_at(room.cx(), room.cy() + 12.0, Align::Center, "opening the photo", ty.caption, t.dim);
            }
            _ => self.draw_picture(g, room),
        }
        self.draw_panel(g, &lay);
        self.draw_hint(g);
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if self.edits != self.kept {
            edits::save(&self.path, &self.edits).ok();
        }
    }
}

// ---------------------------------------------------------------- drawing ---

impl Editor {
    fn draw_picture(&mut self, g: &mut Gfx, room: Rect) {
        let t = g.tokens;
        self.refresh((room.w, room.h));
        if self.before {
            if let Some(work) = &self.work {
                let (w, h) = develop::fit(work.lo.w as f32, work.lo.h as f32, room.w, room.h);
                let dest = Rect::new((room.cx() - w as f32 * 0.5).round(), (room.cy() - h as f32 * 0.5).round(), w as f32, h as f32);
                draw_picture(g, dest, &work.lo, whole(&work.lo), g.radius(6.0).min(8.0), 1.0);
                let words = if g.measure("before · o: after", g.ty.caption) + 26.0 <= dest.w { "before · o: after" } else { "before" };
                chip(g, dest.x + 6.0, dest.y + 6.0, words, 1.0);
            }
            return;
        }
        let pic = &self.shown;
        let dest = Rect::new((room.cx() - pic.w as f32 * 0.5).round(), (room.cy() - pic.h as f32 * 0.5).round(), pic.w as f32, pic.h as f32);
        let radius = if self.crop_tool.is_some() { 0.0 } else { g.radius(6.0).min(8.0) };
        draw_picture(g, dest, pic, whole(pic), radius, 1.0);
        let white = Color(255, 255, 255);
        if self.crop_tool.is_some() {
            // The crop over the whole frame: the rest dimmed, its thirds, its corners.
            let [x, y, w, h] = [0, 1, 2, 3].map(|i| self.crop_box[i].value());
            let b = Rect::new(dest.x + x * dest.w, dest.y + y * dest.h, w * dest.w, h * dest.h);
            let dim = Color(0, 0, 0);
            for r in [Rect::new(dest.x, dest.y, dest.w, b.y - dest.y), Rect::new(dest.x, b.bottom(), dest.w, dest.bottom() - b.bottom()), Rect::new(dest.x, b.y, b.x - dest.x, b.h), Rect::new(b.right(), b.y, dest.right() - b.right(), b.h)] {
                if r.w > 0.0 && r.h > 0.0 {
                    g.canvas.fill_rect(r.offset(g.ox, 0.0), dim, 0.58);
                }
            }
            for k in 1..3 {
                let f = k as f32 / 3.0;
                g.canvas.fill_rect(Rect::new(b.x + b.w * f - 0.5 + g.ox, b.y, 1.0, b.h), white, 0.4);
                g.canvas.fill_rect(Rect::new(b.x + g.ox, b.y + b.h * f - 0.5, b.w, 1.0), white, 0.4);
            }
            g.canvas.stroke_rrect(b.offset(g.ox, 0.0), 0.0, 1.0, white, 0.9);
            let arm = (b.w.min(b.h) * 0.2).clamp(5.0, 12.0);
            let size = self.crop_tool == Some(CropTool::Size);
            let corner = if size { t.signal } else { white };
            for (cx, cy, sx, sy) in [(b.x, b.y, 1.0, 1.0), (b.right(), b.y, -1.0, 1.0), (b.right(), b.bottom(), -1.0, -1.0), (b.x, b.bottom(), 1.0, -1.0)] {
                let hr = Rect::new(if sx > 0.0 { cx - 1.0 } else { cx - arm + 1.0 }, if sy > 0.0 { cy - 1.0 } else { cy - 2.0 }, arm, 3.0);
                let vr = Rect::new(if sx > 0.0 { cx - 1.0 } else { cx - 2.0 }, if sy > 0.0 { cy - 1.0 } else { cy - arm + 1.0 }, 3.0, arm);
                g.canvas.rrect(hr.offset(g.ox, 0.0), 1.5, corner, 1.0);
                g.canvas.rrect(vr.offset(g.ox, 0.0), 1.5, corner, 1.0);
            }
            let word = format!("{} · {}", self.edits.aspect.name(), if size { "size" } else { "move" });
            chip(g, dest.x + 6.0, dest.y + 6.0, &word, 1.0);
        } else {
            // Straightening: a grid to level against.
            if self.panel == Panel::Geometry && self.row() == STRAIGHTEN {
                for k in 1..6 {
                    let f = k as f32 / 6.0;
                    g.canvas.fill_rect(Rect::new(dest.x + dest.w * f - 0.5 + g.ox, dest.y, 1.0, dest.h), white, 0.32);
                }
                for k in 1..4 {
                    let f = k as f32 / 4.0;
                    g.canvas.fill_rect(Rect::new(dest.x + g.ox, dest.y + dest.h * f - 0.5, dest.w, 1.0), white, 0.32);
                }
            }
            draw_marks(g, dest, &self.edits.marks, 1.0);
            self.draw_placing(g, dest);
        }
        // The export's progress along the picture's foot.
        if self.export.is_some() {
            // A card: the words and the share above, the bar the card's width.
            let cap = g.ty.caption;
            let h = (cap.size + 19.0).round();
            let card = Rect::new(dest.x + 6.0, dest.bottom() - 6.0 - h, dest.w - 12.0, h);
            g.canvas.rrect(card.offset(g.ox, 0.0), g.radius(10.0).min(h * 0.5), Color(0, 0, 0), 0.62);
            let cy = card.y + 5.0 + cap.size * 0.5;
            g.text_alpha(card.x + 10.0, cy, "exporting", cap, white, 1.0);
            let share = format!("{:.0}%", self.bar.value() * 100.0);
            let sw = g.measure(&share, cap);
            g.text_alpha(card.right() - 10.0 - sw, cy, &share, cap, white, 1.0);
            g.meter(Rect::new(card.x + 10.0, card.bottom() - 8.0, card.w - 20.0, 3.0), self.bar.value(), t.signal);
        }
        if let Some((words, age)) = &self.note {
            let a = ((NOTE_FOR + 0.4 - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0);
            let words = g.text.fit(words, &g.ty.caption, dest.w - 26.0);
            let y = if self.crop_tool.is_some() { dest.y + 10.0 + g.ty.caption.size + 8.0 } else { dest.y + 6.0 };
            chip(g, dest.x + 6.0, y, &words, a);
        }
    }

    /// The mark being placed: the cursor, and the shape from its start.
    fn draw_placing(&self, g: &mut Gfx, dest: Rect) {
        let Some(placing) = &self.placing else { return };
        let cur = (self.cursor_at[0].value(), self.cursor_at[1].value());
        let (x, y) = (dest.x + cur.0 * dest.w + g.ox, dest.y + cur.1 * dest.h);
        match placing {
            Placing::End(a) => {
                let m = Mark { shape: self.tool, ink: self.ink, a: *a, b: cur, text: String::new() };
                draw_marks(g, dest, &[m], 0.75);
            }
            Placing::Typing(at, words) => {
                let shown = if words.is_empty() { "type…".to_owned() } else { format!("{words}|") };
                let m = Mark { shape: Shape::Text, ink: self.ink, a: *at, b: *at, text: shown };
                draw_marks(g, dest, &[m], if words.is_empty() { 0.7 } else { 1.0 });
                return;
            }
            Placing::Start => {}
        }
        // A crosshair: white over a dark rim, so it shows on any picture.
        let (ink, _) = markup::colours(self.ink);
        for (w, c, a) in [(3.0, Color(0, 0, 0), 0.55), (1.0, Color(255, 255, 255), 1.0)] {
            g.canvas.polyline(&[(x - 7.0, y), (x - 2.5, y)], w, c, a);
            g.canvas.polyline(&[(x + 2.5, y), (x + 7.0, y)], w, c, a);
            g.canvas.polyline(&[(x, y - 7.0), (x, y - 2.5)], w, c, a);
            g.canvas.polyline(&[(x, y + 2.5), (x, y + 7.0)], w, c, a);
        }
        g.canvas.circle(x, y, 1.6, ink, 1.0);
    }

    fn draw_panel(&mut self, g: &mut Gfx, lay: &Layout) {
        let t = g.tokens;
        let ty = g.ty;
        let p = lay.panel;
        // The mode buttons: the chosen one ice, a capsule; the rest tonal.
        let gap = 3.0;
        let bw = ((p.w - gap * 3.0) / 4.0).floor();
        for (i, (_, icon)) in PANELS.iter().enumerate() {
            let r = Rect::new(p.x + i as f32 * (bw + gap), PAD, bw, lay.tab_h);
            let on = self.tab_on[i].value().clamp(0.0, 1.0);
            let shape = self.tab_shape[i].value();
            let radius = lay.tab_h * (0.25 + 0.25 * shape).clamp(0.0, 0.5);
            g.surface(r, radius, t.surface2.mix(t.ice, on));
            g.icon(icon, on > 0.5, r.cx(), r.cy(), 14.0, t.ink.mix(t.on_ice, on));
        }
        let (top, bottom) = (lay.list_top, lay.bottom);
        let i = self.panel.index();
        match self.panel {
            Panel::Adjust => {
                let row_h = (ty.label.size + 14.0).round();
                let rows = self.lists[i].layout(g, ADJUSTS.len(), top, row_h, 4.0, bottom);
                let focus = self.lists[i].focus;
                for (k, y) in rows {
                    self.slider_row(g, Rect::new(p.x, y, p.w, row_h), k, k == focus);
                }
                self.lists[i].scrollbar(g, ADJUSTS.len(), top, bottom);
            }
            Panel::Looks => {
                let row_h = g.row_h() + 4.0;
                let rows = self.lists[i].layout(g, LOOKS.len(), top, row_h, 4.0, bottom);
                let focus = self.lists[i].focus;
                for (k, y) in rows {
                    let r = Rect::new(p.x, y, p.w, row_h);
                    let radius = g.radius(7.0).min(r.h * 0.5);
                    g.surface(r, radius, t.surface1);
                    let th = r.h - 8.0;
                    let thumb = Rect::new(r.x + 4.0, r.y + 4.0, (th * 4.0 / 3.0).round(), th);
                    match self.thumbs.get(k) {
                        // Pictures are drawn past the list's clip: cut to it here.
                        Some(pic) => draw_within(g, thumb, pic, (radius - 4.0).max(2.0), top - 4.0, bottom + crate::gfx::FOOT_AIR),
                        None => g.surface(thumb, (radius - 4.0).max(2.0), t.surface3),
                    }
                    let name = LOOKS[k].name();
                    let room = r.right() - 8.0 - (thumb.right() + 7.0);
                    let mut style = ty.body;
                    if g.measure(name, style) > room {
                        style = TextStyle { wdth: 78.0, ..style };
                    }
                    g.text_fit(thumb.right() + 7.0, r.cy(), name, style, t.ink, room);
                    if k == focus {
                        g.focus_ring(r, radius);
                    }
                }
                self.lists[i].scrollbar(g, LOOKS.len(), top, bottom);
            }
            Panel::Geometry => {
                let row_h = g.row_h();
                let rows = self.lists[i].layout(g, GEO_ROWS.len(), top, row_h, 4.0, bottom);
                let focus = self.lists[i].focus;
                for (k, y) in rows {
                    let value = match k {
                        0 if self.edits.aspect == Aspect::Free && self.edits.crop != WHOLE => "custom".to_owned(),
                        0 => self.edits.aspect.name().to_owned(),
                        1 => format!("{}°", u32::from(self.edits.turns) * 90),
                        STRAIGHTEN if self.edits.straighten == 0.0 => "0°".to_owned(),
                        STRAIGHTEN => format!("{:+.1}°", self.edits.straighten).replace('-', "−"),
                        _ => match (self.edits.flip_h, self.edits.flip_v) {
                            (false, false) => "off",
                            (true, false) => "mirror",
                            (false, true) => "upside down",
                            (true, true) => "both",
                        }
                        .to_owned(),
                    };
                    let rest = matches!((k, value.as_str()), (0, "free") | (1, "0°") | (STRAIGHTEN, "0°") | (3, "off"));
                    g.row(Rect::new(p.x, y, p.w, row_h), GEO_ROWS[k], &value, if rest { t.dim } else { t.ink }, k == focus);
                }
                self.lists[i].scrollbar(g, GEO_ROWS.len(), top, bottom);
            }
            Panel::Markup => {
                let row_h = g.row_h();
                let rows = self.lists[i].layout(g, MARK_ROWS.len(), top, row_h, 4.0, bottom);
                let focus = self.lists[i].focus;
                for (k, y) in rows {
                    let r = Rect::new(p.x, y, p.w, row_h);
                    let n = self.edits.marks.len();
                    let value = match k {
                        0 => self.tool.name().to_owned(),
                        1 => self.ink.name().to_owned(),
                        _ if n == 0 => "none".to_owned(),
                        _ => n.to_string(),
                    };
                    g.row(r, MARK_ROWS[k], &value, if k == 2 && n == 0 { t.dim } else { t.ink }, k == focus);
                    // The tool's icon and the colour's swatch, before the value.
                    // The value is in Sans or mono by the theme: room for the wider.
                    let vx = r.right() - 10.0 - g.measure(&value, ty.body).max(g.measure(&value, ty.label)) - 10.0;
                    match k {
                        0 => g.icon(self.tool.icon(), false, vx - 1.0, r.cy(), 13.0, t.dim),
                        1 => {
                            let (ink, _) = markup::colours(self.ink);
                            g.dot(vx, r.cy(), 4.5, ink);
                        }
                        _ => {}
                    }
                }
                self.lists[i].scrollbar(g, MARK_ROWS.len(), top, bottom);
            }
        }
    }

    /// An adjustment's row: its name and value, and under them its track,
    /// the ice fill running from nothing to the value and the handle there.
    fn slider_row(&self, g: &mut Gfx, r: Rect, k: usize, focused: bool) {
        let t = g.tokens;
        let ty = g.ty;
        let a = ADJUSTS[k];
        let radius = g.radius(7.0).min(r.h * 0.5);
        g.surface(r, radius, t.surface1);
        let cy = r.y + 4.0 + ty.label.size * 0.5;
        g.text_at(r.x + 9.0, cy, Align::Left, a.name(), ty.label, t.ink);
        let v = self.edits.get(a);
        g.text_at(r.right() - 9.0, cy, Align::Right, &a.words(v), ty.label, if v != 0 { t.ink } else { t.dim });
        let track = Rect::new(r.x + 9.0, r.bottom() - 8.0, r.w - 18.0, 3.0);
        g.surface(track, 1.5, t.surface3);
        let f = self.fills[k].value();
        let two_way = a.min() < 0;
        let (from, to): (f32, f32) = if two_way { (0.5, 0.5 + f * 0.5) } else { (0.0, f) };
        let (x0, x1) = (track.x + track.w * from.min(to), track.x + track.w * from.max(to));
        if two_way {
            g.surface(Rect::new(track.cx() - 0.5, track.y - 1.5, 1.0, 6.0), 0.5, t.dim);
        }
        if x1 - x0 > 0.5 {
            g.surface(Rect::new(x0, track.y, x1 - x0, track.h), 1.5, t.ice);
        }
        let hx = track.x + track.w * to.clamp(0.0, 1.0);
        g.surface(Rect::new(hx - 1.5, track.cy() - 4.5, 3.0, 9.0), 1.5, t.ink);
        if focused {
            g.focus_ring(r, radius);
        }
    }

    fn draw_hint(&self, g: &mut Gfx) {
        if self.export.is_some() {
            g.hint_any(&["exporting at full size · esc: leave it to finish", "exporting at full size"]);
            return;
        }
        let choices: &[&str] = match (&self.placing, self.crop_tool) {
            (Some(Placing::Typing(..)), _) => &["type the label · enter: place it · esc: none", "enter: place it · esc: none"],
            (Some(Placing::Start), _) if self.tool == Shape::Text => &["arrows: the label's place · enter · esc: none", "arrows · enter · esc"],
            (Some(Placing::Start), _) => &["arrows: where it starts · enter · esc: none", "arrows · enter · esc"],
            (Some(Placing::End(_)), _) => &["arrows: where it ends · enter: draw it · esc: none", "arrows · enter: draw it"],
            (None, Some(CropTool::Move)) => &["arrows move · + − size · tab: width and height · enter: done", "arrows move · + − size · enter: done", "arrows · + − · enter"],
            (None, Some(CropTool::Size)) => &["←→ width · ↑↓ height · tab: move · enter: done", "←→ ↑↓ size · enter: done"],
            _ if self.before => &["o: after · esc: back to the edit"],
            _ => match (self.panel, self.row()) {
                (Panel::Adjust, _) => &["←→ change · backspace: reset · tab: panels · x: export", "←→ change · tab: panels · x: export", "←→ · tab · x: export"],
                (Panel::Looks, _) if self.edits.look == Look::Tint => &["↑↓ looks · ←→ tint's colour · tab: panels · x: export", "↑↓ looks · ←→ colour · x: export"],
                (Panel::Looks, _) => &["↑↓ lay a look on · backspace: none · tab · x: export", "↑↓ looks · tab: panels · x: export", "↑↓ · tab · x: export"],
                (Panel::Geometry, 0) => &["←→ shape · enter: move the crop · tab · x: export", "←→ shape · enter: move it · x: export", "←→ · enter · x"],
                (Panel::Geometry, 1) => &["←→ turn a quarter · backspace: upright · x: export", "←→ turn · x: export"],
                (Panel::Geometry, STRAIGHTEN) => &["←→ half a degree · backspace: level · x: export", "←→ straighten · x: export"],
                (Panel::Geometry, _) => &["←→ flip · backspace: unflip · tab: panels · x: export", "←→ flip · x: export"],
                (Panel::Markup, _) => &["enter: place a mark · ←→ change · backspace: undo · x", "enter: place · backspace: undo", "enter: place"],
            },
        };
        g.hint_any(choices);
    }
}

/// A picture drawn into `dest`, only where it lies between `lo` and `hi`
/// (pictures go past a list's clip, which they must not).
fn draw_within(g: &mut Gfx, dest: Rect, pic: &Picture, radius: f32, lo: f32, hi: f32) {
    let (y0, y1) = (dest.y.max(lo), dest.bottom().min(hi));
    if y1 - y0 < 1.0 {
        return;
    }
    let k = pic.h as f32 / dest.h;
    let view = [0.0, (y0 - dest.y) * k, pic.w as f32, (y1 - y0) * k];
    let cut = y0 > dest.y + 0.5 || y1 < dest.bottom() - 0.5;
    draw_picture(g, Rect::new(dest.x, y0, dest.w, y1 - y0), pic, view, if cut { 0.0 } else { radius }, 1.0);
}

/// Words on a dark chip over the picture.
fn chip(g: &mut Gfx, x: f32, y: f32, words: &str, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let cap = g.ty.caption;
    let w = g.measure(words, cap) + 14.0;
    let h = cap.size + 8.0;
    g.canvas.rrect(Rect::new(x + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.6 * alpha);
    g.text_alpha(x + 7.0, y + h * 0.5, words, cap, Color(240, 240, 240), alpha);
}

/// The marks over the picture at `dest`, as the export will draw them.
fn draw_marks(g: &mut Gfx, dest: Rect, marks: &[Mark], alpha: f32) {
    let width = markup::line_width(dest.w, dest.h);
    for m in marks {
        let (ink, on) = markup::colours(m.ink);
        if m.shape == Shape::Text {
            let style = markup::text_style(markup::text_size(dest.w, dest.h));
            let words = if m.text.trim().is_empty() { "label" } else { m.text.as_str() };
            let tw = g.measure(words, style);
            let [x, y, bw, bh] = markup::label_box(m, dest.w, dest.h, tw);
            g.canvas.rrect(Rect::new(dest.x + x + g.ox, dest.y + y, bw, bh), bh * 0.5, ink, alpha);
            let fitted = g.text.fit(words, &style, bw - style.size * 0.9);
            let fw = g.measure(&fitted, style);
            g.text_alpha(dest.x + x + (bw - fw) * 0.5, dest.y + y + bh * 0.5, &fitted, style, on, alpha);
            continue;
        }
        for path in markup::paths(m, dest.w, dest.h) {
            let pts: Vec<(f32, f32)> = path.iter().map(|(x, y)| (dest.x + x + g.ox, dest.y + y)).collect();
            g.canvas.polyline(&pts, width, ink, alpha);
        }
    }
}

// ------------------------------------------------------- the preview's thread ---

/// What the preview's thread is asked: the working copy, the edit, the view
/// it is for, the geometry's picture when that has not changed, the look.
struct Order {
    work: Arc<Working>,
    edits: Edits,
    key: GeoKey,
    geo: Option<Picture>,
    look: Option<crate::filters::Compiled>,
}

/// What it makes: the geometry's picture, and it toned.
struct Made {
    key: GeoKey,
    geo: Picture,
    shown: Picture,
}

impl Order {
    fn make(self) -> Made {
        let geo = self.geo.unwrap_or_else(|| {
            let (whole_frame, out) = (self.key.whole, self.key.out);
            let crop = if whole_frame { WHOLE } else { self.edits.crop };
            // From the smaller copy when it has the pixels for the view.
            let (lw, _) = develop::turned(self.work.lo.w, self.work.lo.h, self.edits.turns);
            let src = if crop[2] * lw as f32 >= out.0 as f32 * 0.98 { &self.work.lo } else { &self.work.hi };
            develop::shape(src, &self.edits, whole_frame, out, 1)
        });
        let tone = Tone::with_look(&self.edits, self.look);
        // Two cores for a full view; one for a small one.
        let threads = if geo.w * geo.h >= 16_384 { 2 } else { 1 };
        let shown = develop::develop(geo.clone(), &tone, threads);
        Made { key: self.key, geo, shown }
    }
}

/// The preview's own thread: an order at a time, the newest sent once the
/// last is made, so the frame never waits on it.
struct Toner {
    orders: Sender<Order>,
    made: Receiver<Made>,
    busy: bool,
}

impl Toner {
    fn new() -> Toner {
        let (orders, rx) = mpsc::channel::<Order>();
        let (tx, made) = mpsc::channel();
        std::thread::Builder::new()
            .name("photo-preview".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                while let Ok(order) = rx.recv() {
                    if tx.send(order.make()).is_err() {
                        return;
                    }
                    crate::platform::wake();
                }
            })
            .ok();
        Toner { orders, made, busy: false }
    }
}
