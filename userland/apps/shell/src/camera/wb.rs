//! Smart white balance (1.5.2): an automatic white balance on top of the
//! camera's own.
//!
//! libcamera's white balance, with the tuning made for the Raspberry Pi
//! Camera v2 (`vc4/imx219.json`), left colour casts on this module: yellow
//! under lamps, sometimes strong, in the viewfinder and photos alike. So
//! the shell measures what cast is left in each preview frame and sets the
//! white balance gains itself, through the preview's order (`gains=r,b`).
//! A photo takes the preview's gains as they are, so it comes out as the
//! viewfinder showed it.
//!
//! Each frame (320 × 240 YUV 4:2:0, full-range BT.601 as catalyst-camera
//! asks the ISP for):
//!
//! 1. **Votes.** 80 × 60 samples, each taken back through the ISP: to
//!    R'G'B', the tuning's gamma undone, its colour matrix undone; then the
//!    gains that would make that sample grey, from the gains the frame was
//!    made with: the sample's vote for the light. Very dark, clipped and
//!    very colourful samples have none.
//! 2. **Plausible lights.** A vote counts as much as it lies near the
//!    lights' locus (the tuning's white point curve, 2000 to 10000 K), and
//!    as the scene's brightness agrees with it: above 2000 lux, votes for a
//!    light warmer than 4000 K count less (a third at 10000 lux).
//! 3. **Near-neutral and white.** The near-neutral estimate is where the
//!    votes are densest (mean shift, its window narrowing), so a colourful
//!    object far from the rest has no say; the white patch, the brightest
//!    4 % near it, pulls it a third of the way.
//! 4. **Faces.** With a face in the picture (the smart camera's), the light
//!    must leave the skin between the Macbeth chart's light and dark skin
//!    (30 mired of room); outside that, it is pulled halfway in.
//! 5. **Locus.** The light is held to the curve from 2300 to 8000 K, with
//!    [`TINT_ROOM`] either side of it for a green or magenta tint, so a big
//!    red wall cannot turn the picture blue-green.
//! 6. **Confidence** falls when few samples agree on a light (a green
//!    field, a blue sky): the gains go back towards the camera's own as it
//!    does, and with almost none for two seconds the camera is handed its
//!    own white balance again until there is something to go by.
//! 7. **Warmth.** A warm light is left a little warm, as phones leave it:
//!    [`kept_warmth`].
//! 8. **Damping.** The gains glide to the target (a 0.18 s time constant,
//!    at most 1.6 log units a second, slower when unsure), start moving
//!    only once it is 1.5 % away and stop within 0.4 %: a still scene stays
//!    still, a new one settles in under a second.
//!
//! It steers in photo mode and the other auto modes, and in pro mode with
//! its white balance on auto. It leaves the camera's own white balance in
//! place where one is set by hand (pro's temperature or grey card) and for
//! a black-and-white look, and holds its gains while the exposure lock is
//! on (camera.rs decides which).

use crate::dng::{self, Tuning};

/// White balance gains as their logarithms: (ln red, ln blue).
pub type LogGains = [f32; 2];

/// The warmest and coolest lights the gains are held to (K).
pub const WARMEST_K: f32 = 2300.0;
pub const COOLEST_K: f32 = 8000.0;
/// Room either side of the curve for a green or magenta light (log gain,
/// across it: 0.12 is about 8 % on both gains). The tuning's own search
/// goes 0.05 either side; libcamera's gains sit 0.07 above the curve (its
/// `sensitivity_r` and `_b` of 1.05), and a fluorescent tube's green asks
/// for about 0.11.
pub const TINT_ROOM: f32 = 0.12;

/// Votes count from 10000 K (100 mired) to 2000 K (500 mired).
const VOTE_COOL_MIRED: f32 = 100.0;
const VOTE_WARM_MIRED: f32 = 500.0;
/// How fast a vote further off the curve than [`TINT_ROOM`] stops
/// counting (log gain).
const VOTE_TINT_SIGMA: f32 = 0.05;
/// How much warmer than the light the near-neutral surfaces of a real
/// scene read (mired): the Kodak set's lean 40 (its whites themselves a
/// few percent yellow), a grey room's none; a part of it taken off, so a
/// truly grey room is shown only a touch warm.
const LEAN_MIRED: f32 = 12.0;
/// How far the white patch pulls the near-neutral estimate.
const WHITE_PULL: f32 = 0.5;
/// A sample is too dark under this (8-bit luma), and clipped at this.
const DARK: f32 = 18.0;
const CLIP: f32 = 250.0;

/// libcamera 0.7.1's `rpi.contrast` gamma curve for the IMX219 (16-bit in,
/// 16-bit out). Its adaptive contrast moves it a little each frame; the
/// same on all three colours, so grey stays grey either way.
const GAMMA: [(f32, f32); 33] = [
    (0.0, 0.0),
    (1024.0, 5040.0),
    (2048.0, 9338.0),
    (3072.0, 12356.0),
    (4096.0, 15312.0),
    (5120.0, 18051.0),
    (6144.0, 20790.0),
    (7168.0, 23193.0),
    (8192.0, 25744.0),
    (9216.0, 27942.0),
    (10240.0, 30035.0),
    (11264.0, 32005.0),
    (12288.0, 33975.0),
    (13312.0, 35815.0),
    (14336.0, 37600.0),
    (15360.0, 39168.0),
    (16384.0, 40642.0),
    (18432.0, 43379.0),
    (20480.0, 45749.0),
    (22528.0, 47753.0),
    (24576.0, 49621.0),
    (26624.0, 51253.0),
    (28672.0, 52698.0),
    (30720.0, 53796.0),
    (32768.0, 54876.0),
    (36864.0, 57012.0),
    (40960.0, 58656.0),
    (45056.0, 59954.0),
    (49152.0, 61183.0),
    (53248.0, 62355.0),
    (57344.0, 63419.0),
    (61440.0, 64476.0),
    (65535.0, 65535.0),
];

/// The Macbeth chart's light and dark skin in linear sRGB (D65).
const SKINS: [[f32; 3]; 2] = [[0.539, 0.305, 0.223], [0.171, 0.084, 0.058]];

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn dist2(a: LogGains, b: LogGains) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn ln_gains(g: [f32; 2]) -> LogGains {
    [g[0].max(0.05).ln(), g[1].max(0.05).ln()]
}

/// Gains from their logarithms, kept to what the ISP takes.
pub fn gains_of(l: LogGains) -> [f32; 2] {
    [l[0].exp().clamp(0.5, 8.0), l[1].exp().clamp(0.5, 8.0)]
}

fn mired(k: f32) -> f32 {
    1e6 / k
}

// ------------------------------------------------------------- warmth ---

