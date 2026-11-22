//! What a match log says, worked out once and kept beside it: a ranked sheet
//! of facts (brownouts and sags, CAN, the loop, motors, the link to the
//! driver station, mechanisms, auto, the code), each with its numbers and
//! when in the match it happened, plus the measures matches are compared by
//! and the range of every value logged.
//!
//! The log is read as a stream, as `crate::wpilog` reads it for the digest:
//! a 50 MB match costs a few hundred kilobytes here, and a log cut short is
//! read up to where it ends. Key names are FRC Catalyst's (`/Catalyst/…`,
//! as DataLogManager records it, `NT:/Catalyst/…`, or as Catalyst's own sink
//! records it, bare), WPILib's driver station entries (`DS:enabled`,
//! `DS:autonomous`, `DS:estop`) and its FMS table (`NT:/FMSInfo/…`). A key
//! logged both ways is read once.
//!
//! Times in a fact are seconds into the match: from the robot first being
//! enabled (negative before it). A log never enabled counts from its start.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};

use serde_json::{json, Value};

use crate::wpilog::Level;

/// A sheet of an older layout is made again from its log.
pub const LAYOUT: u64 = 1;

/// Under this the battery is sagging hard; the robot controller browns out
/// at 6.8 V unless the log says otherwise (`Systemcore/BrownoutVolts`).
const SAG_V: f64 = 8.0;
const SAG_CLEAR_V: f64 = 8.4;
const BROWNOUT_V: f64 = 6.8;
/// A battery resting under this before the match was not fully charged.
const CHARGED_V: f64 = 12.3;
const LOOP_BUDGET_MS: f64 = 20.0;
const HOT_C: f64 = 70.0;
const OVERHEAT_C: f64 = 80.0;
/// Drawing this much without moving, for this long, while enabled: stalled.
const STALL_A: f64 = 40.0;
const STALL_S: f64 = 0.5;
const CAN_BUSY: f64 = 0.9;
/// A mechanism short of its setpoint this long is worth saying.
const MISS_S: f64 = 2.0;
/// REBUILT (2026): autonomous is the match's first 20 seconds.
pub const AUTO_S: f64 = 20.0;
/// The auto-to-teleop pause, and how near the end of auto it falls.
const TRANSITION_S: f64 = 3.5;
/// Values whose ranges are kept, at most.
const SIGNALS_KEPT: usize = 600;
const MAX_RECORD: u64 = 64 * 1024;
/// A log comes from the robot: whatever it holds, what is gathered from it
/// is bounded. Keys read, a key's length, motors and mechanisms, checks,
/// events of each sort, names kept.
const KEYS_KEPT: usize = 4096;
const KEY_LONGEST: usize = 160;
const NAMED_KEPT: usize = 128;
const CHECKS_KEPT: usize = 256;
const EVENTS_KEPT: usize = 500;
const TEXT_LONGEST: usize = 64;

// ------------------------------------------------------------------ facts ---

/// What a fact is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Brownout,
    Sag,
    BatteryStart,
    Battery,
    Estop,
    Comms,
    Disabled,
    CodeError,
    MotorLost,
    CanDown,
    CanErrors,
    CanBusy,
    Can,
    Heat,
    Stall,
    Current,
    Mechanism,
    Loop,
    Cpu,
    Health,
    Auto,
    Code,
    Outcome,
}

impl Kind {
    pub const ALL: [Kind; 23] = [
        Kind::Brownout,
        Kind::Sag,
        Kind::BatteryStart,
        Kind::Battery,
        Kind::Estop,
        Kind::Comms,
        Kind::Disabled,
        Kind::CodeError,
        Kind::MotorLost,
        Kind::CanDown,
        Kind::CanErrors,
        Kind::CanBusy,
        Kind::Can,
        Kind::Heat,
        Kind::Stall,
        Kind::Current,
        Kind::Mechanism,
        Kind::Loop,
        Kind::Cpu,
        Kind::Health,
        Kind::Auto,
        Kind::Code,
        Kind::Outcome,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Brownout => "brownout",
            Kind::Sag => "sag",
            Kind::BatteryStart => "battery-start",
            Kind::Battery => "battery",
            Kind::Estop => "estop",
            Kind::Comms => "comms",
            Kind::Disabled => "disabled",
            Kind::CodeError => "code-error",
            Kind::MotorLost => "motor-lost",
            Kind::CanDown => "can-down",
            Kind::CanErrors => "can-errors",
            Kind::CanBusy => "can-busy",
            Kind::Can => "can",
            Kind::Heat => "heat",
            Kind::Stall => "stall",
            Kind::Current => "current",
            Kind::Mechanism => "mechanism",
            Kind::Loop => "loop",
            Kind::Cpu => "cpu",
            Kind::Health => "health",
            Kind::Auto => "auto",
            Kind::Code => "code",
            Kind::Outcome => "outcome",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// The unit a number is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Volts,
    Amps,
    Celsius,
    Percent,
    Ms,
    Seconds,
    Count,
    Meters,
    Degrees,
    Rps,
    Plain,
}

impl Unit {
    fn name(self) -> &'static str {
        match self {
            Unit::Volts => "V",
            Unit::Amps => "A",
            Unit::Celsius => "C",
            Unit::Percent => "%",
            Unit::Ms => "ms",
            Unit::Seconds => "s",
            Unit::Count => "n",
            Unit::Meters => "m",
            Unit::Degrees => "deg",
            Unit::Rps => "rps",
            Unit::Plain => "",
        }
    }

    fn from_name(name: &str) -> Unit {
        [Unit::Volts, Unit::Amps, Unit::Celsius, Unit::Percent, Unit::Ms, Unit::Seconds, Unit::Count, Unit::Meters, Unit::Degrees, Unit::Rps].into_iter().find(|u| u.name() == name).unwrap_or(Unit::Plain)
    }
}

/// A number as a fact shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub value: f64,
    pub unit: Unit,
}

impl Num {
    pub fn new(value: f64, unit: Unit) -> Self {
        Num { value, unit }
    }

    /// "6.4 V", "58 A", "84 °C", "94%", "63 ms", "2.4 s", "1.10 m".
    pub fn shown(&self) -> String {
        let v = self.value;
        match self.unit {
            Unit::Volts => format!("{v:.1} V"),
            Unit::Amps => format!("{v:.0} A"),
            Unit::Celsius => format!("{v:.0} °C"),
            Unit::Percent => format!("{v:.0}%"),
            Unit::Ms => format!("{v:.0} ms"),
            Unit::Seconds => seconds(v),
            Unit::Count => format!("{v:.0}"),
            Unit::Meters => format!("{v:.2} m"),
            Unit::Degrees => format!("{v:.0}°"),
            Unit::Rps => format!("{v:.0} rps"),
            Unit::Plain => trimmed(v),
        }
    }
}

/// "0.4 s", "2.4 s", "41 s".
pub fn seconds(s: f64) -> String {
    if s < 9.95 {
        format!("{s:.1} s")
    } else {
        format!("{s:.0} s")
    }
}

/// A number with no more places than it needs: "3", "0.25", "12.4".
fn trimmed(v: f64) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A match time: "1:18" for 78 seconds in.
pub fn clock(at: f64) -> String {
    let s = at.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// One thing the log shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub kind: Kind,
    pub level: Level,
    /// Higher is worth saying first.
    pub score: u32,
    /// What it is about: "battery", a motor's or a mechanism's name, a CAN
    /// bus, "loop", "auto", "comms", "code".
    pub subject: String,
    /// When, in seconds into the match.
    pub at: Option<f64>,
    pub dur: Option<f64>,
    /// Its numbers, the one that answers it first.
    pub nums: Vec<Num>,
    /// Other names it mentions: motors drawing at a brownout, a routine, an
    /// exception.
    pub names: Vec<String>,
    /// One plain clause, without the match: "the robot browned out 2 times…".
    pub text: String,
}

/// A value's range over the match.
#[derive(Clone, Debug, PartialEq)]
pub struct Signal {
    pub key: String,
    pub boolean: bool,
    pub n: u64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// When it was lowest and highest, in seconds into the match.
    pub at_min: f64,
    pub at_max: f64,
    pub last: f64,
    /// For a flag: how long it was true, and how often it turned true.
    pub true_s: f64,
    pub rises: u32,
}

/// A match's sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchFacts {
    /// The log's file name, and its size when read.
    pub log: String,
    pub bytes: u64,
    /// "Q14", "P2", or the log's time when it names no match.
    pub label: String,
    pub seconds: f64,
    pub enabled_seconds: Option<f64>,
    pub code: Option<String>,
    pub routine: Option<String>,
    /// Worst first.
    pub facts: Vec<Fact>,
    /// What matches are compared by: "battery_low", "brownouts",
    /// "temp:Elevator", "can:can_s0"…
    pub metrics: BTreeMap<String, f64>,
    pub signals: Vec<Signal>,
    pub truncated: bool,
}

impl MatchFacts {
    pub fn metric(&self, name: &str) -> Option<f64> {
        self.metrics.get(name).copied()
    }

    #[cfg(test)]
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Fact> {
        self.facts.iter().filter(move |f| f.kind == kind)
    }

}

/// "Q14" from WPILib's `FRC_20261116_101500_NYRO_Q14.wpilog` (QM14 too), "P2"
/// for practice, "E3" or "F1" in the playoffs, "M5" for a double-elimination
/// match; `None` when the name carries none.
pub fn label_from_name(name: &str) -> Option<String> {
    let stem = name.trim_end_matches(".wpilog");
    let parts: Vec<&str> = stem.split('_').skip(3).collect();
    parts.into_iter().rev().find_map(|part: &str| {
        let upper = part.to_ascii_uppercase();
        let (letters, digits) = upper.split_at(upper.find(|c: char| c.is_ascii_digit())?);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let number = digits.trim_start_matches('0');
        let number = if number.is_empty() { "0" } else { number };
        let letter = match letters {
            "Q" | "QM" => "Q",
            "P" | "PM" => "P",
            "E" | "EM" => "E",
            "SF" => "SF",
            "QF" => "QF",
            "F" => "F",
            "M" => "M",
            _ => return None,
        };
        Some(format!("{letter}{number}"))
    })
}

// --------------------------------------------------------------- reading ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Double,
    Float,
    Int,
    Bool,
    Str,
    DoubleArr,
    FloatArr,
    IntArr,
    StrArr,
    Other,
}

fn ty_of(name: &str) -> Ty {
    match name {
        "double" => Ty::Double,
        "float" => Ty::Float,
        "int64" => Ty::Int,
        "boolean" => Ty::Bool,
        "string" | "json" => Ty::Str,
        "double[]" => Ty::DoubleArr,
        "float[]" => Ty::FloatArr,
        "int64[]" => Ty::IntArr,
        "string[]" => Ty::StrArr,
        _ => Ty::Other,
    }
}

/// How a mechanism's speed is measured, for telling it is not moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speed {
    MetersPerSecond,
    DegreesPerSecond,
    Rps,
}

impl Speed {
    fn still(self, v: f64) -> bool {
        v.abs()
            < match self {
                Speed::MetersPerSecond => 0.02,
                Speed::DegreesPerSecond => 3.0,
                Speed::Rps => 0.5,
            }
    }
}

