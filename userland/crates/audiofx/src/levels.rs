//! The level of what plays, for anything that draws it (the island's bars):
//! `catalyst-play --levels FILE` writes each ALSA period's RMS there, eight
//! bytes over the same eight bytes each time, so a reader just reads it
//! when it draws; nothing wakes for it. The bytes: a sequence number (u16,
//! little-endian), the left and right RMS as 0 to 65535 of full scale, and
//! the sequence number again (a read caught mid-write has two that differ,
//! and is tried again).

pub fn encode(seq: u16, left: f32, right: f32) -> [u8; 8] {
    let q = |x: f32| ((x.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes();
    let (s, l, r) = (seq.to_le_bytes(), q(left), q(right));
    [s[0], s[1], l[0], l[1], r[0], r[1], s[0], s[1]]
}

/// The sequence number and the two levels (0 to 1).
pub fn decode(b: &[u8]) -> Option<(u16, f32, f32)> {
    if b.len() < 8 || b[0..2] != b[6..8] {
        return None;
    }
    let v = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as f32 / 65535.0;
    Some((u16::from_le_bytes([b[0], b[1]]), v(2), v(4)))
}

/// The levels in a file `catalyst-play --levels` writes, or none if it is
/// not there (nothing playing).
pub fn read(path: &std::path::Path) -> Option<(u16, f32, f32)> {
    for _ in 0..3 {
        let b = std::fs::read(path).ok()?;
        if let Some(v) = decode(&b) {
            return Some(v);
        }
    }
    None
}

/// The RMS of interleaved stereo frames, each channel.
pub fn rms(frames: &[f32]) -> (f32, f32) {
    let n = (frames.len() / 2).max(1) as f32;
    let (mut l, mut r) = (0.0f32, 0.0f32);
    for f in frames.chunks_exact(2) {
        l += f[0] * f[0];
        r += f[1] * f[1];
    }
    ((l / n).sqrt(), (r / n).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_written_and_read_back() {
        let b = encode(7, 0.5, 1.5);
        let (seq, l, r) = decode(&b).unwrap();
        assert_eq!(seq, 7);
        assert!((l - 0.5).abs() < 1e-4 && r == 1.0);
        let mut torn = b;
        torn[6] = 9;
        assert_eq!(decode(&torn), None);
        let (l, r) = rms(&[0.5, 0.0, -0.5, 0.0]);
        assert!((l - 0.5).abs() < 1e-6 && r == 0.0);
    }
}
