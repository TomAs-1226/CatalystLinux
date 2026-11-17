//! The robot as the shell sees it: FRC Catalyst's telemetry under
//! `/Catalyst/` on the robot's NT4 server, read by key.
//!
//! Catalyst's rule is "absent, never 0": a value the robot does not know is
//! not published, so every read here is an `Option` and screens show a dash.

use std::collections::{BTreeMap, VecDeque};

use nt4::{Client, Value};

use crate::logs::Logs;

const ROOT: &str = "/Catalyst/";

enum Source {
    Live(Client),
    /// A scripted robot, for developing and demonstrating the UI with no robot.
    Demo { elapsed: f64, table: BTreeMap<String, Value>, tick: u64 },
    None,
}

pub struct Robot {
    source: Source,
    pub address: String,
    /// Recent loop times in ms, sampled as they change, for the loop screen.
    pub loop_history: VecDeque<f32>,
    /// Match logs pulled from this robot.
    pub logs: Logs,
    last_generation: u64,
}

impl Robot {
    pub fn none() -> Self {
        Robot { source: Source::None, address: String::new(), loop_history: VecDeque::new(), logs: Logs::none(), last_generation: u64::MAX }
    }

    pub fn connect(address: &str) -> Self {
        let client = Client::connect(address, "catalyst-linux", &[ROOT]);
        let host = address.split(':').next().unwrap_or_default();
        Robot { source: Source::Live(client), address: address.to_owned(), logs: Logs::for_robot(host), ..Self::none() }
    }

    pub fn demo() -> Self {
        let mut robot = Robot {
            source: Source::Demo { elapsed: 0.0, table: BTreeMap::new(), tick: 0 },
            address: "demo".into(),
            logs: Logs::demo(),
            ..Self::none()
        };
        robot.refresh(0.0);
        robot
    }

    pub fn is_demo(&self) -> bool {
        matches!(self.source, Source::Demo { .. })
    }

    pub fn linked(&self) -> bool {
        match &self.source {
            Source::Live(c) => c.connected(),
            Source::Demo { .. } => true,
            Source::None => false,
        }
    }

    /// Why there is no link, in a few words.
    pub fn status(&self) -> String {
        match &self.source {
            Source::Live(c) => c.status(),
            Source::Demo { .. } => "demo robot".into(),
            Source::None => "no robot set".into(),
        }
    }

    pub fn rtt_ms(&self) -> Option<f32> {
        match &self.source {
            Source::Live(c) => c.rtt_ms(),
            Source::Demo { .. } => Some(1.8),
            Source::None => None,
        }
    }

    /// Advance the demo and sample histories. Returns whether anything
    /// changed since the last call, i.e. whether to redraw.
    pub fn refresh(&mut self, dt: f32) -> bool {
        let generation = match &mut self.source {
            Source::Live(c) => c.generation(),
            Source::Demo { elapsed, table, tick } => {
                *elapsed += dt as f64;
                let now = (*elapsed * 4.0) as u64; // 4 Hz
                if now != *tick || table.is_empty() {
                    *tick = now;
                    *table = demo_table(now as f64 / 4.0);
                }
                *tick
            }
            Source::None => 0,
        };
        // Logs are pulled only while the robot is reachable and not running an
        // opmode: a match is no time to be reading its disk.
        self.logs.set_allowed(self.linked() && self.bool("OpMode/Running") != Some(true));
        let generation = generation.wrapping_add(self.logs.generation().wrapping_mul(0x9E37_79B9));
        if generation == self.last_generation {
            return false;
        }
        self.last_generation = generation;
        if let Some(ms) = self.f64("Loop/Robot/LastMs") {
            if self.loop_history.len() >= 120 {
                self.loop_history.pop_front();
            }
            self.loop_history.push_back(ms as f32);
        } else {
            self.loop_history.clear();
        }
        true
    }

