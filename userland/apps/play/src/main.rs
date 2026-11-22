//! catalyst-play: Music's player, and its tool for songs on the card.
//!
//! ```text
//! catalyst-play [--start S] [--size BYTES] [--device PCM] [--album KEY]
//!               [--levels FILE] [--raw] FILE
//! catalyst-play --probe FILE [--picture-out PATH]
//! catalyst-play --remux IN OUT [--tags FILE] [--picture JPEG]
//! catalyst-play --dsp-cost
//! ```
//!
//! **Playing** decodes in this one process (symphonia's demuxers and
//! decoders, libopus for Opus) and writes to ALSA: to "default" (the
//! system volume, dmix, Bluetooth) at the rate the device runs at,
//! resampling only a song at another; or, to USB headphones with a volume
//! of their own, straight to their `hw` device at the song's own rate and
//! depth, bit for bit (route.rs). Spatial sound, the equalizer, loudness
//! matching and crossfades (Settings › Sound, read from /data/catalyst/
//! audio-fx while it plays) run only while they are on (play.rs). It wakes
//! once an ALSA period (23 times a second) and uses a few percent of one
//! A53 core; ffmpeg's command line tool took four times the processor and
//! woke 200 times a second (16 threads for one song). `--size` says how big
//! the file will be when its download finishes: reading catches up with a
//! file still being written and waits there (saying `buffering`), so a
//! song starts while it downloads and seeks anywhere already on the card.
//! `--raw` writes the sound to standard output instead (tests and the xrun
//! harness). `--album` names the song's album (a crossfade is never made
//! within one). `--levels` writes each period's level to FILE for the
//! island's bars (audiofx::levels). Standard input takes the next song, and
//! a fade (control.rs).
//!
//! What it says, a line at a time on standard output (standard error with
//! `--raw`): `ready codec=opus rate=48000 channels=2 seconds=232.1`, then
//! `at 12.0` every few seconds, `buffering` and `playing` as a download
//! runs dry and catches up, `xrun 3` (ALSA ran dry, the count so far),
//! `track PATH` as the next song given takes over, `faded`, and `end`.
//! Exit status: 0 played to the end, 1 failed, 2 asked wrongly, 3 a format
//! this does not read (the caller tries ffmpeg), 4 the sound device is busy.
//!
//! **--probe** prints a song's tags and length as `key=value` lines
//! (title, artist, album, album_artist, track, year, seconds, codec, rate)
//! and, with `--picture-out`, writes its embedded cover there.
//!
//! **--remux** copies an Opus song (YouTube's WebM, or Ogg) into Ogg Opus
//! as it is (no re-encoding), with the tags in FILE (`KEY=value` lines) and
//! the JPEG as its embedded cover (METADATA_BLOCK_PICTURE).
//!
//! **--dsp-cost** measures spatial sound and the equalizer on this
//! processor: nanoseconds a frame, and the share of a core at 48 kHz.

mod alsa;
mod chain;
mod cleanup;
mod control;
mod ogg;
mod opus;
mod play;
mod probe;
mod resample;
mod route;
mod song;
mod source;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const RATE: u32 = 48_000;

static RAW: AtomicBool = AtomicBool::new(false);

/// A line for whoever started this: standard output, or standard error
/// when the sound itself goes to standard output.
pub fn say(line: &str) {
    if RAW.load(Ordering::Relaxed) {
        eprintln!("{line}");
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").ok();
        out.flush().ok();
    }
}

pub enum Fail {
    Usage(String),
    Unreadable(String),
    Busy(String),
    Other(String),
}

