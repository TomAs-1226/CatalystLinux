//! Objects in three dimensions, rendered beforehand: the app icons on Home,
//! setup's objects, the moon of deep standby. The handheld only lays down
//! pictures; which picture, and where, is the springs' business.
//!
//! Each object was rendered in Blender (`tools/bake3d`) from every angle it
//! is ever shown at, under Bezel's one light from above and to the left,
//! with soft shadows and ambient occlusion, and kept in `objects.pack` as
//! four planes a view: how light it is there (a neutral grey render), how
//! much of it takes each of its two tints, how much of the pixel it covers,
//! and how dark its shadow makes what is under it. Colour is put on here,
//! from the palette in use, so one render serves dark and light, every
//! accent and the user's own colour: a surface of the shell takes a tone of
//! the palette's neutrals, a part in the accent takes the signal, a part in
//! its second tint takes ice or amber.
//!
//! Frames are decoded when first wanted, coloured, and kept in a small
//! cache (`CACHE_BYTES`), the least recently used let go first; the pack
//! itself stays in the program file and is read from there. A view between
//! two rendered angles is the two frames blended, so an object turns
//! smoothly however few angles it carries.

use std::collections::HashMap;
use std::io::Cursor;

use bezel::{Canvas, Color, Tokens, Tone};

/// The objects the shell carries (`tools/bake3d/pack.py` makes it).
pub static SHIPPED: &[u8] = include_bytes!("../../../assets/objects/objects.pack");

/// What the decoded, coloured frames may take at most, in bytes. A Home
/// page's six icons at rest are 22 kB; the focused one's every angle 97 kB;
/// a hero object turning all the way round would be 810 kB, so its frames
/// come and go as it turns.
pub const CACHE_BYTES: usize = 384 * 1024;

/// The neutral lightness the two kinds of surface were rendered at
/// (`tools/bake3d/kit.py`): a palette colour is anchored there, so a surface
/// facing the light shows the palette's colour itself.
const BODY_ALBEDO: f32 = 0.62;
const ACCENT_ALBEDO: f32 = 0.55;

/// What a tint takes from the palette.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Signal,
    Ice,
    Amber,
    Fault,
    Ink,
}

impl Role {
    fn from_byte(b: u8) -> Role {
        match b {
            1 => Role::Ice,
            2 => Role::Amber,
            3 => Role::Fault,
            4 => Role::Ink,
            _ => Role::Signal,
        }
    }
}

/// Which way an object is turned, in degrees from where it rests: `yaw`
/// about the vertical (positive turns its front to the right), `pitch` the
/// camera rising over it (positive looks down on it more).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
}

impl View {
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub const REST: View = View { yaw: 0.0, pitch: 0.0 };

    pub fn yaw(yaw: f32) -> View {
        View { yaw, pitch: 0.0 }
    }
}

struct Size {
    label: String,
    px: u16,
    yaws: Vec<i16>,
    pitches: Vec<i8>,
    /// Where each frame's PNG lies in the pack, pitch by pitch, yaw by yaw.
    frames: Vec<(u32, u32)>,
    /// The yaws go all the way round (a turntable), so the last blends into the first.
    wraps: bool,
}

struct Entry {
    name: String,
    tints: [Role; 2],
    sizes: Vec<Size>,
}

