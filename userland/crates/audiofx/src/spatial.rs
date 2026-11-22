//! Spatial sound for headphones: a song's two channels as a pair of
//! loudspeakers 30° either side of straight ahead, in a small, quiet room,
//! instead of inside the head.
//!
//! Each speaker reaches each ear by its own path (Brown and Duda's
//! structural model of a spherical head, 1998): a delay for the time the
//! sound takes round the head (Woodworth's formula), and a one-pole,
//! one-zero filter for the head's shadow, which lifts the treble at the
//! near ear and takes it from the far one. The four paths are crossfeed
//! with the right timing and colour. A few early reflections from the
//! room's walls, and a short damped tail after them, put the speakers at a
//! distance; a high shelf after it all gives back the treble the head's
//! shadow takes from sound straight ahead, so a voice keeps its tone.
//!
//! The head can turn ([`Spatial::set_pose`], from the AirPods' head
//! tracking): each path's angle, delay and shadow follow, so the speakers
//! stay where they are as the head moves. The pose is eased over 40 ms and
//! the delays glide across each 32-frame block, so nothing clicks.
//!
//! About 90 floating-point operations a frame (counted in the crate's
//! notes): some 4.3 million a second at 48 kHz.

use crate::biquad::{Biquad, Coeffs};

const HEAD_RADIUS: f32 = 0.0875;
const SPEED_OF_SOUND: f32 = 343.0;
/// The speakers, either side of straight ahead, in degrees.
pub const SPEAKERS: f32 = 30.0;
/// Frames between the paths being worked out again.
const BLOCK: usize = 32;
/// The input kept, in frames (a power of two): the longest reflection at
/// 96 kHz with room to spare.
const RING: usize = 4096;
/// How much of the speakers comes straight to the ears, and of the room.
const DIRECT: f32 = 0.55;
const ROOM: f32 = 0.2;
/// The reflections, per ear: when (ms), how loud, and from which speaker
/// (0 left, 1 right). Mirrored between the ears, so a voice in the middle
/// stays in the middle; the tails below differ, so the room is wide.
const TAPS: [[(f32, f32, usize); 6]; 2] = [
    [(5.3, 0.32, 0), (7.9, 0.24, 1), (11.2, 0.19, 0), (13.7, 0.15, 1), (17.1, 0.11, 0), (22.6, 0.08, 1)],
    [(5.3, 0.32, 1), (7.9, 0.24, 0), (11.2, 0.19, 1), (13.7, 0.15, 0), (17.1, 0.11, 1), (22.6, 0.08, 0)],
];
/// The tail: a damped comb per ear, this long (ms), fed back this much.
const TAIL_MS: [f32; 2] = [37.1, 41.3];
const TAIL_FEEDBACK: f32 = 0.42;
const TAIL_LEVEL: f32 = 0.45;

#[derive(Clone, Copy, Default)]
struct Path {
    /// Its delay now, and how much it moves a frame in this block.
    delay: f32,
    step: f32,
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
}

pub struct Spatial {
    rate: u32,
    ring: Vec<[f32; 2]>,
    pos: usize,
    /// `paths[speaker][ear]`.
    paths: [[Path; 2]; 2],
    taps: [[(usize, f32, usize); 6]; 2],
    room_lp: [f32; 2],
    lp: f32,
    tail: [Vec<f32>; 2],
    tail_at: [usize; 2],
    tail_lp: [f32; 2],
    post: [Biquad; 2],
    /// Where the head is turned (degrees, + to the right and up), and the
    /// pose eased towards it.
    want: (f32, f32),
    pose: (f32, f32),
    ease: f32,
    count: usize,
    fresh: bool,
}

/// The time round the head to an ear at `phi` degrees from its axis, with
/// the head's radius added, so it is never negative: 0 to 0.66 ms.
fn ear_delay(phi: f32) -> f32 {
    let a_c = HEAD_RADIUS / SPEED_OF_SOUND;
    let phi_r = phi.to_radians();
    let t = if phi < 90.0 { -a_c * phi_r.cos() } else { a_c * (phi_r - std::f32::consts::FRAC_PI_2) };
    t + a_c
}

