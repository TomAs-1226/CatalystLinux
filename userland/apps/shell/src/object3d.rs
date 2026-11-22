//! The robot in three dimensions, rendered beforehand: the Robot app's own
//! (Numbers, FRC 5805's), or a team's own made from its CAD. Everything
//! else in the shell is flat. The handheld only lays down pictures; which
//! picture, posed and coloured how, and where, is up to the page and its
//! springs.
//!
//! The robot was rendered in Blender (`tools/bake3d`) from every angle it
//! is shown at, under Bezel's one light from above and to the left, with
//! soft shadows and ambient occlusion, and kept as four planes a view: how
//! light it is there (a neutral grey render), how much of it takes each of
//! its two tints, how much of the pixel it covers, and how dark its shadow
//! makes what is under it; and a fifth, which part each pixel is
//! (drivetrain, intake, hood...). Colour is put on here, from the palette
//! in use (and the robot's own colour, Numbers' blue), so one render serves
//! dark and light and every accent; and a part can be painted (bumpers in
//! the alliance's colour) or glow (a motor running hot, a fault), the whole
//! dimmed (disabled, or not there), and the flywheel shine as it spins.
//!
//! Its mechanisms (the intake's carriage, the hood) were rendered in each
//! of a few steps. The rest pose is kept whole; another pose only where it
//! differs, as patches laid over the rest pose before colouring. A pose
//! between two steps (the intake on its way out) is the two blended, as an
//! angle between two rendered ones is.
//!
//! Frames are decoded when first wanted, coloured, and kept in a small cache
//! (`CACHE_BYTES`), the least recently used let go first; the packs stay
//! where they are (the shell's own in the program file).
//!
//! A pack in `/data/catalyst/robot3d.pack` (a team's own robot, made from
//! its CAD by `tools/bake3d/robot.sh`) is read too, and its objects come
//! before the shell's own: no system update needed to change it.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;

use bezel::{Canvas, Color, Tokens, Tone};

/// The objects the shell carries (`tools/bake3d/pack.py` makes it).
pub static SHIPPED: &[u8] = include_bytes!("../../../assets/objects/objects.pack");

/// What the decoded, coloured frames may take at most, in bytes. The
/// Robot page's robot at one angle is 50 kB (and 12 kB of part numbers),
/// blended between two 125 kB; turning all the way round it would be 5.6
/// MB, so its frames come and go.
pub const CACHE_BYTES: usize = 384 * 1024;

/// The most mechanisms a pose names.
pub const MECHANISMS: usize = 4;

/// The neutral lightness the two kinds of surface were rendered at
/// (`tools/bake3d/kit.py`): a palette colour is anchored there, so a surface
/// facing the light shows the palette's colour itself.
const BODY_ALBEDO: f32 = 0.62;
const ACCENT_ALBEDO: f32 = 0.55;

/// What a tint takes from the palette, or the object's own colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Signal,
    Ice,
    Amber,
    Fault,
    Ink,
    Own,
}

impl Role {
    fn from_byte(b: u8) -> Role {
        match b {
            1 => Role::Ice,
            2 => Role::Amber,
            3 => Role::Fault,
            4 => Role::Ink,
            5 => Role::Own,
            _ => Role::Signal,
        }
    }
}

/// Which way the robot is turned, in degrees from where it rests (`yaw`
/// about the vertical, positive turning its front to the right; `pitch`
/// the camera rising over it), and how it is posed: each mechanism's
/// place among its steps (0 the first; 1.5 half way from the second to the
/// third), in the order the object lists them.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub pose: [f32; MECHANISMS],
}

impl View {
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub const REST: View = View { yaw: 0.0, pitch: 0.0, pose: [0.0; MECHANISMS] };

    pub fn yaw(yaw: f32) -> View {
        View { yaw, ..View::REST }
    }

    pub fn posed(self, pose: [f32; MECHANISMS]) -> View {
        View { pose, ..self }
    }
}

/// How one part of an object looks just now, beyond the palette.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PartLook {
    /// Its tinted surfaces in this colour instead of their tint (bumpers in
    /// the alliance's red or blue).
    pub paint: Option<Color>,
    /// Lit from within in this colour, by this much (0 to 1): a part
    /// running hot in amber, a fault in red, at work in ice.
    pub glow: Option<(Color, f32)>,
}

/// A part shining as it spins (the flywheel): bands of `colour` running
/// across it, `phase` of the way round (0 to 1), `amount` strong.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sheen {
    pub colour: Color,
    pub phase: f32,
    pub amount: f32,
}

/// How an object looks just now: each part named, the whole dimmed (0 to
/// 1: a robot disabled, or with no link), and a part shining as it spins.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Looks {
    pub parts: Vec<(String, PartLook)>,
    pub dim: f32,
    /// Laid over the frame as it is drawn, not kept with it: it moves every frame.
    pub sheen: Option<(String, Sheen)>,
}

impl Looks {
    pub fn plain() -> Looks {
        Looks::default()
    }

    pub fn part(mut self, name: &str, look: PartLook) -> Looks {
        self.parts.push((name.to_owned(), look));
        self
    }

    fn key(&self) -> u64 {
        if self.parts.is_empty() && self.dim == 0.0 {
            return 0;
        }
        let mut h = DefaultHasher::new();
        for (name, look) in &self.parts {
            name.hash(&mut h);
            look.paint.map(|c| (c.0, c.1, c.2)).hash(&mut h);
            look.glow.map(|(c, a)| (c.0, c.1, c.2, (a * 255.0) as u8)).hash(&mut h);
        }
        ((self.dim * 255.0) as u8).hash(&mut h);
        h.finish() | 1
    }
}

