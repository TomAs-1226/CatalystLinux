//! The 3D objects laid out to be looked at and approved before they ship:
//! `catalyst-shell --objects DIR [--preview-pack PATH]`.
//!
//! Everything here is drawn by the shell's own code (`object3d`), so what
//! is seen is what the handheld draws: contact sheets of every object in
//! dark and light (and in other accents), turntables (animated PNGs, from
//! the larger preview pack when given), the objects in place on the panel
//! at its own size and enlarged three times (dithered to 16 bits as the
//! panel shows them), the motion they make as animated PNGs, what they
//! cost, and `index.html` showing all of it.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use bezel::{Accent, Align, Canvas, Color, Rect, TextStyle, Tokens, Tone};

use crate::gfx::{Gfx, H, W};
use crate::object3d::{Objects, View};
use crate::platform::{self, Key};
use crate::robot::Robot;
use crate::shell::Shell;

const APPS: [&str; 35] = [
    "Robot", "Health", "Logs", "Schedule", "Tables", "Loop", "Motors", "Console", "Terminal", "Files", "Git", "Catalogue", "Monitor", "Net tools", "Connect", "Actions", "Camera", "Photos",
    "AprilTag", "Graphs", "Pit", "Remote", "Level", "Compass", "Weather", "Browse", "Calculator", "Clock", "Notes", "Units", "HTTP", "Share", "Assistant", "Web", "Settings",
];

/// The other objects, with where each is used.
const THINGS: [(&str, &str, &str); 16] = [
    ("handheld", "hero", "setup: the welcome; working; About's header"),
    ("laptop", "hero", "setup: from a computer (Higgsfield)"),
    ("wifi", "hero", "setup: Wi-Fi"),
    ("robot", "hero", "setup: the team (Higgsfield)"),
    ("globe", "hero", "setup: name and time zone (Higgsfield)"),
    ("key", "hero", "setup: keys for services"),
    ("toggle", "hero", "setup: choices"),
    ("battery", "hero", "setup: battery and power"),
    ("trophy", "hero", "setup: all set (Higgsfield)"),
    ("rocket", "hero", "System update: ready (Higgsfield)"),
    ("moon", "hero", "Display: deep standby"),
    ("tray", "m", "empty pages"),
    ("sunmoon", "m", "control centre: light and dark"),
    ("chip", "hero", "made, not used yet: the hardware check"),
    ("keycaps", "hero", "made, not used yet: the keyboard lesson"),
    ("palette", "hero", "made, not used yet: the look"),
];

/// Frames as an animated PNG, each after the first kept only where it
/// differs from the one before (APNG's own frame regions), so motion over a
/// still screen stays small.
fn save_apng(frames: &[Vec<u32>], w: usize, h: usize, path: &Path, delay_ms: u16) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create animation"));
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    enc.set_animated(frames.len() as u32, 0).unwrap();
    enc.set_frame_delay(delay_ms, 1000).unwrap();
    enc.set_dispose_op(png::DisposeOp::None).unwrap();
    enc.set_blend_op(png::BlendOp::Source).unwrap();
    let mut writer = enc.write_header().unwrap();
    for (i, frame) in frames.iter().enumerate() {
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, w, h);
        if i > 0 {
            let before = &frames[i - 1];
            (x0, y0, x1, y1) = (w, h, 0, 0);
            for y in 0..h {
                for x in 0..w {
                    if frame[y * w + x] != before[y * w + x] {
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                    }
                }
            }
            if x1 <= x0 {
                (x0, y0, x1, y1) = (0, 0, 1, 1);
            }
        }
        let mut rgb = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
        for y in y0..y1 {
            for &p in &frame[y * w + x0..y * w + x1] {
                rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        // Each is checked against the other as it is set: back to the corner first.
        writer.set_frame_position(0, 0).unwrap();
        writer.set_frame_dimension((x1 - x0) as u32, (y1 - y0) as u32).unwrap();
        writer.set_frame_position(x0 as u32, y0 as u32).unwrap();
        writer.write_image_data(&rgb).unwrap();
    }
    writer.finish().unwrap();
}