/// What the sheet wants from an entry, decided once from its name and type.
#[derive(Clone, Debug, PartialEq)]
enum Role {
    Battery,
    BrownedOut,
    BrownoutVolts,
    CanUtil,
    CanDown,
    Cpu,
    TotalCurrent,
    LoopMs,
    Enabled,
    Autonomous,
    Estop,
    FmsControl,
    MatchNumber,
    MatchType,
    TimeLeft,
    Messages,
    OpModeCommand,
    OpModeError,
    AutoChoice,
    StartDistance,
    StartHeading,
    Code,
    MotorsConnected,
    MotorsExpected,
    MotorRows,
    AlertErrors,
    CanOk(String),
    CanCount(String, &'static str),
    HealthFiring(usize),
    HealthDetail(usize),
    Temp(usize),
    Current(usize),
    Velocity(usize, Speed),
    State(usize),
    AtGoal(usize),
    Setpoint(usize, Unit),
    Measured(usize),
    Generic,
}

impl Role {
    /// The roles only one key may fill (the first to give a value).
    fn single(&self) -> Option<&'static str> {
        Some(match self {
            Role::Battery => "battery",
            Role::BrownedOut => "brownedout",
            Role::CanUtil => "canutil",
            Role::LoopMs => "loop",
            Role::Enabled => "enabled",
            Role::Autonomous => "autonomous",
            Role::Estop => "estop",
            Role::FmsControl => "fms",
            Role::OpModeCommand => "command",
            Role::Code => "code",
            Role::MotorRows => "rows",
            _ => return None,
        })
    }
}

/// A key as the sheet names it: without `NT:` and `/Catalyst/`.
fn normal(name: &str) -> String {
    if name.starts_with("DS:") || name == "messages" || name == "console" {
        return name.to_owned();
    }
    let key = name.strip_prefix("NT:").unwrap_or(name).trim_start_matches('/');
    let key = key.strip_prefix("Catalyst/").unwrap_or(key);
    let key = key.strip_prefix("AdvantageKit/").unwrap_or(key);
    let key = key.strip_prefix("RealOutputs/").unwrap_or(key);
    key.to_owned()
}

/// Groups of Catalyst's own, never a mechanism.
const NOT_MECHANISMS: [&str; 16] = ["Systemcore", "Loop", "Health", "Devices", "CAN", "Alerts", "OpMode", "Robot", "Auto", "Match", "FMSInfo", "Brownout", "Status", "Preflight", "MotorHistory", "Tablet"];

/// "Elevator" from "Elevator", "FrontLeft Drive" from "Swerve/FrontLeft/Drive".
fn display(owner: &str) -> String {
    let parts: Vec<&str> = owner.split('/').collect();
    let kept: Vec<&str> = if parts.len() > 1 { parts.iter().copied().filter(|p| !matches!(*p, "Swerve" | "Mechanisms" | "Subsystems")).collect() } else { parts };
    let name = if kept.is_empty() { owner.replace('/', " ") } else { kept.join(" ") };
    name.chars().take(40).collect()
}

#[derive(Default)]
struct Stat {
    key: String,
    boolean: bool,
    n: u64,
    min: f64,
    max: f64,
    sum: f64,
    t_min: f64,
    t_max: f64,
    last: f64,
    last_t: f64,
    true_s: f64,
    rises: u32,
}

impl Stat {
    fn number(&mut self, t: f64, v: f64) {
        if !v.is_finite() {
            return;
        }
        if self.n == 0 || v < self.min {
            (self.min, self.t_min) = (v, t);
        }
        if self.n == 0 || v > self.max {
            (self.max, self.t_max) = (v, t);
        }
        self.n += 1;
        self.sum += v;
        self.last = v;
        self.last_t = t;
    }

    fn flag(&mut self, t: f64, on: bool) {
        if self.n > 0 && self.last > 0.5 {
            self.true_s += (t - self.last_t).clamp(0.0, 5.0);
        }
        if on && (self.n == 0 || self.last < 0.5) {
            self.rises += 1;
        }
        self.boolean = true;
        self.number(t, if on { 1.0 } else { 0.0 });
    }
}

/// One key as read: its role, which entry fills it (a key logged twice is
/// read from the first that gives a value), and the ranges kept of it.
struct Canon {
    ty: Ty,
    role: Role,
    owner: Option<u32>,
    stats: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Span {
    start: f64,
    end: f64,
}

impl Span {
    fn len(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// A sag of the battery: its span, its low point and when, and what was
/// drawing then.
#[derive(Clone, Debug, Default)]
struct Sag {
    span: Span,
    min: f64,
    at_min: f64,
    load: Vec<(String, f64)>,
    total: Option<f64>,
}

#[derive(Default)]
struct Motor {
    name: String,
    temp_peak: Option<(f64, f64)>,
    temp_last_t: Option<f64>,
    hot_since: Option<f64>,
    hot_s: f64,
    cur_peak: Option<(f64, f64)>,
    cur_last: Option<(f64, f64)>,
    /// The mechanism it drives, for telling a stall (a follower's is not).
    mech: Option<usize>,
    stall_open: Option<(f64, f64)>,
    stalls: Vec<(Span, f64)>,
}

#[derive(Default)]
struct Mech {
    name: String,
    vel: Option<(f64, f64, Option<Speed>)>,
    state: String,
    goal: Option<bool>,
    setpoint: Option<(f64, Unit)>,
    measured: Option<f64>,
    miss_open: Option<Miss>,
    misses: Vec<Miss>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Miss {
    span: Span,
    setpoint: Option<f64>,
    measured: Option<f64>,
    unit: Option<Unit>,
}

#[derive(Default)]
struct Check {
    subsystem: String,
    id: String,
    firing: bool,
    since: f64,
    spans: Vec<Span>,
    detail: String,
    /// The detail as it fired.
    fired_detail: String,
}

#[derive(Default)]
struct Bus {
    peak: f64,
    at_peak: f64,
    busy_s: f64,
    last: Option<(f64, f64)>,
}

struct Counter {
    first: f64,
    last: f64,
    max: f64,
    at_rise: Option<f64>,
}

/// Everything gathered while reading, before it becomes facts.
#[derive(Default)]
struct Scan {
    canons: Vec<Canon>,
    by_key: HashMap<String, usize>,
    entries: HashMap<u32, usize>,
    singles: HashMap<&'static str, usize>,
    stats: Vec<Stat>,
    first_t: Option<f64>,
    last_t: f64,
    truncated: bool,

    enabled: Option<bool>,
    enabled_changes: Vec<(f64, bool)>,
    fms_enabled_changes: Vec<(f64, bool)>,
    auto_changes: Vec<(f64, bool)>,
    estop_changes: Vec<(f64, bool)>,
    attached_changes: Vec<(f64, bool)>,
    fms_attached: bool,
    match_number: Option<i64>,
    match_type: Option<i64>,
    time_left: Option<(f64, f64)>,
    time_left_at: Vec<(f64, f64)>,

    battery_low: Option<(f64, f64)>,
    battery_before: Vec<(f64, f64)>,
    battery_first: Vec<f64>,
    sag: Option<Sag>,
    sags: Vec<Sag>,
    brownout_v: Option<f64>,
    brown_open: Option<f64>,
    browns: Vec<Span>,
    saw_brown_flag: bool,
    total_current: Option<(f64, f64)>,

    loop_worst: Option<(f64, f64)>,
    overruns: Vec<f64>,
    loop_n: u64,

    buses: Vec<Bus>,
    can_down_open: HashMap<String, f64>,
    can_down: Vec<(String, Span)>,
    counters: BTreeMap<(String, &'static str), Counter>,

    cpu_peak: Option<(f64, f64)>,
    cpu_high_s: f64,
    cpu_last: Option<(f64, f64)>,

    errors: Vec<(f64, String)>,
    error_count: u32,
    commands: Vec<(f64, String)>,
    auto_choice: Option<String>,
    start_distance: Option<f64>,
    start_heading: Option<f64>,
    code: Option<String>,

    motors_expected: Option<i64>,
    motors_connected: Vec<(f64, i64)>,
    rows_state: BTreeMap<String, (String, bool, Option<f64>)>,
    lost: Vec<(String, String, Span)>,
    alerts: Vec<(f64, String)>,

    checks: Vec<Check>,
    check_ids: HashMap<String, usize>,
    motors: Vec<Motor>,
    motor_ids: HashMap<String, usize>,
    mechs: Vec<Mech>,
    mech_ids: HashMap<String, usize>,
}

fn read_uint(r: &mut impl Read, width: usize) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf[..width])?;
    Ok(u64::from_le_bytes(buf))
}

/// The strings of a `string[]` payload: a count, then each length-prefixed.
fn strings(payload: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(count) = payload.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) else { return out };
    let mut at = 4;
    for _ in 0..count.min(512) {
        let Some(len) = payload.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else { break };
        let Some(text) = payload.get(at + 4..at + 4 + len) else { break };
        out.push(String::from_utf8_lossy(text).into_owned());
        at += 4 + len;
    }
    out
}

/// Read a whole log and work out its sheet. `log` is its file name, `bytes`
/// its size (kept, so a sheet made while it was still arriving is made again).
pub fn analyse(mut r: impl Read, log: &str, bytes: u64) -> io::Result<MatchFacts> {
    let mut magic = [0u8; 6];
    r.read_exact(&mut magic)?;
    if &magic != b"WPILOG" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a wpilog file"));
    }
    read_uint(&mut r, 2)?;
    let extra = read_uint(&mut r, 4)?;
    io::copy(&mut r.by_ref().take(extra), &mut io::sink())?;
    let mut s = Scan::default();
    let mut payload = Vec::with_capacity(256);
    loop {
        let mut head = [0u8; 1];
        match r.read(&mut head) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        let widths = ((head[0] & 3) as usize + 1, ((head[0] >> 2) & 3) as usize + 1, ((head[0] >> 4) & 7) as usize + 1);
        let record = (|| -> io::Result<(u32, u64, u64)> { Ok((read_uint(&mut r, widths.0)? as u32, read_uint(&mut r, widths.1)?, read_uint(&mut r, widths.2)?)) })();
        let Ok((entry, size, time)) = record else {
            s.truncated = true;
            break;
        };
        let wanted = entry == 0 || s.entries.contains_key(&entry);
        if !wanted || size > MAX_RECORD {
            if io::copy(&mut r.by_ref().take(size), &mut io::sink())? < size {
                s.truncated = true;
                break;
            }
            continue;
        }
        payload.resize(size as usize, 0);
        if r.read_exact(&mut payload).is_err() {
            s.truncated = true;
            break;
        }
        let t = time as f64 / 1e6;
        if entry == 0 {
            if payload.first() == Some(&0) && payload.len() >= 9 {
                let id = u32::from_le_bytes(payload[1..5].try_into().unwrap());
                let mut at = 5;
                let mut text = || -> Option<String> {
                    let len = u32::from_le_bytes(payload.get(at..at + 4)?.try_into().ok()?) as usize;
                    let out = std::str::from_utf8(payload.get(at + 4..at + 4 + len)?).ok()?.to_owned();
                    at += 4 + len;
                    Some(out)
                };
                if let (Some(name), Some(kind)) = (text(), text()) {
                    s.start(id, &name, ty_of(&kind));
                }
            }
            continue;
        }
        s.first_t.get_or_insert(t);
        s.last_t = s.last_t.max(t);
        s.value(entry, t, &payload);
    }
    Ok(s.finish(log, bytes))
}

impl Scan {
    fn start(&mut self, id: u32, name: &str, ty: Ty) {
        if name.starts_with("DS:joystick") || ty == Ty::Other || name.len() > KEY_LONGEST {
            return;
        }
        let key = normal(name);
        if let Some(&c) = self.by_key.get(&key) {
            self.entries.insert(id, c);
            return;
        }
        if self.canons.len() >= KEYS_KEPT {
            return;
        }
        let role = self.role(&key, ty);
        let mut stats = Vec::new();
        let numeric = matches!(ty, Ty::Double | Ty::Float | Ty::Int | Ty::Bool);
        if numeric && !key.starts_with("FMSInfo/") && self.stats.len() < SIGNALS_KEPT {
            stats.push(self.stats.len());
            self.stats.push(Stat { key: key.clone(), boolean: ty == Ty::Bool, ..Stat::default() });
        }
        let c = self.canons.len();
        self.canons.push(Canon { ty, role, owner: None, stats });
        self.by_key.insert(key, c);
        self.entries.insert(id, c);
    }

