//! Numbers in 3D, laid out to be looked at and approved before it ships:
//! `catalyst-shell --objects DIR`.
//!
//! Everything here is drawn by the shell's own code (`object3d`,
//! `robot3d`), so what is seen is what the handheld draws: contact sheets
//! of the robot in each state (idle, intake out, hood up with the flywheel
//! spinning, a motor hot, disabled, no link, each alliance) and every pose
//! it carries, turntables, the Robot page at the panel's own size and three
//! times it (dithered to 16 bits as the panel shows them), its motion as
//! animated PNGs, the pages that are flat again, what it costs, and
//! `index.html` showing all of it.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use bezel::{Accent, Align, Canvas, Color, Rect, TextStyle, Tokens, Tone};
use nt4::Value;

use crate::gfx::{Gfx, H, W};
use crate::object3d::{Looks, Objects, Sheen, View, MECHANISMS};
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

/// The robot's states, as the demo robot is told to report them (as
/// Numbers' code publishes them: `Deploy/LengthInches`, `Hood/AngleDegrees`,
/// `Shooter/VelocityRPS`).
#[derive(Clone, Copy, PartialEq)]
enum Story {
    Idle,
    IntakeOut,
    Shooting,
    Hot,
    Disabled,
    NoLink,
    Red,
    Blue,
}

impl Story {
    const ALL: [Story; 8] = [Story::Idle, Story::IntakeOut, Story::Shooting, Story::Hot, Story::Disabled, Story::NoLink, Story::Red, Story::Blue];

    fn name(self) -> &'static str {
        match self {
            Story::Idle => "idle",
            Story::IntakeOut => "intake out",
            Story::Shooting => "hood up, flywheel",
            Story::Hot => "hood motor hot",
            Story::Disabled => "disabled",
            Story::NoLink => "no link",
            Story::Red => "red alliance",
            Story::Blue => "blue alliance",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Story::Idle => "idle",
            Story::IntakeOut => "intake-out",
            Story::Shooting => "hood-up-flywheel",
            Story::Hot => "hot-motor",
            Story::Disabled => "disabled",
            Story::NoLink => "no-link",
            Story::Red => "red-alliance",
            Story::Blue => "blue-alliance",
        }
    }

    /// What the robot says in this story, over a quiet idle robot.
    fn says(self) -> Vec<(&'static str, Value)> {
        let s = |v: &str| Value::String(v.into());
        match self {
            Story::IntakeOut => vec![("Deploy/LengthInches", Value::Double(11.8)), ("Intake/DutyCycle", Value::Double(1.0)), ("HopperManager/State", s("INTAKING"))],
            Story::Shooting => vec![
                ("Deploy/LengthInches", Value::Double(11.8)),
                ("Hood/AngleDegrees", Value::Double(33.0)),
                ("Shooter/VelocityRPS", Value::Double(27.0)),
                ("Shooter/PrimaryVelocityRPS", Value::Double(27.0)),
                ("Shooter/AtSpeed", Value::Bool(true)),
            ],
            Story::Hot => vec![
                ("Hood/TemperatureC", Value::Double(78.0)),
                ("Health/Hood/HighTemp/firing", Value::Bool(true)),
                ("Health/Hood/HighTemp/severity", s("WARN")),
                ("Health/Hood/HighTemp/detail", s("78 °C, limit 70")),
            ],
            Story::Disabled => vec![("FMSInfo/FMSControlData", Value::Int(32))],
            Story::Red => vec![("FMSInfo/IsRedAlliance", Value::Bool(true))],
            Story::Blue => vec![("FMSInfo/IsRedAlliance", Value::Bool(false))],
            Story::Idle | Story::NoLink => vec![],
        }
    }

    /// The demo robot as Numbers, quiet (nothing firing, nothing hot, the
    /// intake stowed, the hood idle, the flywheel idling), then told this.
    fn robot(self) -> Robot {
        if let Story::NoLink = self {
            // A robot set, its address not answering.
            return Robot::connect("127.0.0.1:9");
        }
        let mut r = Robot::demo();
        r.refresh(0.1);
        for (k, v) in quiet().into_iter().chain(self.says()) {
            r.demo_say(k, v);
        }
        r
    }
}