fn panel(c: &Canvas) -> Canvas {
    let mut out = c.clone();
    let w = c.w as usize;
    for (i, p) in out.px.iter_mut().enumerate() {
        *p = bezel::canvas::rgb565_to_rgb(bezel::canvas::rgb565_dithered(*p, i % w, i / w));
    }
    out
}

fn tokens(accent: Accent, tone: Tone) -> Tokens {
    Tokens::of(accent, tone)
}

/// A Gfx drawing into a canvas of its own size, with every object made
/// (shipped or not) when that pack is given.
fn sheet_gfx(w: i32, h: i32, t: Tokens) -> Gfx {
    let mut g = Gfx::new(t);
    if let Some(all) = ALL.get() {
        g.objects = Objects::from_static(all).expect("the pack of every object");
    }
    g.canvas = Canvas::new(w, h);
    g.canvas.clear(t.ground);
    g
}

/// Every app's object at both its sizes, on a tile, named.
fn apps_sheet(dir: &Path, name: &str, accent: Accent, tone: Tone) -> String {
    let t = tokens(accent, tone);
    let (cw, ch, across) = (112, 84, 7);
    let rows = APPS.len().div_ceil(across);
    let mut g = sheet_gfx(across as i32 * cw + 8, rows as i32 * ch + 8, t);
    for (i, app) in APPS.iter().enumerate() {
        let (x, y) = (8.0 + (i % across) as f32 * cw as f32, 8.0 + (i / across) as f32 * ch as f32);
        let tile = Rect::new(x, y, cw as f32 - 8.0, ch as f32 - 8.0);
        g.canvas.rrect(tile, 12.0, t.surface1, 1.0);
        let object = crate::object3d::for_app(app);
        g.object(&object, "s", x + 22.0, y + 30.0, View::REST, 1.0, 1.0);
        g.object(&object, "l", x + 70.0, y + 30.0, View::REST, 1.0, 1.0);
        g.text_at(tile.cx(), tile.bottom() - 9.0, Align::Center, app, TextStyle::label(11.0), t.dim);
    }
    let file = format!("{name}.png");
    platform::sim::save_png(&g.canvas, &dir.join(&file), 2);
    file
}

/// The other objects at their sizes, named, with where they are used.
fn things_sheet(dir: &Path, name: &str, accent: Accent, tone: Tone) -> String {
    let t = tokens(accent, tone);
    let (cw, ch, across) = (150, 136, 5);
    let rows = THINGS.len().div_ceil(across);
    let mut g = sheet_gfx(across as i32 * cw + 8, rows as i32 * ch + 8, t);
    for (i, (object, size, _)) in THINGS.iter().enumerate() {
        let (x, y) = (8.0 + (i % across) as f32 * cw as f32, 8.0 + (i / across) as f32 * ch as f32);
        let tile = Rect::new(x, y, cw as f32 - 8.0, ch as f32 - 8.0);
        g.canvas.rrect(tile, 14.0, t.surface1, 1.0);
        g.object(object, size, tile.cx(), y + 56.0, View::REST, 1.0, 1.0);
        g.text_at(tile.cx(), tile.bottom() - 12.0, Align::Center, object, TextStyle::label(11.0), t.dim);
    }
    let file = format!("{name}.png");
    platform::sim::save_png(&g.canvas, &dir.join(&file), 2);
    file
}

