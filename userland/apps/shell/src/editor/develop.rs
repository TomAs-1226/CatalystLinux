//! The editor's pictures: the geometry (quarter turns, flips, the
//! straightening and the crop: one affine map, sampled smoothly) and the
//! tone (white balance and the tone curve folded into a table a channel,
//! saturation and vibrance, the look, the vignette, then sharpening). The
//! same code makes the preview, on the working copy at the view's size, and
//! the export, at the photo's own size on every core.

use std::ops::Range;

use super::edits::{Adjust, Edits};
use crate::filters::{Compiled, Look};
use crate::photos::Picture;

// --------------------------------------------------------------- geometry ---

/// A picture's size after `turns` quarter turns.
pub fn turned(w: usize, h: usize, turns: u8) -> (usize, usize) {
    if turns % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// How much a `w` × `h` frame is enlarged when it is turned by `deg`, so the
/// turned picture still covers it with no empty corners: each of the
/// frame's corners, turned back, must lie inside the picture.
pub fn fill_scale(w: f32, h: f32, deg: f32) -> f32 {
    let t = deg.to_radians().abs();
    t.cos() + (w / h).max(h / w) * t.sin()
}

/// From a pixel of the made picture to the place in the source it comes
/// from: `src = (a x + b y + c, d x + e y + f)`, in pixels (edges, not centres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    #[cfg(test)]
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.b * y + self.c, self.d * x + self.e * y + self.f)
    }
}

/// The geometry of `e` as a map from an `out`-sized picture to a
/// `src`-sized one, showing `view` (x, y, width, height, 0 to 1 of the
/// straightened frame: the crop, or the whole frame while it is chosen).
/// The picture is turned a quarter at a time, flipped, then straightened
/// about its middle (enlarged to fill), then cut; this undoes each in turn.
pub fn mapping(src: (usize, usize), e: &Edits, view: [f32; 4], out: (usize, usize)) -> Affine {
    let (sw, sh) = (src.0 as f32, src.1 as f32);
    let (tw, th) = if e.turns % 2 == 1 { (sh, sw) } else { (sw, sh) };
    let s = fill_scale(tw, th, e.straighten);
    let (sin, cos) = e.straighten.to_radians().sin_cos();
    let (ow, oh) = (out.0.max(1) as f32, out.1.max(1) as f32);
    let point = |x: f32, y: f32| -> (f32, f32) {
        // Into the straightened frame's pixels.
        let px = (view[0] + x / ow * view[2]) * tw;
        let py = (view[1] + y / oh * view[3]) * th;
        // Turned back (clockwise was forward, on a panel's downward y) and
        // shrunk by the fill, about the middle.
        let (dx, dy) = ((px - tw * 0.5) / s, (py - th * 0.5) / s);
        let (mut qx, mut qy) = (tw * 0.5 + dx * cos + dy * sin, th * 0.5 - dx * sin + dy * cos);
        if e.flip_h {
            qx = tw - qx;
        }
        if e.flip_v {
            qy = th - qy;
        }
        // The quarter turns undone (clockwise: the top-left went top-right).
        match e.turns % 4 {
            1 => (qy, sh - qx),
            2 => (sw - qx, sh - qy),
            3 => (sw - qy, qx),
            _ => (qx, qy),
        }
    };
    let (c, f) = point(0.0, 0.0);
    let (x1, y1) = point(1.0, 0.0);
    let (x2, y2) = point(0.0, 1.0);
    Affine { a: x1 - c, b: x2 - c, c, d: y1 - f, e: y2 - f, f }
}

