//! A serial line with its two control lines, as the installer needs one:
//! the UnitV's USB serial bridge on the handheld (a tty, raw), the stand-in
//! K210 in tests and the simulator.

use std::time::{Duration, Instant};

pub trait Serial: Send {
    fn set_baud(&mut self, baud: u32) -> bool;
    /// DTR and RTS, `true` asserted (the line pulled low, as pyserial's
    /// `setDTR(True)`). A K210 board wires one to its reset and the other to
    /// its boot pin (IO16).
    fn lines(&mut self, dtr: bool, rts: bool);
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// What has arrived, waiting up to `wait` for something.
    fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize;
    /// Whatever has arrived and not been read, dropped.
    fn discard(&mut self);
}

/// Bytes read until `needle` ends them, keeping what came after for the
/// next call. `Err` with what came, if it did not within `within`.
pub struct Reader {
    held: Vec<u8>,
}

impl Reader {
    pub fn new() -> Reader {
        Reader { held: Vec::new() }
    }

    pub fn clear(&mut self, port: &mut dyn Serial) {
        self.held.clear();
        port.discard();
    }

    pub fn until(&mut self, port: &mut dyn Serial, needle: &[u8], within: Duration) -> Result<Vec<u8>, Vec<u8>> {
        let deadline = Instant::now() + within;
        let mut buf = [0u8; 1024];
        loop {
            if let Some(at) = self.held.windows(needle.len()).position(|w| w == needle) {
                let rest = self.held.split_off(at + needle.len());
                return Ok(std::mem::replace(&mut self.held, rest));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(std::mem::take(&mut self.held));
            }
            let n = port.read(&mut buf, (deadline - now).min(Duration::from_millis(50)));
            self.held.extend_from_slice(&buf[..n]);
        }
    }

    /// Exactly `n` bytes, or what came within `within`.
    pub fn exactly(&mut self, port: &mut dyn Serial, n: usize, within: Duration) -> Result<Vec<u8>, Vec<u8>> {
        let deadline = Instant::now() + within;
        let mut buf = [0u8; 256];
        while self.held.len() < n {
            let now = Instant::now();
            if now >= deadline {
                return Err(std::mem::take(&mut self.held));
            }
            let got = port.read(&mut buf, (deadline - now).min(Duration::from_millis(50)));
            self.held.extend_from_slice(&buf[..got]);
        }
        let rest = self.held.split_off(n);
        Ok(std::mem::replace(&mut self.held, rest))
    }

    /// Whether a byte waits, without waiting.
    pub fn waiting(&mut self, port: &mut dyn Serial) -> bool {
        if self.held.is_empty() {
            let mut buf = [0u8; 256];
            let n = port.read(&mut buf, Duration::ZERO);
            self.held.extend_from_slice(&buf[..n]);
        }
        !self.held.is_empty()
    }
}

/// The time `bytes` take on the wire at `baud` (ten bits each).
pub fn wire_time(bytes: usize, baud: u32) -> Duration {
    Duration::from_secs_f64(bytes as f64 * 10.0 / f64::from(baud.max(1)))
}

#[cfg(target_os = "linux")]
pub use tty::Tty;

#[cfg(target_os = "linux")]
mod tty {
    use super::Serial;
    use std::time::Duration;

    /// A USB serial port (ttyUSB*, ttyACM*), raw, 8N1, no flow control, its
    /// lines left as they are when it closes.
    pub struct Tty {
        fd: i32,
    }

    fn speed(baud: u32) -> libc::speed_t {
        match baud {
            3_000_000 => libc::B3000000,
            2_000_000 => libc::B2000000,
            1_500_000 => libc::B1500000,
            1_000_000 => libc::B1000000,
            921_600 => libc::B921600,
            460_800 => libc::B460800,
            230_400 => libc::B230400,
            _ => libc::B115200,
        }
    }

    impl Tty {
        /// Opened at 115 200 with both lines released together (the open
        /// itself asserts them, which holds a K210 in reset meanwhile).
        pub fn open(path: &str) -> Option<Tty> {
            let c = std::ffi::CString::new(path).ok()?;
            // SAFETY: plain system calls on a descriptor this owns.
            unsafe {
                let fd = libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC);
                if fd < 0 {
                    return None;
                }
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(fd, &mut t);
                libc::cfmakeraw(&mut t);
                t.c_cflag |= libc::CLOCAL | libc::CREAD;
                t.c_cflag &= !(libc::CRTSCTS | libc::CSTOPB | libc::PARENB | libc::HUPCL);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::cfsetispeed(&mut t, libc::B115200);
                libc::cfsetospeed(&mut t, libc::B115200);
                libc::tcsetattr(fd, libc::TCSANOW, &t);
                let mut tty = Tty { fd };
                tty.lines(false, false);
                libc::tcflush(fd, libc::TCIOFLUSH);
                Some(tty)
            }
        }
    }

    impl Drop for Tty {
        fn drop(&mut self) {
            // SAFETY: the descriptor this opened, closed once.
            unsafe { libc::close(self.fd) };
        }
    }

    impl Serial for Tty {
        fn set_baud(&mut self, baud: u32) -> bool {
            // SAFETY: termios on a descriptor this owns.
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(self.fd, &mut t);
                libc::cfsetispeed(&mut t, speed(baud));
                libc::cfsetospeed(&mut t, speed(baud));
                libc::tcdrain(self.fd);
                libc::tcsetattr(self.fd, libc::TCSANOW, &t) == 0
            }
        }

        fn lines(&mut self, dtr: bool, rts: bool) {
            // Both in one request where they change together, so the bridge
            // moves them at the same moment.
            let (mut set, mut clear) = (0, 0);
            for (on, bit) in [(dtr, libc::TIOCM_DTR), (rts, libc::TIOCM_RTS)] {
                if on {
                    set |= bit;
                } else {
                    clear |= bit;
                }
            }
            // SAFETY: modem-line ioctls on a descriptor this owns.
            unsafe {
                if clear != 0 {
                    libc::ioctl(self.fd, libc::TIOCMBIC, &clear);
                }
                if set != 0 {
                    libc::ioctl(self.fd, libc::TIOCMBIS, &set);
                }
            }
        }

        fn write(&mut self, bytes: &[u8]) -> bool {
            let mut at = 0;
            while at < bytes.len() {
                // SAFETY: writing from a live slice to a descriptor this owns.
                let n = unsafe { libc::write(self.fd, bytes[at..].as_ptr().cast(), bytes.len() - at) };
                if n <= 0 {
                    let mut p = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: one pollfd.
                    if unsafe { libc::poll(&mut p, 1, 1000) } <= 0 {
                        return false;
                    }
                    continue;
                }
                at += n as usize;
            }
            true
        }

        fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize {
            let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd; reading into a live buffer.
            unsafe {
                if libc::poll(&mut p, 1, wait.as_millis() as i32) <= 0 {
                    return 0;
                }
                libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()).max(0) as usize
            }
        }

        fn discard(&mut self) {
            // SAFETY: a flush on a descriptor this owns.
            unsafe { libc::tcflush(self.fd, libc::TCIFLUSH) };
        }
    }
}
