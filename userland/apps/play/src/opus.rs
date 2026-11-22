//! Opus, decoded by libopus (loaded when the player starts; symphonia has
//! no Opus decoder of its own). YouTube's songs are Opus nearly always.

/// Samples (at 48 kHz, each channel) in one Opus packet, from its first
/// byte (RFC 6716 § 3.1): the frame length its configuration says, times
/// the frames it holds.
pub fn packet_samples(packet: &[u8]) -> u32 {
    let Some(&toc) = packet.first() else { return 0 };
    let config = toc >> 3;
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][(config % 4) as usize],
        12..=15 => [480, 960][(config % 2) as usize],
        _ => [120, 240, 480, 960][(config % 4) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => packet.get(1).map_or(0, |b| u32::from(b & 0x3f)),
    };
    frame * frames
}

/// The most one packet can hold: 120 ms.
const MAX_FRAME: usize = 5760;

pub struct Opus {
    dec: *mut std::ffi::c_void,
    lib: Lib,
    channels: usize,
    /// Samples still to leave out at the start (the encoder's look-ahead).
    pre_skip: usize,
    pcm: Vec<i16>,
}

#[derive(Clone, Copy)]
struct Lib {
    create: unsafe extern "C" fn(i32, i32, *mut i32) -> *mut std::ffi::c_void,
    decode: unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, i32, *mut i16, i32, i32) -> i32,
    destroy: unsafe extern "C" fn(*mut std::ffi::c_void),
}

#[cfg(target_os = "linux")]
fn lib() -> Result<Lib, String> {
    unsafe {
        let mut h = libc::dlopen(c"libopus.so.0".as_ptr(), libc::RTLD_NOW);
        if h.is_null() {
            h = libc::dlopen(c"libopus.so".as_ptr(), libc::RTLD_NOW);
        }
        if h.is_null() {
            return Err("no Opus library (libopus.so.0)".into());
        }
        let sym = |name: &std::ffi::CStr| {
            let p = libc::dlsym(h, name.as_ptr());
            if p.is_null() {
                Err(format!("libopus has no {}", name.to_string_lossy()))
            } else {
                Ok(p)
            }
        };
        Ok(Lib {
            create: std::mem::transmute::<*mut libc::c_void, _>(sym(c"opus_decoder_create")?),
            decode: std::mem::transmute::<*mut libc::c_void, _>(sym(c"opus_decode")?),
            destroy: std::mem::transmute::<*mut libc::c_void, _>(sym(c"opus_decoder_destroy")?),
        })
    }
}

#[cfg(not(target_os = "linux"))]
fn lib() -> Result<Lib, String> {
    Err("Opus is played only on Linux".into())
}

impl Opus {
    pub fn new(channels: usize, pre_skip: usize) -> Result<Opus, String> {
        let channels = channels.clamp(1, 2);
        let lib = lib()?;
        let mut err = 0;
        let dec = unsafe { (lib.create)(48_000, channels as i32, &mut err) };
        if dec.is_null() || err != 0 {
            return Err(format!("libopus would not start ({err})"));
        }
        Ok(Opus { dec, lib, channels, pre_skip, pcm: vec![0; MAX_FRAME * channels] })
    }

    /// One packet's sound appended to `out`; its channels returned.
    pub fn decode(&mut self, data: &[u8], out: &mut Vec<i16>) -> Result<usize, String> {
        let n = unsafe { (self.lib.decode)(self.dec, data.as_ptr(), data.len() as i32, self.pcm.as_mut_ptr(), MAX_FRAME as i32, 0) };
        if n < 0 {
            // A damaged packet: left out, the rest plays.
            return Ok(self.channels);
        }
        let n = n as usize;
        let skip = self.pre_skip.min(n);
        self.pre_skip -= skip;
        out.extend_from_slice(&self.pcm[skip * self.channels..n * self.channels]);
        Ok(self.channels)
    }

    /// Fresh, after a seek.
    pub fn reset(&mut self) {
        let mut err = 0;
        let fresh = unsafe { (self.lib.create)(48_000, self.channels as i32, &mut err) };
        if !fresh.is_null() && err == 0 {
            unsafe { (self.lib.destroy)(self.dec) };
            self.dec = fresh;
        }
    }
}

impl Drop for Opus {
    fn drop(&mut self) {
        unsafe { (self.lib.destroy)(self.dec) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packets_length_is_read_from_its_first_byte() {
        // YouTube's: CELT fullband 20 ms (config 31), one frame.
        assert_eq!(packet_samples(&[31 << 3]), 960);
        // SILK 60 ms, two frames.
        assert_eq!(packet_samples(&[(3 << 3) | 1]), 5760);
        // Hybrid 10 ms, a count of three in its second byte.
        assert_eq!(packet_samples(&[(12 << 3) | 3, 3]), 1440);
        assert_eq!(packet_samples(&[]), 0);
    }
}
