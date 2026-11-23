//! The depth effect: Numbers, FRC 5805's robot, standing in front of the
//! time on the lock screen and on StandBy's robot clock, as a phone's
//! wallpaper subject stands in front of its clock.
//!
//! - **Depth.** The time is drawn first, large, then the robot over it: its
//!   own cover (the render's silhouette, see-through between its rails)
//!   hides what is behind it, and the date and the status chips are drawn
//!   last, over everything. The camera is placed so the robot reaches at
//!   most the lower third of the digits (`COVER`, held below `MOST_COVER`
//!   at every angle and pose it is shown at), so the hours and minutes
//!   always read.
//! - **Poses.** What the robot does follows the handheld (Settings › Lock
//!   screen › Pose follows status): idle, it turns slowly on its stand; on
//!   the charger it rests "docked", stowed, over a soft light in the signal
//!   colour; low on battery it slumps (hood down, sunk, dimmed); with a match
//!   coming it stands ready, facing the viewer, intake out, hood up, bumpers
//!   in the alliance's colour. It moves between them on Bezel's `settle`
//!   spring (the mechanisms on the spatial spring the Robot app uses, the
//!   light on `smooth`); a change of look (dimmed, painted) crossfades on
//!   `effect`.
//! - **Cost.** The turntable is ambient, not motion: the shell looks in
//!   every `DRIFT_EVERY` for it (`Shell::ambient`), so the frame beat
//!   rests. The lock screen keeps its picture under the robot, so a frame
//!   of the turn is a copy and the robot laid over it. Each rendered angle
//!   (every 4°) is decoded as the turn reaches it and kept while near; in
//!   between, two are blended. While it turns fast between poses it shows
//!   the rendered angles themselves, unblended. On StandBy, in the battery
//!   saver and in the calm version it stands still.

use bezel::{Color, Rect};
use detent::{role, Animated};

use crate::gfx::Gfx;
use crate::object3d::{Looks, PartLook, View, MECHANISMS};
use crate::power::Reading;

/// The robot shown: Numbers, the shell's own, whose extent is known (a
/// team's own robot shows in the Robot app).
pub const OBJECT: &str = crate::robot3d::BUILT_IN;
const SIZE: &str = "hero";
/// Its frame, and where in it Numbers stands at any angle and pose shown
/// here (measured with `Objects::extent`; the test below holds them): its
/// highest point (the hood raised, the intake out), its lowest (the
/// bumpers), and how far it reaches either side of the middle.
const FRAME: f32 = 112.0;
pub const TOP: f32 = 18.0;
pub const BOTTOM: f32 = 95.0;
pub const HALF_WIDTH: f32 = 54.0;

/// How much of the time's height the robot stands in front of, at its
/// highest; never more than the lower third (`MOST_COVER`), so the digits
/// read. (Our own tuning.)
pub const COVER: f32 = 0.30;
pub const MOST_COVER: f32 = 1.0 / 3.0;
const _: () = assert!(COVER < MOST_COVER);

/// The turntable: degrees a second (a turn in 72 s), and how often the
/// shell draws it: 12.5 a second, smooth at this speed (a third of a
/// degree a frame) and a fifth of the panel's frames. (Our own tuning.)
pub const DRIFT_SPEED: f32 = 5.0;
pub const DRIFT_EVERY: std::time::Duration = std::time::Duration::from_millis(80);
/// Faster than a rendered angle a frame (4° at 30 a second), a turn shows
/// the rendered angles themselves, each decoded once, unblended.
const SNAP_ABOVE: f32 = 120.0;
const STEP: f32 = 4.0;

/// The handheld on battery at this charge or less is low (as the widgets say).
pub const LOW_PERCENT: u32 = 15;
/// A match this near (or its queue this near, or Nexus calling it) readies
/// the robot.
pub const SOON: i64 = 20 * 60;
const QUEUE_SOON: i64 = 10 * 60;