/// A pose's patch: the tiles of a view it changes (numbered across the view,
/// row by row), and where its PNGs lie: the tiles a row at a time, each
/// byte the difference from what it is laid over (the four planes, then the
/// part numbers exclusive-or'd).
struct Patch {
    tiles: Vec<u16>,
    at: u32,
    len: u32,
    parts_len: u32,
}

struct Size {
    label: String,
    px: u16,
    yaws: Vec<i16>,
    pitches: Vec<i8>,
    /// Where each frame's PNG lies in the pack, pitch by pitch, yaw by yaw,
    /// and how long its parts' PNG (right after it) is, if it has parts.
    frames: Vec<(u32, u32, u32)>,
    /// The yaws go all the way round (a turntable), so the last blends into the first.
    wraps: bool,
    /// Each pose's patches, by view and pose number, and their tiles' size
    /// and how many a patch's PNG has to a row.
    patches: HashMap<(u16, u16), Patch>,
    tile: usize,
    across: usize,
}

struct Entry {
    name: String,
    tints: [Role; 2],
    own: Color,
    /// Its parts, numbered from 1 in the part plane, each with the turn
    /// (from rest) that brings it round to face the camera.
    parts: Vec<(String, i16)>,
    /// Its mechanisms, each with the steps it was rendered at (in the
    /// robot's own units: inches, degrees).
    mechanisms: Vec<(String, Vec<f32>)>,
    sizes: Vec<Size>,
}

impl Entry {
    /// A pose's number: the first mechanism's step, plus the second's times
    /// the first's count, and so on (as pack.py numbers them).
    fn code(&self, steps: &[usize]) -> u16 {
        let (mut code, mut k) = (0, 1);
        for ((_, s), &i) in self.mechanisms.iter().zip(steps) {
            code += i * k;
            k *= s.len();
        }
        code as u16
    }
}

/// An index of a pack: what it holds and where.
pub struct Pack {
    data: Cow<'static, [u8]>,
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
    /// Version 3; a team's robot made by an older robot.sh (version 2, no
    /// mechanisms) reads too.
    pub fn parse(data: Cow<'static, [u8]>) -> Result<Pack, String> {
        let (entries, blobs) = {
            let mut r = Reader { data: &data, at: 0 };
            if r.take(4)? != b"C3DP" {
                return Err("not a pack of objects".into());
            }
            let version = r.u16()?;
            if !(2..=3).contains(&version) {
                return Err("a pack of another version".into());
            }
            let n = r.u16()? as usize;
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let name = r.text()?;
                let tints = [Role::from_byte(r.u8()?), Role::from_byte(r.u8()?)];
                let own = if version >= 3 {
                    let c = r.take(3)?;
                    Color(c[0], c[1], c[2])
                } else {
                    Color::BLACK
                };
                // Where it stands in its frame (the middle of its footprint), as
                // a fraction: kept in the pack, not needed yet.
                r.take(4)?;
                let parts = (0..r.u8()?).map(|_| Ok((r.text()?, r.u16()? as i16))).collect::<Result<Vec<_>, String>>()?;
                let mut mechanisms = Vec::new();
                if version >= 3 {
                    for _ in 0..r.u8()? {
                        let name = r.text()?;
                        let steps = (0..r.u8()?).map(|_| r.u16().map(|v| v as i16 as f32 / 10.0)).collect::<Result<Vec<_>, String>>()?;
                        mechanisms.push((name, steps));
                    }
                }
                let count = r.u8()? as usize;
                let mut sizes = Vec::with_capacity(count);
                for _ in 0..count {
                    let label = r.text()?;
                    let px = r.u16()?;
                    let (ny, np) = (r.u8()? as usize, r.u8()? as usize);
                    let yaws: Vec<i16> = (0..ny).map(|_| r.u16().map(|v| v as i16)).collect::<Result<_, _>>()?;
                    let pitches: Vec<i8> = (0..np).map(|_| r.u8().map(|v| v as i8)).collect::<Result<_, _>>()?;
                    let frames = (0..ny * np).map(|_| Ok((r.u32()?, r.u32()?, r.u32()?))).collect::<Result<Vec<_>, String>>()?;
                    let wraps = ny > 2 && {
                        let step = (yaws[1] - yaws[0]) as i32;
                        (yaws[ny - 1] - yaws[0]) as i32 + step == 360
                    };
                    let (mut patches, mut tile, mut across) = (HashMap::new(), 8, 8);
                    if version >= 3 {
                        (tile, across) = (r.u8()?.max(1) as usize, r.u8()?.max(1) as usize);
                        let count = (px as usize).div_ceil(tile).pow(2);
                        for _ in 0..r.u32()? {
                            let (view, code, n) = (r.u16()?, r.u16()?, r.u16()? as usize);
                            let tiles = (0..n).map(|_| r.u16()).collect::<Result<Vec<_>, String>>()?;
                            if tiles.iter().any(|t| *t as usize >= count) {
                                return Err("a patch outside its view".into());
                            }
                            let (at, len, parts_len) = (r.u32()?, r.u32()?, r.u32()?);
                            patches.insert((view, code), Patch { tiles, at, len, parts_len });
                        }
                    }
                    sizes.push(Size { label, px, yaws, pitches, frames, wraps, patches, tile, across });
                }
                entries.push(Entry { name, tints, own, parts, mechanisms, sizes });
            }
            (entries, r.u32()? as usize)
        };
        if blobs > data.len() {
            return Err("the pack is cut short".into());
        }
        Ok(Pack { data, entries, blobs })
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// The names of the objects in the pack.
    #[cfg(test)]
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The pack's size, in bytes: what it costs the program file.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    fn blob(&self, at: u32, len: u32) -> Option<&[u8]> {
        let start = self.blobs + at as usize;
        self.data.get(start..start + len as usize)
    }
}

