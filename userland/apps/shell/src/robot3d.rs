//! The robot in three dimensions, as the Robot app shows it: each part
//! coloured by what the robot says of it.
//!
//! - A part running hot or warned about glows amber, one in fault red,
//!   one at work (a shooter at speed, an intake running, an elevator on its
//!   way) a faint ice; the bumpers take the alliance's red or blue.
//! - Disabled, the robot is dimmed; with no link, it is grey and faded.
//! - Its mode (disabled, auto, teleop, test, stopped) and link, its battery
//!   and what needs attention are written around it.
//! - At rest it turns slowly on its stand; when a part turns warm or
//!   faults, it turns to show that part, holds there a while, and turns on.
//!   Calm, it stands still (and is put facing a part at once).
//!
//! Which part is which comes from the robot's own words: a health check, a
//! temperature or a current published under a subsystem's name is the
//! part whose name (or one like it) is in that name: "Elevator" is the
//! elevator, "FrontLeftDrive" the drivetrain. A team's own robot, baked
//! from its CAD (`tools/bake3d/robot.sh`), may name parts of its own
//! ("climber"): telemetry under that name colours it.

use bezel::{Align, Color, Rect};
use detent::{role, Animated};
use nt4::Value;

use crate::gfx::Gfx;
use crate::object3d::{Looks, PartLook, View};
use crate::robot::Robot;

/// The shell's own robot, and the one a team may put on the data partition.
pub const GENERIC: &str = "robot-generic";
pub const TEAM: &str = "robot-team";

/// The file a team's robot is read from: `/data/catalyst/robot3d.pack`.
pub fn team_pack() -> std::path::PathBuf {
    crate::config_dir().join("robot3d.pack")
}

/// What the robot is doing, as the Driver Station has it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Disabled,
    Auto,
    Teleop,
    Test,
    Stopped,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Disabled => "disabled",
            Mode::Auto => "auto",
            Mode::Teleop => "teleop",
            Mode::Test => "test",
            Mode::Stopped => "e-stopped",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Alliance {
    Red,
    Blue,
}

/// How a part is doing, worst last.
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
pub enum Condition {
    Normal,
    Working,
    Warm,
    Fault,
}

/// The robot as its picture shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct State {
    pub linked: bool,
    pub mode: Option<Mode>,
    pub alliance: Option<Alliance>,
    pub volts: Option<f64>,
    pub browned_out: bool,
    /// Each part: how it is, and a few words why (for the worst).
    pub parts: Vec<(String, Condition, String)>,
}

/// Other names a part goes by in telemetry.
fn kin(part: &str) -> &'static [&'static str] {
    match part {
        "drivetrain" => &["swerve", "drive", "steer", "module", "chassis", "wheel", "odometry"],
        "intake" => &["intake", "roller", "indexer", "feeder", "hopper", "serializer"],
        "elevator" => &["elevator", "arm", "lift", "wrist", "pivot", "climb", "extension"],
        "shooter" => &["shooter", "flywheel", "launcher", "turret", "hood", "scorer", "outtake"],
        "bumpers" => &["bumper"],
        "frame" => &["systemcore", "can", "power", "pdh", "pdp", "battery", "radio", "electrical"],
        _ => &[],
    }
}

/// The part a subsystem's name belongs to, if any: the one with the
/// longest of its names in it ("Flywheel" is the shooter's flywheel, not
/// the drivetrain's wheel), the first part listed on a tie.
fn part_for<'a>(name: &str, parts: &'a [String]) -> Option<&'a String> {
    let name = name.to_lowercase();
    let best = |p: &String| std::iter::once(p.as_str()).chain(kin(p).iter().copied()).filter(|k| name.contains(k)).map(str::len).max();
    let mut found: Option<(&String, usize)> = None;
    for p in parts {
        if let Some(len) = best(p) {
            if found.is_none_or(|(_, l)| len > l) {
                found = Some((p, len));
            }
        }
    }
    found.map(|(p, _)| p)
}