/// How much of a warm light's colour the picture keeps, as a share of its
/// distance (in mired) from 5000 K: none at 4500 K and cooler, rising to
/// 10 % at 2700 K (a household lamp) and 14 % at 2300 K and warmer, in
/// straight lines in mired between. The picture is balanced as if a
/// 2700 K lamp were 2830 K: a little warm, not yellow (the colour matrix
/// makes every mired left count: 30 % left was a plainly yellow grey);
/// 3200 K halogen keeps 6 % (balanced as 3270 K), a 2300 K bulb is
/// balanced as 2490 K.
pub fn kept_warmth(m: f32) -> f32 {
    let (cool, lamp, candle) = (mired(4500.0), mired(2700.0), mired(2300.0));
    if m <= cool {
        0.0
    } else if m <= lamp {
        0.10 * (m - cool) / (lamp - cool)
    } else {
        (0.10 + 0.04 * (m - lamp) / (candle - lamp)).min(0.14)
    }
}

/// The light (mired) the picture is balanced for, for a light of `m`.
pub fn shown_mired(m: f32) -> f32 {
    m - kept_warmth(m) * (m - mired(5000.0)).max(0.0)
}

// -------------------------------------------------------------- locus ---

/// The lights' locus: the tuning's white point curve, as log gains, with a
/// table to find where any gains fall against it.
#[derive(Clone, Debug)]
pub struct Locus {
    tuning: Tuning,
    origin: LogGains,
    /// Along the curve, cool to warm; across it, both gains up (magenta).
    along: [f32; 2],
    across: [f32; 2],
    s0: f32,
    ds: f32,
    /// Per step along: the curve's offset across, and its mired.
    table: Vec<(f32, f32)>,
}

impl Locus {
    pub fn new(mut tuning: Tuning) -> Locus {
        // Points closer than 50 K made one: the stock curve steps at 2911
        // and 2919 K (red ÷ green 3.7 % down, blue ÷ green 7.9 % up in 8 K),
        // where a lamp's light falls; a step there would jump the gains.
        let mut merged: Vec<(f64, f64, f64)> = Vec::new();
        for p in &tuning.ct_curve {
            match merged.last_mut() {
                Some(q) if p.0 - q.0 < 50.0 => *q = ((q.0 + p.0) / 2.0, (q.1 + p.1) / 2.0, (q.2 + p.2) / 2.0),
                _ => merged.push(*p),
            }
        }
        if merged.len() >= 2 {
            tuning.ct_curve = merged;
        }
        let mut l = Locus { tuning, origin: [0.0; 2], along: [1.0, 0.0], across: [0.0, 1.0], s0: 0.0, ds: 1.0, table: Vec::new() };
        let (warm, cool) = (l.curve(mired(2500.0)), l.curve(mired(8000.0)));
        let d = [warm[0] - cool[0], warm[1] - cool[1]];
        let len = d[0].hypot(d[1]).max(1e-6);
        l.along = [d[0] / len, d[1] / len];
        l.across = [l.along[1], -l.along[0]];
        if l.across[0] + l.across[1] < 0.0 {
            l.across = [-l.across[0], -l.across[1]];
        }
        l.origin = l.curve(mired(5000.0));
        // The curve every mired from 10000 K to 2000 K, then by steps along.
        let samples: Vec<(f32, f32, f32)> = (0..=((VOTE_WARM_MIRED - VOTE_COOL_MIRED) as usize))
            .map(|i| {
                let m = VOTE_COOL_MIRED + i as f32;
                let (s, p) = l.project(l.curve(m));
                (s, p, m)
            })
            .collect();
        let (lo, hi) = samples.iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.0), hi.max(s.0)));
        const STEPS: usize = 256;
        l.s0 = lo;
        l.ds = (hi - lo) / (STEPS - 1) as f32;
        let mut sorted = samples;
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut j = 0;
        l.table = (0..STEPS)
            .map(|i| {
                let s = lo + l.ds * i as f32;
                while j + 2 < sorted.len() && sorted[j + 1].0 < s {
                    j += 1;
                }
                let (a, b) = (sorted[j], sorted[j + 1]);
                let k = ((s - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0);
                (a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
            })
            .collect();
        l
    }

    /// The gains that make a light of `m` mired grey (the tuning's curve,
    /// carried on straight in mired past its ends).
    pub fn curve(&self, m: f32) -> LogGains {
        let c = &self.tuning.ct_curve;
        let n = c.len();
        let m = f64::from(m);
        let line = |a: (f64, f64, f64), b: (f64, f64, f64)| {
            let (ma, mb) = (1e6 / a.0, 1e6 / b.0);
            let f = (m - ma) / (mb - ma);
            (a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
        };
        let k = 1e6 / m;
        let (r, b) = if k < c[0].0 {
            line(c[0], c[1])
        } else if k > c[n - 1].0 {
            line(c[n - 2], c[n - 1])
        } else {
            self.tuning.white(k)
        };
        [(-(r.max(0.05)).ln()) as f32, (-(b.max(0.05)).ln()) as f32]
    }

    fn project(&self, g: LogGains) -> (f32, f32) {
        let v = [g[0] - self.origin[0], g[1] - self.origin[1]];
        (v[0] * self.along[0] + v[1] * self.along[1], v[0] * self.across[0] + v[1] * self.across[1])
    }

    /// Where gains fall against the curve: the light's mired, and the tint
    /// across the curve (positive: green light, gains up).
    pub fn place(&self, g: LogGains) -> (f32, f32) {
        let (s, p) = self.project(g);
        let x = (s - self.s0) / self.ds;
        let last = self.table.len() - 1;
        let at = |i: usize| self.table[i];
        let (pc, m) = if x <= 0.0 {
            let slope = at(1).1 - at(0).1;
            (at(0).0, at(0).1 + slope * x)
        } else if x >= last as f32 {
            let slope = at(last).1 - at(last - 1).1;
            (at(last).0, at(last).1 + slope * (x - last as f32))
        } else {
            let i = x as usize;
            let k = x - i as f32;
            let (a, b) = (at(i), at((i + 1).min(last)));
            (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
        };
        (m, p - pc)
    }

    /// The gains for a light of `m` mired with a tint across the curve.
    pub fn point(&self, m: f32, tint: f32) -> LogGains {
        let c = self.curve(m);
        [c[0] + tint * self.across[0], c[1] + tint * self.across[1]]
    }

    /// Gains held to the curve from [`WARMEST_K`] to [`COOLEST_K`] and
    /// [`TINT_ROOM`] either side.
    pub fn constrain(&self, g: LogGains) -> LogGains {
        let (m, t) = self.place(g);
        self.point(m.clamp(mired(COOLEST_K), mired(WARMEST_K)), t.clamp(-TINT_ROOM, TINT_ROOM))
    }
}

// -------------------------------------------------------- measurement ---

/// A preview frame and what it was made with.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// YUV 4:2:0, full-range BT.601.
    pub yuv: &'a [u8],
    pub w: usize,
    pub h: usize,
    /// The white balance gains (red, blue) and temperature it was made with.
    pub gains: [f32; 2],
    pub ct: u32,
    pub lux: f32,
    /// A face in it (0 to 1: left, top, right, bottom).
    pub face: Option<[f32; 4]>,
}

/// What a frame says of its light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The gains that make the light grey, held to the locus.
    pub light: LogGains,
    pub mired: f32,
    pub tint: f32,
    /// 0 (nothing to go by) to 1.
    pub confidence: f32,
}

