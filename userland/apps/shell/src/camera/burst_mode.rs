//! Burst (1.5.1): a fast shutter, about ten frames, and the sharpest of
//! them kept: for a robot crossing the field in a match. (Detail mode,
//! burst.rs, is another thing: a burst merged into one photo for less
//! noise in dim light. On the screen it is "detail", and this is "burst".)
//!
//! - **The exposure** ([`plan`]): the shutter chosen (`s`, or its chip:
//!   1/1000, 1/500 or 1/250 s; 1/500 to begin with, which holds a robot
//!   crossing the field at full speed to a few pixels), the gain making up
//!   the rest up to 8 (ISO 800: past that the noise costs more than the
//!   blur saves); in less light than that the shutter lengthens, to 1/60 s
//!   at the most, then the gain goes on to its 10; in more light than the
//!   least gain needs, the shutter shortens. Its chip says the shutter it
//!   will take, and when the light has made it longer than asked.
//! - **The frames**: ten at the sensor's full size, each scored for
//!   sharpness by catalyst-camera as it comes (compose.h: the brightness's
//!   gradient energy over the noise floor; blur from a hand or from the
//!   robot moving takes the fine steps first) and only the sharpest so far
//!   kept.
//! - **While it runs** (about a second and a half): the countdown ring
//!   counts the frames. Esc stops it there, keeping the sharpest so far.
//!   The saved photo says which frame it was: "burst · frame 4 of 10, the
//!   sharpest".
//! - **Memory**: two full-size frames in catalyst-camera (24 MB), the
//!   sharpest and the next; binned (6 MB) when the system has not that to
//!   spare. Nothing in the shell.
//!
//! On a computer the simulated camera takes the burst of its own scene,
//! each frame blurred its own amount (a hand, and the robot driving), and
//! keeps the sharpest by the same score ([`sharpness`]).

use super::*;

/// Frames a burst takes (catalyst-camera's compose.h allows 16).
pub const FRAMES: u32 = 10;
/// The shutters offered (1/N s), and the first.
pub const SHUTTERS: [u32; 3] = [1000, 500, 250];
pub const SHUTTER_DEFAULT: u32 = 500;
/// The most gain before the shutter lengthens instead (ISO 800).
pub const GAIN_MOST: f32 = 8.0;
/// The longest the shutter goes in low light: 1/60 s.
pub const LONGEST_US: u32 = 16_667;

/// The burst's exposure for the light (`needed_us`, µs at a gain of 1) and
/// the shutter asked for (1/`shutter` s): (µs, gain).
pub fn plan(needed_us: f32, shutter: u32) -> (u32, f32) {
    let want = 1e6 / shutter.max(1) as f32;
    let needed = needed_us.max(100.0);
    if needed <= want {
        return ((needed.round() as u32).max(100), 1.0);
    }
    if needed <= want * GAIN_MOST {
        return (want.round() as u32, needed / want);
    }
    let e = (needed / GAIN_MOST).min(LONGEST_US as f32);
    (e.round() as u32, (needed / e).clamp(1.0, pro::MAX_GAIN))
}

/// What a burst saved says: "burst · frame 4 of 10, the sharpest".
pub fn saved_words(at: u32, of: u32) -> String {
    if of <= 1 {
        "burst · one frame (memory was short)".to_owned()
    } else {
        format!("burst · frame {at} of {of}, the sharpest")
    }
}

/// A frame's sharpness, as catalyst-camera's compose.h scores it (the
/// simulator's, and checked against it): the mean of each sampled pixel's
/// squared steps right and down, less the noise floor's, never below none.
pub fn sharpness(y: &[u8], w: usize, h: usize, floor: i32) -> f64 {
    if w < 2 || h < 2 {
        return 0.0;
    }
    let less = 2 * i64::from(floor) * i64::from(floor);
    let (mut sum, mut n) = (0i64, 0i64);
    for py in (0..h - 1).step_by(2) {
        let (row, below) = (&y[py * w..(py + 1) * w], &y[(py + 1) * w..(py + 2) * w]);
        for px in (0..w - 1).step_by(2) {
            let dx = i64::from(row[px + 1]) - i64::from(row[px]);
            let dy = i64::from(below[px]) - i64::from(row[px]);
            let g = dx * dx + dy * dy - less;
            if g > 0 {
                sum += g;
            }
            n += 1;
        }
    }
    if n > 0 {
        sum as f64 / n as f64
    } else {
        0.0
    }
}

