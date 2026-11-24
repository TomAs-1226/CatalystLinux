//! Photos: the album of what the Camera took, and the viewer.
//!
//! Photos are JPEG files in `/data/photos` (beside the settings on a
//! computer), named for when they were taken (`IMG_20261121_140312.jpg`), so
//! Files and a computer reading the card see them in order too. The album
//! shows them newest first as a grid of thumbnails; each thumbnail is made
//! once, on a thread, from the photo decoded at an eighth of its size (the
//! decoder scales as it decodes, so a 3280 × 2464 photo costs 410 × 308 of
//! memory, not 24 MB), and kept small beside the photos in `.thumbs`.
//!
//! The viewer fits a photo to the panel, and zooms (+ and −) to two and four
//! times, panning with the arrows; Left and Right go between photos when it
//! is not zoomed. It decodes only as much as the zoom needs (an eighth, a
//! quarter, half), on a thread, showing the thumbnail meanwhile; only one
//! photo is ever held at a time. `e` opens the editor (editor/, 1.0).
//!
//! Pixel pictures (the Camera's pixel mode, 1.2) are PNGs named
//! `PIX_…png`: the album's is the copy blown up for sharing, and the one at
//! its true size is kept beside it as its original. They are drawn with
//! whole pixels, never smoothed: the viewer shows the true size at a whole
//! number of panel pixels each (the largest that fits, then twice and four
//! times that), and their thumbnails are sampled, not averaged.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bezel::{Align, Rect};
use chrono::{NaiveDateTime, TimeZone};
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// A thumbnail's size: 4:3, as the camera's photos are.
pub const THUMB_W: usize = 96;
pub const THUMB_H: usize = 72;
/// Columns in the album's grid.
const COLS: usize = 4;
const GAP: f32 = 4.0;

// --------------------------------------------------------------- pictures ---

/// A picture in memory: RGB, three bytes a pixel, rows packed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

impl Picture {
    pub fn new(w: usize, h: usize) -> Self {
        Picture { w, h, rgb: vec![0; w * h * 3] }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The picture shrunk to `w` × `h`, each pixel the average of the ones it
    /// stands for, cropped to the new shape (centred) rather than squeezed.
    pub fn cover(&self, w: usize, h: usize) -> Picture {
        let mut out = Picture::new(w, h);
        if self.is_empty() || w == 0 || h == 0 {
            return out;
        }
        // The part of the picture with the new shape.
        let (sw, sh) = if self.w * h > self.h * w { (self.h * w / h, self.h) } else { (self.w, self.w * h / w) };
        let (x0, y0) = ((self.w - sw) / 2, (self.h - sh) / 2);
        for oy in 0..h {
            let (ya, yb) = (y0 + oy * sh / h, (y0 + (oy + 1) * sh / h).max(y0 + oy * sh / h + 1));
            for ox in 0..w {
                let (xa, xb) = (x0 + ox * sw / w, (x0 + (ox + 1) * sw / w).max(x0 + ox * sw / w + 1));
                let mut sum = [0u32; 3];
                for y in ya..yb.min(self.h) {
                    for x in xa..xb.min(self.w) {
                        let at = (y * self.w + x) * 3;
                        for (s, c) in sum.iter_mut().zip(&self.rgb[at..at + 3]) {
                            *s += u32::from(*c);
                        }
                    }
                }
                let n = ((yb.min(self.h) - ya) * (xb.min(self.w) - xa)).max(1) as u32;
                let at = (oy * w + ox) * 3;
                for (o, s) in out.rgb[at..at + 3].iter_mut().zip(sum) {
                    *o = (s / n) as u8;
                }
            }
        }
        out
    }

    /// The picture at `w` × `h`, each pixel the one under its middle (for
    /// pixel pictures: their pixels kept whole, never averaged into new
    /// colours), cropped to the new shape as [`Picture::cover`] is.
    pub fn cover_nearest(&self, w: usize, h: usize) -> Picture {
        let mut out = Picture::new(w, h);
        if self.is_empty() || w == 0 || h == 0 {
            return out;
        }
        let (sw, sh) = if self.w * h > self.h * w { (self.h * w / h, self.h) } else { (self.w, self.w * h / w) };
        let (x0, y0) = ((self.w - sw) / 2, (self.h - sh) / 2);
        for oy in 0..h {
            let y = (y0 + (2 * oy + 1) * sh / (2 * h)).min(self.h - 1);
            for ox in 0..w {
                let x = (x0 + (2 * ox + 1) * sw / (2 * w)).min(self.w - 1);
                out.rgb[(oy * w + ox) * 3..][..3].copy_from_slice(&self.rgb[(y * self.w + x) * 3..][..3]);
            }
        }
        out
    }
}

/// Draw the part `view` (x, y, w, h in the picture's pixels) of `pic` into
/// `dest` on the panel, sampled smoothly (bilinear), with its corners
/// rounded to `radius` and at `alpha` over what is there.
pub fn draw_picture(g: &mut Gfx, dest: Rect, pic: &Picture, view: [f32; 4], radius: f32, alpha: f32) {
    draw_sampled(g, dest, pic, view, radius, alpha, false);
}

/// As [`draw_picture`], each panel pixel the picture's pixel under its
/// middle: a pixel picture's pixels stay whole and sharp at any size.
pub fn draw_picture_crisp(g: &mut Gfx, dest: Rect, pic: &Picture, view: [f32; 4], radius: f32, alpha: f32) {
    draw_sampled(g, dest, pic, view, radius, alpha, true);
}

fn draw_sampled(g: &mut Gfx, dest: Rect, pic: &Picture, view: [f32; 4], radius: f32, alpha: f32, crisp: bool) {
    if pic.is_empty() || dest.w < 1.0 || dest.h < 1.0 || alpha <= 0.0 {
        return;
    }
    let (cw, ch) = (g.canvas.w, g.canvas.h);
    let dx = dest.x + g.ox;
    let x0 = dx.floor().max(0.0) as i32;
    let y0 = dest.y.floor().max(0.0) as i32;
    let x1 = (dx + dest.w).ceil().min(cw as f32) as i32;
    let y1 = (dest.y + dest.h).ceil().min(ch as f32) as i32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let [vx, vy, vw, vh] = view;
    let (sx, sy) = (vw / dest.w, vh / dest.h);
    let (pw, ph) = (pic.w as i32, pic.h as i32);
    let a_all = alpha.clamp(0.0, 1.0);
    let r = radius.max(0.0);
    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        // Vertical coverage at the edges: whole pixels inside, part at the rims.
        let cover_y = (fy - dest.y + 0.5).clamp(0.0, 1.0) * (dest.y + dest.h - fy + 0.5).clamp(0.0, 1.0);
        let v = (vy + (fy - dest.y) * sy - 0.5).clamp(0.0, (ph - 1) as f32);
        let (v0, vf) = (v.floor() as i32, v.fract());
        let v1 = (v0 + 1).min(ph - 1);
        let wy = (vf * 256.0) as u32;
        let row = (py * cw) as usize;
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            let mut a = a_all * cover_y * (fx - dx + 0.5).clamp(0.0, 1.0) * (dx + dest.w - fx + 0.5).clamp(0.0, 1.0);
            if r > 0.0 {
                // Inside a corner's square: how far in from its circle.
                let qx = (fx - dx).min(dx + dest.w - fx);
                let qy = (fy - dest.y).min(dest.y + dest.h - fy);
                if qx < r && qy < r {
                    let d = ((r - qx).powi(2) + (r - qy).powi(2)).sqrt();
                    a *= (r - d + 0.5).clamp(0.0, 1.0);
                }
            }
            if a <= 0.0 {
                continue;
            }
            let at = row + px as usize;
            if crisp {
                let u = ((vx + (fx - dx) * sx).floor() as i32).clamp(0, pw - 1);
                let v = ((vy + (fy - dest.y) * sy).floor() as i32).clamp(0, ph - 1);
                let p = (v * pw + u) as usize * 3;
                let new = (u32::from(pic.rgb[p]) << 16) | (u32::from(pic.rgb[p + 1]) << 8) | u32::from(pic.rgb[p + 2]);
                g.canvas.px[at] = if a >= 0.999 { new } else { blend(g.canvas.px[at], new, a) };
                continue;
            }
            let u = (vx + (fx - dx) * sx - 0.5).clamp(0.0, (pw - 1) as f32);
            let (u0, uf) = (u.floor() as i32, u.fract());
            let u1 = (u0 + 1).min(pw - 1);
            let wx = (uf * 256.0) as u32;
            let p = |x: i32, y: i32| (y * pw + x) as usize * 3;
            let (a00, a10, a01, a11) = (p(u0, v0), p(u1, v0), p(u0, v1), p(u1, v1));
            let mut rgb = [0u32; 3];
            for (c, out) in rgb.iter_mut().enumerate() {
                let top = u32::from(pic.rgb[a00 + c]) * (256 - wx) + u32::from(pic.rgb[a10 + c]) * wx;
                let bottom = u32::from(pic.rgb[a01 + c]) * (256 - wx) + u32::from(pic.rgb[a11 + c]) * wx;
                *out = (top * (256 - wy) + bottom * wy) >> 16;
            }
            let new = (rgb[0] << 16) | (rgb[1] << 8) | rgb[2];
            g.canvas.px[at] = if a >= 0.999 { new } else { blend(g.canvas.px[at], new, a) };
        }
    }
}

