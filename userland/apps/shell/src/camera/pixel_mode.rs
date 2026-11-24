//! Pixel mode (1.2): a toy camera's pixel pictures, made live from the
//! preview (filters/pixel.rs) and kept as they are shown.
//!
//! - **The viewfinder** shows the picture with whole pixels: 56 × 42 at
//!   four panel pixels each, 75 × 56 at three, 112 × 84 at two, every one
//!   the same size (224 or 225 × 168 of the viewfinder's 227 × 170).
//! - **Its choices** are the bar's chips (camera/bar.rs), each with its key
//!   and a picker that shows the choice on the picture while it is in hand:
//!   the size (`s` steps it), the palette (`k`, each with its swatches),
//!   the dither (`d` steps it, each picker cell a sample of its pattern) and
//!   the tone (↑ ↓ the brightness at any time, `-` `=` or its picker the
//!   contrast). All of it is remembered. `a` opens the album, since ↓ is
//!   the tone's here.
//! - **The shutter** keeps the picture in sight (no photo is asked of the
//!   camera, so it is at once): an indexed PNG at its true size, kept as the
//!   original, and one blown up with whole pixels for sharing (×8, ×6 or ×4,
//!   about 450 across), the album's own, `PIX_…png`. The flash is the
//!   palette's lightest colour laid over the picture's own pixels and taken
//!   away in the Bayer matrix's order, on Material 3's slow effects spring
//!   (`m3-slow-effects`: stiffness 800, critically damped, settled in a third of a second); the
//!   shutter's disc is drawn in pixels too, and presses in a step. The
//!   timer works as in photo mode.
//! - At rest nothing here keeps the frame beat: frames are drawn as they
//!   arrive, as in every mode.

use super::bar::{Chip, Picker};
use super::*;
use crate::filters::pixel::{Contrast, Settings, BRIGHT_STEPS, CONTRASTS, DITHERS, PALETTES, SIZES};

/// The shutter's flash: Material 3's slow effects spring (`m3-slow-effects`,
/// stiffness 800, damping ratio 1), an opacity that never bounces.
pub(super) fn flash_spring() -> detent::Spring {
    detent::Spring::stiffness_ratio(800.0, 1.0)
}

/// "+1", "±0", "-2".
pub(super) fn bright_words(b: i8) -> String {
    match b {
        0 => "±0".to_owned(),
        b if b > 0 => format!("+{b}"),
        b => format!("−{}", -b),
    }
}

/// Whole panel pixels to each of a picture's in the viewfinder.
fn view_scale(vf: Rect, pic: &Picture) -> f32 {
    if pic.is_empty() {
        return 0.0;
    }
    (vf.w / pic.w as f32).min(vf.h / pic.h as f32).floor()
}

/// A pixel picture (not yet a frame of the whole preview): at least two
/// panel pixels to each of its own.
pub(super) fn fits_whole(vf: Rect, pic: &Picture) -> bool {
    view_scale(vf, pic) >= 2.0
}

/// Where the picture goes in the viewfinder: in its middle, whole pixels.
fn view_rect(vf: Rect, pic: &Picture) -> (Rect, f32) {
    let s = view_scale(vf, pic);
    let (w, h) = (pic.w as f32 * s, pic.h as f32 * s);
    (Rect::new((vf.x + (vf.w - w) * 0.5).floor(), (vf.y + (vf.h - h) * 0.5).floor(), w, h), s)
}

/// The viewfinder's pixel picture, and the shutter's flash over it: the
/// palette's lightest colour on the picture's own pixels (two or three
/// together where they are small), each going as the flash falls past its
/// place in the Bayer matrix.
pub(super) fn draw_view(g: &mut Gfx, vf: Rect, pic: &Picture, alpha: f32, flash: f32, palette: pixel::Palette) {
    let (dest, s) = view_rect(vf, pic);
    draw_picture_crisp(g, dest, pic, whole(pic), 0.0, alpha);
    if flash <= 0.01 {
        return;
    }
    let light = palette.colours().iter().copied().max_by_key(|c| 77 * u32::from(c[0]) + 150 * u32::from(c[1]) + 29 * u32::from(c[2])).unwrap_or([255; 3]);
    let colour = Color(light[0], light[1], light[2]);
    let cell = s * (4.0 / s).ceil().max(1.0);
    let (nx, ny) = ((dest.w / cell).ceil() as usize, (dest.h / cell).ceil() as usize);
    for by in 0..ny {
        for bx in 0..nx {
            let t = (f32::from(pixel::BAYER[by & 3][bx & 3]) + 0.5) / 16.0;
            if t < flash {
                let r = Rect::new(dest.x + bx as f32 * cell, dest.y + by as f32 * cell, cell.min(dest.right() - dest.x - bx as f32 * cell), cell.min(dest.bottom() - dest.y - by as f32 * cell));
                g.canvas.fill_rect(r.offset(g.ox, 0.0), colour, 0.92 * alpha);
            }
        }
    }
}

/// A disc of radius `r` about (`cx`, `cy`), drawn in blocks of three
/// panel pixels: pixel mode's shutter.
pub(super) fn draw_pixel_disc(g: &mut Gfx, cx: f32, cy: f32, r: f32, colour: Color, alpha: f32) {
    let b = 3.0;
    let rows = ((2.0 * r) / b).round().max(1.0) as i32;
    let top = (cy - rows as f32 * b * 0.5).round();
    for i in 0..rows {
        let yc = top + (i as f32 + 0.5) * b;
        let half = (r * r - (yc - cy).powi(2)).max(0.0).sqrt();
        let cols = ((2.0 * half) / b).round() as i32;
        if cols <= 0 {
            continue;
        }
        let left = (cx - cols as f32 * b * 0.5).round();
        g.canvas.fill_rect(Rect::new(left + g.ox, top + i as f32 * b, cols as f32 * b, b), colour, alpha);
    }
}

/// A small bitmap (rows of '#' and '.') drawn `q` panel pixels a dot, its
/// left edge at `x`, centred on `cy`.
fn glyph(g: &mut Gfx, rows: &[&str], x: f32, cy: f32, q: f32, colour: Color, alpha: f32) {
    let top = (cy - rows.len() as f32 * q * 0.5).round();
    for (j, row) in rows.iter().enumerate() {
        for (i, c) in row.chars().enumerate() {
            if c == '#' {
                g.canvas.fill_rect(Rect::new(x.round() + i as f32 * q + g.ox, top + j as f32 * q, q, q), colour, alpha);
            }
        }
    }
}

/// Pixel mode's camera, for its mode chip: a body, its viewfinder, its lens.
const CAMERA_GLYPH: [&str; 5] = [".##...", "######", "##..##", "##..##", "######"];

/// Each dither's pattern, for its chip.
fn dither_glyph(d: pixel::Dither) -> [&'static str; 4] {
    match d {
        pixel::Dither::None => ["##..", "##..", "##..", "##.."],
        pixel::Dither::Bayer => ["#.#.", ".#.#", "#.#.", ".#.#"],
        pixel::Dither::Floyd => ["#.#.", ".##.", "#..#", ".#.#"],
        pixel::Dither::Atkinson => ["#...", "..#.", ".#..", "...#"],
    }
}

