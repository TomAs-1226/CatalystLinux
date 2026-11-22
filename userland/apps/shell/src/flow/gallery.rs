//! Flow's previews, to be looked at before it ships:
//! `catalyst-shell --flow-shots <dir>` writes stills, strips of motion (a
//! frame every 50 ms) and animated PNGs at the handheld's 25 ms beat, in
//! both looks and both modes, and an `index.html` showing them all.

use std::path::Path;

use crate::platform::{sim, Key};
use crate::robot::Robot;
use crate::shell::{HomeStyle, Shell};

const BEAT: f32 = 0.025;
const PW: usize = crate::gfx::W as usize;
const PH: usize = crate::gfx::H as usize;

/// A shell on Home in Flow, at rest, in the look given.
fn flow_shell(light: bool, bezel: bool, text: crate::gfx::TextSize, calm: bool) -> Shell {
    let mut shell = Shell::new(Robot::demo());
    let s = &mut shell.settings;
    s.home = HomeStyle::Flow;
    (s.light, s.calm, s.text) = (light, calm, text);
    if bezel {
        s.theme = crate::look::Theme::Bezel;
        (s.wallpaper, s.accent) = s.theme.defaults();
    }
    shell.apply_settings();
    film(&mut shell, 1.6);
    shell
}

/// Frames from now on, one every beat, for `seconds`.
fn film(shell: &mut Shell, seconds: f32) -> Vec<Vec<u32>> {
    (0..(seconds / BEAT).round() as usize)
        .map(|_| {
            shell.tick(BEAT);
            shell.draw();
            shell.gfx.canvas.px.clone()
        })
        .collect()
}

/// A key held for `seconds`, repeating as the handheld's keyboard does
/// (`platform::Repeat`: at 0.4 s, then every 75 ms), filmed every beat.
fn hold(shell: &mut Shell, key: Key, seconds: f32) -> Vec<Vec<u32>> {
    let mut frames = Vec::new();
    let mut next = 0.0f32;
    let mut t = 0.0f32;
    let mut first = true;
    while t < seconds {
        if t + 1e-4 >= next {
            shell.key(key);
            next += if first { 0.4 } else { 0.075 };
            first = false;
        }
        shell.tick(BEAT);
        shell.draw();
        frames.push(shell.gfx.canvas.px.clone());
        t += BEAT;
    }
    frames
}

fn still(shell: &Shell, dir: &Path, name: &str, shots: &mut Vec<(String, String)>, caption: &str) {
    sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    shots.push((format!("{name}.png"), caption.to_owned()));
}

/// Some of `frames` side by side, three to a row, and the whole run animated.
fn strip(frames: &[Vec<u32>], at: &[usize], dir: &Path, name: &str, clips: &mut Vec<(String, String, String)>, caption: &str) {
    const GAP: usize = 4;
    let cols = 3;
    let rows = at.len().div_ceil(cols);
    let (sw, sh) = (cols * PW + (cols - 1) * GAP, rows * PH + (rows - 1) * GAP);
    let mut sheet = bezel::Canvas::new(sw as i32, sh as i32);
    sheet.px.fill(0x0b0b0e);
    for (n, &i) in at.iter().enumerate() {
        let frame = &frames[i.min(frames.len() - 1)];
        let (ox, oy) = ((n % cols) * (PW + GAP), (n / cols) * (PH + GAP));
        for y in 0..PH {
            sheet.px[(oy + y) * sw + ox..][..PW].copy_from_slice(&frame[y * PW..][..PW]);
        }
    }
    // At the panel's own size (the page shows them enlarged, pixel for pixel):
    // small enough to keep beside the code.
    sim::save_png(&sheet, &dir.join(format!("{name}-strip.png")), 1);
    sim::save_apng(frames, PW, PH, &dir.join(format!("{name}.png")), 1, 25);
    clips.push((format!("{name}.png"), format!("{name}-strip.png"), caption.to_owned()));
}

fn steps_to(name: &str) -> usize {
    crate::screens::builtin_apps().position(|(_, _, n)| n == name).expect("a built-in app")
}

fn go(shell: &mut Shell, name: &str) {
    shell.key(Key::Back);
    film(shell, 1.2);
    for _ in 0..steps_to(name) {
        shell.key(Key::Right);
        film(shell, 0.2);
    }
    film(shell, 1.2);
}