    fn motor(&mut self, owner: &str, variant: &str) -> usize {
        let base = display(owner);
        let name = if variant.is_empty() { base } else { format!("{base} {variant}") };
        if let Some(&i) = self.motor_ids.get(&name) {
            return i;
        }
        let mech = variant.is_empty().then(|| self.mech(owner));
        let i = self.motors.len();
        self.motors.push(Motor { name: name.clone(), mech, ..Motor::default() });
        self.motor_ids.insert(name, i);
        i
    }

    fn mech(&mut self, owner: &str) -> usize {
        if let Some(&i) = self.mech_ids.get(owner) {
            return i;
        }
        let i = self.mechs.len();
        self.mechs.push(Mech { name: display(owner), ..Mech::default() });
        self.mech_ids.insert(owner.to_owned(), i);
        i
    }

    fn check(&mut self, subsystem: &str, id: &str) -> usize {
        let key = format!("{subsystem}/{id}");
        if let Some(&i) = self.check_ids.get(&key) {
            return i;
        }
        let i = self.checks.len();
        self.checks.push(Check { subsystem: subsystem.to_owned(), id: id.to_owned(), ..Check::default() });
        self.check_ids.insert(key, i);
        i
    }

    fn role(&mut self, key: &str, ty: Ty) -> Role {
        let number = matches!(ty, Ty::Double | Ty::Float | Ty::Int);
        let flag = ty == Ty::Bool;
        match key {
            "DS:enabled" if flag => return Role::Enabled,
            "DS:autonomous" if flag => return Role::Autonomous,
            "DS:estop" if flag => return Role::Estop,
            "FMSInfo/FMSControlData" if number => return Role::FmsControl,
            "FMSInfo/MatchNumber" if number => return Role::MatchNumber,
            "FMSInfo/MatchType" if number => return Role::MatchType,
            "messages" | "console" if ty == Ty::Str => return Role::Messages,
            "Systemcore/BatteryVolts" | "Status/BatteryVolts" if number => return Role::Battery,
            "Systemcore/BrownedOut" if flag => return Role::BrownedOut,
            "Systemcore/BrownoutVolts" if number => return Role::BrownoutVolts,
            "Systemcore/CanUtilization" if number || ty == Ty::DoubleArr || ty == Ty::FloatArr => return Role::CanUtil,
            "Systemcore/CanDown" if flag => return Role::CanDown,
            "Systemcore/CanDownCount" if number => return Role::CanCount("all".into(), "down"),
            "Systemcore/CpuPercent" if number => return Role::Cpu,
            "Brownout/TotalCurrent" if number => return Role::TotalCurrent,
            "Loop/Robot/LastMs" if number => return Role::LoopMs,
            "Match/TimeLeft" if number => return Role::TimeLeft,
            "OpMode/Command" if ty == Ty::Str => return Role::OpModeCommand,
            "OpMode/Error" if ty == Ty::Str => return Role::OpModeError,
            "Auto/StartCheck/DistanceMeters" if number => return Role::StartDistance,
            "Auto/StartCheck/HeadingErrorDeg" if number => return Role::StartHeading,
            "Robot/Software/RobotCodeVersion" if ty == Ty::Str => return Role::Code,
            "Devices/Motors/Connected" if number => return Role::MotorsConnected,
            "Devices/Motors/Expected" if number => return Role::MotorsExpected,
            "Devices/Motors/Rows" if ty == Ty::StrArr => return Role::MotorRows,
            "Alerts/Errors" if ty == Ty::StrArr => return Role::AlertErrors,
            _ => {}
        }
        if ty == Ty::Str && (key.ends_with("Auto Selector/selected") || key.ends_with("Auto choices/selected")) {
            return Role::AutoChoice;
        }
        if let Some(rest) = key.strip_prefix("CAN/Health/") {
            if let Some((bus, field)) = rest.split_once('/') {
                let field = match field {
                    "OK" if flag => return Role::CanOk(bus.to_owned()),
                    "BusOffCount" => "busoff",
                    "TxFullCount" => "txfull",
                    "REC" => "rec",
                    "TEC" => "tec",
                    _ => return Role::Generic,
                };
                return if number { Role::CanCount(bus.to_owned(), field) } else { Role::Generic };
            }
        }
        if let Some(path) = key.strip_prefix("Health/") {
            let Some((path, field)) = path.rsplit_once('/') else { return Role::Generic };
            let Some((subsystem, id)) = path.split_once('/') else { return Role::Generic };
            if self.checks.len() >= CHECKS_KEPT && !self.check_ids.contains_key(&format!("{subsystem}/{id}")) {
                return Role::Generic;
            }
            return match field {
                "firing" if flag => Role::HealthFiring(self.check(subsystem, id)),
                "detail" if ty == Ty::Str => Role::HealthDetail(self.check(subsystem, id)),
                _ => Role::Generic,
            };
        }
        let Some((owner, field)) = key.rsplit_once('/') else { return Role::Generic };
        if NOT_MECHANISMS.iter().any(|g| owner == *g || owner.starts_with(&format!("{g}/"))) || owner.starts_with("DS:") {
            return Role::Generic;
        }
        let known = self.mech_ids.contains_key(owner) || self.motor_ids.contains_key(&display(owner));
        if !known && (self.motors.len() >= NAMED_KEPT || self.mechs.len() >= NAMED_KEPT) {
            return Role::Generic;
        }
        let variant = |prefix: &str| match prefix {
            "" | "Primary" => "",
            "Follower" => "follower",
            "Secondary" => "secondary",
            _ => "?",
        };
        if number {
            for (suffix, temp) in [("TemperatureC", true), ("Temperature", true), ("StatorCurrentAmps", false)] {
                if let Some(prefix) = field.strip_suffix(suffix) {
                    let v = variant(prefix);
                    if v == "?" {
                        break;
                    }
                    let m = self.motor(owner, v);
                    return if temp { Role::Temp(m) } else { Role::Current(m) };
                }
            }
            if field == "Current" && self.by_key.contains_key(&format!("{owner}/Temperature")) {
                return Role::Current(self.motor(owner, ""));
            }
            let speed = match field {
                "VelocityMPS" => Some(Speed::MetersPerSecond),
                "AngularVelocityDPS" => Some(Speed::DegreesPerSecond),
                "VelocityRPS" | "PrimaryVelocityRPS" => Some(Speed::Rps),
                _ => None,
            };
            if let Some(speed) = speed {
                return Role::Velocity(self.mech(owner), speed);
            }
            match field {
                "SetpointMeters" => return Role::Setpoint(self.mech(owner), Unit::Meters),
                "SetpointDegrees" => return Role::Setpoint(self.mech(owner), Unit::Degrees),
                "SetpointRPS" | "PrimarySetpointRPS" => return Role::Setpoint(self.mech(owner), Unit::Rps),
                "PositionMeters" | "AngleDegrees" => return Role::Measured(self.mech(owner)),
                _ => {}
            }
        }
        match field {
            "State" if ty == Ty::Str => Role::State(self.mech(owner)),
            "AtSetpoint" | "AtSpeed" if flag => Role::AtGoal(self.mech(owner)),
            _ => Role::Generic,
        }
    }

    fn enabled_now(&self) -> bool {
        self.enabled == Some(true)
    }