    /// A value by its key under `/Catalyst/`.
    pub fn get(&self, key: &str) -> Option<Value> {
        match &self.source {
            Source::Live(c) => c.get(&format!("{ROOT}{key}")),
            Source::Demo { table, .. } => table.get(key).cloned(),
            Source::None => None,
        }
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.as_f64())
    }
    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.as_i64())
    }
    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| v.as_bool())
    }
    pub fn string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str().map(str::to_owned))
    }
    pub fn strings(&self, key: &str) -> Vec<String> {
        self.get(key).and_then(|v| v.as_strings().map(<[String]>::to_vec)).unwrap_or_default()
    }

    /// Every published value under a key prefix, as (key under `/Catalyst/`, value).
    pub fn under(&self, prefix: &str) -> Vec<(String, Value)> {
        match &self.source {
            Source::Live(c) => c
                .with_prefix(&format!("{ROOT}{prefix}"))
                .into_iter()
                .filter_map(|(k, e)| Some((k[ROOT.len()..].to_owned(), e.value?)))
                .collect(),
            Source::Demo { table, .. } => {
                table.range(prefix.to_owned()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(k, v)| (k.clone(), v.clone())).collect()
            }
            Source::None => Vec::new(),
        }
    }

    /// The health checks that are firing, worst first: (severity, subsystem, id, detail).
    pub fn firing(&self) -> Vec<(String, String, String, String)> {
        let entries = self.under("Health/");
        let mut out = Vec::new();
        for (key, value) in &entries {
            // Health/<subsystem>/<id…>/firing; CAN check ids contain a slash.
            let Some(path) = key.strip_prefix("Health/").and_then(|k| k.strip_suffix("/firing")) else { continue };
            if value.as_bool() != Some(true) {
                continue;
            }
            let Some((subsystem, id)) = path.split_once('/') else { continue };
            let field = |name: &str| {
                let wanted = format!("Health/{path}/{name}");
                entries.iter().find(|(k, _)| *k == wanted).and_then(|(_, v)| v.as_str().map(str::to_owned)).unwrap_or_default()
            };
            out.push((field("severity"), subsystem.to_owned(), id.to_owned(), field("detail")));
        }
        let rank = |s: &str| match s {
            "ERROR" => 0,
            "WARN" => 1,
            _ => 2,
        };
        out.sort_by_key(|(severity, ..)| rank(severity));
        out
    }
}

