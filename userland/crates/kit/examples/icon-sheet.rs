//! See an app's icon as the shell will draw it, before it is packed:
//!
//! ```text
//! cargo run -p catalyst-kit --example icon-sheet -- sheet.png icon.svg [more.svg…]
//! ```
//!
//! Each icon is drawn at the sizes the shell uses (a Home tile's 14 px, a
//! list's 16 and 18, the catalogue's 24) in the ink and the signal colour,
//! on both tones, as a PNG three times the panel's pixels.

use catalyst_kit::bezel::{icons, Canvas, Icons, Tokens};
use catalyst_kit::Accent;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, svgs)) = args.split_first() else {
        eprintln!("icon-sheet OUT.png ICON.svg…");
        std::process::exit(2);
    };
    let sizes = [14.0, 16.0, 18.0, 24.0, 32.0];
    let (cell, rows) = (40.0, svgs.len());
    let w = (sizes.len() as f32 * cell * 2.0) as i32;
    let h = (rows as f32 * cell).max(cell) as i32;
    let mut canvas = Canvas::new(w, h);
    let mut drawer = Icons::new();
    for (row, file) in svgs.iter().enumerate() {
        let svg = std::fs::read_to_string(file).unwrap_or_default();
        let Some(path) = icons::svg_path(&svg) else {
            eprintln!("{file}: not a Material Symbol in its 960-unit box");
            std::process::exit(1);
        };
        let name = format!("sheet:{row}");
        icons::register(&name, path);
        for (half, tokens) in [Tokens::of(Accent::Rose, catalyst_kit::Tone::Dark), Tokens::of(Accent::Rose, catalyst_kit::Tone::Light)].iter().enumerate() {
            let x0 = half as f32 * sizes.len() as f32 * cell;
            let y = row as f32 * cell;
            canvas.fill_rect(catalyst_kit::Rect::new(x0, y, sizes.len() as f32 * cell, cell), tokens.ground, 1.0);
            for (i, size) in sizes.iter().enumerate() {
                let color = if i % 2 == 0 { tokens.ink } else { tokens.signal };
                drawer.draw(&mut canvas, &name, false, x0 + (i as f32 + 0.5) * cell, y + cell * 0.5, *size, color, 1.0);
            }
        }
    }
    let scale = 3usize;
    let (cw, ch) = (canvas.w as usize, canvas.h as usize);
    let mut rgb = vec![0u8; cw * scale * ch * scale * 3];
    for y in 0..ch * scale {
        for x in 0..cw * scale {
            let p = canvas.px[(y / scale) * cw + x / scale];
            rgb[(y * cw * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(out).expect("write the sheet"));
    let mut enc = png::Encoder::new(file, (cw * scale) as u32, (ch * scale) as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("{} icons drawn to {out}", svgs.len());
}
