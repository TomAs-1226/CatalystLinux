//! The motion sensor: a Bosch BMI270 (accelerometer and gyroscope) on the
//! board's I2C bus at 0x68, and on most units a BMM150 magnetometer behind
//! it, on the BMI270's own auxiliary bus at 0x10. The CardputerZero C154 has
//! both; the C154-V1 has no magnetometer; the Lite has no motion sensor at
//! all. Whatever answers is what is offered: the Level app needs the BMI270,
//! the Compass the BMM150 too.
//!
//! The shell drives the chip itself through `/dev/i2c-1`, not through a
//! kernel driver: standby wants the chip's low-power mode, which the kernel's
//! driver does not offer, and nothing else on the handheld reads it. The
//! BMI270 needs an 8 KB program loaded at every power-up; it is Bosch's own
//! (`bmi270_config_file` from the BMI270 Sensor API, BSD-3-Clause, the same
//! bytes M5Stack's image loads), kept in `userland/assets/bmi270/`.
//!
//! Readings are in the screen's frame: x to the right, y towards the top
//! edge, z out of the screen. The accelerometer gives the way up (1 g along
//! +z lying face up), the gyroscope degrees a second about each axis, the
//! magnetometer microtesla. The chips sit turned on the board; M5Stack's own
//! Compass app gives how (the BMI270 is (y, −x, z) in the screen's frame, the
//! BMM150 (y, x, −z)), and the same turn is made here.
//!
//! Nothing runs until something asks: an open Level or Compass holds the
//! sensor at full rate ([`hold`]), and standby sets it watching at its
//! slowest ([`set_watch`]) for a lift while the screen is off and for being
//! put face down while it is on. Otherwise everything on the chip is off. On
//! a computer, a simulated sensor lies on a bench and is nudged
//! now and then, so every screen can be seen.

// Off the handheld the chip's driver is only exercised by the tests.
#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]

use std::collections::VecDeque;
use std::io;
use std::time::Duration;
#[cfg(target_arch = "aarch64")]
use std::time::Instant;

// ----------------------------------------------------------------- maths ---

/// A direction or a field in the screen's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn scale(self, k: f32) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }

    pub fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    pub fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    /// The same direction, one long; none for a vector too short to have one.
    pub fn unit(self) -> Option<Vec3> {
        let len = self.len();
        (len.is_finite() && len > 1e-6).then(|| self.scale(1.0 / len))
    }

    /// The angle between two directions, in degrees.
    pub fn angle_to(self, o: Vec3) -> f32 {
        match (self.unit(), o.unit()) {
            (Some(a), Some(b)) => a.dot(b).clamp(-1.0, 1.0).acos().to_degrees(),
            _ => 0.0,
        }
    }
}

/// How a handheld lying (near enough) flat is tipped, from the way up: the
/// pitch (its top edge raised, positive, or lowered) and the roll (its right
/// edge raised), in degrees, as a bubble level reads them.
pub fn tilt(up: Vec3) -> (f32, f32) {
    let pitch = up.y.atan2((up.x * up.x + up.z * up.z).sqrt()).to_degrees();
    let roll = up.x.atan2((up.y * up.y + up.z * up.z).sqrt()).to_degrees();
    (pitch, roll)
}

/// Standing on an edge (on its bottom edge, the screen facing you): how far
/// it is turned from upright, in degrees, positive turned clockwise (its
/// right end low). Up to 180 either way.
pub fn edge(up: Vec3) -> f32 {
    (-up.x).atan2(up.y).to_degrees()
}

/// The magnetic heading of the top edge, in degrees clockwise from north,
/// from the way up and the magnetic field, both in the screen's frame. The
/// field is laid level against the way up first, so the heading holds as the
/// handheld tips. None when the top edge points too nearly straight up or
/// down to have a heading, or the field is too weak to read.
pub fn heading(up: Vec3, field: Vec3) -> Option<f32> {
    let up = up.unit()?;
    let top = Vec3::new(0.0, 1.0, 0.0);
    let forward = top.sub(up.scale(top.dot(up)));
    // The top edge within about 3° of straight up or down: no heading.
    if forward.len() < 0.05 {
        return None;
    }
    let forward = forward.unit()?;
    let level = field.sub(up.scale(field.dot(up)));
    if level.len() < 1.0 {
        return None;
    }
    let north = level.unit()?;
    // To the right of the way the top edge points, on the ground: the same
    // whichever way up the handheld is.
    let right = forward.cross(up);
    let degrees = (-north.dot(right)).atan2(north.dot(forward)).to_degrees();
    Some(degrees.rem_euclid(360.0))
}

/// The heading as the compass card names it: N, NNE, NE and so on.
pub fn point(degrees: f32) -> &'static str {
    const POINTS: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    POINTS[((degrees.rem_euclid(360.0) / 22.5).round() as usize) % 16]
}

// ------------------------------------------------------------- the chips ---

/// Register access to one I2C device: the handheld's bus, or a test's stand-in.
pub trait Bus {
    fn read(&mut self, reg: u8, buf: &mut [u8]) -> io::Result<()>;
    fn write(&mut self, reg: u8, data: &[u8]) -> io::Result<()>;
}

/// The BMI270's I2C address.
pub const ADDRESS: u16 = 0x68;
const CHIP_ID: u8 = 0x24;
/// The BMM150's address on the BMI270's auxiliary bus, and its chip id.
const MAG_ADDRESS: u8 = 0x10;
const MAG_ID: u8 = 0x32;

/// The BMI270's registers used here (from its datasheet; the same numbers as
/// Bosch's Sensor API and the Linux driver).
mod reg {
    pub const CHIP_ID: u8 = 0x00;
    pub const STATUS: u8 = 0x03;
    pub const AUX_DATA: u8 = 0x04;
    pub const ACC_X: u8 = 0x0c;
    pub const INTERNAL_STATUS: u8 = 0x21;
    pub const ACC_CONF: u8 = 0x40;
    pub const ACC_RANGE: u8 = 0x41;
    pub const GYR_CONF: u8 = 0x42;
    pub const GYR_RANGE: u8 = 0x43;
    pub const AUX_CONF: u8 = 0x44;
    pub const AUX_DEV_ID: u8 = 0x4b;
    pub const AUX_IF_CONF: u8 = 0x4c;
    pub const AUX_RD_ADDR: u8 = 0x4d;
    pub const AUX_WR_ADDR: u8 = 0x4e;
    pub const AUX_WR_DATA: u8 = 0x4f;
    pub const INIT_CTRL: u8 = 0x59;
    pub const INIT_ADDR_0: u8 = 0x5b;
    pub const INIT_DATA: u8 = 0x5e;
    pub const IF_CONF: u8 = 0x6b;
    pub const PWR_CONF: u8 = 0x7c;
    pub const PWR_CTRL: u8 = 0x7d;
    pub const CMD: u8 = 0x7e;
}

/// The BMM150's registers, reached through the BMI270.
mod mag {
    pub const CHIP_ID: u8 = 0x40;
    pub const DATA: u8 = 0x42;
    pub const POWER: u8 = 0x4b;
    pub const OP_MODE: u8 = 0x4c;
    pub const REP_XY: u8 = 0x51;
    pub const REP_Z: u8 = 0x52;
    /// Its factory trim: 0x5d to 0x71.
    pub const TRIM: u8 = 0x5d;
}

