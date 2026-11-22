//! Flow's pictures: an album card for every app, the back of the card that
//! is enlarged, and the atlas they are kept in, laid out as the GPU takes
//! them up so both draw from the same texels.
//!
//! A card is art, not a tile: a gradient in the app's own colours (Bezel's
//! case gradient for its hue, grown by Detent's generator), lit from the top
//! left, its flat icon large on it and a larger ghost of it cropped at the
//! corner, and its name. Icons stay flat; only the cards stand in 3D.

use bezel::canvas::sd_rrect;
use bezel::{Canvas, Color, Icons, Rect, Text, TextStyle};

use super::palette::CARDS;

/// The largest texture side the VideoCore IV takes.
const MAX_SIDE: usize = 2048;

/// Levels kept of each picture: whole, half and quarter width. A card turned
/// away is squeezed sideways only, so its narrower levels are squeezed only
/// sideways too, and it is sampled from the one that matches.
pub const LEVELS: usize = 3;

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Pictures of one size, each at its three widths with a transparent texel
/// all round, premultiplied `0xAARRGGBB`.
///
/// Kept a column at a time (texel (x, y) at `x * h + y`): a card is drawn
/// down the panel's columns, each reading down one column of its picture,
/// and those texels are then side by side in memory rather than a whole
/// atlas row (8 KB) apart. The GPU takes it the same way, as a texture `h`
/// wide and `w` tall, and reads it across.
pub struct Atlas {
    /// Its size: `w` across (the GPU's rows), `h` down.
    #[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
    /// One picture's size in texels at its full width.
    pub sw: usize,
    pub sh: usize,
    cols: usize,
    /// Which taking of the pictures this is, so the GPU sends them up only
    /// when they change.
    pub generation: u64,
}

impl Atlas {
    pub fn new(sw: usize, sh: usize, count: usize) -> Atlas {
        assert!(sw % 4 == 0 && sw >= 8, "a picture's width divides by four");
        let slot = Self::slot_w(sw);
        let count = count.max(1);
        let cols = ((MAX_SIDE - 1) / slot).clamp(1, count);
        let rows = count.div_ceil(cols);
        // One more column and row, transparent, so a sample at the very
        // edge reads nothing that is not there.
        let (w, h) = (cols * slot + 1, rows * (sh + 2) + 1);
        Atlas { w, h, px: vec![0; w * h], sw, sh, cols, generation: next_generation() }
    }

    fn slot_w(sw: usize) -> usize {
        (sw + 2) + (sw / 2 + 2) + (sw / 4 + 2)
    }

    /// Where level `l` of picture `i` begins (its transparent edge
    /// included) and how wide it is, edges and all.
    pub fn level(&self, i: usize, l: usize) -> (usize, usize, usize) {
        let x = (i % self.cols) * Self::slot_w(self.sw)
            + match l {
                0 => 0,
                1 => self.sw + 2,
                _ => self.sw + 2 + self.sw / 2 + 2,
            };
        (x, (i / self.cols) * (self.sh + 2), (self.sw >> l) + 2)
    }

    /// Where texel (`x`, `y`) is kept.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> usize {
        x * self.h + y
    }

    /// Texel (`x`, `y`).
    #[cfg(test)]
    pub fn texel(&self, x: usize, y: usize) -> u32 {
        self.px[self.at(x, y)]
    }

    /// Picture `i` (`sw` × `sh`, premultiplied, row by row), with its narrower levels.
    pub fn put(&mut self, i: usize, art: &[u32]) {
        let (sw, sh) = (self.sw, self.sh);
        assert_eq!(art.len(), sw * sh);
        let mut level = art.to_vec();
        let mut lw = sw;
        for l in 0..LEVELS {
            let (x0, y0, _) = self.level(i, l);
            for x in 0..lw {
                let at = self.at(x0 + 1 + x, y0 + 1);
                for y in 0..sh {
                    self.px[at + y] = level[y * lw + x];
                }
            }
            // Half as wide: each pair of texels averaged.
            let half = lw / 2;
            let mut next = vec![0u32; half * sh];
            for y in 0..sh {
                for x in 0..half {
                    let (a, b) = (level[y * lw + 2 * x], level[y * lw + 2 * x + 1]);
                    next[y * half + x] = ((a & 0xfefe_fefe) >> 1) + ((b & 0xfefe_fefe) >> 1) + (a & b & 0x0101_0101);
                }
            }
            (level, lw) = (next, half);
        }
        self.generation = next_generation();
    }
}