/// The noise floor at a gain (compose.h's noise_floor): two and a half of
/// the noise's deviations.
pub fn noise_floor(gain: f32) -> i32 {
    ((2.5 * (1.5 + 0.8 * gain.clamp(1.0, 16.0))).round() as i32).clamp(2, 30)
}

/// The sharpest frame so far (compose.h's Best): a frame offered is kept
/// only when sharper by more than a hair; a tie keeps the earlier.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Best {
    pub at: u32,
    pub score: f64,
}

impl Best {
    pub fn offer(&mut self, frame: u32, score: f64) -> bool {
        if self.at != 0 && score <= self.score * 1.002 {
            return false;
        }
        (self.at, self.score) = (frame, score);
        true
    }
}

impl Camera {
    /// A burst's exposure as it would be taken now.
    pub(super) fn burst_plan(&self) -> Option<(u32, f32)> {
        let m = self.metered.as_ref()?;
        let auto = self.told.is_some_and(|c| c.exposure_us.is_none() && c.gain.is_none());
        Some(plan(pro::needed_us(m, self.lux_scale.k, auto), self.burst_shutter))
    }

    /// A burst's words for catalyst-camera: the camera's white balance, the
    /// plan's exposure and its processing, and how many frames.
    pub(super) fn burst_words(&self) -> Vec<String> {
        let Some(m) = self.metered else { return Vec::new() };
        let (us, gain) = self.burst_plan().unwrap_or((2000, 4.0));
        let mut words = pro::still(None, &m, false).words();
        words.retain(|w| !w.starts_with("exp=") && !w.starts_with("gain=") && !w.starts_with("sharp=") && !w.starts_with("nr="));
        let (sharp, nr) = pro::processing(gain);
        words.extend([format!("exp={us}"), format!("gain={gain:.3}"), format!("sharp={sharp:.2}"), format!("nr={}", nr.word()), format!("best={FRAMES}")]);
        words
    }

    /// Its plan for the countdown ring: ten frames, each a frame's length.
    pub(super) fn burst_ring(&self) -> pro::NightPlan {
        let (us, gain) = self.burst_plan().unwrap_or((2000, 4.0));
        pro::NightPlan { frames: FRAMES, frame_us: us, gain, lift: 0.0 }
    }

    /// The shutter's chip and the words on the picture: "1/500 · iso 640",
    /// or "1/250 in this light" where the light made it longer than asked.
    pub(super) fn burst_words_now(&self) -> String {
        let asked = format!("1/{}", self.burst_shutter);
        match self.burst_plan() {
            Some((us, _)) if us as f32 > 1.1e6 / self.burst_shutter as f32 => format!("{} in this light", pro::shutter_words(us)),
            Some((us, gain)) => format!("{} · {}", pro::shutter_words(us), pro::iso_words((gain * 100.0).round() as u32).to_lowercase()),
            None => asked,
        }
    }

    /// The burst's shutter a step along its choices (`s`), kept.
    pub(super) fn step_burst_shutter(&mut self) {
        let at = SHUTTERS.iter().position(|s| *s == self.burst_shutter).unwrap_or(1);
        self.set_burst_shutter(SHUTTERS[(at + 1) % SHUTTERS.len()]);
    }

    pub(super) fn set_burst_shutter(&mut self, shutter: u32) {
        self.burst_shutter = shutter;
        self.keep_prefs();
        self.say(format!("burst: 1/{shutter} s · {}", self.burst_words_now()));
    }
}

