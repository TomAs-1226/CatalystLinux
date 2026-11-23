//! Flattening a page: its true shape recovered from its corners and the
//! lens, A4 or Letter guessed from that, and the page drawn flat at that
//! shape (a homography from the flat page to its corners in the photo,
//! sampled smoothly, rows split among the cores).

use super::quad::{Pt, Quad};
use crate::photos::Picture;

/// The IMX219's lens: 3.04 mm over 1.12 µm pixels, in the full photo's
/// pixels (3280 across); a smaller picture of the same view scales it.
pub const FOCAL_FULL: f32 = 3.04 / 0.001_12;

/// The paper a page is taken to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Paper {
    /// A4 or Letter as the page's shape says, else its own shape.
    #[default]
    Auto,
    A4,
    Letter,
    /// Its own shape, as measured.
    Free,
}

pub const PAPERS: [Paper; 4] = [Paper::Auto, Paper::A4, Paper::Letter, Paper::Free];

impl Paper {
    pub fn name(self) -> &'static str {
        match self {
            Paper::Auto => "auto",
            Paper::A4 => "a4",
            Paper::Letter => "letter",
            Paper::Free => "free",
        }
    }

    pub fn from_name(name: &str) -> Paper {
        PAPERS.iter().copied().find(|p| p.name() == name).unwrap_or_default()
    }

    /// Its long side over its short one, if it has a fixed shape.
    pub fn ratio(self) -> Option<f32> {
        match self {
            Paper::A4 => Some(297.0 / 210.0),
            Paper::Letter => Some(11.0 / 8.5),
            _ => None,
        }
    }

    /// Its size in points (1/72 in), upright.
    pub fn points(self) -> Option<(f32, f32)> {
        match self {
            Paper::A4 => Some((595.28, 841.89)),
            Paper::Letter => Some((612.0, 792.0)),
            _ => None,
        }
    }

    /// What `Auto` comes to for a page of `ratio` (long over short): the
    /// nearer of A4 and Letter when within 4% of it, else its own shape.
    pub fn resolve(self, ratio: f32) -> Paper {
        if self != Paper::Auto {
            return self;
        }
        let near = |p: Paper| p.ratio().map_or(f32::MAX, |r| (ratio / r - 1.0).abs());
        let (a4, letter) = (near(Paper::A4), near(Paper::Letter));
        match (a4.min(letter) < 0.04, a4 <= letter) {
            (true, true) => Paper::A4,
            (true, false) => Paper::Letter,
            _ => Paper::Free,
        }
    }
}

/// A page's shape, its height over its width, from its corners (top-left,
/// top-right, bottom-right, bottom-left) in a `size` picture whose lens has
/// focal length `focal` (pixels): the rectangle's own shape recovered from
/// its perspective (Zhang and He, "Whiteboard scanning and image
/// enhancement", 2007), the principal point at the picture's middle. When
/// that cannot be worked out (the page square on), its sides' lengths.
pub fn page_ratio(q: &Quad, size: (f32, f32), focal: f32) -> f32 {
    let (u0, v0) = (f64::from(size.0) * 0.5, f64::from(size.1) * 0.5);
    let f = f64::from(focal);
    let m = |p: Pt| [f64::from(p.0), f64::from(p.1), 1.0];
    // m1 top-left, m2 top-right, m3 bottom-left, m4 bottom-right.
    let (m1, m2, m3, m4) = (m(q[0]), m(q[1]), m(q[3]), m(q[2]));
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let k2 = dot(cross(m1, m4), m3) / dot(cross(m2, m4), m3);
    let k3 = dot(cross(m1, m4), m2) / dot(cross(m3, m4), m2);
    let n2 = [k2 * m2[0] - m1[0], k2 * m2[1] - m1[1], k2 * m2[2] - m1[2]];
    let n3 = [k3 * m3[0] - m1[0], k3 * m3[1] - m1[1], k3 * m3[2] - m1[2]];
    // Through the lens: A⁻¹ n, A the camera's matrix.
    let back = |n: [f64; 3]| [(n[0] - u0 * n[2]) / f, (n[1] - v0 * n[2]) / f, n[2]];
    let (a, b) = (back(n2), back(n3));
    let ratio = (dot(b, b) / dot(a, a)).sqrt() as f32;
    if ratio.is_finite() && (0.25..4.0).contains(&ratio) {
        return ratio;
    }
    let len = |p: Pt, r: Pt| ((p.0 - r.0).powi(2) + (p.1 - r.1).powi(2)).sqrt();
    (len(q[0], q[3]) + len(q[1], q[2])) / (len(q[0], q[1]) + len(q[3], q[2])).max(1.0)
}

