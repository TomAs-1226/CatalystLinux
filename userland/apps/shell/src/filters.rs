//! Looks: the Camera's filters, shown live in the viewfinder and laid over
//! the saved photo.
//!
//! Each look is a short recipe (white balance shift, saturation or a black
//! and white channel mix, a contrast curve with its black point and fade,
//! split toning, or a duotone for Tint) worked out once in floating point
//! ([`Recipe::map`]), then made fast two ways from that one definition:
//!
//! - for the preview, a 3 × 3 matrix in fixed point, one tone curve of
//!   1024 steps and a 256-step finish by brightness, folded into the
//!   frame's YUV → RGB conversion ([`Compiled::develop`]: each 2 × 2
//!   block's colour through the matrix once, then three multiplies a
//!   pixel), the frame's halves on two cores, on the camera's own thread.
//!   A 320 × 240 frame with a look and the zebras takes 0.28 ms on the
//!   computer these were measured on, against 0.13 ms for the plain
//!   conversion it replaces; at the A53's reckoned fifteenth of that speed,
//!   about 4 ms on the handheld (2 ms of it the conversion it always did),
//!   inside the 6 ms a frame may spend;
//! - for the photo, a 33 × 33 × 33 table from the camera's YUV straight to
//!   YUV ([`bake`]), handed to the camera program as a file and laid over
//!   the 8 MP frame in its background save, on three threads
//!   (catalyst-camera's `apply_lut`, the same sums as [`apply_yuv420`]).
//!
//! The iOS-style looks (Vivid, Vivid Warm, Vivid Cool, Dramatic, Dramatic
//! Warm, Dramatic Cool, Mono, Silvertone, Noir) and the four inspired by a
//! Leica's (Classic, Contemporary, BW Natural, BW High Contrast) are this
//! file's own numbers, made by eye on the simulated scene and on photos:
//! no one else's tables are copied. Tint is a duotone of any colour.
//!
//! A photo's look is written into its Exif (UserComment, "look: Vivid"),
//! and the photo as the camera took it is kept beside it in `.originals`,
//! which Photos offers with `o`.
//!
//! The film looks (1.2, their own group after the digital ones) are this
//! file's own too, after kinds of film rather than any stock: no name or
//! table is anyone's, and none claims to match a real film. Each adds to
//! the recipe a film's tone (a toe that deepens the blacks, a soft shoulder
//! that rolls the highlights off) and its colour response: hue shifts and
//! saturation by range, for skin, sky and foliage apart ([`Hues`]), worked
//! on the colour alone (the frame's chroma) before the matrix, so the
//! preview's form gains one table lookup for each 2 × 2 block. Grain and
//! halation (film.rs) are laid over the picture after the look, at the
//! strength chosen (off, light or strong).

pub mod film;
pub mod pixel;

/// The looks, in the strip's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Look {
    #[default]
    Original,
    Vivid,
    VividWarm,
    VividCool,
    Dramatic,
    DramaticWarm,
    DramaticCool,
    Mono,
    Silvertone,
    Noir,
    Classic,
    Contemporary,
    BwNatural,
    BwHighContrast,
    Tint,
    /// Film (1.2): a daylight slide, punchy, its blacks deep.
    Sunlit,
    /// A portrait negative: soft, low in contrast, warm in the skin.
    Portrait,
    /// A faded slide: muted colour, hard shadows.
    Faded,
    /// A tungsten night stock: cool, a red-orange glow round the lights.
    Tungsten,
    /// Black and white pushed: gritty, hard, heavy grain.
    Pushed,
    /// Soft black and white, fine grain.
    Pearl,
    /// A cine look: teal shadows, warm highlights, a gentle roll-off.
    Cine,
}

/// The film looks, the strip's second group.
pub const FILM: [Look; 7] = [Look::Sunlit, Look::Portrait, Look::Faded, Look::Tungsten, Look::Pushed, Look::Pearl, Look::Cine];

pub const LOOKS: [Look; 22] = [
    Look::Original,
    Look::Vivid,
    Look::VividWarm,
    Look::VividCool,
    Look::Dramatic,
    Look::DramaticWarm,
    Look::DramaticCool,
    Look::Mono,
    Look::Silvertone,
    Look::Noir,
    Look::Classic,
    Look::Contemporary,
    Look::BwNatural,
    Look::BwHighContrast,
    Look::Tint,
    Look::Sunlit,
    Look::Portrait,
    Look::Faded,
    Look::Tungsten,
    Look::Pushed,
    Look::Pearl,
    Look::Cine,
];

/// BT.601's brightness weights, as the camera's YUV has them.
const LUMA: [f32; 3] = [0.299, 0.587, 0.114];

