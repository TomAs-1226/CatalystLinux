//! Night, HDR and the exposure lock (1.4.0): the Camera's automatic
//! exposure deciding from each frame, and the countdown ring while a stack
//! is taken. The decisions themselves are pro.rs's (each checked on its
//! own); this is where the screen takes them in and shows them.
//!
//! - **Night** (`n`: auto, off, 1 s, 2 s, 4 s, remembered). On auto it is in
//!   play once the dark has lasted a moment ([`pro::NightSense`]): the
//!   preview turns to Night's own (a tenth of a second, more gain), the
//!   motion sensor is held to tell a hand from a brace ([`crate::imu::
//!   Steadiness`]), and the shutter takes a stack ([`pro::night_plan`]):
//!   eighths of a second for about a second in the hand, up to a second
//!   each for two to four braced, merged by catalyst-camera as they come.
//! - **HDR** (`b` in photo mode: auto, off). A backlit or contrasty scene
//!   ([`pro::hdr_scene`]) takes four frames short enough for the sky,
//!   merged as Night's are, the shadows given back as it is finished.
//! - **The highlights' guard** (photo mode, always): the exposure eased
//!   down while part of the picture is blown ([`pro::HighlightGuard`]),
//!   the mid-tones given back as the photo is finished.
//! - **The lock** (`e` in photo mode): the exposure read on the middle of the
//!   picture, then held, until `e` again (or another mode).
//!
//! **Burst and light trails** (1.5.1, camera/burst_mode.rs, trails.rs) are
//! runs of frames told as they come too, and go through [`Stacking`]: a
//! burst's ring counts its frames, the trails draw a meter of their own over
//! the trails as they are made; Esc ends either, keeping what is taken.
//!
//! **The countdown ring**: over the picture held as it was (then the stack's
//! own first frame, small, in its place), a ring that fills as the frames
//! come, the seconds left in it and "hold still" under it; Esc cancels.
//! Its fill follows the frames on `apple-wwdc18-pip` (Bezel's meter, a
//! value following its source: detent's `role::settle`); it comes and goes
//! on Material's effects spring (`m3-default-effects`), never bouncing.
//! A jolt part way (the motion sensor) ends the stack as soon as enough of
//! it is merged, and keeps it.

use super::*;
use crate::pro::{HdrMode, Lock, NightMode};

/// A Night or HDR photo being taken: what it is, its frames as the camera
/// program tells them, when it began and how long it should take, and its
/// ring (its fill, and how far in sight).
pub struct Stacking {
    pub what: camkit::Exposing,
    pub planned: u32,
    news: Option<(camkit::StackNews, Instant)>,
    began: Instant,
    expect: Duration,
    /// Still being exposed (false once the sensor is done: the ring goes).
    pub live: bool,
    ring: Animated,
    shown: Animated,
    /// Told to end once enough is merged (a jolt), or cancelled (Esc).
    pub ended: bool,
    pub cancelled: bool,
}

impl Stacking {
    fn new(what: camkit::Exposing, plan: &pro::NightPlan, now: Instant) -> Stacking {
        // As catalyst-camera reckons it (still.h's stack_expect_ms): the
        // first frame at two of its lengths, then each its length and a merge.
        let frame = plan.frame_us.max(66_667) as f32 / 1e6;
        let expect = match what {
            // Light trails: their length.
            camkit::Exposing::Trails => plan.frames as f32 * plan.frame_us as f32 / 1e6,
            // A burst's frames come at every other one the sensor sends.
            camkit::Exposing::Burst => 2.0 * frame * plan.frames as f32,
            _ => 2.0 * frame + plan.frames.saturating_sub(1) as f32 * (frame + 0.06),
        };
        let mut shown = Animated::new(0.0, role::effect()).with_rest(0.002);
        shown.set(1.0);
        Stacking {
            what,
            planned: plan.frames.max(1),
            news: None,
            began: now,
            expect: Duration::from_secs_f32(expect),
            live: true,
            ring: Animated::new(0.0, role::settle()).with_rest(0.001),
            shown,
            ended: false,
            cancelled: false,
        }
    }

