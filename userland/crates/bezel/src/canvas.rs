//! The scene pass: an opaque RGB framebuffer and signed-distance shapes drawn
//! into it with one pixel of anti-aliasing, as the Bezel specimen draws its
//! Material surfaces.

use crate::color::Color;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    pub fn cx(&self) -> f32 { self.x + self.w * 0.5 }
    pub fn cy(&self) -> f32 { self.y + self.h * 0.5 }
    /// Grow (or, negative, shrink) on every side.
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
    /// Scale about the centre.
    pub fn scaled(&self, s: f32) -> Rect {
        let (w, h) = (self.w * s, self.h * s);
        Rect::new(self.cx() - w * 0.5, self.cy() - h * 0.5, w, h)
    }
    pub fn lerp(&self, o: &Rect, t: f32) -> Rect {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Rect::new(l(self.x, o.x), l(self.y, o.y), l(self.w, o.w), l(self.h, o.h))
    }
}

/// Signed distance to a rounded rectangle: negative inside.
#[inline]
pub fn sd_rrect(px: f32, py: f32, r: &Rect, radius: f32) -> f32 {
    let radius = radius.min(r.w * 0.5).min(r.h * 0.5);
    let qx = (px - r.cx()).abs() - (r.w * 0.5 - radius);
    let qy = (py - r.cy()).abs() - (r.h * 0.5 - radius);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - radius
}

#[inline]
fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

#[inline]
pub fn blend(dst: u32, c: Color, a: f32) -> u32 {
    let a = (a * 256.0) as u32;
    if a >= 256 {
        return c.to_u32();
    }
    let inv = 256 - a;
    let r = (((dst >> 16) & 0xff) * inv + c.0 as u32 * a) >> 8;
    let g = (((dst >> 8) & 0xff) * inv + c.1 as u32 * a) >> 8;
    let b = ((dst & 0xff) * inv + c.2 as u32 * a) >> 8;
    r << 16 | g << 8 | b
}