impl Look {
    pub fn name(self) -> &'static str {
        match self {
            Look::Original => "Original",
            Look::Vivid => "Vivid",
            Look::VividWarm => "Vivid Warm",
            Look::VividCool => "Vivid Cool",
            Look::Dramatic => "Dramatic",
            Look::DramaticWarm => "Dramatic Warm",
            Look::DramaticCool => "Dramatic Cool",
            Look::Mono => "Mono",
            Look::Silvertone => "Silvertone",
            Look::Noir => "Noir",
            Look::Classic => "Classic",
            Look::Contemporary => "Contemporary",
            Look::BwNatural => "BW Natural",
            Look::BwHighContrast => "BW High Contrast",
            Look::Tint => "Tint",
            Look::Sunlit => "Sunlit Slide",
            Look::Portrait => "Soft Negative",
            Look::Faded => "Faded Slide",
            Look::Tungsten => "Tungsten",
            Look::Pushed => "Pushed Mono",
            Look::Pearl => "Pearl Mono",
            Look::Cine => "Cine",
        }
    }

    /// One of the film looks (the strip's second group).
    pub fn film(self) -> bool {
        FILM.contains(&self)
    }

    /// Its grain and halation at full strength (film.rs; none for the
    /// digital looks).
    pub fn texture(self) -> film::Texture {
        let (grain, halation) = match self {
            Look::Sunlit => (0.030, 0.0),
            Look::Portrait => (0.034, 0.14),
            Look::Faded => (0.040, 0.0),
            Look::Tungsten => (0.046, 0.62),
            Look::Pushed => (0.085, 0.0),
            Look::Pearl => (0.030, 0.0),
            Look::Cine => (0.026, 0.30),
            _ => (0.0, 0.0),
        };
        film::Texture { grain, halation }
    }

    pub fn id(self) -> &'static str {
        match self {
            Look::Original => "original",
            Look::Vivid => "vivid",
            Look::VividWarm => "vivid-warm",
            Look::VividCool => "vivid-cool",
            Look::Dramatic => "dramatic",
            Look::DramaticWarm => "dramatic-warm",
            Look::DramaticCool => "dramatic-cool",
            Look::Mono => "mono",
            Look::Silvertone => "silvertone",
            Look::Noir => "noir",
            Look::Classic => "classic",
            Look::Contemporary => "contemporary",
            Look::BwNatural => "bw-natural",
            Look::BwHighContrast => "bw-high-contrast",
            Look::Tint => "tint",
            Look::Sunlit => "film-sunlit",
            Look::Portrait => "film-negative",
            Look::Faded => "film-faded",
            Look::Tungsten => "film-tungsten",
            Look::Pushed => "film-pushed",
            Look::Pearl => "film-pearl",
            Look::Cine => "film-cine",
        }
    }

    pub fn from_id(id: &str) -> Option<Look> {
        LOOKS.iter().copied().find(|l| l.id() == id)
    }

    /// Black and white, whatever comes in.
    #[cfg(test)]
    pub fn neutral(self) -> bool {
        matches!(self, Look::Mono | Look::Silvertone | Look::Noir | Look::BwNatural | Look::BwHighContrast | Look::Pushed | Look::Pearl)
    }

    /// The look's recipe; `hue` is Tint's colour (degrees).
    pub fn recipe(self, hue: f32) -> Recipe {
        let plain = Recipe::default();
        let vivid = Recipe { saturation: 1.32, contrast: 0.18, ..plain };
        let dramatic = Recipe { saturation: 0.92, contrast: 0.55, exposure: 0.9, black: 0.02, ..plain };
        let natural = [0.30, 0.59, 0.11];
        match self {
            Look::Original => plain,
            Look::Vivid => vivid,
            Look::VividWarm => Recipe { wb: [1.07, 1.0, 0.88], ..vivid },
            Look::VividCool => Recipe { wb: [0.90, 1.0, 1.09], ..vivid },
            Look::Dramatic => dramatic,
            Look::DramaticWarm => Recipe { wb: [1.10, 1.0, 0.84], saturation: 0.88, ..dramatic },
            Look::DramaticCool => Recipe { wb: [0.86, 0.98, 1.12], saturation: 0.88, ..dramatic },
            Look::Mono => Recipe { mono: Some(LUMA), contrast: 0.22, ..plain },
            // Bright, glowing black and white: lifted, soft blacks.
            Look::Silvertone => Recipe { mono: Some([0.36, 0.50, 0.14]), contrast: 0.45, exposure: 1.06, fade: 0.03, ..plain },
            // Hard black and white, the shadows crushed.
            Look::Noir => Recipe { mono: Some([0.42, 0.46, 0.12]), contrast: 0.85, black: 0.06, ..plain },
            // Muted colour with a film's lifted blacks: cool shadows, warm highlights.
            Look::Classic => Recipe {
                wb: [1.03, 1.0, 0.95],
                saturation: 0.78,
                contrast: 0.1,
                fade: 0.045,
                shadows: [-0.02, 0.0, 0.025],
                highlights: [0.025, 0.01, -0.02],
                ..plain
            },
            // Clear, punchy colour, a touch cool in the shadows.
            Look::Contemporary => Recipe { wb: [0.98, 1.0, 1.02], saturation: 1.12, contrast: 0.32, shadows: [-0.015, 0.0, 0.02], ..plain },
            // Soft, open black and white: no added contrast, a little lift.
            Look::BwNatural => Recipe { mono: Some(natural), exposure: 1.05, fade: 0.04, ..plain },
            // A red filter's black and white: skies darken, faces lighten.
            Look::BwHighContrast => Recipe { mono: Some([0.50, 0.40, 0.10]), contrast: 0.75, black: 0.03, ..plain },
            Look::Tint => Recipe { mono: Some(natural), contrast: 0.15, duotone: Some(tint_colour(hue)), ..plain },
            // Film (1.2). Bands by hue: skin about 25°, foliage 100°, sky 212°.
            // A daylight slide: lively and a touch cool, deep blacks, sky
            // deepened toward blue, greens fresh, skin held back.
            Look::Sunlit => Recipe {
                wb: [0.99, 1.0, 1.02],
                saturation: 1.2,
                contrast: 0.4,
                black: 0.012,
                toe: 0.6,
                shoulder: 0.3,
                hues: Some(Hues { bands: [skin(3.0, 0.96), foliage(-5.0, 1.14), sky(7.0, 1.24)], roll: 0.3 }),
                ..plain
            },
            // A portrait negative: soft and warm, lifted blacks, a long
            // shoulder, skin a little richer, greens and sky quietened.
            Look::Portrait => Recipe {
                wb: [1.04, 1.0, 0.95],
                saturation: 0.92,
                exposure: 1.03,
                contrast: 0.06,
                fade: 0.035,
                shoulder: 0.7,
                hues: Some(Hues { bands: [skin(-2.0, 1.07), foliage(-8.0, 0.84), sky(-4.0, 0.86)], roll: 0.45 }),
                ..plain
            },
            // A faded slide: muted, its shadows hard, highlights warm and
            // shadows teal, greens and sky drained toward cyan.
            Look::Faded => Recipe {
                saturation: 0.74,
                contrast: 0.34,
                black: 0.03,
                toe: 0.8,
                shoulder: 0.5,
                shadows: [-0.005, 0.01, 0.02],
                highlights: [0.025, 0.015, -0.01],
                hues: Some(Hues { bands: [skin(0.0, 0.92), foliage(7.0, 0.76), sky(-9.0, 0.8)], roll: 0.2 }),
                ..plain
            },
            // A tungsten night stock: balanced for lamps, so cool by day,
            // teal-blue shadows; its glow is the halation (film.rs).
            Look::Tungsten => Recipe {
                wb: [0.88, 0.98, 1.12],
                saturation: 1.04,
                contrast: 0.24,
                toe: 0.4,
                shoulder: 0.55,
                shadows: [-0.02, 0.0, 0.03],
                hues: Some(Hues { bands: [skin(0.0, 1.0), foliage(8.0, 0.9), sky(4.0, 1.1)], roll: 0.3 }),
                ..plain
            },
            // Black and white pushed two stops: hard, the shadows crushed.
            Look::Pushed => Recipe { mono: Some([0.40, 0.47, 0.13]), contrast: 0.95, black: 0.04, toe: 0.5, shoulder: 0.2, ..plain },
            // Soft black and white: open, low contrast, a long shoulder.
            Look::Pearl => Recipe { mono: Some([0.30, 0.58, 0.12]), contrast: 0.1, exposure: 1.04, fade: 0.03, shoulder: 0.8, ..plain },
            // A cine look: teal shadows, warm highlights, a gentle roll-off;
            // skin kept warm while greens and sky lean teal.
            Look::Cine => Recipe {
                wb: [1.02, 1.0, 0.98],
                saturation: 0.95,
                contrast: 0.28,
                toe: 0.3,
                shoulder: 0.8,
                shadows: [-0.03, 0.01, 0.035],
                highlights: [0.035, 0.012, -0.03],
                hues: Some(Hues { bands: [skin(2.0, 1.04), foliage(12.0, 0.8), sky(-8.0, 0.9)], roll: 0.25 }),
                ..plain
            },
        }
    }

    /// The look made fast for the preview.
    pub fn compiled(self, hue: f32) -> Compiled {
        self.recipe(hue).compile()
    }
}

/// Tint's colour for a hue: a gentle saturation, its brightness half way,
/// so the duotone runs black → the colour → white.
pub fn tint_colour(hue: f32) -> [f32; 3] {
    let c = hsv(hue, 0.55, 1.0);
    let l = dot(c, LUMA).max(1e-3);
    // Scaled to a brightness of a half; any channel over 1 pulled back
    // toward grey, keeping the brightness.
    let mut t = c.map(|v| v * 0.5 / l);
    let over = t.iter().copied().fold(0.0f32, f32::max);
    if over > 1.0 {
        let k = (1.0 - 0.5) / (over - 0.5);
        t = t.map(|v| 0.5 + (v - 0.5) * k);
    }
    t
}