#[derive(Clone, Copy)]
struct Vote {
    at: LogGains,
    w: f32,
    luma: f32,
}

/// The ISP turned round: its gamma and colour matrices, and the locus.
#[derive(Clone, Debug)]
pub struct Model {
    pub locus: Locus,
    linear: Vec<f32>,
}

impl Default for Model {
    fn default() -> Self {
        Model::new(Tuning::stock())
    }
}

/// The ISP's gamma at `x` (0 to 1), for the tests' frames.
pub fn gamma(x: f32) -> f32 {
    let v = x.clamp(0.0, 1.0) * 65535.0;
    let i = GAMMA.windows(2).position(|p| v <= p[1].0).unwrap_or(GAMMA.len() - 2);
    let (a, b) = (GAMMA[i], GAMMA[i + 1]);
    (a.1 + (b.1 - a.1) * (v - a.0) / (b.0 - a.0)) / 65535.0
}

impl Model {
    pub fn new(tuning: Tuning) -> Model {
        // The gamma undone, for each 8-bit value (a tenth of a step).
        let linear = (0..=2550)
            .map(|i| {
                let v = i as f32 / 2550.0 * 65535.0;
                let j = GAMMA.windows(2).position(|p| v <= p[1].1).unwrap_or(GAMMA.len() - 2);
                let (a, b) = (GAMMA[j], GAMMA[j + 1]);
                (a.0 + (b.0 - a.0) * (v - a.1) / (b.1 - a.1).max(1.0)) / 65535.0
            })
            .collect();
        Model { locus: Locus::new(tuning), linear }
    }

    fn lin(&self, v: f32) -> f32 {
        self.linear[((v * 10.0).round().max(0.0) as usize).min(2550)]
    }

    /// The colour matrix a frame was made with, turned round.
    fn unmatrix(&self, ct: u32, gains: LogGains) -> [f32; 9] {
        let ct = if ct > 0 { f64::from(ct) } else { 1e6 / f64::from(self.locus.place(gains).0.max(50.0)) };
        let m = dng::invert(&self.locus.tuning.ccm(ct)).unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        m.map(|v| v as f32)
    }

    /// A pixel's R'G'B' back to the light the sensor saw, white balanced.
    fn sensor(&self, inv: &[f32; 9], rgb: [f32; 3]) -> [f32; 3] {
        let l = rgb.map(|c| self.lin(c));
        [inv[0] * l[0] + inv[1] * l[1] + inv[2] * l[2], inv[3] * l[0] + inv[4] * l[1] + inv[5] * l[2], inv[6] * l[0] + inv[7] * l[1] + inv[8] * l[2]]
    }

    /// What a frame says of its light; None when nothing in it can say.
    pub fn measure(&self, f: &Frame) -> Option<Reading> {
        let (w, h) = (f.w & !1, f.h & !1);
        if w < 16 || h < 16 || f.yuv.len() < f.w * f.h * 3 / 2 {
            return None;
        }
        let (cw, ch) = (f.w / 2, f.h / 2);
        let (ys, us, vs) = (&f.yuv[..f.w * f.h], &f.yuv[f.w * f.h..f.w * f.h + cw * ch], &f.yuv[f.w * f.h + cw * ch..f.w * f.h + 2 * cw * ch]);
        let base = ln_gains(f.gains);
        let inv = self.unmatrix(f.ct, base);
        let step = (cw / 80).max(1);
        let pixel = |cx: usize, cy: usize| -> Option<(f32, [f32; 3])> {
            let a = 2 * cy * f.w + 2 * cx;
            let y = (f32::from(ys[a]) + f32::from(ys[a + 1]) + f32::from(ys[a + f.w]) + f32::from(ys[a + f.w + 1])) / 4.0;
            let (u, v) = (f32::from(us[cy * cw + cx]) - 128.0, f32::from(vs[cy * cw + cx]) - 128.0);
            let rgb = [y + 1.402 * v, y - 0.344_136 * u - 0.714_136 * v, y + 1.772 * u];
            if y < DARK || rgb.iter().any(|c| *c >= CLIP) || u.hypot(v) > 100.0 {
                return None;
            }
            let s = self.sensor(&inv, rgb);
            s.iter().all(|c| *c > 1e-4).then_some((y, s))
        };
        let mut votes = Vec::with_capacity((cw / step + 1) * (ch / step + 1));
        let (mut samples, mut valid, mut valid_w) = (0.0f32, 0.0f32, 0.0f32);
        for cy in (0..ch).step_by(step) {
            for cx in (0..cw).step_by(step) {
                samples += 1.0;
                let Some((y, s)) = pixel(cx, cy) else { continue };
                // Bright samples count more: a dark one is noisier, and more
                // often a dark colour (wood, earth) than a grey.
                let w_luma = smoothstep(DARK, DARK + 22.0, y);
                let w_bright = w_luma * (y / 160.0).clamp(0.25, 1.5);
                valid += w_luma;
                valid_w += w_bright;
                let at = [base[0] + (s[1] / s[0]).ln(), base[1] + (s[1] / s[2]).ln()];
                let (m, tint) = self.locus.place(at);
                let w_band = (-0.5 * ((tint.abs() - TINT_ROOM).max(0.0) / VOTE_TINT_SIGMA).powi(2)).exp();
                let beyond = (VOTE_COOL_MIRED - m).max(m - VOTE_WARM_MIRED).max(0.0);
                let w_range = (-0.5 * (beyond / 25.0).powi(2)).exp();
                let wt = w_bright * w_band * w_range * lux_prior(f.lux, m);
                if wt > 1e-3 {
                    votes.push(Vote { at, w: wt, luma: y });
                }
            }
        }
        let total: f32 = votes.iter().map(|v| v.w).sum();
        if votes.len() < 30 || total < 2.0 {
            return None;
        }
        // Near-neutral: the densest votes, the window narrowing.
        let mut c = votes.iter().fold([0.0f32; 2], |a, v| [a[0] + v.at[0] * v.w, a[1] + v.at[1] * v.w]).map(|x| x / total);
        let mut support = 0.0;
        for sigma in [0.30f32, 0.20, 0.12, 0.08, 0.05, 0.05] {
            let k = -0.5 / (sigma * sigma);
            let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
            for v in &votes {
                let wk = v.w * (k * dist2(v.at, c)).exp();
                sx += v.at[0] * wk;
                sy += v.at[1] * wk;
                sw += wk;
            }
            if sw > 1e-6 {
                c = [sx / sw, sy / sw];
            }
            support = sw;
        }
        let grey = c;
        // White patch: the brightest 4 % near it.
        let mut near: Vec<Vote> = votes.iter().copied().filter(|v| dist2(v.at, grey) < 0.25 * 0.25).collect();
        let mut light = grey;
        let top = near.len() / 25;
        if top >= 8 {
            near.select_nth_unstable_by(top, |a, b| b.luma.total_cmp(&a.luma));
            let (sw, sx, sy) = near[..top].iter().fold((0.0, 0.0, 0.0), |(w, x, y), v| (w + v.w, x + v.at[0] * v.w, y + v.at[1] * v.w));
            if sw > 1e-6 {
                let white = [sx / sw, sy / sw];
                light = [grey[0] + (white[0] - grey[0]) * WHITE_PULL, grey[1] + (white[1] - grey[1]) * WHITE_PULL];
            }
        }
        // A face: its skin between the chart's light and dark skin.
        if let Some(face) = f.face {
            light = self.skin_prior(f, face, &inv, base, light, &pixel);
        }
        // Grey surfaces in real scenes lean warm (beige, wood, earth, skin):
        // the light read from them is taken a little cooler.
        let (m, tint) = self.locus.place(light);
        let m = m - LEAN_MIRED;
        let light = self.locus.point(m, tint);
        let off = (tint.abs() - TINT_ROOM).max(0.0) / 0.1 + (mired(COOLEST_K) - m).max(m - mired(WARMEST_K)).max(0.0) / 60.0;
        let confidence = smoothstep(0.03, 0.15, support / valid_w.max(1e-6)) * smoothstep(0.05, 0.25, valid / samples) / (1.0 + off * off);
        let light = self.locus.constrain(light);
        let (m, tint) = self.locus.place(light);
        Some(Reading { light, mired: m, tint, confidence })
    }

