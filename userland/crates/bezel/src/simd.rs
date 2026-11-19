//! The per-pixel loops a frame runs through, many pixels at a time with the
//! Cortex-A53's NEON (four or eight to an instruction), one at a time on
//! anything else. Both give exactly the same pixels, and the tests hold them
//! to it (on the ARM build, which runs them under emulation):
//!
//! - the 16-bit, dithered pixels every changed row of every frame becomes
//!   on its way to the panel;
//! - one frame blended into another, or faded toward black: a look changing,
//!   a finished app fading out, the screen dimming behind the control centre;
//! - a row of a screen scaled into a window, as an app grows out of its tile.

/// Mix two packed pixels: `f` is 0 (all `a`) to 256 (all `b`). Integer only.
#[inline]
pub fn mix_px(a: u32, b: u32, f: u32) -> u32 {
    let rb = (((a & 0x00ff_00ff) * (256 - f) + (b & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let g = (((a & 0x0000_ff00) * (256 - f) + (b & 0x0000_ff00) * f) >> 8) & 0x0000_ff00;
    rb | g
}

/// Blend `over` into `dst` by `f` (0: `dst` as it is, 256: all `over`);
/// with no `over`, fade `dst` toward black.
pub fn mix_into(dst: &mut [u32], over: Option<&[u32]>, f: u32) {
    let f = f.min(256);
    #[cfg(target_arch = "aarch64")]
    let done = unsafe { neon::mix_into(dst, over, f) };
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    scalar::mix_into(&mut dst[done..], over.map(|o| &o[done..]), f);
}

/// One row of the frame as the panel takes it: RGB565, native byte order,
/// dithered by place (`y` is the row; the row starts at x = 0).
pub fn rgb565_row(src: &[u32], y: usize, out: &mut [u8]) {
    let out = &mut out[..src.len() * 2];
    #[cfg(target_arch = "aarch64")]
    let done = unsafe { neon::rgb565_row(src, y, out) };
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    for (x, (o, p)) in out[done * 2..].chunks_exact_mut(2).zip(&src[done..]).enumerate() {
        o.copy_from_slice(&crate::canvas::rgb565_dithered(*p, done + x, y).to_ne_bytes());
    }
}

/// Where one column of a scaled picture reads from: the two source columns
/// either side, and how far between them in 256ths.
pub type Column = (usize, usize, u32);

/// A row of a picture scaled: each output pixel the bilinear blend of the
/// four source pixels around it, `top` and `bottom` being the source rows
/// either side and `fv` how far between them in 256ths.
pub fn bilinear_row(top: &[u32], bottom: &[u32], columns: &[Column], fv: u32, out: &mut [u32]) {
    let out = &mut out[..columns.len()];
    #[cfg(target_arch = "aarch64")]
    let done = unsafe { neon::bilinear_row(top, bottom, columns, fv, out) };
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    for (o, &(u0, u1, fu)) in out[done..].iter_mut().zip(&columns[done..]) {
        *o = mix_px(mix_px(top[u0], top[u1], fu), mix_px(bottom[u0], bottom[u1], fu), fv);
    }
}

mod scalar {
    use super::mix_px;

    pub fn mix_into(dst: &mut [u32], over: Option<&[u32]>, f: u32) {
        match over {
            Some(over) => dst.iter_mut().zip(over).for_each(|(p, &o)| *p = mix_px(*p, o, f)),
            None => dst.iter_mut().for_each(|p| *p = mix_px(*p, 0, f)),
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::*;

    /// Two halves of four packed pixels (bytes widened to 16 bits) mixed by
    /// per-lane weights: (x × (256 − w) + y × w) >> 8, as `mix_px` does it.
    /// No product passes 255 × 256, so 16 bits hold every one.
    #[inline(always)]
    unsafe fn mix16(x: uint8x16_t, y: uint8x16_t, w_lo: uint16x8_t, w_hi: uint16x8_t) -> uint8x16_t {
        let all = vdupq_n_u16(256);
        let (k_lo, k_hi) = (vsubq_u16(all, w_lo), vsubq_u16(all, w_hi));
        let lo = vaddq_u16(vmulq_u16(vmovl_u8(vget_low_u8(x)), k_lo), vmulq_u16(vmovl_u8(vget_low_u8(y)), w_lo));
        let hi = vaddq_u16(vmulq_u16(vmovl_u8(vget_high_u8(x)), k_hi), vmulq_u16(vmovl_u8(vget_high_u8(y)), w_hi));
        vcombine_u8(vshrn_n_u16(lo, 8), vshrn_n_u16(hi, 8))
    }

    /// The byte that is not a colour comes out 0, as `mix_px` leaves it.
    #[inline(always)]
    unsafe fn colour_only(v: uint8x16_t) -> uint32x4_t {
        vandq_u32(vreinterpretq_u32_u8(v), vdupq_n_u32(0x00ff_ffff))
    }

    /// Returns how many pixels it did: whole fours; the caller does the rest.
    pub unsafe fn mix_into(dst: &mut [u32], over: Option<&[u32]>, f: u32) -> usize {
        let w = vdupq_n_u16(f as u16);
        let n = dst.len() / 4 * 4;
        let black = vdupq_n_u8(0);
        for i in (0..n).step_by(4) {
            let a = vld1q_u8(dst.as_ptr().add(i) as *const u8);
            let b = match over {
                Some(o) => vld1q_u8(o.as_ptr().add(i) as *const u8),
                None => black,
            };
            vst1q_u32(dst.as_mut_ptr().add(i), colour_only(mix16(a, b, w, w)));
        }
        n
    }

    /// Eight pixels at a time: their channels pulled apart, the row's dither
    /// thresholds added with saturation (the `min(255)`), then packed.
    pub unsafe fn rgb565_row(src: &[u32], y: usize, out: &mut [u8]) -> usize {
        let row = crate::canvas::BAYER[y & 3];
        let d = [row[0], row[1], row[2], row[3], row[0], row[1], row[2], row[3]];
        let d = vld1_u8(d.as_ptr());
        let (d5, d6) = (vshr_n_u8(d, 1), vshr_n_u8(d, 2));
        let n = src.len() / 8 * 8;
        for i in (0..n).step_by(8) {
            // Memory holds blue, green, red, unused for each pixel.
            let px = vld4_u8(src.as_ptr().add(i) as *const u8);
            let (b, g, r) = (vqadd_u8(px.0, d5), vqadd_u8(px.1, d6), vqadd_u8(px.2, d5));
            let r = vandq_u16(vshll_n_u8(r, 8), vdupq_n_u16(0xf800));
            let g = vandq_u16(vshll_n_u8(g, 3), vdupq_n_u16(0x07e0));
            let b = vshrq_n_u16(vmovl_u8(b), 3);
            vst1q_u8(out.as_mut_ptr().add(i * 2), vreinterpretq_u8_u16(vorrq_u16(vorrq_u16(r, g), b)));
        }
        n
    }

    /// Four output pixels at a time: their sixteen source pixels gathered,
    /// then blended across and down with each pixel's own weight.
    pub unsafe fn bilinear_row(top: &[u32], bottom: &[u32], columns: &[super::Column], fv: u32, out: &mut [u32]) -> usize {
        let n = columns.len() / 4 * 4;
        let down = vdupq_n_u16(fv as u16);
        for i in (0..n).step_by(4) {
            let c = &columns[i..i + 4];
            let gather = |row: &[u32], pick: fn(&super::Column) -> usize| -> uint8x16_t {
                let v = [row[pick(&c[0])], row[pick(&c[1])], row[pick(&c[2])], row[pick(&c[3])]];
                vld1q_u8(v.as_ptr() as *const u8)
            };
            let (a, b) = (gather(top, |c| c.0), gather(top, |c| c.1));
            let (e, f) = (gather(bottom, |c| c.0), gather(bottom, |c| c.1));
            // Each pixel's weight across, for each of its four bytes.
            let w = |k: usize| c[k].2 as u16;
            let w_lo = [w(0), w(0), w(0), w(0), w(1), w(1), w(1), w(1)];
            let w_hi = [w(2), w(2), w(2), w(2), w(3), w(3), w(3), w(3)];
            let (w_lo, w_hi) = (vld1q_u16(w_lo.as_ptr()), vld1q_u16(w_hi.as_ptr()));
            let upper = vreinterpretq_u8_u32(colour_only(mix16(a, b, w_lo, w_hi)));
            let lower = vreinterpretq_u8_u32(colour_only(mix16(e, f, w_lo, w_hi)));
            vst1q_u32(out.as_mut_ptr().add(i), colour_only(mix16(upper, lower, down, down)));
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic jumble of pixels, colour bytes only, as a frame holds them.
    fn noise(n: usize, seed: u32) -> Vec<u32> {
        let mut s = seed.wrapping_mul(2_654_435_761).max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s & 0x00ff_ffff
            })
            .collect()
    }

    #[test]
    fn blends_are_exactly_the_plain_ones() {
        // Lengths that leave every remainder of four, and every weight's edges.
        for len in [0, 1, 3, 4, 7, 320, 54_403] {
            for f in [0, 1, 37, 128, 255, 256] {
                let (a, b) = (noise(len, 1 + f), noise(len, 99 + f));
                let mut fast = a.clone();
                mix_into(&mut fast, Some(&b), f);
                let plain: Vec<u32> = a.iter().zip(&b).map(|(&x, &y)| mix_px(x, y, f)).collect();
                assert_eq!(fast, plain, "into, {len} pixels, f {f}");
                let mut dark = a.clone();
                mix_into(&mut dark, None, f);
                assert!(dark.iter().zip(&a).all(|(&d, &x)| d == mix_px(x, 0, f)), "to black, {len} pixels, f {f}");
            }
        }
    }

    #[test]
    fn panel_pixels_are_exactly_the_plain_ones() {
        for len in [1, 7, 8, 9, 320] {
            for y in 0..4 {
                let src = noise(len, len as u32 * 7 + y as u32);
                // White and black too: the dither must saturate, not wrap.
                let src: Vec<u32> = src.iter().enumerate().map(|(i, &p)| if i % 5 == 0 { 0x00ff_ffff } else if i % 7 == 0 { 0 } else { p }).collect();
                let mut fast = vec![0u8; len * 2];
                rgb565_row(&src, y, &mut fast);
                let plain: Vec<u8> = src.iter().enumerate().flat_map(|(x, &p)| crate::canvas::rgb565_dithered(p, x, y).to_ne_bytes()).collect();
                assert_eq!(fast, plain, "{len} pixels, row {y}");
            }
        }
    }

    #[test]
    fn a_scaled_row_is_exactly_the_plain_one() {
        let (top, bottom) = (noise(320, 5), noise(320, 6));
        for (len, fv) in [(0, 0), (3, 10), (150, 128), (151, 255), (320, 1)] {
            let columns: Vec<Column> = (0..len).map(|x| {
                let u = (x * 319 / len.max(1)) as usize;
                (u, (u + 1).min(319), ((x * 37) % 256) as u32)
            }).collect();
            let mut fast = vec![0u32; len];
            bilinear_row(&top, &bottom, &columns, fv, &mut fast);
            let plain: Vec<u32> = columns.iter().map(|&(u0, u1, fu)| mix_px(mix_px(top[u0], top[u1], fu), mix_px(bottom[u0], bottom[u1], fu), fv)).collect();
            assert_eq!(fast, plain, "{len} pixels, fv {fv}");
        }
    }
}
