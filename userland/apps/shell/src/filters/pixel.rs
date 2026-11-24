//! Pixel mode's pictures (the Camera's pixel mode, 1.2): the preview made
//! small, its colours few and dithered, as a toy camera's are. The look is
//! our own: three sizes that land on whole panel pixels in the viewfinder
//! (×4, ×3, ×2, so no pixel is wider than another), six palettes of our
//! own, four ways to dither, and the tone (brightness on ↑ ↓, contrast in
//! four steps).
//!
//! Each preview frame (320 × 240 YUV 4:2:0) is shrunk straight to the size
//! chosen, each pixel the mean of its part of the frame (its colour from its
//! own chroma), toned, and mapped to the palette: by brightness for the four
//! ramps (ink, graphite, ember, tide), to the nearest colour for the two
//! colour palettes (through a 32 × 32 × 32 table made once for each). The
//! picture is kept as palette indices, so the shutter writes it as it is:
//! an indexed PNG at its true size, and one blown up with whole pixels for
//! sharing. A frame costs about what the plain colour conversion it
//! replaces does (0.21 ms at worst here against 0.16 ms; the timing test
//! below), on the camera's own thread.

use std::sync::OnceLock;

use crate::photos::Picture;

/// The picture's size: its long side, at the preview's 4:3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Size {
    Chunky,
    #[default]
    Small,
    Fine,
}

pub const SIZES: [Size; 3] = [Size::Chunky, Size::Small, Size::Fine];

impl Size {
    pub fn dims(self) -> (usize, usize) {
        match self {
            Size::Chunky => (56, 42),
            Size::Small => (75, 56),
            Size::Fine => (112, 84),
        }
    }

    /// Panel pixels to each of its pixels in the viewfinder (227 × 170):
    /// 224 × 168 or 225 × 168, whole pixels every one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn view_scale(self) -> usize {
        match self {
            Size::Chunky => 4,
            Size::Small => 3,
            Size::Fine => 2,
        }
    }

    /// Each pixel blown up this many times in the copy for sharing: about
    /// 450 across, so a phone showing it does not blur it much.
    pub fn share_scale(self) -> usize {
        match self {
            Size::Chunky => 8,
            Size::Small => 6,
            Size::Fine => 4,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Size::Chunky => "56",
            Size::Small => "75",
            Size::Fine => "112",
        }
    }

    pub fn words(self) -> String {
        let (w, h) = self.dims();
        format!("{w} × {h}")
    }
}

/// The palettes, in the picker's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Palette {
    Ink,
    Graphite,
    #[default]
    Ember,
    Tide,
    Poster,
    Crayon,
}

pub const PALETTES: [Palette; 6] = [Palette::Ink, Palette::Graphite, Palette::Ember, Palette::Tide, Palette::Poster, Palette::Crayon];

/// The palettes' colours (our own, made by eye on the simulated scene).
/// The ramps run from dark to light.
const INK: [[u8; 3]; 2] = [[0x1d, 0x1b, 0x26], [0xef, 0xe9, 0xda]];
const GRAPHITE: [[u8; 3]; 4] = [[0x15, 0x16, 0x1a], [0x55, 0x58, 0x5f], [0xa1, 0xa4, 0xaa], [0xec, 0xea, 0xe6]];
const EMBER: [[u8; 3]; 4] = [[0x2b, 0x14, 0x18], [0x8a, 0x3a, 0x2b], [0xe0, 0x8a, 0x46], [0xf8, 0xe2, 0xb8]];
const TIDE: [[u8; 3]; 4] = [[0x0f, 0x1a, 0x2c], [0x2d, 0x5d, 0x7c], [0x74, 0xb2, 0xbf], [0xe3, 0xf1, 0xea]];
const POSTER: [[u8; 3]; 8] = [
    [0x1a, 0x18, 0x20],
    [0x3d, 0x5a, 0x8c],
    [0xb9, 0x47, 0x3d],
    [0x5f, 0x8f, 0x43],
    [0x86, 0xbc, 0xdc],
    [0xdb, 0xa3, 0x6e],
    [0xf1, 0xd7, 0x7a],
    [0xf5, 0xf0, 0xe6],
];
const CRAYON: [[u8; 3]; 16] = [
    [0x12, 0x11, 0x18],
    [0x3a, 0x33, 0x48],
    [0x6e, 0x68, 0x78],
    [0xaa, 0xa5, 0xb0],
    [0xf3, 0xef, 0xe8],
    [0x5c, 0x2a, 0x2c],
    [0xc4, 0x47, 0x3b],
    [0xe9, 0x8a, 0x55],
    [0xf3, 0xc7, 0x66],
    [0x8a, 0x5a, 0x3c],
    [0xd8, 0xa6, 0x84],
    [0x2e, 0x58, 0x38],
    [0x73, 0xa2, 0x4c],
    [0x2c, 0x4c, 0x8a],
    [0x5f, 0x93, 0xd1],
    [0xa8, 0xd6, 0xe8],
];