/// A frame decoded and coloured: `0xKKRRGGBB`, colour premultiplied by
/// cover, `KK` what is left of what is under it; and its part numbers, for
/// a part shining as it spins.
struct Frame {
    px: Vec<u32>,
    parts: Option<Vec<u8>>,
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
            // The object's own colour is its own, not the palette's (`Objects::tint`).
            Role::Own => self.signal,
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

type Ramp = [[u8; 3]; 256];

/// What each shade (0 to 255) looks like on a surface of `colour`: the
/// neutral render relit with it, so a face toward the light shows `colour`
/// and one turned away a darker tone of it; overexposure (a highlight, a
/// lit part) bleeds toward white rather than shifting its hue.
fn ramp(colour: Color, albedo: f32) -> Ramp {
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

/// A part's look worked out for colouring: its paint's ramp, and its glow
/// as a colour already lit (a part glowing is lit from within, so its
/// shading is lifted) and how much.
struct Resolved {
    paint: Option<Box<Ramp>>,
    glow: Option<(Box<Ramp>, u32)>,
}

/// One object's frames as the cache keys them: which pack and entry, which
/// size, which frame, which pose, and the looks they were coloured with.
type Key = (u8, u16, u8, u16, u16, u64);

/// The objects, and the frames of them already coloured for the palette in use.
pub struct Objects {
    /// Searched in order: one loaded from the data partition first (a
    /// team's own robot), then the shell's own.
    packs: Vec<Pack>,
    palette: Option<Palette>,
    ramps: HashMap<Role, Ramp>,
    /// An object's own colour's ramp, by the colour.
    own: HashMap<(u8, u8, u8), Ramp>,
    body: Ramp,
    frames: HashMap<Key, Frame>,
    bytes: usize,
    clock: u64,
    /// The most the cache has held, and how many frames were decoded: for
    /// the gallery's account of what objects cost.
    pub peak: usize,
    pub decoded: usize,
    /// Kept between frames, so drawing allocates nothing: the frames mixed,
    /// their sums, the part numbers a sheen follows, and the mix moved or scaled.
    scratch: Vec<u32>,
    acc: Vec<[u32; 4]>,
    sheen_parts: Vec<u8>,
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
        let pack = Pack::parse(Cow::Borrowed(SHIPPED)).inspect_err(|why| eprintln!("catalyst-shell: no 3D objects: {why}")).ok();
        Objects::from(pack.into_iter().collect())
    }

    fn from(packs: Vec<Pack>) -> Objects {
        Objects {
            packs,
            palette: None,
            ramps: HashMap::new(),
            own: HashMap::new(),
            body: [[0; 3]; 256],
            frames: HashMap::new(),
            bytes: 0,
            clock: 0,
            peak: 0,
            decoded: 0,
            scratch: Vec::new(),
            acc: Vec::new(),
            sheen_parts: Vec::new(),
            moved: Vec::new(),
        }
    }

    /// Read a pack from a file and put its objects first, in place of any
    /// read before (a team's robot, replaced by a newer one). Nothing
    /// changes if it cannot be read.
    pub fn load(&mut self, path: &std::path::Path) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let pack = Pack::parse(Cow::Owned(data))?;
        // The shell's own pack is the borrowed one; any other is replaced.
        self.packs.retain(|p| matches!(p.data, Cow::Borrowed(_)));
        self.packs.insert(0, pack);
        self.frames.clear();
        self.bytes = 0;
        Ok(())
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))] // the gallery's, and the tests'
    pub fn pack(&self) -> Option<&Pack> {
        self.packs.last()
    }

    fn find(&self, name: &str) -> Option<(usize, usize)> {
        self.packs.iter().enumerate().find_map(|(i, p)| p.find(name).map(|e| (i, e)))
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        let (p, e) = self.find(name)?;
        Some(&self.packs[p].entries[e])
    }

    /// Whether there is an object of this name.
    pub fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// An object's size in pixels at a size label ("hero").
    pub fn px(&self, name: &str, size: &str) -> Option<f32> {
        self.entry(name)?.sizes.iter().find(|s| s.label == size).map(|s| s.px as f32)
    }

    /// An object's parts, each with the turn (from rest) that faces it to the camera.
    pub fn parts(&self, name: &str) -> Vec<(String, i16)> {
        self.entry(name).map(|e| e.parts.clone()).unwrap_or_default()
    }

    /// An object's mechanisms, each with the steps it was rendered at.
    pub fn mechanisms(&self, name: &str) -> Vec<(String, Vec<f32>)> {
        self.entry(name).map(|e| e.mechanisms.clone()).unwrap_or_default()
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

    /// A tint's ramp: the palette's, or the object's own colour's.
    fn tint(&mut self, role: Role, own: Color) -> Ramp {
        match role {
            Role::Own => *self.own.entry((own.0, own.1, own.2)).or_insert_with(|| ramp(own, ACCENT_ALBEDO)),
            r => self.ramps[&r],
        }
    }

    /// The frame `index` of size `size` of the object at `at`, in pose
    /// `code`, decoded and coloured with `looks`.
    fn frame(&mut self, at: (usize, usize), size: usize, index: usize, code: u16, looks: &Looks, key: u64) -> Option<&Frame> {
        self.clock += 1;
        let k = (at.0 as u8, at.1 as u16, size as u8, index as u16, code, key);
        if !self.frames.contains_key(&k) {
            let palette = self.palette?;
            let (roles, own) = {
                let e = &self.packs[at.0].entries[at.1];
                (e.tints, e.own)
            };
            let (a, b) = (self.tint(roles[0], own), self.tint(roles[1], own));
            let pack = &self.packs[at.0];
            let e = &pack.entries[at.1];
            let s = &e.sizes[size];
            let px = s.px as usize;
            let (off, len, parts_len) = s.frames[index];
            let mut planes = decode(pack.blob(off, len)?, px, px, png::ColorType::Rgba)?;
            let mut part_plane = if parts_len > 0 { decode(pack.blob(off + len, parts_len)?, px, px, png::ColorType::Grayscale) } else { None };
            if code != 0 {
                // A patch for each mechanism moved, then, with more than one
                // moved, the pose's own where they meet.
                let view = index as u16;
                let (mut rest, mut k, mut moved) = (code as usize, 1usize, 0);
                let mut patches: Vec<&Patch> = Vec::new();
                for (_, steps) in &e.mechanisms {
                    let step = rest % steps.len();
                    rest /= steps.len();
                    if step != 0 {
                        moved += 1;
                        patches.extend(s.patches.get(&(view, (step * k) as u16)));
                    }
                    k *= steps.len();
                }
                if moved > 1 {
                    patches.extend(s.patches.get(&(view, code)));
                }
                for p in patches {
                    lay(pack, s, p, &mut planes, part_plane.as_deref_mut())?;
                }
            }
            let resolved: Vec<Option<Resolved>> = e
                .parts
                .iter()
                .map(|(name, _)| {
                    looks.parts.iter().find(|(n, _)| n == name).map(|(_, l)| Resolved {
                        paint: l.paint.map(|c| Box::new(ramp(c, ACCENT_ALBEDO))),
                        glow: l.glow.map(|(c, a)| (Box::new(ramp(c, ACCENT_ALBEDO)), (a.clamp(0.0, 1.0) * 255.0) as u32)),
                    })
                })
                .collect();
            let colours = colour(&planes, px, part_plane.as_deref(), &resolved, &self.body, &a, &b, palette.shadow, looks.dim);
            let bytes = colours.len() * 4 + part_plane.as_ref().map_or(0, |p| p.len());
            // Room for it: the least recently used frames go first.
            while self.bytes + bytes > CACHE_BYTES && !self.frames.is_empty() {
                let oldest = *self.frames.iter().min_by_key(|(_, f)| f.used).map(|(k, _)| k).unwrap();
                if let Some(f) = self.frames.remove(&oldest) {
                    self.bytes -= f.px.len() * 4 + f.parts.as_ref().map_or(0, |p| p.len());
                }
            }
            self.bytes += bytes;
            self.peak = self.peak.max(self.bytes);
            self.decoded += 1;
            self.frames.insert(k, Frame { px: colours, parts: part_plane, used: 0 });
        }
        let f = self.frames.get_mut(&k)?;
        f.used = self.clock;
        Some(f)
    }

    /// Draw an object centred on (`cx`, `cy`), turned and posed as `view`
    /// says, `scale` times its size, faded by `alpha`, in the palette's
    /// colours. False if there is no such object (the caller draws
    /// something else).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, canvas: &mut Canvas, tokens: &Tokens, fault: Color, name: &str, size: &str, cx: f32, cy: f32, view: View, scale: f32, alpha: f32) -> bool {
        self.draw_looking(canvas, tokens, fault, name, size, cx, cy, view, scale, alpha, &Looks::plain())
    }

    /// The same, its parts and the whole looking as `looks` says.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_looking(&mut self, canvas: &mut Canvas, tokens: &Tokens, fault: Color, name: &str, size: &str, cx: f32, cy: f32, view: View, scale: f32, alpha: f32, looks: &Looks) -> bool {
        let Some(at) = self.find(name) else { return false };
        let Some(si) = self.packs[at.0].entries[at.1].sizes.iter().position(|s| s.label == size) else { return false };
        if alpha <= 0.0 || scale <= 0.0 {
            return true;
        }
        let (px, weights, sheen_part) = {
            let e = &self.packs[at.0].entries[at.1];
            let s = &e.sizes[si];
            let sheen_part = looks.sheen.as_ref().and_then(|(part, _)| e.parts.iter().position(|(n, _)| n == part)).map(|i| i as u8 + 1);
            (s.px as usize, blend(e, s, view), sheen_part)
        };
        self.follow(tokens, fault);
        let key = looks.key();
        // One frame at its own size and on whole pixels is laid straight
        // down; otherwise the frames are mixed (and moved or scaled) first.
        let n = px * px;
        let mut mixed = std::mem::take(&mut self.scratch);
        mixed.clear();
        mixed.resize(n, 0);
        let mut sheen_parts = std::mem::take(&mut self.sheen_parts);
        sheen_parts.clear();
        let heaviest = weights.iter().enumerate().max_by(|a, b| a.1 .2.total_cmp(&b.1 .2)).map_or(0, |(i, _)| i);
        if weights.len() > 1 {
            // Weights in 256ths that add up to 256 exactly, so a blend of
            // equal frames is that frame.
            let mut w256: Vec<u32> = weights.iter().map(|(.., w)| (w * 256.0).round() as u32).collect();
            let sum: u32 = w256.iter().sum();
            w256[0] = (w256[0] + 256).saturating_sub(sum);
            let mut acc = std::mem::take(&mut self.acc);
            acc.clear();
            acc.resize(n, [0u32; 4]);
            for (i, (&(index, code, _), &w)) in weights.iter().zip(&w256).enumerate() {
                let Some(f) = self.frame(at, si, index, code, looks, key) else { return false };
                for (a, p) in acc.iter_mut().zip(&f.px) {
                    a[0] += ((p >> 16) & 0xff) * w;
                    a[1] += ((p >> 8) & 0xff) * w;
                    a[2] += (p & 0xff) * w;
                    a[3] += (p >> 24) * w;
                }
                if i == heaviest && sheen_part.is_some() {
                    sheen_parts.extend_from_slice(f.parts.as_deref().unwrap_or(&[]));
                }
            }
            for (m, a) in mixed.iter_mut().zip(&acc) {
                let c = a.map(|v| ((v + 128) >> 8).min(255));
                *m = c[3] << 24 | c[0] << 16 | c[1] << 8 | c[2];
            }
            self.acc = acc;
        } else {
            let Some(f) = self.frame(at, si, weights[0].0, weights[0].1, looks, key) else { return false };
            mixed.copy_from_slice(&f.px);
            if sheen_part.is_some() {
                sheen_parts.extend_from_slice(f.parts.as_deref().unwrap_or(&[]));
            }
        }
        if let (Some(part), Some((_, sheen)), true) = (sheen_part, looks.sheen.as_ref(), sheen_parts.len() == n) {
            shine(&mut mixed, &sheen_parts, px, part, sheen);
        }
        self.sheen_parts = sheen_parts;
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

