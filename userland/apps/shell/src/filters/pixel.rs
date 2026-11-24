//! Pixel mode's pictures (the Camera's pixel mode, 1.2; its quality, auto
//! palette, loop and date stamp 1.4.0): a picture made small, its colours
//! few and dithered, as a toy camera's are. The look is our own: three
//! sizes that land on whole panel pixels in the viewfinder (×4, ×3, ×2),
//! seven palettes of our own and one taken from the scene, four ways to
//! dither, and the tone (brightness on ↑ ↓, contrast in four steps).
//!
//! A picture is made in four steps, the same for the live viewfinder (from
//! each 320 × 240 preview frame) and the shutter (from the camera's own
//! full-size still, decoded at a quarter):
//!
//! 1. **Shrunk in linear light.** Each pixel is the mean of its part of the
//!    picture in light, not in its encoded values, so a fine black-and-white
//!    pattern becomes the grey it looks like rather than a darker one. (The
//!    preview averages its brightness so, its colour as encoded.)
//! 2. **Made ready for few colours:** the brightness stretched to the
//!    palette's range (the scene's darkest and lightest hundredth, eased
//!    between live frames), a contrast curve and a vibrance each palette is
//!    tuned for, then a sharpening of the brightness held inside each
//!    pixel's neighbours' range, so edges land crisply on pixel boundaries
//!    with no halo.
//! 3. **Matched perceptually:** to the nearest colour in OKLab (Björn
//!    Ottosson's perceptual colour space), its hue weighted 1.9 times over
//!    its lightness (the dither makes lightness up; a wrong hue shows),
//!    through a 32³ table made once for each fixed palette; the ramps by
//!    OKLab lightness, the scene's darkest to the ramp's darkest and its
//!    lightest to the ramp's lightest.
//! 4. **Dithered:** ordered (Bayer) in encoded values, or by error
//!    diffusion carried in linear light for the colour palettes and in
//!    perceptual lightness for the ramps.
//!
//! The auto palette is eight colours found in the scene (k-means in the
//! matching's weighted OKLab on every third pixel each way, a couple of
//! rounds every third frame on from the last colours, so it follows the
//! scene without flicker; a small patch of its own colour that no centre
//! serves takes one; their lightness stretched to the scene's own range),
//! locked when the shutter is pressed and while a loop records. A date stamp in a pixel font of our own (3 × 5) can go in
//! the corner. The picture is kept as its colours' indices, so it is
//! written as it is: an indexed PNG (or GIF, for a loop: gif.rs) at its
//! true size, and blown up with whole pixels for sharing.

use std::sync::{Mutex, OnceLock};

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

    /// The sharpening's strength: more where each pixel stands for more.
    fn sharpen(self) -> f32 {
        match self {
            Size::Chunky => 0.9,
            Size::Small => 0.75,
            Size::Fine => 0.6,
        }
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
    /// Two inks and their overprint on paper (1.4.0).
    Print,
    Poster,
    Crayon,
    /// Eight colours found in the scene (1.4.0).
    Auto,
}

pub const PALETTES: [Palette; 8] = [Palette::Ink, Palette::Graphite, Palette::Ember, Palette::Tide, Palette::Print, Palette::Poster, Palette::Crayon, Palette::Auto];

/// The auto palette's size.
pub const AUTO_COLOURS: usize = 8;

/// The palettes' colours (our own, made by eye on the simulated scene and
/// photographs). The ramps run from dark to light.
const INK: [[u8; 3]; 2] = [[0x1d, 0x1b, 0x26], [0xef, 0xe9, 0xda]];
const GRAPHITE: [[u8; 3]; 4] = [[0x15, 0x16, 0x1a], [0x55, 0x58, 0x5f], [0xa1, 0xa4, 0xaa], [0xec, 0xea, 0xe6]];
const EMBER: [[u8; 3]; 4] = [[0x2b, 0x14, 0x18], [0x8a, 0x3a, 0x2b], [0xe0, 0x8a, 0x46], [0xf8, 0xe2, 0xb8]];
const TIDE: [[u8; 3]; 4] = [[0x0f, 0x1a, 0x2c], [0x2d, 0x5d, 0x7c], [0x74, 0xb2, 0xbf], [0xe3, 0xf1, 0xea]];
/// Paper, a coral ink, a teal ink, and where both are printed.
const PRINT: [[u8; 3]; 4] = [[0xf2, 0xeb, 0xdd], [0xe2, 0x56, 0x40], [0x23, 0x70, 0x7a], [0x2a, 0x27, 0x36]];
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

/// What a palette is made ready for: how much more contrast it wants, and
/// the vibrance that brings duller colours out to meet its few (our own
/// tuning, by eye on photographs).
struct Tuning {
    contrast: f32,
    vibrance: f32,
}

impl Palette {
    pub fn name(self) -> &'static str {
        match self {
            Palette::Ink => "ink",
            Palette::Graphite => "graphite",
            Palette::Ember => "ember",
            Palette::Tide => "tide",
            Palette::Print => "print",
            Palette::Poster => "poster",
            Palette::Crayon => "crayon",
            Palette::Auto => "auto",
        }
    }

    pub fn from_id(id: &str) -> Option<Palette> {
        PALETTES.iter().copied().find(|p| p.name() == id)
    }

    /// Its fixed colours (none for auto, whose are the scene's).
    pub fn colours(self) -> &'static [[u8; 3]] {
        match self {
            Palette::Ink => &INK,
            Palette::Graphite => &GRAPHITE,
            Palette::Ember => &EMBER,
            Palette::Tide => &TIDE,
            Palette::Print => &PRINT,
            Palette::Poster => &POSTER,
            Palette::Crayon => &CRAYON,
            Palette::Auto => &[],
        }
    }

    /// Mapped by lightness alone: a ramp from dark to light.
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
            Palette::Print => "two inks",
            Palette::Poster => "8 colours",
            Palette::Crayon => "16 colours",
            Palette::Auto => "from the scene",
        }
    }

    fn tuning(self) -> Tuning {
        let (contrast, vibrance) = match self {
            Palette::Ink => (1.25, 0.0),
            Palette::Graphite => (1.05, 0.0),
            Palette::Ember | Palette::Tide => (1.1, 0.0),
            Palette::Print => (1.1, 0.3),
            Palette::Poster => (1.05, 0.55),
            Palette::Crayon => (1.0, 0.4),
            Palette::Auto => (1.0, 0.2),
        };
        Tuning { contrast, vibrance }
    }

    /// The ordered dither's spread for a colour palette (in encoded
    /// values): about the distance between its neighbouring colours.
    fn spread(self) -> f32 {
        match self {
            Palette::Print | Palette::Poster => 0.25,
            Palette::Auto => 0.2,
            _ => 0.17,
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

    /// The S-curve's steepness about mid grey (0: a straight line) and how
    /// much of the range it keeps (soft draws it in toward grey).
    fn curve(self) -> (f32, f32) {
        match self {
            Contrast::Soft => (0.0, 0.82),
            // Nearly straight: the stretch and the sharpening give the
            // punch, and a steeper curve sank darker skin and green fields.
            Contrast::Normal => (1.0, 1.0),
            Contrast::Punchy => (2.8, 1.0),
            Contrast::Hard => (5.5, 1.0),
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
    /// The date in the corner, in a pixel font (off at first).
    pub stamp: bool,
}

impl Settings {
    /// As the Camera's file keeps it: "75,ember,bayer,0,normal,0".
    pub fn line(&self) -> String {
        format!("{},{},{},{},{},{}", self.size.id(), self.palette.name(), self.dither.name(), self.bright, self.contrast.name(), u8::from(self.stamp))
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
        s.stamp = parts.get(5) == Some(&"1");
        s
    }

    /// What a pixel picture says it is (its PNG's "look").
    pub fn look_words(&self) -> String {
        format!("Pixel · {} · {}", self.palette.name(), self.dither.name())
    }
}

/// 4 × 4 Bayer: thresholds 0 to 15.
pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

// ------------------------------------------------------------- colour ---

/// Encoded (sRGB) to linear light, for each byte.
fn to_linear_table() -> &'static [f32; 256] {
    static T: OnceLock<[f32; 256]> = OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| decode_srgb(i as f32 / 255.0)))
}

