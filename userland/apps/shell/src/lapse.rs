//! Time-lapses (1.5.0): what the Camera's lapse mode keeps, and how Photos
//! plays it.
//!
//! **The file: Motion JPEG in an AVI** (`LAPSE_20261124_101500.avi`, in
//! the album beside the photos). Each frame is a JPEG of its own, as the
//! camera wrote it (960 × 720, quality 85, 4:2:0), so a frame looks like a
//! photo, not a GIF's 256 colours; the container is the one digital
//! cameras recorded video in for a decade, and every iPhone's
//! specifications list it ("Motion JPEG (M-JPEG) up to 35 Mbps, 1280 by
//! 720 pixels, 30 frames per second … in .avi file format"): a time-lapse
//! downloaded to a phone plays in its own Photos and Files, and in VLC and
//! on any computer. 960 × 720 at up to 15 frames a second is inside those
//! limits (about 12 Mbit/s).
//!
//! **Why not MP4.** What the image has was looked at (1.5.0): its ffmpeg
//! is built with one encoder, `pcm_s16le`, and no video encoder at all (no
//! x264, no `h264_v4l2m2m`); the VideoCore's own H.264 encoder is there as
//! a module (`bcm2835-codec.ko`, CONFIG_VIDEO_CODEC_BCM2835=m, loadable
//! once `vchiq` is up, which the camera loads), but driving it means a
//! V4L2 memory-to-memory encoder and an MP4 muxer that can only be proven
//! on the handheld, taking its buffers from the same 128 MB CMA pool the
//! photos already fill; and Photos would then need an H.264 decoder to play
//! it, which the shell has not got. The JPEG decoder it has already plays
//! these, a frame at a time. If MP4 comes later, these frames are exactly
//! what that encoder would take.
//!
//! **While it is taken** each frame is a file of its own in a folder out
//! of the album (`.lapse/LAPSE_…/0001.jpg`), so a time-lapse stopped by
//! anything (Esc, a flat battery, the shell ending) keeps every frame
//! taken; the AVI is made from them at the end ([`assemble`]), and a folder
//! left behind is made into its AVI the next time the Camera opens
//! ([`recover`]). Making it reads one frame at a time: its memory is one
//! frame's bytes and the index (16 bytes a frame).
//!
//! **Playing it** ([`Player`]): frames decoded in order on a thread, two
//! ahead, at the size the panel needs (the decoder's own halving: 480 ×
//! 360), so a time-lapse of any length plays in about a megabyte and a
//! half.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;

use chrono::NaiveDateTime;

use crate::photos::Picture;

/// A frame's size: 4:3 as the sensor, and inside what a phone plays as
/// Motion JPEG (1280 × 720 at the most).
pub const FRAME: (u32, u32) = (960, 720);
/// Its JPEG quality: a video's, not a photo's 95 (a third of the bytes,
/// and no difference a frame shown for a fifteenth of a second shows).
pub const QUALITY: u32 = 85;
/// The most frames one takes: two hours at five seconds, two minutes of
/// video at 15 a second (about 180 MB), as much as a phone takes in.
pub const MOST_FRAMES: u32 = 1800;

// ----------------------------------------------------------------- names ---

/// Whether a file is a time-lapse (the album plays it).
pub fn is_lapse(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.starts_with('.') && lower.ends_with(".avi")
}

/// The folder frames wait in while one is taken, out of the album.
pub fn work_root() -> PathBuf {
    crate::photos::dir().join(".lapse")
}

/// Where a time-lapse started at `now` goes, in `dir`: `LAPSE_…avi`, never
/// over one there or one still being taken (`_2`, `_3`).
pub fn new_path_in(dir: &Path, now: NaiveDateTime) -> PathBuf {
    let stem = format!("LAPSE_{}", now.format("%Y%m%d_%H%M%S"));
    let mut path = dir.join(format!("{stem}.avi"));
    let mut n = 2;
    while path.exists() || dir.join(".lapse").join(path.file_stem().unwrap_or_default()).exists() {
        path = dir.join(format!("{stem}_{n}.avi"));
        n += 1;
    }
    path
}