/// One object turning all the way round, dark beside light (an app icon
/// dark only: the sheets show it light): from the preview pack (larger) when
/// there is one, else at its shipped size.
fn turntable(dir: &Path, objects: &mut Objects, object: &str, size: &str, accent: Accent) -> Option<(String, i32, i32)> {
    let px = objects.px(object, size)? as i32;
    let cell = px + 16;
    let tones: &[Tone] = if object.starts_with("app-") { &[Tone::Dark] } else { &[Tone::Dark, Tone::Light] };
    let (w, h) = (cell * tones.len() as i32, cell);
    let mut frames = Vec::new();
    // Each rendered angle once, 15° apart: what was rendered, nothing blended.
    for step in 0..24 {
        let yaw = step as f32 * 15.0;
        let mut c = Canvas::new(w, h);
        for (k, &tone) in tones.iter().enumerate() {
            let t = tokens(accent, tone);
            c.fill_rect(Rect::new((k as i32 * cell) as f32, 0.0, cell as f32, cell as f32), t.surface1, 1.0);
            let fault = if tone == Tone::Dark { Color::hex(0xff6b5e) } else { Color::hex(0xb3261e) };
            objects.draw(&mut c, &t, fault, object, size, (k as i32 * cell + cell / 2) as f32, cell as f32 * 0.5, View::yaw(yaw), 1.0, 1.0);
        }
        frames.push(c.px);
    }
    let file = format!("turn-{object}.png");
    save_apng(&frames, w as usize, h as usize, &dir.join(&file), 70);
    Some((file, w, h))
}

/// The shell, driven by keys, its frames kept.
struct Run {
    shell: Shell,
}

impl Run {
    fn new(light: bool, calm: bool) -> Run {
        let mut shell = Shell::new(Robot::demo());
        shell.settings.light = light;
        shell.settings.calm = calm;
        shell.apply_settings();
        let mut run = Run { shell };
        run.settle(1.6);
        run
    }
    /// Frames at the handheld's beat (not dithered: the stills show that,
    /// and the animations stay small).
    fn frames(&mut self, seconds: f32) -> Vec<Vec<u32>> {
        (0..(seconds / 0.025).round() as usize)
            .map(|_| {
                self.shell.tick(0.025);
                self.shell.draw();
                self.shell.gfx.canvas.px.clone()
            })
            .collect()
    }
    fn settle(&mut self, seconds: f32) {
        self.frames(seconds);
    }
    fn keys(&mut self, keys: &[Key], between: f32) {
        for k in keys {
            self.shell.key(*k);
            self.settle(between);
        }
    }
    fn still(&mut self, dir: &Path, name: &str, shots: &mut Vec<String>) {
        self.shell.draw();
        let c = panel(&self.shell.gfx.canvas);
        platform::sim::save_png(&c, &dir.join(format!("{name}.png")), 1);
        platform::sim::save_png(&c, &dir.join(format!("{name}@3x.png")), 3);
        shots.push(name.to_owned());
    }
}

fn movie(dir: &Path, name: &str, frames: &[Vec<u32>], movies: &mut Vec<String>) {
    save_apng(frames, W as usize, H as usize, &dir.join(format!("{name}.png")), 25);
    movies.push(name.to_owned());
}

