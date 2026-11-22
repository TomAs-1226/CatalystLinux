//! Where speech comes from: the handheld's microphone, or on a computer
//! without one, a WAV file or a test signal, all behind [`Source`].

use std::io::Read;
use std::process::{ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Procs, Resampler, WavStream, RATE};

/// Sound arriving as 16 kHz mono.
pub trait Source: Send {
    /// Wait for more sound and add it to `out`. False once there is no more
    /// (the program was stopped, the file ran out).
    fn read(&mut self, out: &mut Vec<i16>) -> Result<bool, String>;
}

// ------------------------------------------------------------ microphone ---

/// ALSA's capture device through `arecord`, writing a WAV stream to a pipe.
/// Asked for 16 kHz mono first; a device that only gives its own rate and
/// channels (dsnoop without a plug in front) is asked for those, and the
/// WAV header says what came, which is turned into 16 kHz mono here.
pub struct Arecord {
    stdout: ChildStdout,
    wav: WavStream,
    conv: Resampler,
    raw: Vec<i16>,
    buf: Vec<u8>,
}

/// Shapes asked of the device in turn: what speech wants, then what
/// dsnoop and the codec run at.
const SHAPES: [(u16, u32); 3] = [(1, RATE), (2, 48_000), (2, 44_100)];

impl Arecord {
    /// Start listening on `device` (ALSA's `default`). The program joins
    /// `procs`: stopping them stops it, and the codec's input powers down.
    pub fn start(device: &str, procs: &Procs) -> Result<Self, String> {
        let mut why = String::new();
        for (channels, rate) in SHAPES {
            match Self::start_as(device, channels, rate, procs) {
                Ok(me) => return Ok(me),
                // No program at all: no other shape will help.
                Err(e) if e.contains("not on this handheld") => return Err(e),
                Err(e) => why = e,
            }
        }
        Err(why)
    }

    fn start_as(device: &str, channels: u16, rate: u32, procs: &Procs) -> Result<Self, String> {
        let mut child = Command::new("arecord")
            // A period of 20 ms, so the meter moves with the voice.
            .args(["-q", "-D", device, "-t", "wav", "-f", "S16_LE", "-c", &channels.to_string(), "-r", &rate.to_string(), "-F", "20000", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "arecord is not on this handheld".to_owned())?;
        let stdout = child.stdout.take().ok_or("the microphone did not open")?;
        let mut stderr = child.stderr.take();
        procs.add(child);
        let mut me = Arecord { stdout, wav: WavStream::default(), conv: Resampler::new(RATE, 1, RATE), raw: Vec::new(), buf: vec![0; 4096] };
        // Its header comes at once; a device that cannot give this shape
        // ends instead, saying why.
        while me.wav.format().is_none() {
            let n = me.stdout.read(&mut me.buf).map_err(|_| "the microphone stopped".to_owned())?;
            if n == 0 {
                let mut said = String::new();
                if let Some(e) = stderr.as_mut() {
                    e.read_to_string(&mut said).ok();
                }
                procs.reap();
                return Err(mic_failure(&said));
            }
            me.wav.push(&me.buf[..n], &mut me.raw)?;
        }
        // From here what it says (an overrun now and then) is let go, so a
        // full pipe can never hold it up.
        if let Some(mut e) = stderr {
            std::thread::Builder::new()
                .name("arecord-stderr".into())
                .stack_size(32 * 1024)
                .spawn(move || {
                    std::io::copy(&mut e, &mut std::io::sink()).ok();
                })
                .ok();
        }
        let format = me.wav.format().expect("read above");
        me.conv = Resampler::new(format.rate, format.channels, RATE);
        Ok(me)
    }
}

/// Why the microphone would not open, in a few words.
fn mic_failure(stderr: &str) -> String {
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    // "arecord: main:850: audio open error: No such file or directory"
    let reason = line.rsplit(": ").next().unwrap_or(line).trim();
    if reason.is_empty() {
        "the microphone did not open".into()
    } else {
        format!("the microphone did not open: {}", reason.to_lowercase())
    }
}

impl Source for Arecord {
    fn read(&mut self, out: &mut Vec<i16>) -> Result<bool, String> {
        if self.raw.is_empty() {
            let n = self.stdout.read(&mut self.buf).map_err(|_| "the microphone stopped".to_owned())?;
            if n == 0 {
                self.conv.finish(out);
                return Ok(false);
            }
            self.wav.push(&self.buf[..n], &mut self.raw)?;
        }
        self.conv.push(&self.raw, out);
        self.raw.clear();
        Ok(true)
    }
}

// -------------------------------------------------- stand-ins (computers) ---

/// Sound given out as fast as a microphone would: 20 ms at a time, in step
/// with the clock. A WAV file (then silence, for as long as it is listened
/// to), or a test signal that rises and falls as speech does.
pub struct Paced {
    kind: Kind,
    sent: u64,
    started: Option<Instant>,
}

enum Kind {
    File(Vec<i16>),
    Signal,
}

const CHUNK: usize = RATE as usize / 50;

impl Paced {
    /// A WAV file, any rate and channels.
    pub fn wav_file(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Paced { kind: Kind::File(crate::read_wav(&bytes)?), sent: 0, started: None })
    }

    /// Syllables and pauses at speech's pace, from a voiced buzz.
    pub fn test_signal() -> Self {
        Paced { kind: Kind::Signal, sent: 0, started: None }
    }
}

