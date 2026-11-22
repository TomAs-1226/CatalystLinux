//! Where the sound goes: ALSA (libasound, loaded when the player starts,
//! so building needs no ALSA), or standard output.

use std::io::Write;

pub trait Out {
    /// Interleaved stereo at 48 kHz; blocks while ALSA's buffer is full.
    fn write(&mut self, pcm: &[i16]) -> Result<(), String>;
    /// Times the sound ran dry so far.
    fn xruns(&self) -> u32;
    /// The rest played out.
    fn drain(&mut self);
}

/// Standard output, as raw S16LE.
pub struct Raw {
    out: std::io::BufWriter<std::io::Stdout>,
}

impl Raw {
    pub fn new() -> Raw {
        Raw { out: std::io::BufWriter::with_capacity(32 * 1024, std::io::stdout()) }
    }
}

impl Out for Raw {
    fn write(&mut self, pcm: &[i16]) -> Result<(), String> {
        let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.out.write_all(&bytes).map_err(|_| "the output closed".to_owned())
    }
    fn xruns(&self) -> u32 {
        0
    }
    fn drain(&mut self) {
        self.out.flush().ok();
    }
}

#[cfg(target_os = "linux")]
pub use linux::Alsa;

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr, CString};

    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
    type One = unsafe extern "C" fn(*mut c_void) -> c_int;
    type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

    const STREAM_PLAYBACK: c_int = 0;
    const FORMAT_S16_LE: c_int = 2;
    const ACCESS_RW_INTERLEAVED: c_int = 3;
    /// What ALSA is asked to buffer, µs (dmix's own buffer decides, 341 ms).
    const LATENCY_US: c_uint = 340_000;

    pub struct Alsa {
        pcm: *mut c_void,
        writei: Writei,
        recover: Recover,
        drain: One,
        close: One,
        xruns: u32,
    }

    unsafe fn symbol<T>(lib: *mut c_void, name: &str) -> Result<T, String> {
        let c = CString::new(name).unwrap();
        let p = libc::dlsym(lib, c.as_ptr());
        if p.is_null() {
            return Err(format!("libasound has no {name}"));
        }
        Ok(std::mem::transmute_copy(&p))
    }

    impl Alsa {
        pub fn open(device: &str, rate: u32) -> Result<Alsa, String> {
            unsafe {
                let lib = libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW);
                if lib.is_null() {
                    return Err("no ALSA library (libasound.so.2)".into());
                }
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
                let e = set_params(pcm, FORMAT_S16_LE, ACCESS_RW_INTERLEAVED, 2, rate, 1, LATENCY_US);
                if e < 0 {
                    close(pcm);
                    return Err(format!("the sound device would not take 48 kHz stereo: {}", words(e)));
                }
                Ok(Alsa { pcm, writei: symbol(lib, "snd_pcm_writei")?, recover: symbol(lib, "snd_pcm_recover")?, drain: symbol(lib, "snd_pcm_drain")?, close, xruns: 0 })
            }
        }
    }

    impl super::Out for Alsa {
        fn write(&mut self, pcm: &[i16]) -> Result<(), String> {
            let mut frames = &pcm[..pcm.len() / 2 * 2];
            while !frames.is_empty() {
                let r = unsafe { (self.writei)(self.pcm, frames.as_ptr().cast(), (frames.len() / 2) as c_ulong) };
                if r >= 0 {
                    frames = &frames[(r as usize * 2).min(frames.len())..];
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
    }

    impl Drop for Alsa {
        fn drop(&mut self) {
            unsafe {
                (self.close)(self.pcm);
            }
        }
    }
}

/// Elsewhere (a computer's own build, for tests): no ALSA to play to.
#[cfg(not(target_os = "linux"))]
pub struct Alsa;

#[cfg(not(target_os = "linux"))]
impl Alsa {
    pub fn open(_device: &str, _rate: u32) -> Result<Alsa, String> {
        Err("ALSA is only on Linux: use --raw".into())
    }
}

#[cfg(not(target_os = "linux"))]
impl Out for Alsa {
    fn write(&mut self, _pcm: &[i16]) -> Result<(), String> {
        Ok(())
    }
    fn xruns(&self) -> u32 {
        0
    }
    fn drain(&mut self) {}
}