/// An index of a pack: what it holds and where.
pub struct Pack {
    data: &'static [u8],
    entries: Vec<Entry>,
    blobs: usize,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len()).ok_or("the pack is cut short")?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn text(&mut self) -> Result<String, String> {
        let n = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

impl Pack {
    /// Read a pack's index (the frames stay where they are until wanted).
    pub fn parse(data: &'static [u8]) -> Result<Pack, String> {
        let mut r = Reader { data, at: 0 };
        if r.take(4)? != b"C3DP" {
            return Err("not a pack of objects".into());
        }
        if r.u16()? != 1 {
            return Err("a pack of another version".into());
        }
        let n = r.u16()? as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.text()?;
            let tints = [Role::from_byte(r.u8()?), Role::from_byte(r.u8()?)];
            // Where it stands in its frame (the middle of its footprint), as
            // a fraction: kept in the pack, not needed yet.
            r.take(4)?;
            let count = r.u8()? as usize;
            let mut sizes = Vec::with_capacity(count);
            for _ in 0..count {
                let label = r.text()?;
                let px = r.u16()?;
                let (ny, np) = (r.u8()? as usize, r.u8()? as usize);
                let yaws: Vec<i16> = (0..ny).map(|_| r.u16().map(|v| v as i16)).collect::<Result<_, _>>()?;
                let pitches: Vec<i8> = (0..np).map(|_| r.u8().map(|v| v as i8)).collect::<Result<_, _>>()?;
                let frames = (0..ny * np).map(|_| Ok((r.u32()?, r.u32()?))).collect::<Result<Vec<_>, String>>()?;
                let wraps = ny > 2 && {
                    let step = (yaws[1] - yaws[0]) as i32;
                    (yaws[ny - 1] - yaws[0]) as i32 + step == 360
                };
                sizes.push(Size { label, px, yaws, pitches, frames, wraps });
            }
            entries.push(Entry { name, tints, sizes });
        }
        let blobs = r.u32()? as usize;
        if blobs > data.len() {
            return Err("the pack is cut short".into());
        }
        Ok(Pack { data, entries, blobs })
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// The names of the objects in the pack.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The pack's size, in bytes: what it costs the program file.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn bytes(&self) -> usize {
        self.data.len()
    }
}

/// A frame decoded and coloured: `0xKKRRGGBB`, colour premultiplied by
/// cover, `KK` what is left of what is under it.
struct Frame {
    px: Vec<u32>,
    used: u64,
}

/// The palette's colours as objects take them.
#[derive(Clone, Copy, PartialEq)]
struct Palette {
    tone: Tone,
    body: Color,
    signal: Color,
    ice: Color,
    amber: Color,
    fault: Color,
    ink: Color,
    shadow: f32,
}

impl Palette {
    fn of(t: &Tokens, fault: Color) -> Palette {
        match t.tone {
            // Dark: the shell is a pale cool grey, from the palette's own
            // neutrals; light: near white, a touch under the tiles so it
            // still reads on them. (Our own tuning.)
            Tone::Dark => Palette {
                tone: t.tone,
                body: t.ink.mix(t.surface3, 0.28),
                signal: t.signal,
                ice: t.ice,
                amber: t.amber,
                fault,
                ink: t.ink,
                shadow: 0.85,
            },
            Tone::Light => Palette {
                tone: t.tone,
                body: t.surface1.mix(t.surface3, 0.18),
                signal: t.signal,
                // Ice is a fill too pale to see on a white object: deeper.
                ice: t.ice.mix(t.on_ice, 0.3),
                amber: t.amber.mix(t.amber_ink, 0.2),
                fault,
                ink: t.ink,
                shadow: 0.55,
            },
        }
    }

    fn role(&self, role: Role) -> Color {
        match role {
            Role::Signal => self.signal,
            Role::Ice => self.ice,
            Role::Amber => self.amber,
            Role::Fault => self.fault,
            Role::Ink => self.ink,
        }
    }
}

fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// What each shade (0 to 255) looks like on a surface of `colour`: the
/// neutral render relit with it, so a face toward the light shows `colour`
/// and one turned away a darker tone of it; overexposure (a highlight, a
/// lit part) bleeds toward white rather than shifting its hue.
fn ramp(colour: Color, albedo: f32) -> [[u8; 3]; 256] {
    let c = [colour.0, colour.1, colour.2].map(|v| to_linear(v as f32 / 255.0) / albedo);
    let mut out = [[0u8; 3]; 256];
    for (s, o) in out.iter_mut().enumerate() {
        let lin = to_linear(s as f32 / 255.0);
        let v = c.map(|k| k * lin);
        let m = v[0].max(v[1]).max(v[2]);
        let over = ((m - 1.0) / 1.5).clamp(0.0, 1.0);
        let base = v.map(|k| k / m.max(1.0));
        *o = base.map(|b| (to_srgb(b + (1.0 - b) * over) * 255.0 + 0.5) as u8);
    }
    out
}

