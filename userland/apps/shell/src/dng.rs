//! RAW photos: the sensor's own frame as a DNG beside the JPEG.
//!
//! In pro mode with RAW on, catalyst-camera asks for the IMX219's Bayer
//! frame with the photo (10-bit, packed four pixels to five bytes as the
//! CSI-2 link carries them), copies it into a memory file and hands it to
//! `catalyst-shell --write-dng` at idle priority, as detail mode hands its
//! burst to the merge. That writes it here: the pixels unpacked to 16 bits
//! (the 10-bit values kept as they are, black at 64 and white at 1023), in
//! one uncompressed strip, with the tags a raw developer needs
//! (darktable, RawTherapee, Lightroom, dcraw):
//!
//! - the Bayer pattern as the frame was read out (the sensor's flip
//!   changes it);
//! - two colour matrices worked out from the camera's own tuning file
//!   (libcamera's `imx219.json`): its colour correction matrix and white
//!   point at 2860 K (for Standard Light A) and 6504 K (D65), turned into
//!   XYZ → camera, as DNG asks, with a Bradford adaptation for A;
//! - the white balance as shot (AsShotNeutral, from the photo's own
//!   colour gains), the exposure, ISO, lens and time in an Exif directory.
//!
//! The frame needs no more of the camera's memory than a JPEG photo: the
//! pipeline keeps no raw frame of its own once the request brings one
//! (vc4.yaml, min_unicam_buffers 0), so 10 MB of raw frame takes the place
//! of the 10 MB it kept before.

use std::path::Path;

/// The raw frame as handed over.
#[derive(Clone, Debug, PartialEq)]
pub struct RawInfo {
    pub w: usize,
    pub h: usize,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// libcamera's name for it: "SBGGR10_CSI2P", "SRGGB10", …
    pub format: String,
    /// Black, in the sensor's 16-bit scale as libcamera reports it (4096 for
    /// 64 at 10 bits).
    pub black16: u32,
    /// The photo's white balance gains (red, blue).
    pub gains: [f32; 2],
    pub exposure_us: u32,
    pub gain: f32,
    /// Seconds since 1970.
    pub time: i64,
    /// Turned half round in software (the JPEG was): the raw needs it said.
    pub turned: bool,
    /// The look the JPEG wears, for the description.
    pub look: Option<String>,
}

impl RawInfo {
    /// Its bits a pixel, from the format's name (10 unless it says 8, 12 or 16).
    pub fn bits(&self) -> u32 {
        ["8", "12", "14", "16"].iter().find(|b| self.format.contains(&format!("GB{b}")) || self.format.contains(&format!("GG{b}")) || self.format.contains(&format!("RG{b}")) || self.format.contains(&format!("BG{b}"))).map_or(10, |b| b.parse().unwrap_or(10))
    }

    pub fn packed(&self) -> bool {
        self.format.ends_with("_CSI2P")
    }

    /// The Bayer pattern as DNG writes it (0 red, 1 green, 2 blue), top left first.
    pub fn cfa(&self) -> Option<[u8; 4]> {
        let order = self.format.trim_start_matches('S').get(..4)?;
        let mut out = [0u8; 4];
        for (o, c) in out.iter_mut().zip(order.chars()) {
            *o = match c {
                'R' => 0,
                'G' => 1,
                'B' => 2,
                _ => return None,
            };
        }
        Some(out)
    }

    pub fn white(&self) -> u32 {
        (1 << self.bits()) - 1
    }

    pub fn black(&self) -> u32 {
        self.black16 >> (16 - self.bits())
    }
}