#[derive(Clone)]
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    /// `0x00RRGGBB`, row-major.
    pub px: Vec<u32>,
    clip: (i32, i32, i32, i32),
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Canvas { w, h, px: vec![0; (w * h) as usize], clip: (0, 0, w, h) }
    }

    pub fn clear(&mut self, c: Color) {
        self.px.fill(c.to_u32());
    }

    /// Restrict drawing to a rectangle (for scrolling regions).
    pub fn set_clip(&mut self, r: Rect) {
        let x0 = (r.x.floor() as i32).clamp(0, self.w);
        let y0 = (r.y.floor() as i32).clamp(0, self.h);
        let x1 = (r.right().ceil() as i32).clamp(x0, self.w);
        let y1 = (r.bottom().ceil() as i32).clamp(y0, self.h);
        self.clip = (x0, y0, x1, y1);
    }

    pub fn clear_clip(&mut self) {
        self.clip = (0, 0, self.w, self.h);
    }

    /// Paint `color` wherever `cover(x, y)` says, over the pixels of `bbox`.
    #[inline]
    fn shade(&mut self, bbox: Rect, color: Color, alpha: f32, cover: impl Fn(f32, f32) -> f32) {
        if alpha <= 0.0 {
            return;
        }
        let x0 = (bbox.x.floor() as i32).max(self.clip.0);
        let y0 = (bbox.y.floor() as i32).max(self.clip.1);
        let x1 = (bbox.right().ceil() as i32).min(self.clip.2);
        let y1 = (bbox.bottom().ceil() as i32).min(self.clip.3);
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            let fy = y as f32 + 0.5;
            for x in x0..x1 {
                let a = cover(x as f32 + 0.5, fy) * alpha;
                if a > 0.004 {
                    let p = &mut self.px[row + x as usize];
                    *p = blend(*p, color, a);
                }
            }
        }
    }

    pub fn fill_rect(&mut self, r: Rect, color: Color, alpha: f32) {
        self.shade(r, color, alpha, |x, y| {
            let dx = (r.x - x).max(x - r.right());
            let dy = (r.y - y).max(y - r.bottom());
            coverage(dx.max(dy))
        });
    }

    /// A Material surface: a filled rounded rectangle.
    ///
    /// Every screen is made of these, so the inside is not worked out a pixel
    /// at a time: each row's solid run is filled in one go, and only the
    /// pixels near an edge or in a corner go through the distance field.
    pub fn rrect(&mut self, r: Rect, radius: f32, color: Color, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5);
        let bbox = r.inset(-1.0);
        let x0 = (bbox.x.floor() as i32).max(self.clip.0);
        let y0 = (bbox.y.floor() as i32).max(self.clip.1);
        let x1 = (bbox.right().ceil() as i32).min(self.clip.2);
        let y1 = (bbox.bottom().ceil() as i32).min(self.clip.3);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        // The pixels whose centres lie strictly between `lo` and `hi`.
        let run = |lo: f32, hi: f32| (((lo - 0.5).floor() as i32 + 1).clamp(x0, x1), ((hi - 0.5).ceil() as i32).clamp(x0, x1));
        // Between the corners a row is solid from edge to edge; through the
        // corners, only between the corner columns; the edge rows, nowhere.
        let wide = run(r.x + 1.0, r.right() - 1.0);
        let narrow = run(r.x + rad + 1.0, r.right() - rad - 1.0);
        let solid = (alpha * 256.0) as u32 >= 256;
        let packed = color.to_u32();
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            let fy = y as f32 + 0.5;
            let (from, to) = if fy > r.y + rad + 1.0 && fy < r.bottom() - rad - 1.0 {
                wide
            } else if fy > r.y + 1.0 && fy < r.bottom() - 1.0 {
                narrow
            } else {
                (x0, x0)
            };
            let to = to.max(from);
            for x in (x0..from).chain(to..x1) {
                let a = coverage(sd_rrect(x as f32 + 0.5, fy, &r, radius)) * alpha;
                if a > 0.004 {
                    let p = &mut self.px[row + x as usize];
                    *p = blend(*p, color, a);
                }
            }
            let span = &mut self.px[row + from as usize..row + to as usize];
            if solid {
                span.fill(packed);
            } else {
                span.iter_mut().for_each(|p| *p = blend(*p, color, alpha));
            }
        }
    }

    /// A fill that takes its track's outer corners: `r` clipped to the
    /// rounded rectangle `clip`.
    pub fn rrect_clipped(&mut self, r: Rect, radius: f32, clip: Rect, clip_radius: f32, color: Color, alpha: f32) {
        self.shade(r.inset(-1.0), color, alpha, |x, y| {
            coverage(sd_rrect(x, y, &r, radius).max(sd_rrect(x, y, &clip, clip_radius)))
        });
    }

    /// A ring of `width` just inside the edge of a rounded rectangle.
    pub fn stroke_rrect(&mut self, r: Rect, radius: f32, width: f32, color: Color, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let half = width * 0.5;
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5);
        let bbox = r.inset(-1.0);
        let x0 = (bbox.x.floor() as i32).max(self.clip.0);
        let y0 = (bbox.y.floor() as i32).max(self.clip.1);
        let x1 = (bbox.right().ceil() as i32).min(self.clip.2);
        let y1 = (bbox.bottom().ceil() as i32).min(self.clip.3);
        // Wholly outside what may be drawn on: nothing to do.
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        // A ring is all edge: between the corners, only the few pixels at
        // each side of a row can be on it, so the middle is not visited.
        let reach = width + 2.0;
        let (left_end, right_start) = (((r.x + reach).ceil() as i32).clamp(x0, x1), ((r.right() - reach).floor() as i32).clamp(x0, x1));
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            let fy = y as f32 + 0.5;
            let sides_only = fy > r.y + rad + reach && fy < r.bottom() - rad - reach && right_start > left_end;
            let (skip_from, skip_to) = if sides_only { (left_end, right_start) } else { (x1, x1) };
            for x in (x0..skip_from).chain(skip_to..x1) {
                let a = coverage((sd_rrect(x as f32 + 0.5, fy, &r, radius) + half).abs() - half) * alpha;
                if a > 0.004 {
                    let p = &mut self.px[row + x as usize];
                    *p = blend(*p, color, a);
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color, alpha: f32) {
        let bbox = Rect::new(cx - radius - 1.0, cy - radius - 1.0, radius * 2.0 + 2.0, radius * 2.0 + 2.0);
        self.shade(bbox, color, alpha, |x, y| {
            coverage(((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - radius)
        });
    }

    /// An arc with round caps. Angles in radians, clockwise from 12 o'clock.
    pub fn arc(&mut self, cx: f32, cy: f32, radius: f32, thickness: f32, start: f32, sweep: f32, color: Color, alpha: f32) {
        if sweep <= 0.0 {
            return;
        }
        let half = thickness * 0.5;
        let ext = radius + half + 1.0;
        let end = start + sweep;
        let cap = |a: f32| (cx + radius * a.sin(), cy - radius * a.cos());
        let (c0, c1) = (cap(start), cap(end));
        let tau = std::f32::consts::TAU;
        self.shade(Rect::new(cx - ext, cy - ext, ext * 2.0, ext * 2.0), color, alpha, |x, y| {
            let (dx, dy) = (x - cx, y - cy);
            let ang = (dx.atan2(-dy) - start).rem_euclid(tau);
            let d = if ang <= sweep {
                ((dx * dx + dy * dy).sqrt() - radius).abs()
            } else {
                let d0 = ((x - c0.0).powi(2) + (y - c0.1).powi(2)).sqrt();
                let d1 = ((x - c1.0).powi(2) + (y - c1.1).powi(2)).sqrt();
                d0.min(d1)
            };
            coverage(d - half)
        });
    }

    /// A stroked polyline with round joins, for sparklines.
    pub fn polyline(&mut self, pts: &[(f32, f32)], width: f32, color: Color, alpha: f32) {
        if pts.len() < 2 {
            return;
        }
        let half = width * 0.5;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        let bbox = Rect::new(x0 - half - 1.0, y0 - half - 1.0, x1 - x0 + width + 2.0, y1 - y0 + width + 2.0);
        // One distance field for the whole line, so joins are covered once.
        self.shade(bbox, color, alpha, |x, y| {
            let mut best = f32::MAX;
            for seg in pts.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                if x < a.0.min(b.0) - half - 1.0 || x > a.0.max(b.0) + half + 1.0 {
                    continue;
                }
                let (bx, by) = (b.0 - a.0, b.1 - a.1);
                let (px, py) = (x - a.0, y - a.1);
                let t = ((px * bx + py * by) / (bx * bx + by * by).max(1e-6)).clamp(0.0, 1.0);
                best = best.min((px - bx * t).powi(2) + (py - by * t).powi(2));
            }
            coverage(best.sqrt() - half)
        });
    }

    /// A filled polygon, anti-aliased: vector artwork such as a logo's mark.
    /// Each pixel's coverage comes from its distance to the nearest edge,
    /// signed by whether it is inside (even–odd), so the edge is smooth at
    /// any size and any position, including between pixels.
    pub fn polygon(&mut self, pts: &[(f32, f32)], color: Color, alpha: f32) {
        if pts.len() < 3 {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        let bbox = Rect::new(x0 - 1.0, y0 - 1.0, x1 - x0 + 2.0, y1 - y0 + 2.0);
        let n = pts.len();
        self.shade(bbox, color, alpha, |x, y| {
            let mut nearest = f32::MAX;
            let mut inside = false;
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                    inside = !inside;
                }
                let (bx, by) = (b.0 - a.0, b.1 - a.1);
                let (px, py) = (x - a.0, y - a.1);
                let t = ((px * bx + py * by) / (bx * bx + by * by).max(1e-6)).clamp(0.0, 1.0);
                nearest = nearest.min((px - bx * t).powi(2) + (py - by * t).powi(2));
            }
            let d = nearest.sqrt();
            coverage(if inside { -d } else { d })
        });
    }

    /// A soft radial glow, for the ground behind pages.
    pub fn glow(&mut self, cx: f32, cy: f32, radius: f32, color: Color, alpha: f32) {
        let inv = 1.0 / radius;
        self.shade(Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0), color, alpha, |x, y| {
            let t = (1.0 - ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() * inv).max(0.0);
            t * t * (3.0 - 2.0 * t)
        });
    }

    /// Blit an 8-bit coverage mask (a glyph or an icon) in one colour.
    pub fn mask(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8], color: Color, alpha: f32) {
        let x0 = x.max(self.clip.0);
        let y0 = y.max(self.clip.1);
        let x1 = (x + w).min(self.clip.2);
        let y1 = (y + h).min(self.clip.3);
        // Text and icons are most of what is drawn, so this stays in whole
        // numbers: coverage 0..255 times alpha, as 256ths.
        let scale = (alpha.clamp(0.0, 1.0) * (256.0 / 255.0) * 65536.0) as u32;
        let (cr, cg, cb) = (color.0 as u32, color.1 as u32, color.2 as u32);
        let packed = color.to_u32();
        for yy in y0..y1 {
            let src = ((yy - y) * w) as usize;
            let dst = (yy * self.w) as usize;
            for xx in x0..x1 {
                let m = data[src + (xx - x) as usize];
                if m == 0 {
                    continue;
                }
                let a = (m as u32 * scale) >> 16;
                let p = &mut self.px[dst + xx as usize];
                if a >= 256 {
                    *p = packed;
                    continue;
                }
                let inv = 256 - a;
                let d = *p;
                *p = ((((d >> 16) & 0xff) * inv + cr * a) >> 8) << 16 | ((((d >> 8) & 0xff) * inv + cg * a) >> 8) << 8 | (((d & 0xff) * inv + cb * a) >> 8);
            }
        }
    }

    /// Lay a premultiplied picture over what is drawn (a pre-rendered 3D
    /// object): each source pixel is `0xKKRRGGBB`, its colour already
    /// multiplied by its cover, and `KK` how much of what is under it is
    /// left (its shadow can darken it more than its cover alone would).
    /// `alpha` fades the whole picture.
    pub fn blit_premultiplied(&mut self, x: i32, y: i32, w: i32, h: i32, src: &[u32], alpha: f32) {
        let x0 = x.max(self.clip.0);
        let y0 = y.max(self.clip.1);
        let x1 = (x + w).min(self.clip.2);
        let y1 = (y + h).min(self.clip.3);
        let a = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
        if a == 0 {
            return;
        }
        for yy in y0..y1 {
            let s = ((yy - y) * w) as usize;
            let d = (yy * self.w) as usize;
            for xx in x0..x1 {
                let p = src[s + (xx - x) as usize];
                let keep = p >> 24;
                if keep == 255 {
                    continue;
                }
                let dst = &mut self.px[d + xx as usize];
                let q = *dst;
                // What is left of the pixel under it: all of it where the
                // picture is faded out, `keep` where it is whole.
                let left = ((256 - a) * 255 + a * keep) >> 8;
                let ch = |shift: u32| -> u32 {
                    let under = (q >> shift) & 0xff;
                    let over = (p >> shift) & 0xff;
                    ((under * left + over * a * 255 / 256) / 255).min(255)
                };
                *dst = ch(16) << 16 | ch(8) << 8 | ch(0);
            }
        }
    }

    /// Pack the frame as RGB565 for the panel, without dithering.
    pub fn to_rgb565(&self, out: &mut [u16]) {
        for (o, p) in out.iter_mut().zip(&self.px) {
            *o = (((p >> 19) & 0x1f) << 11 | ((p >> 10) & 0x3f) << 5 | ((p >> 3) & 0x1f)) as u16;
        }
    }
}

