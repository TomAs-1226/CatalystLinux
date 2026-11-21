//! The 3D objects laid out to be looked at and approved before they ship:
//! `catalyst-shell --objects DIR [--preview-pack PATH] [--all-pack PATH]`.
//!
//! Everything here is drawn by the shell's own code (`object3d`,
//! `robot3d`), so what is seen is what the handheld draws: contact sheets of
//! the objects the apps show, in dark and light and in the states they
//! show (the robot disabled, in auto, a motor hot, a fault, no link; the
//! battery's charge; the weather; Wi-Fi's signal), turntables, the objects
//! in place on the panel at its own size and three times it (dithered to
//! 16 bits as the panel shows them), their motion as animated PNGs, what
//! they cost, and `index.html` showing all of it.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use bezel::{Accent, Align, Canvas, Color, Rect, TextStyle, Tokens, Tone};
use nt4::Value;

use crate::gfx::{Gfx, H, W};
use crate::object3d::{Looks, Objects, View};
use crate::platform::{self, Key};
use crate::robot::Robot;
use crate::robot3d::{self, State};
use crate::shell::Shell;

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

/// Every object made, shipped or not, when that pack is given.
static ALL: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// A Gfx drawing into a canvas of its own size.
fn sheet_gfx(w: i32, h: i32, t: Tokens) -> Gfx {
    let mut g = Gfx::new(t);
    if let Some(all) = ALL.get() {
        g.objects = Objects::from_bytes(all.clone()).expect("the pack of every object");
    }
    g.canvas = Canvas::new(w, h);
    g.canvas.clear(t.ground);
    g
}

/// The robot's states, as the demo robot is told to report them.
#[derive(Clone, Copy)]
enum Story {
    Teleop,
    Disabled,
    Auto,
    Hot,
    Fault,
    LowBattery,
    Red,
    NoLink,
}

impl Story {
    const ALL: [Story; 8] = [Story::Teleop, Story::Disabled, Story::Auto, Story::Hot, Story::Fault, Story::LowBattery, Story::Red, Story::NoLink];

    fn name(self) -> &'static str {
        match self {
            Story::Teleop => "teleop, all well",
            Story::Disabled => "disabled",
            Story::Auto => "auto",
            Story::Hot => "elevator hot",
            Story::Fault => "intake fault",
            Story::LowBattery => "battery low",
            Story::Red => "red alliance",
            Story::NoLink => "no link",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Story::Teleop => "teleop",
            Story::Disabled => "disabled",
            Story::Auto => "auto",
            Story::Hot => "hot-motor",
            Story::Fault => "fault",
            Story::LowBattery => "low-battery",
            Story::Red => "red-alliance",
            Story::NoLink => "no-link",
        }
    }

    /// The demo robot, quiet (nothing firing, nothing hot), then told this.
    fn robot(self) -> Robot {
        if let Story::NoLink = self {
            // A robot set, its address not answering.
            return Robot::connect("127.0.0.1:9");
        }
        let mut r = Robot::demo();
        r.refresh(0.1);
        let quiet: [(&str, Value); 4] = [
            ("Health/CAN/can_s0/HighUtilization/firing", Value::Bool(false)),
            ("Health/Elevator/HighTemp/firing", Value::Bool(false)),
            ("Elevator/TemperatureC", Value::Double(52.0)),
            ("Health/History", Value::StringArray(vec![])),
        ];
        for (k, v) in quiet {
            r.demo_say(k, v);
        }
        let say: Vec<(&str, Value)> = match self {
            Story::Disabled => vec![("FMSInfo/FMSControlData", Value::Int(32))],
            Story::Auto => vec![("FMSInfo/FMSControlData", Value::Int(1 | 2 | 32))],
            Story::Hot => vec![("Elevator/TemperatureC", Value::Double(79.0)), ("Health/Elevator/HighTemp/firing", Value::Bool(true)), ("Health/Elevator/HighTemp/detail", Value::String("79 °C, limit 70".into()))],
            Story::Fault => vec![
                ("Health/Intake/OverCurrent/firing", Value::Bool(true)),
                ("Health/Intake/OverCurrent/severity", Value::String("ERROR".into())),
                ("Health/Intake/OverCurrent/detail", Value::String("stalled, 42 A".into())),
            ],
            Story::LowBattery => vec![("Systemcore/BatteryVolts", Value::Double(10.8))],
            Story::Red => vec![("FMSInfo/IsRedAlliance", Value::Bool(true))],
            _ => vec![],
        };
        for (k, v) in say {
            r.demo_say(k, v);
        }
        r
    }
}