/// Bands of light running across a part as it spins: diagonal, a few
/// pixels apart, moving on with `phase`; each pixel of the part (by
/// `parts`) mixed toward the sheen's colour by how near a band's middle it is.
fn shine(px: &mut [u32], parts: &[u8], size: usize, part: u8, sheen: &Sheen) {
    const PERIOD: f32 = 6.0;
    let amount = sheen.amount.clamp(0.0, 1.0);
    for (i, (p, &k)) in px.iter_mut().zip(parts).enumerate() {
        if k != part {
            continue;
        }
        let (x, y) = ((i % size) as f32, (i / size) as f32);
        let band = ((x * 0.8 + y * 0.6) / PERIOD - sheen.phase).rem_euclid(1.0);
        let t = ((1.0 - (band - 0.5).abs() * 5.0).max(0.0) * amount * 256.0) as u32;
        if t == 0 {
            continue;
        }
        let keep = *p >> 24;
        let cover = 255 - keep;
        let to = [sheen.colour.0, sheen.colour.1, sheen.colour.2].map(|c| c as u32 * cover / 255);
        let from = [(*p >> 16) & 0xff, (*p >> 8) & 0xff, *p & 0xff];
        let c: Vec<u32> = (0..3).map(|j| (from[j] * (256 - t) + to[j] * t) >> 8).collect();
        *p = keep << 24 | c[0] << 16 | c[1] << 8 | c[2];
    }
}

