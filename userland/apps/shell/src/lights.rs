//! The lights: the keyboard's Shift, Fn and Sym lights, and the red light.
//!
//! The keyboard's three lights hang off the M5IOE1, the board's I/O chip,
//! which blinks them by itself: one register each, 0 off, 1 a slow blink,
//! 2 a fast one, 3 on. The keyboard driver writes them to show a modifier's
//! state (on while held, a slow blink for one key, fast while locked). The
//! shell borrows them for two things, and gives them back:
//!
//! - a **notice**: something happened while the screen was off (a robot
//!   linked, an update is ready). Fn blinks slowly until the screen wakes.
//! - a **warning**: the battery reached a warning level, a match timer
//!   moved to its next phase, it is time to queue. All three blink fast for
//!   a few seconds, then stop: a warning, not a light left on.
//!
//! What a light showed before is read first and put back after, unless the
//! keyboard changed it meanwhile (a modifier pressed), which then stands.
//!
//! The red light follows the CM0's activity LED, which flickers on every
//! card access. Quiet, it stays dark, and blinks only if the kernel panics.

use std::cell::Cell;

/// How long a warning blinks.
pub const WARNING_SECONDS: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Light {
    Shift,
    Fn,
    Sym,
}

impl Light {
    const ALL: [Light; 3] = [Light::Shift, Light::Fn, Light::Sym];

    /// Its mode register on the M5IOE1 (the keyboard driver's capslock,
    /// fn and tables-sel lights).
    fn register(self) -> u8 {
        match self {
            Light::Shift => 0xBD,
            Light::Fn => 0xBE,
            Light::Sym => 0xBF,
        }
    }
}

/// What the shell has a light do (off is the keyboard's own state, given back).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Blink = 1,
    Fast = 2,
}

thread_local! {
    /// A warning asked for by a screen (the pit timer) since the last tick.
    static CUED: Cell<bool> = const { Cell::new(false) };
}

/// Ask for a warning blink, from anywhere in the shell.
pub fn warn() {
    CUED.with(|c| c.set(true));
}

/// Where the lights are written: the board's I/O chip, or nowhere (a
/// computer, the emulator), with what was written kept for the tests.
pub trait Board {
    fn read(&mut self, register: u8) -> Option<u8>;
    fn write(&mut self, register: u8, value: u8) -> bool;
}

/// The M5IOE1 on I2C bus 1, at 0x4F. Only on a CardputerZero: its driver
/// makes `/proc/cardputerzero_hw_id` when it finds the chip.
#[cfg(target_os = "linux")]
pub struct Ioe(Option<std::fs::File>);

#[cfg(target_os = "linux")]
impl Ioe {
    pub fn open() -> Ioe {
        use std::os::fd::AsRawFd;
        const I2C_SLAVE_FORCE: libc::c_ulong = 0x0706;
        if !std::path::Path::new("/proc/cardputerzero_hw_id").exists() {
            return Ioe(None);
        }
        let file = std::fs::OpenOptions::new().read(true).write(true).open("/dev/i2c-1").ok();
        // Forced: the chip's own driver holds the address. The bus serialises
        // the two, and only these three registers are touched.
        Ioe(file.filter(|f| unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE_FORCE as _, 0x4F as libc::c_ulong) } >= 0))
    }
}

#[cfg(target_os = "linux")]
impl Board for Ioe {
    fn read(&mut self, register: u8) -> Option<u8> {
        use std::io::{Read, Write};
        let f = self.0.as_mut()?;
        f.write_all(&[register]).ok()?;
        let mut b = [0u8];
        f.read_exact(&mut b).ok()?;
        Some(b[0])
    }

    fn write(&mut self, register: u8, value: u8) -> bool {
        use std::io::Write;
        self.0.as_mut().is_some_and(|f| f.write_all(&[register, value]).is_ok())
    }
}

/// No lights to write.
pub struct NoBoard;

impl Board for NoBoard {
    fn read(&mut self, _register: u8) -> Option<u8> {
        None
    }
    fn write(&mut self, _register: u8, _value: u8) -> bool {
        false
    }
}

pub struct Lights {
    board: Box<dyn Board>,
    /// For each light: what it showed before the shell borrowed it, and
    /// what the shell wrote.
    borrowed: [Option<(u8, u8)>; 3],
    /// The notice blink is on.
    notice: bool,
    /// Seconds of warning blink left.
    warning: f32,
}

impl Lights {
    pub fn new() -> Lights {
        #[cfg(target_os = "linux")]
        let board: Box<dyn Board> = Box::new(Ioe::open());
        #[cfg(not(target_os = "linux"))]
        let board: Box<dyn Board> = Box::new(NoBoard);
        Lights::on(board)
    }

    pub fn on(board: Box<dyn Board>) -> Lights {
        Lights { board, borrowed: [None; 3], notice: false, warning: 0.0 }
    }

    /// Something happened while the screen was off.
    pub fn notice(&mut self) {
        if !self.notice {
            self.notice = true;
            self.show();
        }
    }

    /// A warning: all three blink fast for a few seconds.
    pub fn warning(&mut self) {
        self.warning = WARNING_SECONDS;
        self.show();
    }

    /// The screen woke: the notice has been seen. (A warning runs its few
    /// seconds either way.)
    pub fn seen(&mut self) {
        if self.notice {
            self.notice = false;
            self.show();
        }
    }