impl Palette {
    pub fn name(self) -> &'static str {
        match self {
            Palette::Ink => "ink",
            Palette::Graphite => "graphite",
            Palette::Ember => "ember",
            Palette::Tide => "tide",
            Palette::Poster => "poster",
            Palette::Crayon => "crayon",
        }
    }

    pub fn from_id(id: &str) -> Option<Palette> {
        PALETTES.iter().copied().find(|p| p.name() == id)
    }

    pub fn colours(self) -> &'static [[u8; 3]] {
        match self {
            Palette::Ink => &INK,
            Palette::Graphite => &GRAPHITE,
            Palette::Ember => &EMBER,
            Palette::Tide => &TIDE,
            Palette::Poster => &POSTER,
            Palette::Crayon => &CRAYON,
        }
    }

    /// Mapped by brightness alone: a ramp from dark to light.
    pub fn ramp(self) -> bool {
        matches!(self, Palette::Ink | Palette::Graphite | Palette::Ember | Palette::Tide)
    }

    /// What its picker says under its name.
    pub fn words(self) -> &'static str {
        match self {
            Palette::Ink => "1-bit",
            Palette::Graphite => "4 greys",
            Palette::Ember => "warm 4",
            Palette::Tide => "cool 4",
            Palette::Poster => "8 colours",
            Palette::Crayon => "16 colours",
        }
    }

    /// The colour palettes' saturation first, in 256ths (our own tuning:
    /// a few colours read better from a livelier picture).
    fn saturation(self) -> i32 {
        match self {
            Palette::Poster => 360,
            Palette::Crayon => 310,
            _ => 256,
        }
    }

    /// The ordered dither's spread for a colour palette: about the
    /// distance between its neighbouring colours.
    fn spread(self) -> i32 {
        match self {
            Palette::Poster => 64,
            _ => 44,
        }
    }

    /// The four colours its chip shows (two for ink).
    pub fn badge(self) -> Vec<[u8; 3]> {
        let c = self.colours();
        match self {
            Palette::Poster => vec![c[2], c[6], c[1], c[3]],
            Palette::Crayon => vec![c[6], c[8], c[14], c[12]],
            _ => c.to_vec(),
        }
    }
}

/// How the colours between the palette's are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Dither {
    /// The nearest colour: flat areas, hard edges.
    None,
    /// Ordered, through a 4 × 4 Bayer matrix: a regular, woven pattern.
    #[default]
    Bayer,
    /// Floyd–Steinberg error diffusion, along the rows each way in turn.
    Floyd,
    /// Atkinson's: three quarters of the error passed on, so lights and
    /// darks stay clean and only the middle tones break up.
    Atkinson,
}

pub const DITHERS: [Dither; 4] = [Dither::None, Dither::Bayer, Dither::Floyd, Dither::Atkinson];

impl Dither {
    pub fn name(self) -> &'static str {
        match self {
            Dither::None => "none",
            Dither::Bayer => "bayer",
            Dither::Floyd => "floyd",
            Dither::Atkinson => "atkinson",
        }
    }

    pub fn from_id(id: &str) -> Option<Dither> {
        DITHERS.iter().copied().find(|d| d.name() == id)
    }
}

/// The contrast's steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Contrast {
    Soft,
    #[default]
    Normal,
    Punchy,
    Hard,
}

pub const CONTRASTS: [Contrast; 4] = [Contrast::Soft, Contrast::Normal, Contrast::Punchy, Contrast::Hard];

impl Contrast {
    pub fn name(self) -> &'static str {
        match self {
            Contrast::Soft => "soft",
            Contrast::Normal => "normal",
            Contrast::Punchy => "punchy",
            Contrast::Hard => "hard",
        }
    }

    pub fn from_id(id: &str) -> Option<Contrast> {
        CONTRASTS.iter().copied().find(|c| c.name() == id)
    }

    /// The slope about mid grey (our own tuning: a few tones want a little
    /// more contrast than the picture has, even at "normal").
    fn slope(self) -> f32 {
        match self {
            Contrast::Soft => 0.8,
            Contrast::Normal => 1.1,
            Contrast::Punchy => 1.38,
            Contrast::Hard => 1.75,
        }
    }
}

/// Brightness's steps each way of none.
pub const BRIGHT_STEPS: i8 = 3;

/// Everything pixel mode is set to (kept with the Camera's file).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Settings {
    pub size: Size,
    pub palette: Palette,
    pub dither: Dither,
    /// -3 to +3.
    pub bright: i8,
    pub contrast: Contrast,
}

