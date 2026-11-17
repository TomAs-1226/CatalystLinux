//! Renders a test frame to a PNG (upscaled for inspection) and times the passes.
//! `cargo run -p bezel --example specimen --release -- out.png [light] [calm]`

use std::time::Instant;

use bezel::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "specimen.png".into());
    let tokens = if args.iter().any(|a| a == "light") { Tokens::light() } else { Tokens::dark() };
    let calm = args.iter().any(|a| a == "calm");

    let (w, h) = (320, 170);
    let mut canvas = Canvas::new(w, h);
    let mut text = Text::new();
    let mut icons = Icons::new();
    let mut glass = Glass::new();

    let t0 = Instant::now();
    canvas.clear(tokens.ground);
    canvas.glow(60.0, 20.0, 120.0, tokens.signal, 0.10);
    canvas.glow(280.0, 150.0, 110.0, tokens.ice, 0.06);

    // Tiles.
    let names = [("smart_toy", "robot", "linked"), ("monitor_heart", "health", "2 warn"), ("description", "logs", "14 files"), ("speed", "loop", "4.2 ms")];
    for (i, (icon, label, value)) in names.iter().enumerate() {
        let r = Rect::new(8.0 + i as f32 * 77.0, 26.0, 71.0, 62.0);
        canvas.rrect(r, 14.0, tokens.surface1, 1.0);
        icons.draw(&mut canvas, icon, i == 0, r.x + 17.0, r.y + 17.0, 18.0, if i == 0 { tokens.ink } else { tokens.dim }, 1.0);
        text.draw(&mut canvas, r.x + 9.0, r.y + 43.0, label, &TextStyle::name(13.0), tokens.ink, 1.0);
        text.draw(&mut canvas, r.x + 9.0, r.y + 55.0, value, &TextStyle::label(9.0), tokens.dim, 1.0);
        if i == 1 {
            canvas.stroke_rrect(r.inset(-3.0), 17.0, 2.0, tokens.signal, 1.0);
        }
    }
    // A glanceable value, a meter and a sparkline.
    let r = Rect::new(8.0, 94.0, 148.0, 68.0);
    canvas.rrect(r, 14.0, tokens.surface1, 1.0);
    text.draw(&mut canvas, r.x + 9.0, r.y + 14.0, "battery", &TextStyle::label(9.0), tokens.dim, 1.0);
    let vw = text.draw(&mut canvas, r.x + 8.0, r.y + 46.0, "12.6", &TextStyle::display(30.0), tokens.ink, 1.0);
    text.draw(&mut canvas, r.x + 11.0 + vw, r.y + 46.0, "v", &TextStyle::label(10.0), tokens.dim, 1.0);
    let track = Rect::new(r.x + 9.0, r.y + 53.0, r.w - 18.0, 8.0);
    canvas.rrect(track, 4.0, tokens.meter, 1.0);
    canvas.rrect_clipped(Rect::new(track.x, track.y, track.w * 0.82, track.h), 2.0, track, 4.0, tokens.ice, 1.0);

    let r = Rect::new(162.0, 94.0, 150.0, 68.0);
    canvas.rrect(r, 14.0, tokens.surface1, 1.0);
    text.draw(&mut canvas, r.x + 9.0, r.y + 14.0, "loop time · ms", &TextStyle::label(9.0), tokens.dim, 1.0);
    let pts: Vec<(f32, f32)> = (0..40)
        .map(|i| (r.x + 10.0 + i as f32 * 3.3, r.y + 42.0 - ((i as f32 * 0.5).sin() * 9.0 + (i as f32 * 1.7).cos() * 4.0)))
        .collect();
    canvas.polyline(&pts, 1.5, tokens.signal, 1.0);
    canvas.arc(r.right() - 22.0, r.y + 46.0, 11.0, 3.0, -2.2, 4.4, tokens.meter, 1.0);
    canvas.arc(r.right() - 22.0, r.y + 46.0, 11.0, 3.0, -2.2, 3.0, tokens.signal, 1.0);
    text.draw(&mut canvas, 10.0, 16.0, "Overview", &TextStyle::title(16.0), tokens.ink, 1.0);
    let scene = t0.elapsed();

    // Glass: the island at the top, a dock over the tiles, and two shapes melting.
    let island = Rect::new(112.0, 4.0, 96.0, 17.0);
    let dock = Rect::new(60.0, 134.0, 200.0, 30.0);
    let shapes = [
        GlassShape::new(island, 8.5, 0),
        GlassShape::new(dock, 15.0, 1),
        GlassShape::new(Rect::new(248.0, 60.0, 30.0, 30.0), 15.0, 2),
        GlassShape { tint: Some(tokens.signal), ..GlassShape::new(Rect::new(276.0, 60.0, 30.0, 30.0), 15.0, 2) },
    ];
    let t1 = Instant::now();
    glass.render(&mut canvas, &shapes, &tokens, LIGHT_REST, calm);
    let glass_time = t1.elapsed();

    // What sits on glass is not glass.
    canvas.circle(island.x + 10.0, island.cy(), 2.5, tokens.ice, 1.0);
    text.draw_aligned(&mut canvas, island.cx() + 4.0, island.cy() + 3.0, Align::Center, "5805 · linked", &TextStyle::label(9.0), tokens.ink, 1.0);
    for (i, icon) in ["home", "smart_toy", "description", "photo_camera", "settings"].iter().enumerate() {
        let cx = dock.x + 22.0 + i as f32 * 39.0;
        if i == 0 {
            canvas.rrect(Rect::new(cx - 17.0, dock.y + 3.0, 34.0, 24.0), 12.0, tokens.ink, 0.075);
        }
        icons.draw(&mut canvas, icon, i == 0, cx, dock.cy(), 16.0, if i == 0 { tokens.ink } else { tokens.dim }, 1.0);
    }

    println!("scene {:.2} ms · glass {:.2} ms", scene.as_secs_f64() * 1e3, glass_time.as_secs_f64() * 1e3);

    // Upscale 4× with nearest neighbour so single pixels can be inspected.
    let scale = 4usize;
    let (ow, oh) = (w as usize * scale, h as usize * scale);
    let mut rgb = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            let p = canvas.px[(y / scale) * w as usize + x / scale];
            let o = (y * ow + x) * 3;
            rgb[o] = (p >> 16) as u8;
            rgb[o + 1] = (p >> 8) as u8;
            rgb[o + 2] = p as u8;
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    let mut enc = png::Encoder::new(file, ow as u32, oh as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    println!("wrote {out}");
}