    fn skin_prior(&self, f: &Frame, face: [f32; 4], inv: &[f32; 9], base: LogGains, light: LogGains, pixel: &dyn Fn(usize, usize) -> Option<(f32, [f32; 3])>) -> LogGains {
        let (cw, ch) = (f.w / 2, f.h / 2);
        // The middle of the box: no hair, no background.
        let (bw, bh) = (face[2] - face[0], face[3] - face[1]);
        let x0 = ((face[0] + bw * 0.2) * cw as f32) as usize;
        let x1 = (((face[2] - bw * 0.2) * cw as f32) as usize).min(cw);
        let y0 = ((face[1] + bh * 0.25) * ch as f32) as usize;
        let y1 = (((face[3] - bh * 0.15) * ch as f32) as usize).min(ch);
        let (mut sum, mut n) = ([0.0f32; 3], 0);
        for cy in y0..y1 {
            for cx in x0..x1 {
                if let Some((_, s)) = pixel(cx, cy) {
                    for i in 0..3 {
                        sum[i] += s[i];
                    }
                    n += 1;
                }
            }
        }
        if n < 12 {
            return light;
        }
        let face_vote = [base[0] + (sum[1] / sum[0]).ln(), base[1] + (sum[1] / sum[2]).ln()];
        // The light each skin would mean, and the room between them.
        let unskin = |skin: [f32; 3]| {
            let s = [inv[0] * skin[0] + inv[1] * skin[1] + inv[2] * skin[2], inv[3] * skin[0] + inv[4] * skin[1] + inv[5] * skin[2], inv[6] * skin[0] + inv[7] * skin[1] + inv[8] * skin[2]].map(|c| c.max(1e-4));
            self.locus.place([face_vote[0] - (s[1] / s[0]).ln(), face_vote[1] - (s[1] / s[2]).ln()]).0
        };
        let (a, b) = (unskin(SKINS[0]), unskin(SKINS[1]));
        let (lo, hi) = (a.min(b) - 30.0, a.max(b) + 30.0);
        let (m, tint) = self.locus.place(light);
        if (lo..=hi).contains(&m) {
            return light;
        }
        self.locus.point(m + (m.clamp(lo, hi) - m) * 0.5, tint)
    }
}

/// Bright scenes are lit by daylight: above 2000 lux a vote for a light
/// warmer than 4000 K counts less, a third as much from 10000 lux.
fn lux_prior(lux: f32, m: f32) -> f32 {
    1.0 - 0.67 * smoothstep(2000.0, 10000.0, lux) * smoothstep(mired(4000.0), mired(3000.0), m)
}

// --------------------------------------------------------- the steering ---

/// The camera's own white balance runs this many frames first.
pub const STARTUP_FRAMES: u32 = 10;
/// The glide's time constant (s) and fastest pace (log gain a second).
const TAU_S: f32 = 0.18;
const MOST_PER_S: f32 = 1.6;
/// The gains start moving once this far from where they should be, and
/// stop once this close (log gain: 1.5 % and 0.4 %).
const START: f32 = 0.015;
const STOP: f32 = 0.004;
/// Handed back to the camera below this confidence for 2 s; taken again
/// above that one for 0.4 s.
const LOST: f32 = 0.12;
const LOST_S: f32 = 2.0;
const FOUND: f32 = 0.3;
const FOUND_S: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The camera's own white balance (opted out, starting, or nothing to go by).
    Camera,
    /// The gains are this one's.
    Steering,
}

/// The smart white balance's state, fed each preview frame.
#[derive(Clone, Debug)]
pub struct Smart {
    model: Model,
    pub state: State,
    /// The gains told to the camera (log), while steering.
    now: LogGains,
    moving: bool,
    /// Confidence, smoothed.
    pub confidence: f32,
    /// The camera's own gains, last seen while it had them.
    anchor: Option<LogGains>,
    frames: u32,
    lost_for: f32,
    found_for: f32,
    /// The newest reading.
    pub last: Option<Reading>,
}

impl Default for Smart {
    fn default() -> Self {
        Smart::new(Model::default())
    }
}

impl Smart {
    pub fn new(model: Model) -> Smart {
        Smart { model, state: State::Camera, now: [0.0; 2], moving: false, confidence: 0.0, anchor: None, frames: 0, lost_for: 0.0, found_for: 0.0, last: None }
    }

    /// The gains to tell the camera, while this steers.
    pub fn gains(&self) -> Option<[f32; 2]> {
        (self.state == State::Steering).then(|| gains_of(self.now))
    }

    /// Left to the camera's own white balance (opted out): starts over.
    pub fn off(&mut self) {
        (self.state, self.moving, self.confidence, self.anchor) = (State::Camera, false, 0.0, None);
        (self.frames, self.lost_for, self.found_for, self.last) = (0, 0.0, 0.0, None);
    }