/// The robot's mode: from the Driver Station's control bits where WPILib
/// publishes them, else from Catalyst's own opmode (running, and whether
/// its name says autonomous).
pub fn mode(robot: &Robot) -> Option<Mode> {
    if let Some(bits) = robot.fms("FMSControlData").and_then(|v| v.as_i64()) {
        return Some(if bits & 8 != 0 {
            Mode::Stopped
        } else if bits & 1 == 0 {
            Mode::Disabled
        } else if bits & 2 != 0 {
            Mode::Auto
        } else if bits & 4 != 0 {
            Mode::Test
        } else {
            Mode::Teleop
        });
    }
    let running = robot.bool("OpMode/Running")?;
    let auto = robot.string("OpMode/Active").is_some_and(|n| n.to_lowercase().contains("auto"));
    Some(match (running, auto) {
        (false, _) => Mode::Disabled,
        (true, true) => Mode::Auto,
        (true, false) => Mode::Teleop,
    })
}

/// Read what the robot says of each of `parts`.
pub fn read(robot: &Robot, parts: &[String]) -> State {
    let mut conditions: Vec<(String, Condition, String)> = parts.iter().map(|p| (p.clone(), Condition::Normal, String::new())).collect();
    let mut worse = |part: Option<&String>, c: Condition, why: String| {
        if let Some(e) = part.and_then(|p| conditions.iter_mut().find(|(n, ..)| n == p)) {
            if c > e.1 {
                e.1 = c;
                e.2 = why;
            }
        }
    };
    let frame = parts.iter().find(|p| *p == "frame");
    if robot.linked() {
        // What is firing: a warning warms its part, an error faults it.
        for (severity, subsystem, id, detail) in robot.firing() {
            let c = if severity == "ERROR" { Condition::Fault } else { Condition::Warm };
            let words = if detail.is_empty() { id.replace('/', " ") } else { detail };
            let part = part_for(&subsystem, parts).or(part_for(&id, parts)).or(frame);
            worse(part, c, format!("{} · {}", subsystem.to_lowercase(), words.to_lowercase()));
        }
        // Temperatures, currents and what is at work, by the name they are under.
        for (key, value) in robot.under("") {
            let Some((owner, field)) = key.rsplit_once('/') else { continue };
            let part = part_for(owner, parts);
            match (field, &value) {
                ("TemperatureC", v) => {
                    if let Some(t) = v.as_f64() {
                        let c = if t >= 85.0 { Condition::Fault } else if t >= 70.0 { Condition::Warm } else { Condition::Normal };
                        worse(part, c, format!("{} {t:.0} °c", owner.to_lowercase()));
                    }
                }
                ("AtSpeed", Value::Bool(true)) | ("HasPiece", Value::Bool(true)) => worse(part, Condition::Working, String::new()),
                ("AtSetpoint", Value::Bool(false)) => worse(part, Condition::Working, String::new()),
                ("DutyCycle", v) if v.as_f64().is_some_and(|d| d.abs() > 0.05) => worse(part, Condition::Working, String::new()),
                ("SpeedMPS", v) if v.as_f64().is_some_and(|s| s.abs() > 0.3) => worse(part, Condition::Working, String::new()),
                _ => {}
            }
        }
        // A motor missing is the robot's to answer for, whichever it is.
        if let (Some(up), Some(want)) = (robot.i64("Devices/Motors/Connected"), robot.i64("Devices/Motors/Expected")) {
            if up < want {
                worse(frame, Condition::Fault, format!("{up} of {want} motors"));
            }
        }
    }
    let browned_out = robot.bool("Systemcore/BrownedOut") == Some(true);
    if browned_out {
        worse(frame, Condition::Fault, "browned out".into());
    }
    State {
        linked: robot.linked(),
        mode: robot.linked().then(|| mode(robot)).flatten(),
        alliance: robot.fms("IsRedAlliance").and_then(|v| v.as_bool()).map(|red| if red { Alliance::Red } else { Alliance::Blue }),
        volts: robot.f64("Systemcore/BatteryVolts").filter(|_| robot.linked()),
        browned_out,
        parts: conditions,
    }
}