/// Swatches of `colours` in `rows` rows, each `q` square, a pixel apart,
/// on a quiet mat so the darkest and lightest show on either glass.
#[allow(clippy::too_many_arguments)]
fn swatches(g: &mut Gfx, colours: &[[u8; 3]], rows: usize, x: f32, cy: f32, q: f32, mat: Color, alpha: f32) -> f32 {
    let cols = colours.len().div_ceil(rows.max(1));
    let (w, h) = (cols as f32 * (q + 1.0) + 1.0, rows as f32 * (q + 1.0) + 1.0);
    let (x, y) = (x.round(), (cy - h * 0.5).round());
    g.canvas.rrect(Rect::new(x + g.ox, y, w, h), 2.0, mat, 0.55 * alpha);
    for (k, c) in colours.iter().enumerate() {
        let (i, j) = (k % cols, k / cols);
        g.canvas.fill_rect(Rect::new(x + 1.0 + i as f32 * (q + 1.0) + g.ox, y + 1.0 + j as f32 * (q + 1.0), q, q), Color(c[0], c[1], c[2]), alpha);
    }
    w
}

/// A palette's swatches' width at `q`.
fn swatches_w(n: usize, rows: usize, q: f32) -> f32 {
    n.div_ceil(rows.max(1)) as f32 * (q + 1.0) + 1.0
}

impl Camera {
    /// The engine for pixel mode's choices, made again only when they change.
    pub(super) fn pixel_engine(&mut self) -> Arc<Pixel> {
        match &self.pixel_made {
            Some(p) if p.settings == self.pixel => p.clone(),
            _ => {
                let p = Arc::new(Pixel::new(self.pixel));
                self.pixel_made = Some(p.clone());
                p
            }
        }
    }

    /// Pixel mode's choices changed: on the picture from the next frame,
    /// kept (unless a picker has them in hand), and said.
    fn set_pixel(&mut self, to: Settings, keep: bool, words: Option<String>) {
        self.pixel = to;
        self.tell_develop();
        if keep {
            self.keep_prefs();
        }
        if let Some(w) = words {
            self.say(w);
        }
    }

    /// Pixel mode's own keys, after the bar's: `Some` if it took the key.
    pub(super) fn pixel_key(&mut self, key: Key) -> Option<Nav> {
        let mut s = self.pixel;
        match key {
            Key::Up | Key::Down => {
                let by = if key == Key::Up { 1 } else { -1 };
                s.bright = (s.bright + by).clamp(-BRIGHT_STEPS, BRIGHT_STEPS);
                let end = if s.bright == self.pixel.bright { if by > 0 { " · brightest" } else { " · darkest" } } else { "" };
                self.set_pixel(s, true, Some(format!("brightness {}{end}", bright_words(s.bright))));
            }
            Key::Char('-' | '_' | '=' | '+') => {
                let by = if matches!(key, Key::Char('-' | '_')) { -1 } else { 1 };
                let at = CONTRASTS.iter().position(|c| *c == s.contrast).unwrap_or(1) as i32;
                s.contrast = CONTRASTS[(at + by).clamp(0, CONTRASTS.len() as i32 - 1) as usize];
                self.set_pixel(s, true, Some(format!("contrast: {}", s.contrast.name())));
            }
            Key::Char('s') => {
                let at = SIZES.iter().position(|z| *z == s.size).unwrap_or(0);
                s.size = SIZES[(at + 1) % SIZES.len()];
                self.set_pixel(s, true, Some(format!("size: {}", s.size.words())));
            }
            Key::Char('d') => {
                let at = DITHERS.iter().position(|d| *d == s.dither).unwrap_or(0);
                s.dither = DITHERS[(at + 1) % DITHERS.len()];
                self.set_pixel(s, true, Some(format!("dither: {}", s.dither.name())));
            }
            // The palettes, with their swatches, in the bar.
            Key::Char('k') if !self.quick => self.open_pixel_picker(Chip::Palette),
            Key::Char('a') => {
                return Some(Nav::Push(Box::new(match &self.last {
                    Some((photo, _)) => photos::Album::at(&photo.path),
                    None => photos::Album::new(),
                })));
            }
            _ => return None,
        }
        Some(Nav::Stay)
    }

    /// One of pixel mode's chips opened: its choices in the bar's place,
    /// the one in use in hand, and what it was kept for Esc.
    pub(super) fn open_pixel_picker(&mut self, chip: Chip) {
        let at = self.chosen(chip, crate::burst::DetailMode::Auto);
        self.bar.at.jump(at as f32);
        self.bar.picker = Some(Picker::Choice { chip, at });
        self.bar.focus = Some(chip);
        self.bar.last = chip;
        self.bar.hidden = false;
        self.pixel_from = Some(self.pixel);
        self.say(if chip == Chip::Tone { "← → contrast · ↑ ↓ brightness · enter: keep" } else { "← → choose · enter: keep · esc: as it was" });
    }

    /// The choice at `at` of a pixel chip, in `s`.
    fn pixel_choice(chip: Chip, at: usize, mut s: Settings) -> Settings {
        match chip {
            Chip::Size => s.size = SIZES[at.min(SIZES.len() - 1)],
            Chip::Palette => s.palette = PALETTES[at.min(PALETTES.len() - 1)],
            Chip::Dither => s.dither = DITHERS[at.min(DITHERS.len() - 1)],
            Chip::Tone => s.contrast = CONTRASTS[at.min(CONTRASTS.len() - 1)],
            _ => {}
        }
        s
    }

    /// A key while a pixel chip's picker is open: ← → choose (on the
    /// picture at once), ↑ ↓ the brightness in the tone's, Enter keeps,
    /// Esc (or ↑ elsewhere) goes back to what it opened on.
    pub(super) fn pixel_picker_key(&mut self, chip: Chip, at: usize, key: Key) -> Option<Nav> {
        let n = self.choices(chip).len();
        let back = |c: &mut Camera| {
            if let Some(s) = c.pixel_from.take() {
                if s != c.pixel {
                    c.set_pixel(s, false, None);
                }
            }
            c.bar.picker = None;
        };
        match key {
            Key::Left | Key::Right => {
                let to = if key == Key::Right { (at + 1).min(n - 1) } else { at.saturating_sub(1) };
                if to == at {
                    self.bar.at.nudge(if key == Key::Right { 3.0 } else { -3.0 });
                } else {
                    let s = Self::pixel_choice(chip, to, self.pixel);
                    self.set_pixel(s, false, None);
                }
                self.bar.picker = Some(Picker::Choice { chip, at: to });
            }
            Key::Up | Key::Down if chip == Chip::Tone => {
                let mut s = self.pixel;
                s.bright = (s.bright + if key == Key::Up { 1 } else { -1 }).clamp(-BRIGHT_STEPS, BRIGHT_STEPS);
                self.set_pixel(s, false, None);
            }
            Key::Enter => {
                self.bar.picker = None;
                self.pixel_from = None;
                self.keep_prefs();
                let words = format!("{} · {}", chip.name(), self.chip_words(chip, crate::burst::DetailMode::Auto));
                self.say(words);
            }
            Key::Back | Key::Up => back(self),
            Key::Down | Key::Tab => {}
            // Another key: the choices put away, as they were, and the key
            // is the camera's.
            _ => {
                back(self);
                return None;
            }
        }
        Some(Nav::Stay)
    }