/// The objects in place, still and moving.
fn in_situ(dir: &Path, shots: &mut Vec<String>, movies: &mut Vec<String>) {
    use Key::*;
    for (tone, light) in [("dark", false), ("light", true)] {
        // Home: the first page, then the focus moving across it.
        let mut run = Run::new(light, false);
        run.still(dir, &format!("home-{tone}"), shots);
        run.shell.key(Right);
        let f = run.frames(0.6);
        if tone == "dark" {
            movie(dir, "motion-home-focus", &f, movies);
        }
        run.keys(&[Down], 0.6);
        run.still(dir, &format!("home-focus-moved-{tone}"), shots);
        // A page turning: the icons turn as they slide.
        run.keys(&[Up, Right, Right], 0.5);
        run.shell.key(Right);
        let mut f = run.frames(0.7);
        if tone == "dark" {
            run.shell.key(Left);
            f.extend(run.frames(0.7));
            movie(dir, "motion-home-page-turn", &f, movies);
        }
    }
    // The carousel, turning.
    let mut run = Run::new(false, false);
    run.shell.settings.home = crate::shell::HomeStyle::Carousel;
    run.shell.apply_settings();
    run.settle(0.8);
    run.still(dir, "home-carousel-dark", shots);
    let mut f = Vec::new();
    for _ in 0..3 {
        run.shell.key(Right);
        f.extend(run.frames(0.45));
    }
    run.shell.key(Left);
    f.extend(run.frames(0.6));
    movie(dir, "motion-carousel", &f, movies);
    // The calm version: the same objects, resting.
    let mut run = Run::new(false, true);
    run.keys(&[Right], 0.3);
    run.still(dir, "home-calm", shots);
    // Bezel's theme and another accent.
    let mut run = Run::new(false, false);
    run.shell.settings.theme = crate::look::Theme::Bezel;
    let (wallpaper, accent) = crate::look::Theme::Bezel.defaults();
    (run.shell.settings.wallpaper, run.shell.settings.accent) = (wallpaper, accent);
    run.shell.apply_settings();
    run.settle(1.2);
    run.still(dir, "home-bezel-theme", shots);
    let mut run = Run::new(true, false);
    run.shell.settings.accent = Accent::Hue(36);
    run.shell.apply_settings();
    run.settle(1.2);
    run.still(dir, "home-own-colour-light", shots);

    // Setup, page by page, and the way through it. (Setup keeps its own
    // dark ground in either mode, so once is enough.)
    let mut run = Run::new(false, false);
    run.shell.start_setup();
    run.settle(1.0);
    run.still(dir, "setup-welcome", shots);
    run.keys(&[Down], 0.8);
    run.still(dir, "setup-from-a-computer", shots);
    run.shell.key(Up);
    let f = run.frames(0.6);
    movie(dir, "motion-setup-flip", &f, movies);
    run.shell.key(Enter);
    let f = run.frames(0.9);
    movie(dir, "motion-setup-travel", &f, movies);
    run.still(dir, "setup-wifi", shots);
    // On past the hardware and the lesson to the team.
    run.keys(&[Tab], 0.9);
    run.still(dir, "setup-hardware", shots);
    run.keys(&[Tab, Tab], 0.9);
    run.shell.key(Char('5'));
    let mut f = run.frames(0.15);
    for c in ['8', '0', '5'] {
        run.shell.key(Char(c));
        f.extend(run.frames(0.15));
    }
    f.extend(run.frames(0.6));
    movie(dir, "motion-setup-team-typed", &f, movies);
    run.still(dir, "setup-team", shots);
    run.keys(&[Enter], 0.9);
    run.still(dir, "setup-name-and-time", shots);
    run.keys(&[Tab], 0.9);
    run.still(dir, "setup-keys", shots);
    run.keys(&[Tab], 0.9);
    run.still(dir, "setup-choices", shots);
    // A choice switched: the switch gives a little turn.
    run.keys(&[Up, Up, Up, Up], 0.15);
    run.shell.key(Enter);
    let f = run.frames(0.8);
    movie(dir, "motion-setup-choice-nudge", &f, movies);
    run.shell.key(Enter);
    run.keys(&[Tab], 0.9);
    run.still(dir, "setup-power", shots);
    run.keys(&[Tab], 0.9);
    run.still(dir, "setup-look", shots);
    run.shell.key(Enter);
    run.settle(0.6);
    run.still(dir, "setup-working", shots);
    let f = run.frames(1.8);
    movie(dir, "motion-setup-finish", &f, movies);
    run.still(dir, "setup-all-set", shots);
    // Calm setup: no travel, objects resting where they stand.
    let mut run = Run::new(false, true);
    run.shell.start_setup();
    run.settle(0.5);
    run.keys(&[Enter], 0.3);
    run.still(dir, "setup-wifi-calm", shots);

    // An empty page: Graphs with nothing graphed yet, its object turning in.
    let mut run = Run::new(false, false);
    let to_graphs = crate::steps_to("Graphs");
    run.keys(&vec![Right; to_graphs], 0.12);
    run.settle(0.8);
    run.shell.key(Enter);
    let f = run.frames(1.0);
    movie(dir, "motion-empty-page", &f, movies);
    run.still(dir, "empty-graphs", shots);
    // The control centre's Light: the coin turning over with the switch.
    let mut run = Run::new(false, false);
    run.shell.key(Ctrl(' '));
    run.settle(0.8);
    run.still(dir, "control-centre", shots);
    run.keys(&[Down], 0.4);
    run.shell.key(Enter);
    let f = run.frames(0.9);
    movie(dir, "motion-control-light", &f, movies);
    run.still(dir, "control-centre-light", shots);
    // About: the handheld in its header.
    let mut run = Run::new(false, false);
    let about = crate::screens::settings_row("About this handheld");
    run.keys(&vec![Right; crate::steps_to("Settings")], 0.1);
    run.keys(&[Enter], 0.9);
    run.keys(&vec![Down; about], 0.1);
    run.keys(&[Enter], 1.2);
    run.still(dir, "about-header", shots);
}

