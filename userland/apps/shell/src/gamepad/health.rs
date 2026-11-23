//! A controller's health from what it reported: each stick at rest (how far
//! from centre it sits, and how much it shakes there), each stick swept round
//! its edge (whether it reaches all the way), and how often reports came.
//! The thresholds are my own tuning, chosen for FRC driving, and say so.

/// A stick at rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rest {
    /// Where it sits on average, −1 to 1 each way.
    pub drift: (f32, f32),
    /// The furthest it was from centre.
    pub furthest: f32,
    /// Its shake: the spread (root mean square) of its distance from where it sits.
    pub noise: f32,
}

pub fn rest(samples: &[(f32, f32)]) -> Rest {
    if samples.is_empty() {
        return Rest::default();
    }
    let n = samples.len() as f32;
    let (sx, sy) = samples.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (mx, my) = (sx / n, sy / n);
    let var = samples.iter().map(|(x, y)| (x - mx).powi(2) + (y - my).powi(2)).sum::<f32>() / n;
    let furthest = samples.iter().map(|(x, y)| x.hypot(*y)).fold(0.0, f32::max);
    Rest { drift: (mx, my), furthest, noise: var.sqrt() }
}

impl Rest {
    pub fn drift_len(&self) -> f32 {
        self.drift.0.hypot(self.drift.1)
    }
}

/// The deadzone that keeps this stick still when untouched: the furthest it
/// sat from centre, with 0.02 to spare, up to the next hundredth, at least
/// 0.03 (my own tuning; WPILib leaves the number to the team).
pub fn deadzone(r: &Rest) -> f32 {
    let want = r.furthest + 0.02;
    ((want * 100.0 - 1e-3).ceil() / 100.0).max(0.03)
}

/// How far a stick reached each way while swept: right, left, down, up.
pub fn reach(samples: &[(f32, f32)]) -> [f32; 4] {
    samples.iter().fold([0.0f32; 4], |r, (x, y)| [r[0].max(*x), r[1].max(-x), r[2].max(*y), r[3].max(-y)])
}

/// The share of the edge a stick must reach each way to count as full
/// throw (my own tuning: worn sticks fall short by more than this).
pub const FULL: f32 = 0.95;

pub fn full(reach: &[f32; 4]) -> bool {
    reach.iter().all(|r| *r >= FULL)
}

/// The sectors round a stick's edge it has been pushed into (of 16), for
/// the sweep's progress.
pub const SECTORS: usize = 16;

pub fn sector(x: f32, y: f32) -> Option<usize> {
    (x.hypot(y) >= 0.85).then(|| (((y.atan2(x) / std::f32::consts::TAU + 1.0) * SECTORS as f32) as usize) % SECTORS)
}

/// The middle of some numbers.
pub fn median(v: &[i64]) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    Some(s[s.len() / 2])
}

/// How often reports came: the usual gap, and the longest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rate {
    pub hz: f32,
    pub median_ms: f32,
    pub worst_ms: f32,
}

/// From report times (µs), gaps over 100 ms left out: those are the stick
/// held still, when a controller only reports changes.
pub fn rate(times: &[i64]) -> Option<Rate> {
    let gaps: Vec<i64> = times.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 0 && *g <= 100_000).collect();
    let m = median(&gaps)?;
    let worst = *gaps.iter().max()?;
    Some(Rate { hz: 1e6 / m as f32, median_ms: m as f32 / 1000.0, worst_ms: worst as f32 / 1000.0 })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Good,
    Fair,
    Worn,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Good => "good",
            Verdict::Fair => "fair",
            Verdict::Worn => "worn",
        }
    }
    pub fn from_word(s: &str) -> Verdict {
        match s {
            "good" => Verdict::Good,
            "fair" => Verdict::Fair,
            _ => Verdict::Worn,
        }
    }
}

/// My own tuning: good with a deadzone of 0.08 or less on both sticks, both
/// reaching their edge and reports at least 100 a second; worn past 0.15,
/// or a stick short of 0.85 somewhere; fair between.
pub fn verdict(deadzones: [f32; 2], reaches: [[f32; 4]; 2], rate: Option<Rate>) -> Verdict {
    let dz = deadzones[0].max(deadzones[1]);
    let short = reaches.iter().flatten().fold(1.0f32, |a, b| a.min(*b));
    if dz > 0.15 + 1e-6 || short < 0.85 {
        Verdict::Worn
    } else if dz <= 0.08 + 1e-6 && short >= FULL && rate.is_none_or(|r| r.hz >= 100.0) {
        Verdict::Good
    } else {
        Verdict::Fair
    }
}
