//! Mech: a mechanism calculator for the pit. Pick a mechanism (a gear
//! train, a drivetrain, an arm, an elevator, a flywheel), set its motor and
//! numbers, and read what it does: speeds, torques, currents and times from
//! a DC motor model ([`model`]) over the motor table in `motors.tsv`. A
//! mechanism can be kept by name in `/data/mech/`.

pub mod model;
mod screen;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

pub use screen::MechApp;

use model::{Stage, StageKind};
use std::path::PathBuf;

/// The mechanisms there is a calculator for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calc {
    Ratio,
    Drive,
    Arm,
    Elevator,
    Flywheel,
}

impl Calc {
    pub const ALL: [Calc; 5] = [Calc::Ratio, Calc::Drive, Calc::Arm, Calc::Elevator, Calc::Flywheel];

    pub fn name(self) -> &'static str {
        match self {
            Calc::Ratio => "Gear ratio",
            Calc::Drive => "Drivetrain",
            Calc::Arm => "Arm",
            Calc::Elevator => "Elevator",
            Calc::Flywheel => "Flywheel",
        }
    }

    /// Its name over its page, short enough to sit beside the island.
    pub fn short(self) -> &'static str {
        match self {
            Calc::Ratio => "Gears",
            Calc::Drive => "Drive",
            other => other.name(),
        }
    }

    /// Its name in a saved file.
    pub fn key(self) -> &'static str {
        match self {
            Calc::Ratio => "ratio",
            Calc::Drive => "drive",
            Calc::Arm => "arm",
            Calc::Elevator => "elevator",
            Calc::Flywheel => "flywheel",
        }
    }

    /// What it works out, on its row.
    pub fn gives(self) -> &'static str {
        match self {
            Calc::Ratio => "speed · torque",
            Calc::Drive => "speed · push",
            Calc::Arm => "hold · move",
            Calc::Elevator => "speed · amps",
            Calc::Flywheel => "spin-up",
        }
    }

    pub fn fields(self) -> &'static [Field] {
        use FieldKind::*;
        const fn f(key: &'static str, label: &'static str, unit: &'static str, default: f64, kind: FieldKind) -> Field {
            Field { key, label, unit, default, kind }
        }
        const MOTOR: Field = f("motor", "motor", "", 0.0, Motor);
        match self {
            Calc::Ratio => {
                const F: &[Field] = &[
                MOTOR,
                f("motors", "motors", "", 1.0, Count),
                f("stage1", "stage 1", "", 0.0, Stage(0)),
                f("stage2", "stage 2", "", 0.0, Stage(1)),
                f("stage3", "stage 3", "", 0.0, Stage(2)),
                f("stage4", "stage 4", "", 0.0, Stage(3)),
                f("limit", "limit", "A", 60.0, Number(0)),
                f("efficiency", "efficiency", "%", 100.0, Percent),
                ];
                F
            }
            Calc::Drive => {
                const F: &[Field] = &[
                MOTOR,
                f("per_side", "motors", "a side", 2.0, Count),
                f("ratio", "ratio", ":1", 6.75, Number(2)),
                f("wheel", "wheel", "in", 4.0, Number(2)),
                f("mass", "robot", "lb", 140.0, Number(0)),
                f("mu", "grip", "", 1.1, Number(2)),
                f("limit", "limit", "A", 60.0, Number(0)),
                f("efficiency", "efficiency", "%", 90.0, Percent),
                f("speed_loss", "real speed", "%", 81.0, Percent),
                f("distance", "distance", "ft", 15.0, Number(1)),
                ];
                F
            }
            Calc::Arm => {
                const F: &[Field] = &[
                MOTOR,
                f("motors", "motors", "", 1.0, Count),
                f("ratio", "ratio", ":1", 100.0, Number(1)),
                f("length", "length", "in", 24.0, Number(1)),
                f("mass", "mass", "lb", 10.0, Number(1)),
                f("com", "CoM", "in", 12.0, Number(1)),
                f("start", "from", "°", 0.0, Signed(0)),
                f("end", "to", "°", 90.0, Signed(0)),
                f("limit", "limit", "A", 40.0, Number(0)),
                f("efficiency", "efficiency", "%", 100.0, Percent),
                ];
                F
            }
            Calc::Elevator => {
                const F: &[Field] = &[
                MOTOR,
                f("motors", "motors", "", 2.0, Count),
                f("ratio", "ratio", ":1", 5.0, Number(2)),
                f("spool", "spool", "in", 1.5, Number(3)),
                f("load", "load", "lb", 15.0, Number(1)),
                f("travel", "travel", "in", 48.0, Number(1)),
                f("limit", "limit", "A", 40.0, Number(0)),
                f("efficiency", "efficiency", "%", 100.0, Percent),
                ];
                F
            }
            Calc::Flywheel => {
                const F: &[Field] = &[
                MOTOR,
                f("motors", "motors", "", 1.0, Count),
                f("ratio", "ratio", ":1", 1.0, Number(3)),
                f("moi", "MOI", "lb·in²", 5.0, Number(2)),
                f("target", "target", "rpm", 4000.0, Number(0)),
                f("drop", "shot drop", "%", 10.0, Percent),
                f("limit", "limit", "A", 60.0, Number(0)),
                f("efficiency", "efficiency", "%", 100.0, Percent),
                ];
                F
            }
        }
    }

    pub fn from_key(key: &str) -> Option<Calc> {
        Calc::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// One input on a calculator's page.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub default: f64,
    pub kind: FieldKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldKind {
    Motor,
    /// A whole number of motors, 1 to 12.
    Count,
    /// A positive number, shown to this many decimals at most.
    Number(usize),
    /// A number that may be negative (an angle).
    Signed(usize),
    /// 1 to 100.
    Percent,
    /// A stage of the train, typed as teeth `12:60`.
    Stage(usize),
}

impl Field {
    /// A typed value made to fit the field, or `None` if it does not.
    pub fn accept(&self, v: f64) -> Option<f64> {
        if !v.is_finite() {
            return None;
        }
        match self.kind {
            FieldKind::Count => Some(v.round().clamp(1.0, 12.0)),
            FieldKind::Percent => (v > 0.0).then(|| v.min(100.0)),
            FieldKind::Signed(_) => Some(v.clamp(-360.0, 360.0)),
            FieldKind::Number(_) => (v > 0.0).then_some(v.min(1e6)),
            FieldKind::Motor | FieldKind::Stage(_) => None,
        }
    }

    /// A value as its row shows it, with its unit.
    pub fn show(&self, v: f64) -> String {
        let n = match self.kind {
            FieldKind::Number(d) | FieldKind::Signed(d) => trim(v, d),
            _ => trim(v, 0),
        };
        match self.unit {
            "" => n,
            ":1" => format!("{n}:1"),
            "°" | "%" => format!("{n}{}", self.unit),
            unit => format!("{n} {unit}"),
        }
    }
}

/// `x` to at most `decimals` places, without trailing zeros.
pub fn trim(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// A number to three significant figures or so, as results read.
pub fn sig(x: f64) -> String {
    let a = x.abs();
    let d = if a >= 100.0 {
        0
    } else if a >= 10.0 {
        1
    } else if a >= 1.0 {
        2
    } else {
        3
    };
    format!("{x:.d$}")
}

/// A figure in the card: two decimals under ten, one under a hundred,
/// whole above (the rows below carry three figures).
pub fn head(x: f64) -> String {
    let a = x.abs();
    let d = if a >= 100.0 {
        0
    } else if a >= 10.0 {
        1
    } else {
        2
    };
    format!("{x:.d$}")
}

/// A mechanism: what it is, its motor and its numbers (in the order of
/// its calculator's fields; a motor's and a stage's places are unused).
#[derive(Clone, Debug, PartialEq)]
pub struct Mech {
    pub calc: Calc,
    pub name: String,
    pub motor: usize,
    pub values: Vec<f64>,
    pub stages: [Option<Stage>; 4],
}

impl Mech {
    pub fn new(calc: Calc) -> Mech {
        let stages = if calc == Calc::Ratio {
            [Some(Stage { kind: StageKind::Gear, driving: 12, driven: 60 }), Some(Stage { kind: StageKind::Belt, driving: 18, driven: 36 }), None, None]
        } else {
            [None; 4]
        };
        Mech { calc, name: String::new(), motor: 0, values: calc.fields().iter().map(|f| f.default).collect(), stages }
    }

    pub fn get(&self, key: &str) -> f64 {
        self.calc.fields().iter().position(|f| f.key == key).and_then(|i| self.values.get(i).copied()).unwrap_or(0.0)
    }

    pub fn motor(&self) -> &'static model::Motor {
        let all = model::motors();
        &all[self.motor.min(all.len() - 1)]
    }

    /// The reduction its own stages make (the gear-ratio calculator).
    pub fn train(&self) -> f64 {
        let stages: Vec<Stage> = self.stages.iter().flatten().copied().collect();
        if stages.is_empty() {
            1.0
        } else {
            model::reduction(&stages)
        }
    }

    /// What it does.
    pub fn solve(&self) -> Solved {
        solve(self)
    }

    // ------------------------------------------------------------ files ---

    pub fn to_json(&self) -> String {
        let mut values = serde_json::Map::new();
        for (f, v) in self.calc.fields().iter().zip(&self.values) {
            if matches!(f.kind, FieldKind::Motor | FieldKind::Stage(_)) {
                continue;
            }
            values.insert(f.key.into(), serde_json::json!(v));
        }
        let stages: Vec<String> = self.stages.iter().map(|s| s.map(|s| format!("{} {}:{}", s.kind.name(), s.driving, s.driven)).unwrap_or_default()).collect();
        let mut doc = serde_json::json!({
            "calc": self.calc.key(),
            "name": self.name,
            "motor": self.motor().name,
            "values": values,
        });
        if self.calc == Calc::Ratio {
            doc["stages"] = serde_json::json!(stages);
        }
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<Mech> {
        let doc: serde_json::Value = serde_json::from_str(text).ok()?;
        let calc = Calc::from_key(doc["calc"].as_str()?)?;
        let mut mech = Mech::new(calc);
        mech.name = doc["name"].as_str().unwrap_or_default().to_owned();
        if let Some(name) = doc["motor"].as_str() {
            mech.motor = model::motors().iter().position(|m| m.name.eq_ignore_ascii_case(name)).unwrap_or(0);
        }
        for (i, f) in calc.fields().iter().enumerate() {
            if let Some(v) = doc["values"][f.key].as_f64().and_then(|v| f.accept(v)) {
                mech.values[i] = v;
            }
        }
        if let Some(stages) = doc["stages"].as_array() {
            mech.stages = [None; 4];
            for (slot, s) in mech.stages.iter_mut().zip(stages) {
                *slot = s.as_str().and_then(parse_stage_line);
            }
        }
        Some(mech)
    }
}

/// `gear 12:60` (or just `12:60`) as a stage.
pub fn parse_stage_line(s: &str) -> Option<Stage> {
    let s = s.trim();
    let (kind, teeth) = match s.split_once(' ') {
        Some((k, rest)) => (StageKind::from_name(k), rest),
        None => (StageKind::Gear, s),
    };
    parse_teeth(teeth).map(|(driving, driven)| Stage { kind, driving, driven })
}

/// Teeth typed as `12:60`: what drives, then what is driven.
pub fn parse_teeth(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.trim().split_once(':')?;
    let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
    (a > 0 && b > 0 && a <= 1000 && b <= 1000).then_some((a, b))
}

// ---------------------------------------------------------------- results ---

/// What a mechanism does, as the page shows it.
pub struct Solved {
    /// The two figures in the card: (what, value, unit).
    pub heads: [(&'static str, String, &'static str); 2],
    /// Every result, as rows: (what, value with unit).
    pub rows: Vec<(String, String)>,
    pub plot: Plot,
}

/// The card's graph: one line, on one pair of axes.
#[derive(Default)]
pub struct Plot {
    pub x_unit: &'static str,
    pub y_unit: &'static str,
    pub line: Vec<(f64, f64)>,
    /// What the line would be without the current limit, faint.
    pub ghost: Vec<(f64, f64)>,
    /// Where the mechanism works, marked on the line.
    pub mark: Option<(f64, f64)>,
    /// A level across the graph (the target, top speed, the limit) and its word.
    pub level: Option<(f64, &'static str)>,
    /// The x axis runs exactly over the line (an arm's angles), not from 0.
    pub exact_x: bool,
}

fn time(t: Option<f64>) -> String {
    t.map(|t| format!("{} s", sig(t))).unwrap_or_else(|| "never".into())
}

fn solve(m: &Mech) -> Solved {
    let motor = m.motor();
    let n = m.get("motors");
    let limit = m.get("limit");
    let eff = m.get("efficiency") / 100.0;
    let source = format!("{} · ReCalc", motor.source);
    match m.calc {
        Calc::Ratio => {
            let ratio = m.train();
            let o = model::ratio(motor, n, ratio, eff, limit);
            let limited = limit < motor.stall_a;
            let mut line = vec![(0.0, o.free_rpm)];
            if limited {
                line.extend([(o.limited_nm, o.limited_rpm), (o.limited_nm, 0.0)]);
            } else {
                line.push((o.stall_nm, 0.0));
            }
            Solved {
                heads: [("ratio", format!("{}:1", head(ratio)), ""), ("free", head(o.free_rpm), "rpm")],
                rows: vec![
                    ("ratio".into(), format!("{}:1", sig(ratio))),
                    ("free".into(), format!("{} rpm", sig(o.free_rpm))),
                    ("stall".into(), format!("{} N·m", sig(o.stall_nm))),
                    (format!("at {} A", trim(limit, 0)), format!("{} N·m", sig(o.limited_nm.min(o.stall_nm)))),
                    ("power".into(), format!("{} W", sig(o.peak_w))),
                    ("data".into(), source),
                ],
                plot: Plot { x_unit: "N·m", y_unit: "rpm", line, ghost: if limited { vec![(0.0, o.free_rpm), (o.stall_nm, 0.0)] } else { Vec::new() }, mark: None, level: None, exact_x: false },
            }
        }
        Calc::Drive => {
            let d = model::Drive {
                per_side: m.get("per_side"),
                ratio: m.get("ratio"),
                wheel_in: m.get("wheel"),
                mass_lb: m.get("mass"),
                mu: m.get("mu"),
                limit,
                efficiency: eff,
                speed_loss: m.get("speed_loss") / 100.0,
                distance_ft: m.get("distance"),
            };
            let o = model::drive(motor, &d);
            let at = o.time_s.map(|t| (t, o.curve.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())).map_or(0.0, |p| p.1)));
            let dist = trim(d.distance_ft, 1);
            Solved {
                heads: [("top", head(o.top_fps), "ft/s"), ("to go", o.time_s.map(head).unwrap_or_else(|| "–".into()), "s")],
                rows: vec![
                    ("free".into(), format!("{} ft/s", sig(o.free_fps))),
                    ("top".into(), format!("{} ft/s", sig(o.top_fps))),
                    ("accel".into(), format!("{} ft/s²", sig(o.accel_fps2))),
                    (format!("{dist} ft"), time(o.time_s)),
                    ("push".into(), format!("{} lbf", sig(o.push_lbf))),
                    ("held by".into(), (if o.traction_limited { "grip" } else { "limit" }).into()),
                    ("slips at".into(), format!("{} A each", sig(o.slip_a))),
                    ("data".into(), source),
                ],
                plot: Plot { x_unit: "s", y_unit: "ft/s", line: o.curve, ghost: Vec::new(), mark: at, level: Some((o.top_fps, "top")), exact_x: false },
            }
        }
        Calc::Arm => {
            let a = model::Arm {
                motors: n,
                ratio: m.get("ratio"),
                length_in: m.get("length"),
                mass_lb: m.get("mass"),
                com_in: m.get("com"),
                start_deg: m.get("start"),
                end_deg: m.get("end"),
                limit,
                efficiency: eff,
            };
            let o = model::arm(motor, &a);
            let over = o.hold_a > limit;
            Solved {
                heads: [("holding", head(o.hold_a), "A"), ("to move", o.time_s.map(head).unwrap_or_else(|| "–".into()), "s")],
                rows: vec![
                    ("holding".into(), if over { format!("{} A · over", sig(o.hold_a)) } else { format!("{} A each", sig(o.hold_a)) }),
                    ("worst at".into(), format!("{}°", trim(o.worst_deg, 0))),
                    ("gravity".into(), format!("{} N·m", sig(o.gravity_nm))),
                    ("move".into(), time(o.time_s)),
                    ("data".into(), source),
                ],
                plot: Plot { x_unit: "°", y_unit: "A", mark: Some((o.worst_deg, o.hold_a)), line: o.curve, ghost: Vec::new(), level: Some((limit, "limit")), exact_x: true },
            }
        }
        Calc::Elevator => {
            let e = model::Elevator { motors: n, ratio: m.get("ratio"), spool_in: m.get("spool"), load_lb: m.get("load"), travel_in: m.get("travel"), limit, efficiency: eff };
            let o = model::elevator(motor, &e);
            Solved {
                heads: [("speed", head(o.loaded_fps), "ft/s"), ("current", head(o.hold_a), "A")],
                rows: vec![
                    ("free".into(), format!("{} ft/s", sig(o.free_fps))),
                    ("loaded".into(), format!("{} ft/s", sig(o.loaded_fps))),
                    ("current".into(), format!("{} A each", sig(o.hold_a))),
                    ("travel".into(), time(o.time_s)),
                    ("most".into(), format!("{} lb", sig(o.max_lb))),
                    ("data".into(), source),
                ],
                plot: Plot { x_unit: "lb", y_unit: "ft/s", mark: Some((e.load_lb, o.loaded_fps)), line: o.curve, ghost: Vec::new(), level: None, exact_x: false },
            }
        }
        Calc::Flywheel => {
            let f = model::Flywheel { motors: n, ratio: m.get("ratio"), moi_lb_in2: m.get("moi"), target_rpm: m.get("target"), drop: m.get("drop") / 100.0, limit, efficiency: eff };
            let o = model::flywheel(motor, &f);
            Solved {
                heads: [("spin-up", o.spin_up_s.map(head).unwrap_or_else(|| "–".into()), "s"), ("recovery", o.recover_s.map(head).unwrap_or_else(|| "–".into()), "s")],
                rows: vec![
                    ("free".into(), format!("{} rpm", sig(o.free_rpm))),
                    ("spin-up".into(), time(o.spin_up_s)),
                    ("recovery".into(), time(o.recover_s)),
                    ("energy".into(), format!("{} J", sig(o.energy_j))),
                    ("start".into(), format!("{} A each", sig(o.start_a))),
                    ("data".into(), source),
                ],
                plot: Plot { x_unit: "s", y_unit: "rpm", mark: o.spin_up_s.map(|t| (t, f.target_rpm)), line: o.curve, ghost: Vec::new(), level: Some((f.target_rpm, "target")), exact_x: false },
            }
        }
    }
}

// ------------------------------------------------------------------ files ---

/// Where mechanisms are kept: `/data/mech` on the handheld, the settings
/// folder's `mech` in the simulator.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/data/mech")
    } else {
        crate::config_dir().join("mech")
    }
}

/// A name as a file's: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let s: String = name.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() {
        "mechanism".into()
    } else {
        s.chars().take(40).collect()
    }
}

pub fn save(mech: &Mech) -> std::io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", slug(&mech.name)));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, mech.to_json())?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every mechanism kept, by name.
pub fn saved() -> Vec<(PathBuf, Mech)> {
    let mut all: Vec<(PathBuf, Mech)> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read_to_string(&p).ok().and_then(|t| Mech::from_json(&t)).map(|m| (p, m)))
        .collect();
    all.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()));
    all
}