    fn value(&mut self, entry: u32, t: f64, payload: &[u8]) {
        let Some(&c) = self.entries.get(&entry) else { return };
        match self.canons[c].owner {
            None => self.canons[c].owner = Some(entry),
            Some(owner) if owner != entry => return,
            _ => {}
        }
        let role = self.canons[c].role.clone();
        if let Some(tag) = role.single() {
            if *self.singles.entry(tag).or_insert(c) != c {
                return;
            }
        }
        let ty = self.canons[c].ty;
        let number = match ty {
            Ty::Double => payload.get(..8).map(|b| f64::from_le_bytes(b.try_into().unwrap())),
            Ty::Float => payload.get(..4).map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap()))),
            Ty::Int => payload.get(..8).map(|b| i64::from_le_bytes(b.try_into().unwrap()) as f64),
            Ty::Bool => payload.first().map(|b| if *b != 0 { 1.0 } else { 0.0 }),
            _ => None,
        };
        if let Some(v) = number.filter(|_| role != Role::CanUtil) {
            if let Some(&si) = self.canons[c].stats.first() {
                if ty == Ty::Bool {
                    self.stats[si].flag(t, v > 0.5);
                } else {
                    self.stats[si].number(t, v);
                }
            }
        }
        let text = || String::from_utf8_lossy(payload).into_owned();
        match role {
            Role::Generic => {}
            Role::Battery => {
                if let Some(v) = number.filter(|v| *v > 0.5) {
                    self.battery(t, v);
                }
            }
            Role::BrownedOut => {
                let on = number == Some(1.0);
                self.saw_brown_flag = true;
                match (on, self.brown_open) {
                    (true, None) => self.brown_open = Some(t),
                    (false, Some(start)) => {
                        keep(&mut self.browns, Span { start, end: t });
                        self.brown_open = None;
                    }
                    _ => {}
                }
            }
            Role::BrownoutVolts => {
                if let Some(v) = number.filter(|v| (4.0..11.0).contains(v)) {
                    self.brownout_v = Some(v);
                }
            }
            Role::TotalCurrent => self.total_current = number.map(|v| (t, v)),
            Role::CanUtil => {
                let values: Vec<f64> = match ty {
                    Ty::DoubleArr => payload.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).collect(),
                    Ty::FloatArr => payload.chunks_exact(4).map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap()))).collect(),
                    _ => number.into_iter().collect(),
                };
                self.can_util(c, t, &values);
            }
            Role::CanDown => self.can_down_flag("all", t, number == Some(1.0)),
            Role::CanOk(bus) => self.can_down_flag(&bus, t, number == Some(0.0)),
            Role::CanCount(bus, field) => {
                if let Some(v) = number {
                    let counter = self.counters.entry((bus, field)).or_insert(Counter { first: v, last: v, max: v, at_rise: None });
                    if v > counter.last && counter.at_rise.is_none() && field != "rec" && field != "tec" {
                        counter.at_rise = Some(t);
                    }
                    if v > counter.max && (field == "rec" || field == "tec") && counter.at_rise.is_none() && v >= 96.0 {
                        counter.at_rise = Some(t);
                    }
                    counter.last = v;
                    counter.max = counter.max.max(v);
                }
            }
            Role::Cpu => {
                if let Some(v) = number {
                    if let Some((lt, lv)) = self.cpu_last {
                        if lv >= 90.0 {
                            self.cpu_high_s += (t - lt).clamp(0.0, 2.0);
                        }
                    }
                    self.cpu_last = Some((t, v));
                    if self.cpu_peak.is_none_or(|(_, p)| v > p) {
                        self.cpu_peak = Some((t, v));
                    }
                }
            }
            Role::LoopMs => {
                if let Some(ms) = number {
                    self.loop_n += 1;
                    if self.loop_worst.is_none_or(|(_, w)| ms > w) {
                        self.loop_worst = Some((t, ms));
                    }
                    if ms > LOOP_BUDGET_MS && self.overruns.len() < 5000 {
                        self.overruns.push(t);
                    }
                }
            }
            Role::Enabled => {
                let on = number == Some(1.0);
                if self.enabled != Some(on) {
                    keep(&mut self.enabled_changes, (t, on));
                }
                self.enabled = Some(on);
            }
            Role::Autonomous => push_change(&mut self.auto_changes, t, number == Some(1.0)),
            Role::Estop => push_change(&mut self.estop_changes, t, number == Some(1.0)),
            Role::FmsControl => {
                let bits = number.unwrap_or(0.0) as i64;
                push_change(&mut self.fms_enabled_changes, t, bits & 0x01 != 0);
                push_change(&mut self.attached_changes, t, bits & 0x20 != 0);
                if bits & 0x08 != 0 {
                    push_change(&mut self.estop_changes, t, true);
                }
                self.fms_attached |= bits & 0x10 != 0;
                // With no DS:enabled logged, the FMS's word for it.
                if !self.by_key.contains_key("DS:enabled") {
                    let on = bits & 0x01 != 0;
                    if self.enabled != Some(on) {
                        keep(&mut self.enabled_changes, (t, on));
                    }
                    self.enabled = Some(on);
                }
            }
            Role::MatchNumber => self.match_number = number.map(|v| v as i64).filter(|n| *n > 0),
            Role::MatchType => self.match_type = number.map(|v| v as i64),
            Role::TimeLeft => {
                if let Some(v) = number {
                    self.time_left = Some((t, v));
                    if self.time_left_at.len() < 4000 {
                        self.time_left_at.push((t, v));
                    }
                }
            }
            Role::Messages | Role::OpModeError => {
                let line = text();
                if let Some(class) = exception_in(&line) {
                    self.error_count += 1;
                    if self.errors.len() < 50 {
                        self.errors.push((t, class));
                    }
                }
            }
            Role::OpModeCommand => {
                let command = text();
                let command: String = command.chars().take(TEXT_LONGEST).collect();
                if self.commands.last().is_none_or(|(_, last)| *last != command) && self.commands.len() < 400 {
                    self.commands.push((t, command));
                }
            }
            Role::AutoChoice => self.auto_choice = Some(text().chars().take(TEXT_LONGEST).collect::<String>()).filter(|s| !s.trim().is_empty()),
            Role::StartDistance => self.start_distance = number,
            Role::StartHeading => self.start_heading = number,
            Role::Code => self.code = Some(text()).filter(|s| !s.trim().is_empty()),
            Role::MotorsExpected => self.motors_expected = number.map(|v| v as i64),
            Role::MotorsConnected => {
                if let Some(v) = number {
                    if self.motors_connected.last().is_none_or(|(_, last)| *last != v as i64) && self.motors_connected.len() < 400 {
                        self.motors_connected.push((t, v as i64));
                    }
                }
            }
            Role::MotorRows => {
                for row in strings(payload) {
                    let parts: Vec<&str> = row.split('|').collect();
                    let [name, bus, _id, connected, ..] = parts[..] else { continue };
                    let connected = connected == "true";
                    if self.rows_state.len() >= NAMED_KEPT && !self.rows_state.contains_key(name) {
                        continue;
                    }
                    let state = self.rows_state.entry(name.to_owned()).or_insert((bus.to_owned(), true, None));
                    match (connected, state.2) {
                        (false, None) => state.2 = Some(t),
                        (true, Some(start)) => {
                            keep(&mut self.lost, (name.to_owned(), bus.to_owned(), Span { start, end: t }));
                            state.2 = None;
                        }
                        _ => {}
                    }
                    state.1 = connected;
                }
            }
            Role::AlertErrors => {
                for alert in strings(payload) {
                    if self.alerts.len() < 8 && !self.alerts.iter().any(|(_, a)| *a == alert) {
                        self.alerts.push((t, alert));
                    }
                }
            }
            Role::HealthFiring(i) => {
                let on = number == Some(1.0);
                let check = &mut self.checks[i];
                if on && !check.firing {
                    check.since = t;
                    check.fired_detail = check.detail.clone();
                } else if !on && check.firing {
                    keep(&mut check.spans, Span { start: check.since, end: t });
                }
                check.firing = on;
            }
            Role::HealthDetail(i) => {
                let detail = text();
                let check = &mut self.checks[i];
                if check.firing && check.fired_detail.is_empty() {
                    check.fired_detail = detail.clone();
                }
                check.detail = detail;
            }
            Role::Temp(m) => {
                if let Some(v) = number {
                    self.temperature(m, t, v);
                }
            }
            Role::Current(m) => {
                if let Some(v) = number {
                    self.current(m, t, v.abs());
                }
            }
            Role::Velocity(i, speed) => {
                if let Some(v) = number {
                    self.mechs[i].vel = Some((t, v, Some(speed)));
                    if speed == Speed::Rps && self.mechs[i].setpoint.is_some_and(|(_, u)| u == Unit::Rps) {
                        self.mechs[i].measured = Some(v);
                    }
                }
            }
            Role::State(i) => {
                self.mechs[i].state = text();
                self.goal(i, t);
            }
            Role::AtGoal(i) => {
                self.mechs[i].goal = number.map(|v| v > 0.5);
                self.goal(i, t);
            }
            Role::Setpoint(i, unit) => {
                if let Some(v) = number {
                    self.mechs[i].setpoint = Some((v, unit));
                    self.miss_values(i, t);
                }
            }
            Role::Measured(i) => {
                if let Some(v) = number {
                    self.mechs[i].measured = Some(v);
                    self.miss_values(i, t);
                }
            }
        }
    }

    fn battery(&mut self, t: f64, v: f64) {
        if self.battery_low.is_none_or(|(_, low)| v < low) {
            self.battery_low = Some((t, v));
        }
        if self.battery_first.len() < 20 {
            self.battery_first.push(v);
        }
        // Resting, before the robot is first enabled (the log may say
        // "disabled" first, which is not the match starting).
        if !self.enabled_changes.iter().any(|(_, on)| *on) {
            self.battery_before.push((t, v));
            if self.battery_before.len() > 200 {
                self.battery_before.remove(0);
            }
        }
        if v < SAG_V {
            // What was drawing at the low point: looked at again while the
            // battery stays there, as the motors' currents may be logged
            // after the battery in the same loop.
            let lower = self.sag.as_ref().is_none_or(|s| v < s.min);
            let level = self.sag.as_ref().is_some_and(|s| (v - s.min).abs() < 1e-9);
            let load = (lower || level).then(|| self.load_at(t));
            let total = self.total_current.filter(|(at, _)| (t - at).abs() < 0.6).map(|(_, a)| a);
            let sag = self.sag.get_or_insert(Sag { span: Span { start: t, end: t }, min: v, at_min: t, ..Sag::default() });
            sag.span.end = t;
            if lower {
                (sag.min, sag.at_min) = (v, t);
            }
            if let Some(load) = load.filter(|l| !l.is_empty() || lower) {
                sag.load = load;
                sag.total = total.or(sag.total);
            }
        } else if v >= SAG_CLEAR_V {
            if let Some(mut sag) = self.sag.take() {
                sag.span.end = t;
                keep(&mut self.sags, sag);
            }
        }
    }

    /// What was drawing most just now: the hungriest motors, at most three,
    /// each over 15 A and at least a third of the hungriest.
    fn load_at(&self, t: f64) -> Vec<(String, f64)> {
        let mut load: Vec<(String, f64)> = self.motors.iter().filter_map(|m| m.cur_last.filter(|(at, a)| (t - at).abs() < 0.6 && *a >= 15.0).map(|(_, a)| (m.name.clone(), a))).collect();
        load.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top = load.first().map_or(0.0, |l| l.1);
        load.retain(|l| l.1 >= top / 3.0);
        load.truncate(3);
        load
    }

    fn can_util(&mut self, c: usize, t: f64, values: &[f64]) {
        let enabled = self.enabled_now();
        for (i, v) in values.iter().copied().enumerate().take(8) {
            if !v.is_finite() {
                continue;
            }
            // Each bus its own range, in percent.
            while self.canons[c].stats.len() <= i && self.stats.len() < SIGNALS_KEPT {
                self.canons[c].stats.push(self.stats.len());
                self.stats.push(Stat::default());
            }
            if let Some(&si) = self.canons[c].stats.get(i) {
                if self.stats[si].n == 0 {
                    self.stats[si].key = format!("Systemcore/CanUtilization/{}", bus_name(i));
                }
                self.stats[si].number(t, v * 100.0);
            }
            if self.buses.len() <= i {
                self.buses.resize_with(i + 1, Bus::default);
            }
            let bus = &mut self.buses[i];
            if let Some((lt, lv)) = bus.last {
                if lv >= CAN_BUSY && enabled {
                    bus.busy_s += (t - lt).clamp(0.0, 2.0);
                }
            }
            bus.last = Some((t, v));
            if v > bus.peak {
                (bus.peak, bus.at_peak) = (v, t);
            }
        }
    }

    fn can_down_flag(&mut self, bus: &str, t: f64, down: bool) {
        match (down, self.can_down_open.get(bus).copied()) {
            (true, None) => {
                self.can_down_open.insert(bus.to_owned(), t);
            }
            (false, Some(start)) => {
                self.can_down_open.remove(bus);
                keep(&mut self.can_down, (bus.to_owned(), Span { start, end: t }));
            }
            _ => {}
        }
    }

    fn temperature(&mut self, m: usize, t: f64, v: f64) {
        let motor = &mut self.motors[m];
        if let Some(lt) = motor.temp_last_t {
            if motor.temp_peak.is_some_and(|_| v >= HOT_C) {
                motor.hot_s += (t - lt).clamp(0.0, 2.0);
            }
        }
        if v >= HOT_C && motor.hot_since.is_none() {
            motor.hot_since = Some(t);
        }
        motor.temp_last_t = Some(t);
        if motor.temp_peak.is_none_or(|(_, p)| v > p) {
            motor.temp_peak = Some((t, v));
        }
    }

    fn current(&mut self, m: usize, t: f64, amps: f64) {
        let enabled = self.enabled_now();
        let motor = &mut self.motors[m];
        motor.cur_last = Some((t, amps));
        if motor.cur_peak.is_none_or(|(_, p)| amps > p) {
            motor.cur_peak = Some((t, amps));
        }
        // A stall: drawing hard while its mechanism does not move.
        let still = motor.mech.and_then(|i| self.mechs[i].vel).is_some_and(|(vt, v, speed)| (t - vt).abs() < 0.6 && speed.is_some_and(|s| s.still(v)));
        let motor = &mut self.motors[m];
        if enabled && amps >= STALL_A && still {
            let open = motor.stall_open.get_or_insert((t, amps));
            open.1 = open.1.max(amps);
        } else if let Some((start, peak)) = motor.stall_open.take() {
            if t - start >= STALL_S {
                keep(&mut motor.stalls, (Span { start, end: t }, peak));
            }
        }
    }

    /// A mechanism's goal: missed while enabled, aiming somewhere, and not there.
    fn goal(&mut self, i: usize, t: f64) {
        let enabled = self.enabled_now();
        let mech = &mut self.mechs[i];
        let state = mech.state.to_ascii_lowercase();
        let aiming = state.is_empty() || ["goto", "profiledgoto", "hold", "spinup", "aim", "track"].iter().any(|s| state.replace(' ', "").starts_with(s));
        let missing = enabled && aiming && mech.goal == Some(false);
        match (missing, mech.miss_open.is_some()) {
            (true, false) => {
                mech.miss_open = Some(Miss { span: Span { start: t, end: t }, setpoint: mech.setpoint.map(|s| s.0), measured: mech.measured, unit: mech.setpoint.map(|s| s.1) });
            }
            (false, true) => {
                let mut miss = mech.miss_open.take().unwrap();
                miss.span.end = t;
                if miss.span.len() >= MISS_S {
                    keep(&mut mech.misses, miss);
                }
            }
            _ => {}
        }
    }

    /// Where a mechanism short of its setpoint got to. A new setpoint is a
    /// new try: the miss so far ends there.
    fn miss_values(&mut self, i: usize, t: f64) {
        let mech = &mut self.mechs[i];
        let (setpoint, measured) = (mech.setpoint, mech.measured);
        let Some(was) = mech.miss_open.as_ref().map(|m| m.setpoint) else { return };
        match (was, setpoint) {
            (Some(was), Some((now, _))) if (was - now).abs() > 1e-6 => {
                let mut ended = mech.miss_open.take().unwrap();
                ended.span.end = t;
                if ended.span.len() >= MISS_S {
                    keep(&mut mech.misses, ended);
                }
                // Still short of the new one: a new miss from here.
                self.goal(i, t);
            }
            (None, Some((sp, unit))) => {
                if let Some(miss) = &mut mech.miss_open {
                    miss.setpoint = Some(sp);
                    miss.unit = Some(unit);
                }
            }
            _ => {
                if let (Some(miss), Some(m)) = (&mut mech.miss_open, measured) {
                    miss.measured = Some(m);
                }
            }
        }
    }

    fn finish(mut self, log: &str, bytes: u64) -> MatchFacts {
        let end_t = self.last_t;
        // Whatever was still under way when the log ended.
        if let Some(mut sag) = self.sag.take() {
            sag.span.end = end_t;
            keep(&mut self.sags, sag);
        }
        if let Some(start) = self.brown_open.take() {
            keep(&mut self.browns, Span { start, end: end_t });
        }
        for (bus, start) in std::mem::take(&mut self.can_down_open) {
            keep(&mut self.can_down, (bus, Span { start, end: end_t }));
        }
        for c in &mut self.checks {
            if c.firing {
                keep(&mut c.spans, Span { start: c.since, end: end_t });
            }
        }
        for m in &mut self.motors {
            if let Some((start, peak)) = m.stall_open.take() {
                if end_t - start >= STALL_S {
                    keep(&mut m.stalls, (Span { start, end: end_t }, peak));
                }
            }
        }
        for mech in &mut self.mechs {
            if let Some(mut miss) = mech.miss_open.take() {
                miss.span.end = end_t;
                if miss.span.len() >= MISS_S {
                    keep(&mut mech.misses, miss);
                }
            }
        }
        for (name, (bus, _, open)) in &self.rows_state {
            if let Some(start) = open {
                keep(&mut self.lost, (name.clone(), bus.clone(), Span { start: *start, end: end_t }));
            }
        }
        Sheet::new(self, log, bytes).build()
    }
}

