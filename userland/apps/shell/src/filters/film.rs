//! Film's texture (1.2): grain and halation, laid over a picture after its
//! look (filters.rs), at the strength chosen in the looks' strip: off,
//! light or strong (remembered with the Camera's settings).
//!
//! - **Grain** is noise of one size relative to the picture, about an
//!   800th of its width, so a photo and the preview of it carry the same
//!   grain at the same size on the panel: on the 3280-wide photo, smooth
//!   specks about four pixels across; in the 320-wide preview, finer than a
//!   pixel, so each pixel carries the mean of several (fainter, as it would
//!   look shrunk). It is luminance-weighted (strongest in the middle tones,
//!   faint in black and white), the same on each channel (a film's grain is
//!   in its brightness), and fixed by a seed: one for the preview, one for
//!   each photo from its name, so a still never shimmers.
//! - **Halation** is the red-orange glow film gets round bright lights,
//!   where light passes through the emulsion and back: the highlights
//!   (above 72% brightness) gathered at a 160th of the picture's width,
//!   blurred about a 70th of the width, and screened back over the picture
//!   in red-orange. Both sizes are the picture's, so the preview's glow is
//!   the photo's, shrunk.
//!
//! The camera program lays the look's table over the full-size photo; the
//! texture is laid on after it is saved, by the shell, at idle priority
//! ([`finish_photo`]), the thumbnail wearing its "refining" badge until it
//! is done. The photo as taken (`.originals`) stays clean.

use std::path::Path;

use crate::photos::Picture;

/// How strong the texture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Strength {
    Off,
    #[default]
    Light,
    Strong,
}

pub const STRENGTHS: [Strength; 3] = [Strength::Off, Strength::Light, Strength::Strong];

impl Strength {
    pub fn name(self) -> &'static str {
        match self {
            Strength::Off => "off",
            Strength::Light => "light",
            Strength::Strong => "strong",
        }
    }

    pub fn from_id(id: &str) -> Option<Strength> {
        STRENGTHS.iter().copied().find(|s| s.name() == id)
    }

    fn factor(self) -> f32 {
        match self {
            Strength::Off => 0.0,
            Strength::Light => 0.5,
            Strength::Strong => 1.0,
        }
    }

    /// The next way (`by` 1) or back.
    pub fn step(self, by: i32) -> Strength {
        let at = STRENGTHS.iter().position(|s| *s == self).unwrap_or(1) as i32;
        STRENGTHS[(at + by).clamp(0, 2) as usize]
    }
}

/// A look's texture at full strength: grain's deviation (a fraction of
/// white, in the middle tones, at the picture's own grain size) and the
/// halation's glow.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Texture {
    pub grain: f32,
    pub halation: f32,
}

impl Texture {
    pub fn at(self, strength: Strength) -> Texture {
        let k = strength.factor();
        Texture { grain: self.grain * k, halation: self.halation * k }
    }

    pub fn is_none(self) -> bool {
        self.grain <= 0.0 && self.halation <= 0.0
    }

    /// What it says in a photo's look ("strong grain", "light grain and glow").
    pub fn words(self, strength: Strength) -> Option<String> {
        if self.is_none() || strength == Strength::Off {
            return None;
        }
        let what = match (self.grain > 0.0, self.halation > 0.0) {
            (true, true) => "grain and glow",
            (false, true) => "glow",
            _ => "grain",
        };
        Some(format!("{} {what}", strength.name()))
    }
}

/// Grain's size: an 800th of the picture's width.
const GRAIN_PER_WIDTH: f32 = 800.0;

/// A seed's noise at a lattice point: roughly normal, deviation 1.
#[inline]
fn noise(seed: u32, x: u32, y: u32) -> f32 {
    let mut h = seed ^ x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    // Two bytes' sum and difference: a triangle, near enough to a bell.
    let (a, b) = ((h & 0xFFFF) as f32 / 65535.0, (h >> 16) as f32 / 65535.0);
    (a + b - 1.0) * 2.449
}

