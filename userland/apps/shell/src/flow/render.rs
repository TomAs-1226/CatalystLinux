//! Flow drawn on the processor, exactly as the GPU draws it (`gpu.rs`,
//! `FLOW_GROUND` and `FLOW_CARD`), so either may draw any frame.
//!
//! A card stands upright and turns only about its vertical axis, so under
//! perspective each column of the panel meets it at one depth: the column
//! reads one column of the card's picture, scaled straight down it. That is
//! worked out once a column, and a pixel costs one or two bilinear reads.
//! The GPU does the same sums a pixel at a time.

use bezel::Color;

use super::art::Atlas;

/// How far away the eye is from the panel, in pixels: the strength of the
/// perspective.
pub const FOCAL: f32 = 260.0;
/// How far below a card its reflection reaches, as a share of its height.
pub const REFLECTION_REACH: f32 = 0.34;

/// One card as it stands: where (its centre, in pixels at the panel's own
/// depth, `z` away from it), turned how far about its vertical axis, how
/// large, and how it is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Which atlas (0: the cards, 1: the back of the enlarged one) and which
    /// picture in it.
    pub atlas: usize,
    pub sprite: usize,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
    pub w: f32,
    pub h: f32,
    /// Seen from behind: its picture runs the other way.
    pub mirror: bool,
    /// How lit it is (1: fully), and how far it fades into the ground with distance.
    pub shade: f32,
    pub fog: f32,
    /// How strongly it is reflected in the floor (0: not at all).
    pub reflect: f32,
}

/// Everything a frame of Flow is drawn from.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// The middle of the flow on the panel (the strip's offset included) and
    /// the eye's height (the horizon).
    pub cx: f32,
    pub cy: f32,
    /// The columns drawn: the part of the panel the flow is on.
    pub cols: (i32, i32),
    pub ground: Color,
    pub glow: Color,
    pub glow_strength: f32,
    /// The glow's middle and how far it reaches each way.
    pub glow_at: (f32, f32),
    pub glow_reach: (f32, f32),
    /// The floor: how much darker it is at its nearest, and over how many
    /// rows below the horizon it gets there.
    pub floor_dim: f32,
    pub floor_reach: f32,
    pub reflection: f32,
    /// The enlarged card's shadow on the ground: where (a rounded
    /// rectangle and its corner), and how dark at its darkest.
    pub shadow: Option<(bezel::Rect, f32, f32)>,
    /// Back to front.
    pub cards: Vec<Placed>,
}

/// How far a shadow spreads beyond what casts it.
pub const SHADOW_REACH: f32 = 14.0;

/// How much darker the shadow makes the ground at the middle of pixel (`x`, `y`).
pub fn shadow_at(scene: &Scene, x: f32, y: f32) -> f32 {
    let Some((r, radius, strength)) = scene.shadow else { return 0.0 };
    let d = bezel::canvas::sd_rrect(x, y, &r, radius);
    let t = (1.0 - d / SHADOW_REACH).clamp(0.0, 1.0);
    strength * t * t
}

/// A soft bump: 1 in the middle, 0 from `u` = ±1 out.
pub fn bump(u: f32) -> f32 {
    let q = (1.0 - u * u).clamp(0.0, 1.0);
    q * q
}

/// How dark the floor is at row `y` (0: as the ground).
pub fn floor_at(scene: &Scene, y: f32) -> f32 {
    let t = ((y - scene.cy) / scene.floor_reach).clamp(0.0, 1.0);
    scene.floor_dim * t * t * (3.0 - 2.0 * t)
}