/// A frame's pixels as 16-bit values (each the sensor's own number).
pub fn unpack(info: &RawInfo, data: &[u8]) -> Result<Vec<u16>, String> {
    let (w, h) = (info.w, info.h);
    let need = if info.packed() { w * info.bits() as usize / 8 } else { w * 2 };
    if info.stride < need || data.len() < info.stride * (h - 1) + need || w % 4 != 0 {
        return Err(format!("the raw frame is {} bytes, not {w} × {h} at {}", data.len(), info.format));
    }
    let mut out = vec![0u16; w * h];
    for y in 0..h {
        let row = &data[y * info.stride..];
        let dst = &mut out[y * w..(y + 1) * w];
        match (info.packed(), info.bits()) {
            (true, 10) => {
                for (px, group) in dst.chunks_exact_mut(4).zip(row.chunks_exact(5)) {
                    let low = group[4];
                    for (i, p) in px.iter_mut().enumerate() {
                        *p = (u16::from(group[i]) << 2) | u16::from((low >> (2 * i)) & 3);
                    }
                }
            }
            (true, 12) => {
                for (px, group) in dst.chunks_exact_mut(2).zip(row.chunks_exact(3)) {
                    px[0] = (u16::from(group[0]) << 4) | u16::from(group[2] & 15);
                    px[1] = (u16::from(group[1]) << 4) | u16::from(group[2] >> 4);
                }
            }
            (false, _) => {
                for (p, b) in dst.iter_mut().zip(row.chunks_exact(2)) {
                    *p = u16::from_le_bytes([b[0], b[1]]);
                }
            }
            (true, bits) => return Err(format!("{bits}-bit packed raw frames are not read here")),
        }
    }
    Ok(out)
}

// ------------------------------------------------------- colour matrices ---

/// What the tuning file says of colour: its colour correction matrices by
/// temperature, and where white falls (red ÷ green, blue ÷ green) at each.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub ccms: Vec<(f64, [f64; 9])>,
    pub ct_curve: Vec<(f64, f64, f64)>,
}

/// libcamera's tuning for the IMX219 on the Raspberry Pi (VC4).
pub const TUNING_FILE: &str = "/usr/share/libcamera/ipa/rpi/vc4/imx219.json";

impl Tuning {
    /// From a tuning file's text.
    pub fn parse(text: &str) -> Option<Tuning> {
        let root: serde_json::Value = serde_json::from_str(text).ok()?;
        let algorithms = root.get("algorithms")?.as_array()?;
        let find = |name: &str| algorithms.iter().find_map(|a| a.get(name));
        let ccms = find("rpi.ccm")?
            .get("ccms")?
            .as_array()?
            .iter()
            .filter_map(|c| {
                let ct = c.get("ct")?.as_f64()?;
                let m: Vec<f64> = c.get("ccm")?.as_array()?.iter().filter_map(serde_json::Value::as_f64).collect();
                (m.len() == 9).then(|| (ct, [m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]]))
            })
            .collect::<Vec<_>>();
        let curve: Vec<f64> = find("rpi.awb")?.get("ct_curve")?.as_array()?.iter().filter_map(serde_json::Value::as_f64).collect();
        let ct_curve = curve.chunks_exact(3).map(|c| (c[0], c[1], c[2])).collect::<Vec<_>>();
        (!ccms.is_empty() && ct_curve.len() >= 2).then_some(Tuning { ccms, ct_curve })
    }

    /// The handheld's tuning file, or (on a computer, or if it cannot be
    /// read) the same numbers as libcamera 0.7.1 ships them.
    pub fn load() -> Tuning {
        std::env::var("CATALYST_TUNING")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .or_else(|| std::fs::read_to_string(TUNING_FILE).ok())
            .and_then(|t| Tuning::parse(&t))
            .unwrap_or_else(Tuning::stock)
    }

    /// libcamera 0.7.1's `vc4/imx219.json`: its CCMs and white point curve.
    pub fn stock() -> Tuning {
        Tuning {
            ccms: vec![
                (2860.0, [2.12089, -0.52461, -0.59629, -0.85342, 2.80445, -0.95103, -0.26897, -1.14788, 2.41685]),
                (2960.0, [2.26962, -0.54174, -0.72789, -0.77008, 2.60271, -0.83262, -0.26036, -1.51254, 2.77289]),
                (3603.0, [2.18644, -0.66148, -0.52496, -0.77828, 2.69474, -0.91645, -0.25239, -0.83059, 2.08298]),
                (4650.0, [2.18174, -0.70887, -0.47287, -0.70196, 2.76426, -1.06231, -0.25157, -0.71978, 1.97135]),
                (5858.0, [2.32392, -0.88421, -0.43971, -0.63821, 2.58348, -0.94527, -0.28541, -0.54112, 1.82653]),
                (7580.0, [2.21175, -0.53242, -0.67933, -0.57875, 3.07922, -1.50047, -0.27709, -0.73338, 2.01048]),
            ],
            ct_curve: vec![(2498.0, 0.9309, 0.3599), (2911.0, 0.8682, 0.4283), (2919.0, 0.8358, 0.4621), (3627.0, 0.7646, 0.5327), (4600.0, 0.6079, 0.6721), (5716.0, 0.5712, 0.7017), (8575.0, 0.4331, 0.8037)],
        }
    }

    /// The CCM at a temperature (straight between the tuned ones).
    pub fn ccm(&self, ct: f64) -> [f64; 9] {
        let c = &self.ccms;
        if ct <= c[0].0 || c.len() == 1 {
            return c[0].1;
        }
        for pair in c.windows(2) {
            let ((t0, m0), (t1, m1)) = (pair[0], pair[1]);
            if ct <= t1 {
                let k = (ct - t0) / (t1 - t0);
                let mut m = [0.0; 9];
                for i in 0..9 {
                    m[i] = m0[i] + (m1[i] - m0[i]) * k;
                }
                return m;
            }
        }
        c[c.len() - 1].1
    }

    /// White at a temperature, as the camera sees it: red ÷ green, blue ÷ green.
    pub fn white(&self, ct: f64) -> (f64, f64) {
        let c = &self.ct_curve;
        let at = |i: usize| (c[i].1, c[i].2);
        if ct <= c[0].0 {
            return at(0);
        }
        for i in 1..c.len() {
            if ct <= c[i].0 {
                let k = (ct - c[i - 1].0) / (c[i].0 - c[i - 1].0);
                return (c[i - 1].1 + (c[i].1 - c[i - 1].1) * k, c[i - 1].2 + (c[i].2 - c[i - 1].2) * k);
            }
        }
        at(c.len() - 1)
    }

    /// DNG's ColorMatrix at a temperature: XYZ (under that light) to the
    /// camera's own RGB. The CCM takes white-balanced camera RGB to linear
    /// sRGB (a D65 white); sRGB goes to XYZ, Bradford carries D65's white
    /// to the light's, and the whole is turned round.
    pub fn colour_matrix(&self, ct: f64, white_xyz: [f64; 3]) -> [f64; 9] {
        let (r, b) = self.white(ct);
        let wb = [1.0 / r, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0 / b];
        let camera_to_xyz = mul(&bradford(D65, white_xyz), &mul(&SRGB_TO_XYZ, &mul(&self.ccm(ct), &wb)));
        invert(&camera_to_xyz).unwrap_or(IDENTITY)
    }
}