fn decode_srgb(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn encode_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear light back to its encoding, through a table of 4096 steps.
fn to_encoded_table() -> &'static [f32; 4096] {
    static T: OnceLock<[f32; 4096]> = OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| encode_srgb(i as f32 / 4095.0)))
}

#[inline]
fn encoded(lin: f32) -> f32 {
    to_encoded_table()[(lin.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize]
}

/// An encoded value (0 to 1) in linear light, between the bytes' values.
#[inline]
fn linear(v: f32) -> f32 {
    let t = to_linear_table();
    let x = v.clamp(0.0, 1.0) * 255.0;
    let i = (x as usize).min(254);
    t[i] + (t[i + 1] - t[i]) * (x - i as f32)
}

/// A cube root, near enough (a guess from the float's bits, then two of
/// Newton's steps): OKLab for every pixel of a preview frame.
#[inline]
fn cbrt_fast(x: f32) -> f32 {
    if x <= 1e-9 {
        return 0.0;
    }
    let mut y = f32::from_bits(x.to_bits() / 3 + 0x2a51_4067);
    y -= (y * y * y - x) / (3.0 * y * y);
    y -= (y * y * y - x) / (3.0 * y * y);
    y
}

/// Linear RGB to OKLab (L from 0 to 1).
#[inline]
pub fn oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_47 * c[0] + 0.536_332_55 * c[1] + 0.051_445_99 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (cbrt_fast(l), cbrt_fast(m), cbrt_fast(s));
    [0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s]
}

/// OKLab back to linear RGB.
fn from_oklab(c: [f32; 3]) -> [f32; 3] {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s, -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s, -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s]
}

/// An encoded colour's OKLab.
pub fn lab_of(c: [u8; 3]) -> [f32; 3] {
    let t = to_linear_table();
    oklab([t[usize::from(c[0])], t[usize::from(c[1])], t[usize::from(c[2])]])
}

#[inline]
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// OKLab with its colour (a, b) weighted over its lightness, for matching
/// to a colour palette's few colours: a wrong hue shows more than a wrong
/// lightness, which the dither makes up (plain OKLab took a dull green
/// field to navy, a pink to sky blue).
#[inline]
fn weighted(c: [f32; 3]) -> [f32; 3] {
    const HUE: f32 = 1.9;
    [c[0], c[1] * HUE, c[2] * HUE]
}

/// The nearest of `labs` to `c` (both OKLab).
#[inline]
fn nearest(labs: &[[f32; 3]], c: [f32; 3]) -> u8 {
    let mut best = (f32::MAX, 0u8);
    for (k, p) in labs.iter().enumerate() {
        let d = dist(*p, c);
        if d < best.0 {
            best = (d, k as u8);
        }
    }
    best.1
}

/// The colour table's resolution: five bits a channel.
const LUT_BITS: usize = 5;

/// A fixed colour palette's nearest colour (in OKLab) for each 32³ cell of
/// encoded RGB, made once for each palette.
fn table(palette: Palette) -> &'static [u8] {
    static TABLES: [OnceLock<Vec<u8>>; 8] = [const { OnceLock::new() }; 8];
    let at = PALETTES.iter().position(|p| *p == palette).unwrap_or(0);
    TABLES[at].get_or_init(|| {
        let labs: Vec<[f32; 3]> = palette.colours().iter().map(|c| weighted(lab_of(*c))).collect();
        let n = 1usize << LUT_BITS;
        let step = 256 / n;
        let t = to_linear_table();
        (0..n * n * n)
            .map(|i| {
                let c = [i >> (2 * LUT_BITS), (i >> LUT_BITS) & (n - 1), i & (n - 1)].map(|v| t[v * step + step / 2]);
                nearest(&labs, weighted(oklab(c)))
            })
            .collect()
    })
}

#[inline]
fn cell(c: [f32; 3]) -> usize {
    let q = |v: f32| ((v.clamp(0.0, 1.0) * 255.0) as usize) >> (8 - LUT_BITS);
    (q(c[0]) << (2 * LUT_BITS)) | (q(c[1]) << LUT_BITS) | q(c[2])
}

// -------------------------------------------------------- the picture ---

/// A small picture being made: encoded RGB, 0 to 1.
#[derive(Clone, Debug)]
pub struct Small {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
}

/// The part of a `w` × `h` picture with the shape `tw` × `th`, centred.
fn crop(w: usize, h: usize, tw: usize, th: usize) -> (usize, usize, usize, usize) {
    let (sw, sh) = if w * th > h * tw { (h * tw / th, h) } else { (w, w * th / tw) };
    ((w - sw) / 2, (h - sh) / 2, sw, sh)
}

/// A YUV 4:2:0 frame (full range, packed planes) shrunk to `tw` × `th`:
/// its brightness averaged in linear light (every pixel), its colour as
/// encoded (the preview's way: cheap, and the eye takes edges from
/// brightness). Worked a 2 × 2 block at a time, as its colour comes.
pub fn small_from_yuv(yuv: &[u8], w: usize, h: usize, tw: usize, th: usize) -> Small {
    let mut out = Small { w: tw, h: th, px: vec![[0.0; 3]; tw * th] };
    if w < 2 || h < 2 || tw == 0 || th == 0 || yuv.len() < w * h * 3 / 2 {
        return out;
    }
    let (x0, y0, sw, sh) = crop(w, h, tw, th);
    let (cw, ch) = (w / 2, h / 2);
    let (ys, uv) = yuv.split_at(w * h);
    let (us, vs) = uv.split_at(cw * ch);
    let lin = to_linear_table();
    let lin16: [u32; 256] = std::array::from_fn(|i| (lin[i] * 16383.0).round() as u32);
    // The crop's 2 × 2 blocks, and which of the small picture's pixels
    // each falls in.
    let (bx0, by0, bw, bh) = (x0 / 2, y0 / 2, (sw / 2).max(1), (sh / 2).max(1));
    let col_of: Vec<usize> = (0..bw).map(|i| i * tw / bw).collect();
    let mut sums = vec![[0u32; 4]; tw * th];
    for j in 0..bh {
        let by = (by0 + j).min(ch - 1);
        let oy = j * th / bh;
        let (r0, r1) = (&ys[2 * by * w..(2 * by + 1) * w], &ys[(2 * by + 1) * w..(2 * by + 2) * w]);
        let (ur, vr) = (&us[by * cw..(by + 1) * cw], &vs[by * cw..(by + 1) * cw]);
        let row = &mut sums[oy * tw..(oy + 1) * tw];
        for (i, &ox) in col_of.iter().enumerate() {
            let bx = (bx0 + i).min(cw - 1);
            let l = lin16[usize::from(r0[2 * bx])] + lin16[usize::from(r0[2 * bx + 1])] + lin16[usize::from(r1[2 * bx])] + lin16[usize::from(r1[2 * bx + 1])];
            let s = &mut row[ox];
            s[0] += l;
            s[1] += u32::from(ur[bx]);
            s[2] += u32::from(vr[bx]);
            s[3] += 1;
        }
    }
    let enc = to_encoded_table();
    for (o, s) in out.px.iter_mut().zip(&sums) {
        let n = s[3].max(1) as f32;
        let l = enc[((s[0] as f32 / (n * 4.0 * 16383.0)).clamp(0.0, 1.0) * 4095.0 + 0.5) as usize];
        let (u, v) = (s[1] as f32 / (n * 255.0) - 128.0 / 255.0, s[2] as f32 / (n * 255.0) - 128.0 / 255.0);
        *o = [(l + 1.402 * v).clamp(0.0, 1.0), (l - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 1.0), (l + 1.772 * u).clamp(0.0, 1.0)];
    }
    out
}
/// An RGB picture (the camera's still, decoded at a quarter) shrunk to
/// `tw` × `th`, every channel averaged in linear light.
pub fn small_from_rgb(pic: &Picture, tw: usize, th: usize) -> Small {
    let mut out = Small { w: tw, h: th, px: vec![[0.0; 3]; tw * th] };
    if pic.is_empty() || tw == 0 || th == 0 {
        return out;
    }
    let (x0, y0, sw, sh) = crop(pic.w, pic.h, tw, th);
    let lin = to_linear_table();
    let col_of: Vec<usize> = (0..sw).map(|x| x * tw / sw).collect();
    let mut sums = vec![[0f32; 3]; tw * th];
    let mut counts = vec![0u32; tw * th];
    for y in 0..sh {
        let oy = y * th / sh;
        let row = &pic.rgb[((y0 + y) * pic.w + x0) * 3..][..sw * 3];
        let (s, n) = (&mut sums[oy * tw..(oy + 1) * tw], &mut counts[oy * tw..(oy + 1) * tw]);
        for (p, &ox) in row.chunks_exact(3).zip(&col_of) {
            let c = &mut s[ox];
            c[0] += lin[usize::from(p[0])];
            c[1] += lin[usize::from(p[1])];
            c[2] += lin[usize::from(p[2])];
            n[ox] += 1;
        }
    }
    for ((o, s), n) in out.px.iter_mut().zip(&sums).zip(&counts) {
        let n = (*n).max(1) as f32;
        *o = s.map(|v| encoded(v / n));
    }
    out
}