/// Where the next time-lapse goes in the album, its moment noted for the
/// build log (photolog/: the Wi-Fi's place, the event), as a photo's is.
pub fn new_path(now: NaiveDateTime) -> PathBuf {
    let path = new_path_in(&crate::photos::dir(), now);
    crate::photolog::taken(&path, now);
    path
}

/// A time-lapse's frames' folder while it is taken.
pub fn work_of(lapse: &Path) -> PathBuf {
    lapse.parent().unwrap_or(Path::new(".")).join(".lapse").join(lapse.file_stem().unwrap_or_default())
}

/// Its `n`th frame (from 1): `0001.jpg`.
pub fn frame_path(work: &Path, n: u32) -> PathBuf {
    work.join(format!("{n:04}.jpg"))
}

// ------------------------------------------------------------- decisions ---

/// The intervals offered (seconds between frames): two at the least (a
/// frame and its save take about one), a minute at the most.
pub const EVERY: [u32; 5] = [2, 5, 10, 30, 60];
pub const EVERY_DEFAULT: u32 = 5;

/// Frames a second it plays at: a short one over about eight seconds (five
/// a second at the least, so it still moves), a long one at 15 (smooth
/// enough to watch a robot come together: an hour at five seconds in 48 s).
pub fn playback_fps(frames: u32) -> u32 {
    ((frames as f32 / 8.0).round() as u32).clamp(5, 15)
}

/// How long `frames` play for (seconds).
pub fn plays_for(frames: u32) -> f32 {
    frames as f32 / playback_fps(frames) as f32
}

/// "48 s", "1 min 12 s".
pub fn seconds_words(s: f32) -> String {
    let s = s.round().max(0.0) as u32;
    crate::camkit::duration_words(s)
}

/// What an hour at `every_s` plays as: "an hour plays in 48 s".
pub fn hour_words(every_s: u32) -> String {
    let frames = (3600 / every_s.max(1)).min(MOST_FRAMES);
    format!("1 h plays in {}", seconds_words(plays_for(frames)))
}

// ---------------------------------------------------------------- the AVI ---

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// A JPEG without its APP1 segments (Exif, XMP): a frame of a video carries
/// none, and some players take a frame's Exif for a picture of its own.
pub fn strip_app1(jpeg: &[u8]) -> Vec<u8> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return jpeg.to_vec();
    }
    let mut out = Vec::with_capacity(jpeg.len());
    out.extend_from_slice(&jpeg[..2]);
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        // The scan: everything from here on, as it is.
        if marker == 0xDA {
            break;
        }
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        let end = (at + 2 + len).min(jpeg.len());
        if marker != 0xE1 {
            out.extend_from_slice(&jpeg[at..end]);
        }
        at = end;
    }
    out.extend_from_slice(&jpeg[at.min(jpeg.len())..]);
    out
}

/// A JPEG's size, from its frame header.
pub fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        // SOF0 to SOF15, less DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = u32::from(u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]));
            let w = u32::from(u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]));
            return Some((w, h));
        }
        if marker == 0xDA {
            return None;
        }
        at += 2 + len;
    }
    None
}