/// Bosch's program for the BMI270, loaded at every power-up.
pub static CONFIG: &[u8; 8192] = include_bytes!("../../../assets/bmi270/bmi270-config.bin");

/// The program goes over in pieces this long (even: it is addressed in
/// words). Short, so a key pressed meanwhile is read without a wait: the
/// keyboard shares the bus.
const CHUNK: usize = 64;

/// Counts per g at ±2 g, and per degree a second at ±2000 °/s.
const ACC_PER_G: f32 = 16384.0;
const GYR_PER_DPS: f32 = 32768.0 / 2000.0;

/// What the sensor is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Everything off, in the chip's power saving.
    Off,
    /// The accelerometer alone, at its slowest and least: watching for a
    /// lift or for being put face down.
    Watch,
    /// Accelerometer and gyroscope at full rate: the Level.
    Motion,
    /// The same, with the magnetometer: the Compass.
    Compass,
}

/// The BMM150's factory trim, read once: what turns its counts into µT.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trim {
    x1: i8,
    y1: i8,
    x2: i8,
    y2: i8,
    z1: u16,
    z2: i16,
    z3: i16,
    z4: i16,
    xy1: u8,
    xy2: i8,
    xyz1: u16,
}

impl Trim {
    /// From the trim registers 0x5d to 0x71, in order.
    fn parse(r: &[u8]) -> Trim {
        let at = |reg: u8| r[(reg - mag::TRIM) as usize];
        let word = |reg: u8| u16::from_le_bytes([at(reg), at(reg + 1)]);
        Trim {
            x1: at(0x5d) as i8,
            y1: at(0x5e) as i8,
            z4: word(0x62) as i16,
            x2: at(0x64) as i8,
            y2: at(0x65) as i8,
            z2: word(0x68) as i16,
            z1: word(0x6a),
            xyz1: word(0x6c) & 0x7fff,
            z3: word(0x6e) as i16,
            xy2: at(0x70) as i8,
            xy1: at(0x71),
        }
    }

    /// The eight data bytes (0x42 to 0x49) as µT, in the BMM150's own axes,
    /// compensated as Bosch's Sensor API does it. None when an axis overflowed.
    fn field(&self, d: &[u8; 8]) -> Option<Vec3> {
        let x = ((d[1] as i8 as i16) * 32) | (d[0] >> 3) as i16;
        let y = ((d[3] as i8 as i16) * 32) | (d[2] >> 3) as i16;
        let z = ((d[5] as i8 as i16) * 128) | (d[4] >> 1) as i16;
        let hall = ((d[7] as u16) << 6) | (d[6] >> 2) as u16;
        if x == -4096 || y == -4096 || z == -16384 || hall == 0 || self.xyz1 == 0 || self.z1 == 0 || self.z2 == 0 {
            return None;
        }
        let xy = |raw: i16, k1: i8, k2: i8| {
            let r = self.xyz1 as f32 * 16384.0 / hall as f32 - 16384.0;
            let a = self.xy2 as f32 * (r * r / 268_435_456.0) + r * self.xy1 as f32 / 16384.0;
            ((raw as f32 * ((a + 256.0) * (k2 as f32 + 160.0))) / 8192.0 + k1 as f32 * 8.0) / 16.0
        };
        let zc = {
            let z0 = z as f32 - self.z4 as f32;
            let z2 = self.z3 as f32 * (hall as f32 - self.xyz1 as f32);
            let z4 = self.z2 as f32 + self.z1 as f32 * hall as f32 / 32768.0;
            (z0 * 131_072.0 - z2) / (z4 * 4.0) / 16.0
        };
        Some(Vec3::new(xy(x, self.x1, self.x2), xy(y, self.y1, self.y2), zc))
    }
}

/// The BMI270 (and the BMM150 behind it, if there is one), started.
pub struct Chip<B: Bus> {
    bus: B,
    trim: Option<Trim>,
    mode: Mode,
}

impl<B: Bus> Chip<B> {
    fn get(&mut self, r: u8) -> io::Result<u8> {
        let mut b = [0u8];
        self.bus.read(r, &mut b)?;
        Ok(b[0])
    }

    fn set(&mut self, r: u8, v: u8) -> io::Result<()> {
        self.bus.write(r, &[v])
    }

    /// The chip on `bus`, started: reset, its program loaded and checked,
    /// the magnetometer looked for, then all of it left asleep. `nap` waits
    /// (a test's does not).
    pub fn start(bus: B, nap: &mut dyn FnMut(Duration)) -> Result<Chip<B>, String> {
        let mut chip = Chip { bus, trim: None, mode: Mode::Off };
        let gone = |e: io::Error| format!("the motion sensor stopped answering: {e}");
        match chip.get(reg::CHIP_ID) {
            Ok(CHIP_ID) => {}
            Ok(other) => return Err(format!("not a BMI270 at 0x68 (it says {other:#04x})")),
            Err(_) => return Err("no motion sensor".into()),
        }
        chip.set(reg::CMD, 0xb6).map_err(gone)?;
        nap(Duration::from_millis(2));
        // Out of its power saving to take the program, a little while for
        // that to take; then the program, and the word that it is all there.
        chip.set(reg::PWR_CONF, 0x00).map_err(gone)?;
        nap(Duration::from_micros(500));
        chip.set(reg::INIT_CTRL, 0x00).map_err(gone)?;
        for (i, piece) in CONFIG.chunks(CHUNK).enumerate() {
            let word = (i * CHUNK / 2) as u16;
            chip.bus.write(reg::INIT_ADDR_0, &[(word & 0x0f) as u8, (word >> 4) as u8]).map_err(gone)?;
            chip.bus.write(reg::INIT_DATA, piece).map_err(gone)?;
        }
        chip.set(reg::INIT_CTRL, 0x01).map_err(gone)?;
        // It checks the program, within 20 ms by the datasheet.
        let mut status = 0;
        for _ in 0..15 {
            nap(Duration::from_millis(10));
            status = chip.get(reg::INTERNAL_STATUS).map_err(gone)? & 0x0f;
            if status == 0x01 {
                break;
            }
        }
        if status != 0x01 {
            return Err(format!("the motion sensor would not take its program (status {status})"));
        }
        chip.trim = chip.find_magnetometer(nap).unwrap_or(None);
        chip.sleep().map_err(gone)?;
        Ok(chip)
    }