    /// Where gains fall against the lights' locus: (mired, tint).
    #[cfg(test)]
    pub fn place(&self, gains: [f32; 2]) -> (f32, f32) {
        self.model.locus.place(ln_gains(gains))
    }

    /// The gains to show a light with: its warmth kept, its tint not.
    pub fn shown(&self, light: LogGains) -> LogGains {
        let (m, tint) = self.model.locus.place(light);
        self.model.locus.point(shown_mired(m), tint)
    }

    /// A new frame, `dt` seconds after the last. `held`: the exposure lock
    /// (or the focus check) holds the gains where they are.
    pub fn feed(&mut self, f: &Frame, dt: f32, held: bool) {
        let made = ln_gains(f.gains);
        if self.state == State::Camera {
            self.frames = self.frames.saturating_add(1);
            self.anchor = Some(made);
            if held {
                // Fixed as the camera has them, for as long as it is held.
                self.take(made);
                return;
            }
        } else if held {
            return;
        }
        let reading = self.model.measure(f);
        self.last = reading;
        let conf = reading.map_or(0.0, |r| r.confidence);
        if self.state == State::Camera {
            self.found_for = if conf > FOUND { self.found_for + dt } else { 0.0 };
            if self.frames >= STARTUP_FRAMES && self.found_for >= FOUND_S {
                self.take(made);
                self.confidence = conf;
            }
            return;
        }
        let a = 1.0 - (-dt / 0.3).exp();
        self.confidence += (conf - self.confidence) * a;
        self.lost_for = if self.confidence < LOST { self.lost_for + dt } else { 0.0 };
        if self.lost_for >= LOST_S {
            // Nothing to go by: the camera's own again, from these gains.
            self.state = State::Camera;
            self.frames = STARTUP_FRAMES;
            self.found_for = 0.0;
            return;
        }
        let anchor = self.anchor.unwrap_or(made);
        let ours = reading.map_or(anchor, |r| self.shown(r.light));
        // Wholly this one's from a confidence of a half; the camera's own at none.
        let c = (self.confidence * 2.0).min(1.0);
        let target = [anchor[0] + (ours[0] - anchor[0]) * c, anchor[1] + (ours[1] - anchor[1]) * c];
        self.glide(target, dt);
    }

    fn take(&mut self, made: LogGains) {
        self.state = State::Steering;
        self.now = made;
        self.moving = false;
        self.lost_for = 0.0;
    }

    /// One step of the glide towards `target`.
    fn glide(&mut self, target: LogGains, dt: f32) {
        let d = [target[0] - self.now[0], target[1] - self.now[1]];
        let dist = d[0].hypot(d[1]);
        if !self.moving && dist > START {
            self.moving = true;
        }
        if !self.moving {
            return;
        }
        let a = 1.0 - (-dt * (0.35 + 0.65 * self.confidence) / TAU_S).exp();
        let mut step = [d[0] * a, d[1] * a];
        let most = MOST_PER_S * dt;
        let len = step[0].hypot(step[1]);
        if len > most {
            step = [step[0] * most / len, step[1] * most / len];
        }
        self.now = [self.now[0] + step[0], self.now[1] + step[1]];
        if dist - len < STOP {
            self.moving = false;
        }
    }