    /// Time passing; a warning asked for elsewhere starts here.
    pub fn tick(&mut self, dt: f32) {
        if CUED.with(|c| c.replace(false)) {
            self.warning();
        }
        if self.warning > 0.0 {
            self.warning -= dt;
            if self.warning <= 0.0 {
                self.warning = 0.0;
                self.show();
            }
        }
    }

    /// What each light should show now, and whether the shell has it.
    fn wanted(&self, light: Light) -> Option<Mode> {
        if self.warning > 0.0 {
            Some(Mode::Fast)
        } else if self.notice && light == Light::Fn {
            Some(Mode::Blink)
        } else {
            None
        }
    }

    fn show(&mut self) {
        for (i, light) in Light::ALL.into_iter().enumerate() {
            let reg = light.register();
            match (self.wanted(light), self.borrowed[i]) {
                (Some(mode), Some((before, wrote))) if wrote != mode as u8 => {
                    if self.board.write(reg, mode as u8) {
                        self.borrowed[i] = Some((before, mode as u8));
                    }
                }
                (Some(_), Some(_)) => {}
                (Some(mode), None) => {
                    let before = self.board.read(reg).unwrap_or(0);
                    if self.board.write(reg, mode as u8) {
                        self.borrowed[i] = Some((before, mode as u8));
                    }
                }
                // Given back: as it was, unless the keyboard has since set it.
                (None, Some((before, wrote))) => {
                    if self.board.read(reg).is_none_or(|now| now == wrote) {
                        self.board.write(reg, before);
                    }
                    self.borrowed[i] = None;
                }
                (None, None) => {}
            }
        }
    }

    /// Lit by the shell now, for the tests and the gallery.
    #[cfg(test)]
    pub fn showing(&self) -> [Option<Mode>; 3] {
        Light::ALL.map(|l| self.wanted(l))
    }
}

/// The red light: quiet (dark, blinking only on a kernel panic) or showing
/// card activity as the CM0 does by default.
pub fn red_light(quiet: bool) {
    let Some(dir) = ["ACT", "led0"].iter().map(|n| std::path::Path::new("/sys/class/leds").join(n)).find(|p| p.exists()) else { return };
    if quiet {
        std::fs::write(dir.join("trigger"), "panic").or_else(|_| std::fs::write(dir.join("trigger"), "none")).ok();
        std::fs::write(dir.join("brightness"), "0").ok();
    } else {
        std::fs::write(dir.join("trigger"), "mmc0").ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::cell::RefCell;

    /// The I/O chip's three registers, shared so the test can play the
    /// keyboard driver writing them too.
    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<HashMap<u8, u8>>>);

    impl Board for Fake {
        fn read(&mut self, register: u8) -> Option<u8> {
            Some(*self.0.borrow().get(&register).unwrap_or(&0))
        }
        fn write(&mut self, register: u8, value: u8) -> bool {
            self.0.borrow_mut().insert(register, value);
            true
        }
    }

    #[test]
    fn a_notice_blinks_fn_until_the_screen_wakes() {
        let chip = Fake::default();
        let mut lights = Lights::on(Box::new(chip.clone()));
        lights.notice();
        assert_eq!(chip.0.borrow()[&0xBE], Mode::Blink as u8);
        assert!(!chip.0.borrow().contains_key(&0xBD), "shift is left alone");
        lights.tick(60.0);
        assert_eq!(lights.showing(), [None, Some(Mode::Blink), None], "a notice waits to be seen");
        lights.seen();
        assert_eq!(chip.0.borrow()[&0xBE], 0, "given back as it was");
    }

    #[test]
    fn a_warning_blinks_fast_for_a_few_seconds_then_stops() {
        let chip = Fake::default();
        // Sym locked by the keyboard before the warning.
        chip.0.borrow_mut().insert(0xBF, 2);
        let mut lights = Lights::on(Box::new(chip.clone()));
        warn();
        lights.tick(0.1);
        assert_eq!(lights.showing(), [Some(Mode::Fast); 3]);
        for reg in [0xBD, 0xBE, 0xBF] {
            assert_eq!(chip.0.borrow()[&reg], Mode::Fast as u8);
        }
        lights.tick(WARNING_SECONDS);
        assert_eq!(lights.showing(), [None; 3], "a warning, not a light left on");
        assert_eq!(chip.0.borrow()[&0xBF], 2, "Sym's lock shows again");
        assert_eq!(chip.0.borrow()[&0xBD], 0);
    }

    #[test]
    fn what_the_keyboard_set_meanwhile_stands() {
        let chip = Fake::default();
        let mut lights = Lights::on(Box::new(chip.clone()));
        lights.notice();
        // The waking key was Fn, held: the driver lit it.
        chip.0.borrow_mut().insert(0xBE, 3);
        lights.seen();
        assert_eq!(chip.0.borrow()[&0xBE], 3);
    }

    #[test]
    fn with_no_board_nothing_breaks() {
        let mut lights = Lights::on(Box::new(NoBoard));
        lights.notice();
        lights.warning();
        lights.tick(1.0);
        lights.seen();
        assert_eq!(lights.showing(), [Some(Mode::Fast); 3], "a warning runs its few seconds, seen or not");
        lights.tick(WARNING_SECONDS);
        assert_eq!(lights.showing(), [None; 3]);
    }
}