#[inline]
fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Pixel mode made ready for pictures: the settings, the tone as a curve,
/// and what it keeps between frames (the stretch, the scene's colours).
pub struct Pixel {
    pub settings: Settings,
    /// The contrast and brightness, on encoded values (1024 steps).
    tone: Vec<f32>,
    /// The auto palette held at these colours.
    locked: Option<Vec<[u8; 3]>>,
    /// The scene's stretch and colours as last seen (the live preview's).
    levels: Mutex<Option<(f32, f32)>>,
    auto: Mutex<Vec<[f32; 3]>>,
    /// The scene's colours as last found, and live frames seen (the
    /// colours are looked for every third: they change slowly).
    found: Mutex<Vec<[u8; 3]>>,
    frames: std::sync::atomic::AtomicU32,
    /// The scene's (or held) colours' table, as far as it has been filled.
    cells: Mutex<(Vec<[u8; 3]>, Vec<u8>)>,
    /// Plain: no stretch, curve, vibrance or sharpening (the tests'
    /// mapping on its own).
    plain: bool,
}

impl Pixel {
    pub fn new(settings: Settings) -> Pixel {
        let (k, hold) = settings.contrast.curve();
        let k = k * settings.palette.tuning().contrast;
        let lift = f32::from(settings.bright) * 0.07;
        let s = |v: f32| if k < 1e-3 { v } else { 0.5 + (k * (v - 0.5)).atan() / (2.0 * (k * 0.5).atan()) };
        let tone = (0..1024).map(|i| (0.5 + (s(i as f32 / 1023.0) - 0.5) * hold + lift).clamp(0.0, 1.0)).collect();
        // A fixed palette's table, made now rather than on a frame.
        if !settings.palette.ramp() && settings.palette != Palette::Auto {
            table(settings.palette);
        }
        Pixel { settings, tone, locked: None, levels: Mutex::new(None), auto: Mutex::new(Vec::new()), found: Mutex::new(Vec::new()), frames: std::sync::atomic::AtomicU32::new(0), cells: Mutex::new((Vec::new(), Vec::new())), plain: false }
    }

    /// With the auto palette held at these colours (a loop being recorded,
    /// a still made as of the moment the shutter was pressed).
    pub fn locked(settings: Settings, colours: Vec<[u8; 3]>) -> Pixel {
        Pixel { locked: Some(colours), ..Pixel::new(settings) }
    }

    /// The colours it is held at, if any.
    pub fn held(&self) -> Option<&[[u8; 3]]> {
        self.locked.as_deref()
    }

    /// The mapping alone, nothing made ready (the tests).
    #[cfg(test)]
    pub fn plain(settings: Settings) -> Pixel {
        Pixel { plain: true, ..Pixel::new(settings) }
    }

    /// A camera preview frame (YUV 4:2:0) as a pixel picture, the stretch
    /// and the scene's colours carried on from the last frame.
    pub fn develop(&self, yuv: &[u8], w: usize, h: usize) -> Frame {
        let (tw, th) = self.settings.size.dims();
        let mut small = small_from_yuv(yuv, w, h, tw, th);
        self.prepare(&mut small, true);
        self.quantise(&small, true)
    }

    /// The camera's still (any RGB picture) as a pixel picture, worked
    /// afresh (the auto palette found in it, unless held).
    pub fn develop_still(&self, pic: &Picture) -> Frame {
        let (tw, th) = self.settings.size.dims();
        let mut small = small_from_rgb(pic, tw, th);
        self.prepare(&mut small, false);
        self.quantise(&small, false)
    }

    /// A small picture made ready for few colours: stretched, toned, its
    /// colour brought out, then its edges sharpened within their neighbours.
    pub fn prepare(&self, small: &mut Small, live: bool) {
        if self.plain || small.px.is_empty() {
            return;
        }
        let tuning = self.settings.palette.tuning();
        // The stretch: the darkest and lightest hundredth to black and
        // white, never more than about threefold, eased between live frames.
        let mut hist = [0u32; 256];
        for p in &small.px {
            hist[(luma(*p) * 255.0).clamp(0.0, 255.0) as usize] += 1;
        }
        let n = small.px.len() as u32;
        let at = |part: u32| {
            let mut seen = 0;
            for (i, c) in hist.iter().enumerate() {
                seen += c;
                if seen > part {
                    return i as f32 / 255.0;
                }
            }
            1.0
        };
        let (mut lo, mut hi) = (at(n / 100).min(0.22), at(n - n / 100 - 1).max(0.6));
        if hi - lo < 0.34 {
            let mid = (hi + lo) * 0.5;
            (lo, hi) = (mid - 0.17, mid + 0.17);
        }
        if live {
            if let Ok(mut last) = self.levels.lock() {
                if let Some((l0, h0)) = *last {
                    (lo, hi) = (l0 + (lo - l0) * 0.25, h0 + (hi - h0) * 0.25);
                }
                *last = Some((lo, hi));
            }
        }
        let span = 1023.0 / (hi - lo);
        let curve = &self.tone[..1024];
        let vib = tuning.vibrance;
        for p in small.px.iter_mut() {
            let [r, g, b] = [curve[((p[0] - lo) * span).clamp(0.0, 1023.0) as usize], curve[((p[1] - lo) * span).clamp(0.0, 1023.0) as usize], curve[((p[2] - lo) * span).clamp(0.0, 1023.0) as usize]];
            if vib == 0.0 {
                *p = [r, g, b];
                continue;
            }
            // Vibrance: the duller colours brought out most.
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            let (dr, dg, db) = (r - y, g - y, b - y);
            let sat = dr.abs().max(dg.abs()).max(db.abs());
            let k = 1.0 + vib * (1.0 - (sat * (1.0 / 0.35)).min(1.0));
            *p = [(y + dr * k).clamp(0.0, 1.0), (y + dg * k).clamp(0.0, 1.0), (y + db * k).clamp(0.0, 1.0)];
        }
        sharpen(small, self.settings.size.sharpen());
    }

