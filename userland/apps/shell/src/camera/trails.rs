//! Light trails (1.5.0): frames one after another, the brightest each
//! pixel has been kept, so a light that moves draws a line: a robot's LEDs
//! round the field, cars' lamps, a sparkler, light painted in the air.
//!
//! - **The frames**: catalyst-camera's own (compose.h): the binned 1640 ×
//!   1232, four frames in flight so none is let go (a light draws a line,
//!   not dashes), each laid over the trails where it is brighter by more
//!   than the noise, so the dark stays as dark as one frame. Each is as
//!   long as a frame at 15 a second allows (1/15 s: the sensor is never
//!   idle between them), at the gain that puts the scene a stop under its
//!   mid-grey: the lights stand out, the rest stays night ([`plan`]).
//! - **Live, as it is made** (Olympus's Live Composite): the trails so far
//!   are the viewfinder's picture, made small keeping their brightest, so
//!   a line a pixel wide is still a line. A meter along the top fills with
//!   the length; a chip says the seconds and the frames.
//! - **Its length** (`s`, or its chip: 10 s, 20 s, 30 s, 1 or 2 min, 30 s
//!   to begin with, kept): Enter ends it sooner, and so does Esc; either
//!   way the trails so far are kept (there is nothing to lose by stopping).
//! - **Memory**: the trails and the frame just copied out, 3 MB each in
//!   catalyst-camera; nothing in the shell.
//!
//! On a computer the simulated camera takes its own street at night with a
//! robot's lights going round, laid over each other by the same rule
//! ([`lighten`], checked against compose.h's in the tests).

use super::*;

/// The lengths offered (s), and the first.
pub const LENGTHS: [u32; 5] = [10, 20, 30, 60, 120];
pub const LENGTH_DEFAULT: u32 = 30;
/// Each frame: as long as a frame at 15 a second (the sensor never idle).
pub const FRAME_US: u32 = 66_666;
/// The scene a stop under its mid-grey: the lights stand out (my own
/// tuning, as a phone's light-trail mode leaves the night dark).
pub const LOOK: f32 = 0.5;
/// The size the simulator takes them at (a quarter of the handheld's
/// binned frame each way: its frames come quickly enough to keep up).
const SIM: (usize, usize) = (820, 616);

/// Light trails' frames for the light (`needed_us`, µs at a gain of 1):
/// (µs, gain). A frame's length at the gain that brings the look; in more
/// light than the least gain needs at that length, shorter.
pub fn plan(needed_us: f32) -> (u32, f32) {
    let target = needed_us * LOOK;
    let frame = FRAME_US as f32;
    if target < frame {
        return ((target.round() as u32).max(100), 1.0);
    }
    (FRAME_US, (target / frame).clamp(1.0, pro::MAX_GAIN))
}

/// "30 s", "1 min", "2 min".
pub fn length_words(s: u32) -> String {
    camkit::duration_words(s)
}

/// What light trails saved say: "trails · 186 frames over 12.4 s".
pub fn saved_words(frames: u32, ms: u32) -> String {
    format!("trails · {frames} frames over {:.1} s", ms as f32 / 1000.0)
}

/// The noise's deviation at a gain (compose.h's noise_at).
fn noise_at(gain: f32) -> f32 {
    1.5 + 0.8 * gain.clamp(1.0, 16.0)
}

/// How much brighter a two by two's mean must be to be lit (compose.h's).
pub fn lighten_margin(gain: f32) -> i32 {
    ((3.0 * noise_at(gain)).round() as i32).clamp(4, 30)
}

