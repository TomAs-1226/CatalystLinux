//! Animated GIFs for pixel mode's loops (1.3.1): written from pixel
//! pictures (their few colours are a GIF's colour table as they are), and
//! read back for Photos to play. Our own small writer and reader, so no
//! library is added: LZW as the GIF89a specification has it, a frame's
//! delay, a loop for ever, the look in a comment.

use super::pixel::Frame;
use crate::photos::Picture;

/// Bits for `n` colours' table (a GIF's table is a power of two, two at least).
fn table_bits(n: usize) -> u8 {
    let mut bits = 1u8;
    while (1usize << bits) < n {
        bits += 1;
    }
    bits
}

/// Codes of varying width, least significant bit first.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, code: u16, size: u8) {
        self.acc |= u32::from(code) << self.n;
        self.n += u32::from(size);
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// `data` (indices below 2^`min`) compressed as a GIF's LZW.
fn lzw(data: &[u8], min: u8) -> Vec<u8> {
    let clear = 1u16 << min;
    let end = clear + 1;
    let mut size = min + 1;
    let mut next = end + 1;
    let mut dict: std::collections::HashMap<(u16, u8), u16> = std::collections::HashMap::new();
    let mut bits = Bits { out: Vec::new(), acc: 0, n: 0 };
    bits.put(clear, size);
    let Some((&first, rest)) = data.split_first() else {
        bits.put(end, size);
        return bits.finish();
    };
    let mut prefix = u16::from(first);
    for &k in rest {
        if let Some(&code) = dict.get(&(prefix, k)) {
            prefix = code;
            continue;
        }
        bits.put(prefix, size);
        if next > (1 << size) - 1 && size < 12 {
            size += 1;
        }
        if next < 4096 {
            dict.insert((prefix, k), next);
            next += 1;
        } else {
            bits.put(clear, size);
            dict.clear();
            size = min + 1;
            next = end + 1;
        }
        prefix = u16::from(k);
    }
    bits.put(prefix, size);
    if next > (1 << size) - 1 && size < 12 {
        size += 1;
    }
    bits.put(end, size);
    bits.finish()
}

/// Bytes in a GIF's sub-blocks (at most 255 each), ended by an empty one.
fn blocks(out: &mut Vec<u8>, data: &[u8]) {
    for chunk in data.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0);
}

/// The loop's frames as an animated GIF, each pixel `scale` square, each
/// frame `delay_cs` hundredths of a second, playing for ever, saying `look`.
/// The first frame's colours are the GIF's; a frame with others carries its own.
pub fn encode(frames: &[Frame], scale: usize, delay_cs: u16, look: &str) -> Vec<u8> {
    let Some(first) = frames.first() else { return Vec::new() };
    let scale = scale.max(1);
    let (w, h) = (first.w * scale, first.h * scale);
    let bits = table_bits(first.colours.len());
    let table = |colours: &[[u8; 3]], bits: u8| {
        let mut t: Vec<u8> = colours.iter().flatten().copied().collect();
        t.resize(3 << bits, 0);
        t
    };
    let mut out = b"GIF89a".to_vec();
    out.extend((w as u16).to_le_bytes());
    out.extend((h as u16).to_le_bytes());
    out.push(0x80 | ((bits - 1) << 4) | (bits - 1));
    out.extend([0, 0]);
    out.extend(table(&first.colours, bits));
    // Played for ever (the Netscape extension every reader knows).
    out.extend(b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x00\x00\x00");
    // What it is, for Photos.
    out.extend([0x21, 0xFE]);
    blocks(&mut out, format!("look: {look}").as_bytes());
    for f in frames {
        out.extend([0x21, 0xF9, 0x04, 0x04]);
        out.extend(delay_cs.to_le_bytes());
        out.extend([0, 0]);
        out.push(0x2C);
        out.extend([0, 0, 0, 0]);
        out.extend((w as u16).to_le_bytes());
        out.extend((h as u16).to_le_bytes());
        let own = f.colours != first.colours;
        let fbits = if own { table_bits(f.colours.len()) } else { bits };
        if own {
            out.push(0x80 | (fbits - 1));
            out.extend(table(&f.colours, fbits));
        } else {
            out.push(0);
        }
        let mut data = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                data.push(f.idx[(y / scale) * f.w + x / scale]);
            }
        }
        let min = fbits.max(2);
        out.push(min);
        blocks(&mut out, &lzw(&data, min));
    }
    out.push(0x3B);
    out
}

/// Codes read back, least significant bit first.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    acc: u32,
    n: u32,
}

impl Reader<'_> {
    fn get(&mut self, size: u8) -> Option<u16> {
        while self.n < u32::from(size) {
            let b = *self.data.get(self.at)?;
            self.at += 1;
            self.acc |= u32::from(b) << self.n;
            self.n += 8;
        }
        let code = (self.acc & ((1 << size) - 1)) as u16;
        self.acc >>= size;
        self.n -= u32::from(size);
        Some(code)
    }
}