    /// How far through it is, 0 to 1: by its frames as told, and between
    /// them by the time a frame takes (never past the next frame's mark).
    pub fn progress(&self, now: Instant) -> f32 {
        // Light trails: by their length, as the seconds go.
        if self.what == camkit::Exposing::Trails {
            return (1.0 - self.left(now) / self.expect.as_secs_f32().max(0.01)).clamp(0.0, 1.0);
        }
        let per = self.expect.as_secs_f32() / self.planned as f32;
        match self.news {
            Some((n, at)) => {
                let within = (now.saturating_duration_since(at).as_secs_f32() / per.max(0.01)).min(0.9);
                ((n.taken as f32 + within) / n.planned.max(1) as f32).clamp(0.0, 1.0)
            }
            None => (now.saturating_duration_since(self.began).as_secs_f32() / self.expect.as_secs_f32().max(0.01) * 0.9).clamp(0.0, 1.0 / self.planned as f32),
        }
    }

    /// How far in sight it is (0 to 1), and its fill as drawn.
    pub fn faded(&self) -> f32 {
        self.shown.value()
    }

    pub fn fill(&self) -> f32 {
        self.ring.value()
    }

    /// Frames taken so far, as the camera program tells them (the first is
    /// there from the start).
    pub fn taken(&self) -> u32 {
        self.news.map_or(1, |(n, _)| n.taken.max(1))
    }

    /// Seconds it has run: by the camera program's own clock once it says
    /// (its length less what it has left), else since it was asked for.
    pub fn elapsed(&self, now: Instant) -> f32 {
        match self.news {
            Some(_) => (self.expect.as_secs_f32() - self.left(now)).max(0.0),
            None => now.saturating_duration_since(self.began).as_secs_f32(),
        }
    }

    /// Held where a picture of it wants it (the shots): in sight, filled so far.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn hold_for_shot(&mut self, fill: f32) {
        self.ring.jump(fill);
        self.shown.jump(1.0);
    }

    /// A run that keeps what it has when ended (a burst, light trails),
    /// not a stack Esc cancels.
    pub fn keeps(&self) -> bool {
        matches!(self.what, camkit::Exposing::Burst | camkit::Exposing::Trails)
    }

    /// The seconds left, by the camera program's own reckoning once it says.
    pub fn left(&self, now: Instant) -> f32 {
        match self.news {
            Some((n, at)) => (n.left_ms as f32 / 1000.0 - now.saturating_duration_since(at).as_secs_f32()).max(0.0),
            None => (self.expect.as_secs_f32() - now.saturating_duration_since(self.began).as_secs_f32()).max(0.0),
        }
    }
}

/// What a saved stack says: its frames ("night · 8 frames merged", or "5 of
/// 8" where the hand moved), and the binned size where memory made it so.
pub(super) fn saved_words((what, merged, planned): (camkit::Exposing, u32, u32), size: (u32, u32)) -> String {
    // A burst's and the trails' own (1.5.1).
    match what {
        camkit::Exposing::Burst => return super::burst_mode::saved_words(merged, planned),
        camkit::Exposing::Trails => return super::trails::saved_words(merged, planned),
        _ => {}
    }
    let name = if what == camkit::Exposing::Hdr { "hdr" } else { "night" };
    let frames = if merged < planned { format!("{merged} of {planned} frames") } else { format!("{merged} frames merged") };
    if size.0.max(size.1) < 2000 {
        format!("{name} · {frames} · {} × {} (memory)", size.0, size.1)
    } else {
        format!("{name} · {frames}")
    }
}

impl Camera {
    /// A stack begun: its ring, the motion sensor held and its jolts
    /// counted from here.
    pub(super) fn begin_stacking(&mut self, what: camkit::Exposing, plan: &pro::NightPlan) {
        self.stacking = Some(Stacking::new(what, plan, Instant::now()));
        if self.night_imu.is_none() {
            self.night_imu = Some(crate::imu::hold(crate::imu::Use::Motion));
        }
        self.steadiness.begin(crate::imu::reading().as_ref());
    }

    /// The camera program's word of its frames so far (kind 8).
    pub(super) fn stack_news(&mut self, line: &str) {
        if let (Some(s), Some(news)) = (&mut self.stacking, camkit::StackNews::read(line)) {
            s.news = Some((news, Instant::now()));
        }
    }