/// The frames a view is made of and how much of each: the two nearest
/// rendered yaws (and pitches), and for each mechanism the two steps either
/// side of where it is, linearly. Within a fiftieth of a step of a rendered
/// angle or a step, that one alone.
fn blend(e: &Entry, s: &Size, view: View) -> Vec<(usize, u16, f32)> {
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
    // Each mechanism's two steps either side of where it is.
    let mut poses: Vec<(Vec<usize>, f32)> = vec![(Vec::new(), 1.0)];
    for (m, (_, steps)) in e.mechanisms.iter().enumerate().take(MECHANISMS) {
        let at = view.pose[m].clamp(0.0, (steps.len() - 1) as f32);
        let i = (at.floor() as usize).min(steps.len() - 1);
        let mut t = at - i as f32;
        if t < 0.02 {
            t = 0.0;
        } else if t > 0.98 {
            t = 1.0;
        }
        let pair = [(i, 1.0 - t), ((i + 1).min(steps.len() - 1), t)];
        poses = poses
            .iter()
            .flat_map(|(p, w)| {
                pair.iter().map(move |&(k, wk)| {
                    let mut q = p.clone();
                    q.push(k);
                    (q, w * wk)
                })
            })
            .filter(|(_, w)| *w > 0.0)
            .collect();
    }
    let mut out: Vec<(usize, u16, f32)> = Vec::with_capacity(8);
    for (pose, wm) in &poses {
        let code = e.code(pose);
        for (p, wp) in pa {
            for (y, wy) in ya {
                let w = wp * wy * wm;
                if w > 1.0 / 64.0 {
                    let index = p * yaws.len() + y;
                    match out.iter_mut().find(|(i, c, _)| *i == index && *c == code) {
                        Some(e) => e.2 += w,
                        None => out.push((index, code, w)),
                    }
                }
            }
        }
    }
    out
}

/// A patch laid over a view's planes (and part numbers): each tile's bytes
/// added (wrapping) to what is there, its part numbers exclusive-or'd.
fn lay(pack: &Pack, s: &Size, p: &Patch, planes: &mut [u8], parts: Option<&mut [u8]>) -> Option<()> {
    let (px, tile) = (s.px as usize, s.tile);
    let per_row = px.div_ceil(tile);
    let w = s.across.min(p.tiles.len()) * tile;
    let h = p.tiles.len().div_ceil(s.across) * tile;
    let delta = decode(pack.blob(p.at, p.len)?, w, h, png::ColorType::Rgba)?;
    let xor = if p.parts_len > 0 { Some(decode(pack.blob(p.at + p.len, p.parts_len)?, w, h, png::ColorType::Grayscale)?) } else { None };
    // Where each tile is in the view, and in the patch.
    let place = |k: usize, t: u16| {
        let (ty, tx) = (t as usize / per_row * tile, t as usize % per_row * tile);
        let (sy, sx) = (k / s.across * tile, k % s.across * tile);
        (ty, tx, sy, sx, tile.min(px - ty), tile.min(px - tx))
    };
    for (k, &t) in p.tiles.iter().enumerate() {
        let (ty, tx, sy, sx, th, tw) = place(k, t);
        for row in 0..th {
            let to = ((ty + row) * px + tx) * 4;
            let from = ((sy + row) * w + sx) * 4;
            for (a, d) in planes[to..to + tw * 4].iter_mut().zip(&delta[from..from + tw * 4]) {
                *a = a.wrapping_add(*d);
            }
        }
    }
    if let (Some(parts), Some(xor)) = (parts, xor) {
        for (k, &t) in p.tiles.iter().enumerate() {
            let (ty, tx, sy, sx, th, tw) = place(k, t);
            for row in 0..th {
                let to = (ty + row) * px + tx;
                let from = (sy + row) * w + sx;
                for (a, d) in parts[to..to + tw].iter_mut().zip(&xor[from..from + tw]) {
                    *a ^= *d;
                }
            }
        }
    }
    Some(())
}