/// Hue (degrees), saturation and value to RGB, 0 to 1.
pub fn hsv(hue: f32, s: f32, v: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A look's recipe, in display terms (0 to 1, as encoded).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    /// Each channel scaled first: a warmer or cooler white.
    pub wb: [f32; 3],
    /// Black and white: the channels' weights.
    pub mono: Option<[f32; 3]>,
    pub saturation: f32,
    /// Brightness, scaled before the curve.
    pub exposure: f32,
    /// The contrast curve's strength: 0 is none, 1 strong (an S about mid grey).
    pub contrast: f32,
    /// Everything below this goes to black.
    pub black: f32,
    /// Blacks lifted to this (a film's fade).
    pub fade: f32,
    /// Split toning: added in the shadows and in the highlights.
    pub shadows: [f32; 3],
    pub highlights: [f32; 3],
    /// Tint: a duotone through this colour.
    pub duotone: Option<[f32; 3]>,
    /// Film's tone (0 none, 1 strong): a toe that deepens the shadows
    /// toward black, and a shoulder that rolls the highlights into white.
    pub toe: f32,
    pub shoulder: f32,
    /// Film's colour response by hue range, on the colour alone.
    pub hues: Option<Hues>,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe {
            wb: [1.0; 3],
            mono: None,
            saturation: 1.0,
            exposure: 1.0,
            contrast: 0.0,
            black: 0.0,
            fade: 0.0,
            shadows: [0.0; 3],
            highlights: [0.0; 3],
            duotone: None,
            toe: 0.0,
            shoulder: 0.0,
            hues: None,
        }
    }
}

/// A range of hues a film treats apart: centred on `hue` (HSV degrees),
/// `width` each way in the colour plane, turned `shift` degrees (toward
/// the next hue round: red toward yellow, green toward cyan, cyan toward
/// blue) and its saturation times `sat`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub hue: f32,
    pub width: f32,
    pub shift: f32,
    pub sat: f32,
}

fn skin(shift: f32, sat: f32) -> Band {
    Band { hue: 25.0, width: 32.0, shift, sat }
}

fn foliage(shift: f32, sat: f32) -> Band {
    Band { hue: 100.0, width: 45.0, shift, sat }
}

fn sky(shift: f32, sat: f32) -> Band {
    Band { hue: 212.0, width: 34.0, shift, sat }
}

/// A film's colour response: its bands, then `roll`, how much the most
/// colourful are held back (a film's saturation rolls off where a
/// sensor's clips).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hues {
    pub bands: [Band; 3],
    pub roll: f32,
}

impl Hues {
    /// A colour's chroma (U and V, 0 to 1, grey at a half) as the film
    /// turns it.
    pub fn apply(&self, u: f32, v: f32) -> (f32, f32) {
        let (cu, cv) = (u - 0.5, v - 0.5);
        let c = (cu * cu + cv * cv).sqrt();
        if c < 1e-4 {
            return (u, v);
        }
        let a = cv.atan2(cu).to_degrees();
        let (mut shift, mut sat) = (0.0, 1.0);
        for b in &self.bands {
            let d = (a - band_angle(b.hue) + 540.0).rem_euclid(360.0) - 180.0;
            if d.abs() < b.width {
                let w = (std::f32::consts::FRAC_PI_2 * d / b.width).cos().powi(2);
                shift += w * b.shift;
                sat += w * (b.sat - 1.0);
            }
        }
        let c = c * sat.max(0.0) / (1.0 + self.roll * (c / 0.25).powi(2)).sqrt();
        let a = (a + shift).to_radians();
        ((0.5 + c * a.cos()).clamp(0.0, 1.0), (0.5 + c * a.sin()).clamp(0.0, 1.0))
    }
}

/// Where a hue (HSV degrees) lies in the colour (U, V) plane, in degrees.
fn band_angle(hue: f32) -> f32 {
    let [_, u, v] = rgb_to_yuv(hsv(hue, 0.6, 0.75));
    (v - 0.5).atan2(u - 0.5).to_degrees()
}

/// A film's response as a table from the frame's U and V (a byte each) to
/// the new ones, made once for each and kept: from a 65 × 65 grid of the
/// definition, filled between.
fn hue_table(hues: &Hues) -> std::sync::Arc<Vec<[u8; 2]>> {
    use std::sync::{Arc, Mutex, OnceLock};
    static MADE: OnceLock<Mutex<Vec<(Hues, Arc<Vec<[u8; 2]>>)>>> = OnceLock::new();
    let made = MADE.get_or_init(Mutex::default);
    if let Some(t) = made.lock().ok().and_then(|m| m.iter().find(|(h, _)| h == hues).map(|(_, t)| t.clone())) {
        return t;
    }
    const G: usize = 64;
    let grid: Vec<(f32, f32)> = (0..=G).flat_map(|i| (0..=G).map(move |j| (i, j))).map(|(i, j)| hues.apply(i as f32 / G as f32, j as f32 / G as f32)).collect();
    let mut table = vec![[0u8; 2]; 256 * 256];
    for u in 0..256usize {
        let fu = u as f32 / 255.0 * G as f32;
        let (i, ku) = ((fu as usize).min(G - 1), fu - (fu as usize).min(G - 1) as f32);
        for v in 0..256usize {
            let fv = v as f32 / 255.0 * G as f32;
            let (j, kv) = ((fv as usize).min(G - 1), fv - (fv as usize).min(G - 1) as f32);
            let at = |a: usize, b: usize| grid[a * (G + 1) + b];
            let lerp = |p: (f32, f32), q: (f32, f32), k: f32| (p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k);
            let o = lerp(lerp(at(i, j), at(i, j + 1), kv), lerp(at(i + 1, j), at(i + 1, j + 1), kv), ku);
            table[(u << 8) | v] = [(o.0 * 255.0).round().clamp(0.0, 255.0) as u8, (o.1 * 255.0).round().clamp(0.0, 255.0) as u8];
        }
    }
    let table = Arc::new(table);
    if let Ok(mut m) = made.lock() {
        m.push((*hues, table.clone()));
    }
    table
}

/// Where film's toe ends and its shoulder begins, on the curve.
const TOE_TOP: f32 = 0.4;
const KNEE: f32 = 0.55;

impl Recipe {
    /// The colour matrix: white balance and brightness, then saturation or
    /// the black and white mix.
    fn matrix(&self) -> [[f32; 3]; 3] {
        let k = self.wb.map(|w| w * self.exposure);
        let mut m = [[0.0; 3]; 3];
        for (row, out) in m.iter_mut().enumerate() {
            for (col, v) in out.iter_mut().enumerate() {
                *v = match self.mono {
                    Some(w) => w[col] * k[col],
                    None => {
                        let keep = if row == col { self.saturation } else { 0.0 };
                        (LUMA[col] * (1.0 - self.saturation) + keep) * k[col]
                    }
                };
            }
        }
        m
    }

    /// The tone curve on one channel (0 to 1 in and out).
    pub fn curve(&self, v: f32) -> f32 {
        let v = ((v - self.black) / (1.0 - self.black)).clamp(0.0, 1.0);
        // An S about mid grey through 0 and 1, steeper in the middle with
        // more contrast (a normalised arctangent: none is a straight line).
        let k = self.contrast * 6.0;
        let mut s = if k < 1e-3 { v } else { 0.5 + (k * (v - 0.5)).atan() / (2.0 * (k * 0.5).atan()) };
        // Film's toe: the shadows' slope eased from 1 - toe at black up to
        // 1 where it ends (a Hermite, below the line: deeper blacks).
        if self.toe > 0.0 && s < TOE_TOP {
            let x = s / TOE_TOP;
            s = TOE_TOP * (x - self.toe * x * (1.0 - x).powi(2));
        }
        // Its shoulder: the slope easing from 1 down to 1 - shoulder at
        // white, so highlights crowd softly into it rather than clip.
        if self.shoulder > 0.0 && s > KNEE {
            let x = (s - KNEE) / (1.0 - KNEE);
            s = KNEE + (1.0 - KNEE) * (x + self.shoulder * x * x * (1.0 - x));
        }
        self.fade + s * (1.0 - self.fade)
    }

