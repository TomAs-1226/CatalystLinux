//! The simulated camera's page: an A4 sheet of a robot team's checklist,
//! lying on the table at a slant and lit from one side, seen through the
//! IMX219's lens (a real perspective, so its true shape can be found back
//! from it), drifting a little as a hand holds the camera.

use std::sync::OnceLock;

use bezel::{Canvas, Color, Text, TextStyle};

use super::quad::Quad;
use super::warp::{Homography, FOCAL_FULL};
use crate::photos::Picture;

/// The sheet: A4 at about 100 dpi.
const SHEET: (usize, usize) = (840, 1188);

/// The page's own picture, made once: a checklist with a heading, ticks,
/// a small drawing and a table, in ink on paper.
pub fn sheet() -> &'static Picture {
    static SHEET_PIC: OnceLock<Picture> = OnceLock::new();
    SHEET_PIC.get_or_init(|| {
        let (w, h) = (SHEET.0 as i32, SHEET.1 as i32);
        let mut c = Canvas::new(w, h);
        let paper = Color(247, 245, 240);
        let ink = Color(28, 30, 36);
        let soft = Color(96, 100, 110);
        let accent = Color(214, 74, 32);
        c.clear(paper);
        let mut text = Text::new();
        let m = 72.0;
        text.draw(&mut c, m, 128.0, "Intake inspection", &TextStyle::sans(58.0, 700.0), ink, 1.0);
        text.draw(&mut c, m, 176.0, "Team 5805 · pit checklist · match 14", &TextStyle::sans(26.0, 450.0), soft, 1.0);
        c.fill_rect(bezel::Rect::new(m, 204.0, w as f32 - 2.0 * m, 4.0), accent, 1.0);
        let items = [
            ("Roller belts seated, no fraying", true),
            ("Polycarbonate guards: 4 bolts each", true),
            ("Intake motor: 40 A breaker, cool", true),
            ("Beam break sensor reads note", false),
            ("Pivot hard stops tight", true),
            ("Zip ties trimmed flush", false),
        ];
        for (i, (words, done)) in items.iter().enumerate() {
            let y = 280.0 + i as f32 * 64.0;
            c.stroke_rrect(bezel::Rect::new(m, y - 26.0, 34.0, 34.0), 6.0, 3.5, ink, 1.0);
            if *done {
                c.polyline(&[(m + 7.0, y - 9.0), (m + 15.0, y), (m + 29.0, y - 20.0)], 5.0, accent, 1.0);
            }
            text.draw(&mut c, m + 54.0, y, words, &TextStyle::sans(30.0, 430.0), ink, 1.0);
        }
        // A drawing of the intake: its frame, two rollers, the note.
        let top = 690.0;
        c.stroke_rrect(bezel::Rect::new(m, top, 330.0, 220.0), 10.0, 4.0, ink, 1.0);
        for cx in [m + 95.0, m + 235.0] {
            c.arc(cx, top + 110.0, 46.0, 4.0, 0.0, std::f32::consts::TAU, ink, 1.0);
            c.circle(cx, top + 110.0, 8.0, ink, 1.0);
        }
        c.arc(m + 165.0, top + 175.0, 30.0, 6.0, 0.0, std::f32::consts::TAU, accent, 1.0);
        text.draw(&mut c, m, top + 258.0, "Fig. 1  intake, side view", &TextStyle::sans(24.0, 450.0), soft, 1.0);
        // A small table of what was measured.
        let (tx, tw) = (m + 380.0, w as f32 - m - (m + 380.0));
        let rows = [("Belt tension", "12 N"), ("Roller gap", "47 mm"), ("Current", "18 A"), ("Cycle", "0.42 s")];
        for (i, (k, v)) in rows.iter().enumerate() {
            let y = top + 40.0 + i as f32 * 52.0;
            text.draw(&mut c, tx, y, k, &TextStyle::sans(26.0, 450.0), ink, 1.0);
            let vw = text.measure(v, &TextStyle::sans(26.0, 650.0));
            text.draw(&mut c, tx + tw - vw, y, v, &TextStyle::sans(26.0, 650.0), ink, 1.0);
            c.fill_rect(bezel::Rect::new(tx, y + 16.0, tw, 2.0), soft, 0.6);
        }
        for i in 0..4 {
            let y = 992.0 + i as f32 * 34.0;
            let len = [0.92, 0.84, 0.95, 0.55][i];
            c.rrect(bezel::Rect::new(m, y, (w as f32 - 2.0 * m) * len, 12.0), 6.0, Color(150, 154, 162), 1.0);
        }
        let mut pic = Picture::new(SHEET.0, SHEET.1);
        for (o, p) in pic.rgb.chunks_exact_mut(3).zip(&c.px) {
            o.copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        pic
    })
}