    /// The sensor done (or the photo given up): the ring fills and goes.
    pub(super) fn end_stacking(&mut self) {
        if let Some(s) = &mut self.stacking {
            if s.live && !s.cancelled {
                s.ring.set(1.0);
            }
            s.live = false;
            s.shown.set(0.0);
        }
    }

    /// Esc (or Enter, for light trails) while a burst or the trails are
    /// taken: ended now, keeping what is taken.
    pub(super) fn end_keeping(&mut self) {
        let Some(s) = &mut self.stacking else { return };
        if s.ended || s.cancelled {
            return;
        }
        s.ended = true;
        let burst = s.what == camkit::Exposing::Burst;
        self.control.send(Order::StackEnd);
        self.say(if burst { "stopped: keeping the sharpest so far" } else { "finishing the trails" });
    }

    /// Esc while a stack is taken: no photo.
    pub(super) fn cancel_stacking(&mut self) {
        let Some(s) = &mut self.stacking else { return };
        if s.cancelled {
            return;
        }
        s.cancelled = true;
        self.control.send(Order::StackCancel);
        self.say("cancelling");
    }

    /// The ring's motion: its fill following the frames, its coming and going.
    pub(super) fn tick_stacking(&mut self, dt: f32) -> bool {
        let Some(s) = &mut self.stacking else { return false };
        if s.live && !s.cancelled {
            let p = s.progress(Instant::now()).max(s.ring.target());
            if (p - s.ring.target()).abs() > 1e-4 {
                s.ring.set(p);
            }
        }
        let changed = s.ring.tick(dt) | s.shown.tick(dt);
        if !s.live && s.shown.value() < 0.01 && s.shown.target() < 0.5 {
            self.stacking = None;
            return true;
        }
        // The seconds left change on their own: drawn while it is taken.
        changed || s.live
    }

    /// The motion sensor while Night is in play: how still the hand is (its
    /// chip's length follows), and a jolt while a stack is taken (it ends
    /// as soon as enough is merged). True if what is shown changed.
    pub(super) fn tick_steady(&mut self, dt: f32) -> bool {
        if self.night_imu.is_none() {
            return false;
        }
        let Some(r) = crate::imu::reading() else { return false };
        let braced = self.steadiness.braced();
        self.steadiness.feed(&r, dt);
        let jolted = self.steadiness.jolted(&r);
        if let Some(s) = &mut self.stacking {
            // (A burst or light trails go on: a light painted is the hand moving.)
            if s.live && jolted && !s.ended && !s.cancelled && !s.keeps() {
                s.ended = true;
                self.control.send(Order::StackEnd);
                self.say("moved: keeping what is merged");
            }
        }
        braced != self.steadiness.braced()
    }