/// The objects, and the frames of them already coloured for the palette in use.
pub struct Objects {
    pack: Option<Pack>,
    palette: Option<Palette>,
    ramps: HashMap<Role, [[u8; 3]; 256]>,
    body: [[u8; 3]; 256],
    frames: HashMap<(u16, u8, u16), Frame>,
    bytes: usize,
    clock: u64,
    /// The most the cache has held, and how many frames were decoded: for
    /// the gallery's account of what objects cost.
    pub peak: usize,
    pub decoded: usize,
    /// Kept between frames, so drawing allocates nothing: the frames mixed,
    /// their sums, and the mix moved or scaled.
    scratch: Vec<u32>,
    acc: Vec<[u32; 4]>,
    moved: Vec<u32>,
}

impl Default for Objects {
    fn default() -> Self {
        Objects::shipped()
    }
}

impl Objects {
    /// The objects the shell carries.
    pub fn shipped() -> Objects {
        let pack = Pack::parse(SHIPPED).inspect_err(|why| eprintln!("catalyst-shell: no 3D objects: {why}")).ok();
        Objects::from(pack)
    }

    /// Another pack (the previews' larger one), leaked to live as long as the program.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn from_bytes(data: Vec<u8>) -> Result<Objects, String> {
        Objects::from_static(Box::leak(data.into_boxed_slice()))
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn from_static(data: &'static [u8]) -> Result<Objects, String> {
        Ok(Objects::from(Some(Pack::parse(data)?)))
    }

    fn from(pack: Option<Pack>) -> Objects {
        Objects {
            pack,
            palette: None,
            ramps: HashMap::new(),
            body: [[0; 3]; 256],
            frames: HashMap::new(),
            bytes: 0,
            clock: 0,
            peak: 0,
            decoded: 0,
            scratch: Vec::new(),
            acc: Vec::new(),
            moved: Vec::new(),
        }
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn pack(&self) -> Option<&Pack> {
        self.pack.as_ref()
    }

    /// Whether there is an object of this name.
    pub fn has(&self, name: &str) -> bool {
        self.pack.as_ref().is_some_and(|p| p.find(name).is_some())
    }

    /// An object's size in pixels at a size label ("s", "l", "m", "hero").
    pub fn px(&self, name: &str, size: &str) -> Option<f32> {
        let p = self.pack.as_ref()?;
        let e = &p.entries[p.find(name)?];
        e.sizes.iter().find(|s| s.label == size).map(|s| s.px as f32)
    }

    /// What the coloured frames now take, in bytes.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn cached(&self) -> usize {
        self.bytes
    }

    /// Follow the palette: a new one colours every frame afresh.
    fn follow(&mut self, tokens: &Tokens, fault: Color) {
        let palette = Palette::of(tokens, fault);
        if self.palette == Some(palette) {
            return;
        }
        self.palette = Some(palette);
        self.frames.clear();
        self.bytes = 0;
        self.body = ramp(palette.body, BODY_ALBEDO);
        self.ramps = [Role::Signal, Role::Ice, Role::Amber, Role::Fault, Role::Ink].into_iter().map(|r| (r, ramp(palette.role(r), ACCENT_ALBEDO))).collect();
    }

    /// The frame `index` of size `size` of object `entry`, decoded and coloured.
    fn frame(&mut self, entry: usize, size: usize, index: usize) -> Option<&Frame> {
        self.clock += 1;
        let key = (entry as u16, size as u8, index as u16);
        if !self.frames.contains_key(&key) {
            let pack = self.pack.as_ref()?;
            let e = &pack.entries[entry];
            let s = &e.sizes[size];
            let (at, len) = s.frames[index];
            let start = pack.blobs + at as usize;
            let png = pack.data.get(start..start + len as usize)?;
            let planes = decode(png, s.px as usize)?;
            let palette = self.palette?;
            let (a, b) = (self.ramps[&e.tints[0]], self.ramps[&e.tints[1]]);
            let px = colour(&planes, &self.body, &a, &b, palette.shadow);
            let bytes = px.len() * 4;
            // Room for it: the least recently used frames go first.
            while self.bytes + bytes > CACHE_BYTES && !self.frames.is_empty() {
                let oldest = *self.frames.iter().min_by_key(|(_, f)| f.used).map(|(k, _)| k).unwrap();
                if let Some(f) = self.frames.remove(&oldest) {
                    self.bytes -= f.px.len() * 4;
                }
            }
            self.bytes += bytes;
            self.peak = self.peak.max(self.bytes);
            self.decoded += 1;
            self.frames.insert(key, Frame { px, used: 0 });
        }
        let f = self.frames.get_mut(&key)?;
        f.used = self.clock;
        Some(f)
    }