/// CIE illuminants' whites (XYZ, Y = 1).
pub const D65: [f64; 3] = [0.950_47, 1.0, 1.088_83];
pub const STD_A: [f64; 3] = [1.098_50, 1.0, 0.355_85];

const IDENTITY: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
const SRGB_TO_XYZ: [f64; 9] = [0.412_456_4, 0.357_576_1, 0.180_437_5, 0.212_672_9, 0.715_152_2, 0.072_175_0, 0.019_333_9, 0.119_192_0, 0.950_304_1];

/// Bradford's chromatic adaptation from one white to another.
fn bradford(from: [f64; 3], to: [f64; 3]) -> [f64; 9] {
    const M: [f64; 9] = [0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296];
    let (a, b) = (apply(&M, from), apply(&M, to));
    let scale = [b[0] / a[0], 0.0, 0.0, 0.0, b[1] / a[1], 0.0, 0.0, 0.0, b[2] / a[2]];
    mul(&invert(&M).unwrap_or(IDENTITY), &mul(&scale, &M))
}

pub fn mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    m
}

pub fn apply(m: &[f64; 9], v: [f64; 3]) -> [f64; 3] {
    [m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2], m[6] * v[0] + m[7] * v[1] + m[8] * v[2]]
}

pub fn invert(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ])
}

// ------------------------------------------------------------------ tiff ---

/// TIFF's types.
const BYTE: u16 = 1;
const ASCII: u16 = 2;
const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;
const SRATIONAL: u16 = 10;

/// One directory's entries: tag, type, count, value bytes.
#[derive(Default)]
struct Ifd(Vec<(u16, u16, u32, Vec<u8>)>);