    /// Whether the magnetometer answered.
    pub fn has_magnetometer(&self) -> bool {
        self.trim.is_some()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Everything off, in the chip's own power saving.
    fn sleep(&mut self) -> io::Result<()> {
        self.set(reg::PWR_CTRL, 0x00)?;
        self.set(reg::PWR_CONF, 0x01)?;
        self.mode = Mode::Off;
        Ok(())
    }

    /// The auxiliary bus: on, with the BMM150's address, by hand, eight bytes a read.
    fn aux_on(&mut self, nap: &mut dyn FnMut(Duration)) -> io::Result<()> {
        let conf = self.get(reg::IF_CONF)?;
        self.set(reg::IF_CONF, conf | 0x20)?;
        nap(Duration::from_millis(1));
        let power = self.get(reg::PWR_CTRL)?;
        self.set(reg::PWR_CTRL, power | 0x01)?;
        nap(Duration::from_millis(1));
        self.set(reg::AUX_DEV_ID, MAG_ADDRESS << 1)?;
        self.aux_idle(nap)?;
        self.set(reg::AUX_IF_CONF, 0x80 | 0x0c | 0x03)?;
        self.aux_idle(nap)?;
        self.set(reg::AUX_CONF, 0x0a)?;
        self.aux_idle(nap)
    }

    fn aux_off(&mut self) -> io::Result<()> {
        let power = self.get(reg::PWR_CTRL)?;
        self.set(reg::PWR_CTRL, power & !0x01)?;
        let conf = self.get(reg::IF_CONF)?;
        self.set(reg::IF_CONF, conf & !0x20)
    }

    /// Wait for the auxiliary bus to finish what it was asked.
    fn aux_idle(&mut self, nap: &mut dyn FnMut(Duration)) -> io::Result<()> {
        for _ in 0..20 {
            if self.get(reg::STATUS)? & 0x04 == 0 {
                return Ok(());
            }
            nap(Duration::from_millis(1));
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "the magnetometer's bus stayed busy"))
    }

    fn aux_write(&mut self, r: u8, v: u8, nap: &mut dyn FnMut(Duration)) -> io::Result<()> {
        self.aux_idle(nap)?;
        self.set(reg::AUX_WR_DATA, v)?;
        self.set(reg::AUX_WR_ADDR, r)?;
        self.aux_idle(nap)?;
        nap(Duration::from_millis(1));
        Ok(())
    }

    /// Eight bytes from the BMM150, from register `r` on.
    fn aux_read(&mut self, r: u8, nap: &mut dyn FnMut(Duration)) -> io::Result<[u8; 8]> {
        self.aux_idle(nap)?;
        self.set(reg::AUX_RD_ADDR, r)?;
        self.aux_idle(nap)?;
        nap(Duration::from_millis(1));
        let mut b = [0u8; 8];
        self.bus.read(reg::AUX_DATA, &mut b)?;
        Ok(b)
    }

    /// The BMM150, if one answers: its trim. Left in its deepest sleep.
    fn find_magnetometer(&mut self, nap: &mut dyn FnMut(Duration)) -> io::Result<Option<Trim>> {
        self.aux_on(nap)?;
        // Out of suspend (its id reads only then), a moment to wake.
        self.aux_write(mag::POWER, 0x01, nap)?;
        nap(Duration::from_millis(3));
        let id = self.aux_read(mag::CHIP_ID, nap)?[0];
        let trim = if id == MAG_ID {
            let mut r = Vec::with_capacity(24);
            for start in [mag::TRIM, mag::TRIM + 8, mag::TRIM + 16] {
                r.extend_from_slice(&self.aux_read(start, nap)?);
            }
            Some(Trim::parse(&r))
        } else {
            None
        };
        self.aux_write(mag::POWER, 0x00, nap).ok();
        self.aux_off()?;
        Ok(trim)
    }

    /// Change what the sensor does.
    pub fn set_mode(&mut self, mode: Mode, nap: &mut dyn FnMut(Duration)) -> io::Result<()> {
        let mode = if mode == Mode::Compass && self.trim.is_none() { Mode::Motion } else { mode };
        if mode == self.mode {
            return Ok(());
        }
        // Out of the chip's power saving first: it takes its settings
        // slowly while in it.
        self.set(reg::PWR_CONF, 0x00)?;
        nap(Duration::from_micros(500));
        if self.mode == Mode::Compass {
            // The magnetometer back to its deepest sleep.
            self.aux_write(mag::OP_MODE, 0x06, nap).ok();
            self.aux_write(mag::POWER, 0x00, nap).ok();
            self.aux_off()?;
        }
        match mode {
            Mode::Off => self.sleep()?,
            Mode::Watch => {
                // 12.5 readings a second, each the average of two, the
                // filter off: the chip's low-power mode, at its slowest.
                self.set(reg::ACC_CONF, 0x15)?;
                self.set(reg::ACC_RANGE, 0x00)?;
                self.set(reg::PWR_CTRL, 0x04)?;
                self.set(reg::PWR_CONF, 0x03)?;
            }
            Mode::Motion | Mode::Compass => {
                // 100 readings a second from the accelerometer, 200 from the
                // gyroscope, both filtered: the datasheet's normal mode.
                self.set(reg::ACC_CONF, 0xa8)?;
                self.set(reg::ACC_RANGE, 0x00)?;
                self.set(reg::GYR_CONF, 0xa9)?;
                self.set(reg::GYR_RANGE, 0x00)?;
                self.set(reg::PWR_CTRL, 0x0e)?;
                if mode == Mode::Compass {
                    self.aux_on(nap)?;
                    self.aux_write(mag::POWER, 0x01, nap)?;
                    nap(Duration::from_millis(3));
                    // Bosch's regular preset, ten readings a second.
                    self.aux_write(mag::REP_XY, 0x04, nap)?;
                    self.aux_write(mag::REP_Z, 0x07, nap)?;
                    self.aux_write(mag::OP_MODE, 0x00, nap)?;
                }
            }
        }
        self.mode = mode;
        Ok(())
    }

    /// The way up (in g) and the turning (in degrees a second), in the
    /// screen's frame.
    pub fn read(&mut self) -> io::Result<(Vec3, Vec3)> {
        let mut b = [0u8; 12];
        self.bus.read(reg::ACC_X, &mut b)?;
        let v = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]) as f32;
        let (a, g) = ((v(0), v(2), v(4)), (v(6), v(8), v(10)));
        Ok((
            Vec3::new(a.1, -a.0, a.2).scale(1.0 / ACC_PER_G),
            Vec3::new(g.1, -g.0, g.2).scale(1.0 / GYR_PER_DPS),
        ))
    }

    /// The magnetic field (µT, screen's frame), while the Compass has it on.
    pub fn read_field(&mut self, nap: &mut dyn FnMut(Duration)) -> Option<Vec3> {
        let trim = self.trim?;
        if self.mode != Mode::Compass {
            return None;
        }
        let d = self.aux_read(mag::DATA, nap).ok()?;
        let m = trim.field(&d)?;
        Some(Vec3::new(m.y, m.x, -m.z))
    }
}

// ---------------------------------------------------------------- gestures ---

/// Readings while watching, a quarter of a second apart.
pub const WATCH_EVERY: Duration = Duration::from_millis(250);
/// How many readings in a row must agree for it to be lying still: 1.5 s.
const STILL_READINGS: usize = 6;
/// How long face down before the screen goes off.
pub const FACE_DOWN_HOLD: f32 = 2.0;

/// What the accelerometer says about the handheld being picked up (a lift,
/// which wakes the screen) and put down on its face (which puts it out),
/// from readings while watching.
///
/// A lift is turning well away from where it last lay still: 35° at once,
/// or 20° on two readings running, and not ending face down. Only from
/// still: carried about, it never lay still, so it is never lifted, and a
/// pocket or a bag does not keep waking the screen; a knock on the bench
/// shakes it without turning it.
#[derive(Default, Debug)]
pub struct Gestures {
    recent: VecDeque<Vec3>,
    /// The way up while it last lay still.
    rest: Option<Vec3>,
    turned: u8,
    face_down_for: f32,
}