#[inline]
fn lerp2(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = (((a & 0x00ff_00ff) * g + (b & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let ag = (((a >> 8) & 0x00ff_00ff) * g + ((b >> 8) & 0x00ff_00ff) * f) & 0xff00_ff00;
    rb | ag
}

/// Every channel times `f` (0 to 256).
#[inline]
fn scale(p: u32, f: u32) -> u32 {
    let rb = (((p & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let ag = (((p >> 8) & 0x00ff_00ff) * f) & 0xff00_ff00;
    rb | ag
}

/// The ground, its glow and the floor, over the flow's columns.
pub fn background(px: &mut [u32], scene: &Scene) {
    ground(px, scene, false);
}

/// The ground: drawn over everything (`under` false), or under what the
/// cards and reflections have left showing through (`under`: each pixel's
/// top byte is how covered it already is, and is cleared as it goes).
fn ground(px: &mut [u32], scene: &Scene, under: bool) {
    let (w, h) = (crate::gfx::W as usize, crate::gfx::H as usize);
    let (c0, c1) = (scene.cols.0.max(0) as usize, (scene.cols.1.max(0) as usize).min(w));
    if c0 >= c1 {
        return;
    }
    // How far across the glow reaches each column, in 1/65536ths.
    let across: Vec<u32> = (c0..c1).map(|x| (bump((x as f32 + 0.5 - scene.glow_at.0) / scene.glow_reach.0) * 65536.0) as u32).collect();
    let lit_px = |c: Color, lit: f32| ((c.0 as f32 * lit) as u32) << 16 | ((c.1 as f32 * lit) as u32) << 8 | (c.2 as f32 * lit) as u32;
    for y in 0..h {
        let fy = y as f32 + 0.5;
        let down = ((scene.glow_strength * bump((fy - scene.glow_at.1) / scene.glow_reach.1)) * 256.0) as u32;
        let lit = 1.0 - floor_at(scene, fy);
        // A row's ground and its glow at their strongest, both as lit as the
        // row is: each pixel is the one blended toward the other.
        let (base, glow) = (lit_px(scene.ground, lit), lit_px(scene.glow, lit));
        let row = &mut px[y * w + c0..y * w + c1];
        // Rows the shadow reaches, worked out a pixel at a time; the rest at once.
        let shaded = scene.shadow.is_some_and(|(r, ..)| fy > r.y - SHADOW_REACH && fy < r.bottom() + SHADOW_REACH);
        for (x, (p, &a)) in row.iter_mut().zip(&across).enumerate() {
            let covered = if under { *p >> 24 } else { 0 };
            if covered == 255 {
                *p &= 0x00ff_ffff;
                continue;
            }
            let mut colour = lerp2(base, glow, (a * down) >> 16);
            if shaded {
                colour = scale(colour, ((1.0 - shadow_at(scene, (c0 + x) as f32 + 0.5, fy)) * 256.0) as u32);
            }
            *p = if covered == 0 { colour } else { (*p + scale(colour, 256 - (covered + (covered >> 7)))) & 0x00ff_ffff };
        }
    }
}

/// A card's sums for one column of the panel: where on the card it is (in
/// the card's own units, 0 at its left edge as its picture runs), how far
/// away, and how many texels of its picture one pixel spans.
pub struct Column {
    pub along: f32,
    pub s: f32,
    pub texels: f32,
}

/// Where a card spans across the panel, and its extent in its own units
/// either side of its middle (its transparent edge included).
pub fn span(scene: &Scene, c: &Placed, atlas: &Atlas) -> Option<(f32, f32, f32)> {
    let (cos, sin) = (c.angle.cos(), c.angle.sin());
    let half = c.w * 0.5 + 4.0 * c.w / atlas.sw as f32;
    let near = FOCAL + c.z - half * sin.abs();
    if near <= 1.0 {
        return None;
    }
    let x_at = |t: f32| scene.cx + (c.x + t * cos) * FOCAL / (FOCAL + c.z + t * sin);
    let (a, b) = (x_at(-half), x_at(half));
    Some((a.min(b), a.max(b), half))
}

/// The sums for the column whose middle is `x`, if it meets the card.
pub fn column(scene: &Scene, c: &Placed, atlas: &Atlas, half: f32, x: f32) -> Option<Column> {
    let (cos, sin) = (c.angle.cos(), c.angle.sin());
    let p = x - scene.cx;
    let den = FOCAL * cos - p * sin;
    if den.abs() < 1e-4 {
        return None;
    }
    let t = (p * (FOCAL + c.z) - FOCAL * c.x) / den;
    if !(-half..=half).contains(&t) {
        return None;
    }
    let z = c.z + t * sin;
    if FOCAL + z <= 1.0 {
        return None;
    }
    let dt = FOCAL * ((FOCAL + c.z) * cos - c.x * sin) / (den * den);
    let along = if c.mirror { c.w * 0.5 - t } else { t + c.w * 0.5 };
    Some(Column { along, s: FOCAL / (FOCAL + z), texels: dt.abs() * atlas.sw as f32 / c.w })
}

/// Which levels a column reads, and how much of the second: the narrower
/// level when a pixel spans more than one texel of the wider.
pub fn levels(texels: f32) -> (usize, f32) {
    let lf = texels.max(1.0).log2().min((super::art::LEVELS - 1) as f32);
    let l = lf.floor();
    (l as usize, lf - l)
}

/// The level a column reads and how much of the next (256ths): `levels`,
/// taken to the nearer level, so a pixel reads one texel square, not two
/// (a card squeezed between two levels' widths is sharp or a touch soft by
/// turns, too slight to see as it turns). The GPU does the same.
pub fn blend_levels(texels: f32) -> (usize, u32) {
    let (l, between) = levels(texels);
    (l, if between < 0.5 { 0 } else { 256 })
}

/// The rows a card (or its reflection) covers in a column at scale `s`.
pub fn rows(scene: &Scene, c: &Placed, atlas: &Atlas, s: f32, reflection: bool) -> (f32, f32) {
    let pad = c.h / atlas.sh as f32;
    if reflection {
        let floor = scene.cy + (c.y + c.h * 0.5) * s;
        (floor - pad * s, floor + c.h * REFLECTION_REACH * s)
    } else {
        (scene.cy + (c.y - c.h * 0.5 - pad) * s, scene.cy + (c.y + c.h * 0.5 + pad) * s)
    }
}

/// Draw the whole frame. It is the ground, then the reflections, then the
/// cards, each over the last, back to front (as the GPU draws it); here it
/// is worked out front to back instead, each under what is already there,
/// to the same picture: so a pixel a nearer card has covered is not worked
/// out again for the cards behind it, nor for the ground.
pub fn render(px: &mut [u32], scene: &Scene, atlases: [&Atlas; 2]) {
    let (w, h) = (crate::gfx::W as usize, crate::gfx::H as usize);
    let (c0, c1) = (scene.cols.0.max(0) as usize, (scene.cols.1.max(0) as usize).min(w));
    if c0 >= c1 {
        return;
    }
    for y in 0..h {
        px[y * w + c0..y * w + c1].fill(0);
    }
    // Each column's longest run of rows already wholly covered: a card
    // further back whose column lies within it is not looked at.
    let mut solid = [(0i32, 0i32); crate::gfx::W as usize];
    for c in scene.cards.iter().rev() {
        draw(px, scene, c, atlases[c.atlas], false, &mut solid);
    }
    for c in scene.cards.iter().rev().filter(|c| c.reflect > 0.003) {
        draw(px, scene, c, atlases[c.atlas], true, &mut solid);
    }
    ground(px, scene, true);
}

/// A frame with everything Flow draws in it, from pictures made here (no
/// fonts needed): cards turned either way and in the middle, reflections,
/// a back turning over, the glow, the floor and a shadow. The GPU's test
/// frame, and the renderer's own tests'.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn test_scene() -> (Scene, Atlas, Atlas) {
    use bezel::canvas::sd_rrect;
    use bezel::Rect;
    let picture = |w: usize, h: usize, seed: u32| -> Vec<u32> {
        let r = Rect::new(0.0, 0.0, w as f32, h as f32);
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as u32, (i / w) as u32);
                // A gradient, stripes, and hard-edged blocks like words.
                let stripe = if (x / 5 + y / 7 + seed) % 2 == 0 { 50 } else { 0 };
                let block = if x % 19 < 8 && y % 13 < 5 { 140 } else { 0 };
                let r8 = (40 + x * 120 / w as u32 + stripe + block).min(255);
                let g8 = (30 + seed * 40 + y * 90 / h as u32 + block / 2).min(255);
                let b8 = (60 + stripe + seed * 25).min(255);
                let cover = (0.5 - sd_rrect(x as f32 + 0.5, y as f32 + 0.5, &r, 9.0)).clamp(0.0, 1.0);
                let a = (cover * 255.0 + 0.5) as u32;
                let pm = |v: u32| (v * a + 127) / 255;
                a << 24 | pm(r8) << 16 | pm(g8) << 8 | pm(b8)
            })
            .collect()
    };
    let n = super::CARD;
    let mut cards = Atlas::new(n, n, 3);
    for i in 0..3 {
        cards.put(i, &picture(n, n, i as u32));
    }
    let mut face = Atlas::new(super::FACE_W, super::FACE_H, 1);
    face.put(0, &picture(super::FACE_W, super::FACE_H, 3));
    let card = |sprite: usize, x: f32, z: f32, angle: f32| Placed {
        atlas: 0,
        sprite,
        x,
        y: 0.0,
        z,
        angle,
        w: n as f32,
        h: n as f32,
        mirror: false,
        shade: super::light(angle, false),
        fog: (z / 64.0).clamp(0.0, 1.0) * 0.3,
        reflect: 1.0,
    };
    let scene = Scene {
        cx: 160.0,
        cy: 78.0,
        cols: (0, crate::gfx::W as i32),
        ground: Color(11, 12, 14),
        glow: Color(255, 96, 91),
        glow_strength: 0.5,
        glow_at: (160.0, 72.0),
        glow_reach: (230.0, 150.0),
        floor_dim: 0.42,
        floor_reach: 82.0,
        reflection: 0.34,
        shadow: Some((Rect::new(60.0, 34.0, 200.0, 100.0), 12.0, 0.4)),
        cards: vec![
            card(2, -126.0, 64.0, 1.08),
            card(1, 104.0, 64.0, -1.08),
            card(0, -40.0, 30.0, 0.6),
            Placed { atlas: 1, sprite: 0, x: 0.0, y: 6.0, z: -12.0, angle: 2.5, w: 170.0, h: 104.0, mirror: true, shade: super::light(2.5, true), fog: 0.0, reflect: 0.4 },
        ],
    };
    (scene, cards, face)
}

/// One level's column: where it starts in the atlas row and how far between
/// its two texels, in 256ths.
struct Tap {
    x: usize,
    f: u32,
}

/// A column's last two texel rows, read across (between its two texels):
/// going down a column, the next pixel mostly reads the same two rows or
/// the next pair, so a row is read across once, not twice.
struct Rows {
    /// Which atlas row the pair starts at (`usize::MAX`: none yet).
    at: usize,
    top: u32,
    bottom: u32,
}

impl Default for Rows {
    fn default() -> Self {
        Rows { at: usize::MAX, top: 0, bottom: 0 }
    }
}

impl Rows {
    /// The bilinear read at texel row `row` (and the one below, `fv`
    /// 256ths of the way to it), between the tap's two columns. The atlas
    /// is kept a column at a time (`ah` texels each).
    #[inline]
    fn read(&mut self, px: &[u32], ah: usize, t: &Tap, row: usize, fv: u32) -> u32 {
        let across = |r: usize| {
            let i = t.x * ah + r;
            lerp2(px[i], px[i + ah], t.f)
        };
        if row != self.at {
            let known = self.at != usize::MAX;
            let (top, bottom) = if known && row == self.at + 1 {
                // One pair on down: its top is this one's bottom.
                (self.bottom, across(row + 1))
            } else if known && row + 1 == self.at {
                // One pair on up (a reflection): its bottom is this one's top.
                (across(row), self.top)
            } else {
                (across(row), across(row + 1))
            };
            (self.top, self.bottom, self.at) = (top, bottom, row);
        }
        lerp2(self.top, self.bottom, fv)
    }
}

fn tap(atlas: &Atlas, sprite: usize, level: usize, along: f32, w: f32) -> Tap {
    let (x0, _, lw) = atlas.level(sprite, level);
    let u = (along * atlas.sw as f32 / w / (1 << level) as f32 + 1.0).clamp(0.5, lw as f32 - 0.5) - 0.5;
    let i = u.floor();
    Tap { x: x0 + i as usize, f: ((u - i) * 256.0) as u32 }
}

fn draw(px: &mut [u32], scene: &Scene, c: &Placed, atlas: &Atlas, reflection: bool, solid: &mut [(i32, i32)]) {
    if reflection {
        draw_as::<true>(px, scene, c, atlas, solid)
    } else {
        draw_as::<false>(px, scene, c, atlas, solid)
    }
}

/// A card, or (`REFLECTION`) its reflection, under what is already drawn.
fn draw_as<const REFLECTION: bool>(px: &mut [u32], scene: &Scene, c: &Placed, atlas: &Atlas, solid: &mut [(i32, i32)]) {
    let reflection = REFLECTION;
    let (w, h) = (crate::gfx::W as i32, crate::gfx::H as i32);
    let Some((x0, x1, half)) = span(scene, c, atlas) else { return };
    let first = (x0.floor() as i32).max(scene.cols.0).max(0);
    let last = (x1.ceil() as i32).min(scene.cols.1).min(w);
    let ground = scene.ground.to_u32();
    let (_, y_origin, _) = atlas.level(c.sprite, 0);
    // The atlas is kept a column at a time, each this many texels.
    let aw = atlas.h;
    let sh = atlas.sh as f32;
    // A card's own light and fog, as 256ths (its reflection's fade row by row).
    let (lit, fog) = (((c.shade * (1.0 - c.fog)) * 256.0) as u32, (c.fog * 256.0) as u32);
    let plain = lit == 256 && fog == 0;
    // The fog over a wholly opaque texel, the same every time.
    let fogged = scale(ground, fog) | 0xff00_0000;
    let reflected = c.reflect * scene.reflection;
    let floor_y = c.y + c.h * 0.5;
    const ONE: f32 = 65536.0;
    let v_most = ((sh + 1.0) * ONE) as i32;
    for col in first..last {
        let Some(k) = column(scene, c, atlas, half, col as f32 + 0.5) else { continue };
        let (l, second) = blend_levels(k.texels);
        let taps = [tap(atlas, c.sprite, l, k.along, c.w), tap(atlas, c.sprite, (l + 1).min(super::art::LEVELS - 1), k.along, c.w)];
        let (ya, yb) = rows(scene, c, atlas, k.s, reflection);
        let (r0, r1) = ((ya.floor() as i32).max(0), (yb.ceil() as i32).min(h));
        // Behind a nearer card all the way down (its first and last rows,
        // the clear edge round its picture, aside): nothing of it shows.
        let (s0, s1) = solid[col as usize];
        if r0 >= r1 || (s0 <= r0 + 1 && r1 - 1 <= s1) {
            continue;
        }
        // The run of wholly covered rows this card leaves in the column.
        let mut run: Option<i32> = None;
        let mut close = |run: &mut Option<i32>, end: i32| {
            if let Some(start) = run.take() {
                let (s0, s1) = solid[col as usize];
                if end - start > s1 - s0 {
                    solid[col as usize] = (start, end);
                }
            }
        };
        // Down the column the picture's row moves on by the same amount each
        // pixel: `v` (in texels, its transparent edge included, less half a
        // texel) is stepped in 16.16, not worked out afresh. A reflection
        // runs up the picture as it runs down the panel.
        let per = sh / c.h / k.s;
        let world0 = (r0 as f32 + 0.5 - scene.cy) / k.s;
        let (v0, dv) = if reflection { ((c.h - (world0 - floor_y)) * sh / c.h + 0.5, -per) } else { ((world0 - c.y + c.h * 0.5) * sh / c.h + 0.5, per) };
        let (mut v, dv) = ((v0 * ONE) as i32, (dv * ONE) as i32);
        // A reflection fades as it goes down: how much of it is left (1 at
        // the floor, 0 its reach below), stepped in 16.16 too.
        let reach = c.h * REFLECTION_REACH;
        let fall0 = 1.0 - (world0 - floor_y) / reach;
        let (mut fall, fall_step) = ((fall0 * ONE) as i64, (-1.0 / (k.s * reach) * ONE) as i64);
        let reflected_fx = (reflected * 256.0) as i64;
        let mut rows_cache = [Rows::default(), Rows::default()];
        let mut at = (r0 * w + col) as usize;
        for row in r0..r1 {
            let here = at;
            let vv = v.clamp(0, v_most);
            let left = fall.min(ONE as i64);
            at += w as usize;
            v += dv;
            fall += fall_step;
            let covered = px[here] >> 24;
            if covered == 255 {
                if !reflection {
                    run.get_or_insert(row);
                }
                continue;
            }
            // A reflection's strength here, in 256ths: as strong as the card
            // is reflected, times what is left of it, squared.
            let strength = if reflection {
                if left <= 0 {
                    break;
                }
                let left = left >> 8;
                ((reflected_fx * left * left) >> 16) as u32
            } else {
                256
            };
            let fv = ((vv >> 8) & 0xff) as u32;
            let row_of = y_origin + (vv >> 16) as usize;
            let s = match second {
                0 => rows_cache[0].read(&atlas.px, aw, &taps[0], row_of, fv),
                256 => rows_cache[1].read(&atlas.px, aw, &taps[1], row_of, fv),
                f => lerp2(rows_cache[0].read(&atlas.px, aw, &taps[0], row_of, fv), rows_cache[1].read(&atlas.px, aw, &taps[1], row_of, fv), f),
            };
            let alpha = s >> 24;
            if alpha == 0 {
                if !reflection {
                    close(&mut run, row);
                }
                continue;
            }
            // Lit, faded into the ground with distance, and as strongly as
            // its reflection is: still premultiplied, its cover in the top byte.
            let src = if !REFLECTION && plain {
                s
            } else if !REFLECTION && alpha == 255 {
                scale(s & 0x00ff_ffff, lit) + fogged
            } else {
                let a = (alpha * strength) >> 8;
                scale(s & 0x00ff_ffff, (lit * strength) >> 8) + scale(ground, (fog * a) >> 8) + (a << 24)
            };
            // Under what is already here: what nearer cards left showing.
            px[here] = if covered == 0 { src } else { px[here] + scale(src, 256 - (covered + (covered >> 7))) };
            if !reflection {
                if px[here] >> 24 == 255 {
                    run.get_or_insert(row);
                } else {
                    close(&mut run, row);
                }
            }
        }
        if !reflection {
            close(&mut run, r1);
        }
    }
}