/// What the objects cost, measured.
fn budget(dir: &Path) -> String {
    let mut out = String::new();
    let mut objects = Objects::shipped();
    let pack = objects.pack().map(|p| p.bytes()).unwrap_or(0);
    let _ = writeln!(out, "pack (in the program file; read in place, never copied whole): {:.0} kB", pack as f64 / 1024.0);
    // Decoding and colouring: every frame of an icon, of a hero.
    let t = Tokens::dark();
    let mut c = Canvas::new(320, 170);
    let started = Instant::now();
    let mut n = 0;
    for yaw in (-48..=48).step_by(12) {
        for pitch in [-12, 0, 12] {
            objects.draw(&mut c, &t, Color::BLACK, "app-robot", "s", 40.0, 40.0, View { yaw: yaw as f32, pitch: pitch as f32 }, 1.0, 1.0);
            n += 1;
        }
    }
    let icon = started.elapsed().as_secs_f64() * 1e6 / n as f64;
    let started = Instant::now();
    for yaw in (0..360).step_by(15) {
        objects.draw(&mut c, &t, Color::BLACK, "globe", "hero", 160.0, 85.0, View::yaw(yaw as f32), 1.0, 1.0);
    }
    let hero = started.elapsed().as_secs_f64() * 1e6 / 24.0;
    // Drawing a frame already decoded, and a blend of two, moved off the pixel grid.
    let started = Instant::now();
    for _ in 0..200 {
        objects.draw(&mut c, &t, Color::BLACK, "globe", "hero", 160.0, 85.0, View::yaw(0.0), 1.0, 1.0);
    }
    let lay = started.elapsed().as_secs_f64() * 1e6 / 200.0;
    let started = Instant::now();
    for i in 0..200 {
        objects.draw(&mut c, &t, Color::BLACK, "globe", "hero", 160.3, 85.0, View::yaw(7.5 + (i % 2) as f32 * 15.0), 1.1, 1.0);
    }
    let blend = started.elapsed().as_secs_f64() * 1e6 / 200.0;
    let _ = writeln!(out, "on this PC: decoding and colouring a frame: {icon:.0} µs (an icon, 30 px), {hero:.0} µs (a hero, 92 px)");
    let _ = writeln!(out, "on this PC: laying a decoded hero down: {lay:.0} µs; two blended, moved and scaled: {blend:.0} µs");
    let _ = writeln!(out, "the handheld, estimated at 20x this PC (README's factor): a hero frame decoded {:.1} ms, laid down {:.2} ms, blended {:.1} ms", hero * 20.0 / 1000.0, lay * 20.0 / 1000.0, blend * 20.0 / 1000.0);
    let _ = writeln!(out, "cache of coloured frames: at most {} kB (CACHE_BYTES); after decoding every frame above, {} kB", crate::object3d::CACHE_BYTES / 1024, objects.cached() / 1024);
    // A Home page at rest, then the focus moving, then setup end to end.
    let mut run = Run::new(false, false);
    let home = run.shell.gfx.objects.cached();
    run.keys(&[Key::Right, Key::Right, Key::Down, Key::Left], 0.4);
    let moving = run.shell.gfx.objects.peak;
    run.shell.start_setup();
    run.settle(0.8);
    run.keys(&[Key::Down, Key::Up, Key::Enter, Key::Tab, Key::Tab, Key::Enter, Key::Tab, Key::Tab, Key::Tab, Key::Tab, Key::Tab, Key::Enter], 0.8);
    run.settle(3.0);
    let _ = writeln!(out, "Home's first page at rest: {} kB of frames; with the focus moving: {} kB at most", home / 1024, moving / 1024);
    let _ = writeln!(out, "through setup end to end: {} kB at most, {} frames decoded", run.shell.gfx.objects.peak / 1024, run.shell.gfx.objects.decoded);
    std::fs::write(dir.join("budget.txt"), &out).ok();
    out
}

