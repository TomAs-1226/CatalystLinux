//! Reading a WPILib data log (`.wpilog`) into a match summary.
//!
//! The file is read as a stream, one record at a time, and only the handful
//! of values the summary needs are decoded, so a 50 MB match costs a few
//! kilobytes of memory. A log cut short (the robot lost power, or the pull is
//! still running) is summarised up to where it ends.
//!
//! Format: `WPILOG`, a u16 version, a u32-length extra header, then records.
//! Each record starts with a byte giving the widths of its entry id, payload
//! size and timestamp (µs). Entry 0 carries control records; a Start record
//! names an entry and its type.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};

/// What stood out in one log.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Time covered by the log.
    pub seconds: f64,
    /// Time the robot was enabled, when the log says.
    pub enabled_seconds: Option<f64>,
    pub battery_low: Option<f64>,
    pub brownouts: u32,
    pub worst_loop_ms: Option<f64>,
    /// Loops that ran over the 20 ms budget.
    pub loop_overruns: u32,
    /// Health checks that fired: ("Subsystem · id", times).
    pub fired: Vec<(String, u32)>,
    /// Hottest and hardest-driven mechanisms: (name, peak °C, peak A).
    pub mechanisms: Vec<(String, Option<f64>, Option<f64>)>,
    /// The busiest CAN bus at its busiest, as a fraction of its capacity.
    pub can_peak: Option<f64>,
    pub records: u64,
    /// The log ended mid-record.
    pub truncated: bool,
}

impl Summary {
    /// The findings worth reading first, worst first, in plain words.
    pub fn highlights(&self) -> Vec<(Level, String)> {
        let mut out = Vec::new();
        if self.brownouts > 0 {
            out.push((Level::Fault, format!("{} brownout{}", self.brownouts, plural(self.brownouts))));
        }
        if let Some(v) = self.battery_low {
            let level = if v < 9.0 { Level::Fault } else if v < 11.0 { Level::Warn } else { Level::Fine };
            out.push((level, format!("battery low point {v:.1} v")));
        }
        for (name, times) in &self.fired {
            out.push((Level::Warn, format!("{name} fired {times}×")));
        }
        if self.loop_overruns > 0 {
            let worst = self.worst_loop_ms.unwrap_or(0.0);
            out.push((Level::Warn, format!("{} loop{} over 20 ms, worst {worst:.0} ms", self.loop_overruns, plural(self.loop_overruns))));
        } else if let Some(ms) = self.worst_loop_ms {
            out.push((Level::Fine, format!("worst loop {ms:.1} ms")));
        }
        for (name, temp, amps) in &self.mechanisms {
            let level = if temp.is_some_and(|c| c >= 80.0) { Level::Fault } else if temp.is_some_and(|c| c >= 70.0) { Level::Warn } else { Level::Fine };
            let mut parts = Vec::new();
            if let Some(c) = temp {
                parts.push(format!("{c:.0} °c"));
            }
            if let Some(a) = amps {
                parts.push(format!("{a:.0} a"));
            }
            out.push((level, format!("{name} peak {}", parts.join(" · "))));
        }
        if let Some(can) = self.can_peak {
            let level = if can >= 0.9 { Level::Warn } else { Level::Fine };
            out.push((level, format!("CAN peak {:.0}%", can * 100.0)));
        }
        if self.truncated {
            out.push((Level::Fine, "log ends mid-record".into()));
        }
        out.sort_by_key(|(level, _)| *level);
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Fault,
    Warn,
    Fine,
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// What the summary wants from an entry, decided once from its name.
#[derive(Clone, Debug)]
enum Want {
    Battery,
    Brownout,
    LoopMs,
    Enabled,
    /// CAN utilisation: one number, or one per bus.
    Can,
    Firing(String),
    Temperature(String),
    Current(String),
}

fn want(name: &str, kind: &str) -> Option<Want> {
    // DataLogManager records NetworkTables as "NT:/Catalyst/…"; Catalyst's own
    // sink records the same keys without the prefix.
    if name == "DS:enabled" && kind == "boolean" {
        return Some(Want::Enabled);
    }
    let key = name.strip_prefix("NT:").unwrap_or(name);
    let key = key.strip_prefix("/Catalyst/").or_else(|| key.strip_prefix("Catalyst/")).unwrap_or(key).trim_start_matches('/');
    match (key, kind) {
        ("Systemcore/BatteryVolts", "double") => Some(Want::Battery),
        ("Systemcore/BrownedOut", "boolean") => Some(Want::Brownout),
        ("Loop/Robot/LastMs", "double") => Some(Want::LoopMs),
        ("Systemcore/CanUtilization", "double[]" | "double") => Some(Want::Can),
        _ => {
            if let Some(path) = key.strip_prefix("Health/").and_then(|k| k.strip_suffix("/firing")) {
                let (subsystem, id) = path.split_once('/')?;
                return (kind == "boolean").then(|| Want::Firing(format!("{subsystem} · {id}")));
            }
            if kind != "double" || key.starts_with("Systemcore/") {
                return None;
            }
            let (owner, field) = key.rsplit_once('/')?;
            match field {
                "TemperatureC" => Some(Want::Temperature(owner.to_owned())),
                "StatorCurrentAmps" => Some(Want::Current(owner.to_owned())),
                _ => None,
            }
        }
    }
}

/// The largest record read into memory. Every record the summary wants (a
/// start record with its name and metadata, a number, a flag) is far smaller.
const MAX_RECORD: u64 = 64 * 1024;

fn read_uint(r: &mut impl Read, width: usize) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf[..width])?;
    Ok(u64::from_le_bytes(buf))
}

