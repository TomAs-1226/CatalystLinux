//! The physics under Mech: a brushed-or-brushless DC motor on 12 V, as
//! ReCalc models it (app/lib/models/Motor.ts), and the five mechanisms it
//! drives. Everything inside is SI; the page shows FRC's units.
//!
//! The motor is linear between its free and stall points:
//!   R  = V / I_stall                       (winding resistance)
//!   kV = ω_free / (V − R·I_free)           (rad/s per volt)
//!   kT = τ_stall / (I_stall − I_free)      (N·m per amp of load current)
//!   I(ω) = (V − ω/kV) / R, capped by the stator current limit
//!   τ(ω) = kT · (I(ω) − I_free)
//! so the torque is τ_stall at rest and nothing at free speed. Motion is
//! integrated with RK4 in 1 ms steps.

use std::f64::consts::TAU;

pub const VOLTS: f64 = 12.0;
pub const G: f64 = 9.806_65;
pub const INCH: f64 = 0.0254;
pub const FOOT: f64 = 0.3048;
pub const POUND: f64 = 0.453_592_37;
pub const POUND_FORCE: f64 = 4.448_221_615_260_5;
/// lb·in² in kg·m².
pub const LB_IN2: f64 = POUND * INCH * INCH;

/// The motor table (motors.tsv), sourced line by line there.
const TABLE: &str = include_str!("motors.tsv");

#[derive(Clone, Debug, PartialEq)]
pub struct Motor {
    pub name: String,
    pub free_rpm: f64,
    pub stall_nm: f64,
    pub stall_a: f64,
    pub free_a: f64,
    /// Whose dyno or spec sheet the numbers are (via ReCalc).
    pub source: String,
}

/// Every motor in the table, in its order.
pub fn motors() -> &'static [Motor] {
    static ALL: std::sync::OnceLock<Vec<Motor>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| parse_table(TABLE))
}

/// A motor by its name.
#[cfg_attr(not(test), allow(dead_code))]
pub fn motor(name: &str) -> Option<&'static Motor> {
    motors().iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

pub fn parse_table(text: &str) -> Vec<Motor> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let n = |i: usize| c.get(i).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| *v > 0.0);
            Some(Motor { name: c.first()?.trim().to_owned(), free_rpm: n(1)?, stall_nm: n(2)?, stall_a: n(3)?, free_a: n(4)?, source: c.get(5)?.trim().to_owned() })
        })
        .collect()
}

impl Motor {
    pub fn free_rad(&self) -> f64 {
        self.free_rpm * TAU / 60.0
    }
    pub fn resistance(&self) -> f64 {
        VOLTS / self.stall_a
    }
    pub fn kv(&self) -> f64 {
        self.free_rad() / (VOLTS - self.resistance() * self.free_a)
    }
    pub fn kt(&self) -> f64 {
        self.stall_nm / (self.stall_a - self.free_a)
    }
    /// The current drawn at a speed (rad/s), within a stator limit.
    pub fn current_at(&self, w: f64, limit: f64) -> f64 {
        ((VOLTS - w / self.kv()) / self.resistance()).min(limit)
    }
    /// The torque given at a speed (rad/s), within a stator limit.
    pub fn torque_at(&self, w: f64, limit: f64) -> f64 {
        self.kt() * (self.current_at(w, limit) - self.free_a)
    }
    /// The current a torque takes.
    pub fn current_for(&self, torque: f64) -> f64 {
        torque / self.kt() + self.free_a
    }
    /// The speed (rad/s) at which it gives a torque on full voltage.
    pub fn speed_for(&self, torque: f64) -> f64 {
        self.kv() * (VOLTS - self.current_for(torque) * self.resistance())
    }
}

/// One step of a train: what drives (teeth) and what is driven.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub kind: StageKind,
    pub driving: u32,
    pub driven: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    Gear,
    Belt,
    Chain,
}

impl StageKind {
    pub const ALL: [StageKind; 3] = [StageKind::Gear, StageKind::Belt, StageKind::Chain];
    pub fn name(self) -> &'static str {
        match self {
            StageKind::Gear => "gear",
            StageKind::Belt => "belt",
            StageKind::Chain => "chain",
        }
    }
    pub fn from_name(s: &str) -> StageKind {
        StageKind::ALL.into_iter().find(|k| k.name() == s).unwrap_or(StageKind::Gear)
    }
}

/// The train's reduction (more than 1 slows the output): the product of
/// each stage's driven over driving teeth.
pub fn reduction(stages: &[Stage]) -> f64 {
    stages.iter().filter(|s| s.driving > 0 && s.driven > 0).map(|s| s.driven as f64 / s.driving as f64).product()
}