    /// The palette's colours for this picture: fixed, held, or found in it
    /// (`live`: a couple of rounds on from the last frame's).
    fn colours(&self, small: &Small, live: bool) -> Vec<[u8; 3]> {
        if let Some(c) = &self.locked {
            return c.clone();
        }
        if self.settings.palette != Palette::Auto {
            return self.settings.palette.colours().to_vec();
        }
        if live {
            let n = self.frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Ok(last) = self.found.lock() {
                if n % 3 != 0 && last.len() == AUTO_COLOURS {
                    return last.clone();
                }
            }
        }
        let mut centres = if live { self.auto.lock().map(|c| c.clone()).unwrap_or_default() } else { Vec::new() };
        let found = auto_palette(small, &mut centres, if live { 2 } else { 8 });
        if live {
            if let Ok(mut c) = self.auto.lock() {
                *c = centres;
            }
            if let Ok(mut f) = self.found.lock() {
                f.clone_from(&found);
            }
        }
        found
    }

    /// The small picture in the palette's colours, dithered (and dated).
    pub fn quantise(&self, small: &Small, live: bool) -> Frame {
        let colours = self.colours(small, live);
        let palette = self.settings.palette;
        let (w, h) = (small.w, small.h);
        let mut idx = vec![0u8; w * h];
        if w > 0 && h > 0 && !colours.is_empty() {
            if palette.ramp() && self.locked.is_none() {
                ramp(small, &colours, self.settings.dither, &mut idx);
            } else {
                let fixed = self.locked.is_none() && palette != Palette::Auto;
                colour(small, &colours, fixed.then(|| table(palette)), &self.cells, palette.spread(), self.settings.dither, &mut idx);
            }
        }
        let mut frame = Frame { w, h, idx, colours, palette };
        if self.settings.stamp {
            frame.stamp(&stamp_text(self.settings.size, chrono::Local::now().date_naive()));
        }
        frame
    }
}

/// Sharpening of a small picture's brightness by `amount`, each pixel held
/// within its neighbours' range (an unsharp mask that cannot overshoot):
/// edges steepen onto pixel boundaries, and no halo is drawn round them.
fn sharpen(small: &mut Small, amount: f32) {
    let (w, h) = (small.w, small.h);
    if w == 0 || h == 0 {
        return;
    }
    // The brightness with its edges repeated a pixel out, so every pixel
    // has eight neighbours; then the sum, least and most of three along
    // the rows, then of those three down the columns.
    let (pw, ph) = (w + 2, h + 2);
    let mut ys = vec![0f32; pw * ph];
    for y in 0..ph {
        let sy = y.saturating_sub(1).min(h - 1);
        for x in 0..pw {
            ys[y * pw + x] = luma(small.px[sy * w + x.saturating_sub(1).min(w - 1)]);
        }
    }
    let (mut sum, mut lo, mut hi) = (vec![0f32; pw * ph], vec![0f32; pw * ph], vec![0f32; pw * ph]);
    for y in 0..ph {
        let r = &ys[y * pw..(y + 1) * pw];
        for x in 1..pw - 1 {
            let (a, b, c) = (r[x - 1], r[x], r[x + 1]);
            sum[y * pw + x] = a + b + c;
            lo[y * pw + x] = a.min(b).min(c);
            hi[y * pw + x] = a.max(b).max(c);
        }
    }
    for y in 0..h {
        let (u, c, d) = (y * pw, (y + 1) * pw, (y + 2) * pw);
        for x in 0..w {
            let k = x + 1;
            let mean = (sum[u + k] + sum[c + k] + sum[d + k]) * (1.0 / 9.0);
            let least = lo[u + k].min(lo[c + k]).min(lo[d + k]);
            let most = hi[u + k].max(hi[c + k]).max(hi[d + k]);
            let v = ys[c + k];
            let delta = (v + amount * (v - mean)).clamp(least, most) - v;
            if delta != 0.0 {
                let p = &mut small.px[y * w + x];
                *p = [(p[0] + delta).clamp(0.0, 1.0), (p[1] + delta).clamp(0.0, 1.0), (p[2] + delta).clamp(0.0, 1.0)];
            }
        }
    }
}
/// A ramp's levels by OKLab lightness: the ramp's colours' lightness spread
/// from black to white (so the scene's whole range uses the whole ramp),
/// dithered in that lightness.
fn ramp(small: &Small, colours: &[[u8; 3]], dither: Dither, out: &mut [u8]) {
    let ls: Vec<f32> = colours.iter().map(|c| lab_of(*c)[0]).collect();
    let (l0, l1) = (ls[0], ls[ls.len() - 1]);
    let q: Vec<f32> = ls.iter().map(|l| ((l - l0) / (l1 - l0).max(1e-3)).clamp(0.0, 1.0)).collect();
    let top = q.len() - 1;
    let (w, h) = (small.w, small.h);
    let lightness = lightness_table();
    let p: Vec<f32> = small.px.iter().map(|c| lightness[((luma(*c) * 1023.0).clamp(0.0, 1023.0) + 0.5) as usize]).collect();
    let nearest_level = |v: f32| {
        let mut best = (f32::MAX, 0usize);
        for (i, l) in q.iter().enumerate() {
            let d = (l - v).abs();
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    };
    match dither {
        Dither::None => {
            for (o, v) in out.iter_mut().zip(&p) {
                *o = nearest_level(*v) as u8;
            }
        }
        Dither::Bayer => {
            for y in 0..h {
                for x in 0..w {
                    let v = p[y * w + x];
                    if top == 0 {
                        continue;
                    }
                    // Between the two levels it falls between, by the matrix.
                    let i = q.iter().rposition(|l| *l <= v).unwrap_or(0).min(top - 1);
                    let t = ((v - q[i]) / (q[i + 1] - q[i]).max(1e-4)).clamp(0.0, 1.0);
                    let th = (f32::from(BAYER[y & 3][x & 3]) + 0.5) / 16.0;
                    out[y * w + x] = if t > th { (i + 1) as u8 } else { i as u8 };
                }
            }
        }
        Dither::Floyd | Dither::Atkinson => {
            let spread = weights(dither);
            let mut carry = vec![0f32; (w + 4) * 3];
            for y in 0..h {
                let back = dither == Dither::Floyd && y % 2 == 1;
                for i in 0..w {
                    let x = if back { w - 1 - i } else { i };
                    let at = (y % 3) * (w + 4) + x + 2;
                    let want = p[y * w + x] + carry[at];
                    carry[at] = 0.0;
                    let level = nearest_level(want);
                    out[y * w + x] = level as u8;
                    let err = want - q[level];
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

/// Light in 4096ths for each encoded byte, and back: each 4096th of light
/// to its encoded value's cell (five bits).
fn light12() -> &'static ([i32; 256], [u8; 4096]) {
    static T: OnceLock<([i32; 256], [u8; 4096])> = OnceLock::new();
    T.get_or_init(|| {
        let lin = to_linear_table();
        let up = std::array::from_fn(|i| (lin[i] * 4095.0).round() as i32);
        let down = std::array::from_fn(|i| ((encode_srgb(i as f32 / 4095.0) * 255.0 + 0.5) as usize >> (8 - LUT_BITS)) as u8);
        (up, down)
    })
}

/// A colour palette's indices, matched in OKLab (through `lut` for a fixed
/// palette, else a table filled in as its cells are met), dithered:
/// ordered in encoded values, diffused in linear light (in integers: a
/// frame's every pixel passes its error on).
fn colour(small: &Small, colours: &[[u8; 3]], lut: Option<&[u8]>, cache: &Mutex<(Vec<[u8; 3]>, Vec<u8>)>, spread: f32, dither: Dither, out: &mut [u8]) {
    let labs: Vec<[f32; 3]> = colours.iter().map(|c| weighted(lab_of(*c))).collect();
    let lin_table = to_linear_table();
    // A frame meets a few thousand of the 32 768 cells: each worked once,
    // and kept for the next frames while the colours stay the same.
    let mut met: Vec<u8> = Vec::new();
    if lut.is_none() {
        if let Ok(mut c) = cache.lock() {
            if c.0 == colours && c.1.len() == 1 << (3 * LUT_BITS) {
                met = std::mem::take(&mut c.1);
            }
        }
        if met.is_empty() {
            met = vec![u8::MAX; 1 << (3 * LUT_BITS)];
        }
    }
    let step = 256 / (1usize << LUT_BITS);
    let mut pick = |i: usize| match lut {
        Some(t) => t[i],
        None => {
            if met[i] == u8::MAX {
                let n = 1usize << LUT_BITS;
                let centre = [i >> (2 * LUT_BITS), (i >> LUT_BITS) & (n - 1), i & (n - 1)].map(|v| lin_table[v * step + step / 2]);
                met[i] = nearest(&labs, weighted(oklab(centre)));
            }
            met[i]
        }
    };
    let (w, h) = (small.w, small.h);
    match dither {
        Dither::None => {
            for (o, c) in out.iter_mut().zip(&small.px) {
                *o = pick(cell(*c));
            }
        }
        Dither::Bayer => {
            for y in 0..h {
                for x in 0..w {
                    let t = ((f32::from(BAYER[y & 3][x & 3]) + 0.5) / 16.0 - 0.5) * spread;
                    out[y * w + x] = pick(cell(small.px[y * w + x].map(|v| v + t)));
                }
            }
        }
        Dither::Floyd | Dither::Atkinson => {
            let (up, down) = light12();
            let lins: Vec<[i32; 3]> = colours.iter().map(|c| c.map(|v| up[usize::from(v)])).collect();
            let px: Vec<[i32; 3]> = small.px.iter().map(|c| c.map(|v| up[(v * 255.0 + 0.5) as usize])).collect();
            // Floyd–Steinberg's sixteenths, Atkinson's eighths.
            let (shift, spread): (u32, &[(i32, usize, i32)]) = match dither {
                Dither::Floyd => (4, &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)]),
                _ => (3, &[(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)]),
            };
            let mut carry = vec![[0i32; 3]; (w + 4) * 3];
            for y in 0..h {
                let back = dither == Dither::Floyd && y % 2 == 1;
                for i in 0..w {
                    let x = if back { w - 1 - i } else { i };
                    let at = (y % 3) * (w + 4) + x + 2;
                    let e = carry[at];
                    carry[at] = [0; 3];
                    let c = px[y * w + x];
                    let want = [0, 1, 2].map(|k| (c[k] + (e[k] >> shift)).clamp(-800, 4895));
                    let q = want.map(|v| usize::from(down[v.clamp(0, 4095) as usize]));
                    let k = pick((q[0] << (2 * LUT_BITS)) | (q[1] << LUT_BITS) | q[2]);
                    out[y * w + x] = k;
                    let got = lins[usize::from(k)];
                    let err = [want[0] - got[0], want[1] - got[1], want[2] - got[2]];
                    for &(dx, dy, wgt) in spread {
                        let dx = if back { -dx } else { dx };
                        let (nx, ny) = (x as i32 + dx, y + dy);
                        if nx >= 0 && (nx as usize) < w && ny < h {
                            let n = &mut carry[(ny % 3) * (w + 4) + nx as usize + 2];
                            n[0] += err[0] * wgt;
                            n[1] += err[1] * wgt;
                            n[2] += err[2] * wgt;
                        }
                    }
                }
            }
        }
    }
    drop(pick);
    if lut.is_none() {
        if let Ok(mut c) = cache.lock() {
            *c = (colours.to_vec(), met);
        }
    }
}
/// OKLab lightness for an encoded brightness (1024 steps): a grey's is
/// the cube root of its light.
fn lightness_table() -> &'static [f32; 1024] {
    static T: OnceLock<[f32; 1024]> = OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| decode_srgb(i as f32 / 1023.0).cbrt()))
}