/// The simulator's burst, as catalyst-camera takes one: its frames of the
/// scene (binned, as the handheld takes one when memory is short), each
/// blurred its own amount (the hand, and the robot driving across), told
/// as they come, each scored and only the sharpest kept; Esc (a stack's
/// end) stops it, keeping that; then written behind the preview.
pub(super) fn sim_best(live: &Arc<Mutex<Live>>, orders: &Receiver<Order>, path: PathBuf, t: f32, words: &[String], later: &mut Vec<Order>) {
    let asked = Instant::now();
    let word = |k: &str| still_word(words, k).and_then(|v| v.parse::<f32>().ok());
    let planned = word("best").unwrap_or(FRAMES as f32).clamp(2.0, 16.0) as u32;
    let gain = word("gain").unwrap_or(4.0);
    let us = word("exp").unwrap_or(2000.0);
    let (w, h) = (FULL.0 / 2, FULL.1 / 2);
    let frame_ms = 2.0 * (us / 1000.0).max(66.7);
    let tell = |e: Event| {
        live.lock().map(|mut l| l.events.push(e)).ok();
        crate::platform::wake();
    };
    tell(Event::Exposing(format!("{:.0} {planned} burst", frame_ms * planned as f32)));
    let ct = still_word(words, "ct").and_then(|v| v.parse().ok()).unwrap_or(SIM_CT);
    let base = {
        let mut pic = scene(w, h, t, false);
        sim_expose(&mut pic, us * gain / sim_needs(0.0), ct);
        rgb_to_yuv420(&pic)
    };
    // Each frame's blur: the hand's and the robot's (my own sample of a
    // match: most frames a little soft, one or two crisp).
    const BLUR: [usize; 10] = [5, 3, 6, 2, 4, 1, 4, 3, 6, 2];
    let floor = noise_floor(gain);
    let mut best = Best::default();
    let mut kept: Option<Vec<u8>> = None;
    let mut seed = 0xA076_1D64_78BD_642Fu64 ^ u64::from(t.to_bits());
    let mut ended = false;
    for o in std::mem::take(later) {
        match o {
            Order::StackEnd | Order::StackCancel => ended = true,
            other => later.push(other),
        }
    }
    let mut taken = 0;
    for i in 0..planned {
        for o in orders.try_iter() {
            match o {
                Order::StackEnd | Order::StackCancel => ended = true,
                other => later.push(other),
            }
        }
        if ended && taken > 0 {
            break;
        }
        std::thread::sleep(Duration::from_secs_f32((frame_ms / 1000.0).min(0.15)));
        let mut yuv = smear(&base, w, h, BLUR[i as usize % BLUR.len()]);
        sim_noise(&mut yuv, w * h, 1.5 + 0.8 * gain, &mut seed);
        taken = i + 1;
        if best.offer(taken, sharpness(&yuv[..w * h], w, h, floor)) {
            kept = Some(yuv);
        }
        let left = (planned - taken) as f32 * frame_ms;
        tell(Event::Stack(format!("{taken} {planned} {} {left:.0}", best.at)));
    }
    tell(Event::Exposed);
    let Some(yuv) = kept else {
        tell(Event::Failed("the burst's frames did not come".into()));
        return;
    };
    let shot = asked.elapsed().as_millis();
    tell(Event::Captured(format!("room 0 stop 0 config 0 start 0 frame {shot} frames {shot} captured {shot}")));
    let live = live.clone();
    std::thread::Builder::new()
        .name("photo-write".into())
        .spawn(move || {
            let event = match crate::burst::write_jpeg(&path, &yuv, w, h, None) {
                Ok(()) => Event::Saved { path, size: (w as u32, h as u32), detail: false, raw_skipped: false, stacked: Some((camkit::Exposing::Burst, best.at, taken)) },
                Err(why) => Event::Failed(why),
            };
            live.lock().map(|mut l| l.events.push(event)).ok();
            crate::platform::wake();
        })
        .ok();
}

