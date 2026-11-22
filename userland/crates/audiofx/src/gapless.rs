//! Gapless playback's trimming: a lossy encoder adds silence before a song
//! (its delay, or Opus's pre-skip) and after it (padding to a whole frame).
//! Left in, the two make a gap between songs meant to run into each other
//! (a live album, a DJ mix). The front is skipped and the song stopped at
//! its true length, as the container says it: Ogg's granule positions and
//! MP3's LAME header (symphonia trims those), WebM's duration and Opus's
//! pre-skip, and iTunes's `iTunSMPB` tag in an MP4.

/// iTunes's gapless tag: " 00000000 00000840 000001CA 00000000003F31F6 …"
/// (hex: unused, encoder delay, padding, the song's length in frames).
pub fn itunsmpb(value: &str) -> Option<(u32, u32, u64)> {
    let mut f = value.split_whitespace();
    f.next()?;
    let delay = u32::from_str_radix(f.next()?, 16).ok()?;
    let padding = u32::from_str_radix(f.next()?, 16).ok()?;
    let frames = u64::from_str_radix(f.next()?, 16).ok()?;
    (frames > 0).then_some((delay, padding, frames))
}

/// Interleaved stereo frames with the front skipped and the song ended at
/// its length.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    skip: u64,
    left: Option<u64>,
}

impl Trim {
    /// `skip` frames off the front; then `length` frames, if it is known.
    pub fn new(skip: u64, length: Option<u64>) -> Trim {
        Trim { skip, left: length }
    }

    pub fn none() -> Trim {
        Trim { skip: 0, left: None }
    }

    /// The chunk trimmed in place. True once the song's length is reached:
    /// anything decoded after is padding.
    pub fn apply<T: Copy>(&mut self, frames: &mut Vec<T>) -> bool {
        let n = frames.len() as u64 / 2;
        if self.skip > 0 {
            let cut = self.skip.min(n);
            frames.drain(..(cut * 2) as usize);
            self.skip -= cut;
        }
        if let Some(left) = self.left.as_mut() {
            let n = frames.len() as u64 / 2;
            if n >= *left {
                frames.truncate((*left * 2) as usize);
                *left = 0;
                return true;
            }
            *left -= n;
        }
        false
    }

    pub fn ended(&self) -> bool {
        self.left == Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itunes_gapless_tags_are_read() {
        // As iTunes writes it (from an AAC file of 4,141,558 frames).
        let tag = " 00000000 00000840 000001CA 00000000003F31F6 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000";
        assert_eq!(itunsmpb(tag), Some((2112, 458, 0x3F31F6)));
        assert_eq!(itunsmpb("rubbish"), None);
        assert_eq!(itunsmpb(" 0 0 0 0"), None);
    }

    #[test]
    fn the_delay_is_skipped_and_the_padding_never_played() {
        // 5 frames of delay, a song of 7 frames, then padding.
        let mut t = Trim::new(5, Some(7));
        let mut a: Vec<i32> = (0..8).collect(); // frames 0-3
        assert!(!t.apply(&mut a));
        assert!(a.is_empty());
        let mut b: Vec<i32> = (8..20).collect(); // frames 4-9
        assert!(!t.apply(&mut b));
        assert_eq!(b, (10..20).collect::<Vec<_>>()); // frames 5-9 kept
        let mut c: Vec<i32> = (20..32).collect(); // frames 10-15
        assert!(t.apply(&mut c));
        assert_eq!(c, [20, 21, 22, 23]); // frames 10 and 11: seven in all
        assert!(t.ended());
        let mut none = Trim::none();
        let mut d = vec![1, 2, 3, 4];
        assert!(!none.apply(&mut d));
        assert_eq!(d, [1, 2, 3, 4]);
    }
}