/// Where a diffusion passes its error: (dx, dy, share).
fn weights(dither: Dither) -> &'static [(i32, usize, f32)] {
    match dither {
        // Floyd–Steinberg: 7, 3, 5 and 1 sixteenths.
        Dither::Floyd => &[(1, 0, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0), (1, 1, 1.0 / 16.0)],
        // Atkinson: an eighth to each of six, two eighths kept back.
        _ => &[(1, 0, 0.125), (2, 0, 0.125), (-1, 1, 0.125), (0, 1, 0.125), (1, 1, 0.125), (0, 2, 0.125)],
    }
}

/// The scene's colours: k-means over every third pixel each way, in OKLab
/// weighted as the matching is (so hues part), `rounds` rounds from
/// `centres` (spread across the brightness when there are none). A small
/// patch of its own colour (a red jacket, a strip of sky) that no centre
/// serves takes one from the two most alike. Then their lightness is
/// stretched to the scene's own darkest and lightest and their colour
/// taken a touch outward (means are duller than what they stand for); as
/// encoded bytes from dark to light.
fn auto_palette(small: &Small, centres: &mut Vec<[f32; 3]>, rounds: usize) -> Vec<[u8; 3]> {
    let samples: Vec<[f32; 3]> = (0..small.h).step_by(3).flat_map(|y| (0..small.w).step_by(3).map(move |x| (x, y))).map(|(x, y)| weighted(oklab(small.px[y * small.w + x].map(linear)))).collect();
    if samples.is_empty() {
        return vec![[0; 3]; AUTO_COLOURS];
    }
    let mut by_l: Vec<f32> = samples.iter().map(|s| s[0]).collect();
    by_l.sort_by(f32::total_cmp);
    if centres.len() != AUTO_COLOURS {
        let mut sorted = samples.clone();
        sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
        *centres = (0..AUTO_COLOURS).map(|i| sorted[(2 * i + 1) * sorted.len() / (2 * AUTO_COLOURS)]).collect();
    }
    let lloyd = |centres: &mut Vec<[f32; 3]>, rounds: usize| {
        let mut sums = vec![[0f32; 3]; AUTO_COLOURS];
        let mut counts = vec![0u32; AUTO_COLOURS];
        for _ in 0..rounds {
            sums.iter_mut().for_each(|s| *s = [0.0; 3]);
            counts.iter_mut().for_each(|c| *c = 0);
            for s in &samples {
                let k = usize::from(nearest(centres, *s));
                for (a, b) in sums[k].iter_mut().zip(s) {
                    *a += b;
                }
                counts[k] += 1;
            }
            for ((c, s), n) in centres.iter_mut().zip(&sums).zip(&counts) {
                if *n > 0 {
                    *c = s.map(|v| v / *n as f32);
                }
            }
        }
    };
    lloyd(centres, rounds);
    for _ in 0..2 {
        // The worst served of the samples that are a patch (a sixtieth of
        // the scene near them) rather than a stray pixel or an edge.
        let mut served: Vec<(f32, [f32; 3])> = samples.iter().map(|s| (centres.iter().map(|c| dist(*c, *s)).fold(f32::MAX, f32::min), *s)).collect();
        served.sort_by(|a, b| b.0.total_cmp(&a.0));
        let patch = served.iter().take(24).take_while(|(far, _)| *far > 0.12 * 0.12).find(|(far, at)| samples.iter().filter(|s| dist(**s, *at) < far * 0.2).count() * 60 >= samples.len());
        let Some(&(_, at)) = patch else { break };
        let mut pair = (f32::MAX, 0, 1);
        for i in 0..AUTO_COLOURS {
            for j in i + 1..AUTO_COLOURS {
                let d = dist(centres[i], centres[j]);
                if d < pair.0 {
                    pair = (d, i, j);
                }
            }
        }
        let (_, i, j) = pair;
        centres[i] = [0, 1, 2].map(|k| (centres[i][k] + centres[j][k]) * 0.5);
        centres[j] = at;
        lloyd(centres, 2);
    }
    let (s0, s1) = (by_l[by_l.len() / 50], by_l[by_l.len() - 1 - by_l.len() / 50]);
    let (c0, c1) = centres.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c[0]), b.max(c[0])));
    let unweight = 1.12 / weighted([0.0, 1.0, 0.0])[1];
    let mut out: Vec<(f32, [u8; 3])> = centres
        .iter()
        .map(|c| {
            let l = if c1 - c0 > 1e-3 { s0 + (c[0] - c0) / (c1 - c0) * (s1 - s0) } else { c[0] };
            let lab = [l.clamp(0.0, 1.0), c[1] * unweight, c[2] * unweight];
            (lab[0], from_oklab(lab).map(|v| (encoded(v) * 255.0).round() as u8))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, c)| c).collect()
}

