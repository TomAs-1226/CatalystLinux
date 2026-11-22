//! `--probe FILE [--picture-out PATH]`: a song's tags and length, as
//! `key=value` lines, for the shell's library index; its embedded cover
//! (ID3's APIC, FLAC's and Vorbis comments' picture, MP4's covr) written
//! out if asked.

use std::path::Path;

use symphonia::core::codecs::{CODEC_TYPE_NULL, CODEC_TYPE_OPUS};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey};

use crate::Fail;

#[derive(Default, Debug, PartialEq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: u32,
    pub year: u32,
    pub youtube: String,
    pub picture: Option<(String, Vec<u8>)>,
}

impl Tags {
    fn take(&mut self, rev: &MetadataRevision) {
        for tag in rev.tags() {
            let v = tag.value.to_string();
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            let first = |slot: &mut String| {
                if slot.is_empty() {
                    *slot = v.to_owned();
                }
            };
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => first(&mut self.title),
                Some(StandardTagKey::Artist) => first(&mut self.artist),
                Some(StandardTagKey::Album) => first(&mut self.album),
                Some(StandardTagKey::AlbumArtist) => first(&mut self.album_artist),
                // "3/12" or "3".
                Some(StandardTagKey::TrackNumber) if self.track == 0 => self.track = v.split('/').next().and_then(|n| n.trim().parse().ok()).unwrap_or(0),
                Some(StandardTagKey::Date | StandardTagKey::ReleaseDate | StandardTagKey::OriginalDate) if self.year == 0 => self.year = v.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0),
                // Music's own downloads say where they came from.
                Some(StandardTagKey::Comment) if v.starts_with("youtube:") => self.youtube = v["youtube:".len()..].to_owned(),
                _ if tag.key.eq_ignore_ascii_case("YOUTUBE_ID") => self.youtube = v.to_owned(),
                _ => {}
            }
        }
        if self.picture.is_none() {
            let front = rev.visuals().iter().find(|v| v.usage == Some(StandardVisualKey::FrontCover)).or_else(|| rev.visuals().first());
            if let Some(v) = front {
                self.picture = Some((v.media_type.clone(), v.data.to_vec()));
            }
        }
    }
}

/// A song's tags, length (seconds, 0 if not known), codec and rate.
pub fn probe(path: &Path) -> Result<(Tags, f64, String, u32), Fail> {
    let file = std::fs::File::open(path).map_err(|e| Fail::Other(format!("{}: {e}", path.display())))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut probed = symphonia::default::get_probe()
        .format(&crate::hint_for(path), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| Fail::Unreadable(format!("not a song this reads: {e}")))?;
    let mut tags = Tags::default();
    // Tags before the stream (ID3v2), then those in it.
    if let Some(m) = probed.metadata.get() {
        if let Some(rev) = m.current() {
            tags.take(rev);
        }
    }
    let mut format = probed.format;
    if let Some(rev) = format.metadata().current() {
        tags.take(rev);
    }
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| Fail::Unreadable("no sound in it".into()))?;
    let p = &track.codec_params;
    let rate = p.sample_rate.unwrap_or(0);
    let seconds = match (p.n_frames, p.time_base) {
        (Some(n), Some(tb)) => {
            let t = tb.calc_time(n);
            t.seconds as f64 + t.frac
        }
        (Some(n), None) if rate > 0 => n as f64 / rate as f64,
        _ => 0.0,
    };
    let codec = if p.codec == CODEC_TYPE_OPUS { "opus".to_owned() } else { symphonia::default::get_codecs().get_codec(p.codec).map_or("audio".to_owned(), |c| c.short_name.to_owned()) };
    // Some readers only find their tags once reading begins.
    if tags.title.is_empty() {
        let _ = format.next_packet();
        if let Some(rev) = format.metadata().current() {
            tags.take(rev);
        }
    }
    Ok((tags, seconds, codec, rate))
}

/// Tag values on one line each, whatever they held.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

pub fn run(args: &[String]) -> Result<(), Fail> {
    let mut file = None;
    let mut picture_out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--picture-out" => picture_out = it.next().cloned(),
            f if file.is_none() => file = Some(f.to_owned()),
            other => return Err(Fail::Usage(format!("not an option: {other}"))),
        }
    }
    let file = file.ok_or_else(|| Fail::Usage("--probe FILE".into()))?;
    let (tags, seconds, codec, rate) = probe(Path::new(&file))?;
    let mut out = String::new();
    for (k, v) in [("title", &tags.title), ("artist", &tags.artist), ("album", &tags.album), ("album_artist", &tags.album_artist), ("youtube", &tags.youtube)] {
        out.push_str(&format!("{k}={}\n", one_line(v)));
    }
    out.push_str(&format!("track={}\nyear={}\nseconds={seconds:.1}\ncodec={codec}\nrate={rate}\n", tags.track, tags.year));
    match (&tags.picture, picture_out) {
        (Some((mime, data)), Some(path)) => {
            std::fs::write(&path, data).map_err(|e| Fail::Other(format!("{path}: {e}")))?;
            out.push_str(&format!("picture={mime}\n"));
        }
        (Some((mime, _)), None) => out.push_str(&format!("picture={mime}\n")),
        _ => out.push_str("picture=\n"),
    }
    print!("{out}");
    Ok(())
}
