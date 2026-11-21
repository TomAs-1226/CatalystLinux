//! Talking to the handheld, and hearing it answer.
//!
//! - **The microphone** ([`source`]): ALSA's `default` capture device, read
//!   through `arecord` only while something listens, so the codec's input
//!   (and its microphone bias) is powered only then. Whatever rate and
//!   channels the device gives come out as 16 kHz mono. A computer without
//!   one is fed a WAV file or a test signal instead, behind the same trait.
//! - **The speaker** ([`sink`]): `aplay` on ALSA's `default`, started for one
//!   answer and gone after it, so the amplifier is powered only while it
//!   speaks.
//! - **Speech to text** ([`openai::transcribe`]): OpenAI's `gpt-transcribe`.
//! - **Text to speech**: OpenAI's `gpt-audio-1.5` ([`openai::speak`]), or
//!   espeak on the handheld itself ([`espeak::speak`]).
//!
//! Keys reach curl in a file only its owner can read, never on its command
//! line, and the sound itself goes to curl through a pipe: nothing heard is
//! written anywhere.

pub mod espeak;
pub mod openai;
pub mod sink;
pub mod source;

use std::process::Child;
use std::sync::{Arc, Mutex};

/// The rate speech is kept and sent at: what speech recognition wants.
pub const RATE: u32 = 16_000;

/// Quieter than this (dBFS) shows as an empty meter; this loud, a full one.
const FLOOR_DB: f64 = -55.0;
const CEIL_DB: f64 = -12.0;

/// How loud some sound is, 0 (silence) to 1 (loud speech close up), on a
/// scale of decibels: a meter of it reads as the ear hears.
pub fn level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum / samples.len() as f64).sqrt() / 32768.0;
    let db = 20.0 * rms.max(1e-9).log10();
    ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0) as f32
}

/// The loudest a clip gets (its loudest tenth of a second) under this, it
/// held no speech: only the room. About −50 dBFS.
pub const SILENT_BELOW: f32 = 0.12;

/// Some speech, at [`RATE`], mono.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub samples: Vec<i16>,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / RATE as f32
    }

    /// The level of its loudest tenth of a second.
    pub fn loudest(&self) -> f32 {
        self.samples.chunks(RATE as usize / 10).map(level).fold(0.0, f32::max)
    }

    /// Nothing in it but the room.
    pub fn silent(&self) -> bool {
        self.loudest() < SILENT_BELOW
    }

    /// As a WAV file (16-bit PCM), for sending.
    pub fn wav(&self) -> Vec<u8> {
        wav_bytes(&self.samples, RATE)
    }
}

/// Mono 16-bit samples as a WAV file.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

// ------------------------------------------------------------------- wav ---

/// What a WAV stream holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
}

#[derive(Debug, PartialEq)]
enum Part {
    Riff,
    Chunk,
    Skip(u64),
    Data(u64),
}

/// A WAV file read as it arrives (`arecord` and `espeak` write one to a
/// pipe, its sizes unknown): its format once the header is in, then its
/// samples, interleaved. Only 16-bit PCM, which is all either writes.
pub struct WavStream {
    pending: Vec<u8>,
    part: Part,
    format: Option<Format>,
}

impl Default for WavStream {
    fn default() -> Self {
        WavStream { pending: Vec::new(), part: Part::Riff, format: None }
    }
}

/// A data chunk this long (or the 0 some writers leave) runs to the end of the stream.
const OPEN_ENDED: u64 = 0x7fff_0000;

impl WavStream {
    pub fn format(&self) -> Option<Format> {
        self.format
    }

    /// Take in `bytes`, adding any samples they complete to `out`.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<i16>) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        let mut at = 0;
        loop {
            let rest = &self.pending[at..];
            match self.part {
                Part::Riff => {
                    if rest.len() < 12 {
                        break;
                    }
                    if &rest[..4] != b"RIFF" || &rest[8..12] != b"WAVE" {
                        return Err("not a WAV stream".into());
                    }
                    at += 12;
                    self.part = Part::Chunk;
                }
                Part::Chunk => {
                    if rest.len() < 8 {
                        break;
                    }
                    let id = [rest[0], rest[1], rest[2], rest[3]];
                    let size = u64::from(u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]));
                    if &id == b"fmt " {
                        let need = 8 + size as usize;
                        if rest.len() < need {
                            break;
                        }
                        self.format = Some(parse_format(&rest[8..need])?);
                        at += need + (size as usize & 1);
                    } else if &id == b"data" {
                        if self.format.is_none() {
                            return Err("sound before its format".into());
                        }
                        at += 8;
                        self.part = Part::Data(if size == 0 || size >= OPEN_ENDED { u64::MAX } else { size });
                    } else {
                        at += 8;
                        self.part = Part::Skip(size + (size & 1));
                    }
                }
                Part::Skip(left) => {
                    let n = (rest.len() as u64).min(left);
                    at += n as usize;
                    if n < left {
                        self.part = Part::Skip(left - n);
                        break;
                    }
                    self.part = Part::Chunk;
                }
                Part::Data(left) => {
                    let whole = (rest.len() as u64).min(left) as usize & !1;
                    out.extend(rest[..whole].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
                    at += whole;
                    let left = left.saturating_sub(whole as u64);
                    if left == 0 {
                        self.part = Part::Chunk;
                        continue;
                    }
                    self.part = Part::Data(left);
                    break;
                }
            }
        }
        self.pending.drain(..at);
        Ok(())
    }
}