// ------------------------------------------------------------- frames ---

/// A pixel picture: its colours' indices, a byte a pixel, and its colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub idx: Vec<u8>,
    pub colours: Vec<[u8; 3]>,
    pub palette: Palette,
}

/// The date stamp's figures, 3 × 5 (our own): a row a byte, three bits.
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b011, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        _ => return None,
    })
}

/// The stamp's words for a size: "26.11.23" (year, month, day), or
/// "11.23" where the picture is only 56 across.
pub fn stamp_text(size: Size, date: chrono::NaiveDate) -> String {
    if size == Size::Chunky {
        date.format("%m.%d").to_string()
    } else {
        date.format("%y.%m.%d").to_string()
    }
}

impl Frame {
    /// As colours, to draw.
    pub fn picture(&self) -> Picture {
        let mut out = Picture::new(self.w, self.h);
        if self.colours.is_empty() {
            return out;
        }
        for (o, k) in out.rgb.chunks_exact_mut(3).zip(&self.idx) {
            o.copy_from_slice(&self.colours[usize::from(*k).min(self.colours.len() - 1)]);
        }
        out
    }

    /// `text` in the bottom right corner in the pixel font: in the palette's
    /// colour nearest a warm orange, shadowed in its darkest.
    pub fn stamp(&mut self, text: &str) {
        if self.colours.is_empty() {
            return;
        }
        let labs: Vec<[f32; 3]> = self.colours.iter().map(|c| lab_of(*c)).collect();
        let ink = nearest(&labs, lab_of([0xf4, 0x8c, 0x2a]));
        let shade = labs.iter().enumerate().min_by(|a, b| a.1[0].total_cmp(&b.1[0])).map_or(0, |(i, _)| i as u8);
        let width: usize = text.chars().map(|c| if c == '.' { 2 } else { 4 }).sum::<usize>().saturating_sub(1);
        let margin = if self.w < 64 { 1 } else { 2 };
        if width + margin + 2 > self.w || self.h < 7 + margin {
            return;
        }
        let (mut x, y) = (self.w - margin - 1 - width, self.h - margin - 1 - 5);
        for c in text.chars() {
            let Some(rows) = glyph(c) else { continue };
            // A dot is a column wide: its middle column only.
            let (cols, shift) = if c == '.' { (1, 1) } else { (3, 0) };
            for (dx, dy, k) in [(1, 1, shade), (0, 0, ink)] {
                for (j, row) in rows.iter().enumerate() {
                    for i in 0..cols {
                        if row >> (2 - i - shift) & 1 == 1 {
                            let (px, py) = (x + i + dx, y + j + dy);
                            if px < self.w && py < self.h {
                                self.idx[py * self.w + px] = k;
                            }
                        }
                    }
                }
            }
            x += cols + 1;
        }
    }

    /// As an indexed PNG, each pixel `scale` square, saying `look`.
    pub fn png(&self, scale: usize, look: &str) -> Result<Vec<u8>, String> {
        let scale = scale.max(1);
        let (depth, bits) = match self.colours.len() {
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
            enc.set_palette(self.colours.iter().flatten().copied().collect::<Vec<u8>>());
            enc.add_text_chunk("Software".into(), "Catalyst Camera".into()).map_err(|e| e.to_string())?;
            enc.add_text_chunk("look".into(), look.to_owned()).map_err(|e| e.to_string())?;
            enc.add_text_chunk("pixel".into(), format!("{} {}", self.w, self.h)).map_err(|e| e.to_string())?;
            let mut writer = enc.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&data).map_err(|e| e.to_string())?;
        }
        Ok(bytes)
    }
}

