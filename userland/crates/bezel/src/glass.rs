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

// Public so a GPU renderer can draw the same glass from the same numbers.
pub const THICKNESS: f32 = 16.0; // 26
pub const BEZEL_MIN: f32 = 5.0; // 8
pub const BEZEL_MAX: f32 = 18.0; // 28
pub const MERGE: f32 = 10.0; // 16
pub const SHADOW_OFFSET: f32 = 3.0; // 5
pub const FROST_RADIUS: i32 = 4; // three box passes ≈ a 4.5 px Gaussian (regular frost 10 × 0.8 × 0.63)
pub const REACH: f32 = 9.0; // how far outside a shape its shadow can show
pub const IOR: f32 = 1.5;
pub const DISPERSION: f32 = 0.03;
pub const SATURATION: f32 = 1.12;
pub const CLEAR_DIMMING: f32 = 0.35;

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
    frost: Frost,
    /// Room for frosting a patch, kept between frames.
    work: (Vec<u64>, Vec<u64>, Vec<u64>),
    /// What each group of glass was last drawn over, and how it came out.
    /// Glass is by far the dearest thing on screen and most frames change
    /// nothing under it, so a group whose shapes and whose patch of the scene
    /// are both as they were is copied back instead of worked out again.
    kept: Vec<Kept>,
}

/// One group's last render: what decided it, the scene it was drawn over,
/// and the result, for the patch of the screen the group can touch.
struct Kept {
    group: u8,
    key: Vec<u32>,
    region: Region,
    before: Vec<u32>,
    after: Vec<u32>,
}

/// A patch of the screen: x0, y0, x1, y1.
type Region = (i32, i32, i32, i32);

/// The pixels of a patch, row by row.
fn gather(buf: &[u32], stride: i32, (x0, y0, x1, y1): Region) -> Vec<u32> {
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0)).max(0) as usize);
    for y in y0..y1 {
        out.extend_from_slice(&buf[(y * stride + x0) as usize..(y * stride + x1) as usize]);
    }
    out
}

