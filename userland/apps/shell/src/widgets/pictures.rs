//! Pictures made once for a widget and laid down each frame: a photo cut to
//! its tile with its corners rounded, a song's cover, the battery's day as
//! a mask. Making one costs what drawing it the long way does (a sample of
//! the picture for every pixel, a distance for every pixel of a line); laying
//! one down is a copy, so a widget with a picture draws as fast as one with
//! words, at 60 frames a second while Home turns.
//!
//! A picture is laid down on whole pixels, or on half pixels for one that
//! fills its tile (whose edges meet the tile's and the focus ring's): a
//! tile at rest on the glance page can stand half a pixel along. One made
//! for each half pixel it has been asked at is kept. They are kept by what
//! they were made from, the most recently used first, within [`BUDGET`]
//! bytes.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use bezel::{Color, Rect};

use crate::gfx::Gfx;
use crate::photos::Picture;

/// The most the kept pictures may weigh, in bytes (a large photo on the
/// glance page is about 150 KB).
const BUDGET: usize = 1024 * 1024;

/// How dark a picture's shade is, where words stand on it.
pub const SHADE: f32 = 0.42;

/// A picture to lay over what is drawn: `0xKKRRGGBB`, its colour already
/// multiplied by its cover and `KK` how much of what is under it shows.
pub struct Sprite {
    w: i32,
    h: i32,
    px: Vec<u32>,
}

/// A coverage mask, 0 to 255, laid down in a colour.
pub struct Mask {
    w: i32,
    h: i32,
    data: Vec<u8>,
}

enum Kept {
    Sprite(Rc<Sprite>),
    Masks(Rc<Vec<Mask>>),
}

impl Kept {
    fn weight(&self) -> usize {
        match self {
            Kept::Sprite(s) => s.px.len() * 4,
            Kept::Masks(m) => m.iter().map(|m| m.data.len()).sum(),
        }
    }
}

/// The pictures kept: by key, the most recently used last.
#[derive(Default)]
pub struct Pictures {
    kept: RefCell<Vec<(u64, Kept)>>,
}

