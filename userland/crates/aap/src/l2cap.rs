//! An L2CAP channel to a Bluetooth device already connected (BlueZ's
//! sockets: AF_BLUETOOTH, SOCK_SEQPACKET, so each read is one packet).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

const BTPROTO_L2CAP: libc::c_int = 0;
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_SECURITY: libc::c_int = 4;
/// Encrypted, as the paired link is.
const BT_SECURITY_MEDIUM: u8 = 2;

#[repr(C)]
struct SockaddrL2 {
    family: libc::sa_family_t,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

/// "AA:BB:CC:DD:EE:FF" as BlueZ keeps an address: the bytes reversed.
pub fn bdaddr(address: &str) -> Option<[u8; 6]> {
    let parts: Vec<u8> = address.split(':').map(|p| u8::from_str_radix(p, 16).ok()).collect::<Option<_>>()?;
    let mut b: [u8; 6] = parts.try_into().ok()?;
    b.reverse();
    Some(b)
}

pub struct Channel {
    fd: OwnedFd,
}

fn timeval(d: Duration) -> libc::timeval {
    libc::timeval { tv_sec: d.as_secs() as _, tv_usec: d.subsec_micros() as _ }
}

impl Channel {
    /// Opened to `address` on `psm`, waiting up to `timeout` for it.
    pub fn connect(address: &str, psm: u16, timeout: Duration) -> io::Result<Channel> {
        let addr = bdaddr(address).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a Bluetooth address"))?;
        unsafe {
            let fd = libc::socket(libc::AF_BLUETOOTH, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, BTPROTO_L2CAP);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(fd);
            let sec = [BT_SECURITY_MEDIUM, 0u8];
            libc::setsockopt(fd.as_raw_fd(), SOL_BLUETOOTH, BT_SECURITY, sec.as_ptr().cast(), 2);
            // connect(2) on a Bluetooth socket waits as long as its send timeout.
            let tv = timeval(timeout);
            libc::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDTIMEO, (&tv as *const libc::timeval).cast(), std::mem::size_of::<libc::timeval>() as libc::socklen_t);
            let sa = SockaddrL2 { family: libc::AF_BLUETOOTH as libc::sa_family_t, psm: psm.to_le(), bdaddr: addr, cid: 0, bdaddr_type: 0 };
            let r = libc::connect(fd.as_raw_fd(), (&sa as *const SockaddrL2).cast(), std::mem::size_of::<SockaddrL2>() as libc::socklen_t);
            if r < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Channel { fd })
        }
    }

    pub fn send(&self, packet: &[u8]) -> io::Result<()> {
        let n = unsafe { libc::send(self.fd.as_raw_fd(), packet.as_ptr().cast(), packet.len(), libc::MSG_NOSIGNAL) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The next packet, or `None` if none came within `wait`.
    pub fn recv(&self, buf: &mut [u8], wait: Duration) -> io::Result<Option<usize>> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, wait.as_millis().min(i32::MAX as u128) as libc::c_int) };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(e) };
        }
        if r == 0 {
            return Ok(None);
        }
        let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        match n {
            n if n > 0 => Ok(Some(n as usize)),
            0 => Err(io::Error::new(io::ErrorKind::ConnectionReset, "the AirPods closed the channel")),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_are_reversed_as_bluez_keeps_them() {
        assert_eq!(super::bdaddr("AA:BB:CC:DD:EE:01"), Some([0x01, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]));
        assert_eq!(super::bdaddr("AA:BB"), None);
        assert_eq!(super::bdaddr("zz:BB:CC:DD:EE:01"), None);
    }
}
