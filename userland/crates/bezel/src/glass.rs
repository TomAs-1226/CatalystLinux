//! The glass pass, after `bezel-glass`: every glass shape is drawn in one pass
//! over the finished scene, so glass refracts what is under it (text
//! included) and never refracts other glass.
//!
//! Per pixel inside a shape: the surface normal is the distance field's
//! gradient steepened by the squircle bezel profile; a straight-down ray is
//! refracted at n = 1.5 through the slab's thickness at that depth; the
//! refracted sample is read from a frosted copy of the scene; then come
//! luminosity evening, saturation, tint, the edge light and the shadow.
//!
//! Lengths are the specimen's CSS pixels scaled from its 508 px screen to a
//! 320 px one (× 0.63).

use crate::canvas::{sd_rrect, Canvas, Rect};
use crate::color::{Color, Tokens, Tone};

const THICKNESS: f32 = 16.0; // 26
const BEZEL_MIN: f32 = 5.0; // 8
const BEZEL_MAX: f32 = 18.0; // 28
const MERGE: f32 = 10.0; // 16
const SHADOW_OFFSET: f32 = 3.0; // 5
const FROST_RADIUS: i32 = 4; // three box passes ≈ a 4.5 px Gaussian (regular frost 10 × 0.8 × 0.63)
const REACH: f32 = 9.0; // how far outside a shape its shadow can show
const IOR: f32 = 1.5;
const DISPERSION: f32 = 0.03;
const SATURATION: f32 = 1.12;
const CLEAR_DIMMING: f32 = 0.35;

/// Where the light rests: above and to the left (y points down).
pub const LIGHT_REST: (f32, f32) = (-0.42, -0.91);

#[derive(Clone, Copy, Debug)]
pub struct GlassShape {
    pub rect: Rect,
    pub radius: f32,
    /// Shapes melt together only inside their own group.
    pub group: u8,
    /// 0 → 1 as the glass materializes: refraction, frost and rim together.
    pub strength: f32,
    /// Coloured glass, for a selected action.
    pub tint: Option<Color>,
    /// Clear glass, for controls over media.
    pub clear: bool,
}

impl GlassShape {
    pub fn new(rect: Rect, radius: f32, group: u8) -> Self {
        GlassShape { rect, radius, group, strength: 1.0, tint: None, clear: false }
    }
}

#[derive(Default)]
pub struct Glass {
    scene: Vec<u32>,
    frost: Vec<u32>,
    tmp: Vec<u32>,
}

#[inline]
fn unpack(p: u32) -> [f32; 3] {
    [((p >> 16) & 0xff) as f32, ((p >> 8) & 0xff) as f32, (p & 0xff) as f32]
}

#[inline]
fn bilinear(buf: &[u32], w: i32, h: i32, x: f32, y: f32) -> [f32; 3] {
    let x = (x - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (y - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as i32, y as i32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let a = unpack(buf[(y0 * w + x0) as usize]);
    let b = unpack(buf[(y0 * w + x1) as usize]);
    let c = unpack(buf[(y1 * w + x0) as usize]);
    let d = unpack(buf[(y1 * w + x1) as usize]);
    let mut out = [0.0; 3];
    for i in 0..3 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bottom - top) * fy;
    }
    out
}

