//! A battery's health from the match it was in, read from the match's log,
//! and its health over the matches it has been in.
//!
//! - **One match** ([`analyse`]): the volts it started at (resting, just
//!   before the robot was first enabled), the lowest it fell to while
//!   enabled, how far it sagged under load (from the start to the 5th
//!   percentile while enabled, so one spike does not decide it), and the
//!   brownouts the log recorded.
//! - **Internal resistance**, where the log has the robot's total current
//!   (a PDH's `TotalCurrent`, AdvantageKit's `SystemStats/BatteryCurrent`):
//!   ΔV/ΔI, as the slope of volts on amps over the enabled samples, fitted
//!   with time beside it so the battery's slow fall in charge over a match
//!   is not taken for resistance. It includes the robot's wiring and main
//!   breaker, so it reads a little above a battery tester's figure; what
//!   matters is that one battery's rises against its own past.
//! - **Without current**, a sag-based score stands in: how far it sagged.
//!
//! The thresholds here are my own tuning (no FRC source gives one number):
//! a new battery measured on a robot reads about 15–20 mΩ, a tired one 30.

use std::io::{self, Read};

use serde_json::{json, Value};

/// What one match took out of a battery.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// Resting volts just before the robot was first enabled.
    pub start_v: f64,
    /// The lowest while enabled.
    pub low_v: f64,
    /// Start less the 5th percentile while enabled: the sag under load.
    pub sag_v: f64,
    /// ΔV/ΔI in milliohms, where the log has the total current.
    pub mohm: Option<f64>,
    pub brownouts: u32,
    /// The highest total current, where logged.
    pub peak_a: Option<f64>,
}

impl Reading {
    /// 0 to 100: from the resistance where it was measured (15 mΩ and under
    /// 100, 35 and over 0), else from the sag (1 V and under 100, 4 V and
    /// over 0); a quarter off for each brownout. My own tuning.
    pub fn score(&self) -> f64 {
        let base = match self.mohm {
            Some(r) => (35.0 - r) / (35.0 - 15.0),
            None => (4.0 - self.sag_v) / (4.0 - 1.0),
        };
        (base.clamp(0.0, 1.0) * 100.0 - 25.0 * self.brownouts as f64).max(0.0)
    }

    pub fn to_json(&self) -> Value {
        json!({ "start": round2(self.start_v), "low": round2(self.low_v), "sag": round2(self.sag_v), "mohm": self.mohm.map(round2), "brownouts": self.brownouts, "peak": self.peak_a.map(round2) })
    }

