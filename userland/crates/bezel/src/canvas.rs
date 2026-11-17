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
    pub fn rrect(&mut self, r: Rect, radius: f32, color: Color, alpha: f32) {
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5);
        let (ix0, ix1) = (r.x + rad + 1.0, r.right() - rad - 1.0);
        let (iy0, iy1) = (r.y + 1.0, r.bottom() - 1.0);
        self.shade(r.inset(-1.0), color, alpha, |x, y| {
            // The body between the corner columns needs no distance field.
            if x > ix0 && x < ix1 && y > iy0 && y < iy1 {
                1.0
            } else {
                coverage(sd_rrect(x, y, &r, radius))
            }
        });
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
        let half = width * 0.5;
        self.shade(r.inset(-1.0), color, alpha, |x, y| {
            coverage((sd_rrect(x, y, &r, radius) + half).abs() - half)
        });
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
        for yy in y0..y1 {
            let src = ((yy - y) * w) as usize;
            let dst = (yy * self.w) as usize;
            for xx in x0..x1 {
                let m = data[src + (xx - x) as usize];
                if m != 0 {
                    let p = &mut self.px[dst + xx as usize];
                    *p = blend(*p, color, m as f32 * (alpha / 255.0));
                }
            }
        }
    }

    /// Pack the frame as RGB565 for the panel.
    pub fn to_rgb565(&self, out: &mut [u16]) {
        for (o, p) in out.iter_mut().zip(&self.px) {
            *o = (((p >> 19) & 0x1f) << 11 | ((p >> 10) & 0x3f) << 5 | ((p >> 3) & 0x1f)) as u16;
        }
    }
}