/// The angles each pose is seen from: a three-quarter view from the front
/// left to start turning from and to rest docked at; facing the viewer
/// (the intake's own facing, from the pack); a little turned away, slumped.
const IDLE_YAW: f32 = 0.0;
const DOCKED_YAW: f32 = 348.0;
pub const FACING_YAW: f32 = 28.0;
const SLUMP_YAW: f32 = 340.0;
/// Slumped: how far it sinks, and how dim it goes. (Our own tuning.)
const SLUMP: f32 = 3.0;
const LOW_DIM: f32 = 0.5;

/// What the robot is doing, as the handheld's state has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    /// Turning slowly on its stand.
    Idle,
    /// On the charger: docked, stowed, over the charge light.
    Charging,
    /// Low on battery: slumped and dimmed.
    Low,
    /// A match coming: facing the viewer, intake out, hood up.
    Ready,
}

/// The pose for the handheld's state: low on battery first (it is what
/// needs doing), then a match coming, then the charger; else idle. With
/// the pose not following (Settings), always idle.
pub fn choose(follows: bool, reading: Option<&Reading>, soon: bool) -> Pose {
    if !follows {
        return Pose::Idle;
    }
    let on_charger = reading.is_some_and(|r| matches!(r.flow, crate::power::Flow::Charging | crate::power::Flow::Full | crate::power::Flow::Holding));
    let low = reading.is_some_and(|r| r.flow.on_battery() && r.percent <= LOW_PERCENT);
    if low {
        Pose::Low
    } else if soon {
        Pose::Ready
    } else if on_charger {
        Pose::Charging
    } else {
        Pose::Idle
    }
}

/// Whether the team's next match is near enough to stand ready for.
pub fn match_soon(robot: &crate::robot::Robot, now: i64) -> bool {
    robot.schedule.view().is_some_and(|v| soon_in(&v, now))
}

/// The same, from a schedule already read: Nexus says it is queuing (or on
/// deck, or on the field), or its queue is within ten minutes, or the match
/// itself within twenty.
pub fn soon_in(view: &crate::schedule::View, now: i64) -> bool {
    let Some(m) = view.next(now) else { return false };
    m.status.is_some() || m.queue_at.is_some_and(|q| q - now <= QUEUE_SOON) || m.time.is_some_and(|t| t - now <= SOON)
}

/// How a pose is shot: the angle (none: wherever the turntable is), how far
/// the robot sinks, its intake's and hood's steps, the charge light, and
/// how dim it is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    pub yaw: Option<f32>,
    pub sink: f32,
    pub intake: f32,
    pub hood: f32,
    pub glow: f32,
    pub dim: f32,
}

impl Pose {
    pub const ALL: [Pose; 4] = [Pose::Idle, Pose::Charging, Pose::Low, Pose::Ready];

    pub fn name(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Charging => "charging",
            Pose::Low => "low",
            Pose::Ready => "ready",
        }
    }

    pub fn camera(self) -> Camera {
        let rest = Camera { yaw: None, sink: 0.0, intake: 0.0, hood: 0.0, glow: 0.0, dim: 0.0 };
        match self {
            Pose::Idle => rest,
            Pose::Charging => Camera { yaw: Some(DOCKED_YAW), glow: 1.0, ..rest },
            Pose::Low => Camera { yaw: Some(SLUMP_YAW), sink: SLUMP, dim: LOW_DIM, ..rest },
            // The intake out and the hood at its highest step.
            Pose::Ready => Camera { yaw: Some(FACING_YAW), intake: 1.0, hood: 2.0, ..rest },
        }
    }
}

/// The robot's colours beyond the palette: dimmed, and its bumpers painted.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Look {
    pub dim: f32,
    pub paint: Option<Color>,
}

impl Look {
    fn looks(self) -> Looks {
        let looks = Looks { dim: self.dim, ..Looks::plain() };
        match self.paint {
            Some(c) => looks.part("bumpers", PartLook { paint: Some(c), glow: None }),
            None => looks,
        }
    }
}

/// The robot as it is drawn this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    pub pose: Pose,
    pub yaw: f32,
    pub sink: f32,
    pub intake: f32,
    pub hood: f32,
    pub glow: f32,
    pub look: Look,
    /// The look it is crossfading from, and how far in the new one is.
    pub before: Option<(Look, f32)>,
}