/// Kept, while there is room for more of its sort.
fn keep<T>(list: &mut Vec<T>, item: T) {
    if list.len() < EVENTS_KEPT {
        list.push(item);
    }
}

fn push_change(changes: &mut Vec<(f64, bool)>, t: f64, on: bool) {
    if changes.last().is_none_or(|(_, last)| *last != on) && changes.len() < 2000 {
        changes.push((t, on));
    }
}

/// The CAN bus at an index of Systemcore's list: can_s0 to can_s4.
fn bus_name(i: usize) -> String {
    format!("can_s{i}")
}

/// The exception a console line reports: "NullPointerException", from a
/// line such as "Unhandled exception: java.lang.NullPointerException: …".
fn exception_in(line: &str) -> Option<String> {
    let word = line
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '$'))
        .map(|w| w.rsplit('.').next().unwrap_or(w))
        .find(|w| (w.ends_with("Exception") || w.ends_with("Error")) && w.len() > 6 && w.chars().next().is_some_and(char::is_uppercase))?;
    Some(word.chars().take(TEXT_LONGEST).collect())
}

// ------------------------------------------------------------ the sheet ---

/// Turning what was gathered into facts, with the match's clock.
struct Sheet {
    s: Scan,
    log: String,
    bytes: u64,
    /// Log seconds the match started (first enabled), and auto ended.
    t0: Option<f64>,
    auto_end: Option<f64>,
    end: f64,
    facts: Vec<Fact>,
    metrics: BTreeMap<String, f64>,
}

impl Sheet {
    fn new(s: Scan, log: &str, bytes: u64) -> Self {
        let t0 = s.enabled_changes.iter().find(|(_, on)| *on).map(|(t, _)| *t);
        let end = s.enabled_changes.iter().rev().find(|(_, on)| !*on).map(|(t, _)| *t).filter(|t| t0.is_some_and(|t0| *t > t0)).unwrap_or(s.last_t);
        let auto_end = match t0 {
            Some(t0) => s.auto_changes.iter().find(|(t, on)| !*on && *t > t0).map(|(t, _)| *t).or(Some(t0 + AUTO_S)),
            None => None,
        };
        Sheet { s, log: log.to_owned(), bytes, t0, auto_end, end, facts: Vec::new(), metrics: BTreeMap::new() }
    }

    /// Log seconds as seconds into the match.
    fn rel(&self, t: f64) -> f64 {
        t - self.t0.unwrap_or(self.s.first_t.unwrap_or(0.0))
    }

    /// "at 1:18 in teleop", "before the match", "after the match".
    fn when(&self, t: f64) -> String {
        let Some(t0) = self.t0 else { return format!("at {} into the log", clock(self.rel(t))) };
        if t < t0 - 0.05 {
            return "before the match".into();
        }
        if t > self.end + 0.5 {
            return "after the match".into();
        }
        let phase = if self.auto_end.is_some_and(|e| t < e) { "auto" } else { "teleop" };
        format!("at {} in {phase}", clock(t - t0))
    }

    fn push(&mut self, kind: Kind, level: Level, score: u32, subject: &str, t: Option<f64>, dur: Option<f64>, nums: Vec<Num>, names: Vec<String>, text: String) {
        let at = t.map(|t| (self.rel(t) * 10.0).round() / 10.0);
        let dur = dur.map(|d| (d * 10.0).round() / 10.0);
        self.facts.push(Fact { kind, level, score, subject: subject.to_owned(), at, dur, nums, names, text });
    }

    fn metric(&mut self, name: &str, v: f64) {
        self.metrics.insert(name.to_owned(), (v * 1000.0).round() / 1000.0);
    }

    fn build(mut self) -> MatchFacts {
        self.battery();
        self.link();
        self.can();
        self.motors();
        self.mechanisms();
        self.loop_and_cpu();
        self.health();
        self.code_and_auto();
        let label = label_from_name(&self.log)
            .or_else(|| {
                let n = self.s.match_number?;
                let letter = match self.s.match_type? {
                    1 => "P",
                    2 => "Q",
                    3 => "E",
                    _ => return None,
                };
                Some(format!("{letter}{n}"))
            })
            .unwrap_or_else(|| crate::logs::Row { name: self.log.clone(), bytes: 0, digest: None }.title());
        let mut facts = std::mem::take(&mut self.facts);
        facts.sort_by(|a, b| b.score.cmp(&a.score).then(a.at.unwrap_or(0.0).total_cmp(&b.at.unwrap_or(0.0))));
        let signals = self
            .s
            .stats
            .iter()
            .filter(|st| st.n > 0)
            .map(|st| Signal {
                key: st.key.clone(),
                boolean: st.boolean,
                n: st.n,
                min: round3(st.min),
                max: round3(st.max),
                mean: round3(st.sum / st.n as f64),
                at_min: round1(self.rel(st.t_min)),
                at_max: round1(self.rel(st.t_max)),
                last: round3(st.last),
                true_s: round1(st.true_s),
                rises: st.rises,
            })
            .collect();
        let enabled_seconds = self.t0.map(|_| enabled_time(&self.s.enabled_changes, self.s.last_t));
        MatchFacts {
            log: self.log.clone(),
            bytes: self.bytes,
            label,
            seconds: round1(self.s.last_t - self.s.first_t.unwrap_or(self.s.last_t)),
            enabled_seconds: enabled_seconds.map(round1),
            code: self.s.code.clone().map(|c| short_code(&c)),
            routine: self.routine(),
            facts,
            metrics: self.metrics,
            signals,
            truncated: self.s.truncated,
        }
    }

    fn battery(&mut self) {
        let brownout_v = self.s.brownout_v.unwrap_or(BROWNOUT_V);
        // Brownouts: as the controller flagged them, or (with no flag
        // logged) every sag below its threshold.
        let browns: Vec<Span> = if self.s.saw_brown_flag { self.s.browns.clone() } else { self.s.sags.iter().filter(|s| s.min < brownout_v).map(|s| s.span).collect() };
        let sags = self.s.sags.clone();
        let sag_of = |b: &Span| sags.iter().filter(|s| s.span.start <= b.end + 0.6 && s.span.end >= b.start - 0.6).min_by(|a, b| a.min.total_cmp(&b.min)).cloned();
        self.metric("brownouts", browns.len() as f64);
        if !browns.is_empty() {
            let mut parts = Vec::new();
            let mut nums = vec![Num::new(browns.len() as f64, Unit::Count)];
            let mut first_load: Option<Sag> = None;
            for (k, b) in browns.iter().take(3).enumerate() {
                let sag = sag_of(b);
                let when = self.when(b.start);
                let when = if k > 0 { when.split(" in ").next().unwrap_or(&when).to_owned() } else { when };
                let mut detail = Vec::new();
                if let Some(sag) = &sag {
                    detail.push(Num::new(sag.min, Unit::Volts).shown());
                    nums.push(Num::new(sag.min, Unit::Volts));
                }
                detail.push(format!("for {}", seconds(b.len().max(0.1))));
                nums.push(Num::new(b.len().max(0.1), Unit::Seconds));
                parts.push(format!("{when} ({})", detail.join(" ")));
                if k == 0 {
                    first_load = sag;
                }
            }
            let times = if browns.len() == 1 { "once".to_owned() } else { format!("{} times", browns.len()) };
            let text = format!("the robot browned out {times}: {}", join_and(&parts));
            let first = browns[0];
            self.push(Kind::Brownout, Level::Fault, 100 + browns.len() as u32, "battery", Some(first.start), Some(first.len().max(0.1)), nums, Vec::new(), text);
            if let Some(sag) = first_load.filter(|s| !s.load.is_empty() || s.total.is_some()) {
                let mut drew: Vec<String> = sag.load.iter().map(|(name, a)| format!("{name} {}", Num::new(*a, Unit::Amps).shown())).collect();
                let mut nums: Vec<Num> = sag.load.iter().map(|(_, a)| Num::new(*a, Unit::Amps)).collect();
                if let Some(total) = sag.total {
                    drew.insert(0, format!("{} in all", Num::new(total, Unit::Amps).shown()));
                    nums.insert(0, Num::new(total, Unit::Amps));
                }
                let names = sag.load.iter().map(|(n, _)| n.clone()).collect();
                let text = format!("at the {} brownout the robot was drawing {}", clock(self.rel(first.start)), join_and(&drew));
                self.push(Kind::Brownout, Level::Warn, 90, "load", Some(first.start), None, nums, names, text);
            }
        }
        // The worst sag that did not brown out.
        let worst_sag = self.s.sags.iter().filter(|s| !browns.iter().any(|b| s.span.start <= b.end + 0.6 && s.span.end >= b.start - 0.6)).min_by(|a, b| a.min.total_cmp(&b.min)).cloned();
        self.metric("sags", self.s.sags.len() as f64);
        if let Some(sag) = worst_sag {
            let text = format!("the battery sagged to {} {} for {}", Num::new(sag.min, Unit::Volts).shown(), self.when(sag.at_min), seconds(sag.span.len().max(0.1)));
            let mut nums = vec![Num::new(sag.min, Unit::Volts), Num::new(sag.span.len().max(0.1), Unit::Seconds)];
            let mut names = Vec::new();
            let mut text = text;
            if let Some((name, a)) = sag.load.first() {
                text.push_str(&format!(", with {name} drawing {}", Num::new(*a, Unit::Amps).shown()));
                nums.push(Num::new(*a, Unit::Amps));
                names.push(name.clone());
            }
            self.push(Kind::Sag, Level::Warn, 70, "battery", Some(sag.at_min), Some(sag.span.len()), nums, names, text);
        }
        if let Some((t, low)) = self.s.battery_low {
            self.metric("battery_low", low);
            let level = if low < 9.0 { Level::Warn } else { Level::Fine };
            let text = format!("the battery's low point was {} {}", Num::new(low, Unit::Volts).shown(), self.when(t));
            self.push(Kind::Battery, level, if low < 9.0 { 30 } else { 8 }, "battery", Some(t), None, vec![Num::new(low, Unit::Volts)], Vec::new(), text);
        }
        // How charged it was: resting, just before the match.
        let resting = {
            let before: Vec<f64> = match self.t0 {
                Some(t0) => self.s.battery_before.iter().filter(|(t, _)| *t >= t0 - 3.0 && *t < t0).map(|(_, v)| *v).collect(),
                None => Vec::new(),
            };
            let pick = if before.is_empty() { self.s.battery_first.clone() } else { before };
            (!pick.is_empty()).then(|| pick.iter().sum::<f64>() / pick.len() as f64)
        };
        if let Some(v) = resting {
            self.metric("battery_start", v);
            let low = v < CHARGED_V;
            let text = if low { format!("the battery started the match at only {}, not fully charged", Num::new(v, Unit::Volts).shown()) } else { format!("the battery started the match at {}", Num::new(v, Unit::Volts).shown()) };
            self.push(Kind::BatteryStart, if low { Level::Warn } else { Level::Fine }, if low { 75 } else { 6 }, "battery", self.t0, None, vec![Num::new(v, Unit::Volts)], Vec::new(), text);
        }
    }

