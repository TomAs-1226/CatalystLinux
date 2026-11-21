//! Where speech goes: the handheld's speaker, or on a computer, nowhere,
//! in the time it would take, all behind [`Sink`].

use std::io::{BufRead, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{Procs, Resampler};

/// Mono sound played as it is given.
pub trait Sink: Send {
    /// Play these (at the rate the sink was made for), waiting while the
    /// device is full.
    fn write(&mut self, samples: &[i16]) -> Result<(), String>;
    /// No more is coming: what was given plays out.
    fn finish(&mut self);
}

/// ALSA's playback device through `aplay`, fed raw 16-bit mono at the
/// rate the voice speaks at. If the device takes another rate as it is
/// (no plug in front of dmix), aplay says so as it starts, and what follows
/// is turned into that rate here.
pub struct Aplay {
    stdin: Option<ChildStdin>,
    rate: u32,
    /// The rate the device actually runs at, as aplay reported it.
    actual: Arc<AtomicU32>,
    conv: Option<(u32, Resampler)>,
    bytes: Vec<u8>,
}

impl Aplay {
    /// Start playing on `device` (ALSA's `default`) at `rate`. The program
    /// joins `procs`: stopping them silences it at once.
    pub fn start(device: &str, rate: u32, procs: &Procs) -> Result<Self, String> {
        Self::start_program("aplay", device, rate, procs)
    }

    fn start_program(program: &str, device: &str, rate: u32, procs: &Procs) -> Result<Self, String> {
        // Not -q: it would keep the "rate is not accurate" warning to itself.
        let mut child = Command::new(program)
            .args(["-D", device, "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate.to_string(), "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "aplay is not on this handheld".to_owned())?;
        let stdin = child.stdin.take();
        let stderr = child.stderr.take();
        procs.add(child);
        let actual = Arc::new(AtomicU32::new(rate));
        if let Some(stderr) = stderr {
            let seen = actual.clone();
            std::thread::Builder::new()
                .name("aplay-stderr".into())
                .stack_size(32 * 1024)
                .spawn(move || {
                    for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                        if let Some(got) = rate_got(&line) {
                            seen.store(got, Ordering::Relaxed);
                        }
                    }
                })
                .ok();
        }
        Ok(Aplay { stdin, rate, actual, conv: None, bytes: Vec::new() })
    }
}

/// "Warning: rate is not accurate (requested = 24000Hz, got = 48000Hz)": the 48000.
fn rate_got(line: &str) -> Option<u32> {
    let after = line.split("got = ").nth(1)?;
    after.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok().filter(|r| *r > 0)
}

impl Sink for Aplay {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let actual = self.actual.load(Ordering::Relaxed);
        let mut converted = Vec::new();
        let samples = if actual != self.rate {
            if self.conv.as_ref().is_none_or(|(to, _)| *to != actual) {
                self.conv = Some((actual, Resampler::new(self.rate, 1, actual)));
            }
            if let Some((_, conv)) = &mut self.conv {
                conv.push(samples, &mut converted);
            }
            &converted[..]
        } else {
            samples
        };
        self.bytes.clear();
        self.bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        let stdin = self.stdin.as_mut().ok_or("the speaker stopped")?;
        stdin.write_all(&self.bytes).map_err(|_| "the speaker stopped".to_owned())
    }

    fn finish(&mut self) {
        // aplay plays what it holds and ends: its caller waits for that.
        self.stdin = None;
    }
}

/// A computer's stand-in for the speaker: nothing is heard, but writing
/// takes as long as hearing it would (with a quarter second of buffer, as a
/// device has), so everything around it keeps its timing.
pub struct Paced {
    rate: u32,
    sent: u64,
    started: Option<Instant>,
}

impl Paced {
    pub fn new(rate: u32) -> Self {
        Paced { rate: rate.max(1), sent: 0, started: None }
    }

    fn played_by(&self) -> Duration {
        Duration::from_secs_f64(self.sent as f64 / f64::from(self.rate))
    }
}

impl Sink for Paced {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let started = *self.started.get_or_insert_with(Instant::now);
        self.sent += samples.len() as u64;
        let ahead = Duration::from_millis(250);
        if let Some(wait) = self.played_by().checked_sub(started.elapsed() + ahead) {
            std::thread::sleep(wait);
        }
        Ok(())
    }

    fn finish(&mut self) {
        if let Some(started) = self.started {
            if let Some(wait) = self.played_by().checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_at_another_rate_is_noticed_from_what_aplay_says() {
        assert_eq!(rate_got("Warning: rate is not accurate (requested = 24000Hz, got = 48000Hz)"), Some(48_000));
        assert_eq!(rate_got("Playing raw data 'stdin' : Signed 16 bit Little Endian, Rate 24000 Hz, Mono"), None);
    }

    /// A stand-in `aplay` on a device that runs at 48 kHz with no plug in
    /// front: it says so, as aplay does, and keeps what it is given.
    #[cfg(unix)]
    #[test]
    fn a_speaker_at_another_rate_is_given_sound_at_its_rate() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("speech-fake-aplay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (program, out) = (dir.join("aplay"), dir.join("played.raw"));
        let script = format!("#!/bin/sh\necho 'Playing raw data' >&2\necho 'Warning: rate is not accurate (requested = 24000Hz, got = 48000Hz)' >&2\ncat > '{}'\n", out.display());
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let procs = Procs::default();
        let mut sink = Aplay::start_program(program.to_str().unwrap(), "default", 24_000, &procs).unwrap();
        // It says what it got as it starts, before any sound comes from the network.
        std::thread::sleep(Duration::from_millis(300));
        for _ in 0..10 {
            sink.write(&[1000; 2400]).unwrap();
        }
        sink.finish();
        procs.wait_all(&std::sync::atomic::AtomicBool::new(false));
        let played = std::fs::read(&out).unwrap().len() / 2;
        assert!((47_800..=48_100).contains(&played), "a second of sound at 48 kHz: {played}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_stand_in_speaker_takes_as_long_as_listening_would() {
        let mut sink = Paced::new(8_000);
        let started = Instant::now();
        sink.write(&[0; 2000]).unwrap();
        assert!(started.elapsed() < Duration::from_millis(100), "a quarter second is taken at once, as a device's buffer");
        sink.write(&[0; 2000]).unwrap();
        sink.finish();
        assert!(started.elapsed() >= Duration::from_millis(480), "{:?}", started.elapsed());
    }
}