/// The AVI's frames, each a JPEG's bytes, written to `out` (beside it as
/// `.part`, renamed into place), at `fps`, `w` × `h`. Takes the frames one
/// at a time from `next`, which gives none at the end.
pub fn write_avi(out: &Path, w: u32, h: u32, fps: u32, mut next: impl FnMut() -> Option<Vec<u8>>) -> Result<u32, String> {
    let part = out.with_extension("avi.part");
    let file = std::fs::File::create(&part).map_err(|e| format!("the time-lapse cannot be written: {e}"))?;
    let mut f = std::io::BufWriter::new(file);
    let io = |e: std::io::Error| format!("the time-lapse cannot be written: {e}");
    // The headers, their counts and sizes filled in at the end.
    let us_per_frame = 1_000_000 / fps.max(1);
    let mut head = Vec::new();
    head.extend_from_slice(b"RIFF\0\0\0\0AVI ");
    head.extend_from_slice(b"LIST");
    head.extend_from_slice(&le32(4 + 8 + 56 + 8 + 4 + 8 + 56 + 8 + 40));
    head.extend_from_slice(b"hdrl");
    head.extend_from_slice(b"avih");
    head.extend_from_slice(&le32(56));
    for v in [us_per_frame, 0, 0, 0x10, 0, 0, 1, 0, w, h, 0, 0, 0, 0] {
        head.extend_from_slice(&le32(v));
    }
    head.extend_from_slice(b"LIST");
    head.extend_from_slice(&le32(4 + 8 + 56 + 8 + 40));
    head.extend_from_slice(b"strl");
    head.extend_from_slice(b"strh");
    head.extend_from_slice(&le32(56));
    head.extend_from_slice(b"vidsMJPG");
    for v in [0u32, 0, 0, 1, fps.max(1), 0, 0, 0, u32::MAX, 0] {
        head.extend_from_slice(&le32(v));
    }
    for v in [0u16, 0, w as u16, h as u16] {
        head.extend_from_slice(&v.to_le_bytes());
    }
    head.extend_from_slice(b"strf");
    head.extend_from_slice(&le32(40));
    head.extend_from_slice(&le32(40));
    head.extend_from_slice(&le32(w));
    head.extend_from_slice(&le32(h));
    head.extend_from_slice(&1u16.to_le_bytes());
    head.extend_from_slice(&24u16.to_le_bytes());
    head.extend_from_slice(b"MJPG");
    for v in [w * h * 3, 0, 0, 0, 0] {
        head.extend_from_slice(&le32(v));
    }
    let movi_at = head.len() as u32;
    head.extend_from_slice(b"LIST\0\0\0\0movi");
    f.write_all(&head).map_err(io)?;
    let mut index: Vec<(u32, u32)> = Vec::new();
    let mut at = 4u32;
    let mut biggest = 0u32;
    while let Some(jpeg) = next() {
        let len = jpeg.len() as u32;
        f.write_all(b"00dc").map_err(io)?;
        f.write_all(&le32(len)).map_err(io)?;
        f.write_all(&jpeg).map_err(io)?;
        if len % 2 == 1 {
            f.write_all(&[0]).map_err(io)?;
        }
        index.push((at, len));
        biggest = biggest.max(len);
        at += 8 + len + len % 2;
    }
    if index.is_empty() {
        drop(f);
        std::fs::remove_file(&part).ok();
        return Err("no frames to make it of".into());
    }
    let movi_len = at;
    f.write_all(b"idx1").map_err(io)?;
    f.write_all(&le32(16 * index.len() as u32)).map_err(io)?;
    for (offset, len) in &index {
        f.write_all(b"00dc").map_err(io)?;
        f.write_all(&le32(0x10)).map_err(io)?;
        f.write_all(&le32(*offset)).map_err(io)?;
        f.write_all(&le32(*len)).map_err(io)?;
    }
    let frames = index.len() as u32;
    let mut file = f.into_inner().map_err(|e| format!("the time-lapse cannot be written: {e}"))?;
    let riff_len = movi_at + 8 + movi_len + 8 + 16 * frames - 8;
    let patch = |file: &mut std::fs::File, at: u64, v: u32| -> std::io::Result<()> {
        file.seek(SeekFrom::Start(at))?;
        file.write_all(&le32(v))
    };
    (|| -> std::io::Result<()> {
        patch(&mut file, 4, riff_len)?;
        // avih: bytes a second, frames, the largest frame.
        let avih = 12 + 12 + 8;
        patch(&mut file, avih + 4, biggest.saturating_mul(fps.max(1)))?;
        patch(&mut file, avih + 16, frames)?;
        patch(&mut file, avih + 28, biggest)?;
        // strh: its length and the largest frame.
        let strh = avih + 56 + 12 + 8;
        patch(&mut file, strh + 32, frames)?;
        patch(&mut file, strh + 36, biggest)?;
        patch(&mut file, u64::from(movi_at) + 4, movi_len)?;
        file.sync_all()
    })()
    .map_err(io)?;
    drop(file);
    std::fs::rename(&part, out).map_err(|e| format!("the time-lapse cannot be kept: {e}"))?;
    Ok(frames)
}