/// Rows `rows` of the `ow`-wide picture `map` makes from `src`, each pixel
/// sampled smoothly (bilinear) at its centre, into `out` (those rows).
pub fn resample_rows(src: &Picture, map: &Affine, ow: usize, rows: Range<usize>, out: &mut [u8]) {
    if src.is_empty() {
        return;
    }
    let (pw, ph) = (src.w, src.h);
    let (max_x, max_y) = ((pw - 1) as f32, (ph - 1) as f32);
    for (k, y) in rows.enumerate() {
        let fy = y as f32 + 0.5;
        // The row's first centre, in the source's pixel centres.
        let mut sx = map.a * 0.5 + map.b * fy + map.c - 0.5;
        let mut sy = map.d * 0.5 + map.e * fy + map.f - 0.5;
        let row = &mut out[k * ow * 3..(k + 1) * ow * 3];
        for px in row.chunks_exact_mut(3) {
            let (u, v) = (sx.clamp(0.0, max_x), sy.clamp(0.0, max_y));
            let (u0, v0) = (u as usize, v as usize);
            let (u1, v1) = ((u0 + 1).min(pw - 1), (v0 + 1).min(ph - 1));
            let (wx, wy) = (((u - u0 as f32) * 256.0) as u32, ((v - v0 as f32) * 256.0) as u32);
            let (a, b, c, d) = ((v0 * pw + u0) * 3, (v0 * pw + u1) * 3, (v1 * pw + u0) * 3, (v1 * pw + u1) * 3);
            for (ch, o) in px.iter_mut().enumerate() {
                let top = u32::from(src.rgb[a + ch]) * (256 - wx) + u32::from(src.rgb[b + ch]) * wx;
                let bottom = u32::from(src.rgb[c + ch]) * (256 - wx) + u32::from(src.rgb[d + ch]) * wx;
                *o = ((top * (256 - wy) + bottom * wy + 32768) >> 16) as u8;
            }
            sx += map.a;
            sy += map.d;
        }
    }
}

/// A picture halved each way, each pixel the mean of four.
pub fn halve(pic: &Picture) -> Picture {
    let (w, h) = ((pic.w / 2).max(1), (pic.h / 2).max(1));
    let mut out = Picture::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let at = |dx: usize, dy: usize| ((y * 2 + dy).min(pic.h - 1) * pic.w + (x * 2 + dx).min(pic.w - 1)) * 3;
            for c in 0..3 {
                let sum: u32 = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)].iter().map(|i| u32::from(pic.rgb[i + c])).sum();
                out.rgb[(y * w + x) * 3 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    out
}

/// The largest `w` × `h` (rounded) of a picture `pw` × `ph` that fits in
/// `room_w` × `room_h`, at its shape.
pub fn fit(pw: f32, ph: f32, room_w: f32, room_h: f32) -> (usize, usize) {
    let s = (room_w / pw.max(1.0)).min(room_h / ph.max(1.0));
    (((pw * s).round() as usize).max(1), ((ph * s).round() as usize).max(1))
}

/// The geometry's picture: `e`'s crop of `src` (or the whole straightened
/// frame, `whole`), made at `out`.
pub fn shape(src: &Picture, e: &Edits, whole: bool, out: (usize, usize), threads: usize) -> Picture {
    let view = if whole { super::edits::WHOLE } else { e.crop };
    let map = mapping((src.w, src.h), e, view, out);
    let mut pic = Picture::new(out.0, out.1);
    by_rows(&mut pic.rgb, out.0 * 3, threads, |y0, rows| {
        let n = rows.len() / (out.0 * 3);
        resample_rows(src, &map, out.0, y0..y0 + n, rows);
    });
    pic
}

// ------------------------------------------------------------------- tone ---

