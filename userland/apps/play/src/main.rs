//! catalyst-play: Music's player, and its tool for songs on the card.
//!
//! ```text
//! catalyst-play [--start S] [--size BYTES] [--device PCM] [--raw] FILE
//! catalyst-play --probe FILE [--picture-out PATH]
//! catalyst-play --remux IN OUT [--tags FILE] [--picture JPEG]
//! ```
//!
//! **Playing** decodes in this one process (symphonia's demuxers and
//! decoders, libopus for Opus), resamples to the 48 kHz the codec runs at
//! only when the song is at another rate, and writes to ALSA ("default":
//! the system volume, dmix, Bluetooth). It wakes once an ALSA period (23
//! times a second) and uses a few percent of one A53 core; ffmpeg's command
//! line tool took four times the processor and woke 200 times a second
//! (16 threads for one song). `--size` says how big the file will be when
//! its download finishes: reading catches up with a file still being
//! written and waits there (saying `buffering`), so a song starts while it
//! downloads and seeks anywhere already on the card. `--raw` writes the
//! sound to standard output instead (tests and the xrun harness).
//!
//! What it says, a line at a time on standard output (standard error with
//! `--raw`): `ready codec=opus rate=48000 channels=2 seconds=232.1`, then
//! `at 12.0` every few seconds, `buffering` and `playing` as a download
//! runs dry and catches up, `xrun 3` (ALSA ran dry, the count so far), and
//! `end`. Exit status: 0 played to the end, 1 failed, 2 asked wrongly,
//! 3 a format this does not read (the caller tries ffmpeg), 4 the sound
//! device is busy.
//!
//! **--probe** prints a song's tags and length as `key=value` lines
//! (title, artist, album, album_artist, track, year, seconds, codec, rate)
//! and, with `--picture-out`, writes its embedded cover there.
//!
//! **--remux** copies an Opus song (YouTube's WebM, or Ogg) into Ogg Opus
//! as it is (no re-encoding), with the tags in FILE (`KEY=value` lines) and
//! the JPEG as its embedded cover (METADATA_BLOCK_PICTURE).

mod alsa;
mod ogg;
mod opus;
mod probe;
mod resample;
mod source;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL, CODEC_TYPE_OPUS};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub const RATE: u32 = 48_000;
/// How often the position is said while playing, seconds.
const SAY_EVERY: f64 = 5.0;

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
    fn words(&self) -> &str {
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
        Some("--help") | None => Err(Fail::Usage("usage: catalyst-play [--start S] [--size BYTES] [--device PCM] [--raw] FILE | --probe FILE [--picture-out PATH] | --remux IN OUT [--tags FILE] [--picture JPEG]".into())),
        _ => play_command(&args),
    };
    if let Err(fail) = result {
        eprintln!("catalyst-play: {}", fail.words());
        std::process::exit(fail.code());
    }
}

struct PlayArgs {
    start: f64,
    size: Option<u64>,
    device: String,
    raw: bool,
    file: PathBuf,
}

fn play_command(args: &[String]) -> Result<(), Fail> {
    let mut a = PlayArgs { start: 0.0, size: None, device: "default".into(), raw: false, file: PathBuf::new() };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| Fail::Usage(format!("{name} needs a value")));
        match arg.as_str() {
            "--start" => a.start = value("--start")?.parse().map_err(|_| Fail::Usage("--start takes seconds".into()))?,
            "--size" => a.size = Some(value("--size")?.parse().map_err(|_| Fail::Usage("--size takes bytes".into()))?),
            "--device" => a.device = value("--device")?,
            "--raw" => a.raw = true,
            f if !f.starts_with("--") && a.file.as_os_str().is_empty() => a.file = PathBuf::from(f),
            other => return Err(Fail::Usage(format!("not an option: {other}"))),
        }
    }
    if a.file.as_os_str().is_empty() {
        return Err(Fail::Usage("which file?".into()));
    }
    RAW.store(a.raw, Ordering::Relaxed);
    play(&a)
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

struct Symphonia {
    inner: Box<dyn Decoder>,
    buf: Option<SampleBuffer<i16>>,
}