    /// The robot's link to the driver station: e-stops, comms lost, and
    /// being disabled in the middle of a match.
    fn link(&mut self) {
        let Some(t0) = self.t0 else { return };
        let changes = self.s.enabled_changes.clone();
        let estop_at = |t: f64, s: &Scan| s.estop_changes.iter().rev().find(|(at, _)| *at <= t + 0.3).is_some_and(|(_, on)| *on);
        let detached_near = |t: f64, s: &Scan| s.attached_changes.iter().any(|(at, on)| !*on && (*at - t).abs() <= 0.6) || s.attached_changes.iter().rev().find(|(at, _)| *at <= t + 0.3).is_some_and(|(_, on)| !*on);
        let mut disabled_s = 0.0;
        let mut kept = 0;
        for w in changes.windows(2) {
            let ((off_t, off), (on_t, on)) = (w[0], w[1]);
            if off || !on || off_t < t0 {
                continue;
            }
            let gap = on_t - off_t;
            // The pause between auto and teleop is the match, not a fault.
            let transition = gap < TRANSITION_S
                && if self.s.auto_changes.is_empty() {
                    (off_t - t0 - AUTO_S).abs() < 2.0
                } else {
                    self.s.auto_changes.iter().any(|(t, a)| !*a && (*t - off_t).abs() < 1.0) || self.auto_end.is_some_and(|e| (e - off_t).abs() < 1.0)
                };
            if transition || gap < 0.3 {
                continue;
            }
            disabled_s += gap;
            kept += 1;
            self.gap(off_t, Some(gap), estop_at(off_t, &self.s), detached_near(off_t, &self.s));
        }
        // Disabled for good before the match's end.
        if let Some(&(last_t, false)) = changes.last() {
            let ended_early = match self.s.time_left_at.iter().rev().find(|(t, _)| *t <= last_t + 0.05) {
                Some((_, left)) => *left > 3.0,
                None => {
                    let full = AUTO_S + 140.0;
                    let real_match = self.s.fms_attached || label_from_name(&self.log).is_some_and(|l| !l.starts_with('P'));
                    real_match && last_t - t0 < full - 8.0
                }
            };
            let estop = estop_at(last_t, &self.s);
            if ended_early || estop {
                kept += 1;
                self.gap(last_t, None, estop, detached_near(last_t, &self.s));
            }
        }
        self.metric("disabled_s", disabled_s);
        self.metric("link_faults", kept as f64);
    }

    fn gap(&mut self, t: f64, dur: Option<f64>, estop: bool, detached: bool) {
        let when = self.when(t);
        let rest = |d: Option<f64>| match d {
            Some(d) => format!("for {}", seconds(d)),
            None => "for the rest of the match".to_owned(),
        };
        let mut nums = Vec::new();
        if let Some(d) = dur {
            nums.push(Num::new(d, Unit::Seconds));
        }
        let (kind, score, text) = if estop {
            (Kind::Estop, 98, format!("the robot was e-stopped {when} and stayed disabled {}", rest(dur)))
        } else if detached {
            (Kind::Comms, 96, format!("the robot lost comms with the driver station {when} and was disabled {}", rest(dur)))
        } else {
            (Kind::Disabled, 92, format!("the robot was disabled {when} {}, with no e-stop or lost link logged", rest(dur)))
        };
        self.push(kind, Level::Fault, score, if kind == Kind::Comms { "comms" } else { "robot" }, Some(t), dur, nums, Vec::new(), text);
    }