/// A card's colours: (the gradient's lit end, its middle, its deep end, the
/// glow it casts), from its hue (0 to 71, every 5°).
pub fn colours(hue: u8) -> (Color, Color, Color, Color) {
    let (a, b, signal, ground) = CARDS[hue as usize % CARDS.len()];
    let (a, b, ground) = (Color::hex(a), Color::hex(b), Color::hex(ground));
    (a, b, b.mix(ground, 0.55), Color::hex(signal))
}

/// What a card shows.
#[derive(Clone, PartialEq, Debug)]
pub struct CardSpec {
    pub icon: String,
    pub name: String,
    pub hue: u8,
}

/// How a card's words are set: the name's styles, largest first, tried in
/// turn until the name fits; whether it goes lowercase (Bezel's mono
/// voice); and the card's corner radius.
#[derive(Clone, Copy)]
pub struct Lettering {
    pub names: [TextStyle; 3],
    pub lowercase: bool,
    pub radius: f32,
    /// How far the name sits in from the card's edges.
    pub margin: f32,
    /// The back's: its title, words and reading.
    pub title: TextStyle,
    pub words: TextStyle,
    pub reading: TextStyle,
    pub caption: TextStyle,
}

/// The lines a name is set in on a card `width` wide: one line at the
/// largest style it fits whole at, else two lines (broken at a space) at the
/// largest that takes both, else one line cut short. The flag says whether
/// it is whole.
pub fn name_lines(text: &mut Text, name: &str, styles: &[TextStyle], width: f32) -> (Vec<String>, TextStyle, bool) {
    for style in styles {
        if text.measure(name, style) <= width {
            return (vec![name.to_owned()], *style, true);
        }
    }
    for style in styles {
        let words: Vec<&str> = name.split(' ').collect();
        for cut in (1..words.len()).rev() {
            let (a, b) = (words[..cut].join(" "), words[cut..].join(" "));
            if text.measure(&a, style) <= width && text.measure(&b, style) <= width {
                return (vec![a, b], *style, true);
            }
        }
    }
    let style = *styles.last().expect("a style");
    (vec![text.fit(name, &style, width)], style, false)
}

/// The rounded card's shape over a picture: transparent outside, its edge
/// smooth, everything premultiplied, and a hairline rim lit from the top
/// left (brighter where it faces the light, a touch darker where it faces
/// away), as a physical card's edge catches one light.
fn finish(c: &Canvas, radius: f32) -> Vec<u32> {
    let (w, h) = (c.w as usize, c.h as usize);
    let r = Rect::new(0.0, 0.0, w as f32, h as f32);
    let mut out = vec![0u32; w * h];
    let light = (-0.6f32, -0.8f32);
    // Well inside the edge and the corners: opaque, no rim; nothing to work out.
    let clear = radius.ceil() as usize + 2;
    let inside = |x: usize, y: usize| (x >= 2 && x + 2 < w && y >= clear && y + clear < h) || (x >= clear && x + clear < w && y >= 2 && y + 2 < h);
    for y in 0..h {
        for x in 0..w {
            if inside(x, y) {
                out[y * w + x] = 0xff00_0000 | (c.px[y * w + x] & 0x00ff_ffff);
                continue;
            }
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = sd_rrect(fx, fy, &r, radius);
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let mut p = Color((c.px[y * w + x] >> 16) as u8, (c.px[y * w + x] >> 8) as u8, c.px[y * w + x] as u8);
            if d > -1.6 {
                // Which way the edge faces: the distance's own slope.
                let (nx, ny) = (sd_rrect(fx + 0.5, fy, &r, radius) - sd_rrect(fx - 0.5, fy, &r, radius), sd_rrect(fx, fy + 0.5, &r, radius) - sd_rrect(fx, fy - 0.5, &r, radius));
                let len = (nx * nx + ny * ny).sqrt().max(1e-4);
                let facing = (nx * light.0 + ny * light.1) / len;
                let line = (1.0 - (-d) / 1.6).clamp(0.0, 1.0);
                p = if facing > 0.0 { p.mix(Color::WHITE, 0.34 * facing * line) } else { p.mix(Color::BLACK, 0.22 * -facing * line) };
            }
            let a = (cover * 255.0 + 0.5) as u32;
            let pm = |v: u8| (v as u32 * a + 127) / 255;
            out[y * w + x] = a << 24 | pm(p.0) << 16 | pm(p.1) << 8 | pm(p.2);
        }
    }
    out
}