/// The preview's grain, worked once: a 128 × 128 tile of the noise at a
/// pixel's size (seed 0), in 40ths of its deviation.
fn preview_tile() -> &'static [i8] {
    static TILE: std::sync::OnceLock<Vec<i8>> = std::sync::OnceLock::new();
    TILE.get_or_init(|| (0..128 * 128).map(|i| (noise(0, (i % 128) as u32, (i / 128) as u32) * 40.0).round().clamp(-127.0, 127.0) as i8).collect())
}

/// The grain's deviation by brightness (a byte), as `Grain` and the photo
/// path use it: luminance-weighted, most in the middle tones, for noise in
/// 40ths of its deviation, in 2^14ths of a level.
fn grain_weights(sigma: f32) -> [i32; 256] {
    std::array::from_fn(|l| {
        let l = l as f32 / 255.0;
        ((0.2 + 3.2 * l * (1.0 - l)) * sigma * 16_384.0 / 40.0).round() as i32
    })
}

/// The deviation of grain specks `tex.grain` strong, seen at a pixel of a
/// picture `w` wide: under a speck a pixel, it carries their mean.
fn grain_sigma(tex: Texture, w: usize) -> f32 {
    tex.grain * 255.0 * (w as f32 / GRAIN_PER_WIDTH).min(1.0)
}

/// The preview's grain, laid on as each pixel is developed (filters.rs's
/// `develop_grained`, on its two cores): the fixed tile, by brightness.
pub struct Grain {
    weights: [i32; 256],
    tile: &'static [i8],
}

impl Grain {
    /// For a picture `w` wide whose grain is finer than a pixel (a
    /// preview's); none for one with no grain, or larger.
    pub fn preview(tex: Texture, w: usize) -> Option<Grain> {
        (tex.grain > 0.0 && w as f32 <= GRAIN_PER_WIDTH).then(|| Grain { weights: grain_weights(grain_sigma(tex, w)), tile: preview_tile() })
    }

    /// Onto a pixel (three bytes) at (`x`, `y`).
    #[inline]
    pub fn add(&self, px: &mut [u8], x: usize, y: usize) {
        let l = ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8) as usize;
        let n = i32::from(self.tile[(y & 127) * 128 + (x & 127)]);
        let d = (n * self.weights[l & 255] + 8192) >> 14;
        for v in px.iter_mut().take(3) {
            *v = (i32::from(*v) + d).clamp(0, 255) as u8;
        }
    }
}

/// The texture over a picture, in place: grain, then the halation's glow
/// over it (as the preview lays them on).
pub fn apply(pic: &mut Picture, tex: Texture, seed: u32) {
    if pic.is_empty() || tex.is_none() {
        return;
    }
    let (w, h) = (pic.w, pic.h);
    match Grain::preview(tex, w).filter(|_| seed == 0) {
        // The preview's own: its tile.
        Some(grain) => {
            for (i, px) in pic.rgb.chunks_exact_mut(3).enumerate() {
                grain.add(px, i % w, i / w);
            }
        }
        None if tex.grain > 0.0 => {
            let speck = w as f32 / GRAIN_PER_WIDTH;
            let weights = grain_weights(grain_sigma(tex, w));
            let rows = |from: usize, out: &mut [u8]| {
                let mut noise_row: Vec<i32> = Vec::with_capacity(w);
                for (j, line) in out.chunks_exact_mut(w * 3).enumerate() {
                    let y = from + j;
                    // The noise along the row, in 40ths of its deviation.
                    noise_row.clear();
                    if speck <= 1.0 {
                        noise_row.extend((0..w).map(|x| (noise(seed, x as u32, y as u32) * 40.0) as i32));
                    } else {
                        noise_row.extend((0..w).map(|x| (smooth_noise(seed, x as f32 / speck, y as f32 / speck) * 40.0) as i32));
                    }
                    for (px, n) in line.chunks_exact_mut(3).zip(&noise_row) {
                        let l = ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8) as usize;
                        let d = (n * weights[l & 255] + 8192) >> 14;
                        for v in px.iter_mut() {
                            *v = (i32::from(*v) + d).clamp(0, 255) as u8;
                        }
                    }
                }
            };
            // A photo's halves on two cores.
            let half = h / 2;
            if w * h >= 32_768 && half > 0 {
                let (top, bottom) = pic.rgb.split_at_mut(half * w * 3);
                std::thread::scope(|s| {
                    s.spawn(|| rows(0, top));
                    rows(half, bottom);
                });
            } else {
                rows(0, &mut pic.rgb);
            }
        }
        None => {}
    }
    halate(pic, tex);
}