/// What a time-lapse's AVI says of itself, and where each frame is.
#[derive(Clone, Debug, PartialEq)]
pub struct Avi {
    pub w: u32,
    pub h: u32,
    /// Each frame's time on screen (ms).
    pub frame_ms: u32,
    /// Each frame's JPEG: where in the file, and how long.
    pub frames: Vec<(u64, u32)>,
}

impl Avi {
    /// Read from its headers and index (or, with no index, its frames in turn).
    pub fn open(path: &Path) -> Result<Avi, String> {
        let mut file = std::fs::File::open(path).map_err(|e| format!("cannot open the time-lapse: {e}"))?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let damaged = || "the time-lapse is damaged".to_owned();
        let mut head = [0u8; 12];
        file.read_exact(&mut head).map_err(|_| damaged())?;
        if &head[..4] != b"RIFF" || &head[8..] != b"AVI " {
            return Err("not a time-lapse this plays".into());
        }
        let (mut w, mut h, mut us, mut movi, mut index) = (0u32, 0u32, 66_666u32, None, None);
        // Its chunks, and inside its lists.
        let mut at = 12u64;
        while at + 8 <= size {
            file.seek(SeekFrom::Start(at)).map_err(|_| damaged())?;
            let mut c = [0u8; 12];
            file.read_exact(&mut c[..8]).map_err(|_| damaged())?;
            let len = u64::from(u32::from_le_bytes([c[4], c[5], c[6], c[7]]));
            match &c[..4] {
                b"LIST" => {
                    file.read_exact(&mut c[8..12]).map_err(|_| damaged())?;
                    match &c[8..12] {
                        b"movi" => {
                            movi = Some((at + 8, len));
                            at += 8 + len + len % 2;
                        }
                        // Into it: its chunks are read as the file's.
                        _ => at += 12,
                    }
                    continue;
                }
                b"avih" if len >= 40 => {
                    let mut b = [0u8; 40];
                    file.read_exact(&mut b).map_err(|_| damaged())?;
                    let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                    us = word(0).max(1000);
                    w = word(32);
                    h = word(36);
                }
                b"idx1" => index = Some((at + 8, len)),
                _ => {}
            }
            at += 8 + len + len % 2;
        }
        let Some((movi_at, movi_len)) = movi else { return Err(damaged()) };
        let mut frames = Vec::new();
        if let Some((idx_at, idx_len)) = index {
            let mut idx = vec![0u8; idx_len.min(16 * u64::from(MOST_FRAMES) * 4) as usize];
            file.seek(SeekFrom::Start(idx_at)).map_err(|_| damaged())?;
            file.read_exact(&mut idx).map_err(|_| damaged())?;
            for e in idx.chunks_exact(16) {
                if &e[..2] != b"00" {
                    continue;
                }
                let offset = u64::from(u32::from_le_bytes([e[8], e[9], e[10], e[11]]));
                let len = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
                // From the `movi` word (as written here, and by most), or
                // from the file's start (as some write it).
                let from_movi = movi_at + offset + 8;
                let start = if from_movi + u64::from(len) <= size { from_movi } else { offset + 8 };
                if start + u64::from(len) <= size {
                    frames.push((start, len));
                }
            }
        }
        // No index (or none that fits): the frames in turn.
        if frames.is_empty() {
            let mut at = movi_at + 4;
            while at + 8 <= (movi_at + movi_len).min(size) {
                file.seek(SeekFrom::Start(at)).map_err(|_| damaged())?;
                let mut c = [0u8; 8];
                file.read_exact(&mut c).map_err(|_| damaged())?;
                let len = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                if &c[2..4] == b"dc" || &c[2..4] == b"db" {
                    frames.push((at + 8, len));
                }
                at += 8 + u64::from(len) + u64::from(len % 2);
            }
        }
        if frames.is_empty() {
            return Err(damaged());
        }
        Ok(Avi { w, h, frame_ms: (us / 1000).max(1), frames })
    }