/// Read a whole log and summarise it.
pub fn summarize(mut r: impl Read) -> io::Result<Summary> {
    let mut magic = [0u8; 6];
    r.read_exact(&mut magic)?;
    if &magic != b"WPILOG" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a wpilog file"));
    }
    read_uint(&mut r, 2)?; // version
    let extra = read_uint(&mut r, 4)?;
    io::copy(&mut r.by_ref().take(extra), &mut io::sink())?;

    let mut s = Summary::default();
    let mut wants: HashMap<u32, Want> = HashMap::new();
    let mut fired: BTreeMap<String, (bool, u32)> = BTreeMap::new();
    let mut peaks: BTreeMap<String, (Option<f64>, Option<f64>)> = BTreeMap::new();
    let (mut first, mut last) = (None::<u64>, 0u64);
    let mut browned = false;
    let mut enabled_since: Option<u64> = None;
    let mut enabled_us = 0u64;
    let mut saw_enabled = false;
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
        let record = (|| -> io::Result<(u32, u64, u64)> {
            Ok((read_uint(&mut r, widths.0)? as u32, read_uint(&mut r, widths.1)?, read_uint(&mut r, widths.2)?))
        })();
        let Ok((entry, size, time)) = record else {
            s.truncated = true;
            break;
        };
        // Control records and wanted entries are read; everything else is skipped unread.
        // So is anything larger than a record worth reading can be: a size
        // that large is a damaged file, and reading it whole would take
        // more memory than the handheld has.
        let wanted = if entry == 0 { None } else { wants.get(&entry) };
        if (entry != 0 && wanted.is_none()) || size > MAX_RECORD {
            if io::copy(&mut r.by_ref().take(size), &mut io::sink())? < size {
                s.truncated = true;
                break;
            }
            if size > MAX_RECORD {
                s.records += 1;
                continue;
            }
        } else {
            payload.resize(size as usize, 0);
            if r.read_exact(&mut payload).is_err() {
                s.truncated = true;
                break;
            }
        }
        s.records += 1;
        first.get_or_insert(time);
        last = last.max(time);

        if entry == 0 {
            // Start: type 0, entry id, then name, type and metadata as length-prefixed strings.
            if payload.first() == Some(&0) && payload.len() >= 9 {
                let id = u32::from_le_bytes(payload[1..5].try_into().unwrap());
                let mut at = 5;
                let mut text = || -> Option<&str> {
                    let len = u32::from_le_bytes(payload.get(at..at + 4)?.try_into().ok()?) as usize;
                    let out = std::str::from_utf8(payload.get(at + 4..at + 4 + len)?).ok()?;
                    at += 4 + len;
                    Some(out)
                };
                if let (Some(name), Some(kind)) = (text(), text()) {
                    if let Some(w) = want(name, kind) {
                        wants.insert(id, w);
                    }
                }
            }
            continue;
        }
        let number = || payload.get(..8).map(|b| f64::from_le_bytes(b.try_into().unwrap()));
        let flag = payload.first().map(|b| *b != 0);
        match wanted {
            Some(Want::Battery) => {
                if let Some(v) = number().filter(|v| *v > 0.5) {
                    s.battery_low = Some(s.battery_low.map_or(v, |low: f64| low.min(v)));
                }
            }
            Some(Want::Brownout) => {
                if flag == Some(true) && !browned {
                    s.brownouts += 1;
                }
                browned = flag == Some(true);
            }
            Some(Want::LoopMs) => {
                if let Some(ms) = number() {
                    s.worst_loop_ms = Some(s.worst_loop_ms.map_or(ms, |w: f64| w.max(ms)));
                    if ms > 20.0 {
                        s.loop_overruns += 1;
                    }
                }
            }
            Some(Want::Enabled) => {
                saw_enabled = true;
                match (flag, enabled_since) {
                    (Some(true), None) => enabled_since = Some(time),
                    (Some(false), Some(since)) => {
                        enabled_us += time.saturating_sub(since);
                        enabled_since = None;
                    }
                    _ => {}
                }
            }
            Some(Want::Can) => {
                // The busiest bus, whether one number or one per bus.
                let busiest = payload.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).filter(|v| v.is_finite()).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
                if let Some(v) = busiest {
                    s.can_peak = Some(s.can_peak.map_or(v, |p: f64| p.max(v)));
                }
            }
            Some(Want::Firing(name)) => {
                let state = fired.entry(name.clone()).or_insert((false, 0));
                if flag == Some(true) && !state.0 {
                    state.1 += 1;
                }
                state.0 = flag == Some(true);
            }
            Some(Want::Temperature(owner)) => {
                if let Some(v) = number() {
                    let peak = &mut peaks.entry(owner.clone()).or_default().0;
                    *peak = Some(peak.map_or(v, |p| p.max(v)));
                }
            }
            Some(Want::Current(owner)) => {
                if let Some(v) = number() {
                    let peak = &mut peaks.entry(owner.clone()).or_default().1;
                    *peak = Some(peak.map_or(v.abs(), |p| p.max(v.abs())));
                }
            }
            None => {}
        }
    }

    if let Some(since) = enabled_since {
        enabled_us += last.saturating_sub(since);
    }
    s.seconds = last.saturating_sub(first.unwrap_or(last)) as f64 / 1e6;
    s.enabled_seconds = saw_enabled.then_some(enabled_us as f64 / 1e6);
    s.fired = fired.into_iter().filter(|(_, (_, n))| *n > 0).map(|(name, (_, n))| (name, n)).collect();
    s.fired.sort_by(|a, b| b.1.cmp(&a.1));
    // The hottest few; a robot has dozens of motors and the screen has five lines.
    let mut mechanisms: Vec<_> = peaks.into_iter().map(|(name, (t, a))| (name, t, a)).collect();
    mechanisms.sort_by(|a, b| b.1.unwrap_or(0.0).total_cmp(&a.1.unwrap_or(0.0)));
    mechanisms.truncate(4);
    s.mechanisms = mechanisms;
    Ok(s)
}