impl Shot {
    /// A pose at rest (the editor's preview, the tests).
    pub fn settled(pose: Pose, paint: Option<Color>) -> Shot {
        let c = pose.camera();
        Shot { pose, yaw: c.yaw.unwrap_or(IDLE_YAW), sink: c.sink, intake: c.intake, hood: c.hood, glow: c.glow, look: Look { dim: c.dim, paint }, before: None }
    }

    /// At another angle (the tests).
    #[cfg(test)]
    pub fn turned(self, yaw: f32) -> Shot {
        Shot { yaw, ..self }
    }
}

/// The robot on the lock screen or StandBy: its pose, and each part of the
/// camera on its spring.
pub struct Robot {
    pose: Option<Pose>,
    yaw: Animated,
    sink: Animated,
    intake: Animated,
    hood: Animated,
    glow: Animated,
    look: Look,
    before: Option<Look>,
    fade: Animated,
    /// Drawn at the last frame: the turntable runs only while it shows.
    pub shown: bool,
}

impl Default for Robot {
    fn default() -> Self {
        Self::new()
    }
}

impl Robot {
    pub fn new() -> Robot {
        Robot {
            pose: None,
            yaw: Animated::new(IDLE_YAW, role::settle()).with_rest(0.05),
            sink: Animated::new(0.0, role::settle()).with_rest(0.02),
            intake: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.005),
            hood: Animated::new(0.0, crate::gfx::spatial::default()).with_rest(0.005),
            glow: Animated::new(0.0, role::smooth()).with_rest(0.004),
            look: Look::default(),
            before: None,
            fade: Animated::new(1.0, role::effect()).with_rest(0.004),
            shown: false,
        }
    }

    /// Take up `pose`, the bumpers in `paint`: on the springs, or at once
    /// (`at_once`: the calm version, the battery saver; and the first time).
    /// True if it set something moving.
    pub fn aim(&mut self, pose: Pose, paint: Option<Color>, at_once: bool) -> bool {
        let c = pose.camera();
        let look = Look { dim: c.dim, paint };
        if self.pose == Some(pose) && self.look == look {
            return false;
        }
        let at_once = at_once || self.pose.is_none();
        let now = self.yaw.value();
        // The nearest way round; idle turns on from wherever it is.
        let yaw = match c.yaw {
            Some(y) => now + ((y - now + 180.0).rem_euclid(360.0) - 180.0),
            None if self.pose.is_none() => IDLE_YAW,
            None => now,
        };
        for (a, to) in [(&mut self.yaw, yaw), (&mut self.sink, c.sink), (&mut self.intake, c.intake), (&mut self.hood, c.hood), (&mut self.glow, c.glow)] {
            if at_once {
                a.jump(to);
            } else {
                a.set(to);
            }
        }
        if look != self.look {
            if at_once {
                self.before = None;
                self.fade.jump(1.0);
            } else {
                self.before = Some(self.look);
                self.fade.jump(0.0);
                self.fade.set(1.0);
            }
            self.look = look;
        }
        self.pose = Some(pose);
        !at_once
    }

    /// The springs on by `md`: true while anything moves.
    pub fn tick(&mut self, md: f32) -> bool {
        let mut moving = false;
        for a in [&mut self.yaw, &mut self.sink, &mut self.intake, &mut self.hood, &mut self.glow, &mut self.fade] {
            moving |= a.tick(md);
        }
        if self.fade.is_settled() {
            self.before = None;
        }
        moving
    }

    /// Whether it turns by itself now: shown, idle, at rest from any move,
    /// and not held `still` (StandBy, the battery saver, the calm version).
    pub fn drifts(&self, still: bool) -> bool {
        !still && self.shown && self.pose == Some(Pose::Idle) && self.yaw.is_settled()
    }

    /// The turntable, on by `dt` seconds: placed, not sprung (it is not motion).
    pub fn drift(&mut self, dt: f32) {
        let yaw = (self.yaw.value() + DRIFT_SPEED * dt.max(0.0)).rem_euclid(360.0);
        self.yaw.jump(yaw);
    }

    pub fn shot(&self) -> Shot {
        // Turning fast, the rendered angles themselves.
        let yaw = if self.yaw.velocity().abs() > SNAP_ABOVE { (self.yaw.value() / STEP).round() * STEP } else { self.yaw.value() };
        Shot {
            pose: self.pose.unwrap_or(Pose::Idle),
            yaw,
            sink: self.sink.value(),
            intake: self.intake.value().max(0.0),
            hood: self.hood.value().max(0.0),
            glow: self.glow.value().clamp(0.0, 1.0),
            look: self.look,
            before: self.before.map(|b| (b, self.fade.value().clamp(0.0, 1.0))),
        }
    }
}