    /// Its `i`th frame's JPEG.
    pub fn jpeg(&self, file: &mut std::fs::File, i: usize) -> Result<Vec<u8>, String> {
        let (at, len) = self.frames.get(i).copied().ok_or("no such frame")?;
        let mut out = vec![0u8; len as usize];
        file.seek(SeekFrom::Start(at)).and_then(|_| file.read_exact(&mut out)).map_err(|_| "the time-lapse is damaged".to_owned())?;
        Ok(out)
    }

    /// How long it plays (s).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn seconds(&self) -> f32 {
        self.frames.len() as f32 * self.frame_ms as f32 / 1000.0
    }
}

/// A JPEG frame decoded at the smallest of its whole, half, quarter or
/// eighth sizes that is at least `want_w` × `want_h`.
pub fn decode(jpeg: &[u8], want_w: usize, want_h: usize) -> Result<Picture, String> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(jpeg));
    let (w, h) = decoder.scale(want_w.min(65535) as u16, want_h.min(65535) as u16).map_err(|e| format!("a frame this cannot read: {e}"))?;
    let data = decoder.decode().map_err(|e| format!("a frame is damaged: {e}"))?;
    let info = decoder.info().ok_or("a frame has no picture")?;
    let (w, h) = (usize::from(w), usize::from(h));
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => data,
        jpeg_decoder::PixelFormat::L8 => data.iter().flat_map(|v| [*v; 3]).collect(),
        _ => return Err("a frame this cannot read".into()),
    };
    if rgb.len() < w * h * 3 {
        return Err("a frame is damaged".into());
    }
    Ok(Picture { w, h, rgb })
}

/// A time-lapse's first frame, for its thumbnail and size.
pub fn first_frame(path: &Path, want_w: usize, want_h: usize) -> Result<Picture, String> {
    let avi = Avi::open(path)?;
    let mut file = std::fs::File::open(path).map_err(|e| format!("cannot open the time-lapse: {e}"))?;
    decode(&avi.jpeg(&mut file, 0)?, want_w, want_h)
}

// ------------------------------------------------------- making the file ---

/// A time-lapse's frames, in order (`0001.jpg` …).
pub fn frames_in(work: &Path) -> Vec<PathBuf> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(work)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jpg") && p.file_stem().is_some_and(|s| s.to_string_lossy().chars().all(|c| c.is_ascii_digit())))
        .collect();
    frames.sort();
    frames
}

/// What became of a time-lapse's frames.
#[derive(Clone, Debug, PartialEq)]
pub enum Made {
    /// The AVI: its frames, and its size.
    Lapse { path: PathBuf, frames: u32, size: (u32, u32) },
    /// One frame alone: kept as a photo.
    Photo(PathBuf),
    /// None taken.
    Nothing,
}

/// The frames taken for `lapse` made into it (the work folder gone once it
/// is); one frame alone becomes a photo in the album, under its own name.
pub fn assemble(lapse: &Path) -> Result<Made, String> {
    // One at a time: a Camera closing makes its own while the next one
    // opening looks for any left behind.
    static MAKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = MAKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let work = work_of(lapse);
    let frames = frames_in(&work);
    let made = match frames.len() {
        0 => Made::Nothing,
        1 => {
            let stem = lapse.file_stem().map(|s| s.to_string_lossy().replace("LAPSE_", "IMG_")).unwrap_or_else(|| "IMG_lapse".into());
            let mut photo = lapse.with_file_name(format!("{stem}.jpg"));
            let mut n = 2;
            while photo.exists() {
                photo = lapse.with_file_name(format!("{stem}_{n}.jpg"));
                n += 1;
            }
            std::fs::rename(&frames[0], &photo).or_else(|_| std::fs::copy(&frames[0], &photo).map(|_| ())).map_err(|e| format!("the frame cannot be kept: {e}"))?;
            Made::Photo(photo)
        }
        n => {
            let first = std::fs::read(&frames[0]).map_err(|e| format!("a frame cannot be read: {e}"))?;
            let (w, h) = jpeg_size(&first).unwrap_or(FRAME);
            let mut i = 0;
            let fps = playback_fps(n as u32);
            // Frames of another size (a failed frame's fallback) left out.
            let mut next = || {
                while i < frames.len() {
                    let at = i;
                    i += 1;
                    if let Ok(bytes) = std::fs::read(&frames[at]) {
                        if jpeg_size(&bytes) == Some((w, h)) {
                            return Some(strip_app1(&bytes));
                        }
                    }
                }
                None
            };
            let count = write_avi(lapse, w, h, fps, &mut next)?;
            Made::Lapse { path: lapse.to_path_buf(), frames: count, size: (w, h) }
        }
    };
    std::fs::remove_dir_all(&work).ok();
    Ok(made)
}