fn robot_state(g: &Gfx, story: Story) -> State {
    let parts: Vec<String> = g.objects.parts(robot3d::GENERIC).into_iter().map(|(n, _)| n).collect();
    robot3d::read(&story.robot(), &parts)
}

/// A sheet of cells, each an object drawn by `draw`, captioned.
#[allow(clippy::type_complexity)]
fn sheet(dir: &Path, file: &str, tone: Tone, accent: Accent, cell: (i32, i32), across: usize, cells: &[(String, Box<dyn Fn(&mut Gfx, Rect)>)]) -> String {
    let t = Tokens::of(accent, tone);
    let rows = cells.len().div_ceil(across);
    let (cw, ch) = cell;
    let mut g = sheet_gfx(across as i32 * cw + 8, rows as i32 * ch + 8, t);
    for (i, (caption, draw)) in cells.iter().enumerate() {
        let (x, y) = (8.0 + (i % across) as f32 * cw as f32, 8.0 + (i / across) as f32 * ch as f32);
        let tile = Rect::new(x, y, cw as f32 - 8.0, ch as f32 - 8.0);
        g.canvas.rrect(tile, 12.0, t.surface1, 1.0);
        draw(&mut g, tile);
        let caption = g.text.fit(caption, &TextStyle::label(11.0), tile.w - 8.0);
        g.text_at(tile.cx(), tile.bottom() - 9.0, Align::Center, &caption, TextStyle::label(11.0), t.dim);
    }
    let name = format!("{file}.png");
    platform::sim::save_png(&g.canvas, &dir.join(&name), 2);
    name
}

type Cell = (String, Box<dyn Fn(&mut Gfx, Rect)>);

fn object_cell(caption: &str, object: &'static str, size: &'static str, looks: Looks) -> Cell {
    (caption.to_owned(), Box::new(move |g: &mut Gfx, r: Rect| {
        g.object_looking(object, size, r.cx(), r.cy() - 8.0, View::REST, 1.0, 1.0, &looks);
    }))
}