/// Put a patch's pixels back.
fn scatter(buf: &mut [u32], stride: i32, (x0, y0, x1, y1): Region, pixels: &[u32]) {
    let w = (x1 - x0) as usize;
    for (row, y) in (y0..y1).enumerate() {
        buf[(y * stride + x0) as usize..(y * stride + x1) as usize].copy_from_slice(&pixels[row * w..(row + 1) * w]);
    }
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

/// The three channels of a pixel spread 20 bits apart in one integer, so a
/// window's three sums are kept, added to and taken from as one. Nine 8-bit
/// values sum to under 2¹², so a lane never carries into the next.
#[inline]
fn spread(p: u32) -> u64 {
    let p = p as u64;
    (p & 0xff) | (p & 0xff00) << 12 | (p & 0xff_0000) << 24
}

#[inline]
fn unspread(v: u64) -> u32 {
    ((v & 0xff) | (v >> 12) & 0xff00 | (v >> 24) & 0xff_0000) as u32
}

const LANE: u64 = (1 << 20) - 1;

/// A window's sums divided by its size, `inv` being 65536 / size + 1: the
/// same result for every sum three 8-bit channels can make, without a
/// division per channel per pixel.
#[inline]
fn mean(sum: u64, inv: u64) -> u64 {
    ((sum & LANE) * inv) >> 16 | (((sum >> 20) & LANE) * inv) >> 16 << 20 | (((sum >> 40) & LANE) * inv) >> 16 << 40
}

/// Three box blurs of radius `r` each way over a `w` × `h` block of spread
/// pixels in `a`, clamping at its edges; the result is left in `a`. The
/// vertical passes go along rows with a running sum per column, so every
/// read is close to the one before.
fn blur_block(a: &mut [u64], w: usize, h: usize, r: i32, b: &mut Vec<u64>, sums: &mut Vec<u64>) {
    if w == 0 || h == 0 {
        return;
    }
    b.resize(w * h, 0);
    let inv = (65536 / (2 * r + 1) + 1) as u64;
    let r = r as isize;
    let clamp = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    for _ in 0..3 {
        // Along each row, a → b.
        for (row_in, row_out) in a.chunks_exact(w).zip(b.chunks_exact_mut(w)) {
            let mut sum: u64 = (-r..=r).map(|i| row_in[clamp(i, w)]).sum();
            for (i, out) in row_out.iter_mut().enumerate() {
                *out = mean(sum, inv);
                let i = i as isize;
                sum = sum + row_in[clamp(i + r + 1, w)] - row_in[clamp(i - r, w)];
            }
        }
        // Down each column, b → a, a row at a time.
        sums.clear();
        sums.resize(w, 0);
        for i in -r..=r {
            let row = &b[clamp(i, h) * w..][..w];
            sums.iter_mut().zip(row).for_each(|(s, &v)| *s += v);
        }
        for y in 0..h {
            let (add, sub) = (clamp(y as isize + r + 1, h) * w, clamp(y as isize - r, h) * w);
            let out = &mut a[y * w..][..w];
            for x in 0..w {
                out[x] = mean(sums[x], inv);
                sums[x] = sums[x] + b[add + x] - b[sub + x];
            }
        }
    }
}

/// Frost a patch at full resolution: `blur_block` read from `src` and written
/// to `dst`, both the whole screen, `stride` wide. Only the tests use it now,
/// to hold `blur_block` to six plain box passes exactly.
#[cfg(test)]
fn frost(src: &[u32], dst: &mut [u32], stride: i32, (x0, y0, x1, y1): Region, r: i32, a: &mut Vec<u64>, b: &mut Vec<u64>, sums: &mut Vec<u64>) {
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    a.clear();
    for y in y0..y1 {
        a.extend(src[(y * stride + x0) as usize..(y * stride + x1) as usize].iter().map(|&p| spread(p)));
    }
    blur_block(a, w, h, r, b, sums);
    for (row, y) in a.chunks_exact(w.max(1)).zip(y0..y1) {
        let base = (y * stride + x0) as usize;
        dst[base..base + w].iter_mut().zip(row).for_each(|(d, &v)| *d = unspread(v));
    }
}

/// The frosted scene under one group of glass, kept at half the panel's
/// resolution: frost is a wide blur, so a quarter of the pixels carry all of
/// it, and blurring them costs a third as much.
#[derive(Default)]
struct Frost {
    px: Vec<u32>,
    /// The patch it covers, in panel pixels, and its size in its own.
    x0: f32,
    y0: f32,
    w: i32,
    h: i32,
}

/// Each lane's low eight bits.
const LANE_BYTES: u64 = 0xff | 0xff << 20 | 0xff << 40;

impl Frost {
    /// Frost `region` of `scene` (the whole screen, `stride` wide): average
    /// each 2 × 2 block, then three box blurs of radius 2 each way, which at
    /// half size is about the full-size blur of radius 4 the specimen asks for.
    fn make(&mut self, scene: &[u32], stride: i32, (x0, y0, x1, y1): Region, a: &mut Vec<u64>, b: &mut Vec<u64>, sums: &mut Vec<u64>) {
        let (w, h) = (((x1 - x0 + 1) / 2) as usize, ((y1 - y0 + 1) / 2) as usize);
        (self.x0, self.y0, self.w, self.h) = (x0 as f32, y0 as f32, w as i32, h as i32);
        a.clear();
        for j in 0..h as i32 {
            let (top, bottom) = (y0 + 2 * j, (y0 + 2 * j + 1).min(y1 - 1));
            for i in 0..w as i32 {
                let (left, right) = (x0 + 2 * i, (x0 + 2 * i + 1).min(x1 - 1));
                let at = |x: i32, y: i32| spread(scene[(y * stride + x) as usize]);
                let sum = at(left, top) + at(right, top) + at(left, bottom) + at(right, bottom);
                a.push((sum >> 2) & LANE_BYTES);
            }
        }
        blur_block(a, w, h, FROST_RADIUS / 2, b, sums);
        self.px.clear();
        self.px.extend(a.iter().map(|&v| unspread(v)));
    }

    /// The frost at a point of the panel, between its pixels.
    #[inline]
    fn at(&self, x: f32, y: f32) -> [f32; 3] {
        bilinear(&self.px, self.w, self.h, (x - self.x0) * 0.5, (y - self.y0) * 0.5)
    }

    /// The frost over a panel pixel, near enough.
    #[inline]
    fn near(&self, x: i32, y: i32) -> [f32; 3] {
        let i = ((x as f32 - self.x0) as i32 / 2).clamp(0, self.w - 1);
        let j = ((y as f32 - self.y0) as i32 / 2).clamp(0, self.h - 1);
        unpack(self.px[(j * self.w + i) as usize])
    }
}

/// One box-blur pass along x or y over a region, clamping at its edges: the
/// straightforward version, which `frost` must match exactly.
#[cfg(test)]
fn box_pass(src: &[u32], dst: &mut [u32], stride: i32, (x0, y0, x1, y1): (i32, i32, i32, i32), r: i32, horizontal: bool) {
    let (outer, inner, step_o, step_i, o0, i0) = if horizontal {
        (y1 - y0, x1 - x0, stride, 1, y0, x0)
    } else {
        (x1 - x0, y1 - y0, 1, stride, x0, y0)
    };
    // Dividing by the window (9, for a radius of 4) as a multiplication: the
    // same result for every sum three 8-bit channels can make, without a
    // division per channel per pixel.
    let n = (2 * r + 1) as u32;
    let inv = 65536 / n + 1;
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
            dst[(base + i * step_i) as usize] = ((sr * inv) >> 16) << 16 | ((sg * inv) >> 16) << 8 | ((sb * inv) >> 16);
            let (add, sub) = (at(i + r + 1), at(i - r));
            sr = sr + ((add >> 16) & 0xff) - ((sub >> 16) & 0xff);
            sg = sg + ((add >> 8) & 0xff) - ((sub >> 8) & 0xff);
            sb = sb + (add & 0xff) - (sub & 0xff);
        }
    }
}