/// The ground every card and back is painted on: its gradient from the lit
/// top-left corner to the deep bottom-right one, and a soft light falling
/// from the top left.
fn ground(c: &mut Canvas, hue: u8, depth: f32) {
    let (lit, mid, deep, _) = colours(hue);
    let (w, h) = (c.w as f32, c.h as f32);
    let span = w * 0.45 + h * 0.8;
    // The gradient's colours, 256 steps of it, looked up a pixel at a time.
    let steps: Vec<u32> = (0..256)
        .map(|i| {
            let t = (i as f32 + 0.5) / 256.0;
            let col = if t < 0.42 { lit.mix(mid, t / 0.42) } else { mid.mix(deep, (t - 0.42) / 0.58) };
            col.mix(Color::BLACK, depth).to_u32()
        })
        .collect();
    let k = 256.0 / span;
    for y in 0..c.h {
        let down = (y as f32 + 0.5) * 0.8 * k;
        let row = &mut c.px[(y * c.w) as usize..((y + 1) * c.w) as usize];
        for (x, p) in row.iter_mut().enumerate() {
            *p = steps[((x as f32 + 0.5) * 0.45 * k + down).min(255.0) as usize];
        }
    }
    c.glow(-0.1 * w, -0.2 * h, w.max(h) * 1.05, Color::WHITE, 0.2);
}

/// A shade falling over the lower part, so the name reads on any colour.
fn scrim(c: &mut Canvas, from: f32, strength: f32) {
    let y0 = (c.h as f32 * from) as i32;
    for y in y0..c.h {
        let t = (y - y0) as f32 / (c.h - y0) as f32;
        c.fill_rect(Rect::new(0.0, y as f32, c.w as f32, 1.0), Color::BLACK, strength * t.powf(1.4));
    }
}

/// One app's album card, `size` texels square.
pub fn card(text: &mut Text, icons: &mut Icons, spec: &CardSpec, lettering: &Lettering, size: usize) -> Vec<u32> {
    let n = size as f32;
    let mut c = Canvas::new(size as i32, size as i32);
    ground(&mut c, spec.hue, 0.0);
    // Its icon again, far larger and faint, cropped at the corner: depth
    // behind the flat one.
    icons.draw(&mut c, &spec.icon, true, n * 0.76, n * 0.72, n * 1.12, Color::WHITE, 0.075);
    scrim(&mut c, 0.5, 0.42);
    // The icon itself, flat and large, on a quiet disc.
    let (icx, icy) = (n * 0.31, n * 0.31);
    c.circle(icx, icy, n * 0.205, Color::WHITE, 0.16);
    icons.draw(&mut c, &spec.icon, true, icx, icy, n * 0.27, Color::WHITE, 1.0);
    // The name, along the bottom left.
    let margin = lettering.margin;
    let name = if lettering.lowercase { spec.name.to_lowercase() } else { spec.name.clone() };
    let (lines, style, _) = name_lines(text, &name, &lettering.names, n - 2.0 * margin);
    let lead = (style.size * 1.08).round();
    let mut baseline = n - margin;
    for line in lines.iter().rev() {
        text.draw(&mut c, margin, baseline, line, &style, Color::WHITE, 1.0);
        baseline -= lead;
    }
    finish(&c, lettering.radius)
}

/// What the back of an enlarged card says.
#[derive(Clone, PartialEq, Debug)]
pub struct FaceSpec {
    pub card: CardSpec,
    /// What the app does, in a few words.
    pub about: String,
    /// What it says just now (its Home tile's line), split as a reading
    /// ("12.4", "v") when it is one.
    pub status: String,
    pub reading: Option<(String, String)>,
    /// A warning or fault: its colour.
    pub attention: Option<Color>,
    /// What Enter does here.
    pub hint: String,
}

