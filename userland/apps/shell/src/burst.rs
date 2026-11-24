//! Detail mode: a burst of full-size frames merged into one photo, the way
//! phones' night and HDR+ modes do it, on the CM0's own cores.
//!
//! catalyst-camera takes the burst (the first frame settled, the rest with
//! its exposure and white balance held), writes the first as the photo at
//! once, and hands every frame (YUV 4:2:0, packed planes, 12 MB each at
//! 3280 × 2464) to `catalyst-shell --merge-burst` in memory files. That runs
//! at idle priority (SCHED_IDLE, nice 19, idle I/O: nothing waits on it, and
//! the music player at nice −10 is never held up), on three of the four
//! cores, and replaces the photo when it is done. While it runs, a marker
//! beside the photo (`IMG_….jpg.refining`) says when it began and how long
//! it should take, for the Camera and the album's "refining" badge.
//!
//! The merge:
//! - **Alignment.** Each frame against the first on a pyramid of its
//!   brightness: one shift for the whole picture at an eighth of the size,
//!   then tiles at a quarter and a half, then 32-pixel tiles at full size,
//!   each searched about twice what the level above found.
//! - **Rejection.** A tile that still differs from the first frame by much
//!   more than the noise does (something moved) counts for nothing, and
//!   within the tiles kept each pixel counts less the further it is from the
//!   first frame's. Nothing that moved is ever smeared: the first frame
//!   stands there alone. The weights are blended between tile centres, so
//!   no tile's edge shows.
//! - **Merge.** A weighted average, the first frame at full weight: in still
//!   parts of the picture the noise falls by about the square root of the
//!   number of frames.
//! - **Finish.** Gentle local tone mapping (the shadows lifted on a heavily
//!   blurred base, the detail on it kept) and gentle sharpening (a small
//!   unsharp mask that leaves the noise floor alone). Inside the faces the
//!   smart camera saw, both are softer and the colour is left as merged, so
//!   skin keeps its tone.
//!
//! Memory: the frames (6 × 12 MB, in the camera's memory files), the merged
//! picture (12 MB), a copy of its brightness for sharpening (8 MB) and each
//! frame's pyramid (2.7 MB): about 110 MB at the peak, given back when the
//! process ends.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Settings › Camera › Detail mode. Since 1.5.2 how many frames every
/// photo merges in the camera program (camera/finish.rs): auto three, on
/// six, off one. The background burst below is no longer asked for by the
/// Camera; `--merge-burst` still merges one handed to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailMode {
    /// Three frames a photo.
    Auto,
    On,
    Off,
}

impl DetailMode {
    pub const ALL: [DetailMode; 3] = [DetailMode::Auto, DetailMode::On, DetailMode::Off];

    pub fn name(self) -> &'static str {
        match self {
            DetailMode::Auto => "auto",
            DetailMode::On => "on",
            DetailMode::Off => "off",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }

    pub fn step(self, step: i32) -> Self {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as i32;
        Self::ALL[(at + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    /// As Settings says it.
    pub fn words(self) -> &'static str {
        match self {
            DetailMode::Auto => "Auto (3 frames)",
            DetailMode::On => "On (6 frames)",
            DetailMode::Off => "Off (1 frame)",
        }
    }
}

/// The alignment's tile at full size.
pub const TILE: usize = 32;
/// Weights are in 64ths.
const ONE: u32 = 64;

/// One plane of bytes, `w` × `h`, its rows packed.
#[derive(Clone, Debug, Default)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Self {
        Plane { w, h, px: vec![0; w * h] }
    }

    fn from(w: usize, h: usize, px: &[u8]) -> Self {
        Plane { w, h, px: px.to_vec() }
    }

    /// Half the size each way, each pixel the mean of four.
    pub fn half(&self) -> Plane {
        let (w, h) = (self.w / 2, self.h / 2);
        let mut out = Plane::new(w, h);
        for y in 0..h {
            let (a, b) = (&self.px[2 * y * self.w..], &self.px[(2 * y + 1) * self.w..]);
            let row = &mut out.px[y * w..(y + 1) * w];
            for (x, o) in row.iter_mut().enumerate() {
                let s = u32::from(a[2 * x]) + u32::from(a[2 * x + 1]) + u32::from(b[2 * x]) + u32::from(b[2 * x + 1]);
                *o = ((s + 2) / 4) as u8;
            }
        }
        out
    }
}

/// The mean absolute difference (in 16ths of a level) between `a`'s square
/// at (x0, y0) and `b`'s at (x0 + dx, y0 + dy), every `step` pixels, over
/// whatever of it both hold; `u32::MAX` where they share none.
#[allow(clippy::too_many_arguments)]
fn sad(a: &Plane, b: &Plane, x0: usize, y0: usize, size_w: usize, size_h: usize, dx: i32, dy: i32, step: usize) -> u32 {
    let xs = (x0 as i64).max(-(dx as i64)).max(0) as usize;
    let xe = ((x0 + size_w).min(a.w) as i64).min(b.w as i64 - dx as i64).max(0) as usize;
    if xe <= xs {
        return u32::MAX;
    }
    let (mut sum, mut n) = (0u64, 0u64);
    let mut y = y0;
    while y < (y0 + size_h).min(a.h) {
        let by = y as i64 + dy as i64;
        if by >= 0 && (by as usize) < b.h {
            let ra = &a.px[y * a.w + xs..y * a.w + xe];
            let start = (by as usize) * b.w + (xs as i64 + dx as i64) as usize;
            let rb = &b.px[start..start + (xe - xs)];
            if step == 1 {
                sum += ra.iter().zip(rb).map(|(p, q)| u64::from(p.abs_diff(*q))).sum::<u64>();
                n += ra.len() as u64;
            } else {
                sum += ra.iter().step_by(step).zip(rb.iter().step_by(step)).map(|(p, q)| u64::from(p.abs_diff(*q))).sum::<u64>();
                n += ra.len().div_ceil(step) as u64;
            }
        }
        y += step;
    }
    if n == 0 {
        u32::MAX
    } else {
        (sum * 16 / n) as u32
    }
}