/// The halation's glow over a picture, in place (the rows near a light
/// only).
pub fn halate(pic: &mut Picture, tex: Texture) {
    if pic.is_empty() || tex.halation <= 0.0 {
        return;
    }
    let glow = Glow::of(pic, tex.halation);
    if glow.row_max.iter().all(|m| *m == 0) {
        return;
    }
    let w = pic.w;
    let columns: Vec<(u16, u16, u8)> = (0..w).map(|x| glow.column(x)).collect();
    let mut along = Vec::new();
    for (y, line) in pic.rgb.chunks_exact_mut(w * 3).enumerate() {
        if !glow.row(y, &mut along) {
            continue;
        }
        for (px, &(a, b, k)) in line.chunks_exact_mut(3).zip(&columns) {
            let (a, b) = (along[usize::from(a)], along[usize::from(b)]);
            // In 4096ths, screened in red-orange: never past white.
            let g = a + (((b - a) * i32::from(k)) >> 8);
            if g > 0 {
                for (v, tint) in px.iter_mut().zip([4096, 1475, 573]) {
                    let add = ((255 - i32::from(*v)) * ((g * tint) >> 12).min(4096)) >> 12;
                    *v = (i32::from(*v) + add).min(255) as u8;
                }
            }
        }
    }
}
/// Noise between lattice points (bilinear, smoothed), deviation about 1:
/// specks a lattice step across.
#[inline]
fn smooth_noise(seed: u32, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (x0 as u32, y0 as u32);
    let a = noise(seed, xi, yi) + (noise(seed, xi + 1, yi) - noise(seed, xi, yi)) * sx;
    let b = noise(seed, xi, yi + 1) + (noise(seed, xi + 1, yi + 1) - noise(seed, xi, yi + 1)) * sx;
    // Interpolating loses some deviation; given back.
    (a + (b - a) * sy) * 1.45
}

/// The halation's glow: the highlights gathered into cells an 80th of the
/// picture's width (every pixel counted, so a small lamp in a photo glows
/// as it does in the preview), blurred, and read back between its cells,
/// in 4096ths.
struct Glow {
    cells: Vec<i32>,
    cw: usize,
    ch: usize,
    step: usize,
    /// Each row of cells' brightest.
    row_max: Vec<i32>,
}

