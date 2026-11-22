//! Bezel's two voices of type: Google Sans Flex for what is read at a glance
//! and lowercase Google Sans Code for quiet labels. Both are variable fonts;
//! glyphs are shaped and rasterized on demand and kept in a bounded cache.
//!
//! Both are cut to Latin. What they lack (Chinese, Japanese and Korean, and
//! Greek and Cyrillic) falls to Noto Sans CJK, a file outside the program
//! ([`CJK_FILE`]) that is memory-mapped the first time a line needs it:
//! until then it costs nothing, and after, only the pages of it that glyphs
//! were drawn from, which the kernel may drop and read again. Its glyphs are
//! sized and placed to sit with Google Sans on one baseline, at the weight
//! of the style they are in. In the mono face a wide character takes two
//! cells, as a terminal gives it. A character no font has is drawn as a
//! box: a square one (the CJK font's) for a wide character.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use swash::scale::{Render, ScaleContext, Source as Outline};
use swash::shape::ShapeContext;
use swash::zeno::{Format, Vector};
use swash::{Charmap, FontRef};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::lines;

static SANS: &[u8] = include_bytes!("../../../assets/fonts/GoogleSansFlex.ttf");
static CODE: &[u8] = include_bytes!("../../../assets/fonts/GoogleSansCode.ttf");

/// The CJK font's file name, and where the image installs it.
pub const CJK_FILE: &str = "NotoSansCJK-Catalyst.otf";
pub const CJK_DIR: &str = "/usr/share/fonts/catalyst";

/// The glyph cache: at most this many glyphs, and this many bytes of them,
/// in each of its two generations.
const GLYPHS: usize = 768;
const GLYPH_BYTES: usize = 192 * 1024;
/// Shaped lines, likewise.
const RUNS: usize = 256;
const RUN_BYTES: usize = 96 * 1024;

/// CJK glyphs beside Google Sans: an ideograph fills its em, a capital not
/// three quarters of one, so the CJK em is set a little smaller than the
/// Latin (its ideographs then stand about 1.15 capitals tall and are centred
/// on the capitals' middle, on the same baseline). Never smaller than
/// `CJK_FLOOR` px unless the Latin itself is: an ideograph under 11 px is
/// hard to read on this panel.
const CJK_SCALE: f32 = 0.92;
const CJK_FLOOR: f32 = 11.0;
/// In the mono face a wide character is set in two cells (1.2 em), and the
/// CJK em is set to nearly fill them, as a CJK terminal font's does.
const CJK_SCALE_MONO: f32 = 1.1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Sans,
    Code,
}

/// Which font a glyph comes from: a face's own, or the fallback.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Font {
    Sans,
    Code,
    Cjk,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub face: Face,
    /// Pixels per em.
    pub size: f32,
    pub wght: f32,
    /// Width axis (Sans only); 100 is normal.
    pub wdth: f32,
    /// Roundness axis (Sans only), 0–100.
    pub rond: f32,
    /// Letter spacing in em.
    pub tracking: f32,
}

impl TextStyle {
    pub const fn sans(size: f32, wght: f32) -> Self {
        TextStyle { face: Face::Sans, size, wght, wdth: 100.0, rond: 0.0, tracking: 0.0 }
    }
    pub const fn code(size: f32) -> Self {
        TextStyle { face: Face::Code, size, wght: 400.0, wdth: 100.0, rond: 0.0, tracking: 0.0 }
    }
    pub const fn weight(mut self, wght: f32) -> Self {
        self.wght = wght;
        self
    }

    // Bezel's type roles. The axis settings are Bezel's; the sizes are the
    // caller's, because a 320 px panel cannot take 720-panel sizes scaled down.

    /// Glanceable values: light, wide, rounded.
    pub const fn display(size: f32) -> Self {
        TextStyle { face: Face::Sans, size, wght: 300.0, wdth: 112.0, rond: 40.0, tracking: -0.03 }
    }
    /// Page and track titles.
    pub const fn title(size: f32) -> Self {
        TextStyle { face: Face::Sans, size, wght: 600.0, wdth: 104.0, rond: 0.0, tracking: -0.01 }
    }
    /// Names of things.
    pub const fn name(size: f32) -> Self {
        Self::sans(size, 600.0)
    }
    /// Control labels and rows.
    pub const fn body(size: f32) -> Self {
        Self::sans(size, 400.0)
    }
    /// Lowercase mono labels, captions and values.
    pub const fn label(size: f32) -> Self {
        Self::code(size)
    }

