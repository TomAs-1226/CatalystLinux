//! Where the sound goes: ALSA (libasound, loaded when the player starts,
//! so building needs no ALSA), or standard output.
//!
//! Samples arrive as interleaved stereo `i32`, full scale (a 16-bit
//! sample in the top 16 bits), and are written in the format the device
//! was opened with: 16-bit, 24-bit in three bytes, or 32-bit. A 16- or
//! 24-bit song to a device opened at its own depth goes out exactly as it
//! was decoded.

use std::io::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    S16,
    /// 24 bits in three bytes, as most USB audio devices take 24-bit.
    S24In3,
    S32,
}

impl Format {
    pub fn bits(self) -> u32 {
        match self {
            Format::S16 => 16,
            Format::S24In3 => 24,
            Format::S32 => 32,
        }
    }

    fn bytes(self) -> usize {
        match self {
            Format::S16 => 2,
            Format::S24In3 => 3,
            Format::S32 => 4,
        }
    }

    /// Frames as the device takes them.
    pub fn pack(self, frames: &[i32], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(frames.len() * self.bytes());
        match self {
            Format::S16 => frames.iter().for_each(|&s| out.extend_from_slice(&((s >> 16) as i16).to_le_bytes())),
            Format::S24In3 => frames.iter().for_each(|&s| out.extend_from_slice(&(s >> 8).to_le_bytes()[..3])),
            Format::S32 => frames.iter().for_each(|&s| out.extend_from_slice(&s.to_le_bytes())),
        }
    }
}

pub trait Out {
    /// Interleaved stereo; blocks while the device's buffer is full.
    fn write(&mut self, frames: &[i32]) -> Result<(), String>;
    /// Times the sound ran dry so far.
    fn xruns(&self) -> u32;
    /// The rest played out.
    fn drain(&mut self);
    fn rate(&self) -> u32;
    fn format(&self) -> Format;
}

/// Standard output, as raw S16LE at the rate asked for.
pub struct Raw {
    out: std::io::BufWriter<std::io::Stdout>,
    rate: u32,
    bytes: Vec<u8>,
}

impl Raw {
    pub fn new(rate: u32) -> Raw {
        Raw { out: std::io::BufWriter::with_capacity(32 * 1024, std::io::stdout()), rate, bytes: Vec::new() }
    }
}

impl Out for Raw {
    fn write(&mut self, frames: &[i32]) -> Result<(), String> {
        Format::S16.pack(frames, &mut self.bytes);
        self.out.write_all(&self.bytes).map_err(|_| "the output closed".to_owned())
    }
    fn xruns(&self) -> u32 {
        0
    }
    fn drain(&mut self) {
        self.out.flush().ok();
    }
    fn rate(&self) -> u32 {
        self.rate
    }
    fn format(&self) -> Format {
        Format::S16
    }
}

#[cfg(target_os = "linux")]
pub use linux::{jack_in, Alsa};