/// A picture's brightness at a half, a quarter and an eighth of its size.
pub struct Pyramid {
    pub levels: Vec<Plane>,
}

impl Pyramid {
    pub fn of(full: &Plane) -> Pyramid {
        let mut levels = vec![full.half()];
        for _ in 0..2 {
            let next = levels.last().map(Plane::half).unwrap_or_default();
            levels.push(next);
        }
        Pyramid { levels }
    }
}

/// Where each full-size tile of a frame lies against the first: its shift
/// and what still differs there once shifted (in 16ths of a level).
#[derive(Clone, Debug, Default)]
pub struct Field {
    pub cols: usize,
    pub rows: usize,
    pub d: Vec<(i32, i32)>,
    pub cost: Vec<u32>,
}

impl Field {
    pub fn at(&self, tx: usize, ty: usize) -> (i32, i32) {
        self.d[ty.min(self.rows - 1) * self.cols + tx.min(self.cols - 1)]
    }
}

/// The best of the shifts within `r` of each of `around` for a square.
#[allow(clippy::too_many_arguments)]
fn search(a: &Plane, b: &Plane, x0: usize, y0: usize, size: usize, around: &[(i32, i32)], r: i32, step: usize) -> ((i32, i32), u32) {
    let mut best: ((i32, i32), u32) = ((0, 0), u32::MAX);
    for &(cx, cy) in around {
        for dy in -r..=r {
            for dx in -r..=r {
                let d = (cx + dx, cy + dy);
                let c = sad(a, b, x0, y0, size, size, d.0, d.1, step);
                // Ties go to the smaller shift (a flat tile stays put).
                if c < best.1 || (c == best.1 && d.0.abs() + d.1.abs() < best.0 .0.abs() + best.0 .1.abs()) {
                    best = (d, c);
                }
            }
        }
    }
    best
}

/// `alt` aligned against `reference`, coarse to fine (see the notes above).
pub fn align(reference: &Plane, rp: &Pyramid, alt: &Plane, ap: &Pyramid) -> Field {
    // An eighth of the size: one shift for the whole picture, within 8
    // pixels (64 at full size), on its middle.
    let (r3, a3) = (&rp.levels[2], &ap.levels[2]);
    let m = 8usize.min(r3.w / 4).min(r3.h / 4);
    let mut global = ((0, 0), u32::MAX);
    for dy in -8..=8 {
        for dx in -8..=8 {
            let c = sad(r3, a3, m, m, r3.w - 2 * m, r3.h - 2 * m, dx, dy, 1);
            if c < global.1 {
                global = ((dx, dy), c);
            }
        }
    }
    // Tiles at a quarter (16 there, 64 at full size), then a half (16
    // there: the full-size grid), each about twice the shift above; at a
    // half the tile's own parent and the two nearest others are tried.
    let level = |plane: &Plane, size: usize| (plane.w.div_ceil(size), plane.h.div_ceil(size));
    let (r2, a2) = (&rp.levels[1], &ap.levels[1]);
    let (c2, w2) = level(r2, 16);
    let mut d2 = vec![(0, 0); c2 * w2];
    for ty in 0..w2 {
        for tx in 0..c2 {
            // Twice the whole picture's shift, or none (a tile that holds
            // still while the rest moves).
            let g = (global.0 .0 * 2, global.0 .1 * 2);
            d2[ty * c2 + tx] = search(r2, a2, tx * 16, ty * 16, 16, &[g, (0, 0)], 3, 1).0;
        }
    }
    let (r1, a1) = (&rp.levels[0], &ap.levels[0]);
    let (cols, rows) = (reference.w.div_ceil(TILE), reference.h.div_ceil(TILE));
    let mut d1 = vec![(0, 0); cols * rows];
    for ty in 0..rows {
        for tx in 0..cols {
            // The parent at a quarter, and the neighbours on this tile's side of it.
            let (px, py) = (tx / 2, ty / 2);
            let (sx, sy) = (if tx % 2 == 0 { -1i64 } else { 1 }, if ty % 2 == 0 { -1i64 } else { 1 });
            let pick = |x: i64, y: i64| d2[(y.clamp(0, w2 as i64 - 1) as usize) * c2 + x.clamp(0, c2 as i64 - 1) as usize];
            let around = [pick(px as i64, py as i64), pick(px as i64 + sx, py as i64), pick(px as i64, py as i64 + sy), (global.0 .0 * 2, global.0 .1 * 2)].map(|(x, y)| (x * 2, y * 2));
            d1[ty * cols + tx] = search(r1, a1, tx * 16, ty * 16, 16, &around, 1, 1).0;
        }
    }
    // Full size: 32-pixel tiles, within two pixels of twice the half's.
    let mut field = Field { cols, rows, d: vec![(0, 0); cols * rows], cost: vec![0; cols * rows] };
    for ty in 0..rows {
        for tx in 0..cols {
            let up = d1[ty * cols + tx];
            let (d, c) = search(reference, alt, tx * TILE, ty * TILE, TILE, &[(up.0 * 2, up.1 * 2)], 2, 2);
            field.d[ty * cols + tx] = d;
            field.cost[ty * cols + tx] = c;
        }
    }
    field
}

/// The noise as the tiles' differences show it: their median (most of a
/// picture holds still), in 16ths of a level, never under one level.
pub fn noise_of(fields: &[Field]) -> u32 {
    let mut all: Vec<u32> = fields.iter().flat_map(|f| f.cost.iter().copied()).filter(|c| *c != u32::MAX).collect();
    if all.is_empty() {
        return 16;
    }
    let mid = all.len() / 2;
    all.select_nth_unstable(mid);
    all[mid].max(16)
}