/// Time-lapses left as frames (the Camera closed, the shell ended, the
/// battery went while one was taken) made into their files now; `taking`
/// is one still being taken, left alone. What was made.
pub fn recover(taking: Option<&Path>) -> Vec<Made> {
    let root = work_root();
    let Ok(dirs) = std::fs::read_dir(&root) else { return Vec::new() };
    let skip = taking.map(work_of);
    let mut made = Vec::new();
    for dir in dirs.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        if skip.as_ref() == Some(&dir) {
            continue;
        }
        let Some(stem) = dir.file_name().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let lapse = crate::photos::dir().join(format!("{stem}.avi"));
        match assemble(&lapse) {
            Ok(m) => made.push(m),
            Err(why) => eprintln!("lapse: {stem} could not be made: {why}"),
        }
    }
    made
}

// ---------------------------------------------------------------- playing ---

/// A time-lapse playing: its frames decoded in order on a thread, a couple
/// ahead, at the size asked; the frame in sight and how long it has left.
pub struct Player {
    rx: Receiver<Result<(usize, Picture), String>>,
    stop: Arc<AtomicBool>,
    pub frames: usize,
    pub frame_ms: u32,
    pub size: (u32, u32),
    pub shown: Option<(usize, Picture)>,
    left_ms: f32,
    pub trouble: Option<String>,
}