#[cfg(target_os = "linux")]
mod linux {
    use super::Format;
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr, CString};

    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
    type One = unsafe extern "C" fn(*mut c_void) -> c_int;
    type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

    const STREAM_PLAYBACK: c_int = 0;
    const ACCESS_RW_INTERLEAVED: c_int = 3;
    /// What ALSA is asked to buffer, µs (dmix's own buffer decides, 341 ms).
    const LATENCY_US: c_uint = 340_000;

    fn alsa_format(f: Format) -> c_int {
        match f {
            Format::S16 => 2,     // SND_PCM_FORMAT_S16_LE
            Format::S24In3 => 32, // SND_PCM_FORMAT_S24_3LE
            Format::S32 => 10,    // SND_PCM_FORMAT_S32_LE
        }
    }

    pub struct Alsa {
        pcm: *mut c_void,
        writei: Writei,
        recover: Recover,
        drain: One,
        close: One,
        xruns: u32,
        rate: u32,
        format: Format,
        bytes: Vec<u8>,
    }

    unsafe fn symbol<T>(lib: *mut c_void, name: &str) -> Result<T, String> {
        let c = CString::new(name).unwrap();
        let p = libc::dlsym(lib, c.as_ptr());
        if p.is_null() {
            return Err(format!("libasound has no {name}"));
        }
        Ok(std::mem::transmute_copy(&p))
    }

    fn library() -> Result<*mut c_void, String> {
        let lib = unsafe { libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW) };
        if lib.is_null() {
            return Err("no ALSA library (libasound.so.2)".into());
        }
        Ok(lib)
    }

    impl Alsa {
        /// `device` at `rate` in `format`. `exact`: the device's own rate and
        /// format or nothing (a `hw` device, for sound bit for bit); else
        /// ALSA converts as it must.
        pub fn open(device: &str, rate: u32, format: Format, exact: bool) -> Result<Alsa, String> {
            unsafe {
                let lib = library()?;
                let open: Open = symbol(lib, "snd_pcm_open")?;
                let set_params: SetParams = symbol(lib, "snd_pcm_set_params")?;
                let strerror: StrError = symbol(lib, "snd_strerror")?;
                let words = |e: c_int| CStr::from_ptr(strerror(e)).to_string_lossy().into_owned();
                let mut pcm: *mut c_void = std::ptr::null_mut();
                let name = CString::new(device).map_err(|_| "a bad device name".to_owned())?;
                let e = open(&mut pcm, name.as_ptr(), STREAM_PLAYBACK, 0);
                if e < 0 {
                    return Err(if e == -libc::EBUSY { "the sound device is busy".into() } else { format!("the sound device: {}", words(e)) });
                }
                let close: One = symbol(lib, "snd_pcm_close")?;
                let e = set_params(pcm, alsa_format(format), ACCESS_RW_INTERLEAVED, 2, rate, if exact { 0 } else { 1 }, LATENCY_US);
                if e < 0 {
                    close(pcm);
                    return Err(format!("the sound device would not take {rate} Hz, {}-bit stereo: {}", format.bits(), words(e)));
                }
                Ok(Alsa { pcm, writei: symbol(lib, "snd_pcm_writei")?, recover: symbol(lib, "snd_pcm_recover")?, drain: symbol(lib, "snd_pcm_drain")?, close, xruns: 0, rate, format, bytes: Vec::new() })
            }
        }
    }

    impl super::Out for Alsa {
        fn write(&mut self, frames: &[i32]) -> Result<(), String> {
            self.format.pack(&frames[..frames.len() / 2 * 2], &mut self.bytes);
            let frame_bytes = self.format.bytes() * 2;
            let mut at = 0;
            while at < self.bytes.len() {
                let rest = &self.bytes[at..];
                let r = unsafe { (self.writei)(self.pcm, rest.as_ptr().cast(), (rest.len() / frame_bytes) as c_ulong) };
                if r >= 0 {
                    at += r as usize * frame_bytes;
                    continue;
                }
                let e = r as c_int;
                if e == -libc::EPIPE {
                    self.xruns += 1;
                }
                // Ran dry, suspended or interrupted: started again where it was.
                if unsafe { (self.recover)(self.pcm, e, 1) } < 0 {
                    return Err("the sound device stopped".into());
                }
            }
            Ok(())
        }

        fn xruns(&self) -> u32 {
            self.xruns
        }

        fn drain(&mut self) {
            unsafe {
                (self.drain)(self.pcm);
            }
        }

        fn rate(&self) -> u32 {
            self.rate
        }

        fn format(&self) -> Format {
            self.format
        }
    }

    impl Drop for Alsa {
        fn drop(&mut self) {
            unsafe {
                (self.close)(self.pcm);
            }
        }
    }

    type CtlOpen = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int) -> c_int;
    type ValueMalloc = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
    type SetInterface = unsafe extern "C" fn(*mut c_void, c_int);
    type SetName = unsafe extern "C" fn(*mut c_void, *const c_char);
    type ElemRead = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
    type GetBoolean = unsafe extern "C" fn(*const c_void, c_uint) -> c_int;
    type Free = unsafe extern "C" fn(*mut c_void);

    /// Whether headphones are in the card's jack, as its "Headphones Jack"
    /// control says (the kernel's jack detection); `None` if it cannot tell.
    pub fn jack_in(card: &str) -> Option<bool> {
        unsafe {
            let lib = library().ok()?;
            let open: CtlOpen = symbol(lib, "snd_ctl_open").ok()?;
            let close: One = symbol(lib, "snd_ctl_close").ok()?;
            let malloc: ValueMalloc = symbol(lib, "snd_ctl_elem_value_malloc").ok()?;
            let free: Free = symbol(lib, "snd_ctl_elem_value_free").ok()?;
            let set_interface: SetInterface = symbol(lib, "snd_ctl_elem_value_set_interface").ok()?;
            let set_name: SetName = symbol(lib, "snd_ctl_elem_value_set_name").ok()?;
            let read: ElemRead = symbol(lib, "snd_ctl_elem_read").ok()?;
            let get: GetBoolean = symbol(lib, "snd_ctl_elem_value_get_boolean").ok()?;
            let name = CString::new(format!("hw:{card}")).ok()?;
            let mut ctl: *mut c_void = std::ptr::null_mut();
            if open(&mut ctl, name.as_ptr(), 0) < 0 {
                return None;
            }
            let mut v: *mut c_void = std::ptr::null_mut();
            if malloc(&mut v) < 0 {
                close(ctl);
                return None;
            }
            let mut answer = None;
            // A jack's control is the card's (SND_CTL_ELEM_IFACE_CARD), or a mixer's.
            for iface in [0, 2] {
                set_interface(v, iface);
                set_name(v, c"Headphones Jack".as_ptr());
                if read(ctl, v) >= 0 {
                    answer = Some(get(v, 0) != 0);
                    break;
                }
            }
            free(v);
            close(ctl);
            answer
        }
    }
}

/// Elsewhere (a computer's own build, for tests): no ALSA to play to.
#[cfg(not(target_os = "linux"))]
pub struct Alsa;

#[cfg(not(target_os = "linux"))]
impl Alsa {
    pub fn open(_device: &str, _rate: u32, _format: Format, _exact: bool) -> Result<Alsa, String> {
        Err("ALSA is only on Linux: use --raw".into())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn jack_in(_card: &str) -> Option<bool> {
    None
}

#[cfg(not(target_os = "linux"))]
impl Out for Alsa {
    fn write(&mut self, _frames: &[i32]) -> Result<(), String> {
        Ok(())
    }
    fn xruns(&self) -> u32 {
        0
    }
    fn drain(&mut self) {}
    fn rate(&self) -> u32 {
        48_000
    }
    fn format(&self) -> Format {
        Format::S16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_packed_bit_for_bit_at_each_depth() {
        // A 16-bit sample, and a 24-bit one, as the decoder gives them.
        let s16 = 0x1234i32 << 16;
        let s24 = 0x123456i32 << 8;
        let mut out = Vec::new();
        Format::S16.pack(&[s16, -65536], &mut out);
        assert_eq!(out, [0x34, 0x12, 0xFF, 0xFF]);
        Format::S24In3.pack(&[s24], &mut out);
        assert_eq!(out, [0x56, 0x34, 0x12]);
        Format::S24In3.pack(&[s16], &mut out);
        assert_eq!(out, [0x00, 0x34, 0x12]);
        Format::S32.pack(&[s24], &mut out);
        assert_eq!(out, [0x00, 0x56, 0x34, 0x12]);
    }
}
