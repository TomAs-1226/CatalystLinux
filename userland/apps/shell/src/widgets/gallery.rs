//! The widgets, to be looked at (`--widget-shots <dir>`, on the demo robot):
//!
//! - `sheet-<look>-<text>-<n>col-<k>.png`: every widget at each size, as the
//!   board `n` cells across draws it (Home's grid three across, the glance
//!   page and the large text size two), in both looks at each text size,
//!   and in light mode; five kinds to a sheet.
//! - `home-*.png`: Home as it shows them: the glance page, the grid with
//!   widgets among its apps, edit mode (as it starts, mid-move, mid-resize,
//!   a removal asked), the picker (open, and turning), Carousel's and
//!   Flow's glance page, and Settings › Home › Widgets.
//! - `timings.txt`: how long each widget takes to draw, and whole frames
//!   of Home with widgets in sight, here and as the handheld would take them.

use std::path::Path;

use bezel::{Canvas, Rect};

use super::{content, Ctx, Facts, Grid, Kind, Placed, Size};
use crate::gfx::{TextSize, H, W};
use crate::platform::Key;
use crate::shell::{HomeStyle, Shell};

/// What the widgets show in the gallery: a song with its cover, a timer
/// running, reminders coming, a note pinned, the demo photos.
pub fn fixtures() {
    crate::photos::seed_demo();
    let cover = crate::photos::list().get(1).map(|p| p.path.clone());
    crate::nowplaying::set(Some(crate::nowplaying::NowPlaying {
        title: "Around the World".into(),
        artist: "Daft Punk".into(),
        album: "Homework".into(),
        cover,
        position: 83.0,
        duration: 427.0,
        playing: true,
        at: std::time::Instant::now(),
    }));
    crate::nowplaying::on_control(|_| {});
    if crate::clock::timers_now().is_empty() {
        crate::clock::add_timer(272.0);
    }
    crate::reminders::pretend(&["charge the spares in 25 min", "pack the bumpers at 15:30", "check the battery cart tomorrow at 8am"]);
    let notes = crate::notes::dir();
    std::fs::create_dir_all(&notes).ok();
    if !notes.join("pit-list.txt").exists() {
        std::fs::write(notes.join("pit-list.txt"), "Pit list for Q31\nswap battery 3, it sagged to 6 v\nzip-tie the intake's cable\ncheck the follower's belt\nbring the spare bumpers\n").ok();
        crate::notes::toggle_pin(&notes, "pit-list.txt");
    }
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    fixtures();
    // Home's grid three across (two at the large size), and the glance page
    // two across, whose tiles are wider; the compact size's narrowest only.
    for look in ["catalyst", "bezel"] {
        for (text, columns) in [(TextSize::Compact, 3), (TextSize::Standard, 3), (TextSize::Standard, 2), (TextSize::Large, 2)] {
            sheets(dir, look, text, columns, false);
        }
    }
    sheets(dir, "bezel", TextSize::Standard, 3, true);
    sheets(dir, "catalyst", TextSize::Large, 2, true);
    screens(dir);
    timings(dir);
    println!("widgets in {}", dir.display());
}

/// The handheld takes about this many times longer over the same drawing
/// (the simulator's `--speed device`).
const HANDHELD: f64 = 20.0;