    /// Draw an object centred on (`cx`, `cy`), turned to `view`, `scale`
    /// times its size, faded by `alpha`. False if there is no such object
    /// (the caller draws something else).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, canvas: &mut Canvas, tokens: &Tokens, fault: Color, name: &str, size: &str, cx: f32, cy: f32, view: View, scale: f32, alpha: f32) -> bool {
        let Some(pack) = &self.pack else { return false };
        let Some(entry) = pack.find(name) else { return false };
        let Some(si) = pack.entries[entry].sizes.iter().position(|s| s.label == size) else { return false };
        if alpha <= 0.0 || scale <= 0.0 {
            return true;
        }
        let (px, weights) = {
            let s = &pack.entries[entry].sizes[si];
            (s.px as usize, blend(s, view))
        };
        self.follow(tokens, fault);
        // One frame at its own size and on whole pixels is laid straight
        // down; otherwise the frames are mixed (and moved or scaled) first.
        let n = px * px;
        let mut mixed = std::mem::take(&mut self.scratch);
        mixed.clear();
        mixed.resize(n, 0);
        if weights.len() > 1 {
            // Weights in 256ths that add up to 256 exactly, so a blend of
            // equal frames is that frame.
            let mut w256: Vec<u32> = weights.iter().map(|(_, w)| (w * 256.0).round() as u32).collect();
            let sum: u32 = w256.iter().sum();
            w256[0] = (w256[0] + 256).saturating_sub(sum);
            let mut acc = std::mem::take(&mut self.acc);
            acc.clear();
            acc.resize(n, [0u32; 4]);
            for (&(index, _), &w) in weights.iter().zip(&w256) {
                let Some(f) = self.frame(entry, si, index) else { return false };
                for (a, p) in acc.iter_mut().zip(&f.px) {
                    a[0] += ((p >> 16) & 0xff) * w;
                    a[1] += ((p >> 8) & 0xff) * w;
                    a[2] += (p & 0xff) * w;
                    a[3] += (p >> 24) * w;
                }
            }
            for (m, a) in mixed.iter_mut().zip(&acc) {
                let c = a.map(|v| ((v + 128) >> 8).min(255));
                *m = c[3] << 24 | c[0] << 16 | c[1] << 8 | c[2];
            }
            self.acc = acc;
        } else {
            let Some(f) = self.frame(entry, si, weights[0].0) else { return false };
            mixed.copy_from_slice(&f.px);
        }
        let size = px as f32 * scale;
        let (x, y) = (cx - size * 0.5, cy - size * 0.5);
        if (scale - 1.0).abs() < 1e-3 && (x - x.round()).abs() < 0.02 && (y - y.round()).abs() < 0.02 {
            canvas.blit_premultiplied(x.round() as i32, y.round() as i32, px as i32, px as i32, &mixed, alpha);
        } else {
            let (ox, oy, w) = resample(&mixed, px, x, y, scale, &mut self.moved);
            canvas.blit_premultiplied(ox, oy, w as i32, w as i32, &self.moved, alpha);
        }
        self.scratch = mixed;
        true
    }
}

