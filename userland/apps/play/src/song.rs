//! One song, opened and decoded: interleaved stereo `i32` frames at its
//! own rate (full scale, a 16-bit sample in the top 16 bits, so a 16- or
//! 24-bit song reaches the device as it was), with the encoder's delay and
//! padding trimmed (audiofx::gapless) so songs meant to run on do.

use std::path::{Path, PathBuf};

use audiofx::gapless::Trim;
use audiofx::loudness::Level;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL, CODEC_TYPE_OPUS};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::{ogg, opus, source, Fail};

enum Dec {
    Opus(opus::Opus),
    Symphonia(Box<dyn Decoder>, Option<SampleBuffer<i32>>),
}

pub struct Song {
    pub path: PathBuf,
    format: Box<dyn FormatReader>,
    track_id: u32,
    dec: Dec,
    pub codec: String,
    pub rate: u32,
    pub channels: usize,
    /// The depth it was made at: its own for FLAC, ALAC and WAV; 16 for
    /// a lossy song (what its decoder gives is no deeper than that).
    pub bits: u32,
    pub seconds: f64,
    /// Where it started (a seek), seconds.
    pub start: f64,
    /// Its loudness, from its tags.
    pub level: Option<Level>,
    /// Opus's own output gain, decibels (applied always, as RFC 7845 says).
    pub output_gain_db: f32,
    trim: Trim,
    /// Opus in Ogg: the reader marks each packet's delay and padding, and
    /// they are cut here (symphonia's own decoders cut theirs).
    packet_trims: bool,
    /// Frames before the start asked for, decoded and left out.
    skip: usize,
    /// Frames given out so far.
    pub given: u64,
    scratch: Vec<i32>,
    opus_pcm: Vec<i16>,
    ended: bool,
}

fn tag_pairs(revision: Option<&symphonia::core::meta::MetadataRevision>, out: &mut Vec<(String, String)>) {
    if let Some(r) = revision {
        out.extend(r.tags().iter().map(|t| (t.key.clone(), t.value.to_string())));
    }
}

impl Song {
    pub fn open(path: &Path, size: Option<u64>, start: f64) -> Result<Song, Fail> {
        let src = source::Follow::open(path, size).map_err(|e| Fail::Other(format!("{}: {e}", path.display())))?;
        let mss = MediaSourceStream::new(Box::new(src), Default::default());
        let options = FormatOptions { enable_gapless: true, ..Default::default() };
        let mut probed = symphonia::default::get_probe().format(&crate::hint_for(path), mss, &options, &MetadataOptions::default()).map_err(|e| Fail::Unreadable(format!("not a song this reads: {e}")))?;
        let mut format = probed.format;
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).cloned().ok_or_else(|| Fail::Unreadable("no sound in it".into()))?;
        let track_id = track.id;
        let params = track.codec_params.clone();
        let mut tags = Vec::new();
        tag_pairs(probed.metadata.get().as_ref().and_then(|m| m.current()), &mut tags);
        tag_pairs(format.metadata().current(), &mut tags);

