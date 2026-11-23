//! QR labels for the fleet, as a sheet to print: US Letter at 300 dpi, laid
//! out as Avery 5160 / 8160 address labels (2⅝ × 1 in, three across, ten
//! down), so it prints onto a sheet of them at actual size. Each label has
//! the battery's QR code (`catalyst-battery:<id>`, which the Camera's scan
//! mode opens), its name large, and its year and team.
//!
//! Written a label row at a time into a greyscale PNG, so the 2550 × 3300
//! sheet never needs more than one row's pixels in memory.

use std::io::Write;
use std::path::{Path, PathBuf};

use bezel::{Align, Canvas, Color, Rect, Text, TextStyle};

use super::{Battery, QR_PREFIX};

/// The sheet and its labels, in pixels at 300 dpi.
pub const SHEET_W: usize = 2550;
pub const SHEET_H: usize = 3300;
pub const LABEL_W: usize = 788;
pub const LABEL_H: usize = 300;
/// Where the first label starts, and from one to the next across.
pub const LEFT: usize = 56;
pub const TOP: usize = 150;
pub const PITCH: usize = 825;
pub const ACROSS: usize = 3;
pub const DOWN: usize = 10;
/// The most one sheet holds.
pub const PER_SHEET: usize = ACROSS * DOWN;

/// Where the sheet is kept for Catalyst Companion (its Batteries place).
pub fn path() -> PathBuf {
    super::dir().join("labels.png")
}

/// Where a label is on the sheet.
pub fn label_rect(i: usize) -> Rect {
    let (col, row) = (i % ACROSS, i / ACROSS);
    Rect::new((LEFT + col * PITCH) as f32, (TOP + row * LABEL_H) as f32, LABEL_W as f32, LABEL_H as f32)
}

/// Write the sheet for these batteries (the first 30) to `out`.
pub fn write(batteries: &[Battery], team: Option<u32>, out: impl Write) -> Result<(), String> {
    let mut enc = png::Encoder::new(out, SHEET_W as u32, SHEET_H as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    // 300 dpi, so it prints at actual size.
    enc.set_pixel_dims(Some(png::PixelDimensions { xppu: 11_811, yppu: 11_811, unit: png::Unit::Meter }));
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut stream = writer.stream_writer().map_err(|e| e.to_string())?;
    let white = vec![255u8; SHEET_W];
    let mut text = Text::new();
    let shown = &batteries[..batteries.len().min(PER_SHEET)];
    let rows = shown.len().div_ceil(ACROSS);
    let mut band = Canvas::new(SHEET_W as i32, LABEL_H as i32);
    let mut grey = vec![0u8; SHEET_W];
    for y in 0..SHEET_H {
        let row = y.checked_sub(TOP).map(|d| d / LABEL_H).filter(|r| *r < rows);
        let Some(row) = row else {
            stream.write_all(&white).map_err(|e| e.to_string())?;
            continue;
        };
        let line = (y - TOP) % LABEL_H;
        if line == 0 {
            band.clear(Color::hex(0xffffff));
            for col in 0..ACROSS {
                if let Some(b) = shown.get(row * ACROSS + col) {
                    let r = label_rect(col);
                    draw_label(&mut band, &mut text, Rect::new(r.x, 0.0, r.w, r.h), b, team);
                }
            }
        }
        let px = &band.px[line * SHEET_W..(line + 1) * SHEET_W];
        for (g, p) in grey.iter_mut().zip(px) {
            *g = (*p >> 8) as u8;
        }
        stream.write_all(&grey).map_err(|e| e.to_string())?;
    }
    stream.finish().map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

/// Make the sheet where Companion finds it; where it went, or why not.
pub fn save(batteries: &[Battery], team: Option<u32>) -> Result<PathBuf, String> {
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let part = path.with_extension("png.part");
    let file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    write(batteries, team, std::io::BufWriter::new(file))?;
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// One label: a faint outline to cut along on plain paper, the QR code at
/// its left, the name large beside it, the year and team under the name.
fn draw_label(c: &mut Canvas, text: &mut Text, r: Rect, b: &Battery, team: Option<u32>) {
    let ink = Color::hex(0x000000);
    let quiet = Color::hex(0x555555);
    c.stroke_rrect(r.inset(3.0), 24.0, 2.0, Color::hex(0xd4d4d4), 1.0);
    let Ok(qr) = qrcodegen::QrCode::encode_text(&format!("{QR_PREFIX}{}", b.id), qrcodegen::QrCodeEcc::Medium) else { return };
    // The code with its quiet zone (two modules) in a square the label's height less a margin.
    let n = qr.size();
    let side = r.h - 40.0;
    let module = (side / (n + 4) as f32).floor();
    let x0 = r.x + 20.0 + 2.0 * module;
    let y0 = r.y + (r.h - module * n as f32) * 0.5;
    for y in 0..n {
        for x in 0..n {
            if qr.get_module(x, y) {
                c.fill_rect(Rect::new(x0 + x as f32 * module, y0 + y as f32 * module, module, module), ink, 1.0);
            }
        }
    }
    let left = r.x + 20.0 + (n + 4) as f32 * module + 24.0;
    let room = r.right() - 24.0 - left;
    let name_style = TextStyle::title(fit_size(text, &b.name, room, 130.0));
    let name = text.fit(&b.name, &name_style, room);
    text.draw_aligned(c, left, r.y + 150.0, Align::Left, &name, &name_style, ink, 1.0);
    let line = match (b.year, team) {
        (Some(y), Some(t)) => format!("{y} · frc {t}"),
        (Some(y), None) => y.to_string(),
        (None, Some(t)) => format!("frc {t}"),
        (None, None) => String::new(),
    };
    let small = TextStyle::label(36.0);
    let line = text.fit(&line, &small, room);
    text.draw_aligned(c, left, r.y + 212.0, Align::Left, &line, &small, quiet, 1.0);
    let foot = TextStyle::label(24.0);
    let words = text.fit("scan in catalyst", &foot, room);
    text.draw_aligned(c, left, r.y + 256.0, Align::Left, &words, &foot, quiet, 1.0);
}

/// The largest size up to `most` at which `s` fits `room`.
fn fit_size(text: &mut Text, s: &str, room: f32, most: f32) -> f32 {
    let mut size = most;
    while size > 48.0 && text.measure(s, &TextStyle::title(size)) > room {
        size -= 4.0;
    }
    size
}