impl Gestures {
    /// A reading `dt` seconds after the last. True if it was lifted.
    pub fn feed(&mut self, up: Vec3, dt: f32) -> bool {
        let Some(dir) = up.unit() else { return false };
        // Face down: within 37° of the screen straight at the ground (and
        // no longer once it is 53° away, so it does not flicker in between).
        if dir.z < -0.8 {
            self.face_down_for += dt;
        } else if dir.z > -0.6 {
            self.face_down_for = 0.0;
        }
        self.recent.push_back(up);
        if self.recent.len() > STILL_READINGS {
            self.recent.pop_front();
        }
        if self.recent.len() == STILL_READINGS {
            let sum = self.recent.iter().fold(Vec3::default(), |s, v| s.add(*v));
            let mean = sum.scale(1.0 / STILL_READINGS as f32);
            let still = mean.unit().is_some_and(|m| self.recent.iter().all(|v| v.angle_to(m) < 3.0 && (v.len() - 1.0).abs() < 0.1));
            if still {
                self.rest = mean.unit();
                self.turned = 0;
                return false;
            }
        }
        let Some(rest) = self.rest else { return false };
        let away = dir.angle_to(rest);
        self.turned = if away >= 20.0 { self.turned.saturating_add(1) } else { 0 };
        let lifted = (away >= 35.0 || self.turned >= 2) && dir.z > -0.6;
        if lifted {
            self.rest = None;
            self.turned = 0;
            self.recent.clear();
        }
        lifted
    }

    /// Face down long enough to put the screen out.
    pub fn face_down(&self) -> bool {
        self.face_down_for >= FACE_DOWN_HOLD
    }
}

// ------------------------------------------------------- what the shell sees ---

/// What answered on the bus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Found {
    /// Still starting it (or finding out that nothing is there).
    Looking,
    /// No motion sensor: the Lite.
    Nothing,
    /// The BMI270 alone (the C154-V1).
    Motion,
    /// The BMI270 and the BMM150 behind it (the C154).
    Compass,
}

/// A reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// The way up, in g (1 g along +z lying face up).
    pub up: Vec3,
    /// Turning, in degrees a second.
    pub turn: Vec3,
    /// The magnetic field in µT, while the Compass has it on.
    pub field: Option<Vec3>,
}

/// What the sensor is held for: the Level's motion, the Compass's field too.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    Motion,
    Compass,
}

/// While kept, the sensor runs at full rate for its use; dropped, it goes
/// back to watching or to sleep.
pub struct Hold {
    use_: Use,
}

#[cfg(test)]
thread_local! {
    /// A test's own answer to what is on the bus.
    static PRETEND: std::cell::Cell<Option<Found>> = const { std::cell::Cell::new(None) };
}

/// For a test: answer `found` (none: as the build would) on this thread.
#[cfg(test)]
pub fn pretend(found: Option<Found>) {
    PRETEND.with(|p| p.set(found));
}

/// What answered.
pub fn found() -> Found {
    #[cfg(test)]
    {
        if let Some(f) = PRETEND.with(std::cell::Cell::get) {
            return f;
        }
    }
    device::found()
}

/// Whether there is a motion sensor (the Level is offered). While it is
/// still being started, what the last start found.
pub fn has_motion() -> bool {
    match found() {
        Found::Motion | Found::Compass => true,
        Found::Nothing => false,
        Found::Looking => matches!(device::remembered(), Some(Found::Motion | Found::Compass)),
    }
}

/// Whether the magnetometer is there too (the Compass is offered).
pub fn has_compass() -> bool {
    match found() {
        Found::Compass => true,
        Found::Motion | Found::Nothing => false,
        Found::Looking => device::remembered() == Some(Found::Compass),
    }
}

/// Start the sensor (on the handheld, on a thread of its own), so what is
/// there is known by the time Home is shown.
pub fn start() {
    device::start();
}

/// Run the sensor at full rate for `use_` while the hold is kept.
pub fn hold(use_: Use) -> Hold {
    device::hold(use_, 1);
    Hold { use_ }
}

impl Drop for Hold {
    fn drop(&mut self) {
        device::hold(self.use_, -1);
    }
}

/// The latest reading, while anything holds the sensor (or it is watching).
pub fn reading() -> Option<Reading> {
    device::reading()
}

/// Watch at the sensor's slowest, for a lift or being put face down.
pub fn set_watch(on: bool) {
    device::set_watch(on);
}

/// Lifted since last asked (only while watching).
pub fn take_lift() -> bool {
    device::take_lift()
}

/// Lying face down long enough to put the screen out (only while watching).
pub fn face_down() -> bool {
    device::face_down()
}

/// What went wrong with the sensor, if anything, for a hardware check.
#[cfg_attr(not(test), allow(dead_code))]
pub fn trouble() -> Option<String> {
    device::trouble()
}

/// For the simulator's Compass: turn the simulated handheld round and round
/// (as a calibration asks), or let it rest.
pub fn simulate_turning(on: bool) {
    device::simulate_turning(on);
}

/// For the simulator's galleries: stand the simulated handheld on its bottom
/// edge, a few degrees off upright, or lay it back on the bench.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn simulate_standing(on: bool) {
    device::simulate_standing(on);
}

// ------------------------------------------------------ on the handheld ---