/// How long each widget takes to draw at each size, as the glance page
/// draws it: the median and the slowest of many draws once its words and
/// pictures are kept, on this computer and as the handheld would take
/// them; then whole frames of Home turning with widgets in sight.
/// Written to `timings.txt`.
fn timings(dir: &Path) {
    use std::time::{Duration, Instant};
    let mut shell = fresh("bezel", TextSize::Standard, false);
    let grid = Grid::of(&shell.gfx, 2);
    let mut facts = Facts::default();
    let h24 = shell.gfx.clock.0;
    let mut out = format!("widgets drawn, {} times longer on the handheld (µs: median, slowest of 200)\n\n", HANDHELD);
    let mut worst: (Duration, String) = (Duration::ZERO, String::new());
    for kind in Kind::ALL {
        for size in Size::ALL {
            let (w, h) = grid.size_of(size);
            facts.ready(kind, &shell.robot, h24, (w.round() as usize, h.round() as usize));
            let g = &mut shell.gfx;
            let r = Rect::new(8.0, 28.0, w, h);
            let mut times = Vec::with_capacity(200);
            for i in 0..210 {
                let ctx = Ctx { facts: &facts, robot: &shell.robot, pinned: Some("Loop/Robot/LastMs"), now: chrono::Local::now(), focused: false, radius: g.tile_radius(), laid: (w, h) };
                let at = Instant::now();
                g.surface(r, g.tile_radius(), g.tokens.surface1);
                g.canvas.set_clip(r);
                content(g, r, kind, size, &ctx);
                g.canvas.clear_clip();
                // The first few fill the caches.
                if i >= 10 {
                    times.push(at.elapsed());
                }
            }
            times.sort();
            let (median, slowest) = (times[times.len() / 2], times[times.len() - 1]);
            let name = format!("{} {}", kind.key(), size.key());
            out += &format!("{name:<22} {:>6} {:>6}   handheld {:>5.2} ms\n", median.as_micros(), slowest.as_micros(), median.as_secs_f64() * 1000.0 * HANDHELD);
            if median > worst.0 {
                worst = (median, name);
            }
        }
    }
    out += &format!("\nslowest: {} at {:.2} ms on the handheld\n", worst.1, worst.0.as_secs_f64() * 1000.0 * HANDHELD);
    // Whole frames: the glance page turning in, and the focus moving on
    // Home's grid with widgets among its apps, against the grid with none.
    let frames = |shell: &mut Shell, key: Key| -> (Duration, usize) {
        shell.key(key);
        let (mut total, mut n) = (Duration::ZERO, 0);
        while n < 90 {
            let moving = shell.tick(1.0 / 60.0);
            let at = Instant::now();
            shell.draw();
            total += at.elapsed();
            n += 1;
            if !moving && n > 4 {
                break;
            }
        }
        (total / n as u32, n)
    };
    let mut plain = fresh("bezel", TextSize::Standard, false);
    plain.settings.widgets.home.clear();
    plain.apply_settings();
    crate::fullheight::settle(&mut plain, 60);
    let mut rich_shell = fresh("bezel", TextSize::Standard, false);
    rich(&mut rich_shell);
    for (what, shell, key) in [("glance page turning in", &mut fresh("bezel", TextSize::Standard, false), Key::Left), ("grid, no widgets, the focus moving", &mut plain, Key::Right), ("grid with widgets, the focus moving", &mut rich_shell, Key::Right)] {
        let (each, n) = frames(shell, key);
        out += &format!("{what:<36} {:>6} µs a frame over {n} frames   handheld {:>5.2} ms\n", each.as_micros(), each.as_secs_f64() * 1000.0 * HANDHELD);
    }
    std::fs::write(dir.join("timings.txt"), &out).ok();
    print!("{out}");
}

/// A shell on the demo robot in a look and at a text size, settled on Home.
fn fresh(look: &str, text: TextSize, light: bool) -> Shell {
    let mut shell = crate::fullheight::shell(look, text);
    shell.settings.light = light;
    shell.apply_settings();
    crate::fullheight::settle(&mut shell, 30);
    shell
}

/// Every kind at each size, `columns` cells across, five kinds a sheet.
fn sheets(dir: &Path, look: &str, text: TextSize, columns: usize, light: bool) {
    let mut shell = fresh(look, text, light);
    let grid = Grid::of(&shell.gfx, columns);
    let (lw, lh) = grid.size_of(Size::Large);
    let (cell_w, cell_h) = (lw.ceil() as i32 + 12, lh.ceil() as i32 + 12);
    let label_w = 132;
    let mut facts = Facts::default();
    let h24 = shell.gfx.clock.0;
    for (k, kinds) in Kind::ALL.chunks(5).enumerate() {
        let (sw, sh) = (label_w + 3 * cell_w + 6, kinds.len() as i32 * cell_h + 6);
        let mut sheet = Canvas::new(sw, sh);
        let ground = shell.gfx.tokens.ground;
        sheet.clear(ground);
        for (row, kind) in kinds.iter().enumerate() {
            let y0 = 3 + row as i32 * cell_h;
            let ty = shell.gfx.ty;
            let (ink, dim) = (shell.gfx.tokens.ink, shell.gfx.tokens.dim);
            shell.gfx.text.draw(&mut sheet, 6.0, y0 as f32 + 18.0, &kind.name().to_lowercase(), &ty.caption, ink, 1.0);
            // What Enter does, on as many lines as the column needs.
            for (i, line) in shell.gfx.text.wrap(kind.enter_does(), &ty.caption, label_w as f32 - 12.0, true).iter().enumerate() {
                shell.gfx.text.draw(&mut sheet, 6.0, y0 as f32 + 34.0 + i as f32 * (ty.caption.size + 4.0), line, &ty.caption, dim, 0.8);
            }
            for (col, size) in Size::ALL.iter().enumerate() {
                let (w, h) = grid.size_of(*size);
                facts.ready(*kind, &shell.robot, h24, (w.round() as usize, h.round() as usize));
                let g = &mut shell.gfx;
                g.canvas.clear(g.tokens.ground);
                let r = Rect::new(6.0, 6.0, w, h);
                g.surface(r, g.tile_radius(), g.tokens.surface1);
                let pinned = Some("Loop/Robot/LastMs");
                let ctx = Ctx { facts: &facts, robot: &shell.robot, pinned, now: chrono::Local::now(), focused: false, radius: g.tile_radius(), laid: (w, h) };
                g.canvas.set_clip(r);
                content(g, r, *kind, *size, &ctx);
                g.canvas.clear_clip();
                // Into its place on the sheet.
                let (x0, y) = (label_w + col as i32 * cell_w, y0);
                let (cw, ch) = ((w.ceil() as i32 + 12).min(W as i32), (h.ceil() as i32 + 12).min(H as i32));
                for dy in 0..ch {
                    for dx in 0..cw {
                        let (sx, sy) = (x0 + dx, y + dy);
                        if sx < sw && sy < sh {
                            sheet.px[(sy * sw + sx) as usize] = g.canvas.px[(dy * g.canvas.w + dx) as usize];
                        }
                    }
                }
            }
        }
        let name = format!("sheet-{look}{}-{}-{columns}col-{}.png", if light { "-light" } else { "" }, text.name(), k + 1);
        crate::platform::sim::save_png(&sheet, &dir.join(name), 2);
    }
}