/// What a healthy-ish Catalyst 2.0 robot publishes, with a little life in it.
fn demo_table(t: f64) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    let mut put = |k: &str, v: Value| {
        m.insert(k.to_owned(), v);
    };
    let s = |v: &str| Value::String(v.to_owned());
    let wave = |period: f64, phase: f64| ((t / period + phase) * std::f64::consts::TAU).sin();
    let hot = 68.0 + 4.5 * wave(40.0, 0.0);

    put("Robot/Identity/Name", s("Atlas"));
    put("Robot/Identity/TeamNumber", Value::Int(5805));
    put("Robot/Software/CatalystVersion", s("2.0.0"));
    put("Robot/Software/WPILibVersion", s("2027.0.0-alpha-7"));
    put("Systemcore/BatteryVolts", Value::Double(12.45 + 0.25 * wave(23.0, 0.2) - 0.1 * wave(3.1, 0.0).abs()));
    put("Systemcore/BrownedOut", Value::Bool(false));
    put("Systemcore/CpuPercent", Value::Double(31.0 + 9.0 * wave(11.0, 0.1)));
    put("Systemcore/TempCelsius", Value::Double(52.0 + 2.0 * wave(60.0, 0.4)));
    put("Systemcore/RamFraction", Value::Double(0.41));
    put("Systemcore/CanUtilization", Value::DoubleArray(vec![0.38 + 0.05 * wave(7.0, 0.0), 0.12, 0.0, 0.21, 0.0]));
    put("Loop/Robot/LastMs", Value::Double(4.2 + 1.3 * wave(1.7, 0.0) + 2.5 * wave(0.9, 0.3).max(0.0).powi(6)));
    put("Loop/Robot/AverageMs", Value::Double(4.6));
    put("Loop/Robot/MaxMs", Value::Double(11.8));
    put("Loop/Robot/OverBudget", Value::Bool(false));

    let elevator_hot = hot > 70.0;
    put("Health/ErrorCount", Value::Int(0));
    put("Health/WarnCount", Value::Int(1 + elevator_hot as i64));
    put("Health/InfoCount", Value::Int(0));
    put("Health/Healthy", Value::Bool(true));
    put("Health/Elevator/HighTemp/firing", Value::Bool(elevator_hot));
    put("Health/Elevator/HighTemp/severity", s("WARN"));
    put("Health/Elevator/HighTemp/detail", s(&format!("{hot:.0} °C, limit 70")));
    put("Health/CAN/can_s0/HighUtilization/firing", Value::Bool(true));
    put("Health/CAN/can_s0/HighUtilization/severity", s("WARN"));
    put("Health/CAN/can_s0/HighUtilization/detail", s("71% for 4 s"));
    put("Health/Intake/OverCurrent/firing", Value::Bool(false));
    put("Health/Intake/OverCurrent/severity", s("WARN"));
    put("Health/History", Value::StringArray(vec![
        "84.120|FIRED|WARN|CAN|can_s0/HighUtilization|71% for 4 s".into(),
        "61.480|CLEARED|WARN|Intake|OverCurrent|".into(),
        "58.900|FIRED|WARN|Intake|OverCurrent|38 A of 40".into(),
    ]));
    put("Alerts/Warnings", Value::StringArray(vec!["[CAN] can_s0 utilization high".into()]));
    put("Alerts/Errors", Value::StringArray(vec![]));
    put("Preflight/Ready", Value::Bool(true));
    put("Preflight/Summary", s("Ready, 1 warning"));
    put("Devices/Summary", s("4/4 cameras · 20/20 motors · Systemcore"));
    put("Devices/Motors/Expected", Value::Int(20));
    put("Devices/Motors/Connected", Value::Int(20));
    put("Devices/Cameras/Expected", Value::Int(4));
    put("Devices/Cameras/Connected", Value::Int(4));

    put("Elevator/State", s("HOLD"));
    put("Elevator/PositionMeters", Value::Double(0.62 + 0.3 * wave(9.0, 0.0)));
    put("Elevator/SetpointMeters", Value::Double(0.8));
    put("Elevator/StatorCurrentAmps", Value::Double(14.0 + 6.0 * wave(2.3, 0.0)));
    put("Elevator/TemperatureC", Value::Double(hot));
    put("Elevator/AtSetpoint", Value::Bool(wave(9.0, 0.0) > 0.5));
    put("Intake/DutyCycle", Value::Double(0.0));
    put("Intake/HasPiece", Value::Bool(false));
    put("Intake/TemperatureC", Value::Double(41.0));
    put("Shooter/PrimaryVelocityRPS", Value::Double(70.0 + 0.6 * wave(0.7, 0.0)));
    put("Shooter/SetpointRPS", Value::Double(70.0));
    put("Shooter/AtSpeed", Value::Bool(true));
    put("Swerve/HeadingDeg", Value::Double((t * 12.0) % 360.0));
    put("Swerve/SpeedMPS", Value::Double(1.9 + 1.4 * wave(6.0, 0.0)));

    put("MotorHistory/Summary", s("20 motors · 41.2 h powered"));
    put("MotorHistory/Rows", Value::StringArray(
        [("Elevator", "Kraken X60", 13, 41.2, 9.8, 61.0, 78.0), ("ElevatorFollower", "Kraken X60", 14, 41.2, 9.7, 58.5, 76.0),
         ("Shooter", "Kraken X60", 20, 38.0, 6.1, 72.0, 71.0), ("Intake", "Kraken X44", 16, 38.0, 4.4, 44.0, 63.0),
         ("FrontLeftDrive", "Kraken X60", 1, 41.2, 18.6, 96.0, 82.0), ("FrontLeftSteer", "Kraken X44", 2, 41.2, 18.6, 31.0, 55.0)]
            .iter()
            .map(|(name, model, id, powered, running, amps, temp)| {
                format!("SN{id:04}|{model}|TalonFX|can_s0|{id}|{name}|26.70.1|{:.0}|{:.0}|0|0|{amps}|{temp}|0|0|37|0|0|1|", powered * 3600.0, running * 3600.0)
            })
            .collect(),
    ));
    m
}