impl Fail {
    fn code(&self) -> i32 {
        match self {
            Fail::Other(_) => 1,
            Fail::Usage(_) => 2,
            Fail::Unreadable(_) => 3,
            Fail::Busy(_) => 4,
        }
    }
    pub fn words(&self) -> &str {
        match self {
            Fail::Usage(w) | Fail::Unreadable(w) | Fail::Busy(w) | Fail::Other(w) => w,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--probe") => probe::run(&args[1..]),
        Some("--remux") => ogg::remux_command(&args[1..]),
        Some("--dsp-cost") => {
            dsp_cost();
            Ok(())
        }
        Some("--help") | None => Err(Fail::Usage("usage: catalyst-play [--start S] [--size BYTES] [--device PCM] [--album KEY] [--levels FILE] [--raw] FILE | --probe FILE [--picture-out PATH] | --remux IN OUT [--tags FILE] [--picture JPEG] | --dsp-cost".into())),
        _ => play_command(&args),
    };
    if let Err(fail) = result {
        crate::cleanup::now();
        eprintln!("catalyst-play: {}", fail.words());
        std::process::exit(fail.code());
    }
}

fn play_command(args: &[String]) -> Result<(), Fail> {
    let mut a = play::PlayArgs { start: 0.0, size: None, device: None, raw: false, file: PathBuf::new(), album: String::new(), levels: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| Fail::Usage(format!("{name} needs a value")));
        match arg.as_str() {
            "--start" => a.start = value("--start")?.parse().map_err(|_| Fail::Usage("--start takes seconds".into()))?,
            "--size" => a.size = Some(value("--size")?.parse().map_err(|_| Fail::Usage("--size takes bytes".into()))?),
            "--device" => a.device = Some(value("--device")?),
            "--album" => a.album = value("--album")?,
            "--levels" => a.levels = Some(PathBuf::from(value("--levels")?)),
            "--raw" => a.raw = true,
            f if !f.starts_with("--") && a.file.as_os_str().is_empty() => a.file = PathBuf::from(f),
            other => return Err(Fail::Usage(format!("not an option: {other}"))),
        }
    }
    if a.file.as_os_str().is_empty() {
        return Err(Fail::Usage("which file?".into()));
    }
    RAW.store(a.raw, Ordering::Relaxed);
    play::run(&a)
}

/// The extension that says what a file is, a download's `.part` left off.
fn extension(path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let name = name.strip_suffix(".part").unwrap_or(name);
    name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())
}

pub fn hint_for(path: &Path) -> Hint {
    let mut hint = Hint::new();
    if let Some(ext) = extension(path) {
        hint.with_extension(&ext);
    }
    hint
}

/// A song's reader and its audio track's id.
pub fn open(path: &Path, size: Option<u64>) -> Result<(Box<dyn FormatReader>, u32), Fail> {
    let src = source::Follow::open(path, size).map_err(|e| Fail::Other(format!("{}: {e}", path.display())))?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint_for(path), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| Fail::Unreadable(format!("not a song this reads: {e}")))?;
    let format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| Fail::Unreadable("no sound in it".into()))?;
    let id = track.id;
    Ok((format, id))
}

/// Spatial sound's and the equalizer's cost here, measured on a second of
/// music-like sound at 48 kHz, many times over.
fn dsp_cost() {
    use std::time::Instant;
    let rate = 48_000;
    let make = || -> Vec<f32> { (0..rate as usize).flat_map(|i| { let t = i as f32 / rate as f32; let x = 0.3 * (t * 220.0 * 6.283).sin() + 0.1 * (t * 3100.0 * 6.283).sin(); [x, x * 0.8] }).collect() };
    let rounds = 10;
    let time = |f: &mut dyn FnMut(&mut [f32])| {
        let mut best = f64::MAX;
        for _ in 0..rounds {
            let mut frames = make();
            let t = Instant::now();
            f(&mut frames);
            best = best.min(t.elapsed().as_secs_f64());
        }
        best / rate as f64
    };
    let mut sp = audiofx::spatial::Spatial::new(rate);
    let mut yaw = 0.0;
    let spatial = time(&mut |f| {
        yaw += 3.0;
        sp.set_pose(yaw % 40.0, 0.0);
        sp.process(f)
    });
    let mut eq = audiofx::eq::Equalizer::new(rate);
    eq.set([4.0, 2.0, -1.0, 2.0, 3.0]);
    let eq_cost = time(&mut |f| eq.process(f));
    let mut m = audiofx::loudness::Meter::new(rate);
    let meter = time(&mut |f| m.push(f));
    let mut r = resample::Resampler::new(44_100, 48_000);
    let mut out = Vec::new();
    let resample = time(&mut |f| {
        out.clear();
        r.process(f, &mut out)
    });
    for (name, s) in [("spatial", spatial), ("eq (five bands)", eq_cost), ("loudness meter", meter), ("resampler 44.1k", resample)] {
        println!("{name}: {:.0} ns a frame, {:.2} % of this core at 48 kHz", s * 1e9, s * rate as f64 * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_still_running_is_known_by_its_own_name() {
        assert_eq!(extension(Path::new("/data/music/cache/abc.webm.part")).as_deref(), Some("webm"));
        assert_eq!(extension(Path::new("/data/music/library/A/B/01 Song.MP3")).as_deref(), Some("mp3"));
        assert_eq!(extension(Path::new("noext")), None);
    }
}