/// A frame blurred along its rows by a box `k` wide (a pan, a robot
/// driving), its colour as it was.
fn smear(yuv: &[u8], w: usize, h: usize, k: usize) -> Vec<u8> {
    let mut out = yuv.to_vec();
    if k <= 1 {
        return out;
    }
    for y in 0..h {
        let row = &yuv[y * w..(y + 1) * w];
        let mut sum: u32 = row[..k.min(w)].iter().map(|v| u32::from(*v)).sum();
        for x in 0..w {
            out[y * w + x] = (sum / k as u32) as u8;
            if x + k < w {
                sum = sum + u32::from(row[x + k]) - u32::from(row[x]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shutter_holds_until_the_gain_runs_out_then_lengthens() {
        // Bright: shorter than asked, at the least gain.
        let (us, gain) = plan(800.0, 500);
        assert_eq!((us, gain), (800, 1.0));
        // Within the gain: the shutter asked, the gain making up the rest.
        let (us, gain) = plan(10_000.0, 500);
        assert_eq!(us, 2000);
        assert!((gain - 5.0).abs() < 0.01);
        // Past ISO 800: the shutter lengthens, to 1/60 at the most, then the gain goes on.
        let (us, gain) = plan(40_000.0, 500);
        assert_eq!((us, gain), (5000, 8.0));
        let (us, gain) = plan(400_000.0, 500);
        assert_eq!(us, LONGEST_US);
        assert!((gain - pro::MAX_GAIN).abs() < 0.01, "{gain}");
        // A faster shutter asks more of the gain.
        assert!(plan(10_000.0, 1000).1 > plan(10_000.0, 250).1);
        assert!(SHUTTERS.contains(&SHUTTER_DEFAULT));
    }

    #[test]
    fn the_sharpest_frame_is_kept_and_noise_alone_scores_little() {
        // A scene with detail, blurred more and less.
        let (w, h) = (320, 240);
        let pic = scene(w, h, 3.1, false);
        let yuv = rgb_to_yuv420(&pic);
        let mut seed = 7u64;
        let floor = noise_floor(4.0);
        let mut scores = Vec::new();
        for k in [1, 2, 3, 5, 8] {
            let mut f = smear(&yuv, w, h, k);
            sim_noise(&mut f, w * h, 1.5 + 0.8 * 4.0, &mut seed);
            scores.push(sharpness(&f[..w * h], w, h, floor));
        }
        assert!(scores.windows(2).all(|p| p[0] > p[1]), "falls as the blur grows: {scores:?}");
        // Noise on grey: little.
        let mut flat = vec![110u8; w * h];
        sim_noise(&mut flat, w * h, 1.5 + 0.8 * 4.0, &mut seed);
        assert!(sharpness(&flat, w, h, floor) < scores[0] * 0.03);
        assert_eq!(sharpness(&flat, 1, 1, floor), 0.0);
        // Of a match's ten frames, the crisp one.
        let mut best = Best::default();
        for (i, k) in [5usize, 3, 6, 2, 4, 1, 4, 3, 6, 2].iter().enumerate() {
            let mut f = smear(&yuv, w, h, *k);
            sim_noise(&mut f, w * h, 4.7, &mut seed);
            best.offer(i as u32 + 1, sharpness(&f[..w * h], w, h, floor));
        }
        assert_eq!(best.at, 6);
        // A tie keeps the earlier.
        let mut tie = Best::default();
        assert!(tie.offer(1, 10.0) && !tie.offer(2, 10.0) && tie.offer(3, 11.0));
        assert_eq!(tie.at, 3);
        assert!(noise_floor(1.0) < noise_floor(8.0));
    }

    #[test]
    fn its_words_say_the_frame_kept() {
        assert_eq!(saved_words(4, 10), "burst · frame 4 of 10, the sharpest");
        assert_eq!(camkit::read_stacked("shot 900 write 300 best 4 10"), Some((camkit::Exposing::Burst, 4, 10)));
        assert_eq!(camkit::read_exposing("1400 10 burst").0, camkit::Exposing::Burst);
    }
}