    /// The automatic exposure's decisions from a new frame: the light
    /// meter's scale, Night's dark, the lock's reading, the highlights'
    /// guard, HDR's scene; the motion sensor held while Night is in play;
    /// the preview told of any change. Nothing moves while a photo is taken.
    pub(super) fn tick_auto(&mut self) -> bool {
        let now = Instant::now();
        let dt = self.auto_at.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32()).min(0.25);
        self.auto_at = Some(now);
        let Some(m) = self.metered else { return false };
        if self.saving.is_some() && !self.captured {
            return false;
        }
        let mut changed = false;
        if self.mode != Mode::Pixel {
            self.tones = pro::Tones::of(&self.histogram);
        }
        // The light meter's scale, from the camera's own exposure only.
        let own = self.told.is_some_and(|c| c.exposure_us.is_none() && c.gain.is_none() && c.ev == 0.0 && !c.spot) && self.face.is_none();
        if own {
            self.lux_scale.feed(&m);
        }
        changed |= self.night_sense.feed(&m, dt);
        if self.lock.feed(&m, dt) {
            self.say("exposure locked · e: let go");
            changed = true;
        }
        let guarding = self.mode == Mode::Photo && !self.lock.on() && !(self.night_on() && self.night_sense.dark);
        if guarding {
            changed |= self.guard.feed(&self.tones, self.face.is_some(), dt);
        } else if self.guard.ev != 0.0 {
            self.guard = pro::HighlightGuard::default();
            changed = true;
        }
        changed |= self.hdr_sense.feed(&self.tones, self.guard.ev, dt);
        // The motion sensor, while Night is in play or a stack is taken.
        let want = self.night_on() || self.stacking.as_ref().is_some_and(|s| !s.keeps());
        if want != self.night_imu.is_some() {
            self.night_imu = want.then(|| crate::imu::hold(crate::imu::Use::Motion));
            if !want {
                self.steadiness = crate::imu::Steadiness::default();
            }
            changed = true;
        }
        // The smart white balance's gains for the frame (camera/wb.rs).
        self.feed_smart_wb(dt);
        // Told only when it changes (Night's gain moves in sixths of a stop,
        // the white balance only while it glides).
        self.tell_pro();
        changed
    }

    /// Night's setting changed (`n`, or its chip's choices).
    pub(super) fn set_night(&mut self, mode: NightMode) {
        self.night_mode = mode;
        self.keep_prefs();
        self.tell_pro();
        self.say(match mode {
            NightMode::Auto => "night: auto · on by itself in the dark".to_owned(),
            NightMode::Off => "night off".to_owned(),
            NightMode::Fixed(s) => format!("night: {s} s · always"),
        });
    }

    /// HDR's setting changed (`b`, or its chip's choices).
    pub(super) fn set_hdr(&mut self, mode: HdrMode) {
        self.hdr_mode = mode;
        self.keep_prefs();
        self.say(match mode {
            HdrMode::Auto => "hdr: auto · backlit scenes merged",
            HdrMode::Off => "hdr off",
        });
    }

    /// The lock (`e`): read on the middle and held, or let go.
    pub(super) fn toggle_lock(&mut self) {
        if self.lock.on() {
            self.lock = Lock::Off;
            self.say("exposure unlocked");
        } else {
            self.lock = Lock::Reading(0.0);
            self.guard = pro::HighlightGuard::default();
            self.say("reading the middle");
        }
        self.tell_pro();
    }

    /// Night's chip's words: its setting, and its length while in play
    /// ("auto · 1 s", "2 s", "auto", "off").
    pub(super) fn night_words(&self) -> String {
        let length = self.night_on().then(|| self.night_plan()).flatten().map(|p| p.length_words());
        match (self.night_mode, length) {
            (NightMode::Off, _) => "off".to_owned(),
            (NightMode::Auto, Some(l)) => format!("auto · {l}"),
            (NightMode::Auto, None) => "auto · not dark".to_owned(),
            // A length chosen, shorter in more light than it needs.
            (m @ NightMode::Fixed(_), Some(l)) if l != m.words() => format!("{} · {l} in this light", m.words()),
            (NightMode::Fixed(_), Some(l)) => l,
            (m, None) => m.words().to_owned(),
        }
    }

    /// Night's length on its chip, while it is in play.
    pub(super) fn night_length(&self) -> Option<String> {
        self.night_on().then(|| self.night_plan()).flatten().map(|p| p.length_words())
    }

    /// The countdown ring over the picture: a dark disc, the ring's track
    /// and its fill in the signal colour (progress), the seconds left in it
    /// and "hold still" under it.
    pub(super) fn draw_stacking(&mut self, g: &mut Gfx, vf: Rect) {
        let Some(s) = &mut self.stacking else { return };
        // Light trails draw their own, over the trails (trails.rs).
        if s.what == camkit::Exposing::Trails {
            return;
        }
        if g.calm {
            s.ring.jump(s.ring.target());
            s.shown.jump(s.shown.target());
        }
        let a = s.shown.value().clamp(0.0, 1.0);
        if a < 0.01 {
            return;
        }
        let (t, ty) = (g.tokens, g.ty);
        let now = Instant::now();
        let white = Color(255, 255, 255);
        // My own sizes: the ring as wide as the timer's disc (64), its
        // stroke as Bezel's dial's track reads at this size.
        let r = 26.0;
        let (cx, cy) = (vf.cx() + g.ox, vf.cy() - 6.0);
        g.canvas.circle(cx, cy, r + 9.0, Color(0, 0, 0), 0.5 * a);
        g.canvas.arc(cx, cy, r, 4.0, 0.0, std::f32::consts::TAU, white, 0.22 * a);
        let fill = s.ring.value().clamp(0.0, 1.0);
        g.canvas.arc(cx, cy, r, 4.0, 0.0, fill * std::f32::consts::TAU, t.signal, a);
        let left = s.left(now);
        let burst = s.what == camkit::Exposing::Burst;
        // A burst counts its frames; a stack, its seconds.
        let words = if s.cancelled || (burst && s.ended) {
            "…".to_owned()
        } else if burst {
            s.taken().to_string()
        } else {
            format!("{left:.1}")
        };
        let big = ty.display(20.0);
        let half = g.measure(&words, big) * 0.5;
        g.text_alpha(vf.cx() - half, cy - 3.0, &words, big, white, a);
        let of = format!("of {}", s.planned);
        let unit = if burst { of.as_str() } else { "s" };
        let unit_x = vf.cx() - g.measure(unit, ty.caption) * 0.5;
        g.text_alpha(unit_x, cy + 13.0, unit, ty.caption, white, 0.7 * a);
        // Under it: what it is, and how to stop it.
        let what = match s.what {
            camkit::Exposing::Hdr => "hdr",
            _ => "night",
        };
        let forms = if burst {
            ["burst · esc: keep the sharpest so far".to_owned(), "burst · esc: keep the sharpest".to_owned(), "burst".to_owned()]
        } else {
            [format!("{what} · hold still · esc: cancel"), format!("{what} · hold still"), "hold still".to_owned()]
        };
        let cap = ty.caption;
        let label = forms.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 16.0).unwrap_or(&forms[2]).clone();
        let w = g.measure(&label, cap) + 16.0;
        let h = cap.size + 8.0;
        let y = cy + r + 14.0;
        g.canvas.rrect(Rect::new(vf.cx() - w * 0.5 + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.55 * a);
        g.text_alpha(vf.cx() - w * 0.5 + 8.0, y + h * 0.5, &label, cap, white, a);
    }

    /// The lock reading the middle: brackets round what it reads (the
    /// tuning's spot metering: the middle zones, about a quarter each way).
    pub(super) fn draw_lock(&mut self, g: &mut Gfx, vf: Rect) {
        if !matches!(self.lock, Lock::Reading(_)) || self.mode != Mode::Photo {
            return;
        }
        let r = Self::on_viewfinder(vf, [0.375, 0.375, 0.625, 0.625]);
        brackets(g, r, Color(255, 255, 255), 0.85);
    }
}