/// What a train gives at its output.
#[derive(Clone, Copy, Debug)]
pub struct RatioOut {
    pub free_rpm: f64,
    pub stall_nm: f64,
    /// Torque at the output with every motor at its current limit.
    pub limited_nm: f64,
    /// The output speed (rpm) at which the current limit stops binding.
    pub limited_rpm: f64,
    /// The most power out (W), halfway down the curve, unlimited.
    pub peak_w: f64,
}

pub fn ratio(m: &Motor, n: f64, ratio: f64, efficiency: f64, limit: f64) -> RatioOut {
    let k = n * ratio * efficiency;
    let limited_nm = (k * m.kt() * (limit.min(m.stall_a) - m.free_a)).max(0.0);
    let limited_rpm = m.speed_for(limited_nm / k).max(0.0) / ratio * 60.0 / TAU;
    RatioOut {
        free_rpm: m.free_rpm / ratio,
        stall_nm: k * m.stall_nm,
        limited_nm,
        limited_rpm,
        peak_w: n * efficiency * m.stall_nm * m.free_rad() / 4.0,
    }
}

// ------------------------------------------------------------ drivetrain ---

#[derive(Clone, Copy, Debug)]
pub struct Drive {
    pub per_side: f64,
    pub ratio: f64,
    pub wheel_in: f64,
    pub mass_lb: f64,
    pub mu: f64,
    pub limit: f64,
    pub efficiency: f64,
    /// JVN's speed-loss constant: the share of free speed a real robot
    /// reaches on carpet (81 % in his calculator), from battery sag and
    /// friction the motor model leaves out.
    pub speed_loss: f64,
    pub distance_ft: f64,
}

#[derive(Clone, Debug)]
pub struct DriveOut {
    pub free_fps: f64,
    pub top_fps: f64,
    /// At a standstill (ft/s²).
    pub accel_fps2: f64,
    pub time_s: Option<f64>,
    pub push_lbf: f64,
    /// Whether pushing is held back by the wheels slipping (else the limit).
    pub traction_limited: bool,
    /// Per motor, at the wheels' grip.
    pub slip_a: f64,
    /// Speed (ft/s) over time, every 20 ms, to the distance and a little past.
    pub curve: Vec<(f64, f64)>,
}

pub fn drive(m: &Motor, d: &Drive) -> DriveOut {
    let n = 2.0 * d.per_side;
    let r = d.wheel_in * INCH / 2.0;
    let mass = d.mass_lb * POUND;
    let grip = d.mu * mass * G;
    let free = m.free_rad() / d.ratio * r;
    // The motor's speed axis stretched so it runs out at the robot's real
    // top speed rather than at free speed.
    let loss = d.speed_loss.clamp(0.05, 1.0);
    let wheel_force = |v: f64| n * m.torque_at(v / r * d.ratio / loss, d.limit) * d.ratio * d.efficiency / r;
    let force = |v: f64| wheel_force(v).min(grip);
    let stall = wheel_force(0.0);
    let distance = d.distance_ft * FOOT;
    let mut curve = vec![(0.0, 0.0)];
    let (mut x, mut v, mut t) = (0.0f64, 0.0f64, 0.0f64);
    let mut time = None;
    let dt = 0.001;
    while t < 10.0 && stall > 0.0 {
        // RK4 on (x, v).
        let a = |v: f64| force(v) / mass;
        let (k1v, k1x) = (a(v), v);
        let (k2v, k2x) = (a(v + 0.5 * dt * k1v), v + 0.5 * dt * k1v);
        let (k3v, k3x) = (a(v + 0.5 * dt * k2v), v + 0.5 * dt * k2v);
        let (k4v, k4x) = (a(v + dt * k3v), v + dt * k3v);
        let x0 = x;
        v += dt / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
        x += dt / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
        t += dt;
        if time.is_none() && x >= distance {
            let f = (distance - x0) / (x - x0);
            time = Some(t - dt + f * dt);
        }
        if ((t * 1000.0).round() as i64) % 20 == 0 {
            curve.push((t, v / FOOT));
        }
        if let Some(at) = time {
            if t > at * 1.25 + 0.1 {
                break;
            }
        }
    }
    let kt = m.kt();
    DriveOut {
        free_fps: free / FOOT,
        top_fps: free * loss / FOOT,
        accel_fps2: force(0.0) / mass / FOOT,
        time_s: time,
        push_lbf: stall.min(grip) / POUND_FORCE,
        traction_limited: grip <= stall,
        slip_a: grip * r / (n * d.ratio * d.efficiency * kt) + m.free_a,
        curve,
    }
}

// ------------------------------------------------------------------- arm ---

#[derive(Clone, Copy, Debug)]
pub struct Arm {
    pub motors: f64,
    pub ratio: f64,
    pub length_in: f64,
    pub mass_lb: f64,
    pub com_in: f64,
    /// Degrees from horizontal, up positive.
    pub start_deg: f64,
    pub end_deg: f64,
    pub limit: f64,
    pub efficiency: f64,
}