fn parse_format(fmt: &[u8]) -> Result<Format, String> {
    if fmt.len() < 16 {
        return Err("a WAV format too short".into());
    }
    let tag = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    // PCM, or the extensible form of it.
    if !(tag == 1 || tag == 0xfffe) || bits != 16 || channels == 0 || rate == 0 {
        return Err(format!("sound the handheld cannot read (format {tag}, {bits} bits)"));
    }
    Ok(Format { rate, channels })
}

/// A whole WAV file, as 16 kHz mono.
pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>, String> {
    let mut wav = WavStream::default();
    let mut interleaved = Vec::new();
    wav.push(bytes, &mut interleaved)?;
    let format = wav.format().ok_or("no sound in that file")?;
    let mut out = Vec::new();
    let mut conv = Resampler::new(format.rate, format.channels, RATE);
    conv.push(&interleaved, &mut out);
    conv.finish(&mut out);
    Ok(out)
}

// -------------------------------------------------------------- resample ---

/// Sound from one rate and number of channels to another rate, mono: the
/// channels averaged, a low-pass (a windowed sinc, so nothing above the new
/// rate's half folds back as a whine), then read off at the new rate.
pub struct Resampler {
    channels: usize,
    /// Input samples per output sample.
    step: f64,
    taps: Vec<f32>,
    /// Mono input not yet passed, and where the next output falls in it.
    buf: Vec<f32>,
    next: f64,
    same: bool,
    /// Part of a frame (one channel of two) that a read ended on.
    carry: Vec<i16>,
}

/// The filter's length: steep enough that a whine above the new rate's half
/// is gone (about 70 dB down), and still only a couple of million sums a
/// second on the A53.
const TAPS: usize = 63;

impl Resampler {
    pub fn new(from: u32, channels: u16, to: u32) -> Self {
        let from = from.max(1);
        let to = to.max(1);
        // Cut under half the lower rate (6.7 kHz on the way to 16 kHz: speech
        // needs no more), as a fraction of the input's.
        let cutoff = 0.42 * f64::from(from.min(to)) / f64::from(from);
        let half = (TAPS / 2) as f64;
        let mut taps: Vec<f32> = (0..TAPS)
            .map(|i| {
                let x = i as f64 - half;
                let sinc = if x == 0.0 { 2.0 * cutoff } else { (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x) };
                // Blackman window.
                let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64).cos() + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64).cos();
                (sinc * w) as f32
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        for t in &mut taps {
            *t /= sum;
        }
        Resampler { channels: usize::from(channels.max(1)), step: f64::from(from) / f64::from(to), taps, buf: Vec::new(), next: 0.0, same: from == to, carry: Vec::new() }
    }

    /// Take in interleaved samples; add what they make at the new rate to `out`.
    pub fn push(&mut self, interleaved: &[i16], out: &mut Vec<i16>) {
        let ch = self.channels;
        if self.same && ch == 1 {
            out.extend_from_slice(interleaved);
            return;
        }
        // Whole frames only: a frame split between two reads waits for its other half.
        let mut frames = std::mem::take(&mut self.carry);
        frames.extend_from_slice(interleaved);
        let whole = frames.len() / ch * ch;
        for frame in frames[..whole].chunks_exact(ch) {
            self.buf.push(frame.iter().map(|&s| f32::from(s)).sum::<f32>() / ch as f32);
        }
        self.carry = frames.split_off(whole);
        if self.same {
            out.extend(self.buf.drain(..).map(to_i16));
            return;
        }
        let half = TAPS / 2;
        loop {
            let i = self.next.floor() as usize;
            if i + 1 + half >= self.buf.len() {
                break;
            }
            let frac = (self.next - i as f64) as f32;
            let (a, b) = (self.filtered(i), self.filtered(i + 1));
            out.push(to_i16(a + (b - a) * frac));
            self.next += self.step;
        }
        // Keep only what the next outputs still reach back to.
        let keep_from = (self.next.floor() as usize).saturating_sub(half).min(self.buf.len());
        self.buf.drain(..keep_from);
        self.next -= keep_from as f64;
    }

