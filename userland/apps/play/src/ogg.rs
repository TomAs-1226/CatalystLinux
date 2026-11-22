//! Ogg Opus, written: a song's Opus packets copied as they are (from
//! YouTube's WebM, or another Ogg) into the container players expect a
//! `.opus` file in, with its tags and cover in the comment header (RFC
//! 7845), so they travel with the file to a computer.

use std::io::Write;
use std::path::Path;

use symphonia::core::codecs::CODEC_TYPE_OPUS;
use symphonia::core::errors::Error;

use crate::Fail;

/// Opus's identification header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpusHead {
    pub channels: u8,
    pub pre_skip: u16,
    /// The gain every decoder applies (Q7.8 dB, RFC 7845).
    pub gain: i16,
}

impl OpusHead {
    pub fn parse(b: &[u8]) -> Option<OpusHead> {
        (b.len() >= 19 && &b[..8] == b"OpusHead").then(|| OpusHead { channels: b[9].max(1), pre_skip: u16::from_le_bytes([b[10], b[11]]), gain: i16::from_le_bytes([b[16], b[17]]) })
    }

    /// One made up, for a container that did not carry it.
    fn bytes(self) -> Vec<u8> {
        let mut b = b"OpusHead".to_vec();
        b.push(1);
        b.push(self.channels);
        b.extend(self.pre_skip.to_le_bytes());
        b.extend(48_000u32.to_le_bytes());
        b.extend(self.gain.to_le_bytes());
        b.push(0);
        b
    }
}

/// The comment header: `KEY=value` tags, the cover among them.
pub fn opus_tags(tags: &[(String, String)]) -> Vec<u8> {
    let vendor = b"Catalyst Linux";
    let mut b = b"OpusTags".to_vec();
    b.extend((vendor.len() as u32).to_le_bytes());
    b.extend(vendor);
    b.extend((tags.len() as u32).to_le_bytes());
    for (k, v) in tags {
        let c = format!("{k}={v}");
        b.extend((c.len() as u32).to_le_bytes());
        b.extend(c.as_bytes());
    }
    b
}

/// A JPEG's width and height, from its frame header.
pub fn jpeg_size(j: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < j.len() {
        if j[i] != 0xFF {
            return None;
        }
        let marker = j[i + 1];
        let len = u16::from_be_bytes([j[i + 2], j[i + 3]]) as usize;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((u16::from_be_bytes([j[i + 7], j[i + 8]]) as u32, u16::from_be_bytes([j[i + 5], j[i + 6]]) as u32));
        }
        i += 2 + len;
    }
    None
}

/// A cover as Vorbis comments carry one: a FLAC picture block (the front
/// cover), in base64.
pub fn picture_comment(jpeg: &[u8]) -> String {
    let (w, h) = jpeg_size(jpeg).unwrap_or((0, 0));
    let mime = b"image/jpeg";
    let mut b = Vec::with_capacity(jpeg.len() + 64);
    b.extend(3u32.to_be_bytes());
    b.extend((mime.len() as u32).to_be_bytes());
    b.extend(mime);
    b.extend(0u32.to_be_bytes());
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend(24u32.to_be_bytes());
    b.extend(0u32.to_be_bytes());
    b.extend((jpeg.len() as u32).to_be_bytes());
    b.extend(jpeg);
    base64(&b)
}

pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn crc(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            }
            *e = r;
        }
        t
    });
    data.iter().fold(0u32, |c, &b| (c << 8) ^ t[((c >> 24) as u8 ^ b) as usize])
}

/// Packets into pages.
pub struct Writer<W: Write> {
    out: W,
    serial: u32,
    seq: u32,
    segments: Vec<u8>,
    data: Vec<u8>,
    /// The granule position of the last packet that ended on this page.
    ended: Option<i64>,
    /// This page goes on with a packet the last one began.
    continued: bool,
    /// Header pages carry granule 0 even where no packet ends.
    header: bool,
}

