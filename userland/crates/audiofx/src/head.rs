//! The head's pose, from the shell (which reads the AirPods' head tracking)
//! to the player (which turns the spatial sound to match).
//!
//! The player listens on a Unix datagram socket while spatial sound plays
//! through headphones, and only then: the shell starts the AirPods' head
//! tracking while something listens there and stops it when nothing does
//! (the song paused, spatial sound off), so their sensors never run for
//! nothing. Each datagram is one pose, 8 bytes: `H`, version 1, then the
//! yaw and pitch in hundredths of a degree (i16, little-endian), then two
//! bytes of flags (none yet). A datagram of one byte is the shell asking
//! whether anyone is there; the player ignores it.

pub const SOCKET: &str = "/run/catalyst-audio/head.sock";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Degrees, + turned right.
    pub yaw: f32,
    /// Degrees, + looking up.
    pub pitch: f32,
}

pub fn encode(p: Pose) -> [u8; 8] {
    let y = ((p.yaw.clamp(-180.0, 180.0) * 100.0).round() as i16).to_le_bytes();
    let q = ((p.pitch.clamp(-90.0, 90.0) * 100.0).round() as i16).to_le_bytes();
    [b'H', 1, y[0], y[1], q[0], q[1], 0, 0]
}

pub fn decode(b: &[u8]) -> Option<Pose> {
    if b.len() < 6 || b[0] != b'H' || b[1] != 1 {
        return None;
    }
    let yaw = i16::from_le_bytes([b[2], b[3]]) as f32 / 100.0;
    let pitch = i16::from_le_bytes([b[4], b[5]]) as f32 / 100.0;
    Some(Pose { yaw, pitch })
}

fn wrap(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// Where "ahead" is, following the head slowly: a turn and the sound stays
/// put; stay turned, and the stage comes round to the new way over some
/// seconds (as Apple's spatial audio does when the phone is not tracked).
pub struct Recenter {
    seconds: f32,
    ahead: Option<(f32, f32)>,
}

impl Recenter {
    pub fn new(seconds: f32) -> Recenter {
        Recenter { seconds, ahead: None }
    }

    /// The head's pose as the AirPods say it, `dt` seconds after the last:
    /// the pose relative to the stage.
    pub fn update(&mut self, raw: Pose, dt: f32) -> Pose {
        let (yaw0, pitch0) = *self.ahead.get_or_insert((raw.yaw, raw.pitch));
        let rel = Pose { yaw: wrap(raw.yaw - yaw0), pitch: raw.pitch - pitch0 };
        let k = 1.0 - (-dt.max(0.0) / self.seconds).exp();
        self.ahead = Some((wrap(yaw0 + rel.yaw * k), pitch0 + rel.pitch * k));
        rel
    }

    pub fn reset(&mut self) {
        self.ahead = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poses_cross_the_socket_whole() {
        let p = Pose { yaw: -37.25, pitch: 12.5 };
        assert_eq!(decode(&encode(p)), Some(p));
        assert_eq!(decode(&[b'H']), None);
        assert_eq!(decode(b"Xx......"), None);
        assert_eq!(decode(&encode(Pose { yaw: 500.0, pitch: -500.0 })), Some(Pose { yaw: 180.0, pitch: -90.0 }));
    }

    #[test]
    fn the_stage_stays_put_for_a_glance_and_comes_round_for_a_turn() {
        let mut r = Recenter::new(6.0);
        assert_eq!(r.update(Pose { yaw: 100.0, pitch: 0.0 }, 0.02), Pose { yaw: 0.0, pitch: 0.0 });
        // A quick glance 40° right: heard as 40° (a little less as it settles).
        let glance = r.update(Pose { yaw: 140.0, pitch: 0.0 }, 0.02);
        assert!((glance.yaw - 40.0).abs() < 0.2);
        // Held there for half a minute: the stage has come round.
        let mut last = glance;
        for _ in 0..1500 {
            last = r.update(Pose { yaw: 140.0, pitch: 0.0 }, 0.02);
        }
        assert!(last.yaw.abs() < 2.0, "{last:?}");
        // Across the ±180° seam, the short way round.
        let mut r = Recenter::new(6.0);
        r.update(Pose { yaw: 175.0, pitch: 0.0 }, 0.02);
        let across = r.update(Pose { yaw: -175.0, pitch: 0.0 }, 0.02);
        assert!((across.yaw - 10.0).abs() < 0.1, "{across:?}");
    }
}