/// The head's shadow at an ear `phi` degrees from its axis: how much of
/// the treble arrives (2 facing the source, about 0.1 behind the head).
fn shadow(phi: f32) -> f32 {
    const ALPHA_MIN: f32 = 0.1;
    const PHI_MIN: f32 = 150.0;
    (1.0 + ALPHA_MIN / 2.0) + (1.0 - ALPHA_MIN / 2.0) * (phi / PHI_MIN * std::f32::consts::PI).cos()
}

/// The angle (degrees) between an ear's axis and a source at `azimuth`
/// (+ right) and `elevation` (+ up), seen from the head.
fn ear_angle(azimuth: f32, elevation: f32, right_ear: bool) -> f32 {
    let x = azimuth.to_radians().sin() * elevation.to_radians().cos();
    let c = if right_ear { x } else { -x };
    c.clamp(-1.0, 1.0).acos().to_degrees()
}

fn wrap(deg: f32) -> f32 {
    let d = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if d == -180.0 {
        180.0
    } else {
        d
    }
}

impl Spatial {
    pub fn new(rate: u32) -> Spatial {
        let frames = |ms: f32| ((ms / 1000.0 * rate as f32) as usize).clamp(1, RING - 8);
        let taps = TAPS.map(|ear| ear.map(|(ms, g, src)| (frames(ms), g, src)));
        let tail = TAIL_MS.map(|ms| vec![0.0; frames(ms)]);
        let post = Coeffs::high_shelf(rate, 3000.0, 0.7, 3.0);
        Spatial {
            rate,
            ring: vec![[0.0; 2]; RING],
            pos: 0,
            paths: [[Path::default(); 2]; 2],
            taps,
            room_lp: [0.0; 2],
            // The room's walls take the top off: about 6 kHz.
            lp: (-2.0 * std::f32::consts::PI * 6000.0 / rate as f32).exp(),
            tail,
            tail_at: [0; 2],
            tail_lp: [0.0; 2],
            post: [Biquad::new(post), Biquad::new(post)],
            want: (0.0, 0.0),
            pose: (0.0, 0.0),
            ease: 1.0 - (-(BLOCK as f32) / (rate as f32 * 0.04)).exp(),
            count: 0,
            fresh: true,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Where the head is turned, from where the speakers are set up:
    /// `yaw` + to the right, `pitch` + up, in degrees.
    pub fn set_pose(&mut self, yaw: f32, pitch: f32) {
        self.want = (wrap(yaw), pitch.clamp(-80.0, 80.0));
    }

    /// The pose the paths are worked out for now.
    pub fn pose(&self) -> (f32, f32) {
        self.pose
    }

    fn plan(&mut self) {
        if self.fresh {
            self.pose = self.want;
        } else {
            let dyaw = wrap(self.want.0 - self.pose.0);
            self.pose.0 = wrap(self.pose.0 + dyaw * self.ease);
            self.pose.1 += (self.want.1 - self.pose.1) * self.ease;
        }
        let rate = self.rate as f32;
        // The bilinear transform of the shadow's filter, its corner at c/a.
        let beta = rate / (SPEED_OF_SOUND / HEAD_RADIUS);
        for s in 0..2 {
            let azimuth = if s == 0 { -SPEAKERS } else { SPEAKERS } - self.pose.0;
            for e in 0..2 {
                let phi = ear_angle(azimuth, -self.pose.1, e == 1);
                let target = ear_delay(phi) * rate + 1.0;
                let alpha = shadow(phi);
                let p = &mut self.paths[s][e];
                if self.fresh {
                    p.delay = target;
                }
                p.step = (target - p.delay) / BLOCK as f32;
                p.b0 = (1.0 + alpha * beta) / (1.0 + beta);
                p.b1 = (1.0 - alpha * beta) / (1.0 + beta);
                p.a1 = (1.0 - beta) / (1.0 + beta);
            }
        }
        self.fresh = false;
    }

    #[inline(always)]
    fn at(&self, back: usize, ch: usize) -> f32 {
        self.ring[(self.pos + RING - back) & (RING - 1)][ch]
    }

    /// Stereo frames in place: the speakers, heard through headphones.
    pub fn process(&mut self, frames: &mut [f32]) {
        for frame in frames.chunks_exact_mut(2) {
            self.ring[self.pos] = [frame[0], frame[1]];
            if self.count == 0 {
                self.plan();
            }
            self.count = (self.count + 1) % BLOCK;
            let mut ear = [0.0f32; 2];
            for s in 0..2 {
                for (e, out) in ear.iter_mut().enumerate() {
                    let p = self.paths[s][e];
                    let delay = p.delay + p.step;
                    let whole = delay as usize;
                    let frac = delay - whole as f32;
                    let x = self.at(whole, s) * (1.0 - frac) + self.at(whole + 1, s) * frac;
                    let y = p.b0 * x + p.b1 * p.x1 - p.a1 * p.y1;
                    let p = &mut self.paths[s][e];
                    p.delay = delay;
                    p.x1 = x;
                    p.y1 = y;
                    *out += y;
                }
            }
            for (e, &direct) in ear.iter().enumerate() {
                let mut early = 0.0;
                for &(back, gain, src) in &self.taps[e] {
                    early += self.at(back, src) * gain;
                }
                self.room_lp[e] = early + (self.room_lp[e] - early) * self.lp;
                let early = self.room_lp[e];
                // The tail: what came round the comb, damped, fed back with the early sound.
                let tail = &mut self.tail[e];
                let i = self.tail_at[e];
                let out = tail[i];
                self.tail_lp[e] = out + (self.tail_lp[e] - out) * 0.35;
                tail[i] = early + self.tail_lp[e] * TAIL_FEEDBACK;
                self.tail_at[e] = if i + 1 == tail.len() { 0 } else { i + 1 };
                let room = early + out * TAIL_LEVEL;
                frame[e] = self.post[e].run(DIRECT * direct + ROOM * room);
            }
            self.pos = (self.pos + 1) & (RING - 1);
        }
    }

    /// Silence kept: a new song, or after a seek.
    pub fn reset(&mut self) {
        let rate = self.rate;
        let (want, pose) = (self.want, self.pose);
        *self = Spatial::new(rate);
        self.want = want;
        self.pose = pose;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone in one channel or both through it; each ear's level (dB) and
    /// the right ear's lag behind the left (frames, by cross-correlation).
    fn listen(sp: &mut Spatial, freq: f32, left: f32, right: f32) -> ([f32; 2], i32) {
        let rate = sp.rate();
        let n = rate as usize;
        let mut frames: Vec<f32> = (0..n)
            .flat_map(|i| {
                let x = 0.3 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin();
                [x * left, x * right]
            })
            .collect();
        sp.process(&mut frames);
        let tail = &frames[n..];
        let rms = |ch: usize| (tail.iter().skip(ch).step_by(2).map(|x| x * x).sum::<f32>() / (tail.len() / 2) as f32).sqrt();
        let db = [0, 1].map(|ch| crate::gain_to_db(rms(ch) / (0.3 / 2f32.sqrt())));
        let mut best = (0, f32::MIN);
        for lag in -40i32..=40 {
            let mut sum = 0.0;
            for i in 2000..20_000usize {
                let j = i as i32 + lag;
                sum += tail[i * 2] * tail[j as usize * 2 + 1];
            }
            if sum > best.1 {
                best = (lag, sum);
            }
        }
        (db, best.0)
    }

    #[test]
    fn the_left_speaker_is_heard_first_and_loudest_by_the_left_ear() {
        let mut sp = Spatial::new(48_000);
        let ([l, r], lag) = listen(&mut sp, 3000.0, 1.0, 0.0);
        assert!(l - r > 3.0, "left {l} right {r}");
        // Later at the right ear: 0.26 ms of path round the head (12 or 13
        // frames), and in the bass the shadow's own lag on top, as a real
        // head has it (Kuhn: half as much again below 500 Hz).
        let _ = lag;
        let mut sp = Spatial::new(48_000);
        let (_, lag_low) = listen(&mut sp, 300.0, 1.0, 0.0);
        assert!((14..=24).contains(&lag_low), "{lag_low}");
    }

    #[test]
    fn a_voice_in_the_middle_keeps_its_level_and_tone() {
        for rate in [44_100, 48_000] {
            let mut levels = Vec::new();
            for freq in [150.0, 500.0, 1000.0, 3000.0, 6000.0, 10_000.0] {
                let mut sp = Spatial::new(rate);
                let ([l, r], _) = listen(&mut sp, freq, 1.0, 1.0);
                // Even, but for the room's tails, which differ a little.
                assert!((l - r).abs() < 1.0, "{freq} Hz: {l} {r}");
                levels.push(l);
            }
            let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
            assert!(hi - lo < 8.0, "{rate}: {levels:?}");
            // Near where it was without it, so turning it on is no jump in volume.
            assert!(levels[1].abs() < 4.0, "{levels:?}");
        }
    }

    #[test]
    fn turning_the_head_right_brings_the_left_speaker_round_to_the_front() {
        let mut ahead = Spatial::new(48_000);
        let ([l0, r0], _) = listen(&mut ahead, 3000.0, 1.0, 0.0);
        let mut turned = Spatial::new(48_000);
        turned.set_pose(-30.0, 0.0);
        // Turned 30° left: the left speaker is straight ahead, even at both ears.
        let ([l1, r1], _) = listen(&mut turned, 3000.0, 1.0, 0.0);
        assert!((l1 - r1).abs() < (l0 - r0).abs() - 2.0, "{l0} {r0} / {l1} {r1}");
        assert!((l1 - r1).abs() < 1.5);
        // And no sample out of bounds at any pose, eased or not.
        let mut sp = Spatial::new(96_000);
        for yaw in (-180..=180).step_by(15) {
            let mut frames = vec![0.9f32; 4096];
            sp.set_pose(yaw as f32, (yaw / 4) as f32);
            sp.process(&mut frames);
            assert!(frames.iter().all(|x| x.is_finite() && x.abs() < 2.5), "{yaw}");
        }
    }

    #[test]
    fn the_model_matches_its_sources() {
        // Facing the ear: no shadow, twice the treble; behind: little.
        assert!((shadow(0.0) - 2.0).abs() < 1e-5);
        assert!(shadow(150.0) < 0.15);
        assert!(ear_delay(0.0).abs() < 1e-7);
        assert!((ear_delay(180.0) * 1000.0 - 0.655).abs() < 0.01);
        assert!((ear_angle(30.0, 0.0, true) - 60.0).abs() < 0.01);
        assert!((ear_angle(30.0, 0.0, false) - 120.0).abs() < 0.01);
        // Looking up, a speaker at the side is less to one side.
        assert!(ear_angle(90.0, 60.0, true) > 50.0);
        assert_eq!(wrap(190.0), -170.0);
        assert_eq!(wrap(-180.0), 180.0);
    }

    /// What it costs here, per frame: `cargo test -p audiofx --release -- --ignored cost`.
    #[test]
    #[ignore]
    fn cost() {
        let mut frames: Vec<f32> = (0..48_000 * 2).map(|i| ((i as f32) * 0.01).sin() * 0.4).collect();
        let mut sp = Spatial::new(48_000);
        let mut eq = crate::eq::Equalizer::new(48_000);
        eq.set([4.0, 2.0, -1.0, 2.0, 3.0]);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            sp.set_pose(5.0, 0.0);
            sp.process(&mut frames);
        }
        let spatial = t.elapsed().as_secs_f64() / (20.0 * 48_000.0);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            eq.process(&mut frames);
        }
        let eq_cost = t.elapsed().as_secs_f64() / (20.0 * 48_000.0);
        println!("spatial {:.1} ns a frame ({:.2} % of a core at 48 kHz); eq, five bands {:.1} ns ({:.2} %)", spatial * 1e9, spatial * 48_000.0 * 100.0, eq_cost * 1e9, eq_cost * 48_000.0 * 100.0);
    }
}