#[cfg(target_arch = "aarch64")]
mod device {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex, OnceLock};

    struct State {
        found: Found,
        trouble: Option<String>,
        reading: Option<Reading>,
        /// Holds for motion and for the compass.
        motion: u32,
        compass: u32,
        watch: bool,
        lifted: bool,
        face_down: bool,
    }

    struct Imu {
        state: Mutex<State>,
        changed: Condvar,
    }

    static IMU: OnceLock<Arc<Imu>> = OnceLock::new();

    fn imu() -> &'static Arc<Imu> {
        IMU.get_or_init(|| {
            let imu = Arc::new(Imu {
                state: Mutex::new(State { found: Found::Looking, trouble: None, reading: None, motion: 0, compass: 0, watch: false, lifted: false, face_down: false }),
                changed: Condvar::new(),
            });
            let worker = imu.clone();
            std::thread::Builder::new().name("motion".into()).spawn(move || work(&worker)).ok();
            imu
        })
    }

    fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
        let imu = imu();
        let mut state = imu.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut state)
    }

    pub fn start() {
        imu();
    }

    pub fn found() -> Found {
        with(|s| s.found)
    }

    /// What the last start found, kept beside the settings.
    fn memory() -> std::path::PathBuf {
        crate::config_dir().join("motion-sensor")
    }

    fn read_memory() -> Option<Found> {
        match std::fs::read_to_string(memory()).ok()?.trim() {
            "compass" => Some(Found::Compass),
            "motion" => Some(Found::Motion),
            "none" => Some(Found::Nothing),
            _ => None,
        }
    }

    /// Read once: asked every frame while the sensor is still starting.
    pub fn remembered() -> Option<Found> {
        static READ: OnceLock<Option<Found>> = OnceLock::new();
        *READ.get_or_init(read_memory)
    }

    fn remember(found: Found) {
        let word = match found {
            Found::Compass => "compass",
            Found::Motion => "motion",
            _ => "none",
        };
        if read_memory() != Some(found) {
            crate::sys::write_atomic(&memory(), word.as_bytes()).ok();
        }
    }

    pub fn hold(use_: Use, by: i32) {
        let imu = imu();
        let mut s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
        let count = match use_ {
            Use::Motion => &mut s.motion,
            Use::Compass => &mut s.compass,
        };
        *count = count.saturating_add_signed(by);
        imu.changed.notify_all();
    }

    pub fn reading() -> Option<Reading> {
        with(|s| s.reading)
    }

    pub fn set_watch(on: bool) {
        let imu = imu();
        let mut s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.watch != on {
            s.watch = on;
            imu.changed.notify_all();
        }
    }

    pub fn take_lift() -> bool {
        with(|s| std::mem::take(&mut s.lifted))
    }

    pub fn face_down() -> bool {
        with(|s| s.face_down)
    }

    pub fn trouble() -> Option<String> {
        with(|s| s.trouble.clone())
    }

    pub fn simulate_turning(_on: bool) {}

    #[allow(dead_code)]
    pub fn simulate_standing(_on: bool) {}

    /// The board's I2C bus, talking to one device by combined transfers
    /// (a register, then its bytes, with nothing else between).
    struct I2c {
        file: std::fs::File,
        address: u16,
    }

    #[repr(C)]
    struct Msg {
        addr: u16,
        flags: u16,
        len: u16,
        buf: *mut u8,
    }

    #[repr(C)]
    struct Transfer {
        msgs: *mut Msg,
        nmsgs: u32,
    }

    const I2C_SLAVE: libc::c_ulong = 0x0703;
    const I2C_RDWR: libc::c_ulong = 0x0707;
    const I2C_M_RD: u16 = 0x0001;

    impl I2c {
        fn open(path: &str, address: u16) -> io::Result<I2c> {
            use std::os::fd::AsRawFd;
            let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
            // Claimed by a kernel driver (an overlay naming it): not ours to drive.
            if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, address as libc::c_ulong) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(I2c { file, address })
        }

        fn transfer(&mut self, msgs: &mut [Msg]) -> io::Result<()> {
            use std::os::fd::AsRawFd;
            let mut t = Transfer { msgs: msgs.as_mut_ptr(), nmsgs: msgs.len() as u32 };
            if unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_RDWR as _, &mut t as *mut Transfer) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Bus for I2c {
        fn read(&mut self, reg: u8, buf: &mut [u8]) -> io::Result<()> {
            let mut r = [reg];
            let mut msgs = [
                Msg { addr: self.address, flags: 0, len: 1, buf: r.as_mut_ptr() },
                Msg { addr: self.address, flags: I2C_M_RD, len: buf.len() as u16, buf: buf.as_mut_ptr() },
            ];
            self.transfer(&mut msgs)
        }

        fn write(&mut self, reg: u8, data: &[u8]) -> io::Result<()> {
            let mut b = Vec::with_capacity(data.len() + 1);
            b.push(reg);
            b.extend_from_slice(data);
            let mut msgs = [Msg { addr: self.address, flags: 0, len: b.len() as u16, buf: b.as_mut_ptr() }];
            self.transfer(&mut msgs)
        }
    }

    /// The sensor's own thread: start it, then run it as it is wanted.
    fn work(imu: &Imu) {
        let mut nap = |d: Duration| std::thread::sleep(d);
        let settle = |found: Found, trouble: Option<String>| {
            let mut s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
            s.found = found;
            s.trouble = trouble;
            remember(found);
        };
        // The bus's device appears with its module, loaded first thing in
        // boot; a few tries in case the shell came up first.
        let mut bus = None;
        let mut why = String::from("no I2C bus");
        for _ in 0..20 {
            match I2c::open("/dev/i2c-1", ADDRESS) {
                Ok(b) => {
                    bus = Some(b);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(libc::EBUSY) => {
                    why = "the motion sensor is claimed by a kernel driver".into();
                    break;
                }
                Err(e) => {
                    why = format!("no I2C bus: {e}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        let Some(bus) = bus else { return settle(Found::Nothing, Some(why)) };
        let mut chip = match Chip::start(bus, &mut nap) {
            Ok(chip) => chip,
            Err(why) => return settle(Found::Nothing, Some(why)),
        };
        settle(if chip.has_magnetometer() { Found::Compass } else { Found::Motion }, None);

        let mut gestures = Gestures::default();
        let mut last = Instant::now();
        let mut errors = 0u32;
        let mut field = None;
        let mut field_at = Instant::now();
        loop {
            let wanted = {
                let s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
                if s.compass > 0 {
                    Mode::Compass
                } else if s.motion > 0 {
                    Mode::Motion
                } else if s.watch {
                    Mode::Watch
                } else {
                    Mode::Off
                }
            };
            if wanted != chip.mode() {
                if let Err(e) = chip.set_mode(wanted, &mut nap) {
                    imu.state.lock().unwrap_or_else(|e| e.into_inner()).trouble = Some(format!("the motion sensor stopped answering: {e}"));
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
                if chip.mode() == Mode::Watch {
                    gestures = Gestures::default();
                }
                field = None;
                // The first reading after a change takes a sample's time.
                std::thread::sleep(Duration::from_millis(20));
                last = Instant::now();
            }
            if chip.mode() == Mode::Off {
                let mut s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
                s.reading = None;
                s.face_down = false;
                while s.compass == 0 && s.motion == 0 && !s.watch {
                    s = imu.changed.wait(s).unwrap_or_else(|e| e.into_inner());
                }
                continue;
            }
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f32();
            last = now;
            match chip.read() {
                Ok((up, turn)) => {
                    errors = 0;
                    if chip.mode() == Mode::Compass && field_at.elapsed() >= Duration::from_millis(100) {
                        field_at = Instant::now();
                        field = chip.read_field(&mut nap).or(field);
                    }
                    let lifted = chip.mode() == Mode::Watch && gestures.feed(up, dt);
                    let mut s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
                    s.reading = Some(Reading { up, turn, field });
                    s.lifted |= lifted;
                    s.face_down = chip.mode() == Mode::Watch && gestures.face_down();
                    s.trouble = None;
                }
                Err(e) => {
                    errors += 1;
                    if errors == 5 {
                        imu.state.lock().unwrap_or_else(|e| e.into_inner()).trouble = Some(format!("the motion sensor stopped answering: {e}"));
                    }
                }
            }
            let every = match chip.mode() {
                Mode::Watch => WATCH_EVERY,
                Mode::Compass => Duration::from_millis(40),
                _ => Duration::from_millis(30),
            };
            // Until the next reading, or until what is wanted changes.
            let s = imu.state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = imu.changed.wait_timeout(s, every);
        }
    }
}

// ------------------------------------------------------ on a computer ---

#[cfg(not(target_arch = "aarch64"))]
mod device {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::Instant;

    static STARTED: OnceLock<Instant> = OnceLock::new();
    static TURNING: AtomicBool = AtomicBool::new(false);
    static STANDING: AtomicBool = AtomicBool::new(false);
    /// Where the simulated handheld had turned to when it began turning, and when.
    static TURN_FROM: std::sync::Mutex<Option<(f32, f32)>> = std::sync::Mutex::new(None);

    fn clock() -> f32 {
        STARTED.get_or_init(Instant::now).elapsed().as_secs_f32()
    }

    pub fn start() {
        clock();
    }

    /// A computer has the simulated sensor, magnetometer and all.
    pub fn found() -> Found {
        Found::Compass
    }

    pub fn remembered() -> Option<Found> {
        Some(Found::Compass)
    }

    pub fn hold(_use: Use, _by: i32) {}

    pub fn reading() -> Option<Reading> {
        let t = clock();
        if STANDING.load(Ordering::Relaxed) {
            // On its bottom edge, a few degrees clockwise of upright.
            let a = (3.4 + 0.3 * (0.8 * t).sin()).to_radians();
            return Some(Reading { up: Vec3::new(-a.sin(), a.cos(), 0.03), turn: Vec3::default(), field: None });
        }
        let turning = TURNING.load(Ordering::Relaxed);
        // How fast the top edge swings, degrees a second, clockwise from above.
        let swing = if turning { 45.0 } else { 70.0 * 0.15 * (0.15 * t).cos() };
        Some(simulated(t, heading_now(), swing))
    }

    pub fn simulate_standing(on: bool) {
        STANDING.store(on, Ordering::Relaxed);
    }

    pub fn set_watch(_on: bool) {}

    /// Lying on the bench, the simulated handheld is never picked up.
    pub fn take_lift() -> bool {
        false
    }

    pub fn face_down() -> bool {
        false
    }

    pub fn trouble() -> Option<String> {
        None
    }

    pub fn simulate_turning(on: bool) {
        let mut from = TURN_FROM.lock().unwrap_or_else(|e| e.into_inner());
        if on && !TURNING.load(Ordering::Relaxed) {
            *from = Some((resting_heading(clock()), clock()));
        }
        TURNING.store(on, Ordering::Relaxed);
    }

    /// Where the simulated handheld's top edge points: swinging gently about
    /// north-east, or turning round once every eight seconds.
    fn heading_now() -> f32 {
        let t = clock();
        match *TURN_FROM.lock().unwrap_or_else(|e| e.into_inner()) {
            Some((start, at)) if TURNING.load(Ordering::Relaxed) => start + (t - at) * 45.0,
            _ => resting_heading(t),
        }
    }

    fn resting_heading(t: f32) -> f32 {
        40.0 + 70.0 * (0.15 * t).sin()
    }
}

/// The field where the simulator is: 20 µT to the north, 42 down (about
/// the Midwest's), in east, north, up.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
const EARTH: Vec3 = Vec3::new(0.0, 20.0, -42.0);
/// The simulated magnetometer's own offset, as a real one has from the
/// metal around it: what a calibration takes away.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub const SIMULATED_OFFSET: Vec3 = Vec3::new(14.0, -9.0, 6.0);

/// A world direction (east, north, up) in the screen's frame of a handheld
/// whose top edge points `heading` degrees from north, tipped `pitch` (top
/// edge up) and then `roll` (right edge up), in degrees.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn in_screen(world: Vec3, heading: f32, pitch: f32, roll: f32) -> Vec3 {
    let (sh, ch) = heading.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    // Lying flat: the top edge to the heading, the right edge clockwise of
    // it, the screen up.
    let (x0, y0, z0) = (Vec3::new(ch, -sh, 0.0), Vec3::new(sh, ch, 0.0), Vec3::new(0.0, 0.0, 1.0));
    // Tipped about its right edge, then about its new top edge.
    let y1 = y0.scale(cp).add(z0.scale(sp));
    let z1 = z0.scale(cp).sub(y0.scale(sp));
    let x2 = x0.scale(cr).add(z1.scale(sr));
    let z2 = z1.scale(cr).sub(x0.scale(sr));
    Vec3::new(world.dot(x2), world.dot(y1), world.dot(z2))
}

/// The simulated sensor at `t` seconds: a handheld on a bench, tipped a
/// couple of degrees and nudged now and then, its top edge to `heading`
/// and swinging `swing` degrees a second clockwise.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn simulated(t: f32, heading: f32, swing: f32) -> Reading {
    let pitch = 2.4 * (0.53 * t).sin() + 0.5 * (1.7 * t).sin();
    let roll = -1.3 + 1.8 * (0.37 * t + 1.0).sin();
    let up = in_screen(Vec3::new(0.0, 0.0, 1.0), heading, pitch, roll);
    let field = in_screen(EARTH, heading, pitch, roll).add(SIMULATED_OFFSET);
    // Turning: how fast the tipping changes, near enough; clockwise from
    // above is turning negatively about the screen's axis.
    let turn = Vec3::new(2.4 * 0.53 * (0.53 * t).cos(), 1.8 * 0.37 * (0.37 * t + 1.0).cos(), -swing);
    Reading { up, turn, field: Some(field) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMI270 with a BMM150 behind it (or not), as registers: what the
    /// driver writes lands here, and what it reads comes from here.
    struct Fake {
        regs: [u8; 128],
        /// The program, as written word by word.
        program: Vec<u8>,
        mag: Option<[u8; 128]>,
        writes: Vec<(u8, Vec<u8>)>,
    }

    impl Fake {
        fn new(with_mag: bool) -> Fake {
            let mut regs = [0u8; 128];
            regs[reg::CHIP_ID as usize] = CHIP_ID;
            let mag = with_mag.then(|| {
                let mut m = [0u8; 128];
                m[mag::CHIP_ID as usize] = MAG_ID;
                // A trim like a real one's, register by register.
                let mut put = |reg: usize, bytes: &[u8]| m[reg..reg + bytes.len()].copy_from_slice(bytes);
                put(0x5d, &[0, 0]); // x1, y1
                put(0x62, &0i16.to_le_bytes()); // z4
                put(0x64, &[29, 29]); // x2, y2
                put(0x68, &26178i16.to_le_bytes()); // z2
                put(0x6a, &25410u16.to_le_bytes()); // z1
                put(0x6c, &6600u16.to_le_bytes()); // xyz1
                put(0x6e, &(-2352i16).to_le_bytes()); // z3
                put(0x70, &[(-3i8) as u8, 25]); // xy2, xy1
                m
            });
            Fake { regs, program: vec![0; 8192], mag, writes: Vec::new() }
        }

        fn mag_on(&self) -> bool {
            self.mag.is_some_and(|m| m[mag::POWER as usize] & 1 == 1)
        }
    }

    impl Bus for Fake {
        fn read(&mut self, r: u8, buf: &mut [u8]) -> io::Result<()> {
            if self.regs[reg::CHIP_ID as usize] != CHIP_ID {
                return Err(io::Error::other("nack"));
            }
            for (i, b) in buf.iter_mut().enumerate() {
                *b = self.regs[(r as usize + i) % 128];
            }
            Ok(())
        }

        fn write(&mut self, r: u8, data: &[u8]) -> io::Result<()> {
            if self.regs[reg::CHIP_ID as usize] != CHIP_ID {
                return Err(io::Error::other("nack"));
            }
            self.writes.push((r, data.to_vec()));
            match r {
                reg::INIT_DATA => {
                    let word = self.regs[reg::INIT_ADDR_0 as usize] as usize & 0x0f | (self.regs[reg::INIT_ADDR_0 as usize + 1] as usize) << 4;
                    self.program[word * 2..word * 2 + data.len()].copy_from_slice(data);
                }
                reg::INIT_CTRL if data[0] == 1 => {
                    // It starts only with Bosch's program, every byte in place.
                    self.regs[reg::INTERNAL_STATUS as usize] = if self.program[..] == CONFIG[..] { 0x01 } else { 0x02 };
                }
                reg::AUX_WR_ADDR => {
                    let v = self.regs[reg::AUX_WR_DATA as usize];
                    if let Some(m) = &mut self.mag {
                        m[data[0] as usize] = v;
                    }
                }
                reg::AUX_RD_ADDR => {
                    let on = self.mag_on();
                    let out: [u8; 8] = match &self.mag {
                        // Its registers read back only out of suspend (bar its power bit).
                        Some(m) if on || data[0] == mag::POWER => std::array::from_fn(|i| m[(data[0] as usize + i) % 128]),
                        _ => [0; 8],
                    };
                    self.regs[reg::AUX_DATA as usize..reg::AUX_DATA as usize + 8].copy_from_slice(&out);
                }
                _ => {
                    for (i, v) in data.iter().enumerate() {
                        self.regs[(r as usize + i) % 128] = *v;
                    }
                }
            }
            Ok(())
        }
    }

    fn started(with_mag: bool) -> Chip<Fake> {
        Chip::start(Fake::new(with_mag), &mut |_| {}).expect("started")
    }

    #[test]
    fn the_program_is_bosch_s_own() {
        assert_eq!(CONFIG.len(), 8192);
        // Its first bytes, as bmi270_config_file[] has them.
        assert_eq!(&CONFIG[..4], &[0xc8, 0x2e, 0x00, 0x2e]);
        assert!(CONFIG.iter().any(|b| *b != 0));
    }

    #[test]
    fn it_starts_with_its_program_in_place_and_sleeps() {
        let chip = started(false);
        // Every byte reached its place, and the chip said so.
        assert_eq!(chip.bus.program[..], CONFIG[..]);
        assert_eq!(chip.mode(), Mode::Off);
        assert!(!chip.has_magnetometer());
        // Reset first; left with everything off and its power saving on.
        assert_eq!(chip.bus.writes[0], (reg::CMD, vec![0xb6]));
        assert_eq!(chip.bus.regs[reg::PWR_CTRL as usize], 0x00);
        assert_eq!(chip.bus.regs[reg::PWR_CONF as usize] & 0x01, 0x01);
        // The program went in short pieces, so the keyboard is never kept waiting.
        assert!(chip.bus.writes.iter().filter(|(r, _)| *r == reg::INIT_DATA).all(|(_, d)| d.len() <= CHUNK));
    }

    #[test]
    fn nothing_on_the_bus_or_the_wrong_chip_is_said_plainly() {
        let mut nothing = Fake::new(false);
        nothing.regs[reg::CHIP_ID as usize] = 0;
        assert_eq!(Chip::start(nothing, &mut |_| {}).err().as_deref(), Some("no motion sensor"));
    }

    #[test]
    fn a_program_that_did_not_take_is_said_plainly() {
        struct Bad(Fake);
        impl Bus for Bad {
            fn read(&mut self, r: u8, buf: &mut [u8]) -> io::Result<()> {
                self.0.read(r, buf)
            }
            fn write(&mut self, r: u8, data: &[u8]) -> io::Result<()> {
                // A byte of the program lost on the way.
                let mut data = data.to_vec();
                if r == reg::INIT_DATA {
                    data[0] ^= 0xff;
                }
                self.0.write(r, &data)
            }
        }
        let why = Chip::start(Bad(Fake::new(false)), &mut |_| {}).err().unwrap();
        assert!(why.contains("would not take its program"), "{why}");
    }

    #[test]
    fn readings_come_in_the_screen_s_frame() {
        let mut chip = started(false);
        chip.set_mode(Mode::Motion, &mut |_| {}).unwrap();
        assert_eq!(chip.bus.regs[reg::PWR_CTRL as usize], 0x0e, "accelerometer, gyroscope and temperature on");
        // The chip's own x at half a g, its z at one, and turning about its y.
        let raw: [i16; 6] = [8192, 0, 16384, 0, 1638, 0];
        for (i, v) in raw.iter().enumerate() {
            chip.bus.regs[reg::ACC_X as usize + i * 2..reg::ACC_X as usize + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        let (up, turn) = chip.read().unwrap();
        // The chip's x is the screen's −y (as M5Stack's Compass app turns it).
        assert_eq!(up, Vec3::new(0.0, -0.5, 1.0));
        assert!((turn.x - 100.0).abs() < 0.1 && turn.y == 0.0, "{turn:?}");
    }

    #[test]
    fn the_magnetometer_is_found_behind_it_and_read() {
        let mut chip = started(true);
        assert!(chip.has_magnetometer());
        // Its trim read back register by register, as the factory left it.
        let trim = chip.trim.unwrap();
        assert_eq!(trim, Trim { x1: 0, y1: 0, x2: 29, y2: 29, z1: 25410, z2: 26178, z3: -2352, z4: 0, xy1: 25, xy2: -3, xyz1: 6600 });
        // Found, then put back in its deepest sleep, its bus off.
        assert!(!chip.bus.mag_on());
        assert_eq!(chip.bus.regs[reg::IF_CONF as usize] & 0x20, 0);
        // On for the Compass: its regular preset, reading continuously.
        chip.set_mode(Mode::Compass, &mut |_| {}).unwrap();
        let m = chip.bus.mag.unwrap();
        assert_eq!((m[mag::REP_XY as usize], m[mag::REP_Z as usize], m[mag::OP_MODE as usize]), (0x04, 0x07, 0x00));
        assert_eq!(chip.bus.regs[reg::PWR_CTRL as usize] & 0x01, 0x01, "the auxiliary bus powered");
        // A reading: 1000 counts along its x, none along y, -2000 along z, hall 6000.
        let (x, y, z, hall) = (1000i16, 0i16, -2000i16, 6000u16);
        let data = [((x << 3) & 0xff) as u8, (x >> 5) as u8, ((y << 3) & 0xff) as u8, (y >> 5) as u8, ((z << 1) & 0xff) as u8, (z >> 7) as u8, ((hall << 2) & 0xff) as u8, (hall >> 6) as u8];
        chip.bus.mag.as_mut().unwrap()[0x42..0x4a].copy_from_slice(&data);
        let field = chip.read_field(&mut |_| {}).expect("a field");
        // Its x is the screen's y; its z the screen's −z.
        assert!(field.y > 0.0 && field.x.abs() < 1.0 && field.z > 0.0, "{field:?}");
        // Back to Motion: the magnetometer sleeps again.
        chip.set_mode(Mode::Motion, &mut |_| {}).unwrap();
        assert!(!chip.bus.mag_on());
        assert_eq!(chip.read_field(&mut |_| {}), None);
    }

    #[test]
    fn a_unit_without_a_magnetometer_keeps_to_motion() {
        let mut chip = started(false);
        chip.set_mode(Mode::Compass, &mut |_| {}).unwrap();
        assert_eq!(chip.mode(), Mode::Motion);
        assert_eq!(chip.read_field(&mut |_| {}), None);
    }

    #[test]
    fn watching_is_the_accelerometer_alone_at_its_least() {
        let mut chip = started(false);
        chip.set_mode(Mode::Watch, &mut |_| {}).unwrap();
        assert_eq!(chip.bus.regs[reg::PWR_CTRL as usize], 0x04);
        assert_eq!(chip.bus.regs[reg::ACC_CONF as usize] & 0x80, 0, "the filter off: the chip's low-power mode");
        assert_eq!(chip.bus.regs[reg::PWR_CONF as usize] & 0x01, 0x01, "its power saving on");
        chip.set_mode(Mode::Off, &mut |_| {}).unwrap();
        assert_eq!(chip.bus.regs[reg::PWR_CTRL as usize], 0x00);
    }

    #[test]
    fn compensation_matches_bosch_s_own_numbers() {
        // A BMM150's trim and one of its readings, worked through Bosch's
        // float equations by hand.
        let trim = Trim { x1: 0, y1: 0, x2: 29, y2: 29, z1: 25410, z2: 26178, z3: -2352, z4: 0, xy1: 25, xy2: -3, xyz1: 6600 };
        let (x, z, hall) = (1000i16, 500i16, 6600u16);
        let d = [((x << 3) & 0xff) as u8, (x >> 5) as u8, 0, 0, ((z << 1) & 0xff) as u8, (z >> 7) as u8, ((hall << 2) & 0xff) as u8, (hall >> 6) as u8];
        let f = trim.field(&d).unwrap();
        // At hall = xyz1 the x equation reduces to x · 256 · (x2 + 160) / 8192 / 16.
        let want_x = 1000.0 * 256.0 * (29.0 + 160.0) / 8192.0 / 16.0;
        assert!((f.x - want_x).abs() < 0.01, "{} vs {want_x}", f.x);
        // And z to z · 131072 / ((z2 + z1 · hall / 32768) · 4) / 16.
        let want_z = 500.0 * 131_072.0 / ((26178.0 + 25410.0 * 6600.0 / 32768.0) * 4.0) / 16.0;
        assert!((f.z - want_z).abs() < 0.01, "{} vs {want_z}", f.z);
        // An overflowed axis is no reading.
        let over = [0, 0x80, 0, 0, 0, 0, 0, 0x80];
        assert_eq!(trim.field(&over), None);
    }

    #[test]
    fn pitch_roll_and_the_edge_read_as_a_level_does() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert_eq!(tilt(Vec3::new(0.0, 0.0, 1.0)), (0.0, 0.0));
        for (p, r) in [(10.0f32, 0.0f32), (0.0, -7.5), (25.0, 3.0), (-4.0, 12.0)] {
            let up = in_screen(Vec3::new(0.0, 0.0, 1.0), 123.0, p, r);
            let (pitch, roll) = tilt(up);
            assert!(near(pitch, p), "pitch {pitch} for {p}");
            // The roll is the right edge's own angle to level, after the pitch.
            let want = (r.to_radians().sin() * p.to_radians().cos()).asin().to_degrees();
            assert!(near(roll, want), "roll {roll} for {r} after {p}");
        }
        // On its bottom edge, upright: level. Turned clockwise 12°: 12, right end low.
        assert!(near(edge(Vec3::new(0.0, 1.0, 0.0)), 0.0));
        let turned = Vec3::new(-(12.0f32.to_radians().sin()), 12.0f32.to_radians().cos(), 0.0);
        assert!(near(edge(turned), 12.0));
        assert!(near(edge(Vec3::new(1.0, 0.0, 0.0)), -90.0), "on its left side");
    }

    #[test]
    fn the_heading_holds_however_it_is_tipped_and_whichever_way_up() {
        for heading_true in [0.0f32, 37.0, 90.0, 181.0, 270.0, 333.0] {
            for (p, r) in [(0.0, 0.0), (20.0, 0.0), (0.0, -25.0), (-30.0, 15.0), (50.0, 10.0), (0.0, 180.0), (10.0, 175.0)] {
                let up = in_screen(Vec3::new(0.0, 0.0, 1.0), heading_true, p, r);
                let field = in_screen(EARTH, heading_true, p, r);
                let h = heading(up, field).expect("a heading");
                let off = (h - heading_true + 540.0).rem_euclid(360.0) - 180.0;
                assert!(off.abs() < 0.05, "{heading_true}° tipped {p}/{r}: read {h}");
            }
        }
        // Top edge straight up: no heading to give.
        assert_eq!(heading(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 20.0, -40.0)), None);
        assert_eq!(point(0.0), "N");
        assert_eq!(point(247.0), "WSW");
        assert_eq!(point(359.0), "N");
    }

    #[test]
    fn lifted_from_still_wakes_and_carried_about_does_not() {
        let flat = Vec3::new(0.0, 0.0, 1.0);
        let mut g = Gestures::default();
        // Lying still for a couple of seconds, then a knock: no lift.
        for _ in 0..8 {
            assert!(!g.feed(flat, 0.25));
        }
        assert!(!g.feed(Vec3::new(0.05, 0.0, 1.4), 0.25), "a knock shakes it without turning it");
        assert!(!g.feed(flat, 0.25));
        // Picked up to look at: tipped 45° towards you.
        let looking = in_screen(Vec3::new(0.0, 0.0, 1.0), 0.0, 45.0, 0.0);
        assert!(g.feed(looking.scale(1.05), 0.25), "lifted");
        // Only once: it must lie still again first.
        assert!(!g.feed(flat, 0.25));
        // Carried about, never still: never lifted, however it turns.
        let mut walking = Gestures::default();
        for i in 0..40 {
            let p = 40.0 * (i as f32 * 0.9).sin();
            let r = 30.0 * (i as f32 * 0.6).cos();
            assert!(!walking.feed(in_screen(Vec3::new(0.0, 0.0, 1.0), 0.0, p, r).scale(1.0 + 0.2 * (i as f32).sin()), 0.25));
        }
        // A slow turn of 20° on two readings running counts.
        let mut slow = Gestures::default();
        for _ in 0..6 {
            slow.feed(flat, 0.25);
        }
        assert!(!slow.feed(in_screen(Vec3::new(0.0, 0.0, 1.0), 0.0, 22.0, 0.0), 0.25));
        assert!(slow.feed(in_screen(Vec3::new(0.0, 0.0, 1.0), 0.0, 26.0, 0.0), 0.25));
    }

    #[test]
    fn face_down_for_two_seconds_puts_it_out_and_turning_it_over_wakes_it() {
        let down = Vec3::new(0.0, 0.05, -1.0);
        let mut g = Gestures::default();
        for _ in 0..7 {
            g.feed(down, 0.25);
            assert!(!g.face_down(), "not yet");
        }
        g.feed(down, 0.25);
        assert!(g.face_down(), "two seconds face down");
        // Lying face down, still: turned over is a lift.
        for _ in 0..4 {
            g.feed(down, 0.25);
        }
        assert!(g.feed(Vec3::new(0.0, 0.3, 0.95), 0.25));
        assert!(!g.face_down());
        // Lifted from face up and put face down is not a lift.
        let mut h = Gestures::default();
        for _ in 0..6 {
            h.feed(Vec3::new(0.0, 0.0, 1.0), 0.25);
        }
        assert!(!h.feed(down, 0.25));
    }

    #[test]
    fn the_simulator_s_sensor_reads_back_its_own_heading() {
        let r = simulated(3.0, 75.0, 0.0);
        let h = heading(r.up, r.field.unwrap().sub(SIMULATED_OFFSET)).unwrap();
        assert!((h - 75.0).abs() < 0.1, "{h}");
        let (pitch, roll) = tilt(r.up);
        assert!(pitch.abs() < 4.0 && roll.abs() < 4.0, "lying on a bench: {pitch} {roll}");
    }
}