        let rate = params.sample_rate.unwrap_or(crate::RATE);
        let seconds = match (params.n_frames, params.time_base) {
            (Some(n), Some(tb)) => {
                let t = tb.calc_time(n);
                t.seconds as f64 + t.frac
            }
            (Some(n), None) => n as f64 / rate as f64,
            _ => 0.0,
        };
        let is_opus = params.codec == CODEC_TYPE_OPUS;
        let mut channels = params.channels.map(|c| c.count()).unwrap_or(2);
        let mut output_gain_db = 0.0;
        let mut packet_trims = false;
        let mut trim = Trim::none();
        let dec = if is_opus {
            let head = params.extra_data.as_deref().and_then(ogg::OpusHead::parse);
            if let Some(h) = &head {
                channels = h.channels as usize;
                output_gain_db = h.gain as f32 / 256.0;
            }
            // Ogg says its delay (and the reader cuts by it, packet by
            // packet); WebM leaves it to the decoder, and ends the song at
            // its duration.
            packet_trims = params.delay.is_some();
            let pre_skip = if start > 0.0 || packet_trims { 0 } else { head.map_or(312, |h| h.pre_skip as usize) };
            if !packet_trims && seconds > 0.0 {
                trim = Trim::new(0, Some(((seconds - start).max(0.0) * 48_000.0).round() as u64));
            }
            Dec::Opus(opus::Opus::new(channels, pre_skip).map_err(Fail::Other)?)
        } else {
            let inner = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| Fail::Unreadable(format!("no decoder for it: {e}")))?;
            // iTunes's AAC: its encoder delay and padding, from its tag.
            if let Some((delay, _, frames)) = tags.iter().find(|(k, _)| k.ends_with("iTunSMPB")).and_then(|(_, v)| audiofx::gapless::itunsmpb(v)) {
                let start_frames = (start * rate as f64) as u64;
                trim = if start > 0.0 { Trim::new(0, Some(frames.saturating_sub(start_frames))) } else { Trim::new(delay as u64, Some(frames)) };
            }
            Dec::Symphonia(inner, None)
        };
        let lossless = matches!(params.codec, c if c != CODEC_TYPE_OPUS && params.bits_per_sample.is_some());
        let bits = if lossless { params.bits_per_sample.unwrap_or(16).clamp(8, 32) } else { 16 };
        let codec = if is_opus { "opus".to_owned() } else { symphonia::default::get_codecs().get_codec(params.codec).map_or("audio".to_owned(), |c| c.short_name.to_owned()) };
        let level = audiofx::loudness::from_tags(tags.iter().map(|(k, v)| (k.as_str(), v.as_str())));

        let mut song = Song {
            path: path.to_owned(),
            format,
            track_id,
            dec,
            codec,
            rate,
            channels,
            bits,
            seconds,
            start,
            level,
            output_gain_db,
            trim,
            packet_trims,
            skip: 0,
            given: 0,
            scratch: Vec::with_capacity(8192),
            opus_pcm: Vec::with_capacity(8192),
            ended: false,
        };
        if start > 0.0 {
            match song.format.seek(SeekMode::Accurate, SeekTo::Time { time: Time::from(start), track_id: Some(track_id) }) {
                Ok(to) => {
                    if let Some(tb) = params.time_base {
                        let early = tb.calc_time(to.required_ts.saturating_sub(to.actual_ts));
                        song.skip = ((early.seconds as f64 + early.frac) * rate as f64) as usize;
                    }
                }
                Err(e) => return Err(Fail::Other(format!("could not go to {start:.1} s: {e}"))),
            }
            match &mut song.dec {
                Dec::Opus(o) => o.reset(),
                Dec::Symphonia(d, _) => d.reset(),
            }
        }
        Ok(song)
    }

    /// Frames still to come, if its length is known.
    pub fn left(&self) -> Option<u64> {
        if self.seconds <= 0.0 {
            return None;
        }
        let total = ((self.seconds - self.start).max(0.0) * self.rate as f64) as u64;
        Some(total.saturating_sub(self.given))
    }

    /// The next frames, appended to `out` (cleared first); false once the
    /// song has ended.
    pub fn next(&mut self, out: &mut Vec<i32>) -> Result<bool, Fail> {
        out.clear();
        while out.is_empty() {
            if self.ended {
                return Ok(false);
            }
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    return Ok(false);
                }
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::TimedOut => return Err(Fail::Other("the download stopped".into())),
                Err(Error::ResetRequired) => {
                    self.ended = true;
                    return Ok(false);
                }
                Err(e) => return Err(Fail::Other(format!("the song could not be read on: {e}"))),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            self.scratch.clear();
            let ch = match &mut self.dec {
                Dec::Opus(o) => {
                    self.opus_pcm.clear();
                    let ch = o.decode(&packet.data, &mut self.opus_pcm).map_err(Fail::Other)?;
                    let (front, back) = if self.packet_trims { (packet.trim_start() as usize, packet.trim_end() as usize) } else { (0, 0) };
                    let frames = self.opus_pcm.len() / ch.max(1);
                    let keep = frames.saturating_sub(front + back);
                    let from = front.min(frames) * ch;
                    self.scratch.extend(self.opus_pcm[from..from + keep * ch].iter().map(|&s| (s as i32) << 16));
                    ch
                }
                Dec::Symphonia(d, buf) => match d.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        if buf.as_ref().is_none_or(|b| b.capacity() < decoded.capacity() * spec.channels.count()) {
                            *buf = Some(SampleBuffer::<i32>::new(decoded.capacity() as u64, spec));
                        }
                        let b = buf.as_mut().unwrap();
                        b.copy_interleaved_ref(decoded);
                        self.scratch.extend_from_slice(b.samples());
                        spec.channels.count()
                    }
                    // A damaged packet: skipped, the rest plays.
                    Err(Error::DecodeError(_)) => 0,
                    Err(e) => return Err(Fail::Other(e.to_string())),
                },
            };
            if ch == 0 || self.scratch.is_empty() {
                continue;
            }
            to_stereo(&self.scratch, ch, out);
            if self.skip > 0 {
                let n = self.skip.min(out.len() / 2);
                out.drain(..n * 2);
                self.skip -= n;
            }
            if self.trim.apply(out) {
                self.ended = true;
            }
        }
        self.given += out.len() as u64 / 2;
        Ok(true)
    }
}

/// Two channels from however many the song has: mono to both sides, more
/// than two folded down to the front pair.
pub fn to_stereo(pcm: &[i32], ch: usize, out: &mut Vec<i32>) {
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
}