/// The adjustments and the look made fast, from one definition ([`tone`]).
pub struct Tone {
    /// White balance and the tone curve, a table a channel.
    lut: [[u8; 256]; 3],
    /// Saturation (256: as it was) and vibrance (256 at 100).
    sat: i32,
    vib: i32,
    look: Option<Compiled>,
    /// −1 (dark corners) to 1 (light ones).
    vignette: f32,
    /// Sharpening's amount (256: the detail once more).
    pub sharp: i32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A soft bump around `at`, a third of the range wide each way.
fn bump(x: f32, at: f32) -> f32 {
    let u = (x - at) / 0.35;
    if u.abs() >= 1.0 {
        0.0
    } else {
        (1.0 - u * u).powi(2)
    }
}

/// The tone curve on a value from 0 to 1: exposure in stops on linear light
/// (50 a stop), contrast as an S (or flattened toward grey), then
/// highlights and shadows, each a bump at its end of the range (at most a
/// fifth, so the curve never turns back on itself).
pub fn tone(x: f32, e: &Edits) -> f32 {
    let v = |a: Adjust| e.get(a) as f32 / 100.0;
    let stops = e.get(Adjust::Exposure) as f32 / 50.0;
    let mut x = (x.max(0.0).powf(2.2) * 2f32.powf(stops)).min(1.0).powf(1.0 / 2.2);
    let c = v(Adjust::Contrast);
    x = if c >= 0.0 { x + c * (smoothstep(0.0, 1.0, x) - x) } else { x + c * 0.5 * (x - 0.5) };
    x += v(Adjust::Highlights) * 0.2 * bump(x, 0.75);
    x += v(Adjust::Shadows) * 0.2 * bump(x, 0.25);
    x.clamp(0.0, 1.0)
}

/// The white balance's gains: warmth toward red from blue, tint toward
/// magenta (less green) or green.
pub fn gains(e: &Edits) -> [f32; 3] {
    let (warm, tint) = (e.get(Adjust::Warmth) as f32 / 100.0, e.get(Adjust::Tint) as f32 / 100.0);
    [1.0 + 0.2 * warm, 1.0 - 0.15 * tint, 1.0 - 0.2 * warm]
}

impl Tone {
    pub fn of(e: &Edits) -> Tone {
        Tone::with_look(e, (e.look != Look::Original).then(|| e.look.compiled(e.hue)))
    }