/// Writes a small, realistic log: for the demo robot and for tests.
pub fn sample() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"WPILOG");
    out.extend_from_slice(&0x0100u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let record = |out: &mut Vec<u8>, entry: u32, time: u64, payload: &[u8]| {
        // 4-byte entry id, 4-byte size, 8-byte timestamp.
        out.push(0b0111_1111);
        out.extend_from_slice(&entry.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(payload);
    };
    let entries: [(&str, &str); 10] = [
        ("NT:/Catalyst/Systemcore/BatteryVolts", "double"),
        ("NT:/Catalyst/Systemcore/BrownedOut", "boolean"),
        ("NT:/Catalyst/Loop/Robot/LastMs", "double"),
        ("DS:enabled", "boolean"),
        ("NT:/Catalyst/Health/Elevator/HighTemp/firing", "boolean"),
        ("NT:/Catalyst/Health/CAN/can_s0/HighUtilization/firing", "boolean"),
        ("Elevator/TemperatureC", "double"),
        ("Elevator/StatorCurrentAmps", "double"),
        ("NT:/Catalyst/Swerve/HeadingDeg", "double"),
        ("NT:/Catalyst/Systemcore/CanUtilization", "double[]"),
    ];
    for (i, (name, kind)) in entries.iter().enumerate() {
        let mut p = vec![0u8];
        p.extend_from_slice(&(i as u32 + 1).to_le_bytes());
        for text in [*name, *kind, ""] {
            p.extend_from_slice(&(text.len() as u32).to_le_bytes());
            p.extend_from_slice(text.as_bytes());
        }
        record(&mut out, 0, 0, &p);
    }
    // 150 s at 10 Hz: disabled, a 135 s match, disabled.
    for step in 0..1500u64 {
        let (t, time) = (step as f64 / 10.0, step * 100_000);
        let playing = (10.0..145.0).contains(&t);
        let sag = if playing { 1.6 + 1.4 * ((t * 0.9).sin().abs()) } else { 0.0 };
        let dip = if (88.0..88.4).contains(&t) { 4.0 } else { 0.0 };
        let volts = 12.7 - sag - dip;
        let double = |out: &mut Vec<u8>, entry, v: f64| record(out, entry, time, &v.to_le_bytes());
        let boolean = |out: &mut Vec<u8>, entry, v: bool| record(out, entry, time, &[v as u8]);
        double(&mut out, 1, volts);
        boolean(&mut out, 2, volts < 6.8);
        double(&mut out, 3, if step == 884 { 41.0 } else if step % 211 == 0 && playing { 23.5 } else { 4.4 + (t * 3.0).sin() });
        boolean(&mut out, 4, playing);
        let temp = 44.0 + if playing { (t - 10.0) * 0.27 } else if t >= 145.0 { 36.0 } else { 0.0 };
        boolean(&mut out, 5, temp > 70.0);
        boolean(&mut out, 6, (60.0..64.0).contains(&t) || (100.0..103.0).contains(&t));
        double(&mut out, 7, temp);
        double(&mut out, 8, if playing { 22.0 + 30.0 * (t * 1.3).sin().abs() } else { 0.0 });
        double(&mut out, 9, (t * 12.0) % 360.0);
        // Five buses; the first busier while the match is played, busiest at
        // the brownout's scramble.
        let busy = if playing { 0.46 + 0.1 * (t * 0.4).sin().abs() } else { 0.18 } + if (86.0..92.0).contains(&t) { 0.18 } else { 0.0 };
        let buses: Vec<u8> = [busy, 0.12, 0.0, 0.21, 0.0].iter().flat_map(|v: &f64| v.to_le_bytes()).collect();
        record(&mut out, 10, time, &buses);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_the_sample_match() {
        let s = summarize(&sample()[..]).unwrap();
        assert!((s.seconds - 149.9).abs() < 0.01, "{}", s.seconds);
        assert_eq!(s.enabled_seconds.map(|e| e.round()), Some(135.0));
        assert_eq!(s.brownouts, 1);
        assert!(s.battery_low.unwrap() < 6.8);
        assert_eq!(s.worst_loop_ms, Some(41.0));
        assert!(s.loop_overruns >= 2, "{}", s.loop_overruns);
        assert_eq!(s.fired, vec![("CAN · can_s0/HighUtilization".to_owned(), 2), ("Elevator · HighTemp".to_owned(), 1)]);
        let (name, temp, amps) = &s.mechanisms[0];
        assert_eq!(name, "Elevator");
        assert!(temp.unwrap() > 79.0 && amps.unwrap() > 50.0);
        assert!(!s.truncated);
        assert_eq!(s.records, 10 + 1500 * 10);
        // The busiest of the five buses, at the scramble after the brownout.
        let can = s.can_peak.unwrap();
        assert!((0.7..0.75).contains(&can), "{can}");
        // The first thing read is the worst thing.
        let highlights = s.highlights();
        assert_eq!(highlights[0], (Level::Fault, "1 brownout".to_owned()));
    }

    #[test]
    fn a_log_cut_short_is_summarised_up_to_the_cut() {
        let bytes = sample();
        let s = summarize(&bytes[..bytes.len() / 2 + 3]).unwrap();
        assert!(s.truncated);
        assert!(s.seconds > 60.0 && s.seconds < 90.0, "{}", s.seconds);
    }

    #[test]
    fn a_damaged_record_claiming_gigabytes_is_not_read_into_memory() {
        // A good start, then a control record whose size says 4 GB, then the rest of the match.
        let good = sample();
        let mut bytes = good[..12].to_vec();
        bytes.push(0b0011_1111); // 4-byte id, 4-byte size, 4-byte time
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&good[12..]);
        // It comes back cut short where the damage is, rather than taking the shell down.
        let s = summarize(&bytes[..]).unwrap();
        assert!(s.truncated);
    }

    #[test]
    fn something_else_is_refused() {
        assert!(summarize(&b"not a log at all"[..]).is_err());
    }
}