pub fn shots(dir: &Path) {
    use crate::gfx::TextSize::{Compact, Large, Standard};
    use Key::*;
    std::fs::create_dir_all(dir).expect("create the preview folder");
    // Each frame as the handheld would show it: the cards painted in time.
    super::PAINT_HERE.store(true, std::sync::atomic::Ordering::Relaxed);
    // Whatever the keys change is kept in a folder of its own.
    let own = std::env::temp_dir().join(format!("catalyst-flow-shots-{}", std::process::id()));
    std::fs::create_dir_all(&own).ok();
    std::env::set_var("CATALYST_CONF", own.join("robot"));
    let (mut shots, mut clips) = (Vec::new(), Vec::new());

    // At rest, in a few colours.
    let mut shell = flow_shell(false, false, Standard, false);
    still(&shell, dir, "01-rest-robot", &mut shots, "At rest on Robot: its card faces you, the glow is its crimson");
    for (n, (name, caption)) in [("Weather", "Weather: a sky-blue glow"), ("Music", "Music"), ("Graphs", "Graphs"), ("Pit", "Pit")].into_iter().enumerate() {
        go(&mut shell, name);
        still(&shell, dir, &format!("0{}-rest-{}", n + 2, name.to_lowercase()), &mut shots, caption);
    }

    // One card on: a tap of Right.
    let mut shell = flow_shell(false, false, Standard, false);
    shell.key(Right);
    let frames = film(&mut shell, 0.8);
    strip(&frames, &[0, 2, 4, 6, 8, 10, 14, 20, 31], dir, "m1-one-card", &mut clips, "Right, tapped: the next card swings into the middle and lands like a dial on its detent");

    // Held: faster and faster, through every app, then let go.
    let mut shell = flow_shell(false, false, Standard, false);
    let mut frames = hold(&mut shell, Right, 1.2);
    let mid = frames.len() - 4;
    shell.gfx.canvas.px.copy_from_slice(&frames[mid]);
    still(&shell, dir, "06-mid-scrub", &mut shots, "Right held, a second in: flying through the apps");
    frames.extend(film(&mut shell, 0.9));
    let n = frames.len();
    strip(&frames, &[0, 16, 20, 24, 30, 36, 44, 50, n - 1], dir, "m2-held-scrub", &mut clips, "Right held for 1.2 s, then let go: it gathers speed, then glides to rest on a card");
    // Held to the end, and on against it: the rubber band.
    let mut frames = hold(&mut shell, Right, 1.0);
    frames.extend(film(&mut shell, 0.8));
    let n = frames.len();
    strip(&frames, &[0, 8, 16, 24, 32, 36, 40, 46, n - 1], dir, "m3-into-the-end", &mut clips, "Held into the last card and on against it: the flow gives a little and springs back");
    shell.key(Right);
    let frames = film(&mut shell, 0.15);
    shell.gfx.canvas.px.copy_from_slice(&frames[frames.len() - 1]);
    still(&shell, dir, "07-against-the-end", &mut shots, "Pushed against the last card: it gives, rubber-banded");
    film(&mut shell, 1.0);

    // Enter: the card comes forward and turns over; Enter again opens it.
    let mut shell = flow_shell(false, false, Standard, false);
    shell.key(Enter);
    let frames = film(&mut shell, 0.9);
    shell.gfx.canvas.px.copy_from_slice(&frames[7]);
    still(&shell, dir, "08-turning-over", &mut shots, "Enter: Robot's card comes forward and turns over");
    shell.gfx.canvas.px.copy_from_slice(&frames[frames.len() - 1]);
    still(&shell, dir, "09-enlarged-robot", &mut shots, "Turned over: what Robot is, what it does, and its battery now");
    strip(&frames, &[0, 3, 5, 7, 9, 11, 14, 20, 35], dir, "m4-enlarge", &mut clips, "Enter: the card comes forward, turns over and grows to show the app");
    shell.key(Enter);
    let frames = film(&mut shell, 0.9);
    shell.gfx.canvas.px.copy_from_slice(&frames[6]);
    still(&shell, dir, "10-launching", &mut shots, "Enter again: Robot opens out of the card");
    strip(&frames, &[0, 2, 4, 6, 8, 10, 13, 18, 35], dir, "m5-launch", &mut clips, "Enter on the turned card: the app opens out of it, the motion carried on");
    shell.key(Back);
    let frames = film(&mut shell, 1.4);
    strip(&frames, &[0, 4, 8, 12, 16, 20, 26, 34, 55], dir, "m6-back-from-the-app", &mut clips, "Esc in the app: it goes back into its card, which then turns back into the flow");

    // Esc from an enlarged card: back into the flow, which stays where it is.
    let mut shell = flow_shell(false, false, Standard, false);
    go(&mut shell, "Weather");
    shell.key(Enter);
    film(&mut shell, 0.9);
    still(&shell, dir, "11-enlarged-weather", &mut shots, "Weather turned over: the forecast's line, live");
    shell.key(Back);
    let frames = film(&mut shell, 0.9);
    strip(&frames, &[0, 3, 5, 7, 9, 12, 16, 22, 35], dir, "m7-esc", &mut clips, "Esc: the card turns back into the flow, which stays on Weather");

    // The other mode, the other look, the other text sizes, the calm version.
    let mut shell = flow_shell(true, false, Standard, false);
    still(&shell, dir, "20-light-rest", &mut shots, "Light mode, at rest");
    go(&mut shell, "Schedule");
    still(&shell, dir, "21-light-schedule", &mut shots, "Light mode, Schedule");
    shell.key(Enter);
    film(&mut shell, 0.9);
    still(&shell, dir, "22-light-enlarged", &mut shots, "Light mode, Schedule turned over: the next match");
    let mut shell = flow_shell(false, true, Standard, false);
    still(&shell, dir, "30-bezel-rest", &mut shots, "The Bezel look: names in its lowercase mono voice");
    go(&mut shell, "Monitor");
    shell.key(Enter);
    film(&mut shell, 0.9);
    still(&shell, dir, "31-bezel-enlarged", &mut shots, "The Bezel look, Monitor turned over");
    let mut shell = flow_shell(true, true, Standard, false);
    go(&mut shell, "Health");
    still(&shell, dir, "32-bezel-light", &mut shots, "The Bezel look in light mode, Health");
    for (size, name) in [(Compact, "compact"), (Large, "large")] {
        let mut shell = flow_shell(false, false, size, false);
        go(&mut shell, "Calculator");
        still(&shell, dir, &format!("4{}-{name}-calculator", if name == "compact" { 0 } else { 2 }), &mut shots, &format!("{name} text: Calculator, the longest name, whole"));
        shell.key(Enter);
        film(&mut shell, 0.9);
        still(&shell, dir, &format!("4{}-{name}-enlarged", if name == "compact" { 1 } else { 3 }), &mut shots, &format!("{name} text, turned over"));
    }
    let mut shell = flow_shell(false, false, Standard, true);
    go(&mut shell, "Files");
    still(&shell, dir, "50-calm", &mut shots, "The calm version: the same picture, nothing moving on its own");
    shell.key(Right);
    let frames = film(&mut shell, 0.3);
    strip(&frames, &[0, 1, 2, 4, 6, 8], dir, "m8-calm-step", &mut clips, "The calm version: Right lands at once, no overshoot");

    // Settings › Home: the three Homes side by side, Flow chosen.
    let mut shell = flow_shell(false, false, Standard, false);
    go(&mut shell, "Settings");
    shell.key(Enter);
    film(&mut shell, 1.0);
    shell.key(Enter);
    film(&mut shell, 1.0);
    for _ in 0..3 {
        shell.key(Down);
        film(&mut shell, 0.3);
    }
    shell.key(Enter);
    film(&mut shell, 1.0);
    still(&shell, dir, "60-settings-home", &mut shots, "Settings › Home: Grid, Carousel and Flow as pictures of themselves, Flow chosen");
    // The control centre and Spotlight over it.
    let mut shell = flow_shell(false, false, Standard, false);
    shell.key(Up);
    film(&mut shell, 1.0);
    still(&shell, dir, "61-control-centre", &mut shots, "Up: the control centre comes down over the flow");
    let mut shell = flow_shell(false, false, Standard, false);
    shell.key(Char(' '));
    film(&mut shell, 1.0);
    still(&shell, dir, "62-spotlight", &mut shots, "Space: Spotlight grows out of the island over it");
    // Arriving: the covers sweep in.
    let mut shell = Shell::new(Robot::demo());
    shell.settings.home = HomeStyle::Flow;
    shell.apply_settings();
    let frames = film(&mut shell, 0.9);
    strip(&frames, &[0, 2, 4, 6, 8, 10, 14, 20, 35], dir, "m9-arrive", &mut clips, "Home arriving in Flow: the covers sweep in from the right");

    index(dir, &shots, &clips);
    std::fs::remove_dir_all(&own).ok();
    println!("{} stills and {} clips in {}", shots.len(), clips.len(), dir.display());
}