    /// What the finish adds or makes of a colour of brightness `l`.
    fn finish(&self, c: [f32; 3], l: f32) -> [f32; 3] {
        if let Some(t) = self.duotone {
            return if l < 0.5 { t.map(|v| v * l * 2.0) } else { t.map(|v| v + (1.0 - v) * (l - 0.5) * 2.0) };
        }
        let (s, h) = ((1.0 - l).powi(2), l * l);
        [c[0] + self.shadows[0] * s + self.highlights[0] * h, c[1] + self.shadows[1] * s + self.highlights[1] * h, c[2] + self.shadows[2] * s + self.highlights[2] * h]
    }

    /// The look on one colour (0 to 1): the definition every fast form
    /// follows.
    pub fn map(&self, rgb: [f32; 3]) -> [f32; 3] {
        // Film's colour response first, on the colour alone.
        let rgb = match &self.hues {
            Some(h) => {
                let [y, u, v] = rgb_to_yuv(rgb);
                let (u, v) = h.apply(u, v);
                yuv_to_rgb(y, u, v)
            }
            None => rgb,
        };
        let m = self.matrix();
        let c = [dot(m[0], rgb), dot(m[1], rgb), dot(m[2], rgb)].map(|v| self.curve(v));
        let l = dot(c, LUMA).clamp(0.0, 1.0);
        self.finish(c, l).map(|v| v.clamp(0.0, 1.0))
    }

    /// The fast form for 8-bit RGB.
    pub fn compile(&self) -> Compiled {
        let m = self.matrix();
        let mut fixed = [[0i32; 3]; 3];
        for (f, row) in fixed.iter_mut().zip(m) {
            for (v, x) in f.iter_mut().zip(row) {
                // Q12, from 0..255 in to 0..1023 out.
                *v = (x * 4096.0 * 1023.0 / 255.0).round() as i32;
            }
        }
        let mut curve = [0u16; 1024];
        for (i, c) in curve.iter_mut().enumerate() {
            *c = (self.curve(i as f32 / 1023.0) * 1023.0).round() as u16;
        }
        let finish = match self.duotone {
            Some(_) => {
                let mut ramp = [[0u8; 3]; 256];
                for (l, out) in ramp.iter_mut().enumerate() {
                    let v = self.finish([0.0; 3], l as f32 / 255.0);
                    *out = v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
                Finish::Ramp(Box::new(ramp))
            }
            None if self.shadows == [0.0; 3] && self.highlights == [0.0; 3] => Finish::None,
            None => {
                let mut add = [[0i16; 3]; 256];
                for (l, out) in add.iter_mut().enumerate() {
                    let v = self.finish([0.0; 3], l as f32 / 255.0);
                    *out = v.map(|x| (x * 1023.0).round() as i16);
                }
                Finish::Split(Box::new(add))
            }
        };
        let identity = *self == Recipe::default();
        Compiled { matrix: fixed, curve: Box::new(curve), finish, hues: self.hues.as_ref().map(hue_table), identity }
    }
}

/// What follows the curve, by brightness.
#[derive(Clone)]
enum Finish {
    None,
    Split(Box<[[i16; 3]; 256]>),
    Ramp(Box<[[u8; 3]; 256]>),
}

/// A look made fast for 8-bit RGB.
#[derive(Clone)]
pub struct Compiled {
    matrix: [[i32; 3]; 3],
    curve: Box<[u16; 1024]>,
    finish: Finish,
    /// Film's colour response: the new U and V for each, by U then V.
    hues: Option<std::sync::Arc<Vec<[u8; 2]>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    identity: bool,
}

impl Compiled {
    #[cfg(test)]
    pub fn is_identity(&self) -> bool {
        self.identity
    }

    /// One pixel (the definition, as the tests check it; the photo editor
    /// lays looks on with it).
    #[inline]
    pub fn pixel(&self, r: u8, g: u8, b: u8) -> [u8; 3] {
        let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
        // Film's colour response: through its table by the colour's own U
        // and V (in sixteenths of a level and back, so nothing is lost).
        let (r, g, b) = match &self.hues {
            Some(t) => {
                let y = 19_595 * r + 38_470 * g + 7_471 * b;
                let u = (128 + ((-11_059 * r - 21_709 * g + 32_768 * b + 32_768) >> 16)).clamp(0, 255) as usize;
                let v = (128 + ((32_768 * r - 27_439 * g - 5_329 * b + 32_768) >> 16)).clamp(0, 255) as usize;
                let o = t[(u << 8) | v];
                let (du, dv) = (i32::from(o[0]) - 128, i32::from(o[1]) - 128);
                let to = |x: i32| ((x + 32_768) >> 16).clamp(0, 255);
                (to(y + 91_881 * dv), to(y - 22_554 * du - 46_802 * dv), to(y + 116_130 * du))
            }
            None => (r, g, b),
        };
        let m = &self.matrix;
        let c = |row: usize| {
            let v = (m[row][0] * r + m[row][1] * g + m[row][2] * b + 2048) >> 12;
            i32::from(self.curve[v.clamp(0, 1023) as usize])
        };
        let (cr, cg, cb) = (c(0), c(1), c(2));
        let l = ((306 * cr + 600 * cg + 118 * cb + 2048) >> 12).clamp(0, 255) as usize;
        let out = |v: i32| ((v * 255 + 511) / 1023).clamp(0, 255) as u8;
        match &self.finish {
            Finish::None => [out(cr), out(cg), out(cb)],
            Finish::Split(add) => {
                let a = add[l];
                [out(cr + i32::from(a[0])), out(cg + i32::from(a[1])), out(cb + i32::from(a[2]))]
            }
            Finish::Ramp(ramp) => ramp[l],
        }
    }

    /// The look over a picture's pixels, in place.
    #[cfg(test)]
    pub fn apply(&self, rgb: &mut [u8]) {
        if self.identity {
            return;
        }
        for p in rgb.chunks_exact_mut(3) {
            let out = self.pixel(p[0], p[1], p[2]);
            p.copy_from_slice(&out);
        }
    }

    /// A camera frame (YUV 4:2:0, full range, packed planes) into `out`
    /// with the look laid over it on the way: the colour conversion and the
    /// look's matrix are one (each 2 × 2 block's colour worked once, then
    /// three multiplies a pixel), the halves of the frame on two cores.
    /// With `zebras` (their phase), pixels clipping before the look (any
    /// channel at [`crate::pro::ZEBRA_AT`]) are striped. Gives how many clip.
    pub fn develop(&self, yuv: &[u8], w: usize, h: usize, out: &mut crate::photos::Picture, zebras: Option<usize>) -> u32 {
        self.develop_grained(yuv, w, h, out, zebras, None)
    }