/// Words broken into at most `lines` lines of `width`, the last cut short if
/// they run on.
pub fn wrap(text: &mut Text, words: &str, style: &TextStyle, width: f32, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut rest = words.split_whitespace().peekable();
    while let Some(word) = rest.next() {
        let tried = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if text.measure(&tried, style) <= width || line.is_empty() {
            line = tried;
            continue;
        }
        if out.len() + 1 == lines {
            // The last line: what is left, cut to fit.
            let left: Vec<&str> = std::iter::once(word).chain(rest.by_ref()).collect();
            line = format!("{line} {}", left.join(" "));
            break;
        }
        let done = std::mem::replace(&mut line, word.to_owned());
        out.push(text.fit(&done, style, width));
    }
    if !line.is_empty() {
        out.push(text.fit(&line, style, width));
    }
    out
}

/// The back of an enlarged card, `w` × `h` texels: the app's name, what it
/// does, and what it says just now, large when it is a reading.
pub fn face(text: &mut Text, icons: &mut Icons, spec: &FaceSpec, lettering: &Lettering, w: usize, h: usize) -> Vec<u32> {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w as i32, h as i32);
    ground(&mut c, spec.card.hue, 0.4);
    icons.draw(&mut c, &spec.card.icon, true, fw - fh * 0.32, fh * 0.62, fh * 1.25, Color::WHITE, 0.06);
    let pad = 12.0;
    let ink = Color::WHITE;
    let quiet = Color::hex(0xd9dce0);
    // The icon on its disc, and the name beside it.
    let (_, mid, _, _) = colours(spec.card.hue);
    let disc = (pad + 15.0, pad + 15.0);
    c.circle(disc.0, disc.1, 15.0, mid, 1.0);
    c.circle(disc.0, disc.1, 15.0, Color::WHITE, 0.1);
    icons.draw(&mut c, &spec.card.icon, true, disc.0, disc.1, 19.0, ink, 1.0);
    let x = disc.0 + 15.0 + 9.0;
    let room = fw - x - pad;
    let name = if lettering.lowercase { spec.card.name.to_lowercase() } else { spec.card.name.clone() };
    let title = text.fit(&name, &lettering.title, room);
    let cap = |s: &TextStyle| (s.size * 0.36).round();
    text.draw(&mut c, x, (disc.1 - 6.0 + cap(&lettering.title)).round(), &title, &lettering.title, ink, 1.0);
    // What it does, under the name (two lines at most).
    let about = wrap(text, &spec.about, &lettering.caption, room, 2);
    let mut y = disc.1 + 9.0;
    for line in &about {
        text.draw(&mut c, x, (y + cap(&lettering.caption)).round(), line, &lettering.caption, quiet, 1.0);
        y += lettering.caption.size + 3.0;
    }
    // What it says just now, along the bottom: large if it is a reading.
    let baseline = fh - pad;
    let hint_w = text.measure(&spec.hint, &lettering.caption);
    let status_room = fw - 2.0 * pad - hint_w - 10.0;
    let mut sx = pad;
    if let Some(colour) = spec.attention {
        c.circle(sx + 4.0, baseline - lettering.words.size * 0.36, 4.0, colour, 1.0);
        sx += 13.0;
    }
    match &spec.reading {
        Some((value, unit)) if text.measure(value, &lettering.reading) + 4.0 + text.measure(unit, &lettering.caption) <= status_room - (sx - pad) => {
            let vw = text.draw(&mut c, sx, baseline, value, &lettering.reading, ink, 1.0);
            text.draw(&mut c, sx + vw + 4.0, baseline, unit, &lettering.caption, quiet, 1.0);
        }
        _ => {
            let status = text.fit(&spec.status, &lettering.words, status_room - (sx - pad));
            text.draw(&mut c, sx, baseline, &status, &lettering.words, ink, 1.0);
        }
    }
    text.draw_aligned(&mut c, fw - pad, baseline, bezel::Align::Right, &spec.hint, &lettering.caption, quiet, 1.0);
    finish(&c, lettering.radius + 2.0)
}
