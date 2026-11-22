//! The equalizer: five bands (a low shelf, three bells and a high shelf),
//! from presets or set by hand. A flat band costs nothing; all flat, the
//! player leaves the samples alone.

use crate::biquad::{Biquad, Coeffs};

/// The bands: where each sits, in hertz.
pub const FREQS: [f32; 5] = [60.0, 250.0, 1000.0, 4000.0, 12_000.0];
/// How far a band may go either way, in decibels.
pub const LIMIT: i8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Flat,
    Bass,
    Vocal,
    Treble,
    /// Bass and treble brought up as the volume goes down, as the ear
    /// hears less of them at low levels (Fletcher and Munson).
    Loudness,
    /// The bands as set by hand.
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 6] = [Preset::Flat, Preset::Bass, Preset::Vocal, Preset::Treble, Preset::Loudness, Preset::Custom];

    pub fn word(self) -> &'static str {
        match self {
            Preset::Flat => "flat",
            Preset::Bass => "bass",
            Preset::Vocal => "vocal",
            Preset::Treble => "treble",
            Preset::Loudness => "loudness",
            Preset::Custom => "custom",
        }
    }

    pub fn parse(word: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.word() == word.trim())
    }

    /// The bands' gains, in decibels, at a volume (0 to 100).
    pub fn gains(self, custom: [i8; 5], volume: u32) -> [f32; 5] {
        match self {
            Preset::Flat => [0.0; 5],
            Preset::Bass => [6.0, 2.5, 0.0, 0.0, 0.0],
            Preset::Vocal => [-2.0, -1.0, 2.0, 3.0, 0.0],
            Preset::Treble => [0.0, 0.0, 0.0, 2.0, 5.0],
            Preset::Loudness => {
                // Nothing at 70 % and above; all of it by 10 %.
                let k = ((70.0 - volume as f32) / 60.0).clamp(0.0, 1.0);
                [9.0 * k, 3.0 * k, 0.0, 0.0, 4.0 * k]
            }
            Preset::Custom => custom.map(|g| g.clamp(-LIMIT, LIMIT) as f32),
        }
    }

    /// Whether it changes anything at this volume.
    pub fn is_flat(self, custom: [i8; 5], volume: u32) -> bool {
        self.gains(custom, volume).iter().all(|g| g.abs() < 0.05)
    }
}

fn band(rate: u32, i: usize, db: f32) -> Coeffs {
    match i {
        0 => Coeffs::low_shelf(rate, FREQS[0], 0.707, db),
        4 => Coeffs::high_shelf(rate, FREQS[4], 0.707, db),
        _ => Coeffs::peaking(rate, FREQS[i], 1.0, db),
    }
}

/// The equalizer on stereo frames.
pub struct Equalizer {
    rate: u32,
    gains: [f32; 5],
    /// Each band's filter for the left and right channels.
    filters: [[Biquad; 2]; 5],
    /// Turned down by half the largest boost, so a lift seldom needs the
    /// soft clip at the end.
    pre: f32,
    /// The bands that are not flat: the first `active` of these.
    order: [usize; 5],
    active: usize,
}

impl Equalizer {
    pub fn new(rate: u32) -> Equalizer {
        Equalizer { rate, gains: [0.0; 5], filters: [[Biquad::default(); 2]; 5], pre: 1.0, order: [0; 5], active: 0 }
    }

    pub fn gains(&self) -> [f32; 5] {
        self.gains
    }

    /// New gains, in decibels. The filters keep their memory, so a band
    /// moving while a song plays does not click.
    pub fn set(&mut self, gains: [f32; 5]) {
        if gains == self.gains {
            return;
        }
        for (i, &db) in gains.iter().enumerate() {
            let c = band(self.rate, i, db);
            self.filters[i].iter_mut().for_each(|f| f.set(c));
        }
        self.gains = gains;
        self.active = 0;
        for (i, db) in gains.iter().enumerate() {
            if db.abs() >= 0.05 {
                self.order[self.active] = i;
                self.active += 1;
            }
        }
        let boost = gains.iter().cloned().fold(0.0f32, f32::max);
        self.pre = crate::db_to_gain(-boost * 0.5);
    }

    pub fn is_flat(&self) -> bool {
        self.gains.iter().all(|g| g.abs() < 0.05)
    }

    pub fn process(&mut self, frames: &mut [f32]) {
        if self.active == 0 {
            return;
        }
        let (pre, order) = (self.pre, self.order);
        let active = &order[..self.active];
        for frame in frames.chunks_exact_mut(2) {
            let (mut l, mut r) = (frame[0] * pre, frame[1] * pre);
            for &i in active {
                let [fl, fr] = &mut self.filters[i];
                l = fl.run(l);
                r = fr.run(r);
            }
            frame[0] = l;
            frame[1] = r;
        }
    }

    pub fn reset(&mut self) {
        self.filters.iter_mut().flatten().for_each(Biquad::reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_through(eq: &mut Equalizer, freq: f32) -> f32 {
        let rate = 48_000;
        let mut frames: Vec<f32> = (0..rate).flat_map(|i| {
            let x = 0.25 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin();
            [x, x]
        }).collect();
        eq.reset();
        eq.process(&mut frames);
        let tail = &frames[rate as usize..];
        let rms = (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt();
        crate::gain_to_db(rms / (0.25 / 2f32.sqrt()))
    }

    #[test]
    fn presets_lift_and_cut_where_they_say_and_flat_is_untouched() {
        let mut eq = Equalizer::new(48_000);
        assert!(eq.is_flat());
        let mut same = vec![0.1, -0.2, 0.3, 0.4];
        eq.process(&mut same);
        assert_eq!(same, [0.1, -0.2, 0.3, 0.4]);
        eq.set(Preset::Bass.gains([0; 5], 50));
        // The preamp takes 3 dB off everything; the bass sits 6 dB over that.
        let low = tone_through(&mut eq, 40.0);
        let mid = tone_through(&mut eq, 2000.0);
        assert!((mid + 3.0).abs() < 0.5, "{mid}");
        assert!(low - mid > 4.5, "{low} {mid}");
        eq.set(Preset::Treble.gains([0; 5], 50));
        assert!(tone_through(&mut eq, 15_000.0) - tone_through(&mut eq, 300.0) > 4.0);
        eq.set(Preset::Custom.gains([0, 0, -6, 0, 0], 50));
        assert!((tone_through(&mut eq, 1000.0) + 6.0).abs() < 0.5);
    }

    #[test]
    fn loudness_grows_as_the_volume_falls_and_custom_bands_stay_in_bounds() {
        assert!(Preset::Loudness.is_flat([0; 5], 80));
        let quiet = Preset::Loudness.gains([0; 5], 10);
        let middling = Preset::Loudness.gains([0; 5], 40);
        assert!(quiet[0] > middling[0] && middling[0] > 0.0);
        assert_eq!(quiet[2], 0.0);
        assert_eq!(Preset::Custom.gains([30, -30, 0, 0, 0], 50)[..2], [12.0, -12.0]);
        assert_eq!(Preset::parse("vocal"), Some(Preset::Vocal));
        assert_eq!(Preset::parse("nonsense"), None);
    }
}