/// Each tile's weight (in 64ths): whole while it differs no more than noise
/// does, nothing once it differs by twice that (it moved).
pub fn tile_weights(field: &Field, noise: u32) -> Vec<u32> {
    let (lo, hi) = (noise * 8 / 5, noise * 16 / 5);
    field.cost.iter().map(|&c| if c <= lo { ONE } else if c >= hi { 0 } else { (hi - c) * ONE / (hi - lo) }).collect()
}

/// A pixel's weight (64ths) by how far it is from the first frame's.
fn pixel_table(noise: u32, scale: f32) -> [u8; 256] {
    let n = noise as f32 / 16.0 * scale;
    let (t1, t2) = ((2.5 * n).max(3.0), (5.0 * n).max(6.0));
    let mut table = [0u8; 256];
    for (d, w) in table.iter_mut().enumerate() {
        let d = d as f32;
        *w = if d <= t1 { ONE as u8 } else if d >= t2 { 0 } else { ((t2 - d) / (t2 - t1) * ONE as f32) as u8 };
    }
    table
}

/// A tile weight blended between tile centres, at full-size pixel (x, y).
fn blended(weights: &[u32], field: &Field, x: usize, y: usize) -> u32 {
    let fx = (x as f32 + 0.5) / TILE as f32 - 0.5;
    let fy = (y as f32 + 0.5) / TILE as f32 - 0.5;
    let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
    let (x1, y1) = ((x0 + 1).min(field.cols - 1), (y0 + 1).min(field.rows - 1));
    let (ax, ay) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
    let w = |tx: usize, ty: usize| weights[ty * field.cols + tx] as f32;
    let top = w(x0, y0) * (1.0 - ax) + w(x1, y0) * ax;
    let bottom = w(x0, y1) * (1.0 - ax) + w(x1, y1) * ax;
    (top * (1.0 - ay) + bottom * ay) as u32
}

/// A face, as a box in the picture (0 to 1 each way).
pub type Face = [f32; 4];

fn in_face(faces: &[Face], u: f32, v: f32) -> bool {
    faces.iter().any(|f| u >= f[0].min(f[2]) && u <= f[0].max(f[2]) && v >= f[1].min(f[3]) && v <= f[1].max(f[3]))
}

/// What the merge was given and how it went.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub frames: usize,
    /// The share of the other frames' pixels that counted, 0 to 1.
    pub kept: f32,
    pub noise: f32,
    pub align_ms: u128,
    pub merge_ms: u128,
    pub finish_ms: u128,
}

/// Rows `0..h` cut into `parts` runs, each handed to `f` on a thread of its
/// own with the rows it owns.
fn by_rows<T: Send>(out: &mut [T], w: usize, h: usize, parts: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let parts = parts.clamp(1, 8);
    let per = h.div_ceil(parts).max(1);
    std::thread::scope(|s| {
        for (i, chunk) in out.chunks_mut(per * w).enumerate() {
            let f = &f;
            s.spawn(move || f(i * per, chunk));
        }
    });
}

