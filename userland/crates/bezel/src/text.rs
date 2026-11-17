//! Bezel's two voices of type: Google Sans Flex for what is read at a glance
//! and lowercase Google Sans Code for quiet labels. Both are variable fonts;
//! glyphs are shaped and rasterized on demand and kept in a bounded cache.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use swash::scale::{Render, ScaleContext, Source};
use swash::shape::ShapeContext;
use swash::zeno::{Format, Vector};
use swash::FontRef;

use crate::canvas::Canvas;
use crate::color::Color;

static SANS: &[u8] = include_bytes!("../../../assets/fonts/GoogleSansFlex.ttf");
static CODE: &[u8] = include_bytes!("../../../assets/fonts/GoogleSansCode.ttf");

const MAX_GLYPHS: usize = 1500;
const MAX_RUNS: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Sans,
    Code,
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
}

struct Glyph {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    data: Vec<u8>,
}

struct Run {
    /// Glyph id and pen x, relative to the run's start.
    glyphs: Vec<(u16, f32)>,
    width: f32,
}

type GlyphKey = ((Face, u16, u16, u16, u8), u16, u8);

pub struct Text {
    sans: FontRef<'static>,
    code: FontRef<'static>,
    shape: ShapeContext,
    scale: ScaleContext,
    glyphs: HashMap<GlyphKey, Option<Glyph>>,
    runs: HashMap<u64, Rc<Run>>,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        Text {
            sans: FontRef::from_index(SANS, 0).expect("Google Sans Flex"),
            code: FontRef::from_index(CODE, 0).expect("Google Sans Code"),
            shape: ShapeContext::new(),
            scale: ScaleContext::new(),
            glyphs: HashMap::new(),
            runs: HashMap::new(),
        }
    }

    fn font(&self, face: Face) -> FontRef<'static> {
        match face {
            Face::Sans => self.sans,
            Face::Code => self.code,
        }
    }

    fn variations(style: &TextStyle) -> [(&'static str, f32); 4] {
        match style.face {
            // The optical size follows the pixel size, which is what keeps
            // small text open and sturdy.
            Face::Sans => [("wght", style.wght), ("wdth", style.wdth), ("ROND", style.rond), ("opsz", style.size.clamp(6.0, 144.0))],
            Face::Code => [("wght", style.wght), ("wght", style.wght), ("wght", style.wght), ("wght", style.wght)],
        }
    }

    fn run(&mut self, text: &str, style: &TextStyle) -> Rc<Run> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        style.key().hash(&mut hasher);
        ((style.tracking * 1000.0) as i32).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(run) = self.runs.get(&key) {
            return run.clone();
        }

        let font = self.font(style.face);
        let mut shaper = self
            .shape
            .builder(font)
            .size(style.size)
            .variations(Self::variations(style))
            // Tabular figures, so values do not shift as they change.
            .features([("tnum", 1u16)])
            .build();
        shaper.add_str(text);
        let track = style.tracking * style.size;
        let mut glyphs = Vec::with_capacity(text.len());
        let mut pen = 0.0f32;
        shaper.shape_with(|cluster| {
            for g in cluster.glyphs {
                glyphs.push((g.id, pen + g.x));
                pen += g.advance + track;
            }
        });
        let width = if glyphs.is_empty() { 0.0 } else { pen - track };
        let run = Rc::new(Run { glyphs, width });
        if self.runs.len() >= MAX_RUNS {
            self.runs.clear();
        }
        self.runs.insert(key, run.clone());
        run
    }

    pub fn measure(&mut self, text: &str, style: &TextStyle) -> f32 {
        self.run(text, style).width
    }

    /// Height of capitals, for centring a line on something.
    pub fn cap_height(&self, style: &TextStyle) -> f32 {
        let m = self.font(style.face).metrics(&[]).scale(style.size);
        if m.cap_height > 0.0 { m.cap_height } else { style.size * 0.7 }
    }

    /// Draw one line with its baseline at `baseline`. Returns its width.
    pub fn draw(&mut self, canvas: &mut Canvas, x: f32, baseline: f32, text: &str, style: &TextStyle, color: Color, alpha: f32) -> f32 {
        let run = self.run(text, style);
        if alpha <= 0.0 {
            return run.width;
        }
        let font = self.font(style.face);
        let style_key = style.key();
        let by = baseline.round() as i32;
        for &(id, gx) in &run.glyphs {
            let px = x + gx;
            let whole = px.floor();
            let sub = ((px - whole) * 4.0) as u8; // quarter-pixel positioning
            let key = (style_key, id, sub);
            if !self.glyphs.contains_key(&key) {
                if self.glyphs.len() >= MAX_GLYPHS {
                    self.glyphs.clear();
                }
                let mut scaler = self
                    .scale
                    .builder(font)
                    .size(style.size)
                    .hint(false)
                    .variations(Self::variations(style))
                    .build();
                let glyph = Render::new(&[Source::Outline])
                    .format(Format::Alpha)
                    .offset(Vector::new(sub as f32 * 0.25, 0.0))
                    .render(&mut scaler, id)
                    .map(|img| Glyph {
                        left: img.placement.left,
                        top: img.placement.top,
                        w: img.placement.width as i32,
                        h: img.placement.height as i32,
                        data: img.data,
                    });
                self.glyphs.insert(key, glyph);
            }
            if let Some(Some(g)) = self.glyphs.get(&key) {
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
        chars[..lo].iter().collect::<String>().trim_end().to_owned() + "…"
    }
}