    /// The last of it, as if silence followed.
    pub fn finish(&mut self, out: &mut Vec<i16>) {
        if !self.same {
            let pad = vec![0i16; (TAPS + 1) * self.channels];
            self.push(&pad, out);
        }
    }

    fn filtered(&self, i: usize) -> f32 {
        let half = TAPS / 2;
        self.taps
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let j = i as isize + k as isize - half as isize;
                if j < 0 {
                    0.0
                } else {
                    t * self.buf[j as usize]
                }
            })
            .sum()
    }
}

fn to_i16(x: f32) -> i16 {
    x.round().clamp(-32768.0, 32767.0) as i16
}

// ---------------------------------------------------------------- base64 ---

/// Standard base64 (with or without padding) back to bytes. OpenAI sends
/// the sound it speaks so.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | value(c).ok_or("not base64")?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// ----------------------------------------------------------- processes ---

/// The programs one listening or one answer started, to stop together: the
/// moment they go, ALSA lets go of the codec and it powers down.
#[derive(Clone, Default)]
pub struct Procs(Arc<Mutex<Vec<Child>>>);

impl Procs {
    pub fn add(&self, child: Child) {
        self.lock().push(child);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Child>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stop every one of them now. Asked to end (SIGTERM), `arecord` and
    /// `aplay` close the sound device as they go, so ALSA (dsnoop and dmix
    /// among clients) is left tidy and the codec powers down; whatever has
    /// not ended a moment later is killed. Never waits: a thread sees to it.
    pub fn kill_all(&self) {
        #[cfg(unix)]
        {
            let mut asked = false;
            for child in self.lock().iter_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    // SAFETY: a signal to a child this process started and has not reaped.
                    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
                    asked = true;
                }
            }
            if asked {
                let procs = self.clone();
                std::thread::Builder::new()
                    .name("speech-stop".into())
                    .stack_size(32 * 1024)
                    .spawn(move || {
                        let until = std::time::Instant::now() + std::time::Duration::from_millis(400);
                        while !procs.lock().iter_mut().all(|c| !matches!(c.try_wait(), Ok(None))) && std::time::Instant::now() < until {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        for child in procs.lock().iter_mut() {
                            child.kill().ok();
                        }
                        procs.reap();
                    })
                    .ok();
            }
            self.reap();
        }
        #[cfg(not(unix))]
        {
            for child in self.lock().iter_mut() {
                child.kill().ok();
            }
            self.reap();
        }
    }

    /// Collect those that have ended (no zombies left behind).
    pub fn reap(&self) {
        self.lock().retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_)) | Err(_)));
    }

    /// Wait for them all to end on their own (an answer playing out).
    pub fn wait_all(&self, stop: &std::sync::atomic::AtomicBool) {
        loop {
            self.reap();
            if self.lock().is_empty() {
                return;
            }
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return self.kill_all();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.reap();
        self.lock().is_empty()
    }
}