/// A dither's sample for its picker: a ramp from dark to light (even by
/// eye) across `w` × `h`, in two colours.
pub fn sample(dither: Dither, w: usize, h: usize) -> Vec<u8> {
    let small = Small { w, h, px: (0..w * h).map(|i| [encoded(((i % w) as f32 / (w - 1).max(1) as f32).powi(3)); 3]).collect() };
    let mut out = vec![0u8; w * h];
    ramp(&small, &[[0, 0, 0], [255, 255, 255]], dither, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, c: [u8; 3]) -> Small {
        Small { w, h, px: vec![c.map(|v| f32::from(v) / 255.0); w * h] }
    }

    fn from_bytes(rgb: &[u8], w: usize, h: usize) -> Small {
        Small { w, h, px: rgb.chunks_exact(3).map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0)).collect() }
    }

    fn with(palette: Palette, dither: Dither) -> Pixel {
        Pixel::plain(Settings { palette, dither, ..Settings::default() })
    }

    #[test]
    fn a_colour_palette_maps_its_own_colours_to_themselves_and_ramps_run_dark_to_light() {
        for palette in [Palette::Print, Palette::Poster, Palette::Crayon] {
            let colours = palette.colours();
            let rgb: Vec<u8> = colours.iter().flatten().copied().collect();
            for dither in [Dither::None, Dither::Floyd, Dither::Atkinson] {
                let f = with(palette, dither).quantise(&from_bytes(&rgb, colours.len(), 1), false);
                for (k, i) in f.idx.iter().enumerate() {
                    assert_eq!(colours[usize::from(*i)], colours[k], "{} {dither:?}: colour {k}", palette.name());
                }
            }
        }
        for palette in PALETTES.into_iter().filter(|p| p.ramp()) {
            let l: Vec<f32> = palette.colours().iter().map(|c| lab_of(*c)[0]).collect();
            assert!(l.windows(2).all(|p| p[0] < p[1]), "{} runs dark to light", palette.name());
            // Its middle colours take the middle tones, in order.
            let f = with(palette, Dither::None).quantise(&from_bytes(&(0..64).flat_map(|i| [(i * 4) as u8; 3]).collect::<Vec<u8>>(), 64, 1), false);
            assert!(f.idx.windows(2).all(|p| p[0] <= p[1]), "{}: {:?}", palette.name(), f.idx);
            assert_eq!((f.idx[0], f.idx[63]), (0, (l.len() - 1) as u8));
        }
        assert_eq!(PALETTES.len(), 8);
        assert_eq!(PALETTES.map(|p| p.colours().len()), [2, 4, 4, 4, 4, 8, 16, 0]);
    }

    #[test]
    fn colours_are_matched_perceptually_not_by_their_values() {
        // A grey of 50 is nearer black by its value (50 against 78), but
        // nearer the mid grey 128 by eye, as OKLab has it; 30 is black's.
        let greys = [[0u8, 0, 0], [128, 128, 128], [255, 255, 255]];
        let labs: Vec<[f32; 3]> = greys.iter().map(|c| lab_of(*c)).collect();
        assert_eq!(nearest(&labs, lab_of([30, 30, 30])), 0);
        assert_eq!(nearest(&labs, lab_of([50, 50, 50])), 1, "50 looks nearer 128 than black");
        // Poster's red and sky for a red and a sky.
        let poster = with(Palette::Poster, Dither::None);
        assert_eq!(POSTER[usize::from(poster.quantise(&from_bytes(&[200, 60, 50], 1, 1), false).idx[0])], POSTER[2]);
        assert_eq!(POSTER[usize::from(poster.quantise(&from_bytes(&[130, 190, 225], 1, 1), false).idx[0])], POSTER[4]);
        // OKLab there and back.
        for c in [[0.2f32, 0.5, 0.8], [1.0, 0.0, 0.0], [0.05, 0.05, 0.05]] {
            let back = from_oklab(oklab(c));
            assert!(c.iter().zip(back).all(|(a, b)| (a - b).abs() < 2e-3), "{c:?} {back:?}");
        }
        assert!((oklab([1.0; 3])[0] - 1.0).abs() < 1e-3 && oklab([0.0; 3])[0] < 1e-3);
    }

    #[test]
    fn black_and_white_land_on_the_ends() {
        for palette in PALETTES.into_iter().filter(|p| *p != Palette::Auto) {
            for dither in DITHERS {
                let p = with(palette, dither);
                let black = p.quantise(&flat(8, 8, [0, 0, 0]), false);
                let white = p.quantise(&flat(8, 8, [255, 255, 255]), false);
                let l: Vec<f32> = palette.colours().iter().map(|c| lab_of(*c)[0]).collect();
                let darkest = l.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).unwrap().0 as u8;
                let lightest = l.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0 as u8;
                assert!(black.idx.iter().all(|k| *k == darkest), "{} {dither:?}: black {:?}", palette.name(), black.idx);
                assert!(white.idx.iter().all(|k| *k == lightest), "{} {dither:?}: white {:?}", palette.name(), white.idx);
            }
        }
    }

    #[test]
    fn mid_grey_dithers_to_half_by_eye_and_the_diffusions_keep_the_mean() {
        // 1-bit ink, a 16 × 16 at perceptual mid grey (OKLab L 0.5: 99).
        let grey = flat(16, 16, [99, 99, 99]);
        let share = |f: &Frame| f.idx.iter().filter(|k| **k == 1).count();
        let bayer = with(Palette::Ink, Dither::Bayer).quantise(&grey, false);
        assert!((120..=136).contains(&share(&bayer)), "{}", share(&bayer));
        assert_eq!(bayer.idx[..64], bayer.idx[64..128], "the pattern repeats every four rows");
        for d in [Dither::Floyd, Dither::Atkinson] {
            let n = share(&with(Palette::Ink, d).quantise(&grey, false));
            assert!((108..=148).contains(&n), "{d:?}: {n}");
        }
        // Atkinson keeps a pale grey cleaner than Floyd–Steinberg.
        let pale = flat(16, 16, [215, 215, 215]);
        let dark_dots = |d| 256 - share(&with(Palette::Ink, d).quantise(&pale, false));
        assert!(dark_dots(Dither::Atkinson) < dark_dots(Dither::Floyd), "{} {}", dark_dots(Dither::Atkinson), dark_dots(Dither::Floyd));
        // A colour palette's diffusion keeps the mean in light: half of
        // print's paper and half its overprint, in light, comes out so.
        let t = to_linear_table();
        let want: [f32; 3] = [0, 1, 2].map(|k| (t[usize::from(PRINT[0][k])] + t[usize::from(PRINT[3][k])]) * 0.5);
        let between = Small { w: 24, h: 24, px: vec![want.map(encoded); 576] };
        for d in [Dither::Floyd, Dither::Atkinson] {
            let f = with(Palette::Print, d).quantise(&between, false);
            for (k, want) in want.iter().enumerate() {
                let mean = f.idx.iter().map(|i| t[usize::from(f.colours[usize::from(*i)][k])]).sum::<f32>() / f.idx.len() as f32;
                let allowed = if d == Dither::Atkinson { 0.08 } else { 0.02 };
                assert!((mean - want).abs() < allowed, "{d:?} channel {k}: {mean} against {want}");
            }
        }
    }

    #[test]
    fn a_picture_is_shrunk_in_linear_light() {
        // A fine checker of black and white: half the light, 188 encoded.
        let (w, h) = (64, 48);
        let mut pic = Picture::new(w, h);
        for (i, p) in pic.rgb.chunks_exact_mut(3).enumerate() {
            let v = if (i % w + i / w) % 2 == 0 { 0 } else { 255 };
            p.copy_from_slice(&[v; 3]);
        }
        let small = small_from_rgb(&pic, 16, 12);
        assert!(small.px.iter().all(|p| (p[0] * 255.0 - 188.0).abs() < 2.0), "{:?}", small.px[0]);
        // From a preview frame, its brightness likewise.
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        let small = small_from_yuv(&yuv, w, h, 16, 12);
        assert!(small.px.iter().all(|p| (luma(*p) * 255.0 - 188.0).abs() < 3.0), "{:?}", small.px[0]);
        // A flat colour stays itself.
        let mut red = Picture::new(320, 240);
        red.rgb.chunks_exact_mut(3).for_each(|p| p.copy_from_slice(&[200, 40, 40]));
        let s = small_from_rgb(&red, 75, 56);
        assert!(s.px.iter().all(|p| (p[0] * 255.0 - 200.0).abs() < 1.0 && (p[1] * 255.0 - 40.0).abs() < 1.0));
    }

    #[test]
    fn sharpening_steepens_an_edge_without_a_halo() {
        // A soft edge from 0.3 to 0.7 across a row of eight.
        let profile = [0.3f32, 0.3, 0.35, 0.5, 0.65, 0.7, 0.7, 0.7];
        let mut small = Small { w: 8, h: 3, px: (0..24).map(|i| [profile[i % 8]; 3]).collect() };
        sharpen(&mut small, 0.75);
        let row: Vec<f32> = (0..8).map(|x| luma(small.px[8 + x])).collect();
        assert!(row[2] < 0.35 - 0.01 && row[4] > 0.65 + 0.01, "steeper: {row:?}");
        assert!(row.iter().all(|v| (0.3 - 1e-4..=0.7 + 1e-4).contains(v)), "no halo past either side: {row:?}");
        assert!((row[3] - 0.5).abs() < 1e-4, "the middle stays: {row:?}");
    }

    #[test]
    fn the_auto_palette_finds_the_scenes_colours_and_follows_it_steadily() {
        let cols = [[30u8, 40, 120], [220, 60, 40], [240, 230, 200], [50, 140, 60]];
        let (w, h) = (32, 24);
        let px: Vec<[f32; 3]> = (0..w * h).map(|i| cols[(i % w) / 16 + 2 * ((i / w) / 12)].map(|v| f32::from(v) / 255.0)).collect();
        let small = Small { w, h, px };
        let p = Pixel::plain(Settings { palette: Palette::Auto, dither: Dither::None, ..Settings::default() });
        let f = p.quantise(&small, true);
        assert_eq!(f.colours.len(), AUTO_COLOURS);
        for c in cols {
            let d = f.colours.iter().map(|k| dist(lab_of(*k), lab_of(c))).fold(f32::MAX, f32::min);
            assert!(d.sqrt() < 0.06, "{c:?}: {} from the nearest of {:?}", d.sqrt(), f.colours);
        }
        let ls: Vec<f32> = f.colours.iter().map(|c| lab_of(*c)[0]).collect();
        assert!(ls.windows(2).all(|p| p[0] <= p[1] + 1e-4), "dark to light");
        assert_eq!(p.quantise(&small, true).colours, f.colours, "the same scene, the same colours");
        // Held: another scene keeps them.
        let held = Pixel::locked(Settings { palette: Palette::Auto, ..Settings::default() }, f.colours.clone());
        assert_eq!(held.quantise(&flat(8, 8, [10, 200, 10]), true).colours, f.colours);
    }

    #[test]
    fn a_small_patch_of_its_own_colour_gets_one_and_hue_counts_in_matching() {
        // A sky-to-ground ramp of blues and greys with a red bumper in it, a
        // fiftieth of the picture: the auto palette keeps a red for it.
        let (w, h) = (60, 45);
        let px: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (26..35).contains(&x) && (30..36).contains(&y) {
                    return [0.8, 0.1, 0.1];
                }
                let t = y as f32 / h as f32;
                [0.25 + 0.4 * (1.0 - t), 0.3 + 0.4 * (1.0 - t), 0.35 + 0.55 * (1.0 - t)]
            })
            .collect();
        let p = Pixel::plain(Settings { palette: Palette::Auto, dither: Dither::None, ..Settings::default() });
        let f = p.quantise(&Small { w, h, px }, false);
        assert!(f.colours.iter().any(|c| lab_of(*c)[1] > 0.1), "a red among {:?}", f.colours);
        // A dull green field goes to the poster's green, not its navy (the
        // nearer in plain OKLab, by lightness).
        let poster = Pixel::plain(Settings { palette: Palette::Poster, dither: Dither::None, ..Settings::default() });
        for field in [[80, 95, 60], [85, 100, 75], [65, 80, 50]] {
            assert_eq!(POSTER[usize::from(poster.quantise(&from_bytes(&field, 1, 1), false).idx[0])], POSTER[3], "{field:?}");
        }
    }

    #[test]
    fn the_sizes_land_on_whole_panel_pixels_and_keep_four_by_three() {
        let vf = ((crate::gfx::H * 4.0 / 3.0).round() as usize, crate::gfx::H as usize);
        for size in SIZES {
            let (w, h) = size.dims();
            let s = size.view_scale();
            assert!(w * s <= vf.0 && h * s <= vf.1, "{size:?} fits the viewfinder");
            assert!(w * s + 4 >= vf.0 && h * s + 3 >= vf.1, "{size:?} fills it");
            assert!((w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.01);
            assert!((440..=460).contains(&(w * size.share_scale())));
        }
        let pic = crate::camera::scene(320, 240, 1.0, false);
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        let still = crate::camera::scene(1640, 1232, 1.0, false);
        for size in SIZES {
            let p = Pixel::new(Settings { size, ..Settings::default() });
            let f = p.develop(&yuv, 320, 240);
            assert_eq!((f.w, f.h, f.idx.len()), (size.dims().0, size.dims().1, size.dims().0 * size.dims().1));
            assert_eq!((p.develop_still(&still).w, p.develop_still(&still).h), size.dims());
        }
    }

    #[test]
    fn the_settings_go_through_their_line_and_odd_lines_fall_back() {
        let s = Settings { size: Size::Fine, palette: Palette::Auto, dither: Dither::Atkinson, bright: -2, contrast: Contrast::Hard, stamp: true };
        assert_eq!(s.line(), "112,auto,atkinson,-2,hard,1");
        assert_eq!(Settings::from_line(&s.line()), s);
        assert_eq!(Settings::from_line("nonsense"), Settings::default());
        assert_eq!(Settings::from_line("56,tide,none,9").bright, BRIGHT_STEPS);
        assert!(!Settings::from_line("75,ember,bayer,0,normal").stamp, "1.3.0's line: no stamp");
        let p = |bright, contrast| Pixel::new(Settings { bright, contrast, ..Settings::default() });
        assert!(p(2, Contrast::Normal).tone[512] > p(0, Contrast::Normal).tone[512]);
        assert!(p(0, Contrast::Hard).tone[256] < p(0, Contrast::Soft).tone[256]);
        assert!(p(0, Contrast::Hard).tone[768] > p(0, Contrast::Soft).tone[768]);
    }

    #[test]
    fn the_date_stamp_sits_in_the_corner_in_the_pixel_font() {
        let mut f = with(Palette::Ember, Dither::None).quantise(&flat(75, 56, [40, 20, 20]), false);
        let before = f.idx.clone();
        let text = stamp_text(Size::Small, chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap());
        assert_eq!(text, "26.11.23");
        f.stamp(&text);
        let changed: Vec<(usize, usize)> = (0..f.idx.len()).filter(|i| f.idx[*i] != before[*i]).map(|i| (i % 75, i / 75)).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|(x, y)| *x >= 75 - 2 - 30 && *x < 75 - 1 && *y >= 56 - 2 - 7 && *y < 56 - 1), "{changed:?}");
        assert!(changed.iter().any(|(x, y)| f.colours[usize::from(f.idx[y * 75 + x])] == EMBER[2]), "in ember's orange");
        assert_eq!(stamp_text(Size::Chunky, chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()), "01.05");
        assert!(!Settings::default().stamp, "off at first");
    }

    #[test]
    fn a_picture_goes_into_an_indexed_png_at_its_size_and_blown_up() {
        let f = with(Palette::Graphite, Dither::Bayer).quantise(&from_bytes(&(0..12 * 9).flat_map(|i| [(i * 2) as u8; 3]).collect::<Vec<u8>>(), 12, 9), false);
        for scale in [1, 4] {
            let bytes = f.png(scale, "Pixel · graphite · bayer").unwrap();
            let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (12 * scale as u32, 9 * scale as u32));
            assert!(reader.info().uncompressed_latin1_text.iter().any(|t| t.keyword == "look" && t.text == "Pixel · graphite · bayer"));
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let out = reader.next_frame(&mut buf).unwrap();
            let rgb = &buf[..out.buffer_size()];
            let want = f.picture();
            for y in 0..9 * scale {
                for x in 0..12 * scale {
                    assert_eq!(&rgb[(y * 12 * scale + x) * 3..][..3], &want.rgb[((y / scale) * 12 + x / scale) * 3..][..3]);
                }
            }
        }
    }

    #[test]
    fn a_preview_frame_in_pixel_mode_costs_little_more_than_converting_it() {
        // As the looks' test: 320 × 240, timed here against the plain
        // conversion timed alongside (the A53 is reckoned at about fifteen
        // times slower than this computer's core, and a frame may spend 6 ms).
        let pic = crate::camera::scene(320, 240, 1.0, false);
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        let mut plain = crate::photos::Picture::default();
        // The quickest of five runs of eight (other tests share the cores:
        // the quickest is the one least got in the way of), the conversion
        // timed again beside each.
        let time = |f: &mut dyn FnMut()| {
            f();
            (0..5)
                .map(|_| {
                    let began = std::time::Instant::now();
                    for _ in 0..8 {
                        f();
                    }
                    began.elapsed().as_secs_f64() * 1000.0 / 8.0
                })
                .fold(f64::MAX, f64::min)
        };
        let mut worst = (0.0f64, 0.0f64, 0.0f64, String::new());
        for size in SIZES {
            for palette in [Palette::Ink, Palette::Crayon, Palette::Auto] {
                for dither in DITHERS {
                    let p = Pixel::new(Settings { size, palette, dither, stamp: true, ..Settings::default() });
                    let convert = time(&mut || crate::camera::yuv420_to_rgb(&yuv, 320, 240, &mut plain)).max(0.02);
                    let took = time(&mut || {
                        let f = p.develop(&yuv, 320, 240);
                        std::hint::black_box(f.picture());
                    });
                    if took / convert > worst.0 {
                        worst = (took / convert, took, convert, format!("{size:?} {} {}", palette.name(), dither.name()));
                    }
                }
            }
        }
        let (ratio, took, convert, what) = worst;
        eprintln!("a 320 × 240 preview here: converted {convert:.3} ms; as a pixel picture at worst {took:.3} ms ({what}), {ratio:.1} times; at the A53's reckoned fifteenth, {:.1} ms", took * 15.0);
        assert!(ratio < 3.5, "{took} ms ({what}), against {convert} ms converted");
    }
}