impl Ifd {
    fn put(&mut self, tag: u16, kind: u16, count: u32, data: Vec<u8>) {
        self.0.push((tag, kind, count, data));
    }
    fn short(&mut self, tag: u16, v: &[u16]) {
        self.put(tag, SHORT, v.len() as u32, v.iter().flat_map(|x| x.to_le_bytes()).collect());
    }
    fn long(&mut self, tag: u16, v: u32) {
        self.put(tag, LONG, 1, v.to_le_bytes().to_vec());
    }
    fn bytes(&mut self, tag: u16, v: &[u8]) {
        self.put(tag, BYTE, v.len() as u32, v.to_vec());
    }
    fn ascii(&mut self, tag: u16, s: &str) {
        let mut d = s.as_bytes().to_vec();
        d.push(0);
        self.put(tag, ASCII, d.len() as u32, d);
    }
    fn rational(&mut self, tag: u16, v: &[(u32, u32)]) {
        self.put(tag, RATIONAL, v.len() as u32, v.iter().flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes())).collect());
    }
    fn srational(&mut self, tag: u16, v: &[f64]) {
        let d = 10_000i32;
        self.put(tag, SRATIONAL, v.len() as u32, v.iter().flat_map(|x| ((x * f64::from(d)).round() as i32).to_le_bytes().into_iter().chain(d.to_le_bytes())).collect());
    }

    /// Its length written: the entries, then each value too long to sit in
    /// one (kept to even offsets).
    fn len(&self) -> usize {
        2 + 12 * self.0.len() + 4 + self.0.iter().filter(|e| e.3.len() > 4).map(|e| (e.3.len() + 1) & !1).sum::<usize>()
    }

    /// Written at `offset` into the file, tags in order, pointing on to `next`.
    fn write(&mut self, out: &mut Vec<u8>, offset: usize, next: u32) {
        self.0.sort_by_key(|e| e.0);
        let mut extra = offset + 2 + 12 * self.0.len() + 4;
        out.extend((self.0.len() as u16).to_le_bytes());
        for (tag, kind, count, data) in &self.0 {
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            out.extend(count.to_le_bytes());
            if data.len() <= 4 {
                let mut v = data.clone();
                v.resize(4, 0);
                out.extend(v);
            } else {
                out.extend((extra as u32).to_le_bytes());
                extra += (data.len() + 1) & !1;
            }
        }
        out.extend(next.to_le_bytes());
        for (_, _, _, data) in &self.0 {
            if data.len() > 4 {
                out.extend(data);
                if data.len() % 2 == 1 {
                    out.push(0);
                }
            }
        }
    }
}

/// When, as TIFF writes it.
fn tiff_time(time: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(time, 0).single().map_or_else(|| "1970:01:01 00:00:00".into(), |t| t.format("%Y:%m:%d %H:%M:%S").to_string())
}