impl State {
    /// The part most in need of a look, and why: the worst that is warm or
    /// worse.
    pub fn attention(&self) -> Option<(&str, Condition, &str)> {
        self.parts
            .iter()
            .filter(|(_, c, _)| *c >= Condition::Warm)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(p, c, why)| (p.as_str(), *c, why.as_str()))
    }

    /// How the picture looks: each part by its condition, the bumpers in
    /// the alliance's colour, the whole dimmed when disabled or unlinked.
    pub fn looks(&self, g: &Gfx) -> Looks {
        let t = g.tokens;
        let mut looks = Looks { dim: if !self.linked { 0.85 } else if matches!(self.mode, Some(Mode::Disabled) | Some(Mode::Stopped)) { 0.6 } else { 0.0 }, ..Looks::plain() };
        for (part, c, _) in &self.parts {
            let glow = match c {
                Condition::Normal => None,
                Condition::Working => Some((t.ice, 0.22)),
                Condition::Warm => Some((t.amber, 0.62)),
                Condition::Fault => Some((g.fault(), 0.7)),
            };
            let paint = (part == "bumpers").then_some(self.alliance).flatten().map(alliance_colour);
            if glow.is_some() || paint.is_some() {
                looks = looks.part(part, PartLook { paint, glow });
            }
        }
        looks
    }
}

/// FRC's bumper colours.
pub fn alliance_colour(a: Alliance) -> Color {
    match a {
        Alliance::Red => Color::hex(0xd93a34),
        Alliance::Blue => Color::hex(0x2f6fdf),
    }
}

/// How slowly it turns at rest, degrees a second, and how long it holds a
/// part it turned to show. (Our own tuning.)
const TURNING: f32 = 9.0;
const HOLD: f32 = 6.0;

/// The robot on its stand: which way it faces, and what it last turned to show.
pub struct Hero {
    yaw: Animated,
    /// Turning freely, or holding a part until then (seconds).
    held: Option<f32>,
    clock: f32,
    shown: Option<(String, Condition)>,
    /// The team's file, when it was last read (a newer one is read again).
    loaded: Option<std::time::SystemTime>,
}

impl Default for Hero {
    fn default() -> Self {
        Hero { yaw: Animated::new(0.0, role::detent()).with_rest(0.05), held: None, clock: 0.0, shown: None, loaded: None }
    }
}

impl Hero {
    /// The object to draw: the team's robot if one is installed, else the shell's.
    pub fn object(&mut self, g: &mut Gfx) -> &'static str {
        let path = team_pack();
        if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            if self.loaded != Some(modified) {
                self.loaded = Some(modified);
                if let Err(why) = g.objects.load(&path) {
                    eprintln!("catalyst-shell: the team's robot: {why}");
                }
            }
        }
        if g.objects.has(TEAM) {
            TEAM
        } else {
            GENERIC
        }
    }

    /// Advance: turning on, or holding. True while it moves.
    pub fn tick(&mut self, dt: f32, calm: bool, turning: bool) -> bool {
        self.clock += dt;
        if self.held.is_some_and(|until| self.clock >= until) && self.yaw.is_settled() {
            self.held = None;
        }
        if self.held.is_none() && turning && !calm {
            let next = self.yaw.value() + TURNING * dt;
            self.yaw.jump(next);
            return true;
        }
        self.yaw.tick(dt)
    }

    /// Something about a part changed for the worse: turn to show it.
    fn attend(&mut self, facing: f32, calm: bool) {
        let now = self.yaw.value();
        // The nearest way round to it.
        let to = now + ((facing - now + 180.0).rem_euclid(360.0) - 180.0);
        if calm {
            self.yaw.jump(to);
        } else {
            self.yaw.set(to);
        }
        self.held = Some(self.clock + HOLD);
    }

    /// Draw the robot centred on (`cx`, `cy`) as `state` has it, turning to
    /// a part newly in need of a look.
    pub fn draw(&mut self, g: &mut Gfx, object: &str, size: &str, cx: f32, cy: f32, state: &State, alpha: f32) {
        let attention = state.attention().map(|(p, c, _)| (p.to_owned(), c));
        if attention != self.shown {
            if let Some((part, _)) = &attention {
                if let Some((_, facing)) = g.objects.parts(object).into_iter().find(|(n, _)| n == part) {
                    self.attend(facing as f32, g.calm);
                    g.started_moving();
                }
            }
            self.shown = attention;
        }
        let looks = state.looks(g);
        let fade = if state.linked { 1.0 } else { 0.75 };
        g.object_looking(object, size, cx, cy, View::yaw(self.yaw.value()), 1.0, alpha * fade, &looks);
    }
}