fn sheets(dir: &Path) -> Vec<(String, String)> {
    let default = crate::shell::Settings::default().accent;
    let mut out = Vec::new();
    for (tone_name, tone) in [("dark", Tone::Dark), ("light", Tone::Light)] {
        // The robot, in each state it shows.
        let cells: Vec<Cell> = Story::ALL
            .iter()
            .map(|&story| {
                let caption = story.name().to_owned();
                (caption, Box::new(move |g: &mut Gfx, r: Rect| {
                    let state = robot_state(g, story);
                    let looks = state.looks(g);
                    g.object_looking(robot3d::GENERIC, "hero", r.cx(), r.cy() - 8.0, View::REST, 1.0, if state.linked { 1.0 } else { 0.75 }, &looks);
                }) as Box<dyn Fn(&mut Gfx, Rect)>)
            })
            .collect();
        out.push((sheet(dir, &format!("sheet-robot-{tone_name}"), tone, default, (150, 150), 4, &cells), format!("The Robot app's robot in each state it shows, {tone_name}: parts glow amber when hot, red on a fault, faint ice at work; bumpers in the alliance's colour; dimmed when disabled, greyed with no link")));
        // The weather.
        let weather = [
            ("sunny", "weather-sunny"),
            ("clear night", "weather-clear-night"),
            ("sun, cloud", "weather-partly-day"),
            ("moon, cloud", "weather-partly-night"),
            ("cloudy", "weather-cloudy"),
            ("rain", "weather-rain"),
            ("snow", "weather-snow"),
            ("sleet", "weather-sleet"),
            ("storm", "weather-storm"),
            ("fog", "weather-fog"),
        ];
        let cells: Vec<Cell> = weather.iter().map(|(c, o)| object_cell(c, o, "l", Looks::plain())).collect();
        out.push((sheet(dir, &format!("sheet-weather-{tone_name}"), tone, default, (96, 92), 5, &cells), format!("Weather's card, each condition, {tone_name}")));
        // The battery's charge, and Wi-Fi's signal.
        let mut cells: Vec<Cell> = [(None, "no gauge"), (Some((8, false)), "8%"), (Some((35, false)), "35%"), (Some((62, false)), "62%"), (Some((100, false)), "100%"), (Some((70, true)), "charging")]
            .into_iter()
            .map(|(charge, c)| {
                (c.to_owned(), Box::new(move |g: &mut Gfx, r: Rect| {
                    let looks = crate::power::battery_looks(g, charge);
                    g.object_looking("battery", "m", r.cx(), r.cy() - 8.0, View::REST, 1.0, 1.0, &looks);
                }) as Box<dyn Fn(&mut Gfx, Rect)>)
            })
            .collect();
        for (arcs, c) in [(None, "wi-fi off"), (Some(0), "looking"), (Some(1), "weak"), (Some(3), "strong")] {
            cells.push(object_cell(c, "wifi", "m", crate::object3d::wifi_looks(arcs)));
        }
        out.push((sheet(dir, &format!("sheet-battery-wifi-{tone_name}"), tone, default, (96, 96), 5, &cells), format!("Settings › Battery's battery by its charge, and Wi-Fi by its signal, {tone_name}")));
        // Empty pages, and the other objects.
        let empty = [
            ("Files", "app-files"),
            ("Logs", "app-logs"),
            ("Schedule", "app-schedule"),
            ("Tables, Git", "app-tables"),
            ("Motors", "app-motors"),
            ("Console", "app-terminal"),
            ("Catalogue", "app-catalogue"),
            ("Net tools", "app-nettools"),
            ("Photos", "app-photos"),
            ("Graphs", "app-graphs"),
            ("Pit", "app-pit"),
            ("Assistant", "app-assistant"),
        ];
        let mut cells: Vec<Cell> = empty.iter().map(|(c, o)| object_cell(c, o, "l", Looks::plain())).collect();
        cells.push(object_cell("no robot", robot3d::GENERIC, "m", Looks { dim: 0.8, ..Looks::plain() }));
        cells.push(object_cell("no key", "key", "m", Looks::plain()));
        cells.push(object_cell("no sensor", "app-level", "l", Looks { dim: 0.6, ..Looks::plain() }));
        cells.push(object_cell("no history", "tray", "m", Looks::plain()));
        out.push((sheet(dir, &format!("sheet-empty-{tone_name}"), tone, default, (96, 92), 6, &cells), format!("Empty pages, {tone_name}: the app's own object, or the robot greyed where there is no robot")));
        let things = [
            ("handheld", "handheld", "hero"),
            ("laptop", "laptop", "hero"),
            ("wi-fi", "wifi", "hero"),
            ("team robot (toy)", "robot", "hero"),
            ("globe", "globe", "hero"),
            ("key", "key", "hero"),
            ("switch", "toggle", "hero"),
            ("battery", "battery", "hero"),
            ("trophy", "trophy", "hero"),
            ("moon (not used)", "moon", "hero"),
            ("rocket (not used)", "rocket", "hero"),
            ("chip (not used)", "chip", "hero"),
        ];
        let cells: Vec<Cell> = things.iter().map(|(c, o, s)| object_cell(c, o, s, Looks::plain())).collect();
        out.push((sheet(dir, &format!("sheet-objects-{tone_name}"), tone, default, (130, 130), 4, &cells), format!("Setup's objects and the rest, {tone_name}")));
    }
    // The user's own colour: the robot and the weather follow it.
    let cells: Vec<Cell> = [Story::Teleop, Story::Hot, Story::Disabled]
        .iter()
        .map(|&story| {
            (story.name().to_owned(), Box::new(move |g: &mut Gfx, r: Rect| {
                let looks = robot_state(g, story).looks(g);
                g.object_looking(robot3d::GENERIC, "hero", r.cx(), r.cy() - 8.0, View::REST, 1.0, 1.0, &looks);
            }) as Box<dyn Fn(&mut Gfx, Rect)>)
        })
        .chain([object_cell("sunny", "weather-sunny", "l", Looks::plain()), object_cell("rain", "weather-rain", "l", Looks::plain())])
        .collect();
    out.push((sheet(dir, "sheet-own-colour", Tone::Dark, Accent::Hue(36), (150, 150), 5, &cells), "The same in the user's own colour (a teal): the robot's own accent parts and the weather follow it".into()));
    out
}