/// How finely the bezel's bend is tabulated across its depth.
const BEND_STEPS: usize = 128;

/// Which way a rounded rectangle's edge faces at a point: the direction its
/// distance field grows fastest. Averaged over a pixel either side of where
/// the nearest edge changes, as measuring the field would give.
#[inline]
fn outward(px: f32, py: f32, r: &Rect, radius: f32) -> (f32, f32) {
    let radius = radius.min(r.w * 0.5).min(r.h * 0.5);
    let (dx, dy) = (px - r.cx(), py - r.cy());
    let qx = dx.abs() - (r.w * 0.5 - radius);
    let qy = dy.abs() - (r.h * 0.5 - radius);
    let (sx, sy) = (if dx < 0.0 { -1.0 } else { 1.0 }, if dy < 0.0 { -1.0 } else { 1.0 });
    // Round a corner: straight out from the corner's centre.
    let (cx, cy) = (qx.max(0.0), qy.max(0.0));
    if cx > 0.0 && cy > 0.0 {
        let len = (cx * cx + cy * cy).sqrt();
        return (sx * cx / len, sy * cy / len);
    }
    // Along a straight edge: straight out from it, easing between the two
    // edges where they are equally near (the shape's spine).
    let lean = ((qx - qy) * 0.5).clamp(-1.0, 1.0) * 0.5 + 0.5;
    let (nx, ny) = (sx * lean, sy * (1.0 - lean));
    let len = (nx * nx + ny * ny).sqrt().max(1e-5);
    (nx / len, ny / len)
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

        let mut groups: Vec<u8> = live.iter().map(|s| s.group).collect();
        groups.sort_unstable();
        groups.dedup();

        // The patch each group can reach: its shapes, their shadows, and as
        // far beyond as the frost under them draws from.
        let pad = REACH + THICKNESS;
        let regions: Vec<Region> = groups
            .iter()
            .map(|group| {
                let (mut rx0, mut ry0, mut rx1, mut ry1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for s in live.iter().filter(|s| s.group == *group) {
                    rx0 = rx0.min(s.rect.x - pad);
                    ry0 = ry0.min(s.rect.y - pad);
                    rx1 = rx1.max(s.rect.right() + pad);
                    ry1 = ry1.max(s.rect.bottom() + pad);
                }
                let (x0, y0) = ((rx0.floor() as i32).clamp(0, w), (ry0.floor() as i32).clamp(0, h));
                (x0, y0, (rx1.ceil() as i32).clamp(x0, w), (ry1.ceil() as i32).clamp(y0, h))
            })
            .collect();

        // Snapshot the scene under every group before any glass is drawn:
        // glass refracts the scene, never other glass.
        self.scene.resize(canvas.px.len(), 0);
        for &(x0, y0, x1, y1) in &regions {
            for y in y0..y1 {
                let row = (y * w + x0) as usize..(y * w + x1) as usize;
                self.scene[row.clone()].copy_from_slice(&canvas.px[row]);
            }
        }
        // Only what was drawn this time is worth keeping for the next.
        let mut kept_before = std::mem::take(&mut self.kept);

        for (group, region) in groups.into_iter().zip(regions) {
            if region.0 >= region.2 || region.1 >= region.3 {
                continue;
            }
            let members: Vec<&GlassShape> = live.iter().copied().filter(|s| s.group == group).collect();

            // Everything the result depends on besides the scene itself.
            let mut key: Vec<u32> = vec![dark as u32, calm as u32, light.0.to_bits(), light.1.to_bits(), tokens.surface2.to_u32()];
            for s in &members {
                key.extend([s.rect.x, s.rect.y, s.rect.w, s.rect.h, s.radius, s.strength].map(f32::to_bits));
                key.extend([s.tint.map_or(u32::MAX, |c| c.to_u32()), s.clear as u32]);
            }
            let before = gather(&self.scene, w, region);
            if let Some(at) = kept_before.iter().position(|k| k.group == group) {
                let last = kept_before.swap_remove(at);
                if last.key == key && last.region == region && last.before == before {
                    // Nothing under this glass has changed: neither has the glass.
                    scatter(&mut canvas.px, w, region, &last.after);
                    self.kept.push(last);
                    continue;
                }
            }

            // Frost the patch.
            self.frost.make(&self.scene, w, region, &mut self.work.0, &mut self.work.1, &mut self.work.2);
            let frost = &self.frost;

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

            // How far each shape's bezel bends light, by depth into the bezel:
            // the squircle profile's slope, refracted at the glass's index.
            // It depends only on the depth, so it is tabulated per shape
            // rather than worked out (four square roots and four
            // trigonometric calls) for every pixel.
            let bends: Vec<(f32, [f32; BEND_STEPS + 1])> = members
                .iter()
                .map(|shape| {
                    let bezel = (0.9 * shape.radius).clamp(BEZEL_MIN, BEZEL_MAX).min(shape.rect.w.min(shape.rect.h) * 0.5);
                    let mut table = [0.0f32; BEND_STEPS + 1];
                    for (i, entry) in table.iter_mut().enumerate() {
                        let t = i as f32 / BEND_STEPS as f32;
                        // Squircle profile y = (1 − (1 − t)⁴)^¼ and its slope.
                        let m = 1.0 - t;
                        let q = (1.0 - m * m * m * m).max(1e-4);
                        let height = q.sqrt().sqrt();
                        let slope = (m * m * m / (q.sqrt() * q.sqrt().sqrt())) * (THICKNESS / bezel);
                        let incident = slope.min(12.0).atan();
                        let refracted = (incident.sin() / IOR).asin();
                        let slab = THICKNESS * (0.3 + 0.7 * height);
                        *entry = slab * (incident - refracted).tan();
                    }
                    (bezel, table)
                })
                .collect();
            // One shape alone has a distance field whose slope is known
            // exactly; shapes melted together need it measured.
            let alone = members.len() == 1;

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
                            let busy = ((luma(&under) - luma(&frost.near(x, y))).abs() / 48.0).min(1.0);
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
                    let (bezel, bend) = &bends[who];
                    let t = (depth / bezel).min(1.0);
                    let (mut nx, mut ny) = (0.0f32, 0.0f32);
                    let mut offset = 0.0f32;
                    if t < 1.0 {
                        (nx, ny) = if alone {
                            outward(fx, fy, &shape.rect, shape.radius)
                        } else {
                            let gx = field(fx + 1.0, fy).0 - field(fx - 1.0, fy).0;
                            let gy = field(fx, fy + 1.0).0 - field(fx, fy - 1.0).0;
                            let len = (gx * gx + gy * gy).sqrt().max(1e-5);
                            (gx / len, gy / len)
                        };
                        // Between the two tabulated depths either side.
                        let at = t * BEND_STEPS as f32;
                        let i = at as usize;
                        offset = (bend[i] + (bend[i + 1] - bend[i]) * (at - i as f32)) * strength;
                    }

                    let mut c = if calm {
                        [tokens.surface2.0 as f32, tokens.surface2.1 as f32, tokens.surface2.2 as f32]
                    } else {
                        // The rim bends light inward: sample from deeper inside the shape.
                        let (sx, sy) = (fx - nx * offset, fy - ny * offset);
                        let frost_mix = strength * if shape.clear { 0.12 } else { 1.0 };
                        let scene = &self.scene;
                        let read = |px: f32, py: f32| -> [f32; 3] {
                            let f = frost.at(px, py);
                            if frost_mix >= 0.999 {
                                return f;
                            }
                            let s = bilinear(scene, w, h, px, py);
                            [s[0] + (f[0] - s[0]) * frost_mix, s[1] + (f[1] - s[1]) * frost_mix, s[2] + (f[2] - s[2]) * frost_mix]
                        };
                        let mut c = read(sx, sy);
                        if offset > 1.5 {
                            // Dispersion: red sampled farther along the refraction, blue nearer.
                            let far = offset * (1.0 + DISPERSION);
                            let near = offset * (1.0 - DISPERSION);
                            c[0] = read(fx - nx * far, fy - ny * far)[0];
                            c[2] = read(fx - nx * near, fy - ny * near)[2];
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
            let after = gather(&canvas.px, w, region);
            self.kept.push(Kept { group, key, region, before, after });
        }
    }
}

#[inline]
fn pack(c: &[f32; 3]) -> u32 {
    let q = |v: f32| v.clamp(0.0, 255.0) as u32;
    q(c[0]) << 16 | q(c[1]) << 8 | q(c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene with something under the glass worth refracting.
    fn scene() -> Canvas {
        let mut canvas = Canvas::new(320, 170);
        canvas.clear(Color::hex(0x101418));
        for i in 0..12 {
            canvas.rrect(Rect::new(10.0 + i as f32 * 26.0, 4.0 + (i % 3) as f32 * 7.0, 20.0, 12.0), 4.0, Color::hex(0xff5a1f + i * 0x0a1420), 1.0);
            canvas.rrect(Rect::new(20.0 + i as f32 * 24.0, 150.0, 14.0, 14.0), 7.0, Color::hex(0x7ad0ff), 1.0);
        }
        canvas
    }

    fn shapes() -> [GlassShape; 3] {
        [
            GlassShape::new(Rect::new(110.0, 4.0, 100.0, 19.0), 9.5, 0),
            GlassShape::new(Rect::new(140.0, 157.0, 43.0, 10.0), 5.0, 1),
            GlassShape::new(Rect::new(190.0, 157.0, 30.0, 10.0), 5.0, 1),
        ]
    }

    /// Where the glass pass spends its time, over a scene that changes every
    /// frame (so nothing is kept): `cargo test --profile sim -p bezel -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn where_the_time_goes() {
        let tokens = Tokens::dark();
        let island = [GlassShape::new(Rect::new(110.0, 4.0, 100.0, 19.0), 9.5, 0)];
        let base = scene();
        let n = 2000;
        let mut glass = Glass::new();
        let started = std::time::Instant::now();
        for i in 0..n {
            let mut canvas = Canvas::new(320, 170);
            canvas.px.copy_from_slice(&base.px);
            canvas.px[(i % 320) + 320 * 10] ^= 0xff;
            glass.render(&mut canvas, &island, &tokens, LIGHT_REST, false);
        }
        let total = started.elapsed().as_secs_f64() * 1e6 / n as f64;
        let pad = REACH + THICKNESS;
        let region = ((110.0 - pad) as i32, 0, (210.0 + pad).ceil() as i32, (23.0 + pad).ceil() as i32);
        let mut half = Frost::default();
        let mut work = (Vec::new(), Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        for _ in 0..n {
            half.make(&base.px, 320, region, &mut work.0, &mut work.1, &mut work.2);
        }
        let frosted = started.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("island over a changing scene: {total:.1} µs a frame, of which frost {frosted:.1} µs; region {region:?}");
    }

    #[test]
    fn frost_matches_six_plain_box_passes_exactly() {
        // A busy scene: hard edges, every channel, the panel's own edges.
        let mut canvas = scene();
        for (i, p) in canvas.px.iter_mut().enumerate() {
            if i % 7 == 0 || i % 13 == 0 {
                *p = (i as u32).wrapping_mul(2_654_435_761) & 0xff_ffff;
            }
        }
        for region in [(85, 0, 235, 48), (0, 0, 320, 170), (300, 160, 320, 170), (10, 10, 13, 11)] {
            let (mut a, mut b) = (canvas.px.clone(), canvas.px.clone());
            box_pass(&canvas.px, &mut a, 320, region, FROST_RADIUS, true);
            box_pass(&a, &mut b, 320, region, FROST_RADIUS, false);
            for _ in 0..2 {
                box_pass(&b, &mut a, 320, region, FROST_RADIUS, true);
                box_pass(&a, &mut b, 320, region, FROST_RADIUS, false);
            }
            let mut out = canvas.px.clone();
            let mut work = (Vec::new(), Vec::new(), Vec::new());
            frost(&canvas.px, &mut out, 320, region, FROST_RADIUS, &mut work.0, &mut work.1, &mut work.2);
            let (x0, y0, x1, y1) = region;
            for y in y0..y1 {
                let row = (y * 320 + x0) as usize..(y * 320 + x1) as usize;
                assert_eq!(out[row.clone()], b[row], "row {y} of {region:?}");
            }
        }
    }

    #[test]
    fn frost_at_half_size_looks_like_frost_at_full_size() {
        // Busy content: hard-edged blocks of colour, as text and tiles are.
        let mut canvas = scene();
        for y in 0..60 {
            for x in 80..240 {
                if (x / 3 + y / 5) % 2 == 0 {
                    canvas.px[(y * 320 + x) as usize] = 0xf0_e0_d0;
                }
            }
        }
        let region = (85, 0, 235, 48);
        let mut full = canvas.px.clone();
        let mut work = (Vec::new(), Vec::new(), Vec::new());
        frost(&canvas.px, &mut full, 320, region, FROST_RADIUS, &mut work.0, &mut work.1, &mut work.2);
        let mut half = Frost::default();
        half.make(&canvas.px, 320, region, &mut work.0, &mut work.1, &mut work.2);
        // Away from the patch's edges, where both clamp differently.
        let mut worst = 0.0f32;
        for y in 12..36 {
            for x in 97..223 {
                let (a, b) = (unpack(full[(y * 320 + x) as usize]), half.at(x as f32 + 0.5, y as f32 + 0.5));
                for i in 0..3 {
                    worst = worst.max((a[i] - b[i]).abs());
                }
            }
        }
        assert!(worst < 12.0, "no more than a few levels apart anywhere: {worst}");
    }

    #[test]
    fn glass_over_an_unchanged_scene_is_kept_and_comes_out_the_same() {
        let tokens = Tokens::dark();
        let mut glass = Glass::new();
        let mut first = scene();
        glass.render(&mut first, &shapes(), &tokens, LIGHT_REST, false);
        assert_ne!(first.px, scene().px, "the glass drew something");
        // The same scene again, through the same Glass: taken from what was kept.
        let mut again = scene();
        glass.render(&mut again, &shapes(), &tokens, LIGHT_REST, false);
        assert_eq!(again.px, first.px);
        // And it is what a Glass with nothing kept makes of it.
        let mut fresh = scene();
        Glass::new().render(&mut fresh, &shapes(), &tokens, LIGHT_REST, false);
        assert_eq!(fresh.px, first.px);
    }

    #[test]
    fn a_change_under_one_group_redraws_that_group_and_only_what_it_reaches() {
        let tokens = Tokens::dark();
        let mut glass = Glass::new();
        let mut first = scene();
        glass.render(&mut first, &shapes(), &tokens, LIGHT_REST, false);
        // Something new under the island; nothing near the pager.
        let changed = || {
            let mut canvas = scene();
            canvas.rrect(Rect::new(150.0, 8.0, 30.0, 10.0), 3.0, Color::hex(0xffffff), 1.0);
            canvas
        };
        let mut second = changed();
        glass.render(&mut second, &shapes(), &tokens, LIGHT_REST, false);
        let mut fresh = changed();
        Glass::new().render(&mut fresh, &shapes(), &tokens, LIGHT_REST, false);
        assert_eq!(second.px, fresh.px, "kept glass must never show a stale scene");
        assert_ne!(second.px[..320 * 60], first.px[..320 * 60], "the island shows what is now under it");
        assert_eq!(second.px[320 * 120..], first.px[320 * 120..], "the pager is as it was");
        // A shape that moved is redrawn even over the same scene.
        let mut moved = shapes();
        moved[0].rect.x += 3.0;
        let mut third = scene();
        glass.render(&mut third, &moved, &tokens, LIGHT_REST, false);
        let mut fresh = scene();
        Glass::new().render(&mut fresh, &moved, &tokens, LIGHT_REST, false);
        assert_eq!(third.px, fresh.px);
    }
}