/// Keys pressed one by one, the shell settling `frames` after each.
fn press(shell: &mut Shell, keys: &[Key], frames: usize) {
    for key in keys {
        shell.key(*key);
        crate::fullheight::settle(shell, frames);
    }
}

/// A few frames on, part way through whatever just began.
fn partway(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        shell.tick(1.0 / 60.0);
        shell.draw();
    }
}

fn save(shell: &mut Shell, dir: &Path, name: &str) {
    // What the widgets read on their thread has come in by now.
    shell.draw();
    std::thread::sleep(std::time::Duration::from_millis(40));
    shell.draw();
    crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
}

/// A layout to show: a full glance page, and widgets among Home's apps.
fn rich(shell: &mut Shell) {
    let w = Placed::new;
    shell.settings.widgets.glance = vec![
        w(Kind::NextMatch, Size::Small, 0),
        w(Kind::Battery, Size::Small, 0),
        w(Kind::Music, Size::Wide, 0),
        w(Kind::Weather, Size::Large, 0),
        w(Kind::Reminders, Size::Wide, 0),
        w(Kind::Timers, Size::Small, 0),
        w(Kind::Clock, Size::Small, 0),
    ];
    shell.settings.widgets.home = vec![w(Kind::Clock, Size::Small, 1), w(Kind::Music, Size::Wide, 3), w(Kind::Photos, Size::Large, 6), w(Kind::System, Size::Wide, 9), w(Kind::World, Size::Wide, 12)];
    shell.apply_settings();
    crate::fullheight::settle(shell, 60);
}

