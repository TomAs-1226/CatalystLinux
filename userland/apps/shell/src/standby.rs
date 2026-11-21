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

use detent::{role, Animated};

/// How the processor chooses its speed: the kernel's governors, the first
/// of each list it has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Power {
    /// Fast when there is work, slow when not.
    Balanced,
    /// Climbs to speed slowly, and never far: the battery lasts longer.
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
/// Woken by a lift, how long it stays on with no key: a look at it.
const GLANCE: u32 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Awake,
    Dim,
    Off,
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
        }
    }

    pub fn state(&self) -> State {
        self.state
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
        self.state = State::Awake;
        if self.dark {
            if self.backlight {
                crate::sys::backlight_power(true);
            }
            self.dark = false;
        }
        if was_off {
            self.apply_power(self.power.governors());
        }
        // Back at once from off; from dimmed, a quick rise.
        if was_off {
            self.level.jump(1.0);
        } else {
            self.level.set(1.0);
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
        if self.state != State::Off {
            self.apply_power(power.governors());
        }
    }

    fn apply_power(&mut self, governors: &'static [&'static str]) {
        if self.power_set != Some(governors[0]) && crate::sys::set_governor(governors) {
            self.power_set = Some(governors[0]);
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
        match self.state {
            State::Awake if screen_off > 0 && self.idle >= dim_at => {
                self.state = State::Dim;
                self.base = if self.backlight { crate::sys::brightness() } else { None };
                self.level.set_with(DIM_LEVEL, role::smooth());
            }
            State::Dim if self.idle >= off_at => {
                self.state = State::Off;
                self.level.set_with(0.0, role::smooth());
            }
            // Something began to need watching while it was dimmed (but
            // not once it was laid face down).
            State::Dim | State::Off if self.idle < dim_at && !self.down => self.wake_now(),
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