/// The size to flatten a page to: its shape `ratio` (height over width),
/// about as many pixels across as it covers in the photo (never more), its
/// long side at most `longest`.
pub fn flat_size(q: &Quad, ratio: f32, longest: usize) -> (usize, usize) {
    let len = |p: Pt, r: Pt| ((p.0 - r.0).powi(2) + (p.1 - r.1).powi(2)).sqrt();
    let across = (len(q[0], q[1]) + len(q[3], q[2])) * 0.5;
    let down = (len(q[0], q[3]) + len(q[1], q[2])) * 0.5;
    // The page's long side, as many pixels as the photo gives it.
    let long_px = across.max(down).min(longest as f32);
    let (w, h) = if ratio >= 1.0 { (long_px / ratio, long_px) } else { (long_px, long_px * ratio) };
    ((w.round() as usize).max(8), (h.round() as usize).max(8))
}

/// A plane-to-plane map: `(x, y)` to `((a x + b y + c) / (g x + h y + 1), (d x + e y + f) / (…))`.
#[derive(Clone, Copy, Debug)]
pub struct Homography(pub [f64; 9]);

impl Homography {
    /// The one taking each of `from` to the same place in `to`.
    pub fn between(from: [Pt; 4], to: [Pt; 4]) -> Option<Homography> {
        // Eight equations in eight unknowns, solved by elimination.
        let mut m = [[0f64; 9]; 8];
        for i in 0..4 {
            let (x, y) = (f64::from(from[i].0), f64::from(from[i].1));
            let (u, v) = (f64::from(to[i].0), f64::from(to[i].1));
            m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        for col in 0..8 {
            let pivot = (col..8).max_by(|a, b| m[*a][col].abs().total_cmp(&m[*b][col].abs()))?;
            if m[pivot][col].abs() < 1e-12 {
                return None;
            }
            m.swap(col, pivot);
            for row in 0..8 {
                if row != col {
                    let k = m[row][col] / m[col][col];
                    for c in col..9 {
                        m[row][c] -= k * m[col][c];
                    }
                }
            }
        }
        let s: Vec<f64> = (0..8).map(|i| m[i][8] / m[i][i]).collect();
        Some(Homography([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], 1.0]))
    }

    pub fn at(&self, x: f64, y: f64) -> (f64, f64) {
        let h = &self.0;
        let z = h[6] * x + h[7] * y + h[8];
        ((h[0] * x + h[1] * y + h[2]) / z, (h[3] * x + h[4] * y + h[5]) / z)
    }
}

/// The page with corners `q` in `src` drawn flat at `w` × `h`.
pub fn flatten(src: &Picture, q: &Quad, w: usize, h: usize, threads: usize) -> Picture {
    let mut out = Picture::new(w, h);
    let corners = [(0.0, 0.0), (w as f32, 0.0), (w as f32, h as f32), (0.0, h as f32)];
    let Some(map) = Homography::between(corners, *q) else { return out };
    crate::editor::by_rows(&mut out.rgb, w * 3, threads, |y0, rows| {
        let n = rows.len() / (w * 3);
        flatten_rows(src, &map, w, y0..y0 + n, rows);
    });
    out
}

fn flatten_rows(src: &Picture, map: &Homography, w: usize, rows: std::ops::Range<usize>, out: &mut [u8]) {
    let (pw, ph) = (src.w, src.h);
    let (max_x, max_y) = ((pw - 1) as f64, (ph - 1) as f64);
    let h = &map.0;
    for (k, y) in rows.enumerate() {
        let fy = y as f64 + 0.5;
        // Along the row the numerators and the denominator each step evenly.
        let (mut nx, mut ny, mut nz) = (h[0] * 0.5 + h[1] * fy + h[2], h[3] * 0.5 + h[4] * fy + h[5], h[6] * 0.5 + h[7] * fy + h[8]);
        for x in 0..w {
            let (u, v) = ((nx / nz - 0.5).clamp(0.0, max_x), (ny / nz - 0.5).clamp(0.0, max_y));
            let (u0, v0) = (u as usize, v as usize);
            let (u1, v1) = ((u0 + 1).min(pw - 1), (v0 + 1).min(ph - 1));
            let (wx, wy) = (((u - u0 as f64) * 256.0) as u32, ((v - v0 as f64) * 256.0) as u32);
            let (a, b, c, d) = ((v0 * pw + u0) * 3, (v0 * pw + u1) * 3, (v1 * pw + u0) * 3, (v1 * pw + u1) * 3);
            let at = (k * w + x) * 3;
            for ch in 0..3 {
                let top = u32::from(src.rgb[a + ch]) * (256 - wx) + u32::from(src.rgb[b + ch]) * wx;
                let bottom = u32::from(src.rgb[c + ch]) * (256 - wx) + u32::from(src.rgb[d + ch]) * wx;
                out[at + ch] = ((top * (256 - wy) + bottom * wy + 32768) >> 16) as u8;
            }
            nx += h[0];
            ny += h[3];
            nz += h[6];
        }
    }
}