    /// The tone with its look already made (the editor keeps it while only
    /// the adjustments change).
    pub fn with_look(e: &Edits, look: Option<Compiled>) -> Tone {
        let g = gains(e);
        let mut lut = [[0u8; 256]; 3];
        // The curve once at each of 257 steps, then read through each gain.
        let curve: Vec<f32> = (0..=256).map(|i| tone(i as f32 / 256.0, e)).collect();
        for (c, table) in lut.iter_mut().enumerate() {
            for (i, out) in table.iter_mut().enumerate() {
                let at = (i as f32 / 255.0 * g[c]).min(1.0) * 256.0;
                let (i0, k) = (at.floor() as usize, at.fract());
                let y = curve[i0] + (curve[(i0 + 1).min(256)] - curve[i0]) * k;
                *out = (y * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        let v = |a: Adjust| e.get(a) as f32 / 100.0;
        Tone {
            lut,
            sat: ((1.0 + v(Adjust::Saturation)) * 256.0).round() as i32,
            vib: (v(Adjust::Vibrance) * 256.0).round() as i32,
            look,
            vignette: v(Adjust::Vignette),
            sharp: (v(Adjust::Sharpness) * 1.2 * 256.0).round() as i32,
        }
    }

    /// One pixel through the point operations: white balance and tone,
    /// saturation and vibrance (which lifts the dull more than the bright),
    /// then the look.
    #[inline]
    pub fn pixel(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let (r, g, b) = (i32::from(self.lut[0][r as usize]), i32::from(self.lut[1][g as usize]), i32::from(self.lut[2][b as usize]));
        let (r, g, b) = if self.sat != 256 || self.vib != 0 {
            let l = (77 * r + 150 * g + 29 * b + 128) >> 8;
            let spread = r.max(g).max(b) - r.min(g).min(b);
            let k = (self.sat + self.vib * (255 - spread) / 255).max(0);
            let f = |c: i32| (l + (((c - l) * k + 128) >> 8)).clamp(0, 255);
            (f(r), f(g), f(b))
        } else {
            (r, g, b)
        };
        match &self.look {
            Some(look) => look.pixel(r as u8, g as u8, b as u8),
            None => [r as u8, g as u8, b as u8],
        }
    }

    /// The vignette's two factors at a place (256: none): one darkening
    /// toward black, one lifting toward white. Nothing in the middle,
    /// rising toward the corners (`r2`: 0 in the middle, 1 at a corner).
    #[inline]
    fn vignette_at(&self, r2: f32) -> (i32, i32) {
        let a = self.vignette * smoothstep(0.15, 1.0, r2);
        if a < 0.0 {
            (((1.0 + 0.75 * a) * 256.0).round() as i32, 0)
        } else {
            (256, (0.6 * a * 256.0).round() as i32)
        }
    }

    /// Rows of a `w` × `h` picture through the point operations and the
    /// vignette, in place; `y0` is where the rows start in the picture.
    pub fn develop_rows(&self, w: usize, h: usize, y0: usize, rows: &mut [u8]) {
        let vig = self.vignette != 0.0;
        let cols: Vec<f32> = if vig {
            (0..w)
                .map(|x| {
                    let d = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                    d * d
                })
                .collect()
        } else {
            Vec::new()
        };
        for (k, row) in rows.chunks_exact_mut(w * 3).enumerate() {
            let dy = ((y0 + k) as f32 + 0.5) / h as f32 * 2.0 - 1.0;
            for (x, p) in row.chunks_exact_mut(3).enumerate() {
                let out = self.pixel([p[0], p[1], p[2]]);
                if vig {
                    let (dark, light) = self.vignette_at((cols[x] + dy * dy) * 0.5);
                    for (o, c) in p.iter_mut().zip(out) {
                        let c = i32::from(c);
                        *o = (((c * dark + 128) >> 8) + (((255 - c) * light + 128) >> 8)).clamp(0, 255) as u8;
                    }
                } else {
                    p.copy_from_slice(&out);
                }
            }
        }
    }
}

/// Unsharp masking: each pixel pushed away from its neighbours' blur (1 2 1
/// each way) by `amount` (256: once), rows `rows` of `src` into `out`.
pub fn sharpen_rows(src: &Picture, amount: i32, rows: Range<usize>, out: &mut [u8]) {
    let (w, h) = (src.w, src.h);
    let stride = w * 3;
    for (k, y) in rows.enumerate() {
        let row = |yy: usize| &src.rgb[yy * stride..(yy + 1) * stride];
        let (up, here, down) = (row(y.saturating_sub(1)), row(y), row((y + 1).min(h - 1)));
        let dst = &mut out[k * stride..(k + 1) * stride];
        // Down the columns first (1 2 1), then along the row.
        let mut col = vec![0i32; stride];
        for (i, c) in col.iter_mut().enumerate() {
            *c = i32::from(up[i]) + 2 * i32::from(here[i]) + i32::from(down[i]);
        }
        for x in 0..w {
            let (l, r) = (x.saturating_sub(1) * 3, (x + 1).min(w - 1) * 3);
            for c in 0..3 {
                let at = x * 3 + c;
                let blur = (col[l + c] + 2 * col[at] + col[r + c] + 8) >> 4;
                let v = i32::from(here[at]);
                dst[at] = (v + (((v - blur) * amount + 128) >> 8)).clamp(0, 255) as u8;
            }
        }
    }
}

/// A picture through the tone: point operations and vignette in place, then
/// sharpening into a fresh picture when asked for.
pub fn develop(mut pic: Picture, tone: &Tone, threads: usize) -> Picture {
    let (w, h) = (pic.w, pic.h);
    if pic.is_empty() {
        return pic;
    }
    by_rows(&mut pic.rgb, w * 3, threads, |y0, rows| tone.develop_rows(w, h, y0, rows));
    if tone.sharp > 0 {
        let mut sharp = Picture::new(w, h);
        by_rows(&mut sharp.rgb, w * 3, threads, |y0, rows| sharpen_rows(&pic, tone.sharp, y0..y0 + rows.len() / (w * 3), rows));
        return sharp;
    }
    pic
}

/// Whole rows of `out` (each `row_len` long) cut into `parts` runs, each
/// handed to `f` with its first row, on a thread of its own (one: here).
pub fn by_rows(out: &mut [u8], row_len: usize, parts: usize, f: impl Fn(usize, &mut [u8]) + Sync) {
    let rows = out.len() / row_len.max(1);
    let parts = parts.clamp(1, 8).min(rows.max(1));
    if parts <= 1 {
        f(0, out);
        return;
    }
    let per = rows.div_ceil(parts);
    std::thread::scope(|s| {
        for (i, chunk) in out.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            s.spawn(move || f(i * per, chunk));
        }
    });
}

/// The cores to work on: the handheld's four (or the computer's, up to that).
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4)
}