// --------------------------------------------------------------- the shots ---

/// Night, HDR and the lock (`--camera-night-shots DIR`): Night's chip in
/// each state (in the light, in play in the hand and braced, a length
/// chosen, off) with its choices; the countdown ring, Night's and HDR's;
/// HDR's chip, auto and in play, and off; the lock reading and held; the
/// bar at rest; the keys' sheet; dark and light at the three text sizes.
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use super::bar::Chip;
    use super::pixel_mode::{render, shot_camera};
    use crate::shell::Shell;
    crate::smartcam::sim::choose(false);
    // An evening scene for the light, and the same street at night.
    let day = scene(PREVIEW_W, PREVIEW_H, 3.1, false);
    let mut dusk = day.clone();
    sim_expose(&mut dusk, 0.32, 3400);
    let developed = |pic: &Picture| {
        let yuv = rgb_to_yuv420(pic);
        let mut out = Picture::default();
        Look::Original.compiled(0.0).develop(&yuv, PREVIEW_W, PREVIEW_H, &mut out, None);
        (out, pro::Histogram::of_luma(&yuv[..PREVIEW_W * PREVIEW_H], PREVIEW_W, PREVIEW_H, 2))
    };
    let (day_pic, day_hist) = developed(&day);
    let (dusk_pic, dusk_hist) = developed(&dusk);
    let lit = Metered { exposure_us: 16_667, analogue: 2.0, digital: 1.0, gains: [1.82, 1.46], ct: 5200, lux: 320.0, settled: true, fom: 900, zoom: 0.0 };
    let dark = Metered { exposure_us: pro::NIGHT_PREVIEW_US, analogue: 10.6, digital: 1.7, gains: [1.45, 2.1], ct: 3100, lux: 4.0, settled: true, fom: 300, zoom: 0.0 };
    type Scene = (String, Box<dyn Fn(&mut Shell)>);
    type Setup = Box<dyn Fn(&mut Camera)>;
    let scene_of = |name: &str, night: bool, setup: Setup, keys: Vec<Key>| -> Scene {
        let (pic, hist, m) = if night { (dusk_pic.clone(), dusk_hist.clone(), dark) } else { (day_pic.clone(), day_hist.clone(), lit) };
        (
            name.to_owned(),
            Box::new(move |shell: &mut Shell| {
                crate::imu::simulate_standing(false);
                let mut c = shot_camera();
                c.set_mode(Mode::Photo);
                c.preview = Some(pic.clone());
                c.histogram = hist.clone();
                c.metered = Some(m);
                c.night_mode = NightMode::Auto;
                c.hdr_mode = HdrMode::Auto;
                c.night_sense.dark = night;
                c.told = Some(pro::PreviewControls::auto(0.0, pro::PREVIEW_CAP_US));
                setup(&mut c);
                c.note = None;
                c.mode_at.jump(c.mode_at.target());
                shell.show(Box::new(c));
                for k in &keys {
                    for _ in 0..30 {
                        shell.tick(1.0 / 60.0);
                        shell.draw();
                    }
                    shell.key(*k);
                }
            }),
        )
    };
    let braced = |c: &mut Camera| {
        crate::imu::simulate_standing(true);
        c.night_imu = Some(crate::imu::hold(crate::imu::Use::Motion));
        for _ in 0..40 {
            if let Some(r) = crate::imu::reading() {
                c.steadiness.feed(&r, 0.03);
            }
        }
    };
    let ring = |what: camkit::Exposing, frames: u32, taken: u32, frame_us: u32| {
        move |c: &mut Camera| {
            let plan = pro::NightPlan { frames, frame_us, gain: 8.0, lift: 1.0 };
            let mut s = Stacking::new(what, &plan, Instant::now());
            let each = u64::from(frame_us.max(66_667)) / 1000 + 60;
            s.news = Some((camkit::StackNews { taken, planned: frames, merged: taken, left_ms: u64::from(frames - taken) * each }, Instant::now()));
            s.ring.jump(taken as f32 / frames as f32);
            s.shown.jump(1.0);
            c.stacking = Some(s);
            c.saving = Some(Instant::now());
            c.captured = false;
            c.bar.hidden = true;
        }
    };
    let scenes: Vec<Scene> = vec![
        scene_of("01-bar", false, Box::new(|_| {}), Vec::new()),
        scene_of("02-night-auto-in-the-light", false, Box::new(|c| c.bar.focus = Some(Chip::Night)), Vec::new()),
        scene_of("03-night-auto-dark-in-the-hand", true, Box::new(|c| c.bar.focus = Some(Chip::Night)), Vec::new()),
        scene_of(
            "04-night-auto-dark-braced",
            true,
            Box::new(move |c| {
                braced(c);
                c.bar.focus = Some(Chip::Night);
            }),
            Vec::new(),
        ),
        scene_of(
            "05-night-2s",
            false,
            Box::new(|c| {
                c.night_mode = NightMode::Fixed(2);
                c.bar.focus = Some(Chip::Night);
            }),
            Vec::new(),
        ),
        scene_of(
            "06-night-off",
            true,
            Box::new(|c| {
                c.night_mode = NightMode::Off;
                c.bar.focus = Some(Chip::Night);
            }),
            Vec::new(),
        ),
        scene_of("07-night-choices", true, Box::new(|c| c.bar.focus = Some(Chip::Night)), vec![Key::Enter, Key::Right]),
        scene_of("08-night-dark-bar-at-rest", true, Box::new(|_| {}), Vec::new()),
        scene_of("09-countdown-night", true, Box::new(ring(camkit::Exposing::Night, 8, 3, 125_000)), Vec::new()),
        scene_of("10-countdown-night-braced", true, Box::new(ring(camkit::Exposing::Night, 4, 1, 1_000_000)), Vec::new()),
        scene_of("11-countdown-hdr", false, Box::new(ring(camkit::Exposing::Hdr, 4, 2, 4_000)), Vec::new()),
        scene_of(
            "12-hdr-auto-backlit",
            false,
            Box::new(|c| {
                c.hdr_sense.on = true;
                c.guard.ev = -1.0;
                c.bar.focus = Some(Chip::Hdr);
            }),
            Vec::new(),
        ),
        scene_of("13-hdr-auto-idle", false, Box::new(|c| c.bar.focus = Some(Chip::Hdr)), Vec::new()),
        scene_of(
            "14-hdr-off",
            false,
            Box::new(|c| {
                c.hdr_mode = HdrMode::Off;
                c.bar.focus = Some(Chip::Hdr);
            }),
            Vec::new(),
        ),
        scene_of("15-lock-reading", false, Box::new(|c| c.lock = Lock::Reading(0.2)), Vec::new()),
        scene_of(
            "16-lock-held",
            false,
            Box::new(|c| {
                c.lock = Lock::Held { exposure_us: 16_667, gain: 2.0 };
                c.bar.focus = Some(Chip::Lock);
            }),
            Vec::new(),
        ),
        scene_of("17-keys", false, Box::new(|_| {}), vec![Key::Char('?')]),
        scene_of("18-keys-more", false, Box::new(|_| {}), vec![Key::Char('?'), Key::PageDown]),
    ];
    render(dir, &scenes);
    crate::imu::simulate_standing(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Settings;

    fn press(camera: &mut Camera, settings: &mut Settings, key: Key) {
        let mut robot = Robot::none();
        let mut say = None;
        camera.key(key, &mut Cx { robot: &mut robot, settings, say: &mut say, at_rest: true });
    }

    #[test]
    fn n_steps_night_round_and_it_is_remembered() {
        let _port = crate::smartcam::sim::exclusive(false);
        crate::shell::tests::sandbox();
        let (mut camera, mut s) = (Camera::new(), Settings::default());
        camera.night_mode = NightMode::Auto;
        let mut seen = Vec::new();
        for _ in 0..5 {
            press(&mut camera, &mut s, Key::Char('n'));
            seen.push(camera.night_mode);
            // Kept: a camera opened now starts where this one is.
            assert_eq!(Prefs::load().night, camera.night_mode);
        }
        assert_eq!(seen, [NightMode::Off, NightMode::Fixed(1), NightMode::Fixed(2), NightMode::Fixed(4), NightMode::Auto]);
        press(&mut camera, &mut s, Key::Char('n'));
        drop(camera);
        let again = Camera::new();
        assert_eq!(again.night_mode, NightMode::Off, "remembered across the Camera's openings");
        // HDR's `b` and the lock's `e` are photo mode's.
        let mut camera = again;
        press(&mut camera, &mut s, Key::Char('b'));
        assert_eq!(camera.hdr_mode, HdrMode::Off);
        assert_eq!(Prefs::load().hdr, HdrMode::Off);
        press(&mut camera, &mut s, Key::Char('e'));
        assert!(matches!(camera.lock, Lock::Reading(_)));
        assert!(camera.preview_controls().spot, "read on the middle");
        press(&mut camera, &mut s, Key::Char('e'));
        assert_eq!(camera.lock, Lock::Off);
        camera.set_mode(Mode::Pro);
        press(&mut camera, &mut s, Key::Char('b'));
        assert_eq!(camera.hdr_mode, HdrMode::Off, "pro's b is its bracketing");
        assert!(camera.kit.bracket.is_some());
    }

    #[test]
    fn in_the_dark_the_shutter_takes_nights_stack_and_the_preview_brightens() {
        let _port = crate::smartcam::sim::exclusive(false);
        crate::shell::tests::sandbox();
        let mut camera = Camera::new();
        camera.night_mode = NightMode::Auto;
        camera.told = Some(pro::PreviewControls::auto(0.0, pro::PREVIEW_CAP_US));
        // Lit: one frame, the camera's own preview.
        // (Its exposure what the light meter says 320 lux needs.)
        camera.metered = Some(Metered { exposure_us: 16_667, analogue: 4.4, lux: 320.0, settled: true, ..Metered::default() });
        for _ in 0..30 {
            camera.auto_at = Some(Instant::now() - Duration::from_millis(33));
            camera.tick_auto();
        }
        assert!(!camera.night_on() && camera.stack_now().is_none());
        assert_eq!(camera.preview_controls().exposure_us, None);
        // Dark: Night in play, its own brighter preview, the motion sensor held.
        let dark = Metered { exposure_us: pro::AE_LONGEST_US, analogue: 10.0, lux: 3.0, settled: true, ..Metered::default() };
        camera.metered = Some(dark);
        for _ in 0..30 {
            camera.auto_at = Some(Instant::now() - Duration::from_millis(33));
            camera.tick_auto();
        }
        assert!(camera.night_on() && camera.night_imu.is_some());
        let p = camera.preview_controls();
        assert_eq!(p.exposure_us, Some(pro::NIGHT_PREVIEW_US));
        assert!(p.gain.unwrap() > pro::MAX_GAIN, "{p:?}");
        // The shutter's words: a stack of eighths of a second in the hand.
        let (words, burst, _) = camera.still_words(std::path::Path::new("/x/IMG_1.jpg"), crate::burst::DetailMode::Auto);
        assert!(burst.is_none(), "Night takes the place of detail's burst");
        for w in ["stack=8", "stackfor=night", "exp=125000", "gain=10.000", "neutral=0.50"] {
            assert!(words.iter().any(|x| x == w), "{w} in {words:?}");
        }
        assert!(words.iter().any(|w| w.starts_with("lift=")) && !words.iter().any(|w| w.starts_with("raw=")));
        assert_eq!(camera.night_words(), "auto · 1 s");
        // Off: the camera's own exposure and one frame, however dark.
        camera.night_mode = NightMode::Off;
        assert!(camera.stack_now().is_none() && camera.preview_controls().exposure_us.is_none());
    }

    #[test]
    fn a_night_photo_is_counted_down_cancelled_with_esc_and_kept_when_done() {
        let _port = crate::smartcam::sim::exclusive(false);
        crate::shell::tests::sandbox();
        // The simulated room in the dark, lit again however this ends.
        struct Lit;
        impl Drop for Lit {
            fn drop(&mut self) {
                sim_light(320.0);
            }
        }
        let _lit = Lit;
        sim_light(3.0);
        let mut camera = Camera::new();
        camera.night_mode = NightMode::Auto;
        let mut s = Settings::default();
        let run = |camera: &mut Camera, ms: u64| {
            let end = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < end {
                camera.ambient_tick(1.0 / 60.0, &Robot::none());
                camera.tick(1.0 / 60.0);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // The simulated room in the dark: Night goes on by itself.
        let start = Instant::now();
        while !camera.night_on() && start.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 50);
        }
        assert!(camera.night_on(), "on by itself in the dark");
        run(&mut camera, 300);
        // Shutter: the ring, then Esc: no photo.
        press(&mut camera, &mut s, Key::Enter);
        assert!(camera.stacking.as_ref().is_some_and(|s| s.live));
        run(&mut camera, 200);
        press(&mut camera, &mut s, Key::Back);
        let start = Instant::now();
        while camera.saving.is_some() && start.elapsed() < Duration::from_secs(5) {
            run(&mut camera, 50);
        }
        assert!(camera.saving.is_none() && camera.pending == 0, "cancelled: nothing saving");
        assert!(camera.note.as_ref().is_some_and(|(n, _)| n.contains("cancelled")), "{:?}", camera.note);
        // Again, to the end: saved, its frames said, the ring gone.
        press(&mut camera, &mut s, Key::Enter);
        let start = Instant::now();
        let mut filled = 0.0f32;
        while camera.saving.is_some() && start.elapsed() < Duration::from_secs(20) {
            run(&mut camera, 50);
            if let Some(st) = &camera.stacking {
                filled = filled.max(st.ring.value());
            }
        }
        assert!(camera.saving.is_none(), "saved");
        assert!(filled > 0.6, "the ring filled as the frames came: {filled}");
        assert!(camera.note.as_ref().is_some_and(|(n, _)| n.starts_with("night · 8 frames merged")), "{:?}", camera.note);
        run(&mut camera, 600);
        assert!(camera.stacking.is_none(), "the ring has gone");
    }
}