/// What a frame of Flow costs on this computer, and what that comes to on
/// the handheld at the simulator's estimate (`--speed device`: its A53 20×
/// slower than one core here, 15 ms to send a whole frame to the panel).
pub fn bench() {
    use std::time::Instant;
    use Key::*;
    super::PAINT_HERE.store(true, std::sync::atomic::Ordering::Relaxed);
    let own = std::env::temp_dir().join(format!("catalyst-flow-bench-{}", std::process::id()));
    std::fs::create_dir_all(&own).ok();
    std::env::set_var("CATALYST_CONF", own.join("robot"));
    let per = |started: Instant, n: u32| started.elapsed().as_secs_f64() * 1e3 / n as f64;
    let mut shell = flow_shell(false, false, crate::gfx::TextSize::Standard, false);

    // Painting every card's picture (on a thread of its own, once, and again
    // only when the apps, the text size or the look change).
    let make = {
        let lettering = super::lettering(&shell.gfx);
        let specs: Vec<super::art::CardSpec> = crate::screens::builtin_apps().map(|(_, icon, name)| super::art::CardSpec { icon: icon.into(), name: name.into(), hue: super::hue_of(name, icon) }).collect();
        let rounds = 5;
        let started = Instant::now();
        for _ in 0..rounds {
            let mut text = bezel::Text::new();
            let mut icons = bezel::Icons::new();
            let mut atlas = super::art::Atlas::new(super::CARD, super::CARD, specs.len());
            for (i, spec) in specs.iter().enumerate() {
                atlas.put(i, &super::art::card(&mut text, &mut icons, spec, &lettering, super::CARD));
            }
        }
        per(started, rounds)
    };
    // Of which, for one card: its picture, and putting it in the atlas.
    {
        let lettering = super::lettering(&shell.gfx);
        let spec = super::art::CardSpec { icon: "smart_toy".into(), name: "Robot".into(), hue: 70 };
        let mut icons = bezel::Icons::new();
        let started = Instant::now();
        for _ in 0..20 {
            icons = bezel::Icons::new();
            super::art::card(&mut shell.gfx.text, &mut icons, &spec, &lettering, super::CARD);
        }
        let painted = per(started, 20);
        let px = super::art::card(&mut shell.gfx.text, &mut icons, &spec, &lettering, super::CARD);
        let mut atlas = super::art::Atlas::new(super::CARD, super::CARD, 37);
        let started = Instant::now();
        for _ in 0..20 {
            atlas.put(3, &px);
        }
        println!("  one card: painted {painted:.3} ms, put in the atlas {:.3} ms", per(started, 20));
    }

    // Every frame of a held run and its glide: the shell's whole frame
    // (the flow, its words, the island and the track's glass).
    let mut frames = 0u32;
    let mut spent = 0.0f64;
    let mut scenes = Vec::new();
    let (mut next, mut t, mut first) = (0.0f32, 0.0f32, true);
    while t < 2.6 {
        if t < 1.4 && t + 1e-4 >= next {
            shell.key(Right);
            next += if first { 0.4 } else { 0.075 };
            first = false;
        }
        let started = Instant::now();
        shell.tick(BEAT);
        shell.draw();
        spent += started.elapsed().as_secs_f64() * 1e3;
        frames += 1;
        t += BEAT;
        if frames % 10 == 0 {
            scenes.push(shell.gfx.canvas.px.clone());
        }
    }
    let frame = spent / frames as f64;

    // The flow alone, as the GPU would take it over: a scene mid-turn,
    // drawn again and again.
    let mut flow = super::Flow::new();
    let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
    let specs: Vec<super::art::CardSpec> = crate::screens::builtin_apps().map(|(_, icon, name)| super::art::CardSpec { icon: icon.into(), name: name.into(), hue: super::hue_of(name, icon) }).collect();
    // Painted on a thread of their own: wait for them.
    while !flow.make_cards(&mut g, &specs) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    flow.place(12);
    flow.pos.jump(12.45);
    let colours: Vec<bezel::Color> = specs.iter().map(|s| super::art::colours(s.hue).3).collect();
    let scene = flow.scene(&mut g, specs.len(), 12, &colours, 0.0);
    let cards = flow.cards.as_ref().unwrap();
    let n = 400;
    let started = Instant::now();
    for _ in 0..n {
        super::render::render(&mut g.canvas.px, &scene, [cards, cards]);
    }
    let alone = per(started, n);
    // Of which the ground (with nothing on it).
    let bare = super::Scene { cards: Vec::new(), ..scene.clone() };
    let started = Instant::now();
    for _ in 0..n {
        super::render::render(&mut g.canvas.px, &bare, [cards, cards]);
    }
    let ground_ms = per(started, n);
    let time = |scene: &super::Scene, g: &mut crate::gfx::Gfx| {
        let started = Instant::now();
        for _ in 0..n {
            super::render::render(&mut g.canvas.px, scene, [cards, cards]);
        }
        per(started, n)
    };
    let no_reflections = super::Scene { cards: scene.cards.iter().map(|c| super::Placed { reflect: 0.0, ..*c }).collect(), ..scene.clone() };
    let middle_only = super::Scene { cards: scene.cards.last().into_iter().copied().collect(), ..no_reflections.clone() };
    println!("  (bodies only {:.3} ms, the middle card only {:.3} ms)", time(&no_reflections, &mut g), time(&middle_only, &mut g));
    let (mut ground, mut cards_drawn) = (0usize, 0usize);
    let mut px = vec![0u32; PW * PH];
    super::render::background(&mut px, &scene);
    ground += PW * PH;
    for c in &scene.cards {
        if let Some((a, b, _)) = super::render::span(&scene, c, cards) {
            cards_drawn += ((b.min(320.0) - a.max(0.0)).max(0.0) * 100.0) as usize;
        }
    }

    let atlas_bytes = cards.px.len() * 4;
    let face_bytes = super::art::Atlas::new(super::FACE_W, super::FACE_H, 1).px.len() * 4;
    println!("Flow on this computer, one core:");
    println!("  every card's picture made ({} apps): {make:.1} ms", specs.len());
    println!("  a frame of the shell while the flow runs: {frame:.3} ms ({frames} frames)");
    println!("  the flow alone (ground, {} cards, reflections): {alone:.3} ms, of which the bare ground {ground_ms:.3} ms", scene.cards.len());
    println!("  pixels: ground {ground}, cards and reflections about {cards_drawn}");
    println!("On the handheld, at the simulator's estimate (A53 20× slower, panel 15 ms a frame):");
    println!("  a frame of the shell: {:.1} ms + 15 ms panel = {:.0} fps at most (the beat is 40)", frame * 20.0, (1000.0 / (frame * 20.0 + 15.0)).min(40.0));
    println!("  the flow alone on the processor: {:.1} ms; pictures made once: {:.0} ms", alone * 20.0, make * 20.0);
    println!("Memory: the cards' atlas {:.2} MB (and the same on the GPU when it draws), the back {:.2} MB", atlas_bytes as f64 / 1048576.0, face_bytes as f64 / 1048576.0);
    std::fs::remove_dir_all(&own).ok();
}