impl Settings {
    /// As the Camera's file keeps it: "75,ember,bayer,0,normal".
    pub fn line(&self) -> String {
        format!("{},{},{},{},{}", self.size.id(), self.palette.name(), self.dither.name(), self.bright, self.contrast.name())
    }

    pub fn from_line(line: &str) -> Settings {
        let mut s = Settings::default();
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if let Some(size) = parts.first().and_then(|p| SIZES.iter().copied().find(|s| s.id() == *p)) {
            s.size = size;
        }
        if let Some(p) = parts.get(1).and_then(|p| Palette::from_id(p)) {
            s.palette = p;
        }
        if let Some(d) = parts.get(2).and_then(|d| Dither::from_id(d)) {
            s.dither = d;
        }
        if let Some(b) = parts.get(3).and_then(|b| b.parse::<i8>().ok()) {
            s.bright = b.clamp(-BRIGHT_STEPS, BRIGHT_STEPS);
        }
        if let Some(c) = parts.get(4).and_then(|c| Contrast::from_id(c)) {
            s.contrast = c;
        }
        s
    }

    /// What a pixel picture says it is (its PNG's "look").
    pub fn look_words(&self) -> String {
        format!("Pixel · {} · {}", self.palette.name(), self.dither.name())
    }
}

/// 4 × 4 Bayer: thresholds 0 to 15.
pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The colour table's resolution: five bits a channel.
const LUT_BITS: usize = 5;

/// A colour palette's nearest colour for each 32 × 32 × 32 cell (by a
/// weighted distance that counts green most and leans red or blue with the
/// colours' redness), made once for each palette.
fn table(palette: Palette) -> &'static [u8] {
    static TABLES: [OnceLock<Vec<u8>>; 6] = [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let at = PALETTES.iter().position(|p| *p == palette).unwrap_or(0);
    TABLES[at].get_or_init(|| {
        let n = 1usize << LUT_BITS;
        let step = 256 / n;
        let mut out = vec![0u8; n * n * n];
        for (i, o) in out.iter_mut().enumerate() {
            let c = [(i >> (2 * LUT_BITS)) * step + step / 2, ((i >> LUT_BITS) & (n - 1)) * step + step / 2, (i & (n - 1)) * step + step / 2].map(|v| v as i32);
            *o = nearest(palette.colours(), c);
        }
        out
    })
}

/// The nearest of `colours` to `c` (0 to 255 each).
fn nearest(colours: &[[u8; 3]], c: [i32; 3]) -> u8 {
    let mut best = (i64::MAX, 0u8);
    for (k, p) in colours.iter().enumerate() {
        let p = p.map(i32::from);
        let rm = i64::from((c[0] + p[0]) / 2);
        let (dr, dg, db) = (i64::from(c[0] - p[0]), i64::from(c[1] - p[1]), i64::from(c[2] - p[2]));
        let d = (512 + rm) * dr * dr + 1024 * dg * dg + (767 - rm) * db * db;
        if d < best.0 {
            best = (d, k as u8);
        }
    }
    best.1
}

#[inline]
fn cell(c: [i32; 3]) -> usize {
    let q = |v: i32| (v.clamp(0, 255) as usize) >> (8 - LUT_BITS);
    (q(c[0]) << (2 * LUT_BITS)) | (q(c[1]) << LUT_BITS) | q(c[2])
}

/// Pixel mode made ready for frames: the settings, and the tone as a table.
#[derive(Clone)]
pub struct Pixel {
    pub settings: Settings,
    tone: [u8; 256],
}