/// The frames (packed YUV 4:2:0, `w` × `h`, the first the one the others are
/// merged onto) merged into one, finished: see the notes at the top.
pub fn merge(frames: &[&[u8]], w: usize, h: usize, faces: &[Face], threads: usize, progress: &mut dyn FnMut(f32)) -> (Vec<u8>, Report) {
    let (cw, ch) = (w / 2, h / 2);
    let ysize = w * h;
    let csize = cw * ch;
    let mut report = Report { frames: frames.len(), ..Report::default() };
    let began = Instant::now();
    let reference = Plane::from(w, h, &frames[0][..ysize]);
    let rp = Pyramid::of(&reference);
    // The frames aligned a few at a time, one to a thread.
    let mut fields: Vec<Field> = Vec::new();
    for batch in frames[1..].chunks(threads.clamp(1, 8)) {
        let done: Vec<Field> = std::thread::scope(|s| {
            let jobs: Vec<_> = batch
                .iter()
                .map(|frame| {
                    let (reference, rp) = (&reference, &rp);
                    s.spawn(move || {
                        let alt = Plane::from(w, h, &frame[..ysize]);
                        let ap = Pyramid::of(&alt);
                        align(reference, rp, &alt, &ap)
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().expect("a frame's alignment")).collect()
        });
        fields.extend(done);
        progress(0.4 * fields.len() as f32 / frames.len().max(2) as f32);
    }
    drop(rp);
    let noise = noise_of(&fields);
    report.noise = noise as f32 / 16.0;
    let tiles: Vec<Vec<u32>> = fields.iter().map(|f| tile_weights(f, noise)).collect();
    let ytable = pixel_table(noise, 1.0);
    let ctable = pixel_table(noise, 0.8);
    report.align_ms = began.elapsed().as_millis();
    progress(0.45);

    // The merge, rows split among the threads.
    let began = Instant::now();
    let mut out = vec![0u8; ysize + 2 * csize];
    let kept_sum = std::sync::atomic::AtomicU64::new(0);
    {
        let (yplane, chroma) = out.split_at_mut(ysize);
        by_rows(yplane, w, h, threads, |first, rows| {
            let mut acc = vec![0u32; w];
            let mut wsum = vec![0u32; w];
            let mut kept = 0u64;
            for (r, row) in rows.chunks_mut(w).enumerate() {
                let y = first + r;
                let base = &frames[0][y * w..(y + 1) * w];
                for x in 0..w {
                    acc[x] = u32::from(base[x]) * ONE;
                    wsum[x] = ONE;
                }
                for (i, field) in fields.iter().enumerate() {
                    let alt = &frames[i + 1][..ysize];
                    let ty = y / TILE;
                    for x in 0..w {
                        let (dx, dy) = field.at(x / TILE, ty);
                        let sx = (x as i64 + dx as i64).clamp(0, w as i64 - 1) as usize;
                        let sy = (y as i64 + dy as i64).clamp(0, h as i64 - 1) as usize;
                        let a = alt[sy * w + sx];
                        let wp = u32::from(ytable[usize::from(a.abs_diff(base[x]))]);
                        let wt = if wp == 0 { 0 } else { blended(&tiles[i], field, x, y) * wp / ONE };
                        acc[x] += wt * u32::from(a);
                        wsum[x] += wt;
                        kept += u64::from(wt);
                    }
                }
                for x in 0..w {
                    row[x] = ((acc[x] + wsum[x] / 2) / wsum[x]) as u8;
                }
            }
            kept_sum.fetch_add(kept, std::sync::atomic::Ordering::Relaxed);
        });
        // Chroma: each frame's tile shift halved, weighed by both planes.
        let (uplane, vplane) = chroma.split_at_mut(csize);
        let merge_chroma = |plane_at: usize, out: &mut [u8]| {
            by_rows(out, cw, ch, threads, |first, rows| {
                for (r, row) in rows.chunks_mut(cw).enumerate() {
                    let y = first + r;
                    let at = |f: &[u8], plane: usize, x: usize, y: usize| f[ysize + plane * csize + y * cw + x];
                    for x in 0..cw {
                        let (ru, rv) = (at(frames[0], 0, x, y), at(frames[0], 1, x, y));
                        let me = at(frames[0], plane_at, x, y);
                        let (mut acc, mut wsum) = (u32::from(me) * ONE, ONE);
                        for (i, field) in fields.iter().enumerate() {
                            let (dx, dy) = field.at(2 * x / TILE, 2 * y / TILE);
                            let sx = (x as i64 + (dx as f32 / 2.0).round() as i64).clamp(0, cw as i64 - 1) as usize;
                            let sy = (y as i64 + (dy as f32 / 2.0).round() as i64).clamp(0, ch as i64 - 1) as usize;
                            let (au, av) = (at(frames[i + 1], 0, sx, sy), at(frames[i + 1], 1, sx, sy));
                            let d = (u32::from(au.abs_diff(ru)) + u32::from(av.abs_diff(rv))).min(255) as usize;
                            let wp = u32::from(ctable[d]);
                            let wt = if wp == 0 { 0 } else { blended(&tiles[i], field, 2 * x, 2 * y) * wp / ONE };
                            acc += wt * u32::from(at(frames[i + 1], plane_at, sx, sy));
                            wsum += wt;
                        }
                        row[x] = ((acc + wsum / 2) / wsum) as u8;
                    }
                }
            });
        };
        merge_chroma(0, uplane);
        merge_chroma(1, vplane);
    }
    let others = (frames.len().saturating_sub(1) * ysize) as f64 * f64::from(ONE);
    report.kept = if others > 0.0 { (kept_sum.into_inner() as f64 / others) as f32 } else { 0.0 };
    report.merge_ms = began.elapsed().as_millis();
    progress(0.8);

    let began = Instant::now();
    finish(&mut out, w, h, faces, threads, frames.len());
    report.finish_ms = began.elapsed().as_millis();
    progress(0.9);
    (out, report)
}

/// The finish on a merged picture, in place: local tone mapping, then
/// sharpening; both softer inside faces, and their colour left as it is.
pub fn finish(yuv: &mut [u8], w: usize, h: usize, faces: &[Face], threads: usize, frames: usize) {
    let (cw, ch) = (w / 2, h / 2);
    let ysize = w * h;
    // The base: the brightness at a sixteenth, spread back out smoothly.
    let mut base = Plane::from(w, h, &yuv[..ysize]);
    for _ in 0..4 {
        base = base.half();
    }
    let mean = base.px.iter().map(|p| u64::from(*p)).sum::<u64>() as f32 / base.px.len().max(1) as f32 / 255.0;
    // The shadows lifted more in a dim picture (and with more frames merged,
    // which leaves less noise to lift).
    let lift = if mean < 0.3 { 0.22 } else { 0.12 } * (frames as f32 / 6.0).clamp(0.5, 1.0);
    let at_base = |x: usize, y: usize| -> f32 {
        let fx = ((x as f32 + 0.5) / 16.0 - 0.5).clamp(0.0, (base.w - 1) as f32);
        let fy = ((y as f32 + 0.5) / 16.0 - 0.5).clamp(0.0, (base.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(base.w - 1), (y0 + 1).min(base.h - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x: usize, y: usize| f32::from(base.px[y * base.w + x]);
        ((p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax) * (1.0 - ay) + (p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax) * ay) / 255.0
    };
    // The tone curve's lift at a base brightness: most in the deep shadows,
    // none at black or white.
    let curve = |b: f32, strength: f32| strength * b * (1.0 - b) * (1.0 - b) * 2.0;
    let face_at = |x: usize, y: usize| !faces.is_empty() && in_face(faces, x as f32 / w as f32, y as f32 / h as f32);
    {
        let (yplane, chroma) = yuv.split_at_mut(ysize);
        by_rows(yplane, w, h, threads, |first, rows| {
            for (r, row) in rows.chunks_mut(w).enumerate() {
                let y = first + r;
                for (x, p) in row.iter_mut().enumerate() {
                    let s = if face_at(x, y) { lift * 0.5 } else { lift };
                    let add = curve(at_base(x, y), s) * 255.0;
                    *p = (f32::from(*p) + add).round().clamp(0.0, 255.0) as u8;
                }
            }
        });
        // Colour kept as saturated as the brightness it now sits at (outside
        // faces, whose colour stays as merged).
        let (u, v) = chroma.split_at_mut(cw * ch);
        for plane in [u, v] {
            by_rows(plane, cw, ch, threads, |first, rows| {
                for (r, row) in rows.chunks_mut(cw).enumerate() {
                    let y = (first + r) * 2;
                    for (x, p) in row.iter_mut().enumerate() {
                        if face_at(2 * x, y) {
                            continue;
                        }
                        let b = at_base(2 * x, y);
                        let k = 1.0 + 0.5 * curve(b, lift) / b.max(0.08);
                        *p = (128.0 + (f32::from(*p) - 128.0) * k).round().clamp(0.0, 255.0) as u8;
                    }
                }
            });
        }
    }
    // Sharpening: a 3 × 3 unsharp mask, its smallest differences (the noise
    // floor) left alone.
    let src = yuv[..ysize].to_vec();
    let yplane = &mut yuv[..ysize];
    by_rows(yplane, w, h, threads, |first, rows| {
        for (r, row) in rows.chunks_mut(w).enumerate() {
            let y = first + r;
            if y == 0 || y + 1 >= h {
                continue;
            }
            let (up, me, down) = (&src[(y - 1) * w..y * w], &src[y * w..(y + 1) * w], &src[(y + 1) * w..(y + 2) * w]);
            for x in 1..w - 1 {
                let blur = (u32::from(up[x - 1]) + 2 * u32::from(up[x]) + u32::from(up[x + 1]) + 2 * u32::from(me[x - 1]) + 4 * u32::from(me[x]) + 2 * u32::from(me[x + 1]) + u32::from(down[x - 1]) + 2 * u32::from(down[x]) + u32::from(down[x + 1]) + 8) / 16;
                let detail = i32::from(me[x]) - blur as i32;
                let soft = detail.signum() * (detail.abs() - 2).max(0);
                let amount = if face_at(x, y) { 0.2 } else { 0.55 };
                let lifted = i32::from(row[x]) + (soft as f32 * amount).round() as i32;
                row[x] = lifted.clamp(0, 255) as u8;
            }
        }
    });
}

// ------------------------------------------------------------ the photo ---

/// A JPEG's Exif (its APP1 segment's contents), to carry onto the merged one.
pub fn exif_of(jpeg: &[u8]) -> Option<Vec<u8>> {
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if marker == 0xE1 && jpeg.get(at + 4..at + 10) == Some(b"Exif\0\0") {
            return jpeg.get(at + 4..at + 2 + len).map(<[u8]>::to_vec);
        }
        if marker == 0xDA {
            break;
        }
        at += 2 + len;
    }
    None
}

/// Planar YUV 4:2:0 as the encoder reads it, a row at a time.
struct Yuv<'a> {
    px: &'a [u8],
    w: usize,
    h: usize,
}

impl jpeg_encoder::ImageBuffer for Yuv<'_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        jpeg_encoder::JpegColorType::Ycbcr
    }
    fn width(&self) -> u16 {
        self.w as u16
    }
    fn height(&self) -> u16 {
        self.h as u16
    }
    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let (w, y) = (self.w, usize::from(y));
        let (cw, ch) = (w / 2, self.h / 2);
        let cy = (y / 2).min(ch - 1);
        buffers[0].extend_from_slice(&self.px[y * w..(y + 1) * w]);
        let u = &self.px[w * self.h + cy * cw..][..cw];
        let v = &self.px[w * self.h + cw * ch + cy * cw..][..cw];
        for x in 0..w {
            buffers[1].push(u[(x / 2).min(cw - 1)]);
            buffers[2].push(v[(x / 2).min(cw - 1)]);
        }
    }
}

/// The merged picture written as `path` (beside it, renamed into place):
/// quality 95, 4:2:0 as the camera's own, the first frame's Exif.
pub fn write_jpeg(path: &Path, yuv: &[u8], w: usize, h: usize, exif: Option<Vec<u8>>) -> Result<(), String> {
    let part = path.with_extension("jpg.merge");
    let file = std::fs::File::create(&part).map_err(|e| format!("the photo cannot be written: {e}"))?;
    let mut encoder = jpeg_encoder::Encoder::new(std::io::BufWriter::new(file), 95);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    if let Some(exif) = exif {
        encoder.add_app_segment(1, exif).map_err(|e| format!("the photo's details: {e}"))?;
    }
    encoder.encode_image(Yuv { px: yuv, w, h }).map_err(|e| format!("the photo could not be made: {e}"))?;
    std::fs::rename(&part, path).map_err(|e| format!("the photo could not be kept: {e}"))
}

// ----------------------------------------------------- the marker, timing ---

/// The marker beside a photo being refined.
pub fn marker(photo: &Path) -> PathBuf {
    let mut name = photo.as_os_str().to_owned();
    name.push(".refining");
    PathBuf::from(name)
}

/// A photo being refined: when it began, how long it should take (ms), and
/// how far it has come.
#[derive(Clone, Debug, PartialEq)]
pub struct Refining {
    pub began_ms: u64,
    pub expect_ms: u64,
    pub progress: f32,
}

impl Refining {
    /// What is left, in whole seconds (at least one while it runs).
    pub fn left_s(&self, now_ms: u64) -> u64 {
        let done = now_ms.saturating_sub(self.began_ms);
        let by_progress = if self.progress > 0.15 { (done as f32 / self.progress) as u64 } else { self.expect_ms };
        let total = (by_progress + self.expect_ms) / 2;
        total.saturating_sub(done).div_ceil(1000).max(1)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn read_marker(photo: &Path) -> Option<Refining> {
    let text = std::fs::read_to_string(marker(photo)).ok()?;
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=').map(str::trim).map(str::to_owned));
    Some(Refining {
        began_ms: field("began")?.parse().ok()?,
        expect_ms: field("expect").and_then(|v| v.parse().ok()).unwrap_or(8000),
        progress: field("progress").and_then(|v| v.parse().ok()).unwrap_or(0.0),
    })
}

fn write_marker(photo: &Path, r: &Refining) {
    let text = format!("began={}\nexpect={}\nprogress={:.2}\n", r.began_ms, r.expect_ms, r.progress);
    let path = marker(photo);
    let part = path.with_extension("refining.part");
    if std::fs::write(&part, text).is_ok() {
        std::fs::rename(part, path).ok();
    }
}

/// How long a merge should take, from how long the last took (per frame and
/// megapixel), kept beside the photos' settings.
fn speed_file() -> PathBuf {
    crate::photos::dir().join(".detail-speed")
}

pub fn expect_ms(frames: usize, w: usize, h: usize) -> u64 {
    let mp = (w * h) as f64 / 1e6;
    let per = std::fs::read_to_string(speed_file()).ok().and_then(|s| s.trim().parse::<f64>().ok());
    // My estimate for the CM0 before any is measured: about 110 ms a frame
    // and megapixel to align and merge, and 250 ms a megapixel to finish
    // and encode, at idle priority on three cores.
    let (per_frame, fixed) = match per {
        Some(per) => (per, 0.0),
        None if cfg!(target_arch = "aarch64") => (110.0, 250.0 * mp),
        None => (25.0, 60.0 * mp),
    };
    (per_frame * mp * frames as f64 + fixed) as u64
}

/// Refine `photo` from `frames`: the marker written while it runs, the
/// merged photo put in its place, the time kept for the next estimate. For
/// a photo wearing a look (filters.rs), the merged frame is written first
/// as the photo as taken (`original`), then with the look's table over it
/// as the photo.
#[allow(clippy::too_many_arguments)]
pub fn refine_with(photo: &Path, frames: &[&[u8]], w: usize, h: usize, faces: &[Face], threads: usize, look: Option<&[u8]>, original: Option<&Path>) -> Result<Report, String> {
    let began = Instant::now();
    let mut marking = Refining { began_ms: now_ms(), expect_ms: expect_ms(frames.len(), w, h), progress: 0.0 };
    write_marker(photo, &marking);
    let result = (|| {
        let exif = std::fs::read(photo).ok().and_then(|j| exif_of(&j));
        let mut last = Instant::now();
        let (mut merged, report) = merge(frames, w, h, faces, threads, &mut |p| {
            if last.elapsed().as_millis() > 400 {
                marking.progress = p;
                write_marker(photo, &marking);
                last = Instant::now();
            }
        });
        if !photo.exists() {
            return Err("the photo was deleted while it was refined".to_owned());
        }
        if let Some(table) = look {
            if let Some(original) = original.filter(|o| o.exists()) {
                write_jpeg(original, &merged, w, h, exif.clone())?;
            }
            crate::filters::apply_yuv420(table, &mut merged, w, h);
        }
        write_jpeg(photo, &merged, w, h, exif)?;
        Ok(report)
    })();
    std::fs::remove_file(marker(photo)).ok();
    if result.is_ok() {
        let mp = (w * h) as f64 / 1e6;
        let per = began.elapsed().as_millis() as f64 / (mp * frames.len() as f64);
        std::fs::write(speed_file(), format!("{per:.1}\n")).ok();
    }
    result
}

/// `catalyst-shell --merge-burst PHOTO W H [face X0 Y0 X1 Y1]... -- FRAME...`:
/// each FRAME a file, or `fd:N` (a memory file catalyst-camera handed down).
/// Says how it went on standard output, one line.
pub fn run(args: &[String]) -> i32 {
    let Some(at) = args.iter().position(|a| a == "--merge-burst") else { return 2 };
    let rest = &args[at + 1..];
    let split = rest.iter().position(|a| a == "--").unwrap_or(rest.len());
    let (head, sources) = (&rest[..split], rest.get(split + 1..).unwrap_or(&[]));
    let (Some(photo), Some(w), Some(h)) = (head.first(), head.get(1).and_then(|v| v.parse::<usize>().ok()), head.get(2).and_then(|v| v.parse::<usize>().ok())) else {
        eprintln!("merge-burst: PHOTO W H [face X0 Y0 X1 Y1]... -- FRAME...");
        return 2;
    };
    let mut faces = Vec::new();
    let (mut look, mut original) = (None, None);
    let mut words = head[3..].iter();
    while let Some(word) = words.next() {
        if word == "face" {
            let v: Vec<f32> = words.by_ref().take(4).filter_map(|v| v.parse().ok()).collect();
            if v.len() == 4 {
                faces.push([v[0], v[1], v[2], v[3]]);
            }
        } else if word == "look" {
            // The look the photo wears (its table's file), laid over the merge too.
            look = words.next().and_then(|f| std::fs::read(f).ok()).and_then(|b| crate::filters::read_lut_file(&b));
        } else if word == "orig" {
            original = words.next().map(PathBuf::from);
        }
    }
    let size = w * h * 3 / 2;
    let mut held = Vec::new();
    for source in sources {
        match load(source, size) {
            Ok(frame) => held.push(frame),
            Err(why) => eprintln!("merge-burst: {source}: {why}"),
        }
    }
    let photo = PathBuf::from(photo);
    if held.len() < 2 {
        std::fs::remove_file(marker(&photo)).ok();
        println!("not enough frames to merge");
        return 1;
    }
    let frames: Vec<&[u8]> = held.iter().map(|f| f.bytes()).collect();
    match refine_with(&photo, &frames, w, h, &faces, 3, look.as_deref(), original.as_deref()) {
        Ok(r) => {
            println!("merged {} frames ({:.0}% kept, noise {:.1}) in {} + {} + {} ms", r.frames, r.kept * 100.0, r.noise, r.align_ms, r.merge_ms, r.finish_ms);
            0
        }
        Err(why) => {
            println!("{why}");
            1
        }
    }
}

/// A frame held for the merge: mapped from a memory file, or read.
enum Held {
    Read(Vec<u8>),
    #[cfg(target_os = "linux")]
    Mapped(*const u8, usize),
}

impl Held {
    fn bytes(&self) -> &[u8] {
        match self {
            Held::Read(v) => v,
            // SAFETY: mapped read-only for `len` bytes until dropped.
            #[cfg(target_os = "linux")]
            Held::Mapped(p, len) => unsafe { std::slice::from_raw_parts(*p, *len) },
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Held {
    fn drop(&mut self) {
        if let Held::Mapped(p, len) = self {
            // SAFETY: the mapping made in `load`, unmapped once.
            unsafe { libc::munmap(*p as *mut libc::c_void, *len) };
        }
    }
}

fn load(source: &str, size: usize) -> Result<Held, String> {
    #[cfg(target_os = "linux")]
    if let Some(fd) = source.strip_prefix("fd:").and_then(|n| n.parse::<i32>().ok()) {
        // SAFETY: a file descriptor handed down to this process; mapped
        // read-only and closed (the mapping keeps its pages).
        unsafe {
            let p = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd, 0);
            libc::close(fd);
            if p == libc::MAP_FAILED {
                return Err("the frame cannot be mapped".into());
            }
            return Ok(Held::Mapped(p as *const u8, size));
        }
    }
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    if bytes.len() < size {
        return Err(format!("{} bytes, not {size}", bytes.len()));
    }
    std::fs::remove_file(source).ok();
    Ok(Held::Read(bytes))
}

// ---------------------------------------------------------------- tests ---

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A scene as a photo has one, `w` × `h`, shifted by (sx, sy) (what a
    /// camera sees moved that far): large shapes and shading, edges, and
    /// fine texture on them.
    pub fn scene(w: usize, h: usize, sx: i32, sy: i32) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 - sx as f32, y as f32 - sy as f32);
                let shade = 70.0 + 60.0 * ((u * 0.013).sin() + (v * 0.017 + 1.0).cos()) * 0.5;
                let block = if ((u / 61.0).floor() as i32 + (v / 47.0).floor() as i32).rem_euclid(3) == 0 { 55.0 } else { 0.0 };
                let disc = if ((u - 0.6 * w as f32).powi(2) + (v - 0.4 * h as f32).powi(2)).sqrt() < 0.12 * w as f32 { 50.0 } else { 0.0 };
                // Fine detail that repeats nowhere (a hash of the place).
                let mut k = (u as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (v as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
                k ^= k >> 29;
                k = k.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                let texture = ((k >> 59) as f32 - 16.0) * 1.2 + 8.0 * ((u * 0.05 + v * 0.031).sin());
                p.px[y * w + x] = (shade + block + disc + texture).clamp(0.0, 255.0) as u8;
            }
        }
        p
    }

    /// A small noise source that needs no crate: xorshift, roughly normal by
    /// summing four uniforms.
    pub struct Noise(u64);
    impl Noise {
        pub fn new(seed: u64) -> Self {
            Noise(seed.max(1))
        }
        fn uniform(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
        pub fn normal(&mut self) -> f32 {
            (self.uniform() + self.uniform() + self.uniform() + self.uniform() - 2.0) * 1.732
        }
    }

    fn noisy(p: &Plane, sigma: f32, noise: &mut Noise) -> Plane {
        let mut out = p.clone();
        for v in &mut out.px {
            *v = (f32::from(*v) + noise.normal() * sigma).round().clamp(0.0, 255.0) as u8;
        }
        out
    }

    /// A frame (packed 4:2:0) from a brightness plane, its colour flat grey.
    fn frame(y: &Plane) -> Vec<u8> {
        let mut f = y.px.clone();
        f.resize(y.w * y.h * 3 / 2, 128);
        f
    }

    fn rms(a: &[u8], b: &[u8], w: usize, margin: usize) -> f32 {
        let h = a.len() / w;
        let (mut sum, mut n) = (0.0f64, 0usize);
        for y in margin..h - margin {
            for x in margin..w - margin {
                let d = f64::from(a[y * w + x]) - f64::from(b[y * w + x]);
                sum += d * d;
                n += 1;
            }
        }
        (sum / n as f64).sqrt() as f32
    }

    #[test]
    fn a_known_shift_is_recovered_coarse_to_fine() {
        let (w, h) = (640, 480);
        let reference = scene(w, h, 0, 0);
        let mut noise = Noise::new(7);
        for (sx, sy) in [(0, 0), (5, -3), (-17, 9), (23, 14), (-40, -30)] {
            let alt = noisy(&scene(w, h, sx, sy), 4.0, &mut noise);
            let field = align(&reference, &Pyramid::of(&reference), &alt, &Pyramid::of(&alt));
            // The tiles inside the picture all find it (the edge ones may
            // look past the edge, where nothing matches).
            let mut right = 0;
            let mut inside = 0;
            for ty in 1..field.rows - 1 {
                for tx in 1..field.cols - 1 {
                    inside += 1;
                    right += usize::from(field.at(tx, ty) == (sx, sy));
                }
            }
            assert!(right * 10 >= inside * 9, "shift ({sx}, {sy}): {right} of {inside} tiles right ({:?})", &field.d[..8]);
        }
    }

    #[test]
    fn a_tile_that_moved_is_left_out_and_nothing_ghosts() {
        let (w, h) = (512, 384);
        let still = scene(w, h, 0, 0);
        let mut noise = Noise::new(11);
        // Five frames; in the last two a bright block has moved into one
        // place (a hand passing through).
        let mut frames = Vec::new();
        for i in 0..5 {
            let mut p = noisy(&still, 5.0, &mut noise);
            if i >= 3 {
                for y in 64..96 {
                    for x in 96..128 {
                        p.px[y * w + x] = 250;
                    }
                }
            }
            frames.push(frame(&p));
        }
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        // The tiles there weigh nothing in the frames where it moved.
        let reference = Plane::from(w, h, &frames[0][..w * h]);
        let rp = Pyramid::of(&reference);
        let fields: Vec<Field> = refs[1..].iter().map(|f| {
            let alt = Plane::from(w, h, &f[..w * h]);
            align(&reference, &rp, &alt, &Pyramid::of(&alt))
        }).collect();
        let n = noise_of(&fields);
        let moved = tile_weights(&fields[3], n);
        let tile = (64 / TILE) * fields[3].cols + 96 / TILE;
        assert_eq!(moved[tile], 0, "the moved tile counts for nothing (cost {}, noise {n})", fields[3].cost[tile]);
        assert_eq!(tile_weights(&fields[0], n)[tile], ONE, "the same tile counts in full where nothing moved");
        // And the merged picture there is the first frame's, not a ghost of
        // the block.
        let (merged, report) = merge(&refs, w, h, &[], 2, &mut |_| {});
        let mean_in = |p: &[u8]| (64..96).flat_map(|y| (96..128).map(move |x| u32::from(p[y * w + x]))).sum::<u32>() / (32 * 32);
        let (got, clean) = (mean_in(&merged), mean_in(&finished(&still, 5)));
        assert!(got.abs_diff(clean) < 12, "no ghost: {got} against the scene's {clean}");
        assert!(report.kept > 0.5 && report.kept < 0.99, "most of the other frames counted ({:.2})", report.kept);
    }

    #[test]
    fn merging_a_burst_takes_the_noise_down() {
        let (w, h) = (512, 384);
        let clean = scene(w, h, 0, 0);
        let mut noise = Noise::new(3);
        let mut frames = Vec::new();
        // Six frames as a hand holds them: a few pixels apart, noisy.
        for (i, (sx, sy)) in [(0, 0), (2, 1), (-3, 2), (1, -2), (4, 3), (-2, -1)].into_iter().enumerate() {
            let p = noisy(&scene(w, h, sx, sy), 9.0, &mut noise);
            frames.push(frame(&p));
            if i == 0 {
                assert!(rms(&p.px, &clean.px, w, 8) > 8.0);
            }
        }
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        let single = rms(&frames[0][..w * h], &clean.px, w, 8);
        let (merged, report) = merge(&refs, w, h, &[], 2, &mut |_| {});
        // Against the clean scene finished the same way (the finish lifts
        // the shadows and sharpens).
        let after = rms(&merged[..w * h], &finished(&clean, 6)[..w * h], w, 8);
        assert!(after < single * 0.62, "noise {single:.1} → {after:.1}");
        assert!(report.kept > 0.85, "a still scene keeps nearly every pixel ({:.2})", report.kept);
    }

    /// A clean plane through the same finish as a merge of `frames`.
    fn finished(p: &Plane, frames: usize) -> Vec<u8> {
        let mut f = frame(p);
        finish(&mut f, p.w, p.h, &[], 1, frames);
        f
    }

    #[test]
    fn faces_are_sharpened_less_and_keep_their_colour() {
        let (w, h) = (128, 96);
        let mut yuv = frame(&scene(w, h, 0, 0));
        // A warm colour everywhere.
        let ysize = w * h;
        for v in &mut yuv[ysize..ysize + ysize / 4] {
            *v = 110;
        }
        for v in &mut yuv[ysize + ysize / 4..] {
            *v = 160;
        }
        let before = yuv.clone();
        let face = [0.25, 0.25, 0.75, 0.75];
        finish(&mut yuv, w, h, &[face], 1, 6);
        // Inside the face, the colour is as it was; outside it has moved.
        let (cw, ch) = (w / 2, h / 2);
        let at = |p: &[u8], x: usize, y: usize| p[ysize + y * cw + x];
        assert_eq!(at(&yuv, cw / 2, ch / 2), at(&before, cw / 2, ch / 2));
        assert!(at(&yuv, 2, 2).abs_diff(128) >= at(&before, 2, 2).abs_diff(128), "outside, never less colourful");
        // And the change to the brightness is smaller inside.
        let change = |x0: usize, y0: usize| (y0..y0 + 16).flat_map(|y| (x0..x0 + 16).map(move |x| (y, x))).map(|(y, x)| u32::from(yuv[y * w + x].abs_diff(before[y * w + x]))).sum::<u32>();
        assert!(change(56, 40) < change(4, 4) + 400, "face {} outside {}", change(56, 40), change(4, 4));
    }

    #[test]
    fn the_merged_photo_is_written_with_the_first_frames_exif() {
        let dir = std::env::temp_dir().join(format!("catalyst-burst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (64, 48);
        let yuv = frame(&scene(w, h, 0, 0));
        let exif = b"Exif\0\0II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec();
        let path = dir.join("IMG_TEST.jpg");
        write_jpeg(&path, &yuv, w, h, Some(exif.clone())).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(exif_of(&bytes), Some(exif));
        assert_eq!(crate::photos::dimensions(&path), Some((w, h)));
        // The marker: written, read, its time left sensible.
        let r = Refining { began_ms: 1_000, expect_ms: 6_000, progress: 0.5 };
        write_marker(&path, &r);
        assert_eq!(read_marker(&path), Some(r.clone()));
        assert_eq!(r.left_s(4_000), 3);
        let overdue = Refining { progress: 0.0, ..r };
        assert_eq!(overdue.left_s(60_000), 1, "never says zero while it runs");
        std::fs::remove_dir_all(&dir).ok();
    }
}