/// A file only its owner can read, holding `text` (a request's headers, the
/// key among them), under a name of its own: requests at the same time never
/// share one. Removed by the caller once curl has read it.
pub fn private_file(dir: &std::path::Path, text: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(dir).ok();
    let path = dir.join(format!(".voice-request-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::remove_file(&path).ok();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|_| "could not prepare the request".to_owned())?;
    file.write_all(text.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    Ok(path)
}

/// A program on this machine's PATH, or at one of the usual places.
pub fn installed(program: &str) -> bool {
    let on_path = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file() || dir.join(format!("{program}.exe")).is_file()));
    on_path || ["/usr/bin", "/bin", "/usr/local/bin"].iter().any(|d| std::path::Path::new(d).join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, seconds: f32, amp: f32) -> Vec<i16> {
        (0..(rate as f32 * seconds) as usize).map(|i| (amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin()) as i16).collect()
    }

    #[test]
    fn levels_read_as_the_ear_hears() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0; 1600]), 0.0, "silence is an empty meter");
        let loud = level(&sine(300.0, RATE, 0.1, 16000.0));
        let speech = level(&sine(300.0, RATE, 0.1, 1200.0));
        let room = level(&sine(300.0, RATE, 0.1, 40.0));
        assert!(loud > 0.95, "{loud}");
        assert!((0.4..0.8).contains(&speech), "{speech}");
        assert!(room < SILENT_BELOW, "{room}");
        let mut clip = Clip { samples: sine(300.0, RATE, 1.0, 40.0) };
        assert!(clip.silent());
        clip.samples.extend(sine(300.0, RATE, 0.2, 2000.0));
        assert!(!clip.silent(), "a word at the end is enough");
        assert!((clip.seconds() - 1.2).abs() < 0.01);
    }

    #[test]
    fn a_wav_comes_in_whole_or_in_pieces_and_says_its_format() {
        let samples: Vec<i16> = (0..5000).map(|i| (i * 7 % 2000 - 1000) as i16).collect();
        let bytes = wav_bytes(&samples, 22_050);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        // In pieces of every awkward size, as a pipe gives them.
        for piece in [1, 3, 7, 44, 45, 1000] {
            let mut wav = WavStream::default();
            let mut out = Vec::new();
            for chunk in bytes.chunks(piece) {
                wav.push(chunk, &mut out).unwrap();
            }
            assert_eq!(wav.format(), Some(Format { rate: 22_050, channels: 1 }));
            assert_eq!(out, samples, "pieces of {piece}");
        }
        // arecord to a pipe: an open-ended data chunk after a LIST chunk.
        let mut piped = b"RIFF\xff\xff\xff\x7fWAVELIST\x04\x00\x00\x00abcdfmt ".to_vec();
        piped.extend_from_slice(&16u32.to_le_bytes());
        piped.extend_from_slice(&[1, 0, 2, 0]);
        piped.extend_from_slice(&48_000u32.to_le_bytes());
        piped.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        piped.extend_from_slice(&[4, 0, 16, 0]);
        piped.extend_from_slice(b"data\xff\xff\xff\x7f");
        piped.extend_from_slice(&[1, 0, 2, 0, 3, 0]);
        let mut wav = WavStream::default();
        let mut out = Vec::new();
        wav.push(&piped, &mut out).unwrap();
        assert_eq!(wav.format(), Some(Format { rate: 48_000, channels: 2 }));
        assert_eq!(out, [1, 2, 3]);
        assert!(WavStream::default().push(b"ID3\x04 not a wav at all", &mut Vec::new()).is_err());
    }

    #[test]
    fn any_rate_and_channels_come_out_as_16_khz_mono() {
        // 48 kHz stereo, a 440 Hz tone in both channels: a third as many
        // samples, the same tone, as loud.
        let tone = sine(440.0, 48_000, 0.5, 8000.0);
        let stereo: Vec<i16> = tone.iter().flat_map(|&s| [s, s]).collect();
        let mut conv = Resampler::new(48_000, 2, RATE);
        let mut out = Vec::new();
        // Reads that end halfway through a frame, as a pipe's do.
        for chunk in stereo.chunks(961) {
            conv.push(chunk, &mut out);
        }
        conv.finish(&mut out);
        assert!((out.len() as i64 - 8000).abs() <= 30, "{}", out.len());
        let expected = sine(440.0, RATE, 0.5, 8000.0);
        let mid = &out[400..7400];
        assert!((level(mid) - level(&expected[400..7400])).abs() < 0.02);
        // Crossing zero 880 times a second is a 440 Hz tone still.
        let crossings = mid.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f32 / (mid.len() as f32 / RATE as f32);
        assert!((crossings - 880.0).abs() < 15.0, "{crossings}");
        // A tone above the new rate's half is filtered out, not folded down
        // (12 kHz would come back as 4 kHz).
        let whine = sine(12_000.0, 48_000, 0.5, 8000.0);
        let mut conv = Resampler::new(48_000, 1, RATE);
        let mut out = Vec::new();
        conv.push(&whine, &mut out);
        assert!(level(&out[400..]) < 0.25, "{}", level(&out[400..]));
        // The same rate and mono goes straight through.
        let mut same = Resampler::new(RATE, 1, RATE);
        let mut out = Vec::new();
        same.push(&[1, 2, 3], &mut out);
        assert_eq!(out, [1, 2, 3]);
        assert_eq!(read_wav(&wav_bytes(&[5; 100], RATE)).unwrap(), vec![5; 100]);
    }

    #[test]
    fn base64_comes_back_as_its_bytes() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64_decode("AAEC/w==").unwrap(), [0, 1, 2, 255]);
        assert_eq!(base64_decode("").unwrap(), b"");
        assert!(base64_decode("a$b").is_err());
    }

    #[test]
    fn a_request_file_is_its_owners_alone_and_never_shared() {
        let dir = std::env::temp_dir().join(format!("speech-private-{}", std::process::id()));
        let a = private_file(&dir, "Authorization: Bearer sk-x\n").unwrap();
        let b = private_file(&dir, "Authorization: Bearer sk-y\n").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "Authorization: Bearer sk-x\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
