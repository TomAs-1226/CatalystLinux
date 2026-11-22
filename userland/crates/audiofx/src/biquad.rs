//! Second-order filters (Robert Bristow-Johnson's cookbook), run in
//! transposed direct form II: five multiply-adds a sample.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Coeffs {
    pub const UNITY: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn normalised(b: [f64; 3], a: [f64; 3]) -> Coeffs {
        let a0 = a[0];
        Coeffs { b0: (b[0] / a0) as f32, b1: (b[1] / a0) as f32, b2: (b[2] / a0) as f32, a1: (a[1] / a0) as f32, a2: (a[2] / a0) as f32 }
    }

    fn w0(rate: u32, freq: f32) -> (f64, f64, f64) {
        let w = 2.0 * std::f64::consts::PI * (freq as f64 / rate as f64).min(0.49);
        (w, w.cos(), w.sin())
    }

    /// A bell `db` high at `freq`, `q` wide.
    pub fn peaking(rate: u32, freq: f32, q: f32, db: f32) -> Coeffs {
        let a = 10f64.powf(db as f64 / 40.0);
        let (_, cos, sin) = Self::w0(rate, freq);
        let alpha = sin / (2.0 * q as f64);
        Self::normalised([1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a], [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a])
    }

    /// Everything below `freq` lifted (or cut) by `db`.
    pub fn low_shelf(rate: u32, freq: f32, q: f32, db: f32) -> Coeffs {
        let a = 10f64.powf(db as f64 / 40.0);
        let (_, cos, sin) = Self::w0(rate, freq);
        let alpha = sin / (2.0 * q as f64);
        let s = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            [a * ((a + 1.0) - (a - 1.0) * cos + s), 2.0 * a * ((a - 1.0) - (a + 1.0) * cos), a * ((a + 1.0) - (a - 1.0) * cos - s)],
            [(a + 1.0) + (a - 1.0) * cos + s, -2.0 * ((a - 1.0) + (a + 1.0) * cos), (a + 1.0) + (a - 1.0) * cos - s],
        )
    }

    /// Everything above `freq` lifted (or cut) by `db`.
    pub fn high_shelf(rate: u32, freq: f32, q: f32, db: f32) -> Coeffs {
        let a = 10f64.powf(db as f64 / 40.0);
        let (_, cos, sin) = Self::w0(rate, freq);
        let alpha = sin / (2.0 * q as f64);
        let s = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            [a * ((a + 1.0) + (a - 1.0) * cos + s), -2.0 * a * ((a - 1.0) + (a + 1.0) * cos), a * ((a + 1.0) + (a - 1.0) * cos - s)],
            [(a + 1.0) - (a - 1.0) * cos + s, 2.0 * ((a - 1.0) - (a + 1.0) * cos), (a + 1.0) - (a - 1.0) * cos - s],
        )
    }

    /// Everything below `freq` taken away.
    pub fn high_pass(rate: u32, freq: f32, q: f32) -> Coeffs {
        let (_, cos, sin) = Self::w0(rate, freq);
        let alpha = sin / (2.0 * q as f64);
        Self::normalised([(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// What it does to a tone at `freq`, in decibels.
    pub fn db_at(&self, rate: u32, freq: f32) -> f32 {
        let w = 2.0 * PI * freq / rate as f32;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    c: Coeffs,
    s1: f32,
    s2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad::new(Coeffs::UNITY)
    }
}

impl Biquad {
    pub fn new(c: Coeffs) -> Biquad {
        Biquad { c, s1: 0.0, s2: 0.0 }
    }

    /// New coefficients, the filter's memory kept (no click as a band moves).
    pub fn set(&mut self, c: Coeffs) {
        self.c = c;
    }

    #[inline(always)]
    pub fn run(&mut self, x: f32) -> f32 {
        let c = &self.c;
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone through the filter, its level after it in decibels.
    pub fn tone_db(c: Coeffs, rate: u32, freq: f32) -> f32 {
        let mut f = Biquad::new(c);
        let n = rate as usize;
        let (mut sum_in, mut sum_out) = (0.0f64, 0.0f64);
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / rate as f32).sin();
            let y = f.run(x);
            if i > n / 2 {
                sum_in += (x * x) as f64;
                sum_out += (y * y) as f64;
            }
        }
        10.0 * (sum_out / sum_in).log10() as f32
    }

    #[test]
    fn the_cookbook_filters_do_what_they_say() {
        let r = 48_000;
        let bell = Coeffs::peaking(r, 1000.0, 1.0, 6.0);
        assert!((bell.db_at(r, 1000.0) - 6.0).abs() < 0.05);
        assert!(bell.db_at(r, 50.0).abs() < 0.2 && bell.db_at(r, 15_000.0).abs() < 0.2);
        assert!((tone_db(bell, r, 1000.0) - 6.0).abs() < 0.1);
        let low = Coeffs::low_shelf(r, 100.0, 0.707, -8.0);
        assert!((low.db_at(r, 20.0) + 8.0).abs() < 0.5 && low.db_at(r, 5000.0).abs() < 0.1);
        let high = Coeffs::high_shelf(r, 8000.0, 0.707, 5.0);
        assert!((high.db_at(r, 20_000.0) - 5.0).abs() < 0.6 && high.db_at(r, 200.0).abs() < 0.1);
        let hp = Coeffs::high_pass(r, 40.0, 0.5);
        assert!(hp.db_at(r, 10.0) < -20.0 && hp.db_at(r, 1000.0).abs() < 0.1);
        // Unity passes everything as it is.
        let mut f = Biquad::default();
        assert_eq!(f.run(0.3), 0.3);
    }
}
