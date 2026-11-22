//! Standby and power saving. After a while with no key the screen dims, and
//! a little later goes off: the backlight fades down and then off, and
//! nothing more is drawn or sent to the panel. Any key wakes it, and is spent
//! doing so (it does not also move or open something). While the screen is
//! off the processor runs at its slowest; awake, at the speed chosen in
//! Settings › Display › Processor.
//!
//! A screen with something to watch (a program running in a terminal, a
//! match timer) keeps the screen awake while it shows: `Screen::keeps_awake`.
//! On a computer, with no backlight to dim, the picture itself dims instead.
//!
//! Where there is a motion sensor, picking the handheld up wakes the screen
//! for a look (it goes off again soon unless a key follows), and putting it
//! face down puts the screen out at once, whatever is showing, until a key
//! or a lift.
//!
//! On the charger, with StandBy on (faces.rs), the screen does not go off:
//! where it would, it goes to StandBy's faces instead (`State::Faces`), lit
//! at the level the shell asks for (lower at night), with the processor at
//! its saving pace. Taken off the charger there, it goes off as it would
//! have, and the wait for deep standby runs from then; a key wakes it as
//! from off. The screen is never dark while the faces show, so deep standby
//! (which waits for a dark screen) cannot begin on the charger.

use detent::{role, Animated};

/// How the processor chooses its speed: the kernel's governors, the first
/// of each list it has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Power {
    /// Fast when there is work, slow when not.
    Balanced,
    /// Climbs to speed slowly, and never far, and never for work at nice
    /// above 0 (the background): the battery lasts longer.
    Saving,
    /// Always at full speed.
    Performance,
}

impl Power {
    pub const ALL: [Power; 3] = [Power::Balanced, Power::Saving, Power::Performance];