/// A PNG back into its planes (four interleaved, or one), `w` by `h`.
fn decode(png: &[u8], w: usize, h: usize, kind: png::ColorType) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let channels = if kind == png::ColorType::Rgba { 4 } else { 1 };
    let mut buf = vec![0u8; w * h * channels];
    let info = reader.next_frame(&mut buf).ok()?;
    (info.width as usize == w && info.height as usize == h && info.color_type == kind).then_some(buf)
}

/// Planes to premultiplied colour, by the palette's ramps; each part as it
/// looks, and the whole dimmed by `dim`. A view is `px` square.
#[allow(clippy::too_many_arguments)]
fn colour(planes: &[u8], px: usize, parts: Option<&[u8]>, looks: &[Option<Resolved>], body: &Ramp, a: &Ramp, b: &Ramp, shadow: f32, dim: f32) -> Vec<u32> {
    let shadow = (shadow * 256.0) as u32;
    let dim = (dim.clamp(0.0, 1.0) * 256.0) as u32;
    // A soft shadow can reach past the view: it fades out over the last
    // sixth of the view to each edge, so it never ends in a straight line.
    let edge = (px / 6).max(1);
    let fade: Vec<u32> = (0..px)
        .map(|k| {
            let t = (k.min(px - 1 - k) * 256 / edge).min(256) as u32;
            t * t * (768 - 2 * t) >> 16
        })
        .collect();
    let fade = &fade;
    let falloff = (0..px).flat_map(|y| (0..px).map(move |x| fade[x] * fade[y] >> 8));
    planes
        .chunks_exact(4)
        .zip(falloff)
        .enumerate()
        .map(|(i, (p, falloff))| {
            let (s, tint, cover, sh) = (p[0] as usize, p[1], p[2] as u32, p[3] as u32);
            let sh = sh * falloff >> 8;
            let (ta, tb) = ((tint >> 4) as u32 * 17, (tint & 15) as u32 * 17);
            let tn = 255u32.saturating_sub(ta + tb);
            let look = parts.and_then(|pp| pp[i].checked_sub(1)).and_then(|k| looks.get(k as usize)).and_then(|l| l.as_ref());
            let (ra, rb) = match look.and_then(|l| l.paint.as_deref()) {
                Some(paint) => (paint, paint),
                None => (a, b),
            };
            let mut c = [0, 1, 2].map(|k| (body[s][k] as u32 * tn + ra[s][k] as u32 * ta + rb[s][k] as u32 * tb) / 255);
            if let Some((glow, amount)) = look.and_then(|l| l.glow.as_ref()) {
                // Lit from within: its colour at a lifted shade, mixed in.
                let lifted = (s + 28).min(255);
                c = [0, 1, 2].map(|k| (c[k] * (255 - amount) + glow[lifted][k] as u32 * amount) / 255);
            }
            if dim > 0 {
                // Dimmed: toward its own grey, and darker.
                let grey = (c[0] * 54 + c[1] * 183 + c[2] * 19) >> 8;
                let flat = grey * 170 >> 8;
                c = c.map(|v| (v * (256 - dim) + flat * dim) >> 8);
            }
            let [r, g, bl] = c.map(|v| v.min(255) * cover / 255);
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

#[cfg(test)]
mod tests {
    use super::*;

    const ROBOT: &str = "robot";

    fn objects() -> Objects {
        Objects::shipped()
    }

    #[test]
    fn the_pack_holds_numbers_with_its_parts_and_mechanisms() {
        let o = objects();
        let pack = o.pack().expect("the shipped pack reads");
        assert_eq!(pack.names().collect::<Vec<_>>(), [ROBOT], "only the robot is 3D");
        let parts: Vec<String> = o.parts(ROBOT).into_iter().map(|(n, _)| n).collect();
        assert_eq!(parts, ["drivetrain", "intake", "hopper", "shooter", "flywheel", "hood", "bumpers", "frame"]);
        let mechanisms = o.mechanisms(ROBOT);
        assert_eq!(mechanisms.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["intake", "hood"]);
        assert_eq!(mechanisms[0].1, [5.0, 11.8], "the intake in Deploy's inches, stowed and out");
        assert_eq!(mechanisms[1].1, [11.0, 25.0, 40.0], "the hood in degrees");
        let e = o.entry(ROBOT).unwrap();
        assert_eq!(e.tints[1], Role::Own);
        assert_eq!((e.own.0, e.own.1, e.own.2), (0x00, 0x57, 0xb8), "Numbers' blue");
        let s = &e.sizes[0];
        assert_eq!((s.label.as_str(), s.px, s.yaws.len()), ("hero", 112, 90));
        // Every pose of one mechanism moved has a patch in every view.
        for view in 0..90u16 {
            for code in [1, 2, 4] {
                assert!(s.patches.contains_key(&(view, code)), "view {view} pose {code}");
            }
        }
    }

    #[test]
    fn a_view_between_two_angles_or_two_steps_blends_the_two() {
        let o = objects();
        let e = o.entry(ROBOT).unwrap();
        let s = &e.sizes[0];
        assert_eq!(blend(e, s, View::REST), vec![(0, 0, 1.0)], "at rest: one frame");
        let half = blend(e, s, View::yaw(2.0));
        assert_eq!(half.len(), 2);
        assert!((half[0].2 - 0.5).abs() < 0.01);
        // Round the turn: the last view blends into the first.
        let round = blend(e, s, View::yaw(358.0));
        assert_eq!(round.iter().map(|(i, ..)| *i).collect::<Vec<_>>(), vec![89, 0]);
        // The intake half way from its first step to its second: the two poses.
        let moving = blend(e, s, View::REST.posed([0.5, 0.0, 0.0, 0.0]));
        assert_eq!(moving.iter().map(|(_, c, _)| *c).collect::<Vec<_>>(), vec![0, 1]);
        // The intake out and the hood at its third step: one pose, numbered as pack.py does.
        assert_eq!(blend(e, s, View::REST.posed([1.0, 2.0, 0.0, 0.0])), vec![(0, 1 + 2 * 2, 1.0)]);
        // Past its last step: held there.
        assert_eq!(blend(e, s, View::REST.posed([0.0, 7.0, 0.0, 0.0])), vec![(0, 2 * 2, 1.0)]);
    }

    fn near(p: u32, c: Color, by: i32) -> bool {
        let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
        (r as i32 - c.0 as i32).abs() < by && (g as i32 - c.1 as i32).abs() < by && (b as i32 - c.2 as i32).abs() < by
    }

    #[test]
    fn a_shadow_fades_out_before_the_edge_of_its_view() {
        // A view all shadow and nothing covered: what is under it is kept
        // whole at the edges, darkened in the middle, and never in a step.
        let px = 30;
        let planes: Vec<u8> = (0..px * px).flat_map(|_| [128, 0, 0, 255]).collect();
        let flat = ramp(Color::hex(0x808080), 0.8);
        let out = colour(&planes, px, None, &[], &flat, &flat, &flat, 0.85, 0.0);
        let keep = |x: usize, y: usize| out[y * px + x] >> 24;
        for k in 0..px {
            for (x, y) in [(k, 0), (0, k), (k, px - 1), (px - 1, k)] {
                assert_eq!(keep(x, y), 255, "shadow at the edge, ({x}, {y})");
            }
        }
        assert!(keep(px / 2, px / 2) < 80, "no shadow in the middle: {}", keep(px / 2, px / 2));
        let row: Vec<u32> = (0..px).map(|x| keep(x, px / 2)).collect();
        assert!(row.windows(2).all(|w| w[0].abs_diff(w[1]) <= 70), "{row:?}");
    }

    #[test]
    fn the_robot_takes_the_palette_and_its_shadow_darkens_what_is_under_it() {
        let mut o = objects();
        let mut c = Canvas::new(130, 130);
        let ground = Color::hex(0x404040);
        c.clear(ground);
        let dark = Tokens::of(bezel::Accent::Orange, Tone::Dark);
        assert!(o.draw(&mut c, &dark, Color::hex(0xff6b5e), ROBOT, "hero", 65.0, 65.0, View::REST, 1.0, 1.0));
        // Its bumpers are the accent, unpainted.
        assert!(c.px.iter().any(|p| near(*p, dark.signal, 40)), "no accent");
        // Its shadow: something darker than the ground beside the object.
        assert!(c.px.iter().any(|p| (p & 0xff) < 0x30), "no shadow");
        assert_eq!(c.px[0], ground.to_u32());
        // Another accent recolours it.
        let blue = Tokens::of(bezel::Accent::Green, Tone::Dark);
        c.clear(ground);
        o.draw(&mut c, &blue, Color::hex(0xff6b5e), ROBOT, "hero", 65.0, 65.0, View::REST, 1.0, 1.0);
        assert!(c.px.iter().any(|p| near(*p, blue.signal, 40)), "not recoloured");
    }

    fn draw(o: &mut Objects, view: View, looks: &Looks) -> Canvas {
        let t = Tokens::of(bezel::Accent::Orange, Tone::Dark);
        let mut c = Canvas::new(130, 130);
        c.clear(Color::BLACK);
        assert!(o.draw_looking(&mut c, &t, Color::hex(0xff6b5e), ROBOT, "hero", 65.0, 65.0, view, 1.0, 1.0, looks));
        c
    }

    #[test]
    fn a_part_is_painted_or_glows_on_its_own_and_the_whole_dims() {
        let mut o = objects();
        let plain = draw(&mut o, View::REST, &Looks::plain());
        // Bumpers painted (in a green far from the accent and Numbers' blue): green appears, the rest is as it was.
        let green = Color::hex(0x2fbf4f);
        let painted = draw(&mut o, View::REST, &Looks::plain().part("bumpers", PartLook { paint: Some(green), glow: None }));
        assert!(!plain.px.iter().any(|p| near(*p, green, 30)) && painted.px.iter().any(|p| near(*p, green, 30)), "bumpers not painted");
        let same = plain.px.iter().zip(&painted.px).filter(|(a, b)| a == b).count();
        assert!(same > plain.px.len() / 2, "only the bumpers changed: {same} of {}", plain.px.len());
        // The hood hot: amber where it was not.
        let amber = Tokens::of(bezel::Accent::Orange, Tone::Dark).amber;
        let hot = draw(&mut o, View::REST, &Looks::plain().part("hood", PartLook { paint: None, glow: Some((amber, 0.6)) }));
        let warm = |c: &Canvas| c.px.iter().filter(|p| ((*p >> 16) & 0xff) > ((*p & 0xff) + 60)).count();
        assert!(warm(&hot) > warm(&plain) + 30, "{} warm pixels, {} before", warm(&hot), warm(&plain));
        // Dimmed: darker overall.
        let dim = draw(&mut o, View::REST, &Looks { dim: 1.0, ..Looks::plain() });
        let light = |c: &Canvas| c.px.iter().map(|p| ((p >> 8) & 0xff) as u64).sum::<u64>();
        assert!(light(&dim) * 10 < light(&plain) * 8, "dimmed");
    }

    /// Where two pictures differ: the box around it (x0, y0, x1, y1), or None.
    fn differs(a: &Canvas, b: &Canvas) -> Option<(usize, usize, usize, usize)> {
        let mut found: Option<(usize, usize, usize, usize)> = None;
        for (i, (p, q)) in a.px.iter().zip(&b.px).enumerate() {
            if !near(*p, Color((q >> 16) as u8, (q >> 8) as u8, *q as u8), 24) {
                let (x, y) = (i % 130, i / 130);
                found = Some(found.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))));
            }
        }
        found
    }

    #[test]
    fn the_intake_slides_out_and_the_hood_rises_each_where_it_is() {
        let mut o = objects();
        let rest = draw(&mut o, View::REST, &Looks::plain());
        let out = draw(&mut o, View::REST.posed([1.0, 0.0, 0.0, 0.0]), &Looks::plain());
        let up = draw(&mut o, View::REST.posed([0.0, 2.0, 0.0, 0.0]), &Looks::plain());
        let both = draw(&mut o, View::REST.posed([1.0, 2.0, 0.0, 0.0]), &Looks::plain());
        let (intake, hood) = (differs(&rest, &out).expect("the intake moved"), differs(&rest, &up).expect("the hood moved"));
        // Each changes only part of the picture: the intake low and to the
        // front (the robot rests turned to show its front at the left), the hood above.
        assert!((intake.2 - intake.0) * (intake.3 - intake.1) < 130 * 130 / 2, "{intake:?}");
        assert!(hood.1 < intake.3, "the hood {hood:?} is higher than the intake's lowest {intake:?}");
        // Both moved: what each changed.
        assert!(differs(&out, &both).is_some() && differs(&up, &both).is_some());
        // Half way out (moving) is neither.
        let half = draw(&mut o, View::REST.posed([0.5, 0.0, 0.0, 0.0]), &Looks::plain());
        assert!(differs(&rest, &half).is_some() && differs(&half, &out).is_some());
    }

    #[test]
    fn the_flywheel_shines_where_it_is_and_nowhere_else() {
        let mut o = objects();
        let still = draw(&mut o, View::yaw(180.0), &Looks::plain());
        let ice = Tokens::dark().ice;
        let spin = |phase: f32| Looks { sheen: Some(("flywheel".into(), Sheen { colour: ice, phase, amount: 0.8 })), ..Looks::plain() };
        let a = draw(&mut o, View::yaw(180.0), &spin(0.0));
        let b = draw(&mut o, View::yaw(180.0), &spin(0.5));
        let changed = |x: &Canvas, y: &Canvas| x.px.iter().zip(&y.px).filter(|(p, q)| p != q).count();
        assert!(changed(&still, &a) > 0, "no sheen on the flywheel from behind");
        assert!(changed(&a, &b) > 0, "the bands move on");
        assert!(changed(&still, &a) < 130 * 130 / 10, "only the flywheel: {}", changed(&still, &a));
    }

    #[test]
    fn frames_are_kept_within_the_cache() {
        let mut o = objects();
        let mut c = Canvas::new(320, 170);
        let t = Tokens::dark();
        for yaw in (0..720).step_by(5) {
            let pose = [(yaw % 2) as f32, (yaw / 5 % 3) as f32, 0.0, 0.0];
            o.draw(&mut c, &t, Color::BLACK, ROBOT, "hero", 160.0, 85.0, View::yaw(yaw as f32).posed(pose), 1.0, 1.0);
        }
        assert!(o.peak <= CACHE_BYTES, "{} bytes", o.peak);
        assert!(o.decoded > 24, "frames came and went: {}", o.decoded);
        assert!(!o.draw(&mut c, &t, Color::BLACK, "no-such-thing", "hero", 0.0, 0.0, View::REST, 1.0, 1.0));
    }

    #[test]
    fn a_pack_on_the_data_partition_comes_first() {
        let dir = std::env::temp_dir().join(format!("catalyst-objects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("robot3d.pack");
        // Not a pack: refused, nothing changed.
        std::fs::write(&path, b"nonsense").unwrap();
        let mut o = objects();
        assert!(o.load(&path).is_err());
        assert!(o.has(ROBOT));
        // The shell's own pack, read as if from the data partition: found first.
        std::fs::write(&path, SHIPPED).unwrap();
        o.load(&path).unwrap();
        assert_eq!(o.find(ROBOT).map(|f| f.0), Some(0));
        o.load(&path).unwrap();
        assert_eq!(o.packs.len(), 2, "a newer one replaces the last, not added to it");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moved_or_scaled_it_lands_where_it_was_put() {
        let mut o = objects();
        let t = Tokens::dark();
        let mut a = Canvas::new(130, 130);
        a.clear(Color::BLACK);
        o.draw(&mut a, &t, Color::BLACK, ROBOT, "hero", 65.0, 65.0, View::REST, 1.0, 1.0);
        let mut b = Canvas::new(130, 130);
        b.clear(Color::BLACK);
        o.draw(&mut b, &t, Color::BLACK, ROBOT, "hero", 65.25, 65.0, View::REST, 1.0, 1.0);
        let centre = |c: &Canvas| {
            let (mut sx, mut n) = (0.0, 0.0);
            for (i, p) in c.px.iter().enumerate() {
                let v = ((p >> 8) & 0xff) as f32;
                sx += v * (i % 130) as f32;
                n += v;
            }
            sx / n
        };
        let d = centre(&b) - centre(&a);
        assert!(d > 0.1 && d < 0.4, "moved by {d}");
        let mut s = Canvas::new(130, 130);
        s.clear(Color::BLACK);
        o.draw(&mut s, &t, Color::BLACK, ROBOT, "hero", 65.0, 65.0, View::REST, 0.7, 1.0);
        let lit = |c: &Canvas| c.px.iter().filter(|p| (*p & 0xff) > 0x40).count() as f32;
        let k = lit(&s) / lit(&a);
        assert!(k > 0.35 && k < 0.65, "0.7 times as large: {k} times the pixels");
    }
}