impl Glow {
    fn of(pic: &Picture, amount: f32) -> Glow {
        let (w, h) = (pic.w, pic.h);
        let step = (w as f32 / 80.0).round().max(1.0) as usize;
        let (cw, ch) = (w.div_ceil(step), h.div_ceil(step));
        // How far over 72% each pixel's brightness is, summed by cell
        // (a cell's run of a row at a time).
        // A preview's every other pixel each way (a photo's every one, so
        // a small lamp in it is never missed).
        let skip = if w <= 1024 && step >= 2 { 2 } else { 1 };
        let mut sums = vec![0u32; cw * ch];
        for (y, line) in pic.rgb.chunks_exact(w * 3).enumerate().step_by(skip) {
            let cells = &mut sums[(y / step) * cw..][..cw];
            for (cell, run) in cells.iter_mut().zip(line.chunks(step * 3)) {
                *cell += run.chunks_exact(3).step_by(skip).map(|p| ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2])) >> 8).saturating_sub(184)).sum::<u32>();
            }
        }
        // As a fraction of a full cell at white (71 levels over), then
        // about a 70th of the width, as three box passes each way.
        let full = (71 * step * step / (skip * skip)) as f32;
        let mut cells: Vec<f32> = sums.iter().map(|s| *s as f32 / full).collect();
        let radius = ((w as f32 / 70.0) / step as f32).round().max(1.0) as usize;
        let mut scratch = Vec::with_capacity(cw.max(ch));
        for _ in 0..3 {
            box_blur(&mut cells, cw, ch, radius, &mut scratch);
        }
        // Spread, so weaker: given back, then scaled by how strong; what
        // the blur leaves as dust is none, so rows far from a light skip it.
        let k = amount * 3.0 * 4096.0;
        let cells: Vec<i32> = cells.iter().map(|c| if *c < 1e-4 { 0 } else { (c * k).round() as i32 }).collect();
        let row_max = cells.chunks_exact(cw).map(|r| r.iter().copied().max().unwrap_or(0)).collect();
        Glow { cells, cw, ch, step, row_max }
    }

    /// Where a column falls between the cells: the two, and how far (256ths).
    fn column(&self, x: usize) -> (u16, u16, u8) {
        let fx = ((x as f32 + 0.5) / self.step as f32 - 0.5).clamp(0.0, (self.cw - 1) as f32);
        let x0 = fx as usize;
        (x0 as u16, (x0 + 1).min(self.cw - 1) as u16, ((fx - x0 as f32) * 255.0) as u8)
    }

    /// The glow along picture row `y`, cell by cell, into `along`: false
    /// (and nothing made) where neither of its rows of cells glows.
    fn row(&self, y: usize, along: &mut Vec<i32>) -> bool {
        let fy = ((y as f32 + 0.5) / self.step as f32 - 0.5).clamp(0.0, (self.ch - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(self.ch - 1);
        if self.row_max[y0] == 0 && self.row_max[y1] == 0 {
            return false;
        }
        let k = ((fy - y0 as f32) * 256.0) as i32;
        let (a, b) = (&self.cells[y0 * self.cw..][..self.cw], &self.cells[y1 * self.cw..][..self.cw]);
        along.clear();
        along.extend(a.iter().zip(b).map(|(a, b)| a + (((b - a) * k) >> 8)));
        true
    }
}

/// A box blur of `r` each way, along rows then columns, edges held.
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize, line: &mut Vec<f32>) {
    let span = (2 * r + 1) as f32;
    // One line of `n`, from `start`, `stride` apart.
    let mut pass = |start: usize, stride: usize, n: usize, v: &mut [f32]| {
        let at = |i: usize| start + i * stride;
        line.clear();
        let mut sum: f32 = (0..=2 * r).map(|i| v[at(i.saturating_sub(r).min(n - 1))]).sum();
        for i in 0..n {
            line.push(sum / span);
            sum += v[at((i + r + 1).min(n - 1))] - v[at(i.saturating_sub(r))];
        }
        for (i, c) in line.iter().enumerate() {
            v[at(i)] = *c;
        }
    };
    for y in 0..h {
        pass(y * w, 1, w, v);
    }
    for x in 0..w {
        pass(x, w, h, v);
    }
}
/// A photo's own seed: from its name, so the grain is the same each time
/// it is laid on, and another photo's is another.
pub fn seed_of(photo: &Path) -> u32 {
    let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193)) | 1
}

