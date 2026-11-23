//! Markup for documenting a robot: arrows, boxes, circles and short labels
//! in the signal colour or ice, over the edited photo. Each mark is a set
//! of strokes (its paths) sized to the picture it lands on, so the preview
//! and the full-size export draw the same thing: on the panel through the
//! canvas, on the photo as round-capped strokes of their own coverage, and
//! a label as a capsule of its colour with its words on it.

use bezel::{Color, TextStyle};

use super::edits::{Ink, Mark, Shape};
use crate::photos::Picture;

/// A mark's colour and the colour of words on it: Bezel's own signal and
/// ice (the dark palette's, the same on every handheld and in either mode,
/// since they are part of the photo).
pub fn colours(ink: Ink) -> (Color, Color) {
    let t = bezel::Tokens::dark();
    match ink {
        Ink::Signal => (t.signal, t.on_signal),
        Ink::Ice => (t.ice, t.on_ice),
    }
}

/// A stroke's width on a `w` × `h` picture.
pub fn line_width(w: f32, h: f32) -> f32 {
    (w.max(h) * 0.009).max(2.0)
}

/// A label's text size on a `w` × `h` picture.
pub fn text_size(w: f32, h: f32) -> f32 {
    (w.min(h) * 0.07).max(9.0)
}

/// The labels' voice: Google Sans, semibold.
pub fn text_style(size: f32) -> TextStyle {
    TextStyle::sans(size, 620.0)
}

/// A mark's strokes on a `w` × `h` picture, in its pixels: an arrow's shaft
/// and head, a box's sides, a circle (an ellipse in its box) as a ring of
/// short strokes. A label has none.
pub fn paths(m: &Mark, w: f32, h: f32) -> Vec<Vec<(f32, f32)>> {
    let (a, b) = ((m.a.0 * w, m.a.1 * h), (m.b.0 * w, m.b.1 * h));
    match m.shape {
        Shape::Arrow => {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-3);
            let (ux, uy) = (dx / len, dy / len);
            let head = (line_width(w, h) * 4.5).min(len * 0.6);
            let (s, c) = 28f32.to_radians().sin_cos();
            let wing = |sign: f32| (b.0 - head * (ux * c - sign * uy * s), b.1 - head * (uy * c + sign * ux * s));
            vec![vec![a, b], vec![wing(1.0), b, wing(-1.0)]]
        }
        Shape::Box => {
            let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
            vec![vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]]
        }
        Shape::Circle => {
            let (cx, cy) = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
            let (rx, ry) = (((b.0 - a.0) * 0.5).abs().max(1.0), ((b.1 - a.1) * 0.5).abs().max(1.0));
            vec![(0..=72).map(|i| (i as f32 / 72.0 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (cx + rx * c, cy + ry * s)).collect()]
        }
        Shape::Text => Vec::new(),
    }
}

/// Where a label's capsule goes on a `w` × `h` picture, its words `text_w`
/// wide: centred on the mark's place, kept whole on the picture.
pub fn label_box(m: &Mark, w: f32, h: f32, text_w: f32) -> [f32; 4] {
    let size = text_size(w, h);
    let (bw, bh) = ((text_w + size * 1.1).min(w), (size * 1.55).min(h));
    let x = (m.a.0 * w - bw * 0.5).clamp(0.0, w - bw);
    let y = (m.a.1 * h - bh * 0.5).clamp(0.0, h - bh);
    [x, y, bw, bh]
}