    /// A pixel chip's face on the bar: the size's coarseness, the palette's
    /// colours, the dither's pattern, or the camera for the mode.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_pixel_face(&self, g: &mut Gfx, chip: Chip, left: f32, cy: f32, face: f32, ink: Color, alpha: f32) {
        let t = g.tokens;
        match chip {
            // A quarter circle drawn at the size's own coarseness: fewer,
            // larger steps for fewer pixels (its number would read as the
            // timer's seconds beside its key).
            Chip::Size => {
                let n = match self.pixel.size {
                    pixel::Size::Chunky => 4,
                    pixel::Size::Small => 6,
                    pixel::Size::Fine => 12,
                };
                let q = (face / n as f32).floor().max(1.0);
                let rows: Vec<String> = (0..n)
                    .map(|j| {
                        (0..n)
                            .map(|i| {
                                let d = ((n as f32 - i as f32 - 0.5).powi(2) + (n as f32 - j as f32 - 0.5).powi(2)).sqrt();
                                if (d - (n as f32 - 0.5)).abs() < 0.6 || (i == 0 && j == n - 1) {
                                    '#'
                                } else {
                                    '.'
                                }
                            })
                            .collect()
                    })
                    .collect();
                let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
                glyph(g, &rows, left + (face - n as f32 * q) * 0.5, cy, q, ink, alpha);
            }
            Chip::Palette => {
                let colours = self.pixel.palette.badge();
                let rows = if colours.len() > 2 { 2 } else { 1 };
                let q = ((face - 3.0) / 2.0).floor().max(2.0);
                let w = swatches_w(colours.len(), rows, q);
                swatches(g, &colours, rows, left + (face - w) * 0.5, cy, q, t.faint, alpha);
            }
            Chip::Dither => {
                let q = (face / 4.0).floor().max(2.0);
                glyph(g, &dither_glyph(self.pixel.dither), left + (face - 4.0 * q) * 0.5, cy, q, ink, alpha);
            }
            Chip::Mode => {
                let q = (face / 6.0).floor().max(1.0);
                glyph(g, &CAMERA_GLYPH, left + (face - 6.0 * q) * 0.5, cy, q, ink, alpha);
            }
            _ => {}
        }
    }

    /// A pixel chip's picker in the bar's place. Gives how much of the
    /// foot it takes.
    pub(super) fn draw_pixel_picker(&mut self, g: &mut Gfx, vf: Rect, shown: f32, chip: Chip, at: usize) -> f32 {
        match chip {
            Chip::Palette | Chip::Dither => self.draw_swatch_row(g, vf, shown, chip, at),
            _ => self.draw_word_row(g, vf, shown, chip, at),
        }
    }

    /// The size's or the contrast's choices: words in equal cells, the one
    /// in hand on ice (and on the picture).
    fn draw_word_row(&mut self, g: &mut Gfx, vf: Rect, shown: f32, chip: Chip, at: usize) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let words = self.choices(chip);
        let bh = (cap.size + 12.0).round();
        let r = Self::track(g, vf, bh, shown);
        let pad = 3.0;
        // Each cell as wide as its words and a margin, the room left shared
        // out, so the large text never cuts a word.
        let room = r.w - 2.0 * pad;
        let natural: Vec<f32> = words.iter().map(|w| g.measure(w, cap) + 12.0).collect();
        let extra = ((room - natural.iter().sum::<f32>()) / words.len().max(1) as f32).max(0.0);
        let cells: Vec<f32> = natural.iter().map(|n| n + extra).collect();
        let x_of = |i: usize| r.x + pad + cells[..i].iter().sum::<f32>();
        let slide = self.glide(g, at as f32);
        let i0 = (slide.floor().max(0.0) as usize).min(cells.len() - 1);
        let i1 = (i0 + 1).min(cells.len() - 1);
        let k = (slide - i0 as f32).clamp(0.0, 1.0);
        let (cx0, cw) = (x_of(i0) + (x_of(i1) - x_of(i0)) * k, cells[i0] + (cells[i1] - cells[i0]) * k);
        g.canvas.rrect(Rect::new(cx0 + g.ox, r.y + pad, cw, bh - 2.0 * pad), (bh - 2.0 * pad) * 0.5, t.ice, shown);
        for (i, w) in words.iter().enumerate() {
            let on = (1.0 - (slide - i as f32).abs()).clamp(0.0, 1.0);
            let w = g.text.fit(w, &cap, cells[i] - 6.0);
            let half = g.measure(&w, cap) * 0.5;
            g.text_alpha(x_of(i) + cells[i] * 0.5 - half, r.cy(), &w, cap, mix(t.ink, t.on_ice, on), shown);
        }
        let forms = if chip == Chip::Tone {
            let b = bright_words(self.pixel.bright);
            vec![format!("contrast · ↑ ↓ brightness {b} · enter"), format!("contrast · brightness {b}"), "contrast".to_owned()]
        } else {
            vec![format!("{} · enter: keep · esc: as it was", chip.name()), format!("{} · enter: keep", chip.name()), chip.name().to_owned()]
        };
        let label = forms.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 28.0).unwrap_or(&forms[2]).clone();
        let label_h = Self::bar_label(g, vf, vf.cx(), r.y - 4.0, &label, None, shown);
        (bh + 6.0 + label_h) * shown
    }

    /// The palettes or the dithers: each its name over its swatches (or a
    /// sample of its pattern), as many whole as fit about the one in hand,
    /// the row sliding to keep it in sight, as pro's row does.
    fn draw_swatch_row(&mut self, g: &mut Gfx, vf: Rect, shown: f32, chip: Chip, at: usize) -> f32 {
        let t = g.tokens;
        let cap = g.ty.caption;
        let bh = (2.0 * cap.size + 14.0).round();
        let r = Self::track(g, vf, bh, shown);
        let pad = 3.0;
        // The swatches' size: two rows of the line's height for sixteen.
        let q = ((cap.size - 1.0) / 2.0).floor().max(3.0);
        let names = self.choices(chip);
        let sample = (16usize, 4usize);
        let picture_w = |i: usize| match chip {
            Chip::Palette => {
                let n = PALETTES[i].colours().len();
                swatches_w(n, if n > 8 { 2 } else { 1 }, if n > 8 { q } else { (cap.size * 0.62).round() })
            }
            _ => sample.0 as f32 * 2.0,
        };
        let cells: Vec<f32> = names.iter().enumerate().map(|(i, n)| g.measure(n, cap).max(picture_w(i)) + 16.0).collect();
        let room = r.w - 2.0 * pad - 20.0;
        let (mut from, mut to) = (at, at);
        let width = |a: usize, b: usize| cells[a..=b].iter().sum::<f32>();
        loop {
            let right = to + 1 < cells.len() && width(from, to + 1) <= room;
            if right {
                to += 1;
            }
            let left = from > 0 && width(from - 1, to) <= room;
            if left {
                from -= 1;
            }
            if !right && !left {
                break;
            }
        }
        let before: f32 = cells[..from].iter().sum();
        let want = before - (r.w - 2.0 * pad - width(from, to)) * 0.5;
        if g.calm || self.frozen || self.bar.on.value() < 0.02 {
            self.bar.scroll.jump(want);
        } else if self.bar.scroll.target() != want {
            self.bar.scroll.set(want);
            g.started_moving();
        }
        let scroll = self.bar.scroll.value();
        let slide = self.glide(g, at as f32);
        let x_of = |i: usize| r.x + pad - scroll + cells[..i].iter().sum::<f32>();
        let i0 = (slide.floor().max(0.0) as usize).min(cells.len() - 1);
        let i1 = (i0 + 1).min(cells.len() - 1);
        let k = (slide - i0 as f32).clamp(0.0, 1.0);
        let (cx0, cw) = (x_of(i0) + (x_of(i1) - x_of(i0)) * k, cells[i0] + (cells[i1] - cells[i0]) * k);
        g.canvas.set_clip(r.inset(1.0).offset(g.ox, 0.0));
        let inner = bh - 2.0 * pad;
        // Concentric: the capsule's radius the bar's less its padding.
        g.canvas.rrect(Rect::new(cx0 + g.ox, r.y + pad, cw, inner), (bh * 0.5 - pad).min(inner * 0.5), t.ice, shown);
        let (name_cy, pic_cy) = (r.y + pad + 3.0 + cap.size * 0.5, r.bottom() - pad - 3.0 - cap.size * 0.5);
        let (inner_l, inner_r) = (r.x + pad + 8.0, r.right() - pad - 8.0);
        let (mut more_l, mut more_r) = (false, false);
        for (i, w) in cells.iter().enumerate() {
            let x = x_of(i);
            if x < inner_l - 9.0 {
                more_l = true;
                continue;
            }
            if x + w > inner_r + 9.0 {
                more_r = true;
                continue;
            }
            let on = (1.0 - (slide - i as f32).abs()).clamp(0.0, 1.0);
            let ink = mix(t.ink, t.on_ice, on);
            let nw = g.measure(&names[i], cap);
            g.text_alpha(x + (w - nw) * 0.5, name_cy, &names[i], cap, ink, shown);
            let pw = picture_w(i);
            let px = x + (w - pw) * 0.5;
            match chip {
                Chip::Palette => {
                    let colours = PALETTES[i].colours();
                    let rows = if colours.len() > 8 { 2 } else { 1 };
                    let side = if rows == 2 { q } else { (cap.size * 0.62).round() };
                    swatches(g, colours, rows, px, pic_cy, side, mix(t.faint, t.on_ice, on * 0.4), shown);
                }
                _ => {
                    // A ramp from dark to light in the dither's own pattern.
                    let dots = pixel::sample(DITHERS[i], sample.0, sample.1);
                    let top = (pic_cy - sample.1 as f32).round();
                    for (k, d) in dots.iter().enumerate() {
                        if *d == 1 {
                            let (dx, dy) = ((k % sample.0) as f32 * 2.0, (k / sample.0) as f32 * 2.0);
                            g.canvas.fill_rect(Rect::new(px.round() + dx + g.ox, top + dy, 2.0, 2.0), ink, shown);
                        }
                    }
                }
            }
        }
        for (more, x, chevron) in [(more_l, r.x + pad + 5.0, "‹"), (more_r, r.right() - pad - 5.0, "›")] {
            if more {
                let half = g.measure(chevron, cap) * 0.5;
                g.text_alpha(x - half, r.cy(), chevron, cap, t.dim, shown);
            }
        }
        g.canvas.clear_clip();
        let what = match chip {
            Chip::Palette => format!("{} · {}", PALETTES[at.min(PALETTES.len() - 1)].name(), PALETTES[at.min(PALETTES.len() - 1)].words()),
            _ => format!("dither · {}", DITHERS[at.min(DITHERS.len() - 1)].name()),
        };
        let forms = [format!("{what} · enter: keep · esc"), format!("{what} · enter: keep"), what.clone()];
        let label = forms.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 28.0).unwrap_or(&forms[2]).clone();
        let label_h = Self::bar_label(g, vf, vf.cx(), r.y - 4.0, &label, None, shown);
        (bh + 6.0 + label_h) * shown
    }

    /// Over the pixel picture: the tone, when it is not as it comes.
    pub(super) fn draw_pixel_overlays(&mut self, g: &mut Gfx, vf: Rect, alpha: f32) {
        if alpha <= 0.01 {
            return;
        }
        let t = g.tokens;
        let s = self.pixel;
        let mut words = Vec::new();
        if s.bright != 0 {
            words.push(bright_words(s.bright));
        }
        if s.contrast != Contrast::Normal {
            words.push(s.contrast.name().to_owned());
        }
        if !words.is_empty() {
            Self::chip(g, vf.x + 6.0, vf.y + 6.0, Some("sunny"), &words.join(" · "), t.ice, alpha);
        }
    }

    /// The `?` sheet's keys for pixel mode.
    pub(super) fn pixel_keys(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        vec![
            keys("pixel", &[("space · enter", "Take the picture"), ("↑ ↓", "Brightness"), ("- =", "Contrast"), ("a", "The album")]),
            keys("its choices", &[("s", "Size: 56, 75, 112"), ("k", "Palette"), ("d", "Dither"), ("tab", "All of them, in the bar")]),
        ]
    }

    /// The shutter in pixel mode: the picture in sight kept, at once.
    pub(super) fn shoot_pixel(&mut self, calm: bool) {
        let Some(frame) = self.pixel_frame.clone() else {
            self.say("the camera is not ready yet");
            return;
        };
        let size = SIZES.iter().copied().find(|s| s.dims() == (frame.w, frame.h)).unwrap_or(self.pixel.size);
        let mut path = photos::new_pixel_path(chrono::Local::now().naive_local());
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut n = 2;
        while self.pending_paths.contains(&path) {
            path = path.with_file_name(format!("{stem}_{n}.png"));
            n += 1;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        // At once, as photo mode's instant shot: its thumbnail, flying in.
        let pic = frame.picture();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.last = Some((Photo { path: path.clone(), name, taken: chrono::Local::now().naive_local(), bytes: 0 }, pic.cover_nearest(photos::THUMB_W, photos::THUMB_H)));
        self.instant = Some(path.clone());
        self.fly_crisp = true;
        if calm {
            self.flying = None;
            self.fly.jump(1.0);
        } else {
            self.flying = Some(pic);
            self.fly.jump(0.0);
            self.fly.set(1.0);
            self.pixel_flash.jump(1.0);
            self.pixel_flash.set(0.0);
        }
        self.press.jump(1.0);
        self.press.set(0.0);
        self.saving = Some(Instant::now());
        self.captured = true;
        self.pending += 1;
        self.pending_paths.push(path.clone());
        // Written behind: the true size kept as its original, then the copy
        // to share (the album's), each whole or not at all.
        let (live, look, scale) = (self.live.clone(), self.pixel.look_words(), size.share_scale());
        std::thread::Builder::new()
            .name("pixel-write".into())
            .spawn(move || {
                let write = || -> Result<(), String> {
                    let original = crate::filters::original_of(&path);
                    if let Some(dir) = original.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| format!("the picture cannot be kept: {e}"))?;
                    }
                    crate::sys::write_atomic(&original, &frame.png(1, &look)?).map_err(|e| format!("the picture cannot be kept: {e}"))?;
                    crate::sys::write_atomic(&path, &frame.png(scale, &look)?).map_err(|e| format!("the picture cannot be kept: {e}"))
                };
                let event = match write() {
                    Ok(()) => Event::Saved { path, size: ((frame.w * scale) as u32, (frame.h * scale) as u32), detail: false, raw_skipped: false, stacked: None },
                    Err(why) => Event::Failed(why),
                };
                live.lock().map(|mut l| l.events.push(event)).ok();
                crate::platform::wake();
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Settings as ShellSettings;

    fn press(camera: &mut Camera, key: Key) -> Nav {
        let mut robot = Robot::none();
        let mut settings = ShellSettings::default();
        let mut say = None;
        camera.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true })
    }

    fn run(camera: &mut Camera, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        let robot = Robot::none();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(16));
            camera.tick(1.0 / 60.0);
            camera.ambient_tick(1.0 / 60.0, &robot);
        }
    }

    /// The Camera in pixel mode with its first pixel pictures in.
    fn in_pixel_mode() -> Camera {
        crate::shell::tests::sandbox();
        Prefs::default().save();
        let mut camera = Camera::new();
        camera.set_mode(Mode::Pixel);
        let started = Instant::now();
        while camera.pixel_frame.is_none() && started.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 30);
        }
        assert!(camera.pixel_frame.is_some(), "pixel pictures arrive");
        camera
    }

    #[test]
    fn pixel_mode_sits_between_pro_and_doc_and_shows_its_picture_live() {
        let _port = crate::smartcam::sim::exclusive(false);
        crate::shell::tests::sandbox();
        Prefs::default().save();
        let mut camera = Camera::new();
        press(&mut camera, Key::Right);
        assert_eq!(camera.mode, Mode::Pro);
        press(&mut camera, Key::Right);
        assert_eq!(camera.mode, Mode::Pixel);
        press(&mut camera, Key::Right);
        assert_eq!(camera.mode, Mode::Doc);
        press(&mut camera, Key::Left);
        assert_eq!(camera.mode, Mode::Pixel);
        // Its frames are pixel pictures at the size chosen, drawn whole.
        let started = Instant::now();
        while camera.pixel_frame.is_none() && started.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 30);
        }
        let frame = camera.pixel_frame.clone().expect("a pixel picture");
        assert_eq!((frame.w, frame.h), camera.pixel.size.dims());
        let shown = camera.preview.clone().unwrap();
        assert_eq!((shown.w, shown.h), (frame.w, frame.h));
        assert!(fits_whole(Camera::viewfinder(), &shown));
        // Only its palette's colours.
        let colours = camera.pixel.palette.colours();
        assert!(shown.rgb.chunks_exact(3).all(|p| colours.iter().any(|c| c == p)));
        // At rest it asks for no frames of its own.
        camera.note = None;
        assert_eq!(camera.ambient(), None);
        assert!(!camera.busy());
        // The chips: the mode, its four choices and the timer.
        assert_eq!(camera.chips(), [Chip::Mode, Chip::Size, Chip::Palette, Chip::Dither, Chip::Tone, Chip::Timer]);
        // Leaving it, the frames are whole photos' again.
        press(&mut camera, Key::Left);
        run(&mut camera, 200);
        assert!(camera.preview.as_ref().is_some_and(|p| p.w == PREVIEW_W), "the full preview again");
        assert!(camera.pixel_frame.is_none());
    }

    #[test]
    fn its_keys_and_pickers_change_the_picture_and_are_remembered() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = in_pixel_mode();
        assert_eq!(camera.pixel, Settings::default());
        // ↑ ↓ the brightness, kept in its range.
        for _ in 0..5 {
            press(&mut camera, Key::Up);
        }
        assert_eq!(camera.pixel.bright, BRIGHT_STEPS);
        press(&mut camera, Key::Down);
        assert_eq!(camera.pixel.bright, BRIGHT_STEPS - 1);
        assert_eq!(camera.mode, Mode::Pixel, "↑ ↓ never leave the mode or switch lens");
        // s steps the size, d the dither, = the contrast.
        press(&mut camera, Key::Char('s'));
        assert_eq!(camera.pixel.size, pixel::Size::Fine);
        press(&mut camera, Key::Char('d'));
        assert_eq!(camera.pixel.dither, pixel::Dither::Floyd);
        press(&mut camera, Key::Char('='));
        assert_eq!(camera.pixel.contrast, Contrast::Punchy);
        // k: the palettes in the bar, seen on the picture as each is in hand.
        press(&mut camera, Key::Char('k'));
        assert_eq!(camera.bar.picker, Some(Picker::Choice { chip: Chip::Palette, at: 2 }));
        press(&mut camera, Key::Right);
        assert_eq!(camera.pixel.palette, pixel::Palette::Tide, "on the picture at once");
        // Esc: back as it was.
        press(&mut camera, Key::Back);
        assert_eq!((camera.pixel.palette, camera.bar.picker), (pixel::Palette::Ember, None));
        // Enter keeps it.
        press(&mut camera, Key::Char('k'));
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Right);
        press(&mut camera, Key::Enter);
        assert_eq!((camera.pixel.palette, camera.bar.picker), (pixel::Palette::Poster, None));
        // The tone's picker: ← → contrast, ↑ ↓ brightness, Enter keeps.
        camera.bar.focus = Some(Chip::Tone);
        press(&mut camera, Key::Enter);
        assert!(matches!(camera.bar.picker, Some(Picker::Choice { chip: Chip::Tone, .. })));
        press(&mut camera, Key::Left);
        press(&mut camera, Key::Down);
        press(&mut camera, Key::Enter);
        assert_eq!((camera.pixel.contrast, camera.pixel.bright), (Contrast::Normal, BRIGHT_STEPS - 2));
        // The picture follows: its size and palette now.
        let started = Instant::now();
        while camera.pixel_frame.as_ref().is_none_or(|f| (f.w, f.h) != pixel::Size::Fine.dims() || f.palette != pixel::Palette::Poster) && started.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 30);
        }
        let f = camera.pixel_frame.clone().unwrap();
        assert_eq!(((f.w, f.h), f.palette), (pixel::Size::Fine.dims(), pixel::Palette::Poster));
        // All of it kept for the next visit.
        let kept = Prefs::load().pixel;
        assert_eq!(kept, Settings { size: pixel::Size::Fine, palette: pixel::Palette::Poster, dither: pixel::Dither::Floyd, bright: BRIGHT_STEPS - 2, contrast: Contrast::Normal });
        drop(camera);
        let again = Camera::new();
        assert_eq!(again.pixel, kept);
        Prefs::default().save();
    }

    #[test]
    fn the_shutter_keeps_the_picture_in_sight_as_two_pngs_in_the_album() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = in_pixel_mode();
        let frame = camera.pixel_frame.clone().unwrap();
        camera.pixel_frame = Some(frame.clone());
        assert!(matches!(press(&mut camera, Key::Enter), Nav::Stay));
        let path = camera.instant.clone().expect("taken at once");
        assert!(photos::is_pixel(&path.file_name().unwrap().to_string_lossy()));
        assert!(camera.flying.is_some() && camera.fly_crisp, "it flies in, whole pixels");
        assert!(camera.pixel_flash.value() > 0.9, "the dithered flash");
        let started = Instant::now();
        while camera.pending > 0 && started.elapsed() < Duration::from_secs(10) {
            run(&mut camera, 20);
        }
        assert_eq!(camera.pending, 0, "saved");
        // The true size, kept as its original; the copy to share, the album's.
        let (w, h) = (frame.w, frame.h);
        let scale = camera.pixel.size.share_scale();
        let original = crate::filters::original_of(&path);
        assert_eq!(photos::dimensions(&original), Some((w, h)));
        assert_eq!(photos::dimensions(&path), Some((w * scale, h * scale)));
        // Each is the picture that was in sight, pixel for pixel.
        let small = photos::decode(&original, w, h).unwrap();
        assert_eq!(small, frame.picture());
        let big = photos::decode(&path, w, h).unwrap();
        for (y, x) in [(0, 0), (h * scale - 1, w * scale - 1), (h * scale / 2, w * scale / 3)] {
            assert_eq!(&big.rgb[(y * w * scale + x) * 3..][..3], &small.rgb[((y / scale) * w + x / scale) * 3..][..3]);
        }
        assert_eq!(crate::filters::look_of(&path).as_deref(), Some(camera.pixel.look_words().as_str()));
        // In the album, newest first, and its thumbnail made of whole pixels.
        let photo = photos::list().into_iter().find(|p| p.path == path).expect("in the album");
        let thumb = photos::thumbnail(&photo).unwrap();
        let colours = camera.pixel.palette.colours();
        assert!(thumb.rgb.chunks_exact(3).all(|p| colours.iter().any(|c| c == p)), "sampled, never averaged");
        // The timer works here too.
        camera.timer = 2;
        press(&mut camera, Key::Enter);
        assert!(camera.countdown.is_some() && camera.pending == 0);
        camera.countdown = None;
        photos::delete(&photo).unwrap();
        assert!(!original.exists());
    }

    #[test]
    fn its_keys_fit_their_sheet_and_its_bar_and_pickers_fit_at_every_text_size() {
        let _port = crate::smartcam::sim::exclusive(false);
        crate::shell::tests::sandbox();
        let mut camera = Camera::new();
        camera.set_mode(Mode::Pixel);
        let mut g = Gfx::new(bezel::Tokens::dark());
        let vf = Camera::viewfinder();
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            for picker in [None, Some(Chip::Size), Some(Chip::Palette), Some(Chip::Dither), Some(Chip::Tone)] {
                camera.bar.picker = picker.map(|chip| Picker::Choice { chip, at: 1 });
                camera.bar.focus = Some(picker.unwrap_or(Chip::Palette));
                let taken = camera.draw_bar(&mut g, vf);
                assert!(taken > 10.0 && vf.bottom() - taken > vf.cy() - 12.0, "{size:?} {picker:?}: {taken}");
            }
        }
        camera.bar.picker = None;
        crate::keysheet::fits(&mut g, &camera.keys_now()).unwrap();
        camera.bar.picker = Some(Picker::Choice { chip: Chip::Tone, at: 1 });
        crate::keysheet::fits(&mut g, &camera.keys_now()).unwrap();
        // None of its keys are the arrows' letters, which never reach it.
        for group in camera.pixel_keys() {
            for (k, _) in group.keys {
                assert!(!k.split(' ').any(|w| matches!(w, "f" | "x" | "z" | "c")), "{k}");
            }
        }
    }

    #[test]
    fn the_flash_falls_through_the_bayer_order_and_the_disc_is_whole_blocks() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let vf = Camera::viewfinder();
        let pic = Picture::new(56, 42);
        // Ember's cream over black, at the flash's opacity: bright red.
        let lit = |g: &mut Gfx| g.canvas.px.iter().filter(|p| (**p >> 16) & 0xff > 200).count();
        draw_view(&mut g, vf, &pic, 1.0, 1.0, pixel::Palette::Ember);
        let all = lit(&mut g);
        assert!(all >= 224 * 168 - 10, "all of it at the flash: {all}");
        let mut g = Gfx::new(bezel::Tokens::dark());
        draw_view(&mut g, vf, &pic, 1.0, 0.5, pixel::Palette::Ember);
        let half = lit(&mut g);
        assert!((half as f32 / all as f32 - 0.5).abs() < 0.02, "half as it falls: {half} of {all}");
        // The shutter's disc: three-pixel blocks, round.
        let mut g = Gfx::new(bezel::Tokens::dark());
        g.canvas.px.fill(0);
        draw_pixel_disc(&mut g, 100.0, 80.0, 15.5, Color(255, 255, 255), 1.0);
        let on: Vec<(i32, i32)> = (0..g.canvas.h).flat_map(|y| (0..g.canvas.w).map(move |x| (x, y))).filter(|(x, y)| g.canvas.px[(y * g.canvas.w + x) as usize] == 0x00ff_ffff).collect();
        let (top, bottom) = (on.iter().map(|p| p.1).min().unwrap(), on.iter().map(|p| p.1).max().unwrap());
        assert_eq!((bottom - top + 1) % 3, 0, "whole blocks tall");
        assert!((bottom - top + 1 - 30).abs() <= 3, "{}", bottom - top + 1);
    }
}