/// `words` on one line `width` wide, else on two as even as they can be
/// ("79 °c," over "limit 70", not "79 °c, limit" over "70"); None if two
/// are not enough. Breaks only at plain spaces.
fn two_lines(g: &mut Gfx, words: &str, style: bezel::TextStyle, width: f32) -> Option<Vec<String>> {
    let plain = |s: &str| s.replace('\u{a0}', " ");
    if g.measure(&plain(words), style) <= width {
        return Some(vec![plain(words)]);
    }
    words
        .match_indices(' ')
        .filter_map(|(at, _)| {
            let (a, b) = (plain(&words[..at]), plain(&words[at + 1..]));
            let longer = g.measure(&a, style).max(g.measure(&b, style));
            (longer <= width).then_some((longer, a, b))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, a, b)| vec![a, b])
}

/// `words` with each figure held to the short word after it ("42 a",
/// "79 °c"), so a line never breaks between them.
fn keep_units(words: &str) -> String {
    let mut out = String::with_capacity(words.len());
    let mut rest = words;
    while let Some(at) = rest.find(' ') {
        let (before, after) = (&rest[..at], &rest[at + 1..]);
        let next = after.split(' ').next().unwrap_or("");
        let unit = next.trim_end_matches([',', '.', ';', ')']);
        let glue = before.ends_with(|c: char| c.is_ascii_digit()) && !unit.is_empty() && unit.chars().count() <= 3 && !unit.starts_with(|c: char| c.is_ascii_digit());
        out.push_str(before);
        out.push(if glue { '\u{a0}' } else { ' ' });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The words around the robot on its stage: its mode at the top left with
/// its link at the top right, its battery at the bottom left, and what
/// needs attention at the bottom right.
pub fn draw_captions(g: &mut Gfx, stage: Rect, state: &State, robot: &Robot) {
    let t = g.tokens;
    let ty = g.ty;
    let top = stage.y + 6.0 + ty.caption.size * 0.5;
    // The mode, as a chip: its dot says it at a glance.
    let (mode, dot) = match (state.linked, state.mode) {
        (false, _) => ("no link".to_owned(), t.faint),
        (true, Some(Mode::Stopped)) => (Mode::Stopped.name().to_owned(), g.fault()),
        (true, Some(Mode::Disabled)) => (Mode::Disabled.name().to_owned(), t.dim),
        (true, Some(m)) => (m.name().to_owned(), if m == Mode::Auto { t.signal } else { g.good() }),
        (true, None) => ("linked".to_owned(), g.good()),
    };
    let chip_w = g.measure(&mode, ty.caption) + 20.0;
    let chip = Rect::new(stage.x + 6.0, top - ty.caption.size * 0.5 - 3.0, chip_w, ty.caption.size + 6.0);
    g.surface(chip, chip.h * 0.5, t.surface2);
    g.dot(chip.x + 7.5, chip.cy(), 2.5, dot);
    g.text_at(chip.x + 13.0, chip.cy(), Align::Left, &mode, ty.caption, t.ink);
    if let Some(ms) = robot.rtt_ms().filter(|_| state.linked) {
        g.text_at(stage.right() - 8.0, top, Align::Right, &format!("{ms:.1} ms"), ty.caption, t.dim);
    }
    // The battery, large: amber when low, red when lower or browned out.
    let base = stage.bottom() - 7.0;
    let big = ty.display(22.0);
    let ink = match state.volts {
        _ if state.browned_out => g.fault(),
        Some(v) if v < 11.0 => g.fault(),
        Some(v) if v < 11.8 => t.amber_ink,
        Some(_) => t.ink,
        None => t.faint,
    };
    let volts = state.volts.map_or_else(|| "–".to_owned(), |v| format!("{v:.1}"));
    let w = g.text(stage.x + 8.0, base, &volts, big, ink);
    let unit = g.text(stage.x + 10.0 + w, base, "v", ty.caption, t.dim);
    // What needs attention, or with no link why not.
    let room = stage.right() - 8.0 - (stage.x + 10.0 + w + unit + 8.0);
    let (words, colour) = match state.attention() {
        _ if !state.linked => (robot.status(), t.dim),
        Some((_, c, why)) => (why.to_owned(), if c == Condition::Fault { g.fault() } else { t.amber_ink }),
        None => (String::new(), t.dim),
    };
    if !words.is_empty() && room > 20.0 {
        // On two lines if it needs them, right-aligned beside the battery, a
        // figure never parted from its unit. Too long for two, the part's
        // name goes first: the robot lights that part and turns to it.
        let words = keep_units(&words);
        let detail = words.split_once(" · ").map(|(_, d)| d);
        let lines = match two_lines(g, &words, ty.caption, room).or_else(|| detail.and_then(|d| two_lines(g, d, ty.caption, room))) {
            Some(lines) => lines,
            None => crate::help::wrap(g, detail.unwrap_or(&words), ty.caption, room),
        };
        let shown: Vec<String> = match lines.len() {
            0 => lines,
            1 => vec![g.text.fit(&lines[0], &ty.caption, room)],
            _ => vec![g.text.fit(&lines[0], &ty.caption, room), g.text.fit(&lines[1..].join(" "), &ty.caption, room)],
        };
        let pitch = ty.caption.size + 2.0;
        for (i, line) in shown.iter().rev().enumerate() {
            g.text_at(stage.right() - 8.0, base - ty.caption.size * 0.36 - i as f32 * pitch, Align::Right, line, ty.caption, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> Vec<String> {
        ["drivetrain", "intake", "elevator", "shooter", "bumpers", "frame"].map(String::from).to_vec()
    }

    #[test]
    fn a_figure_keeps_its_unit_on_its_line() {
        assert_eq!(keep_units("intake · stalled, 42 a"), "intake · stalled, 42\u{a0}a");
        assert_eq!(keep_units("elevator · 79 °c, limit 70"), "elevator · 79\u{a0}°c, limit 70");
        // A short word after a count is held too (harmless); a long one is
        // not, nor a figure after a figure.
        assert_eq!(keep_units("19 of 20 motors"), "19\u{a0}of 20 motors");
        assert_eq!(keep_units("between 3 4"), "between 3 4");
        assert_eq!(keep_units(""), "");
    }

    #[test]
    fn each_part_follows_what_is_said_of_it() {
        let mut robot = Robot::demo();
        robot.refresh(0.1);
        let s = read(&robot, &parts());
        assert!(s.linked);
        assert_eq!(s.mode, Some(Mode::Teleop));
        assert_eq!(s.alliance, Some(Alliance::Blue));
        let of = |s: &State, p: &str| s.parts.iter().find(|(n, ..)| n == p).unwrap().1;
        // The demo's shooter is at speed: at work.
        assert_eq!(of(&s, "shooter"), Condition::Working);
        // The CAN bus warning is the frame's.
        assert_eq!(of(&s, "frame"), Condition::Warm);
        // The elevator running hot: warm, and the robot says why.
        robot.demo_say("Elevator/TemperatureC", Value::Double(78.0));
        robot.demo_say("Health/Elevator/HighTemp/firing", Value::Bool(true));
        let s = read(&robot, &parts());
        assert_eq!(of(&s, "elevator"), Condition::Warm);
        // A motor dropping out is a fault, the worst there is: shown first.
        robot.demo_say("Devices/Motors/Connected", Value::Int(19));
        let s = read(&robot, &parts());
        assert_eq!(s.attention(), Some(("frame", Condition::Fault, "19 of 20 motors")));
        // Disabled, auto, the red alliance.
        robot.demo_say("FMSInfo/FMSControlData", Value::Int(32));
        robot.demo_say("FMSInfo/IsRedAlliance", Value::Bool(true));
        let s = read(&robot, &parts());
        assert_eq!((s.mode, s.alliance), (Some(Mode::Disabled), Some(Alliance::Red)));
        robot.demo_say("FMSInfo/FMSControlData", Value::Int(1 | 2 | 32));
        assert_eq!(read(&robot, &parts()).mode, Some(Mode::Auto));
        // No robot: nothing is said, and it is not linked.
        let none = read(&Robot::none(), &parts());
        assert!(!none.linked && none.mode.is_none() && none.volts.is_none() && none.attention().is_none());
    }

    #[test]
    fn a_part_is_found_by_any_of_its_names() {
        let p = parts();
        assert_eq!(part_for("FrontLeftDrive", &p).map(String::as_str), Some("drivetrain"));
        assert_eq!(part_for("Swerve", &p).map(String::as_str), Some("drivetrain"));
        assert_eq!(part_for("ElevatorFollower", &p).map(String::as_str), Some("elevator"));
        assert_eq!(part_for("Flywheel", &p).map(String::as_str), Some("shooter"));
        assert_eq!(part_for("CAN", &p).map(String::as_str), Some("frame"));
        assert_eq!(part_for("Vision", &p), None);
        // A team's own part, by its own name.
        let mut mine = p.clone();
        mine.push("climber".into());
        assert_eq!(part_for("ClimberWinch", &mine).map(String::as_str), Some("climber"), "its own name, longer than the elevator's \"climb\"");
        assert_eq!(part_for("Arm", &mine).map(String::as_str), Some("elevator"));
    }

    #[test]
    fn it_turns_slowly_and_turns_to_show_a_part_in_trouble() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let mut hero = Hero::default();
        let mut robot = Robot::demo();
        robot.refresh(0.1);
        let object = hero.object(&mut g);
        assert_eq!(object, GENERIC);
        let names: Vec<String> = g.objects.parts(object).into_iter().map(|(n, _)| n).collect();
        let state = read(&robot, &names);
        hero.draw(&mut g, object, "hero", 80.0, 90.0, &state, 1.0);
        // The frame's CAN warning is shown first: it turned toward the frame.
        let facing = g.objects.parts(object).into_iter().find(|(n, _)| n == "frame").unwrap().1 as f32;
        for _ in 0..60 {
            hero.tick(0.025, false, true);
        }
        assert!((hero.yaw.value() - facing).abs() < 2.0, "facing the frame: {} vs {facing}", hero.yaw.value());
        // It holds there, then turns on slowly.
        for _ in 0..(HOLD / 0.025) as usize {
            hero.tick(0.025, false, true);
        }
        let before = hero.yaw.value();
        assert!(hero.tick(1.0, false, true));
        assert!((hero.yaw.value() - before - TURNING).abs() < 0.5, "on at {TURNING}° a second");
        // Calm: it stands still.
        let mut calm = Hero::default();
        assert!(!calm.tick(1.0, true, true));
        assert_eq!(calm.yaw.value(), 0.0);
    }
}