    /// As [`Compiled::develop`], a film look's grain laid on each pixel as
    /// it is made (film.rs): in the same pass, on the same two cores.
    pub fn develop_grained(&self, yuv: &[u8], w: usize, h: usize, out: &mut crate::photos::Picture, zebras: Option<usize>, grain: Option<&film::Grain>) -> u32 {
        out.w = w;
        out.h = h;
        out.rgb.resize(w * h * 3, 0);
        if yuv.len() < w * h * 3 / 2 || w < 2 || h < 2 {
            return 0;
        }
        // Split on an even row, so each half has whole blocks.
        let half = (h / 4) * 2;
        if w * h >= 32_768 && half > 0 {
            let (top, bottom) = out.rgb.split_at_mut(half * w * 3);
            std::thread::scope(|s| {
                let first = s.spawn(|| self.develop_rows(yuv, w, h, 0..half, top, zebras, grain));
                let second = self.develop_rows(yuv, w, h, half..h, bottom, zebras, grain);
                first.join().unwrap_or(0) + second
            })
        } else {
            self.develop_rows(yuv, w, h, 0..h, &mut out.rgb, zebras, grain)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn develop_rows(&self, yuv: &[u8], w: usize, h: usize, rows: std::ops::Range<usize>, out: &mut [u8], zebras: Option<usize>, grain: Option<&film::Grain>) -> u32 {
        let cw = w / 2;
        let (ys, uv) = yuv.split_at(w * h);
        let (us, vs) = uv.split_at(cw * (h / 2));
        let m = &self.matrix;
        let s = [m[0][0] + m[0][1] + m[0][2], m[1][0] + m[1][1] + m[1][2], m[2][0] + m[2][1] + m[2][2]];
        let curve = &*self.curve;
        let at = |v: i32| i32::from(curve[v.clamp(0, 1023) as usize]);
        let to8 = |v: i32| ((v * 16_336 + 32_768) >> 16).clamp(0, 255) as u8;
        let mut clipped = 0u32;
        let first = rows.start;
        for y in rows {
            let lumas = &ys[y * w..(y + 1) * w];
            let (ur, vr) = (&us[(y / 2) * cw..(y / 2 + 1) * cw], &vs[(y / 2) * cw..(y / 2 + 1) * cw]);
            let line = &mut out[(y - first) * w * 3..(y - first + 1) * w * 3];
            for (bx, ((u, v), (pair, px))) in ur.iter().zip(vr).zip(lumas.chunks_exact(2).zip(line.chunks_exact_mut(6))).enumerate() {
                // Film's colour response: the block's colour through its table.
                let (u, v) = match &self.hues {
                    Some(t) => {
                        let o = t[(usize::from(*u) << 8) | usize::from(*v)];
                        (o[0], o[1])
                    }
                    None => (*u, *v),
                };
                let (u, v) = (i32::from(u) - 128, i32::from(v) - 128);
                // The block's colour, as yuv420_to_rgb works it, then through the matrix.
                let (cr, cg, cb) = ((359 * v) >> 8, -((88 * u + 183 * v) >> 8), (454 * u) >> 8);
                let k = [m[0][0] * cr + m[0][1] * cg + m[0][2] * cb + 2048, m[1][0] * cr + m[1][1] * cg + m[1][2] * cb + 2048, m[2][0] * cr + m[2][1] * cg + m[2][2] * cb + 2048];
                let reach = cr.max(cg).max(cb);
                for (i, (l, o)) in pair.iter().zip(px.chunks_exact_mut(3)).enumerate() {
                    let l = i32::from(*l);
                    let (r, g, b) = (at((s[0] * l + k[0]) >> 12), at((s[1] * l + k[1]) >> 12), at((s[2] * l + k[2]) >> 12));
                    match &self.finish {
                        Finish::None => o.copy_from_slice(&[to8(r), to8(g), to8(b)]),
                        Finish::Split(add) => {
                            let a = add[((306 * r + 600 * g + 118 * b + 2048) >> 12).clamp(0, 255) as usize];
                            o.copy_from_slice(&[to8(r + i32::from(a[0])), to8(g + i32::from(a[1])), to8(b + i32::from(a[2]))]);
                        }
                        Finish::Ramp(ramp) => o.copy_from_slice(&ramp[((306 * r + 600 * g + 118 * b + 2048) >> 12).clamp(0, 255) as usize]),
                    }
                    if let Some(grain) = grain {
                        grain.add(o, 2 * bx + i, y);
                    }
                    if let Some(phase) = zebras {
                        if l + reach >= i32::from(crate::pro::ZEBRA_AT) {
                            clipped += 1;
                            if crate::pro::zebra_stripe(2 * bx + i, y, phase) {
                                o.copy_from_slice(&[36, 36, 36]);
                            }
                        }
                    }
                }
            }
        }
        clipped
    }
}

// -------------------------------------------------------- the photo's LUT ---

/// The photo's table: this many steps each way of Y, U and V.
pub const LUT_N: usize = 33;

/// Full-range BT.601 (as the camera's YUV) to RGB, 0 to 1.
pub fn yuv_to_rgb(y: f32, u: f32, v: f32) -> [f32; 3] {
    let (u, v) = (u - 0.5, v - 0.5);
    [y + 1.402 * v, y - 0.344_136 * u - 0.714_136 * v, y + 1.772 * u].map(|c| c.clamp(0.0, 1.0))
}

pub fn rgb_to_yuv(c: [f32; 3]) -> [f32; 3] {
    let y = dot(c, LUMA);
    [y, 0.5 + (c[2] - y) / 1.772, 0.5 + (c[0] - y) / 1.402].map(|v| v.clamp(0.0, 1.0))
}

/// The look as a YUV → YUV table, Y slowest, then U, then V.
pub fn bake(recipe: &Recipe) -> Vec<u8> {
    let n = LUT_N;
    let step = 1.0 / (n - 1) as f32;
    let mut out = Vec::with_capacity(n * n * n * 3);
    for yi in 0..n {
        for ui in 0..n {
            for vi in 0..n {
                let rgb = yuv_to_rgb(yi as f32 * step, ui as f32 * step, vi as f32 * step);
                let yuv = rgb_to_yuv(recipe.map(rgb));
                out.extend(yuv.map(|v| (v * 255.0).round() as u8));
            }
        }
    }
    out
}

/// The table as the camera program reads it: "CLUT", its size, then the
/// entries.
pub fn lut_file(table: &[u8]) -> Vec<u8> {
    let mut out = b"CLUT".to_vec();
    out.push(LUT_N as u8);
    out.extend_from_slice(table);
    out
}

/// A table back from its file.
pub fn read_lut_file(bytes: &[u8]) -> Option<Vec<u8>> {
    let n = usize::from(*bytes.get(4)?);
    (bytes.starts_with(b"CLUT") && n == LUT_N && bytes.len() == 5 + n * n * n * 3).then(|| bytes[5..].to_vec())
}

/// One colour through the table (trilinear between its steps).
#[inline]
pub fn lut_at(table: &[u8], y: u8, u: u8, v: u8) -> [u8; 3] {
    let n = LUT_N;
    // Each 8-bit value as a step and a fraction in 1/256s: 255 is the last step.
    let pos = |x: u8| {
        let f = u32::from(x) * (n as u32 - 1) * 256 / 255;
        ((f >> 8).min(n as u32 - 2) as usize, f - ((f >> 8).min(n as u32 - 2) << 8))
    };
    let ((y0, fy), (u0, fu), (v0, fv)) = (pos(y), pos(u), pos(v));
    let at = |a: usize, b: usize, c: usize| ((a * n + b) * n + c) * 3;
    let mut out = [0u8; 3];
    for (ch, o) in out.iter_mut().enumerate() {
        let p = |a: usize, b: usize, c: usize| table[at(a, b, c) + ch] as u32;
        let lerp = |a: u32, b: u32, f: u32| a * (256 - f) + b * f;
        let c00 = lerp(p(y0, u0, v0), p(y0, u0, v0 + 1), fv);
        let c01 = lerp(p(y0, u0 + 1, v0), p(y0, u0 + 1, v0 + 1), fv);
        let c10 = lerp(p(y0 + 1, u0, v0), p(y0 + 1, u0, v0 + 1), fv);
        let c11 = lerp(p(y0 + 1, u0 + 1, v0), p(y0 + 1, u0 + 1, v0 + 1), fv);
        let c0 = (c00 * (256 - fu) + c01 * fu) >> 8;
        let c1 = (c10 * (256 - fu) + c11 * fu) >> 8;
        *o = ((c0 * (256 - fy) + c1 * fy + (1 << 15)) >> 16).min(255) as u8;
    }
    out
}

/// The table over a YUV 4:2:0 picture (packed planes), in place: each pixel
/// through it with its block's colour, the block's new colour the mean of
/// its four. catalyst-camera's `apply_lut` does the same to each photo.
pub fn apply_yuv420(table: &[u8], yuv: &mut [u8], w: usize, h: usize) {
    let (cw, ch) = (w / 2, h / 2);
    let (ys, uv) = yuv.split_at_mut(w * h);
    let (us, vs) = uv.split_at_mut(cw * ch);
    for by in 0..ch {
        for bx in 0..cw {
            let c = by * cw + bx;
            let (u, v) = (us[c], vs[c]);
            let (mut su, mut sv) = (0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = (2 * by + dy) * w + 2 * bx + dx;
                let o = lut_at(table, ys[at], u, v);
                ys[at] = o[0];
                su += u32::from(o[1]);
                sv += u32::from(o[2]);
            }
            us[c] = ((su + 2) / 4) as u8;
            vs[c] = ((sv + 2) / 4) as u8;
        }
    }
}

/// Where looks' tables are kept for the camera program to read: made once
/// each, small (107 KB), in the system's temporary files.
pub fn lut_path(look: Look, hue: f32) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("catalyst-looks");
    let name = if look == Look::Tint { format!("tint-{:03}.lut", hue.rem_euclid(360.0).round() as u32) } else { format!("{}.lut", look.id()) };
    dir.join(name)
}

/// The look's table written for the camera program (once; kept after).
pub fn ensure_lut(look: Look, hue: f32) -> Option<std::path::PathBuf> {
    let path = lut_path(look, hue);
    if !path.exists() {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let bytes = lut_file(&bake(&look.recipe(hue)));
        crate::sys::write_atomic(&path, &bytes).ok()?;
    }
    Some(path)
}

/// A photo's look, from its Exif's "look: …" (a pixel picture's PNG says
/// it in a text chunk of that name).
pub fn look_of(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        let reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).read_info().ok()?;
        return reader.info().uncompressed_latin1_text.iter().find(|t| t.keyword == "look").map(|t| t.text.clone());
    }
    let mut head = vec![0u8; 16 * 1024];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    look_in(&head[..n])
}