fn quiet() -> Vec<(&'static str, Value)> {
    vec![
        ("Robot/Identity/Name", Value::String("Numbers".into())),
        ("Health/CAN/can_s0/HighUtilization/firing", Value::Bool(false)),
        ("Health/Elevator/HighTemp/firing", Value::Bool(false)),
        ("Elevator/TemperatureC", Value::Double(52.0)),
        ("Health/History", Value::StringArray(vec![])),
        ("Deploy/LengthInches", Value::Double(5.0)),
        ("Hood/AngleDegrees", Value::Double(13.0)),
        ("Hood/State", Value::String("IDLE".into())),
        ("Hood/TemperatureC", Value::Double(44.0)),
        ("Shooter/VelocityRPS", Value::Double(4.2)),
        ("Shooter/PrimaryVelocityRPS", Value::Double(4.2)),
        ("Shooter/AtSpeed", Value::Bool(false)),
        ("Intake/DutyCycle", Value::Double(0.0)),
    ]
}

fn robot_state(g: &Gfx, story: Story) -> State {
    let parts: Vec<String> = g.objects.parts(robot3d::BUILT_IN).into_iter().map(|(n, _)| n).collect();
    robot3d::read(&story.robot(), &parts)
}

/// Where a state puts the robot's mechanisms, as the Robot page shows them.
fn pose_of(g: &Gfx, state: &State) -> [f32; MECHANISMS] {
    let mut pose = [0.0; MECHANISMS];
    for (m, (name, steps)) in g.objects.mechanisms(robot3d::BUILT_IN).iter().enumerate().take(MECHANISMS) {
        let value = match name.as_str() {
            "intake" => state.intake,
            "hood" => state.hood,
            _ => None,
        };
        if let Some(v) = value {
            let i = steps.iter().enumerate().min_by(|a, b| (a.1 - v as f32).abs().total_cmp(&(b.1 - v as f32).abs())).map_or(0, |(i, _)| i);
            pose[m] = i as f32;
        }
    }
    pose
}

/// A Gfx drawing into a canvas of its own size.
fn sheet_gfx(w: i32, h: i32, t: Tokens) -> Gfx {
    let mut g = Gfx::new(t);
    g.canvas = Canvas::new(w, h);
    g.canvas.clear(t.ground);
    g
}

type Cell = (String, Box<dyn Fn(&mut Gfx, Rect)>);