    fn can(&mut self) {
        let buses: Vec<(usize, f64, f64, f64)> = self.s.buses.iter().enumerate().map(|(i, b)| (i, b.peak, b.at_peak, b.busy_s)).collect();
        for &(i, peak, _, _) in &buses {
            self.metric(&format!("can:{}", bus_name(i)), peak * 100.0);
        }
        if let Some(&(i, peak, at, busy_s)) = buses.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            self.metric("can_peak", peak * 100.0);
            let name = bus_name(i);
            if peak >= CAN_BUSY {
                let mut text = format!("CAN bus {name} reached {} {}", Num::new(peak * 100.0, Unit::Percent).shown(), self.when(at));
                let mut nums = vec![Num::new(peak * 100.0, Unit::Percent)];
                if busy_s >= 0.5 {
                    text.push_str(&format!(", over 90% for {}", seconds(busy_s)));
                    nums.push(Num::new(busy_s, Unit::Seconds));
                }
                self.push(Kind::CanBusy, Level::Warn, 62, &name, Some(at), None, nums, Vec::new(), text);
            } else {
                let text = format!("the busiest CAN bus was {name} at {} {}", Num::new(peak * 100.0, Unit::Percent).shown(), self.when(at));
                self.push(Kind::Can, Level::Fine, 7, &name, Some(at), None, vec![Num::new(peak * 100.0, Unit::Percent)], Vec::new(), text);
            }
        }
        let mut down: Vec<(String, Span)> = self.s.can_down.clone();
        // A bus named down on its own says more than Systemcore's "a bus is down".
        if down.iter().any(|(b, _)| b != "all") {
            down.retain(|(b, _)| b != "all");
        }
        self.metric("can_down", down.len() as f64);
        let mut seen: Vec<String> = Vec::new();
        for (bus, span) in down {
            if seen.contains(&bus) {
                continue;
            }
            let times = self.s.can_down.iter().filter(|(b, _)| *b == bus).count();
            seen.push(bus.clone());
            let name = if bus == "all" { "a CAN bus".to_owned() } else { format!("CAN bus {bus}") };
            let mut text = format!("{name} went down {} for {}", self.when(span.start), seconds(span.len().max(0.1)));
            let mut nums = vec![Num::new(span.len().max(0.1), Unit::Seconds)];
            if times > 1 {
                text.push_str(&format!(", {times} times in all"));
                nums.push(Num::new(times as f64, Unit::Count));
            }
            let subject = if bus == "all" { "CAN" } else { bus.as_str() };
            self.push(Kind::CanDown, Level::Fault, 88, subject, Some(span.start), Some(span.len()), nums, Vec::new(), text);
        }
        // Error counters that rose during the match.
        let counters: Vec<((String, &'static str), f64, f64, Option<f64>)> = self.s.counters.iter().map(|(k, c)| (k.clone(), c.last - c.first, c.max, c.at_rise)).collect();
        let mut by_bus: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
        let mut first_at: BTreeMap<String, f64> = BTreeMap::new();
        for ((bus, field), rose, max, at) in counters {
            let (words, n) = match field {
                "busoff" if rose >= 1.0 => (if rose == 1.0 { "bus-off".to_owned() } else { "bus-offs".to_owned() }, rose),
                "txfull" if rose >= 1.0 => (if rose == 1.0 { "full transmit buffer".to_owned() } else { "full transmit buffers".to_owned() }, rose),
                "down" if rose >= 1.0 => (if rose == 1.0 { "time down".to_owned() } else { "times down".to_owned() }, rose),
                "tec" | "rec" if max >= 96.0 => (format!("{} error count up to", if field == "tec" { "transmit" } else { "receive" }), max),
                _ => continue,
            };
            if let Some(at) = at {
                let entry = first_at.entry(bus.clone()).or_insert(at);
                *entry = entry.min(at);
            }
            by_bus.entry(bus).or_default().push((words, n));
        }
        let mut errors_total = 0.0;
        for (bus, list) in by_bus {
            let parts: Vec<String> = list.iter().map(|(w, n)| if w.ends_with(" up to") { format!("{w} {n:.0}") } else { format!("{n:.0} {w}") }).collect();
            errors_total += list.iter().filter(|(w, _)| !w.ends_with(" up to")).map(|(_, n)| n).sum::<f64>();
            let name = if bus == "all" { "the CAN buses".to_owned() } else { format!("CAN bus {bus}") };
            let at = first_at.get(&bus).copied();
            let when = at.map(|t| format!(", from {}", self.when(t).trim_start_matches("at "))).unwrap_or_default();
            let text = format!("{name} logged {}{when}", join_and(&parts));
            let nums = list.iter().map(|(_, n)| Num::new(*n, Unit::Count)).collect();
            let subject = if bus == "all" { "CAN".to_owned() } else { bus.clone() };
            self.push(Kind::CanErrors, Level::Warn, 66, &subject, at, None, nums, Vec::new(), text);
        }
        self.metric("can_errors", errors_total);
    }

    fn motors(&mut self) {
        let motors: Vec<_> = self.s.motors.iter().map(|m| (m.name.clone(), m.temp_peak, m.hot_since, m.hot_s, m.cur_peak, m.stalls.clone())).collect();
        let mut hottest: Option<(String, f64, f64)> = None;
        let mut hungriest: Vec<(String, f64, f64)> = Vec::new();
        for (name, temp, hot_since, hot_s, cur, stalls) in motors {
            if let Some((t, peak)) = temp {
                self.metric(&format!("temp:{name}"), peak);
                if peak >= HOT_C {
                    let level = if peak >= OVERHEAT_C { Level::Fault } else { Level::Warn };
                    let mut text = format!("{name} reached {} {}", Num::new(peak, Unit::Celsius).shown(), self.when(t));
                    let mut nums = vec![Num::new(peak, Unit::Celsius)];
                    if let Some(since) = hot_since.filter(|s| t - s > 1.0) {
                        text.push_str(&format!(", over 70 °C from {}", clock(self.rel(since))));
                    }
                    if hot_s >= 1.0 {
                        nums.push(Num::new(hot_s, Unit::Seconds));
                    }
                    let score = if peak >= OVERHEAT_C { 80 } else { 60 } + (peak - HOT_C).clamp(0.0, 19.0) as u32;
                    self.push(Kind::Heat, level, score, &name, Some(t), None, nums, Vec::new(), text);
                }
                if hottest.as_ref().is_none_or(|(_, p, _)| peak > *p) {
                    hottest = Some((name.clone(), peak, t));
                }
            }
            if let Some((t, amps)) = cur {
                self.metric(&format!("current:{name}"), amps);
                hungriest.push((name.clone(), amps, t));
            }
            if let Some((span, amps)) = stalls.iter().max_by(|a, b| a.0.len().total_cmp(&b.0.len())) {
                let level = if span.len() >= 2.0 { Level::Fault } else { Level::Warn };
                let mut text = format!("{name} stalled {} for {}, drawing {}", self.when(span.start), seconds(span.len()), Num::new(*amps, Unit::Amps).shown());
                let mut nums = vec![Num::new(*amps, Unit::Amps), Num::new(span.len(), Unit::Seconds)];
                if stalls.len() > 1 {
                    text.push_str(&format!(" ({} stalls in all)", stalls.len()));
                    nums.push(Num::new(stalls.len() as f64, Unit::Count));
                }
                self.push(Kind::Stall, level, 72 + (span.len() * 2.0).min(10.0) as u32, &name, Some(span.start), Some(span.len()), nums, Vec::new(), text);
                self.metric(&format!("stall:{name}"), span.len());
            }
        }
        if let Some((name, peak, t)) = hottest {
            self.metric("temp_peak", peak);
            if peak < HOT_C {
                let text = format!("the hottest motor was {name} at {}, {}", Num::new(peak, Unit::Celsius).shown(), self.when(t));
                self.push(Kind::Heat, Level::Fine, 9, &name, Some(t), None, vec![Num::new(peak, Unit::Celsius)], Vec::new(), text);
            }
        }
        hungriest.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (k, (name, amps, t)) in hungriest.into_iter().take(4).enumerate() {
            let text = format!("{name} peaked at {} {}", Num::new(amps, Unit::Amps).shown(), self.when(t));
            self.push(Kind::Current, Level::Fine, 5 - k as u32, &name, Some(t), None, vec![Num::new(amps, Unit::Amps)], Vec::new(), text);
        }
        // Motors that dropped off their bus.
        let mut lost = self.s.lost.clone();
        lost.sort_by(|a, b| a.2.start.total_cmp(&b.2.start));
        self.metric("motors_lost", lost.iter().map(|l| &l.0).collect::<std::collections::BTreeSet<_>>().len() as f64);
        let mut said: Vec<String> = Vec::new();
        for (name, bus, span) in lost {
            if said.contains(&name) {
                continue;
            }
            said.push(name.clone());
            let back = if span.end >= self.s.last_t - 0.01 { " and never came back".to_owned() } else { format!(" for {}", seconds(span.len().max(0.1))) };
            let on = if bus.is_empty() { String::new() } else { format!(" {bus}") };
            let text = format!("{name} dropped off{on} {}{back}", self.when(span.start));
            let subject = display(&name);
            let names = if bus.is_empty() { Vec::new() } else { vec![bus.clone()] };
            self.push(Kind::MotorLost, Level::Fault, 86, &subject, Some(span.start), Some(span.len()), vec![Num::new(span.len().max(0.1), Unit::Seconds)], names, text);
        }
        // With no rows, the count of those connected falling short.
        if said.is_empty() {
            if let Some(expected) = self.s.motors_expected {
                let low = self.s.motors_connected.iter().filter(|(t, _)| self.t0.is_none_or(|t0| *t >= t0 - 1.0)).min_by_key(|(_, n)| *n).copied();
                if let Some((t, n)) = low.filter(|(_, n)| *n < expected) {
                    let text = format!("only {n} of {expected} motors were connected {}", self.when(t));
                    self.push(Kind::MotorLost, Level::Fault, 84, "motors", Some(t), None, vec![Num::new(n as f64, Unit::Count), Num::new(expected as f64, Unit::Count)], Vec::new(), text);
                }
            }
        }
    }

    fn mechanisms(&mut self) {
        let misses: Vec<(String, Vec<Miss>)> = self.s.mechs.iter().filter(|m| !m.misses.is_empty()).map(|m| (m.name.clone(), m.misses.clone())).collect();
        for (name, list) in misses {
            let worst = *list.iter().max_by(|a, b| a.span.len().total_cmp(&b.span.len())).unwrap();
            let level = if worst.span.len() >= 5.0 { Level::Fault } else { Level::Warn };
            let mut text = format!("{name} was short of its setpoint for {} {}", seconds(worst.span.len()), self.when(worst.span.start).replacen("at ", "from ", 1));
            let mut nums = vec![Num::new(worst.span.len(), Unit::Seconds)];
            if let (Some(sp), Some(unit)) = (worst.setpoint, worst.unit) {
                let asked = Num::new(sp, unit);
                text.push_str(&format!(": {} asked", asked.shown()));
                nums.push(asked);
                if let Some(m) = worst.measured {
                    let got = Num::new(m, unit);
                    text.push_str(&format!(", {} reached", got.shown()));
                    nums.push(got);
                }
            }
            if list.len() > 1 {
                text.push_str(&format!(" ({} times)", list.len()));
                nums.push(Num::new(list.len() as f64, Unit::Count));
            }
            self.metric(&format!("miss:{name}"), worst.span.len());
            self.push(Kind::Mechanism, level, 58 + (worst.span.len() * 2.0).min(12.0) as u32, &name, Some(worst.span.start), Some(worst.span.len()), nums, Vec::new(), text);
        }
    }

    fn loop_and_cpu(&mut self) {
        let overruns = self.s.overruns.clone();
        self.metric("overruns", overruns.len() as f64);
        if let Some((t, worst)) = self.s.loop_worst {
            self.metric("worst_loop_ms", worst);
            if overruns.len() >= 3 || worst >= 50.0 {
                let in_auto = overruns.iter().filter(|o| self.t0.is_some_and(|t0| **o >= t0) && self.auto_end.is_some_and(|e| **o < e)).count();
                let mut text = format!("the loop overran its budget {} times, worst {} {}", overruns.len(), Num::new(worst, Unit::Ms).shown(), self.when(t));
                let mut nums = vec![Num::new(overruns.len() as f64, Unit::Count), Num::new(worst, Unit::Ms)];
                if in_auto > 0 && in_auto < overruns.len() {
                    text.push_str(&format!(" ({in_auto} in auto)"));
                    nums.push(Num::new(in_auto as f64, Unit::Count));
                }
                let score = 50 + (overruns.len() as u32).min(20) + if worst >= 50.0 { 5 } else { 0 };
                self.push(Kind::Loop, Level::Warn, score, "loop", Some(t), None, nums, Vec::new(), text);
            } else {
                let text = if overruns.is_empty() {
                    format!("the loop never overran its budget; its worst was {}", Num::new(worst, Unit::Ms).shown())
                } else {
                    format!("the loop overran its budget {} time{}, worst {} {}", overruns.len(), if overruns.len() == 1 { "" } else { "s" }, Num::new(worst, Unit::Ms).shown(), self.when(t))
                };
                let nums = vec![Num::new(worst, Unit::Ms), Num::new(overruns.len() as f64, Unit::Count)];
                self.push(Kind::Loop, Level::Fine, 6, "loop", Some(t), None, nums, Vec::new(), text);
            }
        }
        if let Some((t, peak)) = self.s.cpu_peak {
            self.metric("cpu_peak", peak);
            if self.s.cpu_high_s >= 5.0 {
                let text = format!("the robot's processor ran over 90% for {}, peaking at {} {}", seconds(self.s.cpu_high_s), Num::new(peak, Unit::Percent).shown(), self.when(t));
                self.push(Kind::Cpu, Level::Warn, 45, "processor", Some(t), Some(self.s.cpu_high_s), vec![Num::new(self.s.cpu_high_s, Unit::Seconds), Num::new(peak, Unit::Percent)], Vec::new(), text);
            }
        }
    }

    /// Health checks that fired and are not said already by another fact.
    fn health(&mut self) {
        let checks: Vec<(String, String, Vec<Span>, String)> = self.s.checks.iter().filter(|c| !c.spans.is_empty()).map(|c| (c.subsystem.clone(), c.id.clone(), c.spans.clone(), c.fired_detail.clone())).collect();
        let mut warnings = 0;
        for (subsystem, id, spans, detail) in checks {
            warnings += spans.len();
            let said = |kind: Kind, subject: &str, facts: &[Fact]| facts.iter().any(|f| f.kind == kind && (subject.is_empty() || f.subject.eq_ignore_ascii_case(subject)));
            let covered = match id.as_str() {
                i if i.starts_with("HighTemp") || i.starts_with("OverTemp") => said(Kind::Heat, &display(&subsystem), &self.facts),
                "Stall" => said(Kind::Stall, &display(&subsystem), &self.facts),
                "BrownedOut" => said(Kind::Brownout, "", &self.facts),
                "HighCpu" => said(Kind::Cpu, "", &self.facts),
                i if subsystem == "CAN" && i.ends_with("/HighUtilization") => said(Kind::CanBusy, "", &self.facts) || said(Kind::Can, "", &self.facts),
                i if subsystem == "CAN" && i.ends_with("/Down") => said(Kind::CanDown, "", &self.facts),
                _ if subsystem == "CAN" => said(Kind::CanErrors, "", &self.facts),
                _ => false,
            };
            if covered {
                continue;
            }
            let first = spans[0].start;
            let times = if spans.len() == 1 { "once".to_owned() } else { format!("{} times", spans.len()) };
            let what = if subsystem == "CAN" { format!("CAN {id}") } else { format!("{} {id}", display(&subsystem)) };
            let mut text = format!("the {what} check fired {times}, first {}", self.when(first));
            if !detail.trim().is_empty() {
                text.push_str(&format!(" ({})", detail.trim().chars().take(40).collect::<String>()));
            }
            let subject = if subsystem == "CAN" { id.split('/').next().unwrap_or("CAN").to_owned() } else { display(&subsystem) };
            self.push(Kind::Health, Level::Warn, 40, &subject, Some(first), Some(spans[0].len()), vec![Num::new(spans.len() as f64, Unit::Count)], vec![id.clone()], text);
        }
        self.metric("checks_fired", warnings as f64);
        let alerts = self.s.alerts.clone();
        for (t, alert) in alerts.into_iter().take(3) {
            let text = format!("an error alert said \"{}\" {}", alert.chars().take(48).collect::<String>(), self.when(t));
            self.push(Kind::Health, Level::Warn, 42, "alerts", Some(t), None, Vec::new(), Vec::new(), text);
        }
        let errors = self.s.errors.clone();
        if let Some((t, class)) = errors.first() {
            let n = self.s.error_count;
            let times = if n == 1 { String::new() } else { format!(" ({n} times)") };
            let text = format!("the robot code threw {class} {}{times}", self.when(*t));
            self.metric("code_errors", f64::from(n));
            self.push(Kind::CodeError, Level::Fault, 85, "code", Some(*t), None, vec![Num::new(f64::from(n), Unit::Count)], vec![class.clone()], text);
        }
    }

    fn routine(&self) -> Option<String> {
        let from_command = self.s.commands.iter().find_map(|(_, c)| c.strip_prefix("Auto:").map(|r| r.trim().to_owned())).filter(|r| !r.is_empty());
        from_command.or_else(|| self.s.auto_choice.clone())
    }

    fn code_and_auto(&mut self) {
        if let Some(code) = self.s.code.clone() {
            let text = format!("the robot ran code {}", short_code(&code));
            self.push(Kind::Code, Level::Fine, 4, "code", None, None, Vec::new(), vec![short_code(&code)], text);
        }
        let Some(t0) = self.t0 else { return };
        let Some(routine) = self.routine() else { return };
        let auto_end = self.auto_end.unwrap_or(t0 + AUTO_S);
        let started = self.s.commands.iter().find(|(t, c)| c.starts_with("Auto:") && *t >= t0 - 2.0).map(|(t, _)| *t);
        let finished = started.and_then(|s| self.s.commands.iter().find(|(t, c)| *t > s && !c.starts_with("Auto:")).map(|(t, _)| *t));
        let mut level = Level::Fine;
        let mut score = 10;
        let mut nums = Vec::new();
        // Its time is when it finished, or when auto ended on it. Ending
        // only as auto did (the robot disabled for the pause) is not finishing.
        let mut at = started.or(Some(t0));
        let mut text = match (started, finished) {
            (Some(_), Some(done)) if done < auto_end - 0.1 => {
                at = Some(done);
                format!("auto {routine} finished at {}", clock(self.rel(done)))
            }
            (Some(_), _) => {
                level = Level::Warn;
                score = 64;
                at = Some(auto_end);
                format!("auto {routine} was still running when auto ended at {}", clock(self.rel(auto_end)))
            }
            _ => format!("auto {routine} ran"),
        };
        let mut off = Vec::new();
        if let Some(d) = self.s.start_distance.filter(|d| *d >= 0.15) {
            off.push(Num::new(d, Unit::Meters));
        }
        if let Some(h) = self.s.start_heading.filter(|h| h.abs() >= 5.0) {
            off.push(Num::new(h.abs(), Unit::Degrees));
        }
        if !off.is_empty() {
            text.push_str(&format!("; the robot started {} off its auto start", off.iter().map(Num::shown).collect::<Vec<_>>().join(" and ")));
            nums.extend(off);
            level = Level::Warn;
            score = score.max(55);
        }
        self.push(Kind::Auto, level, score, "auto", at, None, nums, vec![routine], text);
    }
}

/// A commit as people write it: the first seven of a hash, else as it is.
fn short_code(code: &str) -> String {
    let code = code.trim();
    if code.len() > 7 && code.chars().all(|c| c.is_ascii_hexdigit()) {
        code[..7].to_owned()
    } else {
        code.chars().take(24).collect()
    }
}

fn enabled_time(changes: &[(f64, bool)], last: f64) -> f64 {
    let mut total = 0.0;
    let mut since = None;
    for (t, on) in changes {
        match (on, since) {
            (true, None) => since = Some(*t),
            (false, Some(s)) => {
                total += t - s;
                since = None;
            }
            _ => {}
        }
    }
    if let Some(s) = since {
        total += last - s;
    }
    total
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// "a", "a and b", "a, b and c".
pub fn join_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// --------------------------------------------------------------- keeping ---

fn level_name(l: Level) -> &'static str {
    match l {
        Level::Fault => "fault",
        Level::Warn => "warn",
        Level::Fine => "fine",
    }
}

fn level_from(s: &str) -> Level {
    match s {
        "fault" => Level::Fault,
        "warn" => Level::Warn,
        _ => Level::Fine,
    }
}

impl MatchFacts {
    pub fn to_json(&self) -> Value {
        json!({
            "layout": LAYOUT,
            "log": self.log, "bytes": self.bytes, "label": self.label,
            "seconds": self.seconds, "enabledSeconds": self.enabled_seconds,
            "code": self.code, "routine": self.routine, "truncated": self.truncated,
            "facts": self.facts.iter().map(|f| json!({
                "kind": f.kind.name(), "level": level_name(f.level), "score": f.score, "subject": f.subject,
                "at": f.at, "dur": f.dur,
                "nums": f.nums.iter().map(|n| json!([n.value, n.unit.name()])).collect::<Vec<_>>(),
                "names": f.names, "text": f.text,
            })).collect::<Vec<_>>(),
            "metrics": self.metrics,
            "signals": self.signals.iter().map(|s| json!([s.key, s.boolean, s.n, s.min, s.max, s.mean, s.at_min, s.at_max, s.last, s.true_s, s.rises])).collect::<Vec<_>>(),
        })
    }