fn blend(under: u32, over: u32, a: f32) -> u32 {
    let k = (a * 256.0) as u32;
    let mix = |shift: u32| ((((over >> shift) & 0xff) * k + ((under >> shift) & 0xff) * (256 - k)) >> 8) << shift;
    mix(16) | mix(8) | mix(0)
}

/// The whole picture's view.
pub fn whole(pic: &Picture) -> [f32; 4] {
    [0.0, 0.0, pic.w as f32, pic.h as f32]
}

// ---------------------------------------------------------------- storage ---

/// Where photos are kept: on the data partition, where Files opens.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/photos")
    } else {
        crate::config_dir().join("photos")
    }
}

/// Whether a file is one the album shows.
pub fn is_photo(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.starts_with('.') && (lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".gif"))
}

/// A pixel picture or loop (the Camera's pixel mode): drawn with whole pixels.
pub fn is_pixel(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with("PIX_") && (lower.ends_with(".png") || lower.ends_with(".gif"))
}

/// A loop (an animated GIF), played in the viewer.
pub fn is_loop(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".gif")
}

/// A loop's frames, each with how long it shows (ms), at most 64.
pub fn loop_frames(path: &Path) -> Result<Vec<(Picture, u32)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot open the loop: {e}"))?;
    crate::filters::gif::decode(&bytes, 64)
}

/// A pixel picture's file at its true size: its original, kept beside it
/// (else the file itself).
pub fn pixel_true_size(path: &Path) -> PathBuf {
    let kept = crate::filters::original_of(path);
    if kept.is_file() {
        kept
    } else {
        path.to_path_buf()
    }
}

fn is_png(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// A photo in the album.
#[derive(Clone, Debug, PartialEq)]
pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub taken: NaiveDateTime,
    pub bytes: u64,
}

/// When a photo was taken: from its name (the camera's IMG_YYYYMMDD_HHMMSS),
/// else from when the file was last written.
fn taken_of(name: &str, modified: Option<std::time::SystemTime>) -> NaiveDateTime {
    let stem = name.split('.').next().unwrap_or(name);
    let digits: String = stem.trim_start_matches("IMG_").trim_start_matches("PIX_").chars().take(15).collect();
    if let Ok(at) = NaiveDateTime::parse_from_str(&digits, "%Y%m%d_%H%M%S") {
        return at;
    }
    let secs = modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    chrono::Local.timestamp_opt(secs, 0).single().map_or_else(|| chrono::DateTime::UNIX_EPOCH.naive_utc(), |d| d.naive_local())
}

/// Every photo, newest first.
pub fn list() -> Vec<Photo> {
    list_in(&dir())
}

/// Every photo in an album's folder, newest first: what the album shows,
/// and all the phone gallery (phonegallery/) ever serves.
pub fn list_in(dir: &Path) -> Vec<Photo> {
    let mut photos: Vec<Photo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata().ok().filter(std::fs::Metadata::is_file)?;
            is_photo(&name).then(|| Photo { path: e.path(), taken: taken_of(&name, meta.modified().ok()), name, bytes: meta.len() })
        })
        .collect();
    photos.sort_by(|a, b| b.taken.cmp(&a.taken).then_with(|| b.name.cmp(&a.name)));
    photos
}

/// Where the next photo goes: named for `now`, and never over one already
/// there (two in the same second become `_2`, `_3`).
pub fn new_path(now: NaiveDateTime) -> PathBuf {
    let path = new_path_in(&dir(), now);
    // Its moment noted for the build log: the Wi-Fi, the event (photolog/).
    crate::photolog::taken(&path, now);
    path
}

fn new_path_in(dir: &Path, now: NaiveDateTime) -> PathBuf {
    let stem = format!("IMG_{}", now.format("%Y%m%d_%H%M%S"));
    let mut path = dir.join(format!("{stem}.jpg"));
    let mut n = 2;
    while path.exists() || path.with_extension("jpg.part").exists() {
        path = dir.join(format!("{stem}_{n}.jpg"));
        n += 1;
    }
    path
}

/// Where the next pixel picture goes: `PIX_…png`, named as photos are.
pub fn new_pixel_path(now: NaiveDateTime) -> PathBuf {
    let dir = dir();
    let stem = format!("PIX_{}", now.format("%Y%m%d_%H%M%S"));
    let mut path = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}.png"));
        n += 1;
    }
    crate::photolog::taken(&path, now);
    path
}

/// A photo's size in pixels, from its header alone.
pub fn dimensions(path: &Path) -> Option<(usize, usize)> {
    if is_loop(&path.to_string_lossy()) {
        use std::io::Read;
        let mut head = [0u8; 10];
        std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
        return crate::filters::gif::dimensions(&head);
    }
    if is_png(path) {
        let reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).read_info().ok()?;
        return Some((reader.info().width as usize, reader.info().height as usize));
    }
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    decoder.read_info().ok()?;
    decoder.info().map(|i| (usize::from(i.width), usize::from(i.height)))
}

/// A photo decoded at the smallest of its whole, half, quarter or eighth
/// sizes that is at least `want_w` × `want_h`.
pub fn decode(path: &Path, want_w: usize, want_h: usize) -> Result<Picture, String> {
    if is_png(path) {
        return decode_png(path);
    }
    // A loop: its first frame.
    if is_loop(&path.to_string_lossy()) {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot open the loop: {e}"))?;
        return crate::filters::gif::decode(&bytes, 1)?.into_iter().next().map(|(p, _)| p).ok_or_else(|| "the loop is damaged".to_owned());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open the photo: {e}"))?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    let (w, h) = decoder.scale(want_w.min(65535) as u16, want_h.min(65535) as u16).map_err(|e| format!("not a photo this reads: {e}"))?;
    let data = decoder.decode().map_err(|e| format!("the photo is damaged: {e}"))?;
    let info = decoder.info().ok_or("the photo has no picture")?;
    let (w, h) = (usize::from(w), usize::from(h));
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => data,
        jpeg_decoder::PixelFormat::L8 => data.iter().flat_map(|v| [*v, *v, *v]).collect(),
        jpeg_decoder::PixelFormat::L16 => data.chunks_exact(2).flat_map(|v| [v[0], v[0], v[0]]).collect(),
        jpeg_decoder::PixelFormat::CMYK32 => data
            .chunks_exact(4)
            .flat_map(|p| {
                let k = 255 - u32::from(p[3]);
                let ch = |c: u8| ((255 - u32::from(c)) * k / 255) as u8;
                [ch(p[0]), ch(p[1]), ch(p[2])]
            })
            .collect(),
    };
    if rgb.len() < w * h * 3 {
        return Err("the photo is damaged".into());
    }
    Ok(Picture { w, h, rgb })
}