/// Coverage of a stroke (round caps) of width `width` from `p` to `q` at
/// (x, y): its distance from the segment, anti-aliased over a pixel.
fn capsule(x: f32, y: f32, p: (f32, f32), q: (f32, f32), width: f32) -> f32 {
    let (vx, vy) = (q.0 - p.0, q.1 - p.1);
    let (wx, wy) = (x - p.0, y - p.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (wx - vx * t, wy - vy * t);
    (width * 0.5 - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
}

/// `color` over a pixel at coverage `k` (0 to 1).
#[inline]
fn lay(px: &mut [u8], color: Color, k: f32) {
    if k <= 0.0 {
        return;
    }
    let k = (k * 256.0) as u32;
    for (o, c) in px.iter_mut().zip([color.0, color.1, color.2]) {
        *o = ((u32::from(*o) * (256 - k) + u32::from(c) * k) >> 8) as u8;
    }
}

/// The marks drawn onto a picture (the export: at its full size).
pub fn draw(pic: &mut Picture, marks: &[Mark]) {
    if pic.is_empty() || marks.is_empty() {
        return;
    }
    let (w, h) = (pic.w as f32, pic.h as f32);
    let width = line_width(w, h);
    let mut text: Option<bezel::Text> = None;
    for m in marks {
        let (ink, on) = colours(m.ink);
        if m.shape == Shape::Text {
            let text = text.get_or_insert_with(bezel::Text::new);
            draw_label(pic, text, m, ink, on);
            continue;
        }
        // The mark's own coverage, so strokes that meet are laid once.
        let strokes: Vec<((f32, f32), (f32, f32))> = paths(m, w, h).iter().flat_map(|p| p.windows(2).map(|s| (s[0], s[1])).collect::<Vec<_>>()).collect();
        let pad = width * 0.5 + 1.0;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (p, q) in &strokes {
            x0 = x0.min(p.0.min(q.0));
            y0 = y0.min(p.1.min(q.1));
            x1 = x1.max(p.0.max(q.0));
            y1 = y1.max(p.1.max(q.1));
        }
        let (bx, by) = ((x0 - pad).floor().max(0.0) as usize, (y0 - pad).floor().max(0.0) as usize);
        let (ex, ey) = (((x1 + pad).ceil() as usize).min(pic.w), ((y1 + pad).ceil() as usize).min(pic.h));
        if bx >= ex || by >= ey {
            continue;
        }
        let bw = ex - bx;
        let mut cover = vec![0u8; bw * (ey - by)];
        for (p, q) in &strokes {
            let (sx0, sy0) = (((p.0.min(q.0) - pad).floor().max(0.0) as usize).max(bx), ((p.1.min(q.1) - pad).floor().max(0.0) as usize).max(by));
            let (sx1, sy1) = (((p.0.max(q.0) + pad).ceil() as usize).min(ex), ((p.1.max(q.1) + pad).ceil() as usize).min(ey));
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let k = (capsule(x as f32 + 0.5, y as f32 + 0.5, *p, *q, width) * 255.0) as u8;
                    let at = (y - by) * bw + x - bx;
                    cover[at] = cover[at].max(k);
                }
            }
        }
        for y in by..ey {
            for x in bx..ex {
                let k = cover[(y - by) * bw + x - bx];
                if k > 0 {
                    let at = (y * pic.w + x) * 3;
                    lay(&mut pic.rgb[at..at + 3], ink, f32::from(k) / 255.0);
                }
            }
        }
    }
}

/// A label: a capsule of its colour, its words on it in the colour for
/// words on that colour, rendered at the picture's own size.
fn draw_label(pic: &mut Picture, text: &mut bezel::Text, m: &Mark, ink: Color, on: Color) {
    let (w, h) = (pic.w as f32, pic.h as f32);
    let words = if m.text.trim().is_empty() { "label" } else { m.text.trim() };
    let style = text_style(text_size(w, h));
    let tw = text.measure(words, &style);
    let [x, y, bw, bh] = label_box(m, w, h, tw);
    let (x, y) = (x.round(), y.round());
    let r = bh * 0.5;
    // The words, white on black, as their own coverage: the canvas's pixel
    // (cx, cy) is the picture's (x + cx − 1, y + cy − 1).
    let (cw, ch) = (bw.ceil() as i32 + 2, bh.ceil() as i32 + 2);
    let mut canvas = bezel::Canvas::new(cw, ch);
    canvas.clear(Color::BLACK);
    let fitted = text.fit(words, &style, bw - style.size * 0.9);
    let fw = text.measure(&fitted, &style);
    text.draw(&mut canvas, 1.0 + ((bw - fw) * 0.5).round(), 1.0 + (bh * 0.5 + style.size * 0.36).round(), &fitted, &style, Color(255, 255, 255), 1.0);
    let (x0, y0) = (x as usize, y as usize);
    for py in y0..((y + bh).ceil() as usize).min(pic.h) {
        for px in x0..((x + bw).ceil() as usize).min(pic.w) {
            // The capsule: within `r` of its middle line.
            let (fx, fy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
            let (dx, dy) = (fx - fx.clamp(r, (bw - r).max(r)), fy - r);
            let k = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            let at = (py * pic.w + px) * 3;
            lay(&mut pic.rgb[at..at + 3], ink, k);
            let (cx, cy) = ((px - x0) as i32 + 1, (py - y0) as i32 + 1);
            if cx < cw && cy < ch {
                let t = canvas.px[(cy * cw + cx) as usize];
                let cover = ((t >> 8) & 0xff) as f32 / 255.0;
                lay(&mut pic.rgb[at..at + 3], on, cover * k);
            }
        }
    }
}