/// A GIF's LZW back to its indices (at most `limit`).
fn unlzw(data: &[u8], min: u8, limit: usize) -> Result<Vec<u8>, String> {
    if !(2..=11).contains(&min) {
        return Err("not a picture this reads".into());
    }
    let clear = 1u16 << min;
    let end = clear + 1;
    let mut size = min + 1;
    // Each code: its prefix code and its last index (the roots their own).
    let mut prefix: Vec<u16> = vec![u16::MAX; 4096];
    let mut suffix: Vec<u8> = (0..4096).map(|i| (i & 0xff) as u8).collect();
    let mut next = end + 1;
    let mut prev: Option<u16> = None;
    let mut out = Vec::with_capacity(limit);
    let mut reader = Reader { data, at: 0, acc: 0, n: 0 };
    let mut word = Vec::new();
    let spell = |code: u16, prefix: &[u16], suffix: &[u8], word: &mut Vec<u8>| {
        word.clear();
        let mut c = code;
        while c != u16::MAX && word.len() < 4096 {
            word.push(suffix[usize::from(c)]);
            c = prefix[usize::from(c)];
        }
        word.reverse();
    };
    while let Some(code) = reader.get(size) {
        if code == clear {
            size = min + 1;
            next = end + 1;
            prev = None;
            continue;
        }
        if code == end {
            break;
        }
        match prev {
            None => {
                if code >= clear {
                    return Err("the picture is damaged".into());
                }
                out.push(code as u8);
            }
            Some(p) => {
                if code < next {
                    spell(code, &prefix, &suffix, &mut word);
                } else if code == next {
                    spell(p, &prefix, &suffix, &mut word);
                    let first = word[0];
                    word.push(first);
                } else {
                    return Err("the picture is damaged".into());
                }
                if next < 4096 {
                    prefix[usize::from(next)] = p;
                    suffix[usize::from(next)] = word[0];
                    next += 1;
                    if next == 1 << size && size < 12 {
                        size += 1;
                    }
                }
                out.extend_from_slice(&word);
            }
        }
        prev = Some(code);
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    Ok(out)
}

/// A GIF's frames as pictures, each with how long it shows (ms); at most
/// `max_frames`. Frames are laid over the last (transparency kept), as
/// pixel mode's loops and most GIFs want.
pub fn decode(bytes: &[u8], max_frames: usize) -> Result<Vec<(Picture, u32)>, String> {
    let bad = || "not a picture this reads".to_owned();
    if bytes.len() < 13 || !(bytes.starts_with(b"GIF89a") || bytes.starts_with(b"GIF87a")) {
        return Err(bad());
    }
    let (w, h) = (usize::from(u16::from_le_bytes([bytes[6], bytes[7]])), usize::from(u16::from_le_bytes([bytes[8], bytes[9]])));
    if w == 0 || h == 0 || w * h > 4096 * 4096 {
        return Err(bad());
    }
    let mut at = 13;
    let mut global: Vec<[u8; 3]> = Vec::new();
    if bytes[10] & 0x80 != 0 {
        let n = 2usize << (bytes[10] & 7);
        global = bytes.get(at..at + 3 * n).ok_or_else(bad)?.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        at += 3 * n;
    }
    let mut canvas = Picture::new(w, h);
    let mut frames = Vec::new();
    let (mut delay, mut transparent): (u32, Option<u8>) = (100, None);
    let read_blocks = |at: &mut usize| -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        loop {
            let n = usize::from(*bytes.get(*at).ok_or_else(bad)?);
            *at += 1;
            if n == 0 {
                return Ok(data);
            }
            data.extend_from_slice(bytes.get(*at..*at + n).ok_or_else(bad)?);
            *at += n;
        }
    };
    while at < bytes.len() && frames.len() < max_frames {
        match bytes[at] {
            0x3B => break,
            0x21 => {
                let label = *bytes.get(at + 1).ok_or_else(bad)?;
                at += 2;
                let data = read_blocks(&mut at)?;
                if label == 0xF9 && data.len() >= 4 {
                    delay = u32::from(u16::from_le_bytes([data[1], data[2]])) * 10;
                    transparent = (data[0] & 1 == 1).then_some(data[3]);
                }
            }
            0x2C => {
                let d = bytes.get(at + 1..at + 10).ok_or_else(bad)?;
                let (fx, fy) = (usize::from(u16::from_le_bytes([d[0], d[1]])), usize::from(u16::from_le_bytes([d[2], d[3]])));
                let (fw, fh) = (usize::from(u16::from_le_bytes([d[4], d[5]])), usize::from(u16::from_le_bytes([d[6], d[7]])));
                let packed = d[8];
                at += 10;
                let local: Vec<[u8; 3]> = if packed & 0x80 != 0 {
                    let n = 2usize << (packed & 7);
                    let t = bytes.get(at..at + 3 * n).ok_or_else(bad)?.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                    at += 3 * n;
                    t
                } else {
                    global.clone()
                };
                let min = *bytes.get(at).ok_or_else(bad)?;
                at += 1;
                let data = read_blocks(&mut at)?;
                let idx = unlzw(&data, min, fw * fh)?;
                // Interlaced rows come every 8th from 0, every 8th from 4,
                // every 4th from 2, then every other from 1.
                let rows: Vec<usize> = if packed & 0x40 != 0 { (0..fh).step_by(8).chain((4..fh).step_by(8)).chain((2..fh).step_by(4)).chain((1..fh).step_by(2)).collect() } else { (0..fh).collect() };
                for (r, &row) in rows.iter().enumerate() {
                    for col in 0..fw {
                        let Some(&k) = idx.get(r * fw + col) else { break };
                        if Some(k) == transparent {
                            continue;
                        }
                        let (x, y) = (fx + col, fy + row);
                        if x < w && y < h {
                            if let Some(c) = local.get(usize::from(k)) {
                                canvas.rgb[(y * w + x) * 3..][..3].copy_from_slice(c);
                            }
                        }
                    }
                }
                frames.push((canvas.clone(), delay.max(20)));
                (delay, transparent) = (100, None);
            }
            _ => return Err(bad()),
        }
    }
    if frames.is_empty() {
        return Err(bad());
    }
    Ok(frames)
}