/// The DNG's header (everything before the pixels, which follow at its
/// end): one image directory holding the raw frame, and the Exif.
pub fn header(info: &RawInfo, tuning: &Tuning) -> Result<Vec<u8>, String> {
    let cfa = info.cfa().ok_or_else(|| format!("{} is not a Bayer frame", info.format))?;
    let (w, h) = (info.w as u32, info.h as u32);
    let when = tiff_time(info.time);
    let mut zero = Ifd::default();
    zero.long(254, 0);
    zero.long(256, w);
    zero.long(257, h);
    zero.short(258, &[16]);
    zero.short(259, &[1]);
    zero.short(262, &[32803]);
    if let Some(look) = &info.look {
        zero.ascii(270, &format!("look: {look} (on the JPEG; this is the sensor's frame)"));
    }
    zero.ascii(271, "M5Stack");
    zero.ascii(272, "CardputerZero");
    zero.long(273, 0);
    zero.short(274, &[if info.turned { 3 } else { 1 }]);
    zero.short(277, &[1]);
    zero.long(278, h);
    zero.long(279, w * h * 2);
    zero.short(284, &[1]);
    zero.ascii(305, &format!("Catalyst Linux {}", crate::sysupdate::version()));
    zero.ascii(306, &when);
    zero.short(33421, &[2, 2]);
    zero.bytes(33422, &cfa);
    zero.long(34665, 0);
    zero.bytes(50706, &[1, 4, 0, 0]);
    zero.bytes(50707, &[1, 1, 0, 0]);
    zero.ascii(50708, "M5Stack CardputerZero IMX219");
    zero.long(50714, info.black());
    zero.long(50717, info.white());
    zero.srational(50721, &tuning.colour_matrix(2860.0, STD_A));
    zero.srational(50722, &tuning.colour_matrix(6504.0, D65));
    let neutral = |g: f32| ((1_000_000.0 / f64::from(g.max(0.05))).round() as u32, 1_000_000);
    zero.rational(50728, &[neutral(info.gains[0]), (1, 1), neutral(info.gains[1])]);
    zero.short(50778, &[17]);
    zero.short(50779, &[21]);
    let mut exif = Ifd::default();
    let us = info.exposure_us.max(1);
    let g = gcd(us, 1_000_000);
    exif.rational(33434, &[(us / g, 1_000_000 / g)]);
    exif.rational(33437, &[(20, 10)]);
    exif.short(34855, &[(100.0 * info.gain).round().clamp(1.0, 65535.0) as u16]);
    exif.ascii(36867, &when);
    exif.rational(37386, &[(304, 100)]);
    exif.ascii(42036, "IMX219 3.04mm f/2.0 (main camera)");
    // Laid out: the header, the first directory, the Exif, then the pixels
    // (at a 16-byte boundary).
    let exif_at = 8 + zero.len();
    let pixels_at = (exif_at + exif.len() + 15) & !15;
    for e in &mut zero.0 {
        match e.0 {
            273 => e.3 = (pixels_at as u32).to_le_bytes().to_vec(),
            34665 => e.3 = (exif_at as u32).to_le_bytes().to_vec(),
            _ => {}
        }
    }
    let mut out = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    zero.write(&mut out, 8, 0);
    exif.write(&mut out, exif_at, 0);
    out.resize(pixels_at, 0);
    Ok(out)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The DNG written: beside its path, then renamed into place.
pub fn write(path: &Path, info: &RawInfo, pixels: &[u16], tuning: &Tuning) -> Result<(), String> {
    use std::io::Write;
    let head = header(info, tuning)?;
    let part = path.with_extension("dng.part");
    let file = std::fs::File::create(&part).map_err(|e| format!("the raw photo cannot be written: {e}"))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    let mut written = || -> std::io::Result<()> {
        out.write_all(&head)?;
        let mut row = Vec::with_capacity(info.w * 2);
        for line in pixels.chunks_exact(info.w) {
            row.clear();
            row.extend(line.iter().flat_map(|p| p.to_le_bytes()));
            out.write_all(&row)?;
        }
        out.flush()?;
        out.get_ref().sync_all()
    };
    if let Err(e) = written() {
        std::fs::remove_file(&part).ok();
        return Err(format!("the raw photo could not be written: {e}"));
    }
    std::fs::rename(&part, path).map_err(|e| format!("the raw photo could not be kept: {e}"))
}

/// `catalyst-shell --write-dng PATH w=… h=… stride=… format=… black=…
/// gains=r,b exp=… gain=… time=… turned=0|1 [look=…] -- fd:3`, started by
/// catalyst-camera with the frame in a memory file: writes the DNG and ends.
pub fn run(args: &[String]) -> i32 {
    let Some(at) = args.iter().position(|a| a == "--write-dng") else { return 2 };
    let Some(path) = args.get(at + 1) else { return 2 };
    let mut info = RawInfo { w: 0, h: 0, stride: 0, format: String::new(), black16: 4096, gains: [1.0, 1.0], exposure_us: 0, gain: 1.0, time: 0, turned: false, look: None };
    let mut fd = None;
    for word in &args[at + 2..] {
        if let Some(n) = word.strip_prefix("fd:") {
            fd = n.parse::<i32>().ok();
            continue;
        }
        let Some((k, v)) = word.split_once('=') else { continue };
        match k {
            "w" => info.w = v.parse().unwrap_or(0),
            "h" => info.h = v.parse().unwrap_or(0),
            "stride" => info.stride = v.parse().unwrap_or(0),
            "format" => info.format = v.to_owned(),
            "black" => info.black16 = v.parse().unwrap_or(4096),
            "gains" => {
                let g: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if g.len() == 2 {
                    info.gains = [g[0], g[1]];
                }
            }
            "exp" => info.exposure_us = v.parse().unwrap_or(0),
            "gain" => info.gain = v.parse().unwrap_or(1.0),
            "time" => info.time = v.parse().unwrap_or(0),
            "turned" => info.turned = v == "1",
            "look" => info.look = Some(v.replace('_', " ")),
            _ => {}
        }
    }
    let data = match fd.map(read_fd) {
        Some(Ok(d)) => d,
        Some(Err(e)) => {
            eprintln!("catalyst-shell: the raw frame: {e}");
            return 1;
        }
        None => return 2,
    };
    let result = unpack(&info, &data).and_then(|px| write(Path::new(path), &info, &px, &Tuning::load()));
    match result {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("catalyst-shell: {why}");
            1
        }
    }
}