    fn key(&self) -> (Face, u16, u16, u16, u8) {
        (self.face, (self.size * 4.0) as u16, self.wght as u16, self.wdth as u16, self.rond as u8)
    }

    /// The pixel size CJK glyphs are set at in this style.
    pub fn cjk_size(&self) -> f32 {
        match self.face {
            Face::Sans => (self.size * CJK_SCALE).max(self.size.min(CJK_FLOOR)),
            Face::Code => self.size * CJK_SCALE_MONO,
        }
    }
}

struct Glyph {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    data: Vec<u8>,
}

/// One glyph of a shaped line: which, from which font, and its pen x
/// relative to the line's start.
#[derive(Clone, Copy, Debug)]
struct Placed {
    id: u16,
    x: f32,
    font: Font,
}

struct Run {
    glyphs: Vec<Placed>,
    width: f32,
}

/// A rasterized glyph: its font, size (quarter pixels), weight, width and
/// roundness, its id, and its quarter-pixel offset.
type GlyphKey = (Font, u16, u16, u16, u8, u16, u8);

/// What a cached thing weighs, roughly, in bytes.
trait Weigh {
    fn weigh(&self) -> usize;
}

impl Weigh for Option<Glyph> {
    fn weigh(&self) -> usize {
        64 + self.as_ref().map_or(0, |g| g.data.len())
    }
}

impl Weigh for Rc<Run> {
    fn weigh(&self) -> usize {
        64 + self.glyphs.len() * std::mem::size_of::<Placed>()
    }
}

/// A cache in two generations. What is used goes into the young one; when
/// that is full (by count or by bytes) the old one is dropped and the young
/// one becomes old. Something still in use is found in the old one and
/// brought back, so a full cache never empties all at once (a frame redraws
/// only what it lost), and it never holds more than two generations' worth.
struct Generations<K, V> {
    young: HashMap<K, V>,
    old: HashMap<K, V>,
    bytes: usize,
    most: usize,
    most_bytes: usize,
}

impl<K: Hash + Eq + Clone, V: Weigh> Generations<K, V> {
    fn new(most: usize, most_bytes: usize) -> Self {
        Generations { young: HashMap::new(), old: HashMap::new(), bytes: 0, most, most_bytes }
    }

    fn get_or_insert_with(&mut self, key: &K, make: impl FnOnce() -> V) -> &V {
        if !self.young.contains_key(key) {
            let value = self.old.remove(key).unwrap_or_else(make);
            let weight = value.weigh();
            if self.young.len() >= self.most || self.bytes + weight > self.most_bytes {
                self.old = std::mem::take(&mut self.young);
                self.bytes = 0;
            }
            self.bytes += weight;
            self.young.insert(key.clone(), value);
        }
        &self.young[key]
    }

    fn len(&self) -> usize {
        self.young.len() + self.old.len()
    }

    fn bytes(&self) -> usize {
        self.bytes + self.old.values().map(Weigh::weigh).sum::<usize>()
    }
}

// ------------------------------------------------------------ the CJK font ---

/// The CJK font's bytes, mapped once for the whole program the first time a
/// line needs it; `None` if there is no such file.
#[cfg(not(cjk_embedded))]
static CJK_DATA: std::sync::OnceLock<Option<&'static [u8]>> = std::sync::OnceLock::new();

/// Where the CJK font is looked for, first found first: `CATALYST_CJK_FONT`,
/// the image's [`CJK_DIR`], beside the program, and (on a computer) the
/// repository it was built from, where scripts/cjk-font.py puts it.
pub fn cjk_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = std::env::var_os("CATALYST_CJK_FONT") {
        paths.push(PathBuf::from(p));
    }
    paths.push(Path::new(CJK_DIR).join(CJK_FILE));
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned)) {
        paths.push(dir.join(CJK_FILE));
        paths.push(dir.join("fonts").join(CJK_FILE));
    }
    #[cfg(not(target_arch = "aarch64"))]
    paths.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/cjk").join(CJK_FILE));
    paths
}