/// The page that shows them all, with paths relative to it.
fn index(dir: &Path, shots: &[(String, String)], clips: &[(String, String, String)]) {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Flow previews</title>\n<style>\n:root{--ground:#0b0c0e;--ink:#eef0f2;--dim:#9aa1a8;--card:#16181b}\nbody{margin:0;padding:24px 16px;background:var(--ground);color:var(--ink);font:15px/1.45 system-ui,sans-serif}\nh1{font-size:22px;margin:0 0 4px}h2{font-size:17px;margin:32px 0 12px}p{color:var(--dim);margin:0 0 16px;max-width:70ch}\n.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:16px}\nfigure{margin:0;background:var(--card);border-radius:12px;padding:10px}\nimg{width:100%;height:auto;image-rendering:pixelated;border-radius:6px;display:block}\nfigcaption{color:var(--dim);font-size:13px;margin-top:8px}\n.wide{grid-column:1/-1}\n</style></head><body>\n<h1>Flow · Home's third style</h1>\n<p>Rendered by the simulator (catalyst-shell --flow-shots): the panel's 320 × 170 pixels, enlarged. Motion clips play at the handheld's 25 ms beat; each strip shows frames of the same run.</p>\n<h2>Stills</h2>\n<div class=\"grid\">\n",
    );
    for (file, caption) in shots {
        html.push_str(&format!("<figure><img src=\"{file}\" alt=\"{0}\"><figcaption>{0}</figcaption></figure>\n", esc(caption)));
    }
    html.push_str("</div>\n<h2>Motion</h2>\n<div class=\"grid\">\n");
    for (anim, sheet, caption) in clips {
        html.push_str(&format!(
            "<figure><img src=\"{anim}\" alt=\"{0}\"><figcaption>{0}</figcaption></figure>\n<figure class=\"wide\"><img src=\"{sheet}\" alt=\"{0}, frames\"><figcaption>frames of the same, 50 ms and more apart</figcaption></figure>\n",
            esc(caption)
        ));
    }
    html.push_str("</div>\n</body></html>\n");
    std::fs::write(dir.join("index.html"), html).expect("write the index");
}