fn html(dir: &Path, sheets: &[(String, String)], turns: &[(String, String, i32, i32)], shots: &[String], movies: &[String], budget: &str) {
    let mut h = String::new();
    h.push_str("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Catalyst 3D objects</title><style>");
    h.push_str(":root{color-scheme:dark;--ground:#0b0c0e;--surface:#16181b;--ink:#eef0f2;--dim:#9aa1a8;--signal:#ff5b1f}body{margin:0;background:var(--ground);color:var(--ink);font:15px/1.5 system-ui,sans-serif}main{max-width:1180px;margin:0 auto;padding:24px 16px 64px}h1{font-size:28px;margin:8px 0 4px}h2{font-size:19px;margin:36px 0 8px;border-top:1px solid #2a2e33;padding-top:20px}p{color:var(--dim);max-width:70ch}img{image-rendering:pixelated;max-width:100%;height:auto;border-radius:8px;display:block}figure{margin:0}figcaption{color:var(--dim);font:12px ui-monospace,monospace;margin:6px 0 0}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:18px}.wide{display:grid;grid-template-columns:repeat(auto-fill,minmax(480px,1fr));gap:18px}pre{background:var(--surface);padding:12px 14px;border-radius:10px;overflow:auto;font-size:13px;color:var(--ink)}");
    h.push_str("</style><main><h1>Catalyst 3D objects</h1><p>Every picture here was drawn by the shell's own code from the pack it would ship (and the larger preview pack for the turntables): the same colouring for dark and light, any accent and the user's own colour. Panel shots are at the panel's 320 × 170, dithered to 16 bits as it shows them, and three times larger.</p>");
    h.push_str("<h2>Where they come from</h2><p>Built in code (<code>tools/bake3d/models.py</code>): the 35 app icons, the handheld, Wi-Fi, the key, the switch, the battery, the tray, the sun and moon coin, the moon, the processor, the keycaps and the swatches. Generated with Higgsfield (Tripo text to 3D, detailed, 12.5 credits each) and kept reduced in <code>tools/bake3d/glb/</code>: the robot, the globe, the laptop, the trophy and the rocket. A first robot came out spindly and was generated again: six generations, 75 credits in all. Everything is rendered by the same Blender bake under Bezel's one light.</p>");
    h.push_str("<h2>Contact sheets</h2><div class=\"wide\">");
    for (file, caption) in sheets {
        let _ = write!(h, "<figure><img src=\"{file}\" alt=\"{caption}\"><figcaption>{caption}</figcaption></figure>");
    }
    h.push_str("</div><h2>Turntables</h2><p>Every rendered angle (15° apart), shown twice their size: setup's objects dark beside light, the app icons dark (the sheets show them light). App icons ship only the turn either side of where they rest (±48°, at 30 and 52 px), setup's objects the whole way round (92 px); between rendered angles the shell blends the two nearest.</p><div class=\"grid\">");
    for (file, caption, w, ht) in turns {
        let _ = write!(h, "<figure><img src=\"{file}\" width=\"{}\" height=\"{}\" alt=\"{caption}\"><figcaption>{caption}</figcaption></figure>", w * 2, ht * 2);
    }
    h.push_str("</div><h2>On the panel</h2><div class=\"wide\">");
    for name in shots {
        let _ = write!(h, "<figure><img src=\"situ/{name}@3x.png\" alt=\"{name}\"><figcaption>{name} · <a href=\"situ/{name}.png\" style=\"color:var(--dim)\">actual size</a></figcaption></figure>");
    }
    h.push_str("</div><h2>Motion</h2><p>At the handheld's beat of 25 ms a frame, shown twice the panel's size.</p><div class=\"wide\">");
    for name in movies {
        let _ = write!(h, "<figure><img src=\"situ/{name}.png\" width=\"{}\" height=\"{}\" alt=\"{name}\"><figcaption>{name}</figcaption></figure>", W as i32 * 2, H as i32 * 2);
    }
    let _ = write!(h, "</div><h2>What it costs</h2><pre>{}</pre></main></html>\n", budget.replace('&', "&amp;").replace('<', "&lt;"));
    std::fs::write(dir.join("index.html"), h).expect("write the preview page");
}