// ------------------------------------------------------------- placing ---

/// The robot's frame's middle, for its feet at `ground`, `scale` times its size.
pub fn centre_for(ground: f32, scale: f32) -> f32 {
    ground - (BOTTOM - FRAME * 0.5) * scale
}

/// Its highest point at any pose, standing on `ground`.
pub fn top_for(ground: f32, scale: f32) -> f32 {
    ground - (BOTTOM - TOP) * scale
}

/// Where it stands, at its widest and tallest, centred on `cx` on `ground`.
pub fn bounds(cx: f32, ground: f32, scale: f32) -> Rect {
    let top = top_for(ground, scale);
    Rect::new(cx - HALF_WIDTH * scale, top, 2.0 * HALF_WIDTH * scale, ground - top)
}

/// The time behind it: the cap height and baseline at which the robot on
/// `ground` stands in front of `COVER` of the digits, their top no higher
/// than `from`, and no larger than `widest` (what the width allows).
pub fn time_for(from: f32, ground: f32, scale: f32, widest: f32) -> (f32, f32) {
    let top = top_for(ground, scale);
    let cap = ((top - from) / (1.0 - COVER)).min(widest).max(12.0);
    (cap, (top + COVER * cap).floor())
}

/// The light under it on the charger, in the signal colour: drawn behind
/// the time, before it.
pub fn draw_light(g: &mut Gfx, shot: &Shot, cx: f32, ground: f32, scale: f32, alpha: f32) {
    if shot.glow <= 0.004 {
        return;
    }
    let t = g.tokens;
    let strength = if t.tone == bezel::Tone::Dark { 0.42 } else { 0.5 };
    g.canvas.glow(cx, ground - 8.0 * scale, 74.0 * scale, t.signal, strength * shot.glow * alpha);
}

/// Draw the robot as `shot` has it, centred on `cx`, its feet on `ground`.
pub fn draw(g: &mut Gfx, shot: &Shot, cx: f32, ground: f32, scale: f32, alpha: f32) {
    let mut pose = [0.0; MECHANISMS];
    for (m, (name, _)) in g.objects.mechanisms(OBJECT).iter().enumerate().take(MECHANISMS) {
        pose[m] = match name.as_str() {
            "intake" => shot.intake,
            "hood" => shot.hood,
            _ => 0.0,
        };
    }
    let mut cy = centre_for(ground + shot.sink, scale);
    // At rest on whole pixels, so a frame at its own size is laid straight down.
    if (shot.sink - shot.sink.round()).abs() < 0.05 {
        cy = cy.round();
    }
    let view = View::yaw(shot.yaw).posed(pose);
    match shot.before {
        Some((before, k)) if k < 0.999 => {
            g.object_looking(OBJECT, SIZE, cx, cy, view, scale, alpha, &before.looks());
            g.object_looking(OBJECT, SIZE, cx, cy, view, scale, alpha * k, &shot.look.looks());
        }
        _ => {
            g.object_looking(OBJECT, SIZE, cx, cy, view, scale, alpha, &shot.look.looks());
        }
    }
}

// ------------------------------------------------------------ the gallery ---