/// `frame` laid over `into` where brighter by more than the noise, as
/// catalyst-camera's compose.h does it (both packed YUV 4:2:0): a two by
/// two is lit where its mean rises by `margin` or a pixel by twice that;
/// there each pixel keeps the brighter, and the colour is the new one's
/// where its brightest is brighter than any the four had. How many pixels
/// it brightened.
pub fn lighten(into: &mut [u8], frame: &[u8], w: usize, h: usize, margin: i32) -> usize {
    let (cw, ch) = (w / 2, h / 2);
    let (y, uv) = into.split_at_mut(w * h);
    let (u, v) = uv.split_at_mut(cw * ch);
    let (fy, fuv) = frame.split_at(w * h);
    let (fu, fv) = fuv.split_at(cw * ch);
    let mut n = 0;
    for cy in 0..ch {
        for cx in 0..cw {
            let a = 2 * cy * w + 2 * cx;
            let at = [a, a + 1, a + w, a + w + 1];
            let (mut was_sum, mut now_sum, mut was_max, mut now_max, mut rise) = (0i32, 0i32, 0i32, 0i32, -255i32);
            for &i in &at {
                was_sum += i32::from(y[i]);
                now_sum += i32::from(fy[i]);
                was_max = was_max.max(i32::from(y[i]));
                now_max = now_max.max(i32::from(fy[i]));
                rise = rise.max(i32::from(fy[i]) - i32::from(y[i]));
            }
            if rise <= 0 || (now_sum <= was_sum + 4 * margin && rise <= 2 * margin) {
                continue;
            }
            for &i in &at {
                if fy[i] > y[i] {
                    y[i] = fy[i];
                    n += 1;
                }
            }
            if now_max > was_max {
                let c = cy * cw + cx;
                u[c] = fu[c];
                v[c] = fv[c];
            }
        }
    }
    n
}

/// A small copy of trails (compose.h's small_brightest): each pixel the
/// brightest of its box, each colour from where the brightest of its box was.
pub fn small_brightest(yuv: &[u8], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0u8; ow * oh * 3 / 2];
    let (y, u, v) = (&yuv[..w * h], &yuv[w * h..w * h + (w / 2) * (h / 2)], &yuv[w * h + (w / 2) * (h / 2)..]);
    let span = |o: usize, n: usize, big: usize| {
        let a = o * big / n;
        (a, ((o + 1) * big / n).min(big).max(a + 1))
    };
    let brightest = |x0: usize, x1: usize, y0: usize, y1: usize| {
        let step = if x1 - x0 > 2 || y1 - y0 > 2 { 2 } else { 1 };
        let mut best = (0u8, x0, y0);
        for yy in (y0..y1).step_by(step) {
            for xx in (x0..x1).step_by(step) {
                if y[yy * w + xx] > best.0 || (yy, xx) == (y0, x0) {
                    best = (y[yy * w + xx], xx, yy);
                }
            }
        }
        best
    };
    let (oy, ouv) = out.split_at_mut(ow * oh);
    let (ou, ov) = ouv.split_at_mut((ow / 2) * (oh / 2));
    for py in 0..oh {
        let (y0, y1) = span(py, oh, h);
        for px in 0..ow {
            let (x0, x1) = span(px, ow, w);
            oy[py * ow + px] = brightest(x0, x1, y0, y1).0;
        }
    }
    for py in 0..oh / 2 {
        let (y0, y1) = (span(2 * py, oh, h).0, span(2 * py + 1, oh, h).1);
        for px in 0..ow / 2 {
            let (x0, x1) = (span(2 * px, ow, w).0, span(2 * px + 1, ow, w).1);
            let (_, bx, by) = brightest(x0, x1, y0, y1);
            let c = (by / 2).min(h / 2 - 1) * (w / 2) + (bx / 2).min(w / 2 - 1);
            ou[py * (ow / 2) + px] = u[c];
            ov[py * (ow / 2) + px] = v[c];
        }
    }
    out
}

/// The simulated scene's lights at `t` (s): a robot's red bumper LEDs
/// driving across the field, a blue signal light going round, a cart's
/// warm lamps coming the other way; each a soft disc with its glow.
pub fn sim_lights(pic: &mut Picture, t: f32) {
    // Over a frame's exposure (1/15 s), as the sensor sees them: a light
    // moves while it is exposed, so each frame holds a short line of it.
    for k in 0..8 {
        lights_at(pic, t + k as f32 / 8.0 * FRAME_US as f32 / 1e6);
    }
}