/// The texture laid over a saved photo, in its place: decoded whole, the
/// texture over it, written again at the camera's quality with its Exif.
pub fn finish_photo(photo: &Path, tex: Texture) -> Result<(), String> {
    let bytes = std::fs::read(photo).map_err(|e| format!("the photo cannot be read: {e}"))?;
    let exif = crate::burst::exif_of(&bytes);
    let (data, info) = {
        let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(&bytes));
        let data = decoder.decode().map_err(|e| format!("the photo is damaged: {e}"))?;
        (data, decoder.info().ok_or("the photo has no picture")?)
    };
    drop(bytes);
    let (w, h) = (usize::from(info.width), usize::from(info.height));
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => data,
        jpeg_decoder::PixelFormat::L8 => data.iter().flat_map(|v| [*v; 3]).collect(),
        _ => return Err("not a photo this reads".into()),
    };
    let mut pic = Picture { w, h, rgb };
    apply(&mut pic, tex, seed_of(photo));
    let yuv = crate::camera::rgb_to_yuv420(&pic);
    drop(pic);
    if !photo.exists() {
        return Err("the photo was deleted while its grain was laid on".into());
    }
    crate::burst::write_jpeg(photo, &yuv, w, h, exif)
}

/// One photo's texture laid on at a time (each holds the photo decoded).
static LAYING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long laying the texture on a photo should take: my estimate for the
/// CM0 at idle priority (decoding the 8 MP photo, the texture on two cores,
/// writing it again), and this computer's.
fn expect_ms() -> u64 {
    if cfg!(target_arch = "aarch64") {
        4500
    } else {
        900
    }
}