/// The packed simulator's own copy (see build.rs).
#[cfg(cjk_embedded)]
static CJK_EMBEDDED: &[u8] = include_bytes!(env!("CATALYST_CJK_EMBEDDED"));

#[cfg(cjk_embedded)]
fn cjk_data() -> Option<&'static [u8]> {
    Some(CJK_EMBEDDED)
}

#[cfg(not(cjk_embedded))]
fn cjk_data() -> Option<&'static [u8]> {
    *CJK_DATA.get_or_init(|| cjk_paths().iter().find_map(|path| map(path).filter(|data| FontRef::from_index(data, 0).is_some())))
}

/// Whether the CJK font was found (mapping it if it has not been yet).
pub fn cjk_available() -> bool {
    cjk_data().is_some()
}

/// The file, mapped for the rest of the program. Its pages are read from
/// the file as glyphs are drawn from them, not before (no read-ahead: a
/// glyph is read where it lies), and they are not locked into memory with
/// the rest of the shell's: they stay the kernel's page cache, shared with
/// every other program drawing with the font, and dropped and read again
/// from flash if it needs the room.
#[cfg(all(unix, not(cjk_embedded)))]
fn map(path: &Path) -> Option<&'static [u8]> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    let len = usize::try_from(file.metadata().ok()?.len()).ok().filter(|&n| n > 0)?;
    // SAFETY: a read-only private mapping of a whole file, never unmapped,
    // so the slice stays valid for the life of the program. The image's
    // fonts are on a read-only filesystem.
    unsafe {
        let at = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0);
        if at == libc::MAP_FAILED {
            return None;
        }
        libc::munlock(at, len);
        libc::madvise(at, len, libc::MADV_RANDOM);
        Some(std::slice::from_raw_parts(at as *const u8, len))
    }
}

/// Elsewhere (the simulator on Windows) the file is read whole.
#[cfg(not(any(unix, cjk_embedded)))]
fn map(path: &Path) -> Option<&'static [u8]> {
    std::fs::read(path).ok().filter(|d| !d.is_empty()).map(|d| &*Box::leak(d.into_boxed_slice()))
}

/// Whether the CJK font may have `c`: the blocks its characters come from
/// (Greek, Cyrillic, symbols and arrows, the CJK blocks, Hangul, full-width
/// forms). Anything else (Arabic, Thai, an emoji) never maps the file.
fn in_cjk_blocks(c: char) -> bool {
    matches!(c as u32, 0x0370..=0x04FF | 0x2010..=0x2BFF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF)
}

/// A character drawn over the one before it (a combining mark, a variation
/// selector, a joiner): it goes with that one's font.
fn joins_previous(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200B..=0x200F | 0x20D0..=0x20FF | 0x3099..=0x309A | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0xE0100..=0xE01EF)
}

// -------------------------------------------------------------------- text ---

pub struct Text {
    sans: FontRef<'static>,
    code: FontRef<'static>,
    sans_map: Charmap<'static>,
    code_map: Charmap<'static>,
    /// Noto Sans CJK, once a line has asked for it: `None` until then, and
    /// `Some(None)` if there is none.
    cjk: Option<Option<(FontRef<'static>, Charmap<'static>)>>,
    shape: ShapeContext,
    scale: ScaleContext,
    glyphs: Generations<GlyphKey, Option<Glyph>>,
    runs: Generations<u64, Rc<Run>>,
    /// The letter style chosen for the sans: how round its corners are at
    /// least (the ROND axis, 0 to 100), and how much wider or narrower than
    /// each role's own width (the wdth axis). Applied wherever text is
    /// measured or drawn, so what is measured is what is drawn.
    letters: (f32, f32),
}