static ALL: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

pub fn run(dir: &Path, preview: Option<&Path>, all: Option<&Path>) {
    if let Some(bytes) = all.and_then(|p| std::fs::read(p).ok()) {
        ALL.set(Box::leak(bytes.into_boxed_slice())).ok();
    }
    let situ = dir.join("situ");
    std::fs::create_dir_all(&situ).expect("create the preview folder");
    let default = crate::shell::Settings::default().accent;
    let mut sheets = Vec::new();
    for (tone, t) in [("dark", Tone::Dark), ("light", Tone::Light)] {
        sheets.push((apps_sheet(dir, &format!("sheet-apps-{tone}"), default, t), format!("Home's apps, {tone}: 30 px (the grid) and 52 px (the carousel)")));
        sheets.push((things_sheet(dir, &format!("sheet-objects-{tone}"), default, t), format!("setup and the system, {tone}")));
    }
    sheets.push((apps_sheet(dir, "sheet-apps-blue", Accent::Blue, Tone::Dark), "the same pack in Blue".into()));
    sheets.push((apps_sheet(dir, "sheet-apps-own-green-light", Accent::Hue(24), Tone::Light), "the user's own colour (a green), light".into()));
    sheets.push((things_sheet(dir, "sheet-objects-own-violet", Accent::Violet, Tone::Dark), "setup's objects in Violet".into()));

    let mut objects = match preview.map(std::fs::read) {
        Some(Ok(bytes)) => Objects::from_bytes(bytes).expect("a preview pack"),
        _ => Objects::shipped(),
    };
    let size = if preview.is_some() { "preview" } else { "hero" };
    let mut turns = Vec::new();
    let names: Vec<String> = objects.pack().map(|p| p.names().map(str::to_owned).collect()).unwrap_or_default();
    for name in names {
        let s = if objects.px(&name, size).is_some() { size } else if name.starts_with("app-") { "l" } else { "m" };
        if let Some((file, w, h)) = turntable(dir, &mut objects, &name, s, default) {
            turns.push((file, name, w, h));
        }
    }
    let (mut shots, mut movies) = (Vec::new(), Vec::new());
    in_situ(&situ, &mut shots, &mut movies);
    let budget = budget(dir);
    print!("{budget}");
    html(dir, &sheets, &turns, &shots, &movies, &budget);
    println!("previews in {}", dir.join("index.html").display());
}