/// Where the sheet's corners are in a full `w` × `h` picture at time `t`:
/// an A4 sheet 0.45 m from the lens, its top tipped 24° away and turned 7°,
/// drifting by a few millimetres as a hand does.
pub fn corners(w: f32, h: f32, t: f32) -> Quad {
    let f = FOCAL_FULL * w / crate::camera::FULL.0 as f32;
    let (tilt, turn) = (24f32.to_radians(), 7f32.to_radians());
    let (dx, dy) = (0.0025 * (t * 1.3).sin(), 0.002 * (t * 0.9).cos());
    let project = |u: f32, v: f32| {
        let (x, y) = (u * turn.cos() - v * turn.sin(), u * turn.sin() + v * turn.cos());
        let (px, py, pz) = (x + 0.012 + dx, y * tilt.cos() + 0.01 + dy, 0.45 - y * tilt.sin());
        (w * 0.5 + f * px / pz, h * 0.5 + f * py / pz)
    };
    let (a, b) = (0.105, 0.1485);
    [project(-a, -b), project(a, -b), project(a, b), project(-a, b)]
}

/// The sheet laid into `pic`, which shows `view` (x0, y0, x1, y1, 0 to 1)
/// of the full picture, at time `t`: its light falling off from the top
/// left, a soft shadow under its far edge.
pub fn paint(pic: &mut Picture, view: [f32; 4], t: f32) {
    let (fw, fh) = (crate::camera::FULL.0 as f32, crate::camera::FULL.1 as f32);
    let q = corners(fw, fh, t);
    // The full picture's pixels to this picture's.
    let (sx, sy) = (pic.w as f32 / ((view[2] - view[0]) * fw), pic.h as f32 / ((view[3] - view[1]) * fh));
    let here = q.map(|(x, y)| ((x - view[0] * fw) * sx, (y - view[1] * fh) * sy));
    paint_at(pic, &here, 1.0, 1.0);
}

/// The sheet laid into `pic` with its corners at `q` (its pixels), its
/// light times `light`, falling off across it by `falloff` (1: as lit on the table).
pub fn paint_at(pic: &mut Picture, q: &Quad, light: f32, falloff: f32) {
    let s = sheet();
    let (sw, sh) = (s.w as f32, s.h as f32);
    let Some(map) = Homography::between(*q, [(0.0, 0.0), (sw, 0.0), (sw, sh), (0.0, sh)]) else { return };
    let (x0, x1) = (q.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (q.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as usize + 1).min(pic.w));
    let (y0, y1) = (q.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (q.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as usize + 1).min(pic.h));
    // How many sheet pixels a picture pixel spans, for a box average there.
    let span = ((sw / (q[1].0 - q[0].0).abs().max(1.0)).max(1.0)) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = map.at(x as f64 + 0.5, y as f64 + 0.5);
            let (u, v) = (u as f32, v as f32);
            if u < 0.0 || v < 0.0 || u >= sw || v >= sh {
                continue;
            }
            // The edge's coverage: a pixel's worth of fade.
            let edge = (u.min(sw - u).min(v).min(sh - v) / (span as f32)).clamp(0.0, 1.0);
            let lit = light * (1.0 - falloff * 0.38 * (u / sw * 0.6 + v / sh * 0.4)) * (1.0 - falloff * 0.06 * (1.0 - v / sh));
            let mut sum = [0f32; 3];
            if span <= 1 {
                // Nearer than the sheet's own pixels: read between them.
                let (fu, fv) = ((u - 0.5).clamp(0.0, sw - 1.0), (v - 0.5).clamp(0.0, sh - 1.0));
                let (u0, v0) = (fu as usize, fv as usize);
                let (u1, v1) = ((u0 + 1).min(s.w - 1), (v0 + 1).min(s.h - 1));
                let (kx, ky) = (fu - u0 as f32, fv - v0 as f32);
                for (k, c) in sum.iter_mut().enumerate() {
                    let p = |xx: usize, yy: usize| f32::from(s.rgb[(yy * s.w + xx) * 3 + k]);
                    let top = p(u0, v0) + (p(u1, v0) - p(u0, v0)) * kx;
                    let bottom = p(u0, v1) + (p(u1, v1) - p(u0, v1)) * kx;
                    *c = top + (bottom - top) * ky;
                }
            } else {
                let (cx, cy) = (u as usize, v as usize);
                let mut n = 0.0;
                for yy in cy..(cy + span).min(s.h) {
                    for xx in cx..(cx + span).min(s.w) {
                        let at = (yy * s.w + xx) * 3;
                        for (k, c) in sum.iter_mut().enumerate() {
                            *c += f32::from(s.rgb[at + k]);
                        }
                        n += 1.0;
                    }
                }
                sum = sum.map(|c| c / f32::max(n, 1.0));
            }
            let at = (y * pic.w + x) * 3;
            for k in 0..3 {
                let v = sum[k] * lit;
                let under = f32::from(pic.rgb[at + k]);
                pic.rgb[at + k] = (under + (v - under) * edge).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}