/// [`finish_photo`] on a thread of its own at idle priority, the photo's
/// "refining" marker (burst.rs's) written first and taken away when done,
/// so the Camera's thumbnail wears its badge meanwhile. Gives the marker.
pub fn lay_on_in_background(photo: std::path::PathBuf, tex: Texture) -> crate::burst::Refining {
    let marking = crate::burst::Refining { began_ms: crate::burst::now_ms(), expect_ms: expect_ms(), progress: 0.0 };
    let marker = crate::burst::marker(&photo);
    std::fs::write(&marker, format!("began={}\nexpect={}\nprogress=0\n", marking.began_ms, marking.expect_ms)).ok();
    std::thread::Builder::new()
        .name("film-grain".into())
        .spawn(move || {
            crate::sched::enter(crate::sched::Class::Idle);
            let held = LAYING.lock();
            if let Err(why) = finish_photo(&photo, tex) {
                eprintln!("camera: film grain: {why}");
            }
            drop(held);
            std::fs::remove_file(marker).ok();
            crate::photos::changed();
            crate::platform::wake();
        })
        .ok();
    marking
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(w: usize, h: usize, v: u8) -> Picture {
        Picture { w, h, rgb: vec![v; w * h * 3] }
    }

    fn deviation(pic: &Picture) -> f32 {
        let l: Vec<f32> = pic.rgb.chunks_exact(3).map(|p| f32::from(p[1])).collect();
        let mean = l.iter().sum::<f32>() / l.len() as f32;
        (l.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / l.len() as f32).sqrt()
    }

    #[test]
    fn grain_is_strongest_in_the_middle_tones_neutral_and_fixed_by_its_seed() {
        let tex = Texture { grain: 0.06, halation: 0.0 };
        let mut mid = grey(800, 60, 128);
        apply(&mut mid, tex, 7);
        let mut dark = grey(800, 60, 12);
        apply(&mut dark, tex, 7);
        let (dm, dd) = (deviation(&mid), deviation(&dark));
        // About its deviation in the middle (6% of white is 15 levels).
        assert!((10.0..20.0).contains(&dm), "{dm}");
        assert!(dd < dm * 0.5, "faint in the shadows: {dd} against {dm}");
        // The same on every channel: grain in the brightness, no colour noise.
        assert!(mid.rgb.chunks_exact(3).all(|p| p[0] == p[1] && p[1] == p[2]));
        // Fixed by its seed: the same photo grains the same; another differs.
        let mut again = grey(800, 60, 128);
        apply(&mut again, tex, 7);
        assert_eq!(again, mid);
        let mut other = grey(800, 60, 128);
        apply(&mut other, tex, 8);
        assert_ne!(other, mid);
        assert_ne!(seed_of(Path::new("/p/IMG_1.jpg")), seed_of(Path::new("/p/IMG_2.jpg")));
        // Off: nothing.
        let mut off = grey(64, 48, 128);
        apply(&mut off, tex.at(Strength::Off), 7);
        assert_eq!(off, grey(64, 48, 128));
    }

    #[test]
    fn grain_is_sized_to_the_picture() {
        let tex = Texture { grain: 0.06, halation: 0.0 };
        // A photo: specks several pixels across (neighbours alike). The
        // preview: finer than a pixel (neighbours unlike), and fainter.
        let mut photo = grey(3200, 40, 128);
        apply(&mut photo, tex, 3);
        let mut preview = grey(320, 240, 128);
        apply(&mut preview, tex, 0);
        let alike = |p: &Picture| {
            let g: Vec<f32> = p.rgb.chunks_exact(3).map(|c| f32::from(c[1]) - 128.0).collect();
            let (mut num, mut den) = (0.0, 0.0);
            for i in 0..g.len() - 1 {
                num += g[i] * g[i + 1];
                den += g[i] * g[i];
            }
            num / den
        };
        assert!(alike(&photo) > 0.6, "smooth specks: {}", alike(&photo));
        assert!(alike(&preview).abs() < 0.2, "a pixel's own: {}", alike(&preview));
        assert!(deviation(&preview) < deviation(&photo) * 0.6, "{} against {}", deviation(&preview), deviation(&photo));
    }

    #[test]
    fn halation_glows_red_orange_round_a_light_and_leaves_the_dark_alone() {
        let (w, h) = (320, 240);
        let mut pic = grey(w, h, 20);
        // A bright lamp in the middle.
        for y in 110..130 {
            for x in 150..170 {
                pic.rgb[(y * w + x) * 3..][..3].copy_from_slice(&[255, 250, 240]);
            }
        }
        let before = pic.clone();
        apply(&mut pic, Texture { grain: 0.0, halation: 0.6 }, 0);
        let px = |p: &Picture, x: usize, y: usize| <[u8; 3]>::try_from(&p.rgb[(y * w + x) * 3..][..3]).unwrap();
        // Just outside the lamp: warmer and brighter, red most.
        let near = px(&pic, 174, 120);
        assert!(near[0] > 40 && near[0] > near[1] && near[1] > near[2], "{near:?}");
        // Far away: the dark as it was.
        assert_eq!(px(&pic, 10, 10), px(&before, 10, 10));
        // The lamp itself stays white, never past it.
        assert!(px(&pic, 160, 120)[0] == 255);
    }

    #[test]
    fn a_saved_photo_takes_its_texture_and_keeps_its_exif() {
        let dir = std::env::temp_dir().join(format!("catalyst-film-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("IMG_20261123_101010.jpg");
        let pic = crate::camera::scene(640, 480, 2.0, false);
        let yuv = crate::camera::rgb_to_yuv420(&pic);
        crate::burst::write_jpeg(&photo, &yuv, 640, 480, Some(crate::filters::exif_with_look("Pushed Mono, strong grain"))).unwrap();
        let before = crate::photos::decode(&photo, 640, 480).unwrap();
        finish_photo(&photo, Texture { grain: 0.08, halation: 0.0 }).unwrap();
        let after = crate::photos::decode(&photo, 640, 480).unwrap();
        assert_eq!((after.w, after.h), (640, 480));
        let moved = before.rgb.iter().zip(&after.rgb).filter(|(a, b)| a.abs_diff(**b) > 6).count();
        assert!(moved > before.rgb.len() / 10, "grain laid on: {moved}");
        assert_eq!(crate::filters::look_of(&photo).as_deref(), Some("Pushed Mono, strong grain"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