/// The test signal's loudness at sample `n`: words of a few syllables,
/// with pauses between them, the same every time.
fn envelope(n: u64) -> f32 {
    let t = n as f32 / RATE as f32;
    // Words 0.9 s apart: syllables of 0.21 s in the first 0.68 s.
    let word = (t / 0.9).floor();
    let within = t - word * 0.9;
    let syllables = 2.0 + (word * 7.3).sin().abs() * 2.0;
    if within > syllables * 0.17 + 0.02 {
        return 0.0;
    }
    let s = (within / 0.17).fract();
    let shape = (s * std::f32::consts::PI).sin().powf(0.8);
    let stress = 0.55 + 0.45 * ((word + (within / 0.17).floor()) * 2.1).sin().abs();
    shape * stress
}

impl Source for Paced {
    fn read(&mut self, out: &mut Vec<i16>) -> Result<bool, String> {
        let started = *self.started.get_or_insert_with(Instant::now);
        let due = Duration::from_secs_f64((self.sent + CHUNK as u64) as f64 / f64::from(RATE));
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
        let from = self.sent as usize;
        match &self.kind {
            Kind::File(samples) => out.extend((from..from + CHUNK).map(|i| samples.get(i).copied().unwrap_or(0))),
            Kind::Signal => out.extend((from..from + CHUNK).map(signal)),
        }
        self.sent += CHUNK as u64;
        Ok(true)
    }
}

/// Sample `i` of the test signal: a pitch wandering around 140 Hz with its
/// harmonics, shaped into syllables, over a little room noise.
fn signal(i: usize) -> i16 {
    let t = i as f32 / RATE as f32;
    let f0 = 140.0 + 25.0 * (t * 1.7).sin();
    let phase = 2.0 * std::f32::consts::PI * f0 * t;
    let voiced: f32 = (1..6).map(|h| (phase * h as f32).sin() / h as f32).sum();
    let noise = ((i as u32).wrapping_mul(2_654_435_761) >> 16) as f32 / 65_536.0 - 0.5;
    (voiced * 4200.0 * envelope(i as u64) + noise * 60.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_signal_speaks_in_words_at_speech_pace() {
        let mut mic = Paced::test_signal();
        let mut out = Vec::new();
        let started = Instant::now();
        for _ in 0..15 {
            assert!(mic.read(&mut out).unwrap());
        }
        assert_eq!(out.len(), 15 * CHUNK);
        assert!(started.elapsed() >= Duration::from_millis(280), "in step with the clock, not all at once");
        assert_eq!(&out[..CHUNK], &(0..CHUNK).map(signal).collect::<Vec<_>>()[..]);
        // Over three seconds it is loud in syllables and quiet between words,
        // as a meter of speech is.
        let levels: Vec<f32> = (0..150).map(|k| crate::level(&(k * CHUNK..(k + 1) * CHUNK).map(signal).collect::<Vec<_>>())).collect();
        let loud = levels.iter().filter(|l| **l > 0.5).count();
        let quiet = levels.iter().filter(|l| **l < crate::SILENT_BELOW).count();
        assert!(loud > 30 && quiet > 20, "{loud} loud, {quiet} quiet");
    }

    #[test]
    fn a_wav_file_is_heard_then_silence() {
        let path = std::env::temp_dir().join(format!("speech-paced-{}.wav", std::process::id()));
        std::fs::write(&path, crate::wav_bytes(&[1000; 400], RATE)).unwrap();
        let mut file = Paced::wav_file(&path).unwrap();
        let mut out = Vec::new();
        file.read(&mut out).unwrap();
        file.read(&mut out).unwrap();
        assert_eq!(&out[..400], &[1000; 400][..]);
        assert!(out[400..].iter().all(|s| *s == 0));
        std::fs::remove_file(&path).ok();
        assert!(Paced::wav_file(std::path::Path::new("no-such.wav")).is_err());
    }

    /// A stand-in `arecord` that behaves as the handheld's would with a
    /// dsnoop that only runs at 48 kHz stereo: it refuses mono, then gives
    /// half a second of sound and keeps listening until it is stopped.
    #[cfg(unix)]
    #[test]
    fn the_microphone_is_asked_for_a_shape_it_gives_and_stops_when_told() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("speech-fake-arecord-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("arecord");
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$*\" in *\"-c 1 \"*) echo 'arecord: set_params:1352: Channels count non available' >&2; exit 1;; esac\n\
             printf 'RIFF\\377\\377\\377\\177WAVEfmt \\020\\000\\000\\000\\001\\000\\002\\000\\200\\273\\000\\000\\000\\356\\002\\000\\004\\000\\020\\000data\\377\\377\\377\\177'\n\
             head -c 96000 /dev/zero\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", dir.display()));
        let procs = Procs::default();
        let mut mic = Arecord::start("default", &procs).expect("the second shape is taken");
        let mut out = Vec::new();
        while out.len() < 7900 {
            assert!(mic.read(&mut out).unwrap());
        }
        assert!(out.len() <= 8100, "half a second at 16 kHz: {}", out.len());
        assert!(!procs.is_empty(), "still listening");
        let stopped = Instant::now();
        procs.kill_all();
        while mic.read(&mut out).unwrap() {}
        assert!(stopped.elapsed() < Duration::from_millis(600), "stopped at once: {:?}", stopped.elapsed());
        std::thread::sleep(Duration::from_millis(100));
        assert!(procs.is_empty(), "and reaped");
        std::env::set_var("PATH", path);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_microphone_that_will_not_open_says_why() {
        assert_eq!(mic_failure("arecord: main:850: audio open error: No such file or directory\n"), "the microphone did not open: no such file or directory");
        assert_eq!(mic_failure(""), "the microphone did not open");
    }
}