#[derive(Clone, Debug)]
pub struct ArmOut {
    /// Gravity's torque at the pivot at the worst angle of the move (N·m).
    pub gravity_nm: f64,
    pub worst_deg: f64,
    /// Per motor, holding still at the worst angle.
    pub hold_a: f64,
    pub time_s: Option<f64>,
    /// Holding current per motor across the move, (degrees, amps).
    pub curve: Vec<(f64, f64)>,
}

/// Its moment of inertia about the pivot: its mass spread along its length
/// as a rod, centred where its centre of mass is (parallel axis).
pub fn arm_moi(a: &Arm) -> f64 {
    let (m, l, c) = (a.mass_lb * POUND, a.length_in * INCH, a.com_in * INCH);
    m * (l * l / 12.0 + c * c)
}

pub fn arm(m: &Motor, a: &Arm) -> ArmOut {
    let mass = a.mass_lb * POUND;
    let c = a.com_in * INCH;
    let k = a.motors * a.ratio * a.efficiency;
    let hold = |deg: f64| m.current_for(mass * G * c * deg.to_radians().cos().abs() / k);
    let (lo, hi) = (a.start_deg.min(a.end_deg), a.start_deg.max(a.end_deg));
    // The worst angle in the move: the one nearest horizontal.
    let worst = [lo, hi, 0.0, 180.0, -180.0].into_iter().filter(|d| *d >= lo && *d <= hi).max_by(|x, y| x.to_radians().cos().abs().total_cmp(&y.to_radians().cos().abs())).unwrap_or(lo);
    let gravity_nm = mass * G * c * worst.to_radians().cos().abs();
    let curve: Vec<(f64, f64)> = (0..=48).map(|i| lo + (hi - lo) * i as f64 / 48.0).map(|d| (d, hold(d))).collect();
    // The move on full power from rest, start to end.
    let j = arm_moi(a);
    let dir = if a.end_deg >= a.start_deg { 1.0 } else { -1.0 };
    let goal = a.end_deg.to_radians();
    let accel = |th: f64, w: f64| (dir * k * m.torque_at(dir * w * a.ratio, a.limit) - mass * G * c * th.cos()) / j;
    let (mut th, mut w, mut t) = (a.start_deg.to_radians(), 0.0f64, 0.0f64);
    let mut time = None;
    let dt = 0.001;
    if (goal - th).abs() < 1e-9 {
        time = Some(0.0);
    }
    while time.is_none() && t < 10.0 && j > 0.0 {
        let (k1w, k1t) = (accel(th, w), w);
        let (k2w, k2t) = (accel(th + 0.5 * dt * k1t, w + 0.5 * dt * k1w), w + 0.5 * dt * k1w);
        let (k3w, k3t) = (accel(th + 0.5 * dt * k2t, w + 0.5 * dt * k2w), w + 0.5 * dt * k2w);
        let (k4w, k4t) = (accel(th + dt * k3t, w + dt * k3w), w + dt * k3w);
        let th0 = th;
        w += dt / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        th += dt / 6.0 * (k1t + 2.0 * k2t + 2.0 * k3t + k4t);
        t += dt;
        if (th - goal) * dir >= 0.0 {
            time = Some(t - dt + dt * (goal - th0) / (th - th0));
        }
        // Falling back past where it started: it cannot lift this.
        if (th - a.start_deg.to_radians()) * dir < -0.02 {
            break;
        }
    }
    ArmOut { gravity_nm, worst_deg: worst, hold_a: hold(worst), time_s: time, curve }
}

// -------------------------------------------------------------- elevator ---

#[derive(Clone, Copy, Debug)]
pub struct Elevator {
    pub motors: f64,
    pub ratio: f64,
    pub spool_in: f64,
    pub load_lb: f64,
    pub travel_in: f64,
    pub limit: f64,
    pub efficiency: f64,
}

#[derive(Clone, Debug)]
pub struct ElevatorOut {
    pub free_fps: f64,
    /// Rising at full power with the load, steady.
    pub loaded_fps: f64,
    /// Per motor, holding (and rising at a steady speed: the same torque).
    pub hold_a: f64,
    pub time_s: Option<f64>,
    /// The most it can hold within the current limit (ReCalc's stall load).
    pub max_lb: f64,
    /// Rising speed (ft/s) against the load lifted (lb), 0 to the most.
    pub curve: Vec<(f64, f64)>,
}