/// A 4 × 4 ordered-dither matrix, 0 to 15.
pub(crate) const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// A pixel as RGB565 for a 16-bit panel, dithered by its place on the panel.
///
/// Truncating 8 bits to 5 turns a dark gradient (the ground, a glow, frosted
/// glass) into a few visible steps. Adding a threshold that depends only on
/// the pixel's position spreads each step into a fine, fixed pattern the eye
/// averages back into the gradient, and a picture that does not change does
/// not shimmer.
#[inline]
pub fn rgb565_dithered(p: u32, x: usize, y: usize) -> u16 {
    let d = BAYER[y & 3][x & 3] as u32;
    // Steps of 8 for red and blue, 4 for green: thresholds across each step.
    let (d5, d6) = (d / 2, d / 4);
    let r = (((p >> 16) & 0xff) + d5).min(255);
    let g = (((p >> 8) & 0xff) + d6).min(255);
    let b = ((p & 0xff) + d5).min(255);
    ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u16
}

/// What the panel shows for an RGB565 value, back in 8 bits a channel.
#[inline]
pub fn rgb565_to_rgb(v: u16) -> u32 {
    let (r, g, b) = ((v >> 11) as u32 & 0x1f, (v >> 5) as u32 & 0x3f, v as u32 & 0x1f);
    (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dark_gradient_keeps_its_average_on_a_16_bit_panel() {
        // A 4 × 4 patch of one dark grey between two of the panel's steps:
        // truncated it falls to the step below; dithered, it averages back.
        let grey = 0x13_13_13u32;
        let mean = |dither: bool| {
            let mut sum = 0u32;
            for y in 0..4 {
                for x in 0..4 {
                    let v = if dither { rgb565_dithered(grey, x, y) } else { ((grey >> 19) << 11 | (grey >> 3 & 0x1f)) as u16 };
                    sum += (rgb565_to_rgb(v) >> 16) & 0xff;
                }
            }
            sum as f32 / 16.0
        };
        assert!((mean(true) - 19.0).abs() < 2.0, "dithered {}", mean(true));
        assert!((mean(false) - 19.0).abs() > 2.0, "truncated {}", mean(false));
        // White stays white, black stays black.
        assert_eq!(rgb565_dithered(0xffffff, 3, 3), 0xffff);
        assert_eq!(rgb565_dithered(0, 3, 3), 0);
    }

    #[test]
    fn shapes_wholly_off_the_panel_draw_nothing() {
        let mut canvas = Canvas::new(320, 170);
        canvas.clear(Color::hex(0x101418));
        let before = canvas.px.clone();
        // Past each edge, as a tile is while its page slides away.
        for r in [Rect::new(330.0, 40.0, 90.0, 60.0), Rect::new(-120.0, 40.0, 90.0, 60.0), Rect::new(40.0, 200.0, 90.0, 60.0), Rect::new(40.0, -90.0, 90.0, 60.0)] {
            canvas.rrect(r, 12.0, Color::hex(0xff5a1f), 1.0);
            canvas.stroke_rrect(r, 12.0, 2.0, Color::hex(0xff5a1f), 1.0);
        }
        assert!(canvas.px == before);
        // Half on: the part on the panel is drawn.
        canvas.stroke_rrect(Rect::new(290.0, 40.0, 90.0, 60.0), 12.0, 2.0, Color::hex(0xff5a1f), 1.0);
        assert!(canvas.px != before);
    }
}