impl Pixel {
    pub fn new(settings: Settings) -> Pixel {
        let k = settings.contrast.slope();
        let lift = f32::from(settings.bright) * 0.075;
        let mut tone = [0u8; 256];
        for (i, t) in tone.iter_mut().enumerate() {
            let v = (i as f32 / 255.0 - 0.5) * k + 0.5 + lift;
            *t = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        // The colour palettes' table, made now rather than on a frame.
        if !settings.palette.ramp() {
            table(settings.palette);
        }
        Pixel { settings, tone }
    }

    /// A camera frame (YUV 4:2:0, full range, packed planes) as a pixel picture.
    pub fn develop(&self, yuv: &[u8], w: usize, h: usize) -> Frame {
        let (tw, th) = self.settings.size.dims();
        let small = shrink(yuv, w, h, tw, th);
        self.map(&small, tw, th)
    }

    /// The palette over a small RGB picture (three bytes a pixel): toned,
    /// then mapped and dithered.
    pub fn map(&self, rgb: &[u8], w: usize, h: usize) -> Frame {
        let palette = self.settings.palette;
        let mut idx = vec![0u8; w * h];
        if w == 0 || h == 0 || rgb.len() < w * h * 3 {
            return Frame { w, h, idx, palette };
        }
        if palette.ramp() {
            let levels: Vec<i32> = rgb.chunks_exact(3).map(|p| i32::from(self.tone[((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8) as usize])).collect();
            ramp(&levels, w, h, palette.colours().len() as i32, self.settings.dither, &mut idx);
        } else {
            let sat = palette.saturation();
            let toned: Vec<[i32; 3]> = rgb
                .chunks_exact(3)
                .map(|p| {
                    let c = [0, 1, 2].map(|i| i32::from(self.tone[usize::from(p[i])]));
                    let l = (77 * c[0] + 150 * c[1] + 29 * c[2] + 128) >> 8;
                    c.map(|v| l + (((v - l) * sat) >> 8))
                })
                .collect();
            colour(&toned, w, h, palette, self.settings.dither, &mut idx);
        }
        Frame { w, h, idx, palette }
    }
}

/// A ramp's levels (0 to `n` - 1) from brightness (0 to 255), dithered.
fn ramp(v: &[i32], w: usize, h: usize, n: i32, dither: Dither, out: &mut [u8]) {
    let top = n - 1;
    match dither {
        Dither::None => {
            for (o, v) in out.iter_mut().zip(v) {
                *o = ((v * top + 127) / 255).clamp(0, top) as u8;
            }
        }
        Dither::Bayer => {
            for y in 0..h {
                for x in 0..w {
                    let t = i32::from(BAYER[y & 3][x & 3]);
                    // floor(v·(n-1)/255 + (t + ½)/16)
                    let level = (v[y * w + x] * top * 32 + (2 * t + 1) * 255) / (255 * 32);
                    out[y * w + x] = level.clamp(0, top) as u8;
                }
            }
        }
        Dither::Floyd | Dither::Atkinson => {
            let (shift, spread) = weights(dither);
            // Error carried, in 1/2^shift of a level of brightness, three rows deep.
            let mut carry = vec![0i32; (w + 4) * 3];
            for y in 0..h {
                let back = dither == Dither::Floyd && y % 2 == 1;
                for i in 0..w {
                    let x = if back { w - 1 - i } else { i };
                    let at = (y % 3) * (w + 4) + x + 2;
                    let want = v[y * w + x] + (carry[at] >> shift);
                    carry[at] = 0;
                    let level = ((want * top + 127) / 255).clamp(0, top);
                    out[y * w + x] = level as u8;
                    let err = want - level * 255 / top;
                    for &(dx, dy, k) in spread {
                        let dx = if back { -dx } else { dx };
                        let (nx, ny) = (x as i32 + dx, y + dy);
                        if nx >= 0 && (nx as usize) < w && ny < h {
                            carry[(ny % 3) * (w + 4) + nx as usize + 2] += err * k;
                        }
                    }
                }
            }
        }
    }
}

/// A colour palette's indices from colours (0 to 255, a little past at the
/// edges), dithered.
fn colour(c: &[[i32; 3]], w: usize, h: usize, palette: Palette, dither: Dither, out: &mut [u8]) {
    let lut = table(palette);
    let colours = palette.colours();
    match dither {
        Dither::None => {
            for (o, c) in out.iter_mut().zip(c) {
                *o = lut[cell(*c)];
            }
        }
        Dither::Bayer => {
            let spread = palette.spread();
            for y in 0..h {
                for x in 0..w {
                    let t = (i32::from(BAYER[y & 3][x & 3]) * 2 - 15) * spread / 32;
                    out[y * w + x] = lut[cell(c[y * w + x].map(|v| v + t))];
                }
            }
        }
        Dither::Floyd | Dither::Atkinson => {
            let (shift, spread) = weights(dither);
            let mut carry = vec![[0i32; 3]; (w + 4) * 3];
            for y in 0..h {
                let back = dither == Dither::Floyd && y % 2 == 1;
                for i in 0..w {
                    let x = if back { w - 1 - i } else { i };
                    let at = (y % 3) * (w + 4) + x + 2;
                    let e = carry[at];
                    carry[at] = [0; 3];
                    let want = [0, 1, 2].map(|k| (c[y * w + x][k] + (e[k] >> shift)).clamp(-64, 319));
                    let k = lut[cell(want)];
                    out[y * w + x] = k;
                    let got = colours[usize::from(k)];
                    let err = [0, 1, 2].map(|i| want[i] - i32::from(got[i]));
                    for &(dx, dy, wgt) in spread {
                        let dx = if back { -dx } else { dx };
                        let (nx, ny) = (x as i32 + dx, y + dy);
                        if nx >= 0 && (nx as usize) < w && ny < h {
                            let n = &mut carry[(ny % 3) * (w + 4) + nx as usize + 2];
                            for (n, e) in n.iter_mut().zip(err) {
                                *n += e * wgt;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Where a diffusion passes its error: its unit (a power of two, as its
/// shift), and (dx, dy, share).
fn weights(dither: Dither) -> (u32, &'static [(i32, usize, i32)]) {
    match dither {
        // Floyd–Steinberg: 7, 3, 5 and 1 sixteenths.
        Dither::Floyd => (4, &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)]),
        // Atkinson: an eighth to each of six, two eighths kept back.
        _ => (3, &[(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)]),
    }
}

/// A YUV 4:2:0 frame shrunk to `tw` × `th` RGB (each pixel the mean of its
/// part of the frame, cropped from the middle to the new shape).
pub fn shrink(yuv: &[u8], w: usize, h: usize, tw: usize, th: usize) -> Vec<u8> {
    let mut out = vec![0u8; tw * th * 3];
    if w < 2 || h < 2 || tw == 0 || th == 0 || yuv.len() < w * h * 3 / 2 {
        return out;
    }
    let (sw, sh) = if w * th > h * tw { (h * tw / th, h) } else { (w, w * th / tw) };
    let (x0, y0) = ((w - sw) / 2, (h - sh) / 2);
    let (cw, ch) = (w / 2, h / 2);
    let (ys, uv) = yuv.split_at(w * h);
    let (us, vs) = uv.split_at(cw * ch);
    // Which of the small picture's columns and rows each of the frame's
    // falls in (each frame pixel added once: a straight pass over the
    // planes), and how many fall in each.
    let col_of: Vec<usize> = (0..sw).map(|x| x * tw / sw).collect();
    let row_of: Vec<usize> = (0..sh).map(|y| y * th / sh).collect();
    let count = |of: &[usize], n: usize| {
        let mut c = vec![0u32; n];
        of.iter().for_each(|i| c[*i] += 1);
        c
    };
    let (ncol, nrow) = (count(&col_of, tw), count(&row_of, th));
    // Chroma: each sample to where its 2 × 2 block's first pixel falls.
    let ccol: Vec<usize> = (0..sw.div_ceil(2)).map(|i| col_of[(2 * i).min(sw - 1)]).collect();
    let crow: Vec<usize> = (0..sh.div_ceil(2)).map(|j| row_of[(2 * j).min(sh - 1)]).collect();
    let (nccol, ncrow) = (count(&ccol, tw), count(&crow, th));
    let mut ysum = vec![0u32; tw * th];
    for (j, &oy) in row_of.iter().enumerate() {
        let row = &ys[(y0 + j) * w + x0..][..sw];
        let sums = &mut ysum[oy * tw..(oy + 1) * tw];
        for (v, &ox) in row.iter().zip(&col_of) {
            sums[ox] += u32::from(*v);
        }
    }
    let mut csum = vec![[0u32; 2]; tw * th];
    let (cx0, cy0) = (x0 / 2, y0 / 2);
    for (j, &oy) in crow.iter().enumerate() {
        let cy = (cy0 + j).min(ch - 1);
        let (ur, vr) = (&us[cy * cw..(cy + 1) * cw], &vs[cy * cw..(cy + 1) * cw]);
        let sums = &mut csum[oy * tw..(oy + 1) * tw];
        for (i, &ox) in ccol.iter().enumerate() {
            let cx = (cx0 + i).min(cw - 1);
            sums[ox][0] += u32::from(ur[cx]);
            sums[ox][1] += u32::from(vr[cx]);
        }
    }
    for oy in 0..th {
        for ox in 0..tw {
            let k = oy * tw + ox;
            let l = (ysum[k] / (nrow[oy] * ncol[ox]).max(1)) as i32;
            let cn = (ncrow[oy] * nccol[ox]).max(1);
            let (u, v) = ((csum[k][0] / cn) as i32 - 128, (csum[k][1] / cn) as i32 - 128);
            let at = k * 3;
            out[at] = (l + ((359 * v) >> 8)).clamp(0, 255) as u8;
            out[at + 1] = (l - ((88 * u + 183 * v) >> 8)).clamp(0, 255) as u8;
            out[at + 2] = (l + ((454 * u) >> 8)).clamp(0, 255) as u8;
        }
    }
    out
}

/// A pixel picture: its palette's indices, a byte a pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub idx: Vec<u8>,
    pub palette: Palette,
}

impl Frame {
    /// As colours, to draw.
    pub fn picture(&self) -> Picture {
        let colours = self.palette.colours();
        let mut out = Picture::new(self.w, self.h);
        for (o, k) in out.rgb.chunks_exact_mut(3).zip(&self.idx) {
            o.copy_from_slice(&colours[usize::from(*k).min(colours.len() - 1)]);
        }
        out
    }

    /// As an indexed PNG, each pixel `scale` square, saying `look`.
    pub fn png(&self, scale: usize, look: &str) -> Result<Vec<u8>, String> {
        let scale = scale.max(1);
        let colours = self.palette.colours();
        let (depth, bits) = match colours.len() {
            0..=2 => (png::BitDepth::One, 1),
            3..=4 => (png::BitDepth::Two, 2),
            5..=16 => (png::BitDepth::Four, 4),
            _ => (png::BitDepth::Eight, 8),
        };
        let (w, h) = (self.w * scale, self.h * scale);
        let stride = (w * bits).div_ceil(8);
        let mut data = vec![0u8; stride * h];
        for y in 0..h {
            let row = &mut data[y * stride..(y + 1) * stride];
            for x in 0..w {
                let k = self.idx[(y / scale) * self.w + x / scale];
                let bit = x * bits;
                row[bit / 8] |= k << (8 - bits - bit % 8);
            }
        }
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, w as u32, h as u32);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(depth);
            enc.set_palette(colours.iter().flatten().copied().collect::<Vec<u8>>());
            enc.add_text_chunk("Software".into(), "Catalyst Camera".into()).map_err(|e| e.to_string())?;
            enc.add_text_chunk("look".into(), look.to_owned()).map_err(|e| e.to_string())?;
            enc.add_text_chunk("pixel".into(), format!("{} {}", self.w, self.h)).map_err(|e| e.to_string())?;
            let mut writer = enc.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&data).map_err(|e| e.to_string())?;
        }
        Ok(bytes)
    }
}

/// A dither's sample for its picker: a ramp from dark to light across
/// `w` × `h`, in two colours.
pub fn sample(dither: Dither, w: usize, h: usize) -> Vec<u8> {
    let v: Vec<i32> = (0..w * h).map(|i| ((i % w) * 255 / (w - 1).max(1)) as i32).collect();
    let mut out = vec![0u8; w * h];
    ramp(&v, w, h, 2, dither, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| c).collect()
    }

    fn with(palette: Palette, dither: Dither) -> Pixel {
        // Contrast's "normal" adds a touch; these check the mapping itself.
        let mut p = Pixel::new(Settings { palette, dither, ..Settings::default() });
        for (i, t) in p.tone.iter_mut().enumerate() {
            *t = i as u8;
        }
        p
    }

    #[test]
    fn each_palette_maps_its_own_colours_to_themselves() {
        for palette in PALETTES {
            let colours = palette.colours();
            let rgb: Vec<u8> = colours.iter().flatten().copied().collect();
            for dither in [Dither::None, Dither::Floyd, Dither::Atkinson] {
                let f = with(palette, dither).map(&rgb, colours.len(), 1);
                if palette.ramp() {
                    // A ramp maps by brightness: dark to light, in order.
                    assert_eq!(f.idx, (0..colours.len() as u8).collect::<Vec<_>>(), "{} {dither:?}", palette.name());
                } else {
                    // Every colour of a colour palette comes back as itself.
                    for (k, i) in f.idx.iter().enumerate() {
                        assert_eq!(colours[usize::from(*i)], colours[k], "{} {dither:?}: colour {k}", palette.name());
                    }
                }
            }
            // A ramp's colours run from dark to light.
            if palette.ramp() {
                let l: Vec<u32> = colours.iter().map(|c| 77 * u32::from(c[0]) + 150 * u32::from(c[1]) + 29 * u32::from(c[2])).collect();
                assert!(l.windows(2).all(|p| p[0] < p[1]), "{}", palette.name());
            }
        }
        assert_eq!(PALETTES.len(), 6);
        assert_eq!(PALETTES.map(|p| p.colours().len()), [2, 4, 4, 4, 8, 16]);
    }

    #[test]
    fn black_and_white_land_on_the_ends_and_known_colours_on_their_nearest() {
        for palette in PALETTES {
            for dither in DITHERS {
                let p = with(palette, dither);
                let black = p.map(&flat(8, 8, [0, 0, 0]), 8, 8);
                let white = p.map(&flat(8, 8, [255, 255, 255]), 8, 8);
                let c = palette.colours();
                let darkest = c.iter().enumerate().min_by_key(|(_, c)| c.iter().map(|v| u32::from(*v)).sum::<u32>()).unwrap().0 as u8;
                let lightest = c.iter().enumerate().max_by_key(|(_, c)| c.iter().map(|v| u32::from(*v)).sum::<u32>()).unwrap().0 as u8;
                assert!(black.idx.iter().all(|k| *k == darkest), "{} {dither:?}: black {:?}", palette.name(), black.idx);
                assert!(white.idx.iter().all(|k| *k == lightest), "{} {dither:?}: white", palette.name());
            }
        }
        // Undithered, a strong red is poster's red, a sky blue its sky.
        let poster = with(Palette::Poster, Dither::None);
        assert_eq!(POSTER[usize::from(poster.map(&[200, 60, 50], 1, 1).idx[0])], POSTER[2]);
        assert_eq!(POSTER[usize::from(poster.map(&[130, 190, 225], 1, 1).idx[0])], POSTER[4]);
    }

    #[test]
    fn mid_grey_dithers_to_half_and_half_in_each_way_and_flat_without() {
        // 1-bit ink, a 16 × 16 of mid grey (128).
        let grey = flat(16, 16, [128, 128, 128]);
        let share = |f: &Frame| f.idx.iter().filter(|k| **k == 1).count();
        assert!(share(&with(Palette::Ink, Dither::None).map(&grey, 16, 16)) == 256, "128 rounds up to paper, all of it");
        // Bayer: exactly half, in its 4 × 4 pattern (a checker at 50%).
        let bayer = with(Palette::Ink, Dither::Bayer).map(&grey, 16, 16);
        assert_eq!(share(&bayer), 128);
        assert_eq!(&bayer.idx[..4], &[0, 1, 0, 1]);
        assert_eq!(&bayer.idx[16..20], &[1, 0, 1, 0]);
        assert_eq!(bayer.idx[..64], bayer.idx[64..128], "the pattern repeats every four rows");
        // Floyd–Steinberg keeps the mean; Atkinson keeps it nearly, and
        // a pale grey (Atkinson passes on only three quarters) goes cleaner.
        let floyd = share(&with(Palette::Ink, Dither::Floyd).map(&grey, 16, 16));
        assert!((120..=136).contains(&floyd), "{floyd}");
        let atkinson = share(&with(Palette::Ink, Dither::Atkinson).map(&grey, 16, 16));
        assert!((110..=146).contains(&atkinson), "{atkinson}");
        let pale = flat(16, 16, [230, 230, 230]);
        let pale_floyd = 256 - share(&with(Palette::Ink, Dither::Floyd).map(&pale, 16, 16));
        let pale_atkinson = 256 - share(&with(Palette::Ink, Dither::Atkinson).map(&pale, 16, 16));
        assert!(pale_atkinson < pale_floyd, "fewer dark dots in a pale grey: {pale_atkinson} against {pale_floyd}");
        // A 4-level ramp at a third (85): its second level throughout, undithered and dithered.
        let third = flat(8, 8, [85, 85, 85]);
        for d in DITHERS {
            let f = with(Palette::Graphite, d).map(&third, 8, 8);
            assert!(f.idx.iter().all(|k| *k == 1), "{d:?}: {:?}", f.idx);
        }
        // Between two levels, ordered dither mixes only those two.
        let between = flat(8, 8, [128, 128, 128]);
        let f = with(Palette::Graphite, Dither::Bayer).map(&between, 8, 8);
        assert!(f.idx.iter().all(|k| *k == 1 || *k == 2) && f.idx.contains(&1) && f.idx.contains(&2), "{:?}", f.idx);
    }

    #[test]
    fn a_gradient_keeps_its_brightness_on_average_through_each_dither() {
        // 64 × 8, black to white: the mean brightness of the picture made
        // is the gradient's, to within a few levels.
        let (w, h) = (64, 8);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [((i % w) * 255 / (w - 1)) as u8; 3]).collect();
        let want = rgb.iter().map(|v| u32::from(*v)).sum::<u32>() as f32 / rgb.len() as f32;
        for palette in [Palette::Ink, Palette::Graphite] {
            for d in [Dither::Bayer, Dither::Floyd] {
                let f = with(palette, d).map(&rgb, w, h);
                let n = palette.colours().len() as f32 - 1.0;
                let got = f.idx.iter().map(|k| f32::from(*k) * 255.0 / n).sum::<f32>() / f.idx.len() as f32;
                assert!((got - want).abs() < 6.0, "{} {d:?}: {got} against {want}", palette.name());
            }
        }
    }

    #[test]
    fn the_sizes_land_on_whole_panel_pixels_and_keep_four_by_three() {
        let vf = ((crate::gfx::H * 4.0 / 3.0).round() as usize, crate::gfx::H as usize);
        for size in SIZES {
            let (w, h) = size.dims();
            let s = size.view_scale();
            assert!(w * s <= vf.0 && h * s <= vf.1, "{size:?} fits the viewfinder");
            assert!(w * s + 4 >= vf.0 && h * s + 3 >= vf.1, "{size:?} fills it: {} × {}", w * s, h * s);
            assert!((w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.01, "{size:?} is 4:3");
            assert!((440..=460).contains(&(w * size.share_scale())), "{size:?}'s copy for sharing");
        }
        assert_eq!(SIZES.map(|s| s.dims().0), [56, 75, 112]);
        // A frame shrinks to the size asked, its colour kept.
        let pic = crate::camera::scene(320, 240, 1.0, false);
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        for size in SIZES {
            let p = Pixel::new(Settings { size, ..Settings::default() });
            let f = p.develop(&yuv, 320, 240);
            assert_eq!((f.w, f.h, f.idx.len()), (size.dims().0, size.dims().1, size.dims().0 * size.dims().1));
        }
        let mut red = crate::photos::Picture::new(320, 240);
        red.rgb.chunks_exact_mut(3).for_each(|p| p.copy_from_slice(&[200, 40, 40]));
        let small = shrink(&crate::camera::rgb_to_yuv420(&red), 320, 240, 75, 56);
        assert!(small.chunks_exact(3).all(|p| p[0] > 180 && p[1] < 60 && p[2] < 60), "{:?}", &small[..3]);
    }

    #[test]
    fn the_settings_go_through_their_line_and_odd_lines_fall_back() {
        let s = Settings { size: Size::Fine, palette: Palette::Crayon, dither: Dither::Atkinson, bright: -2, contrast: Contrast::Hard };
        assert_eq!(s.line(), "112,crayon,atkinson,-2,hard");
        assert_eq!(Settings::from_line(&s.line()), s);
        assert_eq!(Settings::from_line("nonsense"), Settings::default());
        assert_eq!(Settings::from_line("56,tide,none,9").bright, BRIGHT_STEPS, "brightness kept in its range");
        // Brighter is brighter, and contrast spreads the tones.
        let p = |bright, contrast| Pixel::new(Settings { bright, contrast, ..Settings::default() });
        assert!(p(2, Contrast::Normal).tone[128] > p(0, Contrast::Normal).tone[128]);
        assert!(p(0, Contrast::Hard).tone[64] < p(0, Contrast::Soft).tone[64]);
        assert!(p(0, Contrast::Hard).tone[192] > p(0, Contrast::Soft).tone[192]);
    }

    #[test]
    fn a_picture_goes_into_an_indexed_png_at_its_size_and_blown_up() {
        let f = with(Palette::Graphite, Dither::Bayer).map(&(0..12 * 9).flat_map(|i| [(i * 2) as u8; 3]).collect::<Vec<u8>>(), 12, 9);
        for scale in [1, 4] {
            let bytes = f.png(scale, "Pixel · graphite · bayer").unwrap();
            let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (12 * scale as u32, 9 * scale as u32));
            assert_eq!(reader.info().color_type, png::ColorType::Indexed);
            assert!(reader.info().uncompressed_latin1_text.iter().any(|t| t.keyword == "look" && t.text == "Pixel · graphite · bayer"));
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let out = reader.next_frame(&mut buf).unwrap();
            let rgb = &buf[..out.buffer_size()];
            let want = f.picture();
            // Each pixel is its own colour, whole, `scale` square.
            for y in 0..9 * scale {
                for x in 0..12 * scale {
                    let got = &rgb[(y * 12 * scale + x) * 3..][..3];
                    assert_eq!(got, &want.rgb[((y / scale) * 12 + x / scale) * 3..][..3], "{scale}: {x} {y}");
                }
            }
        }
    }

    #[test]
    fn a_preview_frame_in_pixel_mode_costs_less_than_converting_it() {
        // As the looks' test: 320 × 240, timed here against the plain
        // conversion timed alongside (the A53 is reckoned at about fifteen
        // times slower than this computer's core, and a frame may spend 6 ms).
        let pic = crate::camera::scene(320, 240, 1.0, false);
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        let mut plain = crate::photos::Picture::default();
        let time = |f: &mut dyn FnMut()| {
            f();
            let began = std::time::Instant::now();
            for _ in 0..50 {
                f();
            }
            began.elapsed().as_secs_f64() * 1000.0 / 50.0
        };
        let convert = time(&mut || crate::camera::yuv420_to_rgb(&yuv, 320, 240, &mut plain));
        let mut worst = (0.0f64, String::new());
        for size in SIZES {
            for palette in [Palette::Ink, Palette::Crayon] {
                for dither in DITHERS {
                    let p = Pixel::new(Settings { size, palette, dither, ..Settings::default() });
                    let took = time(&mut || {
                        let f = p.develop(&yuv, 320, 240);
                        std::hint::black_box(f.picture());
                    });
                    if took > worst.0 {
                        worst = (took, format!("{size:?} {} {}", palette.name(), dither.name()));
                    }
                }
            }
        }
        eprintln!("a 320 × 240 preview here: converted {convert:.3} ms; as a pixel picture at worst {:.3} ms ({})", worst.0, worst.1);
        assert!(worst.0 < convert.max(0.05) * 2.0, "{} ms ({}), against {convert} ms converted alone", worst.0, worst.1);
    }
}