/// The lock screen and StandBy's robot clock in each pose, dark and light,
/// at each text size, as the simulator draws them, into `dir`; and what
/// drawing them costs (`budget.txt`).
#[cfg(not(target_arch = "aarch64"))]
pub fn shots(dir: &std::path::Path) {
    use crate::gfx::{TextSize, H, W};
    use crate::lock::{Choices, ClockStyle, Face, Info};
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let morning = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).and_then(|d| d.and_hms_opt(9, 41, 0)).expect("a time");
    crate::faces::pretend_time(Some(morning));
    let unix = chrono::Utc::now().timestamp();
    let robot = crate::robot::Robot::demo();
    let reading = |pose: Pose| match pose {
        Pose::Charging => Reading { flow: crate::power::Flow::Charging, minutes_to_full: Some(42), ..Reading::demo() },
        Pose::Low => Reading { percent: 9, flow: crate::power::Flow::Discharging, ..Reading::demo() },
        _ => Reading { flow: crate::power::Flow::Discharging, ..Reading::demo() },
    };
    // The demo's next match, shown with every pose (as the lock screen's chip).
    let next = crate::upnext::next_match(&robot, unix).ok();
    // Ready: the match's live activity too, counting down to its queue.
    let activity = crate::liveactivity::next_match(robot.schedule.view().as_ref(), unix);
    let info = |pose: Pose| Info {
        h24: false,
        reading: Some(reading(pose)),
        next: next.clone(),
        soon: pose == Pose::Ready,
        activities: activity.clone().filter(|_| pose == Pose::Ready).into_iter().collect(),
        ..Info::default()
    };
    let mut n = 0;
    for (tone, tokens) in [("dark", bezel::Tokens::dark()), ("light", bezel::Tokens::light())] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let mut g = Gfx::new(tokens);
            g.set_text_size(size);
            for pose in Pose::ALL {
                let choices = Choices::default();
                let mut info = info(pose);
                let paint = (pose == Pose::Ready).then(|| info.next.as_ref().filter(|m| m.side.is_some()).map(|m| m.color)).flatten();
                info.depth = Some(Shot::settled(pose, paint));
                crate::lock::draw_face(&mut g, &choices, &info, morning, false);
                let mut field = crate::lock::Field::new();
                field.draw(&mut g, W * 0.5, crate::lock::FIELD_Y, W - 2.0 * crate::gfx::PAD - 12.0, &crate::lock::Prompt::Asks("type your passcode".into()));
                crate::platform::sim::save_png(&g.canvas, &dir.join(format!("lock-{}-{tone}-{}.png", pose.name(), size.name())), 1);
                n += 1;
                // StandBy's robot clock, on the charger (or as the pose has it).
                let mut faces = crate::faces::Faces::new();
                let choices = Choices { face: Face::Clock, clock: ClockStyle::Robot, ..Choices::default() };
                faces.show(&choices, true);
                let r = reading(pose);
                let acts = info.activities.clone();
                let feed = crate::faces::Feed { reading: Some(&r), h24: false, notices: 0, generation: 0, activities: &acts, opened: None, open: 0.0, level: 0.0 };
                faces.pretend_soon(pose == Pose::Ready);
                // Past the moment the page dots show for, so they have tucked away.
                faces.tick(3.0, 3.0, &feed, &choices, true);
                for _ in 0..3 {
                    faces.tick(1.0 / 30.0, 1.0 / 30.0, &feed, &choices, true);
                    g.canvas.clear(tokens.ground);
                    faces.draw(&mut g, &robot, &feed, &choices);
                }
                crate::platform::sim::save_png(&g.canvas, &dir.join(format!("standby-{}-{tone}-{}.png", pose.name(), size.name())), 1);
                n += 1;
            }
        }
    }
    // Settings › Lock screen, at its robot's two rows, at each size and tone.
    for (tone, light) in [("dark", false), ("light", true)] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            let mut shell = crate::fullheight::shell("catalyst", size);
            shell.settings.light = light;
            shell.apply_settings();
            let (index, ..) = crate::screens::setting_sections().into_iter().find(|(_, n, _)| *n == "Lock screen").expect("Settings › Lock screen");
            shell.show(crate::screens::open_setting(index));
            crate::fullheight::settle(&mut shell, 90);
            for _ in 0..2 {
                shell.key(crate::platform::Key::Down);
                crate::fullheight::settle(&mut shell, 40);
            }
            shell.draw();
            crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("settings-lock-{tone}-{}.png", size.name())), 1);
            n += 1;
        }
    }
    // What it costs, as the lock screen itself draws it: a frame of the
    // turntable (the picture under the robot kept, the robot laid over it),
    // the whole screen drawn again (the minute turning, a key), and the same
    // screen without the robot; and the robot alone.
    let mut g = Gfx::new(bezel::Tokens::dark());
    let mut lock = crate::lock::Lock::new();
    lock.show(true);
    lock.hold_still(false);
    let mut scratch = Vec::new();
    let frames = 300;
    let dt = DRIFT_EVERY.as_secs_f32();
    let mut run = |g: &mut Gfx, depth: bool, whole: bool, lock: &mut crate::lock::Lock| {
        let choices = Choices { depth, ..Choices::default() };
        // Risen long since: the same picture, but never kept.
        let info = Info { reading: Some(reading(Pose::Idle)), rise: whole.then_some(10.0), ..Info::default() };
        lock.draw(g, &choices, &info, &mut scratch);
        let start = std::time::Instant::now();
        for _ in 0..frames {
            lock.tick(dt, dt, false, false);
            lock.draw(g, &choices, &info, &mut scratch);
        }
        start.elapsed().as_secs_f64() * 1e6 / frames as f64
    };
    // Once round first (each angle decoded as the turn reaches it, and
    // kept while it is near, as on the handheld).
    for _ in 0..3 {
        run(&mut g, true, false, &mut lock);
    }
    assert!(lock.drifting(), "the turntable runs");
    let decoded = g.objects.decoded;
    let with = run(&mut g, true, false, &mut lock);
    let whole = run(&mut g, true, true, &mut lock);
    let without = run(&mut g, false, true, &mut lock);
    let alone = {
        let mut r = Robot::new();
        r.aim(Pose::Idle, None, true);
        let start = std::time::Instant::now();
        for _ in 0..frames {
            r.drift(dt);
            draw(&mut g, &r.shot(), W * 0.5, H - 45.0, crate::lock::DEPTH_SCALE, 1.0);
        }
        start.elapsed().as_secs_f64() * 1e6 / frames as f64
    };
    let decode = {
        let mut cold = crate::object3d::Objects::shipped();
        let t = bezel::Tokens::dark();
        let mut c = bezel::Canvas::new(W as i32, H as i32);
        let start = std::time::Instant::now();
        for k in 0..30 {
            cold.draw_looking(&mut c, &t, Color::hex(0xff6b5e), OBJECT, SIZE, 160.0, 85.0, View::yaw(k as f32 * STEP), 1.0, 1.0, &Looks::plain());
        }
        start.elapsed().as_secs_f64() * 1e6 / 30.0
    };
    let k = 20.0;
    let budget = format!(
        "the robot depth lock screen, drawn on this PC (the handheld estimated at {k}x, README's factor):\n\
         a frame of the turntable (the picture under the robot kept; two angles blended, scaled to {scale}): {with:.0} µs ({:.1} ms on the handheld)\n\
         the whole lock screen drawn again with the robot (the minute turning, a key): {whole:.0} µs ({:.1} ms)\n\
         the same screen with the depth effect off: {without:.0} µs ({:.1} ms)\n\
         the robot alone, a frame of the turntable: {alone:.0} µs ({:.1} ms)\n\
         a new angle decoded and coloured (every {STEP}° of the turn: once in {:.1} s at {DRIFT_SPEED}°/s): {decode:.0} µs ({:.1} ms)\n\
         frames: {:.1} a second while it turns ({} ms apart: ambient, not motion); none while it stands still\n\
         angles decoded over the first turns: {decoded}; the cache of coloured frames stays within {} kB\n",
        with * k / 1000.0,
        whole * k / 1000.0,
        without * k / 1000.0,
        alone * k / 1000.0,
        STEP / DRIFT_SPEED,
        decode * k / 1000.0,
        1000.0 / DRIFT_EVERY.as_millis() as f64,
        DRIFT_EVERY.as_millis(),
        crate::object3d::CACHE_BYTES / 1024,
        scale = crate::lock::DEPTH_SCALE,
    );
    std::fs::write(dir.join("budget.txt"), &budget).ok();
    print!("{budget}");
    println!("{n} pictures in {}", dir.display());
    crate::faces::pretend_time(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::Flow;

    fn reading(percent: u32, flow: Flow) -> Reading {
        Reading { percent, flow, ..Reading::default() }
    }

    #[test]
    fn the_pose_follows_the_charge_and_the_next_match() {
        let on_battery = reading(64, Flow::Discharging);
        let low = reading(LOW_PERCENT, Flow::Discharging);
        let charging = reading(9, Flow::Charging);
        assert_eq!(choose(true, Some(&on_battery), false), Pose::Idle);
        assert_eq!(choose(true, None, false), Pose::Idle, "no gauge: idle");
        assert_eq!(choose(true, Some(&reading(80, Flow::Unknown)), false), Pose::Idle, "a charge it cannot say is not the charger");
        assert_eq!(choose(true, Some(&charging), false), Pose::Charging, "low but charging: docked");
        assert_eq!(choose(true, Some(&reading(100, Flow::Full)), false), Pose::Charging);
        assert_eq!(choose(true, Some(&reading(80, Flow::Holding)), false), Pose::Charging);
        assert_eq!(choose(true, Some(&low), false), Pose::Low);
        assert_eq!(choose(true, Some(&reading(LOW_PERCENT + 1, Flow::Discharging)), false), Pose::Idle);
        assert_eq!(choose(true, Some(&on_battery), true), Pose::Ready);
        assert_eq!(choose(true, Some(&charging), true), Pose::Ready, "a match outranks the charger");
        assert_eq!(choose(true, Some(&low), true), Pose::Low, "running out outranks a match");
        // Not following: idle whatever happens.
        for r in [&on_battery, &low, &charging] {
            for soon in [false, true] {
                assert_eq!(choose(false, Some(r), soon), Pose::Idle);
            }
        }
    }

    #[test]
    fn a_match_is_soon_within_twenty_minutes_or_its_queue_within_ten() {
        let now = 1_000_000;
        let view = |m: crate::schedule::Match| crate::schedule::View { team: 5805, matches: vec![m], ..Default::default() };
        let at = |t: i64| crate::schedule::Match { key: "q1".into(), label: "Q1".into(), red: vec![5805], time: Some(t), ..Default::default() };
        assert!(soon_in(&view(at(now + SOON)), now));
        assert!(!soon_in(&view(at(now + SOON + 60)), now));
        assert!(soon_in(&view(at(now - 300)), now), "being played");
        assert!(soon_in(&view(crate::schedule::Match { queue_at: Some(now + 300), ..at(now + 3600) }), now));
        assert!(soon_in(&view(crate::schedule::Match { status: Some("On deck".into()), ..at(now + 3600) }), now));
        assert!(!soon_in(&view(crate::schedule::Match { score: Some((80, 60)), ..at(now + 60) }), now), "played");
        assert!(!soon_in(&crate::schedule::View::default(), now));
    }

    #[test]
    fn each_pose_has_its_camera_and_the_ready_one_faces_the_viewer() {
        let o = crate::object3d::Objects::shipped();
        let intake = o.parts(OBJECT).into_iter().find(|(n, _)| n == "intake").map(|(_, f)| f as f32);
        assert_eq!(intake, Some(FACING_YAW), "ready faces the intake to the camera");
        let mech = o.mechanisms(OBJECT);
        let steps = |name: &str| mech.iter().find(|(n, _)| n == name).map(|(_, s)| s.len() as f32 - 1.0).unwrap();
        let ready = Pose::Ready.camera();
        assert_eq!((ready.intake, ready.hood), (steps("intake"), steps("hood")), "intake out, hood at its highest");
        let low = Pose::Low.camera();
        assert!(low.sink > 0.0 && low.dim > 0.0 && low.hood == 0.0, "slumped: sunk, dimmed, hood down");
        let docked = Pose::Charging.camera();
        assert!(docked.glow > 0.0 && docked.intake == 0.0 && docked.hood == 0.0, "docked: stowed, the charge light on");
        assert_eq!(Pose::Idle.camera().yaw, None, "idle turns on from wherever it is");
    }

    #[test]
    fn poses_ease_on_springs_and_settle_and_the_calm_version_jumps() {
        let mut r = Robot::new();
        assert!(!r.aim(Pose::Idle, None, false), "the first placing is at once");
        r.shown = true;
        assert!(r.drifts(false));
        r.drift(10.0 * DRIFT_EVERY.as_secs_f32());
        assert!((r.shot().yaw - DRIFT_SPEED * 0.8).abs() < 1e-3, "{}", r.shot().yaw);
        assert!(!r.tick(1.0 / 60.0), "the turntable is not motion");
        // To the charger: it turns the nearest way round, and settles.
        assert!(r.aim(Pose::Charging, None, false));
        assert!(!r.drifts(false), "not while it moves");
        let mut frames = 0;
        while r.tick(1.0 / 60.0) {
            frames += 1;
            let s = r.shot();
            assert!(s.yaw > DOCKED_YAW - 360.0 - 1.0 && s.yaw < 5.0, "nearest way round: {}", s.yaw);
            assert!(frames < 120, "settles within two seconds");
        }
        let s = r.shot();
        assert!((s.yaw.rem_euclid(360.0) - DOCKED_YAW).abs() < 0.1 && (s.glow - 1.0).abs() < 0.01);
        assert!(!r.drifts(false), "docked, it stands still");
        // Low: the dimmed look crossfades in.
        r.aim(Pose::Low, None, false);
        r.tick(1.0 / 60.0);
        assert!(r.shot().before.is_some(), "crossfading");
        while r.tick(1.0 / 60.0) {}
        assert_eq!(r.shot().before, None);
        assert_eq!(r.shot().look.dim, LOW_DIM);
        // Calm: at once.
        assert!(!r.aim(Pose::Ready, Some(Color::hex(0xd93a34)), true));
        assert!(!r.tick(1.0 / 60.0));
        assert_eq!(r.shot(), Shot { before: None, ..Shot::settled(Pose::Ready, Some(Color::hex(0xd93a34))).turned(r.shot().yaw) });
        assert!((r.shot().yaw.rem_euclid(360.0) - FACING_YAW).abs() < 0.01);
        // Held still (StandBy, the saver, calm): idle does not turn.
        r.aim(Pose::Idle, None, true);
        assert!(r.drifts(false) && !r.drifts(true));
    }

    #[test]
    fn turning_fast_it_shows_the_rendered_angles_themselves() {
        let mut r = Robot::new();
        r.aim(Pose::Idle, None, true);
        // Round to the back: a long way to turn to face the viewer.
        r.drift(200.0 / DRIFT_SPEED);
        assert!(r.aim(Pose::Ready, None, false));
        let mut snapped = 0;
        while r.tick(1.0 / 30.0) {
            let y = r.shot().yaw;
            if (y / STEP - (y / STEP).round()).abs() < 1e-4 {
                snapped += 1;
            }
        }
        assert!(snapped > 0, "at speed, on rendered angles");
    }

    /// Numbers' extent at every rendered angle, its mechanisms stowed or
    /// out (as it turns between poses they move as it turns), stays within
    /// TOP, BOTTOM and HALF_WIDTH: so the layouts built on them keep the
    /// robot off the top of the time.
    #[test]
    fn the_robot_stands_within_its_measured_extent_at_every_angle() {
        let mut o = crate::object3d::Objects::shipped();
        let t = bezel::Tokens::dark();
        for k in 0..90 {
            let yaw = k as f32 * STEP;
            for pose in [[0.0, 0.0], [1.0, 0.0], [0.0, 2.0], [1.0, 2.0]] {
                let view = View::yaw(yaw).posed([pose[0], pose[1], 0.0, 0.0]);
                let (l, top, r, bottom) = o.extent(&t, Color::hex(0xff6b5e), OBJECT, SIZE, view).unwrap();
                assert!(top as f32 >= TOP && bottom as f32 <= BOTTOM, "{pose:?} at {yaw}°: rows {top} to {bottom}");
                assert!(FRAME * 0.5 - l as f32 <= HALF_WIDTH && r as f32 + 1.0 - FRAME * 0.5 <= HALF_WIDTH, "{pose:?} at {yaw}°: columns {l} to {r}");
            }
        }
    }
}