/// Audio pages are let go at about this size.
const PAGE_BYTES: usize = 4096;

impl<W: Write> Writer<W> {
    pub fn new(out: W, serial: u32) -> Self {
        Writer { out, serial, seq: 0, segments: Vec::new(), data: Vec::new(), ended: None, continued: false, header: true }
    }

    pub fn packet(&mut self, p: &[u8], granule: i64) -> std::io::Result<()> {
        let mut rest = p;
        loop {
            if self.segments.len() == 255 {
                self.page(false)?;
                self.continued = true;
            }
            let n = rest.len().min(255);
            self.segments.push(n as u8);
            self.data.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            if n < 255 {
                break;
            }
        }
        self.ended = Some(granule);
        Ok(())
    }

    /// The header packets are each a page (or more) of their own; `last`:
    /// the audio follows.
    pub fn end_header(&mut self, last: bool) -> std::io::Result<()> {
        self.page(false)?;
        self.header = !last;
        Ok(())
    }

    pub fn maybe_page(&mut self) -> std::io::Result<()> {
        if self.data.len() >= PAGE_BYTES || self.segments.len() >= 240 {
            self.page(false)?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.page(true)?;
        self.out.flush()?;
        Ok(self.out)
    }

    fn page(&mut self, last: bool) -> std::io::Result<()> {
        if self.segments.is_empty() && !last {
            return Ok(());
        }
        let granule = if self.header { 0 } else { self.ended.unwrap_or(-1) };
        let mut kind = 0u8;
        if self.continued {
            kind |= 1;
        }
        if self.seq == 0 {
            kind |= 2;
        }
        if last {
            kind |= 4;
        }
        let mut page = Vec::with_capacity(27 + self.segments.len() + self.data.len());
        page.extend(b"OggS");
        page.push(0);
        page.push(kind);
        page.extend(granule.to_le_bytes());
        page.extend(self.serial.to_le_bytes());
        page.extend(self.seq.to_le_bytes());
        page.extend([0u8; 4]);
        page.push(self.segments.len() as u8);
        page.extend(&self.segments);
        page.extend(&self.data);
        let sum = crc(&page);
        page[22..26].copy_from_slice(&sum.to_le_bytes());
        self.out.write_all(&page)?;
        self.seq += 1;
        self.segments.clear();
        self.data.clear();
        self.ended = None;
        self.continued = false;
        Ok(())
    }
}

/// `--remux IN OUT [--tags FILE] [--picture JPEG]`.
pub fn remux_command(args: &[String]) -> Result<(), Fail> {
    let (mut input, mut output, mut tags_file, mut picture) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tags" => tags_file = it.next().cloned(),
            "--picture" => picture = it.next().cloned(),
            f if input.is_none() => input = Some(f.to_owned()),
            f if output.is_none() => output = Some(f.to_owned()),
            other => return Err(Fail::Usage(format!("not an option: {other}"))),
        }
    }
    let (Some(input), Some(output)) = (input, output) else { return Err(Fail::Usage("--remux IN OUT".into())) };
    let mut tags: Vec<(String, String)> = match &tags_file {
        Some(f) => std::fs::read_to_string(f)
            .map_err(|e| Fail::Other(format!("{f}: {e}")))?
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_owned())))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect(),
        None => Vec::new(),
    };
    if let Some(p) = &picture {
        let jpeg = std::fs::read(p).map_err(|e| Fail::Other(format!("{p}: {e}")))?;
        tags.push(("METADATA_BLOCK_PICTURE".into(), picture_comment(&jpeg)));
    }
    remux(Path::new(&input), Path::new(&output), &tags)
}

