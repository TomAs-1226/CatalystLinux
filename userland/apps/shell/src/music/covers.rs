//! Album covers: on the card beside every song kept, and decoded for the
//! pages that show them.
//!
//! Each album (or played song) has its cover twice: `cover.jpg` at 512 px
//! for a computer or a big view, and `cover-small.jpg` at 96 px, which is
//! what the player, the lock screen's live activity, the island and lists
//! decode (a 96 px JPEG is 4 KB and a millisecond). YouTube Music's own
//! square album art is asked for at each size (its image server scales);
//! a music video with only a 16:9 frame has the frame's middle square cut
//! out. A song of your own with no cover gets the one inside its file, or
//! the folder's picture, or one drawn: its album's initials on a colour
//! from its name.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::photos::Picture;

use super::polite::{self, Manner};
use super::{store, Track};

pub const BIG_PX: usize = 512;
pub const SMALL_PX: usize = 96;

// ------------------------------------------------------------ addresses ---

/// YouTube Music's square art at `px`: its image servers take the size in
/// the address. `None` for a video's 16:9 frame (cut square instead).
pub fn square_art(art: &str, px: usize) -> Option<String> {
    if !(art.contains("googleusercontent.com") || art.contains("ggpht.com")) {
        return None;
    }
    let base = match art.rfind('=') {
        Some(at) if art[at..].contains('w') || art[at..].contains('s') => &art[..at],
        _ => art,
    };
    Some(format!("{base}=w{px}-h{px}-l90-rj"))
}

/// A video's frame, the biggest first.
fn frames(id: &str) -> [String; 2] {
    [format!("https://i.ytimg.com/vi/{id}/maxresdefault.jpg"), format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")]
}

fn download(url: &str, to: &Path) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-f", "--max-time", "20", "--proto", "=https", "-o"]).arg(to).arg(url);
    let ok = polite::output_within(polite::polite(&mut cmd, Manner::Background), Duration::from_secs(25)).is_ok_and(|o| o.status.success());
    if !ok {
        std::fs::remove_file(to).ok();
    }
    ok && std::fs::metadata(to).is_ok_and(|m| m.len() > 100)
}

// ------------------------------------------------------------- pictures ---

/// A picture file's bytes (JPEG or PNG), decoded whole.
pub fn decode_bytes(bytes: &[u8]) -> Option<Picture> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut d = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
        // Big covers (3000 px scans) decoded at an eighth or so, never whole.
        if let Ok((w, h)) = d.scale(BIG_PX as u16, BIG_PX as u16) {
            let data = d.decode().ok()?;
            let info = d.info()?;
            let rgb = match info.pixel_format {
                jpeg_decoder::PixelFormat::RGB24 => data,
                jpeg_decoder::PixelFormat::L8 => data.iter().flat_map(|&v| [v, v, v]).collect(),
                _ => return None,
            };
            return Some(Picture { w: w as usize, h: h as usize, rgb });
        }
        return None;
    }
    if bytes.starts_with(b"\x89PNG") {
        let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
        d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut r = d.read_info().ok()?;
        let mut buf = vec![0; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        let (w, h) = (info.width as usize, info.height as usize);
        let px = &buf[..info.buffer_size()];
        let rgb: Vec<u8> = match info.color_type {
            png::ColorType::Rgb => px.to_vec(),
            png::ColorType::Rgba => px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
            png::ColorType::Grayscale => px.iter().flat_map(|&v| [v, v, v]).collect(),
            png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect(),
            _ => return None,
        };
        return Some(Picture { w, h, rgb });
    }
    None
}

/// The middle square of a picture (a 16:9 frame's, or a 4:3 one's with the
/// letterbox's black bars left out), at `px` a side.
pub fn square(p: &Picture, px: usize) -> Picture {
    // A 4:3 frame holding 16:9 between black bars: the picture is the middle 3/4.
    let (top, h) = if p.w * 3 == p.h * 4 && letterboxed(p) { (p.h / 8, p.h * 3 / 4) } else { (0, p.h) };
    let side = p.w.min(h).max(1);
    let (x0, y0) = ((p.w - side) / 2, top + (h - side) / 2);
    let mut rgb = Vec::with_capacity(px * px * 3);
    for y in 0..px {
        // The source rows (and below, columns) this pixel covers, averaged.
        let (sy0, sy1) = (y0 + y * side / px, (y0 + ((y + 1) * side / px).max(y * side / px + 1)).min(p.h));
        for x in 0..px {
            let (sx0, sx1) = (x0 + x * side / px, (x0 + ((x + 1) * side / px).max(x * side / px + 1)).min(p.w));
            let mut sum = [0u32; 3];
            let mut n = 0;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let i = (sy * p.w + sx) * 3;
                    for k in 0..3 {
                        sum[k] += p.rgb[i + k] as u32;
                    }
                    n += 1;
                }
            }
            for s in sum {
                rgb.push((s / n.max(1)) as u8);
            }
        }
    }
    Picture { w: px, h: px, rgb }
}