/// The frames a view is made of and how much of each: the two nearest
/// rendered yaws (and pitches), linearly. Within a fiftieth of a step of a
/// rendered angle, that frame alone.
fn blend(s: &Size, view: View) -> Vec<(usize, f32)> {
    let along = |angles: &[f32], v: f32, wraps: bool| -> [(usize, f32); 2] {
        let n = angles.len();
        if n == 1 {
            return [(0, 1.0), (0, 0.0)];
        }
        let v = if wraps { (v - angles[0]).rem_euclid(360.0) + angles[0] } else { v.clamp(angles[0], angles[n - 1]) };
        let mut i = angles.iter().rposition(|a| *a <= v).unwrap_or(0);
        if !wraps {
            i = i.min(n - 2);
        }
        let (a0, a1, j) = if i + 1 < n { (angles[i], angles[i + 1], i + 1) } else { (angles[i], angles[0] + 360.0, 0) };
        let mut t = ((v - a0) / (a1 - a0)).clamp(0.0, 1.0);
        if t < 0.02 {
            t = 0.0;
        } else if t > 0.98 {
            t = 1.0;
        }
        [(i, 1.0 - t), (j, t)]
    };
    let yaws: Vec<f32> = s.yaws.iter().map(|v| *v as f32).collect();
    let pitches: Vec<f32> = s.pitches.iter().map(|v| *v as f32).collect();
    let ya = along(&yaws, view.yaw, s.wraps);
    let pa = along(&pitches, view.pitch, false);
    let mut out: Vec<(usize, f32)> = Vec::with_capacity(4);
    for (p, wp) in pa {
        for (y, wy) in ya {
            let w = wp * wy;
            if w > 1.0 / 64.0 {
                let index = p * yaws.len() + y;
                match out.iter_mut().find(|(i, _)| *i == index) {
                    Some(e) => e.1 += w,
                    None => out.push((index, w)),
                }
            }
        }
    }
    out
}

/// A frame's PNG back into its four planes, interleaved.
fn decode(png: &[u8], px: usize) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; px * px * 4];
    let info = reader.next_frame(&mut buf).ok()?;
    (info.width as usize == px && info.height as usize == px && info.color_type == png::ColorType::Rgba).then_some(buf)
}

/// Planes to premultiplied colour, by the palette's ramps.
fn colour(planes: &[u8], body: &[[u8; 3]; 256], a: &[[u8; 3]; 256], b: &[[u8; 3]; 256], shadow: f32) -> Vec<u32> {
    let shadow = (shadow * 256.0) as u32;
    planes
        .chunks_exact(4)
        .map(|p| {
            let (s, tint, cover, sh) = (p[0] as usize, p[1], p[2] as u32, p[3] as u32);
            let (ta, tb) = ((tint >> 4) as u32 * 17, (tint & 15) as u32 * 17);
            let tn = 255u32.saturating_sub(ta + tb);
            let ch = |k: usize| (body[s][k] as u32 * tn + a[s][k] as u32 * ta + b[s][k] as u32 * tb) / 255;
            let (r, g, bl) = (ch(0) * cover / 255, ch(1) * cover / 255, ch(2) * cover / 255);
            // What is left of what lies under the pixel: none where it is
            // covered, and in the rest, what its shadow leaves.
            let keep = (255 - cover) * (256 - ((sh * shadow) >> 8)) >> 8;
            keep.min(255) << 24 | r << 16 | g << 8 | bl
        })
        .collect()
}

/// A premultiplied picture moved to (`x`, `y`) (not on whole pixels) and
/// scaled, into `out`: sampled between its pixels, in whole numbers, each
/// column's and row's place in the source worked out once. Returns where
/// it now lies and how wide it is.
fn resample(src: &[u32], px: usize, x: f32, y: f32, scale: f32, out: &mut Vec<u32>) -> (i32, i32, usize) {
    let size = px as f32 * scale;
    let (ox, oy) = (x.floor() as i32, y.floor() as i32);
    let w = (size + (x - ox as f32)).ceil() as usize + 1;
    out.clear();
    out.resize(w * w, 0xff00_0000);
    // Where each column's (and row's) middle falls in the source: the
    // pixel before it, and how far on toward the next, in 256ths.
    let axis = |o: i32, start: f32| -> Vec<(i32, u32)> {
        (0..w)
            .map(|i| {
                let s = ((o + i as i32) as f32 + 0.5 - start) / scale - 0.5;
                let f = s.floor();
                (f as i32, ((s - f) * 256.0) as u32)
            })
            .collect()
    };
    let (xs, ys) = (axis(ox, x), axis(oy, y));
    let n = px as i32;
    let get = |sx: i32, sy: i32| -> u32 {
        if sx < 0 || sy < 0 || sx >= n || sy >= n {
            0xff00_0000
        } else {
            src[(sy * n + sx) as usize]
        }
    };
    for (j, &(sy, ty)) in ys.iter().enumerate() {
        if sy < -1 || sy >= n {
            continue;
        }
        for (i, &(sx, tx)) in xs.iter().enumerate() {
            if sx < -1 || sx >= n {
                continue;
            }
            let (a, b, c, d) = (get(sx, sy), get(sx + 1, sy), get(sx, sy + 1), get(sx + 1, sy + 1));
            let lerp = |shift: u32| -> u32 {
                let ch = |p: u32| (p >> shift) & 0xff;
                let top = ch(a) * (256 - tx) + ch(b) * tx;
                let bottom = ch(c) * (256 - tx) + ch(d) * tx;
                ((top * (256 - ty) + bottom * ty + 32768) >> 16).min(255)
            };
            out[j * w + i] = lerp(24) << 24 | lerp(16) << 16 | lerp(8) << 8 | lerp(0);
        }
    }
    (ox, oy, w)
}