    pub fn name(self) -> &'static str {
        match self {
            Power::Balanced => "balanced",
            Power::Saving => "power saving",
            Power::Performance => "performance",
        }
    }

    /// The name settings are saved by.
    pub fn key(self) -> &'static str {
        match self {
            Power::Balanced => "balanced",
            Power::Saving => "saving",
            Power::Performance => "performance",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn step(self, step: i32) -> Self {
        let at = Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as i32;
        Self::ALL[(at + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    fn governors(self) -> &'static [&'static str] {
        match self {
            Power::Balanced => &["schedutil", "ondemand", "conservative"],
            Power::Saving => &["conservative", "powersave"],
            Power::Performance => &["performance"],
        }
    }
}

/// How long with no key before the screen goes off: the choices, in
/// seconds (0: never).
pub const SCREEN_OFF_CHOICES: [u32; 5] = [30, 60, 120, 300, 0];

/// The choice as Settings shows it.
pub fn screen_off_name(seconds: u32) -> String {
    match seconds {
        0 => "never".into(),
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

/// How long before the screen goes off it dims (or half the wait, if that
/// is shorter).
const DIM_BEFORE: f32 = 8.0;
/// How bright it is while dimmed, as a share of its brightness.
const DIM_LEVEL: f32 = 0.3;
/// With the screen off and dark, how long the shell waits between looks
/// (a key ends the wait at once, and so does a lift: the motion sensor's
/// thread wakes the main loop). Two seconds: a Clock timer or a reminder is
/// said at most that late, and the shell wakes half a time a second where
/// it woke four.
pub const DARK_LOOK: std::time::Duration = std::time::Duration::from_secs(2);
/// Woken by a lift, how long it stays on with no key: a look at it.
const GLANCE: u32 = 10;

/// StandBy shows this long past where the screen would have gone off, then
/// it goes dark as it would off the charger; a key or a lift brings it back.
/// My own tuning: long enough to glance at from the bed, short enough that
/// a handheld left on a computer's USB all day does not stay lit.
const FACES_FOR: f32 = 300.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Awake,
    Dim,
    Off,
    /// On the charger, where it would have gone off: StandBy's faces show.
    Faces,
}

pub struct Standby {
    /// Seconds since the last key (or since something kept it awake).
    idle: f32,
    state: State,
    /// How lit the screen is: 1 awake, `DIM_LEVEL` dimmed, 0 off; gliding.
    level: Animated,
    /// Whether there is a backlight to dim (the handheld), or the picture
    /// dims instead (a computer).
    backlight: bool,
    /// The backlight's brightness before it was dimmed, to come back to,
    /// and the last brightness written while gliding.
    base: Option<u32>,
    written: Option<u32>,
    /// The backlight switched off, while off.
    dark: bool,
    /// The processor speed chosen, and the one last set.
    power: Power,
    power_set: Option<&'static str>,
    /// Woken by a lift: it goes off again after `GLANCE` unless a key follows.
    glance: bool,
    /// Put out by being laid face down: it stays out, even with something to
    /// watch, until a key or a lift.
    down: bool,
    /// StandBy may show now (on the charger, switched on), at this level of
    /// light; none: the screen goes off as ever.
    faces: Option<f32>,
    /// The wait last ticked with, in seconds: leaving the faces, the time is
    /// counted from where the screen would have gone off.
    off_at: f32,
}

impl Standby {
    pub fn new() -> Self {
        Standby {
            idle: 0.0,
            state: State::Awake,
            level: Animated::new(1.0, role::smooth()).with_rest(0.004),
            backlight: crate::sys::brightness().is_some(),
            base: None,
            written: None,
            dark: false,
            power: Power::Balanced,
            power_set: None,
            glance: false,
            down: false,
            faces: None,
            off_at: 60.0,
        }
    }

    /// Whether StandBy may show where the screen would go off (on the
    /// charger, switched on), and how lit: asked by the shell every tick.
    pub fn set_faces(&mut self, level: Option<f32>) {
        self.faces = level.map(|l| l.clamp(0.05, 1.0));
    }

    /// Put on the charger with the screen dimmed or off: StandBy comes up
    /// at once (not face down: laid so, it stays out). True if it did.
    pub fn to_faces(&mut self) -> bool {
        let Some(level) = self.faces else { return false };
        if !matches!(self.state, State::Dim | State::Off) || self.down {
            return false;
        }
        if self.state == State::Awake || self.base.is_none() {
            self.base = if self.backlight { crate::sys::brightness().filter(|b| *b > 0) } else { None };
        }
        if self.dark {
            if self.backlight {
                crate::sys::backlight_power(true);
            }
            self.dark = false;
        }
        self.state = State::Faces;
        self.glance = false;
        // Set down on the charger: StandBy's while starts now.
        self.idle = self.off_at;
        self.apply_power(Power::Saving.governors());
        self.level.set_with(level, role::smooth());
        self.light();
        true
    }

    /// Leaving the faces other than by a key: from now on it is as if the
    /// screen had just gone off, so deep standby's wait starts now, not
    /// when the last key was pressed (hours ago, on the charger overnight).
    fn leave_faces(&mut self) {
        if self.state == State::Faces {
            self.idle = self.idle.min(self.off_at);
        }
    }

    /// Off the charger: deep standby's wait starts now, as on leaving StandBy.
    pub fn unplugged(&mut self) {
        if matches!(self.state, State::Faces | State::Off) {
            self.idle = self.idle.min(self.off_at);
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Seconds since the last key, or since something last needed the
    /// screen: deep standby counts its wait from here.
    pub fn idle(&self) -> f32 {
        self.idle
    }

    /// Time that passed without ticks (music standby looks in once a
    /// minute and does not tick standby): still time with no key, so deep
    /// standby's wait counts it.
    pub fn add_idle(&mut self, seconds: f32) {
        self.idle += seconds.max(0.0);
    }

    /// A key came. True if it was spent waking the screen.
    pub fn key(&mut self) -> bool {
        self.idle = 0.0;
        // Someone is using it: no longer just a look.
        self.glance = false;
        if self.state == State::Awake {
            return false;
        }
        self.wake_now();
        true
    }

    /// Awake at once: something (a key, a robot plugged in) wants the screen.
    pub fn wake_now(&mut self) {
        self.idle = 0.0;
        self.glance = false;
        self.down = false;
        if self.state == State::Awake {
            return;
        }
        let was_off = self.state == State::Off;
        let was_faces = self.state == State::Faces;
        self.state = State::Awake;
        if self.dark {
            if self.backlight {
                crate::sys::backlight_power(true);
            }
            self.dark = false;
        }
        if was_off || was_faces {
            self.apply_power(self.power.governors());
        }
        // Back at once from off; from dimmed (or StandBy's night), a quick rise.
        if was_off {
            self.level.jump(1.0);
        } else {
            self.level.set_with(1.0, role::smooth());
        }
        self.light();
    }

    /// Picked up while the screen was off: on for a look.
    pub fn lifted(&mut self) {
        self.wake_now();
        self.glance = true;
    }

    /// Laid face down: the screen glides out now.
    pub fn put_down(&mut self) {
        if self.state == State::Awake {
            self.base = if self.backlight { crate::sys::brightness() } else { None };
        }
        self.leave_faces();
        self.down = true;
        self.glance = false;
        if self.state != State::Off {
            self.state = State::Off;
            self.level.set_with(0.0, role::smooth());
        }
    }

    /// The first picture is on the panel: the backlight comes up to
    /// `percent` from dark, on the smooth spring (at once in the calm
    /// version). A shell started again over a lit panel leaves it as it is.
    pub fn rise_from_dark(&mut self, percent: u32, calm: bool) {
        if !self.backlight || crate::sys::brightness().is_some_and(|now| now > 0) {
            return;
        }
        crate::sys::backlight_power(true);
        self.base = Some(percent.clamp(10, 100));
        self.written = None;
        self.level.jump(0.0);
        if calm {
            self.level.jump(1.0);
        } else {
            self.level.set_with(1.0, role::smooth());
        }
        self.light();
    }

    /// The processor speed chosen in Settings: set now unless the screen is off.
    pub fn set_power(&mut self, power: Power) {
        self.power = power;
        if !matches!(self.state, State::Off | State::Faces) {
            self.apply_power(power.governors());
        }
    }

    fn apply_power(&mut self, governors: &'static [&'static str]) {
        if self.power_set != Some(governors[0]) && crate::sys::set_governor(governors) {
            self.power_set = Some(governors[0]);
            // Its tunables are the kernel's defaults again: set ours.
            crate::sched::tune_governor();
        }
    }

    /// Time passing: `screen_off` is the wait in seconds (0: never), and
    /// `keep_awake` says something on screen needs watching. True while the
    /// light is changing.
    pub fn tick(&mut self, dt: f32, screen_off: u32, keep_awake: bool, calm: bool) -> bool {
        // Only a look: off again soon (sooner than the wait chosen).
        let screen_off = if self.glance && (screen_off == 0 || screen_off > GLANCE) { GLANCE } else { screen_off };
        if (keep_awake && !self.down) || screen_off == 0 {
            self.idle = 0.0;
        } else {
            self.idle += dt;
        }
        let off_at = screen_off as f32;
        let dim_at = (off_at - DIM_BEFORE).max(off_at * 0.5);
        if screen_off > 0 {
            self.off_at = off_at;
        }
        match self.state {
            State::Awake if screen_off > 0 && self.idle >= dim_at => {
                self.state = State::Dim;
                self.base = if self.backlight { crate::sys::brightness() } else { None };
                self.level.set_with(DIM_LEVEL, role::smooth());
            }
            // On the charger, StandBy: the faces come up where the screen
            // would have gone off, the processor at its saving pace.
            State::Dim if self.idle >= off_at && self.faces.is_some() => {
                self.state = State::Faces;
                self.level.set_with(self.faces.unwrap_or(1.0), role::smooth());
                self.apply_power(Power::Saving.governors());
            }
            State::Dim if self.idle >= off_at => {
                self.state = State::Off;
                self.level.set_with(0.0, role::smooth());
            }
            // Something began to need watching while it was dimmed (but
            // not once it was laid face down).
            State::Dim | State::Off | State::Faces if self.idle < dim_at && !self.down => self.wake_now(),
            // Off the charger (or StandBy switched off): out, as it would
            // have gone without the charger, and the wait for deep standby
            // starts now.
            State::Faces if self.faces.is_none() => {
                self.leave_faces();
                self.state = State::Off;
                self.level.set_with(0.0, role::smooth());
            }
            State::Faces if self.idle >= off_at + FACES_FOR => {
                self.state = State::Off;
                self.level.set_with(0.0, role::smooth());
            }
            // Night coming or going: the light glides to its new level, slowly.
            State::Faces => {
                let want = self.faces.unwrap_or(1.0);
                if (self.level.target() - want).abs() > 0.001 {
                    // My own tuning: three seconds, no bounce, as a room's
                    // light changes rather than a control's.
                    self.level.set_with(want, detent::Spring::duration_bounce(3.0, 0.0));
                }
            }
            _ => {}
        }
        if calm {
            self.level.jump(self.level.target());
        }
        let moving = self.level.tick(dt);
        self.light();
        if self.state == State::Off && self.level.value() <= 0.005 && !self.dark {
            // Off: the backlight goes out (where it cannot be switched off,
            // it stays at its least), and the processor slows to its least.
            self.dark = true;
            if self.backlight {
                crate::sys::backlight_power(false);
            }
            self.apply_power(&["powersave"]);
        }
        moving
    }

    /// The backlight follows the level as it glides.
    fn light(&mut self) {
        let Some(base) = self.base else { return };
        let level = self.level.value().clamp(0.0, 1.0);
        let want = ((base as f32 * level).round() as u32).max(1);
        if self.written != Some(want) && crate::sys::set_brightness(want) {
            self.written = Some(want);
        }
        // Back to full: the brightness is the user's again.
        if self.state == State::Awake && level >= 0.999 {
            crate::sys::set_brightness(base);
            self.base = None;
            self.written = None;
        }
    }

    /// How much the picture itself is darkened: only where there is no
    /// backlight to dim (a computer), 0 to 1.
    pub fn shade(&self) -> f32 {
        if self.backlight {
            0.0
        } else {
            1.0 - self.level.value().clamp(0.0, 1.0)
        }
    }

    /// Off, and dark: nothing need be drawn.
    pub fn asleep(&self) -> bool {
        self.state == State::Off && self.level.value() <= 0.005
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Standby, seconds: f32, screen_off: u32, keep: bool) {
        for _ in 0..(seconds * 20.0) as usize {
            s.tick(0.05, screen_off, keep, false);
        }
    }

    #[test]
    fn the_screen_dims_then_goes_off_and_a_key_wakes_it() {
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 50.0, 60, false);
        assert_eq!(s.state(), State::Awake);
        run(&mut s, 3.0, 60, false);
        assert_eq!(s.state(), State::Dim, "eight seconds before it goes off");
        assert!(s.shade() > 0.5 && s.shade() < 0.9, "dimmed, not dark: {}", s.shade());
        run(&mut s, 9.0, 60, false);
        assert_eq!(s.state(), State::Off);
        assert!(s.asleep() && s.shade() > 0.99);
        assert!(s.key(), "the first key is spent waking it");
        assert_eq!(s.state(), State::Awake);
        assert_eq!(s.shade(), 0.0, "back at once");
        assert!(!s.key(), "the next key is a key");
    }

    #[test]
    fn something_to_watch_or_never_keeps_it_awake() {
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 120.0, 60, true);
        assert_eq!(s.state(), State::Awake, "a program running in a terminal");
        run(&mut s, 600.0, 0, false);
        assert_eq!(s.state(), State::Awake, "never");
        // Dimmed when something needs watching again: it wakes by itself.
        run(&mut s, 55.0, 60, false);
        assert_eq!(s.state(), State::Dim);
        s.tick(0.05, 60, true, false);
        assert_eq!(s.state(), State::Awake);
    }

    #[test]
    fn a_lift_wakes_it_for_a_look_and_a_key_keeps_it() {
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 70.0, 60, false);
        assert_eq!(s.state(), State::Off);
        // Picked up: on at once, and off again ten seconds later with no key.
        s.lifted();
        assert_eq!((s.state(), s.shade()), (State::Awake, 0.0));
        run(&mut s, 4.0, 60, false);
        assert_eq!(s.state(), State::Awake);
        run(&mut s, 7.0, 60, false);
        assert_eq!(s.state(), State::Off, "a look, not a minute");
        // Picked up again and a key pressed: the minute chosen, as ever.
        s.lifted();
        assert!(!s.key(), "the key is a key: it was already awake");
        run(&mut s, 30.0, 60, false);
        assert_eq!(s.state(), State::Awake);
        // A look while something is to be watched lasts as long as that.
        run(&mut s, 40.0, 60, false);
        s.lifted();
        run(&mut s, 30.0, 60, true);
        assert_eq!(s.state(), State::Awake);
    }

    #[test]
    fn face_down_puts_it_out_even_with_something_to_watch() {
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 5.0, 60, true);
        s.put_down();
        run(&mut s, 2.0, 60, true);
        assert_eq!(s.state(), State::Off, "a match timer showing, laid face down: out");
        assert!(s.asleep());
        run(&mut s, 120.0, 60, true);
        assert_eq!(s.state(), State::Off, "and it stays out");
        // A key brings it back; then what is showing keeps it awake again.
        assert!(s.key());
        run(&mut s, 120.0, 60, true);
        assert_eq!(s.state(), State::Awake);
        // Laid face down, then picked up: a lift wakes it.
        s.put_down();
        run(&mut s, 2.0, 60, false);
        s.lifted();
        assert_eq!(s.state(), State::Awake);
    }

    #[test]
    fn on_the_charger_the_faces_come_where_it_would_go_off_and_a_key_wakes_it() {
        let mut s = Standby::new();
        s.backlight = false;
        s.set_faces(Some(1.0));
        run(&mut s, 53.0, 60, false);
        assert_eq!(s.state(), State::Dim, "it dims first, as ever");
        run(&mut s, 8.0, 60, false);
        assert_eq!(s.state(), State::Faces, "and then StandBy, not dark");
        assert!(!s.asleep(), "lit, not dark");
        run(&mut s, 60.0, 60, false);
        assert_eq!(s.state(), State::Faces, "for a while");
        assert!(s.shade() < 0.01, "lit");
        // Night: the light glides down, slowly.
        s.set_faces(Some(0.3));
        run(&mut s, 0.5, 60, false);
        assert!(s.shade() > 0.02 && s.shade() < 0.6, "on its way: {}", s.shade());
        run(&mut s, 6.0, 60, false);
        assert!((s.shade() - 0.7).abs() < 0.02, "night: {}", s.shade());
        // Then dark, as off the charger.
        run(&mut s, FACES_FOR, 60, false);
        assert_eq!(s.state(), State::Off);
        run(&mut s, 2.0, 60, false);
        assert!(s.asleep(), "dark");
        // A key wakes it, and is spent doing so.
        assert!(s.key());
        assert_eq!(s.state(), State::Awake);
        run(&mut s, 1.0, 60, false);
        assert!(s.shade() < 0.05);
    }

    #[test]
    fn off_the_charger_the_faces_go_out_and_deep_standby_counts_from_then() {
        let mut s = Standby::new();
        s.backlight = false;
        s.set_faces(Some(1.0));
        run(&mut s, 180.0, 60, false);
        assert_eq!(s.state(), State::Faces);
        assert!(s.idle() > 175.0, "{}", s.idle());
        // Unplugged: it goes off as it would have, and its idle time is as
        // if the screen had only now gone off.
        s.set_faces(None);
        run(&mut s, 2.0, 60, false);
        assert_eq!(s.state(), State::Off);
        assert!(s.asleep());
        assert!((61.0..63.0).contains(&s.idle()), "{}", s.idle());
        // Laid face down on the charger: out, and counted from then too.
        let mut s = Standby::new();
        s.backlight = false;
        s.set_faces(Some(1.0));
        run(&mut s, 200.0, 60, false);
        s.put_down();
        run(&mut s, 2.0, 60, false);
        assert_eq!(s.state(), State::Off);
        assert!(s.idle() < 63.0, "{}", s.idle());
        // StandBy switched off, or the screen never to go off: no faces.
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 70.0, 60, false);
        assert_eq!(s.state(), State::Off);
        s.set_faces(Some(1.0));
        let mut s = Standby::new();
        s.set_faces(Some(1.0));
        run(&mut s, 600.0, 0, false);
        assert_eq!(s.state(), State::Awake, "never: on, not StandBy");
    }

    #[test]
    fn a_short_wait_dims_eight_seconds_before() {
        let mut s = Standby::new();
        s.backlight = false;
        run(&mut s, 21.0, 30, false);
        assert_eq!(s.state(), State::Awake);
        run(&mut s, 2.0, 30, false);
        assert_eq!(s.state(), State::Dim);
        assert_eq!(screen_off_name(30), "30 s");
        assert_eq!(screen_off_name(300), "5 min");
        assert_eq!(screen_off_name(0), "never");
        assert_eq!(Power::from_key(Power::Saving.key()), Some(Power::Saving));
        assert_eq!(Power::Performance.step(1), Power::Balanced);
    }
}