impl Symphonia {
    fn decode_packet(&mut self, packet: &symphonia::core::formats::Packet, out: &mut Vec<i16>) -> Result<usize, String> {
        match self.inner.decode(packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let frames = decoded.capacity() as u64;
                if self.buf.as_ref().is_none_or(|b| b.capacity() < decoded.capacity() * spec.channels.count()) {
                    self.buf = Some(SampleBuffer::<i16>::new(frames, spec));
                }
                let buf = self.buf.as_mut().unwrap();
                buf.copy_interleaved_ref(decoded);
                out.extend_from_slice(buf.samples());
                Ok(spec.channels.count())
            }
            // A damaged packet: skipped, the rest plays.
            Err(Error::DecodeError(_)) => Ok(0),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn play(a: &PlayArgs) -> Result<(), Fail> {
    let (mut format, track_id) = open(&a.file, a.size)?;
    let track = format.tracks().iter().find(|t| t.id == track_id).cloned().ok_or_else(|| Fail::Unreadable("no sound in it".into()))?;
    let params = track.codec_params.clone();
    let rate = params.sample_rate.unwrap_or(RATE);
    let seconds = match (params.n_frames, params.time_base) {
        (Some(n), Some(tb)) => {
            let t = tb.calc_time(n);
            t.seconds as f64 + t.frac
        }
        (Some(n), None) => n as f64 / rate as f64,
        _ => 0.0,
    };
    let opus = params.codec == CODEC_TYPE_OPUS;
    let mut channels = params.channels.map(|c| c.count()).unwrap_or(2);
    let mut opus_dec = None;
    let mut sym = None;
    if opus {
        let head = params.extra_data.as_deref().and_then(ogg::OpusHead::parse);
        if let Some(h) = &head {
            channels = h.channels as usize;
        }
        let pre_skip = if a.start > 0.0 { 0 } else { head.map_or(312, |h| h.pre_skip as usize) };
        opus_dec = Some(opus::Opus::new(channels, pre_skip).map_err(Fail::Other)?);
    } else {
        let inner = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| Fail::Unreadable(format!("no decoder for it: {e}")))?;
        sym = Some(Symphonia { inner, buf: None });
    }
    // Where to start: the reader goes to the packet before, the rest is
    // decoded and left out.
    let mut skip_frames = 0usize;
    if a.start > 0.0 {
        match format.seek(SeekMode::Accurate, SeekTo::Time { time: Time::from(a.start), track_id: Some(track_id) }) {
            Ok(to) => {
                if let Some(tb) = params.time_base {
                    let early = tb.calc_time(to.required_ts.saturating_sub(to.actual_ts));
                    skip_frames = ((early.seconds as f64 + early.frac) * rate as f64) as usize;
                }
            }
            Err(e) => return Err(Fail::Other(format!("could not go to {:.1} s: {e}", a.start))),
        }
        if let Some(o) = opus_dec.as_mut() {
            o.reset();
        }
        if let Some(s) = sym.as_mut() {
            s.inner.reset();
        }
    }
    let mut out: Box<dyn alsa::Out> = if a.raw { Box::new(alsa::Raw::new()) } else { Box::new(alsa::Alsa::open(&a.device, RATE).map_err(|e| if e.contains("busy") { Fail::Busy(e) } else { Fail::Other(e) })?) };
    let mut resampler = resample::Resampler::new(rate, RATE);
    let codec = if opus { "opus".to_owned() } else { symphonia::default::get_codecs().get_codec(params.codec).map_or("audio".to_owned(), |c| c.short_name.to_owned()) };
    say(&format!("ready codec={codec} rate={rate} channels={channels} seconds={seconds:.1}"));
    say(&format!("at {:.1}", a.start));

    let mut decoded: Vec<i16> = Vec::with_capacity(8192);
    let mut stereo: Vec<i16> = Vec::with_capacity(8192);
    let mut resampled: Vec<i16> = Vec::with_capacity(9000);
    let mut written: u64 = 0;
    let mut next_say = SAY_EVERY;
    let mut xruns_said = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::TimedOut => return Err(Fail::Other("the download stopped".into())),
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(Fail::Other(format!("the song could not be read on: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        decoded.clear();
        let ch = if let Some(o) = opus_dec.as_mut() { o.decode(&packet.data, &mut decoded).map_err(Fail::Other)? } else { sym.as_mut().unwrap().decode_packet(&packet, &mut decoded).map_err(Fail::Other)? };
        if ch == 0 || decoded.is_empty() {
            continue;
        }
        to_stereo(&decoded, ch, &mut stereo);
        // The part before the start asked for, left out.
        let mut from = 0;
        if skip_frames > 0 {
            let frames = stereo.len() / 2;
            let n = skip_frames.min(frames);
            skip_frames -= n;
            from = n * 2;
        }
        let pcm: &[i16] = if resampler.passes_through() {
            &stereo[from..]
        } else {
            resampled.clear();
            resampler.process(&stereo[from..], &mut resampled);
            &resampled
        };
        if pcm.is_empty() {
            continue;
        }
        out.write(pcm).map_err(Fail::Other)?;
        written += (pcm.len() / 2) as u64;
        let at = a.start + written as f64 / RATE as f64;
        if at - a.start >= next_say {
            next_say += SAY_EVERY;
            say(&format!("at {at:.1}"));
        }
        if out.xruns() != xruns_said {
            xruns_said = out.xruns();
            say(&format!("xrun {xruns_said}"));
        }
    }
    resampled.clear();
    resampler.finish(&mut resampled);
    if !resampled.is_empty() {
        out.write(&resampled).map_err(Fail::Other)?;
    }
    out.drain();
    say("end");
    Ok(())
}

/// Two channels from however many the song has: mono to both sides, more
/// than two folded down to the front pair.
fn to_stereo(pcm: &[i16], ch: usize, out: &mut Vec<i16>) {
    out.clear();
    match ch {
        2 => out.extend_from_slice(pcm),
        1 => pcm.iter().for_each(|&s| out.extend_from_slice(&[s, s])),
        _ => pcm.chunks_exact(ch).for_each(|f| out.extend_from_slice(&[f[0], f[1]])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_goes_to_both_sides_and_surround_to_the_front_pair() {
        let mut out = Vec::new();
        to_stereo(&[1, 2, 3], 1, &mut out);
        assert_eq!(out, [1, 1, 2, 2, 3, 3]);
        to_stereo(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 6, &mut out);
        assert_eq!(out, [1, 2, 7, 8]);
    }

    #[test]
    fn a_download_still_running_is_known_by_its_own_name() {
        assert_eq!(extension(Path::new("/data/music/cache/abc.webm.part")).as_deref(), Some("webm"));
        assert_eq!(extension(Path::new("/data/music/library/A/B/01 Song.MP3")).as_deref(), Some("mp3"));
        assert_eq!(extension(Path::new("noext")), None);
    }
}