/// An object turning all the way round, in each look given, side by side.
fn turntable(dir: &Path, objects: &mut Objects, file: &str, object: &str, size: &str, cells: &[(Tone, Looks, f32)]) -> Option<(String, i32, i32)> {
    let px = objects.px(object, size)? as i32;
    let cell = px + 16;
    let (w, h) = (cell * cells.len() as i32, cell);
    let mut frames = Vec::new();
    // Each rendered angle once, 15° apart: what was rendered, nothing blended.
    for step in 0..24 {
        let yaw = step as f32 * 15.0;
        let mut c = Canvas::new(w, h);
        for (k, (tone, looks, alpha)) in cells.iter().enumerate() {
            let t = Tokens::of(crate::shell::Settings::default().accent, *tone);
            c.fill_rect(Rect::new((k as i32 * cell) as f32, 0.0, cell as f32, cell as f32), t.surface1, 1.0);
            let fault = if *tone == Tone::Dark { Color::hex(0xff6b5e) } else { Color::hex(0xb3261e) };
            objects.draw_looking(&mut c, &t, fault, object, size, (k as i32 * cell + cell / 2) as f32, cell as f32 * 0.5, View::yaw(yaw), 1.0, *alpha, looks);
        }
        frames.push(c.px);
    }
    let name = format!("turn-{file}.png");
    save_apng(&frames, w as usize, h as usize, &dir.join(&name), 70);
    Some((name, w, h))
}

fn turntables(dir: &Path, preview: Option<&Path>) -> Vec<(String, String, i32, i32)> {
    let mut objects = match preview.map(std::fs::read) {
        Some(Ok(bytes)) => Objects::from_bytes(bytes).expect("a preview pack"),
        _ => Objects::shipped(),
    };
    let size = |o: &Objects, name: &str| if o.px(name, "preview").is_some() { "preview" } else if o.px(name, "hero").is_some() { "hero" } else { "l" };
    let mut out = Vec::new();
    // The robot in four states at once.
    let g = Gfx::new(Tokens::dark());
    let states: Vec<(Tone, Looks, f32)> = [Story::Teleop, Story::Hot, Story::Disabled, Story::NoLink]
        .iter()
        .map(|&s| {
            let st = robot_state(&g, s);
            (Tone::Dark, st.looks(&g), if st.linked { 1.0 } else { 0.75 })
        })
        .collect();
    let s = size(&objects, robot3d::GENERIC);
    if let Some((f, w, h)) = turntable(dir, &mut objects, "robot-states", robot3d::GENERIC, s, &states) {
        out.push((f, "the robot: teleop · elevator hot · disabled · no link".into(), w, h));
    }
    let both = |looks: Looks| vec![(Tone::Dark, looks.clone(), 1.0), (Tone::Light, looks, 1.0)];
    let list: Vec<(&str, Looks)> = vec![
        ("robot-generic", Looks::plain()),
        ("battery", crate::power::battery_looks(&g, Some((62, false)))),
        ("wifi", crate::object3d::wifi_looks(Some(2))),
        ("handheld", Looks::plain()),
        ("laptop", Looks::plain()),
        ("robot", Looks::plain()),
        ("globe", Looks::plain()),
        ("key", Looks::plain()),
        ("toggle", Looks::plain()),
        ("trophy", Looks::plain()),
        ("moon", Looks::plain()),
        ("weather-sunny", Looks::plain()),
        ("weather-partly-day", Looks::plain()),
        ("weather-rain", Looks::plain()),
        ("weather-snow", Looks::plain()),
        ("weather-storm", Looks::plain()),
        ("weather-clear-night", Looks::plain()),
    ];
    for (name, looks) in list {
        let s = size(&objects, name);
        let cells = if name.starts_with("weather-") { vec![(Tone::Dark, looks, 1.0)] } else { both(looks) };
        if let Some((f, w, h)) = turntable(dir, &mut objects, name, name, s, &cells) {
            out.push((f, name.to_owned(), w, h));
        }
    }
    out
}