    pub fn from_json(v: &Value) -> Option<Reading> {
        Some(Reading { start_v: v["start"].as_f64()?, low_v: v["low"].as_f64()?, sag_v: v["sag"].as_f64()?, mohm: v["mohm"].as_f64(), brownouts: v["brownouts"].as_u64().unwrap_or(0) as u32, peak_a: v["peak"].as_f64() })
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// What a log entry is to the analysis.
#[derive(Clone, Copy)]
enum Want {
    Volts,
    Amps,
    Brownout,
    Enabled,
}

/// A battery's volts: Catalyst's, or AdvantageKit's.
fn is_volts(key: &str) -> bool {
    matches!(key, "Systemcore/BatteryVolts" | "SystemStats/BatteryVoltage" | "PowerDistribution/Voltage")
}

/// The robot's total current: a PDH or PDP's, AdvantageKit's, or Catalyst's own.
fn is_amps(key: &str) -> bool {
    ["TotalCurrent", "TotalCurrentAmps", "BatteryCurrent", "BatteryCurrentAmps"].iter().any(|end| key.rsplit('/').next() == Some(end))
}

/// A current sample this close to a voltage sample (µs) is taken as its pair.
const PAIRED_US: u64 = 40_000;
/// Samples from this long (µs) before the first enable give the resting volts.
const RESTING_US: u64 = 3_000_000;

/// Least squares of volts on amps and time, kept as sums.
#[derive(Default)]
struct Fit {
    n: f64,
    // Means are taken from the sums; time in seconds from the first pair.
    s: [f64; 9],
    t0: Option<u64>,
}

impl Fit {
    fn add(&mut self, t_us: u64, i: f64, v: f64) {
        let t = (t_us - *self.t0.get_or_insert(t_us)) as f64 / 1e6;
        self.n += 1.0;
        let s = &mut self.s;
        s[0] += t;
        s[1] += i;
        s[2] += v;
        s[3] += t * t;
        s[4] += t * i;
        s[5] += i * i;
        s[6] += t * v;
        s[7] += i * v;
        s[8] += v * v;
    }

    /// R (ohms) from v = a + b·t − R·i: the two-regressor fit on centred sums.
    /// None when there are too few samples, the current hardly varies, or
    /// the fit explains too little to be believed.
    fn resistance(&self) -> Option<f64> {
        let n = self.n;
        if n < 50.0 {
            return None;
        }
        let s = &self.s;
        let (mt, mi, mv) = (s[0] / n, s[1] / n, s[2] / n);
        let stt = s[3] - n * mt * mt;
        let sti = s[4] - n * mt * mi;
        let sii = s[5] - n * mi * mi;
        let stv = s[6] - n * mt * mv;
        let siv = s[7] - n * mi * mv;
        let svv = s[8] - n * mv * mv;
        // The current must swing: at least 5 A standard deviation.
        if sii / n < 25.0 || svv <= 0.0 {
            return None;
        }
        let det = stt * sii - sti * sti;
        if det.abs() < 1e-9 {
            return None;
        }
        let b_i = (stt * siv - sti * stv) / det;
        let b_t = (sii * stv - sti * siv) / det;
        // How much of the volts' variance the fit explains.
        let r2 = (b_i * siv + b_t * stv) / svv;
        let r = -b_i;
        (r2 >= 0.25 && (0.002..0.15).contains(&r)).then_some(r)
    }
}

/// Read a match's log for the battery's part in it. None when the log has
/// no battery volts at all.
pub fn analyse(r: impl Read) -> io::Result<Option<Reading>> {
    let mut claimed = (false, false, false);
    let mut volts: Vec<(u64, f32)> = Vec::new();
    let mut amps: Option<(u64, f64)> = None;
    let mut peak_a: Option<f64> = None;
    let mut fit = Fit::default();
    let mut brownouts = 0u32;
    let mut browned = false;
    let mut enabled = false;
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let mut since: Option<u64> = None;
    let mut first_enable: Option<u64> = None;
    let mut last = 0u64;
    let start = |name: &str, kind: &str| -> Option<Want> {
        if name == "DS:enabled" && kind == "boolean" {
            return Some(Want::Enabled);
        }
        let key = crate::wpilog::catalyst_key(name);
        // Each taken once: a log can record the same value twice (the
        // NetworkTables copy and the robot's own), at the same times.
        match kind {
            "double" if is_volts(key) && !claimed.0 => {
                claimed.0 = true;
                Some(Want::Volts)
            }
            "double" if is_amps(key) && !claimed.1 => {
                claimed.1 = true;
                Some(Want::Amps)
            }
            "boolean" if matches!(key, "Systemcore/BrownedOut" | "SystemStats/BrownedOut") && !claimed.2 => {
                claimed.2 = true;
                Some(Want::Brownout)
            }
            _ => None,
        }
    };
    let value = |w: &Want, time: u64, payload: &[u8]| {
        last = last.max(time);
        let number = payload.get(..8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).filter(|v| v.is_finite());
        let flag = payload.first().map(|b| *b != 0);
        match w {
            Want::Volts => {
                let Some(v) = number.filter(|v| *v > 0.5) else { return };
                volts.push((time, v as f32));
                if enabled {
                    if let Some((at, i)) = amps.filter(|(at, _)| time.abs_diff(*at) <= PAIRED_US) {
                        let _ = at;
                        fit.add(time, i, v);
                    }
                }
            }
            Want::Amps => {
                if let Some(i) = number {
                    amps = Some((time, i));
                    if enabled {
                        peak_a = Some(peak_a.map_or(i, |p: f64| p.max(i)));
                    }
                }
            }
            Want::Brownout => {
                if flag == Some(true) && !browned {
                    brownouts += 1;
                }
                browned = flag == Some(true);
            }
            Want::Enabled => match (flag, since) {
                (Some(true), None) => {
                    enabled = true;
                    since = Some(time);
                    first_enable.get_or_insert(time);
                }
                (Some(false), Some(from)) => {
                    enabled = false;
                    spans.push((from, time));
                    since = None;
                }
                _ => {}
            },
        }
    };
    crate::wpilog::read_entries(r, start, value)?;
    if let Some(from) = since {
        spans.push((from, last.max(from)));
    }
    if volts.is_empty() {
        return Ok(None);
    }
    // Resting: the samples just before the first enable; with no enable
    // recorded, the log's first seconds.
    let first = volts[0].0;
    let (from, to) = match first_enable {
        Some(at) => (at.saturating_sub(RESTING_US), at),
        None => (first, first + RESTING_US),
    };
    let mut resting: Vec<f32> = volts.iter().filter(|(t, _)| (from..to).contains(t)).map(|(_, v)| *v).collect();
    if resting.is_empty() {
        resting.push(volts.iter().find(|(t, _)| *t >= from).map_or(volts[0].1, |(_, v)| *v));
    }
    let start_v = median(&mut resting) as f64;
    let mut loaded: Vec<f32> = if spans.is_empty() { volts.iter().map(|(_, v)| *v).collect() } else { volts.iter().filter(|(t, _)| spans.iter().any(|(a, b)| (*a..=*b).contains(t))).map(|(_, v)| *v).collect() };
    if loaded.is_empty() {
        loaded = volts.iter().map(|(_, v)| *v).collect();
    }
    loaded.sort_by(f32::total_cmp);
    let low_v = loaded[0] as f64;
    let p5 = loaded[(loaded.len() - 1) * 5 / 100] as f64;
    Ok(Some(Reading { start_v, low_v, sag_v: (start_v - p5).max(0.0), mohm: fit.resistance().map(|r| r * 1000.0), brownouts, peak_a }))
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn median64(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

// ---------------------------------------------------------- over matches ---

/// Rising past this much above its own first matches, a battery's
/// resistance says retire it soon; and at this many milliohms, whatever its
/// past. My own tuning.
pub const RESISTANCE_RISE: f64 = 1.25;
pub const RESISTANCE_MOST: f64 = 30.0;
/// The same for the sag where no current is logged: this many volts more
/// than its first matches, or this deep at all.
pub const SAG_RISE: f64 = 0.6;
pub const SAG_MOST: f64 = 3.2;
/// Brownouts in this many of its latest matches flag it.
pub const BROWNOUT_MATCHES: usize = 3;

/// What its trend line shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    Resistance,
    Sag,
}

/// A battery's health over the matches it was in, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Health {
    /// 0 to 100, from its last three matches; None before its first.
    pub score: Option<f64>,
    pub measure: Measure,
    /// The trend: resistance (mΩ) where measured in two matches or more,
    /// else sag (V), one point a match, oldest first.
    pub trend: Vec<f64>,
    /// Why it should be retired soon, if it should.
    pub retire: Option<String>,
    pub brownouts: u32,
}

/// Its health from its matches, each (label, reading), oldest first.
pub fn health(matches: &[(&str, &Reading)]) -> Health {
    let resistances: Vec<f64> = matches.iter().filter_map(|(_, r)| r.mohm).collect();
    let measure = if resistances.len() >= 2 || (!resistances.is_empty() && resistances.len() == matches.len()) { Measure::Resistance } else { Measure::Sag };
    let trend: Vec<f64> = match measure {
        Measure::Resistance => resistances.clone(),
        Measure::Sag => matches.iter().map(|(_, r)| r.sag_v).collect(),
    };
    let recent = &matches[matches.len().saturating_sub(3)..];
    let score = (!recent.is_empty()).then(|| recent.iter().map(|(_, r)| r.score()).sum::<f64>() / recent.len() as f64);
    Health { score, measure, retire: retire(matches, measure, &trend), trend, brownouts: matches.iter().map(|(_, r)| r.brownouts).sum() }
}

/// Why to retire it soon: brownouts lately, or its measure risen past its
/// own first matches (or past what any battery should read).
fn retire(matches: &[(&str, &Reading)], measure: Measure, trend: &[f64]) -> Option<String> {
    let lately = &matches[matches.len().saturating_sub(BROWNOUT_MATCHES)..];
    if let Some((label, _)) = lately.iter().rev().find(|(_, r)| r.brownouts > 0) {
        return Some(format!("browned out in {label}"));
    }
    if trend.is_empty() {
        return None;
    }
    // Its latest matches against its first, as many of each (up to three)
    // as keeps the two apart.
    let k = (trend.len() / 2).clamp(1, 3);
    let now = median64(trend[trend.len() - k..].to_vec());
    let base = median64(trend[..k].to_vec());
    let enough = trend.len() >= 3;
    match measure {
        Measure::Resistance if now >= RESISTANCE_MOST => Some(format!("{now:.0} mOhm")),
        Measure::Resistance if enough && now >= base * RESISTANCE_RISE => Some(format!("resistance up {:.0}%", (now / base - 1.0) * 100.0)),
        Measure::Sag if now >= SAG_MOST => Some(format!("sags {now:.1} v")),
        Measure::Sag if enough && now >= base + SAG_RISE => Some(format!("sag up {:.1} v", now - base)),
        _ => None,
    }
}

// ------------------------------------------------------- synthetic logs ---

/// A match's log made up for the tests and the gallery: volts, the total
/// current where `logs_current`, brownouts and the driver station's enable,
/// at 50 Hz: five seconds resting, a 2:40 match, five seconds after. The
/// battery has `mohm` of resistance and starts at `rest_v`, losing half a
/// volt of charge over the match; `hard` scales the load (1 is a usual
/// match, peaks near 200 A).
#[cfg(any(test, not(target_arch = "aarch64")))]
pub fn synthetic(rest_v: f64, mohm: f64, hard: f64, logs_current: bool, seed: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"WPILOG");
    out.extend_from_slice(&0x0100u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let record = |out: &mut Vec<u8>, entry: u32, time: u64, payload: &[u8]| {
        out.push(0b0111_1111);
        out.extend_from_slice(&entry.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(payload);
    };
    let mut entries = vec![("NT:/Catalyst/Systemcore/BatteryVolts", "double"), ("NT:/Catalyst/Systemcore/BrownedOut", "boolean"), ("DS:enabled", "boolean")];
    if logs_current {
        entries.push(("NT:/PowerDistribution/TotalCurrent", "double"));
    }
    for (i, (name, kind)) in entries.iter().enumerate() {
        let mut p = vec![0u8];
        p.extend_from_slice(&(i as u32 + 1).to_le_bytes());
        for text in [*name, *kind, ""] {
            p.extend_from_slice(&(text.len() as u32).to_le_bytes());
            p.extend_from_slice(text.as_bytes());
        }
        record(&mut out, 0, 0, &p);
    }
    let mut noise = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        noise ^= noise << 13;
        noise ^= noise >> 7;
        noise ^= noise << 17;
        (noise >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let (pre, play, post) = (5.0, 160.0, 5.0);
    let steps = ((pre + play + post) * 50.0) as u64;
    let mut was_enabled = None;
    for k in 0..steps {
        let t = k as f64 / 50.0;
        let time = k * 20_000;
        let m = t - pre;
        let enabled = (0.0..play).contains(&m);
        if was_enabled != Some(enabled) {
            record(&mut out, 3, time, &[enabled as u8]);
            was_enabled = Some(enabled);
        }
        // The load: the drive's surges, a mechanism's pulls, a shove now and then.
        let amps = if enabled {
            let drive = 70.0 * (m * 0.9 + seed as f64).sin().abs() + 35.0 * (m * 2.3).sin().max(0.0);
            let shove = if (m % 23.0) < 1.2 { 70.0 } else { 0.0 };
            (12.0 + drive + shove) * hard + 3.0 * next()
        } else {
            1.5 + 0.3 * next()
        };
        let charge = if m > 0.0 { 0.5 * (m.min(play) / play) } else { 0.0 };
        let volts = rest_v - charge - amps * mohm / 1000.0 + 0.02 * next();
        record(&mut out, 1, time, &volts.to_le_bytes());
        record(&mut out, 2, time, &[(volts < 6.8) as u8]);
        if logs_current {
            record(&mut out, 4, time + 4_000, &amps.max(0.0).to_le_bytes());
        }
    }
    out
}
