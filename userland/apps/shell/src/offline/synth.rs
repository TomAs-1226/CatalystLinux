//! Match logs made up as a robot running FRC Catalyst writes them, with
//! faults put in at known times: the demo robot's event, the offline
//! assistant's evaluation, and the tests of both.
//!
//! A match here runs as one on the field does: a few seconds disabled, auto
//! for 20 seconds, a half-second pause, teleop for 2:20, then disabled. The
//! entries are named as DataLogManager records Catalyst's telemetry
//! (`NT:/Catalyst/…`), with the battery also logged bare as Catalyst's own
//! sink does, WPILib's driver station entries (`DS:…`) and the FMS table.
//! Every value that a fault is about is set exactly at its time, so a test
//! knows what the analysis must find.

use std::collections::HashMap;

/// Seconds disabled before the match starts.
pub const PRE: f64 = 6.0;
const AUTO: f64 = 20.0;
const PAUSE: f64 = 0.5;
const TELEOP: f64 = 140.0;
const POST: f64 = 5.0;
/// The match second teleop starts.
pub const TELEOP_START: f64 = AUTO + PAUSE;
pub const MATCH_END: f64 = AUTO + PAUSE + TELEOP;

/// A dip of the battery: from `at`, for `dur`, held at `to` volts, while
/// the motors named draw what is given.
#[derive(Clone, Debug)]
pub struct Dip {
    pub at: f64,
    pub dur: f64,
    pub to: f64,
    pub load: Vec<(&'static str, f64)>,
}

/// A motor: its temperature from the start to its peak (at a match second),
/// its current peak (at a match second), and how it drives a mechanism.
#[derive(Clone, Debug)]
pub struct Motor {
    pub name: &'static str,
    pub temp0: f64,
    pub peak_c: f64,
    pub peak_c_at: f64,
    pub amps: f64,
    pub amps_at: f64,
    /// Drives a mechanism with a position and a setpoint (an elevator).
    pub mechanism: bool,
    /// Stalled: from, for, drawing.
    pub stall: Option<(f64, f64, f64)>,
    /// The bus and CAN id it is on.
    pub bus: usize,
    pub id: u32,
}

impl Motor {
    pub fn new(name: &'static str, temp0: f64, peak_c: f64, amps: f64) -> Self {
        Motor { name, temp0, peak_c, peak_c_at: MATCH_END - 1.0, amps, amps_at: 64.0, mechanism: false, stall: None, bus: 0, id: 10 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub enum Cause {
    /// The driver station's link dropped (the radio).
    Comms,
    Estop,
}

/// The robot disabled from `at` for `dur` (to the end, if `None`).
#[derive(Clone, Debug)]
pub struct Gap {
    pub at: f64,
    pub dur: Option<f64>,
    pub cause: Cause,
}

#[derive(Clone, Debug)]
pub struct Auto {
    pub routine: &'static str,
    /// When its command finished; `None`: still running when auto ended.
    pub done_at: Option<f64>,
    /// How far from its start it was placed: metres and degrees.
    pub start_off: Option<(f64, f64)>,
}

/// A mechanism short of its setpoint: from `at` for `dur`, asked `setpoint`
/// metres, held at `reached`.
#[derive(Clone, Debug)]
pub struct Miss {
    pub mechanism: &'static str,
    pub at: f64,
    pub dur: f64,
    pub setpoint: f64,
    pub reached: f64,
}

/// A match to write.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    /// Samples a second.
    pub hz: f64,
    pub seed: u64,
    pub code: &'static str,
    /// Resting volts, and the most it sags while driving.
    pub rest_v: f64,
    pub sag: f64,
    pub dips: Vec<Dip>,
    /// Each bus's usual utilisation while playing, and spikes: (at, for, bus, to).
    pub can: [f64; 5],
    pub can_spikes: Vec<(f64, f64, usize, f64)>,
    /// A bus down: (at, for, bus, bus-offs).
    pub can_down: Vec<(f64, f64, usize, u32)>,
    /// Loops that ran long: (at, ms); the worst loop otherwise stays under 10.
    pub overruns: Vec<(f64, f64)>,
    /// The processor: usual percent, and high stretches (at, for, percent).
    pub cpu: f64,
    pub cpu_high: Vec<(f64, f64, f64)>,
    pub motors: Vec<Motor>,
    pub gaps: Vec<Gap>,
    pub auto: Option<Auto>,
    pub misses: Vec<Miss>,
    /// Console lines with an exception: (at, the line).
    pub errors: Vec<(f64, &'static str)>,
    /// Motors that dropped off their bus: (name, at, for).
    pub lost: Vec<(&'static str, f64, f64)>,
    /// Health checks that fire besides those the values make: (subsystem,
    /// id, at, for, detail).
    pub checks: Vec<(&'static str, &'static str, f64, f64, &'static str)>,
    /// Values logged besides those the faults are in: a real robot logs
    /// hundreds (each a slow wave, under `Robot/`).
    pub extra: usize,
    /// The FMS's match type and number (2: qualification).
    pub fms: Option<(i64, i64)>,
    /// On the red alliance (the FMS's `IsRedAlliance`, logged with the match).
    pub red: bool,
}

impl Spec {
    /// A quiet match: nothing wrong, the usual four motors.
    pub fn quiet(name: &str, seed: u64) -> Self {
        let mut elevator = Motor::new("Elevator", 31.0, 52.0, 36.0);
        elevator.mechanism = true;
        elevator.id = 13;
        let mut drive = Motor::new("FrontLeftDrive", 30.0, 48.0, 58.0);
        drive.id = 1;
        let mut shooter = Motor::new("Shooter", 30.0, 44.0, 38.0);
        shooter.id = 20;
        let mut intake = Motor::new("Intake", 29.0, 41.0, 30.0);
        intake.id = 16;
        Spec {
            name: name.to_owned(),
            hz: 20.0,
            seed,
            code: "4f2c9e1a7b",
            rest_v: 12.7,
            sag: 1.6,
            dips: Vec::new(),
            can: [0.38, 0.12, 0.0, 0.21, 0.0],
            can_spikes: Vec::new(),
            can_down: Vec::new(),
            overruns: Vec::new(),
            cpu: 34.0,
            cpu_high: Vec::new(),
            motors: vec![elevator, drive, shooter, intake],
            gaps: Vec::new(),
            auto: Some(Auto { routine: "TwoPiece", done_at: Some(15.0), start_off: None }),
            misses: Vec::new(),
            errors: Vec::new(),
            lost: Vec::new(),
            checks: Vec::new(),
            extra: 0,
            fms: crate::offline::facts::label_from_name(name).and_then(|l| l.strip_prefix('Q').and_then(|n| n.parse().ok())).map(|n| (2, n)),
            red: false,
        }
    }

    pub fn motor(&mut self, name: &str) -> &mut Motor {
        self.motors.iter_mut().find(|m| m.name == name).expect("a motor of the spec")
    }
}

/// A small, steady pseudo-random wobble: the same for the same seed.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// A log being written: entries started as first used.
struct Log {
    out: Vec<u8>,
    ids: HashMap<String, u32>,
}

impl Log {
    fn new() -> Self {
        let mut out = Vec::with_capacity(1 << 20);
        out.extend_from_slice(b"WPILOG");
        out.extend_from_slice(&0x0100u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        Log { out, ids: HashMap::new() }
    }

    fn record(&mut self, entry: u32, t: f64, payload: &[u8]) {
        self.out.push(0b0111_1111);
        self.out.extend_from_slice(&entry.to_le_bytes());
        self.out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&((t * 1e6).round() as u64).to_le_bytes());
        self.out.extend_from_slice(payload);
    }

    fn id(&mut self, name: &str, kind: &str, t: f64) -> u32 {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.ids.len() as u32 + 1;
        let mut p = vec![0u8];
        p.extend_from_slice(&id.to_le_bytes());
        for text in [name, kind, ""] {
            p.extend_from_slice(&(text.len() as u32).to_le_bytes());
            p.extend_from_slice(text.as_bytes());
        }
        self.record(0, t, &p);
        self.ids.insert(name.to_owned(), id);
        id
    }

    fn double(&mut self, name: &str, t: f64, v: f64) {
        let id = self.id(name, "double", t);
        self.record(id, t, &v.to_le_bytes());
    }

    fn int(&mut self, name: &str, t: f64, v: i64) {
        let id = self.id(name, "int64", t);
        self.record(id, t, &v.to_le_bytes());
    }

    fn flag(&mut self, name: &str, t: f64, v: bool) {
        let id = self.id(name, "boolean", t);
        self.record(id, t, &[v as u8]);
    }

    fn text(&mut self, name: &str, t: f64, v: &str) {
        let id = self.id(name, "string", t);
        self.record(id, t, v.as_bytes());
    }

    fn doubles(&mut self, name: &str, t: f64, v: &[f64]) {
        let id = self.id(name, "double[]", t);
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.record(id, t, &bytes);
    }

    fn raw(&mut self, name: &str, kind: &str, t: f64, bytes: &[u8]) {
        let id = self.id(name, kind, t);
        self.record(id, t, bytes);
    }

    fn texts(&mut self, name: &str, t: f64, v: &[String]) {
        let id = self.id(name, "string[]", t);
        let mut bytes = (v.len() as u32).to_le_bytes().to_vec();
        for s in v {
            bytes.extend_from_slice(&(s.len() as u32).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        self.record(id, t, &bytes);
    }
}

const NT: &str = "NT:/Catalyst/";

/// Whether match second `m` falls in [at, at + dur).
fn within(m: f64, at: f64, dur: f64) -> bool {
    m >= at - 1e-6 && m < at + dur - 1e-6
}

/// Write a match's log.
pub fn write(spec: &Spec) -> Vec<u8> {
    let mut log = Log::new();
    let mut noise = Noise(spec.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let step = 1.0 / spec.hz;
    let total = PRE + MATCH_END + POST;
    let steps = (total * spec.hz).round() as u64;
    // The match second nearest each sample, so a fault set at a second lands on one.
    let on_grid = |m: f64, at: f64| (m - at).abs() < step * 0.5;
    let phase = spec.seed as f64 * 0.37;

    // Once, at the start.
    log.text(&format!("{NT}Robot/Software/RobotCodeVersion"), 0.0, spec.code);
    log.double(&format!("{NT}Systemcore/BrownoutVolts"), 0.0, 6.8);
    log.int(&format!("{NT}Devices/Motors/Expected"), 0.0, spec.motors.len() as i64);
    if let Some((kind, number)) = spec.fms {
        log.int("NT:/FMSInfo/MatchType", 0.0, kind);
        log.int("NT:/FMSInfo/MatchNumber", 0.0, number);
        log.text("NT:/FMSInfo/EventName", 0.0, "NYRO");
        log.flag("NT:/FMSInfo/IsRedAlliance", 0.0, spec.red);
    }
    if let Some(auto) = &spec.auto {
        if let Some((metres, degrees)) = auto.start_off {
            log.double(&format!("{NT}Auto/StartCheck/DistanceMeters"), PRE - 1.0, metres);
            log.double(&format!("{NT}Auto/StartCheck/HeadingErrorDeg"), PRE - 1.0, degrees);
        } else {
            log.double(&format!("{NT}Auto/StartCheck/DistanceMeters"), PRE - 1.0, 0.03);
            log.double(&format!("{NT}Auto/StartCheck/HeadingErrorDeg"), PRE - 1.0, 1.0);
        }
    }

    let mut last_enabled = None;
    let mut last_auto = None;
    let mut last_estop = None;
    let mut last_fms = None;
    let mut last_command = String::new();
    let mut last_state: HashMap<&str, String> = HashMap::new();
    let mut checks: HashMap<String, bool> = HashMap::new();
    let mut busoffs = [0u32; 5];
    let mut enabled_s = 0.0;
    let mut errors_said = vec![false; spec.errors.len()];
    let extra: Vec<String> = (0..spec.extra).map(|j| format!("{NT}Robot/Group{}/Value{}", j / 10, j % 10)).collect();

    for k in 0..steps {
        let t = k as f64 * step;
        let m = t - PRE;
        // Enabled: auto, the pause, teleop, less the gaps.
        let gap = spec.gaps.iter().find(|g| m >= g.at - 1e-6 && g.dur.is_none_or(|d| m < g.at + d - 1e-6));
        let in_match = (0.0..AUTO).contains(&m) || (TELEOP_START..MATCH_END).contains(&m);
        let enabled = in_match && gap.is_none();
        let estop = spec.gaps.iter().any(|g| g.cause == Cause::Estop && m >= g.at - 1e-6);
        let attached = !matches!(gap, Some(Gap { cause: Cause::Comms, .. }));
        if enabled {
            enabled_s += step;
        }

        if last_enabled != Some(enabled) {
            log.flag("DS:enabled", t, enabled);
            last_enabled = Some(enabled);
        }
        // The driver station sits in auto until auto ends.
        let auto_now = m < AUTO;
        if last_auto != Some(auto_now) {
            log.flag("DS:autonomous", t, auto_now);
            last_auto = Some(auto_now);
        }
        if last_estop != Some(estop) {
            log.flag("DS:estop", t, estop);
            last_estop = Some(estop);
        }
        let fms_bits = (enabled as i64) | ((auto_now as i64) << 1) | ((estop as i64) << 3) | (0x10 * spec.fms.is_some() as i64) | ((attached as i64) << 5);
        if last_fms != Some(fms_bits) {
            log.int("NT:/FMSInfo/FMSControlData", t, fms_bits);
            last_fms = Some(fms_bits);
        }
        // What the robot is running.
        let command = if !enabled {
            String::new()
        } else if m < AUTO {
            match &spec.auto {
                Some(a) if a.done_at.is_none_or(|d| m < d) => format!("Auto:{}", a.routine),
                _ => String::new(),
            }
        } else {
            "Teleop".to_owned()
        };
        if command != last_command {
            log.text(&format!("{NT}OpMode/Command"), t, &command);
            log.flag(&format!("{NT}OpMode/Running"), t, enabled);
            last_command = command;
        }
        if enabled && (m < 0.05 || (m - TELEOP_START).abs() < step * 0.5) {
            log.text(&format!("{NT}OpMode/Active"), t, if m < AUTO { "Auto" } else { "Teleop" });
        }
        if enabled {
            log.double(&format!("{NT}Match/TimeLeft"), t, if m < AUTO { AUTO - m } else { (MATCH_END - m).max(0.0) });
        }

        // The battery: resting, sagging under load, and any dip.
        let wobble = noise.next();
        let driving = if enabled { spec.sag * (0.45 + 0.5 * ((m * 0.9 + phase).sin().abs())) + 0.04 * wobble } else { 0.02 * wobble };
        let mut volts = spec.rest_v - driving.max(0.0);
        let dip = spec.dips.iter().find(|d| within(m, d.at, d.dur));
        if let Some(d) = dip {
            volts = d.to;
        } else if let Some(d) = spec.dips.iter().find(|d| within(m, d.at - 0.2, 0.2) || within(m, d.at + d.dur, 0.2)) {
            // The edges of a dip: on the way down and back up, not below it.
            volts = volts.min(d.to + 0.9);
        }
        for name in [format!("{NT}Systemcore/BatteryVolts"), "Systemcore/BatteryVolts".to_owned()] {
            log.double(&name, t, volts);
        }
        log.flag(&format!("{NT}Systemcore/BrownedOut"), t, volts < 6.8);

        // The loop.
        let mut loop_ms = 4.6 + 1.1 * (m * 2.7 + phase).sin() + 0.5 * wobble.abs();
        if let Some((_, ms)) = spec.overruns.iter().find(|(at, _)| on_grid(m, *at)) {
            loop_ms = *ms;
        }
        log.double(&format!("{NT}Loop/Robot/LastMs"), t, loop_ms);

        // The processor.
        let mut cpu = spec.cpu + 6.0 * (m * 0.21 + phase).sin().abs();
        if let Some((at, dur, high)) = spec.cpu_high.iter().find(|(at, dur, _)| within(m, *at, *dur)) {
            cpu = if on_grid(m, at + dur * 0.5) { *high } else { *high - 0.2 - 1.5 * wobble.abs() };
        }
        log.double(&format!("{NT}Systemcore/CpuPercent"), t, cpu);

        // CAN: each bus busier while playing, the spikes, a bus down.
        let mut util = [0.0; 5];
        for (bus, base) in spec.can.iter().enumerate() {
            let busy = if enabled { base * (0.88 + 0.1 * (m * 0.4 + bus as f64).sin().abs()) } else { base * 0.45 };
            util[bus] = busy;
        }
        for (at, dur, bus, to) in &spec.can_spikes {
            if within(m, *at, *dur) {
                util[*bus] = if on_grid(m, at + dur * 0.5) { *to } else { (*to - 0.03).max(util[*bus]) };
            }
        }
        let mut down = [false; 5];
        for (at, dur, bus, offs) in &spec.can_down {
            if within(m, *at, *dur) {
                down[*bus] = true;
                util[*bus] = 0.0;
            }
            if on_grid(m, *at) {
                busoffs[*bus] += offs;
            }
        }
        log.doubles(&format!("{NT}Systemcore/CanUtilization"), t, &util);
        log.flag(&format!("{NT}Systemcore/CanDown"), t, down.iter().any(|d| *d));
        if k % (spec.hz as u64 / 4).max(1) == 0 {
            for bus in 0..5 {
                if spec.can[bus] > 0.0 {
                    let name = format!("can_s{bus}");
                    log.flag(&format!("{NT}CAN/Health/{name}/OK"), t, !down[bus]);
                    log.int(&format!("{NT}CAN/Health/{name}/BusOffCount"), t, i64::from(busoffs[bus]));
                }
            }
        }

        // The motors and the mechanism they drive.
        let mut rows = Vec::new();
        let mut connected = 0;
        for (i, motor) in spec.motors.iter().enumerate() {
            let lost = spec.lost.iter().any(|(name, at, dur)| *name == motor.name && within(m, *at, *dur));
            rows.push(format!("{}|can_s{}|{}|{}", motor.name, motor.bus, motor.id, !lost));
            if !lost {
                connected += 1;
            }
            let progress = if motor.peak_c_at > 0.0 { (enabled_s.min(motor.peak_c_at + step) / motor.peak_c_at).min(1.0) } else { 1.0 };
            let mut temp = motor.temp0 + (motor.peak_c - motor.temp0) * progress;
            if m > motor.peak_c_at {
                temp -= (m - motor.peak_c_at) * 0.04;
            }
            if on_grid(m, motor.peak_c_at) {
                temp = motor.peak_c;
            } else {
                temp = temp.min(motor.peak_c - 0.2);
            }
            let mut amps = if enabled && !lost { motor.amps * (0.35 + 0.45 * ((m * 1.3 + i as f64 + phase).sin().abs())) } else { 0.0 };
            if enabled && on_grid(m, motor.amps_at) {
                amps = motor.amps;
            }
            for d in &spec.dips {
                if let Some((_, a)) = d.load.iter().find(|(n, _)| *n == motor.name) {
                    if within(m, d.at, d.dur) {
                        amps = *a;
                    }
                }
            }
            let mut velocity = if enabled { 0.6 * (m * 0.8 + i as f64).sin() + 0.25 } else { 0.0 };
            if let Some((at, dur, stall_amps)) = motor.stall {
                if within(m, at, dur) && enabled {
                    amps = stall_amps;
                    velocity = 0.0;
                }
            }
            if lost {
                continue;
            }
            if motor.mechanism {
                log.double(&format!("{}/VelocityMPS", motor.name), t, velocity);
            }
            log.double(&format!("{}/TemperatureC", motor.name), t, temp);
            log.double(&format!("{}/StatorCurrentAmps", motor.name), t, amps);
            // Health: the motor's own checks.
            let hot = temp > 70.0;
            set_check(&mut log, &mut checks, t, motor.name, "HighTemp", hot, &format!("{temp:.0} °C, limit 70"));
            if motor.mechanism {
                let miss = spec.misses.iter().find(|x| x.mechanism == motor.name && within(m, x.at, x.dur) && enabled);
                let (state, setpoint, position, at_setpoint) = match miss {
                    Some(x) => (format!("GoTo {:.2}m", x.setpoint), x.setpoint, x.reached, false),
                    None => {
                        let goal = 0.4 + 0.4 * ((m / 9.0).floor() % 2.0);
                        ("Hold".to_owned(), goal, goal + 0.004 * wobble, true)
                    }
                };
                if last_state.get(motor.name) != Some(&state) {
                    log.text(&format!("{}/State", motor.name), t, &state);
                    last_state.insert(motor.name, state);
                }
                log.double(&format!("{}/SetpointMeters", motor.name), t, setpoint);
                log.double(&format!("{}/PositionMeters", motor.name), t, position);
                log.flag(&format!("{}/AtSetpoint", motor.name), t, at_setpoint || !enabled);
            }
        }
        if k % (spec.hz as u64 / 2).max(1) == 0 {
            log.texts(&format!("{NT}Devices/Motors/Rows"), t, &rows);
            log.int(&format!("{NT}Devices/Motors/Connected"), t, connected);
        }
        // CAN's own checks: busy, and down.
        for (bus, u) in util.iter().enumerate() {
            if spec.can[bus] > 0.0 {
                set_check(&mut log, &mut checks, t, "CAN", &format!("can_s{bus}/HighUtilization"), *u > 0.55, &format!("{:.0}%", u * 100.0));
                set_check(&mut log, &mut checks, t, "CAN", &format!("can_s{bus}/Down"), down[bus], "bus off");
            }
        }
        for (subsystem, id, at, dur, detail) in &spec.checks {
            set_check(&mut log, &mut checks, t, subsystem, id, within(m, *at, *dur), detail);
        }
        // Console lines.
        for (i, (at, line)) in spec.errors.iter().enumerate() {
            if !errors_said[i] && m >= *at - 1e-6 {
                log.text("messages", t, line);
                errors_said[i] = true;
            }
        }
        for (j, name) in extra.iter().enumerate() {
            log.double(name, t, (m * 0.1 + j as f64).sin());
        }
        // Something else every robot logs, for the reader to step over.
        log.double(&format!("{NT}Swerve/HeadingDeg"), t, (m * 11.0 + phase * 40.0).rem_euclid(360.0));
        log.double(&format!("{NT}Swerve/SpeedMPS"), t, if enabled { 1.8 + 1.2 * (m * 0.5).sin() } else { 0.0 });
        // Where it is on the field: AdvantageKit's odometry pose, or (every
        // other match) WPILib's Field2d, as DataLogManager records them.
        let pose = crate::field::demo_match(m as f32, spec.seed, spec.red);
        let (x, y, heading) = (pose.x as f64, pose.y as f64, pose.heading as f64);
        if spec.seed % 2 == 0 {
            let bytes: Vec<u8> = [x, y, heading].iter().flat_map(|v| v.to_le_bytes()).collect();
            log.raw("NT:/AdvantageKit/RealOutputs/Odometry/Robot", "struct:Pose2d", t, &bytes);
        } else {
            log.doubles("NT:/SmartDashboard/Field/Robot", t, &[x, y, heading.to_degrees()]);
        }
    }
    log.out
}

/// A health check's flag, logged as it changes, with its detail as it fires.
fn set_check(log: &mut Log, checks: &mut HashMap<String, bool>, t: f64, subsystem: &str, id: &str, firing: bool, detail: &str) {
    let key = format!("{subsystem}/{id}");
    if checks.get(&key) == Some(&firing) {
        return;
    }
    if firing {
        log.text(&format!("{NT}Health/{key}/detail"), t, detail);
        log.text(&format!("{NT}Health/{key}/severity"), t, "WARN");
    }
    log.flag(&format!("{NT}Health/{key}/firing"), t, firing);
    checks.insert(key, firing);
}

// ------------------------------------------------------------ the demo ---

/// The demo robot's event: a day and a half of qualifying at NYRO in which
/// the robot slowly gets worse (the battery sags further, the elevator runs
/// hotter, the loop starts to overrun) until the newest match browns out, a
/// match after new code went on. Newest first, as Logs lists them.
pub fn demo_event() -> Vec<Spec> {
    // (name, battery low, overruns (at, ms), elevator °C and A, CAN peak, code)
    let rows: [(&str, f64, &[(f64, f64)], (f64, f64), f64, &str); 6] = [
        ("FRC_20261116_095800_NYRO_Q12.wpilog", 9.8, &[(46.0, 22.0), (88.0, 24.0), (131.0, 21.5)], (74.0, 49.0), 0.58, "b71d2a0c33"),
        ("FRC_20261116_093200_NYRO_Q9.wpilog", 10.5, &[], (58.0, 41.0), 0.47, "b71d2a0c33"),
        ("FRC_20261115_164000_NYRO_Q7.wpilog", 10.7, &[], (56.0, 40.0), 0.44, "b71d2a0c33"),
        ("FRC_20261115_151200_NYRO_Q5.wpilog", 10.9, &[(71.0, 21.4)], (53.0, 38.0), 0.42, "93e0f5d871"),
        ("FRC_20261115_135000_NYRO_Q3.wpilog", 11.0, &[], (54.0, 37.0), 0.41, "93e0f5d871"),
        ("FRC_20261115_110500_NYRO_Q1.wpilog", 11.2, &[], (51.0, 36.0), 0.40, "93e0f5d871"),
    ];
    let mut out = vec![demo_q14()];
    for (i, (name, low, overruns, (temp, amps), can, code)) in rows.into_iter().enumerate() {
        let mut s = Spec::quiet(name, 40 + i as u64);
        s.hz = 10.0;
        s.code = code;
        s.sag = 1.2;
        s.dips.push(Dip { at: 97.0, dur: 0.3, to: low, load: vec![("FrontLeftDrive", 55.0), ("Elevator", amps - 4.0)] });
        s.overruns = overruns.to_vec();
        let e = s.motor("Elevator");
        e.peak_c = temp;
        e.amps = amps;
        e.amps_at = 83.0;
        let shooter = s.motor("Shooter");
        shooter.peak_c = temp - 9.0;
        s.can_spikes.push((84.0, 0.6, 0, can));
        s.auto = Some(Auto { routine: "ThreePiece", done_at: Some(16.5 + i as f64 * 0.2), start_off: None });
        // The alliance each was played on (Q9 as the demo schedule has it).
        s.red = matches!(i, 0 | 1 | 3 | 5);
        out.push(s);
    }
    out
}

/// Q14, the newest: a brownout at 1:18 while the drive and the elevator
/// pulled hard, the elevator over 80 °C, CAN can_s0 at 72%, the loop
/// overrunning in the scramble.
fn demo_q14() -> Spec {
    let mut s = Spec::quiet("FRC_20261116_101500_NYRO_Q14.wpilog", 14);
    s.hz = 10.0;
    s.rest_v = 12.4;
    s.sag = 1.9;
    s.dips.push(Dip { at: 78.0, dur: 0.4, to: 6.3, load: vec![("FrontLeftDrive", 71.0), ("Elevator", 62.0), ("Intake", 24.0)] });
    s.dips.push(Dip { at: 112.0, dur: 0.5, to: 7.7, load: vec![("FrontLeftDrive", 64.0)] });
    s.overruns = vec![(41.0, 23.5), (63.0, 22.1), (77.9, 41.0), (78.2, 34.0), (79.0, 26.0), (102.0, 23.0), (133.0, 22.4)];
    s.can_spikes.push((76.0, 6.0, 0, 0.72));
    let e = s.motor("Elevator");
    e.peak_c = 81.0;
    e.peak_c_at = 152.0;
    e.amps = 62.0;
    e.amps_at = 78.1;
    s.motor("Shooter").peak_c = 72.0;
    s.auto = Some(Auto { routine: "ThreePiece", done_at: Some(17.1), start_off: None });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::facts::{analyse, Kind};

    #[test]
    fn a_quiet_match_reads_back_as_quiet() {
        let spec = Spec::quiet("FRC_20261116_101500_NYRO_Q3.wpilog", 3);
        let bytes = write(&spec);
        let f = analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap();
        assert_eq!(f.label, "Q3");
        assert!(!f.truncated);
        assert_eq!(f.enabled_seconds.map(f64::round), Some(160.0));
        assert!(f.facts.iter().all(|x| x.level == crate::wpilog::Level::Fine), "{:#?}", f.facts.iter().filter(|x| x.level != crate::wpilog::Level::Fine).collect::<Vec<_>>());
        assert_eq!(f.routine.as_deref(), Some("TwoPiece"));
        assert_eq!(f.code.as_deref(), Some("4f2c9e1"));
        // The battery and the loop said once each, from both copies of the battery.
        assert_eq!(f.metric("overruns"), Some(0.0));
        assert!(f.of_kind(Kind::Auto).next().unwrap().text.contains("finished at 0:15"));
    }

    #[test]
    fn faults_put_in_are_found_where_they_were_put() {
        let mut spec = Spec::quiet("FRC_20261116_101500_NYRO_Q31.wpilog", 31);
        spec.dips.push(Dip { at: 78.0, dur: 0.4, to: 6.4, load: vec![("FrontLeftDrive", 70.0), ("Elevator", 55.0)] });
        spec.gaps.push(Gap { at: 100.0, dur: Some(4.0), cause: Cause::Comms });
        spec.can_down.push((65.0, 2.5, 1, 3));
        spec.can[1] = 0.3;
        spec.motor("Elevator").stall = Some((52.0, 1.2, 58.0));
        spec.misses.push(Miss { mechanism: "Elevator", at: 104.0 + 0.5, dur: 3.6, setpoint: 1.1, reached: 0.86 });
        spec.errors.push((33.0, "Unhandled exception: java.lang.NullPointerException: vision"));
        spec.lost.push(("Shooter", 65.0, 2.0));
        spec.auto = Some(Auto { routine: "FourPiece", done_at: None, start_off: Some((0.42, 12.0)) });
        let bytes = write(&spec);
        let f = analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap();
        let fact = |k: Kind| f.of_kind(k).next().unwrap_or_else(|| panic!("no {k:?} in {:#?}", f.facts));
        let b = fact(Kind::Brownout);
        assert_eq!((b.at, b.nums[1].value), (Some(78.0), 6.4), "{}", b.text);
        assert!(b.text.contains("at 1:18 in teleop") && b.text.contains("6.4 V"), "{}", b.text);
        let load = f.facts.iter().find(|x| x.kind == Kind::Brownout && x.subject == "load").unwrap();
        assert_eq!(load.names[..2], ["FrontLeftDrive", "Elevator"], "{}", load.text);
        let c = fact(Kind::Comms);
        assert_eq!((c.at, c.dur), (Some(100.0), Some(4.0)), "{}", c.text);
        let down = fact(Kind::CanDown);
        assert_eq!(down.subject, "can_s1");
        assert_eq!(down.at, Some(65.0));
        let errors = fact(Kind::CanErrors);
        assert!(errors.text.contains("3 bus-offs"), "{}", errors.text);
        let stall = fact(Kind::Stall);
        assert_eq!((stall.subject.as_str(), stall.at, stall.nums[0].value), ("Elevator", Some(52.0), 58.0), "{}", stall.text);
        let miss = fact(Kind::Mechanism);
        assert!(miss.text.contains("1.10 m asked, 0.86 m reached"), "{}", miss.text);
        let code = fact(Kind::CodeError);
        assert_eq!(code.names, ["NullPointerException"]);
        assert_eq!(code.at, Some(33.0));
        let lost = fact(Kind::MotorLost);
        assert_eq!((lost.subject.as_str(), lost.at), ("Shooter", Some(65.0)), "{}", lost.text);
        let auto = fact(Kind::Auto);
        assert!(auto.text.contains("still running when auto ended") && auto.text.contains("0.42 m"), "{}", auto.text);
        // A pause between auto and teleop is the match itself, not a fault.
        assert_eq!(f.of_kind(Kind::Disabled).count(), 0, "{:#?}", f.facts);
    }

    #[test]
    fn an_estop_disables_the_robot_for_the_rest_of_the_match() {
        let mut spec = Spec::quiet("FRC_20261116_101500_NYRO_Q36.wpilog", 36);
        spec.gaps.push(Gap { at: 47.0, dur: None, cause: Cause::Estop });
        let bytes = write(&spec);
        let f = analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap();
        assert_eq!(f.facts[0].kind, Kind::Estop, "{:#?}", f.facts);
        assert_eq!(f.facts[0].at, Some(47.0));
        assert!(f.facts[0].text.contains("for the rest of the match"), "{}", f.facts[0].text);
    }

    #[test]
    fn the_battery_at_the_start_is_read_just_before_the_match_not_at_boot() {
        let mut spec = Spec::quiet("FRC_20261116_101500_NYRO_Q3.wpilog", 3);
        // Just after boot the battery reads high for a second.
        spec.dips.push(Dip { at: -PRE, dur: 1.0, to: 12.95, load: vec![] });
        let bytes = write(&spec);
        let f = analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap();
        let start = f.metric("battery_start").unwrap();
        assert!((start - 12.7).abs() < 0.05, "{start}");
    }

    #[test]
    fn a_hostile_log_gathers_only_so_much() {
        // Thousands of motors, a flapping brownout flag, names a page long.
        let mut log = Log::new();
        for k in 0..6000u32 {
            let t = f64::from(k) * 0.01;
            log.double(&format!("Motor{k}/TemperatureC"), t, 90.0);
            log.flag("NT:/Catalyst/Systemcore/BrownedOut", t, k % 2 == 0);
            log.flag(&format!("NT:/Catalyst/Health/Sub{k}/Check/firing"), t, true);
            log.double(&format!("{}/TemperatureC", "x".repeat(5000)), t, 1.0);
        }
        let f = analyse(&log.out[..], "FRC_20261116_101500_NYRO_Q1.wpilog", log.out.len() as u64).unwrap();
        let sheet = f.to_json().to_string();
        assert!(sheet.len() < 400_000, "a sheet of {} bytes", sheet.len());
        assert!(f.facts.iter().all(|x| x.text.chars().count() < 400), "no fact a page long");
    }

    #[test]
    fn the_demo_digests_are_the_summaries_of_its_logs() {
        for spec in demo_event() {
            let bytes = write(&spec);
            let s = crate::wpilog::summarize(&bytes[..]).unwrap();
            assert!(!s.truncated, "{}", spec.name);
            let low = spec.dips.iter().map(|d| d.to).fold(f64::INFINITY, f64::min);
            assert!((s.battery_low.unwrap() - low).abs() < 1e-9, "{}: {:?} vs {low}", spec.name, s.battery_low);
            assert_eq!(s.loop_overruns as usize, spec.overruns.len(), "{}", spec.name);
        }
    }
}