// ------------------------------------------------------------- the shots ---

/// The Camera as the shots set it: frozen, as its defaults, with `pic` in
/// sight.
#[cfg(not(target_arch = "aarch64"))]
pub(super) fn shot_camera() -> Camera {
    let mut camera = Camera::new();
    camera.frozen = true;
    camera.flash.jump(0.0);
    (camera.look, camera.hue, camera.raw, camera.zebras, camera.pro, camera.timer) = (Look::Original, 32.0, false, false, Pro::default(), 0);
    camera.kit = Kit::default();
    camera.last = None;
    camera.last_rx = None;
    camera.note = None;
    camera.metered = Some(Metered { exposure_us: 16_667, analogue: 2.0, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom: 0.0 });
    camera
}

/// Renders a set of scenes dark and light at the three text sizes into
/// `dir`, each alone (`name-tone-size.png`) and on a sheet of its six.
#[cfg(not(target_arch = "aarch64"))]
pub(super) fn render(dir: &std::path::Path, scenes: &[(String, Box<dyn Fn(&mut crate::shell::Shell)>)]) {
    use crate::gfx::TextSize;
    use crate::shell::Shell;
    std::fs::create_dir_all(dir).ok();
    for (name, setup) in scenes {
        let mut shots = Vec::new();
        for light in [false, true] {
            for (size, tag) in [(TextSize::Compact, "compact"), (TextSize::Standard, "standard"), (TextSize::Large, "large")] {
                let mut shell = Shell::new(crate::robot::Robot::demo());
                (shell.settings.text, shell.settings.light) = (size, light);
                shell.apply_settings();
                setup(&mut shell);
                for _ in 0..90 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                let tone = if light { "light" } else { "dark" };
                crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}-{tone}-{tag}.png")), 3);
                shots.push(shell.gfx.canvas.clone());
            }
        }
        let (w, h) = (W as i32, H as i32);
        let gap = 6;
        let mut sheet = bezel::Canvas::new(3 * w + 2 * gap, 2 * h + gap);
        sheet.px.fill(0x0080_8080);
        for (i, shot) in shots.iter().enumerate() {
            let (ox, oy) = ((i % 3) as i32 * (w + gap), (i / 3) as i32 * (h + gap));
            for y in 0..h {
                let from = (y * shot.w) as usize;
                let to = ((oy + y) * sheet.w + ox) as usize;
                sheet.px[to..to + w as usize].copy_from_slice(&shot.px[from..from + w as usize]);
            }
        }
        crate::platform::sim::save_png(&sheet, &dir.join(format!("sheet-{name}.png")), 2);
    }
}