/// A sheet of cells, each the robot drawn by `draw`, captioned.
fn sheet(dir: &Path, file: &str, tone: Tone, accent: Accent, cell: (i32, i32), across: usize, cells: &[Cell]) -> String {
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

/// The robot in a story: posed and coloured as the Robot page has it.
fn story_cell(story: Story, yaw: f32) -> Cell {
    (story.name().to_owned(), Box::new(move |g: &mut Gfx, r: Rect| {
        let state = robot_state(g, story);
        let mut looks = state.looks(g);
        if state.flywheel.is_some_and(|rps| rps > 2.0) {
            looks.sheen = Some(("flywheel".into(), Sheen { colour: g.tokens.ice, phase: 0.3, amount: 0.45 }));
        }
        let view = View::yaw(yaw).posed(pose_of(g, &state));
        g.object_looking(robot3d::BUILT_IN, "hero", r.cx(), r.cy() - 8.0, view, 1.0, if state.linked { 1.0 } else { 0.75 }, &looks);
    }))
}

fn sheets(dir: &Path) -> Vec<(String, String)> {
    let default = crate::shell::Settings::default().accent;
    let mut out = Vec::new();
    for (tone, tone_name) in [(Tone::Dark, "dark"), (Tone::Light, "light")] {
        let cells: Vec<Cell> = Story::ALL.iter().map(|&s| story_cell(s, 0.0)).collect();
        out.push((sheet(dir, &format!("sheet-robot-{tone_name}"), tone, default, (150, 150), 4, &cells), format!("Numbers in each state, {tone_name}, at rest")));
    }
    // Every pose the pack carries: the intake's two steps by the hood's three.
    let mut cells: Vec<Cell> = Vec::new();
    for hood in 0..3 {
        for intake in 0..2 {
            let caption = format!("{} · hood {}°", ["stowed", "out"][intake], [11, 25, 40][hood]);
            cells.push((caption, Box::new(move |g: &mut Gfx, r: Rect| {
                let view = View::yaw(-50.0).posed([intake as f32, hood as f32, 0.0, 0.0]);
                g.object_looking(robot3d::BUILT_IN, "hero", r.cx(), r.cy() - 8.0, view, 1.0, 1.0, &Looks::plain());
            })));
        }
    }
    out.push((sheet(dir, "sheet-poses", Tone::Dark, default, (150, 150), 2, &cells), "Every pose carried, from the side: the intake carriage (Deploy inches) by the hood angle".into()));
    // The accents, and the user's own colour: the bumpers follow it until the alliance is known.
    let cells: Vec<Cell> = [("orange", Accent::Orange), ("green", Accent::Green), ("blue", Accent::Blue), ("own colour (teal)", Accent::Hue(36))]
        .into_iter()
        .map(|(c, accent)| {
            (c.to_owned(), Box::new(move |g: &mut Gfx, r: Rect| {
                let t = Tokens::of(accent, Tone::Dark);
                let saved = g.tokens;
                g.tokens = t;
                g.object_looking(robot3d::BUILT_IN, "hero", r.cx(), r.cy() - 8.0, View::REST, 1.0, 1.0, &Looks::plain());
                g.tokens = saved;
            }) as Box<dyn Fn(&mut Gfx, Rect)>)
        })
        .collect();
    out.push((sheet(dir, "sheet-accents", Tone::Dark, default, (150, 150), 4, &cells), "Before the alliance is known its bumpers take the accent, or the user's own colour; Numbers' blue stays its own".into()));
    out
}

/// The robot turning all the way round, each 8° rendered view once, in each
/// state given, side by side.
fn turntable(dir: &Path, file: &str, cells: &[(Tone, Story)]) -> (String, i32, i32) {
    let mut objects = Objects::shipped();
    let px = objects.px(robot3d::BUILT_IN, "hero").unwrap_or(112.0) as i32;
    let cell = px + 16;
    let (w, h) = (cell * cells.len() as i32, cell);
    let looks: Vec<(Tokens, Looks, [f32; MECHANISMS], f32)> = cells
        .iter()
        .map(|&(tone, story)| {
            let t = Tokens::of(crate::shell::Settings::default().accent, tone);
            let g = Gfx::new(t);
            let state = robot_state(&g, story);
            (t, state.looks(&g), pose_of(&g, &state), if state.linked { 1.0 } else { 0.75 })
        })
        .collect();
    let mut frames = Vec::new();
    for step in 0..45 {
        let yaw = step as f32 * 8.0;
        let mut c = Canvas::new(w, h);
        for (k, (t, looks, pose, alpha)) in looks.iter().enumerate() {
            c.fill_rect(Rect::new((k as i32 * cell) as f32, 0.0, cell as f32, cell as f32), t.surface1, 1.0);
            let fault = if t.tone == Tone::Dark { Color::hex(0xff6b5e) } else { Color::hex(0xb3261e) };
            objects.draw_looking(&mut c, t, fault, robot3d::BUILT_IN, "hero", (k as i32 * cell + cell / 2) as f32, cell as f32 * 0.5, View::yaw(yaw).posed(*pose), 1.0, *alpha, looks);
        }
        frames.push(c.px);
    }
    let name = format!("turn-{file}.png");
    save_apng(&frames, w as usize, h as usize, &dir.join(&name), 90);
    (name, w, h)
}

fn turntables(dir: &Path) -> Vec<(String, String, i32, i32)> {
    let mut out = Vec::new();
    let (f, w, h) = turntable(dir, "robot-states", &[(Tone::Dark, Story::Idle), (Tone::Dark, Story::IntakeOut), (Tone::Dark, Story::Shooting), (Tone::Dark, Story::Hot)]);
    out.push((f, "idle · intake out · hood up, flywheel · hood motor hot".into(), w, h));
    let (f, w, h) = turntable(dir, "robot-light", &[(Tone::Light, Story::Idle), (Tone::Light, Story::Red), (Tone::Dark, Story::Disabled), (Tone::Dark, Story::NoLink)]);
    out.push((f, "light: idle · red alliance; dark: disabled · no link".into(), w, h));
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
    fn say(&mut self, story: Story) {
        for (k, v) in story.says() {
            self.shell.robot.demo_say(k, v);
        }
    }
}

fn movie(dir: &Path, name: &str, frames: &[Vec<u32>], movies: &mut Vec<String>) {
    save_apng(frames, W as usize, H as usize, &dir.join(format!("{name}.png")), 25);
    movies.push(name.to_owned());
}

/// The robot in place on its page, still and moving; and the pages that
/// are flat again, to see them finished.
fn in_situ(dir: &Path, shots: &mut Vec<String>, flat: &mut Vec<String>, movies: &mut Vec<String>) {
    use Key::*;
    // The Robot app, in each state.
    for story in Story::ALL {
        let mut run = Run::with(story.robot(), false, false);
        run.open("Robot");
        run.settle(2.2);
        run.still(dir, &format!("robot-{}", story.file()), shots);
    }
    for story in [Story::Idle, Story::Shooting] {
        let mut run = Run::with(story.robot(), true, false);
        run.open("Robot");
        run.settle(1.0);
        run.still(dir, &format!("robot-{}-light", story.file()), shots);
    }
    let mut run = Run::with(Story::Hot.robot(), false, true);
    run.open("Robot");
    run.still(dir, "robot-hot-motor-calm", shots);
    // Motion: at rest it turns slowly.
    let mut run = Run::with(Story::Idle.robot(), false, false);
    run.open("Robot");
    let f = run.frames(3.0);
    movie(dir, "motion-robot-turning", &f, movies);
    // The intake deploys: it turns to show it, and the carriage slides out.
    run.say(Story::IntakeOut);
    let f = run.frames(2.4);
    movie(dir, "motion-intake-deploys", &f, movies);
    // Aiming: the hood rises and the flywheel spins up.
    run.say(Story::Shooting);
    let f = run.frames(2.6);
    movie(dir, "motion-hood-rises-flywheel-spins", &f, movies);
    run.still(dir, "robot-aiming", shots);
    // The hood's motor running hot: it turns to show the hood.
    let mut run = Run::with(Story::Idle.robot(), false, false);
    run.open("Robot");
    run.settle(0.5);
    run.say(Story::Hot);
    let f = run.frames(2.5);
    movie(dir, "motion-hood-motor-hot", &f, movies);
    // Enabled: from disabled to teleop, the robot brightens.
    let mut run = Run::with(Story::Disabled.robot(), false, false);
    run.open("Robot");
    run.settle(0.5);
    run.shell.robot.demo_say("FMSInfo/FMSControlData", Value::Int(1 | 32));
    let f = run.frames(1.2);
    movie(dir, "motion-robot-enabled", &f, movies);

    // Flat again, as on main: Home, Weather, Settings › Battery, Health,
    // an empty page, About, and setup's pages (Next first, no object).
    let mut run = Run::with(Robot::demo(), false, false);
    run.still(dir, "flat-home", flat);
    let mut run = Run::with(Robot::demo(), false, false);
    run.open("Weather");
    run.still(dir, "flat-weather", flat);
    let mut run = Run::with(Robot::demo(), false, false);
    run.settings("Battery");
    run.still(dir, "flat-battery", flat);
    let mut run = Run::with(Story::Idle.robot(), false, false);
    run.open("Health");
    run.still(dir, "flat-health", flat);
    let mut run = Run::with(Robot::none(), false, false);
    run.open("Tables");
    run.still(dir, "flat-tables-no-robot", flat);
    let mut run = Run::with(Robot::demo(), false, false);
    let about = crate::screens::settings_row("About this handheld");
    run.open("Settings");
    run.keys(&vec![Down; about], 0.08);
    run.keys(&[Enter], 1.2);
    run.still(dir, "flat-about", flat);
    let mut run = Run::with(Robot::demo(), false, false);
    run.shell.start_setup();
    run.settle(1.0);
    run.still(dir, "flat-setup-welcome", flat);
    run.shell.key(Enter);
    run.settle(1.0);
    run.still(dir, "flat-setup-wifi", flat);
    run.keys(&[Tab, Tab, Tab], 0.9);
    run.still(dir, "flat-setup-team", flat);
}

/// What the robot costs, measured.
fn budget(dir: &Path) -> String {
    let mut out = String::new();
    let mut objects = Objects::shipped();
    let pack = objects.pack().map(|p| p.bytes()).unwrap_or(0);
    let _ = writeln!(out, "pack (in the program file; read in place, never copied whole): {:.0} kB, Numbers alone", pack as f64 / 1024.0);
    let t = Tokens::dark();
    let mut c = Canvas::new(320, 170);
    let g = Gfx::new(t);
    let looks = robot_state(&g, Story::Hot).looks(&g);
    // Each of its views once (they are 4 degrees apart), so each draw is one decode.
    let started = Instant::now();
    for yaw in (0..360).step_by(4) {
        objects.draw_looking(&mut c, &t, Color::BLACK, robot3d::BUILT_IN, "hero", 80.0, 90.0, View::yaw(yaw as f32), 1.0, 1.0, &looks);
    }
    let decode = started.elapsed().as_secs_f64() * 1e6 / 90.0;
    // The same with the intake out and the hood up: two patches laid over each.
    let started = Instant::now();
    for yaw in (0..360).step_by(4) {
        objects.draw_looking(&mut c, &t, Color::BLACK, robot3d::BUILT_IN, "hero", 80.0, 90.0, View::yaw(yaw as f32).posed([2.0, 2.0, 0.0, 0.0]), 1.0, 1.0, &looks);
    }
    let posed = started.elapsed().as_secs_f64() * 1e6 / 90.0;
    let started = Instant::now();
    for i in 0..200 {
        objects.draw_looking(&mut c, &t, Color::BLACK, robot3d::BUILT_IN, "hero", 80.0, 90.0, View::yaw(6.0 + (i % 2) as f32 * 8.0), 1.0, 1.0, &looks);
    }
    let turning = started.elapsed().as_secs_f64() * 1e6 / 200.0;
    let _ = writeln!(out, "on this PC: decoding and colouring a robot frame (112 px, its parts by state): {decode:.0} µs at rest, {posed:.0} µs posed (intake out, hood up); a frame while it turns (two views blended): {turning:.0} µs");
    let _ = writeln!(out, "the handheld, estimated at 20x this PC (README's factor): {:.1} ms to decode a frame ({:.1} ms posed), {:.1} ms a frame while it turns", decode * 20.0 / 1000.0, posed * 20.0 / 1000.0, turning * 20.0 / 1000.0);
    let _ = writeln!(out, "cache of coloured frames: at most {} kB (CACHE_BYTES)", crate::object3d::CACHE_BYTES / 1024);
    // The Robot page open and turning for twenty seconds, at rest and then aiming.
    for story in [Story::Idle, Story::Shooting] {
        let mut run = Run::with(story.robot(), false, false);
        run.open("Robot");
        run.settle(20.0);
        let _ = writeln!(out, "the Robot page, {}, turning for 20 s: {} kB of frames at most, {} frames decoded", story.name(), run.shell.gfx.objects.peak / 1024, run.shell.gfx.objects.decoded);
    }
    let mut run = Run::with(Robot::demo(), false, false);
    run.open("Weather");
    let _ = writeln!(out, "any other page (Weather, flat): {} kB", run.shell.gfx.objects.peak / 1024);
    std::fs::write(dir.join("budget.txt"), &out).ok();
    out
}

fn html(dir: &Path, sheets: &[(String, String)], turns: &[(String, String, i32, i32)], shots: &[String], flat: &[String], movies: &[String], budget: &str) {
    let mut h = String::new();
    h.push_str("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Numbers in 3D</title><style>");
    h.push_str(":root{color-scheme:dark;--ground:#0b0c0e;--surface:#16181b;--ink:#eef0f2;--dim:#9aa1a8;--signal:#ff5b1f}body{margin:0;background:var(--ground);color:var(--ink);font:15px/1.5 system-ui,sans-serif}main{max-width:1180px;margin:0 auto;padding:24px 16px 64px}h1{font-size:28px;margin:8px 0 4px}h2{font-size:19px;margin:36px 0 8px;border-top:1px solid #2a2e33;padding-top:20px}p{color:var(--dim);max-width:72ch}img{image-rendering:pixelated;max-width:100%;height:auto;border-radius:8px;display:block}figure{margin:0}figcaption{color:var(--dim);font:12px ui-monospace,monospace;margin:6px 0 0}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:18px}.wide{display:grid;grid-template-columns:repeat(auto-fill,minmax(480px,1fr));gap:18px}pre{background:var(--surface);padding:12px 14px;border-radius:10px;overflow:auto;font-size:13px;color:var(--ink)}code{color:var(--ink)}a{color:var(--dim)}");
    h.push_str("</style><main><h1>Numbers, in the Robot app</h1><p>Only the robot is 3D; every other icon and page is flat, as on main. The robot is Numbers, FRC 5805's, from the team website's own model (<code>smblyrequired/public/models/robot.glb</code>). Its parts are coloured by what the robot reports, its intake and hood are posed as it reports them (<code>Deploy/LengthInches</code>, <code>Hood/AngleDegrees</code>), and its flywheel shines as it spins (<code>Shooter/VelocityRPS</code>). Every picture here was drawn by the shell's own code from the pack it would ship. Panel shots are at the panel's 320 × 170, dithered to 16 bits as it shows them, and three times larger.</p>");
    h.push_str("<h2>Contact sheets</h2><div class=\"wide\">");
    for (file, caption) in sheets {
        let _ = write!(h, "<figure><img src=\"{file}\" alt=\"{caption}\"><figcaption>{caption}</figcaption></figure>");
    }
    h.push_str("</div><h2>Turntables</h2><p>Every second rendered view (8° apart), shown twice their size.</p><div class=\"wide\">");
    for (file, caption, w, ht) in turns {
        let _ = write!(h, "<figure><img src=\"{file}\" width=\"{}\" height=\"{}\" alt=\"{caption}\"><figcaption>{caption}</figcaption></figure>", w * 2, ht * 2);
    }
    h.push_str("</div><h2>The Robot page</h2><div class=\"wide\">");
    for name in shots {
        let _ = write!(h, "<figure><img src=\"situ/{name}@3x.png\" alt=\"{name}\"><figcaption>{name} · <a href=\"situ/{name}.png\">actual size</a></figcaption></figure>");
    }
    h.push_str("</div><h2>Motion</h2><p>At the handheld's beat of 25 ms a frame, shown twice the panel's size.</p><div class=\"wide\">");
    for name in movies {
        let _ = write!(h, "<figure><img src=\"situ/{name}.png\" width=\"{}\" height=\"{}\" alt=\"{name}\"><figcaption>{name}</figcaption></figure>", W as i32 * 2, H as i32 * 2);
    }
    h.push_str("</div><h2>Flat again</h2><p>Home, Weather, Settings › Battery, Health, empty pages, About and setup, as on main (setup keeps its quicker flow: pages that start on Next, and Wi-Fi carrying on by itself).</p><div class=\"wide\">");
    for name in flat {
        let _ = write!(h, "<figure><img src=\"situ/{name}@3x.png\" alt=\"{name}\"><figcaption>{name} · <a href=\"situ/{name}.png\">actual size</a></figcaption></figure>");
    }
    let _ = write!(h, "</div><h2>What it costs</h2><pre>{}</pre></main></html>\n", budget.replace('&', "&amp;").replace('<', "&lt;"));
    std::fs::write(dir.join("index.html"), h).expect("write the preview page");
}

pub fn run(dir: &Path) {
    let situ = dir.join("situ");
    std::fs::create_dir_all(&situ).expect("create the preview folder");
    let sheets = sheets(dir);
    let turns = turntables(dir);
    let (mut shots, mut flat, mut movies) = (Vec::new(), Vec::new(), Vec::new());
    in_situ(&situ, &mut shots, &mut flat, &mut movies);
    let budget = budget(dir);
    print!("{budget}");
    html(dir, &sheets, &turns, &shots, &flat, &movies, &budget);
    println!("previews in {}", dir.join("index.html").display());
}