/// A key for a picture: a hash of what it is made from.
pub fn key(parts: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Where a picture goes, on whole pixels or (`halves`) half pixels: the
/// pixel its first column and row are laid at, and how far into that
/// pixel the picture begins (0 or 0.5).
fn place(at: f32, halves: bool) -> (i32, f32) {
    if halves {
        let doubled = (at * 2.0).round();
        let whole = (doubled / 2.0).floor();
        (whole as i32, doubled / 2.0 - whole)
    } else {
        (at.round() as i32, 0.0)
    }
}

impl Pictures {
    fn find(&self, key: u64) -> Option<usize> {
        self.kept.borrow().iter().position(|(k, _)| *k == key)
    }

    /// Keep `made` under `key`, letting the least used go past the budget.
    fn keep(&self, key: u64, made: Kept) {
        let mut kept = self.kept.borrow_mut();
        kept.push((key, made));
        while kept.len() > 1 && kept.iter().map(|(_, k)| k.weight()).sum::<usize>() > BUDGET {
            kept.remove(0);
        }
    }

    /// The one under `key`, made by `make` if it is not kept.
    fn sprite(&self, key: u64, make: impl FnOnce() -> Sprite) -> Rc<Sprite> {
        if let Some(at) = self.find(key) {
            let mut kept = self.kept.borrow_mut();
            let entry = kept.remove(at);
            let Kept::Sprite(s) = &entry.1 else { unreachable!("a key names one kind of picture") };
            let s = s.clone();
            kept.push(entry);
            return s;
        }
        let s = Rc::new(make());
        self.keep(key, Kept::Sprite(s.clone()));
        s
    }

    fn masks(&self, key: u64, make: impl FnOnce() -> Vec<Mask>) -> Rc<Vec<Mask>> {
        if let Some(at) = self.find(key) {
            let mut kept = self.kept.borrow_mut();
            let entry = kept.remove(at);
            let Kept::Masks(m) = &entry.1 else { unreachable!("a key names one kind of picture") };
            let m = m.clone();
            kept.push(entry);
            return m;
        }
        let m = Rc::new(make());
        self.keep(key, Kept::Masks(m.clone()));
        m
    }

    /// `pic` (its part `view`: x, y, w, h in its pixels) drawn into `dest`
    /// with its corners rounded by `radius`, at `alpha`, and shaded from
    /// `shade` down (that far from its top) for words to stand on. `source`
    /// names the picture (its file), for keeping it; `halves` lays it on
    /// half pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn picture(&self, g: &mut Gfx, source: &impl Hash, pic: &Picture, view: [f32; 4], dest: Rect, radius: f32, alpha: f32, halves: bool, shade: Option<f32>) {
        if pic.w == 0 || pic.h == 0 || dest.w < 1.0 || dest.h < 1.0 || alpha <= 0.0 {
            return;
        }
        let (x, fx) = place(dest.x + g.ox, halves);
        let (y, fy) = place(dest.y, halves);
        let (w, h) = ((dest.w + fx).ceil() as i32, (dest.h + fy).ceil() as i32);
        let k = key((source, pic.w, pic.h, view.map(f32::to_bits), dest.w.to_bits(), dest.h.to_bits(), radius.to_bits(), (fx * 2.0) as u8, (fy * 2.0) as u8, shade.map(f32::to_bits)));
        let sprite = self.sprite(k, || cut(pic, view, dest.w, dest.h, fx, fy, w, h, radius, shade));
        g.canvas.blit_premultiplied(x, y, sprite.w, sprite.h, &sprite.px, alpha);
    }

    /// Something drawn once as a coverage mask and laid down in `color`
    /// each frame: `paint` draws it in white on a canvas `area` big (its
    /// own coordinates); `source` names what it shows.
    pub fn mask(&self, g: &mut Gfx, source: &impl Hash, area: Rect, color: Color, alpha: f32, paint: impl FnOnce(&mut bezel::Canvas)) {
        let (x, y) = ((area.x + g.ox).round() as i32, area.y.round() as i32);
        let (w, h) = (area.w.ceil() as i32, area.h.ceil() as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let k = key((source, w, h));
        let masks = self.masks(k, || {
            let mut canvas = bezel::Canvas::new(w, h);
            paint(&mut canvas);
            vec![Mask { w, h, data: canvas.px.iter().map(|p| (p >> 16) as u8).collect() }]
        });
        if let Some(mask) = masks.first() {
            g.canvas.mask(x, y, mask.w, mask.h, &mask.data, color, alpha);
        }
    }

    /// Lines drawn once as masks, each in a colour of its own each frame:
    /// `lines` are polylines in `area`'s own coordinates, each with the
    /// index of its colour in `colors`; `source` names what they show.
    pub fn lines(&self, g: &mut Gfx, source: &impl Hash, area: Rect, lines: &[(usize, Vec<(f32, f32)>)], width: f32, colors: &[Color], alpha: f32) {
        let (x, y) = ((area.x + g.ox).round() as i32, area.y.round() as i32);
        let (w, h) = (area.w.ceil() as i32 + 1, area.h.ceil() as i32 + 1);
        if w <= 1 || h <= 1 {
            return;
        }
        let k = key((source, w, h, width.to_bits(), colors.len()));
        let masks = self.masks(k, || {
            (0..colors.len())
                .map(|c| {
                    // White on black: the red of each pixel is its cover.
                    let mut canvas = bezel::Canvas::new(w, h);
                    for (_, pts) in lines.iter().filter(|(i, _)| *i == c) {
                        canvas.polyline(pts, width, Color(255, 255, 255), 1.0);
                    }
                    Mask { w, h, data: canvas.px.iter().map(|p| (p >> 16) as u8).collect() }
                })
                .collect()
        });
        for (mask, color) in masks.iter().zip(colors) {
            g.canvas.mask(x, y, mask.w, mask.h, &mask.data, *color, alpha);
        }
    }
}

/// `pic`'s part `view` sampled into a `w` × `h` picture whose own pixels
/// begin `fx`, `fy` into its first column and row, `dest_w` × `dest_h`
/// across, its corners rounded by `radius` and its edges covered as far as
/// they reach into a pixel; from `shade` down, darkened by [`SHADE`].
#[allow(clippy::too_many_arguments)]
fn cut(pic: &Picture, view: [f32; 4], dest_w: f32, dest_h: f32, fx: f32, fy: f32, w: i32, h: i32, radius: f32, shade: Option<f32>) -> Sprite {
    let [vx, vy, vw, vh] = view;
    let (sx, sy) = (vw / dest_w, vh / dest_h);
    let (pw, ph) = (pic.w as i32, pic.h as i32);
    let r = radius.max(0.0);
    let mut px = vec![255u32 << 24; (w * h) as usize];
    for j in 0..h {
        // This pixel's middle, in the destination's own coordinates.
        let cy = j as f32 + 0.5 - fy;
        let cover_y = (cy + 0.5).clamp(0.0, 1.0) * (dest_h - cy + 0.5).clamp(0.0, 1.0);
        if cover_y <= 0.0 {
            continue;
        }
        let v = (vy + cy * sy - 0.5).clamp(0.0, (ph - 1) as f32);
        let (v0, vf) = (v.floor() as i32, v.fract());
        let v1 = (v0 + 1).min(ph - 1);
        let wy = (vf * 256.0) as u32;
        // How much of the picture is left under the shade, in 256ths: the
        // band's top edge covered as far as it reaches into this row.
        let lit = shade.map_or(256, |top| 256 - (SHADE * 256.0 * (cy - top + 0.5).clamp(0.0, 1.0)) as u32);
        for i in 0..w {
            let cx = i as f32 + 0.5 - fx;
            let mut a = cover_y * (cx + 0.5).clamp(0.0, 1.0) * (dest_w - cx + 0.5).clamp(0.0, 1.0);
            if r > 0.0 {
                let qx = cx.min(dest_w - cx);
                let qy = cy.min(dest_h - cy);
                if qx < r && qy < r {
                    let d = ((r - qx).powi(2) + (r - qy).powi(2)).sqrt();
                    a *= (r - d + 0.5).clamp(0.0, 1.0);
                }
            }
            if a <= 0.0 {
                continue;
            }
            let u = (vx + cx * sx - 0.5).clamp(0.0, (pw - 1) as f32);
            let (u0, uf) = (u.floor() as i32, u.fract());
            let u1 = (u0 + 1).min(pw - 1);
            let wx = (uf * 256.0) as u32;
            let p = |x: i32, y: i32| (y * pw + x) as usize * 3;
            let (a00, a10, a01, a11) = (p(u0, v0), p(u1, v0), p(u0, v1), p(u1, v1));
            let cover = (a.min(1.0) * 255.0).round() as u32;
            let mut out = (255 - cover) << 24;
            for (c, shift) in [16u32, 8, 0].into_iter().enumerate() {
                let top = u32::from(pic.rgb[a00 + c]) * (256 - wx) + u32::from(pic.rgb[a10 + c]) * wx;
                let bottom = u32::from(pic.rgb[a01 + c]) * (256 - wx) + u32::from(pic.rgb[a11 + c]) * wx;
                let value = ((top * (256 - wy) + bottom * wy) >> 16) * lit >> 8;
                out |= (value * cover / 255) << shift;
            }
            px[(j * w + i) as usize] = out;
        }
    }
    Sprite { w, h, px }
}