/// Pixel mode's pictures (`--camera-pixel-shots DIR`): the viewfinder in
/// each palette and each dither and size, the tone changed, each picker,
/// the shutter's flash, the keys' sheet, and a picture in Photos (the
/// album and the viewer, fitted and zoomed), dark and light at each text
/// size; and every palette and dither on the sample scene, on one sheet.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::shell::Shell;
    crate::smartcam::sim::choose(false);
    let sample = scene(PREVIEW_W, PREVIEW_H, 3.1, false);
    let yuv = rgb_to_yuv420(&sample);
    let with = |s: Settings| {
        let f = Pixel::new(s).develop(&yuv, PREVIEW_W, PREVIEW_H);
        (f.picture(), f)
    };
    type Scene = (String, Box<dyn Fn(&mut Shell)>);
    let camera_scene = |name: String, s: Settings, more: Box<dyn Fn(&mut Camera)>, keys: Vec<Key>| -> Scene {
        let (pic, frame) = with(s);
        (
            name,
            Box::new(move |shell: &mut Shell| {
                let mut c = shot_camera();
                c.set_mode(Mode::Pixel);
                c.pixel = s;
                c.preview = Some(pic.clone());
                c.pixel_frame = Some(frame.clone());
                c.note = None;
                more(&mut c);
                c.mode_at.jump(c.mode_at.target());
                shell.show(Box::new(c));
                for k in &keys {
                    for _ in 0..30 {
                        shell.tick(1.0 / 60.0);
                        shell.draw();
                    }
                    shell.key(*k);
                }
            }),
        )
    };
    let base = Settings::default();
    let mut scenes: Vec<Scene> = Vec::new();
    for p in PALETTES {
        scenes.push(camera_scene(format!("01-palette-{}", p.name()), Settings { palette: p, ..base }, Box::new(|_| {}), Vec::new()));
    }
    for d in DITHERS {
        scenes.push(camera_scene(format!("02-dither-{}", d.name()), Settings { dither: d, palette: pixel::Palette::Graphite, ..base }, Box::new(|_| {}), Vec::new()));
    }
    for z in SIZES {
        scenes.push(camera_scene(format!("03-size-{}", z.id()), Settings { size: z, palette: pixel::Palette::Crayon, ..base }, Box::new(|_| {}), Vec::new()));
    }
    scenes.push(camera_scene("04-tone-bright-hard".into(), Settings { bright: 2, contrast: Contrast::Hard, palette: pixel::Palette::Ink, dither: pixel::Dither::Atkinson, ..base }, Box::new(|_| {}), Vec::new()));
    scenes.push(camera_scene("05-bar-in-hand".into(), base, Box::new(|c| c.bar.focus = Some(Chip::Palette)), Vec::new()));
    for (n, chip) in [("06-picker-size", Chip::Size), ("07-picker-palette", Chip::Palette), ("08-picker-dither", Chip::Dither), ("09-picker-tone", Chip::Tone)] {
        scenes.push(camera_scene(n.into(), base, Box::new(move |c| c.open_pixel_picker(chip)), Vec::new()));
    }
    scenes.push(camera_scene("10-picker-palette-crayon".into(), Settings { palette: pixel::Palette::Crayon, ..base }, Box::new(|c| c.open_pixel_picker(Chip::Palette)), Vec::new()));
    scenes.push(camera_scene(
        "11-shutter-flash".into(),
        base,
        Box::new(|c| {
            c.pixel_flash.jump(0.45);
            c.press.jump(0.6);
            c.bar.hidden = true;
        }),
        Vec::new(),
    ));
    scenes.push(camera_scene("12-keys".into(), base, Box::new(|_| {}), vec![Key::Char('?')]));
    scenes.push(camera_scene("13-keys-more".into(), base, Box::new(|_| {}), vec![Key::Char('?'), Key::PageDown]));
    // In Photos: a picture taken, in the album (among photos) and the viewer.
    photos::seed_demo();
    let (pic, frame) = with(Settings { palette: pixel::Palette::Ember, ..base });
    let taken = photos::dir().join("PIX_20261123_101500.png");
    std::fs::create_dir_all(photos::dir().join(".originals")).ok();
    let look = base.look_words();
    crate::sys::write_atomic(&crate::filters::original_of(&taken), &frame.png(1, &look).unwrap_or_default()).ok();
    crate::sys::write_atomic(&taken, &frame.png(base.size.share_scale(), &look).unwrap_or_default()).ok();
    photos::changed();
    drop(pic);
    // Its thumbnail made now, so the album has it at once.
    if let Some(photo) = photos::list().into_iter().find(|p| p.path == taken) {
        photos::thumbnail(&photo);
    }
    let viewer = |zoom: usize, details: bool| -> Box<dyn Fn(&mut Shell)> {
        let taken = taken.clone();
        Box::new(move |shell: &mut Shell| {
            shell.show(Box::new(photos::Viewer::single(taken.clone())));
            // Its true size decoded (on a thread of its own) before the shot.
            for _ in 0..40 {
                shell.tick(1.0 / 60.0);
                shell.draw();
                std::thread::sleep(Duration::from_millis(5));
            }
            for _ in 0..zoom {
                shell.key(Key::Enter);
            }
            if details {
                shell.key(Key::Char('i'));
            }
        })
    };
    scenes.push(("14-photos-viewer".into(), viewer(0, false)));
    scenes.push(("15-photos-viewer-zoomed".into(), viewer(1, false)));
    scenes.push(("16-photos-viewer-details".into(), viewer(0, true)));
    {
        let taken = taken.clone();
        scenes.push((
            "17-photos-album".into(),
            Box::new(move |shell: &mut Shell| {
                shell.show(Box::new(photos::Album::at(&taken)));
                for _ in 0..40 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                    std::thread::sleep(Duration::from_millis(5));
                }
            }),
        ));
    }
    render(dir, &scenes);
    // Every palette under each dither on the sample scene, at the middle size,
    // the panel's own pixels three to one.
    let (cw, ch) = (pixel::Size::Small.dims().0 * 3, pixel::Size::Small.dims().1 * 3);
    let label = 20;
    let mut sheet = bezel::Canvas::new((PALETTES.len() * cw) as i32, (DITHERS.len() * (ch + label)) as i32);
    sheet.px.fill(0x0010_1214);
    let mut shell = Shell::new(crate::robot::Robot::demo());
    shell.apply_settings();
    for (j, d) in DITHERS.iter().enumerate() {
        for (i, p) in PALETTES.iter().enumerate() {
            let (pic, _) = with(Settings { palette: *p, dither: *d, ..base });
            let (ox, oy) = (i * cw, j * (ch + label));
            for y in 0..ch {
                for x in 0..cw {
                    let c = &pic.rgb[((y / 3) * pic.w + x / 3) * 3..][..3];
                    sheet.px[(oy + y) * sheet.w as usize + ox + x] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
                }
            }
            let g = &mut shell.gfx;
            std::mem::swap(&mut g.canvas, &mut sheet);
            let ty = g.ty;
            g.text_at(ox as f32 + 6.0, (oy + ch) as f32 + label as f32 * 0.5, Align::Left, &format!("{} · {}", p.name(), d.name()), ty.caption, Color(236, 238, 242));
            std::mem::swap(&mut g.canvas, &mut sheet);
        }
    }
    crate::platform::sim::save_png(&sheet, &dir.join("palettes-and-dithers.png"), 2);
    std::fs::remove_file(crate::filters::original_of(&taken)).ok();
    std::fs::remove_file(&taken).ok();
    println!("pixel mode shots in {}", dir.display());
}

