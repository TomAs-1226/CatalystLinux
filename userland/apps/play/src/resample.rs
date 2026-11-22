//! A song at another rate than the codec's 48 kHz (CDs' 44.1 kHz, mostly),
//! brought to the rate the device plays at (48 kHz, mostly): a polyphase windowed-sinc filter, 32 taps, one table row
//! per output phase (160 rows for 44.1 kHz, which is 147:160). About three
//! million multiply-adds a second for stereo: well under one percent of an
//! A53. Opus is 48 kHz already and passes straight through.

const HALF: usize = 16;

pub struct Resampler {
    up: u64,
    down: u64,
    /// `up` rows of `2 * HALF` taps.
    table: Vec<f32>,
    /// Input still needed, with the taps' history before it.
    buf: Vec<[f32; 2]>,
    /// The next output's place, in 1/`up` input samples from `buf[0]`.
    t: u64,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        let g = gcd(from as u64, to as u64).max(1);
        let (mut up, mut down) = (to as u64 / g, from as u64 / g);
        // A rate with no small ratio to 48 kHz: near enough, in 1/1000ths.
        if up > 2000 {
            let r = to as f64 / from as f64;
            up = 1000;
            down = (1000.0 / r).round() as u64;
        }
        let mut table = vec![0f32; (up as usize) * 2 * HALF];
        if up != down {
            // Below the lower of the two Nyquist frequencies, a little under.
            let cutoff = (up as f64 / down as f64).min(1.0) * 0.97;
            for p in 0..up as usize {
                let phase = p as f64 / up as f64;
                let row = &mut table[p * 2 * HALF..(p + 1) * 2 * HALF];
                let mut sum = 0.0;
                for (k, tap) in row.iter_mut().enumerate() {
                    // Input sample k sits at offset (k - HALF + 1) from the output's floor.
                    let x = phase - (k as f64 - HALF as f64 + 1.0);
                    let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * cutoff * x) };
                    // Blackman window across the taps.
                    let w = 0.5 + x / (2.0 * HALF as f64);
                    let window = if (0.0..=1.0).contains(&w) { 0.42 - 0.5 * (2.0 * std::f64::consts::PI * w).cos() + 0.08 * (4.0 * std::f64::consts::PI * w).cos() } else { 0.0 };
                    let v = sinc * window;
                    *tap = v as f32;
                    sum += v;
                }
                // Each row passes a steady level unchanged.
                for tap in row.iter_mut() {
                    *tap = (*tap as f64 / sum) as f32;
                }
            }
        }
        Resampler { up, down, table, buf: vec![[0.0; 2]; HALF - 1], t: 0 }
    }

    pub fn passes_through(&self) -> bool {
        self.up == self.down
    }

    /// Interleaved stereo in, at the song's rate (full scale ±1); out at the device's.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend(input.chunks_exact(2).map(|f| [f[0], f[1]]));
        loop {
            let at = (self.t / self.up) as usize;
            if at + 2 * HALF > self.buf.len() {
                break;
            }
            let phase = (self.t % self.up) as usize;
            let row = &self.table[phase * 2 * HALF..(phase + 1) * 2 * HALF];
            let window = &self.buf[at..at + 2 * HALF];
            let (mut l, mut r) = (0f32, 0f32);
            for (tap, s) in row.iter().zip(window) {
                l += tap * s[0];
                r += tap * s[1];
            }
            out.push(l);
            out.push(r);
            self.t += self.down;
        }
        // What no output needs any more, let go.
        let used = (self.t / self.up) as usize;
        if used > 0 {
            let used = used.min(self.buf.len());
            self.buf.drain(..used);
            self.t -= used as u64 * self.up;
        }
    }

    /// The song's last moments, through the filter.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if !self.passes_through() {
            self.process(&[0.0; 2 * HALF * 2], out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cd_rate_tone_comes_out_at_48k_the_same_pitch_and_level() {
        let mut r = Resampler::new(44_100, 48_000);
        assert!(!r.passes_through());
        // One second of a 1 kHz tone at half scale, in blocks as packets come.
        let tone: Vec<f32> = (0..44_100).flat_map(|n| {
            let v = ((2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 44_100.0).sin() * 0.5) as f32;
            [v, v]
        }).collect();
        let mut out = Vec::new();
        for block in tone.chunks(1152 * 2) {
            r.process(block, &mut out);
        }
        r.finish(&mut out);
        let frames = out.len() / 2;
        // A second, and the filter's half-length of tail.
        assert!((47_990..=48_030).contains(&frames), "{frames} frames");
        // Its zero crossings: 2000 a second, as before.
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).skip(100).take(47_000).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((1950..=1970).contains(&crossings), "{crossings}");
        let peak = left[1000..40_000].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((0.487..=0.513).contains(&peak), "{peak}");
    }

    #[test]
    fn opus_rate_passes_straight_through() {
        assert!(Resampler::new(48_000, 48_000).passes_through());
        let mut r = Resampler::new(96_000, 48_000);
        let mut out = Vec::new();
        r.process(&vec![0.0; 9600 * 2], &mut out);
        assert!((4780..=4800).contains(&(out.len() / 2)), "half as many");
    }
}