/// The top and bottom eighths are black: a 16:9 frame in a 4:3 thumbnail.
fn letterboxed(p: &Picture) -> bool {
    let band = p.h / 10;
    let dark = |y: usize| (0..p.w).step_by(8).all(|x| p.rgb[(y * p.w + x) * 3..][..3].iter().all(|&v| v < 24));
    band > 0 && (0..band).all(dark) && (p.h - band..p.h).all(dark)
}

pub fn write_jpeg(p: &Picture, to: &Path) -> bool {
    let mut bytes = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut bytes, 88);
    if enc.encode(&p.rgb, p.w as u16, p.h as u16, jpeg_encoder::ColorType::Rgb).is_err() {
        return false;
    }
    store::write_atomic(to, &bytes).is_ok()
}

/// A picture as the two covers (512 px, at most the picture's own size,
/// and 96 px).
pub fn write_pair(p: &Picture, big: &Path, small: &Path) -> bool {
    let side = p.w.min(p.h);
    write_jpeg(&square(p, side.min(BIG_PX)), big) && write_jpeg(&square(p, SMALL_PX), small)
}

/// A song's two covers, fetched: YouTube Music's square art at each size,
/// or its video's frame cut square.
pub fn fetch_pair(track: &Track, big: &Path, small: &Path) -> bool {
    if big.exists() && small.exists() {
        return true;
    }
    if !super::on_device() {
        return write_pair(&super::demo_cover(track), big, small);
    }
    if let (Some(b), Some(s)) = (square_art(&track.art, BIG_PX), square_art(&track.art, SMALL_PX)) {
        if download(&b, big) && download(&s, small) {
            return true;
        }
    }
    let tmp = super::private_file("frame");
    let got = frames(&track.id).iter().any(|u| download(u, &tmp));
    let picture = got.then(|| std::fs::read(&tmp).ok()).flatten().and_then(|b| decode_bytes(&b));
    std::fs::remove_file(&tmp).ok();
    picture.is_some_and(|p| write_pair(&p, big, small))
}

/// A played song's covers, kept with it in the cache.
pub fn cache_pair(id: &str) -> (PathBuf, PathBuf) {
    let id = store::safe_id(id);
    (store::cache_dir().join(format!("{id}.jpg")), store::cache_dir().join(format!("{id}-small.jpg")))
}

/// An album folder's covers.
pub fn folder_pair(dir: &Path) -> (PathBuf, PathBuf) {
    (dir.join("cover.jpg"), dir.join("cover-small.jpg"))
}

/// The cover drawn for an album that came without one: its initials, on a
/// colour from its name. Kept apart from your folders, in /data/music/covers.
pub fn made_pair(album: &str, artist: &str) -> (PathBuf, PathBuf) {
    let key = format!("{artist}\u{1f}{album}");
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    let dir = store::covers_dir();
    (dir.join(format!("{hash:016x}.jpg")), dir.join(format!("{hash:016x}-small.jpg")))
}

/// Up to two letters for a name: "Low Water" → "LW", "Homework" → "H".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '-' || c == '&').filter(|w| w.chars().next().is_some_and(char::is_alphanumeric)).collect();
    let skip = ["the", "a", "an", "of", "and"];
    let mut letters: Vec<char> = words.iter().filter(|w| !skip.contains(&w.to_lowercase().as_str())).filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).take(2).collect();
    if letters.is_empty() {
        letters = name.chars().filter(|c| c.is_alphanumeric()).take(1).flat_map(char::to_uppercase).collect();
    }
    if letters.is_empty() {
        "♪".into()
    } else {
        letters.into_iter().collect()
    }
}

/// A cover drawn: a soft gradient of two colours from the name, its
/// initials large and white in the middle.
pub fn draw_made(album: &str, artist: &str, px: usize) -> Picture {
    let name = if album.is_empty() { artist } else { album };
    let mut base = super::demo_cover(&Track { title: format!("{name}{artist}"), ..Track::default() });
    // The demo's 112 px gradient, smoothly to this size.
    if base.w != px {
        base = resize(&base, px);
    }
    let mut canvas = bezel::Canvas::new(px as i32, px as i32);
    for (i, c) in base.rgb.chunks_exact(3).enumerate() {
        // Darkened a little, so white letters read on any of its colours.
        canvas.px[i] = (c[0] as u32 * 3 / 4) << 16 | (c[1] as u32 * 3 / 4) << 8 | (c[2] as u32 * 3 / 4);
    }
    let mut text = bezel::Text::new();
    let letters = initials(name);
    let style = bezel::TextStyle::sans(px as f32 * 0.42, 640.0);
    let baseline = px as f32 * 0.5 + text.cap_height(&style) * 0.5;
    text.draw_aligned(&mut canvas, px as f32 * 0.5, baseline, bezel::Align::Center, &letters, &style, bezel::Color::WHITE, 0.95);
    let rgb = canvas.px.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect();
    Picture { w: px, h: px, rgb }
}