#[cfg(unix)]
fn read_fd(fd: i32) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    // Handed over by catalyst-camera under this number, ours alone.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(data)
}

#[cfg(not(unix))]
fn read_fd(_fd: i32) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::other("no memory files here"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> RawInfo {
        RawInfo { w: 8, h: 4, stride: 12, format: "SBGGR10_CSI2P".into(), black16: 4096, gains: [1.75, 1.42], exposure_us: 8000, gain: 2.0, time: 1_795_000_000, turned: false, look: Some("Vivid".into()) }
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }
    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// Each entry of the directory at `at`: tag, type, count, value or offset.
    fn entries(b: &[u8], at: usize) -> Vec<(u16, u16, u32, u32)> {
        let n = usize::from(u16_at(b, at));
        (0..n).map(|i| at + 2 + 12 * i).map(|e| (u16_at(b, e), u16_at(b, e + 2), u32_at(b, e + 4), u32_at(b, e + 8))).collect()
    }

    #[test]
    fn the_dng_header_is_a_valid_tiff_with_the_tags_a_raw_developer_needs() {
        let tuning = Tuning::stock();
        let head = header(&info(), &tuning).unwrap();
        assert_eq!(&head[..4], b"II*\0");
        assert_eq!(u32_at(&head, 4), 8);
        let zero = entries(&head, 8);
        let tags: Vec<u16> = zero.iter().map(|e| e.0).collect();
        let mut sorted = tags.clone();
        sorted.sort_unstable();
        assert_eq!(tags, sorted, "tags in order, as TIFF asks");
        let get = |tag: u16| *zero.iter().find(|e| e.0 == tag).unwrap_or_else(|| panic!("tag {tag}"));
        assert_eq!(get(256).3, 8);
        assert_eq!(get(257).3, 4);
        assert_eq!((get(258).1, get(258).3), (SHORT, 16));
        assert_eq!(get(262).3, 32803, "a colour filter array");
        // The pixels right after the header, all of them.
        assert_eq!(get(273).3 as usize, head.len());
        assert_eq!(head.len() % 16, 0);
        assert_eq!(get(279).3, 8 * 4 * 2);
        // BGGR as read out: blue, green / green, red.
        assert_eq!(get(33422).3.to_le_bytes(), [2, 1, 1, 0]);
        assert_eq!(get(33421).3, 2 | (2 << 16));
        assert_eq!(get(50706).3.to_le_bytes(), [1, 4, 0, 0]);
        assert_eq!((get(50714).3, get(50717).3), (64, 1023), "10-bit black and white");
        assert_eq!((get(50778).3, get(50779).3), (17, 21), "illuminant A, then D65");
        // ColorMatrix2 as written, back from its rationals.
        let (kind, count, at) = (get(50722).1, get(50722).2, get(50722).3 as usize);
        assert_eq!((kind, count), (SRATIONAL, 9));
        let written: Vec<f64> = (0..9).map(|i| f64::from(u32_at(&head, at + 8 * i) as i32) / f64::from(u32_at(&head, at + 8 * i + 4) as i32)).collect();
        let want = tuning.colour_matrix(6504.0, D65);
        for (a, b) in written.iter().zip(want) {
            assert!((a - b).abs() < 2e-4, "{a} {b}");
        }
        // As shot: the photo's gains turned round.
        let neutral_at = get(50728).3 as usize;
        assert_eq!(u32_at(&head, neutral_at), (1_000_000.0f64 / 1.75).round() as u32);
        // The Exif: 1/125 s at f/2.0 and ISO 200.
        let exif = entries(&head, get(34665).3 as usize);
        let ex = |tag: u16| *exif.iter().find(|e| e.0 == tag).unwrap();
        let t = ex(33434).3 as usize;
        assert_eq!((u32_at(&head, t), u32_at(&head, t + 4)), (1, 125));
        assert_eq!(ex(34855).3, 200);
        // Bayer patterns by name.
        let mut i = info();
        i.format = "SRGGB10".into();
        assert_eq!((i.cfa(), i.packed(), i.bits()), (Some([0, 1, 1, 2]), false, 10));
        i.format = "SGBRG12_CSI2P".into();
        assert_eq!((i.cfa(), i.bits(), i.black()), (Some([1, 2, 0, 1]), 12, 256));
        i.format = "NV12".into();
        assert!(header(&i, &tuning).is_err());
    }

    #[test]
    fn the_colour_matrices_come_from_the_tuning_and_map_white_to_the_cameras_white() {
        let tuning = Tuning::stock();
        // D65's white lands on the camera's white at 6504 K, A's at 2860 K.
        for (ct, white) in [(6504.0, D65), (2860.0, STD_A)] {
            let cam = apply(&tuning.colour_matrix(ct, white), white);
            let (r, b) = tuning.white(ct);
            assert!((cam[0] / cam[1] - r).abs() < 1e-4 && (cam[2] / cam[1] - b).abs() < 1e-4, "{ct}: {cam:?} vs {r} {b}");
        }
        // The CCM between the tuned temperatures, and at their ends.
        assert_eq!(tuning.ccm(2000.0), tuning.ccms[0].1);
        let mid = tuning.ccm((5858.0 + 7580.0) / 2.0);
        assert!((mid[0] - (2.32392 + 2.21175) / 2.0).abs() < 1e-9);
        // The tuning file as libcamera ships it reads the same.
        let json = r#"{"version": 2.0, "algorithms": [
            {"rpi.awb": {"ct_curve": [2498.0, 0.9309, 0.3599, 8575.0, 0.4331, 0.8037]}},
            {"rpi.ccm": {"ccms": [{"ct": 2860, "ccm": [2.12089, -0.52461, -0.59629, -0.85342, 2.80445, -0.95103, -0.26897, -1.14788, 2.41685]}]}}]}"#;
        let t = Tuning::parse(json).unwrap();
        assert_eq!(t.ccms[0].1, Tuning::stock().ccms[0].1);
        assert_eq!(t.white(2498.0), (0.9309, 0.3599));
        assert!(Tuning::parse("{}").is_none());
    }

    #[test]
    fn packed_raw_frames_unpack_to_the_sensors_own_numbers() {
        // Four pixels in five bytes: high bits, then the low two of each.
        let px = [1023u16, 0, 512, 77];
        let mut row: Vec<u8> = px.iter().map(|p| (p >> 2) as u8).collect();
        row.push(px.iter().enumerate().fold(0u8, |acc, (i, p)| acc | (((p & 3) as u8) << (2 * i))));
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&row);
            data.extend_from_slice(&row);
            data.extend_from_slice(&[0, 0]); // the stride's padding
        }
        let i = RawInfo { w: 8, h: 2, ..info() };
        let out = unpack(&i, &data).unwrap();
        assert_eq!(&out[..8], &[1023, 0, 512, 77, 1023, 0, 512, 77]);
        assert!(unpack(&i, &data[..10]).is_err(), "too short");
        // Written out: the header, then the pixels, little-endian.
        let dir = std::env::temp_dir().join(format!("catalyst-dng-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("IMG_1.dng");
        write(&path, &i, &out, &Tuning::stock()).unwrap();
        let file = std::fs::read(&path).unwrap();
        let head = header(&i, &Tuning::stock()).unwrap();
        assert_eq!(file.len(), head.len() + 8 * 2 * 2);
        assert_eq!(u16_at(&file, head.len()), 1023);
        assert_eq!(u16_at(&file, head.len() + 6), 77);
        std::fs::remove_dir_all(&dir).ok();
    }
}