/// A GIF's size, from its header.
pub fn dimensions(bytes: &[u8]) -> Option<(usize, usize)> {
    (bytes.len() >= 10 && bytes.starts_with(b"GIF8")).then(|| (usize::from(u16::from_le_bytes([bytes[6], bytes[7]])), usize::from(u16::from_le_bytes([bytes[8], bytes[9]]))))
}

/// The "look: …" its comment says.
pub fn look(bytes: &[u8]) -> Option<String> {
    let at = bytes.windows(8).position(|w| w[0] == 0x21 && w[1] == 0xFE && &w[3..] == b"look:")?;
    let n = usize::from(bytes[at + 2]);
    let text = std::str::from_utf8(bytes.get(at + 3..at + 3 + n)?).ok()?;
    text.strip_prefix("look: ").map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::pixel::Palette;

    fn frame(w: usize, h: usize, colours: &[[u8; 3]], f: impl Fn(usize, usize) -> u8) -> Frame {
        Frame { w, h, idx: (0..w * h).map(|i| f(i % w, i / w)).collect(), colours: colours.to_vec(), palette: Palette::Auto }
    }

    #[test]
    fn a_loop_goes_into_a_gif_and_comes_back_frame_for_frame() {
        let colours = [[10u8, 20, 30], [200, 40, 40], [240, 230, 200], [40, 140, 60], [90, 90, 200]];
        // Noise-like content, so the dictionary fills and is cleared.
        let frames: Vec<Frame> = (0..6).map(|t| frame(75, 56, &colours, |x, y| ((x * 7 + y * 13 + t * 3 + (x * y) % 11) % 5) as u8)).collect();
        for scale in [1, 4] {
            let bytes = encode(&frames, scale, 12, "Pixel loop · auto · bayer");
            assert_eq!(dimensions(&bytes), Some((75 * scale, 56 * scale)));
            assert_eq!(look(&bytes).as_deref(), Some("Pixel loop · auto · bayer"));
            let back = decode(&bytes, 100).unwrap();
            assert_eq!(back.len(), frames.len());
            for ((pic, delay), f) in back.iter().zip(&frames) {
                assert_eq!(*delay, 120);
                let want = f.picture();
                for y in 0..pic.h {
                    for x in 0..pic.w {
                        assert_eq!(&pic.rgb[(y * pic.w + x) * 3..][..3], &want.rgb[((y / scale) * f.w + x / scale) * 3..][..3], "{scale}: {x} {y}");
                    }
                }
            }
        }
        // Two colours, flat: the smallest tables and codes.
        let flat = vec![frame(8, 8, &[[0, 0, 0], [255, 255, 255]], |x, _| (x % 2) as u8)];
        let back = decode(&encode(&flat, 1, 12, "x"), 10).unwrap();
        assert_eq!(back[0].0, flat[0].picture());
        // A frame with colours of its own carries them.
        let other = vec![frame(4, 4, &colours, |_, _| 1), frame(4, 4, &[[1, 2, 3], [9, 9, 9]], |_, _| 1)];
        let back = decode(&encode(&other, 1, 12, "x"), 10).unwrap();
        assert_eq!(&back[1].0.rgb[..3], &[9, 9, 9]);
        // Damaged or not a GIF: said, not a panic.
        assert!(decode(b"GIF89a\x01", 10).is_err());
        assert!(decode(&encode(&flat, 1, 12, "x")[..30], 10).is_err());
    }
}