/// The "look: …" written in a photo's first bytes.
pub fn look_in(bytes: &[u8]) -> Option<String> {
    let mark = b"ASCII\0\0\0look: ";
    let at = bytes.windows(mark.len()).position(|w| w == mark)? + mark.len();
    // ASCII, as UserComment says: it ends at its count, a NUL, or anything else.
    let end = bytes[at..].iter().position(|b| !(0x20..0x7f).contains(b)).map_or(bytes.len(), |e| at + e);
    std::str::from_utf8(&bytes[at..end]).ok().map(str::to_owned).filter(|s| !s.is_empty() && s.len() < 40)
}

/// A small Exif (APP1's contents) saying the look, as catalyst-camera
/// writes it: the simulator's photos wear it.
pub fn exif_with_look(name: &str) -> Vec<u8> {
    let mut comment = b"ASCII\0\0\0look: ".to_vec();
    comment.extend_from_slice(name.as_bytes());
    comment.push(0);
    let mut out = b"Exif\0\0II*\0\x08\0\0\0".to_vec();
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    // The first directory: one entry, pointing at the Exif one (at 26).
    out.extend(le16(1));
    out.extend(le16(0x8769));
    out.extend(le16(4));
    out.extend(le32(1));
    out.extend(le32(26));
    out.extend(le32(0));
    // The Exif directory: its UserComment (at 44).
    out.extend(le16(1));
    out.extend(le16(0x9286));
    out.extend(le16(7));
    out.extend(le32(comment.len() as u32));
    out.extend(le32(44));
    out.extend(le32(0));
    out.extend(comment);
    out
}