    /// A kept sheet, if it is of this layout and was made from a log of
    /// exactly `bytes`.
    pub fn from_json(v: &Value, bytes: u64) -> Option<Self> {
        if v["layout"].as_u64()? != LAYOUT || v["bytes"].as_u64()? != bytes {
            return None;
        }
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let facts = v["facts"]
            .as_array()?
            .iter()
            .filter_map(|f| {
                Some(Fact {
                    kind: Kind::from_name(f["kind"].as_str()?)?,
                    level: level_from(f["level"].as_str()?),
                    score: f["score"].as_u64()? as u32,
                    subject: s(&f["subject"])?,
                    at: f["at"].as_f64(),
                    dur: f["dur"].as_f64(),
                    nums: f["nums"].as_array()?.iter().filter_map(|n| Some(Num::new(n[0].as_f64()?, Unit::from_name(n[1].as_str()?)))).collect(),
                    names: f["names"].as_array()?.iter().filter_map(s).collect(),
                    text: s(&f["text"])?,
                })
            })
            .collect();
        let metrics = v["metrics"].as_object()?.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect();
        let signals = v["signals"]
            .as_array()?
            .iter()
            .filter_map(|x| {
                Some(Signal {
                    key: s(&x[0])?,
                    boolean: x[1].as_bool()?,
                    n: x[2].as_u64()?,
                    min: x[3].as_f64()?,
                    max: x[4].as_f64()?,
                    mean: x[5].as_f64()?,
                    at_min: x[6].as_f64()?,
                    at_max: x[7].as_f64()?,
                    last: x[8].as_f64()?,
                    true_s: x[9].as_f64()?,
                    rises: x[10].as_u64()? as u32,
                })
            })
            .collect();
        Some(MatchFacts {
            log: s(&v["log"])?,
            bytes,
            label: s(&v["label"])?,
            seconds: v["seconds"].as_f64()?,
            enabled_seconds: v["enabledSeconds"].as_f64(),
            code: s(&v["code"]),
            routine: s(&v["routine"]),
            facts,
            metrics,
            signals,
            truncated: v["truncated"].as_bool().unwrap_or(false),
        })
    }
}

/// Where a log's sheet is kept: beside it.
pub fn sheet_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    dir.join(format!("{name}.facts.json"))
}

/// A log's sheet, and whether it was worked out just now: the kept one if
/// it is current, else worked out from the log (once: it is kept beside
/// it). `None` if the log cannot be read.
pub fn for_log(dir: &std::path::Path, name: &str) -> Option<(MatchFacts, bool)> {
    let path = dir.join(name);
    let bytes = std::fs::metadata(&path).ok()?.len();
    // A sheet is tens of kilobytes; one far larger is not one of these.
    let sane = std::fs::metadata(sheet_path(dir, name)).is_ok_and(|m| m.len() < 2_000_000);
    let kept = sane.then(|| std::fs::read(sheet_path(dir, name)).ok()).flatten().and_then(|t| serde_json::from_slice::<Value>(&t).ok()).and_then(|v| MatchFacts::from_json(&v, bytes));
    if let Some(kept) = kept {
        return Some((kept, false));
    }
    let file = std::fs::File::open(&path).ok()?;
    let facts = analyse(std::io::BufReader::with_capacity(64 * 1024, file), name, bytes).ok()?;
    crate::sys::write_atomic(&sheet_path(dir, name), facts.to_json().to_string().as_bytes()).ok();
    Some((facts, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_come_from_the_log_name() {
        assert_eq!(label_from_name("FRC_20261116_101500_NYRO_Q14.wpilog").as_deref(), Some("Q14"));
        assert_eq!(label_from_name("FRC_20261116_101500_NYRO_QM07.wpilog").as_deref(), Some("Q7"));
        assert_eq!(label_from_name("FRC_20261116_101500_NYRO_P2.wpilog").as_deref(), Some("P2"));
        assert_eq!(label_from_name("FRC_20261116_101500_NYRO_E11.wpilog").as_deref(), Some("E11"));
        assert_eq!(label_from_name("FRC_20261115_171800.wpilog"), None);
        assert_eq!(label_from_name("practice.wpilog"), None);
    }

    #[test]
    fn numbers_and_clocks_read_as_people_write_them() {
        assert_eq!(clock(78.0), "1:18");
        assert_eq!(clock(0.4), "0:00");
        assert_eq!(Num::new(6.42, Unit::Volts).shown(), "6.4 V");
        assert_eq!(Num::new(0.94 * 100.0, Unit::Percent).shown(), "94%");
        assert_eq!(Num::new(1.1, Unit::Meters).shown(), "1.10 m");
        assert_eq!(seconds(0.4), "0.4 s");
        assert_eq!(seconds(41.2), "41 s");
        assert_eq!(join_and(&["a".into(), "b".into(), "c".into()]), "a, b and c");
        assert_eq!(exception_in("Unhandled exception: java.lang.NullPointerException: x").as_deref(), Some("NullPointerException"));
        assert_eq!(exception_in("all good"), None);
    }

    #[test]
    fn the_sample_match_brownout_is_found_with_its_time_and_load() {
        let bytes = crate::wpilog::sample();
        let f = analyse(&bytes[..], "FRC_20261116_101500_NYRO_Q14.wpilog", bytes.len() as u64).unwrap();
        assert_eq!(f.label, "Q14");
        let brownout = f.of_kind(Kind::Brownout).next().expect("a brownout");
        assert_eq!(brownout.level, Level::Fault);
        // Enabled at 10 s, the dip at 88 s: 1:18 into the match.
        assert_eq!(brownout.at, Some(78.0));
        assert!(brownout.text.contains("at 1:18 in teleop"), "{}", brownout.text);
        assert_eq!(f.metric("brownouts"), Some(1.0));
        assert!(f.facts[0].kind == Kind::Brownout, "the worst first: {:?}", f.facts[0]);
        let heat = f.of_kind(Kind::Heat).next().expect("the elevator ran hot");
        assert_eq!(heat.subject, "Elevator");
        assert!(heat.nums[0].value > 79.0);
        let kept = MatchFacts::from_json(&f.to_json(), bytes.len() as u64).unwrap();
        assert_eq!(kept, f, "a kept sheet reads back whole");
        assert!(MatchFacts::from_json(&f.to_json(), 1).is_none(), "a sheet of another length is not it");
    }

    #[test]
    fn something_else_is_refused() {
        assert!(analyse(&b"not a log"[..], "x.wpilog", 9).is_err());
    }
}
