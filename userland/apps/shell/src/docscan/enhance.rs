//! A flattened page made clean: the paper's own brightness under the light
//! found (each block's brightest tenth, smoothed), and every pixel taken
//! against it, so the paper comes out white however it was lit and the
//! ink dark. Then in colour, in grey, or in black and white, whose
//! threshold is each pixel's neighbourhood's (Bradley's adaptive
//! threshold, over a summed-area table), so text stays whole across a
//! shadow.

use crate::photos::Picture;

/// How a page comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Enhance {
    Colour,
    Grey,
    /// Black and white, for text.
    #[default]
    Mono,
}

pub const ENHANCES: [Enhance; 3] = [Enhance::Colour, Enhance::Grey, Enhance::Mono];

impl Enhance {
    pub fn name(self) -> &'static str {
        match self {
            Enhance::Colour => "colour",
            Enhance::Grey => "grey",
            Enhance::Mono => "b&w",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Enhance::Colour => "colour",
            Enhance::Grey => "grey",
            Enhance::Mono => "mono",
        }
    }

    pub fn from_id(id: &str) -> Enhance {
        ENHANCES.iter().copied().find(|e| e.id() == id).unwrap_or_default()
    }
}

/// A pixel's brightness (BT.601).
#[inline]
fn luma(p: &[u8]) -> u32 {
    (77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8
}

/// The paper's brightness at each pixel, as a coarse map: blocks about a
/// fortieth of the page across, each its brightest tenth's brightness;
/// a block with little paper in it (a picture, a dark edge) takes its
/// brighter neighbours'; smoothed; then read between blocks smoothly.
pub struct Light {
    bw: usize,
    bh: usize,
    block: usize,
    grid: Vec<f32>,
}

impl Light {
    pub fn of(page: &Picture) -> Light {
        let block = (page.w.max(page.h) / 40).clamp(8, 64);
        let (bw, bh) = (page.w.div_ceil(block), page.h.div_ceil(block));
        let mut grid = vec![0f32; bw * bh];
        let mut hist = [0u32; 256];
        for by in 0..bh {
            for bx in 0..bw {
                hist.fill(0);
                let mut n = 0;
                for y in by * block..((by + 1) * block).min(page.h) {
                    for x in bx * block..((bx + 1) * block).min(page.w) {
                        hist[luma(&page.rgb[(y * page.w + x) * 3..]) as usize] += 1;
                        n += 1;
                    }
                }
                let mut seen = 0;
                let mut v = 255;
                for (i, c) in hist.iter().enumerate().rev() {
                    seen += c;
                    if seen * 10 >= n {
                        v = i;
                        break;
                    }
                }
                grid[by * bw + bx] = v as f32;
            }
        }
        // Blocks darker than their neighbourhood's paper take its brightest
        // (twice: a dark patch two blocks wide is filled too), then a 3 × 3 mean.
        for _ in 0..2 {
            let before = grid.clone();
            for by in 0..bh {
                for bx in 0..bw {
                    let mut most = 0f32;
                    for y in by.saturating_sub(1)..=(by + 1).min(bh - 1) {
                        for x in bx.saturating_sub(1)..=(bx + 1).min(bw - 1) {
                            most = most.max(before[y * bw + x]);
                        }
                    }
                    let here = &mut grid[by * bw + bx];
                    if *here < most * 0.8 {
                        *here = most * 0.92;
                    }
                }
            }
        }
        let before = grid.clone();
        for by in 0..bh {
            for bx in 0..bw {
                let (mut sum, mut n) = (0.0, 0.0);
                for y in by.saturating_sub(1)..=(by + 1).min(bh - 1) {
                    for x in bx.saturating_sub(1)..=(bx + 1).min(bw - 1) {
                        sum += before[y * bw + x];
                        n += 1.0;
                    }
                }
                grid[by * bw + bx] = (sum / n).max(24.0);
            }
        }
        Light { bw, bh, block, grid }
    }

    /// The paper's brightness at (x, y).
    pub fn at(&self, x: usize, y: usize) -> f32 {
        let fx = ((x as f32 + 0.5) / self.block as f32 - 0.5).clamp(0.0, (self.bw - 1) as f32);
        let fy = ((y as f32 + 0.5) / self.block as f32 - 0.5).clamp(0.0, (self.bh - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.bw - 1), (y0 + 1).min(self.bh - 1));
        let (kx, ky) = (fx - x0 as f32, fy - y0 as f32);
        let g = |x: usize, y: usize| self.grid[y * self.bw + x];
        let top = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * kx;
        let bottom = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * kx;
        top + (bottom - top) * ky
    }
}

/// A value against the paper's (1 is paper), to the page's: the paper white,
/// the ink pressed toward black.
#[inline]
fn clean(v: f32) -> u8 {
    let t = ((v - 0.1) / 0.78).clamp(0.0, 1.0);
    (t.powf(1.25) * 255.0).round() as u8
}

/// The page enhanced. Grey and black and white come out with their three
/// channels equal.
pub fn enhance(page: &Picture, how: Enhance) -> Picture {
    let mut out = Picture::new(page.w, page.h);
    if page.is_empty() {
        return out;
    }
    let light = Light::of(page);
    let (w, h) = (page.w, page.h);
    match how {
        Enhance::Colour => {
            for y in 0..h {
                for x in 0..w {
                    let at = (y * w + x) * 3;
                    let l = light.at(x, y);
                    for c in 0..3 {
                        out.rgb[at + c] = clean(f32::from(page.rgb[at + c]) / l);
                    }
                }
            }
        }
        Enhance::Grey => {
            for y in 0..h {
                for x in 0..w {
                    let at = (y * w + x) * 3;
                    let v = clean(luma(&page.rgb[at..]) as f32 / light.at(x, y));
                    out.rgb[at..at + 3].fill(v);
                }
            }
        }
        Enhance::Mono => {
            let mut norm = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    norm[y * w + x] = clean(luma(&page.rgb[(y * w + x) * 3..]) as f32 / light.at(x, y));
                }
            }
            let bits = threshold(&norm, w, h);
            for (o, b) in out.rgb.chunks_exact_mut(3).zip(bits) {
                o.fill(if b { 255 } else { 0 });
            }
        }
    }
    out
}

/// Bradley's adaptive threshold: a pixel is paper (true) unless it is more
/// than an eighth darker than the mean of the square around it (a
/// sixteenth of the page wide), or dark outright.
pub fn threshold(v: &[u8], w: usize, h: usize) -> Vec<bool> {
    // The summed-area table, a row and column of nothing before the first.
    let mut sum = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(v[y * w + x]);
            sum[(y + 1) * (w + 1) + x + 1] = sum[y * (w + 1) + x + 1] + row;
        }
    }
    let half = (w.max(h) / 32).max(4);
    let mut out = vec![true; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(half), (y + half + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(half), (x + half + 1).min(w));
            let n = ((x1 - x0) * (y1 - y0)) as u32;
            let total = sum[y1 * (w + 1) + x1] + sum[y0 * (w + 1) + x0] - sum[y0 * (w + 1) + x1] - sum[y1 * (w + 1) + x0];
            let here = u32::from(v[y * w + x]);
            out[y * w + x] = here * n * 8 > total * 7 && here > 40;
        }
    }
    out
}