/// The object for a built-in app, by its name on Home ("Net tools" is
/// "app-nettools").
pub fn for_app(name: &str) -> String {
    let key: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    format!("app-{key}")
}

/// The object an empty page shows for its icon, and at which size: the
/// app's own where the page belongs to one (an empty folder shows the
/// folder), the empty tray where nothing more particular fits. Icons with
/// no object here (a link gone, Wi-Fi off) stay icons.
pub fn for_empty<'a>(icon: &str, objects: &Objects) -> Option<(&'a str, &'a str)> {
    const BY_ICON: [(&str, &str, &str); 17] = [
        ("storefront", "app-catalogue", "l"),
        ("folder", "app-files", "l"),
        ("insights", "app-graphs", "l"),
        ("trending_up", "app-graphs", "l"),
        ("checklist", "app-pit", "l"),
        ("photo_library", "app-photos", "l"),
        ("event", "app-schedule", "l"),
        ("description", "app-logs", "l"),
        ("precision_manufacturing", "app-motors", "l"),
        ("network_check", "app-nettools", "l"),
        ("straighten", "app-level", "l"),
        ("explore", "app-compass", "l"),
        ("travel_explore", "app-browse", "l"),
        ("account_tree", "app-tables", "l"),
        ("forum", "app-assistant", "l"),
        ("history", "tray", "m"),
        ("search", "tray", "m"),
    ];
    BY_ICON.iter().find(|(i, ..)| *i == icon).filter(|(_, o, s)| objects.px(o, s).is_some()).map(|(_, o, s)| (*o, *s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objects() -> Objects {
        Objects::shipped()
    }

    #[test]
    fn the_pack_reads_and_has_every_apps_object() {
        let o = objects();
        let pack = o.pack().expect("the shipped pack reads");
        assert!(pack.names().count() >= 40, "{} objects", pack.names().count());
        let apps = [
            "Robot", "Health", "Logs", "Schedule", "Tables", "Loop", "Motors", "Console", "Terminal", "Files", "Git", "Catalogue", "Monitor", "Net tools", "Connect", "Actions", "Camera",
            "Photos", "AprilTag", "Graphs", "Pit", "Remote", "Level", "Compass", "Weather", "Browse", "Calculator", "Clock", "Notes", "Units", "HTTP", "Share", "Assistant", "Web", "Settings",
        ];
        for app in apps {
            assert!(o.has(&for_app(app)), "no object for {app}");
            assert_eq!(o.px(&for_app(app), "s"), Some(30.0));
            assert_eq!(o.px(&for_app(app), "l"), Some(52.0));
        }
        for name in ["handheld", "wifi", "key", "toggle", "battery", "robot", "globe", "laptop", "trophy", "tray", "sunmoon"] {
            assert!(o.has(name), "no {name}");
        }
    }

    #[test]
    fn a_view_between_two_angles_blends_the_two_and_a_turn_wraps_round() {
        let o = objects();
        let pack = o.pack().unwrap();
        let icon = &pack.entries[pack.find("app-robot").unwrap()].sizes[1];
        assert_eq!(blend(icon, View::REST).len(), 1, "at rest: one frame");
        let half = blend(icon, View::yaw(6.0));
        assert_eq!(half.len(), 2);
        assert!((half[0].1 - 0.5).abs() < 0.01);
        // Beyond the angles carried: held at the last.
        let past = blend(icon, View::yaw(90.0));
        assert_eq!(past, vec![(icon.yaws.len() - 1, 1.0)]);
        let hero = &pack.entries[pack.find("handheld").unwrap()].sizes[0];
        assert!(hero.wraps);
        let round = blend(hero, View::yaw(352.5));
        let last = hero.yaws.len() - 1;
        assert_eq!(round.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![last, 0]);
        assert_eq!(blend(hero, View::yaw(-15.0)), vec![(last, 1.0)]);
    }

    #[test]
    fn an_object_takes_the_palette_and_its_shadow_darkens_what_is_under_it() {
        let mut o = objects();
        let mut c = Canvas::new(80, 80);
        let ground = Color::hex(0x404040);
        c.clear(ground);
        let dark = Tokens::of(bezel::Accent::Orange, Tone::Dark);
        assert!(o.draw(&mut c, &dark, Color::hex(0xff6b5e), "app-actions", "l", 40.0, 40.0, View::REST, 1.0, 1.0));
        // The play button's triangle is the accent: some pixel is near the signal's hue.
        let signal = dark.signal;
        let near = |p: u32, c: Color| {
            let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
            (r as i32 - c.0 as i32).abs() < 40 && (g as i32 - c.1 as i32).abs() < 40 && (b as i32 - c.2 as i32).abs() < 40
        };
        assert!(c.px.iter().any(|p| near(*p, signal)), "no accent");
        // Its shadow: something darker than the ground beside the object.
        assert!(c.px.iter().any(|p| (p & 0xff) < 0x30), "no shadow");
        // Away from it, the ground is untouched.
        assert_eq!(c.px[0], ground.to_u32());
        // Another accent recolours it.
        let blue = Tokens::of(bezel::Accent::Blue, Tone::Dark);
        c.clear(ground);
        o.draw(&mut c, &blue, Color::hex(0xff6b5e), "app-actions", "l", 40.0, 40.0, View::REST, 1.0, 1.0);
        assert!(c.px.iter().any(|p| near(*p, blue.signal)), "not recoloured");
        assert!(!c.px.iter().any(|p| near(*p, signal) && !near(*p, blue.signal)), "the old accent stayed");
    }

    #[test]
    fn frames_are_kept_within_the_cache() {
        let mut o = objects();
        let mut c = Canvas::new(320, 170);
        let t = Tokens::dark();
        for yaw in (0..720).step_by(5) {
            o.draw(&mut c, &t, Color::BLACK, "handheld", "hero", 160.0, 85.0, View::yaw(yaw as f32), 1.0, 1.0);
        }
        assert!(o.peak <= CACHE_BYTES, "{} bytes", o.peak);
        assert!(o.decoded > 24, "frames came and went: {}", o.decoded);
        // Unknown objects draw nothing and say so.
        assert!(!o.draw(&mut c, &t, Color::BLACK, "no-such-thing", "s", 0.0, 0.0, View::REST, 1.0, 1.0));
    }

    #[test]
    fn moved_or_scaled_it_lands_where_it_was_put() {
        let mut o = objects();
        let t = Tokens::dark();
        let mut a = Canvas::new(120, 120);
        a.clear(Color::BLACK);
        o.draw(&mut a, &t, Color::BLACK, "app-settings", "l", 60.0, 60.0, View::REST, 1.0, 1.0);
        let mut b = Canvas::new(120, 120);
        b.clear(Color::BLACK);
        o.draw(&mut b, &t, Color::BLACK, "app-settings", "l", 60.25, 60.0, View::REST, 1.0, 1.0);
        let centre = |c: &Canvas| {
            let (mut sx, mut n) = (0.0, 0.0);
            for (i, p) in c.px.iter().enumerate() {
                let v = ((p >> 8) & 0xff) as f32;
                sx += v * (i % 120) as f32;
                n += v;
            }
            sx / n
        };
        let d = centre(&b) - centre(&a);
        assert!(d > 0.1 && d < 0.4, "moved by {d}");
        let mut s = Canvas::new(120, 120);
        s.clear(Color::BLACK);
        o.draw(&mut s, &t, Color::BLACK, "app-settings", "l", 60.0, 60.0, View::REST, 1.5, 1.0);
        let lit = |c: &Canvas| c.px.iter().filter(|p| (*p & 0xff) > 0x40).count() as f32;
        let k = lit(&s) / lit(&a);
        assert!(k > 1.8 && k < 2.7, "1.5 times as large: {k} times the pixels");
    }
}
