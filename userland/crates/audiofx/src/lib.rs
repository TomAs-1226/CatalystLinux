//! Music's sound between the decoder and the device (catalyst-play), and
//! what the shell and the player agree on about it.
//!
//! Everything here works on interleaved stereo `f32` frames, at whatever
//! rate the device was opened at, and costs nothing while it is off: the
//! player keeps the decoder's samples as they are (bit-perfect) unless one
//! of these is on. Per frame, as counted for the CM0's A53 (one core at
//! 1 GHz, scalar floating point):
//!
//! - [`spatial`]: four head-shadowed paths with their delays, a light room
//!   and the timbre's correction: about 90 floating-point operations, some
//!   4.3 million a second at 48 kHz.
//! - [`eq`]: two biquads (five multiply-adds each) a band that is not
//!   flat; all five bands, 50 multiply-adds, 2.4 million a second.
//! - [`loudness`]: a multiply (the gain), and on a song's first play a
//!   meter of two biquads a channel (1.9 million a second).
//!
//! `cargo test -p audiofx --release -- --ignored cost` measures them, and
//! `catalyst-play --dsp-cost` on the handheld. With the screen off, music
//! plays with the processor held at its slowest (600 MHz, powersave: music
//! standby): all of these on, with an 8-second crossfade's second decoder
//! and a CD's resampling, are about 5 % of one core there, the decoders a
//! few percent more, so the sound keeps its real-time margin many times
//! over with no clock floor. (The device's release build under user-mode
//! emulation, which runs floating point slower than the A53 at 600 MHz:
//! 9.4 % of one emulated core for all of it, 1.4 % with it all off.)

pub mod biquad;
pub mod crossfade;
pub mod eq;
pub mod gapless;
pub mod head;
pub mod levels;
pub mod loudness;
pub mod settings;
pub mod spatial;

/// A level in decibels as a gain.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A gain as decibels.
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-9).log10()
}

/// The last stage before the device: unchanged below 0.9 of full scale,
/// then bent smoothly so a boost or a sum never clips harshly. Never
/// reaches 1.0.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    // Above the knee: a curve with the same slope at it, towards 1.0.
    let over = (a - KNEE) / (1.0 - KNEE);
    let bent = KNEE + (1.0 - KNEE) * (over / (1.0 + over));
    bent.copysign(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soft_clip_leaves_quiet_sound_alone_and_never_reaches_full_scale() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert_eq!(soft_clip(-0.9), -0.9);
        assert!(soft_clip(1.0) < 1.0 && soft_clip(1.0) > 0.9);
        assert!(soft_clip(10.0) < 1.0);
        assert!(soft_clip(-3.0) > -1.0);
        // Rising throughout.
        let mut last = 0.0;
        for i in 0..400 {
            let y = soft_clip(i as f32 / 100.0);
            assert!(y >= last);
            last = y;
        }
        assert!((db_to_gain(gain_to_db(0.25)) - 0.25).abs() < 1e-5);
    }
}