/// A PNG (a pixel picture, small) decoded whole, as RGB.
fn decode_png(path: &Path) -> Result<Picture, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open the picture: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a picture this reads: {e}"))?;
    let (w, h) = (reader.info().width as usize, reader.info().height as usize);
    // Larger than a pixel picture's copy ever is: not for this album.
    if w * h > 4096 * 4096 {
        return Err("too large a picture".into());
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("too large a picture")?];
    let out = reader.next_frame(&mut buf).map_err(|e| format!("the picture is damaged: {e}"))?;
    let data = &buf[..out.buffer_size()];
    let rgb: Vec<u8> = match out.color_type {
        png::ColorType::Rgb => data.to_vec(),
        png::ColorType::Rgba => data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|v| [*v; 3]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0]; 3]).collect(),
        png::ColorType::Indexed => return Err("the picture is damaged".into()),
    };
    if rgb.len() < w * h * 3 {
        return Err("the picture is damaged".into());
    }
    Ok(Picture { w, h, rgb })
}

/// A thumbnail on disk: "CTH1", its width and height, the photo's size in
/// bytes when it was made (so a changed photo gets a new one), its pixels.
fn thumb_file(photo: &Photo) -> PathBuf {
    thumb_file_in(&dir(), photo)
}

fn thumb_file_in(album: &Path, photo: &Photo) -> PathBuf {
    album.join(".thumbs").join(format!("{}.thumb", photo.name))
}

fn read_thumb(path: &Path, bytes: u64) -> Option<Picture> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 16 || &data[..4] != b"CTH1" {
        return None;
    }
    let w = usize::from(u16::from_le_bytes([data[4], data[5]]));
    let h = usize::from(u16::from_le_bytes([data[6], data[7]]));
    let made_from = u64::from_le_bytes(data[8..16].try_into().ok()?);
    (made_from == bytes && data.len() == 16 + w * h * 3).then(|| Picture { w, h, rgb: data[16..].to_vec() })
}

fn write_thumb(path: &Path, bytes: u64, pic: &Picture) {
    let mut data = Vec::with_capacity(16 + pic.rgb.len());
    data.extend_from_slice(b"CTH1");
    data.extend_from_slice(&(pic.w as u16).to_le_bytes());
    data.extend_from_slice(&(pic.h as u16).to_le_bytes());
    data.extend_from_slice(&bytes.to_le_bytes());
    data.extend_from_slice(&pic.rgb);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    crate::sys::write_atomic(path, &data).ok();
}

/// A photo's thumbnail: the one kept, else made now (and kept).
pub fn thumbnail(photo: &Photo) -> Option<Picture> {
    thumbnail_in(&dir(), photo)
}

/// As [`thumbnail`], for a photo in the album kept in `album` (the phone
/// gallery is given its folder, so its tests can have one of their own).
pub fn thumbnail_in(album: &Path, photo: &Photo) -> Option<Picture> {
    let file = thumb_file_in(album, photo);
    if let Some(t) = read_thumb(&file, photo.bytes) {
        return Some(t);
    }
    let decoded = decode(&photo.path, THUMB_W * 2, THUMB_H * 2).ok()?;
    // A pixel picture's pixels sampled, never averaged into new colours.
    let made = if is_pixel(&photo.name) { decoded.cover_nearest(THUMB_W, THUMB_H) } else { decoded.cover(THUMB_W, THUMB_H) };
    write_thumb(&file, photo.bytes, &made);
    Some(made)
}

/// Remove a photo and its thumbnail.
pub fn delete(photo: &Photo) -> std::io::Result<()> {
    std::fs::remove_file(&photo.path)?;
    std::fs::remove_file(thumb_file(photo)).ok();
    // And what was kept beside it: the photo as taken (under a look), its RAW.
    std::fs::remove_file(crate::filters::original_of(&photo.path)).ok();
    std::fs::remove_file(photo.path.with_extension("dng")).ok();
    std::fs::remove_file(crate::editor::edits::sidecar(&photo.path)).ok();
    // Its caption and place (photolog/).
    crate::photolog::forget(&photo.path);
    TILE_COUNT.store(u64::MAX, Ordering::Relaxed);
    Ok(())
}