impl Player {
    pub fn open(path: &Path, want_w: usize, want_h: usize) -> Result<Player, String> {
        let avi = Avi::open(path)?;
        let (tx, rx): (SyncSender<Result<(usize, Picture), String>>, _) = mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let (path, halt, index) = (path.to_path_buf(), stop.clone(), avi.clone());
        std::thread::Builder::new()
            .name("lapse-play".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let Ok(mut file) = std::fs::File::open(&path) else {
                    tx.send(Err("cannot open the time-lapse".into())).ok();
                    return;
                };
                let mut i = 0;
                while !halt.load(Ordering::Relaxed) {
                    let frame = index.jpeg(&mut file, i).and_then(|j| decode(&j, want_w, want_h)).map(|p| (i, p));
                    let failed = frame.is_err();
                    if tx.send(frame).is_err() || failed {
                        return;
                    }
                    i = (i + 1) % index.frames.len();
                }
            })
            .map_err(|e| format!("cannot play it: {e}"))?;
        Ok(Player { rx, stop, frames: avi.frames.len(), frame_ms: avi.frame_ms, size: (avi.w, avi.h), shown: None, left_ms: 0.0, trouble: None })
    }

    /// Time passing: the next frame when it is due and decoded (a frame
    /// late to decode is shown late, never skipped). True if it changed.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.left_ms -= dt * 1000.0;
        if self.left_ms > 0.0 && self.shown.is_some() {
            return false;
        }
        match self.rx.try_recv() {
            Ok(Ok(frame)) => {
                self.shown = Some(frame);
                // Due from when the last was due, never further behind than a frame.
                self.left_ms = (self.left_ms + self.frame_ms as f32).max(self.frame_ms as f32 * 0.5);
                true
            }
            Ok(Err(why)) => {
                self.trouble = Some(why);
                true
            }
            Err(_) => false,
        }
    }

    /// How soon it wants looking at again (ms).
    pub fn due_ms(&self) -> f32 {
        if self.shown.is_none() {
            16.0
        } else {
            self.left_ms.max(4.0)
        }
    }

    pub fn seconds(&self) -> f32 {
        self.frames as f32 * self.frame_ms as f32 / 1000.0
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Its thread may be waiting to hand a frame over: taken, so it sees the stop.
        while self.rx.try_recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-lapse-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A frame as the camera writes one: a JPEG of `w` × `h`, a shade of
    /// its own, with an Exif segment.
    fn frame(w: usize, h: usize, shade: u8) -> Vec<u8> {
        let mut yuv = vec![128u8; w * h * 3 / 2];
        for (i, p) in yuv[..w * h].iter_mut().enumerate() {
            *p = shade.wrapping_add(((i % w) / 8) as u8);
        }
        let path = std::env::temp_dir().join(format!("catalyst-lapse-frame-{}-{shade}.jpg", std::process::id()));
        crate::burst::write_jpeg(&path, &yuv, w, h, Some(b"Exif\0\0not really".to_vec())).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        bytes
    }

    #[test]
    fn names_are_lapse_and_never_over_one_there_or_being_taken() {
        let dir = scratch("names");
        let at = NaiveDateTime::parse_from_str("20261124_101500", "%Y%m%d_%H%M%S").unwrap();
        let first = new_path_in(&dir, at);
        assert_eq!(first.file_name().unwrap(), "LAPSE_20261124_101500.avi");
        assert!(is_lapse("LAPSE_20261124_101500.avi") && is_lapse("x.AVI") && !is_lapse(".lapse") && !is_lapse("IMG_1.jpg"));
        // One being taken (its frames' folder) holds its name too.
        std::fs::create_dir_all(work_of(&first)).unwrap();
        let second = new_path_in(&dir, at);
        assert_eq!(second.file_name().unwrap(), "LAPSE_20261124_101500_2.avi");
        assert_eq!(work_of(&first), dir.join(".lapse").join("LAPSE_20261124_101500"));
        assert_eq!(frame_path(&work_of(&first), 7).file_name().unwrap(), "0007.jpg");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn it_plays_at_five_to_fifteen_frames_a_second() {
        assert_eq!(playback_fps(2), 5, "a short one still moves");
        assert_eq!(playback_fps(40), 5);
        assert_eq!(playback_fps(80), 10, "about eight seconds");
        assert_eq!(playback_fps(720), 15, "an hour at 5 s: 15 a second");
        assert!((plays_for(720) - 48.0).abs() < 0.01);
        assert_eq!(hour_words(5), "1 h plays in 48 s");
        assert_eq!(hour_words(60), "1 h plays in 8 s");
        assert!(EVERY.contains(&EVERY_DEFAULT) && EVERY.windows(2).all(|p| p[0] < p[1]));
        // Inside what a phone plays as Motion JPEG.
        assert!(FRAME.0 <= 1280 && FRAME.1 <= 720 && playback_fps(MOST_FRAMES) <= 30);
    }

    #[test]
    fn a_frames_exif_is_taken_out_and_its_picture_kept() {
        let jpeg = frame(64, 48, 40);
        assert!(jpeg.windows(4).any(|w| w == b"Exif"));
        let bare = strip_app1(&jpeg);
        assert!(!bare.windows(4).any(|w| w == b"Exif") && bare.len() < jpeg.len());
        assert_eq!(jpeg_size(&bare), Some((64, 48)));
        let a = decode(&jpeg, 64, 48).unwrap();
        let b = decode(&bare, 64, 48).unwrap();
        assert_eq!(a, b, "the same picture");
        assert_eq!(strip_app1(b"not a jpeg"), b"not a jpeg");
    }

    #[test]
    fn an_avi_is_written_read_back_and_played() {
        let dir = scratch("avi");
        let path = dir.join("LAPSE_1.avi");
        let frames: Vec<Vec<u8>> = (0..7).map(|i| strip_app1(&frame(96, 72, 20 + 30 * i))).collect();
        let mut it = frames.clone().into_iter();
        assert_eq!(write_avi(&path, 96, 72, 5, || it.next()).unwrap(), 7);
        assert!(!path.with_extension("avi.part").exists());
        let bytes = std::fs::read(&path).unwrap();
        // A RIFF AVI of Motion JPEG, its sizes right.
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"AVI ");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
        assert!(bytes.windows(8).any(|w| w == b"vidsMJPG"));
        let avi = Avi::open(&path).unwrap();
        assert_eq!((avi.w, avi.h, avi.frames.len(), avi.frame_ms), (96, 72, 7, 200));
        assert!((avi.seconds() - 1.4).abs() < 0.01);
        let mut file = std::fs::File::open(&path).unwrap();
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(&avi.jpeg(&mut file, i).unwrap(), f, "frame {i} as written");
        }
        assert_eq!(first_frame(&path, 96, 72).unwrap().w, 96);
        // Played: in order, round and round, at its pace.
        let mut player = Player::open(&path, 48, 36).unwrap();
        assert_eq!((player.frames, player.frame_ms), (7, 200));
        let mut seen = Vec::new();
        let start = std::time::Instant::now();
        while seen.len() < 9 && start.elapsed() < std::time::Duration::from_secs(10) {
            if player.tick(0.25) {
                seen.push(player.shown.as_ref().unwrap().0);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 0, 1]);
        assert_eq!(player.shown.as_ref().unwrap().1.w, 48, "decoded at the size asked");
        // Not a time-lapse, or none at all.
        std::fs::write(dir.join("bad.avi"), b"RIFF\0\0\0\0WAVEjunk").unwrap();
        assert!(Avi::open(&dir.join("bad.avi")).is_err() && Avi::open(&dir.join("none.avi")).is_err());
        assert!(write_avi(&dir.join("empty.avi"), 96, 72, 5, || None).is_err() && !dir.join("empty.avi.part").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frames_left_behind_are_made_into_a_time_lapse_and_one_frame_is_a_photo() {
        let dir = scratch("assemble");
        let lapse = dir.join("LAPSE_20261124_101500.avi");
        let work = work_of(&lapse);
        std::fs::create_dir_all(&work).unwrap();
        for n in 1..=5u32 {
            std::fs::write(frame_path(&work, n), frame(64, 48, 10 * n as u8)).unwrap();
        }
        // A frame of another size (a still's fallback) is left out.
        std::fs::write(frame_path(&work, 6), frame(32, 24, 1)).unwrap();
        std::fs::write(work.join("notes.txt"), b"not a frame").unwrap();
        assert_eq!(frames_in(&work).len(), 6);
        match assemble(&lapse).unwrap() {
            Made::Lapse { frames, size, .. } => assert_eq!((frames, size), (5, (64, 48))),
            other => panic!("{other:?}"),
        }
        assert!(!work.exists(), "its frames' folder goes once it is made");
        let avi = Avi::open(&lapse).unwrap();
        assert_eq!(avi.frame_ms, 200, "five frames play at five a second");
        // One frame alone: a photo, under a photo's name.
        let one = dir.join("LAPSE_20261124_120000.avi");
        std::fs::create_dir_all(work_of(&one)).unwrap();
        std::fs::write(frame_path(&work_of(&one), 1), frame(64, 48, 99)).unwrap();
        assert_eq!(assemble(&one).unwrap(), Made::Photo(dir.join("IMG_20261124_120000.jpg")));
        assert!(!one.exists() && dir.join("IMG_20261124_120000.jpg").is_file());
        // None: nothing, and the folder gone.
        let none = dir.join("LAPSE_20261124_130000.avi");
        std::fs::create_dir_all(work_of(&none)).unwrap();
        assert_eq!(assemble(&none).unwrap(), Made::Nothing);
        assert!(!work_of(&none).exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
