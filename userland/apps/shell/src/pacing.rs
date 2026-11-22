//! The frames as the handheld drew them, for checking it on the handheld
//! itself. With `/run/catalyst-pacing` there (`: > /run/catalyst-pacing`),
//! each motion is added to it as one line once it settles:
//!
//! ```text
//! motion frames=35 key=7.9ms first=6.1ms typical=25.0ms worst=26.3ms late=0 beat=25ms mhz=600>1000 boost=on
//! ```
//!
//! `key`: from the key going down (the kernel's time for the event) to the
//! first frame handed to the panel; `first`: that frame's own work; the
//! gaps between frame starts after it, typical (the median) and worst;
//! `late`: frames that took more than one beat; `mhz`: the clock as the
//! first frame began and as the last ended. Without the file nothing is
//! recorded, and a motion costs one look for it as it starts.
//! `CATALYST_PACING` names another file (the simulator, the emulator).

use std::time::Duration;

const FILE: &str = "/run/catalyst-pacing";

/// Now, in nanoseconds on the monotonic clock: the clock evdev stamps keys
/// with once asked to (see `platform::device`).
pub fn now_ns() -> i64 {
    #[cfg(target_os = "linux")]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a plain system call writing the struct given.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64
    }
    #[cfg(not(target_os = "linux"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as i64
    }
}

fn file() -> std::path::PathBuf {
    std::env::var_os("CATALYST_PACING").map_or_else(|| FILE.into(), Into::into)
}

/// The clock now, MHz (0 where there is no cpufreq).
fn mhz() -> u32 {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").ok().and_then(|s| s.trim().parse::<u32>().ok()).map_or(0, |khz| khz / 1000)
}

/// One motion being recorded.
#[derive(Default)]
pub struct Pacing {
    /// Recording this motion (the file was there as it began).
    on: bool,
    /// A motion is under way (its first frame drawn).
    begun: bool,
    /// The key that began it, by the kernel's clock.
    key_ns: Option<i64>,
    /// Each frame's start, ns.
    starts: Vec<i64>,
    /// The first frame: its work, and when it was handed to the panel.
    first: Option<(i64, i64)>,
    late: u32,
    mhz: (u32, u32),
    boost: bool,
}

impl Pacing {
    /// Keys arrived, the first stamped `at` (ns, monotonic). Only a key
    /// that begins a motion is timed to its frame.
    pub fn key(&mut self, at: Option<i64>) {
        if !self.begun && self.key_ns.is_none() {
            self.key_ns = Some(at.unwrap_or_else(now_ns));
        }
    }

    /// A frame drawn: begun at `start`, handed to the panel at `done` (ns);
    /// `beats` is how many beats it took.
    pub fn frame(&mut self, start: i64, done: i64, beats: u32, boost: bool) {
        if !self.begun {
            self.begun = true;
            self.on = file().exists();
            if self.on {
                self.mhz.0 = mhz();
                self.boost = boost;
                self.first = Some((done - start, done));
            }
        }
        if self.on {
            self.starts.push(start);
            self.late += u32::from(beats > 1);
        }
    }

    /// The motion has settled at this beat: its line, if it was recorded.
    pub fn settle(&mut self, beat: Duration) {
        if self.on {
            self.mhz.1 = mhz();
            let line = self.line(beat);
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(file()) {
                f.write_all(line.as_bytes()).ok();
            }
        }
        *self = Pacing { starts: std::mem::take(&mut self.starts), ..Pacing::default() };
        self.starts.clear();
    }

    fn line(&self, beat: Duration) -> String {
        let ms = |ns: i64| ns as f64 / 1e6;
        let mut gaps: Vec<i64> = self.starts.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_unstable();
        let typical = gaps.get(gaps.len() / 2).copied();
        let worst = gaps.last().copied();
        let opt = |v: Option<i64>| v.map_or("-".to_owned(), |ns| format!("{:.1}ms", ms(ns)));
        let key = match (self.key_ns, self.first) {
            (Some(k), Some((_, done))) if done >= k => opt(Some(done - k)),
            _ => "-".into(),
        };
        format!(
            "motion frames={} key={key} first={} typical={} worst={} late={} beat={:.0}ms mhz={}>{} boost={}\n",
            self.starts.len(),
            opt(self.first.map(|f| f.0)),
            opt(typical),
            opt(worst),
            self.late,
            beat.as_secs_f64() * 1e3,
            self.mhz.0,
            self.mhz.1,
            if self.boost { "on" } else { "off" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_motion_is_one_line_with_its_key_its_gaps_and_its_clock() {
        let mut p = Pacing { on: true, begun: true, key_ns: Some(1_000_000), boost: true, mhz: (600, 1000), ..Pacing::default() };
        p.first = Some((6_100_000, 8_900_000));
        p.starts = vec![2_800_000, 27_800_000, 52_800_000, 103_800_000, 128_800_000];
        p.late = 1;
        let line = p.line(Duration::from_millis(25));
        assert_eq!(line, "motion frames=5 key=7.9ms first=6.1ms typical=25.0ms worst=51.0ms late=1 beat=25ms mhz=600>1000 boost=on\n");
    }

    #[test]
    fn nothing_is_recorded_without_the_file() {
        std::env::set_var("CATALYST_PACING", std::env::temp_dir().join("catalyst-pacing-not-there"));
        let mut p = Pacing::default();
        p.key(Some(5));
        p.frame(10, 20, 1, true);
        p.frame(30, 40, 1, true);
        assert!(!p.on && p.starts.is_empty());
        p.settle(Duration::from_millis(25));
        assert!(p.key_ns.is_none() && !p.begun, "ready for the next motion");
    }
}
