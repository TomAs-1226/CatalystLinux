//! espeak, on the handheld itself: offline, about 2 MB of voices, and fast
//! (it speaks many times quicker than real time on the A53). Robotic, but
//! clear. Buildroot packages espeak 1.48, the engine espeak-ng grew from;
//! espeak-ng itself is not packaged there and its cross build needs a host
//! build of itself to compile its dictionaries.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Procs, WavStream};

/// Whether espeak is on this machine.
pub fn available() -> bool {
    crate::installed("espeak")
}

/// Say `text`, handing the sound to `emit` (rate, samples) as espeak makes
/// it. Blocks: call it from a thread. espeak joins `procs`.
pub fn speak(text: &str, procs: &Procs, stop: &AtomicBool, emit: &mut dyn FnMut(u32, &[i16]) -> Result<(), String>) -> Result<(), String> {
    // American English, a little slower than its default 175 words a
    // minute; the text on stdin, UTF-8 (-b 1); a WAV stream on stdout.
    let mut child = Command::new("espeak")
        .args(["-v", "en-us", "-s", "160", "-b", "1", "--stdin", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "espeak is not on this handheld".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|_| "espeak stopped".to_owned())?;
    }
    let mut stdout = child.stdout.take().ok_or("espeak did not start")?;
    procs.add(child);
    let mut wav = WavStream::default();
    let mut buf = vec![0u8; 8192];
    let mut samples = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let n = stdout.read(&mut buf).map_err(|_| "espeak stopped".to_owned())?;
        if n == 0 {
            return Ok(());
        }
        samples.clear();
        wav.push(&buf[..n], &mut samples)?;
        if let (Some(format), false) = (wav.format(), samples.is_empty()) {
            emit(format.rate, &samples)?;
        }
    }
}