/// Bilinear, for the drawn cover's background.
fn resize(p: &Picture, px: usize) -> Picture {
    let mut rgb = Vec::with_capacity(px * px * 3);
    for y in 0..px {
        let fy = (y as f32 + 0.5) * p.h as f32 / px as f32 - 0.5;
        let (y0, ty) = (fy.floor().max(0.0) as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(p.h - 1);
        for x in 0..px {
            let fx = (x as f32 + 0.5) * p.w as f32 / px as f32 - 0.5;
            let (x0, tx) = (fx.floor().max(0.0) as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(p.w - 1);
            for k in 0..3 {
                let at = |xx: usize, yy: usize| p.rgb[(yy * p.w + xx) * 3 + k] as f32;
                let v = at(x0, y0) * (1.0 - tx) * (1.0 - ty) + at(x1, y0) * tx * (1.0 - ty) + at(x0, y1) * (1.0 - tx) * ty + at(x1, y1) * tx * ty;
                rgb.push(v.round() as u8);
            }
        }
    }
    Picture { w: px, h: px, rgb }
}

/// The drawn covers for an album, made if they are not there yet.
pub fn ensure_made(album: &str, artist: &str) -> PathBuf {
    let (big, small) = made_pair(album, artist);
    if !small.exists() {
        write_jpeg(&draw_made(album, artist, BIG_PX), &big);
        write_jpeg(&draw_made(album, artist, SMALL_PX), &small);
    }
    small
}

/// The small cover for a song on the card, if it has one there: the path
/// the lock screen's live activity is given, and what lists decode.
pub fn small_for(track: &Track) -> Option<PathBuf> {
    if let Some(local) = &track.local {
        return super::library::small_cover(local);
    }
    if let Some(song) = super::library::by_youtube(&track.id) {
        return super::library::small_cover(&song.path);
    }
    Some(cache_pair(&track.id).1).filter(|p| p.exists())
}

// -------------------------------------------------- decoded, for pages ---

/// Covers decoded for the pages, fetched one at a time (a few dozen KB
/// each kept).
struct Covers {
    ready: HashMap<String, Arc<Picture>>,
    order: VecDeque<String>,
    wanted: VecDeque<Track>,
    fetching: Option<String>,
    busy: bool,
}

static COVERS: Mutex<Option<Covers>> = Mutex::new(None);
/// Counts covers as they arrive, so a page showing them draws again.
pub static ARRIVED: AtomicU64 = AtomicU64::new(0);
const KEPT: usize = 40;

/// A song's cover if it is to hand; else asked for, and `None` for now.
pub fn cover(track: &Track) -> Option<Arc<Picture>> {
    let key = cover_key(track);
    let mut covers = COVERS.lock().unwrap();
    let c = covers.get_or_insert_with(|| Covers { ready: HashMap::new(), order: VecDeque::new(), wanted: VecDeque::new(), fetching: None, busy: false });
    if let Some(p) = c.ready.get(&key) {
        return Some(p.clone());
    }
    if !super::on_device() && track.local.is_none() {
        let p = Arc::new(super::demo_cover(track));
        c.ready.insert(key.clone(), p.clone());
        c.order.push_back(key);
        return Some(p);
    }
    if c.fetching.as_deref() == Some(key.as_str()) || c.wanted.iter().any(|w| cover_key(w) == key) {
        return None;
    }
    c.wanted.push_back(track.clone());
    if !c.busy {
        c.busy = true;
        std::thread::Builder::new().name("music-covers".into()).stack_size(256 * 1024).spawn(fetch_covers).ok();
    }
    None
}

/// Songs of one album share a cover.
fn cover_key(t: &Track) -> String {
    match &t.local {
        Some(p) => p.parent().map_or_else(|| t.id.clone(), |d| d.display().to_string()),
        None => t.id.clone(),
    }
}

fn fetch_covers() {
    polite::background_thread();
    loop {
        let track = {
            let mut covers = COVERS.lock().unwrap();
            let c = covers.as_mut().unwrap();
            match c.wanted.pop_front() {
                Some(t) => {
                    c.fetching = Some(cover_key(&t));
                    t
                }
                None => {
                    c.busy = false;
                    return;
                }
            }
        };
        let picture = load(&track).unwrap_or_default();
        let mut covers = COVERS.lock().unwrap();
        let c = covers.as_mut().unwrap();
        let key = cover_key(&track);
        c.fetching = None;
        c.ready.insert(key.clone(), Arc::new(picture));
        c.order.push_back(key);
        while c.order.len() > KEPT {
            if let Some(old) = c.order.pop_front() {
                c.ready.remove(&old);
            }
        }
        ARRIVED.fetch_add(1, Ordering::Relaxed);
    }
}

/// From the card if it is there (the small one), else YouTube's 96 px.
fn load(track: &Track) -> Option<Picture> {
    if let Some(small) = small_for(track) {
        if let Ok(p) = crate::photos::decode(&small, SMALL_PX, SMALL_PX) {
            return Some(p);
        }
    }
    if track.local.is_some() {
        let s = super::library::song_for(track)?;
        return crate::photos::decode(&ensure_made(&s.album, &s.album_artist_or_artist()), SMALL_PX, SMALL_PX).ok();
    }
    let url = square_art(&track.art, SMALL_PX).or_else(|| (!track.art.is_empty()).then(|| track.art.clone()))?;
    let path = super::private_file("cover");
    let picture = if download(&url, &path) { std::fs::read(&path).ok().and_then(|b| decode_bytes(&b)).map(|p| square(&p, SMALL_PX)) } else { None };
    std::fs::remove_file(&path).ok();
    picture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_musics_art_is_asked_for_square_at_each_size() {
        let art = "https://lh3.googleusercontent.com/abc=w226-h226-l90-rj";
        assert_eq!(square_art(art, 512).as_deref(), Some("https://lh3.googleusercontent.com/abc=w512-h512-l90-rj"));
        assert_eq!(square_art("https://yt3.ggpht.com/xyz=s88-c-k", 96).as_deref(), Some("https://yt3.ggpht.com/xyz=w96-h96-l90-rj"));
        assert_eq!(square_art("https://i.ytimg.com/vi/a/hqdefault.jpg", 96), None, "a video's frame is cut square instead");
    }

    #[test]
    fn a_letterboxed_frame_gives_its_picture_not_its_black_bars() {
        // 480 × 360 with black top and bottom eighths, red between.
        let (w, h) = (480, 360);
        let rgb = (0..w * h).flat_map(|i| if (45..315).contains(&(i / w)) { [200u8, 30, 30] } else { [0, 0, 0] }).collect();
        let s = square(&Picture { w, h, rgb }, 96);
        assert_eq!((s.w, s.h), (96, 96));
        assert!(s.rgb.chunks_exact(3).all(|p| p[0] > 190 && p[1] < 40), "no black in the square");
        // A wide frame: its middle.
        let rgb = (0..64 * 36).flat_map(|i| if (14..50).contains(&(i % 64)) { [0u8, 0, 250] } else { [250, 250, 0] }).collect();
        let s = square(&Picture { w: 64, h: 36, rgb }, 12);
        assert!(s.rgb.chunks_exact(3).all(|p| p[2] > 240), "the middle square only");
    }

    #[test]
    fn a_cover_is_drawn_from_the_albums_initials() {
        assert_eq!((initials("Low Water"), initials("The Northfields"), initials("Homework"), initials("¡!"), initials("")), ("LW".into(), "N".into(), "H".into(), "♪".into(), "♪".into()));
        let p = draw_made("Low Water", "Wren & the Tide", SMALL_PX);
        assert_eq!((p.w, p.h, p.rgb.len()), (96, 96, 96 * 96 * 3));
        // White letters in the middle; the corners are the colour.
        let at = |x: usize, y: usize| &p.rgb[(y * 96 + x) * 3..][..3];
        let bright = (30..66).flat_map(|y| (20..76).map(move |x| (x, y))).filter(|&(x, y)| at(x, y).iter().all(|&v| v > 220)).count();
        assert!(bright > 150, "{bright} white pixels");
        assert!(at(2, 2).iter().any(|&v| v < 200));
        // Written and read back as a JPEG.
        let dir = store::root().join("covers-test");
        let (big, small) = (dir.join("c.jpg"), dir.join("c-small.jpg"));
        assert!(write_pair(&draw_made("Low Water", "Wren", BIG_PX), &big, &small));
        let back = decode_bytes(&std::fs::read(&big).unwrap()).unwrap();
        assert_eq!((back.w, back.h), (512, 512));
        assert_eq!(crate::photos::decode(&small, 96, 96).map(|p| (p.w, p.h)), Ok((96, 96)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