/// Thumbnails made on a thread, for whichever photos are wanted (those in
/// sight first); a few hundred kept in memory at most.
struct Thumbs {
    shared: Arc<Mutex<ThumbState>>,
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct ThumbState {
    wanted: Vec<Photo>,
    ready: HashMap<String, Picture>,
    failed: HashSet<String>,
    generation: u64,
}

const THUMBS_KEPT: usize = 160;

impl Thumbs {
    fn new() -> Self {
        let shared = Arc::new(Mutex::new(ThumbState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new().name("thumbs".into()).stack_size(512 * 1024).spawn(move || make_thumbs(&s, &st)).ok();
        Thumbs { shared, stop }
    }

    fn want(&self, photos: &[Photo]) {
        if let Ok(mut s) = self.shared.lock() {
            if s.wanted != photos {
                s.wanted = photos.to_vec();
            }
        }
    }

    fn get(&self, name: &str) -> Option<Picture> {
        self.shared.lock().ok()?.ready.get(name).cloned()
    }

    fn failed(&self, name: &str) -> bool {
        self.shared.lock().is_ok_and(|s| s.failed.contains(name))
    }

    fn generation(&self) -> u64 {
        self.shared.lock().map_or(0, |s| s.generation)
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn make_thumbs(shared: &Mutex<ThumbState>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let next = shared.lock().ok().and_then(|s| s.wanted.iter().find(|p| !s.ready.contains_key(&p.name) && !s.failed.contains(&p.name)).cloned());
        let Some(photo) = next else {
            std::thread::sleep(Duration::from_millis(60));
            continue;
        };
        let made = thumbnail(&photo);
        let Ok(mut s) = shared.lock() else { return };
        match made {
            Some(t) => {
                s.ready.insert(photo.name.clone(), t);
            }
            None => {
                s.failed.insert(photo.name.clone());
            }
        }
        // Too many held: those no longer wanted go.
        if s.ready.len() > THUMBS_KEPT {
            let keep: HashSet<String> = s.wanted.iter().map(|p| p.name.clone()).collect();
            s.ready.retain(|name, _| keep.contains(name));
        }
        s.generation += 1;
    }
}

/// How many photos, for Home's tile (counted again after a while, or at once
/// when one is taken or deleted).
static TILE_COUNT: AtomicU64 = AtomicU64::new(u64::MAX);

thread_local! {
    static TILE_AT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// A photo was taken: Home's tile counts again.
pub fn changed() {
    TILE_COUNT.store(u64::MAX, Ordering::Relaxed);
}

/// Home's tile: how many photos there are.
pub fn tile() -> (String, Option<&'static str>) {
    let stale = TILE_AT.with(|at| at.get().is_none_or(|t| t.elapsed() > Duration::from_secs(30)));
    if stale || TILE_COUNT.load(Ordering::Relaxed) == u64::MAX {
        TILE_COUNT.store(list().len() as u64, Ordering::Relaxed);
        TILE_AT.with(|at| at.set(Some(std::time::Instant::now())));
    }
    let n = TILE_COUNT.load(Ordering::Relaxed);
    (
        match n {
            0 => "none yet".into(),
            1 => "1 photo".into(),
            n => format!("{n} photos"),
        },
        None,
    )
}

/// "nov 21 · 14:03", or with the year when it is not this one.
pub fn when(at: NaiveDateTime, h24: bool) -> String {
    let now = chrono::Local::now().naive_local();
    let clock = if h24 { at.format("%H:%M").to_string() } else { at.format("%-I:%M %P").to_string() };
    let day = if at.format("%Y").to_string() == now.format("%Y").to_string() { at.format("%b %-d").to_string() } else { at.format("%b %-d %Y").to_string() };
    format!("{} · {clock}", day.to_lowercase())
}

/// "2.4 mb", "860 kb".
pub fn size_words(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} mb", bytes as f64 / 1e6)
    } else {
        format!("{} kb", bytes.div_ceil(1000))
    }
}

// ------------------------------------------------------------------ album ---

pub struct Album {
    photos: Vec<Photo>,
    focus: usize,
    /// The first row in sight, gliding to where the focus needs it.
    scroll: Animated,
    thumbs: Thumbs,
    seen: u64,
    deleting: Option<usize>,
}

impl Album {
    pub fn new() -> Self {
        #[cfg(not(target_arch = "aarch64"))]
        seed_demo();
        Album { photos: list(), focus: 0, scroll: Animated::new(0.0, role::settle()).with_rest(0.002), thumbs: Thumbs::new(), seen: u64::MAX, deleting: None }
    }

    /// The album open on a photo (by its path), e.g. the one just taken.
    pub fn at(path: &Path) -> Self {
        let mut album = Self::new();
        album.focus = album.photos.iter().position(|p| p.path == path).unwrap_or(0);
        album
    }

    /// A thumbnail's place in the grid: four across, at 4:3.
    fn cell() -> (f32, f32) {
        let w = ((W - 2.0 * PAD - 5.0 - GAP * (COLS as f32 - 1.0)) / COLS as f32).floor();
        (w, (w * 3.0 / 4.0).floor())
    }

    /// Rows that fit between the title and the line along the bottom.
    fn rows_in_sight(g: &Gfx) -> usize {
        let (_, ch) = Self::cell();
        let room = g.above_hint() + crate::gfx::FOOT_AIR - crate::gfx::RING_ROOM - g.top();
        (((room + GAP) / (ch + GAP)).floor() as usize).max(1)
    }

    fn keep_in_sight(&mut self, rows: usize) {
        let row = (self.focus / COLS) as f32;
        let first = self.scroll.target();
        if row < first {
            self.scroll.set(row);
        } else if row >= first + rows as f32 {
            self.scroll.set(row + 1.0 - rows as f32);
        }
    }
}

impl Screen for Album {
    fn title(&self) -> &str {
        "Photos"
    }

    fn resume(&mut self) {
        let focused = self.photos.get(self.focus).map(|p| p.path.clone());
        self.photos = list();
        self.focus = focused.and_then(|f| self.photos.iter().position(|p| p.path == f)).unwrap_or(self.focus).min(self.photos.len().saturating_sub(1));
        self.seen = u64::MAX;
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let n = self.photos.len();
        match key {
            // Esc keeps a photo it was asked about rather than leaving.
            Key::Back if armed.is_some() => {}
            Key::Back | Key::Leave => return Nav::Pop,
            // The build log: days, places, captions (photolog/).
            Key::Char('l') => return Nav::Push(Box::new(crate::photolog::BuildLog::new())),
            _ if n == 0 => {}
            // On a phone, in its browser (phonegallery/, 1.5.0).
            Key::Char('p') => return Nav::Push(Box::new(crate::phonegallery::GalleryScreen::new(cx.settings))),
            Key::Left => self.focus = self.focus.saturating_sub(1),
            Key::Right => self.focus = (self.focus + 1).min(n - 1),
            Key::Up => self.focus = self.focus.saturating_sub(COLS),
            Key::Down => self.focus = (self.focus + COLS).min(n - 1),
            Key::Home => self.focus = 0,
            Key::End => self.focus = n - 1,
            Key::Enter => return Nav::Push(Box::new(Viewer::new(self.photos.clone(), self.focus))),
            Key::Char('d') => {
                if armed == Some(self.focus) {
                    let photo = self.photos[self.focus].clone();
                    *cx.say = Some(match delete(&photo) {
                        Ok(()) => {
                            self.photos.remove(self.focus);
                            self.focus = self.focus.min(self.photos.len().saturating_sub(1));
                            "photo deleted".into()
                        }
                        Err(e) => format!("could not delete it: {e}"),
                    });
                } else {
                    self.deleting = Some(self.focus);
                    *cx.say = Some("d again deletes this photo".into());
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.deleting.is_some_and(|d| d == self.focus) {
            return vec![keys("deleting", &[("d", "Delete it"), ("esc", "Keep it")])];
        }
        vec![keys("photos", &[("enter", "Open it"), ("p", "On your phone"), ("l", "The build log"), ("d twice", "Delete it"), ("fn k · fn n", "First, last")])]
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.scroll.tick(dt);
        let generation = self.thumbs.generation();
        let fresh = generation != self.seen;
        self.seen = generation;
        moving || fresh
    }

    fn busy(&self) -> bool {
        // Thumbnails arrive from their thread while any in sight are missing.
        true
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if self.photos.is_empty() {
            g.empty("photo_library", "no photos yet · the camera keeps them here");
            g.hint("esc: back");
            return;
        }
        let rows = Self::rows_in_sight(g);
        self.keep_in_sight(rows);
        if g.calm {
            self.scroll.jump(self.scroll.target());
        }
        let (cw, ch) = Self::cell();
        let top = g.top();
        let bottom = g.above_hint();
        let first = self.scroll.value();
        // Those in sight (and a row each side) are the thumbnails wanted.
        let from = ((first.floor() as usize).saturating_sub(1)) * COLS;
        let to = ((first.ceil() as usize + rows + 1) * COLS).min(self.photos.len());
        self.thumbs.want(&self.photos[from..to]);
        let radius = g.radius(8.0).min(ch * 0.25);
        // Under the whole rows, the next one fades out above the line along the bottom.
        let foot = bottom + crate::gfx::FOOT_AIR;
        g.canvas.set_clip(Rect::new(0.0, top - 2.0, W, foot - top + 2.0));
        let mut fade = crate::gfx::Fade::default();
        fade.take(g, top + rows as f32 * (ch + GAP) - GAP + crate::gfx::RING_ROOM, foot);
        let mut focused_cell = None;
        for (i, photo) in self.photos.iter().enumerate().take(to).skip(from) {
            let (row, col) = (i / COLS, i % COLS);
            let y = top + (row as f32 - first) * (ch + GAP);
            if y + ch < top - 2.0 || y > foot {
                continue;
            }
            let r = Rect::new(PAD + col as f32 * (cw + GAP), y, cw, ch);
            match self.thumbs.get(&photo.name) {
                Some(thumb) => {
                    // Fade out at the top edge rather than cut (at the foot, the band does).
                    let edge = ((r.bottom() - top) / 8.0).clamp(0.0, 1.0);
                    if is_pixel(&photo.name) {
                        draw_picture_crisp(g, r, &thumb, whole(&thumb), radius, edge);
                    } else {
                        draw_picture(g, r, &thumb, whole(&thumb), radius, edge);
                    }
                    // A loop says so in its corner.
                    if is_loop(&photo.name) {
                        let cap = g.ty.caption;
                        let h = cap.size + 4.0;
                        let w = (h + g.measure("loop", cap) + 4.0).round();
                        let b = Rect::new(r.x + 3.0, r.bottom() - 3.0 - h, w, h);
                        g.canvas.rrect(b.offset(g.ox, 0.0), h * 0.5, bezel::Color(0, 0, 0), 0.6 * edge);
                        g.icon_alpha("animation", false, b.x + h * 0.5 + 1.0, b.cy(), h * 0.7, bezel::Color(255, 255, 255), edge);
                        g.text_alpha(b.x + h, b.cy(), "loop", cap, bezel::Color(255, 255, 255), edge);
                    }
                }
                None => {
                    g.surface(r, radius, t.surface1);
                    let icon = if self.thumbs.failed(&photo.name) { "broken_image" } else { "photo" };
                    g.icon(icon, false, r.cx(), r.cy(), 16.0, t.faint);
                }
            }
            if i == self.focus {
                focused_cell = Some(r);
            }
        }
        g.canvas.clear_clip();
        fade.apply(g);
        if let Some(r) = focused_cell {
            g.focus_ring(r, radius);
        }
        let photo = &self.photos[self.focus];
        if self.deleting == Some(self.focus) {
            g.hint("d again deletes this photo · esc: keep it");
        } else {
            // Where it is, and the way to a phone where that fits too.
            let at = format!("{} of {} · {}", self.focus + 1, self.photos.len(), when(photo.taken, g.clock.0));
            let phone = format!("{at} · p: phone");
            g.hint_any(&[&phone, &at]);
        }
    }
}

// ----------------------------------------------------------------- viewer ---

/// One photo on the whole panel.
pub struct Viewer {
    photos: Vec<Photo>,
    at: usize,
    /// The photo decoded, at the size `scale` was asked for, and which.
    shown: Option<(usize, Picture)>,
    /// Its thumbnail, shown until it is decoded.
    placeholder: Option<(usize, Picture)>,
    decoding: Option<(usize, usize, Receiver<Result<Picture, String>>)>,
    trouble: Option<String>,
    dims: Option<(usize, usize)>,
    /// How far in (1, 2 or 4) and where the middle of the view is (0..1 of
    /// the photo each way).
    zoom: Animated,
    pan_x: Animated,
    pan_y: Animated,
    /// A photo arriving from the side it was gone to.
    slide: Animated,
    /// Seconds the date and count show for after a change.
    chrome: f32,
    details: bool,
    deleting: bool,
    /// The photo's look (from its Exif), whether the photo as taken is kept
    /// beside it and shown now (`o`), and whether it has a RAW.
    look: Option<String>,
    has_original: bool,
    original: bool,
    has_raw: bool,
    /// A pixel picture: its true size shown, with whole pixels.
    pixel: bool,
    /// A loop's frames (each with its time, ms), the one in sight, and
    /// how long it has left.
    frames: Vec<(Picture, u32)>,
    frame_at: usize,
    frame_left: f32,
    /// Under the photo: its caption and place (photolog/).
    about: crate::photolog::Under,
}

const ZOOMS: [f32; 3] = [1.0, 2.0, 4.0];

impl Viewer {
    pub fn new(photos: Vec<Photo>, at: usize) -> Self {
        let mut v = Viewer {
            photos,
            at,
            shown: None,
            placeholder: None,
            decoding: None,
            trouble: None,
            dims: None,
            zoom: Animated::new(1.0, role::settle()).with_rest(0.001),
            pan_x: Animated::new(0.5, role::settle()).with_rest(0.0005),
            pan_y: Animated::new(0.5, role::settle()).with_rest(0.0005),
            slide: Animated::new(0.0, role::page()).with_rest(0.002),
            chrome: 2.5,
            details: false,
            deleting: false,
            look: None,
            has_original: false,
            original: false,
            has_raw: false,
            pixel: false,
            frames: Vec::new(),
            frame_at: 0,
            frame_left: 0.0,
            about: crate::photolog::Under::new(),
        };
        v.arrive();
        v
    }

    /// The file in sight: the photo, or the photo as taken (a pixel
    /// picture's true size).
    fn shown_path(&self, photo: &Photo) -> PathBuf {
        if self.pixel {
            pixel_true_size(&photo.path)
        } else if self.original {
            crate::filters::original_of(&photo.path)
        } else {
            photo.path.clone()
        }
    }

    /// A single photo (opened from Files).
    pub fn single(path: PathBuf) -> Self {
        let meta = std::fs::metadata(&path).ok();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let photo = Photo { taken: taken_of(&name, meta.as_ref().and_then(|m| m.modified().ok())), bytes: meta.map_or(0, |m| m.len()), name, path };
        Self::new(vec![photo], 0)
    }

    /// A photo just turned to: its thumbnail now, its size from its header.
    fn arrive(&mut self) {
        let Some(photo) = self.photos.get(self.at) else { return };
        self.trouble = None;
        // The last photo's decode goes: one photo decoded at a time.
        self.shown = None;
        self.pixel = is_pixel(&photo.name);
        self.dims = dimensions(&if self.pixel { pixel_true_size(&photo.path) } else { photo.path.clone() });
        self.placeholder = read_thumb(&thumb_file(photo), photo.bytes).map(|t| (self.at, t));
        self.chrome = 2.5;
        self.deleting = false;
        self.look = crate::filters::look_of(&photo.path);
        // A pixel picture's original is its true size, which is what shows.
        self.has_original = !self.pixel && crate::filters::original_of(&photo.path).is_file();
        // A loop: its frames at their true size (small: read at once), played.
        self.frames.clear();
        if is_loop(&photo.name) {
            let path = if self.pixel { pixel_true_size(&photo.path) } else { photo.path.clone() };
            match loop_frames(&path) {
                Ok(frames) => {
                    self.dims = frames.first().map(|(p, _)| (p.w, p.h));
                    self.shown = frames.first().map(|(p, _)| (self.at, p.clone()));
                    self.frame_left = frames.first().map_or(0.0, |(_, d)| *d as f32);
                    self.frames = frames;
                }
                Err(why) => self.trouble = Some(why),
            }
            self.frame_at = 0;
        }
        self.has_raw = photo.path.with_extension("dng").is_file();
        self.original = false;
        self.about.arrive(&photo.path);
    }

    /// How large a decode the zoom asks for: the panel's size at that zoom.
    fn wanted(&self) -> (usize, usize) {
        let z = self.zoom.target();
        ((W * z).ceil() as usize, (H * z).ceil() as usize)
    }

    fn step_zoom(&mut self, by: isize) {
        let now = ZOOMS.iter().position(|z| *z >= self.zoom.target() - 0.01).unwrap_or(0) as isize;
        let to = (now + by).clamp(0, ZOOMS.len() as isize - 1) as usize;
        self.zoom.set(ZOOMS[to]);
        if to == 0 {
            self.pan_x.set(0.5);
            self.pan_y.set(0.5);
        }
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        // A step is a fifth of what is in sight; never past where the photo's
        // edge meets the panel's.
        let (pw, ph) = self.dims.unwrap_or((4, 3));
        let (fx, fy) = in_sight_of(pw, ph, self.zoom.target(), self.pixel);
        let (hx, hy) = (fx * 0.5, fy * 0.5);
        self.pan_x.set((self.pan_x.target() + dx * fx * 0.2).clamp(hx, 1.0 - hx));
        self.pan_y.set((self.pan_y.target() + dy * fy * 0.2).clamp(hy, 1.0 - hy));
    }

    fn turn(&mut self, by: isize) {
        let to = self.at as isize + by;
        if to < 0 || to >= self.photos.len() as isize {
            self.slide.nudge(-by as f32 * 2.0);
            return;
        }
        self.at = to as usize;
        self.slide.jump(by as f32);
        self.slide.set(0.0);
        self.arrive();
    }
}

impl Screen for Viewer {
    fn title(&self) -> &str {
        "Photo"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    /// Back from the editor: an export it made is in the album beside the
    /// photo, which stays in sight.
    fn resume(&mut self) {
        let Some(here) = self.photos.get(self.at).map(|p| p.path.clone()) else { return };
        if self.photos.len() > 1 || here.parent() == Some(dir().as_path()) {
            let fresh = list();
            if let Some(at) = fresh.iter().position(|p| p.path == here) {
                self.photos = fresh;
                self.at = at;
            }
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let zoomed = self.zoom.target() > 1.01;
        let deleting = std::mem::take(&mut self.deleting);
        // Its caption and place, under it (↓, t, n, a).
        if !zoomed && !deleting && !self.details {
            if let Some(nav) = self.about.key(key, cx) {
                return nav;
            }
        }
        match key {
            // Esc keeps a photo it was asked about, puts the details away,
            // and comes back out to the whole photo, before it leaves.
            Key::Back if deleting => {}
            Key::Back if self.details => self.details = false,
            Key::Back | Key::Leave if zoomed => self.step_zoom(-(ZOOMS.len() as isize)),
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Left if zoomed => self.pan(-1.0, 0.0),
            Key::Right if zoomed => self.pan(1.0, 0.0),
            Key::Up if zoomed => self.pan(0.0, -1.0),
            Key::Down if zoomed => self.pan(0.0, 1.0),
            Key::Left => self.turn(-1),
            Key::Right => self.turn(1),
            Key::Char('+' | '=') | Key::Enter => self.step_zoom(1),
            Key::Char('-' | '_') => self.step_zoom(-1),
            Key::Char('i') => {
                self.details = !self.details;
                self.chrome = 2.5;
            }
            // The photo as the camera took it, before its look, and back.
            Key::Char('o') if self.has_original => {
                self.original = !self.original;
                self.shown = None;
                self.decoding = None;
                self.trouble = None;
                self.chrome = 2.5;
            }
            Key::Char('o') if self.pixel => *cx.say = Some("this is its true size, kept beside the copy to share".into()),
            Key::Char('o') => *cx.say = Some("no look: this is the original".into()),
            // A pixel picture keeps its pixels: nothing to straighten or tone.
            Key::Char('e') if self.pixel => *cx.say = Some("a pixel picture keeps its pixels as taken".into()),
            // The editor (1.0): its edits kept beside the photo, never in it.
            Key::Char('e') => {
                if let Some(photo) = self.photos.get(self.at) {
                    return Nav::Push(Box::new(crate::editor::Editor::open(photo.path.clone())));
                }
            }
            Key::Char('d') => {
                if deleting {
                    let photo = self.photos[self.at].clone();
                    match delete(&photo) {
                        Ok(()) => {
                            self.photos.remove(self.at);
                            if self.photos.is_empty() {
                                *cx.say = Some("photo deleted".into());
                                return Nav::Pop;
                            }
                            self.at = self.at.min(self.photos.len() - 1);
                            self.shown = None;
                            self.arrive();
                            *cx.say = Some("photo deleted".into());
                        }
                        Err(e) => *cx.say = Some(format!("could not delete it: {e}")),
                    }
                } else {
                    self.deleting = true;
                    self.chrome = 3.0;
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        if self.deleting {
            return vec![keys("deleting", &[("d", "Delete it"), ("esc", "Keep it")])];
        }
        let mut v = Vec::new();
        if self.zoom.target() > 1.01 {
            v.push(keys("zoomed in", &[("↑ ↓ ← →", "Look around"), ("esc", "The whole photo")]));
        } else {
            v.push(keys("photo", &[("← →", "Last, next photo")]));
        }
        v.push(keys("zoom", &[("enter · +", "Closer"), ("-", "Further out")]));
        if self.zoom.target() <= 1.01 {
            v.extend(self.about.key_groups());
        }
        if self.pixel {
            v.push(keys("this picture", &[("i", "Details"), ("d twice", "Delete it")]));
            return v;
        }
        v.push(keys("this photo", &[("e", "Edit it"), ("i", "Details"), ("d twice", "Delete it")]));
        if self.has_original {
            v.push(keys("its look", &[("o", "As taken, or the look")]));
        }
        v
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.zoom.tick(dt) | self.pan_x.tick(dt) | self.pan_y.tick(dt) | self.slide.tick(dt) | self.about.tick(dt);
        if self.chrome > 0.0 && !self.details && !self.deleting {
            self.chrome -= dt;
            changed |= true;
        }
        // A decode finished.
        if let Some((at, _, rx)) = &self.decoding {
            if let Ok(result) = rx.try_recv() {
                let at = *at;
                self.decoding = None;
                match result {
                    Ok(pic) if at == self.at => self.shown = Some((at, pic)),
                    Ok(_) => {}
                    Err(why) if at == self.at => self.trouble = Some(why),
                    Err(_) => {}
                }
                changed = true;
            }
        }
        // The photo in sight, decoded as large as the zoom needs.
        let (ww, wh) = self.wanted();
        let have = self.shown.as_ref().filter(|(at, _)| *at == self.at).map(|(_, p)| (p.w, p.h));
        let enough = have.is_some_and(|(w, h)| w >= ww.min(self.dims.map_or(ww, |d| d.0)) || h >= wh.min(self.dims.map_or(wh, |d| d.1)));
        // One decode at a time, even turning quickly: the next starts when the
        // last is done (its photo kept only if it is still the one in sight).
        if !enough && self.decoding.is_none() && self.trouble.is_none() {
            if let Some(photo) = self.photos.get(self.at) {
                let (tx, rx) = mpsc::channel();
                let path = self.shown_path(photo);
                std::thread::Builder::new()
                    .name("photo".into())
                    .stack_size(512 * 1024)
                    .spawn(move || {
                        tx.send(decode(&path, ww, wh)).ok();
                    })
                    .ok();
                self.decoding = Some((self.at, ww, rx));
            }
        }
        changed
    }

    fn busy(&self) -> bool {
        self.decoding.is_some() || self.about.busy()
    }

    /// A loop plays at its own pace: looked in on as its next frame is due,
    /// never on the frame beat.
    fn ambient(&self) -> Option<Duration> {
        (self.frames.len() > 1).then(|| Duration::from_secs_f32((self.frame_left / 1000.0).max(0.01)))
    }

    fn ambient_tick(&mut self, dt: f32, _robot: &Robot) -> bool {
        if self.frames.len() < 2 {
            return false;
        }
        self.frame_left -= dt * 1000.0;
        let mut changed = false;
        while self.frame_left <= 0.0 {
            self.frame_at = (self.frame_at + 1) % self.frames.len();
            self.frame_left += self.frames[self.frame_at].1.max(20) as f32;
            changed = true;
        }
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if g.calm {
            for a in [&mut self.zoom, &mut self.pan_x, &mut self.pan_y, &mut self.slide] {
                a.jump(a.target());
            }
        }
        g.canvas.fill_rect(Rect::new(g.ox, 0.0, W, H), bezel::Color(0, 0, 0), 1.0);
        let Some(photo) = self.photos.get(self.at).cloned() else { return };
        // The photo: its decode, else its thumbnail, fitted and zoomed.
        let pic = match (&self.shown, &self.placeholder) {
            // A loop: the frame it has come to.
            _ if !self.frames.is_empty() => self.frames.get(self.frame_at).map(|(p, _)| p),
            (Some((at, p)), _) if *at == self.at => Some(p),
            (_, Some((at, p))) if *at == self.at => Some(p),
            _ => None,
        };
        let offset = self.slide.value() * W;
        // Shrunk into the room above the sheet under it, when that is up
        // (photolog/): by `k`, about the panel's top middle, `down` from it.
        let (k, down) = if self.zoom.target() > 1.01 || self.details { (1.0, 0.0) } else { self.about.room(g) };
        let fit = |r: Rect| Rect::new(W * 0.5 + (r.x - W * 0.5) * k, down + r.y * k, r.w * k, r.h * k);
        // Where the photo is on the panel: the chips go in its corners.
        let mut frame = Rect::new(0.0, 0.0, W, H);
        if let Some(pic) = pic {
            let (pw, ph) = self.dims.unwrap_or((pic.w, pic.h));
            let (dest, view) = zoomed_of(pw, ph, pic, self.zoom.value(), self.pan_x.value(), self.pan_y.value(), self.pixel);
            let at = fit(Rect::new(dest.x + offset, dest.y, dest.w, dest.h));
            if self.pixel {
                draw_picture_crisp(g, at, pic, view, 0.0, 1.0);
            } else {
                draw_picture(g, at, pic, view, 0.0, 1.0);
            }
            frame = fit(dest);
        } else if let Some(why) = &self.trouble {
            let why = g.text.fit(why, &ty.caption, W - 40.0);
            g.icon("broken_image", false, W * 0.5 + offset, H * 0.5 - 14.0, 20.0, t.faint);
            g.text_at(W * 0.5 + offset, H * 0.5 + 10.0, Align::Center, &why, ty.caption, t.dim);
        } else {
            g.icon("photo", false, W * 0.5 + offset, H * 0.5, 22.0, t.faint);
        }

        // The date and where it is in the album, for a moment after a change.
        let shade = self.chrome.clamp(0.0, 1.0).max(if self.details || self.deleting { 1.0 } else { 0.0 });
        // The chips give way as the photo shrinks for the sheet under it.
        let shade = shade * ((k - 0.8) / 0.2).clamp(0.0, 1.0);
        // Where the chips along the photo's foot end, on the left.
        let mut left_end = frame.x;
        if shade > 0.0 {
            let words = when(photo.taken, g.clock.0);
            let count = format!("{} / {}", self.at + 1, self.photos.len());
            let inset = 6.0;
            let (left, right) = (frame.x + inset, frame.right() - inset);
            chip(g, left, frame.y + inset, &words, Align::Left, shade);
            chip(g, right, frame.y + inset, &count, Align::Right, shade);
            if self.zoom.target() > 1.01 {
                let h = g.ty.caption.size + 8.0;
                chip(g, right, frame.bottom() - inset - h, &format!("{:.0}×", self.zoom.target()), Align::Right, shade);
            }
            if !self.details {
                let h = g.ty.caption.size + 8.0;
                let words = match (&self.look, self.original) {
                    (_, true) => Some("original".to_owned()),
                    (Some(look), false) if self.has_original => Some(format!("{} · o: original", look.to_lowercase())),
                    (Some(look), false) => Some(look.to_lowercase()),
                    _ => None,
                };
                if let Some(words) = words {
                    chip(g, left, frame.bottom() - inset - h, &words, Align::Left, shade);
                    left_end = left + g.measure(&words, g.ty.caption) + 14.0;
                }
            }
        }
        if self.zoom.target() <= 1.01 && !self.details && !self.deleting {
            self.about.draw(g, frame, shade, left_end);
        }
        if self.details {
            let dims = self.dims.map_or("–".into(), |(w, h)| format!("{w} × {h}"));
            let mp = self.dims.map_or(String::new(), |(w, h)| format!(" · {:.1} mp", (w * h) as f32 / 1e6));
            // A pixel picture: its true size, and its copy's for sharing.
            let size = if self.pixel {
                // Its copy for sharing, by how many times larger (short
                // enough for the line at the largest text).
                let copy = match (dimensions(&photo.path), self.dims) {
                    (Some((cw, _)), Some((w, _))) if w > 0 && cw > w => format!(" · shared at {}×", cw / w),
                    _ => String::new(),
                };
                format!("{dims} pixels{copy}")
            } else {
                format!("{dims}{mp}")
            };
            // A loop's frames and length, on the line with its bytes.
            let bytes = if self.frames.len() > 1 {
                format!("{} frames · {:.1} s · {}", self.frames.len(), self.frames.iter().map(|(_, d)| *d).sum::<u32>() as f32 / 1000.0, size_words(photo.bytes))
            } else {
                size_words(photo.bytes)
            };
            let mut lines = vec![photo.name.clone(), size, bytes, photo.taken.format("%A %-d %B %Y, %H:%M:%S").to_string()];
            // What it wears and what is kept beside it.
            let edited = !self.pixel && crate::editor::edits::has_edit(&photo.path);
            let kept: Vec<String> = [
                self.look.as_ref().map(|l| format!("look: {}", l.to_lowercase())),
                self.has_original.then(|| "o: the original".to_owned()),
                self.has_raw.then(|| "raw kept (.dng)".to_owned()),
                (!self.pixel).then(|| if edited { "e: its edit" } else { "e: edit" }.to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !kept.is_empty() {
                lines.push(kept.join(" · "));
            }
            let n = lines.len() as f32;
            let card = Rect::new(PAD, H - PAD - n * (ty.caption.size + 5.0) - 14.0, W - 2.0 * PAD, n * (ty.caption.size + 5.0) + 14.0);
            g.canvas.rrect(card.offset(g.ox, 0.0), g.radius(10.0), t.surface1, 1.0);
            for (i, line) in lines.iter().enumerate() {
                let fitted = g.text.fit(line, &ty.caption, card.w - 20.0);
                let color = if i == 0 { t.ink } else { t.dim };
                g.text_at(card.x + 10.0, card.y + 7.0 + (i as f32 + 0.5) * (ty.caption.size + 5.0), Align::Left, &fitted, ty.caption, color);
            }
        }
        if self.deleting {
            let card = Rect::new(PAD, H - PAD - 44.0, W - 2.0 * PAD, 44.0);
            g.canvas.rrect(card.offset(g.ox, 0.0), g.radius(10.0), t.surface1, 1.0);
            g.icon("delete", false, card.x + 18.0, card.cy(), 16.0, g.fault());
            g.text_at(card.x + 34.0, card.cy() - 7.0, Align::Left, "Delete this photo?", ty.body, t.ink);
            g.text_at(card.x + 34.0, card.cy() + 9.0, Align::Left, "d again: delete · any other key: keep", ty.caption, t.dim);
        }
    }
}

/// How much of a `pw` × `ph` photo is in sight at zoom `z`, as fractions of
/// it each way (1 when the whole of it is).
#[cfg_attr(not(test), allow(dead_code))]
fn in_sight(pw: usize, ph: usize, z: f32) -> (f32, f32) {
    in_sight_of(pw, ph, z, false)
}

/// Panel pixels to a photo's pixel, fitted whole: for a pixel picture, the
/// largest whole number that fits (at least one), so its pixels stay even.
fn fit_scale(pw: usize, ph: usize, pixel: bool) -> f32 {
    let fit = (W / pw as f32).min(H / ph as f32);
    if pixel {
        fit.floor().max(1.0)
    } else {
        fit
    }
}

fn in_sight_of(pw: usize, ph: usize, z: f32, pixel: bool) -> (f32, f32) {
    let s = fit_scale(pw, ph, pixel) * z.max(1.0);
    ((W / (pw as f32 * s)).min(1.0), (H / (ph as f32 * s)).min(1.0))
}

/// Where a photo of `pw` × `ph` goes on the panel at zoom `z` (1: fitted
/// whole), centred on (`px`, `py`) (0..1 of it each way) as far as its edges
/// allow, and which part of `pic` (its decode, any size) shows there.
#[cfg_attr(not(test), allow(dead_code))]
fn zoomed(pw: usize, ph: usize, pic: &Picture, z: f32, px: f32, py: f32) -> (Rect, [f32; 4]) {
    zoomed_of(pw, ph, pic, z, px, py, false)
}

/// As [`zoomed`]; a pixel picture at whole panel pixels, on whole pixels.
fn zoomed_of(pw: usize, ph: usize, pic: &Picture, z: f32, px: f32, py: f32, pixel: bool) -> (Rect, [f32; 4]) {
    let s = fit_scale(pw, ph, pixel) * z.max(1.0);
    let (sw, sh) = (pw as f32 * s, ph as f32 * s);
    // Larger than the panel: as near the pan point as the edges allow;
    // smaller: in the middle.
    let place = |size: f32, room: f32, at: f32| {
        let p = if size > room { (room * 0.5 - at * size).clamp(room - size, 0.0) } else { (room - size) * 0.5 };
        if pixel {
            p.round()
        } else {
            p
        }
    };
    let (left, top) = (place(sw, W, px), place(sh, H, py));
    let dest = Rect::new(left.max(0.0), top.max(0.0), sw.min(W - left.max(0.0)).min(left + sw), sh.min(H - top.max(0.0)).min(top + sh));
    // Panel pixels to the decode's pixels.
    let k = pic.w as f32 / sw;
    (dest, [(dest.x - left) * k, (dest.y - top) * k, dest.w * k, dest.h * k])
}

/// A word on a dark chip over the photo, its edge at `x`.
fn chip(g: &mut Gfx, x: f32, y: f32, words: &str, align: Align, alpha: f32) {
    let ty = g.ty;
    let w = g.measure(words, ty.caption) + 14.0;
    let h = ty.caption.size + 8.0;
    let left = if align == Align::Right { x - w } else { x };
    g.canvas.rrect(Rect::new(left + g.ox, y, w, h), h * 0.5, bezel::Color(0, 0, 0), 0.55 * alpha);
    g.text_alpha(left + 7.0, y + h * 0.5, words, ty.caption, bezel::Color(240, 240, 240), alpha);
}

// ------------------------------------------------------------------- demo ---

/// The simulator's album: a few photos of the simulated camera's scene, at
/// different times of day, made the first time it opens empty (and not
/// again once they are deleted: `.seeded` beside them says so).
#[cfg(not(target_arch = "aarch64"))]
pub fn seed_demo() {
    let dir = dir();
    if crate::sys::on_handheld() || dir.join(".seeded").exists() || !list().is_empty() {
        return;
    }
    std::fs::create_dir_all(&dir).ok();
    // Moments of the simulated camera's day: the light and where the robot
    // has driven to differ from one to the next.
    let days = [("20261120_140312", 0.2), ("20261120_093044", 14.0), ("20261119_181530", 28.0), ("20261119_172201", 36.0), ("20261118_120005", 43.0), ("20261117_101532", 50.0)];
    for (stamp, t) in days {
        let pic = crate::camera::scene(640, 480, t, false);
        let path = dir.join(format!("IMG_{stamp}.jpg"));
        crate::camera::save_jpeg(&path, &pic).ok();
    }
    std::fs::write(dir.join(".seeded"), b"").ok();
    changed();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-photos-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_albums_and_a_photos_keys_fit_the_sheet() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut fits = |s: &dyn Screen| crate::keysheet::fits(&mut g, &s.key_sheet()).unwrap();
        let mut album = Album { photos: Vec::new(), focus: 0, scroll: Animated::new(0.0, role::settle()), thumbs: Thumbs::new(), seen: 0, deleting: None };
        fits(&album);
        album.deleting = Some(0);
        fits(&album);
        let mut v = Viewer::new(Vec::new(), 0);
        fits(&v);
        v.has_original = true;
        v.zoom.jump(2.0);
        fits(&v);
        v.deleting = true;
        fits(&v);
    }

    #[test]
    fn photos_are_named_for_when_they_were_taken_and_never_overwritten() {
        let dir = scratch("names");
        let at = NaiveDateTime::parse_from_str("2026-11-21 14:03:12", "%Y-%m-%d %H:%M:%S").unwrap();
        let first = new_path_in(&dir, at);
        assert_eq!(first.file_name().unwrap(), "IMG_20261121_140312.jpg");
        std::fs::write(&first, b"x").unwrap();
        let second = new_path_in(&dir, at);
        assert_eq!(second.file_name().unwrap(), "IMG_20261121_140312_2.jpg", "the same second: a second name");
        assert_eq!(taken_of("IMG_20261121_140312_2.jpg", None), at);
        assert!(is_photo("IMG_1.JPG") && is_photo("a.jpeg") && !is_photo(".thumbs") && !is_photo("notes.txt") && !is_photo("x.jpg.part"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_album_is_newest_first() {
        let dir = scratch("order");
        for name in ["IMG_20261119_120005.jpg", "IMG_20261121_140312.jpg", "IMG_20261120_181530.jpg", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let names: Vec<String> = list_in(&dir).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["IMG_20261121_140312.jpg", "IMG_20261120_181530.jpg", "IMG_20261119_120005.jpg"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shrinking_averages_and_crops_to_the_new_shape() {
        // A 8 × 4 picture: left half red, right half blue; shrunk to 2 × 2
        // (square) it keeps the middle 4 × 4, half red and half blue.
        let mut pic = Picture::new(8, 4);
        for y in 0..4 {
            for x in 0..8 {
                let at = (y * 8 + x) * 3;
                pic.rgb[at..at + 3].copy_from_slice(if x < 4 { &[200, 0, 0] } else { &[0, 0, 200] });
            }
        }
        let small = pic.cover(2, 2);
        assert_eq!(&small.rgb[0..3], &[200, 0, 0]);
        assert_eq!(&small.rgb[3..6], &[0, 0, 200]);
    }

    fn press(screen: &mut dyn Screen, key: Key) -> Nav {
        let mut robot = Robot::none();
        let mut settings = crate::shell::Settings::default();
        let mut say = None;
        screen.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true })
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn esc_keeps_a_photo_it_was_asked_about_and_comes_out_of_a_zoom_before_leaving() {
        crate::shell::tests::sandbox();
        let dir = scratch("esc");
        let path = dir.join("IMG_20261120_101010.jpg");
        crate::camera::save_jpeg(&path, &crate::camera::scene(320, 240, 1.0, false)).unwrap();
        let mut viewer = Viewer::single(path.clone());
        assert!(matches!(press(&mut viewer, Key::Char('d')), Nav::Stay) && viewer.deleting);
        assert!(matches!(press(&mut viewer, Key::Back), Nav::Stay), "esc keeps it");
        assert!(path.exists() && !viewer.deleting);
        press(&mut viewer, Key::Char('i'));
        assert!(matches!(press(&mut viewer, Key::Back), Nav::Stay) && !viewer.details, "esc puts the details away");
        press(&mut viewer, Key::Char('+'));
        assert!(matches!(press(&mut viewer, Key::Back), Nav::Stay), "out of the zoom first");
        assert!(matches!(press(&mut viewer, Key::Back), Nav::Pop));
        // Asked twice, it goes; it was the only one, so the viewer goes too.
        let mut viewer = Viewer::single(path.clone());
        press(&mut viewer, Key::Char('d'));
        assert!(matches!(press(&mut viewer, Key::Char('d')), Nav::Pop));
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_photo_fits_whole_and_zooms_about_where_it_is_panned_to() {
        // A full-size photo, decoded at an eighth.
        let pic = Picture::new(410, 308);
        let close = |a: f32, b: f32| (a - b).abs() < 0.2;
        // Whole: the panel's height, at its own shape, in the middle.
        let (dest, view) = zoomed(3280, 2464, &pic, 1.0, 0.5, 0.5);
        assert!(close(dest.h, 170.0) && close(dest.w, 226.3) && close(dest.x, (320.0 - 226.3) / 2.0) && close(dest.y, 0.0), "{dest:?}");
        assert!(close(view[0], 0.0) && close(view[2], 410.0) && close(view[3], 308.0), "{view:?}");
        // Twice as close, in the middle: the whole panel, the photo's middle.
        let (dest, view) = zoomed(3280, 2464, &pic, 2.0, 0.5, 0.5);
        assert!(close(dest.x, 0.0) && close(dest.w, 320.0) && close(dest.h, 170.0), "{dest:?}");
        assert!(close(view[0] + view[2] / 2.0, 205.0) && close(view[1] + view[3] / 2.0, 154.0), "{view:?}");
        assert!(view[2] < 410.0 * 0.75, "less of it in sight: {view:?}");
        // Panned past an edge: it stops at the edge, never showing beyond it.
        let (_, view) = zoomed(3280, 2464, &pic, 4.0, 0.0, 1.0);
        assert!(close(view[0], 0.0) && close(view[1] + view[3], 308.0), "{view:?}");
        // How much is in sight: all of it fitted, a quarter of its width at 4×
        // (it is narrower than the panel, so a little more than that).
        assert_eq!(in_sight(3280, 2464, 1.0), (1.0, 1.0));
        let (sx, sy) = in_sight(3280, 2464, 4.0);
        assert!(close(sy, 0.25) && sx > 0.25 && sx < 0.4, "{sx} {sy}");
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn a_photo_is_decoded_at_the_size_needed_and_its_thumbnail_kept() {
        crate::shell::tests::sandbox();
        let dir = scratch("decode");
        let pic = crate::camera::scene(1640, 1232, 1.0, false);
        let path = dir.join("IMG_20261121_140312.jpg");
        crate::camera::save_jpeg(&path, &pic).unwrap();
        assert_eq!(dimensions(&path), Some((1640, 1232)));
        // The panel's size asks for an eighth (205 × 154 is at least 160 × 120 ...).
        let small = decode(&path, 160, 120).unwrap();
        assert_eq!((small.w, small.h), (205, 154), "an eighth: the smallest that is large enough");
        let half = decode(&path, 700, 500).unwrap();
        assert_eq!((half.w, half.h), (820, 616));
        // Colours survive the trip (the scene's sky at the top is blue-ish).
        let sky = &small.rgb[(5 * small.w + 100) * 3..(5 * small.w + 100) * 3 + 3];
        assert!(sky[2] > sky[0], "{sky:?}");
        // A thumbnail, made once and then read back.
        let bytes = std::fs::metadata(&path).unwrap().len();
        let photo = Photo { path: path.clone(), name: "IMG_20261121_140312.jpg".into(), taken: taken_of("IMG_20261121_140312.jpg", None), bytes };
        let made = thumbnail(&photo).unwrap();
        assert_eq!((made.w, made.h), (THUMB_W, THUMB_H));
        assert_eq!(read_thumb(&thumb_file(&photo), bytes), Some(made.clone()));
        assert_eq!(read_thumb(&thumb_file(&photo), bytes + 1), None, "a changed photo gets a new one");
        delete(&photo).unwrap();
        assert!(!path.exists() && !thumb_file(&photo).exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