pub fn remux(input: &Path, output: &Path, tags: &[(String, String)]) -> Result<(), Fail> {
    let (mut format, track_id) = crate::open(input, None)?;
    let params = format.tracks().iter().find(|t| t.id == track_id).map(|t| t.codec_params.clone()).ok_or_else(|| Fail::Unreadable("no sound in it".into()))?;
    if params.codec != CODEC_TYPE_OPUS {
        return Err(Fail::Unreadable("only Opus goes into Ogg Opus".into()));
    }
    let head = params.extra_data.as_deref().filter(|b| OpusHead::parse(b).is_some()).map(<[u8]>::to_vec).unwrap_or_else(|| OpusHead { channels: params.channels.map_or(2, |c| c.count() as u8), pre_skip: 312, gain: 0 }.bytes());
    let file = std::fs::File::create(output).map_err(|e| Fail::Other(format!("{}: {e}", output.display())))?;
    let serial = std::process::id() ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let mut w = Writer::new(std::io::BufWriter::with_capacity(64 * 1024, file), serial);
    let io = |e: std::io::Error| Fail::Other(format!("could not write {}: {e}", output.display()));
    w.packet(&head, 0).map_err(io)?;
    w.end_header(false).map_err(io)?;
    w.packet(&opus_tags(tags), 0).map_err(io)?;
    w.end_header(true).map_err(io)?;
    let mut granule: i64 = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(Fail::Other(format!("the song could not be read: {e}"))),
        };
        if packet.track_id() != track_id || packet.data.is_empty() {
            continue;
        }
        granule += crate::opus::packet_samples(&packet.data) as i64;
        w.packet(&packet.data, granule).map_err(io)?;
        w.maybe_page().map_err(io)?;
    }
    w.finish().map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_carry_their_checksum_and_long_packets_go_on_across_pages() {
        // The CRC Ogg uses (polynomial 0x04C11DB7, no reflection): "123456789".
        assert_eq!(crc(b"123456789"), 0x89A1_897F);
        let mut w = Writer::new(Vec::new(), 7);
        w.packet(&OpusHead { channels: 2, pre_skip: 312, gain: 0 }.bytes(), 0).unwrap();
        w.end_header(false).unwrap();
        // A comment header with a 70 KB cover: more than one page holds.
        w.packet(&vec![b'x'; 70_000], 0).unwrap();
        w.end_header(true).unwrap();
        w.packet(&[0xF8; 300], 960).unwrap();
        let bytes = w.finish().unwrap();
        let mut pages = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            assert_eq!(&bytes[i..i + 4], b"OggS");
            let n = bytes[i + 26] as usize;
            let body: usize = bytes[i + 27..i + 27 + n].iter().map(|&b| b as usize).sum();
            let len = 27 + n + body;
            let mut page = bytes[i..i + len].to_vec();
            let said = u32::from_le_bytes(page[22..26].try_into().unwrap());
            page[22..26].fill(0);
            assert_eq!(crc(&page), said, "page {}", pages.len());
            pages.push((bytes[i + 5], i64::from_le_bytes(bytes[i + 6..i + 14].try_into().unwrap()), u32::from_le_bytes(bytes[i + 18..i + 22].try_into().unwrap())));
            i += len;
        }
        // Head (first), the comment over two pages (the second goes on), the audio (last).
        assert_eq!(pages.len(), 4, "{pages:?}");
        assert_eq!(pages[0], (2, 0, 0));
        assert_eq!((pages[1].0, pages[1].1), (0, 0));
        assert_eq!((pages[2].0, pages[2].1), (1, 0), "goes on, granule 0 as headers have");
        assert_eq!((pages[3].0, pages[3].1, pages[3].2), (4, 960, 3));
    }

    #[test]
    fn a_cover_is_a_flac_picture_in_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        // A JPEG's frame header: 2 × 3 pixels.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 0, 3, 0, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(jpeg_size(&jpeg), Some((2, 3)));
        let tags = opus_tags(&[("TITLE".into(), "Harbour Lights".into())]);
        assert!(tags.starts_with(b"OpusTags") && tags.ends_with(b"TITLE=Harbour Lights"));
    }
}