/// The film looks' pictures (`--camera-film-shots DIR`): each film look
/// in the viewfinder (its grain at light), the strip on the film group
/// with its grain's choice at each strength, a tungsten night with its
/// glow, and the photo editor's looks; dark and light at each text size;
/// and every film look on the sample scene, off and strong, on one sheet.
#[cfg(not(target_arch = "aarch64"))]
pub fn film_shots(dir: &std::path::Path) {
    use crate::filters::FILM;
    use crate::shell::Shell;
    crate::smartcam::sim::choose(false);
    let sample = scene(PREVIEW_W, PREVIEW_H, 3.1, false);
    let yuv = rgb_to_yuv420(&sample);
    // A dusk with lamps for the halation: the scene dimmed, bright lights in it.
    let mut night = sample.clone();
    sim_expose(&mut night, 0.35, SIM_CT);
    for (lx, ly, r) in [(60.0f32, 70.0f32, 7.0f32), (250.0, 60.0, 5.0), (170.0, 110.0, 4.0)] {
        for y in 0..PREVIEW_H {
            for x in 0..PREVIEW_W {
                let d = ((x as f32 - lx).powi(2) + (y as f32 - ly).powi(2)).sqrt();
                if d < r {
                    night.rgb[(y * PREVIEW_W + x) * 3..][..3].copy_from_slice(&[255, 244, 214]);
                }
            }
        }
    }
    let night_yuv = rgb_to_yuv420(&night);
    let developed = |yuv: &[u8], look: Look, s: Strength| {
        let mut out = Picture::default();
        look.compiled(32.0).develop(yuv, PREVIEW_W, PREVIEW_H, &mut out, None);
        film::apply(&mut out, look.texture().at(s), 0);
        out
    };
    let small = shrink_yuv420(&yuv, PREVIEW_W, PREVIEW_H, THUMB.0, THUMB.1);
    let thumbs: Vec<Picture> = LOOKS
        .iter()
        .map(|l| {
            let mut p = Picture::default();
            l.compiled(32.0).develop(&small, THUMB.0, THUMB.1, &mut p, None);
            p
        })
        .collect();
    type Scene = (String, Box<dyn Fn(&mut Shell)>);
    let look_scene = |name: String, yuv: &[u8], look: Look, s: Strength, strip: bool| -> Scene {
        let pic = developed(yuv, look, s);
        let thumbs = thumbs.clone();
        (
            name,
            Box::new(move |shell: &mut Shell| {
                let mut c = shot_camera();
                c.look = look;
                c.texture = s;
                c.preview = Some(pic.clone());
                c.thumbs = thumbs.clone();
                if strip {
                    c.strip_at.jump(LOOKS.iter().position(|l| *l == look).unwrap_or(0) as f32);
                    c.set_strip(true);
                    c.strip_fade.jump(1.0);
                    c.note = None;
                }
                c.mode_at.jump(c.mode_at.target());
                shell.show(Box::new(c));
            }),
        )
    };
    let mut scenes: Vec<Scene> = Vec::new();
    for (i, look) in FILM.iter().enumerate() {
        scenes.push(look_scene(format!("0{}-{}", i + 1, look.id()), &yuv, *look, Strength::Light, false));
    }
    for s in film::STRENGTHS {
        scenes.push(look_scene(format!("10-strip-grain-{}", s.name()), &yuv, Look::Pushed, s, true));
    }
    scenes.push(look_scene("11-strip-sunlit".into(), &yuv, Look::Sunlit, Strength::Light, true));
    scenes.push(look_scene("12-strip-tungsten-glow".into(), &night_yuv, Look::Tungsten, Strength::Strong, true));
    scenes.push(look_scene("13-tungsten-night-strong".into(), &night_yuv, Look::Tungsten, Strength::Strong, false));
    scenes.push(look_scene("14-cine-night-light".into(), &night_yuv, Look::Cine, Strength::Light, false));
    {
        let pic = developed(&yuv, Look::Portrait, Strength::Light);
        let thumbs = thumbs.clone();
        scenes.push((
            "15-bar-look-chip".into(),
            Box::new(move |shell: &mut Shell| {
                let mut c = shot_camera();
                c.look = Look::Portrait;
                c.texture = Strength::Light;
                c.preview = Some(pic.clone());
                c.thumbs = thumbs.clone();
                c.bar.focus = Some(Chip::Look);
                shell.show(Box::new(c));
            }),
        ));
    }
    {
        let pic = developed(&yuv, Look::Faded, Strength::Light);
        let thumbs = thumbs.clone();
        scenes.push((
            "16-keys-strip-film".into(),
            Box::new(move |shell: &mut Shell| {
                let mut c = shot_camera();
                c.look = Look::Faded;
                c.preview = Some(pic.clone());
                c.thumbs = thumbs.clone();
                c.strip_at.jump(LOOKS.iter().position(|l| *l == Look::Faded).unwrap_or(0) as f32);
                c.set_strip(true);
                c.strip_fade.jump(1.0);
                c.note = None;
                shell.show(Box::new(c));
                for _ in 0..30 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                }
                shell.key(Key::Char('?'));
            }),
        ));
    }
    // The photo editor's looks, on a photo, a film look in hand.
    let photo = photos::dir().join("IMG_20261123_101600.jpg");
    std::fs::create_dir_all(photos::dir()).ok();
    save_jpeg(&photo, &scene(1280, 960, 3.1, false)).ok();
    {
        let photo = photo.clone();
        scenes.push((
            "17-editor-film-looks".into(),
            Box::new(move |shell: &mut Shell| {
                let mut e = crate::editor::Editor::open(photo.clone());
                e.show_looks_at(Look::Cine);
                shell.show(Box::new(e));
                for _ in 0..60 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                    std::thread::sleep(Duration::from_millis(10));
                }
            }),
        ));
    }
    {
        let photo = photo.clone();
        scenes.push((
            "18-editor-soft-negative".into(),
            Box::new(move |shell: &mut Shell| {
                let mut e = crate::editor::Editor::open(photo.clone());
                e.show_looks_at(Look::Portrait);
                shell.show(Box::new(e));
                for _ in 0..60 {
                    shell.tick(1.0 / 60.0);
                    shell.draw();
                    std::thread::sleep(Duration::from_millis(10));
                }
            }),
        ));
    }
    render(dir, &scenes);
    // Every film look on the sample scene and at dusk, its grain off and
    // strong (at twice the preview's own size: grain shows as it would
    // shrunk, a pixel of the panel's three).
    let (cw, ch) = (PREVIEW_W, PREVIEW_H);
    let label = 22;
    let cols = [("day, grain off", &yuv, Strength::Off), ("day, strong", &yuv, Strength::Strong), ("dusk, strong", &night_yuv, Strength::Strong)];
    let mut sheet = bezel::Canvas::new((cols.len() * cw) as i32, (FILM.len() * (ch + label)) as i32);
    sheet.px.fill(0x0010_1214);
    let mut shell = Shell::new(crate::robot::Robot::demo());
    shell.apply_settings();
    for (j, look) in FILM.iter().enumerate() {
        for (i, (what, yuv, s)) in cols.iter().enumerate() {
            let pic = developed(yuv, *look, *s);
            let (ox, oy) = (i * cw, j * (ch + label));
            for y in 0..ch {
                for x in 0..cw {
                    let c = &pic.rgb[(y * cw + x) * 3..][..3];
                    sheet.px[(oy + y) * sheet.w as usize + ox + x] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
                }
            }
            let g = &mut shell.gfx;
            std::mem::swap(&mut g.canvas, &mut sheet);
            let ty = g.ty;
            g.text_at(ox as f32 + 8.0, (oy + ch) as f32 + label as f32 * 0.5, Align::Left, &format!("{} · {what}", look.name().to_lowercase()), ty.caption, Color(236, 238, 242));
            std::mem::swap(&mut g.canvas, &mut sheet);
        }
    }
    crate::platform::sim::save_png(&sheet, &dir.join("film-looks-on-the-sample-scene.png"), 2);
    // A full-size photo's grain at 1:1 beside the preview's, both as the
    // panel would show them: the texture is the picture's own size.
    let mut full = scene(1640, 1232, 3.1, false);
    let compiled = Look::Pushed.compiled(0.0);
    for p in full.rgb.chunks_exact_mut(3) {
        let o = compiled.pixel(p[0], p[1], p[2]);
        p.copy_from_slice(&o);
    }
    film::apply(&mut full, Look::Pushed.texture().at(Strength::Strong), film::seed_of(&photo));
    let (cw, ch) = (480, 320);
    let mut crop = bezel::Canvas::new(cw as i32, ch as i32);
    for y in 0..ch {
        for x in 0..cw {
            let c = &full.rgb[((y + 600) * full.w + x + 600) * 3..][..3];
            crop.px[y * cw + x] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
        }
    }
    crate::platform::sim::save_png(&crop, &dir.join("pushed-mono-strong-grain-1to1.png"), 2);
    std::fs::remove_file(&photo).ok();
    std::fs::remove_file(crate::editor::edits::sidecar(&photo)).ok();
    println!("film shots in {}", dir.display());
}