pub fn elevator(m: &Motor, e: &Elevator) -> ElevatorOut {
    let r = e.spool_in * INCH / 2.0;
    let k = e.motors * e.ratio * e.efficiency;
    let mass = e.load_lb * POUND;
    let per_motor = |lb: f64| lb * POUND * G * r / k;
    let speed = |lb: f64| (m.speed_for(per_motor(lb)).max(0.0) / e.ratio * r) / FOOT;
    let max_lb = k * m.kt() * (e.limit.min(m.stall_a) - m.free_a) / r / G / POUND;
    // Past the most it can hold within the limit, it stalls: the line drops.
    let mut curve: Vec<(f64, f64)> = (0..=40).map(|i| max_lb * i as f64 / 40.0).map(|lb| (lb, speed(lb))).collect();
    curve.push((max_lb, 0.0));
    // Up the travel from rest on full power.
    let travel = e.travel_in * INCH;
    let accel = |v: f64| (k * m.torque_at(v / r * e.ratio, e.limit) / r - mass * G) / mass.max(1e-6);
    let (mut x, mut v, mut t) = (0.0f64, 0.0f64, 0.0f64);
    let mut time = None;
    let dt = 0.001;
    while time.is_none() && t < 10.0 && accel(0.0) > 0.0 {
        let (k1v, k1x) = (accel(v), v);
        let (k2v, k2x) = (accel(v + 0.5 * dt * k1v), v + 0.5 * dt * k1v);
        let (k3v, k3x) = (accel(v + 0.5 * dt * k2v), v + 0.5 * dt * k2v);
        let (k4v, k4x) = (accel(v + dt * k3v), v + dt * k3v);
        let x0 = x;
        v += dt / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
        x += dt / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
        t += dt;
        if x >= travel {
            time = Some(t - dt + dt * (travel - x0) / (x - x0));
        }
    }
    ElevatorOut { free_fps: m.free_rad() / e.ratio * r / FOOT, loaded_fps: speed(e.load_lb), hold_a: m.current_for(per_motor(e.load_lb)), time_s: time, max_lb, curve }
}

// -------------------------------------------------------------- flywheel ---

#[derive(Clone, Copy, Debug)]
pub struct Flywheel {
    pub motors: f64,
    /// Motor turns per flywheel turn (under 1 speeds it up).
    pub ratio: f64,
    pub moi_lb_in2: f64,
    pub target_rpm: f64,
    /// How much a shot takes off its speed.
    pub drop: f64,
    pub limit: f64,
    pub efficiency: f64,
}

#[derive(Clone, Debug)]
pub struct FlywheelOut {
    pub free_rpm: f64,
    pub spin_up_s: Option<f64>,
    pub recover_s: Option<f64>,
    /// Kinetic energy at the target speed (J).
    pub energy_j: f64,
    /// Per motor, at the start.
    pub start_a: f64,
    /// Flywheel speed (rpm) over time, every 20 ms, from rest.
    pub curve: Vec<(f64, f64)>,
}

pub fn flywheel(m: &Motor, f: &Flywheel) -> FlywheelOut {
    let j = f.moi_lb_in2 * LB_IN2;
    let k = f.motors * f.ratio * f.efficiency;
    let target = f.target_rpm * TAU / 60.0;
    let accel = |w: f64| k * m.torque_at(w * f.ratio, f.limit) / j;
    // From one speed to another on full power, with the curve on the way.
    let run = |from: f64, to: f64, curve: &mut Vec<(f64, f64)>| -> Option<f64> {
        let (mut w, mut t) = (from, 0.0f64);
        let dt = 0.0005;
        if from >= to {
            return Some(0.0);
        }
        while t < 20.0 {
            let k1 = accel(w);
            let k2 = accel(w + 0.5 * dt * k1);
            let k3 = accel(w + 0.5 * dt * k2);
            let k4 = accel(w + dt * k3);
            let w0 = w;
            w += dt / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
            t += dt;
            if ((t * 2000.0).round() as i64) % 40 == 0 {
                curve.push((t, w * 60.0 / TAU));
            }
            if w >= to {
                return Some(t - dt + dt * (to - w0) / (w - w0));
            }
            if w - w0 < 1e-9 {
                return None;
            }
        }
        None
    };
    let mut curve = vec![(0.0, 0.0)];
    let free_rpm = m.free_rpm / f.ratio;
    let reachable = f.target_rpm < free_rpm * 0.999;
    let spin_up_s = if reachable && j > 0.0 { run(0.0, target, &mut curve) } else { None };
    // Then held there, as its controller holds it, a while longer.
    if let Some(t0) = spin_up_s {
        curve.push((t0, f.target_rpm));
        curve.push((t0 * 1.5, f.target_rpm));
    }
    let recover_s = if reachable && j > 0.0 { run(target * (1.0 - f.drop.clamp(0.0, 1.0)), target, &mut Vec::new()) } else { None };
    FlywheelOut { free_rpm, spin_up_s, recover_s, energy_j: 0.5 * j * target * target, start_a: m.current_at(0.0, f.limit), curve }
}