/// The shell, driven by keys, its frames kept.
struct Run {
    shell: Shell,
}

impl Run {
    fn with(robot: Robot, light: bool, calm: bool) -> Run {
        let mut shell = Shell::new(robot);
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
    /// Open a built-in app from Home's first tile.
    fn open(&mut self, app: &str) {
        self.keys(&vec![Key::Right; crate::steps_to(app)], 0.08);
        self.settle(0.6);
        self.shell.key(Key::Enter);
        self.settle(1.2);
    }
    /// Open one of Settings' own pages, by its row.
    fn settings(&mut self, row: &str) {
        self.open("Settings");
        self.keys(&vec![Key::Down; crate::screens::settings_row(row)], 0.08);
        self.shell.key(Key::Enter);
        self.settle(1.2);
    }
}

fn movie(dir: &Path, name: &str, frames: &[Vec<u32>], movies: &mut Vec<String>) {
    save_apng(frames, W as usize, H as usize, &dir.join(format!("{name}.png")), 25);
    movies.push(name.to_owned());
}

/// The objects in place, still and moving.
fn in_situ(dir: &Path, shots: &mut Vec<String>, movies: &mut Vec<String>) {
    use Key::*;
    // The Robot app, in each state.
    for story in Story::ALL {
        let mut run = Run::with(story.robot(), false, false);
        run.open("Robot");
        run.settle(2.2);
        run.still(dir, &format!("robot-{}", story.file()), shots);
    }
    let mut run = Run::with(Story::Teleop.robot(), true, false);
    run.open("Robot");
    run.settle(1.0);
    run.still(dir, "robot-teleop-light", shots);
    let mut run = Run::with(Story::Hot.robot(), false, true);
    run.open("Robot");
    run.still(dir, "robot-hot-motor-calm", shots);
    // Motion: at rest it turns slowly; a motor running hot, it turns to show it.
    let mut run = Run::with(Story::Teleop.robot(), false, false);
    run.open("Robot");
    let f = run.frames(3.0);
    movie(dir, "motion-robot-turning", &f, movies);
    run.shell.robot.demo_say("Elevator/TemperatureC", Value::Double(79.0));
    run.shell.robot.demo_say("Health/Elevator/HighTemp/firing", Value::Bool(true));
    run.shell.robot.demo_say("Health/Elevator/HighTemp/detail", Value::String("79 °C, limit 70".into()));
    let f = run.frames(2.5);
    movie(dir, "motion-robot-motor-hot", &f, movies);
    run.still(dir, "robot-turned-to-the-elevator", shots);
    // Enabled: from disabled to teleop, the robot brightens.
    let mut run = Run::with(Story::Disabled.robot(), false, false);
    run.open("Robot");
    run.settle(0.5);
    run.shell.robot.demo_say("FMSInfo/FMSControlData", Value::Int(1 | 32));
    let f = run.frames(1.2);
    movie(dir, "motion-robot-enabled", &f, movies);

    // Health with nothing firing: the robot, all well; with no robot, greyed.
    let mut run = Run::with(Story::Teleop.robot(), false, false);
    run.open("Health");
    run.still(dir, "health-all-well", shots);
    let mut run = Run::with(Robot::none(), false, false);
    run.open("Tables");
    run.still(dir, "tables-no-robot", shots);

    // Weather: the condition, swinging in as the page arrives.
    let mut run = Run::with(Robot::demo(), false, false);
    run.keys(&vec![Right; crate::steps_to("Weather")], 0.08);
    run.settle(0.6);
    run.shell.key(Enter);
    let f = run.frames(1.0);
    movie(dir, "motion-weather-arrives", &f, movies);
    run.still(dir, "weather", shots);
    let mut run = Run::with(Robot::demo(), true, false);
    run.open("Weather");
    run.still(dir, "weather-light", shots);

    // Settings › Battery: the charge in cells, the bolt lit when charging.
    let mut run = Run::with(Robot::demo(), false, false);
    run.settings("Battery");
    run.still(dir, "battery", shots);
    let mut run = Run::with(Robot::demo(), true, false);
    run.settings("Battery");
    run.still(dir, "battery-light", shots);

    // Empty pages.
    let mut run = Run::with(Robot::demo(), false, false);
    run.open("Graphs");
    run.still(dir, "empty-graphs", shots);
    let mut run = Run::with(Robot::demo(), false, false);
    run.open("Browse");
    run.still(dir, "empty-browse-no-key", shots);

    // Setup's Wi-Fi: its arcs dark until there is a network.
    let mut run = Run::with(Robot::demo(), false, false);
    run.shell.start_setup();
    run.settle(1.0);
    run.shell.key(Enter);
    run.settle(1.0);
    run.still(dir, "setup-wifi", shots);
    run.keys(&[Tab, Tab, Tab], 0.9);
    run.still(dir, "setup-team", shots);
    // About: the handheld in its header.
    let mut run = Run::with(Robot::demo(), false, false);
    let about = crate::screens::settings_row("About this handheld");
    run.open("Settings");
    run.keys(&vec![Down; about], 0.08);
    run.keys(&[Enter], 1.2);
    run.still(dir, "about-header", shots);
    // Home's icons stay flat.
    let mut run = Run::with(Robot::demo(), false, false);
    run.still(dir, "home-flat", shots);
}

/// What the objects cost, measured.
fn budget(dir: &Path) -> String {
    let mut out = String::new();
    let mut objects = Objects::shipped();
    let pack = objects.pack().map(|p| p.bytes()).unwrap_or(0);
    let _ = writeln!(out, "pack (in the program file; read in place, never copied whole): {:.0} kB", pack as f64 / 1024.0);
    let t = Tokens::dark();
    let mut c = Canvas::new(320, 170);
    let g = Gfx::new(t);
    let looks = robot_state(&g, Story::Hot).looks(&g);
    // Each of its views once (they are 4 degrees apart), so each draw is one decode.
    let started = Instant::now();
    for yaw in (0..360).step_by(4) {
        objects.draw_looking(&mut c, &t, Color::BLACK, robot3d::GENERIC, "hero", 80.0, 90.0, View::yaw(yaw as f32), 1.0, 1.0, &looks);
    }
    let decode = started.elapsed().as_secs_f64() * 1e6 / 90.0;
    let started = Instant::now();
    for i in 0..200 {
        objects.draw_looking(&mut c, &t, Color::BLACK, robot3d::GENERIC, "hero", 80.0, 90.0, View::yaw(7.0 + (i % 2) as f32 * 15.0), 1.0, 1.0, &looks);
    }
    let turning = started.elapsed().as_secs_f64() * 1e6 / 200.0;
    let _ = writeln!(out, "on this PC: decoding and colouring a robot frame (112 px, its parts by state): {decode:.0} µs; a frame while it turns (two angles blended): {turning:.0} µs");
    let _ = writeln!(out, "the handheld, estimated at 20x this PC (README's factor): {:.1} ms to decode a frame, {:.1} ms a frame while it turns", decode * 20.0 / 1000.0, turning * 20.0 / 1000.0);
    let _ = writeln!(out, "cache of coloured frames: at most {} kB (CACHE_BYTES)", crate::object3d::CACHE_BYTES / 1024);
    // The Robot page open and turning for twenty seconds.
    let mut run = Run::with(Story::Teleop.robot(), false, false);
    run.open("Robot");
    run.settle(20.0);
    let _ = writeln!(out, "the Robot page turning for 20 s: {} kB of frames at most, {} frames decoded", run.shell.gfx.objects.peak / 1024, run.shell.gfx.objects.decoded);
    let mut run = Run::with(Robot::demo(), false, false);
    run.open("Weather");
    let _ = writeln!(out, "Weather's page: {} kB", run.shell.gfx.objects.peak / 1024);
    let run = Run::with(Robot::demo(), false, false);
    let home = run.shell.gfx.objects.peak;
    let _ = writeln!(out, "Home (flat icons): {} kB", home / 1024);
    std::fs::write(dir.join("budget.txt"), &out).ok();
    out
}

fn html(dir: &Path, sheets: &[(String, String)], turns: &[(String, String, i32, i32)], shots: &[String], movies: &[String], budget: &str) {
    let mut h = String::new();
    h.push_str("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Catalyst 3D objects</title><style>");
    h.push_str(":root{color-scheme:dark;--ground:#0b0c0e;--surface:#16181b;--ink:#eef0f2;--dim:#9aa1a8;--signal:#ff5b1f}body{margin:0;background:var(--ground);color:var(--ink);font:15px/1.5 system-ui,sans-serif}main{max-width:1180px;margin:0 auto;padding:24px 16px 64px}h1{font-size:28px;margin:8px 0 4px}h2{font-size:19px;margin:36px 0 8px;border-top:1px solid #2a2e33;padding-top:20px}p{color:var(--dim);max-width:72ch}img{image-rendering:pixelated;max-width:100%;height:auto;border-radius:8px;display:block}figure{margin:0}figcaption{color:var(--dim);font:12px ui-monospace,monospace;margin:6px 0 0}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:18px}.wide{display:grid;grid-template-columns:repeat(auto-fill,minmax(480px,1fr));gap:18px}pre{background:var(--surface);padding:12px 14px;border-radius:10px;overflow:auto;font-size:13px;color:var(--ink)}code{color:var(--ink)}");
    h.push_str("</style><main><h1>Catalyst 3D objects, in the apps</h1><p>Home's icons stay flat. The objects are in the apps, where they show something: the robot in the Robot app with each part coloured by what the robot reports, the battery's charge, the weather as it is, Wi-Fi's signal, setup's steps, and empty pages. Every picture here was drawn by the shell's own code from the pack it would ship (the turntables from a larger preview pack of whole turns). Panel shots are at the panel's 320 × 170, dithered to 16 bits as it shows them, and three times larger.</p>");
    h.push_str("<h2>Where they come from</h2><p>Built in code (<code>tools/bake3d/models.py</code>): the robot in the Robot app (its parts: drivetrain, intake, elevator, shooter, bumpers, frame), the battery (five cells and a bolt), Wi-Fi (a dot and three arcs), the weather, the handheld, the key, the switch, the tray, and the apps' objects for their empty pages (and a moon, made but not carried). Generated with Higgsfield (Tripo text to 3D) and kept reduced in <code>tools/bake3d/glb/</code>: setup's toy robot, the globe, the laptop and the trophy (and a rocket, made but not carried). A team's own robot is baked from its CAD with <code>tools/bake3d/robot.sh</code> and read from <code>/data/catalyst/robot3d.pack</code>, in place of the shell's.</p>");
    h.push_str("<h2>Contact sheets</h2><div class=\"wide\">");
    for (file, caption) in sheets {
        let _ = write!(h, "<figure><img src=\"{file}\" alt=\"{caption}\"><figcaption>{caption}</figcaption></figure>");
    }
    h.push_str("</div><h2>Turntables</h2><p>Every rendered angle (15° apart), shown twice their size. The robot's states side by side; setup's objects dark beside light; the weather dark.</p><div class=\"grid\">");
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

pub fn run(dir: &Path, preview: Option<&Path>, all: Option<&Path>) {
    if let Some(bytes) = all.and_then(|p| std::fs::read(p).ok()) {
        ALL.set(bytes).ok();
    }
    let situ = dir.join("situ");
    std::fs::create_dir_all(&situ).expect("create the preview folder");
    let sheets = sheets(dir);
    let turns = turntables(dir, preview);
    let (mut shots, mut movies) = (Vec::new(), Vec::new());
    in_situ(&situ, &mut shots, &mut movies);
    let budget = budget(dir);
    print!("{budget}");
    html(dir, &sheets, &turns, &shots, &movies, &budget);
    println!("previews in {}", dir.join("index.html").display());
}