    /// For the log: the gains, the light and how sure.
    pub fn words(&self) -> String {
        let g = gains_of(self.now);
        match (self.state, self.last) {
            (State::Steering, Some(r)) => format!("r {:.3} b {:.3} · light {:.0} K tint {:+.3} · sure {:.2}", g[0], g[1], 1e6 / r.mired, r.tint, self.confidence),
            (State::Steering, None) => format!("r {:.3} b {:.3} · no reading", g[0], g[1]),
            (State::Camera, _) => "the camera's own".into(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A light: its temperature, and its green against the curve's (1: on it).
    #[derive(Clone, Copy, Debug)]
    pub struct Light {
        pub k: f32,
        pub green: f32,
    }

    /// A scene: linear sRGB reflectances, `w` × `h`.
    pub struct Scene {
        pub name: String,
        pub w: usize,
        pub h: usize,
        pub px: Vec<[f32; 3]>,
    }

    fn srgb_to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// The gains that make `light` grey.
    pub fn light_gains(model: &Model, light: Light) -> LogGains {
        let c = model.locus.curve(mired(light.k));
        [c[0] + light.green.ln(), c[1] + light.green.ln()]
    }

    /// Scene to the picture's linear RGB: the sensor under `light`, the ISP's
    /// gains, and its colour matrix for the temperature the gains mean.
    fn pipeline(model: &Model, light: Light, gains: [f32; 2]) -> [f32; 9] {
        let t = &model.locus.tuning;
        let lg = model.locus.curve(mired(light.k));
        let (r, b) = (f64::from((-lg[0]).exp()), f64::from((-lg[1]).exp()));
        let g = f64::from(light.green);
        let raw = dng::mul(&[r / g, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, b / g], &dng::invert(&t.ccm(f64::from(light.k))).unwrap());
        let m = model.locus.place(ln_gains(gains)).0.clamp(100.0, 500.0);
        let isp = dng::mul(&t.ccm(1e6 / f64::from(m)), &[f64::from(gains[0]), 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, f64::from(gains[1])]);
        dng::mul(&isp, &raw).map(|v| v as f32)
    }

    fn apply(m: &[f32; 9], s: [f32; 3]) -> [f32; 3] {
        [m[0] * s[0] + m[1] * s[1] + m[2] * s[2], m[3] * s[0] + m[4] * s[1] + m[5] * s[2], m[6] * s[0] + m[7] * s[1] + m[8] * s[2]]
    }

    /// The picture as R'G'B' (8-bit), as the camera would make it.
    pub fn picture(model: &Model, scene: &Scene, light: Light, gains: [f32; 2]) -> Vec<[u8; 3]> {
        let m = pipeline(model, light, gains);
        scene.px.iter().map(|s| apply(&m, *s).map(|c| (gamma(c) * 255.0).round() as u8)).collect()
    }

    /// The picture as the camera's YUV 4:2:0 (full-range BT.601).
    pub fn frame(model: &Model, scene: &Scene, light: Light, gains: [f32; 2]) -> Vec<u8> {
        let rgb = picture(model, scene, light, gains);
        let (w, h) = (scene.w, scene.h);
        let mut out = vec![0u8; w * h * 3 / 2];
        let f = |p: [u8; 3]| p.map(f32::from);
        for (i, p) in rgb.iter().enumerate() {
            let [r, g, b] = f(*p);
            out[i] = (0.299 * r + 0.587 * g + 0.114 * b).round().clamp(0.0, 255.0) as u8;
        }
        let (cw, ch) = (w / 2, h / 2);
        for cy in 0..ch {
            for cx in 0..cw {
                let mut s = [0.0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = f(rgb[(2 * cy + dy) * w + 2 * cx + dx]);
                    for i in 0..3 {
                        s[i] += p[i] / 4.0;
                    }
                }
                let [r, g, b] = s;
                out[w * h + cy * cw + cx] = (-0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0).round().clamp(0.0, 255.0) as u8;
                out[w * h + cw * ch + cy * cw + cx] = (0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }

    fn lab(rgb: [f32; 3]) -> [f32; 3] {
        let x = 0.412_456_4 * rgb[0] + 0.357_576_1 * rgb[1] + 0.180_437_5 * rgb[2];
        let y = 0.212_672_9 * rgb[0] + 0.715_152_2 * rgb[1] + 0.072_175 * rgb[2];
        let z = 0.019_333_9 * rgb[0] + 0.119_192 * rgb[1] + 0.950_304_1 * rgb[2];
        let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
        let (fx, fy, fz) = (f(x / 0.950_47), f(y), f(z / 1.088_83));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// A mid-grey card's colour in the picture (linear sRGB), at the grey's
    /// own brightness.
    fn grey_seen(model: &Model, light: Light, gains: [f32; 2]) -> [f32; 3] {
        let c = apply(&pipeline(model, light, gains), [0.18; 3]);
        let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        c.map(|v| v * 0.18 / y.max(1e-6))
    }

    /// ΔE (CIE 1976) between a grey card's colour with these gains and with those.
    pub fn grey_delta_e(model: &Model, light: Light, gains: [f32; 2], against: [f32; 2]) -> f32 {
        let (a, b) = (lab(grey_seen(model, light, gains)), lab(grey_seen(model, light, against)));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    /// What the picture should be balanced with: grey for a daylight (the
    /// lean's touch of warmth on it), a little warmth kept for a lamp.
    pub fn wanted(model: &Model, light: Light) -> [f32; 2] {
        let smart = Smart::new(model.clone());
        let (m, tint) = model.locus.place(light_gains(model, light));
        gains_of(smart.shown(model.locus.point(m - LEAN_MIRED, tint)))
    }

    /// The closed loop: the camera's own gains first, then this steering,
    /// each frame made with the gains last told. The gains told each frame.
    pub fn steer(model: &Model, scene: &Scene, light: Light, start: [f32; 2], frames: usize, face: Option<[f32; 4]>) -> (Smart, Vec<[f32; 2]>) {
        let mut smart = Smart::new(model.clone());
        let mut path = Vec::new();
        let mut gains = start;
        for _ in 0..frames {
            let yuv = frame(model, scene, light, gains);
            let f = Frame { yuv: &yuv, w: scene.w, h: scene.h, gains, ct: 0, lux: 300.0, face };
            smart.feed(&f, 1.0 / 30.0, false);
            gains = smart.gains().unwrap_or(start);
            path.push(gains);
        }
        (smart, path)
    }

    // ------------------------------------------------------------ scenes ---

    /// A room: a grey wall lit unevenly, a white sheet, a wooden table, red
    /// and blue things, a plant.
    pub fn room() -> Scene {
        let (w, h) = (160, 120);
        let mut px = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let shade = 0.55 + 0.45 * (x as f32 / w as f32);
                let mut c = [0.30 * shade; 3];
                if y > 80 {
                    c = [0.30, 0.16, 0.07];
                }
                if (20..60).contains(&x) && (20..55).contains(&y) {
                    c = [0.80; 3];
                }
                if (90..115).contains(&x) && (50..80).contains(&y) {
                    c = [0.45, 0.05, 0.04];
                }
                if (125..150).contains(&x) && (30..80).contains(&y) {
                    c = [0.05, 0.10, 0.40];
                }
                if (65..85).contains(&x) && (60..80).contains(&y) {
                    c = [0.06, 0.20, 0.05];
                }
                let n = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.02;
                px[y * w + x] = c.map(|v| (v + n * v).max(0.0));
            }
        }
        Scene { name: "room".into(), w, h, px }
    }

    /// One colour filling the picture, a little textured.
    pub fn plain(name: &str, c: [f32; 3]) -> Scene {
        let (w, h) = (160, 120);
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let k = 0.8 + 0.4 * (((x * 31 + y * 17) % 23) as f32 / 23.0) * (y as f32 / h as f32 + 0.5);
                c.map(|v| v * k)
            })
            .collect();
        Scene { name: name.into(), w, h, px }
    }

    /// A red wall with a grey card on it (an eighth of the picture).
    pub fn red_wall() -> Scene {
        let mut s = plain("red wall", [0.42, 0.06, 0.05]);
        for y in 45..75 {
            for x in 60..100 {
                s.px[y * s.w + x] = [0.25; 3];
            }
        }
        s
    }

    /// A Kodak test picture (`kodim*.png`), at 160 × 120 (each pixel the
    /// mean of its part, in linear light).
    pub fn kodak(path: &std::path::Path) -> Option<Scene> {
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        let ch = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => return None,
        };
        let (iw, ih) = (info.width as usize, info.height as usize);
        let (w, h) = (160, 120);
        let mut px = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let (x0, x1, y0, y1) = (x * iw / w, (x + 1) * iw / w, y * ih / h, (y + 1) * ih / h);
                let mut s = [0.0f32; 3];
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let a = (yy * iw + xx) * ch;
                        for c in 0..3 {
                            s[c] += srgb_to_linear(f32::from(buf[a + c]) / 255.0);
                        }
                    }
                }
                let n = ((x1 - x0) * (y1 - y0)) as f32;
                // Exposed as a camera would: its brightest a little under white.
                px[y * w + x] = s.map(|v| v / n * 0.85);
            }
        }
        let name = path.file_stem()?.to_string_lossy().into_owned();
        Some(Scene { name, w, h, px })
    }

    /// The Kodak pictures, where they are (`CATALYST_KODAK`, or the folder
    /// they were first tried from): none on a computer without them.
    pub fn kodaks() -> Vec<Scene> {
        let dir = std::env::var("CATALYST_KODAK").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(r"C:\Users\User\AppData\Local\Temp\claude\C--Users-User\285ae435-f125-40f4-935e-c7f2c06ea5ab\scratchpad\photos"));
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "png")).collect()).unwrap_or_default();
        files.sort();
        files.iter().filter_map(|p| kodak(p)).collect()
    }

    /// The casts tried: the light, and the camera's own gains under it.
    pub fn casts(model: &Model) -> Vec<(&'static str, Light, [f32; 2])> {
        let at = |k: f32| gains_of(model.locus.curve(mired(k)));
        let five = at(5000.0);
        vec![
            // Tungsten, the camera's own balanced for 3600 K (the yellow).
            ("tungsten 2700 K, yellow", Light { k: 2700.0, green: 1.0 }, at(3600.0)),
            // Fluorescent: 4000 K with a green the curve has not.
            ("fluorescent 4000 K, green", Light { k: 4000.0, green: 1.08 }, at(4000.0)),
            // Shade, balanced for daylight: blue.
            ("shade 7500 K, blue", Light { k: 7500.0, green: 1.0 }, at(5500.0)),
            // Extremes: a candle-warm lamp balanced for daylight, daylight
            // balanced for a lamp, and a magenta 15 % off.
            ("lamp 2300 K, orange", Light { k: 2300.0, green: 1.0 }, at(5500.0)),
            ("daylight 5500 K, deep blue", Light { k: 5500.0, green: 1.0 }, at(2500.0)),
            ("daylight 5000 K, magenta", Light { k: 5000.0, green: 1.0 }, [five[0] * 1.15, five[1] * 1.15]),
        ]
    }

    // ------------------------------------------------------------- tests ---

    #[test]
    fn the_locus_round_trips_and_holds_gains_to_it() {
        let model = Model::default();
        let l = &model.locus;
        for k in [2300.0f32, 2700.0, 3200.0, 4000.0, 5000.0, 6500.0, 8000.0] {
            for tint in [-0.1f32, 0.0, 0.08] {
                let (m, t) = l.place(l.point(mired(k), tint));
                assert!((m - mired(k)).abs() < 1.5 && (t - tint).abs() < 0.01, "{k} K {tint}: {m} {t}");
            }
        }
        // Anything at all comes back on the curve, 2300 to 8000 K, a little either side.
        for r in [0.3f32, 0.8, 1.5, 2.5, 4.0] {
            for b in [0.3f32, 0.8, 1.5, 2.5, 4.0] {
                let (m, t) = l.place(l.constrain(ln_gains([r, b])));
                assert!((mired(COOLEST_K) - 0.5..=mired(WARMEST_K) + 0.5).contains(&m), "{r} {b}: {m}");
                assert!(t.abs() <= TINT_ROOM + 0.005, "{r} {b}: tint {t}");
            }
        }
        // No step where the tuning's curve has one (2911 to 2919 K).
        let (a, b) = (l.curve(mired(2905.0)), l.curve(mired(2925.0)));
        assert!(dist2(a, b).sqrt() < 0.02, "{a:?} {b:?}");
        // The curve's gains are the tuning's: 2.31 red at 8575 K.
        let g = gains_of(l.curve(mired(8575.0)));
        assert!((g[0] - 1.0 / 0.4331).abs() < 0.01 && (g[1] - 1.0 / 0.8037).abs() < 0.01, "{g:?}");
    }

    #[test]
    fn warm_light_keeps_a_documented_warmth() {
        assert_eq!(kept_warmth(mired(5000.0)), 0.0);
        assert_eq!(kept_warmth(mired(4500.0)), 0.0);
        assert!((kept_warmth(mired(2700.0)) - 0.10).abs() < 1e-4);
        assert!((kept_warmth(mired(2300.0)) - 0.14).abs() < 1e-4);
        assert!((kept_warmth(mired(1800.0)) - 0.14).abs() < 1e-4);
        // A 2700 K lamp balanced as about 2830 K; daylight as itself.
        let shown = |k: f32| 1e6 / shown_mired(mired(k));
        assert!((shown(2700.0) - 2830.0).abs() < 15.0, "{}", shown(2700.0));
        assert!((shown(3200.0) - 3270.0).abs() < 15.0, "{}", shown(3200.0));
        assert!((shown(2300.0) - 2490.0).abs() < 15.0, "{}", shown(2300.0));
        assert_eq!(shown_mired(mired(6500.0)), mired(6500.0));
        // Warmer lights are always shown warmer.
        let mut last = 0.0;
        for m in (100..520).map(|m| m as f32) {
            let s = shown_mired(m);
            assert!(s > last, "{m}");
            last = s;
        }
    }

    /// Each cast on each scene, steered for three seconds: the grey card's
    /// ΔE against what is wanted, before and after.
    fn recover(scenes: &[Scene]) -> Vec<(String, f32, f32)> {
        let model = Model::default();
        let mut out = Vec::new();
        for scene in scenes {
            for (name, light, start) in casts(&model) {
                let (_, path) = steer(&model, scene, light, start, 90, None);
                let end = *path.last().unwrap();
                let want = wanted(&model, light);
                let neutral = gains_of(light_gains(&model, light));
                let before = grey_delta_e(&model, light, start, neutral);
                let after = grey_delta_e(&model, light, end, want);
                eprintln!("{:>8} · {name:<28} ΔE before {before:5.1} · after {after:4.1} (to grey {:4.1})", scene.name, grey_delta_e(&model, light, end, neutral));
                out.push((format!("{} · {name}", scene.name), before, after));
            }
        }
        out
    }

    #[test]
    fn casts_on_a_room_come_back_to_grey() {
        for (what, before, after) in recover(&[room()]) {
            assert!(after < 3.0 && after < before, "{what}: {before} → {after}");
        }
    }

    #[test]
    fn casts_on_kodak_pictures_come_back_to_grey() {
        let scenes = kodaks();
        if scenes.is_empty() {
            eprintln!("no Kodak pictures here (CATALYST_KODAK): skipped");
            return;
        }
        let results = recover(&scenes);
        let n = results.len() as f32;
        let mean_before = results.iter().map(|r| r.1).sum::<f32>() / n;
        let mean_after = results.iter().map(|r| r.2).sum::<f32>() / n;
        let good = results.iter().filter(|r| r.2 < 3.0).count();
        let median = {
            let mut a: Vec<f32> = results.iter().map(|r| r.2).collect();
            a.sort_by(f32::total_cmp);
            a[a.len() / 2]
        };
        eprintln!("{} pictures × {} casts: mean ΔE {mean_before:.1} → {mean_after:.1} (median {median:.1}); {good} under 3", scenes.len(), results.len() / scenes.len());
        // The Kodak pictures' own whites lean yellow (b/g 0.93 to 0.95 in
        // linear light), and their grey surfaces warmer still: what is left
        // is mostly that, a touch cool.
        assert!(mean_after < mean_before / 5.0 && mean_after < 8.0, "mean ΔE {mean_before} → {mean_after}");
        for (what, before, after) in &results {
            assert!(after < before && *after < 15.0, "{what}: {before} → {after}");
        }
    }

    #[test]
    fn one_colour_scenes_fall_back_to_the_cameras_own() {
        let model = Model::default();
        let light = Light { k: 5500.0, green: 1.0 };
        let own = gains_of(model.locus.curve(mired(5200.0)));
        for scene in [plain("grass", [0.06, 0.22, 0.03]), plain("sky", [0.12, 0.25, 0.65]), plain("leaves", [0.10, 0.20, 0.02])] {
            let yuv = frame(&model, &scene, light, own);
            let r = model.measure(&Frame { yuv: &yuv, w: scene.w, h: scene.h, gains: own, ct: 0, lux: 300.0, face: None });
            let conf = r.map_or(0.0, |r| r.confidence);
            assert!(conf < 0.2, "{}: sure {conf}", scene.name);
            // Steered: never far from the camera's own, and handed back to it.
            let (smart, path) = steer(&model, &scene, light, own, 150, None);
            for g in &path {
                assert!(dist2(ln_gains(*g), ln_gains(own)).sqrt() < 0.05, "{}: {g:?} against {own:?}", scene.name);
            }
            assert_eq!(smart.state, State::Camera, "{}", scene.name);
        }
    }

    #[test]
    fn a_red_wall_cannot_turn_the_picture_blue_green() {
        let model = Model::default();
        let light = Light { k: 4500.0, green: 1.0 };
        let neutral = gains_of(light_gains(&model, light));
        let (_, path) = steer(&model, &red_wall(), light, neutral, 90, None);
        let end = *path.last().unwrap();
        let (_, tint) = model.locus.place(ln_gains(end));
        assert!(tint.abs() <= TINT_ROOM + 0.01, "tint {tint}");
        let de = grey_delta_e(&model, light, end, wanted(&model, light));
        assert!(de < 4.0, "the grey card {de}");
    }

    #[test]
    fn a_face_keeps_skin_from_going_blue() {
        let model = Model::default();
        // A wall of warm beige under daylight (it reads as a lamp's grey)
        // with a face in the middle.
        let mut scene = plain("beige", [0.42, 0.30, 0.17]);
        for y in 30..90 {
            for x in 55..105 {
                scene.px[y * scene.w + x] = SKINS[0];
            }
        }
        let light = Light { k: 5500.0, green: 1.0 };
        let neutral = gains_of(light_gains(&model, light));
        let face = Some([55.0 / 160.0, 30.0 / 120.0, 105.0 / 160.0, 90.0 / 120.0]);
        let measure = |face| {
            let yuv = frame(&model, &scene, light, neutral);
            model.measure(&Frame { yuv: &yuv, w: scene.w, h: scene.h, gains: neutral, ct: 0, lux: 300.0, face }).unwrap()
        };
        let (without, with) = (measure(None), measure(face));
        // The face pulls the estimate back towards the real light.
        let truth = mired(5500.0);
        eprintln!("beige wall: {:.0} K without the face, {:.0} K with it (5500 K)", 1e6 / without.mired, 1e6 / with.mired);
        assert!((with.mired - truth).abs() < (without.mired - truth).abs(), "without {} with {} (truth {truth})", without.mired, with.mired);
    }

    #[test]
    fn a_still_scene_stays_still_and_a_new_one_settles_within_a_second() {
        let model = Model::default();
        let scene = room();
        let lamp = Light { k: 2700.0, green: 1.0 };
        let start = gains_of(model.locus.curve(mired(3600.0)));
        let (mut smart, path) = steer(&model, &scene, lamp, start, 120, None);
        // No hunting: once there, not one change in the last two seconds.
        let tail = &path[60..];
        assert!(tail.iter().all(|g| g == &tail[0]), "{:?}", &tail[..5]);
        // Never past where it ends (no overshoot, no ringing).
        let end = ln_gains(*path.last().unwrap());
        let mut worst = 0.0f32;
        let mut nearest = f32::MAX;
        for g in &path[STARTUP_FRAMES as usize..] {
            let d = dist2(ln_gains(*g), end).sqrt();
            worst = worst.max(d - nearest);
            nearest = nearest.min(d);
        }
        assert!(worst < 0.004, "moved away by {worst}");
        // Then daylight: within 2 % of where it ends in a second.
        let day = Light { k: 6000.0, green: 1.0 };
        let mut gains = *path.last().unwrap();
        let mut seen = Vec::new();
        for _ in 0..120 {
            let yuv = frame(&model, &scene, day, gains);
            smart.feed(&Frame { yuv: &yuv, w: scene.w, h: scene.h, gains, ct: 0, lux: 300.0, face: None }, 1.0 / 30.0, false);
            gains = smart.gains().unwrap();
            seen.push(gains);
        }
        let end = ln_gains(gains);
        let settled = seen.iter().position(|g| dist2(ln_gains(*g), end).sqrt() < 0.02).unwrap();
        eprintln!("2700 K → 6000 K: within 2 % after {settled} frames ({:.2} s)", settled as f32 / 30.0);
        assert!(settled <= 30, "{settled} frames");
        assert!(grey_delta_e(&model, day, gains, wanted(&model, day)) < 2.0);
    }

    #[test]
    fn held_gains_do_not_move() {
        let model = Model::default();
        let scene = room();
        let lamp = Light { k: 2700.0, green: 1.0 };
        let start = gains_of(model.locus.curve(mired(4000.0)));
        let mut smart = Smart::new(model.clone());
        // The lock from the first frame: the camera's own gains, fixed.
        for _ in 0..40 {
            let yuv = frame(&model, &scene, lamp, start);
            smart.feed(&Frame { yuv: &yuv, w: scene.w, h: scene.h, gains: start, ct: 0, lux: 300.0, face: None }, 1.0 / 30.0, true);
        }
        assert_eq!(smart.gains(), Some(gains_of(ln_gains(start))));
        // Off: the camera's own again.
        smart.off();
        assert_eq!(smart.gains(), None);
    }

    /// The before-and-after sheet: each picture with each cast as the
    /// camera's own gains left it, and steered. Written where
    /// `CATALYST_WB_SHEET` says.
    #[test]
    #[ignore]
    fn contact_sheet() {
        let Ok(out) = std::env::var("CATALYST_WB_SHEET") else { return };
        let model = Model::default();
        let mut scenes = kodaks();
        scenes.push(room());
        let casts = casts(&model);
        let (tw, th, gap) = (160usize, 120usize, 4usize);
        // Each cast a pair (as the camera left it, steered), pairs apart.
        let pair = 2 * (tw + gap) + 2 * gap;
        let (w, h) = (gap + casts.len() * pair, scenes.len() * (th + gap) + gap);
        let mut img = vec![24u8; w * h * 3];
        for (row, scene) in scenes.iter().enumerate() {
            for (i, (_, light, start)) in casts.iter().enumerate() {
                let (_, path) = steer(&model, scene, *light, *start, 90, None);
                for (j, gains) in [*start, *path.last().unwrap()].into_iter().enumerate() {
                    let pic = picture(&model, scene, *light, gains);
                    let (ox, oy) = (gap + i * pair + j * (tw + gap), gap + row * (th + gap));
                    for y in 0..th {
                        for x in 0..tw {
                            let a = ((oy + y) * w + ox + x) * 3;
                            img[a..a + 3].copy_from_slice(&pic[y * scene.w + x]);
                        }
                    }
                }
            }
        }
        let file = std::fs::File::create(&out).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&img).unwrap();
        eprintln!("wrote {out}");
    }
}