fn screens(dir: &Path) {
    use Key::*;
    // The glance page as a new handheld has it: its first page and the next,
    // in both looks, at the large text size, and compact in light mode.
    for (look, text, light) in [("catalyst", TextSize::Standard, false), ("bezel", TextSize::Standard, false), ("catalyst", TextSize::Large, false), ("bezel", TextSize::Compact, true)] {
        let mut shell = fresh(look, text, light);
        press(&mut shell, &[Left], 60);
        let name = format!("{look}-{}{}", text.name(), if light { "-light" } else { "" });
        save(&mut shell, dir, &format!("home-glance-start-{name}"));
        press(&mut shell, &[Left, Left], 60);
        save(&mut shell, dir, &format!("home-glance-start-2-{name}"));
    }
    // A fuller glance page: each of its pages, and half way between two.
    for look in ["catalyst", "bezel"] {
        let mut shell = fresh(look, TextSize::Standard, false);
        rich(&mut shell);
        press(&mut shell, &[Left], 60);
        save(&mut shell, dir, &format!("home-glance-1-{look}"));
        press(&mut shell, &[Left, Left], 60);
        save(&mut shell, dir, &format!("home-glance-2-{look}"));
        press(&mut shell, &[Left], 60);
        save(&mut shell, dir, &format!("home-glance-3-{look}"));
        shell.key(Right);
        partway(&mut shell, 7);
        save(&mut shell, dir, &format!("home-glance-turning-{look}"));
    }
    // Home's grid with widgets among the apps: its first page, a widget in
    // focus, and the next page; in light mode too.
    for (look, text, light) in [("catalyst", TextSize::Standard, false), ("bezel", TextSize::Standard, false), ("catalyst", TextSize::Large, false), ("bezel", TextSize::Standard, true)] {
        let mut shell = fresh(look, text, light);
        rich(&mut shell);
        let name = format!("{look}-{}{}", text.name(), if light { "-light" } else { "" });
        save(&mut shell, dir, &format!("home-grid-1-{name}"));
        press(&mut shell, &[Right], 60);
        save(&mut shell, dir, &format!("home-grid-focus-widget-{name}"));
        if !light {
            press(&mut shell, &[Right; 8], 40);
            save(&mut shell, dir, &format!("home-grid-2-{name}"));
        }
    }
    // Edit mode on the grid, in Bezel: as it starts (the tiles jiggle), a
    // move under way and done, down, a resize under way and done, a removal
    // asked; the picker opening, turning, a size up, added; done.
    let mut shell = fresh("bezel", TextSize::Standard, false);
    rich(&mut shell);
    press(&mut shell, &[Right], 60);
    shell.key(Char('e'));
    partway(&mut shell, 5);
    save(&mut shell, dir, "home-edit-start");
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-edit");
    shell.key(Right);
    partway(&mut shell, 6);
    save(&mut shell, dir, "home-edit-mid-move");
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-edit-moved");
    shell.key(Down);
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-edit-moved-down");
    shell.key(Char('r'));
    partway(&mut shell, 6);
    save(&mut shell, dir, "home-edit-mid-resize");
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-edit-resized");
    shell.key(Char('d'));
    crate::fullheight::settle(&mut shell, 20);
    save(&mut shell, dir, "home-edit-remove-asked");
    shell.key(Back);
    crate::fullheight::settle(&mut shell, 60);
    shell.key(Char('e'));
    crate::fullheight::settle(&mut shell, 30);
    shell.key(Char('a'));
    partway(&mut shell, 6);
    save(&mut shell, dir, "home-picker-opening");
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-picker");
    shell.key(Right);
    partway(&mut shell, 5);
    save(&mut shell, dir, "home-picker-turning");
    crate::fullheight::settle(&mut shell, 60);
    shell.key(Up);
    crate::fullheight::settle(&mut shell, 60);
    save(&mut shell, dir, "home-picker-wide");
    press(&mut shell, &[Enter], 60);
    save(&mut shell, dir, "home-picker-added");
    press(&mut shell, &[Enter], 60);
    save(&mut shell, dir, "home-edit-done");
    // The same in Catalyst's look: editing, and the picker.
    let mut shell = fresh("catalyst", TextSize::Standard, false);
    rich(&mut shell);
    press(&mut shell, &[Right, Char('e')], 60);
    save(&mut shell, dir, "home-edit-catalyst");
    press(&mut shell, &[Char('a')], 60);
    save(&mut shell, dir, "home-picker-catalyst");
    // Edit mode on the glance page: a move down, and the picker; at the
    // large text size too.
    for (look, text) in [("bezel", TextSize::Standard), ("catalyst", TextSize::Large)] {
        let mut shell = fresh(look, text, false);
        let name = format!("{look}-{}", text.name());
        press(&mut shell, &[Left], 60);
        press(&mut shell, &[Char('e')], 60);
        save(&mut shell, dir, &format!("home-glance-edit-{name}"));
        shell.key(Down);
        partway(&mut shell, 6);
        save(&mut shell, dir, &format!("home-glance-edit-mid-move-{name}"));
        crate::fullheight::settle(&mut shell, 60);
        save(&mut shell, dir, &format!("home-glance-edit-moved-{name}"));
        press(&mut shell, &[Char('a')], 60);
        save(&mut shell, dir, &format!("home-glance-picker-{name}"));
    }
    // Carousel's and Flow's glance page, and Flow half way to it.
    for style in [HomeStyle::Carousel, HomeStyle::Flow] {
        let mut shell = fresh("catalyst", TextSize::Standard, false);
        shell.settings.home = style;
        shell.apply_settings();
        crate::fullheight::settle(&mut shell, 90);
        if style == HomeStyle::Flow {
            shell.key(Left);
            partway(&mut shell, 7);
            save(&mut shell, dir, "home-glance-flow-turning");
            crate::fullheight::settle(&mut shell, 80);
        } else {
            press(&mut shell, &[Left], 80);
        }
        save(&mut shell, dir, &format!("home-glance-{}", style.name()));
    }
    // Settings › Home › Widgets.
    let mut shell = fresh("catalyst", TextSize::Standard, false);
    let (index, ..) = crate::screens::setting_sections().into_iter().find(|(_, n, _)| *n == "Widgets").expect("Settings › Home › Widgets");
    shell.show(crate::screens::open_setting(index));
    crate::fullheight::settle(&mut shell, 90);
    save(&mut shell, dir, "home-settings-widgets");
}