/// The width axis as far as the built-in subset of Google Sans Flex carries it.
const WDTH: (f32, f32) = (85.0, 125.0);

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        let sans = FontRef::from_index(SANS, 0).expect("Google Sans Flex");
        let code = FontRef::from_index(CODE, 0).expect("Google Sans Code");
        Text {
            sans,
            code,
            sans_map: sans.charmap(),
            code_map: code.charmap(),
            cjk: None,
            shape: ShapeContext::new(),
            scale: ScaleContext::new(),
            glyphs: Generations::new(GLYPHS, GLYPH_BYTES),
            runs: Generations::new(RUNS, RUN_BYTES),
            letters: (0.0, 1.0),
        }
    }

    /// Set the sans's letter style: `round` (0 crisp to 100 fully rounded)
    /// and `width` (1 is each role's own; 0.9 narrower, 1.1 wider).
    pub fn set_letters(&mut self, round: f32, width: f32) {
        self.letters = (round.clamp(0.0, 100.0), width.clamp(0.8, 1.2));
    }

    /// A style as the letter style makes it.
    fn adjusted(&self, style: &TextStyle) -> TextStyle {
        match style.face {
            Face::Sans => TextStyle { rond: style.rond.max(self.letters.0), wdth: (style.wdth * self.letters.1).clamp(WDTH.0, WDTH.1), ..*style },
            Face::Code => *style,
        }
    }

    /// Whether this has needed the CJK font yet (and so mapped it).
    pub fn cjk_loaded(&self) -> bool {
        matches!(self.cjk, Some(Some(_)))
    }

    /// The CJK font, found the first time it is asked for.
    fn cjk(&mut self) -> Option<(FontRef<'static>, Charmap<'static>)> {
        *self.cjk.get_or_insert_with(|| {
            let font = FontRef::from_index(cjk_data()?, 0)?;
            Some((font, font.charmap()))
        })
    }

    /// Which font draws `c` in a face: the face's own, then the other Latin
    /// face, then the CJK font. A character none of them has stays with the
    /// face's own (its box), or, if it is wide, goes to the CJK font's.
    fn font_of(&mut self, c: char, face: Face) -> Font {
        let (own, own_map, other, other_map) = match face {
            Face::Sans => (Font::Sans, self.sans_map, Font::Code, self.code_map),
            Face::Code => (Font::Code, self.code_map, Font::Sans, self.sans_map),
        };
        if own_map.map(c) != 0 || (c as u32) < 0x370 {
            return own;
        }
        if other_map.map(c) != 0 {
            return other;
        }
        if !in_cjk_blocks(c) {
            return own;
        }
        match self.cjk() {
            Some((_, map)) if map.map(c) != 0 || lines::is_wide(c) => Font::Cjk,
            _ => own,
        }
    }

    /// `text` in pieces, each drawn by one font.
    fn itemize(&mut self, text: &str, face: Face) -> Vec<(Font, std::ops::Range<usize>)> {
        let mut pieces: Vec<(Font, std::ops::Range<usize>)> = Vec::new();
        for (i, c) in text.char_indices() {
            let end = i + c.len_utf8();
            let font = match pieces.last() {
                Some((font, _)) if joins_previous(c) => *font,
                _ => self.font_of(c, face),
            };
            match pieces.last_mut() {
                Some((f, range)) if *f == font => range.end = end,
                _ => pieces.push((font, i..end)),
            }
        }
        pieces
    }

    /// The fonts `text` is drawn with in a face, character by character: for
    /// tests and for what shows which font a screen leaned on.
    pub fn fonts_of(&mut self, text: &str, face: Face) -> Vec<Font> {
        let mut out = Vec::new();
        for (font, range) in self.itemize(text, face) {
            out.extend(text[range].chars().map(|_| font));
        }
        out
    }

    /// A font, the size it is set at and its axes, for a style.
    fn voice(&self, font: Font, style: &TextStyle) -> (FontRef<'static>, f32, [(&'static str, f32); 4]) {
        match font {
            // The optical size follows the pixel size, which is what keeps
            // small text open and sturdy.
            Font::Sans => (self.sans, style.size, [("wght", style.wght), ("wdth", style.wdth), ("ROND", style.rond), ("opsz", style.size.clamp(6.0, 144.0))]),
            Font::Code => (self.code, style.size, [("wght", style.wght); 4]),
            Font::Cjk => {
                let cjk = self.cjk.flatten().expect("a CJK glyph without the CJK font").0;
                (cjk, style.cjk_size(), [("wght", style.wght); 4])
            }
        }
    }

    /// The width of one cell of the mono face at a size.
    fn cell(&self, size: f32) -> f32 {
        self.code.glyph_metrics(&[]).scale(size).advance_width(self.code_map.map('0'))
    }

    fn run(&mut self, text: &str, style: &TextStyle) -> Rc<Run> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        style.key().hash(&mut hasher);
        ((style.tracking * 1000.0) as i32).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(run) = self.runs.young.get(&key) {
            return run.clone();
        }
        if let Some(run) = self.runs.old.get(&key).cloned() {
            return self.runs.get_or_insert_with(&key, || run).clone();
        }

        let track = style.tracking * style.size;
        let mut glyphs = Vec::with_capacity(text.len());
        let mut pen = 0.0f32;
        for (font, range) in self.itemize(text, style.face) {
            let piece = &text[range];
            let (font_ref, size, axes) = self.voice(font, style);
            // In the mono face a wide character takes two cells.
            let cells = (font == Font::Cjk && style.face == Face::Code).then(|| 2.0 * self.cell(style.size));
            let mut shaper = self
                .shape
                .builder(font_ref)
                .size(size)
                .variations(axes)
                // Tabular figures, so values do not shift as they change.
                .features([("tnum", 1u16)])
                .build();
            shaper.add_str(piece);
            shaper.shape_with(|cluster| {
                let room = cells.filter(|_| piece.get(cluster.source.to_range()).and_then(|s| s.chars().next()).is_some_and(lines::is_wide));
                for g in cluster.glyphs {
                    match room {
                        Some(room) => {
                            glyphs.push(Placed { id: g.id, x: pen + g.x + (room - g.advance) * 0.5, font });
                            pen += room + track;
                        }
                        None => {
                            glyphs.push(Placed { id: g.id, x: pen + g.x, font });
                            pen += g.advance + track;
                        }
                    }
                }
            });
        }
        let width = if glyphs.is_empty() { 0.0 } else { pen - track };
        let run = Rc::new(Run { glyphs, width });
        self.runs.get_or_insert_with(&key, || run).clone()
    }

    pub fn measure(&mut self, text: &str, style: &TextStyle) -> f32 {
        let style = self.adjusted(style);
        self.run(text, &style).width
    }

    /// Height of capitals, for centring a line on something.
    pub fn cap_height(&self, style: &TextStyle) -> f32 {
        let font = match style.face {
            Face::Sans => self.sans,
            Face::Code => self.code,
        };
        let m = font.metrics(&[]).scale(style.size);
        if m.cap_height > 0.0 { m.cap_height } else { style.size * 0.7 }
    }

    /// Draw one line with its baseline at `baseline`. Returns its width.
    pub fn draw(&mut self, canvas: &mut Canvas, x: f32, baseline: f32, text: &str, style: &TextStyle, color: Color, alpha: f32) -> f32 {
        let style = &self.adjusted(style);
        let run = self.run(text, style);
        if alpha <= 0.0 {
            return run.width;
        }
        let by = baseline.round() as i32;
        for p in &run.glyphs {
            let px = x + p.x;
            let whole = px.floor();
            let sub = ((px - whole) * 4.0) as u8; // quarter-pixel positioning
            let (font, size, axes) = self.voice(p.font, style);
            let key: GlyphKey = match p.font {
                Font::Sans => (Font::Sans, (size * 4.0) as u16, style.wght as u16, style.wdth as u16, style.rond as u8, p.id, sub),
                other => (other, (size * 4.0) as u16, style.wght as u16, 0, 0, p.id, sub),
            };
            let scale = &mut self.scale;
            let glyph = self.glyphs.get_or_insert_with(&key, || {
                let mut scaler = scale.builder(font).size(size).hint(false).variations(axes).build();
                Render::new(&[Outline::Outline])
                    .format(Format::Alpha)
                    .offset(Vector::new(sub as f32 * 0.25, 0.0))
                    .render(&mut scaler, p.id)
                    .map(|img| Glyph { left: img.placement.left, top: img.placement.top, w: img.placement.width as i32, h: img.placement.height as i32, data: img.data })
            });
            if let Some(g) = glyph {
                canvas.mask(whole as i32 + g.left, by - g.top, g.w, g.h, &g.data, color, alpha);
            }
        }
        run.width
    }

    /// Draw a line placed by its left edge, centre or right edge.
    pub fn draw_aligned(&mut self, canvas: &mut Canvas, anchor: f32, baseline: f32, align: Align, text: &str, style: &TextStyle, color: Color, alpha: f32) -> f32 {
        let w = self.measure(text, style);
        let x = match align {
            Align::Left => anchor,
            Align::Center => anchor - w * 0.5,
            Align::Right => anchor - w,
        };
        self.draw(canvas, x, baseline, text, style, color, alpha);
        w
    }

    /// The longest prefix of `text` that fits `max_width`, ending in an
    /// ellipsis when it had to be cut.
    pub fn fit(&mut self, text: &str, style: &TextStyle, max_width: f32) -> String {
        if self.measure(text, style) <= max_width {
            return text.to_owned();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if self.measure(&candidate, style) <= max_width {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        // Not cut after a space, nor after a CJK opening bracket ("夜に「…").
        while lo > 0 && (chars[lo - 1].is_whitespace() || (lines::no_line_end(chars[lo - 1]) && lines::is_wide(chars[lo - 1]))) {
            lo -= 1;
        }
        chars[..lo].iter().collect::<String>() + "…"
    }

    /// `text` broken into lines no wider than `width`: at spaces, and
    /// between Chinese and Japanese characters as kinsoku allows (see
    /// [`lines::wrap`]). A word too long for a line is broken inside if
    /// `split_long`.
    pub fn wrap(&mut self, text: &str, style: &TextStyle, width: f32, split_long: bool) -> Vec<String> {
        lines::wrap(text, width, split_long, &mut |s| self.measure(s, style))
    }

    /// How many glyphs the cache holds, and about how many bytes.
    pub fn cached(&self) -> (usize, usize) {
        (self.glyphs.len(), self.glyphs.bytes())
    }

    /// The most the glyph cache can hold: glyphs, and bytes.
    pub const CACHE_MOST: (usize, usize) = (2 * GLYPHS, 2 * GLYPH_BYTES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use Font::{Cjk, Code, Sans};

    /// Whether the CJK font is here; if not, says so (the test that asked
    /// then checks nothing more).
    fn cjk_here() -> bool {
        let here = cjk_available();
        if !here {
            eprintln!("no {CJK_FILE} (scripts/cjk-font.py makes it): the CJK part of this test is skipped");
        }
        here
    }

    /// The rows a line's ink covers, drawn white on black with its baseline at 40.
    fn ink_rows(t: &mut Text, s: &str, style: &TextStyle) -> (i32, i32) {
        let mut c = Canvas::new(200, 64);
        t.draw(&mut c, 4.0, 40.0, s, style, Color::hex(0xffffff), 1.0);
        let rows: Vec<i32> = (0..64).filter(|&y| (0..200).any(|x| c.px[(y * 200 + x) as usize] & 0xff > 96)).collect();
        (*rows.first().unwrap(), *rows.last().unwrap())
    }

    #[test]
    fn latin_never_loads_the_cjk_font() {
        let mut t = Text::new();
        let mut c = Canvas::new(320, 40);
        for style in [TextStyle::body(13.0), TextStyle::label(11.0), TextStyle::title(16.0)] {
            t.draw(&mut c, 2.0, 20.0, "Harbour Lights · Wren & the Tide — 3:34 ü ñ ½ “ok” …", &style, Color::hex(0xffffff), 1.0);
        }
        // Nor do scripts it has nothing of.
        t.measure("مرحبا สวัสดี नमस्ते 😀", &TextStyle::body(13.0));
        assert!(!t.cjk_loaded());
        assert_eq!(t.fonts_of("Café 3:07", Face::Sans), [Sans; 9]);
        // An arrow the sans lacks is the mono face's, which has it.
        assert_eq!(t.fonts_of("a→b", Face::Sans), [Sans, Code, Sans]);
        assert!(!t.cjk_loaded());
    }

    #[test]
    fn each_character_is_drawn_by_the_first_font_that_has_it() {
        let mut t = Text::new();
        // Outside the Basic Multilingual Plane (an emoji) the CJK font is
        // never asked: the face's own box.
        assert_eq!(t.fonts_of("😀", Face::Sans), [Sans]);
        assert!(!t.cjk_loaded());
        if !cjk_here() {
            return;
        }
        assert_eq!(t.fonts_of("Aé夜に한Жω", Face::Sans), [Sans, Sans, Cjk, Cjk, Cjk, Cjk, Cjk]);
        assert!(t.cjk_loaded());
        assert_eq!(t.fonts_of("a1 漢字。", Face::Code), [Code, Code, Code, Cjk, Cjk, Cjk]);
        // A combining voiced mark goes with the kana it marks.
        assert_eq!(t.fonts_of("か\u{3099}", Face::Sans), [Cjk, Cjk]);
        // Full-width forms and the ideographic space are the CJK font's.
        assert_eq!(t.fonts_of("Ａ\u{3000}１", Face::Sans), [Cjk; 3]);
        // A wide character the subset lacks still goes to the CJK font,
        // which draws its box as wide as an ideograph.
        let (cjk, map) = t.cjk().unwrap();
        let _ = cjk;
        let missing = (0x3400u32..0x4DB5).filter_map(char::from_u32).find(|&c| map.map(c) == 0).expect("an extension A character outside the subset");
        assert_eq!(t.fonts_of(&missing.to_string(), Face::Sans), [Cjk]);
        let style = TextStyle::body(13.0);
        assert!((t.measure(&missing.to_string(), &style) - style.cjk_size()).abs() < 0.5);
    }

    #[test]
    fn mixed_text_measures_as_the_sum_of_its_parts() {
        if !cjk_here() {
            return;
        }
        let mut t = Text::new();
        for style in [TextStyle::body(13.0), TextStyle::title(17.0), TextStyle::display(30.0), TextStyle::body(11.0)] {
            let style = TextStyle { tracking: 0.0, ..style };
            let latin = t.measure("YOASOBI ", &style);
            let cjk = t.measure("夜に駆ける", &style);
            let both = t.measure("YOASOBI 夜に駆ける", &style);
            assert!((both - latin - cjk).abs() < 0.01, "{both} != {latin} + {cjk}");
            // An ideograph or a kana is one CJK em wide.
            assert!((cjk - 5.0 * style.cjk_size()).abs() < 0.6, "{cjk} at {}", style.size);
        }
        // Never under 11 px where the Latin is not: the caption's CJK is legible.
        assert_eq!(TextStyle::body(11.0).cjk_size(), 11.0);
        assert!((TextStyle::body(15.0).cjk_size() - 13.8).abs() < 0.01);
        // In the mono face a wide character is two cells.
        let code = TextStyle::label(12.0);
        let cell = t.measure("0", &code);
        assert!((t.measure("漢字", &code) - 4.0 * cell).abs() < 0.01);
        assert!((t.measure("ab漢字。", &code) - 8.0 * cell).abs() < 0.01);
    }

    #[test]
    fn fitting_cuts_cjk_to_the_width_with_an_ellipsis() {
        if !cjk_here() {
            return;
        }
        let mut t = Text::new();
        let style = TextStyle::name(15.0);
        let long = "千と千尋の神隠し オリジナル・サウンドトラック";
        for width in [40.0, 80.0, 120.0, 200.0] {
            let fitted = t.fit(long, &style, width);
            assert!(fitted.ends_with('…'), "{fitted}");
            assert!(t.measure(&fitted, &style) <= width, "{fitted} at {width}");
            // As much as fits: one more character would not.
            let kept = fitted.trim_end_matches('…').chars().count();
            let more: String = long.chars().take(kept + 1).collect::<String>() + "…";
            assert!(t.measure(&more, &style) > width || more.trim_end_matches('…').ends_with(char::is_whitespace), "{more} would fit {width}");
        }
        // Not cut just after an opening bracket.
        let room = t.measure("曲「…", &style) + 1.0;
        let fitted = t.fit("曲「夜に駆ける」", &style, room);
        assert_eq!(fitted, "曲…");
    }

    #[test]
    fn cjk_sits_on_the_latin_baseline_a_little_taller_than_capitals() {
        if !cjk_here() {
            return;
        }
        let mut t = Text::new();
        for style in [TextStyle::body(13.0), TextStyle::title(18.0), TextStyle::label(12.0)] {
            let (cap_top, cap_bottom) = ink_rows(&mut t, "H", &style);
            let (han_top, han_bottom) = ink_rows(&mut t, "漢", &style);
            let em = style.size;
            // Its foot on the baseline (an ideograph's ink comes a touch below it).
            assert!((han_bottom - cap_bottom).abs() as f32 <= 0.12 * em + 1.0, "{style:?}: foot {han_bottom} vs {cap_bottom}");
            // Its head above the capitals' by less than a quarter em.
            let over = (cap_top - han_top) as f32;
            assert!((0.0..=0.3 * em + 1.0).contains(&over), "{style:?}: head {han_top} vs {cap_top}");
        }
    }

    #[test]
    fn cjk_titles_are_as_heavy_as_their_latin() {
        if !cjk_here() {
            return;
        }
        let mut t = Text::new();
        let ink = |t: &mut Text, style: &TextStyle| {
            let mut c = Canvas::new(60, 40);
            t.draw(&mut c, 4.0, 30.0, "国", style, Color::hex(0xffffff), 1.0);
            c.px.iter().map(|p| (p & 0xff) as u32).sum::<u32>()
        };
        let (light, body, title) = (ink(&mut t, &TextStyle::display(18.0)), ink(&mut t, &TextStyle::body(18.0)), ink(&mut t, &TextStyle::title(18.0)));
        assert!(light < body && body < title, "{light} {body} {title}");
    }

    #[test]
    fn the_glyph_cache_stays_within_its_bounds() {
        let mut t = Text::new();
        let mut c = Canvas::new(320, 170);
        let han: String = if cjk_here() { (0x4E00u32..0x4E00 + 2500).filter_map(char::from_u32).collect() } else { String::new() };
        let latin: String = ('!'..='~').collect();
        // Every size and quarter-pixel offset of every letter: far more
        // glyphs than the cache keeps.
        for size in (10..64).step_by(3) {
            for q in 0..4 {
                let style = TextStyle::body(size as f32);
                t.draw(&mut c, 2.0 + q as f32 * 0.25, 60.0, &latin, &style, Color::hex(0xffffff), 1.0);
                let (glyphs, bytes) = t.cached();
                assert!(glyphs <= Text::CACHE_MOST.0 && bytes <= Text::CACHE_MOST.1, "{glyphs} glyphs, {bytes} bytes");
            }
        }
        for chunk in han.chars().collect::<Vec<_>>().chunks(20) {
            let s: String = chunk.iter().collect();
            t.draw(&mut c, 2.0, 60.0, &s, &TextStyle::body(13.0), Color::hex(0xffffff), 1.0);
            let (glyphs, bytes) = t.cached();
            assert!(glyphs <= Text::CACHE_MOST.0 && bytes <= Text::CACHE_MOST.1, "{glyphs} glyphs, {bytes} bytes");
        }
        // What was drawn last is kept: drawing it again adds nothing.
        let before = t.cached();
        t.draw(&mut c, 2.0, 60.0, &latin, &TextStyle::body(13.0), Color::hex(0xffffff), 1.0);
        t.draw(&mut c, 2.0, 60.0, &latin, &TextStyle::body(13.0), Color::hex(0xffffff), 1.0);
        let after = t.cached();
        t.draw(&mut c, 2.0, 60.0, &latin, &TextStyle::body(13.0), Color::hex(0xffffff), 1.0);
        assert_eq!(t.cached(), after, "a line drawn twice is drawn from the cache");
        let _ = before;
    }

    #[test]
    fn wrapping_real_text_keeps_every_line_inside_the_width() {
        if !cjk_here() {
            return;
        }
        let mut t = Text::new();
        let style = TextStyle::body(13.0);
        let text = "今日はとても良い天気ですね。明日は雨が降るかもしれません。「傘」を忘れずに！ Catalyst 0.5.6 は新しい機能を追加しました。";
        for width in [80.0, 120.0, 200.0, 300.0] {
            let lines = t.wrap(text, &style, width, true);
            assert_eq!(lines.concat().replace(' ', ""), text.replace(' ', ""), "at {width}");
            for (i, line) in lines.iter().enumerate() {
                assert!(t.measure(line, &style) <= width, "{line} wider than {width}");
                if i > 0 {
                    assert!(!lines::no_line_start(line.chars().next().unwrap()), "{line} starts a line at {width}");
                }
            }
        }
    }
}