/// Where the photo as taken is kept when a look is laid over it.
pub fn original_of(photo: &std::path::Path) -> std::path::PathBuf {
    let dir = photo.parent().map(|d| d.join(".originals")).unwrap_or_else(|| std::path::PathBuf::from(".originals"));
    dir.join(photo.file_name().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb8(c: [f32; 3]) -> [u8; 3] {
        c.map(|v| (v * 255.0).round() as u8)
    }

    fn mapped(look: Look, c: [u8; 3]) -> [u8; 3] {
        look.compiled(30.0).pixel(c[0], c[1], c[2])
    }

    #[test]
    fn original_changes_nothing() {
        let c = Look::Original.compiled(0.0);
        assert!(c.is_identity());
        for v in [0u8, 1, 64, 128, 200, 254, 255] {
            assert_eq!(c.pixel(v, 255 - v, v / 2), [v, 255 - v, v / 2]);
        }
        let mut px = vec![10, 20, 30, 250, 128, 0];
        c.apply(&mut px);
        assert_eq!(px, vec![10, 20, 30, 250, 128, 0]);
    }

    #[test]
    fn black_and_white_looks_are_neutral_whatever_comes_in() {
        for look in LOOKS.into_iter().filter(|l| l.neutral()) {
            let c = look.compiled(0.0);
            for colour in [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [212, 40, 52], [90, 160, 230], [128, 128, 128]] {
                let [r, g, b] = c.pixel(colour[0], colour[1], colour[2]);
                assert!(r == g && g == b, "{}: {colour:?} → {:?}", look.name(), [r, g, b]);
            }
            // Black stays black (Silvertone and BW Natural lift theirs a
            // little), white stays white.
            assert!(c.pixel(0, 0, 0)[0] <= 12, "{}", look.name());
            assert!(c.pixel(255, 255, 255)[0] >= 250, "{}", look.name());
        }
        // A red filter's mix: red lighter than in Mono, blue darker.
        assert!(mapped(Look::BwHighContrast, [200, 40, 40])[0] > mapped(Look::Mono, [200, 40, 40])[0]);
        assert!(mapped(Look::BwHighContrast, [40, 60, 200])[0] < mapped(Look::Mono, [40, 60, 200])[0]);
        // Noir crushes the shadows.
        assert_eq!(mapped(Look::Noir, [16, 16, 16]), [0, 0, 0]);
    }

    #[test]
    fn colour_looks_map_known_colours_as_designed() {
        let grey = [128u8, 128, 128];
        // Vivid and Dramatic keep grey grey; their warm and cool kinds tint it.
        for look in [Look::Vivid, Look::Dramatic] {
            let [r, g, b] = mapped(look, grey);
            assert!(r == g && g == b, "{}: {:?}", look.name(), [r, g, b]);
        }
        for look in [Look::VividWarm, Look::DramaticWarm] {
            let [r, g, b] = mapped(look, grey);
            assert!(r > g && g > b, "{} warms grey: {:?}", look.name(), [r, g, b]);
        }
        for look in [Look::VividCool, Look::DramaticCool] {
            let [r, g, b] = mapped(look, grey);
            assert!(b > g && g >= r, "{} cools grey: {:?}", look.name(), [r, g, b]);
        }
        // Vivid: a muted red further from grey; Classic: nearer it.
        let red = [180u8, 90, 80];
        let spread = |c: [u8; 3]| i32::from(c[0]) - i32::from(c[1]);
        assert!(spread(mapped(Look::Vivid, red)) > spread(red) + 15);
        assert!(spread(mapped(Look::Classic, red)) < spread(red));
        // Dramatic: shadows deeper and more between dark and light grey.
        let (dark, light) = (mapped(Look::Dramatic, [64, 64, 64])[0], mapped(Look::Dramatic, [192, 192, 192])[0]);
        assert!(dark < 64 && i32::from(light) - i32::from(dark) > 128, "{dark} {light}");
        // Classic lifts its blacks (a film's fade) and its shadows lean cool.
        let black = mapped(Look::Classic, [0, 0, 0]);
        assert!(black[2] > 10 && black[2] > black[0], "{black:?}");
        // Tint: a duotone through its colour, black and white kept.
        let warm = Look::Tint.compiled(30.0);
        let [r, g, b] = warm.pixel(128, 128, 128);
        assert!(r > g && g > b, "orange: {:?}", [r, g, b]);
        assert_eq!(warm.pixel(0, 0, 0), [0, 0, 0]);
        assert_eq!(warm.pixel(255, 255, 255), [255, 255, 255]);
        let [r, g, b] = Look::Tint.compiled(210.0).pixel(128, 128, 128);
        assert!(b > g && g > r, "blue: {:?}", [r, g, b]);
        // Its colour has a brightness of a half, whatever the hue.
        for hue in [0.0, 60.0, 120.0, 240.0, 300.0] {
            assert!((dot(tint_colour(hue), LUMA) - 0.5).abs() < 0.01, "{hue}");
        }
        assert_eq!(Look::from_id("bw-high-contrast"), Some(Look::BwHighContrast));
        assert_eq!(LOOKS.len(), 22);
        assert_eq!(LOOKS.iter().filter(|l| l.film()).count(), FILM.len());
    }

    #[test]
    fn the_fast_forms_follow_the_definition() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 24) as u8
        };
        for look in LOOKS {
            let recipe = look.recipe(40.0);
            let fast = recipe.compile();
            let table = bake(&recipe);
            let mut worst = (0, 0);
            for _ in 0..400 {
                let c = [next(), next(), next()];
                let want = rgb8(recipe.map(c.map(|v| f32::from(v) / 255.0)));
                let got = fast.pixel(c[0], c[1], c[2]);
                for i in 0..3 {
                    worst.0 = worst.0.max((i32::from(want[i]) - i32::from(got[i])).abs());
                }
                // Through the photo's table, from the colour's own YUV.
                let yuv = rgb_to_yuv(c.map(|v| f32::from(v) / 255.0)).map(|v| (v * 255.0).round() as u8);
                let want = rgb_to_yuv(recipe.map(yuv_to_rgb(f32::from(yuv[0]) / 255.0, f32::from(yuv[1]) / 255.0, f32::from(yuv[2]) / 255.0))).map(|v| (v * 255.0).round() as i32);
                let got = lut_at(&table, yuv[0], yuv[1], yuv[2]);
                for i in 0..3 {
                    worst.1 = worst.1.max((want[i] - i32::from(got[i])).abs());
                }
            }
            // A film look's colour response goes through a table of the
            // chroma (a level each way), then the curve's steeper slope.
            let allowed = if look.film() { 4 } else { 2 };
            assert!(worst.0 <= allowed, "{}: the preview's form is {} off", look.name(), worst.0);
            assert!(worst.1 <= 4, "{}: the photo's table is {} off", look.name(), worst.1);
        }
    }

    #[test]
    fn the_photos_table_goes_over_yuv_and_through_its_file() {
        // Original's table leaves a picture as it is (within a level): a
        // picture of real colours, as a camera gives (each 2 × 2 block one).
        let table = bake(&Look::Original.recipe(0.0));
        let (w, h) = (8, 4);
        let mut pic = crate::photos::Picture::new(w, h);
        for (i, p) in pic.rgb.chunks_exact_mut(3).enumerate() {
            let block = (i % w) / 2 + (i / w / 2) * 7;
            p.copy_from_slice(&[(block * 53 % 256) as u8, (block * 97 % 256) as u8, (block * 151 % 256) as u8]);
        }
        let mut yuv = crate::camera::rgb_to_yuv420(&pic);
        let before = yuv.clone();
        apply_yuv420(&table, &mut yuv, w, h);
        for (a, b) in yuv.iter().zip(&before) {
            assert!((i32::from(*a) - i32::from(*b)).abs() <= 1, "{a} {b}");
        }
        // Mono's takes every colour to grey: U and V at the middle.
        let mono = bake(&Look::Mono.recipe(0.0));
        apply_yuv420(&mono, &mut yuv, w, h);
        assert!(yuv[w * h..].iter().all(|c| (i32::from(*c) - 128).abs() <= 1), "{:?}", &yuv[w * h..]);
        let file = lut_file(&mono);
        assert_eq!(&file[..5], b"CLUT\x21");
        assert_eq!(read_lut_file(&file).as_deref(), Some(mono.as_slice()));
        assert!(read_lut_file(&file[..100]).is_none());
        // The look's name read back from a photo's Exif.
        let exif = b"Exif\0\0junk ASCII\0\0\0look: Vivid Warm\0more";
        assert_eq!(look_in(exif).as_deref(), Some("Vivid Warm"));
        let dir = std::env::temp_dir().join(format!("catalyst-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("IMG_1.jpg");
        crate::burst::write_jpeg(&photo, &vec![128u8; 16 * 16 * 3 / 2], 16, 16, Some(exif_with_look("Noir"))).unwrap();
        assert_eq!(look_of(&photo).as_deref(), Some("Noir"), "{:?}", &std::fs::read(&photo).unwrap()[..64]);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(original_of(std::path::Path::new("/data/photos/IMG_1.jpg")), std::path::PathBuf::from("/data/photos/.originals/IMG_1.jpg"));
    }

    #[test]
    fn a_preview_frame_takes_well_under_six_milliseconds() {
        // 320 × 240 through the busiest look, timed here: the handheld's A53
        // at 1 GHz is reckoned at about fifteen times slower than this
        // computer's core (the shell's own pacing estimate for its frames).
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
        let mut out = crate::photos::Picture::default();
        let mut worst = 0.0f64;
        for look in [Look::Original, Look::Classic, Look::Tint, Look::Vivid] {
            let c = look.compiled(30.0);
            let both = time(&mut || {
                c.develop(&yuv, 320, 240, &mut out, Some(0));
            });
            worst = worst.max(both);
            // The same colours as converting and then laying the look over.
            let mut want = plain.clone();
            c.apply(&mut want.rgb);
            let mut fresh = crate::photos::Picture::default();
            c.develop(&yuv, 320, 240, &mut fresh, None);
            let off = want.rgb.iter().zip(&fresh.rgb).map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max().unwrap_or(0);
            assert!(off <= 3, "{}: {off} off", look.name());
        }
        eprintln!("a 320 × 240 preview here: converted {convert:.3} ms; converted with a look and zebras {worst:.3} ms (two cores)");
        // Against the plain conversion timed alongside, so a machine busy
        // with other work (a whole test suite, builds) slows both alike.
        assert!(worst < (convert * 4.0).max(2.0), "{worst} ms, against {convert} ms converted alone");
        // A film look, its grain and halation strong, on a dusk with lamps
        // (the halation's busiest): its colour response, then its texture.
        let mut dusk = crate::camera::scene(320, 240, 1.0, false);
        for (i, p) in dusk.rgb.chunks_exact_mut(3).enumerate() {
            let (x, y) = (i % 320, i / 320);
            let lamp = (x as i32 - 80).pow(2) + (y as i32 - 60).pow(2) < 49 || (x as i32 - 240).pow(2) + (y as i32 - 90).pow(2) < 25;
            p.copy_from_slice(&if lamp { [255, 244, 214] } else { [p[0] / 3, p[1] / 3, p[2] / 3] });
        }
        let dusk = crate::camera::rgb_to_yuv420(&dusk);
        let mut film_worst = (0.0f64, "");
        for look in FILM {
            let c = look.compiled(0.0);
            let tex = look.texture().at(film::Strength::Strong);
            // As the camera's thread does it: the grain in the developing
            // pass, then the glow.
            let took = time(&mut || {
                let grain = film::Grain::preview(tex, 320);
                c.develop_grained(&dusk, 320, 240, &mut out, None, grain.as_ref());
                film::halate(&mut out, tex);
            });
            // The same as developing, then laying the texture on whole.
            let mut whole = crate::photos::Picture::default();
            c.develop(&dusk, 320, 240, &mut whole, None);
            apply_texture_for_test(&mut whole, tex);
            assert_eq!(whole, out, "{}", look.name());
            if took > film_worst.0 {
                film_worst = (took, look.name());
            }
        }
        let tungsten = Look::Tungsten.compiled(0.0);
        let colour = time(&mut || {
            tungsten.develop(&dusk, 320, 240, &mut out, None);
        });
        let mut textured = out.clone();
        let texture = time(&mut || {
            let tex = Look::Tungsten.texture().at(film::Strength::Strong);
            let grain = film::Grain::preview(tex, 320);
            tungsten.develop_grained(&dusk, 320, 240, &mut textured, None, grain.as_ref());
            film::halate(&mut textured, tex);
        }) - colour;
        eprintln!(
            "a film look with strong grain and halation: {:.3} ms at worst ({}; tungsten's colour {colour:.3} ms, its texture {texture:.3} ms); at the A53's reckoned fifteenth, {:.1} ms",
            film_worst.0,
            film_worst.1,
            film_worst.0 * 15.0
        );
        assert!(film_worst.0 < (convert * 4.5).max(2.0), "{} ms ({}), against {convert} ms converted alone", film_worst.0, film_worst.1);
    }

    fn apply_texture_for_test(pic: &mut crate::photos::Picture, tex: film::Texture) {
        film::apply(pic, tex, 0);
    }

    fn chroma(c: [f32; 3]) -> f32 {
        let [_, u, v] = rgb_to_yuv(c);
        ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt()
    }

    #[test]
    fn film_looks_have_a_films_tone() {
        // The toe deepens the shadows, black kept; the shoulder rolls the
        // highlights into white, white kept; both rise all the way.
        let toe = Recipe { toe: 0.6, ..Recipe::default() };
        let shoulder = Recipe { shoulder: 0.7, ..Recipe::default() };
        assert!(toe.curve(0.12) < 0.1 && toe.curve(0.0) == 0.0 && (toe.curve(0.7) - 0.7).abs() < 1e-6);
        assert!(shoulder.curve(0.85) > 0.88 && (shoulder.curve(1.0) - 1.0).abs() < 1e-6 && (shoulder.curve(0.3) - 0.3).abs() < 1e-6);
        // Near white the shoulder's slope is gentle: highlights roll off.
        assert!(shoulder.curve(1.0) - shoulder.curve(0.95) < 0.05 * 0.5);
        for r in [toe, shoulder, Look::Sunlit.recipe(0.0), Look::Pushed.recipe(0.0), Look::Pearl.recipe(0.0), Look::Cine.recipe(0.0)] {
            let mut last = -1.0;
            for i in 0..=200 {
                let v = r.curve(i as f32 / 200.0);
                assert!(v >= last - 1e-6, "{r:?} falls at {i}");
                last = v;
            }
        }
        // The slide's blacks are deep; the portrait's lifted; Pushed is harder than Pearl.
        assert!(mapped(Look::Sunlit, [30, 30, 30])[1] < 22);
        assert!(mapped(Look::Portrait, [0, 0, 0])[1] > 6);
        let spread = |l: Look| i32::from(mapped(l, [200, 200, 200])[0]) - i32::from(mapped(l, [60, 60, 60])[0]);
        assert!(spread(Look::Pushed) > spread(Look::Pearl) + 40, "{} {}", spread(Look::Pushed), spread(Look::Pearl));
    }

    #[test]
    fn film_looks_treat_skin_sky_and_foliage_apart() {
        let f = |c: [u8; 3]| c.map(|v| f32::from(v) / 255.0);
        let (skin, sky, leaf) = (f([214, 160, 130]), f([110, 160, 220]), f([90, 140, 60]));
        let gain = |look: Look, c: [f32; 3]| chroma(look.recipe(0.0).map(c)) / chroma(c);
        // The slide deepens the sky more than it warms the skin.
        assert!(gain(Look::Sunlit, sky) > gain(Look::Sunlit, skin) + 0.15, "{} {}", gain(Look::Sunlit, sky), gain(Look::Sunlit, skin));
        // The portrait keeps skin rich while it quietens foliage and sky.
        assert!(gain(Look::Portrait, skin) > gain(Look::Portrait, leaf) + 0.1 && gain(Look::Portrait, skin) > gain(Look::Portrait, sky));
        // The faded slide drains greens most.
        assert!(gain(Look::Faded, leaf) < gain(Look::Faded, skin));
        // A band turns only its own hues: grey and a magenta (in none) keep
        // their hue through the colour response.
        let h = Look::Cine.recipe(0.0).hues.unwrap();
        assert_eq!(h.apply(0.5, 0.5), (0.5, 0.5));
        let [_, u, v] = rgb_to_yuv(f([200, 60, 200]));
        let (u2, v2) = h.apply(u, v);
        assert!(((v2 - 0.5).atan2(u2 - 0.5) - (v - 0.5).atan2(u - 0.5)).abs() < 1e-3);
        // Cine: shadows lean teal, highlights warm. Tungsten: day goes cool.
        let dark = mapped(Look::Cine, [50, 50, 50]);
        let light = mapped(Look::Cine, [205, 205, 205]);
        assert!(dark[2] > dark[0] && light[0] > light[2], "{dark:?} {light:?}");
        let grey = mapped(Look::Tungsten, [128, 128, 128]);
        assert!(grey[2] > grey[0] + 10, "{grey:?}");
        // Texture: the night stock glows, the pushed stock is grainiest,
        // the digital looks have none.
        assert!(Look::Tungsten.texture().halation > 0.4);
        assert!(FILM.iter().all(|l| l.texture().grain <= Look::Pushed.texture().grain && l.texture().grain > 0.0));
        assert!(LOOKS.iter().filter(|l| !l.film()).all(|l| l.texture().is_none()));
        // Our own names: no film's.
        for look in FILM {
            assert!(look.id().starts_with("film-"));
            let name = look.name().to_lowercase();
            for stock in ["velvia", "portra", "chrome", "tri-x", "ektar", "cinestill", "provia", "kodak", "fuji", "ilford", "agfa", "superia", "gold"] {
                assert!(!name.contains(stock), "{name}");
            }
        }
    }
}