fn lights_at(pic: &mut Picture, t: f32) {
    let (w, h) = (pic.w as f32, pic.h as f32);
    let u = (t * 0.22).fract();
    let lamps = [
        (0.12 + 0.76 * u, 0.66 + 0.07 * (u * std::f32::consts::TAU).sin(), [255.0, 52.0, 36.0]),
        (0.14 + 0.76 * u, 0.66 + 0.07 * (u * std::f32::consts::TAU).sin(), [255.0, 52.0, 36.0]),
        (0.52 + 0.2 * (t * 1.3).cos(), 0.36 + 0.14 * (t * 1.3).sin(), [70.0, 150.0, 255.0]),
        (0.92 - 0.8 * ((t * 0.17 + 0.4).fract()), 0.80, [255.0, 214.0, 140.0]),
    ];
    let r = (w * 0.006).max(1.6);
    for (fx, fy, colour) in lamps {
        let (cx, cy) = (fx * w, fy * h);
        let reach = r * 3.0;
        let (x0, x1) = ((cx - reach).floor().max(0.0) as usize, ((cx + reach).ceil() as usize).min(pic.w));
        let (y0, y1) = ((cy - reach).floor().max(0.0) as usize, ((cy + reach).ceil() as usize).min(pic.h));
        for py in y0..y1 {
            for px in x0..x1 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt() / r;
                // A white-hot core, its colour round it, a glow fading out.
                let glow = if d < 1.0 { 1.0 } else { (1.0 - (d - 1.0) / 2.0).clamp(0.0, 1.0).powi(2) * 0.55 };
                if glow <= 0.0 {
                    continue;
                }
                let core = (1.0 - d).clamp(0.0, 1.0);
                let at = (py * pic.w + px) * 3;
                for c in 0..3 {
                    let lit = colour[c] + (255.0 - colour[c]) * core * 0.85;
                    let was = f32::from(pic.rgb[at + c]);
                    pic.rgb[at + c] = (was + (lit - was) * glow).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

impl Camera {
    /// Light trails' exposure as it would be taken now.
    pub(super) fn trails_plan(&self) -> Option<(u32, f32)> {
        let m = self.metered.as_ref()?;
        let auto = self.told.is_some_and(|c| c.exposure_us.is_none() && c.gain.is_none());
        Some(plan(pro::needed_us(m, self.lux_scale.k, auto)))
    }

    /// Its words for catalyst-camera: the camera's white balance, the
    /// plan's exposure, and how long at the most.
    pub(super) fn trails_words(&self) -> Vec<String> {
        let Some(m) = self.metered else { return Vec::new() };
        let (us, gain) = self.trails_plan().unwrap_or((FRAME_US, 4.0));
        let mut words = pro::still(None, &m, false).words();
        words.retain(|w| !w.starts_with("exp=") && !w.starts_with("gain=") && !w.starts_with("sharp=") && !w.starts_with("nr="));
        let (sharp, nr) = pro::processing(gain);
        words.extend([format!("exp={us}"), format!("gain={gain:.3}"), format!("sharp={sharp:.2}"), format!("nr={}", nr.word()), format!("trails={}", self.trails_s)]);
        words
    }

    /// Its plan for the countdown's clock: as many frames as its length
    /// holds, each a frame's length.
    pub(super) fn trails_ring(&self) -> pro::NightPlan {
        let (us, gain) = self.trails_plan().unwrap_or((FRAME_US, 4.0));
        let each = us.max(33_333) + 1000;
        let frames = (self.trails_s * 1_000_000 / each).max(1);
        pro::NightPlan { frames, frame_us: each, gain, lift: 0.0 }
    }

    /// Light trails being taken.
    pub(super) fn trails_running(&self) -> bool {
        self.stacking.as_ref().is_some_and(|s| s.live && s.what == camkit::Exposing::Trails)
    }

    /// Its length a step along its choices (`s`), kept.
    pub(super) fn step_trails_length(&mut self) {
        let at = LENGTHS.iter().position(|s| *s == self.trails_s).unwrap_or(2);
        self.set_trails_length(LENGTHS[(at + 1) % LENGTHS.len()]);
    }

    pub(super) fn set_trails_length(&mut self, s: u32) {
        self.trails_s = s;
        self.keep_prefs();
        self.say(format!("trails: up to {} · enter ends it sooner", length_words(s)));
    }

    /// Its chip's words: "up to 30 s · 1/15 at iso 400".
    pub(super) fn trails_words_now(&self) -> String {
        let length = format!("up to {}", length_words(self.trails_s));
        match self.trails_plan() {
            Some((us, gain)) => format!("{length} · {} at {}", pro::shutter_words(us), pro::iso_words((gain * 100.0).round() as u32).to_lowercase()),
            None => length,
        }
    }

    /// While light trails are taken: a meter along the top of the picture
    /// filling with its length (on the countdown ring's spring), and a chip
    /// at its start with the seconds and frames (its dot breathing, the
    /// signal colour: it is being taken); at the foot, how to finish.
    pub(super) fn draw_trails(&mut self, g: &mut Gfx, vf: Rect, top: f32) {
        let Some(s) = &self.stacking else { return };
        if s.what != camkit::Exposing::Trails {
            return;
        }
        let a = s.faded().clamp(0.0, 1.0);
        if a < 0.01 {
            return;
        }
        let (t, cap) = (g.tokens, g.ty.caption);
        let white = Color(255, 255, 255);
        let now = Instant::now();
        // The meter: its track, and its fill.
        let m = 6.0;
        let track = Rect::new(vf.x + m, vf.y + 3.0, vf.w - 2.0 * m, 3.0);
        g.canvas.rrect(track.offset(g.ox, 0.0), 1.5, white, 0.22 * a);
        let fill = s.fill().clamp(0.0, 1.0);
        if fill > 0.001 {
            g.canvas.rrect(Rect::new(track.x + g.ox, track.y, (track.w * fill).max(3.0), track.h), 1.5, t.signal, a);
        }
        // The chip: a breathing dot, the seconds and the frames, as many
        // words as fit (its width held by the widest seconds it could show).
        let elapsed = s.elapsed(now);
        let frames = s.taken();
        let ended = s.ended;
        let secs = format!("{elapsed:.1} s");
        let forms = if ended {
            vec!["finishing the trails…".to_owned(), "finishing…".to_owned()]
        } else {
            vec![format!("trails · {secs} · {frames} frames"), format!("{secs} · {frames} frames"), secs.clone()]
        };
        let room = vf.w - 2.0 * m - 20.0;
        let words = forms.iter().find(|f| g.measure(f, cap) <= room).unwrap_or(&forms[forms.len() - 1]).clone();
        let h = cap.size + 8.0;
        let dot = 6.0;
        let w = (g.measure(&words, cap) + 16.0 + dot + 6.0).round();
        let y = (vf.y + 11.0).max(top);
        g.canvas.rrect(Rect::new(vf.x + m + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.55 * a);
        let breathe = if g.calm { 1.0 } else { 0.65 + 0.35 * (elapsed * std::f32::consts::PI).cos().abs() };
        g.canvas.circle(vf.x + m + 8.0 + dot * 0.5 + g.ox, y + h * 0.5, dot * 0.5, t.signal, a * breathe);
        g.text_alpha(vf.x + m + 8.0 + dot + 6.0, y + h * 0.5, &words, cap, white, a);
        // How to finish, at the foot.
        if !ended {
            let hints = ["enter or esc: finish and keep", "enter: finish", "finish: enter"];
            let hint = hints.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 2.0 * m).unwrap_or(&hints[2]);
            let hw = g.measure(hint, cap) + 16.0;
            let hy = vf.bottom() - m - h;
            g.canvas.rrect(Rect::new(vf.cx() - hw * 0.5 + g.ox, hy, hw, h), h * 0.5, Color(0, 0, 0), 0.55 * a);
            g.text_alpha(vf.cx() - hw * 0.5 + 8.0, hy + h * 0.5, hint, cap, white, a);
        }
    }
}

/// The simulator's light trails, as catalyst-camera takes them: frames of
/// its street with the lights moving (at a quarter of the binned size, to
/// keep up), each laid over the trails as compose.h lays them, the trails
/// so far the viewfinder's picture every other frame; until its length, or
/// a stack's end (Enter, Esc); then written behind the preview.
pub(super) fn sim_trails(live: &Arc<Mutex<Live>>, develop: &Mutex<Develop>, orders: &Receiver<Order>, path: PathBuf, t: f32, words: &[String], later: &mut Vec<Order>) {
    let asked = Instant::now();
    let word = |k: &str| still_word(words, k).and_then(|v| v.parse::<f32>().ok());
    let length = word("trails").unwrap_or(LENGTH_DEFAULT as f32).clamp(1.0, 120.0);
    let gain = word("gain").unwrap_or(4.0);
    let us = word("exp").unwrap_or(FRAME_US as f32);
    let ct = still_word(words, "ct").and_then(|v| v.parse().ok()).unwrap_or(SIM_CT);
    let (w, h) = SIM;
    let each = (us.max(33_333.0) + 1000.0) / 1e6;
    let planned = ((length / each) as u32).max(1);
    let tell = |e: Event| {
        live.lock().map(|mut l| l.events.push(e)).ok();
        crate::platform::wake();
    };
    tell(Event::Exposing(format!("{:.0} {planned} trails", length * 1000.0 + 2.0 * each * 1000.0)));
    let base = {
        let mut pic = scene(w, h, t, false);
        sim_expose(&mut pic, us * gain / sim_needs(0.0), ct);
        pic
    };
    let margin = lighten_margin(gain);
    let sigma = noise_at(gain);
    let mut seed = 0x5851_F42D_4C95_7F2Du64 ^ u64::from(t.to_bits());
    let frame_at = |i: u32, seed: &mut u64| {
        let mut pic = base.clone();
        sim_lights(&mut pic, t + i as f32 * each);
        let mut yuv = rgb_to_yuv420(&pic);
        sim_noise(&mut yuv, w * h, sigma, seed);
        yuv
    };
    let mut trails = frame_at(0, &mut seed);
    let show = |trails: &[u8]| {
        let small = small_brightest(trails, w, h, PREVIEW_W, PREVIEW_H);
        develop_frame(live, develop, &small, PREVIEW_W, PREVIEW_H, &mut Picture::default(), None);
        crate::platform::wake();
    };
    show(&trails);
    let began = Instant::now();
    let (mut taken, mut ended, mut cancelled) = (1u32, false, false);
    let take_orders = |o: Order, ended: &mut bool, cancelled: &mut bool, later: &mut Vec<Order>| match o {
        Order::StackEnd => *ended = true,
        Order::StackCancel => *cancelled = true,
        other => later.push(other),
    };
    for o in std::mem::take(later) {
        take_orders(o, &mut ended, &mut cancelled, later);
    }
    let left = |began: Instant| (length * 1000.0 - began.elapsed().as_secs_f32() * 1000.0).max(0.0);
    tell(Event::Stack(format!("1 {planned} 1 {:.0}", left(began))));
    while !ended && !cancelled && began.elapsed().as_secs_f32() < length {
        for o in orders.try_iter() {
            take_orders(o, &mut ended, &mut cancelled, later);
        }
        let due = began + Duration::from_secs_f32(taken as f32 * each);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait.min(Duration::from_millis(70)));
        }
        let frame = frame_at(taken, &mut seed);
        lighten(&mut trails, &frame, w, h, margin);
        taken += 1;
        if taken % 2 == 0 {
            show(&trails);
        }
        tell(Event::Stack(format!("{taken} {} {taken} {:.0}", planned.max(taken), left(began))));
    }
    tell(Event::Exposed);
    if cancelled {
        tell(Event::Failed("cancelled".into()));
        return;
    }
    let ms = began.elapsed().as_millis() as u32;
    let shot = asked.elapsed().as_millis();
    tell(Event::Captured(format!("room 0 stop 0 config 0 start 0 frame {shot} frames {shot} captured {shot}")));
    let live = live.clone();
    std::thread::Builder::new()
        .name("photo-write".into())
        .spawn(move || {
            let event = match crate::burst::write_jpeg(&path, &trails, w, h, None) {
                Ok(()) => Event::Saved { path, size: (w as u32, h as u32), detail: false, raw_skipped: false, stacked: Some((camkit::Exposing::Trails, taken, ms)) },
                Err(why) => Event::Failed(why),
            };
            live.lock().map(|mut l| l.events.push(event)).ok();
            crate::platform::wake();
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark street, noisy, with a light at (lx, ly) if asked (red).
    fn night(w: usize, h: usize, sigma: f32, seed: &mut u64, light: Option<(usize, usize)>) -> Vec<u8> {
        let mut yuv = vec![128u8; w * h * 3 / 2];
        for y in 0..h {
            for x in 0..w {
                yuv[y * w + x] = (20 + ((x / 16 + y / 12) % 3 * 6)) as u8;
            }
        }
        sim_noise(&mut yuv, w * h, sigma, seed);
        if let Some((lx, ly)) = light {
            for y in ly..ly + 4 {
                for x in lx..lx + 4 {
                    yuv[y * w + x] = 245;
                }
            }
            let c = (ly / 2) * (w / 2) + lx / 2;
            for i in [c, c + 1, c + w / 2, c + w / 2 + 1] {
                yuv[w * h + i] = 90;
                yuv[w * h + (w / 2) * (h / 2) + i] = 230;
            }
        }
        yuv
    }

    #[test]
    fn a_moving_light_draws_a_line_and_the_dark_stays_dark() {
        let (w, h) = (320, 240);
        let gain = 4.0;
        let sigma = noise_at(gain);
        let mut seed = 11u64;
        let mut trails = night(w, h, sigma, &mut seed, Some((20, 100)));
        let first = trails.clone();
        let mut plain = trails.clone();
        for i in 1..40 {
            let f = night(w, h, sigma, &mut seed, Some((20 + 6 * i, 100 + i)));
            lighten(&mut trails, &f, w, h, lighten_margin(gain));
            lighten(&mut plain, &f, w, h, 0);
        }
        assert!((0..40).all(|i| trails[(101 + i) * w + 21 + 6 * i] > 215), "a line all the way along");
        let c = ((100 + 20) / 2) * (w / 2) + (20 + 6 * 20) / 2;
        assert!(trails[w * h + c] < 110 && trails[w * h + (w / 2) * (h / 2) + c] > 200, "in the light's own colour");
        // Away from it: exactly the first frame; a plain lighten greys it.
        let mean = |p: &[u8]| (0..60).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| f32::from(p[y * w + x])).sum::<f32>() / (60 * w) as f32;
        assert_eq!(mean(&trails), mean(&first));
        assert!(mean(&plain) - mean(&first) > 1.5 * sigma, "{} {}", mean(&plain), mean(&first));
        // A darker frame changes nothing.
        let before = trails.clone();
        let dim: Vec<u8> = night(w, h, 0.0, &mut seed, None).iter().map(|v| v / 2).collect();
        assert_eq!(lighten(&mut trails, &dim, w, h, lighten_margin(gain)), 0);
        assert_eq!(trails, before);
        assert!(lighten_margin(1.0) < lighten_margin(4.0) && lighten_margin(4.0) < lighten_margin(10.0));
    }

    #[test]
    fn the_viewfinders_copy_keeps_a_thin_line_bright() {
        let (w, h) = (1640, 1232);
        let mut yuv = vec![128u8; w * h * 3 / 2];
        yuv[..w * h].fill(20);
        for x in 0..w {
            yuv[600 * w + x] = 250;
        }
        let small = small_brightest(&yuv, w, h, 320, 240);
        let kept = (110..125).map(|y| small[y * 320 + 160]).max().unwrap();
        let shrunk = shrink_yuv420(&yuv, w, h, 320, 240);
        let averaged = (110..125).map(|y| shrunk[y * 320 + 160]).max().unwrap();
        assert!(kept >= 245 && averaged < 80, "{kept} kept, {averaged} averaged");
        assert_eq!(small[10 * 320 + 10], 20);
    }

    #[test]
    fn each_frame_is_a_frames_length_a_stop_under_and_shorter_in_bright_light() {
        // Night: the frame's length, the gain for a stop under, up to its most.
        let (us, gain) = plan(400_000.0);
        assert_eq!(us, FRAME_US);
        assert!((gain - 3.0).abs() < 0.01, "{gain}");
        assert_eq!(plan(1e8).1, pro::MAX_GAIN);
        // Daylight: shorter, at the least gain.
        let (us, gain) = plan(2_000.0);
        assert_eq!((us, gain), (1_000, 1.0));
        assert!(LENGTHS.contains(&LENGTH_DEFAULT));
        assert_eq!(saved_words(186, 12_400), "trails · 186 frames over 12.4 s");
        assert_eq!(camkit::read_stacked("shot 13000 write 400 trails 186 12400"), Some((camkit::Exposing::Trails, 186, 12_400)));
        assert_eq!(length_words(60), "1 min");
    }
}