#[inline]
fn luma(c: &[f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// One box-blur pass along x or y over a region, clamping at its edges.
fn box_pass(src: &[u32], dst: &mut [u32], stride: i32, (x0, y0, x1, y1): (i32, i32, i32, i32), r: i32, horizontal: bool) {
    let (outer, inner, step_o, step_i, o0, i0) = if horizontal {
        (y1 - y0, x1 - x0, stride, 1, y0, x0)
    } else {
        (x1 - x0, y1 - y0, 1, stride, x0, y0)
    };
    let n = (2 * r + 1) as u32;
    for o in 0..outer {
        let base = if horizontal { (o0 + o) * step_o + i0 } else { i0 * step_i + (o0 + o) };
        let at = |i: i32| src[(base + i.clamp(0, inner - 1) * step_i) as usize];
        let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
        for i in -r..=r {
            let p = at(i);
            sr += (p >> 16) & 0xff;
            sg += (p >> 8) & 0xff;
            sb += p & 0xff;
        }
        for i in 0..inner {
            dst[(base + i * step_i) as usize] = (sr / n) << 16 | (sg / n) << 8 | (sb / n);
            let (add, sub) = (at(i + r + 1), at(i - r));
            sr = sr + ((add >> 16) & 0xff) - ((sub >> 16) & 0xff);
            sg = sg + ((add >> 8) & 0xff) - ((sub >> 8) & 0xff);
            sb = sb + (add & 0xff) - (sub & 0xff);
        }
    }
}

/// Smooth minimum: shapes closer than `k` melt together.
#[inline]
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

impl Glass {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draw `shapes` as glass over what is already on the canvas. `light` is
    /// the direction the light comes from. `calm` draws the reduced-
    /// transparency version: solid `surface2` that keeps its edge light.
    pub fn render(&mut self, canvas: &mut Canvas, shapes: &[GlassShape], tokens: &Tokens, light: (f32, f32), calm: bool) {
        let live: Vec<&GlassShape> = shapes.iter().filter(|s| s.strength > 0.004 && s.rect.w > 0.5 && s.rect.h > 0.5).collect();
        if live.is_empty() {
            return;
        }
        let (w, h) = (canvas.w, canvas.h);
        let dark = tokens.tone == Tone::Dark;
        let light_len = (light.0 * light.0 + light.1 * light.1).sqrt().max(1e-6);
        let light = (light.0 / light_len, light.1 / light_len);

        // Snapshot the scene and frost the part any glass can reach.
        self.scene.clone_from(&canvas.px);
        self.frost.resize(self.scene.len(), 0);
        self.tmp.resize(self.scene.len(), 0);
        let pad = REACH + THICKNESS;
        let (mut rx0, mut ry0, mut rx1, mut ry1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for s in &live {
            rx0 = rx0.min(s.rect.x - pad);
            ry0 = ry0.min(s.rect.y - pad);
            rx1 = rx1.max(s.rect.right() + pad);
            ry1 = ry1.max(s.rect.bottom() + pad);
        }
        let region = (
            (rx0.floor() as i32).clamp(0, w),
            (ry0.floor() as i32).clamp(0, h),
            (rx1.ceil() as i32).clamp(0, w),
            (ry1.ceil() as i32).clamp(0, h),
        );
        if region.0 >= region.2 || region.1 >= region.3 {
            return;
        }
        box_pass(&self.scene, &mut self.tmp, w, region, FROST_RADIUS, true);
        box_pass(&self.tmp, &mut self.frost, w, region, FROST_RADIUS, false);
        for _ in 0..2 {
            box_pass(&self.frost, &mut self.tmp, w, region, FROST_RADIUS, true);
            box_pass(&self.tmp, &mut self.frost, w, region, FROST_RADIUS, false);
        }

        let mut groups: Vec<u8> = live.iter().map(|s| s.group).collect();
        groups.sort_unstable();
        groups.dedup();

        for group in groups {
            let members: Vec<&GlassShape> = live.iter().copied().filter(|s| s.group == group).collect();
            // The merged field, and the member a point belongs to.
            let field = |x: f32, y: f32| -> (f32, usize) {
                let mut d = f32::MAX;
                let mut nearest = (f32::MAX, 0usize);
                for (i, s) in members.iter().enumerate() {
                    let di = sd_rrect(x, y, &s.rect, s.radius);
                    if di < nearest.0 {
                        nearest = (di, i);
                    }
                    d = if i == 0 { di } else { smin(d, di, MERGE) };
                }
                (d, nearest.1)
            };

            let (mut gx0, mut gy0, mut gx1, mut gy1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            let mut area = 0.0f32;
            for s in &members {
                gx0 = gx0.min(s.rect.x - REACH);
                gy0 = gy0.min(s.rect.y - REACH);
                gx1 = gx1.max(s.rect.right() + REACH);
                gy1 = gy1.max(s.rect.bottom() + REACH + SHADOW_OFFSET);
                area += s.rect.w * s.rect.h;
            }
            // Bigger glass casts a deeper shadow.
            let size_scale = 0.6 + 0.4 * (area / 5000.0).min(1.0);

            let x_range = (gx0.floor() as i32).max(0)..(gx1.ceil() as i32).min(w);
            for y in (gy0.floor() as i32).max(0)..(gy1.ceil() as i32).min(h) {
                for x in x_range.clone() {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let (d, who) = field(fx, fy);
                    if d > REACH {
                        continue;
                    }
                    let shape = members[who];
                    let strength = shape.strength.clamp(0.0, 1.0);
                    let idx = (y * w + x) as usize;
                    let under = unpack(self.scene[idx]);

                    // Outside: the adaptive shadow, darker over busy content.
                    let cover = (0.5 - d / 1.5).clamp(0.0, 1.0);
                    let mut outside = under;
                    if cover < 1.0 {
                        let ds = field(fx, fy - SHADOW_OFFSET).0;
                        let fall = (1.0 - (ds + 2.0) / (REACH - 1.0)).clamp(0.0, 1.0);
                        if fall > 0.0 {
                            let busy = ((luma(&under) - luma(&unpack(self.frost[idx]))).abs() / 48.0).min(1.0);
                            let k = 1.0 - (0.08 + 0.12 * busy) * size_scale * fall * fall * strength;
                            outside = [under[0] * k, under[1] * k, under[2] * k];
                        }
                    }
                    if cover <= 0.0 {
                        canvas.px[idx] = pack(&outside);
                        continue;
                    }

                    // Inside: where on the bezel is this pixel, and which way does it face?
                    let depth = (-d).max(0.0);
                    let bezel = (0.9 * shape.radius)
                        .clamp(BEZEL_MIN, BEZEL_MAX)
                        .min(shape.rect.w.min(shape.rect.h) * 0.5);
                    let t = (depth / bezel).min(1.0);
                    let (mut nx, mut ny) = (0.0f32, 0.0f32);
                    let mut offset = 0.0f32;
                    if t < 1.0 {
                        let gx = field(fx + 1.0, fy).0 - field(fx - 1.0, fy).0;
                        let gy = field(fx, fy + 1.0).0 - field(fx, fy - 1.0).0;
                        let len = (gx * gx + gy * gy).sqrt().max(1e-5);
                        nx = gx / len;
                        ny = gy / len;
                        // Squircle profile y = (1 − (1 − t)⁴)^¼ and its slope.
                        let m = 1.0 - t;
                        let q = (1.0 - m * m * m * m).max(1e-4);
                        let height = q.sqrt().sqrt();
                        let slope = (m * m * m / (q.sqrt() * q.sqrt().sqrt())) * (THICKNESS / bezel);
                        let incident = slope.min(12.0).atan();
                        let refracted = (incident.sin() / IOR).asin();
                        let slab = THICKNESS * (0.3 + 0.7 * height);
                        offset = slab * (incident - refracted).tan() * strength;
                    }

                    let mut c = if calm {
                        [tokens.surface2.0 as f32, tokens.surface2.1 as f32, tokens.surface2.2 as f32]
                    } else {
                        // The rim bends light inward: sample from deeper inside the shape.
                        let (sx, sy) = (fx - nx * offset, fy - ny * offset);
                        let frost_mix = strength * if shape.clear { 0.12 } else { 1.0 };
                        let read = |buf_sharp: &[u32], buf_frost: &[u32], px: f32, py: f32| -> [f32; 3] {
                            let f = bilinear(buf_frost, w, h, px, py);
                            if frost_mix >= 0.999 {
                                return f;
                            }
                            let s = bilinear(buf_sharp, w, h, px, py);
                            [s[0] + (f[0] - s[0]) * frost_mix, s[1] + (f[1] - s[1]) * frost_mix, s[2] + (f[2] - s[2]) * frost_mix]
                        };
                        let mut c = read(&self.scene, &self.frost, sx, sy);
                        if offset > 1.5 {
                            // Dispersion: red sampled farther along the refraction, blue nearer.
                            let far = offset * (1.0 + DISPERSION);
                            let near = offset * (1.0 - DISPERSION);
                            c[0] = read(&self.scene, &self.frost, fx - nx * far, fy - ny * far)[0];
                            c[2] = read(&self.scene, &self.frost, fx - nx * near, fy - ny * near)[2];
                        }
                        // Even the brightness toward the panel's tone, then lift saturation.
                        let (grey, amount) = if dark { (0.17 * 255.0, 0.22) } else { (0.93 * 255.0, 0.38) };
                        let amount = amount * strength * if shape.clear { 0.0 } else { 1.0 };
                        let l = luma(&c);
                        for v in &mut c {
                            *v += (grey - *v) * amount;
                            *v = l + (*v - l) * (1.0 + (SATURATION - 1.0) * strength);
                        }
                        if shape.clear {
                            let k = 1.0 - CLEAR_DIMMING * strength;
                            c = [c[0] * k, c[1] * k, c[2] * k];
                        }
                        c
                    };

                    // Coloured glass keeps following the brightness behind it.
                    if let Some(tint) = shape.tint {
                        let k = 0.82 + 0.35 * (luma(&c) / 255.0);
                        let mix = 0.82 * strength;
                        c[0] += (tint.0 as f32 * k - c[0]) * mix;
                        c[1] += (tint.1 as f32 * k - c[1]) * mix;
                        c[2] += (tint.2 as f32 * k - c[2]) * mix;
                    }

                    // One light: a thin edge line brightest where it faces the
                    // light, a fainter return opposite, a soft band just inside.
                    if t < 1.0 {
                        let facing = nx * light.0 + ny * light.1;
                        let line = (1.0 - depth / 1.3).clamp(0.0, 1.0);
                        let band = (1.0 - t) * (1.0 - t) * 0.05;
                        let lit = (0.16 + 0.62 * facing.max(0.0) + 0.26 * (-facing).max(0.0)) * line + band;
                        let lit = lit * strength;
                        if dark {
                            for v in &mut c {
                                *v += (255.0 - *v) * lit;
                            }
                        } else {
                            // In light mode the side facing away darkens slightly.
                            let shade = 0.10 * (-facing).max(0.0) * line * strength;
                            for v in &mut c {
                                *v += (255.0 - *v) * lit;
                                *v *= 1.0 - shade;
                            }
                        }
                    }

                    let out = [
                        outside[0] + (c[0] - outside[0]) * cover,
                        outside[1] + (c[1] - outside[1]) * cover,
                        outside[2] + (c[2] - outside[2]) * cover,
                    ];
                    canvas.px[idx] = pack(&out);
                }
            }
        }
    }
}

#[inline]
fn pack(c: &[f32; 3]) -> u32 {
    let q = |v: f32| v.clamp(0.0, 255.0) as u32;
    q(c[0]) << 16 | q(c[1]) << 8 | q(c[2])
}
